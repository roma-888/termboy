use super::*;
use crate::bus::{Bus, CYCLES_PER_LINE};

fn bus() -> Bus {
    Bus::new(vec![0u8; 0x200])
}

/// DISPCNT lives at 0x04000000; the PPU reads it per scanline.
fn set_dispcnt(b: &mut Bus, v: u16) {
    b.write16(0x0400_0000, v);
}

/// Advance the clock so scanline `line` has just completed, and render.
fn render_line(b: &mut Bus, line: u64) {
    b.cycles = (line + 1) * CYCLES_PER_LINE;
    b.catch_up();
}

/// One 8x8 4bpp tile at char_base, all pixels = `idx`, into tile slot `tile`.
fn fill_tile_4bpp(b: &mut Bus, char_base: u32, tile: u32, idx: u8) {
    let byte = idx | (idx << 4);
    for i in (0..32).step_by(2) {
        b.write16(0x0600_0000 + char_base + tile * 32 + i, u16::from_le_bytes([byte, byte]));
    }
}

/// Map entry at tile position (tx, ty) of screen block `sbb`.
fn set_entry(b: &mut Bus, screen_base: u32, sbb: u32, tx: u32, ty: u32, entry: u16) {
    b.write16(0x0600_0000 + screen_base + sbb * 0x800 + (ty * 32 + tx) * 2, entry);
}

#[test]
fn catch_up_renders_only_completed_lines() {
    let mut b = bus();
    b.cycles = crate::bus::HBLANK_AT - 1; // line 0 still drawing
    b.catch_up();
    assert!(!b.ppu.frame_ready);
    b.write16(0x0500_0000, 0x7C00); // backdrop = blue, set BEFORE hblank
    b.cycles = crate::bus::HBLANK_AT; // line 0 completes at its hblank
    b.catch_up();
    assert_eq!(b.ppu.frame[0], 0x7C00); // line 0 rendered with current state
    assert_eq!(b.ppu.frame[WIDTH], 0x0000); // line 1 untouched so far
}

#[test]
fn frame_ready_after_line_159_and_only_then() {
    let mut b = bus();
    b.cycles = 159 * CYCLES_PER_LINE;
    b.catch_up();
    assert!(!b.ppu.frame_ready);
    b.cycles = 160 * CYCLES_PER_LINE;
    b.catch_up();
    assert!(b.ppu.frame_ready);
    // vblank lines render nothing and don't re-trigger
    b.ppu.frame_ready = false;
    b.cycles = 228 * CYCLES_PER_LINE;
    b.catch_up();
    assert!(!b.ppu.frame_ready);
    // ...until the next frame's line 159 completes
    b.cycles = (228 + 160) * CYCLES_PER_LINE;
    b.catch_up();
    assert!(b.ppu.frame_ready);
}

#[test]
fn forced_blank_renders_white() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0080);
    b.cycles = CYCLES_PER_LINE;
    b.catch_up();
    assert!(b.ppu.frame[..WIDTH].iter().all(|&p| p == 0x7FFF));
}

#[test]
fn unimplemented_modes_render_backdrop() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0100); // mode 0, BG0 on — tiled, lands in G3
    b.write16(0x0500_0000, 0x03E0); // green backdrop
    b.cycles = CYCLES_PER_LINE;
    b.catch_up();
    assert!(b.ppu.frame[..WIDTH].iter().all(|&p| p == 0x03E0));
}

#[test]
fn bgr555_conversion_expands_channels() {
    use termboy_core::Rgb;
    assert_eq!(bgr555_to_rgb(0x0000), Rgb(0, 0, 0));
    assert_eq!(bgr555_to_rgb(0x7FFF), Rgb(255, 255, 255));
    assert_eq!(bgr555_to_rgb(0x001F), Rgb(255, 0, 0)); // red is low bits
    assert_eq!(bgr555_to_rgb(0x7C00), Rgb(0, 0, 255)); // blue is high bits
}

#[test]
fn mode3_renders_direct_color_pixels() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0403); // mode 3, BG2 on
    b.write16(0x0600_0000, 0x001F); // pixel (0,0) red
    b.write16(0x0600_0000 + 2 * (WIDTH as u32), 0x7C00); // pixel (0,1) blue
    b.cycles = 2 * CYCLES_PER_LINE;
    b.catch_up();
    assert_eq!(b.ppu.frame[0], 0x001F);
    assert_eq!(b.ppu.frame[WIDTH], 0x7C00);
    assert_eq!(b.ppu.frame[1], 0x0000); // untouched VRAM is black
}

#[test]
fn mode3_without_bg2_shows_backdrop() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0003); // mode 3, BG2 OFF
    b.write16(0x0500_0000, 0x7C00); // blue backdrop
    b.write16(0x0600_0000, 0x001F);
    b.cycles = CYCLES_PER_LINE;
    b.catch_up();
    assert_eq!(b.ppu.frame[0], 0x7C00);
}

#[test]
fn mode4_renders_palette_indices_with_page_flip() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0404); // mode 4, BG2, page 0
    b.write16(0x0500_0002, 0x001F); // palette[1] = red
    b.write16(0x0500_0004, 0x03E0); // palette[2] = green
    b.write16(0x0600_0000, 0x0201); // pixels (0,0)=idx1, (1,0)=idx2
    b.write16(0x0600_A000, 0x0102); // page 1 has them swapped
    b.cycles = CYCLES_PER_LINE;
    b.catch_up();
    assert_eq!(b.ppu.frame[0], 0x001F);
    assert_eq!(b.ppu.frame[1], 0x03E0);
    set_dispcnt(&mut b, 0x0414); // flip to page 1
    b.cycles = 228 * CYCLES_PER_LINE + CYCLES_PER_LINE; // next frame, line 0 done
    b.catch_up();
    assert_eq!(b.ppu.frame[0], 0x03E0);
    assert_eq!(b.ppu.frame[1], 0x001F);
}

#[test]
fn text_bg_renders_4bpp_tile_with_palette_bank() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0100); // mode 0, BG0
    b.write16(0x0400_0008, 0x0000); // BG0CNT: char base 0, screen base 0, 32x32
    b.write16(0x0500_0022, 0x001F); // palette bank 1, index 1 = red
    fill_tile_4bpp(&mut b, 0, 1, 1);
    set_entry(&mut b, 0, 0, 0, 0, 0x1001); // tile 1, palbank 1 at (0,0)
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[0], 0x001F);
    assert_eq!(b.ppu.frame[8], 0x0000); // next tile is map entry 0 = tile 0 = blank
}

#[test]
fn text_bg_scrolls_and_wraps() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0100);
    // screen base block 1: keep the map clear of tile 0's character data
    b.write16(0x0400_0008, 0x0100);
    b.write16(0x0500_0002, 0x03E0); // palette index 1 = green
    fill_tile_4bpp(&mut b, 0, 1, 1);
    set_entry(&mut b, 0x800, 0, 0, 0, 0x0001); // tile at map (0,0)
    b.write16(0x0400_0010, 250); // HOFS=250: map x wraps on-screen at x=6
    b.write16(0x0400_0012, 0x00F8); // VOFS=248: line 8 wraps to map y=0
    render_line(&mut b, 8);
    // screen (0,8): map coords (250, 0) -> tile (31,0) = blank
    assert_eq!(b.ppu.frame[8 * WIDTH], 0x0000);
    // screen x=6 -> map x=(6+250)&255=0 -> tile (0,0) -> green
    assert_eq!(b.ppu.frame[8 * WIDTH + 6], 0x03E0);
}

#[test]
fn text_bg_8bpp_and_flips() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0200); // mode 0, BG1
    b.write16(0x0400_000A, 0x0080); // BG1CNT: 8bpp
    b.write16(0x0500_0000 + 2 * 7, 0x7C00); // palette index 7 = blue
    // 8bpp tile 1: only leftmost column = index 7
    for row in 0..8u32 {
        b.write16(0x0600_0000 + 64 + row * 8, 0x0007);
    }
    set_entry(&mut b, 0, 0, 0, 0, 0x0001); // tile 1
    set_entry(&mut b, 0, 0, 1, 0, 0x0401); // tile 1, hflip
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[0], 0x7C00); // left column
    assert_eq!(b.ppu.frame[1], 0x0000);
    assert_eq!(b.ppu.frame[15], 0x7C00); // hflip: column shows at right edge
    assert_eq!(b.ppu.frame[8], 0x0000);
}

#[test]
fn text_bg_512_wide_uses_second_screen_block() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0100);
    b.write16(0x0400_0008, 0x4000); // BG0CNT: size 1 (512x256)
    b.write16(0x0500_0002, 0x001F);
    fill_tile_4bpp(&mut b, 0, 1, 1);
    set_entry(&mut b, 0, 1, 0, 0, 0x0001); // tile in screen block 1 (x 256-511)
    b.write16(0x0400_0010, 256); // HOFS=256
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[0], 0x001F);
}

#[test]
fn affine_bg_identity_renders_8bpp_map() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0402); // mode 2, BG2
    b.write16(0x0400_000C, 0x0000); // BG2CNT: 128x128, char/screen base 0
    b.write16(0x0500_000E, 0x001F); // palette index 7 = red
    // affine map: one byte per entry; entry (1,0) = tile 1
    b.write8(0x0600_0000, 0x01); // byte-write quirk duplicates: entries 0+1
    b.write8(0x0600_0002, 0x00); // clear entries 2+3 back (only 0/1 = tile 1)
    // 8bpp tile 1, all pixels index 7
    for i in 0..32 {
        b.write16(0x0600_0000 + 64 + i * 2, 0x0707);
    }
    // entry 0 must be tile 0: rewrite the halfword precisely
    b.write16(0x0600_0000, 0x0100); // entry 0 = 0x00, entry 1 = 0x01
    // identity matrix: PA=PD=1.0 (0x100), PB=PC=0
    b.write16(0x0400_0020, 0x0100);
    b.write16(0x0400_0026, 0x0100);
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[8], 0x001F); // x 8-15 = tile (1,0)
    assert_eq!(b.ppu.frame[0], 0x0000); // tile (0,0) = tile 0 = blank
}

#[test]
fn affine_bg_scale_and_reference_point() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0402);
    b.write16(0x0400_000C, 0x0000);
    b.write16(0x0500_000E, 0x001F);
    b.write16(0x0600_0000, 0x0100); // map entry 1 = tile 1
    for i in 0..32 {
        b.write16(0x0600_0000 + 64 + i * 2, 0x0707);
    }
    b.write16(0x0400_0020, 0x0200); // PA = 2.0: texture moves 2px per screen px
    b.write16(0x0400_0026, 0x0100);
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[4], 0x001F); // screen x4 -> texture x8
    assert_eq!(b.ppu.frame[8], 0x0000); // screen x8 -> texture x16: blank
    // reference point: start texture at (8,0) -> tile shows from screen x0
    b.write32(0x0400_0028, 8 << 8); // BG2X = 8.0 (write relatches)
    b.write16(0x0400_0020, 0x0100); // PA back to 1.0
    render_line(&mut b, 1); // next line picks up the relatch
    assert_eq!(b.ppu.frame[WIDTH], 0x001F); // screen x0 -> texture x8
}

#[test]
fn affine_bg_wrap_bit() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0402);
    b.write16(0x0500_000E, 0x001F);
    b.write16(0x0600_0000, 0x0100); // map entry 1 = tile 1
    for i in 0..32 {
        b.write16(0x0600_0000 + 64 + i * 2, 0x0707);
    }
    b.write16(0x0400_0020, 0x0100);
    b.write16(0x0400_0026, 0x0100);
    b.write32(0x0400_0028, (-8i32 << 8) as u32); // start 8px left of the map
    b.write16(0x0400_000C, 0x0000); // no wrap: out-of-map = transparent
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[0], 0x0000);
    b.write32(0x0400_0028, (-120i32 << 8) as u32); // wraps to texture x=8
    b.write16(0x0400_000C, 0x2000); // wrap bit
    render_line(&mut b, 1);
    assert_eq!(b.ppu.frame[WIDTH], 0x001F);
}

/// Minimal sprite writer: attrs for OAM entry n.
fn set_obj(b: &mut Bus, n: u32, a0: u16, a1: u16, a2: u16) {
    b.write16(0x0700_0000 + n * 8, a0);
    b.write16(0x0700_0000 + n * 8 + 2, a1);
    b.write16(0x0700_0000 + n * 8 + 4, a2);
}

/// 8x8 4bpp OBJ tile in slot `tile`, all pixels = `idx`.
fn fill_obj_tile_4bpp(b: &mut Bus, tile: u32, idx: u8) {
    let byte = idx | (idx << 4);
    for i in 0..16 {
        b.write16(0x0601_0000 + tile * 32 + i * 2, u16::from_le_bytes([byte, byte]));
    }
}

/// Affine parameter group g: PA/PB/PC/PD as 8.8 fixed point.
fn set_affine_params(b: &mut Bus, g: u32, pa: i16, pb: i16, pc: i16, pd: i16) {
    b.write16(0x0700_0000 + g * 32 + 6, pa as u16);
    b.write16(0x0700_0000 + g * 32 + 14, pb as u16);
    b.write16(0x0700_0000 + g * 32 + 22, pc as u16);
    b.write16(0x0700_0000 + g * 32 + 30, pd as u16);
}

#[test]
fn obj_renders_at_position_with_priority_over_bg() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x1100); // mode 0, BG0 + OBJ
    b.write16(0x0400_0008, 0x0002); // BG0 priority 2
    b.write16(0x0500_0002, 0x03E0); // BG palette 1 = green
    fill_tile_4bpp(&mut b, 0, 1, 1);
    set_entry(&mut b, 0x800, 0, 1, 0, 0x0001); // bg tile at x 8-15
    b.write16(0x0400_0008, 0x0102); // BG0: prio 2, screen base 1
    b.write16(0x0500_0202, 0x001F); // OBJ palette 1 = red
    fill_obj_tile_4bpp(&mut b, 1, 1);
    set_obj(&mut b, 0, 0x0000, 0x0008, 0x0401); // 8x8 at (8,0), prio 1, tile 1
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[8], 0x001F); // sprite (prio 1) over bg (prio 2)
    assert_eq!(b.ppu.frame[16], 0x0000); // sprite is 8 wide
    // flip priorities: bg wins
    b.write16(0x0400_0008, 0x0100); // BG0: prio 0
    set_obj(&mut b, 0, 0x0000, 0x0008, 0x0801); // prio 2
    render_line(&mut b, 1);
    assert_eq!(b.ppu.frame[WIDTH + 8], 0x03E0);
}

#[test]
fn obj_y_wrap_disable_and_lowest_index_wins() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x1000); // OBJ only
    b.write16(0x0500_0202, 0x001F); // OBJ pal 1 = red
    b.write16(0x0500_0204, 0x03E0); // OBJ pal 2 = green
    fill_obj_tile_4bpp(&mut b, 1, 1);
    fill_obj_tile_4bpp(&mut b, 2, 2);
    fill_obj_tile_4bpp(&mut b, 33, 1); // 2D mapping: row 1 of the tall sprite
    // y=248 wraps to -8: 16-tall sprite shows rows 8-15 on lines 0-7
    set_obj(&mut b, 0, 0x80F8, 0x0000, 0x0001); // 8x16 (shape vertical, size 0) at (0,-8)
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[0], 0x001F); // ly=8 -> tile slot 1 + 32 (2D stride)
    // disabled sprite renders nothing; overlapping: index 1 loses to index 0
    set_obj(&mut b, 0, 0x0000, 0x0000, 0x0001); // idx 0: red at (0,0)
    set_obj(&mut b, 1, 0x0000, 0x0000, 0x0002); // idx 1: green, same spot
    render_line(&mut b, 1);
    assert_eq!(b.ppu.frame[WIDTH], 0x001F); // lowest OAM index wins
    set_obj(&mut b, 0, 0x0200, 0x0000, 0x0001); // disable bit
    render_line(&mut b, 2);
    assert_eq!(b.ppu.frame[2 * WIDTH], 0x03E0); // idx 1 shows now
}

#[test]
fn obj_flips_and_1d_mapping() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x1040); // OBJ + 1D mapping
    b.write16(0x0500_0202, 0x001F);
    // tile 1: left column only; tile 2: all pixels (the 16x8 sprite's right half)
    for row in 0..8u32 {
        b.write16(0x0601_0000 + 32 + row * 4, 0x0001);
        b.write16(0x0601_0000 + 32 + row * 4 + 2, 0x0000);
    }
    fill_obj_tile_4bpp(&mut b, 2, 1);
    // 16x8 (shape horizontal, size 0), 1D: tiles 1,2 consecutive
    set_obj(&mut b, 0, 0x4000, 0x0000, 0x0001);
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[0], 0x001F); // tile 1 left column
    assert_eq!(b.ppu.frame[1], 0x0000);
    assert_eq!(b.ppu.frame[8], 0x001F); // tile 2 solid
    // hflip the whole sprite: solid half moves left, column to x=15
    set_obj(&mut b, 0, 0x4000, 0x1000, 0x0001);
    render_line(&mut b, 1);
    assert_eq!(b.ppu.frame[WIDTH], 0x001F); // solid tile now left
    assert_eq!(b.ppu.frame[WIDTH + 15], 0x001F); // column at right edge
    assert_eq!(b.ppu.frame[WIDTH + 14], 0x0000);
}

#[test]
fn affine_obj_identity_matches_regular() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x1000);
    b.write16(0x0500_0202, 0x001F);
    fill_obj_tile_4bpp(&mut b, 1, 1);
    set_affine_params(&mut b, 0, 0x100, 0, 0, 0x100);
    set_obj(&mut b, 0, 0x0100, 0x0000, 0x0001); // affine, group 0, 8x8 at (0,0)
    render_line(&mut b, 3);
    assert_eq!(b.ppu.frame[3 * WIDTH], 0x001F);
    assert_eq!(b.ppu.frame[3 * WIDTH + 7], 0x001F);
    assert_eq!(b.ppu.frame[3 * WIDTH + 8], 0x0000);
}

#[test]
fn affine_obj_half_scale_shrinks_with_double_size_canvas() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x1000);
    b.write16(0x0500_0202, 0x001F);
    fill_obj_tile_4bpp(&mut b, 1, 1);
    set_affine_params(&mut b, 0, 0x200, 0, 0, 0x200); // PA=2.0: half size on screen
    // affine + double (bit9): 8x8 sprite on a 16x16 canvas at (0,0)
    set_obj(&mut b, 0, 0x0300, 0x0000, 0x0001);
    render_line(&mut b, 8); // canvas centre row (ly=8, dy=0)
    // tx = 2*(lx-8) + 4, visible while tx in 0..8 -> canvas x 6..=9
    assert_eq!(b.ppu.frame[8 * WIDTH + 6], 0x001F);
    assert_eq!(b.ppu.frame[8 * WIDTH + 9], 0x001F);
    assert_eq!(b.ppu.frame[8 * WIDTH + 5], 0x0000); // tx = -2: outside
    assert_eq!(b.ppu.frame[8 * WIDTH + 10], 0x0000); // tx = 8: outside
}

#[test]
fn win0_gates_bg_inside_and_winout_outside() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x2100); // mode 0, BG0, WIN0
    b.write16(0x0400_0008, 0x0100); // BG0: screen base 1
    b.write16(0x0500_0002, 0x03E0);
    fill_tile_4bpp(&mut b, 0, 1, 1);
    for tx in 0..30 {
        set_entry(&mut b, 0x800, 0, tx, 0, 0x0001); // solid green line of tiles
    }
    b.write16(0x0400_0040, (8 << 8) | 16); // WIN0H: x 8..16
    b.write16(0x0400_0044, 160); // WIN0V: y 0..160
    b.write16(0x0400_0048, 0x0001); // WININ: BG0 inside
    b.write16(0x0400_004A, 0x0000); // WINOUT: nothing outside
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[8], 0x03E0); // inside window: BG0 shows
    assert_eq!(b.ppu.frame[7], 0x0000); // outside: backdrop only
    assert_eq!(b.ppu.frame[16], 0x0000); // right edge is exclusive
}

#[test]
fn win0_wraps_when_start_exceeds_end() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x2100);
    b.write16(0x0400_0008, 0x0100);
    b.write16(0x0500_0002, 0x03E0);
    fill_tile_4bpp(&mut b, 0, 1, 1);
    for tx in 0..30 {
        set_entry(&mut b, 0x800, 0, tx, 0, 0x0001);
    }
    b.write16(0x0400_0040, (200 << 8) | 16); // x >= 200 or x < 16
    b.write16(0x0400_0044, 160);
    b.write16(0x0400_0048, 0x0001);
    b.write16(0x0400_004A, 0x0000);
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[0], 0x03E0);
    assert_eq!(b.ppu.frame[210], 0x03E0);
    assert_eq!(b.ppu.frame[100], 0x0000);
}

#[test]
fn obj_window_uses_winout_high_byte() {
    let mut b = bus();
    // 0x9100 = OBJ-window enable (bit 15) + OBJ enable (bit 12) + BG0 (bit 8);
    // the window sprite needs the OBJ layer scanned to mark the mask.
    set_dispcnt(&mut b, 0x9100);
    b.write16(0x0400_0008, 0x0100); // BG0: screen base 1
    b.write16(0x0500_0002, 0x03E0);
    fill_tile_4bpp(&mut b, 0, 1, 1);
    for tx in 0..30 {
        set_entry(&mut b, 0x800, 0, tx, 0, 0x0001);
    }
    fill_obj_tile_4bpp(&mut b, 1, 1);
    set_obj(&mut b, 0, 0x0800, 0x0000, 0x0001); // mode 2 = OBJ-window sprite at (0,0)
    // WINOUT low byte (outside): BG0 visible; high byte (obj window): nothing
    b.write16(0x0400_004A, 0x0001);
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[0], 0x0000); // inside obj window: BG0 hidden
    assert_eq!(b.ppu.frame[8], 0x03E0); // outside: BG0 shows
}

#[test]
fn alpha_blend_mixes_first_and_second_target() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0300); // mode 0, BG0 + BG1
    b.write16(0x0400_0008, 0x0100); // BG0: prio 0 (top), screen base 1
    b.write16(0x0400_000A, 0x0201); // BG1: prio 1, screen base 2
    b.write16(0x0500_0002, 0x001F); // pal 1 = red (BG0 tile)
    b.write16(0x0500_0004, 0x7C00); // pal 2 = blue (BG1 tile)
    fill_tile_4bpp(&mut b, 0, 1, 1);
    fill_tile_4bpp(&mut b, 0, 2, 2);
    set_entry(&mut b, 0x800, 0, 0, 0, 0x0001);
    set_entry(&mut b, 0x1000, 0, 0, 0, 0x0002); // into BG1's screen block
    b.write16(0x0400_0050, 0x0241); // BLDCNT: effect 1 (alpha), 1st=BG0, 2nd=BG1
    b.write16(0x0400_0052, 0x0808); // EVA=EVB=8/16: 50/50
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[0], 0x3C0F); // red 31->15, blue 31->15
}

#[test]
fn brighten_and_darken() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0100);
    b.write16(0x0400_0008, 0x0100); // BG0: screen base 1
    b.write16(0x0500_0002, 0x0010); // pal 1 = mid red (16)
    fill_tile_4bpp(&mut b, 0, 1, 1);
    set_entry(&mut b, 0x800, 0, 0, 0, 0x0001);
    b.write16(0x0400_0050, 0x0081); // brighten, 1st = BG0
    b.write16(0x0400_0054, 8); // EVY 8/16
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[0] & 31, 16 + (31 - 16) * 8 / 16); // 23
    b.write16(0x0400_0050, 0x00C1); // darken
    render_line(&mut b, 1);
    assert_eq!(b.ppu.frame[WIDTH] & 31, 16 - 16 * 8 / 16); // 8
}

#[test]
fn semi_transparent_obj_forces_alpha() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x1100); // BG0 + OBJ
    b.write16(0x0400_0008, 0x0100); // BG0: screen base 1
    b.write16(0x0500_0002, 0x7C00); // BG: blue
    fill_tile_4bpp(&mut b, 0, 1, 1);
    set_entry(&mut b, 0x800, 0, 0, 0, 0x0001);
    set_entry(&mut b, 0x800, 0, 1, 0, 0x0001); // BG continues past the sprite
    b.write16(0x0500_0202, 0x001F); // OBJ: red
    fill_obj_tile_4bpp(&mut b, 1, 1);
    set_obj(&mut b, 0, 0x0400, 0x0000, 0x0001); // mode 1 = semi-transparent
    b.write16(0x0400_0050, 0x0100); // effect=none, but 2nd target = BG0
    b.write16(0x0400_0052, 0x0808);
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[0], 0x3C0F); // forced 50/50 despite effect=0
    assert_eq!(b.ppu.frame[8], 0x7C00); // off-sprite: plain BG
}

#[test]
fn bg_mosaic_repeats_cell_origin() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0100);
    b.write16(0x0400_0008, 0x0140); // BG0CNT: mosaic + screen base 1
    b.write16(0x0400_004C, 0x0033); // BG mosaic 4x4
    b.write16(0x0500_0002, 0x001F);
    // tile 1: only leftmost column = index 1
    for row in 0..8u32 {
        b.write16(0x0600_0000 + 32 + row * 4, u16::from_le_bytes([0x01, 0x00]));
    }
    set_entry(&mut b, 0x800, 0, 0, 0, 0x0001);
    render_line(&mut b, 0);
    // the 1px column smears across the 4px mosaic cell
    assert_eq!(b.ppu.frame[0], 0x001F);
    assert_eq!(b.ppu.frame[3], 0x001F);
    assert_eq!(b.ppu.frame[4], 0x0000);
    // line 5 samples line 4's content (cell origin), same column
    render_line(&mut b, 5);
    assert_eq!(b.ppu.frame[5 * WIDTH], 0x001F);
}

#[test]
fn obj_mosaic_smears_horizontally() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x1000);
    b.write16(0x0400_004C, 0x0300); // OBJ mosaic h=4
    b.write16(0x0500_0202, 0x001F);
    // obj tile 1: leftmost column only
    for row in 0..8u32 {
        b.write16(0x0601_0000 + 32 + row * 4, u16::from_le_bytes([0x01, 0x00]));
        b.write16(0x0601_0000 + 32 + row * 4 + 2, 0x0000);
    }
    set_obj(&mut b, 0, 0x1000, 0x0000, 0x0001); // mosaic bit (attr0 bit 12)
    render_line(&mut b, 0);
    assert_eq!(b.ppu.frame[0], 0x001F);
    assert_eq!(b.ppu.frame[3], 0x001F); // smeared
    assert_eq!(b.ppu.frame[4], 0x0000);
}

#[test]
fn mode5_small_bitmap_with_backdrop_border() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0405); // mode 5, BG2, page 0
    b.write16(0x0500_0000, 0x7C00); // blue backdrop
    b.write16(0x0600_0000, 0x001F); // pixel (0,0) of the 160x128 window
    b.cycles = CYCLES_PER_LINE;
    b.catch_up();
    assert_eq!(b.ppu.frame[0], 0x001F);
    assert_eq!(b.ppu.frame[160], 0x7C00); // x >= 160: backdrop
    // line 128+ is all backdrop
    b.cycles = 129 * CYCLES_PER_LINE;
    b.catch_up();
    assert!(b.ppu.frame[128 * WIDTH..129 * WIDTH].iter().all(|&p| p == 0x7C00));
}
