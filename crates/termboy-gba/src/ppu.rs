//! Scanline PPU. G2 renders the bitmap modes (3/4/5); tiled modes are G3.
//! Catch-up model: the bus cycle counter is the clock, and `ppu_catch_up`
//! renders every scanline the CPU has fully crossed since the last call —
//! the GBA-scaled version of the GB core's lockstep PPU (design spec §2).

use termboy_core::Rgb;

use crate::bus::{Bus, CYCLES_PER_LINE, LINES};

pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 160;

pub struct Ppu {
    /// BGR555 per pixel, row-major 240x160.
    pub frame: Box<[u16; WIDTH * HEIGHT]>,
    /// Set when scanline 159 completes; consumed by GbaCore::run_frame.
    pub frame_ready: bool,
    /// Absolute count of fully-elapsed scanlines since power-on.
    lines_done: u64,
}

impl Ppu {
    pub fn new() -> Self {
        Self { frame: Box::new([0; WIDTH * HEIGHT]), frame_ready: false, lines_done: 0 }
    }
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

/// GBA color: 0bbbbbgggggrrrrr. Expand 5 bits to 8 (top bits replicated).
pub fn bgr555_to_rgb(c: u16) -> Rgb {
    let e = |v: u16| ((v << 3) | (v >> 2)) as u8;
    Rgb(e(c & 31), e((c >> 5) & 31), e((c >> 10) & 31))
}

impl Bus {
    /// Render every scanline the cycle counter has completed since the last
    /// call. GbaCore calls this after each CPU step, so at most a line or two
    /// pass per call.
    pub fn ppu_catch_up(&mut self) {
        let completed = self.cycles / CYCLES_PER_LINE;
        while self.ppu.lines_done < completed {
            let line = (self.ppu.lines_done % LINES) as usize;
            self.ppu.lines_done += 1;
            if line < HEIGHT {
                self.render_scanline(line);
                if line == HEIGHT - 1 {
                    self.ppu.frame_ready = true;
                }
            }
        }
    }

    fn render_scanline(&mut self, line: usize) {
        let dispcnt = u16::from_le_bytes([self.io[0], self.io[1]]);
        let row = &mut self.ppu.frame[line * WIDTH..(line + 1) * WIDTH];
        if dispcnt & 0x80 != 0 {
            row.fill(0x7FFF); // forced blank: the LCD shows white
            return;
        }
        let backdrop = u16::from_le_bytes([self.palette[0], self.palette[1]]);
        row.fill(backdrop);
        let bg2 = dispcnt & (1 << 10) != 0;
        if !bg2 {
            return;
        }
        match dispcnt & 7 {
            3 => {
                for (x, px) in row.iter_mut().enumerate() {
                    let i = (line * WIDTH + x) * 2;
                    *px = u16::from_le_bytes([self.vram[i], self.vram[i + 1]]);
                }
            }
            4 => {
                let page = if dispcnt & (1 << 4) != 0 { 0xA000 } else { 0 };
                for (x, px) in row.iter_mut().enumerate() {
                    let idx = self.vram[page + line * WIDTH + x] as usize * 2;
                    *px = u16::from_le_bytes([self.palette[idx], self.palette[idx + 1]]);
                }
            }
            5 => {
                let page = if dispcnt & (1 << 4) != 0 { 0xA000 } else { 0 };
                for (x, px) in row.iter_mut().enumerate() {
                    if x < 160 && line < 128 {
                        let i = page + (line * 160 + x) * 2;
                        *px = u16::from_le_bytes([self.vram[i], self.vram[i + 1]]);
                    }
                }
            }
            _ => {} // tiled modes 0-2 land in G3; backdrop until then
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;

    fn bus() -> Bus {
        Bus::new(vec![0u8; 0x200])
    }

    /// DISPCNT lives at 0x04000000; the PPU reads it per scanline.
    fn set_dispcnt(b: &mut Bus, v: u16) {
        b.write16(0x0400_0000, v);
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
}
