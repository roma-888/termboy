//! HLE BIOS: SWI functions implemented in Rust (design spec §4 — the
//! self-contained choice). G1 ships only Div; G4 fills in the table that
//! commercial games need (CpuSet, LZ77, sqrt, IntrWait, ...). The
//! `Cpu::hle_bios` flag is the seam for swapping in a real BIOS image.

use crate::cpu::Cpu;

impl Cpu {
    pub(crate) fn bios_call(&mut self, function: u32) {
        match function {
            0x02 | 0x03 => self.bus.halted = true, // Halt / Stop
            // Div: r0/r1 -> r0 = quotient, r1 = remainder, r3 = |quotient|
            0x06 => {
                let n = self.regs.get(0) as i32;
                let d = self.regs.get(1) as i32;
                assert!(d != 0, "BIOS Div by zero at {:#010X}", self.exec_addr());
                let q = n.wrapping_div(d);
                self.regs.set(0, q as u32);
                self.regs.set(1, n.wrapping_rem(d) as u32);
                self.regs.set(3, q.unsigned_abs());
            }
            f => unimplemented!(
                "HLE BIOS function {f:#04X} at {:#010X} (lands in G4)",
                self.exec_addr()
            ),
        }
    }
}
