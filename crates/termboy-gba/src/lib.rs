//! Game Boy Advance core: ARM7TDMI CPU, coarse bus, bitmap-mode PPU, keypad.
//! GbaCore is the frontend-facing entry point behind termboy_core::Core.

pub mod bios;
pub mod bus;
pub mod cpu;
pub mod keypad;
pub mod ppu;

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
}

impl Core for GbaCore {
    fn run_frame(&mut self, buttons: Buttons) -> &FrameBuffer {
        self.cpu.bus.set_buttons(buttons);
        self.cpu.bus.ppu.frame_ready = false;
        // The cap mirrors GameBoy::run_frame: never hang the frontend even if
        // the PPU somehow never signals (e.g. a ROM that stops the clock).
        let cap = self.cpu.bus.cycles + CYCLES_PER_FRAME;
        while !self.cpu.bus.ppu.frame_ready && self.cpu.bus.cycles < cap {
            self.cpu.step();
            self.cpu.bus.catch_up();
        }
        for (i, &c) in self.cpu.bus.ppu.frame.iter().enumerate() {
            self.frame.pixels[i] = ppu::bgr555_to_rgb(c);
        }
        &self.frame
    }

    fn save_ram(&self) -> Option<Vec<u8>> {
        None // cartridge save types (SRAM/Flash/EEPROM) land in G5
    }

    fn load_ram(&mut self, _data: &[u8]) {}
}
