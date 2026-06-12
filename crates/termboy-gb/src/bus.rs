//! The bus owns every component and the master clock. `tick()` advances one
//! M-cycle (4 T-cycles); the CPU calls it once per memory access, which is
//! what keeps all components in hardware-true lockstep (design spec §3).

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::joypad::Joypad;
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
    pub cgb: bool,
    wram: Box<[u8; 0x8000]>, // 8 banks; DMG uses the first two
    svbk: u8,
    vbk: u8,
    key1_armed: bool,
    pub double_speed: bool,
    hram: [u8; 0x7F],
    pub timer: Timer,
    pub serial: Serial,
    pub joypad: Joypad,
    pub apu: Apu,
    /// IF (0xFF0F) and IE (0xFFFF)
    pub intf: u8,
    pub inte: u8,
    /// Total elapsed T-cycles since power-on.
    pub cycles: u64,
    dma_src: u16,
    dma_idx: u8, // 0xA0 = idle
    dma_reg: u8,
    hdma_src: u16,
    hdma_dst: u16,
    hdma_len: u8, // remaining 16-byte blocks minus one
    hdma_active: bool,
}

impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        Self {
            cart,
            ppu: Ppu::new(),
            cgb: false,
            wram: Box::new([0; 0x8000]),
            svbk: 0,
            vbk: 0,
            key1_armed: false,
            double_speed: false,
            hram: [0; 0x7F],
            timer: Timer::default(),
            serial: Serial::default(),
            joypad: Joypad::default(),
            apu: Apu::new(),
            intf: 0,
            inte: 0,
            cycles: 0,
            dma_src: 0,
            dma_idx: 0xA0,
            dma_reg: 0xFF,
            hdma_src: 0,
            hdma_dst: 0,
            hdma_len: 0x7F,
            hdma_active: false,
        }
    }

    fn wram_index(&self, addr: u16) -> usize {
        match addr & 0x1FFF {
            // 0xC000-0xCFFF (and its echo): always bank 0
            off @ 0x0000..=0x0FFF => off as usize,
            off => {
                let bank = if self.cgb { (self.svbk & 7).max(1) } else { 1 } as usize;
                bank * 0x1000 + (off as usize - 0x1000)
            }
        }
    }

    /// CGB speed switch: STOP with KEY1 armed toggles double speed.
    pub fn try_speed_switch(&mut self) {
        if self.cgb && self.key1_armed {
            self.double_speed = !self.double_speed;
            self.key1_armed = false;
        }
    }

    /// Advance one M-cycle of CPU time. The timer runs on the CPU clock; the
    /// PPU runs in real time, so it gets half the T-cycles in double speed.
    pub fn tick(&mut self) {
        self.cycles += 4;
        for _ in 0..4 {
            if self.timer.tick() {
                self.intf |= IF_TIMER;
            }
        }
        let ppu_t = if self.double_speed { 2 } else { 4 };
        for _ in 0..ppu_t {
            self.intf |= self.ppu.tick();
        }
        self.apu.tick(ppu_t); // APU runs in real time, like the PPU
        self.cart.tick(ppu_t);
        // OAM DMA: one byte per M-cycle, 160 M-cycles total.
        if self.dma_idx < 0xA0 {
            let value = self.read(self.dma_src + self.dma_idx as u16);
            self.ppu.oam[self.dma_idx as usize] = value;
            self.dma_idx += 1;
        }
        // HBlank DMA: one 16-byte block per hblank entry.
        if self.ppu.hblank_pulse {
            self.ppu.hblank_pulse = false;
            if self.hdma_active {
                self.hdma_copy_block();
                if self.hdma_len == 0xFF {
                    self.hdma_active = false;
                } else {
                    self.hdma_len = self.hdma_len.wrapping_sub(1);
                    if self.hdma_len == 0xFF {
                        self.hdma_active = false;
                        self.hdma_len = 0x7F;
                    }
                }
            }
        }
    }

    /// Copy one 16-byte HDMA block into VRAM (current VBK bank). Instant —
    /// the cycle-stealing cost is a documented simplification.
    fn hdma_copy_block(&mut self) {
        for _ in 0..16 {
            let v = self.read(self.hdma_src);
            let dst = 0x8000 + (self.hdma_dst & 0x1FFF);
            self.ppu.vram[(self.vbk as usize & 1) * 0x2000 + (dst - 0x8000) as usize] = v;
            self.hdma_src = self.hdma_src.wrapping_add(1);
            self.hdma_dst = (self.hdma_dst.wrapping_add(1)) & 0x1FFF;
        }
    }

    pub fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cart.read_rom(addr),
            0x8000..=0x9FFF => self.ppu.vram[(self.vbk as usize & 1) * 0x2000 + (addr - 0x8000) as usize],
            0xA000..=0xBFFF => self.cart.read_ram(addr),
            0xC000..=0xDFFF | 0xE000..=0xFDFF => self.wram[self.wram_index(addr)],
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
            0x8000..=0x9FFF => {
                self.ppu.vram[(self.vbk as usize & 1) * 0x2000 + (addr - 0x8000) as usize] = value
            }
            0xA000..=0xBFFF => self.cart.write_ram(addr, value),
            0xC000..=0xDFFF | 0xE000..=0xFDFF => self.wram[self.wram_index(addr)] = value,
            0xFE00..=0xFE9F => self.ppu.oam[(addr - 0xFE00) as usize] = value,
            0xFEA0..=0xFEFF => {}
            0xFF00..=0xFF7F => self.write_io(addr, value),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = value,
            0xFFFF => self.inte = value,
        }
    }

    fn read_io(&mut self, addr: u16) -> u8 {
        match addr {
            0xFF00 => self.joypad.read(),
            0xFF01 => self.serial.read_sb(),
            0xFF02 => self.serial.read_sc(),
            0xFF04 => self.timer.read_div(),
            0xFF05 => self.timer.tima,
            0xFF06 => self.timer.tma,
            0xFF07 => self.timer.read_tac(),
            0xFF0F => self.intf | 0xE0,
            0xFF10..=0xFF3F => self.apu.read(addr),
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
            0xFF4D if self.cgb => {
                ((self.double_speed as u8) << 7) | self.key1_armed as u8 | 0x7E
            }
            0xFF4F if self.cgb => self.vbk | 0xFE,
            0xFF68 if self.cgb => self.ppu.read_bcps(),
            0xFF69 if self.cgb => self.ppu.read_bcpd(),
            0xFF6A if self.cgb => self.ppu.read_ocps(),
            0xFF6B if self.cgb => self.ppu.read_ocpd(),
            0xFF55 if self.cgb => ((!self.hdma_active as u8) << 7) | self.hdma_len,
            0xFF70 if self.cgb => self.svbk | 0xF8,
            _ => 0xFF,
        }
    }

    fn write_io(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF00 => self.joypad.write(value),
            0xFF01 => self.serial.write_sb(value),
            0xFF02 => self.serial.write_sc(value),
            0xFF04 => self.timer.write_div(),
            0xFF05 => self.timer.write_tima(value),
            0xFF06 => self.timer.tma = value,
            0xFF07 => self.timer.write_tac(value),
            0xFF0F => self.intf = value & 0x1F,
            0xFF10..=0xFF3F => self.apu.write(addr, value),
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
            0xFF4D if self.cgb => self.key1_armed = value & 1 != 0,
            0xFF4F if self.cgb => self.vbk = value & 1,
            0xFF68 if self.cgb => self.ppu.write_bcps(value),
            0xFF69 if self.cgb => self.ppu.write_bcpd(value),
            0xFF6A if self.cgb => self.ppu.write_ocps(value),
            0xFF6B if self.cgb => self.ppu.write_ocpd(value),
            0xFF51 if self.cgb => self.hdma_src = (self.hdma_src & 0x00FF) | ((value as u16) << 8),
            0xFF52 if self.cgb => self.hdma_src = (self.hdma_src & 0xFF00) | (value as u16 & 0xF0),
            0xFF53 if self.cgb => self.hdma_dst = (self.hdma_dst & 0x00FF) | ((value as u16 & 0x1F) << 8),
            0xFF54 if self.cgb => self.hdma_dst = (self.hdma_dst & 0xFF00) | (value as u16 & 0xF0),
            0xFF55 if self.cgb => {
                if value & 0x80 != 0 {
                    // arm HBlank DMA
                    self.hdma_len = value & 0x7F;
                    self.hdma_active = true;
                } else if self.hdma_active {
                    // cancel mid-transfer; remaining length stays readable
                    self.hdma_active = false;
                } else {
                    // general-purpose DMA: copy everything now
                    self.hdma_len = value & 0x7F;
                    for _ in 0..=(value & 0x7F) {
                        self.hdma_copy_block();
                    }
                    self.hdma_len = 0x7F; // reads back 0xFF (complete)
                }
            }
            0xFF70 if self.cgb => self.svbk = value & 7,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::joypad::Joypad;

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

    fn cgb_bus() -> Bus {
        let mut b = bus();
        b.cgb = true;
        b.ppu.cgb = true;
        b
    }

    #[test]
    fn svbk_banks_wram_d000_region() {
        let mut b = cgb_bus();
        b.write(0xFF70, 2);
        b.write(0xD000, 0x22);
        b.write(0xFF70, 3);
        b.write(0xD000, 0x33);
        b.write(0xC000, 0xCC); // bank 0 region unaffected by SVBK
        b.write(0xFF70, 2);
        assert_eq!(b.read(0xD000), 0x22);
        assert_eq!(b.read(0xC000), 0xCC);
        b.write(0xFF70, 0); // 0 selects bank 1
        b.write(0xD000, 0x11);
        b.write(0xFF70, 1);
        assert_eq!(b.read(0xD000), 0x11);
        assert_eq!(b.read(0xFF70), 0xF8 | 1);
    }

    #[test]
    fn vbk_banks_vram() {
        let mut b = cgb_bus();
        b.write(0xFF4F, 0);
        b.write(0x8000, 0xAA);
        b.write(0xFF4F, 1);
        assert_eq!(b.read(0x8000), 0x00); // bank 1 is separate
        b.write(0x8000, 0xBB);
        b.write(0xFF4F, 0);
        assert_eq!(b.read(0x8000), 0xAA);
        assert_eq!(b.read(0xFF4F), 0xFE | 0);
    }

    #[test]
    fn dmg_mode_ignores_banking_registers() {
        let mut b = bus(); // cgb = false
        b.write(0xFF70, 5);
        b.write(0xFF4F, 1);
        assert_eq!(b.read(0xFF70), 0xFF);
        assert_eq!(b.read(0xFF4F), 0xFF);
        b.write(0x8000, 0xAA);
        assert_eq!(b.read(0x8000), 0xAA); // always bank 0
    }

    #[test]
    fn speed_switch_toggles_and_halves_ppu_rate() {
        let mut b = cgb_bus();
        b.write(0xFF4D, 1); // arm
        b.try_speed_switch();
        assert!(b.double_speed);
        assert_eq!(b.read(0xFF4D) & 0x81, 0x80); // doubled, no longer armed
        // one scanline now takes 228 M-cycles instead of 114
        for _ in 0..114 {
            b.tick();
        }
        assert_eq!(b.read(0xFF44), 0);
        for _ in 0..114 {
            b.tick();
        }
        assert_eq!(b.read(0xFF44), 1);
    }

    #[test]
    fn general_hdma_copies_immediately() {
        let mut b = cgb_bus();
        for i in 0..32u16 {
            b.write(0xC000 + i, i as u8 + 1);
        }
        b.write(0xFF51, 0xC0);
        b.write(0xFF52, 0x00);
        b.write(0xFF53, 0x00); // dest 0x8000
        b.write(0xFF54, 0x00);
        b.write(0xFF55, 0x01); // 2 blocks, general
        assert_eq!(b.read(0x8000), 1);
        assert_eq!(b.read(0x801F), 32);
        assert_eq!(b.read(0xFF55), 0xFF); // complete
    }

    #[test]
    fn hblank_hdma_copies_one_block_per_hblank() {
        let mut b = cgb_bus();
        for i in 0..32u16 {
            b.write(0xC000 + i, 0xAB);
        }
        b.write(0xFF51, 0xC0);
        b.write(0xFF52, 0x00);
        b.write(0xFF53, 0x00);
        b.write(0xFF54, 0x00);
        b.write(0xFF55, 0x81); // 2 blocks, hblank mode
        assert_eq!(b.read(0xFF55), 0x01); // active, 2 blocks left
        assert_eq!(b.read(0x8000), 0x00); // nothing copied yet
        // run to the first hblank (dot 252 of line 0 = 63 M-cycles)
        for _ in 0..64 {
            b.tick();
        }
        assert_eq!(b.read(0x8000), 0xAB);
        assert_eq!(b.read(0x8010), 0x00); // second block not yet
        assert_eq!(b.read(0xFF55), 0x00); // active, 1 block left
        // run to the next hblank (one full scanline = 114 M-cycles)
        for _ in 0..114 {
            b.tick();
        }
        assert_eq!(b.read(0x8010), 0xAB);
        assert_eq!(b.read(0xFF55), 0xFF); // done
    }

    #[test]
    fn hblank_hdma_cancel_keeps_remaining_length() {
        let mut b = cgb_bus();
        b.write(0xFF55, 0x85); // 6 blocks, hblank mode
        b.write(0xFF55, 0x00); // cancel
        assert_eq!(b.read(0xFF55), 0x85); // inactive bit set, length preserved
    }

    #[test]
    fn joypad_routes_through_p1() {
        let mut b = bus();
        assert_eq!(b.read(0xFF00), 0xFF); // post-boot: nothing selected, nothing pressed
        b.write(0xFF00, 0x20);
        assert_eq!(b.read(0xFF00), 0xEF); // directions selected, none pressed
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
