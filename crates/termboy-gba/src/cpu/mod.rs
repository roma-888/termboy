pub mod psr;
pub mod registers;
mod arm;
#[cfg(test)]
mod tests;

use crate::bus::Bus;
use psr::{I, Mode, T};
use registers::Registers;

pub struct Cpu {
    pub regs: Registers,
    pub bus: Bus,
    /// Prefetch queue: [0] = next to execute, [1] = the one after.
    pipeline: [u32; 2],
    flushed: bool,
    /// HLE BIOS seam: true = SWIs dispatch to bios.rs, false = jump to the
    /// hardware vector (requires a real BIOS image — future option).
    pub hle_bios: bool,
}

impl Cpu {
    pub fn new(bus: Bus) -> Self {
        let mut cpu = Self {
            regs: Registers::new_post_bios(),
            bus,
            pipeline: [0; 2],
            flushed: false,
            hle_bios: true,
        };
        cpu.regs.set(15, 0x0800_0000);
        cpu.flush();
        cpu
    }

    /// Address of the instruction `step()` will execute next.
    pub fn exec_addr(&self) -> u32 {
        let i = if self.regs.cpsr.thumb() { 2 } else { 4 };
        self.regs.get(15).wrapping_sub(2 * i)
    }

    /// Refill the pipeline after any write to r15. Also masks alignment,
    /// so callers can pass BX-style targets straight through.
    pub(crate) fn flush(&mut self) {
        if self.regs.cpsr.thumb() {
            let pc = self.regs.get(15) & !1;
            self.pipeline[0] = self.bus.read16(pc) as u32;
            self.pipeline[1] = self.bus.read16(pc.wrapping_add(2)) as u32;
            self.regs.set(15, pc.wrapping_add(4));
        } else {
            let pc = self.regs.get(15) & !3;
            self.pipeline[0] = self.bus.read32(pc);
            self.pipeline[1] = self.bus.read32(pc.wrapping_add(4));
            self.regs.set(15, pc.wrapping_add(8));
        }
        self.flushed = true;
    }

    /// Execute one instruction.
    pub fn step(&mut self) {
        self.flushed = false;
        if self.regs.cpsr.thumb() {
            let op = self.pipeline[0] as u16;
            self.pipeline[0] = self.pipeline[1];
            self.pipeline[1] = self.bus.read16(self.regs.get(15)) as u32;
            self.exec_thumb(op);
            if !self.flushed {
                self.regs.set(15, self.regs.get(15).wrapping_add(2));
            }
        } else {
            let op = self.pipeline[0];
            self.pipeline[0] = self.pipeline[1];
            self.pipeline[1] = self.bus.read32(self.regs.get(15));
            if self.check_cond((op >> 28) as u8) {
                self.exec_arm(op);
            }
            if !self.flushed {
                self.regs.set(15, self.regs.get(15).wrapping_add(4));
            }
        }
    }

    pub(crate) fn check_cond(&self, cond: u8) -> bool {
        use psr::{C, N, V, Z};
        let p = self.regs.cpsr;
        match cond {
            0x0 => p.flag(Z),
            0x1 => !p.flag(Z),
            0x2 => p.flag(C),
            0x3 => !p.flag(C),
            0x4 => p.flag(N),
            0x5 => !p.flag(N),
            0x6 => p.flag(V),
            0x7 => !p.flag(V),
            0x8 => p.flag(C) && !p.flag(Z),
            0x9 => !p.flag(C) || p.flag(Z),
            0xA => p.flag(N) == p.flag(V),
            0xB => p.flag(N) != p.flag(V),
            0xC => !p.flag(Z) && p.flag(N) == p.flag(V),
            0xD => p.flag(Z) || p.flag(N) != p.flag(V),
            0xE => true,
            _ => false, // 0xF: unconditional-never on ARMv4
        }
    }

    /// Enter an exception: bank-switch, stash CPSR in the new SPSR, go ARM,
    /// mask IRQs, jump to the vector. `lr` is mode-specific (see callers).
    pub(crate) fn exception(&mut self, vector: u32, mode: Mode, lr: u32) {
        let old = self.regs.cpsr;
        self.regs.switch_mode(mode);
        self.regs.set_spsr(old);
        self.regs.cpsr.set_flag(T, false);
        self.regs.cpsr.set_flag(I, true);
        self.regs.set(14, lr);
        self.regs.set(15, vector);
        self.flush();
    }

    /// Hardware IRQ entry. Wired to IE/IF/IME in G4; the exception mechanics
    /// are unit-tested now so G4 is pure plumbing.
    pub fn irq(&mut self) {
        if self.regs.cpsr.flag(I) {
            return;
        }
        let lr = self.exec_addr().wrapping_add(4);
        self.exception(0x18, Mode::Irq, lr);
    }

    pub(crate) fn set_nz(&mut self, result: u32) {
        self.regs.cpsr.set_flag(psr::N, result >> 31 != 0);
        self.regs.cpsr.set_flag(psr::Z, result == 0);
    }

    pub(crate) fn set_nzc(&mut self, result: u32, carry: bool) {
        self.set_nz(result);
        self.regs.cpsr.set_flag(psr::C, carry);
    }

    /// a + b + carry_in, optionally setting NZCV.
    pub(crate) fn add_flags(&mut self, a: u32, b: u32, carry_in: u32, s: bool) -> u32 {
        let result = a.wrapping_add(b).wrapping_add(carry_in);
        if s {
            self.set_nz(result);
            let c = (a as u64) + (b as u64) + (carry_in as u64) > 0xFFFF_FFFF;
            self.regs.cpsr.set_flag(psr::C, c);
            self.regs.cpsr.set_flag(psr::V, ((a ^ result) & (b ^ result)) >> 31 != 0);
        }
        result
    }

    /// a - b - !carry_in. SUB/CMP pass carry_in=1, SBC passes the C flag.
    /// (Computed as a + !b + carry_in, which yields the right C and V.)
    pub(crate) fn sub_flags(&mut self, a: u32, b: u32, carry_in: u32, s: bool) -> u32 {
        self.add_flags(a, !b, carry_in, s)
    }

    /// Barrel shifter. `imm` = the amount came from an immediate field
    /// (amount 0 then has the special meanings). Returns (result, carry-out).
    /// Does NOT touch flags — callers decide (memory offsets never do).
    pub(crate) fn shift(&mut self, ty: u32, value: u32, amount: u32, imm: bool) -> (u32, bool) {
        let cin = self.regs.cpsr.flag(psr::C);
        match ty {
            0 => match amount {
                // LSL
                0 => (value, cin),
                1..=31 => (value << amount, (value >> (32 - amount)) & 1 != 0),
                32 => (0, value & 1 != 0),
                _ => (0, false),
            },
            1 => {
                // LSR (imm 0 = 32)
                let amount = if imm && amount == 0 { 32 } else { amount };
                match amount {
                    0 => (value, cin),
                    1..=31 => (value >> amount, (value >> (amount - 1)) & 1 != 0),
                    32 => (0, value >> 31 != 0),
                    _ => (0, false),
                }
            }
            2 => {
                // ASR (imm 0 = 32; >=32 saturates to the sign bit)
                let amount = if imm && amount == 0 { 32 } else { amount };
                match amount {
                    0 => (value, cin),
                    1..=31 => {
                        (((value as i32) >> amount) as u32, (value >> (amount - 1)) & 1 != 0)
                    }
                    _ => {
                        let r = ((value as i32) >> 31) as u32;
                        (r, r & 1 != 0)
                    }
                }
            }
            _ => {
                // ROR (imm 0 = RRX)
                if imm && amount == 0 {
                    ((value >> 1) | ((cin as u32) << 31), value & 1 != 0)
                } else if amount == 0 {
                    (value, cin)
                } else if amount & 31 == 0 {
                    (value, value >> 31 != 0)
                } else {
                    let amount = amount & 31;
                    (value.rotate_right(amount), (value >> (amount - 1)) & 1 != 0)
                }
            }
        }
    }

    /// Word load with the ARM7 misalignment behavior: aligned fetch, then
    /// rotate so the addressed byte lands in the low lane.
    pub(crate) fn load_rotated(&mut self, addr: u32) -> u32 {
        let value = self.bus.read32(addr & !3);
        value.rotate_right((addr & 3) * 8)
    }

    // exec_thumb lives in thumb.rs (Task 12); stub so the step loop compiles:
    pub(crate) fn exec_thumb(&mut self, op: u16) {
        unimplemented!("thumb opcode {op:#06X} at {:#010X}", self.exec_addr())
    }
}
