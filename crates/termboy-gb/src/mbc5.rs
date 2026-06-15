//! MBC5: 9-bit ROM banking (bank 0 mappable — no 0->1 translation),
//! 16 RAM banks, battery. The standard GBC-era mapper.

use crate::cartridge::Mbc;
use termboy_core::state::{Reader, StateError, Writer};

pub(crate) struct Mbc5 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    battery: bool,
    ram_enable: bool,
    rom_bank: u16, // 9 bits
    ram_bank: u8,  // 4 bits
}

impl Mbc5 {
    pub(crate) fn new(rom: Vec<u8>, ram_bytes: usize, battery: bool) -> Self {
        Self {
            rom,
            ram: vec![0; ram_bytes],
            battery,
            ram_enable: false,
            rom_bank: 1,
            ram_bank: 0,
        }
    }

    fn ram_index(&self, addr: u16) -> Option<usize> {
        if self.ram.is_empty() || !self.ram_enable {
            return None;
        }
        Some((self.ram_bank as usize * 0x2000 + (addr as usize - 0xA000)) % self.ram.len())
    }
}

impl Mbc for Mbc5 {
    fn read_rom(&self, addr: u16) -> u8 {
        let banks = (self.rom.len() / 0x4000).max(1);
        let bank = match addr {
            0x0000..=0x3FFF => 0,
            _ => self.rom_bank as usize % banks,
        };
        self.rom[bank * 0x4000 + (addr as usize & 0x3FFF)]
    }

    fn write_rom(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram_enable = value & 0x0F == 0x0A,
            0x2000..=0x2FFF => self.rom_bank = (self.rom_bank & 0x100) | value as u16,
            0x3000..=0x3FFF => self.rom_bank = (self.rom_bank & 0xFF) | ((value as u16 & 1) << 8),
            0x4000..=0x5FFF => self.ram_bank = value & 0x0F,
            _ => {}
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
        w.put_u16(self.rom_bank);
        w.put_u8(self.ram_bank);
    }

    fn deserialize(&mut self, r: &mut Reader) -> Result<(), StateError> {
        let n = self.ram.len();
        self.ram.copy_from_slice(r.get_bytes(n)?);
        self.ram_enable = r.get_bool()?;
        self.rom_bank = r.get_u16()?;
        self.ram_bank = r.get_u8()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mbc() -> Mbc5 {
        let mut rom = vec![0u8; 0x80_0000]; // 8 MiB = 512 banks
        for bank in 0..512usize {
            rom[bank * 0x4000] = (bank & 0xFF) as u8;
            rom[bank * 0x4000 + 1] = (bank >> 8) as u8;
        }
        Mbc5::new(rom, 0x8000, true)
    }

    #[test]
    fn bank_zero_is_mappable() {
        let mut m = mbc();
        m.write_rom(0x2000, 0x00); // unlike MBC1/3, bank 0 stays bank 0
        assert_eq!(m.read_rom(0x4000), 0);
        assert_eq!(m.read_rom(0x4001), 0);
    }

    #[test]
    fn nine_bit_banking() {
        let mut m = mbc();
        m.write_rom(0x2000, 0x34);
        m.write_rom(0x3000, 0x01); // bank 0x134
        assert_eq!(m.read_rom(0x4000), 0x34);
        assert_eq!(m.read_rom(0x4001), 0x01);
    }

    #[test]
    fn ram_banks_and_battery() {
        let mut m = mbc();
        m.write_rom(0x0000, 0x0A);
        m.write_rom(0x4000, 0x00);
        m.write_ram(0xA000, 0x11);
        m.write_rom(0x4000, 0x03);
        m.write_ram(0xA000, 0x33);
        m.write_rom(0x4000, 0x00);
        assert_eq!(m.read_ram(0xA000), 0x11);
        let saved = m.save(0).unwrap();
        let mut m2 = mbc();
        m2.load(&saved, 0);
        m2.write_rom(0x0000, 0x0A);
        m2.write_rom(0x4000, 0x03);
        assert_eq!(m2.read_ram(0xA000), 0x33);
    }
}
