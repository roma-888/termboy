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
        self.todo(op)
    }
    fn arm_multiply_long(&mut self, op: u32) {
        self.todo(op)
    }
    fn arm_swap(&mut self, op: u32) {
        self.todo(op)
    }
    fn arm_halfword_transfer(&mut self, op: u32) {
        self.todo(op)
    }
    fn arm_mrs(&mut self, op: u32) {
        self.todo(op)
    }
    fn arm_msr(&mut self, op: u32) {
        self.todo(op)
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
        self.todo(op)
    }
    fn arm_block_transfer(&mut self, op: u32) {
        self.todo(op)
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
