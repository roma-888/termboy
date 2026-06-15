//! Scanline compositor. Three stages per line: (1) layer producers fill
//! bg_line[0..4] and obj_line (bg.rs, obj.rs, and the bitmap arms below),
//! (2) the window mask gates layers per pixel, (3) compose picks the top
//! two layers by priority and applies color effects.

mod bg;
mod obj;
#[cfg(test)]
mod tests;

use termboy_core::Rgb;
use termboy_core::state::{Reader, StateError, Writer};

use crate::bus::Bus;

pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 160;

/// Bit 15 is unused by BGR555 — it marks "no pixel from this layer".
pub(crate) const TRANSPARENT: u16 = 0x8000;

#[derive(Clone, Copy)]
pub(crate) struct ObjPx {
    pub color: u16, // TRANSPARENT if none
    pub prio: u8,
    pub semi: bool,
}

pub(crate) const NO_OBJ: ObjPx = ObjPx { color: TRANSPARENT, prio: 4, semi: false };

pub struct Ppu {
    /// BGR555 per pixel, row-major 240x160.
    pub frame: Box<[u16; WIDTH * HEIGHT]>,
    /// Set when scanline 159 completes; consumed by GbaCore::run_frame.
    pub frame_ready: bool,
    // -- per-scanline scratch --
    pub(crate) bg_line: [[u16; WIDTH]; 4],
    pub(crate) obj_line: [ObjPx; WIDTH],
    pub(crate) obj_window: [bool; WIDTH],
    /// Affine BG2/BG3 internal reference point (current position, 8-bit
    /// fraction). Latched from BG2X/Y at frame start and on register write.
    pub(crate) bg_ref: [(i32, i32); 2],
    pub(crate) bg_ref_dirty: [bool; 2],
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            frame: Box::new([0; WIDTH * HEIGHT]),
            frame_ready: false,
            bg_line: [[TRANSPARENT; WIDTH]; 4],
            obj_line: [NO_OBJ; WIDTH],
            obj_window: [false; WIDTH],
            bg_ref: [(0, 0); 2],
            bg_ref_dirty: [true, true],
        }
    }

    /// Only the latched affine state persists; frame/scratch is regenerated.
    pub(crate) fn serialize(&self, w: &mut Writer) {
        for &(x, y) in &self.bg_ref {
            w.put_u32(x as u32);
            w.put_u32(y as u32);
        }
        for &d in &self.bg_ref_dirty {
            w.put_bool(d);
        }
    }

    pub(crate) fn deserialize(&mut self, r: &mut Reader) -> Result<(), StateError> {
        for bg in &mut self.bg_ref {
            bg.0 = r.get_u32()? as i32;
            bg.1 = r.get_u32()? as i32;
        }
        for d in &mut self.bg_ref_dirty {
            *d = r.get_bool()?;
        }
        Ok(())
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
    /// Render one completed scanline — driven by Bus::catch_up's hblank event.
    pub(crate) fn render_scanline(&mut self, line: usize) {
        let dispcnt = self.io16(0x000);
        if dispcnt & 0x80 != 0 {
            // forced blank: the LCD shows white
            self.ppu.frame[line * WIDTH..(line + 1) * WIDTH].fill(0x7FFF);
            return;
        }

        // Stage 1: layer producers
        self.ppu.bg_line = [[TRANSPARENT; WIDTH]; 4];
        self.ppu.obj_line = [NO_OBJ; WIDTH];
        self.ppu.obj_window = [false; WIDTH];
        let mode = dispcnt & 7;
        let bg_on = |i: usize| dispcnt & (1 << (8 + i)) != 0;
        match mode {
            0 => {
                for i in 0..4 {
                    if bg_on(i) {
                        self.render_text_bg(i, line);
                    }
                }
            }
            1 => {
                if bg_on(0) {
                    self.render_text_bg(0, line);
                }
                if bg_on(1) {
                    self.render_text_bg(1, line);
                }
                self.render_affine_bg(2, line, bg_on(2));
            }
            2 => {
                self.render_affine_bg(2, line, bg_on(2));
                self.render_affine_bg(3, line, bg_on(3));
            }
            3 => {
                if bg_on(2) {
                    for x in 0..WIDTH {
                        let i = (line * WIDTH + x) * 2;
                        self.ppu.bg_line[2][x] =
                            u16::from_le_bytes([self.vram[i], self.vram[i + 1]]);
                    }
                }
            }
            4 => {
                if bg_on(2) {
                    let page = if dispcnt & (1 << 4) != 0 { 0xA000 } else { 0 };
                    for x in 0..WIDTH {
                        let idx = self.vram[page + line * WIDTH + x] as usize;
                        if idx != 0 {
                            self.ppu.bg_line[2][x] = self.palette16(idx);
                        }
                    }
                }
            }
            _ => {
                if bg_on(2) && line < 128 {
                    let page = if dispcnt & (1 << 4) != 0 { 0xA000 } else { 0 };
                    for x in 0..160 {
                        let i = page + (line * 160 + x) * 2;
                        self.ppu.bg_line[2][x] =
                            u16::from_le_bytes([self.vram[i], self.vram[i + 1]]);
                    }
                }
            }
        }
        if dispcnt & (1 << 12) != 0 {
            self.render_objects(line, dispcnt);
        }

        // Stages 2+3
        self.compose(line, dispcnt);
    }

    fn compose(&mut self, line: usize, dispcnt: u16) {
        let backdrop = self.palette16(0);
        let bg_prio = [
            (self.io16(0x008) & 3) as u8,
            (self.io16(0x00A) & 3) as u8,
            (self.io16(0x00C) & 3) as u8,
            (self.io16(0x00E) & 3) as u8,
        ];
        let bldcnt = self.io16(0x050);
        let bldalpha = self.io16(0x052);
        let bldy = (self.io16(0x054) & 0x1F).min(16) as u32;
        let eva = (bldalpha & 0x1F).min(16) as u32;
        let evb = ((bldalpha >> 8) & 0x1F).min(16) as u32;

        for x in 0..WIDTH {
            let win = self.window_mask(x, line, dispcnt);
            // top two visible layers; layer ids: 0-3 BG, 4 OBJ, 5 backdrop
            let mut top = (backdrop, 5u8);
            let mut second = (backdrop, 5u8);
            let mut found = 0u8;
            let op = self.ppu.obj_line[x];
            'scan: for p in 0..4u8 {
                if win & 0x10 != 0 && op.color != TRANSPARENT && op.prio == p {
                    if found == 0 {
                        top = (op.color, 4);
                        found = 1;
                    } else {
                        second = (op.color, 4);
                        break 'scan;
                    }
                }
                for bg in 0..4usize {
                    if bg_prio[bg] != p || win & (1 << bg) == 0 {
                        continue;
                    }
                    let c = self.ppu.bg_line[bg][x];
                    if c == TRANSPARENT {
                        continue;
                    }
                    if found == 0 {
                        top = (c, bg as u8);
                        found = 1;
                    } else {
                        second = (c, bg as u8);
                        break 'scan;
                    }
                }
            }

            // Stage 3: color special effects
            let first_target = bldcnt & (1 << top.1) != 0;
            let second_target = bldcnt & (1 << (8 + second.1)) != 0;
            let special_ok = win & 0x20 != 0;
            let mut color = top.0;
            if top.1 == 4 && op.semi && second_target && special_ok {
                color = alpha_blend(top.0, second.0, eva, evb);
            } else if special_ok && first_target {
                match (bldcnt >> 6) & 3 {
                    1 if second_target => color = alpha_blend(top.0, second.0, eva, evb),
                    2 => color = brighten(top.0, bldy),
                    3 => color = darken(top.0, bldy),
                    _ => {}
                }
            }
            self.ppu.frame[line * WIDTH + x] = color;
        }
    }

    /// Per-pixel 6-bit enable mask: bits 0-3 BGs, 4 OBJ, 5 color effects.
    /// With no window enabled everything is allowed.
    fn window_mask(&self, x: usize, line: usize, dispcnt: u16) -> u8 {
        if dispcnt & 0xE000 == 0 {
            return 0x3F;
        }
        for w in 0..2usize {
            if dispcnt & (1 << (13 + w)) != 0
                && win_contains(self.io16(0x040 + w * 2), x)
                && win_contains(self.io16(0x044 + w * 2), line)
            {
                return (self.io16(0x048) >> (8 * w)) as u8 & 0x3F;
            }
        }
        if dispcnt & (1 << 15) != 0 && self.ppu.obj_window[x] {
            return (self.io16(0x04A) >> 8) as u8 & 0x3F;
        }
        self.io16(0x04A) as u8 & 0x3F
    }
}

/// WINnH/WINnV: high byte = start, low byte = end+1; start > end wraps.
fn win_contains(reg: u16, p: usize) -> bool {
    let lo = (reg >> 8) as usize;
    let hi = (reg & 0xFF) as usize;
    if lo <= hi { (lo..hi).contains(&p) } else { p >= lo || p < hi }
}

fn alpha_blend(a: u16, b: u16, eva: u32, evb: u32) -> u16 {
    let mut out = 0u16;
    for shift in [0u16, 5, 10] {
        let ca = ((a >> shift) & 31) as u32;
        let cb = ((b >> shift) & 31) as u32;
        out |= (((ca * eva + cb * evb) / 16).min(31) as u16) << shift;
    }
    out
}

fn brighten(c: u16, evy: u32) -> u16 {
    let mut out = 0u16;
    for shift in [0u16, 5, 10] {
        let ch = ((c >> shift) & 31) as u32;
        out |= ((ch + (31 - ch) * evy / 16) as u16) << shift;
    }
    out
}

fn darken(c: u16, evy: u32) -> u16 {
    let mut out = 0u16;
    for shift in [0u16, 5, 10] {
        let ch = ((c >> shift) & 31) as u32;
        out |= ((ch - ch * evy / 16) as u16) << shift;
    }
    out
}
