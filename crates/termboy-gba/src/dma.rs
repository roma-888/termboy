//! DMA controller: 4 channels with immediate/vblank/hblank timing. Coarse
//! model: a triggered channel runs its whole transfer between CPU steps,
//! cycles accruing through the normal bus read/write paths. Timing mode 3
//! (sound FIFO / video capture) is inert until G6.

use crate::bus::Bus;

const CNT_H: [usize; 4] = [0x0BA, 0x0C6, 0x0D2, 0x0DE];

#[derive(Default, Clone, Copy)]
pub struct Dma {
    /// Internal (latched) registers — reloaded from IO on enable.
    src: u32,
    dst: u32,
    count: u32,
    pub(crate) pending: bool,
}

impl Bus {
    /// Called on every CNT_H byte store: latch on a 0->1 enable edge,
    /// drop pending work on disable.
    pub(crate) fn dma_ctrl_write(&mut self, ch: usize, was_enabled: bool) {
        let ctrl = self.io16(CNT_H[ch]);
        if ctrl & 0x8000 == 0 {
            self.dma[ch].pending = false;
            return;
        }
        if was_enabled {
            return;
        }
        let base = 0x0B0 + ch * 0xC;
        self.dma[ch].src = self.io32(base);
        self.dma[ch].dst = self.io32(base + 4);
        self.dma[ch].count = Self::dma_count(ch, self.io16(base + 8));
        if (ctrl >> 12) & 3 == 0 {
            self.dma[ch].pending = true;
        }
    }

    /// 14-bit count on channels 0-2, 16-bit on 3; zero means the maximum.
    fn dma_count(ch: usize, raw: u16) -> u32 {
        let (mask, max) = if ch == 3 { (0xFFFF, 0x1_0000) } else { (0x3FFF, 0x4000) };
        let c = (raw as u32) & mask;
        if c == 0 { max } else { c }
    }

    pub(crate) fn dma_on_vblank(&mut self) {
        self.dma_trigger(1);
    }

    pub(crate) fn dma_on_hblank(&mut self) {
        self.dma_trigger(2);
    }

    fn dma_trigger(&mut self, timing: u16) {
        for ch in 0..4 {
            let ctrl = self.io16(CNT_H[ch]);
            if ctrl & 0x8000 != 0 && (ctrl >> 12) & 3 == timing {
                self.dma[ch].pending = true;
            }
        }
    }

    /// Lowest pending channel — channel 0 has the highest priority.
    pub fn dma_ready(&self) -> Option<usize> {
        (0..4).find(|&ch| self.dma[ch].pending)
    }

    /// Run channel `ch` to completion.
    pub fn run_dma(&mut self, ch: usize) {
        self.dma[ch].pending = false;
        let ctrl = self.io16(CNT_H[ch]);
        // Direct Sound DMA (special timing, dest = a FIFO port): always 4
        // words to the fixed port, regardless of the programmed unit/count.
        let fifo_dma =
            (ctrl >> 12) & 3 == 3 && matches!(self.dma[ch].dst, 0x0400_00A0 | 0x0400_00A4);
        let unit: u32 = if fifo_dma || ctrl & (1 << 10) != 0 { 4 } else { 2 };
        let dst_adj = if fifo_dma { 2 } else { (ctrl >> 5) & 3 };
        let src_adj = (ctrl >> 7) & 3;
        let count = if fifo_dma { 4 } else { self.dma[ch].count };
        let mut src = self.dma[ch].src & !(unit - 1);
        let mut dst = self.dma[ch].dst & !(unit - 1);
        // EEPROM rides the DMA: its transfer length selects command + size.
        if dst >> 24 == 0x0D {
            self.save.eeprom_begin(count, true);
        } else if src >> 24 == 0x0D {
            self.save.eeprom_begin(count, false);
        }
        for _ in 0..count {
            if unit == 4 {
                let v = self.read32(src);
                self.write32(dst, v);
            } else {
                let v = self.read16(src);
                self.write16(dst, v);
            }
            src = match src_adj {
                1 => src.wrapping_sub(unit),
                2 => src,
                _ => src.wrapping_add(unit), // 3 is prohibited; treat as inc
            };
            dst = match dst_adj {
                1 => dst.wrapping_sub(unit),
                2 => dst,
                _ => dst.wrapping_add(unit),
            };
        }
        self.dma[ch].src = src;
        self.dma[ch].dst = dst;

        if ctrl & (1 << 9) != 0 && (ctrl >> 12) & 3 != 0 {
            // repeat: reload the count; mode 3 also re-latches the dest
            self.dma[ch].count = Self::dma_count(ch, self.io16(0x0B0 + ch * 0xC + 8));
            if dst_adj == 3 {
                self.dma[ch].dst = self.io32(0x0B0 + ch * 0xC + 4);
            }
        } else {
            self.io[CNT_H[ch] + 1] &= 0x7F; // clear the enable bit
        }
        if ctrl & (1 << 14) != 0 {
            self.raise_irq(1 << (8 + ch));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::bus::Bus;

    fn bus() -> Bus {
        let mut rom = vec![0u8; 0x1000];
        for (i, b) in rom.iter_mut().enumerate() {
            *b = i as u8;
        }
        Bus::new(rom)
    }

    /// Program channel 3: ROM 0x08000000 -> EWRAM, n units, ctrl bits.
    fn program_ch3(b: &mut Bus, count: u16, ctrl: u16) {
        b.write32(0x0400_00D4, 0x0800_0000);
        b.write32(0x0400_00D8, 0x0200_0000);
        b.write16(0x0400_00DC, count);
        b.write16(0x0400_00DE, ctrl);
    }

    #[test]
    fn immediate_word_dma_copies_and_self_disables() {
        let mut b = bus();
        program_ch3(&mut b, 4, 0x8000 | (1 << 10)); // enable, 32-bit, immediate
        let ch = b.dma_ready().expect("immediate DMA should be pending");
        b.run_dma(ch);
        assert_eq!(b.read32(0x0200_0000), 0x03020100);
        assert_eq!(b.read32(0x0200_000C), 0x0F0E0D0C);
        assert_eq!(b.read32(0x0200_0010), 0); // exactly 4 units
        assert_eq!(b.read16(0x0400_00DE) & 0x8000, 0); // enable cleared
        assert!(b.dma_ready().is_none());
        assert_eq!(b.read16(0x0400_0202), 0); // no IRQ unless requested
    }

    #[test]
    fn halfword_dma_with_fixed_source_fills() {
        let mut b = bus();
        b.write16(0x0200_0000, 0xBEEF); // source value in EWRAM
        b.write32(0x0400_00D4, 0x0200_0000);
        b.write32(0x0400_00D8, 0x0200_1000);
        b.write16(0x0400_00DC, 3);
        b.write16(0x0400_00DE, 0x8000 | (2 << 7)); // src fixed, 16-bit
        b.run_dma(b.dma_ready().unwrap());
        assert_eq!(b.read16(0x0200_1000), 0xBEEF);
        assert_eq!(b.read16(0x0200_1004), 0xBEEF);
        assert_eq!(b.read16(0x0200_1006), 0); // 3 units only
    }

    #[test]
    fn count_zero_means_max_and_irq_fires() {
        let mut b = bus();
        // ch0 count field is 14-bit: 0 -> 0x4000 units
        b.write32(0x0400_00B0, 0x0800_0000);
        b.write32(0x0400_00B4, 0x0200_0000);
        b.write16(0x0400_00B8, 0);
        b.write16(0x0400_00BA, 0x8000 | (1 << 14)); // enable + IRQ
        b.run_dma(b.dma_ready().unwrap());
        assert_eq!(b.read16(0x0400_0202) & (1 << 8), 1 << 8); // DMA0 IRQ
        // last halfword written: unit 0x3FFF read past the 4K ROM, so the
        // open-bus pattern (addr/2) proves all 0x4000 units transferred
        assert_eq!(b.read16(0x0200_7FFE), ((0x0800_7FFEu32 >> 1) & 0xFFFF) as u16);
    }

    #[test]
    fn vblank_dma_waits_for_line_160() {
        let mut b = bus();
        program_ch3(&mut b, 1, 0x8000 | (1 << 12)); // vblank timing
        assert!(b.dma_ready().is_none());
        b.cycles = 160 * 1232;
        b.catch_up();
        assert_eq!(b.dma_ready(), Some(3));
    }

    #[test]
    fn hblank_repeat_dma_stays_enabled_and_reloads_dest() {
        let mut b = bus();
        // dest inc+reload (3<<5), repeat (1<<9), hblank timing (2<<12)
        program_ch3(&mut b, 2, 0x8000 | (2 << 12) | (1 << 9) | (3 << 5));
        b.cycles = 1006; // first visible hblank
        b.catch_up();
        assert_eq!(b.dma_ready(), Some(3));
        b.run_dma(3);
        assert_eq!(b.read16(0x0400_00DE) & 0x8000, 0x8000); // still enabled
        b.write16(0x0200_0000, 0); // scrub, then prove dest reloaded
        b.cycles = 2 * 1232 + 1006;
        b.catch_up();
        b.run_dma(b.dma_ready().unwrap());
        // dest snapped back to 0x02000000 while src kept going (ROM bytes 4-5)
        assert_eq!(b.read16(0x0200_0000), 0x0504);
    }

    #[test]
    fn channel_zero_outranks_channel_three() {
        let mut b = bus();
        program_ch3(&mut b, 1, 0x8000);
        b.write32(0x0400_00B0, 0x0800_0000);
        b.write32(0x0400_00B4, 0x0200_0100);
        b.write16(0x0400_00B8, 1);
        b.write16(0x0400_00BA, 0x8000);
        assert_eq!(b.dma_ready(), Some(0));
    }

    #[test]
    fn disabling_a_channel_drops_pending_work() {
        let mut b = bus();
        program_ch3(&mut b, 1, 0x8000);
        b.write16(0x0400_00DE, 0);
        assert!(b.dma_ready().is_none());
    }

    #[test]
    fn eeprom_write_then_read_round_trips_via_dma() {
        let mut rom = vec![0u8; 0x400];
        rom[0x200..0x208].copy_from_slice(b"EEPROM_V");
        let mut b = Bus::new(rom);

        // Build a 73-bit write command for addr 3, data 0x00FF00FF00FF00FF,
        // as halfwords (bit 0 each) in EWRAM at 0x02000000.
        let mut bits = vec![1u8, 0]; // write
        for i in (0..6).rev() {
            bits.push(((3usize >> i) & 1) as u8);
        }
        let data: u64 = 0x00FF_00FF_00FF_00FF;
        for i in (0..64).rev() {
            bits.push(((data >> i) & 1) as u8);
        }
        bits.push(0); // stop -> 73 units
        for (i, &bit) in bits.iter().enumerate() {
            b.write16(0x0200_0000 + i as u32 * 2, bit as u16);
        }
        // DMA3: EWRAM -> EEPROM (0x0D000000), 16-bit, 73 units, immediate
        b.write32(0x0400_00D4, 0x0200_0000);
        b.write32(0x0400_00D8, 0x0D00_0000);
        b.write16(0x0400_00DC, 73);
        b.write16(0x0400_00DE, 0x8000);
        b.run_dma(b.dma_ready().unwrap());

        // Read request (9 units) for addr 3.
        let mut req = vec![1u8, 1];
        for i in (0..6).rev() {
            req.push(((3usize >> i) & 1) as u8);
        }
        req.push(0);
        for (i, &bit) in req.iter().enumerate() {
            b.write16(0x0200_0100 + i as u32 * 2, bit as u16);
        }
        b.write32(0x0400_00D4, 0x0200_0100);
        b.write32(0x0400_00D8, 0x0D00_0000);
        b.write16(0x0400_00DC, 9);
        b.write16(0x0400_00DE, 0x8000);
        b.run_dma(b.dma_ready().unwrap());

        // Read reply DMA: EEPROM -> EWRAM, 68 units; reassemble the word.
        b.write32(0x0400_00D4, 0x0D00_0000);
        b.write32(0x0400_00D8, 0x0200_0200);
        b.write16(0x0400_00DC, 68);
        b.write16(0x0400_00DE, 0x8000);
        b.run_dma(b.dma_ready().unwrap());

        let mut got = 0u64;
        for i in 4..68u32 {
            got = (got << 1) | (b.read16(0x0200_0200 + i * 2) & 1) as u64;
        }
        assert_eq!(got, data);
    }
}
