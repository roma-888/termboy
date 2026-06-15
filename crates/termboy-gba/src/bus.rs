//! GBA memory map at coarse timing: every access is one cycle (real
//! waitstates land in G7). Bus::catch_up walks scanline-granular timing
//! events to drive rendering and IRQ sources between CPU steps.

pub(crate) const CYCLES_PER_LINE: u64 = 1232;
pub(crate) const LINES: u64 = 228;
const VBLANK_START: u64 = 160;
/// Cycle within a line at which hblank begins.
pub(crate) const HBLANK_AT: u64 = 1006;

pub struct Bus {
    pub rom: Vec<u8>,
    ewram: Box<[u8; 0x4_0000]>,
    iwram: Box<[u8; 0x8000]>,
    pub(crate) io: Box<[u8; 0x400]>,
    pub palette: Box<[u8; 0x400]>,
    pub vram: Box<[u8; 0x1_8000]>,
    pub oam: Box<[u8; 0x400]>,
    /// Cartridge save memory, detected from the ROM at construction.
    pub(crate) save: crate::save::Save,
    /// Frontend input, pre-encoded as KEYINPUT bits (active low).
    keyinput: u16,
    pub ppu: crate::ppu::Ppu,
    /// Total elapsed (coarse) cycles since power-on.
    pub cycles: u64,
    /// Number of timing events processed (2 per line: start, hblank).
    events_done: u64,
    pub(crate) dma: [crate::dma::Dma; 4],
    pub(crate) timers: [crate::timers::Timer; 4],
    /// Cycle count the timers were last advanced to.
    pub(crate) timers_synced: u64,
    pub(crate) apu: crate::apu::Apu,
    /// Cycle count the APU was last advanced to.
    apu_synced: u64,
    /// Set by HALTCNT / the Halt SWI; cleared when IE & IF intersect.
    pub halted: bool,
}

impl Bus {
    pub fn new(rom: Vec<u8>) -> Self {
        let save = crate::save::Save::detect(&rom);
        Self {
            rom,
            ewram: Box::new([0; 0x4_0000]),
            iwram: Box::new([0; 0x8000]),
            io: Box::new([0; 0x400]),
            palette: Box::new([0; 0x400]),
            vram: Box::new([0; 0x1_8000]),
            oam: Box::new([0; 0x400]),
            save,
            keyinput: 0x03FF,
            ppu: crate::ppu::Ppu::new(),
            cycles: 0,
            events_done: 0,
            dma: [crate::dma::Dma::default(); 4],
            timers: [crate::timers::Timer::default(); 4],
            timers_synced: 0,
            apu: crate::apu::Apu::new(),
            apu_synced: 0,
            halted: false,
        }
    }

    /// Advance the APU by the elapsed cycle delta (called from catch_up).
    pub(crate) fn apu_catch_up(&mut self) {
        let elapsed = (self.cycles - self.apu_synced) as u32;
        self.apu_synced = self.cycles;
        if elapsed > 0 {
            self.apu.tick(elapsed);
        }
    }

    /// Process every timing event the cycle counter has passed: scanline
    /// rendering, vblank/hblank/vcount IRQs, and (later tasks) DMA triggers
    /// and timer ticks. Two events per line: line start and hblank.
    pub fn catch_up(&mut self) {
        self.timers_catch_up();
        self.apu_catch_up();
        loop {
            let k = self.events_done;
            let at = (k / 2) * CYCLES_PER_LINE + (k % 2) * HBLANK_AT;
            if at > self.cycles {
                break;
            }
            self.events_done += 1;
            let line = (k / 2) % LINES;
            let dispstat = self.io16(0x004);
            if k % 2 == 0 {
                if line == self.io[0x005] as u64 && dispstat & (1 << 5) != 0 {
                    self.raise_irq(1 << 2); // VCOUNT match
                }
                if line == VBLANK_START {
                    if dispstat & (1 << 3) != 0 {
                        self.raise_irq(1 << 0);
                    }
                    self.dma_on_vblank();
                }
            } else {
                if dispstat & (1 << 4) != 0 {
                    self.raise_irq(1 << 1); // hblank fires on every line
                }
                if line < VBLANK_START {
                    self.render_scanline(line as usize);
                    if line == VBLANK_START - 1 {
                        self.ppu.frame_ready = true;
                    }
                    self.dma_on_hblank(); // hblank DMA pauses during vblank
                }
            }
        }
    }

    /// Frontend input, pre-encoded as KEYINPUT bits.
    pub fn set_buttons(&mut self, buttons: termboy_core::Buttons) {
        self.keyinput = crate::keypad::keyinput(buttons);
        self.check_keypad_irq();
    }

    /// KEYCNT: raise the keypad IRQ when the selected keys are down
    /// (bit 15 chooses OR / AND across the selection).
    fn check_keypad_irq(&mut self) {
        let keycnt = self.io16(0x132);
        if keycnt & (1 << 14) == 0 {
            return;
        }
        let sel = keycnt & 0x3FF;
        let down = !self.keyinput & 0x3FF;
        let hit = if keycnt & (1 << 15) != 0 {
            sel != 0 && down & sel == sel
        } else {
            down & sel != 0
        };
        if hit {
            self.raise_irq(1 << 12);
        }
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
            0x0E | 0x0F => self.save.read(addr),
            _ => 0,
        }
    }

    pub fn read16(&mut self, addr: u32) -> u16 {
        self.cycles += 1;
        if addr >> 24 == 0x0D {
            if let Some(e) = self.save.eeprom() {
                return e.read_bit();
            }
        }
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
            0x0E | 0x0F => self.save.write(addr, value),
            _ => {} // BIOS and ROM are read-only; unmapped writes vanish
        }
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        self.cycles += 1;
        if addr >> 24 == 0x0D {
            if let Some(e) = self.save.eeprom() {
                e.write_bit(value);
                return;
            }
        }
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

    pub(crate) fn io16(&self, reg: usize) -> u16 {
        u16::from_le_bytes([self.io[reg], self.io[reg + 1]])
    }

    pub(crate) fn io32(&self, reg: usize) -> u32 {
        u32::from_le_bytes([
            self.io[reg],
            self.io[reg + 1],
            self.io[reg + 2],
            self.io[reg + 3],
        ])
    }

    pub(crate) fn oam16(&self, i: usize) -> u16 {
        u16::from_le_bytes([self.oam[i], self.oam[i + 1]])
    }

    /// Palette entry as BGR555 (index 0-255 = BG palette, 256-511 = OBJ).
    pub(crate) fn palette16(&self, index: usize) -> u16 {
        u16::from_le_bytes([self.palette[index * 2], self.palette[index * 2 + 1]])
    }

    /// IE & IF intersect — the wake condition for HALT (IME is ignored).
    pub fn irq_asserted(&self) -> bool {
        self.io16(0x200) & self.io16(0x202) & 0x3FFF != 0
    }

    /// An interrupt should actually be delivered to the CPU.
    pub fn irq_pending(&self) -> bool {
        self.io[0x208] & 1 != 0 && self.irq_asserted()
    }

    /// Raise IF bits the way hardware peripherals do.
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
                if self.cycles % CYCLES_PER_LINE >= HBLANK_AT {
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
            // Timer counters read live; their io slots hold the reload value
            0x100 | 0x104 | 0x108 | 0x10C => self.timers[(reg - 0x100) / 4].counter as u8,
            0x101 | 0x105 | 0x109 | 0x10D => (self.timers[(reg - 0x100) / 4].counter >> 8) as u8,
            // Sound registers + wave RAM live in the APU
            0x060..=0x085 | 0x090..=0x09F => self.apu.read(reg),
            _ => self.io[reg],
        }
    }

    fn write_io(&mut self, addr: u32, value: u8) {
        let reg = (addr as usize) & 0x3FF;
        match reg {
            0x202 | 0x203 => self.io[reg] &= !value, // IF: write-1-to-acknowledge
            // KEYCNT: a new selection can match the keys already held. Only
            // the high byte (written last in a 16-bit store) triggers the
            // check, so a half-written register never matches transiently.
            0x133 => {
                self.io[reg] = value;
                self.check_keypad_irq();
            }
            // Timer control: settle the old configuration first, then latch
            // the reload into the counter on an enable rising edge
            0x102 | 0x106 | 0x10A | 0x10E => {
                let t = (reg - 0x100) / 4;
                let was = self.io[reg] & 0x80 != 0;
                self.timers_catch_up();
                self.io[reg] = value;
                if !was && value & 0x80 != 0 {
                    self.timers[t].counter = self.io16(0x100 + t * 4);
                    self.timers[t].rem = 0;
                }
            }
            // DMA control bytes: latch on the enable rising edge
            0x0BA | 0x0BB | 0x0C6 | 0x0C7 | 0x0D2 | 0x0D3 | 0x0DE | 0x0DF => {
                let ch = (reg - 0x0B0) / 0xC;
                let was = self.io[0x0BB + ch * 0xC] & 0x80 != 0;
                self.io[reg] = value;
                self.dma_ctrl_write(ch, was);
            }
            // Sound: settle the APU to now, then apply the register change.
            0x060..=0x085 | 0x090..=0x09F => {
                self.apu_catch_up();
                self.apu.write(reg, value);
            }
            0x0A0..=0x0A7 => self.apu.fifo_push((reg - 0x0A0) / 4, value),
            // HALTCNT: any write halts (bit 7 = Stop, same thing at coarse timing)
            0x301 => {
                self.io[reg] = value;
                self.halted = true;
            }
            // BG2X/Y, BG3X/Y: writing relatches the affine reference point
            0x028..=0x02F => {
                self.io[reg] = value;
                self.ppu.bg_ref_dirty[0] = true;
            }
            0x038..=0x03F => {
                self.io[reg] = value;
                self.ppu.bg_ref_dirty[1] = true;
            }
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
    fn vblank_irq_raised_once_at_line_160() {
        let mut b = bus();
        b.write16(0x0400_0004, 1 << 3); // DISPSTAT: vblank IRQ enable
        b.cycles = 159 * 1232;
        b.catch_up();
        assert_eq!(b.read16(0x0400_0202), 0); // not yet
        b.cycles = 160 * 1232;
        b.catch_up();
        assert_eq!(b.read16(0x0400_0202) & 1, 1);
        b.write16(0x0400_0202, 1); // acknowledge
        b.cycles = 170 * 1232;
        b.catch_up(); // still the same vblank: no re-raise
        assert_eq!(b.read16(0x0400_0202) & 1, 0);
    }

    #[test]
    fn vblank_irq_not_raised_when_disabled() {
        let mut b = bus();
        b.cycles = 161 * 1232;
        b.catch_up();
        assert_eq!(b.read16(0x0400_0202), 0);
    }

    #[test]
    fn hblank_irq_fires_every_line_including_vblank_lines() {
        let mut b = bus();
        b.write16(0x0400_0004, 1 << 4); // hblank IRQ enable
        b.cycles = 1005;
        b.catch_up();
        assert_eq!(b.read16(0x0400_0202), 0);
        b.cycles = 1006;
        b.catch_up();
        assert_eq!(b.read16(0x0400_0202) & 2, 2);
        b.write16(0x0400_0202, 2);
        b.cycles = 200 * 1232 + 1006; // deep in vblank
        b.catch_up();
        assert_eq!(b.read16(0x0400_0202) & 2, 2);
    }

    #[test]
    fn vcount_irq_on_matching_line_only() {
        let mut b = bus();
        b.write16(0x0400_0004, (42 << 8) | (1 << 5)); // VCOUNT=42 + IRQ enable
        b.cycles = 42 * 1232 - 1;
        b.catch_up();
        assert_eq!(b.read16(0x0400_0202), 0);
        b.cycles = 42 * 1232;
        b.catch_up();
        assert_eq!(b.read16(0x0400_0202) & 4, 4);
    }

    #[test]
    fn keypad_irq_or_and_and_modes() {
        use termboy_core::Buttons;
        let mut b = bus();
        b.write16(0x0400_0132, (1 << 14) | 0x0003); // IRQ on A or B
        b.set_buttons(Buttons::B);
        assert_eq!(b.read16(0x0400_0202) & (1 << 12), 1 << 12);
        b.write16(0x0400_0202, 1 << 12); // acknowledge
        b.write16(0x0400_0132, (1 << 14) | (1 << 15) | 0x0003); // A AND B
        b.set_buttons(Buttons::A);
        assert_eq!(b.read16(0x0400_0202), 0);
        b.set_buttons(Buttons::A.with(Buttons::B));
        assert_eq!(b.read16(0x0400_0202) & (1 << 12), 1 << 12);
    }

    #[test]
    fn keypad_irq_needs_the_enable_bit() {
        use termboy_core::Buttons;
        let mut b = bus();
        b.write16(0x0400_0132, 0x03FF); // all keys selected, IRQ disabled
        b.set_buttons(Buttons::START);
        assert_eq!(b.read16(0x0400_0202), 0);
    }

    #[test]
    fn out_of_bounds_rom_reads_open_bus() {
        let mut b = bus();
        // beyond rom.len(): open bus returns the address bus pattern (addr/2)
        assert_eq!(b.read16(0x0800_2000), 0x1000);
    }
}
