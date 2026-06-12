//! GBA memory map at coarse timing: every access is one cycle (real
//! waitstates land in G7). The CPU is the only bus master until DMA (G4).

pub(crate) const CYCLES_PER_LINE: u64 = 1232;
pub(crate) const LINES: u64 = 228;
const VBLANK_START: u64 = 160;

pub struct Bus {
    pub rom: Vec<u8>,
    ewram: Box<[u8; 0x4_0000]>,
    iwram: Box<[u8; 0x8000]>,
    pub(crate) io: Box<[u8; 0x400]>,
    pub palette: Box<[u8; 0x400]>,
    pub vram: Box<[u8; 0x1_8000]>,
    pub oam: Box<[u8; 0x400]>,
    sram: Box<[u8; 0x1_0000]>,
    /// Frontend input, pre-encoded as KEYINPUT bits (active low).
    keyinput: u16,
    pub ppu: crate::ppu::Ppu,
    /// Total elapsed (coarse) cycles since power-on.
    pub cycles: u64,
}

impl Bus {
    pub fn new(rom: Vec<u8>) -> Self {
        Self {
            rom,
            ewram: Box::new([0; 0x4_0000]),
            iwram: Box::new([0; 0x8000]),
            io: Box::new([0; 0x400]),
            palette: Box::new([0; 0x400]),
            vram: Box::new([0; 0x1_8000]),
            oam: Box::new([0; 0x400]),
            sram: Box::new([0xFF; 0x1_0000]),
            keyinput: 0x03FF,
            ppu: crate::ppu::Ppu::new(),
            cycles: 0,
        }
    }

    /// Frontend input, pre-encoded as KEYINPUT bits. KEYCNT IRQs land in G4.
    pub fn set_buttons(&mut self, buttons: termboy_core::Buttons) {
        self.keyinput = crate::keypad::keyinput(buttons);
    }

    /// One internal (non-memory) cycle.
    pub fn idle(&mut self) {
        self.cycles += 1;
    }

    fn line(&self) -> u64 {
        (self.cycles / CYCLES_PER_LINE) % LINES
    }

    /// VRAM is 96K arranged 64K+32K; the 32K half is mirrored to fill each
    /// 128K window, and the window repeats through the 16MB page.
    fn vram_index(addr: u32) -> usize {
        let i = (addr as usize) & 0x1_FFFF;
        if i >= 0x1_8000 { i - 0x8000 } else { i }
    }

    pub fn read8(&mut self, addr: u32) -> u8 {
        self.cycles += 1;
        self.read8_raw(addr)
    }

    fn read8_raw(&self, addr: u32) -> u8 {
        match addr >> 24 {
            // BIOS region: SWIs are HLE'd, so nothing executes from here.
            // Seam for loading a real bios.bin later (design spec §4).
            0x00 => 0,
            0x02 => self.ewram[(addr as usize) & 0x3_FFFF],
            0x03 => self.iwram[(addr as usize) & 0x7FFF],
            0x04 => self.read_io(addr),
            0x05 => self.palette[(addr as usize) & 0x3FF],
            0x06 => self.vram[Self::vram_index(addr)],
            0x07 => self.oam[(addr as usize) & 0x3FF],
            0x08..=0x0D => {
                let i = (addr as usize) & 0x1FF_FFFF;
                // Past the ROM's end the gamepak bus floats: reads return the
                // address bus, i.e. (addr/2) as a stream of halfwords.
                self.rom.get(i).copied().unwrap_or(((addr >> 1) >> ((addr & 1) * 8)) as u8)
            }
            0x0E | 0x0F => self.sram[(addr as usize) & 0xFFFF],
            _ => 0,
        }
    }

    pub fn read16(&mut self, addr: u32) -> u16 {
        self.cycles += 1;
        let addr = addr & !1;
        u16::from_le_bytes([self.read8_raw(addr), self.read8_raw(addr + 1)])
    }

    pub fn read32(&mut self, addr: u32) -> u32 {
        self.cycles += 1;
        let addr = addr & !3;
        u32::from_le_bytes([
            self.read8_raw(addr),
            self.read8_raw(addr + 1),
            self.read8_raw(addr + 2),
            self.read8_raw(addr + 3),
        ])
    }

    pub fn write8(&mut self, addr: u32, value: u8) {
        self.cycles += 1;
        match addr >> 24 {
            // 16-bit video memories: byte stores write both halves...
            0x05 | 0x06 => {
                self.write8_raw(addr & !1, value);
                self.write8_raw(addr | 1, value);
            }
            // ...and byte stores to OAM do nothing at all.
            0x07 => {}
            _ => self.write8_raw(addr, value),
        }
    }

    fn write8_raw(&mut self, addr: u32, value: u8) {
        match addr >> 24 {
            0x02 => self.ewram[(addr as usize) & 0x3_FFFF] = value,
            0x03 => self.iwram[(addr as usize) & 0x7FFF] = value,
            0x04 => self.write_io(addr, value),
            0x05 => self.palette[(addr as usize) & 0x3FF] = value,
            0x06 => self.vram[Self::vram_index(addr)] = value,
            0x07 => self.oam[(addr as usize) & 0x3FF] = value,
            0x0E | 0x0F => self.sram[(addr as usize) & 0xFFFF] = value,
            _ => {} // BIOS and ROM are read-only; unmapped writes vanish
        }
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        self.cycles += 1;
        let addr = addr & !1;
        let [a, b] = value.to_le_bytes();
        self.write8_raw(addr, a);
        self.write8_raw(addr + 1, b);
    }

    pub fn write32(&mut self, addr: u32, value: u32) {
        self.cycles += 1;
        let addr = addr & !3;
        let [a, b, c, d] = value.to_le_bytes();
        self.write8_raw(addr, a);
        self.write8_raw(addr + 1, b);
        self.write8_raw(addr + 2, c);
        self.write8_raw(addr + 3, d);
    }

    /// Raise IF bits the way hardware peripherals do (G4 wires real sources).
    pub fn raise_irq(&mut self, bits: u16) {
        let cur = u16::from_le_bytes([self.io[0x202], self.io[0x203]]);
        let [a, b] = (cur | bits).to_le_bytes();
        self.io[0x202] = a;
        self.io[0x203] = b;
    }

    fn read_io(&self, addr: u32) -> u8 {
        let reg = (addr as usize) & 0x3FF;
        match reg {
            // DISPSTAT low byte: vblank/hblank/vcount-match computed from the
            // cycle counter (stub until the real PPU lands in G2)
            0x004 => {
                let line = self.line();
                let mut v = self.io[reg] & !0x07;
                if (VBLANK_START..LINES - 1).contains(&line) {
                    v |= 1;
                }
                if self.cycles % CYCLES_PER_LINE >= 1006 {
                    v |= 2;
                }
                if line == self.io[0x005] as u64 {
                    v |= 4;
                }
                v
            }
            0x006 => self.line() as u8, // VCOUNT
            0x007 => 0,
            0x130 => self.keyinput as u8,
            0x131 => (self.keyinput >> 8) as u8,
            _ => self.io[reg],
        }
    }

    fn write_io(&mut self, addr: u32, value: u8) {
        let reg = (addr as usize) & 0x3FF;
        match reg {
            0x202 | 0x203 => self.io[reg] &= !value, // IF: write-1-to-acknowledge
            _ => self.io[reg] = value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus() -> Bus {
        let mut rom = vec![0u8; 0x1000];
        rom[0] = 0xAA;
        rom[1] = 0xBB;
        Bus::new(rom)
    }

    #[test]
    fn regions_route_and_mirror() {
        let mut b = bus();
        b.write32(0x0200_0000, 0x1122_3344);
        assert_eq!(b.read32(0x0200_0000), 0x1122_3344);
        assert_eq!(b.read32(0x0204_0000), 0x1122_3344); // EWRAM mirrors every 256K
        b.write16(0x0300_0000, 0xBEEF);
        assert_eq!(b.read16(0x0300_8000), 0xBEEF); // IWRAM mirrors every 32K
        b.write8(0x0500_0001, 0x7C);
        assert_eq!(b.read8(0x0500_0401), 0x7C); // palette mirrors every 1K
        assert_eq!(b.read16(0x0800_0000), 0xBBAA); // ROM, little-endian
        assert_eq!(b.read16(0x0A00_0000), 0xBBAA); // ROM mirror (waitstate 1)
        b.write16(0x0800_0000, 0x0000); // ROM writes ignored
        assert_eq!(b.read16(0x0800_0000), 0xBBAA);
    }

    #[test]
    fn vram_mirror_quirk() {
        let mut b = bus();
        b.write8(0x0601_0000, 0x42);
        // 96K arranged 64K+32K; the 32K block appears twice per 128K window
        assert_eq!(b.read8(0x0601_8000), 0x42);
        assert_eq!(b.read8(0x0602_0000 + 0x1_0000), 0x42);
    }

    #[test]
    fn accesses_are_force_aligned() {
        let mut b = bus();
        b.write32(0x0200_0000, 0x1122_3344);
        // The bus always performs aligned accesses; rotation is the CPU's job
        assert_eq!(b.read32(0x0200_0002), 0x1122_3344);
        assert_eq!(b.read16(0x0200_0001), 0x3344);
    }

    #[test]
    fn every_access_costs_one_cycle() {
        let mut b = bus();
        b.read32(0x0200_0000);
        b.write8(0x0300_0000, 1);
        b.idle();
        assert_eq!(b.cycles, 3);
    }

    #[test]
    fn dispstat_vblank_toggles_with_cycles() {
        let mut b = bus();
        assert_eq!(b.read16(0x0400_0004) & 1, 0); // line 0: not vblank
        b.cycles = 160 * 1232;
        assert_eq!(b.read16(0x0400_0004) & 1, 1); // line 160: vblank
        assert_eq!(b.read16(0x0400_0006), 160); // VCOUNT
        b.cycles = 227 * 1232;
        assert_eq!(b.read16(0x0400_0004) & 1, 0); // last line: flag clear
        b.cycles = 228 * 1232;
        assert_eq!(b.read16(0x0400_0006), 0); // wrapped to line 0
    }

    #[test]
    fn io_stores_and_if_acknowledge() {
        let mut b = bus();
        b.write16(0x0400_0000, 0x0403); // DISPCNT readback
        assert_eq!(b.read16(0x0400_0000), 0x0403);
        b.raise_irq(0x0005); // peripherals set IF bits (the way G4 will)
        b.write16(0x0400_0202, 0x0001); // write-1-to-acknowledge clears bit 0
        assert_eq!(b.read16(0x0400_0202), 0x0004);
        assert_eq!(b.read16(0x0400_0130), 0x03FF); // KEYINPUT: nothing pressed
    }

    #[test]
    fn byte_writes_to_palette_and_vram_duplicate_into_halfword() {
        let mut b = bus();
        b.write8(0x0500_0002, 0x4A);
        assert_eq!(b.read16(0x0500_0002), 0x4A4A);
        b.write8(0x0600_0001, 0x7C); // odd address: same halfword
        assert_eq!(b.read16(0x0600_0000), 0x7C7C);
    }

    #[test]
    fn byte_writes_to_oam_are_ignored_and_16bit_writes_are_not_duplicated() {
        let mut b = bus();
        b.write8(0x0700_0000, 0xFF);
        assert_eq!(b.read16(0x0700_0000), 0x0000);
        b.write16(0x0500_0000, 0x1234); // must store 0x34, 0x12 — no quirk
        assert_eq!(b.read16(0x0500_0000), 0x1234);
        b.write32(0x0601_0000, 0xAABB_CCDD);
        assert_eq!(b.read32(0x0601_0000), 0xAABB_CCDD);
    }

    #[test]
    fn keyinput_reflects_buttons() {
        use termboy_core::Buttons;
        let mut b = bus();
        b.set_buttons(Buttons::A.with(Buttons::L));
        assert_eq!(b.read16(0x0400_0130), 0x01FE);
        b.set_buttons(Buttons::default());
        assert_eq!(b.read16(0x0400_0130), 0x03FF);
    }

    #[test]
    fn out_of_bounds_rom_reads_open_bus() {
        let mut b = bus();
        // beyond rom.len(): open bus returns the address bus pattern (addr/2)
        assert_eq!(b.read16(0x0800_2000), 0x1000);
    }
}
