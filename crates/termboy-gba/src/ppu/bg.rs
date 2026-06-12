//! Background line producers: text and affine.

use super::WIDTH;
use crate::bus::Bus;

impl Bus {
    pub(crate) fn render_text_bg(&mut self, bg: usize, line: usize) {
        let cnt = self.io16(0x008 + bg * 2);
        let hofs = (self.io16(0x010 + bg * 4) & 0x1FF) as usize;
        let vofs = (self.io16(0x012 + bg * 4) & 0x1FF) as usize;
        let char_base = ((cnt >> 2) & 3) as usize * 0x4000;
        let screen_base = ((cnt >> 8) & 0x1F) as usize * 0x800;
        let bpp8 = cnt & (1 << 7) != 0;
        let size = (cnt >> 14) & 3; // 0:256x256 1:512x256 2:256x512 3:512x512
        let (xmask, ymask) = match size {
            0 => (255, 255),
            1 => (511, 255),
            2 => (255, 511),
            _ => (511, 511),
        };
        let y = (line + vofs) & ymask;
        for x in 0..WIDTH {
            let xx = (x + hofs) & xmask;
            // 512-px maps are stitched from 32x32-tile screen blocks
            let sbb = match size {
                0 => 0,
                1 => xx >> 8,
                2 => y >> 8,
                _ => (xx >> 8) + ((y >> 8) << 1),
            };
            let entry_addr =
                screen_base + sbb * 0x800 + (((y >> 3) & 31) * 32 + ((xx >> 3) & 31)) * 2;
            let entry = u16::from_le_bytes([self.vram[entry_addr], self.vram[entry_addr + 1]]);
            let tile = (entry & 0x3FF) as usize;
            let mut px = xx & 7;
            let mut py = y & 7;
            if entry & 0x400 != 0 {
                px = 7 - px;
            }
            if entry & 0x800 != 0 {
                py = 7 - py;
            }
            let idx = if bpp8 {
                self.vram[(char_base + tile * 64 + py * 8 + px) & 0xFFFF] as usize
            } else {
                let byte = self.vram[(char_base + tile * 32 + py * 4 + px / 2) & 0xFFFF];
                let nib = (if px & 1 == 0 { byte & 0xF } else { byte >> 4 }) as usize;
                if nib == 0 {
                    0
                } else {
                    ((entry >> 12) & 0xF) as usize * 16 + nib
                }
            };
            if idx != 0 {
                self.ppu.bg_line[bg][x] = self.palette16(idx);
            }
        }
    }

    /// Affine BG (BG2/BG3). Renders into bg_line when `enabled`; the internal
    /// reference point advances by PB/PD per scanline either way.
    pub(crate) fn render_affine_bg(&mut self, bg: usize, line: usize, enabled: bool) {
        let slot = bg - 2;
        let reg_base = 0x020 + slot * 0x10;
        if self.ppu.bg_ref_dirty[slot] || line == 0 {
            let sext28 = |v: u32| ((v << 4) as i32) >> 4;
            self.ppu.bg_ref[slot] = (
                sext28(self.io32(0x028 + slot * 0x10)),
                sext28(self.io32(0x02C + slot * 0x10)),
            );
            self.ppu.bg_ref_dirty[slot] = false;
        }
        let pa = self.io16(reg_base) as i16 as i32;
        let pb = self.io16(reg_base + 2) as i16 as i32;
        let pc = self.io16(reg_base + 4) as i16 as i32;
        let pd = self.io16(reg_base + 6) as i16 as i32;
        let (ref_x, ref_y) = self.ppu.bg_ref[slot];

        if enabled {
            let cnt = self.io16(0x008 + bg * 2);
            let char_base = ((cnt >> 2) & 3) as usize * 0x4000;
            let screen_base = ((cnt >> 8) & 0x1F) as usize * 0x800;
            let size_tiles = 16usize << ((cnt >> 14) & 3); // 16..128 tiles square
            let size_px = (size_tiles * 8) as i32;
            let wrap = cnt & (1 << 13) != 0;
            let (mut cx, mut cy) = (ref_x, ref_y);
            for x in 0..WIDTH {
                let (mut sx, mut sy) = (cx >> 8, cy >> 8);
                cx += pa;
                cy += pc;
                if wrap {
                    sx = sx.rem_euclid(size_px);
                    sy = sy.rem_euclid(size_px);
                } else if sx < 0 || sx >= size_px || sy < 0 || sy >= size_px {
                    continue;
                }
                let (sx, sy) = (sx as usize, sy as usize);
                let tile =
                    self.vram[(screen_base + (sy / 8) * size_tiles + sx / 8) & 0xFFFF] as usize;
                let idx = self.vram[(char_base + tile * 64 + (sy & 7) * 8 + (sx & 7)) & 0xFFFF]
                    as usize;
                if idx != 0 {
                    self.ppu.bg_line[bg][x] = self.palette16(idx);
                }
            }
        }
        // advance the internal reference to the next scanline
        self.ppu.bg_ref[slot].0 = ref_x + pb;
        self.ppu.bg_ref[slot].1 = ref_y + pd;
    }
}
