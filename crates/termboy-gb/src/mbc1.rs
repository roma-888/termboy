//! MBC1: 5+2-bit ROM banking, optional banked RAM, two addressing modes.
//! MBC1M multicart wiring is not supported (no targeted game uses it).

use crate::cartridge::Mbc;
use termboy_core::state::{Reader, StateError, Writer};

pub(crate) struct Mbc1 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    battery: bool,
    ram_enable: bool,
    bank_lo: u8, // 5 bits, never 0
    bank2: u8,   // 2 bits
    mode: bool,  // false = simple, true = advanced
}

impl Mbc1 {
    pub(crate) fn new(rom: Vec<u8>, ram_bytes: usize, battery: bool) -> Self {
        Self {
            rom,
            ram: vec![0; ram_bytes],
            battery,
            ram_enable: false,
            bank_lo: 1,
            bank2: 0,
            mode: false,
        }
    }

    fn rom_banks(&self) -> usize {
        (self.rom.len() / 0x4000).max(1)
    }

    fn ram_index(&self, addr: u16) -> Option<usize> {
        if self.ram.is_empty() || !self.ram_enable {
            return None;
        }
        let bank = if self.mode { self.bank2 as usize } else { 0 };
        Some((bank * 0x2000 + (addr as usize - 0xA000)) % self.ram.len())
    }
}

impl Mbc for Mbc1 {
    fn read_rom(&self, addr: u16) -> u8 {
        let bank = match addr {
            0x0000..=0x3FFF if self.mode => ((self.bank2 as usize) << 5) % self.rom_banks(),
            0x0000..=0x3FFF => 0,
            _ => (((self.bank2 as usize) << 5) | self.bank_lo as usize) % self.rom_banks(),
        };
        self.rom[bank * 0x4000 + (addr as usize & 0x3FFF)]
    }

    fn write_rom(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram_enable = value & 0x0F == 0x0A,
            0x2000..=0x3FFF => self.bank_lo = if value & 0x1F == 0 { 1 } else { value & 0x1F },
            0x4000..=0x5FFF => self.bank2 = value & 0x03,
            _ => self.mode = value & 0x01 != 0,
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        self.ram_index(addr).map_or(0xFF, |i| self.ram[i])
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if let Some(i) = self.ram_index(addr) {
            self.ram[i] = value;
        }
    }

    fn save(&self, _now: u64) -> Option<Vec<u8>> {
        (self.battery && !self.ram.is_empty()).then(|| self.ram.clone())
    }

    fn load(&mut self, data: &[u8], _now: u64) {
        let n = data.len().min(self.ram.len());
        self.ram[..n].copy_from_slice(&data[..n]);
    }

    fn serialize(&self, w: &mut Writer) {
        w.put_bytes(&self.ram);
        w.put_bool(self.ram_enable);
        w.put_u8(self.bank_lo);
        w.put_u8(self.bank2);
        w.put_bool(self.mode);
    }

    fn deserialize(&mut self, r: &mut Reader) -> Result<(), StateError> {
        let n = self.ram.len();
        self.ram.copy_from_slice(r.get_bytes(n)?);
        self.ram_enable = r.get_bool()?;
        self.bank_lo = r.get_u8()?;
        self.bank2 = r.get_u8()?;
        self.mode = r.get_bool()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 MiB ROM (64 banks); first byte of each bank = its bank number.
    fn mbc(ram_bytes: usize, battery: bool) -> Mbc1 {
        let mut rom = vec![0u8; 64 * 0x4000];
        for bank in 0..64 {
            rom[bank * 0x4000] = bank as u8;
        }
        Mbc1::new(rom, ram_bytes, battery)
    }

    #[test]
    fn bank_zero_translates_to_one() {
        let mut m = mbc(0, false);
        m.write_rom(0x2000, 0x00);
        assert_eq!(m.read_rom(0x4000), 1);
        m.write_rom(0x2000, 0x1F);
        assert_eq!(m.read_rom(0x4000), 0x1F);
    }

    #[test]
    fn bank2_extends_rom_bank_in_switchable_region() {
        let mut m = mbc(0, false);
        m.write_rom(0x2000, 0x01);
        m.write_rom(0x4000, 0x01); // bank2 = 1 -> bank 0x21
        assert_eq!(m.read_rom(0x4000), 0x21);
        // bank 0x20's low-5 bits are 0 -> writing 0x20 reads bank 0x21 too
        m.write_rom(0x2000, 0x20);
        assert_eq!(m.read_rom(0x4000), 0x21);
    }

    #[test]
    fn mode1_remaps_fixed_region() {
        let mut m = mbc(0, false);
        m.write_rom(0x4000, 0x01); // bank2 = 1
        assert_eq!(m.read_rom(0x0000), 0); // mode 0: fixed bank 0
        m.write_rom(0x6000, 0x01); // mode 1
        assert_eq!(m.read_rom(0x0000), 0x20); // bank2<<5
    }

    #[test]
    fn ram_requires_enable_and_banks_in_mode1() {
        let mut m = mbc(0x8000, true); // 32 KB = 4 banks
        m.write_ram(0xA000, 0x55);
        assert_eq!(m.read_ram(0xA000), 0xFF); // disabled: open bus, write dropped
        m.write_rom(0x0000, 0x0A); // enable
        m.write_ram(0xA000, 0x55);
        assert_eq!(m.read_ram(0xA000), 0x55);
        m.write_rom(0x6000, 0x01); // mode 1
        m.write_rom(0x4000, 0x02); // RAM bank 2
        assert_eq!(m.read_ram(0xA000), 0x00); // different bank
        m.write_ram(0xA000, 0xAA);
        m.write_rom(0x4000, 0x00);
        assert_eq!(m.read_ram(0xA000), 0x55); // bank 0 intact
    }

    #[test]
    fn battery_save_roundtrip() {
        let mut m = mbc(0x2000, true);
        m.write_rom(0x0000, 0x0A);
        m.write_ram(0xA123, 0x77);
        let saved = m.save(0).expect("battery cart saves");
        let mut m2 = mbc(0x2000, true);
        m2.load(&saved, 0);
        m2.write_rom(0x0000, 0x0A);
        assert_eq!(m2.read_ram(0xA123), 0x77);
        assert!(mbc(0x2000, false).save(0).is_none()); // no battery, no save
    }
}
