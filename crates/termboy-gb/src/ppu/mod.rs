//! PPU mode state machine and registers. Rendering itself lives in
//! `render.rs` — the module behind the FIFO upgrade seam (design spec §3).

mod render;
#[cfg(test)]
mod tests;

use crate::bus::{IF_STAT, IF_VBLANK};

pub const LCDC_BG_ON: u8 = 0x01;
pub const LCDC_OBJ_ON: u8 = 0x02;
pub const LCDC_OBJ_TALL: u8 = 0x04;
pub const LCDC_BG_MAP: u8 = 0x08;
pub const LCDC_TILE_DATA: u8 = 0x10;
pub const LCDC_WIN_ON: u8 = 0x20;
pub const LCDC_WIN_MAP: u8 = 0x40;
pub const LCDC_LCD_ON: u8 = 0x80;

pub struct Ppu {
    /// Two 8 KB banks; DMG uses only bank 0. Bank 1 holds CGB BG attributes.
    pub vram: Box<[u8; 0x4000]>,
    pub oam: [u8; 0xA0],
    pub cgb: bool,
    pub lcdc: u8,
    stat: u8, // enable bits 3-6 only; mode/LYC computed on read
    pub scy: u8,
    pub scx: u8,
    pub ly: u8,
    pub lyc: u8,
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub wy: u8,
    pub wx: u8,
    dot: u16,        // 0..456 within the current line
    window_line: u8, // window's internal line counter
    wy_hit: bool,    // WY==LY matched somewhere this frame
    stat_line: bool, // previous STAT interrupt line (edge detector)
    /// 160x144 shade indices (0..=3). GameBoy applies the palette. (DMG mode)
    pub frame: [u8; 160 * 144],
    /// 160x144 RGB555 pixels. (CGB mode)
    pub frame_rgb: Vec<u16>,
    pub frame_ready: bool,
    /// Set on mode-0 (hblank) entry; consumed by the bus for HBlank DMA.
    pub hblank_pulse: bool,
    // CGB palette RAM: 8 palettes x 4 colors x 2 bytes, BG and OBJ
    pub(crate) bg_pal: [u8; 64],
    pub(crate) obj_pal: [u8; 64],
    bcps: u8,
    ocps: u8,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            vram: Box::new([0; 0x4000]),
            oam: [0; 0xA0],
            cgb: false,
            // DMG post-boot I/O state (the other GBC seam, spec §3)
            lcdc: 0x91,
            stat: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            wy: 0,
            wx: 0,
            dot: 0,
            window_line: 0,
            wy_hit: false,
            stat_line: false,
            frame: [0; 160 * 144],
            frame_rgb: vec![0; 160 * 144],
            frame_ready: false,
            hblank_pulse: false,
            bg_pal: [0xFF; 64],
            obj_pal: [0xFF; 64],
            bcps: 0,
            ocps: 0,
        }
    }

    /// Advance one T-cycle. Returns IF bits to raise.
    pub fn tick(&mut self) -> u8 {
        if self.lcdc & LCDC_LCD_ON == 0 {
            return 0;
        }
        let mut irq = 0;
        self.dot += 1;
        if self.dot == 456 {
            self.dot = 0;
            self.ly += 1;
            if self.ly == 154 {
                self.ly = 0;
                self.window_line = 0;
                self.wy_hit = false;
            }
            if self.ly == self.wy {
                self.wy_hit = true;
            }
            if self.ly == 144 {
                irq |= IF_VBLANK;
                self.frame_ready = true;
            }
        }
        if self.dot == 252 && self.ly < 144 {
            self.render_line(); // entering hblank: line is complete on real HW
            self.hblank_pulse = true;
        }
        irq | self.update_stat()
    }

    fn mode(&self) -> u8 {
        if self.lcdc & LCDC_LCD_ON == 0 {
            0
        } else if self.ly >= 144 {
            1
        } else if self.dot < 80 {
            2
        } else if self.dot < 252 {
            3
        } else {
            0
        }
    }

    /// STAT IRQ fires on the rising edge of the combined condition line.
    fn update_stat(&mut self) -> u8 {
        let mode = self.mode();
        let line = (self.stat & 0x08 != 0 && mode == 0)
            || (self.stat & 0x10 != 0 && mode == 1)
            || (self.stat & 0x20 != 0 && mode == 2)
            || (self.stat & 0x40 != 0 && self.ly == self.lyc);
        let irq = if line && !self.stat_line { IF_STAT } else { 0 };
        self.stat_line = line;
        irq
    }

    pub fn read_stat(&self) -> u8 {
        0x80 | self.stat | (((self.ly == self.lyc) as u8) << 2) | self.mode()
    }

    pub fn write_stat(&mut self, value: u8) {
        self.stat = value & 0x78; // only the enable bits are writable
    }

    pub fn read_bcps(&self) -> u8 {
        self.bcps | 0x40
    }
    pub fn write_bcps(&mut self, value: u8) {
        self.bcps = value & 0xBF;
    }
    pub fn read_bcpd(&self) -> u8 {
        self.bg_pal[(self.bcps & 0x3F) as usize]
    }
    /// Data writes auto-increment the index when BCPS bit 7 is set.
    pub fn write_bcpd(&mut self, value: u8) {
        self.bg_pal[(self.bcps & 0x3F) as usize] = value;
        if self.bcps & 0x80 != 0 {
            self.bcps = 0x80 | ((self.bcps + 1) & 0x3F);
        }
    }
    pub fn read_ocps(&self) -> u8 {
        self.ocps | 0x40
    }
    pub fn write_ocps(&mut self, value: u8) {
        self.ocps = value & 0xBF;
    }
    pub fn read_ocpd(&self) -> u8 {
        self.obj_pal[(self.ocps & 0x3F) as usize]
    }
    pub fn write_ocpd(&mut self, value: u8) {
        self.obj_pal[(self.ocps & 0x3F) as usize] = value;
        if self.ocps & 0x80 != 0 {
            self.ocps = 0x80 | ((self.ocps + 1) & 0x3F);
        }
    }

    pub fn write_lcdc(&mut self, value: u8) {
        let was_on = self.lcdc & LCDC_LCD_ON != 0;
        self.lcdc = value;
        if was_on && value & LCDC_LCD_ON == 0 {
            // Turning the LCD off resets the whole frame state.
            self.ly = 0;
            self.dot = 0;
            self.window_line = 0;
            self.wy_hit = false;
        }
    }
}
