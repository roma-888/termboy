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
    fn arm_data_processing(&mut self, op: u32) {
        self.todo(op)
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
