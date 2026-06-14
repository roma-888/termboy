//! HLE BIOS: SWI functions implemented in Rust (design spec §4 — the
//! self-contained choice). Calls run in the caller's context (no SVC entry);
//! the `Cpu::hle_bios` flag is the seam for swapping in a real BIOS image.

use crate::cpu::Cpu;
use crate::cpu::registers::Registers;

/// BIOS_IF: the user ISR records acknowledged IRQs here for IntrWait.
const BIOS_IF: u32 = 0x0300_7FF8;

impl Cpu {
    pub(crate) fn bios_call(&mut self, function: u32) {
        match function {
            0x00 => self.soft_reset(),
            0x01 => self.register_ram_reset(),
            0x02 | 0x03 => self.bus.halted = true, // Halt / Stop
            0x04 => self.intr_wait(self.regs.get(0) != 0, self.regs.get(1) as u16),
            0x05 => self.intr_wait(true, 1), // VBlankIntrWait
            0x06 => self.div(self.regs.get(0) as i32, self.regs.get(1) as i32),
            0x07 => self.div(self.regs.get(1) as i32, self.regs.get(0) as i32), // DivArm
            0x08 => {
                let r = isqrt(self.regs.get(0));
                self.regs.set(0, r);
            }
            0x09 => {
                let r = arctan(self.regs.get(0) as i32);
                self.regs.set(0, r as u32);
            }
            0x0A => {
                let r = arctan2(self.regs.get(0) as i32, self.regs.get(1) as i32);
                self.regs.set(0, r);
            }
            0x0B => self.cpu_set(),
            0x0C => self.cpu_fast_set(),
            0x0D => self.regs.set(0, 0xBAAE_187F), // GetBiosChecksum
            0x10 => self.bit_unpack(),
            0x11 => {
                let d = self.lz77_decompress();
                self.write_out_bytes(&d);
            }
            0x12 => {
                let d = self.lz77_decompress();
                self.write_out_halfwords(&d);
            }
            0x13 => self.huff_decompress(),
            0x14 => {
                let d = self.rl_decompress();
                self.write_out_bytes(&d);
            }
            0x15 => {
                let d = self.rl_decompress();
                self.write_out_halfwords(&d);
            }
            0x16 => self.diff8_unfilter(false),
            0x17 => self.diff8_unfilter(true),
            0x18 => self.diff16_unfilter(),
            0x19 => {} // SoundBias: no APU until G6
            // The BIOS-resident sound driver: m4a games carry their own copy
            // in ROM and never call these (except MidiKey2Freq below).
            0x1A..=0x1E | 0x20..=0x24 | 0x28 | 0x29 => {}
            0x1F => self.midi_key_to_freq(),
            0x25 => self.regs.set(0, 1), // MultiBoot: report failure
            f => unimplemented!(
                "HLE BIOS function {f:#04X} at {:#010X}",
                self.exec_addr()
            ),
        }
    }

    /// Div: r0 = n/d, r1 = n%d, r3 = |n/d|.
    fn div(&mut self, n: i32, d: i32) {
        assert!(d != 0, "BIOS Div by zero at {:#010X}", self.exec_addr());
        let q = n.wrapping_div(d);
        self.regs.set(0, q as u32);
        self.regs.set(1, n.wrapping_rem(d) as u32);
        self.regs.set(3, q.unsigned_abs());
    }

    /// IntrWait. While unsatisfied the PC is parked back on the SWI so it
    /// re-executes after every ISR — the real BIOS's Halt loop, HLE'd.
    /// `intrwait` remembers that the discard already happened.
    fn intr_wait(&mut self, discard: bool, flags: u16) {
        let flags = flags & 0x3FFF;
        if self.intrwait.is_none() {
            self.intrwait = Some(flags);
            if discard {
                let cur = self.bus.read16(BIOS_IF);
                self.bus.write16(BIOS_IF, cur & !flags);
            }
        }
        self.bus.write16(0x0400_0208, 1); // IntrWait forces IME on
        let cur = self.bus.read16(BIOS_IF);
        if cur & flags != 0 {
            self.bus.write16(BIOS_IF, cur & !flags);
            self.intrwait = None;
            return; // satisfied: fall through past the SWI
        }
        self.bus.halted = true;
        let pc = self.exec_addr();
        self.regs.set(15, pc);
        self.flush();
    }

    /// MidiKey2Freq: r0 = WaveData*, r1 = key, r2 = fine. The one BIOS
    /// sound call m4a games (Pokémon included) actually make.
    fn midi_key_to_freq(&mut self) {
        let base = self.bus.read32(self.regs.get(0).wrapping_add(4)) as f64;
        let key = self.regs.get(1) as f64;
        let fine = self.regs.get(2) as f64;
        let freq = base / f64::exp2((180.0 - key - fine / 256.0) / 12.0);
        self.regs.set(0, freq as u32);
    }

    /// SoftReset: clear the BIOS work area, reset banks/stacks/mode, and
    /// restart at the entry the flag at 0x03007FFA selects.
    fn soft_reset(&mut self) {
        let flag = self.bus.read8(0x0300_7FFA);
        for a in (0x0300_7E00..0x0300_8000).step_by(4) {
            self.bus.write32(a, 0);
        }
        self.regs = Registers::new_post_bios();
        self.regs.set(15, if flag == 0 { 0x0800_0000 } else { 0x0200_0000 });
        self.flush();
    }

    /// RegisterRamReset, coarsely: wipe the selected regions through the
    /// normal bus paths (so the IO write hooks fire) and blank the screen.
    fn register_ram_reset(&mut self) {
        let f = self.regs.get(0);
        if f & 0x01 != 0 {
            self.wipe(0x0200_0000, 0x0204_0000);
        }
        if f & 0x02 != 0 {
            self.wipe(0x0300_0000, 0x0300_7E00); // IWRAM minus the BIOS area
        }
        if f & 0x04 != 0 {
            self.wipe(0x0500_0000, 0x0500_0400);
        }
        if f & 0x08 != 0 {
            self.wipe(0x0600_0000, 0x0601_8000);
        }
        if f & 0x10 != 0 {
            self.wipe(0x0700_0000, 0x0700_0400);
        }
        if f & 0x20 != 0 {
            // SIO registers (the keypad pair at 0x130/0x132 is excluded)
            self.wipe(0x0400_0120, 0x0400_0130);
            self.wipe(0x0400_0134, 0x0400_015C);
        }
        if f & 0x40 != 0 {
            self.wipe(0x0400_0060, 0x0400_00B0); // sound registers
        }
        if f & 0x80 != 0 {
            // "all other registers": video (sans DISPCNT), DMA, timers, IE/IME
            self.wipe(0x0400_0004, 0x0400_0060);
            self.wipe(0x0400_00B0, 0x0400_0110);
            self.wipe(0x0400_0200, 0x0400_020C);
        }
        self.bus.write16(0x0400_0000, 0x0080); // always leaves forced blank
    }

    /// CpuSet: r0=src, r1=dst, r2 = count | fill<<24 | word<<26.
    fn cpu_set(&mut self) {
        let ctrl = self.regs.get(2);
        let count = ctrl & 0x1F_FFFF;
        let fill = ctrl & (1 << 24) != 0;
        let unit: u32 = if ctrl & (1 << 26) != 0 { 4 } else { 2 };
        let mut src = self.regs.get(0) & !(unit - 1);
        let mut dst = self.regs.get(1) & !(unit - 1);
        let mut value = 0u32;
        for i in 0..count {
            if !fill || i == 0 {
                value = if unit == 4 {
                    self.bus.read32(src)
                } else {
                    self.bus.read16(src) as u32
                };
                if !fill {
                    src = src.wrapping_add(unit);
                }
            }
            if unit == 4 {
                self.bus.write32(dst, value);
            } else {
                self.bus.write16(dst, value as u16);
            }
            dst = dst.wrapping_add(unit);
        }
    }

    /// CpuFastSet: 32-bit only, count rounded up to a multiple of 8 words.
    fn cpu_fast_set(&mut self) {
        let ctrl = self.regs.get(2);
        let count = (ctrl & 0x1F_FFFF).div_ceil(8) * 8;
        let fill = ctrl & (1 << 24) != 0;
        let mut src = self.regs.get(0) & !3;
        let mut dst = self.regs.get(1) & !3;
        let mut value = 0u32;
        for i in 0..count {
            if !fill || i == 0 {
                value = self.bus.read32(src);
                if !fill {
                    src = src.wrapping_add(4);
                }
            }
            self.bus.write32(dst, value);
            dst = dst.wrapping_add(4);
        }
    }

    /// BitUnPack: widen src_w-bit fields to dst_w bits, adding `offset` to
    /// non-zero values (and zero too if bit 31 of the info word says so).
    fn bit_unpack(&mut self) {
        let mut src = self.regs.get(0);
        let mut dst = self.regs.get(1);
        let info = self.regs.get(2);
        let len = self.bus.read16(info) as u32;
        let src_w = self.bus.read8(info.wrapping_add(2)) as u32;
        let dst_w = self.bus.read8(info.wrapping_add(3)) as u32;
        let off = self.bus.read32(info.wrapping_add(4));
        let offset = off & 0x7FFF_FFFF;
        let zero_too = off >> 31 != 0;
        let mut out = 0u32;
        let mut out_bits = 0u32;
        for _ in 0..len {
            let byte = self.bus.read8(src) as u32;
            src = src.wrapping_add(1);
            let mut bit = 0;
            while bit < 8 {
                let v = (byte >> bit) & ((1 << src_w) - 1);
                let v = if v != 0 || zero_too { v.wrapping_add(offset) } else { v };
                out |= v.wrapping_shl(out_bits);
                out_bits += dst_w;
                if out_bits >= 32 {
                    self.bus.write32(dst, out);
                    dst = dst.wrapping_add(4);
                    out = 0;
                    out_bits = 0;
                }
                bit += src_w;
            }
        }
    }

    /// GBA LZ77 (type-1 header): flag byte then 8 blocks, MSB first;
    /// a set bit is a (len 3-18, disp 1-4096) back-reference. Inflated into
    /// a Vec first so back-references never read half-written memory.
    fn lz77_decompress(&mut self) -> Vec<u8> {
        let mut src = self.regs.get(0);
        let size = (self.bus.read32(src) >> 8) as usize;
        src = src.wrapping_add(4);
        let mut out = Vec::with_capacity(size);
        while out.len() < size {
            let flags = self.bus.read8(src);
            src = src.wrapping_add(1);
            for i in (0..8).rev() {
                if out.len() >= size {
                    break;
                }
                if flags & (1 << i) == 0 {
                    out.push(self.bus.read8(src));
                    src = src.wrapping_add(1);
                } else {
                    let b0 = self.bus.read8(src) as usize;
                    let b1 = self.bus.read8(src.wrapping_add(1)) as usize;
                    src = src.wrapping_add(2);
                    let len = (b0 >> 4) + 3;
                    let disp = ((b0 & 0xF) << 8 | b1) + 1;
                    for _ in 0..len {
                        if out.len() >= size {
                            break;
                        }
                        let v = out[out.len() - disp];
                        out.push(v);
                    }
                }
            }
        }
        out
    }

    /// GBA RLE (type-3 header): bit 7 of the flag byte picks run/literal.
    fn rl_decompress(&mut self) -> Vec<u8> {
        let mut src = self.regs.get(0);
        let size = (self.bus.read32(src) >> 8) as usize;
        src = src.wrapping_add(4);
        let mut out = Vec::with_capacity(size);
        while out.len() < size {
            let f = self.bus.read8(src) as usize;
            src = src.wrapping_add(1);
            if f & 0x80 != 0 {
                let v = self.bus.read8(src);
                src = src.wrapping_add(1);
                for _ in 0..(f & 0x7F) + 3 {
                    out.push(v);
                }
            } else {
                for _ in 0..(f & 0x7F) + 1 {
                    out.push(self.bus.read8(src));
                    src = src.wrapping_add(1);
                }
            }
        }
        out.truncate(size);
        out
    }

    /// HuffUnComp: 4/8-bit symbols; tree of (offset, leaf-flag) nodes after
    /// the size byte; the bitstream is consumed as 32-bit words, MSB first.
    fn huff_decompress(&mut self) {
        let src = self.regs.get(0);
        let header = self.bus.read32(src);
        let sym_bits = header & 0xF;
        let size = (header >> 8) as usize;
        let tree_base = src.wrapping_add(4);
        let tree_size = self.bus.read8(tree_base) as u32;
        let mut stream = tree_base.wrapping_add((tree_size + 1) * 2);
        let root = tree_base.wrapping_add(1);
        let mut out: Vec<u8> = Vec::with_capacity(size);
        let mut node = root;
        let mut nibble: Option<u8> = None;
        'outer: while out.len() < size {
            let word = self.bus.read32(stream);
            stream = stream.wrapping_add(4);
            for i in (0..32).rev() {
                let nv = self.bus.read8(node) as u32;
                let bit = (word >> i) & 1;
                let next = (node & !1).wrapping_add(((nv & 0x3F) + 1) * 2).wrapping_add(bit);
                let leaf = if bit == 0 { nv & 0x80 != 0 } else { nv & 0x40 != 0 };
                if !leaf {
                    node = next;
                    continue;
                }
                let sym = self.bus.read8(next);
                node = root;
                if sym_bits == 8 {
                    out.push(sym);
                } else {
                    match nibble.take() {
                        None => nibble = Some(sym),
                        Some(lo) => out.push(lo | (sym << 4)),
                    }
                }
                if out.len() >= size {
                    break 'outer;
                }
            }
        }
        let mut dst = self.regs.get(1);
        for chunk in out.chunks(4) {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            self.bus.write32(dst, u32::from_le_bytes(w));
            dst = dst.wrapping_add(4);
        }
    }

    fn diff8_unfilter(&mut self, vram: bool) {
        let src = self.regs.get(0);
        let size = (self.bus.read32(src) >> 8) as usize;
        let mut s = src.wrapping_add(4);
        let mut out = Vec::with_capacity(size);
        let mut acc: u8 = 0;
        for _ in 0..size {
            acc = acc.wrapping_add(self.bus.read8(s));
            s = s.wrapping_add(1);
            out.push(acc);
        }
        if vram {
            self.write_out_halfwords(&out);
        } else {
            self.write_out_bytes(&out);
        }
    }

    fn diff16_unfilter(&mut self) {
        let src = self.regs.get(0);
        let size = (self.bus.read32(src) >> 8) as usize;
        let mut s = src.wrapping_add(4);
        let mut dst = self.regs.get(1) & !1;
        let mut acc: u16 = 0;
        for _ in 0..size / 2 {
            acc = acc.wrapping_add(self.bus.read16(s));
            s = s.wrapping_add(2);
            self.bus.write16(dst, acc);
            dst = dst.wrapping_add(2);
        }
    }

    fn write_out_bytes(&mut self, data: &[u8]) {
        let mut dst = self.regs.get(1);
        for &b in data {
            self.bus.write8(dst, b);
            dst = dst.wrapping_add(1);
        }
    }

    /// VRAM-safe flavor: byte stores to VRAM duplicate through the bus
    /// quirk, so emit halfwords.
    fn write_out_halfwords(&mut self, data: &[u8]) {
        let mut dst = self.regs.get(1) & !1;
        for pair in data.chunks(2) {
            let v = u16::from_le_bytes([pair[0], *pair.get(1).unwrap_or(&0)]);
            self.bus.write16(dst, v);
            dst = dst.wrapping_add(2);
        }
    }

    fn wipe(&mut self, lo: u32, hi: u32) {
        for a in (lo..hi).step_by(4) {
            self.bus.write32(a, 0);
        }
    }
}

/// Floor square root, exact for all u32 (the float seed is then corrected).
fn isqrt(n: u32) -> u32 {
    let n = n as u64;
    let mut x = (n as f64).sqrt() as u64;
    while x * x > n {
        x -= 1;
    }
    while (x + 1) * (x + 1) <= n {
        x += 1;
    }
    x as u32
}

/// The BIOS's own arctan polynomial, bit-exact (1.14 fixed-point in/out).
fn arctan(i: i32) -> i32 {
    let a = -((i.wrapping_mul(i)) >> 14);
    let mut b = ((0xA9 * a) >> 14) + 0x390;
    b = ((b * a) >> 14) + 0x91C;
    b = ((b * a) >> 14) + 0xFB6;
    b = ((b * a) >> 14) + 0x16AA;
    b = ((b * a) >> 14) + 0x2081;
    b = ((b * a) >> 14) + 0x3651;
    b = ((b * a) >> 14) + 0xA2F9;
    i.wrapping_mul(b) >> 16
}

/// ArcTan2 with the BIOS's quadrant handling (result: 0..0xFFFF, 0x4000=90°).
fn arctan2(x: i32, y: i32) -> u32 {
    if y == 0 {
        return ((x >> 16) & 0x8000) as u32;
    }
    if x == 0 {
        return (((y >> 16) & 0x8000) + 0x4000) as u32;
    }
    if x.abs() > y.abs() || (x.abs() == y.abs() && (x >= 0 || y >= 0)) {
        let t = arctan((y << 14).wrapping_div(x));
        if x < 0 {
            (0x8000 + t) as u32
        } else {
            ((((y >> 16) & 0x8000) << 1) + t) as u32
        }
    } else {
        let t = arctan((x << 14).wrapping_div(y));
        ((((y >> 16) & 0x8000) + 0x4000) - t) as u32
    }
}

#[cfg(test)]
mod tests {
    use crate::cpu::Cpu;
    use crate::cpu::tests::cpu_with;

    /// Execute one SWI `f` with r0-r2 preloaded.
    fn swi(f: u32, r: [u32; 3]) -> Cpu {
        let mut cpu = cpu_with(&[0xEF00_0000 | (f << 16)]);
        for (i, v) in r.into_iter().enumerate() {
            cpu.regs.set(i, v);
        }
        cpu.step();
        cpu
    }

    #[test]
    fn divarm_swaps_the_operands() {
        let cpu = swi(0x07, [3, 100, 0]);
        assert_eq!(cpu.regs.get(0), 33);
        assert_eq!(cpu.regs.get(1), 1);
    }

    #[test]
    fn sqrt_is_exact_at_boundaries() {
        for (n, r) in [(0u32, 0u32), (1, 1), (3, 1), (4, 2), (99, 9), (100, 10), (0xFFFF_FFFF, 0xFFFF)] {
            assert_eq!(swi(0x08, [n, 0, 0]).regs.get(0), r, "sqrt({n})");
        }
    }

    #[test]
    fn arctan2_hits_the_cardinals_and_diagonal() {
        assert_eq!(swi(0x0A, [0x100, 0, 0]).regs.get(0), 0);
        assert_eq!(swi(0x0A, [(-0x100i32) as u32, 0, 0]).regs.get(0), 0x8000);
        assert_eq!(swi(0x0A, [0, 0x100, 0]).regs.get(0), 0x4000);
        assert_eq!(swi(0x0A, [0, (-0x100i32) as u32, 0]).regs.get(0), 0xC000);
        let diag = swi(0x0A, [0x100, 0x100, 0]).regs.get(0);
        assert!((0x1FFF..=0x2001).contains(&diag), "45 deg = {diag:#X}");
    }

    #[test]
    fn bios_checksum_is_the_ds_value() {
        assert_eq!(swi(0x0D, [0, 0, 0]).regs.get(0), 0xBAAE_187F);
    }

    #[test]
    fn midi_key_to_freq_matches_the_reference_points() {
        let mut cpu = cpu_with(&[0xEF1F_0000]);
        cpu.bus.write32(0x0200_0004, 88_0000); // WaveData base frequency word
        cpu.regs.set(0, 0x0200_0000);
        cpu.regs.set(1, 180); // at key 180, fine 0: result == base
        cpu.regs.set(2, 0);
        cpu.step();
        assert_eq!(cpu.regs.get(0), 88_0000);
        let mut cpu = cpu_with(&[0xEF1F_0000]);
        cpu.bus.write32(0x0200_0004, 88_0000);
        cpu.regs.set(0, 0x0200_0000);
        cpu.regs.set(1, 168); // one octave down: half
        cpu.step();
        assert_eq!(cpu.regs.get(0), 44_0000);
    }

    #[test]
    fn sound_driver_calls_are_inert() {
        let cpu = swi(0x1A, [0, 0, 0]); // SoundDriverInit must not panic
        assert_eq!(cpu.regs.cpsr.mode(), crate::cpu::psr::Mode::System);
    }

    #[test]
    fn register_ram_reset_wipes_selected_regions_and_blanks() {
        let mut cpu = cpu_with(&[0xEF01_0000]);
        cpu.bus.write32(0x0200_0000, 0xDEAD_BEEF);
        cpu.bus.write32(0x0300_0000, 0xDEAD_BEEF);
        cpu.bus.write32(0x0300_7F00, 0xCAFE_F00D); // stack area: preserved
        cpu.regs.set(0, 0x01); // EWRAM only
        cpu.step();
        assert_eq!(cpu.bus.read32(0x0200_0000), 0);
        assert_eq!(cpu.bus.read32(0x0300_0000), 0xDEAD_BEEF);
        assert_eq!(cpu.bus.read32(0x0300_7F00), 0xCAFE_F00D);
        assert_eq!(cpu.bus.read16(0x0400_0000), 0x0080); // forced blank
    }

    #[test]
    fn cpu_set_halfword_copy_and_word_fill() {
        let mut cpu = cpu_with(&[0xEF0B_0000]);
        for i in 0..4u32 {
            cpu.bus.write16(0x0200_0000 + i * 2, 0x1110 + i as u16);
        }
        cpu.regs.set(0, 0x0200_0000);
        cpu.regs.set(1, 0x0200_1000);
        cpu.regs.set(2, 4); // 4 halfwords, copy
        cpu.step();
        assert_eq!(cpu.bus.read16(0x0200_1000), 0x1110);
        assert_eq!(cpu.bus.read16(0x0200_1006), 0x1113);
        assert_eq!(cpu.bus.read16(0x0200_1008), 0);

        let mut cpu = cpu_with(&[0xEF0B_0000]);
        cpu.bus.write32(0x0200_0000, 0xAABB_CCDD);
        cpu.regs.set(0, 0x0200_0000);
        cpu.regs.set(1, 0x0200_2000);
        cpu.regs.set(2, 3 | (1 << 24) | (1 << 26)); // 3 words, fill, 32-bit
        cpu.step();
        assert_eq!(cpu.bus.read32(0x0200_2008), 0xAABB_CCDD);
        assert_eq!(cpu.bus.read32(0x0200_200C), 0);
    }

    #[test]
    fn cpu_fast_set_rounds_up_to_eight_words() {
        let mut cpu = cpu_with(&[0xEF0C_0000]);
        cpu.bus.write32(0x0200_0000, 0x5555_AAAA);
        cpu.regs.set(0, 0x0200_0000);
        cpu.regs.set(1, 0x0200_1000);
        cpu.regs.set(2, 1 | (1 << 24)); // 1 word requested -> 8 written
        cpu.step();
        assert_eq!(cpu.bus.read32(0x0200_101C), 0x5555_AAAA);
        assert_eq!(cpu.bus.read32(0x0200_1020), 0);
    }

    #[test]
    fn bit_unpack_expands_1bpp_to_4bpp() {
        let mut cpu = cpu_with(&[0xEF10_0000]);
        cpu.bus.write8(0x0200_0000, 0b1000_0001); // source byte
        // info block: len=1, srcW=1, dstW=4, offset=1
        cpu.bus.write16(0x0200_0100, 1);
        cpu.bus.write8(0x0200_0102, 1);
        cpu.bus.write8(0x0200_0103, 4);
        cpu.bus.write32(0x0200_0104, 1);
        cpu.regs.set(0, 0x0200_0000);
        cpu.regs.set(1, 0x0200_1000);
        cpu.regs.set(2, 0x0200_0100);
        cpu.step();
        // LSB-first: set bits become 1+1=2, clear bits stay 0 (no zero flag)
        assert_eq!(cpu.bus.read32(0x0200_1000), 0x2000_0002);
    }

    /// Place `blob` in EWRAM at 0x02000000 and run SWI `f` with dst as given.
    fn unpack(f: u32, blob: &[u8], dst: u32) -> Cpu {
        let mut cpu = cpu_with(&[0xEF00_0000 | (f << 16)]);
        for (i, &b) in blob.iter().enumerate() {
            cpu.bus.write8(0x0200_0000 + i as u32, b);
        }
        cpu.regs.set(0, 0x0200_0000);
        cpu.regs.set(1, dst);
        cpu.step();
        cpu
    }

    #[test]
    fn lz77_literal_plus_backreference() {
        // "AB" then a len-4 disp-2 backref => "ABABAB"
        let blob = [0x10, 6, 0, 0, 0b0010_0000, b'A', b'B', 0x10, 0x01];
        let mut cpu = unpack(0x11, &blob, 0x0200_1000);
        for (i, c) in b"ABABAB".iter().enumerate() {
            assert_eq!(cpu.bus.read8(0x0200_1000 + i as u32), *c);
        }
        // the Vram flavor writes the same bytes, halfword-wise, into VRAM
        let mut cpu = unpack(0x12, &blob, 0x0600_0000);
        assert_eq!(cpu.bus.read16(0x0600_0000), u16::from_le_bytes([b'A', b'B']));
        assert_eq!(cpu.bus.read16(0x0600_0004), u16::from_le_bytes([b'A', b'B']));
    }

    #[test]
    fn rl_run_and_literal() {
        // run of five 0x7C, then two literals
        let blob = [0x30, 7, 0, 0, 0x82, 0x7C, 0x01, b'x', b'y'];
        let mut cpu = unpack(0x14, &blob, 0x0200_1000);
        assert_eq!(cpu.bus.read8(0x0200_1004), 0x7C);
        assert_eq!(cpu.bus.read8(0x0200_1005), b'x');
        assert_eq!(cpu.bus.read8(0x0200_1006), b'y');
    }

    #[test]
    fn huffman_8bit_two_symbol_tree() {
        // header: 8-bit symbols, type 2, size 4; tree: size byte 1,
        // root (both children leaves, offset 0), leaves 'A' and 'B';
        // stream: one word, bits MSB-first: 0,1,1,0 -> "ABBA"
        let blob = [
            0x28, 4, 0, 0, // header
            1, 0xC0, b'A', b'B', // tree
            0x00, 0x00, 0x00, 0x60, // stream word 0x60000000 (LE)
        ];
        let mut cpu = unpack(0x13, &blob, 0x0200_1000);
        assert_eq!(cpu.bus.read32(0x0200_1000), u32::from_le_bytes(*b"ABBA"));
    }

    #[test]
    fn diff_filters_integrate() {
        let blob8 = [0x80, 3, 0, 0, 1, 1, 1];
        let mut cpu = unpack(0x16, &blob8, 0x0200_1000);
        assert_eq!(cpu.bus.read8(0x0200_1002), 3);
        let blob16 = [0x81, 4, 0, 0, 0x10, 0x00, 0x10, 0x00];
        let mut cpu = unpack(0x18, &blob16, 0x0200_1000);
        assert_eq!(cpu.bus.read16(0x0200_1002), 0x20);
    }

    #[test]
    fn soft_reset_restarts_at_the_cartridge() {
        let mut cpu = cpu_with(&[0xEF00_0000]);
        cpu.bus.write8(0x0300_7FFA, 0); // return-to-ROM flag
        cpu.regs.set(4, 0x1234);
        cpu.step();
        assert_eq!(cpu.exec_addr(), 0x0800_0000);
        assert_eq!(cpu.regs.get(13), 0x0300_7F00); // stacks re-set
        assert_eq!(cpu.bus.read32(0x0300_7FF8), 0); // BIOS area cleared
    }
}
