//! MBC3: 7-bit ROM banking, 4 RAM banks, battery, real-time clock.
//! MBC30's 8 RAM banks are not supported (no targeted game uses them).

use crate::cartridge::Mbc;

#[derive(Default)]
struct Rtc {
    subsec: u32, // T-cycles toward the next second
    secs: u8,
    mins: u8,
    hours: u8,
    days: u16, // 9 bits
    halt: bool,
    carry: bool,
    latched: [u8; 5],
}

impl Rtc {
    fn tick(&mut self, tcycles: u32) {
        if self.halt {
            return;
        }
        self.subsec += tcycles;
        while self.subsec >= 4_194_304 {
            self.subsec -= 4_194_304;
            self.advance_seconds(1);
        }
    }

    fn advance_seconds(&mut self, n: u64) {
        if n == 0 {
            return;
        }
        let total = self.secs as u64
            + self.mins as u64 * 60
            + self.hours as u64 * 3600
            + self.days as u64 * 86_400
            + n;
        self.secs = (total % 60) as u8;
        self.mins = (total / 60 % 60) as u8;
        self.hours = (total / 3600 % 24) as u8;
        let days = total / 86_400;
        if days > 0x1FF {
            self.carry = true; // sticky until written
        }
        self.days = (days & 0x1FF) as u16;
    }

    fn dh(&self) -> u8 {
        ((self.days >> 8) as u8 & 1) | ((self.halt as u8) << 6) | ((self.carry as u8) << 7)
    }

    fn latch(&mut self) {
        self.latched = [self.secs, self.mins, self.hours, self.days as u8, self.dh()];
    }

    fn read(&self, reg: u8) -> u8 {
        self.latched[(reg - 0x08) as usize]
    }

    fn write(&mut self, reg: u8, value: u8) {
        match reg {
            0x08 => {
                self.secs = value & 0x3F;
                self.subsec = 0;
            }
            0x09 => self.mins = value & 0x3F,
            0x0A => self.hours = value & 0x1F,
            0x0B => self.days = (self.days & 0x100) | value as u16,
            _ => {
                self.days = (self.days & 0xFF) | ((value as u16 & 1) << 8);
                self.halt = value & 0x40 != 0;
                self.carry = value & 0x80 != 0;
            }
        }
    }
}

pub(crate) struct Mbc3 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    battery: bool,
    has_rtc: bool,
    enable: bool,
    rom_bank: u8, // 7 bits, never 0
    select: u8,   // 0x00-0x03 RAM bank, 0x08-0x0C RTC register
    rtc: Rtc,
    latch_pending: bool,
}

impl Mbc3 {
    pub(crate) fn new(rom: Vec<u8>, ram_bytes: usize, battery: bool, has_rtc: bool) -> Self {
        Self {
            rom,
            ram: vec![0; ram_bytes],
            battery,
            has_rtc,
            enable: false,
            rom_bank: 1,
            select: 0,
            rtc: Rtc::default(),
            latch_pending: false,
        }
    }

    fn ram_index(&self, addr: u16) -> Option<usize> {
        if self.ram.is_empty() || !self.enable || self.select > 0x03 {
            return None;
        }
        Some((self.select as usize * 0x2000 + (addr as usize - 0xA000)) % self.ram.len())
    }

    fn rtc_selected(&self) -> bool {
        self.has_rtc && self.enable && (0x08..=0x0C).contains(&self.select)
    }
}

impl Mbc for Mbc3 {
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
            0x0000..=0x1FFF => self.enable = value & 0x0F == 0x0A,
            0x2000..=0x3FFF => self.rom_bank = if value & 0x7F == 0 { 1 } else { value & 0x7F },
            0x4000..=0x5FFF => self.select = value & 0x0F,
            _ => {
                // latch sequence: write 0x00 then 0x01
                if value == 0x00 {
                    self.latch_pending = true;
                } else {
                    if value == 0x01 && self.latch_pending {
                        self.rtc.latch();
                    }
                    self.latch_pending = false;
                }
            }
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        if let Some(i) = self.ram_index(addr) {
            return self.ram[i];
        }
        if self.rtc_selected() {
            return self.rtc.read(self.select);
        }
        0xFF
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if let Some(i) = self.ram_index(addr) {
            self.ram[i] = value;
        } else if self.rtc_selected() {
            self.rtc.write(self.select, value);
        }
    }

    fn save(&self, now: u64) -> Option<Vec<u8>> {
        if !self.battery {
            return None;
        }
        let mut out = self.ram.clone();
        if self.has_rtc {
            out.extend_from_slice(&[
                self.rtc.secs,
                self.rtc.mins,
                self.rtc.hours,
                self.rtc.days as u8,
                self.rtc.dh(),
            ]);
            out.extend_from_slice(&now.to_le_bytes());
        }
        (!out.is_empty()).then_some(out)
    }

    fn load(&mut self, data: &[u8], now: u64) {
        let n = data.len().min(self.ram.len());
        self.ram[..n].copy_from_slice(&data[..n]);
        if self.has_rtc && data.len() >= self.ram.len() + 13 {
            let r = &data[self.ram.len()..];
            self.rtc.secs = r[0] & 0x3F;
            self.rtc.mins = r[1] & 0x3F;
            self.rtc.hours = r[2] & 0x1F;
            self.rtc.days = r[3] as u16 | ((r[4] as u16 & 1) << 8);
            self.rtc.halt = r[4] & 0x40 != 0;
            self.rtc.carry = r[4] & 0x80 != 0;
            let ts = u64::from_le_bytes(r[5..13].try_into().unwrap());
            if !self.rtc.halt {
                self.rtc.advance_seconds(now.saturating_sub(ts));
            }
        }
    }

    fn tick(&mut self, tcycles: u32) {
        if self.has_rtc {
            self.rtc.tick(tcycles);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mbc() -> Mbc3 {
        let mut rom = vec![0u8; 128 * 0x4000]; // 2 MiB, Pokémon-sized
        for bank in 0..128 {
            rom[bank * 0x4000] = bank as u8;
        }
        Mbc3::new(rom, 0x8000, true, false)
    }

    #[test]
    fn seven_bit_rom_banking_zero_becomes_one() {
        let mut m = mbc();
        m.write_rom(0x2000, 0x00);
        assert_eq!(m.read_rom(0x4000), 1);
        m.write_rom(0x2000, 0x7F);
        assert_eq!(m.read_rom(0x4000), 0x7F);
        assert_eq!(m.read_rom(0x0000), 0); // fixed region stays bank 0
    }

    #[test]
    fn ram_banks_switch_directly() {
        let mut m = mbc();
        m.write_rom(0x0000, 0x0A);
        m.write_rom(0x4000, 0x00);
        m.write_ram(0xA000, 0x11);
        m.write_rom(0x4000, 0x03);
        m.write_ram(0xA000, 0x33);
        m.write_rom(0x4000, 0x00);
        assert_eq!(m.read_ram(0xA000), 0x11);
        m.write_rom(0x4000, 0x03);
        assert_eq!(m.read_ram(0xA000), 0x33);
    }

    #[test]
    fn disabled_ram_is_open_bus() {
        let mut m = mbc();
        m.write_ram(0xA000, 0x55);
        assert_eq!(m.read_ram(0xA000), 0xFF);
    }

    #[test]
    fn battery_save_roundtrip_without_rtc() {
        let mut m = mbc();
        m.write_rom(0x0000, 0x0A);
        m.write_ram(0xA000, 0x99);
        let saved = m.save(1000).unwrap();
        assert_eq!(saved.len(), 0x8000); // RAM only, no RTC footer
        let mut m2 = mbc();
        m2.load(&saved, 2000);
        m2.write_rom(0x0000, 0x0A);
        assert_eq!(m2.read_ram(0xA000), 0x99);
    }

    // ---- RTC ----

    const SEC: u32 = 4_194_304;

    fn rtc_mbc() -> Mbc3 {
        let mut m = Mbc3::new(vec![0u8; 0x8000], 0x2000, true, true);
        m.write_rom(0x0000, 0x0A); // enable
        m
    }

    fn latch(m: &mut Mbc3) {
        m.write_rom(0x6000, 0x00);
        m.write_rom(0x6000, 0x01);
    }

    fn read_rtc(m: &mut Mbc3, reg: u8) -> u8 {
        m.write_rom(0x4000, reg);
        m.read_ram(0xA000)
    }

    #[test]
    fn rtc_counts_emulated_seconds_with_cascade() {
        let mut m = rtc_mbc();
        // set 23:59:59 day 0 via register writes
        m.write_rom(0x4000, 0x08);
        m.write_ram(0xA000, 59);
        m.write_rom(0x4000, 0x09);
        m.write_ram(0xA000, 59);
        m.write_rom(0x4000, 0x0A);
        m.write_ram(0xA000, 23);
        for _ in 0..32 {
            m.tick(SEC / 32);
        }
        latch(&mut m);
        assert_eq!(read_rtc(&mut m, 0x08), 0);
        assert_eq!(read_rtc(&mut m, 0x09), 0);
        assert_eq!(read_rtc(&mut m, 0x0A), 0);
        assert_eq!(read_rtc(&mut m, 0x0B), 1); // day rolled over
    }

    #[test]
    fn rtc_latch_freezes_reads_until_next_latch() {
        let mut m = rtc_mbc();
        latch(&mut m);
        m.tick(SEC * 5);
        assert_eq!(read_rtc(&mut m, 0x08), 0); // latched value unchanged
        latch(&mut m);
        assert_eq!(read_rtc(&mut m, 0x08), 5);
    }

    #[test]
    fn rtc_halt_freezes_clock() {
        let mut m = rtc_mbc();
        m.write_rom(0x4000, 0x0C);
        m.write_ram(0xA000, 0x40); // halt
        m.tick(SEC * 10);
        m.write_rom(0x4000, 0x0C);
        m.write_ram(0xA000, 0x00); // resume
        m.tick(SEC * 2);
        latch(&mut m);
        assert_eq!(read_rtc(&mut m, 0x08), 2); // only post-resume time counted
    }

    #[test]
    fn rtc_day_overflow_sets_sticky_carry() {
        let mut m = rtc_mbc();
        m.write_rom(0x4000, 0x0B);
        m.write_ram(0xA000, 0xFF);
        m.write_rom(0x4000, 0x0C);
        m.write_ram(0xA000, 0x01); // day = 511
        m.write_rom(0x4000, 0x0A);
        m.write_ram(0xA000, 23);
        m.write_rom(0x4000, 0x09);
        m.write_ram(0xA000, 59);
        m.write_rom(0x4000, 0x08);
        m.write_ram(0xA000, 59);
        m.tick(SEC);
        latch(&mut m);
        assert_eq!(read_rtc(&mut m, 0x0B), 0); // days wrapped
        assert_eq!(read_rtc(&mut m, 0x0C) & 0x80, 0x80); // carry set
    }

    #[test]
    fn rtc_save_load_fast_forwards_elapsed_wall_time() {
        let mut m = rtc_mbc();
        m.tick(SEC * 10); // clock at 10s
        let saved = m.save(1_000_000).unwrap();
        assert_eq!(saved.len(), 0x2000 + 5 + 8); // ram + regs + timestamp
        let mut m2 = rtc_mbc();
        m2.load(&saved, 1_000_000 + 3600); // reopened an hour later
        latch(&mut m2);
        assert_eq!(read_rtc(&mut m2, 0x08), 10);
        assert_eq!(read_rtc(&mut m2, 0x09), 0);
        assert_eq!(read_rtc(&mut m2, 0x0A), 1); // +1 hour
    }

    #[test]
    fn rtc_halted_save_does_not_fast_forward() {
        let mut m = rtc_mbc();
        m.tick(SEC * 7);
        m.write_rom(0x4000, 0x0C);
        m.write_ram(0xA000, 0x40); // halt
        let saved = m.save(1_000_000).unwrap();
        let mut m2 = rtc_mbc();
        m2.load(&saved, 2_000_000);
        latch(&mut m2);
        assert_eq!(read_rtc(&mut m2, 0x08), 7); // frozen
    }
}
