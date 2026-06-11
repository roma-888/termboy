//! Scanline renderer — the one deliberately-pragmatic module (design spec §3).
//! Swapping this for a pixel-FIFO later touches nothing outside `ppu/`.

use super::*;

impl Ppu {
    pub(super) fn render_line(&mut self) {
        let y = self.ly as usize;
        let mut line = [0u8; 160]; // BGP/OBP-mapped shades (what the screen shows)
        let mut bg_color = [0u8; 160]; // raw BG color numbers (for sprite priority)

        if self.lcdc & LCDC_BG_ON != 0 {
            self.draw_background(&mut line, &mut bg_color);
            self.draw_window(&mut line, &mut bg_color);
        }
        if self.lcdc & LCDC_OBJ_ON != 0 {
            self.draw_sprites(&mut line, &bg_color);
        }

        self.frame[y * 160..(y + 1) * 160].copy_from_slice(&line);
    }

    fn draw_background(&self, line: &mut [u8; 160], bg_color: &mut [u8; 160]) {
        let map = if self.lcdc & LCDC_BG_MAP != 0 { 0x1C00 } else { 0x1800 };
        let by = self.scy.wrapping_add(self.ly);
        let trow = (by / 8) as usize;
        let py = (by % 8) as usize;
        for x in 0u8..160 {
            let bx = self.scx.wrapping_add(x);
            let tile = self.vram[map + trow * 32 + (bx / 8) as usize];
            let color = self.bg_tile_pixel(tile, (bx % 8) as usize, py);
            bg_color[x as usize] = color;
            line[x as usize] = (self.bgp >> (color * 2)) & 3;
        }
    }

    fn draw_window(&mut self, line: &mut [u8; 160], bg_color: &mut [u8; 160]) {
        if self.lcdc & LCDC_WIN_ON == 0 || !self.wy_hit || self.wx > 166 {
            return;
        }
        let map = if self.lcdc & LCDC_WIN_MAP != 0 { 0x1C00 } else { 0x1800 };
        let wl = self.window_line as usize;
        let trow = wl / 8;
        let py = wl % 8;
        let start = self.wx.saturating_sub(7) as usize;
        for x in start..160 {
            let wxp = x + 7 - self.wx as usize; // pixel within the window
            let tile = self.vram[map + trow * 32 + wxp / 8];
            let color = self.bg_tile_pixel(tile, wxp % 8, py);
            bg_color[x] = color;
            line[x] = (self.bgp >> (color * 2)) & 3;
        }
        self.window_line += 1; // only advances on lines the window rendered
    }

    fn draw_sprites(&self, line: &mut [u8; 160], bg_color: &[u8; 160]) {
        // implemented in Task 4
        let _ = (line, bg_color);
    }

    /// BG/window tile pixel via LCDC bit-4 addressing. Returns color number 0-3.
    fn bg_tile_pixel(&self, tile: u8, px: usize, py: usize) -> u8 {
        let base = if self.lcdc & LCDC_TILE_DATA != 0 {
            tile as usize * 16
        } else {
            (0x1000i32 + (tile as i8 as i32) * 16) as usize
        };
        let lo = (self.vram[base + py * 2] >> (7 - px)) & 1;
        let hi = (self.vram[base + py * 2 + 1] >> (7 - px)) & 1;
        (hi << 1) | lo
    }
}
