//! Object (sprite) line producer. Among overlapping sprites the lowest OAM
//! index wins the pixel; its priority attribute then competes with BGs in
//! compose. OBJ-window sprites only mark the window mask.

use super::{ObjPx, TRANSPARENT, WIDTH};
use crate::bus::Bus;

const SIZES: [[(i32, i32); 4]; 3] = [
    [(8, 8), (16, 16), (32, 32), (64, 64)], // square
    [(16, 8), (32, 8), (32, 16), (64, 32)], // horizontal
    [(8, 16), (8, 32), (16, 32), (32, 64)], // vertical
];

impl Bus {
    pub(crate) fn render_objects(&mut self, line: usize, dispcnt: u16) {
        let map_1d = dispcnt & (1 << 6) != 0;
        let bitmap_mode = dispcnt & 7 >= 3;
        for n in 0..128usize {
            let a0 = self.oam16(n * 8);
            let a1 = self.oam16(n * 8 + 2);
            let a2 = self.oam16(n * 8 + 4);
            let affine = a0 & 0x100 != 0;
            if !affine && a0 & 0x200 != 0 {
                continue; // disabled
            }
            let mode = (a0 >> 10) & 3;
            if mode == 3 {
                continue; // prohibited
            }
            let shape = (a0 >> 14) as usize;
            if shape == 3 {
                continue;
            }
            let (w, h) = SIZES[shape][(a1 >> 14) as usize];
            let double = affine && a0 & 0x200 != 0;
            let (bw, bh) = if double { (w * 2, h * 2) } else { (w, h) };
            let mut oy = (a0 & 0xFF) as i32;
            if oy + bh > 256 {
                oy -= 256;
            }
            let mut ox = (a1 & 0x1FF) as i32;
            if ox >= 240 {
                ox -= 512;
            }
            let ly_raw = line as i32 - oy;
            if ly_raw < 0 || ly_raw >= bh {
                continue;
            }
            let bpp8 = a0 & 0x2000 != 0;
            let base_tile = (a2 & 0x3FF) as usize;
            let prio = ((a2 >> 10) & 3) as u8;
            let palbank = ((a2 >> 12) & 0xF) as usize;
            // mosaic: local coords derive from the mosaic-cell origin
            let (mh, mv) = if a0 & 0x1000 != 0 {
                let m = self.io16(0x04C);
                (((m >> 8) & 0xF) as i32 + 1, ((m >> 12) & 0xF) as i32 + 1)
            } else {
                (1, 1)
            };
            let ly = ((line as i32 - line as i32 % mv) - oy).clamp(0, bh - 1);
            if affine {
                let group = ((a1 >> 9) & 0x1F) as usize;
                let pa = self.oam16(group * 32 + 6) as i16 as i32;
                let pb = self.oam16(group * 32 + 14) as i16 as i32;
                let pc = self.oam16(group * 32 + 22) as i16 as i32;
                let pd = self.oam16(group * 32 + 30) as i16 as i32;
                let dy = ly - bh / 2;
                for lx in 0..bw {
                    let sx = ox + lx;
                    if !(0..WIDTH as i32).contains(&sx) {
                        continue;
                    }
                    let lx = ((sx - sx % mh) - ox).clamp(0, bw - 1);
                    let dx = lx - bw / 2;
                    let tx = ((pa * dx + pb * dy) >> 8) + w / 2;
                    let ty = ((pc * dx + pd * dy) >> 8) + h / 2;
                    if !(0..w).contains(&tx) || !(0..h).contains(&ty) {
                        continue;
                    }
                    let Some(idx) =
                        self.obj_pixel(base_tile, w, tx, ty, bpp8, map_1d, bitmap_mode)
                    else {
                        continue;
                    };
                    let color_index = if bpp8 { 256 + idx } else { 256 + palbank * 16 + idx };
                    self.put_obj_pixel(sx as usize, color_index, prio, mode);
                }
                continue;
            }
            for lx in 0..bw {
                let sx = ox + lx;
                if !(0..WIDTH as i32).contains(&sx) {
                    continue;
                }
                let mut tx = ((sx - sx % mh) - ox).clamp(0, bw - 1);
                let mut ty = ly;
                if a1 & 0x1000 != 0 {
                    tx = w - 1 - tx;
                }
                if a1 & 0x2000 != 0 {
                    ty = h - 1 - ty;
                }
                let Some(idx) = self.obj_pixel(base_tile, w, tx, ty, bpp8, map_1d, bitmap_mode)
                else {
                    continue;
                };
                let color_index = if bpp8 { 256 + idx } else { 256 + palbank * 16 + idx };
                self.put_obj_pixel(sx as usize, color_index, prio, mode);
            }
        }
    }

    /// Fetch one sprite texel (palette index 1-255), or None if transparent
    /// or addressing an unusable tile.
    fn obj_pixel(
        &self,
        base_tile: usize,
        w: i32,
        tx: i32,
        ty: i32,
        bpp8: bool,
        map_1d: bool,
        bitmap_mode: bool,
    ) -> Option<usize> {
        let step = if bpp8 { 2 } else { 1 };
        let row_stride = if map_1d { (w as usize / 8) * step } else { 32 };
        let slot = base_tile + (ty as usize / 8) * row_stride + (tx as usize / 8) * step;
        if bitmap_mode && slot < 512 {
            return None; // tiles 0-511 overlap bitmap VRAM: unusable
        }
        let (px, py) = (tx as usize & 7, ty as usize & 7);
        let idx = if bpp8 {
            self.vram[0x1_0000 + ((slot & 0x3FF) * 32 + py * 8 + px) % 0x8000] as usize
        } else {
            let byte = self.vram[0x1_0000 + ((slot & 0x3FF) * 32 + py * 4 + px / 2) % 0x8000];
            (if px & 1 == 0 { byte & 0xF } else { byte >> 4 }) as usize
        };
        if idx == 0 { None } else { Some(idx) }
    }

    /// First (lowest-index) sprite wins; OBJ-window pixels only mark the mask.
    fn put_obj_pixel(&mut self, sx: usize, color_index: usize, prio: u8, mode: u16) {
        if mode == 2 {
            self.ppu.obj_window[sx] = true;
            return;
        }
        if self.ppu.obj_line[sx].color == TRANSPARENT {
            self.ppu.obj_line[sx] =
                ObjPx { color: self.palette16(color_index), prio, semi: mode == 1 };
        }
    }
}
