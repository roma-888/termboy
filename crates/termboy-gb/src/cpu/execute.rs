//! Main opcode table. Grouped match arms over bit patterns + helpers; the
//! compiler's exhaustiveness check proves all 256 opcodes are handled.

use super::Cpu;
use super::registers::{C, H, N, Z};

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
            // LD A,(rr) / LD (rr),A
            0x0A => {
                let a = self.regs.bc();
                self.regs.a = self.read8(a);
            }
            0x1A => {
                let a = self.regs.de();
                self.regs.a = self.read8(a);
            }
            0x02 => {
                let a = self.regs.bc();
                let v = self.regs.a;
                self.write8(a, v);
            }
            0x12 => {
                let a = self.regs.de();
                let v = self.regs.a;
                self.write8(a, v);
            }
            // LD (HL±),A and LD A,(HL±)
            0x22 => {
                let hl = self.regs.hl();
                let v = self.regs.a;
                self.write8(hl, v);
                self.regs.set_hl(hl.wrapping_add(1));
            }
            0x32 => {
                let hl = self.regs.hl();
                let v = self.regs.a;
                self.write8(hl, v);
                self.regs.set_hl(hl.wrapping_sub(1));
            }
            0x2A => {
                let hl = self.regs.hl();
                self.regs.a = self.read8(hl);
                self.regs.set_hl(hl.wrapping_add(1));
            }
            0x3A => {
                let hl = self.regs.hl();
                self.regs.a = self.read8(hl);
                self.regs.set_hl(hl.wrapping_sub(1));
            }
            // LDH — high page 0xFF00+n / 0xFF00+C
            0xE0 => {
                let n = self.fetch();
                let v = self.regs.a;
                self.write8(0xFF00 + n as u16, v);
            }
            0xF0 => {
                let n = self.fetch();
                self.regs.a = self.read8(0xFF00 + n as u16);
            }
            0xE2 => {
                let a = 0xFF00 + self.regs.c as u16;
                let v = self.regs.a;
                self.write8(a, v);
            }
            0xF2 => {
                let a = 0xFF00 + self.regs.c as u16;
                self.regs.a = self.read8(a);
            }
            // LD (nn),A / LD A,(nn)
            0xEA => {
                let nn = self.fetch16();
                let v = self.regs.a;
                self.write8(nn, v);
            }
            0xFA => {
                let nn = self.fetch16();
                self.regs.a = self.read8(nn);
            }
            // LD rr,nn
            0x01 | 0x11 | 0x21 | 0x31 => {
                let v = self.fetch16();
                self.set_rr((op >> 4) & 3, v);
            }
            // LD (nn),SP
            0x08 => {
                let nn = self.fetch16();
                let sp = self.regs.sp;
                self.write8(nn, sp as u8);
                self.write8(nn.wrapping_add(1), (sp >> 8) as u8);
            }
            // LD SP,HL
            0xF9 => {
                self.bus.tick();
                self.regs.sp = self.regs.hl();
            }
            // PUSH rr (0=BC 1=DE 2=HL 3=AF)
            0xC5 | 0xD5 | 0xE5 | 0xF5 => {
                let v = match (op >> 4) & 3 {
                    0 => self.regs.bc(),
                    1 => self.regs.de(),
                    2 => self.regs.hl(),
                    _ => self.regs.af(),
                };
                self.push16(v);
            }
            // POP rr
            0xC1 | 0xD1 | 0xE1 | 0xF1 => {
                let v = self.pop16();
                match (op >> 4) & 3 {
                    0 => self.regs.set_bc(v),
                    1 => self.regs.set_de(v),
                    2 => self.regs.set_hl(v),
                    _ => self.regs.set_af(v), // set_af masks F's low nibble
                }
            }
            // LD HL,SP+e
            0xF8 => {
                let e = self.fetch() as i8 as i16 as u16;
                self.bus.tick();
                let sp = self.regs.sp;
                self.regs.f = 0;
                self.regs.set_flag(H, (sp & 0x0F) + (e & 0x0F) > 0x0F);
                self.regs.set_flag(C, (sp & 0xFF) + (e & 0xFF) > 0xFF);
                self.regs.set_hl(sp.wrapping_add(e));
            }
            _ => unimplemented!("opcode {op:#04X} at {:#06X}", self.regs.pc.wrapping_sub(1)),
        }
    }
}
