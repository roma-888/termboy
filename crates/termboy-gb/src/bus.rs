//! The bus owns every component and the master clock. `tick()` advances one
//! M-cycle (4 T-cycles); the CPU calls it once per memory access, which is
//! what keeps all components in hardware-true lockstep (design spec §3).

use crate::cartridge::Cartridge;
use crate::ppu::Ppu;
use crate::serial::Serial;
use crate::timer::Timer;

pub const IF_VBLANK: u8 = 0x01;
pub const IF_STAT: u8 = 0x02;
pub const IF_TIMER: u8 = 0x04;
pub const IF_SERIAL: u8 = 0x08;
pub const IF_JOYPAD: u8 = 0x10;

pub struct Bus {
    pub cart: Cartridge,
    pub ppu: Ppu,
    wram: [u8; 0x2000],
    hram: [u8; 0x7F],
    pub timer: Timer,
    pub serial: Serial,
    /// IF (0xFF0F) and IE (0xFFFF)
    pub intf: u8,
    pub inte: u8,
    /// Total elapsed T-cycles since power-on.
    pub cycles: u64,
    dma_src: u16,
    dma_idx: u8, // 0xA0 = idle
    dma_reg: u8,
}

impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        Self {
            cart,
            ppu: Ppu::new(),
            wram: [0; 0x2000],
            hram: [0; 0x7F],
            timer: Timer::default(),
            serial: Serial::default(),
            intf: 0,
            inte: 0,
            cycles: 0,
            dma_src: 0,
            dma_idx: 0xA0,
            dma_reg: 0xFF,
        }
    }

    /// Advance one M-cycle: timer and PPU per T-cycle, OAM-DMA per M-cycle.
    pub fn tick(&mut self) {
        self.cycles += 4;
        for _ in 0..4 {
            if self.timer.tick() {
                self.intf |= IF_TIMER;
            }
            self.intf |= self.ppu.tick();
        }
        // OAM DMA: one byte per M-cycle, 160 M-cycles total.
        if self.dma_idx < 0xA0 {
            let value = self.read(self.dma_src + self.dma_idx as u16);
            self.ppu.oam[self.dma_idx as usize] = value;
            self.dma_idx += 1;
        }
    }

    pub fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cart.read_rom(addr),
            0x8000..=0x9FFF => self.ppu.vram[(addr - 0x8000) as usize],
            0xA000..=0xBFFF => self.cart.read_ram(addr),
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize],
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize],
            0xFE00..=0xFE9F => self.ppu.oam[(addr - 0xFE00) as usize],
            0xFEA0..=0xFEFF => 0xFF,
            0xFF00..=0xFF7F => self.read_io(addr),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF => self.inte,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x7FFF => self.cart.write_rom(addr, value),
            0x8000..=0x9FFF => self.ppu.vram[(addr - 0x8000) as usize] = value,
            0xA000..=0xBFFF => self.cart.write_ram(addr, value),
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize] = value,
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize] = value,
            0xFE00..=0xFE9F => self.ppu.oam[(addr - 0xFE00) as usize] = value,
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
            0xFF40 => self.ppu.lcdc,
            0xFF41 => self.ppu.read_stat(),
            0xFF42 => self.ppu.scy,
            0xFF43 => self.ppu.scx,
            0xFF44 => self.ppu.ly,
            0xFF45 => self.ppu.lyc,
            0xFF46 => self.dma_reg,
            0xFF47 => self.ppu.bgp,
            0xFF48 => self.ppu.obp0,
            0xFF49 => self.ppu.obp1,
            0xFF4A => self.ppu.wy,
            0xFF4B => self.ppu.wx,
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
            0xFF40 => self.ppu.write_lcdc(value),
            0xFF41 => self.ppu.write_stat(value),
            0xFF42 => self.ppu.scy = value,
            0xFF43 => self.ppu.scx = value,
            0xFF44 => {} // LY is read-only
            0xFF45 => self.ppu.lyc = value,
            0xFF46 => {
                self.dma_reg = value;
                self.dma_src = (value as u16) << 8;
                self.dma_idx = 0;
            }
            0xFF47 => self.ppu.bgp = value,
            0xFF48 => self.ppu.obp0 = value,
            0xFF49 => self.ppu.obp1 = value,
            0xFF4A => self.ppu.wy = value,
            0xFF4B => self.ppu.wx = value,
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
    fn oam_dma_copies_160_bytes_over_160_mcycles() {
        let mut b = bus();
        for i in 0..0xA0u16 {
            b.write(0xC000 + i, i as u8);
        }
        b.write(0xFF46, 0xC0); // start DMA from 0xC000
        assert_eq!(b.read(0xFF46), 0xC0); // register reads back
        for _ in 0..159 {
            b.tick();
        }
        assert_eq!(b.read(0xFE9F), 0x00); // not finished yet
        b.tick();
        assert_eq!(b.read(0xFE00), 0x00);
        assert_eq!(b.read(0xFE9F), 0x9F); // all 160 bytes landed
    }

    #[test]
    fn ly_register_reflects_ppu_state() {
        let mut b = bus();
        assert_eq!(b.read(0xFF44), 0);
        for _ in 0..114 {
            b.tick(); // 456 T-cycles = one scanline
        }
        assert_eq!(b.read(0xFF44), 1);
    }
}
