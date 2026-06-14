//! Flash save memory, driven by the JEDEC-style command sequence GBA games
//! use to identify, erase, and program the chip. Two sizes: 64KB (one bank)
//! and 128KB (two 64KB banks selected by a bank-switch command).

const BANK: usize = 0x1_0000;

/// 512Kbit (64KB) vs 1Mbit (128KB). The reported device ID tells the game
/// which it is, so each size must answer with a matching, recognized ID.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FlashKind {
    M512,
    M1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Ready,
    Unlock1, // saw 0xAA at 0x5555
    Unlock2, // saw 0x55 at 0x2AAA
    Program, // 0xA0: the next write is one data byte
    Bank,    // 0xB0: the next write selects the 64KB bank
}

pub struct Flash {
    data: Vec<u8>,
    banks: usize,
    /// (manufacturer, device) returned in ID mode.
    id: (u8, u8),
    phase: Phase,
    id_mode: bool,
    /// 0x80 seen within an unlock; the next unlocked 0x10/0x30 erases.
    erase_armed: bool,
    bank: usize,
}

impl Flash {
    pub fn new(kind: FlashKind) -> Self {
        let (banks, id) = match kind {
            FlashKind::M512 => (1, (0x32, 0x1B)), // Panasonic MN63F805MNP
            FlashKind::M1 => (2, (0x62, 0x13)),   // Sanyo LE26FV10N1TS
        };
        Self {
            data: vec![0xFF; banks * BANK],
            banks,
            id,
            phase: Phase::Ready,
            id_mode: false,
            erase_armed: false,
            bank: 0,
        }
    }

    /// Raw contents for the .sav file.
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// Restore from a .sav (truncated/zero-padded to the chip size).
    pub fn load(&mut self, data: &[u8]) {
        let n = data.len().min(self.data.len());
        self.data[..n].copy_from_slice(&data[..n]);
    }

    pub fn read(&self, addr: u32) -> u8 {
        let off = (addr as usize) & 0xFFFF;
        if self.id_mode {
            match off {
                0 => self.id.0,
                1 => self.id.1,
                _ => self.data[self.bank * BANK + off],
            }
        } else {
            self.data[self.bank * BANK + off]
        }
    }

    pub fn write(&mut self, addr: u32, val: u8) {
        let off = (addr as usize) & 0xFFFF;
        match self.phase {
            Phase::Program => {
                self.data[self.bank * BANK + off] = val;
                self.phase = Phase::Ready;
            }
            Phase::Bank => {
                self.bank = (val as usize) & (self.banks - 1); // 1-bank chip: always 0
                self.phase = Phase::Ready;
            }
            Phase::Ready => {
                if off == 0x5555 && val == 0xAA {
                    self.phase = Phase::Unlock1;
                }
            }
            Phase::Unlock1 => {
                self.phase = if off == 0x2AAA && val == 0x55 {
                    Phase::Unlock2
                } else {
                    Phase::Ready
                };
            }
            Phase::Unlock2 => {
                self.phase = Phase::Ready;
                if off == 0x5555 {
                    match val {
                        0x90 => self.id_mode = true,
                        0xF0 => self.id_mode = false,
                        0x80 => self.erase_armed = true,
                        0xA0 => self.phase = Phase::Program,
                        0xB0 => self.phase = Phase::Bank,
                        0x10 if self.erase_armed => {
                            self.data.fill(0xFF); // whole-chip erase
                            self.erase_armed = false;
                        }
                        _ => {}
                    }
                } else if self.erase_armed && val == 0x30 {
                    // sector erase: the 4KB sector containing `off`
                    let base = self.bank * BANK + (off & 0xF000);
                    self.data[base..base + 0x1000].fill(0xFF);
                    self.erase_armed = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unlock(f: &mut Flash) {
        f.write(0x0E00_5555, 0xAA);
        f.write(0x0E00_2AAA, 0x55);
    }
    fn prog(f: &mut Flash, addr: u32, val: u8) {
        unlock(f);
        f.write(0x0E00_5555, 0xA0);
        f.write(addr, val);
    }
    fn switch_bank(f: &mut Flash, b: u8) {
        unlock(f);
        f.write(0x0E00_5555, 0xB0);
        f.write(0x0E00_0000, b);
    }

    #[test]
    fn reports_size_specific_id_only_in_id_mode() {
        let mut f = Flash::new(FlashKind::M1);
        assert_eq!(f.read(0x0E00_0000), 0xFF);
        unlock(&mut f);
        f.write(0x0E00_5555, 0x90);
        assert_eq!((f.read(0x0E00_0000), f.read(0x0E00_0001)), (0x62, 0x13));
        unlock(&mut f);
        f.write(0x0E00_5555, 0xF0);
        assert_eq!(f.read(0x0E00_0000), 0xFF);

        let mut f = Flash::new(FlashKind::M512);
        unlock(&mut f);
        f.write(0x0E00_5555, 0x90);
        assert_eq!((f.read(0x0E00_0000), f.read(0x0E00_0001)), (0x32, 0x1B));
    }

    #[test]
    fn program_writes_exactly_one_byte() {
        let mut f = Flash::new(FlashKind::M1);
        prog(&mut f, 0x0E00_1234, 0x42);
        assert_eq!(f.read(0x0E00_1234), 0x42);
        f.write(0x0E00_1235, 0x99); // a fresh command, not data
        assert_eq!(f.read(0x0E00_1235), 0xFF);
    }

    #[test]
    fn sector_erase_clears_one_4k_sector() {
        let mut f = Flash::new(FlashKind::M1);
        prog(&mut f, 0x0E00_0010, 0x11);
        prog(&mut f, 0x0E00_2000, 0x22);
        unlock(&mut f);
        f.write(0x0E00_5555, 0x80);
        unlock(&mut f);
        f.write(0x0E00_0000, 0x30);
        assert_eq!(f.read(0x0E00_0010), 0xFF);
        assert_eq!(f.read(0x0E00_2000), 0x22);
    }

    #[test]
    fn bank_switch_addresses_the_upper_64k_on_m1() {
        let mut f = Flash::new(FlashKind::M1);
        prog(&mut f, 0x0E00_0000, 0xAB);
        switch_bank(&mut f, 1);
        prog(&mut f, 0x0E00_0000, 0xCD);
        assert_eq!(f.read(0x0E00_0000), 0xCD);
        switch_bank(&mut f, 0);
        assert_eq!(f.read(0x0E00_0000), 0xAB);
    }

    #[test]
    fn m512_is_64k_single_bank_and_round_trips() {
        let mut f = Flash::new(FlashKind::M512);
        assert_eq!(f.bytes().len(), 0x1_0000);
        prog(&mut f, 0x0E00_0000, 0x7E);
        switch_bank(&mut f, 1); // ignored on a 1-bank chip
        assert_eq!(f.read(0x0E00_0000), 0x7E);
        let saved = f.bytes().to_vec();
        let mut g = Flash::new(FlashKind::M512);
        g.load(&saved);
        assert_eq!(g.read(0x0E00_0000), 0x7E);
    }
}
