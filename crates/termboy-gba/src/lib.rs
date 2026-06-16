//! Game Boy Advance core: ARM7TDMI CPU, coarse bus, bitmap-mode PPU, keypad.
//! GbaCore is the frontend-facing entry point behind termboy_core::Core.

pub mod apu;
pub mod bios;
pub mod bus;
mod color;
pub mod cpu;
pub mod dma;
pub mod eeprom;
pub mod flash;
pub mod keypad;
pub mod ppu;
pub mod save;
pub mod timers;
pub mod timing;

use termboy_core::state::{Reader, StateError, Writer};
use termboy_core::{Buttons, Core, FrameBuffer, Rgb};

use crate::bus::Bus;
use crate::cpu::Cpu;

/// 1232 cycles/line * 228 lines = 59.7275 Hz — the same frame rate as the GB.
pub const CYCLES_PER_FRAME: u64 = 280_896;

/// Master clock in Hz. `CYCLES_PER_FRAME / CLOCK_HZ` = 59.7275 Hz — the same
/// frame period as the GB; the GBA has no double-speed mode.
pub const CLOCK_HZ: u64 = 16_777_216;

pub struct GbaCore {
    cpu: Cpu,
    frame: FrameBuffer,
    rom_id: u64,
    correction: Option<Box<[Rgb; 32768]>>,
}

impl GbaCore {
    pub fn new(rom: Vec<u8>) -> Result<Self, String> {
        if rom.len() < 0xC0 {
            return Err("ROM is too small to contain a GBA cartridge header".into());
        }
        let rom_id = termboy_core::state::rom_id(&rom);
        Ok(Self {
            cpu: Cpu::new(Bus::new(rom)),
            frame: FrameBuffer::new(ppu::WIDTH, ppu::HEIGHT),
            rom_id,
            correction: None,
        })
    }

    /// Total elapsed bus cycles (test/debug hook).
    pub fn cycles(&self) -> u64 {
        self.cpu.bus.cycles
    }

    /// Test/debug: drive a raw byte access to the save region.
    pub fn debug_write8(&mut self, addr: u32, value: u8) {
        self.cpu.bus.write8(addr, value);
    }

    pub fn debug_read8(&mut self, addr: u32) -> u8 {
        self.cpu.bus.read8(addr)
    }

    pub fn debug_write16(&mut self, addr: u32, value: u16) {
        self.cpu.bus.write16(addr, value);
    }

    pub fn debug_read16(&mut self, addr: u32) -> u16 {
        self.cpu.bus.read16(addr)
    }

    /// Program and immediately run a 16-bit immediate DMA3 (test helper).
    pub fn debug_dma3(&mut self, src: u32, dst: u32, count: u16) {
        self.cpu.bus.write32(0x0400_00D4, src);
        self.cpu.bus.write32(0x0400_00D8, dst);
        self.cpu.bus.write16(0x0400_00DC, count);
        self.cpu.bus.write16(0x0400_00DE, 0x8000);
        if let Some(ch) = self.cpu.bus.dma_ready() {
            self.cpu.bus.run_dma(ch);
        }
    }
}

impl Core for GbaCore {
    fn run_frame(&mut self, buttons: Buttons) -> &FrameBuffer {
        self.cpu.bus.set_buttons(buttons);
        self.cpu.bus.ppu.frame_ready = false;
        // The cap mirrors GameBoy::run_frame: never hang the frontend even if
        // the PPU somehow never signals (e.g. a ROM that stops the clock).
        let cap = self.cpu.bus.cycles + CYCLES_PER_FRAME;
        while !self.cpu.bus.ppu.frame_ready && self.cpu.bus.cycles < cap {
            self.cpu.step_system();
        }
        if let Some(lut) = self.correction.as_deref() {
            for (i, &c) in self.cpu.bus.ppu.frame.iter().enumerate() {
                self.frame.pixels[i] = lut[(c & 0x7FFF) as usize];
            }
        } else {
            for (i, &c) in self.cpu.bus.ppu.frame.iter().enumerate() {
                self.frame.pixels[i] = ppu::bgr555_to_rgb(c);
            }
        }
        &self.frame
    }

    fn save_ram(&self) -> Option<Vec<u8>> {
        self.cpu.bus.save.backup()
    }

    fn load_ram(&mut self, data: &[u8]) {
        self.cpu.bus.save.restore(data);
    }

    fn drain_audio(&mut self, out: &mut Vec<(f32, f32)>) {
        out.append(&mut self.cpu.bus.apu.samples);
    }

    fn set_audio_rate(&mut self, hz: u32) {
        self.cpu.bus.apu.set_sample_rate(hz);
    }

    fn set_color_correction(&mut self, on: bool) {
        self.correction = on.then(crate::color::build_lut);
    }

    fn save_state(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.put_bytes(b"TBSS");
        w.put_u16(termboy_core::STATE_VERSION);
        w.put_u8(1); // console 1 = GBA
        w.put_u64(self.rom_id);
        self.cpu.serialize(&mut w);
        w.buf
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), StateError> {
        let mut r = Reader::new(data);
        if r.get_bytes(4)? != b"TBSS" {
            return Err(StateError::BadMagic);
        }
        if r.get_u16()? != termboy_core::STATE_VERSION {
            return Err(StateError::BadVersion);
        }
        if r.get_u8()? != 1 {
            return Err(StateError::WrongConsole);
        }
        if r.get_u64()? != self.rom_id {
            return Err(StateError::WrongRom);
        }
        self.cpu.deserialize(&mut r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color;

    /// ROM whose entry instruction is `b .` (0xEAFFFFFE), so the CPU spins
    /// forever without touching display registers — lets us pin a rendered
    /// backdrop deterministically.
    fn spinning_core() -> GbaCore {
        let mut rom = vec![0u8; 0xC0];
        rom[0..4].copy_from_slice(&0xEAFF_FFFEu32.to_le_bytes());
        GbaCore::new(rom).unwrap()
    }

    #[test]
    fn set_color_correction_toggles_the_lut() {
        let mut core = spinning_core();
        assert!(core.correction.is_none());
        core.set_color_correction(true);
        assert!(core.correction.is_some());
        core.set_color_correction(false);
        assert!(core.correction.is_none());
    }

    #[test]
    fn run_frame_applies_correction_to_the_backdrop() {
        let mut core = spinning_core();
        core.debug_write16(0x0400_0000, 0x0000); // DISPCNT: no forced blank, no layers
        core.debug_write16(0x0500_0000, 0x001F); // backdrop palette[0] = red

        let raw = core.run_frame(Buttons::default()).pixels[0];
        assert_eq!(raw, ppu::bgr555_to_rgb(0x001F)); // raw expand of red

        core.set_color_correction(true);
        let corrected = core.run_frame(Buttons::default()).pixels[0];
        assert_eq!(corrected, color::correct(0x001F));
        assert_ne!(corrected, raw);
    }
}
