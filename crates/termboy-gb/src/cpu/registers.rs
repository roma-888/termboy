//! SM83 register file. Flag bits live in F's high nibble; the low nibble
//! is hardwired to zero on real hardware, so every F write masks it.

pub const Z: u8 = 0x80; // zero
pub const N: u8 = 0x40; // subtract
pub const H: u8 = 0x20; // half-carry
pub const C: u8 = 0x10; // carry

pub struct Registers {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
}

impl Registers {
    /// DMG state after the boot ROM hands off (we skip the copyrighted boot
    /// ROM and start here — design spec §3). This is one of the two GBC seams.
    pub fn new_post_boot() -> Self {
        Self {
            a: 0x01, f: 0xB0, b: 0x00, c: 0x13, d: 0x00, e: 0xD8,
            h: 0x01, l: 0x4D, sp: 0xFFFE, pc: 0x0100,
        }
    }

    pub fn af(&self) -> u16 { u16::from_be_bytes([self.a, self.f]) }
    pub fn bc(&self) -> u16 { u16::from_be_bytes([self.b, self.c]) }
    pub fn de(&self) -> u16 { u16::from_be_bytes([self.d, self.e]) }
    pub fn hl(&self) -> u16 { u16::from_be_bytes([self.h, self.l]) }

    pub fn set_af(&mut self, v: u16) { self.a = (v >> 8) as u8; self.f = v as u8 & 0xF0; }
    pub fn set_bc(&mut self, v: u16) { self.b = (v >> 8) as u8; self.c = v as u8; }
    pub fn set_de(&mut self, v: u16) { self.d = (v >> 8) as u8; self.e = v as u8; }
    pub fn set_hl(&mut self, v: u16) { self.h = (v >> 8) as u8; self.l = v as u8; }

    pub fn flag(&self, mask: u8) -> bool { self.f & mask != 0 }
    pub fn set_flag(&mut self, mask: u8, on: bool) {
        if on { self.f |= mask } else { self.f &= !mask }
        self.f &= 0xF0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_compose_and_decompose() {
        let mut r = Registers::new_post_boot();
        r.set_bc(0x1234);
        assert_eq!(r.b, 0x12);
        assert_eq!(r.c, 0x34);
        assert_eq!(r.bc(), 0x1234);
    }

    #[test]
    fn f_low_nibble_always_zero() {
        let mut r = Registers::new_post_boot();
        r.set_af(0xABCD);
        assert_eq!(r.f, 0xC0); // 0xCD masked to 0xC0
    }

    #[test]
    fn post_boot_state_matches_dmg() {
        let r = Registers::new_post_boot();
        assert_eq!(r.af(), 0x01B0);
        assert_eq!(r.bc(), 0x0013);
        assert_eq!(r.de(), 0x00D8);
        assert_eq!(r.hl(), 0x014D);
        assert_eq!(r.sp, 0xFFFE);
        assert_eq!(r.pc, 0x0100);
    }
}
