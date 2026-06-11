use super::*;
use crate::bus::{IF_STAT, IF_VBLANK};

/// Tick `n` T-cycles, ORing all interrupt requests.
pub fn tick_n(ppu: &mut Ppu, n: u32) -> u8 {
    let mut irq = 0;
    for _ in 0..n {
        irq |= ppu.tick();
    }
    irq
}

#[test]
fn ly_advances_every_456_dots() {
    let mut ppu = Ppu::new();
    tick_n(&mut ppu, 455);
    assert_eq!(ppu.ly, 0);
    tick_n(&mut ppu, 1);
    assert_eq!(ppu.ly, 1);
    tick_n(&mut ppu, 456 * 152);
    assert_eq!(ppu.ly, 153);
    tick_n(&mut ppu, 456);
    assert_eq!(ppu.ly, 0); // wraps after line 153
}

#[test]
fn modes_follow_the_hardware_sequence() {
    let mut ppu = Ppu::new();
    assert_eq!(ppu.read_stat() & 3, 2); // OAM scan at line start
    tick_n(&mut ppu, 80);
    assert_eq!(ppu.read_stat() & 3, 3); // drawing
    tick_n(&mut ppu, 172);
    assert_eq!(ppu.read_stat() & 3, 0); // hblank
    tick_n(&mut ppu, 456 * 144); // same dot, line 144
    assert_eq!(ppu.read_stat() & 3, 1); // vblank
}

#[test]
fn vblank_irq_and_frame_ready_at_line_144() {
    let mut ppu = Ppu::new();
    let irq = tick_n(&mut ppu, 456 * 144);
    assert_eq!(irq & IF_VBLANK, IF_VBLANK);
    assert!(ppu.frame_ready);
}

#[test]
fn lcd_off_holds_ly_at_zero() {
    let mut ppu = Ppu::new();
    tick_n(&mut ppu, 1000);
    ppu.write_lcdc(0x11); // bit 7 clear -> LCD off
    assert_eq!(ppu.ly, 0);
    tick_n(&mut ppu, 10_000);
    assert_eq!(ppu.ly, 0);
    assert_eq!(ppu.read_stat() & 3, 0);
}

#[test]
fn stat_lyc_irq_fires_on_rising_edge_only() {
    let mut ppu = Ppu::new();
    ppu.lyc = 2;
    ppu.write_stat(0x40); // LYC interrupt enable
    let irq = tick_n(&mut ppu, 456 * 2); // reach line 2
    assert_eq!(irq & IF_STAT, IF_STAT);
    let irq = tick_n(&mut ppu, 100); // still on line 2: no re-fire
    assert_eq!(irq & IF_STAT, 0);
}

#[test]
fn stat_reads_with_bit7_set_and_lyc_flag() {
    let mut ppu = Ppu::new();
    ppu.lyc = 0;
    assert_eq!(ppu.read_stat() & 0x84, 0x84); // bit7 + LYC==LY
    ppu.lyc = 5;
    assert_eq!(ppu.read_stat() & 0x04, 0);
}
