//! Game Boy Advance core: ARM7TDMI CPU, coarse bus, bitmap-mode PPU, keypad.
//! GbaCore is the frontend-facing entry point behind termboy_core::Core.

pub mod bios;
pub mod bus;
pub mod cpu;
pub mod dma;
pub mod eeprom;
pub mod flash;
pub mod keypad;
pub mod ppu;
pub mod save;
pub mod timers;

use termboy_core::{Buttons, Core, FrameBuffer};

use crate::bus::Bus;
use crate::cpu::Cpu;

/// 1232 cycles/line * 228 lines = 59.7275 Hz — the same frame rate as the GB.
pub const CYCLES_PER_FRAME: u64 = 280_896;

pub struct GbaCore {
    cpu: Cpu,
    frame: FrameBuffer,
}

impl GbaCore {
    pub fn new(rom: Vec<u8>) -> Result<Self, String> {
        if rom.len() < 0xC0 {
            return Err("ROM is too small to contain a GBA cartridge header".into());
        }
        Ok(Self {
            cpu: Cpu::new(Bus::new(rom)),
            frame: FrameBuffer::new(ppu::WIDTH, ppu::HEIGHT),
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
        for (i, &c) in self.cpu.bus.ppu.frame.iter().enumerate() {
            self.frame.pixels[i] = ppu::bgr555_to_rgb(c);
        }
        &self.frame
    }

    fn save_ram(&self) -> Option<Vec<u8>> {
        self.cpu.bus.save.backup()
    }

    fn load_ram(&mut self, data: &[u8]) {
        self.cpu.bus.save.restore(data);
    }
}
