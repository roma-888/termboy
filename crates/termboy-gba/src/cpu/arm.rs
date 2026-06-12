//! ARM (32-bit) instruction set. Decode is an ordered mask cascade — the
//! specific bit patterns (BX, multiplies, swaps) must be tested before the
//! broad classes (data processing, transfers) that they overlap.

use super::Cpu;
use super::psr::T;

impl Cpu {
    pub(crate) fn exec_arm(&mut self, op: u32) {
        if op & 0x0FFF_FFF0 == 0x012F_FF10 {
            self.arm_bx(op)
        } else if op & 0x0E00_0000 == 0x0A00_0000 {
            self.arm_branch(op)
        } else if op & 0x0FC0_00F0 == 0x0000_0090 {
            self.arm_multiply(op)
        } else if op & 0x0F80_00F0 == 0x0080_0090 {
            self.arm_multiply_long(op)
        } else if op & 0x0FB0_0FF0 == 0x0100_0090 {
            self.arm_swap(op)
        } else if op & 0x0E00_0090 == 0x0000_0090 {
            self.arm_halfword_transfer(op)
        } else if op & 0x0FBF_0FFF == 0x010F_0000 {
            self.arm_mrs(op)
        } else if op & 0x0FB0_FFF0 == 0x0120_F000 || op & 0x0FB0_F000 == 0x0320_F000 {
            self.arm_msr(op)
        } else if op & 0x0C00_0000 == 0x0000_0000 {
            self.arm_data_processing(op)
        } else if op & 0x0E00_0010 == 0x0600_0010 {
            self.arm_undefined(op)
        } else if op & 0x0C00_0000 == 0x0400_0000 {
            self.arm_single_transfer(op)
        } else if op & 0x0E00_0000 == 0x0800_0000 {
            self.arm_block_transfer(op)
        } else if op & 0x0F00_0000 == 0x0F00_0000 {
            self.arm_swi(op)
        } else {
            self.arm_undefined(op)
        }
    }

    fn arm_branch(&mut self, op: u32) {
        // 24-bit signed word offset; r15 already reads as instruction+8
        let offset = ((op << 8) as i32 >> 6) as u32;
        if op & (1 << 24) != 0 {
            let lr = self.regs.get(15).wrapping_sub(4);
            self.regs.set(14, lr);
        }
        let target = self.regs.get(15).wrapping_add(offset);
        self.regs.set(15, target);
        self.flush();
    }

    fn arm_bx(&mut self, op: u32) {
        let target = self.regs.get((op & 0xF) as usize);
        self.regs.cpsr.set_flag(T, target & 1 != 0);
        self.regs.set(15, target);
        self.flush(); // flush masks the alignment bit(s) per state
    }

    // The rest of the cascade lands in Tasks 6-11; stubs keep it honest:
    fn arm_multiply(&mut self, op: u32) {
        let rd = ((op >> 16) & 0xF) as usize;
        let rn = ((op >> 12) & 0xF) as usize;
        let rs = ((op >> 8) & 0xF) as usize;
        let rm = (op & 0xF) as usize;
        let mut result = self.regs.get(rm).wrapping_mul(self.regs.get(rs));
        if op & (1 << 21) != 0 {
            result = result.wrapping_add(self.regs.get(rn)); // MLA
            self.bus.idle();
        }
        self.bus.idle(); // coarse internal multiply time
        if op & (1 << 20) != 0 {
            self.set_nz(result); // C unpredictable on ARM7 muls: left unchanged
        }
        self.regs.set(rd, result);
    }

    fn arm_multiply_long(&mut self, op: u32) {
        use super::psr::{N, Z};
        let rdhi = ((op >> 16) & 0xF) as usize;
        let rdlo = ((op >> 12) & 0xF) as usize;
        let rs = self.regs.get(((op >> 8) & 0xF) as usize);
        let rm = self.regs.get((op & 0xF) as usize);
        let mut result = if op & (1 << 22) != 0 {
            (rm as i32 as i64).wrapping_mul(rs as i32 as i64) as u64 // signed
        } else {
            (rm as u64) * (rs as u64)
        };
        if op & (1 << 21) != 0 {
            let acc = ((self.regs.get(rdhi) as u64) << 32) | self.regs.get(rdlo) as u64;
            result = result.wrapping_add(acc);
            self.bus.idle();
        }
        self.bus.idle();
        self.bus.idle();
        if op & (1 << 20) != 0 {
            self.regs.cpsr.set_flag(N, result >> 63 != 0);
            self.regs.cpsr.set_flag(Z, result == 0);
        }
        self.regs.set(rdhi, (result >> 32) as u32);
        self.regs.set(rdlo, result as u32);
    }
    fn arm_swap(&mut self, op: u32) {
        let addr = self.regs.get(((op >> 16) & 0xF) as usize);
        let rm = self.regs.get((op & 0xF) as usize);
        let rd = ((op >> 12) & 0xF) as usize;
        let old = if op & (1 << 22) != 0 {
            let v = self.bus.read8(addr) as u32;
            self.bus.write8(addr, rm as u8);
            v
        } else {
            let v = self.load_rotated(addr);
            self.bus.write32(addr & !3, rm);
            v
        };
        self.bus.idle();
        self.regs.set(rd, old);
    }

    fn arm_halfword_transfer(&mut self, op: u32) {
        let p = op & (1 << 24) != 0;
        let u = op & (1 << 23) != 0;
        let w = op & (1 << 21) != 0;
        let l = op & (1 << 20) != 0;
        let rn = ((op >> 16) & 0xF) as usize;
        let rd = ((op >> 12) & 0xF) as usize;
        // bit 22: immediate (split nibbles) vs register offset
        let offset = if op & (1 << 22) != 0 {
            ((op >> 4) & 0xF0) | (op & 0xF)
        } else {
            self.regs.get((op & 0xF) as usize)
        };
        let base = self.regs.get(rn);
        let offset_addr = if u { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
        let addr = if p { offset_addr } else { base };
        if l {
            let value = match (op >> 5) & 3 {
                1 => (self.bus.read16(addr & !1) as u32).rotate_right((addr & 1) * 8), // LDRH
                2 => self.bus.read8(addr) as i8 as i32 as u32,                         // LDRSB
                _ => {
                    // LDRSH; misaligned degrades to LDRSB on ARM7
                    if addr & 1 != 0 {
                        self.bus.read8(addr) as i8 as i32 as u32
                    } else {
                        self.bus.read16(addr) as i16 as i32 as u32
                    }
                }
            };
            self.bus.idle();
            if !p || w {
                self.regs.set(rn, offset_addr);
            }
            self.regs.set(rd, value);
            if rd == 15 {
                self.flush();
            }
        } else {
            // STRH (SH=01 is the only store form that reaches here)
            let value = self.regs.get(rd).wrapping_add(if rd == 15 { 4 } else { 0 });
            self.bus.write16(addr & !1, value as u16);
            if !p || w {
                self.regs.set(rn, offset_addr);
            }
        }
    }
    fn arm_mrs(&mut self, op: u32) {
        let v = if op & (1 << 22) != 0 { self.regs.spsr().0 } else { self.regs.cpsr.0 };
        self.regs.set(((op >> 12) & 0xF) as usize, v);
    }

    fn arm_msr(&mut self, op: u32) {
        use super::psr::{Mode, Psr, T};
        let value = if op & (1 << 25) != 0 {
            let rot = (op >> 8) & 0xF;
            (op & 0xFF).rotate_right(rot * 2)
        } else {
            self.regs.get((op & 0xF) as usize)
        };
        let mut mask = 0u32;
        if op & (1 << 16) != 0 {
            mask |= 0x0000_00FF;
        }
        if op & (1 << 17) != 0 {
            mask |= 0x0000_FF00;
        }
        if op & (1 << 18) != 0 {
            mask |= 0x00FF_0000;
        }
        if op & (1 << 19) != 0 {
            mask |= 0xFF00_0000;
        }
        if op & (1 << 22) != 0 {
            // SPSR target
            let cur = self.regs.spsr().0;
            self.regs.set_spsr(Psr((cur & !mask) | (value & mask)));
        } else {
            if self.regs.cpsr.mode() == Mode::User {
                mask &= 0xF000_0000; // unprivileged: flags only
            }
            let cur = self.regs.cpsr.0;
            // The T bit cannot be changed by MSR on ARM7 — only BX switches state.
            let new = (((cur & !mask) | (value & mask)) & !T) | (cur & T);
            self.regs.write_cpsr(Psr(new));
        }
    }
    /// Decode operand 2: rotated immediate, or rm shifted by imm/register.
    /// Returns (value, shifter carry-out).
    fn arm_operand2(&mut self, op: u32) -> (u32, bool) {
        if op & (1 << 25) != 0 {
            let rot = (op >> 8) & 0xF;
            let imm = op & 0xFF;
            if rot == 0 {
                (imm, self.regs.cpsr.flag(super::psr::C))
            } else {
                let v = imm.rotate_right(rot * 2);
                (v, v >> 31 != 0)
            }
        } else {
            let rm = (op & 0xF) as usize;
            let ty = (op >> 5) & 3;
            if op & (1 << 4) != 0 {
                // register-specified shift: one internal cycle, r15 reads +12
                let amount = self.regs.get(((op >> 8) & 0xF) as usize) & 0xFF;
                let value = self.regs.get(rm).wrapping_add(if rm == 15 { 4 } else { 0 });
                self.bus.idle();
                self.shift(ty, value, amount, false)
            } else {
                let amount = (op >> 7) & 0x1F;
                let value = self.regs.get(rm);
                self.shift(ty, value, amount, true)
            }
        }
    }

    fn arm_data_processing(&mut self, op: u32) {
        use super::psr::C;
        let opcode = (op >> 21) & 0xF;
        let s = op & (1 << 20) != 0;
        let rd = ((op >> 12) & 0xF) as usize;
        let reg_shift = op & (1 << 25) == 0 && op & (1 << 4) != 0;
        let (op2, shifter_carry) = self.arm_operand2(op);
        let rn_i = ((op >> 16) & 0xF) as usize;
        let rn = self.regs.get(rn_i).wrapping_add(if rn_i == 15 && reg_shift { 4 } else { 0 });
        let c = self.regs.cpsr.flag(C) as u32;
        // S with rd=15 means "restore CPSR from SPSR", not "compute flags"
        let set_flags = s && rd != 15;
        let result = match opcode {
            0x0 => { let r = rn & op2; if set_flags { self.set_nzc(r, shifter_carry) } r } // AND
            0x1 => { let r = rn ^ op2; if set_flags { self.set_nzc(r, shifter_carry) } r } // EOR
            0x2 => self.sub_flags(rn, op2, 1, set_flags),                                  // SUB
            0x3 => self.sub_flags(op2, rn, 1, set_flags),                                  // RSB
            0x4 => self.add_flags(rn, op2, 0, set_flags),                                  // ADD
            0x5 => self.add_flags(rn, op2, c, set_flags),                                  // ADC
            0x6 => self.sub_flags(rn, op2, c, set_flags),                                  // SBC
            0x7 => self.sub_flags(op2, rn, c, set_flags),                                  // RSC
            0x8 => { let r = rn & op2; self.set_nzc(r, shifter_carry); r }                 // TST
            0x9 => { let r = rn ^ op2; self.set_nzc(r, shifter_carry); r }                 // TEQ
            0xA => self.sub_flags(rn, op2, 1, true),                                       // CMP
            0xB => self.add_flags(rn, op2, 0, true),                                       // CMN
            0xC => { let r = rn | op2; if set_flags { self.set_nzc(r, shifter_carry) } r } // ORR
            0xD => { if set_flags { self.set_nzc(op2, shifter_carry) } op2 }               // MOV
            0xE => { let r = rn & !op2; if set_flags { self.set_nzc(r, shifter_carry) } r } // BIC
            _ => { let r = !op2; if set_flags { self.set_nzc(r, shifter_carry) } r }       // MVN
        };
        if !(0x8..=0xB).contains(&opcode) {
            if rd == 15 {
                if s {
                    let spsr = self.regs.spsr();
                    self.regs.write_cpsr(spsr);
                }
                self.regs.set(15, result);
                self.flush();
            } else {
                self.regs.set(rd, result);
            }
        }
    }
    fn arm_single_transfer(&mut self, op: u32) {
        let p = op & (1 << 24) != 0;
        let u = op & (1 << 23) != 0;
        let b = op & (1 << 22) != 0;
        let w = op & (1 << 21) != 0;
        let l = op & (1 << 20) != 0;
        let rn = ((op >> 16) & 0xF) as usize;
        let rd = ((op >> 12) & 0xF) as usize;
        let offset = if op & (1 << 25) != 0 {
            // register offset shifted by immediate; never flag-setting
            let ty = (op >> 5) & 3;
            let amount = (op >> 7) & 0x1F;
            let value = self.regs.get((op & 0xF) as usize);
            self.shift(ty, value, amount, true).0
        } else {
            op & 0xFFF
        };
        let base = self.regs.get(rn);
        let offset_addr = if u { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
        let addr = if p { offset_addr } else { base };
        if l {
            let value =
                if b { self.bus.read8(addr) as u32 } else { self.load_rotated(addr) };
            self.bus.idle();
            // post-index always writes back; rd wins over writeback when equal
            if !p || w {
                self.regs.set(rn, offset_addr);
            }
            self.regs.set(rd, value);
            if rd == 15 {
                self.flush();
            }
        } else {
            // STR of r15 stores PC+12 on ARM7
            let value = self.regs.get(rd).wrapping_add(if rd == 15 { 4 } else { 0 });
            if b {
                self.bus.write8(addr, value as u8);
            } else {
                self.bus.write32(addr & !3, value);
            }
            if !p || w {
                self.regs.set(rn, offset_addr);
            }
        }
    }
    fn arm_block_transfer(&mut self, op: u32) {
        let p = op & (1 << 24) != 0;
        let u = op & (1 << 23) != 0;
        let s = op & (1 << 22) != 0;
        let w = op & (1 << 21) != 0;
        let l = op & (1 << 20) != 0;
        let rn = ((op >> 16) & 0xF) as usize;
        let mut rlist = op & 0xFFFF;
        let base = self.regs.get(rn);

        // Empty rlist (ARMv4): r15 alone transfers, base moves by 0x40.
        let empty = rlist == 0;
        if empty {
            rlist = 1 << 15;
        }
        let count = if empty { 16 } else { rlist.count_ones() };

        let new_base =
            if u { base.wrapping_add(count * 4) } else { base.wrapping_sub(count * 4) };
        // The transfer loop always ascends from the bottom of the block.
        let mut addr = if u { base } else { new_base };
        if p == u {
            addr = addr.wrapping_add(4);
        }

        // S bit: SPSR restore if (and only if) this is an LDM with r15;
        // otherwise it means "use the user-mode register bank".
        let user_bank = s && !(l && rlist & (1 << 15) != 0);

        if l {
            self.bus.idle();
            // Writeback before the loads: a base inside the list ends up
            // with the loaded value (ARMv4 behavior).
            if w {
                self.regs.set(rn, new_base);
            }
            for i in 0..16usize {
                if rlist & (1 << i) == 0 {
                    continue;
                }
                let v = self.bus.read32(addr);
                addr = addr.wrapping_add(4);
                if user_bank {
                    self.regs.set_user(i, v);
                } else {
                    self.regs.set(i, v);
                }
            }
            if rlist & (1 << 15) != 0 {
                if s {
                    let spsr = self.regs.spsr();
                    self.regs.write_cpsr(spsr);
                }
                self.flush();
            }
        } else {
            let mut first = true;
            for i in 0..16usize {
                if rlist & (1 << i) == 0 {
                    continue;
                }
                let mut v =
                    if user_bank { self.regs.get_user(i) } else { self.regs.get(i) };
                if i == 15 {
                    v = v.wrapping_add(4); // STM stores PC+12
                }
                if i == rn && !first && w {
                    v = new_base; // base already written back by the time it stores
                }
                self.bus.write32(addr, v);
                addr = addr.wrapping_add(4);
                first = false;
            }
            if w {
                self.regs.set(rn, new_base);
            }
        }
    }
    fn arm_swi(&mut self, op: u32) {
        self.todo(op)
    }
    fn arm_undefined(&mut self, op: u32) {
        self.todo(op)
    }

    fn todo(&mut self, op: u32) {
        unimplemented!("ARM opcode {op:#010X} at {:#010X}", self.exec_addr().wrapping_sub(4))
    }
}
