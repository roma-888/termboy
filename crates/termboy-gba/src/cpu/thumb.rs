//! Thumb (16-bit) instruction set: 19 formats, all thin encodings over the
//! ARM helpers — semantics live in one place (cpu/mod.rs and the shifter).

use super::Cpu;
use super::psr::{C, Mode, T};

impl Cpu {
    pub(crate) fn exec_thumb(&mut self, op: u16) {
        match op >> 12 {
            0x0 | 0x1 => {
                if op >> 11 == 0b00011 {
                    self.t_add_sub(op)
                } else {
                    self.t_shift_imm(op)
                }
            }
            0x2 | 0x3 => self.t_alu_imm(op),
            0x4 => match (op >> 10) & 3 {
                0 => self.t_alu_reg(op),
                1 => self.t_hi_reg_bx(op),
                _ => self.t_pc_load(op),
            },
            0x5 => {
                if op & (1 << 9) == 0 {
                    self.t_mem_reg(op)
                } else {
                    self.t_mem_signed(op)
                }
            }
            0x6 | 0x7 => self.t_mem_imm(op),
            0x8 => self.t_mem_half(op),
            0x9 => self.t_mem_sp(op),
            0xA => self.t_load_addr(op),
            0xB => {
                // 0xB0 = SP adjust; 0xB4/B5/BC/BD = push/pop. Other 0xBx
                // encodings don't exist on ARMv4T.
                if (op >> 8) & 0xF == 0 {
                    self.t_adjust_sp(op)
                } else {
                    self.t_push_pop(op)
                }
            }
            0xC => self.t_multiple(op),
            0xD => {
                if (op >> 8) & 0xF == 0xF {
                    self.t_swi(op)
                } else {
                    self.t_cond_branch(op)
                }
            }
            0xE => self.t_branch(op),
            _ => self.t_long_branch(op),
        }
    }

    // ---- Format 1: move shifted register ----
    fn t_shift_imm(&mut self, op: u16) {
        let ty = ((op >> 11) & 3) as u32;
        let amount = ((op >> 6) & 0x1F) as u32;
        let value = self.regs.get(((op >> 3) & 7) as usize);
        let (result, carry) = self.shift(ty, value, amount, true);
        self.set_nzc(result, carry);
        self.regs.set((op & 7) as usize, result);
    }

    // ---- Format 2: add/subtract (register or 3-bit immediate) ----
    fn t_add_sub(&mut self, op: u16) {
        let rs = self.regs.get(((op >> 3) & 7) as usize);
        let field = ((op >> 6) & 7) as u32;
        let operand =
            if op & (1 << 10) != 0 { field } else { self.regs.get(field as usize) };
        let result = if op & (1 << 9) != 0 {
            self.sub_flags(rs, operand, 1, true)
        } else {
            self.add_flags(rs, operand, 0, true)
        };
        self.regs.set((op & 7) as usize, result);
    }

    // ---- Format 3: MOV/CMP/ADD/SUB with 8-bit immediate ----
    fn t_alu_imm(&mut self, op: u16) {
        let rd = ((op >> 8) & 7) as usize;
        let imm = (op & 0xFF) as u32;
        let value = self.regs.get(rd);
        match (op >> 11) & 3 {
            0 => {
                self.set_nz(imm);
                self.regs.set(rd, imm);
            }
            1 => {
                self.sub_flags(value, imm, 1, true);
            }
            2 => {
                let r = self.add_flags(value, imm, 0, true);
                self.regs.set(rd, r);
            }
            _ => {
                let r = self.sub_flags(value, imm, 1, true);
                self.regs.set(rd, r);
            }
        }
    }

    // ---- Format 4: ALU operations ----
    fn t_alu_reg(&mut self, op: u16) {
        let rd = (op & 7) as usize;
        let rs = ((op >> 3) & 7) as usize;
        let a = self.regs.get(rd);
        let b = self.regs.get(rs);
        let c = self.regs.cpsr.flag(C) as u32;
        match (op >> 6) & 0xF {
            0x0 => { let r = a & b; self.set_nz(r); self.regs.set(rd, r); } // AND
            0x1 => { let r = a ^ b; self.set_nz(r); self.regs.set(rd, r); } // EOR
            0x2 | 0x3 | 0x4 | 0x7 => {
                // LSL/LSR/ASR/ROR by register: bottom byte, internal cycle
                let ty = match (op >> 6) & 0xF {
                    0x2 => 0,
                    0x3 => 1,
                    0x4 => 2,
                    _ => 3,
                };
                self.bus.idle();
                let (r, carry) = self.shift(ty, a, b & 0xFF, false);
                self.set_nzc(r, carry);
                self.regs.set(rd, r);
            }
            0x5 => { let r = self.add_flags(a, b, c, true); self.regs.set(rd, r); } // ADC
            0x6 => { let r = self.sub_flags(a, b, c, true); self.regs.set(rd, r); } // SBC
            0x8 => { self.set_nz(a & b); }                                          // TST
            0x9 => { let r = self.sub_flags(0, b, 1, true); self.regs.set(rd, r); } // NEG
            0xA => { self.sub_flags(a, b, 1, true); }                               // CMP
            0xB => { self.add_flags(a, b, 0, true); }                               // CMN
            0xC => { let r = a | b; self.set_nz(r); self.regs.set(rd, r); }         // ORR
            0xD => {
                self.bus.idle();
                let r = a.wrapping_mul(b);
                self.set_nz(r); // C unpredictable on ARM7 muls: unchanged
                self.regs.set(rd, r);
            }
            0xE => { let r = a & !b; self.set_nz(r); self.regs.set(rd, r); } // BIC
            _ => { let r = !b; self.set_nz(r); self.regs.set(rd, r); }       // MVN
        }
    }

    // ---- Format 5: hi-register ops / BX (no flags except CMP) ----
    fn t_hi_reg_bx(&mut self, op: u16) {
        let rd = ((op & 7) | ((op >> 4) & 8)) as usize;
        let rs = (((op >> 3) & 7) | ((op >> 3) & 8)) as usize;
        let vs = self.regs.get(rs);
        match (op >> 8) & 3 {
            0 => {
                let r = self.regs.get(rd).wrapping_add(vs);
                self.regs.set(rd, r);
                if rd == 15 {
                    self.flush();
                }
            }
            1 => {
                let vd = self.regs.get(rd);
                self.sub_flags(vd, vs, 1, true);
            }
            2 => {
                self.regs.set(rd, vs);
                if rd == 15 {
                    self.flush();
                }
            }
            _ => {
                // BX
                self.regs.cpsr.set_flag(T, vs & 1 != 0);
                self.regs.set(15, vs);
                self.flush();
            }
        }
    }

    // ---- Format 6: PC-relative load ----
    fn t_pc_load(&mut self, op: u16) {
        let base = self.regs.get(15) & !2; // bit 1 of PC reads as 0 here
        let addr = base.wrapping_add(((op & 0xFF) as u32) * 4);
        let v = self.bus.read32(addr);
        self.bus.idle();
        self.regs.set(((op >> 8) & 7) as usize, v);
    }

    // Formats 7-19 land in Tasks 13-14:
    fn t_mem_reg(&mut self, op: u16) {
        self.t_todo(op)
    }
    fn t_mem_signed(&mut self, op: u16) {
        self.t_todo(op)
    }
    fn t_mem_imm(&mut self, op: u16) {
        self.t_todo(op)
    }
    fn t_mem_half(&mut self, op: u16) {
        self.t_todo(op)
    }
    fn t_mem_sp(&mut self, op: u16) {
        self.t_todo(op)
    }
    fn t_load_addr(&mut self, op: u16) {
        self.t_todo(op)
    }
    fn t_adjust_sp(&mut self, op: u16) {
        self.t_todo(op)
    }
    fn t_push_pop(&mut self, op: u16) {
        self.t_todo(op)
    }
    fn t_multiple(&mut self, op: u16) {
        self.t_todo(op)
    }
    fn t_cond_branch(&mut self, op: u16) {
        self.t_todo(op)
    }
    fn t_swi(&mut self, op: u16) {
        self.t_todo(op)
    }
    fn t_branch(&mut self, op: u16) {
        self.t_todo(op)
    }
    fn t_long_branch(&mut self, op: u16) {
        self.t_todo(op)
    }

    fn t_todo(&mut self, op: u16) {
        let _ = Mode::User; // used by t_swi in Task 14
        unimplemented!("thumb opcode {op:#06X} at {:#010X}", self.exec_addr().wrapping_sub(2))
    }
}
