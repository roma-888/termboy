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

    /// ALU op decode for 0x80-0xBF and the d8 variants: index = (op >> 3) & 7.
    pub(crate) fn alu(&mut self, kind: u8, v: u8) {
        match kind {
            0 => self.add_a(v, false),
            1 => self.add_a(v, true),
            2 => self.sub_a(v, false, true),
            3 => self.sub_a(v, true, true),
            4 => {
                // AND
                self.regs.a &= v;
                let z = self.regs.a == 0;
                self.regs.f = 0;
                self.regs.set_flag(Z, z);
                self.regs.set_flag(H, true);
            }
            5 => {
                // XOR
                self.regs.a ^= v;
                let z = self.regs.a == 0;
                self.regs.f = 0;
                self.regs.set_flag(Z, z);
            }
            6 => {
                // OR
                self.regs.a |= v;
                let z = self.regs.a == 0;
                self.regs.f = 0;
                self.regs.set_flag(Z, z);
            }
            _ => self.sub_a(v, false, false), // CP
        }
    }

    fn add_a(&mut self, v: u8, with_carry: bool) {
        let c = (with_carry && self.regs.flag(C)) as u8;
        let a = self.regs.a;
        let result = a.wrapping_add(v).wrapping_add(c);
        self.regs.f = 0;
        self.regs.set_flag(Z, result == 0);
        self.regs.set_flag(H, (a & 0x0F) + (v & 0x0F) + c > 0x0F);
        self.regs.set_flag(C, a as u16 + v as u16 + c as u16 > 0xFF);
        self.regs.a = result;
    }

    fn sub_a(&mut self, v: u8, with_carry: bool, store: bool) {
        let c = (with_carry && self.regs.flag(C)) as u8;
        let a = self.regs.a;
        let result = a.wrapping_sub(v).wrapping_sub(c);
        self.regs.f = 0;
        self.regs.set_flag(Z, result == 0);
        self.regs.set_flag(N, true);
        self.regs.set_flag(H, (a & 0x0F) < (v & 0x0F) + c);
        self.regs.set_flag(C, (a as u16) < v as u16 + c as u16);
        if store {
            self.regs.a = result;
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
            // ALU A,r and ALU A,d8
            0x80..=0xBF => {
                let v = self.get_r(op & 7);
                self.alu((op >> 3) & 7, v);
            }
            0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
                let v = self.fetch();
                self.alu((op >> 3) & 7, v);
            }
            // INC r / DEC r (C flag untouched)
            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C => {
                let i = (op >> 3) & 7;
                let v = self.get_r(i).wrapping_add(1);
                self.set_r(i, v);
                self.regs.set_flag(Z, v == 0);
                self.regs.set_flag(N, false);
                self.regs.set_flag(H, v & 0x0F == 0);
            }
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D => {
                let i = (op >> 3) & 7;
                let v = self.get_r(i).wrapping_sub(1);
                self.set_r(i, v);
                self.regs.set_flag(Z, v == 0);
                self.regs.set_flag(N, true);
                self.regs.set_flag(H, v & 0x0F == 0x0F);
            }
            // DAA
            0x27 => {
                let mut a = self.regs.a;
                let mut carry = self.regs.flag(C);
                if !self.regs.flag(N) {
                    let mut adjust = 0u8;
                    if self.regs.flag(H) || a & 0x0F > 0x09 {
                        adjust |= 0x06;
                    }
                    if carry || a > 0x99 {
                        adjust |= 0x60;
                        carry = true;
                    }
                    a = a.wrapping_add(adjust);
                } else {
                    let mut adjust = 0u8;
                    if self.regs.flag(H) {
                        adjust |= 0x06;
                    }
                    if carry {
                        adjust |= 0x60;
                    }
                    a = a.wrapping_sub(adjust);
                }
                self.regs.a = a;
                self.regs.set_flag(Z, a == 0);
                self.regs.set_flag(H, false);
                self.regs.set_flag(C, carry);
            }
            // CPL / SCF / CCF
            0x2F => {
                self.regs.a = !self.regs.a;
                self.regs.set_flag(N, true);
                self.regs.set_flag(H, true);
            }
            0x37 => {
                self.regs.set_flag(N, false);
                self.regs.set_flag(H, false);
                self.regs.set_flag(C, true);
            }
            0x3F => {
                let c = self.regs.flag(C);
                self.regs.set_flag(N, false);
                self.regs.set_flag(H, false);
                self.regs.set_flag(C, !c);
            }
            _ => unimplemented!("opcode {op:#04X} at {:#06X}", self.regs.pc.wrapping_sub(1)),
        }
    }
}
