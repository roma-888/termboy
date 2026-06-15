pub mod registers;
mod execute;
#[cfg(test)]
mod tests;

use crate::bus::Bus;
use registers::Registers;
use termboy_core::state::{Reader, StateError, Writer};

pub struct Cpu {
    pub regs: Registers,
    pub bus: Bus,
    pub ime: bool,
    ime_pending: bool,
    pub halted: bool,
    halt_bug: bool,
}

impl Cpu {
    pub fn new(bus: Bus) -> Self {
        Self {
            regs: Registers::new_post_boot(),
            bus,
            ime: false,
            ime_pending: false,
            halted: false,
            halt_bug: false,
        }
    }

    /// Every CPU memory access costs exactly one M-cycle — this pair of
    /// methods is the timing model (design spec §3). Never bypass them.
    pub(crate) fn read8(&mut self, addr: u16) -> u8 {
        self.bus.tick();
        self.bus.read(addr)
    }

    pub(crate) fn write8(&mut self, addr: u16, value: u8) {
        self.bus.tick();
        self.bus.write(addr, value);
    }

    pub(crate) fn fetch(&mut self) -> u8 {
        let v = self.read8(self.regs.pc);
        if self.halt_bug {
            self.halt_bug = false; // PC fails to advance once: next fetch repeats this byte
        } else {
            self.regs.pc = self.regs.pc.wrapping_add(1);
        }
        v
    }

    pub(crate) fn fetch16(&mut self) -> u16 {
        let lo = self.fetch();
        let hi = self.fetch();
        u16::from_le_bytes([lo, hi])
    }

    /// Execute one instruction (or service one interrupt / halt cycle).
    pub fn step(&mut self) {
        if self.halted {
            if self.bus.inte & self.bus.intf & 0x1F == 0 {
                self.bus.tick(); // time passes while halted
                return;
            }
            self.halted = false;
        }
        if self.ime && (self.bus.inte & self.bus.intf & 0x1F) != 0 {
            self.service_interrupt();
            return;
        }
        // EI takes effect after the *following* instruction completes.
        if self.ime_pending {
            self.ime_pending = false;
            self.ime = true;
        }
        let op = self.fetch();
        self.execute(op);
    }

    fn service_interrupt(&mut self) {
        // 5 M-cycles total: 2 internal + push (internal + 2 writes) below.
        self.ime = false;
        self.bus.tick();
        self.bus.tick();
        self.push16(self.regs.pc);
        let pending = self.bus.inte & self.bus.intf & 0x1F;
        let n = pending.trailing_zeros() as u16; // bit 0 (VBlank) has priority
        self.bus.intf &= !(1 << n) as u8;
        self.regs.pc = 0x0040 + n * 8;
    }

    pub(crate) fn push16(&mut self, value: u16) {
        self.bus.tick(); // internal cycle before the writes
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.write8(self.regs.sp, (value >> 8) as u8);
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.write8(self.regs.sp, value as u8);
    }

    pub(crate) fn pop16(&mut self) -> u16 {
        let lo = self.read8(self.regs.sp);
        self.regs.sp = self.regs.sp.wrapping_add(1);
        let hi = self.read8(self.regs.sp);
        self.regs.sp = self.regs.sp.wrapping_add(1);
        u16::from_le_bytes([lo, hi])
    }

    pub(crate) fn serialize(&self, w: &mut Writer) {
        self.regs.serialize(w);
        w.put_bool(self.ime);
        w.put_bool(self.ime_pending);
        w.put_bool(self.halted);
        w.put_bool(self.halt_bug);
        self.bus.serialize(w);
    }

    pub(crate) fn deserialize(&mut self, r: &mut Reader) -> Result<(), StateError> {
        self.regs.deserialize(r)?;
        self.ime = r.get_bool()?;
        self.ime_pending = r.get_bool()?;
        self.halted = r.get_bool()?;
        self.halt_bug = r.get_bool()?;
        self.bus.deserialize(r)?;
        Ok(())
    }
}
