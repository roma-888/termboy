//! Cartridge save backend, detected from the ROM. SRAM and Flash are
//! byte-mapped into region 0x0E/0x0F; EEPROM (region 0x0D, DMA-serial) is a
//! separate follow-up and detects as `None` for now.

use crate::eeprom::Eeprom;
use crate::flash::{Flash, FlashKind};

const SRAM_SIZE: usize = 0x8000; // 32KB

pub enum Save {
    None,
    Sram(Box<[u8; SRAM_SIZE]>),
    Flash(Flash),
    Eeprom(Eeprom),
}

impl Save {
    /// Pick the backend from the SDK signature strings the linker embeds in
    /// the ROM (word-aligned). Size-specific Flash IDs are checked before the
    /// generic one; EEPROM and "no signature" fall through to `None`.
    pub fn detect(rom: &[u8]) -> Save {
        if contains(rom, b"FLASH1M_V") {
            Save::Flash(Flash::new(FlashKind::M1))
        } else if contains(rom, b"FLASH512_V") || contains(rom, b"FLASH_V") {
            Save::Flash(Flash::new(FlashKind::M512))
        } else if contains(rom, b"SRAM_V") || contains(rom, b"SRAM_F_V") {
            Save::Sram(Box::new([0xFF; SRAM_SIZE]))
        } else if contains(rom, b"EEPROM_V") {
            Save::Eeprom(Eeprom::new())
        } else {
            Save::None
        }
    }

    pub fn read(&self, addr: u32) -> u8 {
        match self {
            Save::None | Save::Eeprom(_) => 0xFF,
            Save::Sram(d) => d[(addr as usize) & (SRAM_SIZE - 1)],
            Save::Flash(f) => f.read(addr),
        }
    }

    pub fn write(&mut self, addr: u32, val: u8) {
        match self {
            Save::None | Save::Eeprom(_) => {}
            Save::Sram(d) => d[(addr as usize) & (SRAM_SIZE - 1)] = val,
            Save::Flash(f) => f.write(addr, val),
        }
    }

    /// Bytes for the .sav file (None for carts with no save memory).
    pub fn backup(&self) -> Option<Vec<u8>> {
        match self {
            Save::None => None,
            Save::Sram(d) => Some(d.to_vec()),
            Save::Flash(f) => Some(f.bytes().to_vec()),
            Save::Eeprom(e) => Some(e.backup()),
        }
    }

    pub fn restore(&mut self, data: &[u8]) {
        match self {
            Save::None => {}
            Save::Sram(d) => {
                let n = data.len().min(SRAM_SIZE);
                d[..n].copy_from_slice(&data[..n]);
            }
            Save::Flash(f) => f.load(data),
            Save::Eeprom(e) => e.restore(data),
        }
    }

    /// The EEPROM device, for the bus's 0x0D bit path (None otherwise).
    pub fn eeprom(&mut self) -> Option<&mut Eeprom> {
        match self {
            Save::Eeprom(e) => Some(e),
            _ => None,
        }
    }

    /// Tell an EEPROM cart a DMA transfer of `count` units is starting,
    /// `to_eeprom` = command/write direction. No-op for other save types.
    pub fn eeprom_begin(&mut self, count: u32, to_eeprom: bool) {
        if let Save::Eeprom(e) = self {
            e.begin(count, to_eeprom);
        }
    }
}

/// True if `sig` appears in `rom` at a 4-byte boundary.
fn contains(rom: &[u8], sig: &[u8]) -> bool {
    (0..rom.len()).step_by(4).any(|i| rom[i..].starts_with(sig))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ROM that embeds `sig` at a word boundary (offset 0x100).
    fn rom_with(sig: &[u8]) -> Vec<u8> {
        let mut rom = vec![0u8; 0x200];
        rom[0x100..0x100 + sig.len()].copy_from_slice(sig);
        rom
    }

    #[test]
    fn detects_each_type_from_its_signature() {
        assert!(matches!(Save::detect(&rom_with(b"FLASH1M_V")), Save::Flash(_)));
        assert!(matches!(Save::detect(&rom_with(b"SRAM_V")), Save::Sram(_)));
        assert!(matches!(Save::detect(&rom_with(b"EEPROM_V")), Save::Eeprom(_)));
        assert!(matches!(Save::detect(&[0u8; 0x200]), Save::None));
        assert_eq!(Save::detect(&rom_with(b"FLASH1M_V")).backup().unwrap().len(), 0x2_0000);
        assert_eq!(Save::detect(&rom_with(b"FLASH512_V")).backup().unwrap().len(), 0x1_0000);
    }

    #[test]
    fn sram_reads_writes_and_round_trips() {
        let mut s = Save::Sram(Box::new([0xFF; SRAM_SIZE]));
        s.write(0x0E00_0005, 0x42);
        assert_eq!(s.read(0x0E00_0005), 0x42);
        assert_eq!(s.read(0x0E01_0005), 0x42); // mirrors every 32KB
        let saved = s.backup().unwrap();
        assert_eq!(saved.len(), SRAM_SIZE);
        let mut t = Save::Sram(Box::new([0xFF; SRAM_SIZE]));
        t.restore(&saved);
        assert_eq!(t.read(0x0E00_0005), 0x42);
    }

    #[test]
    fn detects_eeprom_and_exposes_the_device() {
        let mut s = Save::detect(&rom_with(b"EEPROM_V"));
        assert!(matches!(s, Save::Eeprom(_)));
        // the 0x0E/0x0F byte path is inert for an EEPROM cart
        s.write(0x0E00_0000, 0x12);
        assert_eq!(s.read(0x0E00_0000), 0xFF);
        // the device is reachable for the DMA/bit path
        assert!(s.eeprom().is_some());
    }

    #[test]
    fn none_is_open_bus_and_has_no_backup() {
        let mut s = Save::None;
        s.write(0x0E00_0000, 0x12); // ignored
        assert_eq!(s.read(0x0E00_0000), 0xFF);
        assert!(s.backup().is_none());
    }
}
