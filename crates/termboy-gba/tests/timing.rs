//! GBATEK-table timing validation at the bus level (toolchain-free).
use termboy_gba::bus::Bus;

fn bus() -> Bus {
    Bus::new(vec![0u8; 0x200_0000]) // 32MB of zeroed ROM so 0x08.. is mapped
}

/// Cycles charged by running `f`.
fn cost(b: &mut Bus, f: impl FnOnce(&mut Bus)) -> u64 {
    let before = b.cycles;
    f(b);
    b.cycles - before
}

#[test]
fn iwram_and_ewram_widths() {
    let mut b = bus();
    assert_eq!(cost(&mut b, |b| { b.read32(0x0300_0000); }), 1); // IWRAM 32-bit = 1
    assert_eq!(cost(&mut b, |b| { b.read32(0x0200_0000); }), 6); // EWRAM 32-bit = 6
    assert_eq!(cost(&mut b, |b| { b.read16(0x0200_0010); }), 3); // EWRAM 16-bit = 3
}

#[test]
fn rom_nonsequential_then_sequential() {
    let mut b = bus();
    b.mark_nonseq();
    assert_eq!(cost(&mut b, |b| { b.read16(0x0800_0000); }), 5); // N
    assert_eq!(cost(&mut b, |b| { b.read16(0x0800_0002); }), 3); // S (contiguous)
    assert_eq!(cost(&mut b, |b| { b.read16(0x0800_0010); }), 5); // N (gap)
}

#[test]
fn waitcnt_write_changes_rom_cost() {
    let mut b = bus();
    b.write16(0x0400_0204, 0b10 << 2); // WS0 N field = 2 -> 1+2 = 3
    b.mark_nonseq();
    assert_eq!(cost(&mut b, |b| { b.read16(0x0800_0000); }), 3);
}
