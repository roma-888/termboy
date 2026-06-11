pub mod bus;
pub mod cartridge;
pub mod cpu;
pub mod serial;
pub mod timer;

use bus::Bus;
use cartridge::{CartError, Cartridge};
use cpu::Cpu;
use termboy_core::{Buttons, Core, FrameBuffer};

/// T-cycles per video frame (154 scanlines x 456 cycles, ~59.73 Hz).
pub const CYCLES_PER_FRAME: u64 = 70224;

pub struct GameBoy {
    cpu: Cpu,
    frame: FrameBuffer,
}

impl GameBoy {
    pub fn new(rom: Vec<u8>) -> Result<Self, CartError> {
        let cart = Cartridge::new(rom)?;
        Ok(Self {
            cpu: Cpu::new(Bus::new(cart)),
            frame: FrameBuffer::new(160, 144),
        })
    }

    /// Everything the game has written to the serial port (Blargg test output).
    pub fn serial_output(&self) -> &[u8] {
        self.cpu.bus.serial.output()
    }
}

impl Core for GameBoy {
    fn run_frame(&mut self, _buttons: Buttons) -> &FrameBuffer {
        // Joypad input lands in Milestone 3; the PPU fills `frame` in Milestone 2.
        let end = self.cpu.bus.cycles + CYCLES_PER_FRAME;
        while self.cpu.bus.cycles < end {
            self.cpu.step();
        }
        &self.frame
    }

    fn save_ram(&self) -> Option<&[u8]> {
        self.cpu.bus.cart.ram()
    }

    fn load_ram(&mut self, data: &[u8]) {
        self.cpu.bus.cart.load_ram(data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_frame_advances_one_frame_of_cycles() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = 0x00;
        // 0x100: JR -2 (spin forever)
        rom[0x100] = 0x18;
        rom[0x101] = 0xFE;
        let mut gb = GameBoy::new(rom).unwrap();
        gb.run_frame(Buttons::default());
        assert!(gb.cpu.bus.cycles >= CYCLES_PER_FRAME);
        assert!(gb.cpu.bus.cycles < CYCLES_PER_FRAME + 16); // overshoot < 1 instruction
    }
}
