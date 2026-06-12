//! Cartridge loading and mapper (MBC) dispatch.
//! M1 implements RomOnly; MBC1/MBC3 land in Milestone 4 as new `Mbc` impls.

use std::fmt;

use crate::mbc1::Mbc1;
use crate::mbc3::Mbc3;
use crate::mbc5::Mbc5;

#[derive(Debug)]
pub enum CartError {
    TooSmall,
    Unsupported { code: u8, name: &'static str },
    /// Header claims no mapper but the ROM is too big to work without one.
    BadDump { code: u8, size_kb: usize },
}

impl fmt::Display for CartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CartError::TooSmall => write!(f, "ROM is too small to contain a cartridge header"),
            CartError::Unsupported { code, name } => {
                write!(f, "{name} (header type {code:#04X}) is not yet supported")
            }
            CartError::BadDump { code, size_kb } => {
                write!(
                    f,
                    "header type {code:#04X} declares no mapper, but the ROM is {size_kb} KB — \
                     impossible on real hardware; this is likely a bad dump, try a clean ROM"
                )
            }
        }
    }
}

impl std::error::Error for CartError {}

pub trait Mbc {
    fn read_rom(&self, addr: u16) -> u8;
    fn write_rom(&mut self, addr: u16, value: u8);
    fn read_ram(&self, addr: u16) -> u8;
    fn write_ram(&mut self, addr: u16, value: u8);
    /// Persistable battery state (RAM, + RTC for MBC3). None without battery.
    /// `now` = unix seconds, stamped into RTC saves.
    fn save(&self, now: u64) -> Option<Vec<u8>>;
    fn load(&mut self, data: &[u8], now: u64);
    /// Advance emulated time — MBC3's RTC counts T-cycles. Default no-op.
    fn tick(&mut self, tcycles: u32) {
        let _ = tcycles;
    }
}

/// RAM size in bytes from header byte 0x149.
pub(crate) fn ram_size(code: u8) -> usize {
    match code {
        0x01 => 0x800,
        0x02 => 0x2000,
        0x03 => 0x8000,
        0x04 => 0x20000,
        0x05 => 0x10000,
        _ => 0,
    }
}

/// Mapper-less cartridge: types 0x00 (no RAM) and 0x08/0x09 (RAM, unbanked).
struct RomOnly {
    rom: Vec<u8>,
    ram: Vec<u8>,
    battery: bool,
}

impl Mbc for RomOnly {
    fn read_rom(&self, addr: u16) -> u8 {
        self.rom.get(addr as usize).copied().unwrap_or(0xFF)
    }
    fn write_rom(&mut self, _addr: u16, _value: u8) {}
    fn read_ram(&self, addr: u16) -> u8 {
        if self.ram.is_empty() {
            return 0xFF;
        }
        let i = (addr as usize - 0xA000) % self.ram.len();
        self.ram[i]
    }
    fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.ram.is_empty() {
            let i = (addr as usize - 0xA000) % self.ram.len();
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
}

pub struct Cartridge {
    mbc: Box<dyn Mbc>,
    cgb: bool,
}

impl fmt::Debug for Cartridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Cartridge")
    }
}

impl Cartridge {
    pub fn new(rom: Vec<u8>) -> Result<Self, CartError> {
        if rom.len() < 0x150 {
            return Err(CartError::TooSmall);
        }
        let cgb = rom[0x143] & 0x80 != 0;
        let mbc: Box<dyn Mbc> = match rom[0x147] {
            code @ (0x00 | 0x08 | 0x09) => {
                if rom.len() > 0x8000 {
                    return Err(CartError::BadDump { code, size_kb: rom.len() / 1024 });
                }
                let ram = vec![0; ram_size(rom[0x149])];
                let battery = code == 0x09;
                Box::new(RomOnly { rom, ram, battery })
            }
            0x01..=0x03 => {
                let (battery, ram) = (rom[0x147] == 0x03, ram_size(rom[0x149]));
                Box::new(Mbc1::new(rom, ram, battery))
            }
            0x0F..=0x13 => {
                let battery = matches!(rom[0x147], 0x0F | 0x10 | 0x13);
                let has_rtc = matches!(rom[0x147], 0x0F | 0x10);
                let ram = ram_size(rom[0x149]);
                Box::new(Mbc3::new(rom, ram, battery, has_rtc))
            }
            0x19..=0x1E => {
                let battery = matches!(rom[0x147], 0x1B | 0x1E);
                let ram = ram_size(rom[0x149]);
                Box::new(Mbc5::new(rom, ram, battery))
            }
            code => {
                let name = match code {
                    0x01..=0x03 => "MBC1",
                    0x05 | 0x06 => "MBC2",
                    0x0F..=0x13 => "MBC3",
                    0x19..=0x1E => "MBC5",
                    _ => "this mapper",
                };
                return Err(CartError::Unsupported { code, name });
            }
        };
        Ok(Self { mbc, cgb })
    }

    /// Header requests Game Boy Color mode (0x143 bit 7).
    pub fn cgb(&self) -> bool { self.cgb }
    pub fn read_rom(&self, addr: u16) -> u8 { self.mbc.read_rom(addr) }
    pub fn write_rom(&mut self, addr: u16, value: u8) { self.mbc.write_rom(addr, value) }
    pub fn read_ram(&self, addr: u16) -> u8 { self.mbc.read_ram(addr) }
    pub fn write_ram(&mut self, addr: u16, value: u8) { self.mbc.write_ram(addr, value) }
    pub fn save(&self, now: u64) -> Option<Vec<u8>> { self.mbc.save(now) }
    pub fn load(&mut self, data: &[u8], now: u64) { self.mbc.load(data, now) }
    pub fn tick(&mut self, tcycles: u32) { self.mbc.tick(tcycles) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom_with_mapper(byte: u8) -> Vec<u8> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = byte;
        rom
    }

    #[test]
    fn rom_only_reads_back_rom_and_ignores_writes() {
        let mut rom = rom_with_mapper(0x00);
        rom[0x1234] = 0xAB;
        let mut cart = Cartridge::new(rom).unwrap();
        assert_eq!(cart.read_rom(0x1234), 0xAB);
        cart.write_rom(0x1234, 0xFF); // must be ignored
        assert_eq!(cart.read_rom(0x1234), 0xAB);
        assert_eq!(cart.read_ram(0xA000), 0xFF); // no RAM -> open bus
        assert!(cart.save(0).is_none());
    }

    #[test]
    fn unsupported_mapper_is_a_named_error() {
        let err = Cartridge::new(rom_with_mapper(0x05)).unwrap_err();
        assert!(err.to_string().contains("MBC2"));
    }

    #[test]
    fn rom_ram_type_08_has_unbanked_ram() {
        let mut rom = rom_with_mapper(0x08);
        rom[0x149] = 0x02; // 8 KB RAM
        let mut cart = Cartridge::new(rom).unwrap();
        cart.write_ram(0xA042, 0x5A);
        assert_eq!(cart.read_ram(0xA042), 0x5A);
        assert!(cart.save(0).is_none()); // 0x08 has no battery
    }

    #[test]
    fn rom_ram_battery_type_09_saves() {
        let mut rom = rom_with_mapper(0x09);
        rom[0x149] = 0x02;
        let mut cart = Cartridge::new(rom).unwrap();
        cart.write_ram(0xA000, 0x77);
        let saved = cart.save(0).expect("battery");
        let mut rom2 = rom_with_mapper(0x09);
        rom2[0x149] = 0x02;
        let mut cart2 = Cartridge::new(rom2).unwrap();
        cart2.load(&saved, 0);
        assert_eq!(cart2.read_ram(0xA000), 0x77);
    }

    #[test]
    fn oversized_mapperless_rom_is_a_bad_dump_error() {
        let mut rom = vec![0u8; 0x10000]; // 64 KB can't be mapper-less
        rom[0x147] = 0x08;
        let err = Cartridge::new(rom).unwrap_err();
        assert!(err.to_string().contains("bad dump"));
    }

    #[test]
    fn cgb_flag_from_header() {
        let mut rom = rom_with_mapper(0x00);
        assert!(!Cartridge::new(rom.clone()).unwrap().cgb());
        rom[0x143] = 0x80;
        assert!(Cartridge::new(rom.clone()).unwrap().cgb());
        rom[0x143] = 0xC0;
        assert!(Cartridge::new(rom).unwrap().cgb());
    }

    #[test]
    fn truncated_rom_is_an_error() {
        assert!(Cartridge::new(vec![0u8; 0x100]).is_err());
    }
}
