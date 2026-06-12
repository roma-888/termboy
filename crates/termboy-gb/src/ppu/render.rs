//! Scanline renderer — the one deliberately-pragmatic module (design spec §3).
//! Swapping this for a pixel-FIFO later touches nothing outside `ppu/`.

use super::*;

fn pal_rgb555(pal: &[u8; 64], palette: usize, color: usize) -> u16 {
    let i = palette * 8 + color * 2;
    pal[i] as u16 | ((pal[i + 1] as u16) << 8)
}

impl Ppu {
    pub(super) fn render_line(&mut self) {
        if self.cgb {
            self.render_line_cgb();
            return;
        }
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
        let height: i16 = if self.lcdc & LCDC_OBJ_TALL != 0 { 16 } else { 8 };
        let ly = self.ly as i16;

        // OAM scan: first 10 sprites overlapping this line, in OAM order.
        let mut hits: Vec<(i16, usize)> = Vec::with_capacity(10); // (x, oam index)
        for i in 0..40 {
            let sy = self.oam[i * 4] as i16 - 16;
            if ly >= sy && ly < sy + height {
                hits.push((self.oam[i * 4 + 1] as i16 - 8, i));
                if hits.len() == 10 {
                    break;
                }
            }
        }
        // Priority: lowest X wins, ties broken by OAM index (DMG rule).
        hits.sort_by_key(|&(x, i)| (x, i));

        for px in 0..160i16 {
            // Winner = highest-priority sprite with an opaque pixel here.
            for &(sx, i) in &hits {
                if px < sx || px >= sx + 8 {
                    continue;
                }
                let attr = self.oam[i * 4 + 3];
                let mut row = ly - (self.oam[i * 4] as i16 - 16);
                if attr & 0x40 != 0 {
                    row = height - 1 - row; // y-flip
                }
                let mut tile = self.oam[i * 4 + 2];
                if height == 16 {
                    tile &= 0xFE; // rows 8-15 fall through into the next tile
                }
                let mut col = (px - sx) as u8;
                if attr & 0x20 != 0 {
                    col = 7 - col; // x-flip
                }
                // OBJ tiles always use unsigned 0x8000 addressing.
                let base = tile as usize * 16 + row as usize * 2;
                let lo = (self.vram[base] >> (7 - col)) & 1;
                let hi = (self.vram[base + 1] >> (7 - col)) & 1;
                let color = (hi << 1) | lo;
                if color == 0 {
                    continue; // transparent: next sprite may win this pixel
                }
                // The WINNER's BG-over-OBJ flag decides; losers don't get a say.
                if attr & 0x80 == 0 || bg_color[px as usize] == 0 {
                    let palette = if attr & 0x10 != 0 { self.obp1 } else { self.obp0 };
                    line[px as usize] = (palette >> (color * 2)) & 3;
                }
                break;
            }
        }
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

// ---- CGB scanline path. Writes RGB555 into frame_rgb; the DMG path above is
// untouched. BG map attributes live at the same offset in VRAM bank 1.
impl Ppu {
    fn render_line_cgb(&mut self) {
        let y = self.ly as usize;
        let mut line = [0u16; 160];
        let mut bg_color = [0u8; 160]; // color number 0-3 (for sprite mixing)
        let mut bg_prio = [false; 160]; // BG attribute bit 7
        // BG and window always render in CGB; LCDC bit 0 is priority, not enable.
        self.draw_bg_cgb(&mut line, &mut bg_color, &mut bg_prio);
        self.draw_window_cgb(&mut line, &mut bg_color, &mut bg_prio);
        if self.lcdc & LCDC_OBJ_ON != 0 {
            self.draw_sprites_cgb(&mut line, &bg_color, &bg_prio);
        }
        self.frame_rgb[y * 160..(y + 1) * 160].copy_from_slice(&line);
    }

    /// Tile pixel honoring the CGB attribute byte (bank + flips).
    fn cgb_tile_pixel(&self, tile: u8, attr: u8, mut px: usize, mut py: usize) -> u8 {
        if attr & 0x20 != 0 {
            px = 7 - px;
        }
        if attr & 0x40 != 0 {
            py = 7 - py;
        }
        let bank = ((attr >> 3) & 1) as usize * 0x2000;
        let base = bank
            + if self.lcdc & LCDC_TILE_DATA != 0 {
                tile as usize * 16
            } else {
                (0x1000i32 + (tile as i8 as i32) * 16) as usize
            };
        let lo = (self.vram[base + py * 2] >> (7 - px)) & 1;
        let hi = (self.vram[base + py * 2 + 1] >> (7 - px)) & 1;
        (hi << 1) | lo
    }

    fn draw_bg_cgb(&self, line: &mut [u16; 160], bg_color: &mut [u8; 160], bg_prio: &mut [bool; 160]) {
        let map = if self.lcdc & LCDC_BG_MAP != 0 { 0x1C00 } else { 0x1800 };
        let by = self.scy.wrapping_add(self.ly);
        let trow = (by / 8) as usize;
        let py = (by % 8) as usize;
        for x in 0u8..160 {
            let bx = self.scx.wrapping_add(x);
            let i = map + trow * 32 + (bx / 8) as usize;
            let tile = self.vram[i];
            let attr = self.vram[0x2000 + i];
            let color = self.cgb_tile_pixel(tile, attr, (bx % 8) as usize, py);
            bg_color[x as usize] = color;
            bg_prio[x as usize] = attr & 0x80 != 0;
            line[x as usize] = pal_rgb555(&self.bg_pal, (attr & 7) as usize, color as usize);
        }
    }

    fn draw_window_cgb(&mut self, line: &mut [u16; 160], bg_color: &mut [u8; 160], bg_prio: &mut [bool; 160]) {
        if self.lcdc & LCDC_WIN_ON == 0 || !self.wy_hit || self.wx > 166 {
            return;
        }
        let map = if self.lcdc & LCDC_WIN_MAP != 0 { 0x1C00 } else { 0x1800 };
        let wl = self.window_line as usize;
        let trow = wl / 8;
        let py = wl % 8;
        let start = self.wx.saturating_sub(7) as usize;
        for x in start..160 {
            let wxp = x + 7 - self.wx as usize;
            let i = map + trow * 32 + wxp / 8;
            let tile = self.vram[i];
            let attr = self.vram[0x2000 + i];
            let color = self.cgb_tile_pixel(tile, attr, wxp % 8, py);
            bg_color[x] = color;
            bg_prio[x] = attr & 0x80 != 0;
            line[x] = pal_rgb555(&self.bg_pal, (attr & 7) as usize, color as usize);
        }
        self.window_line += 1;
    }

    fn draw_sprites_cgb(&self, line: &mut [u16; 160], bg_color: &[u8; 160], bg_prio: &[bool; 160]) {
        let height: i16 = if self.lcdc & LCDC_OBJ_TALL != 0 { 16 } else { 8 };
        let ly = self.ly as i16;
        let master = self.lcdc & LCDC_BG_ON != 0; // CGB: master priority enable

        // First 10 sprites on this line, in OAM order — which IS the CGB
        // priority order (no X sorting, unlike DMG).
        let mut hits: Vec<(i16, usize)> = Vec::with_capacity(10);
        for i in 0..40 {
            let sy = self.oam[i * 4] as i16 - 16;
            if ly >= sy && ly < sy + height {
                hits.push((self.oam[i * 4 + 1] as i16 - 8, i));
                if hits.len() == 10 {
                    break;
                }
            }
        }

        for px in 0..160i16 {
            for &(sx, i) in &hits {
                if px < sx || px >= sx + 8 {
                    continue;
                }
                let attr = self.oam[i * 4 + 3];
                let mut row = ly - (self.oam[i * 4] as i16 - 16);
                if attr & 0x40 != 0 {
                    row = height - 1 - row;
                }
                let mut tile = self.oam[i * 4 + 2];
                if height == 16 {
                    tile &= 0xFE;
                }
                let mut col = (px - sx) as u8;
                if attr & 0x20 != 0 {
                    col = 7 - col;
                }
                let bank = ((attr >> 3) & 1) as usize * 0x2000;
                let base = bank + tile as usize * 16 + row as usize * 2;
                let lo = (self.vram[base] >> (7 - col)) & 1;
                let hi = (self.vram[base + 1] >> (7 - col)) & 1;
                let color = (hi << 1) | lo;
                if color == 0 {
                    continue;
                }
                let x = px as usize;
                let visible = !master
                    || bg_color[x] == 0
                    || (!bg_prio[x] && attr & 0x80 == 0);
                if visible {
                    line[x] = pal_rgb555(&self.obj_pal, (attr & 7) as usize, color as usize);
                }
                break;
            }
        }
    }
}
