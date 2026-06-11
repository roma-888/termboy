//! Cartridge loading and mapper (MBC) dispatch.
//! M1 implements RomOnly; MBC1/MBC3 land in Milestone 4 as new `Mbc` impls.

use std::fmt;

#[derive(Debug)]
pub enum CartError {
    TooSmall,
    Unsupported { code: u8, name: &'static str },
}

impl fmt::Display for CartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CartError::TooSmall => write!(f, "ROM is too small to contain a cartridge header"),
            CartError::Unsupported { code, name } => {
                write!(f, "{name} (header type {code:#04X}) is not yet supported")
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

struct RomOnly {
    rom: Vec<u8>,
}

impl Mbc for RomOnly {
    fn read_rom(&self, addr: u16) -> u8 {
        self.rom.get(addr as usize).copied().unwrap_or(0xFF)
    }
    fn write_rom(&mut self, _addr: u16, _value: u8) {}
    fn read_ram(&self, _addr: u16) -> u8 { 0xFF }
    fn write_ram(&mut self, _addr: u16, _value: u8) {}
    fn save(&self, _now: u64) -> Option<Vec<u8>> { None }
    fn load(&mut self, _data: &[u8], _now: u64) {}
}

pub struct Cartridge {
    mbc: Box<dyn Mbc>,
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
        let mbc: Box<dyn Mbc> = match rom[0x147] {
            0x00 => Box::new(RomOnly { rom }),
            // A 32 KB MBC1 ROM never banks, so it behaves identically to
            // ROM-only. Blargg's test ROMs declare MBC1 despite being 32 KB.
            0x01 if rom.len() <= 0x8000 => Box::new(RomOnly { rom }),
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
        Ok(Self { mbc })
    }

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
        let err = Cartridge::new(rom_with_mapper(0x19)).unwrap_err();
        assert!(err.to_string().contains("MBC5"));
    }

    #[test]
    fn truncated_rom_is_an_error() {
        assert!(Cartridge::new(vec![0u8; 0x100]).is_err());
    }
}
