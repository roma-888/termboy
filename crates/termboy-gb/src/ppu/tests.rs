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

#[test]
fn window_overlays_bg_from_wx_minus_7() {
    let mut ppu = Ppu::new();
    ppu.lcdc = LCDC_LCD_ON | LCDC_BG_ON | LCDC_WIN_ON | LCDC_TILE_DATA | LCDC_WIN_MAP;
    ppu.bgp = 0b11_10_01_00;
    put_tile(&mut ppu, 1, 3);
    // window map at 0x9C00 all tile 1; BG map stays tile 0
    for i in 0..32 * 32 {
        ppu.vram[0x1C00 + i] = 1;
    }
    ppu.wy = 0;
    ppu.wx = 17; // window starts at screen x=10
    ppu.wy_hit = true;
    let line = render_one_line(&mut ppu, 0);
    assert_eq!(&line[0..10], &[0; 10]);
    assert_eq!(&line[10..160], &[3; 150][..]);
}

#[test]
fn window_line_counter_freezes_while_hidden() {
    let mut ppu = Ppu::new();
    ppu.lcdc = LCDC_LCD_ON | LCDC_BG_ON | LCDC_WIN_ON | LCDC_TILE_DATA | LCDC_WIN_MAP;
    ppu.bgp = 0b11_10_01_00;
    // window map row 0 = tile 1 (shade 3); row 1 = tile 0
    put_tile(&mut ppu, 1, 3);
    for i in 0..32 {
        ppu.vram[0x1C00 + i] = 1;
    }
    ppu.wy = 0;
    ppu.wy_hit = true;
    ppu.wx = 7;
    let l0 = render_one_line(&mut ppu, 0); // window line 0 -> shade 3
    assert_eq!(l0[0], 3);
    ppu.wx = 200; // hide the window for a line
    render_one_line(&mut ppu, 1);
    ppu.wx = 7; // bring it back: must resume at window line 1 (map row 0 still)
    let l2 = render_one_line(&mut ppu, 2);
    assert_eq!(l2[0], 3);
    // after 7 more visible window lines we reach map row 1 (tile 0)
    for ly in 3..10 {
        render_one_line(&mut ppu, ly);
    }
    let l10 = render_one_line(&mut ppu, 10); // window line 9 -> map row 1 -> shade 0
    assert_eq!(l10[0], 0);
}

#[test]
fn window_requires_wy_hit() {
    let mut ppu = Ppu::new();
    ppu.lcdc = LCDC_LCD_ON | LCDC_BG_ON | LCDC_WIN_ON | LCDC_TILE_DATA | LCDC_WIN_MAP;
    ppu.bgp = 0b11_10_01_00;
    put_tile(&mut ppu, 1, 3);
    for i in 0..32 {
        ppu.vram[0x1C00 + i] = 1;
    }
    ppu.wx = 7;
    ppu.wy_hit = false; // WY never matched this frame
    let line = render_one_line(&mut ppu, 0);
    assert_eq!(line[0], 0);
}

pub fn put_sprite(ppu: &mut Ppu, i: usize, y: u8, x: u8, tile: u8, attr: u8) {
    ppu.oam[i * 4] = y;
    ppu.oam[i * 4 + 1] = x;
    ppu.oam[i * 4 + 2] = tile;
    ppu.oam[i * 4 + 3] = attr;
}

fn sprite_ppu() -> Ppu {
    let mut ppu = Ppu::new();
    ppu.lcdc = LCDC_LCD_ON | LCDC_BG_ON | LCDC_OBJ_ON | LCDC_TILE_DATA;
    ppu.bgp = 0b11_10_01_00;
    ppu.obp0 = 0b11_10_01_00;
    ppu.obp1 = 0b00_01_10_11; // inverted, to tell palettes apart
    put_tile(&mut ppu, 2, 1); // sprite tile: solid color 1
    put_tile(&mut ppu, 3, 2); // second sprite tile: solid color 2
    ppu
}

#[test]
fn sprite_draws_at_position_with_obp0() {
    let mut ppu = sprite_ppu();
    put_sprite(&mut ppu, 0, 16, 16, 2, 0); // at screen (8,0)
    let line = render_one_line(&mut ppu, 0);
    assert_eq!(line[7], 0);
    assert_eq!(&line[8..16], &[1; 8]);
    assert_eq!(line[16], 0);
}

#[test]
fn obp1_and_transparency() {
    let mut ppu = sprite_ppu();
    put_sprite(&mut ppu, 0, 16, 8, 2, 0x10); // OBP1: color 1 -> shade 2
    let line = render_one_line(&mut ppu, 0);
    assert_eq!(line[0], 2);
}

#[test]
fn lower_x_wins_overlap() {
    let mut ppu = sprite_ppu();
    put_sprite(&mut ppu, 0, 16, 20, 2, 0); // OAM 0 at x=12, color 1
    put_sprite(&mut ppu, 1, 16, 16, 3, 0); // OAM 1 at x=8, color 2 — lower x wins
    let line = render_one_line(&mut ppu, 0);
    assert_eq!(line[12], 2); // overlap pixels show sprite 1
    assert_eq!(line[16], 1); // past sprite 1's right edge, sprite 0 shows
}

#[test]
fn ten_sprite_limit_in_oam_order() {
    let mut ppu = sprite_ppu();
    for i in 0..11 {
        put_sprite(&mut ppu, i, 16, 8 + 8 * i as u8, 2, 0);
    }
    let line = render_one_line(&mut ppu, 0);
    assert_eq!(line[8 * 9], 1); // 10th sprite (OAM 9) drawn
    assert_eq!(line[8 * 10], 0); // 11th (OAM 10) dropped
}

#[test]
fn bg_over_obj_hides_behind_nonzero_bg() {
    let mut ppu = sprite_ppu();
    put_tile(&mut ppu, 1, 2);
    ppu.vram[0x1800] = 1; // BG tile (0,0) = color 2 (nonzero)
    put_sprite(&mut ppu, 0, 16, 12, 2, 0x80); // sx=4: half over BG tile, half past it
    let line = render_one_line(&mut ppu, 0);
    assert_eq!(line[4], 2); // BG color 2 wins over the behind-BG sprite
    assert_eq!(line[8], 1); // BG color 0 there -> sprite shows
}

#[test]
fn tall_sprites_and_yflip() {
    let mut ppu = sprite_ppu();
    ppu.lcdc |= LCDC_OBJ_TALL;
    // tile pair 4/5: top solid color 1, bottom solid color 2
    put_tile(&mut ppu, 4, 1);
    put_tile(&mut ppu, 5, 2);
    put_sprite(&mut ppu, 0, 16, 8, 4, 0); // rows 0-7 tile 4, 8-15 tile 5
    assert_eq!(render_one_line(&mut ppu, 0)[0], 1);
    assert_eq!(render_one_line(&mut ppu, 8)[0], 2);
    put_sprite(&mut ppu, 0, 16, 8, 4, 0x40); // y-flipped: bottom tile on top
    assert_eq!(render_one_line(&mut ppu, 0)[0], 2);
}
