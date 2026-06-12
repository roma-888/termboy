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
    b.ppu_catch_up();
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
    b.cycles = CYCLES_PER_LINE - 1; // line 0 not finished yet
    b.ppu_catch_up();
    assert!(!b.ppu.frame_ready);
    b.write16(0x0500_0000, 0x7C00); // backdrop = blue, set BEFORE line completes
    b.cycles = CYCLES_PER_LINE; // line 0 done
    b.ppu_catch_up();
    assert_eq!(b.ppu.frame[0], 0x7C00); // line 0 rendered with current state
    assert_eq!(b.ppu.frame[WIDTH], 0x0000); // line 1 untouched so far
}

#[test]
fn frame_ready_after_line_159_and_only_then() {
    let mut b = bus();
    b.cycles = 159 * CYCLES_PER_LINE;
    b.ppu_catch_up();
    assert!(!b.ppu.frame_ready);
    b.cycles = 160 * CYCLES_PER_LINE;
    b.ppu_catch_up();
    assert!(b.ppu.frame_ready);
    // vblank lines render nothing and don't re-trigger
    b.ppu.frame_ready = false;
    b.cycles = 228 * CYCLES_PER_LINE;
    b.ppu_catch_up();
    assert!(!b.ppu.frame_ready);
    // ...until the next frame's line 159 completes
    b.cycles = (228 + 160) * CYCLES_PER_LINE;
    b.ppu_catch_up();
    assert!(b.ppu.frame_ready);
}

#[test]
fn forced_blank_renders_white() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0080);
    b.cycles = CYCLES_PER_LINE;
    b.ppu_catch_up();
    assert!(b.ppu.frame[..WIDTH].iter().all(|&p| p == 0x7FFF));
}

#[test]
fn unimplemented_modes_render_backdrop() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0100); // mode 0, BG0 on — tiled, lands in G3
    b.write16(0x0500_0000, 0x03E0); // green backdrop
    b.cycles = CYCLES_PER_LINE;
    b.ppu_catch_up();
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
    b.ppu_catch_up();
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
    b.ppu_catch_up();
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
    b.ppu_catch_up();
    assert_eq!(b.ppu.frame[0], 0x001F);
    assert_eq!(b.ppu.frame[1], 0x03E0);
    set_dispcnt(&mut b, 0x0414); // flip to page 1
    b.cycles = 228 * CYCLES_PER_LINE + CYCLES_PER_LINE; // next frame, line 0 done
    b.ppu_catch_up();
    assert_eq!(b.ppu.frame[0], 0x03E0);
    assert_eq!(b.ppu.frame[1], 0x001F);
}

#[test]
fn mode5_small_bitmap_with_backdrop_border() {
    let mut b = bus();
    set_dispcnt(&mut b, 0x0405); // mode 5, BG2, page 0
    b.write16(0x0500_0000, 0x7C00); // blue backdrop
    b.write16(0x0600_0000, 0x001F); // pixel (0,0) of the 160x128 window
    b.cycles = CYCLES_PER_LINE;
    b.ppu_catch_up();
    assert_eq!(b.ppu.frame[0], 0x001F);
    assert_eq!(b.ppu.frame[160], 0x7C00); // x >= 160: backdrop
    // line 128+ is all backdrop
    b.cycles = 129 * CYCLES_PER_LINE;
    b.ppu_catch_up();
    assert!(b.ppu.frame[128 * WIDTH..129 * WIDTH].iter().all(|&p| p == 0x7C00));
}
