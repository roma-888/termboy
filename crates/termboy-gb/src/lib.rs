pub mod bus;
pub mod cartridge;
pub mod joypad;
mod mbc1;
mod mbc3;
mod mbc5;
pub mod cpu;
pub mod ppu;
pub mod serial;
pub mod timer;

use bus::Bus;
use cartridge::{CartError, Cartridge};
use cpu::Cpu;
use termboy_core::{Buttons, Core, FrameBuffer, Rgb};

/// Classic DMG green ramp, lightest to darkest.
pub const DMG_GREEN: [Rgb; 4] = [
    Rgb(0xE0, 0xF8, 0xD0),
    Rgb(0x88, 0xC0, 0x70),
    Rgb(0x34, 0x68, 0x56),
    Rgb(0x08, 0x18, 0x20),
];

/// Expand 5-bit channels to 8 bits: (v << 3) | (v >> 2).
pub fn rgb555_to_rgb888(c: u16) -> Rgb {
    let e = |v: u16| ((v << 3) | (v >> 2)) as u8;
    Rgb(e(c & 31), e((c >> 5) & 31), e((c >> 10) & 31))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// T-cycles per video frame (154 scanlines x 456 cycles, ~59.73 Hz).
pub const CYCLES_PER_FRAME: u64 = 70224;

pub struct GameBoy {
    cpu: Cpu,
    frame: FrameBuffer,
    palette: [Rgb; 4],
}

impl GameBoy {
    pub fn new(rom: Vec<u8>) -> Result<Self, CartError> {
        let cart = Cartridge::new(rom)?;
        let cgb = cart.cgb();
        let mut cpu = Cpu::new(Bus::new(cart));
        if cgb {
            // CGB post-boot register state — A=0x11 is how games detect color.
            cpu.regs.set_af(0x1180);
            cpu.regs.set_bc(0x0000);
            cpu.regs.set_de(0xFF56);
            cpu.regs.set_hl(0x000D);
            cpu.bus.cgb = true;
            cpu.bus.ppu.cgb = true;
        }
        Ok(Self {
            cpu,
            frame: FrameBuffer::new(160, 144),
            palette: DMG_GREEN,
        })
    }

    /// Everything the game has written to the serial port (Blargg test output).
    pub fn serial_output(&self) -> &[u8] {
        self.cpu.bus.serial.output()
    }

    pub fn set_palette(&mut self, palette: [Rgb; 4]) {
        self.palette = palette;
    }

    /// Raw shade indices (0-3) of the last completed frame — used by tests.
    pub fn frame_shades(&self) -> &[u8] {
        &self.cpu.bus.ppu.frame
    }

    /// Raw RGB555 pixels of the last completed CGB frame — used by tests.
    pub fn frame_rgb555(&self) -> &[u16] {
        &self.cpu.bus.ppu.frame_rgb
    }

    pub fn cycles(&self) -> u64 {
        self.cpu.bus.cycles
    }
}

impl Core for GameBoy {
    fn run_frame(&mut self, buttons: Buttons) -> &FrameBuffer {
        if self.cpu.bus.joypad.set_buttons(buttons) {
            self.cpu.bus.intf |= bus::IF_JOYPAD;
        }
        self.cpu.bus.ppu.frame_ready = false;
        let cap = self.cpu.bus.cycles + CYCLES_PER_FRAME;
        while !self.cpu.bus.ppu.frame_ready && self.cpu.bus.cycles < cap {
            self.cpu.step();
        }
        if self.cpu.bus.cgb {
            for (i, &c) in self.cpu.bus.ppu.frame_rgb.iter().enumerate() {
                self.frame.pixels[i] = rgb555_to_rgb888(c);
            }
        } else {
            for (i, &shade) in self.cpu.bus.ppu.frame.iter().enumerate() {
                self.frame.pixels[i] = self.palette[shade as usize];
            }
        }
        &self.frame
    }

    fn save_ram(&self) -> Option<Vec<u8>> {
        self.cpu.bus.cart.save(unix_now())
    }

    fn load_ram(&mut self, data: &[u8]) {
        self.cpu.bus.cart.load(data, unix_now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spin_rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = 0x00;
        rom[0x100] = 0x18; // JR -2: spin forever
        rom[0x101] = 0xFE;
        rom
    }

    #[test]
    fn frames_are_70224_cycles_apart() {
        let mut gb = GameBoy::new(spin_rom()).unwrap();
        gb.run_frame(Buttons::default());
        let c1 = gb.cycles();
        gb.run_frame(Buttons::default());
        assert_eq!(gb.cycles() - c1, CYCLES_PER_FRAME);
    }

    #[test]
    fn rgb555_expands_full_range() {
        assert_eq!(rgb555_to_rgb888(0x7FFF), Rgb(255, 255, 255));
        assert_eq!(rgb555_to_rgb888(0x0000), Rgb(0, 0, 0));
        assert_eq!(rgb555_to_rgb888(0x001F), Rgb(255, 0, 0)); // red is low bits
    }

    #[test]
    fn cgb_header_boots_with_a_11() {
        let mut rom = spin_rom();
        rom[0x143] = 0x80;
        let gb = GameBoy::new(rom).unwrap();
        assert_eq!(gb.cpu.regs.a, 0x11);
        assert!(gb.cpu.bus.cgb);
    }

    #[test]
    fn buttons_reach_p1_and_raise_irq() {
        let mut gb = GameBoy::new(spin_rom()).unwrap();
        gb.run_frame(Buttons::A);
        // IF bit 4 was raised by the new press (spin rom never clears IF)
        assert_eq!(gb.cpu.bus.intf & bus::IF_JOYPAD, bus::IF_JOYPAD);
    }

    #[test]
    fn framebuffer_uses_palette() {
        let mut gb = GameBoy::new(spin_rom()).unwrap();
        let fb = gb.run_frame(Buttons::default());
        // BGP post-boot = 0xFC maps color 0 -> shade 0 -> lightest green
        assert_eq!(fb.pixels[0], DMG_GREEN[0]);
    }
}
