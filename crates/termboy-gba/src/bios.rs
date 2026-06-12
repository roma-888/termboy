//! HLE BIOS: SWI functions implemented in Rust (design spec §4 — the
//! self-contained choice). Calls run in the caller's context (no SVC entry);
//! the `Cpu::hle_bios` flag is the seam for swapping in a real BIOS image.

use crate::cpu::Cpu;
use crate::cpu::registers::Registers;

/// BIOS_IF: the user ISR records acknowledged IRQs here for IntrWait.
const BIOS_IF: u32 = 0x0300_7FF8;

impl Cpu {
    pub(crate) fn bios_call(&mut self, function: u32) {
        match function {
            0x00 => self.soft_reset(),
            0x01 => self.register_ram_reset(),
            0x02 | 0x03 => self.bus.halted = true, // Halt / Stop
            0x04 => self.intr_wait(self.regs.get(0) != 0, self.regs.get(1) as u16),
            0x05 => self.intr_wait(true, 1), // VBlankIntrWait
            0x06 => self.div(self.regs.get(0) as i32, self.regs.get(1) as i32),
            0x07 => self.div(self.regs.get(1) as i32, self.regs.get(0) as i32), // DivArm
            0x08 => {
                let r = isqrt(self.regs.get(0));
                self.regs.set(0, r);
            }
            0x09 => {
                let r = arctan(self.regs.get(0) as i32);
                self.regs.set(0, r as u32);
            }
            0x0A => {
                let r = arctan2(self.regs.get(0) as i32, self.regs.get(1) as i32);
                self.regs.set(0, r);
            }
            0x0D => self.regs.set(0, 0xBAAE_187F), // GetBiosChecksum
            0x19 => {} // SoundBias: no APU until G6
            // The BIOS-resident sound driver: m4a games carry their own copy
            // in ROM and never call these (except MidiKey2Freq below).
            0x1A..=0x1E | 0x20..=0x24 | 0x28 | 0x29 => {}
            0x1F => self.midi_key_to_freq(),
            0x25 => self.regs.set(0, 1), // MultiBoot: report failure
            f => unimplemented!(
                "HLE BIOS function {f:#04X} at {:#010X}",
                self.exec_addr()
            ),
        }
    }

    /// Div: r0 = n/d, r1 = n%d, r3 = |n/d|.
    fn div(&mut self, n: i32, d: i32) {
        assert!(d != 0, "BIOS Div by zero at {:#010X}", self.exec_addr());
        let q = n.wrapping_div(d);
        self.regs.set(0, q as u32);
        self.regs.set(1, n.wrapping_rem(d) as u32);
        self.regs.set(3, q.unsigned_abs());
    }

    /// IntrWait. While unsatisfied the PC is parked back on the SWI so it
    /// re-executes after every ISR — the real BIOS's Halt loop, HLE'd.
    /// `intrwait` remembers that the discard already happened.
    fn intr_wait(&mut self, discard: bool, flags: u16) {
        let flags = flags & 0x3FFF;
        if self.intrwait.is_none() {
            self.intrwait = Some(flags);
            if discard {
                let cur = self.bus.read16(BIOS_IF);
                self.bus.write16(BIOS_IF, cur & !flags);
            }
        }
        self.bus.write16(0x0400_0208, 1); // IntrWait forces IME on
        let cur = self.bus.read16(BIOS_IF);
        if cur & flags != 0 {
            self.bus.write16(BIOS_IF, cur & !flags);
            self.intrwait = None;
            return; // satisfied: fall through past the SWI
        }
        self.bus.halted = true;
        let pc = self.exec_addr();
        self.regs.set(15, pc);
        self.flush();
    }

    /// MidiKey2Freq: r0 = WaveData*, r1 = key, r2 = fine. The one BIOS
    /// sound call m4a games (Pokémon included) actually make.
    fn midi_key_to_freq(&mut self) {
        let base = self.bus.read32(self.regs.get(0).wrapping_add(4)) as f64;
        let key = self.regs.get(1) as f64;
        let fine = self.regs.get(2) as f64;
        let freq = base / f64::exp2((180.0 - key - fine / 256.0) / 12.0);
        self.regs.set(0, freq as u32);
    }

    /// SoftReset: clear the BIOS work area, reset banks/stacks/mode, and
    /// restart at the entry the flag at 0x03007FFA selects.
    fn soft_reset(&mut self) {
        let flag = self.bus.read8(0x0300_7FFA);
        for a in (0x0300_7E00..0x0300_8000).step_by(4) {
            self.bus.write32(a, 0);
        }
        self.regs = Registers::new_post_bios();
        self.regs.set(15, if flag == 0 { 0x0800_0000 } else { 0x0200_0000 });
        self.flush();
    }

    /// RegisterRamReset, coarsely: wipe the selected regions through the
    /// normal bus paths (so the IO write hooks fire) and blank the screen.
    fn register_ram_reset(&mut self) {
        let f = self.regs.get(0);
        if f & 0x01 != 0 {
            self.wipe(0x0200_0000, 0x0204_0000);
        }
        if f & 0x02 != 0 {
            self.wipe(0x0300_0000, 0x0300_7E00); // IWRAM minus the BIOS area
        }
        if f & 0x04 != 0 {
            self.wipe(0x0500_0000, 0x0500_0400);
        }
        if f & 0x08 != 0 {
            self.wipe(0x0600_0000, 0x0601_8000);
        }
        if f & 0x10 != 0 {
            self.wipe(0x0700_0000, 0x0700_0400);
        }
        if f & 0x20 != 0 {
            // SIO registers (the keypad pair at 0x130/0x132 is excluded)
            self.wipe(0x0400_0120, 0x0400_0130);
            self.wipe(0x0400_0134, 0x0400_015C);
        }
        if f & 0x40 != 0 {
            self.wipe(0x0400_0060, 0x0400_00B0); // sound registers
        }
        if f & 0x80 != 0 {
            // "all other registers": video (sans DISPCNT), DMA, timers, IE/IME
            self.wipe(0x0400_0004, 0x0400_0060);
            self.wipe(0x0400_00B0, 0x0400_0110);
            self.wipe(0x0400_0200, 0x0400_020C);
        }
        self.bus.write16(0x0400_0000, 0x0080); // always leaves forced blank
    }

    fn wipe(&mut self, lo: u32, hi: u32) {
        for a in (lo..hi).step_by(4) {
            self.bus.write32(a, 0);
        }
    }
}

/// Floor square root, exact for all u32 (the float seed is then corrected).
fn isqrt(n: u32) -> u32 {
    let n = n as u64;
    let mut x = (n as f64).sqrt() as u64;
    while x * x > n {
        x -= 1;
    }
    while (x + 1) * (x + 1) <= n {
        x += 1;
    }
    x as u32
}

/// The BIOS's own arctan polynomial, bit-exact (1.14 fixed-point in/out).
fn arctan(i: i32) -> i32 {
    let a = -((i.wrapping_mul(i)) >> 14);
    let mut b = ((0xA9 * a) >> 14) + 0x390;
    b = ((b * a) >> 14) + 0x91C;
    b = ((b * a) >> 14) + 0xFB6;
    b = ((b * a) >> 14) + 0x16AA;
    b = ((b * a) >> 14) + 0x2081;
    b = ((b * a) >> 14) + 0x3651;
    b = ((b * a) >> 14) + 0xA2F9;
    i.wrapping_mul(b) >> 16
}

/// ArcTan2 with the BIOS's quadrant handling (result: 0..0xFFFF, 0x4000=90°).
fn arctan2(x: i32, y: i32) -> u32 {
    if y == 0 {
        return ((x >> 16) & 0x8000) as u32;
    }
    if x == 0 {
        return (((y >> 16) & 0x8000) + 0x4000) as u32;
    }
    if x.abs() > y.abs() || (x.abs() == y.abs() && (x >= 0 || y >= 0)) {
        let t = arctan((y << 14).wrapping_div(x));
        if x < 0 {
            (0x8000 + t) as u32
        } else {
            ((((y >> 16) & 0x8000) << 1) + t) as u32
        }
    } else {
        let t = arctan((x << 14).wrapping_div(y));
        ((((y >> 16) & 0x8000) + 0x4000) - t) as u32
    }
}

#[cfg(test)]
mod tests {
    use crate::cpu::Cpu;
    use crate::cpu::tests::cpu_with;

    /// Execute one SWI `f` with r0-r2 preloaded.
    fn swi(f: u32, r: [u32; 3]) -> Cpu {
        let mut cpu = cpu_with(&[0xEF00_0000 | (f << 16)]);
        for (i, v) in r.into_iter().enumerate() {
            cpu.regs.set(i, v);
        }
        cpu.step();
        cpu
    }

    #[test]
    fn divarm_swaps_the_operands() {
        let cpu = swi(0x07, [3, 100, 0]);
        assert_eq!(cpu.regs.get(0), 33);
        assert_eq!(cpu.regs.get(1), 1);
    }

    #[test]
    fn sqrt_is_exact_at_boundaries() {
        for (n, r) in [(0u32, 0u32), (1, 1), (3, 1), (4, 2), (99, 9), (100, 10), (0xFFFF_FFFF, 0xFFFF)] {
            assert_eq!(swi(0x08, [n, 0, 0]).regs.get(0), r, "sqrt({n})");
        }
    }

    #[test]
    fn arctan2_hits_the_cardinals_and_diagonal() {
        assert_eq!(swi(0x0A, [0x100, 0, 0]).regs.get(0), 0);
        assert_eq!(swi(0x0A, [(-0x100i32) as u32, 0, 0]).regs.get(0), 0x8000);
        assert_eq!(swi(0x0A, [0, 0x100, 0]).regs.get(0), 0x4000);
        assert_eq!(swi(0x0A, [0, (-0x100i32) as u32, 0]).regs.get(0), 0xC000);
        let diag = swi(0x0A, [0x100, 0x100, 0]).regs.get(0);
        assert!((0x1FFF..=0x2001).contains(&diag), "45 deg = {diag:#X}");
    }

    #[test]
    fn bios_checksum_is_the_ds_value() {
        assert_eq!(swi(0x0D, [0, 0, 0]).regs.get(0), 0xBAAE_187F);
    }

    #[test]
    fn midi_key_to_freq_matches_the_reference_points() {
        let mut cpu = cpu_with(&[0xEF1F_0000]);
        cpu.bus.write32(0x0200_0004, 88_0000); // WaveData base frequency word
        cpu.regs.set(0, 0x0200_0000);
        cpu.regs.set(1, 180); // at key 180, fine 0: result == base
        cpu.regs.set(2, 0);
        cpu.step();
        assert_eq!(cpu.regs.get(0), 88_0000);
        let mut cpu = cpu_with(&[0xEF1F_0000]);
        cpu.bus.write32(0x0200_0004, 88_0000);
        cpu.regs.set(0, 0x0200_0000);
        cpu.regs.set(1, 168); // one octave down: half
        cpu.step();
        assert_eq!(cpu.regs.get(0), 44_0000);
    }

    #[test]
    fn sound_driver_calls_are_inert() {
        let cpu = swi(0x1A, [0, 0, 0]); // SoundDriverInit must not panic
        assert_eq!(cpu.regs.cpsr.mode(), crate::cpu::psr::Mode::System);
    }

    #[test]
    fn register_ram_reset_wipes_selected_regions_and_blanks() {
        let mut cpu = cpu_with(&[0xEF01_0000]);
        cpu.bus.write32(0x0200_0000, 0xDEAD_BEEF);
        cpu.bus.write32(0x0300_0000, 0xDEAD_BEEF);
        cpu.bus.write32(0x0300_7F00, 0xCAFE_F00D); // stack area: preserved
        cpu.regs.set(0, 0x01); // EWRAM only
        cpu.step();
        assert_eq!(cpu.bus.read32(0x0200_0000), 0);
        assert_eq!(cpu.bus.read32(0x0300_0000), 0xDEAD_BEEF);
        assert_eq!(cpu.bus.read32(0x0300_7F00), 0xCAFE_F00D);
        assert_eq!(cpu.bus.read16(0x0400_0000), 0x0080); // forced blank
    }

    #[test]
    fn soft_reset_restarts_at_the_cartridge() {
        let mut cpu = cpu_with(&[0xEF00_0000]);
        cpu.bus.write8(0x0300_7FFA, 0); // return-to-ROM flag
        cpu.regs.set(4, 0x1234);
        cpu.step();
        assert_eq!(cpu.exec_addr(), 0x0800_0000);
        assert_eq!(cpu.regs.get(13), 0x0300_7F00); // stacks re-set
        assert_eq!(cpu.bus.read32(0x0300_7FF8), 0); // BIOS area cleared
    }
}
