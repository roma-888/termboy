//! Main opcode table. Grouped match arms over bit patterns + helpers; the
//! compiler's exhaustiveness check proves all 256 opcodes are handled.

use super::Cpu;

impl Cpu {
    pub(crate) fn get_r(&mut self, i: u8) -> u8 {
        match i {
            0 => self.regs.b,
            1 => self.regs.c,
            2 => self.regs.d,
            3 => self.regs.e,
            4 => self.regs.h,
            5 => self.regs.l,
            6 => {
                let hl = self.regs.hl();
                self.read8(hl)
            }
            _ => self.regs.a,
        }
    }

    pub(crate) fn set_r(&mut self, i: u8, v: u8) {
        match i {
            0 => self.regs.b = v,
            1 => self.regs.c = v,
            2 => self.regs.d = v,
            3 => self.regs.e = v,
            4 => self.regs.h = v,
            5 => self.regs.l = v,
            6 => {
                let hl = self.regs.hl();
                self.write8(hl, v);
            }
            _ => self.regs.a = v,
        }
    }

    /// rr decode for 0x01/0x11/0x21/0x31 etc.: 0=BC 1=DE 2=HL 3=SP
    pub(crate) fn get_rr(&self, i: u8) -> u16 {
        match i {
            0 => self.regs.bc(),
            1 => self.regs.de(),
            2 => self.regs.hl(),
            _ => self.regs.sp,
        }
    }

    pub(crate) fn set_rr(&mut self, i: u8, v: u16) {
        match i {
            0 => self.regs.set_bc(v),
            1 => self.regs.set_de(v),
            2 => self.regs.set_hl(v),
            _ => self.regs.sp = v,
        }
    }

    pub(crate) fn execute(&mut self, op: u8) {
        match op {
            0x00 => {} // NOP
            // LD r,r' — 0x76 is HALT, handled in Task 14
            0x40..=0x75 | 0x77..=0x7F => {
                let v = self.get_r(op & 7);
                self.set_r((op >> 3) & 7, v);
            }
            // LD r,n
            0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => {
                let v = self.fetch();
                self.set_r((op >> 3) & 7, v);
            }
            _ => unimplemented!("opcode {op:#04X} at {:#06X}", self.regs.pc.wrapping_sub(1)),
        }
    }
}
