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

    /// Shared by CB rotates and the A-register variants (which clear Z).
    fn rotate(&mut self, kind: u8, v: u8) -> u8 {
        let carry_in = self.regs.flag(C) as u8;
        let (result, carry_out) = match kind {
            0 => (v.rotate_left(1), v & 0x80 != 0),           // RLC
            1 => (v.rotate_right(1), v & 0x01 != 0),          // RRC
            2 => ((v << 1) | carry_in, v & 0x80 != 0),        // RL
            3 => ((v >> 1) | (carry_in << 7), v & 0x01 != 0), // RR
            4 => (v << 1, v & 0x80 != 0),                     // SLA
            5 => ((v >> 1) | (v & 0x80), v & 0x01 != 0),      // SRA (sign extends)
            6 => (v.rotate_left(4), false),                   // SWAP
            _ => (v >> 1, v & 0x01 != 0),                     // SRL
        };
        self.regs.f = 0;
        self.regs.set_flag(Z, result == 0);
        self.regs.set_flag(C, carry_out);
        result
    }

    fn execute_cb(&mut self) {
        let op = self.fetch();
        let i = op & 7;
        let n = (op >> 3) & 7; // rotate kind, or bit number
        match op >> 6 {
            0 => {
                let v = self.get_r(i);
                let r = self.rotate(n, v);
                self.set_r(i, r);
            }
            1 => {
                // BIT n,r — read-only, C preserved
                let v = self.get_r(i);
                self.regs.set_flag(Z, v & (1 << n) == 0);
                self.regs.set_flag(N, false);
                self.regs.set_flag(H, true);
            }
            2 => {
                // RES n,r
                let v = self.get_r(i);
                self.set_r(i, v & !(1 << n));
            }
            _ => {
                // SET n,r
                let v = self.get_r(i);
                self.set_r(i, v | (1 << n));
            }
        }
    }

    fn condition(&self, op: u8) -> bool {
        match (op >> 3) & 3 {
            0 => !self.regs.flag(Z),
            1 => self.regs.flag(Z),
            2 => !self.regs.flag(C),
            _ => self.regs.flag(C),
        }
    }

    fn jr(&mut self, take: bool) {
        let e = self.fetch() as i8;
        if take {
            self.bus.tick();
            self.regs.pc = self.regs.pc.wrapping_add(e as u16);
        }
    }

    fn jp(&mut self, take: bool) {
        let nn = self.fetch16();
        if take {
            self.bus.tick();
            self.regs.pc = nn;
        }
    }

    fn call(&mut self, take: bool) {
        let nn = self.fetch16();
        if take {
            self.push16(self.regs.pc); // internal tick + 2 writes
            self.regs.pc = nn;
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
            // ADD HL,rr
            0x09 | 0x19 | 0x29 | 0x39 => {
                self.bus.tick();
                let hl = self.regs.hl();
                let v = self.get_rr((op >> 4) & 3);
                let result = hl.wrapping_add(v);
                self.regs.set_flag(N, false);
                self.regs.set_flag(H, (hl & 0x0FFF) + (v & 0x0FFF) > 0x0FFF);
                self.regs.set_flag(C, hl as u32 + v as u32 > 0xFFFF);
                self.regs.set_hl(result);
            }
            // INC rr / DEC rr (no flags)
            0x03 | 0x13 | 0x23 | 0x33 => {
                self.bus.tick();
                let i = (op >> 4) & 3;
                let v = self.get_rr(i).wrapping_add(1);
                self.set_rr(i, v);
            }
            0x0B | 0x1B | 0x2B | 0x3B => {
                self.bus.tick();
                let i = (op >> 4) & 3;
                let v = self.get_rr(i).wrapping_sub(1);
                self.set_rr(i, v);
            }
            // ADD SP,e
            0xE8 => {
                let e = self.fetch() as i8 as i16 as u16;
                self.bus.tick();
                self.bus.tick();
                let sp = self.regs.sp;
                self.regs.f = 0;
                self.regs.set_flag(H, (sp & 0x0F) + (e & 0x0F) > 0x0F);
                self.regs.set_flag(C, (sp & 0xFF) + (e & 0xFF) > 0xFF);
                self.regs.sp = sp.wrapping_add(e);
            }
            // rotates on A — same helpers as CB but Z is always cleared
            0x07 | 0x0F | 0x17 | 0x1F => {
                let a = self.regs.a;
                let r = self.rotate((op >> 3) & 3, a);
                self.regs.a = r;
                self.regs.set_flag(Z, false);
            }
            0xCB => self.execute_cb(),
            // JR / JR cc
            0x18 => self.jr(true),
            0x20 | 0x28 | 0x30 | 0x38 => {
                let take = self.condition(op);
                self.jr(take);
            }
            // JP / JP cc / JP (HL)
            0xC3 => self.jp(true),
            0xC2 | 0xCA | 0xD2 | 0xDA => {
                let take = self.condition(op);
                self.jp(take);
            }
            0xE9 => self.regs.pc = self.regs.hl(),
            // CALL / CALL cc
            0xCD => self.call(true),
            0xC4 | 0xCC | 0xD4 | 0xDC => {
                let take = self.condition(op);
                self.call(take);
            }
            // RET / RET cc / RETI
            0xC9 => {
                let pc = self.pop16();
                self.bus.tick();
                self.regs.pc = pc;
            }
            0xC0 | 0xC8 | 0xD0 | 0xD8 => {
                self.bus.tick(); // condition check costs an internal cycle
                if self.condition(op) {
                    let pc = self.pop16();
                    self.bus.tick();
                    self.regs.pc = pc;
                }
            }
            0xD9 => {
                // RETI — like RET but IME is restored immediately
                let pc = self.pop16();
                self.bus.tick();
                self.regs.pc = pc;
                self.ime = true;
            }
            // RST — vector = op & 0x38
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
                self.push16(self.regs.pc);
                self.regs.pc = (op & 0x38) as u16;
            }
            0xF3 => {
                // DI
                self.ime = false;
                self.ime_pending = false;
            }
            0xFB => self.ime_pending = true, // EI (delayed one instruction)
            0x76 => {
                // HALT
                if !self.ime && (self.bus.inte & self.bus.intf & 0x1F) != 0 {
                    self.halt_bug = true; // pending IRQ with IME off: the infamous halt bug
                } else {
                    self.halted = true;
                }
            }
            0x10 => {
                // STOP consumes its padding byte; no-op until GBC speed switch
                self.fetch();
            }
            // invalid opcodes — real hardware locks up; fail loudly during development
            0xD3 | 0xDB | 0xDD | 0xE3 | 0xE4 | 0xEB | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD => {
                panic!("invalid opcode {op:#04X} at {:#06X}", self.regs.pc.wrapping_sub(1));
            }
        }
    }
}
