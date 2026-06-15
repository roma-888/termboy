//! GBA bus access timing: per-region cost in cycles, WAITCNT-driven gamepak
//! waitstates, sequential/non-sequential classification. Numbers from GBATEK
//! "GBA Memory Timing" / "GBA System Control". The gamepak prefetch buffer
//! (WAITCNT bit 14) is a deferred non-goal — decoded but not modelled here.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Width {
    Bits8,
    Bits16,
    Bits32,
}

impl Width {
    pub fn bytes(self) -> u32 {
        match self {
            Width::Bits8 => 1,
            Width::Bits16 => 2,
            Width::Bits32 => 4,
        }
    }
}

/// Access time = 1 + waitstate. Non-sequential waitstate per 2-bit field.
const N_WAIT: [u32; 4] = [4, 3, 2, 8];
/// Sequential waitstate, selected by each WS region's 1-bit field.
const S_WAIT: [[u32; 2]; 3] = [[2, 1], [4, 1], [8, 1]];

/// Cycles for one gamepak-ROM access. `ws` = 0/1/2 (WS0/WS1/WS2). A 32-bit
/// access is two 16-bit accesses: the first N-or-S, the second sequential.
pub fn rom_cycles(waitcnt: u16, ws: usize, width: Width, seq: bool) -> u32 {
    let n = 1 + N_WAIT[((waitcnt >> (2 + ws * 3)) & 3) as usize];
    let s_bit = (waitcnt >> (4 + ws * 3)) & 1;
    let s = 1 + S_WAIT[ws][s_bit as usize];
    let first = if seq { s } else { n };
    if width == Width::Bits32 { first + s } else { first }
}

/// Cycles for a gamepak-SRAM access (8-bit bus; one byte regardless of width).
pub fn sram_cycles(waitcnt: u16) -> u32 {
    1 + N_WAIT[(waitcnt & 3) as usize]
}

/// Cycles for an access to `region` (= addr >> 24), given WAITCNT, width, and
/// whether it is sequential to the previous access.
pub fn access_cycles(region: u32, waitcnt: u16, width: Width, seq: bool) -> u32 {
    match region {
        0x00 | 0x03 | 0x04 | 0x07 => 1, // BIOS, IWRAM, I/O, OAM — 32-bit bus
        0x02 => if width == Width::Bits32 { 6 } else { 3 }, // EWRAM, 16-bit bus
        0x05 | 0x06 => if width == Width::Bits32 { 2 } else { 1 }, // palette, VRAM
        0x08..=0x0D => rom_cycles(waitcnt, (region as usize - 8) / 2, width, seq),
        0x0E | 0x0F => sram_cycles(waitcnt),
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_regions_are_one_cycle() {
        for r in [0x00, 0x03, 0x04, 0x07] {
            for w in [Width::Bits8, Width::Bits16, Width::Bits32] {
                assert_eq!(access_cycles(r, 0, w, false), 1, "region {r:#x} {w:?}");
            }
        }
    }

    #[test]
    fn sixteen_bit_buses_double_on_32bit() {
        assert_eq!(access_cycles(0x02, 0, Width::Bits16, false), 3); // EWRAM
        assert_eq!(access_cycles(0x02, 0, Width::Bits32, false), 6);
        assert_eq!(access_cycles(0x05, 0, Width::Bits16, false), 1); // palette
        assert_eq!(access_cycles(0x05, 0, Width::Bits32, false), 2);
        assert_eq!(access_cycles(0x06, 0, Width::Bits32, false), 2); // VRAM
    }

    #[test]
    fn rom_default_waitstates() {
        // WAITCNT=0: WS0 N=4 -> 5, S=2 -> 3; 32-bit N = 5 + 3 = 8.
        assert_eq!(access_cycles(0x08, 0, Width::Bits16, false), 5);
        assert_eq!(access_cycles(0x08, 0, Width::Bits16, true), 3);
        assert_eq!(access_cycles(0x08, 0, Width::Bits32, false), 8);
        assert_eq!(access_cycles(0x08, 0, Width::Bits32, true), 6);
    }

    #[test]
    fn rom_waitcnt_selects_region_and_values() {
        // WS0 N field (bits2-3)=2 -> value 2 -> 1+2=3 (16-bit N).
        assert_eq!(access_cycles(0x08, 0b10 << 2, Width::Bits16, false), 3);
        // WS1 region is addr 0x0A; N field bits5-6.
        assert_eq!(access_cycles(0x0A, 0, Width::Bits16, false), 5);
        // WS2 region is addr 0x0C; S field bit10 set -> 1 -> 1+1=2.
        assert_eq!(access_cycles(0x0C, 1 << 10, Width::Bits16, true), 2);
    }

    #[test]
    fn sram_default_and_configured() {
        assert_eq!(access_cycles(0x0E, 0, Width::Bits8, false), 5); // 1+4
        assert_eq!(access_cycles(0x0E, 0b10, Width::Bits8, false), 3); // field=2 -> 1+2
    }
}
