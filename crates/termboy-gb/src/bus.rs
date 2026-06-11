//! The bus owns every component and the master clock. `tick()` advances one
//! M-cycle (4 T-cycles); the CPU calls it once per memory access, which is
//! what keeps all components in hardware-true lockstep (design spec §3).

use crate::cartridge::Cartridge;
use crate::serial::Serial;
use crate::timer::Timer;

pub const IF_VBLANK: u8 = 0x01;
pub const IF_STAT: u8 = 0x02;
pub const IF_TIMER: u8 = 0x04;
pub const IF_SERIAL: u8 = 0x08;
pub const IF_JOYPAD: u8 = 0x10;

pub struct Bus {
    pub cart: Cartridge,
    vram: [u8; 0x2000],
    wram: [u8; 0x2000],
    oam: [u8; 0xA0],
    hram: [u8; 0x7F],
    pub timer: Timer,
    pub serial: Serial,
    /// IF (0xFF0F) and IE (0xFFFF)
    pub intf: u8,
    pub inte: u8,
    /// Total elapsed T-cycles since power-on.
    pub cycles: u64,
}

impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        Self {
            cart,
            vram: [0; 0x2000],
            wram: [0; 0x2000],
            oam: [0; 0xA0],
            hram: [0; 0x7F],
            timer: Timer::default(),
            serial: Serial::default(),
            intf: 0,
            inte: 0,
            cycles: 0,
        }
    }

    /// Advance one M-cycle. PPU and OAM-DMA join this loop in Milestone 2.
    pub fn tick(&mut self) {
        self.cycles += 4;
        for _ in 0..4 {
            if self.timer.tick() {
                self.intf |= IF_TIMER;
            }
        }
    }

    pub fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cart.read_rom(addr),
            0x8000..=0x9FFF => self.vram[(addr - 0x8000) as usize],
            0xA000..=0xBFFF => self.cart.read_ram(addr),
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize],
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize],
            0xFE00..=0xFE9F => self.oam[(addr - 0xFE00) as usize],
            0xFEA0..=0xFEFF => 0xFF,
            0xFF00..=0xFF7F => self.read_io(addr),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF => self.inte,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x7FFF => self.cart.write_rom(addr, value),
            0x8000..=0x9FFF => self.vram[(addr - 0x8000) as usize] = value,
            0xA000..=0xBFFF => self.cart.write_ram(addr, value),
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize] = value,
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize] = value,
            0xFE00..=0xFE9F => self.oam[(addr - 0xFE00) as usize] = value,
            0xFEA0..=0xFEFF => {}
            0xFF00..=0xFF7F => self.write_io(addr, value),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = value,
            0xFFFF => self.inte = value,
        }
    }

    fn read_io(&mut self, addr: u16) -> u8 {
        match addr {
            0xFF01 => self.serial.read_sb(),
            0xFF02 => self.serial.read_sc(),
            0xFF04 => self.timer.read_div(),
            0xFF05 => self.timer.tima,
            0xFF06 => self.timer.tma,
            0xFF07 => self.timer.read_tac(),
            0xFF0F => self.intf | 0xE0,
            // LY stub until the PPU exists (Milestone 2): report line 144 so
            // ROMs polling for vblank make progress.
            0xFF44 => 0x90,
            _ => 0xFF,
        }
    }

    fn write_io(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF01 => self.serial.write_sb(value),
            0xFF02 => self.serial.write_sc(value),
            0xFF04 => self.timer.write_div(),
            0xFF05 => self.timer.write_tima(value),
            0xFF06 => self.timer.tma = value,
            0xFF07 => self.timer.write_tac(value),
            0xFF0F => self.intf = value & 0x1F,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;

    fn bus() -> Bus {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = 0x00;
        rom[0x1000] = 0x5A;
        Bus::new(Cartridge::new(rom).unwrap())
    }

    #[test]
    fn memory_map_routes_correctly() {
        let mut b = bus();
        assert_eq!(b.read(0x1000), 0x5A); // cart ROM
        b.write(0xC123, 0x42); // WRAM
        assert_eq!(b.read(0xC123), 0x42);
        assert_eq!(b.read(0xE123), 0x42); // echo RAM mirrors WRAM
        b.write(0xFF80, 0x99); // HRAM
        assert_eq!(b.read(0xFF80), 0x99);
        b.write(0xFFFF, 0x1F); // IE
        assert_eq!(b.read(0xFFFF), 0x1F);
        assert_eq!(b.read(0xFEA0), 0xFF); // unusable region
    }

    #[test]
    fn tick_advances_4_tcycles_and_drives_timer() {
        let mut b = bus();
        for _ in 0..64 {
            b.tick();
        }
        assert_eq!(b.cycles, 256);
        assert_eq!(b.read(0xFF04), 1); // DIV advanced once
    }

    #[test]
    fn timer_interrupt_sets_if_bit_2() {
        let mut b = bus();
        b.write(0xFF07, 0b101); // enable, fastest rate
        b.write(0xFF06, 0x00);
        b.write(0xFF05, 0xFF);
        for _ in 0..16 {
            b.tick(); // overflow + 4-cycle reload happen within these 64 T-cycles
        }
        assert_eq!(b.read(0xFF0F) & 0x04, 0x04);
    }

    #[test]
    fn ly_stub_reports_vblank_line() {
        let mut b = bus();
        assert_eq!(b.read(0xFF44), 0x90);
    }
}
