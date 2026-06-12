//! Program status registers (CPSR/SPSR). The mode bits select the active
//! register bank; the T bit selects the ARM/Thumb decoder.

pub const N: u32 = 1 << 31; // negative
pub const Z: u32 = 1 << 30; // zero
pub const C: u32 = 1 << 29; // carry / not-borrow
pub const V: u32 = 1 << 28; // overflow
pub const I: u32 = 1 << 7; // IRQ disable
pub const F: u32 = 1 << 6; // FIQ disable (unused on GBA, kept for fidelity)
pub const T: u32 = 1 << 5; // Thumb state

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    User = 0x10,
    Fiq = 0x11,
    Irq = 0x12,
    Supervisor = 0x13,
    Abort = 0x17,
    Undefined = 0x1B,
    System = 0x1F,
}

impl Mode {
    pub fn from_bits(bits: u32) -> Mode {
        match bits & 0x1F {
            0x10 => Mode::User,
            0x11 => Mode::Fiq,
            0x12 => Mode::Irq,
            0x13 => Mode::Supervisor,
            0x17 => Mode::Abort,
            0x1B => Mode::Undefined,
            0x1F => Mode::System,
            m => panic!("invalid CPU mode bits {m:#04X}"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Psr(pub u32);

impl Psr {
    pub fn mode(self) -> Mode {
        Mode::from_bits(self.0)
    }
    pub fn thumb(self) -> bool {
        self.0 & T != 0
    }
    pub fn flag(self, mask: u32) -> bool {
        self.0 & mask != 0
    }
    pub fn set_flag(&mut self, mask: u32, on: bool) {
        if on {
            self.0 |= mask
        } else {
            self.0 &= !mask
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_bits_roundtrip() {
        assert_eq!(Mode::from_bits(0x10), Mode::User);
        assert_eq!(Mode::from_bits(0xFF), Mode::System); // only low 5 bits count
        assert_eq!(Mode::Supervisor as u32, 0x13);
    }

    #[test]
    fn flags_set_and_clear() {
        let mut p = Psr(0);
        p.set_flag(N, true);
        p.set_flag(T, true);
        assert!(p.flag(N) && p.thumb() && !p.flag(Z));
        p.set_flag(N, false);
        assert!(!p.flag(N) && p.thumb());
    }

    #[test]
    #[should_panic]
    fn invalid_mode_panics() {
        Mode::from_bits(0x00);
    }
}
