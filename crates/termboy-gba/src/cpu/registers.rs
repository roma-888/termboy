//! ARM7TDMI banked register file. `r` is the *visible* window; mode switches
//! save/restore the banked slices. r15 lives in `r[15]` and is the fetch
//! address (see cpu/mod.rs for the pipeline convention).

use super::psr::{Mode, Psr};
use termboy_core::state::{Reader, StateError, Writer};

pub struct Registers {
    r: [u32; 16],
    bank_usr: [u32; 7], // r8-r14 for User/System (and r8-r12 for all non-FIQ)
    bank_fiq: [u32; 7], // r8-r14
    bank_irq: [u32; 2], // r13-r14
    bank_svc: [u32; 2],
    bank_abt: [u32; 2],
    bank_und: [u32; 2],
    pub cpsr: Psr,
    spsr: [Psr; 5], // fiq, irq, svc, abt, und
}

impl Registers {
    /// State after the BIOS hands off to the cartridge (we skip the BIOS —
    /// design spec §4): System mode, ARM state, stacks pre-set per GBATEK.
    pub fn new_post_bios() -> Self {
        let mut regs = Self {
            r: [0; 16],
            bank_usr: [0; 7],
            bank_fiq: [0; 7],
            bank_irq: [0x0300_7FA0, 0],
            bank_svc: [0x0300_7FE0, 0],
            bank_abt: [0; 2],
            bank_und: [0; 2],
            cpsr: Psr(Mode::System as u32),
            spsr: [Psr(0); 5],
        };
        regs.r[13] = 0x0300_7F00;
        regs
    }

    pub fn get(&self, i: usize) -> u32 {
        self.r[i]
    }

    pub fn set(&mut self, i: usize, v: u32) {
        self.r[i] = v;
    }

    pub fn switch_mode(&mut self, new: Mode) {
        let old = self.cpsr.mode();
        if old != new {
            self.save_bank(old);
            self.load_bank(new);
        }
        self.cpsr.0 = (self.cpsr.0 & !0x1F) | new as u32;
    }

    /// Replace the whole CPSR (MSR, SPSR restore), switching banks to match.
    pub fn write_cpsr(&mut self, new: Psr) {
        self.switch_mode(new.mode());
        self.cpsr = new;
    }

    fn save_bank(&mut self, mode: Mode) {
        if mode == Mode::Fiq {
            self.bank_fiq.copy_from_slice(&self.r[8..15]);
        } else {
            self.bank_usr[..5].copy_from_slice(&self.r[8..13]);
            let (lo, hi) = (self.r[13], self.r[14]);
            match mode {
                Mode::User | Mode::System => {
                    self.bank_usr[5] = lo;
                    self.bank_usr[6] = hi;
                }
                Mode::Irq => self.bank_irq = [lo, hi],
                Mode::Supervisor => self.bank_svc = [lo, hi],
                Mode::Abort => self.bank_abt = [lo, hi],
                Mode::Undefined => self.bank_und = [lo, hi],
                Mode::Fiq => unreachable!(),
            }
        }
    }

    fn load_bank(&mut self, mode: Mode) {
        if mode == Mode::Fiq {
            self.r[8..15].copy_from_slice(&self.bank_fiq);
        } else {
            self.r[8..13].copy_from_slice(&self.bank_usr[..5]);
            let [lo, hi] = match mode {
                Mode::User | Mode::System => [self.bank_usr[5], self.bank_usr[6]],
                Mode::Irq => self.bank_irq,
                Mode::Supervisor => self.bank_svc,
                Mode::Abort => self.bank_abt,
                Mode::Undefined => self.bank_und,
                Mode::Fiq => unreachable!(),
            };
            self.r[13] = lo;
            self.r[14] = hi;
        }
    }

    fn spsr_index(mode: Mode) -> Option<usize> {
        match mode {
            Mode::Fiq => Some(0),
            Mode::Irq => Some(1),
            Mode::Supervisor => Some(2),
            Mode::Abort => Some(3),
            Mode::Undefined => Some(4),
            Mode::User | Mode::System => None,
        }
    }

    /// Current mode's SPSR. User/System have none on hardware (reading is
    /// unpredictable there); returning CPSR is the safe emulator convention.
    pub fn spsr(&self) -> Psr {
        match Self::spsr_index(self.cpsr.mode()) {
            Some(i) => self.spsr[i],
            None => self.cpsr,
        }
    }

    pub fn set_spsr(&mut self, v: Psr) {
        if let Some(i) = Self::spsr_index(self.cpsr.mode()) {
            self.spsr[i] = v;
        }
    }

    /// User-bank access from any mode — LDM/STM with the S bit use these.
    pub fn get_user(&self, i: usize) -> u32 {
        match self.cpsr.mode() {
            Mode::User | Mode::System => self.r[i],
            Mode::Fiq if (8..15).contains(&i) => self.bank_usr[i - 8],
            _ if i == 13 || i == 14 => self.bank_usr[i - 8],
            _ => self.r[i],
        }
    }

    pub fn set_user(&mut self, i: usize, v: u32) {
        match self.cpsr.mode() {
            Mode::User | Mode::System => self.r[i] = v,
            Mode::Fiq if (8..15).contains(&i) => self.bank_usr[i - 8] = v,
            _ if i == 13 || i == 14 => self.bank_usr[i - 8] = v,
            _ => self.r[i] = v,
        }
    }

    pub(crate) fn serialize(&self, w: &mut Writer) {
        for &v in &self.r {
            w.put_u32(v);
        }
        for &v in &self.bank_usr {
            w.put_u32(v);
        }
        for &v in &self.bank_fiq {
            w.put_u32(v);
        }
        for &v in &self.bank_irq {
            w.put_u32(v);
        }
        for &v in &self.bank_svc {
            w.put_u32(v);
        }
        for &v in &self.bank_abt {
            w.put_u32(v);
        }
        for &v in &self.bank_und {
            w.put_u32(v);
        }
        w.put_u32(self.cpsr.0);
        for p in &self.spsr {
            w.put_u32(p.0);
        }
    }

    pub(crate) fn deserialize(&mut self, r: &mut Reader) -> Result<(), StateError> {
        for v in &mut self.r {
            *v = r.get_u32()?;
        }
        for v in &mut self.bank_usr {
            *v = r.get_u32()?;
        }
        for v in &mut self.bank_fiq {
            *v = r.get_u32()?;
        }
        for v in &mut self.bank_irq {
            *v = r.get_u32()?;
        }
        for v in &mut self.bank_svc {
            *v = r.get_u32()?;
        }
        for v in &mut self.bank_abt {
            *v = r.get_u32()?;
        }
        for v in &mut self.bank_und {
            *v = r.get_u32()?;
        }
        self.cpsr = Psr(r.get_u32()?);
        for p in &mut self.spsr {
            *p = Psr(r.get_u32()?);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::psr::Mode;
    use super::*;

    #[test]
    fn post_bios_state() {
        let r = Registers::new_post_bios();
        assert_eq!(r.cpsr.mode(), Mode::System);
        assert!(!r.cpsr.thumb());
        assert_eq!(r.get(13), 0x0300_7F00);
    }

    #[test]
    fn mode_switch_banks_r13_r14() {
        let mut r = Registers::new_post_bios();
        r.set(13, 0x1111);
        r.set(14, 0x2222);
        r.switch_mode(Mode::Irq);
        assert_eq!(r.cpsr.mode(), Mode::Irq);
        assert_eq!(r.get(13), 0x0300_7FA0); // IRQ bank, pre-set by the BIOS
        r.set(14, 0x3333);
        r.switch_mode(Mode::System);
        assert_eq!(r.get(13), 0x1111);
        assert_eq!(r.get(14), 0x2222);
        r.switch_mode(Mode::Irq);
        assert_eq!(r.get(14), 0x3333); // IRQ bank survived the round trip
    }

    #[test]
    fn fiq_banks_r8_to_r14_others_do_not() {
        let mut r = Registers::new_post_bios();
        r.set(8, 0xAAAA);
        r.set(12, 0xBBBB);
        r.switch_mode(Mode::Fiq);
        r.set(8, 0xCCCC);
        assert_ne!(r.get(8), 0xAAAA);
        r.switch_mode(Mode::Supervisor);
        assert_eq!(r.get(8), 0xAAAA); // r8-r12 shared by all non-FIQ modes
        assert_eq!(r.get(12), 0xBBBB);
    }

    #[test]
    fn spsr_is_per_mode() {
        let mut r = Registers::new_post_bios();
        r.switch_mode(Mode::Irq);
        r.set_spsr(Psr(0x1234_0011));
        r.switch_mode(Mode::Supervisor);
        r.set_spsr(Psr(0x5678_0012));
        assert_eq!(r.spsr().0, 0x5678_0012);
        r.switch_mode(Mode::Irq);
        assert_eq!(r.spsr().0, 0x1234_0011);
    }

    #[test]
    fn user_bank_access_from_exception_mode() {
        let mut r = Registers::new_post_bios();
        r.set(13, 0xDEAD);
        r.switch_mode(Mode::Irq);
        assert_eq!(r.get_user(13), 0xDEAD);
        r.set_user(14, 0xBEEF);
        r.switch_mode(Mode::System);
        assert_eq!(r.get(14), 0xBEEF);
    }

    #[test]
    fn write_cpsr_switches_banks() {
        let mut r = Registers::new_post_bios();
        r.set(13, 0x1111);
        r.write_cpsr(Psr(0x12)); // IRQ mode bits
        assert_eq!(r.cpsr.mode(), Mode::Irq);
        assert_ne!(r.get(13), 0x1111);
    }
}
