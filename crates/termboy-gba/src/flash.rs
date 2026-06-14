//! Flash save memory: 128KB in two 64KB banks, driven by the JEDEC-style
//! command sequence GBA games use to identify, erase, and program the chip.
//! In-memory only — persisting to a .sav file lands in G5.

const BANK: usize = 0x1_0000;
const SIZE: usize = 2 * BANK;

// Sanyo LE26FV10N1TS (128KB) — the chip the Pokémon FireRed family probes for.
const MANUFACTURER: u8 = 0x62;
const DEVICE: u8 = 0x13;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Ready,
    Unlock1, // saw 0xAA at 0x5555
    Unlock2, // saw 0x55 at 0x2AAA
    Program, // 0xA0: the next write is one data byte
    Bank,    // 0xB0: the next write selects the 64KB bank
}

pub struct Flash {
    data: Box<[u8; SIZE]>,
    phase: Phase,
    id_mode: bool,
    /// 0x80 seen within an unlock; the next unlocked 0x10/0x30 erases.
    erase_armed: bool,
    bank: usize,
}

impl Flash {
    pub fn new() -> Self {
        Self {
            data: Box::new([0xFF; SIZE]),
            phase: Phase::Ready,
            id_mode: false,
            erase_armed: false,
            bank: 0,
        }
    }

    pub fn read(&self, addr: u32) -> u8 {
        let off = (addr as usize) & 0xFFFF;
        if self.id_mode {
            match off {
                0 => MANUFACTURER,
                1 => DEVICE,
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
                self.bank = (val as usize) & 1;
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

impl Default for Flash {
    fn default() -> Self {
        Self::new()
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
        f.write(0x0E00_0000, b as u8);
    }

    #[test]
    fn reports_chip_id_only_in_id_mode() {
        let mut f = Flash::new();
        assert_eq!(f.read(0x0E00_0000), 0xFF); // data, not ID
        unlock(&mut f);
        f.write(0x0E00_5555, 0x90); // enter ID
        assert_eq!(f.read(0x0E00_0000), 0x62);
        assert_eq!(f.read(0x0E00_0001), 0x13);
        unlock(&mut f);
        f.write(0x0E00_5555, 0xF0); // exit
        assert_eq!(f.read(0x0E00_0000), 0xFF);
    }

    #[test]
    fn program_writes_exactly_one_byte() {
        let mut f = Flash::new();
        prog(&mut f, 0x0E00_1234, 0x42);
        assert_eq!(f.read(0x0E00_1234), 0x42);
        // the following raw write is a fresh command, not flash data
        f.write(0x0E00_1235, 0x99);
        assert_eq!(f.read(0x0E00_1235), 0xFF);
    }

    #[test]
    fn sector_erase_clears_one_4k_sector() {
        let mut f = Flash::new();
        prog(&mut f, 0x0E00_0010, 0x11);
        prog(&mut f, 0x0E00_2000, 0x22);
        unlock(&mut f);
        f.write(0x0E00_5555, 0x80); // erase setup
        unlock(&mut f);
        f.write(0x0E00_0000, 0x30); // sector erase @ 0
        assert_eq!(f.read(0x0E00_0010), 0xFF);
        assert_eq!(f.read(0x0E00_2000), 0x22); // untouched
    }

    #[test]
    fn chip_erase_clears_both_banks() {
        let mut f = Flash::new();
        prog(&mut f, 0x0E00_0000, 0x01);
        switch_bank(&mut f, 1);
        prog(&mut f, 0x0E00_0000, 0x02);
        switch_bank(&mut f, 0);
        unlock(&mut f);
        f.write(0x0E00_5555, 0x80);
        unlock(&mut f);
        f.write(0x0E00_5555, 0x10); // chip erase
        assert_eq!(f.read(0x0E00_0000), 0xFF);
        switch_bank(&mut f, 1);
        assert_eq!(f.read(0x0E00_0000), 0xFF);
    }

    #[test]
    fn bank_switch_addresses_the_upper_64k() {
        let mut f = Flash::new();
        prog(&mut f, 0x0E00_0000, 0xAB);
        switch_bank(&mut f, 1);
        prog(&mut f, 0x0E00_0000, 0xCD);
        assert_eq!(f.read(0x0E00_0000), 0xCD);
        switch_bank(&mut f, 0);
        assert_eq!(f.read(0x0E00_0000), 0xAB);
    }
}
