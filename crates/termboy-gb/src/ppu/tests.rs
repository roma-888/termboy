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

/// Write a solid-color tile (all pixels = `color` 0..=3) at tile index `i`.
pub fn put_tile(ppu: &mut Ppu, i: usize, color: u8) {
    for row in 0..8 {
        ppu.vram[i * 16 + row * 2] = if color & 1 != 0 { 0xFF } else { 0x00 };
        ppu.vram[i * 16 + row * 2 + 1] = if color & 2 != 0 { 0xFF } else { 0x00 };
    }
}

/// Render line `ly` in isolation and return it.
pub fn render_one_line(ppu: &mut Ppu, ly: u8) -> [u8; 160] {
    ppu.ly = ly;
    ppu.render_line();
    let y = ly as usize;
    let mut out = [0u8; 160];
    out.copy_from_slice(&ppu.frame[y * 160..(y + 1) * 160]);
    out
}

#[test]
fn bg_renders_tiles_through_bgp() {
    let mut ppu = Ppu::new();
    ppu.lcdc = LCDC_LCD_ON | LCDC_BG_ON | LCDC_TILE_DATA; // unsigned tiles, map 0x9800
    ppu.bgp = 0b11_10_01_00; // identity: color n -> shade n
    put_tile(&mut ppu, 1, 3);
    ppu.vram[0x1800] = 1; // map (0,0) = tile 1; rest = tile 0 (color 0)
    let line = render_one_line(&mut ppu, 0);
    assert_eq!(&line[0..8], &[3; 8]);
    assert_eq!(&line[8..16], &[0; 8]);
}

#[test]
fn bg_respects_scroll_and_wraps() {
    let mut ppu = Ppu::new();
    ppu.lcdc = LCDC_LCD_ON | LCDC_BG_ON | LCDC_TILE_DATA;
    ppu.bgp = 0b11_10_01_00;
    put_tile(&mut ppu, 1, 2);
    ppu.vram[0x1800] = 1; // tile (0,0)
    ppu.scx = 4; // shifts tile 4px left
    let line = render_one_line(&mut ppu, 0);
    assert_eq!(&line[0..4], &[2; 4]);
    assert_eq!(line[4], 0);
    ppu.scx = 0;
    ppu.scy = 8; // now map row 1 (all tile 0) is used for ly=0
    let line = render_one_line(&mut ppu, 0);
    assert_eq!(line[0], 0);
}

#[test]
fn signed_tile_addressing() {
    let mut ppu = Ppu::new();
    ppu.lcdc = LCDC_LCD_ON | LCDC_BG_ON; // bit4 clear -> signed from 0x9000
    ppu.bgp = 0b11_10_01_00;
    // tile index 0x80 (-128) lives at 0x9000 - 128*16 = 0x8800 (VRAM offset 0x800)
    for row in 0..8 {
        ppu.vram[0x800 + row * 2] = 0xFF;
        ppu.vram[0x800 + row * 2 + 1] = 0x00;
    }
    ppu.vram[0x1800] = 0x80;
    let line = render_one_line(&mut ppu, 0);
    assert_eq!(&line[0..8], &[1; 8]);
}

#[test]
fn bg_disabled_renders_shade_zero() {
    let mut ppu = Ppu::new();
    ppu.lcdc = LCDC_LCD_ON | LCDC_TILE_DATA; // BG off
    ppu.bgp = 0b11_10_01_00;
    put_tile(&mut ppu, 0, 3);
    let line = render_one_line(&mut ppu, 0);
    assert_eq!(&line[..], &[0u8; 160][..]);
}
