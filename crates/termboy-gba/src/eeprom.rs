//! GBA cartridge EEPROM: a bit-serial save accessed only via DMA. Each DMA
//! halfword carries one bit (in bit 0). The DMA *length* selects both the
//! command and the chip size: write-direction counts 9/73 mean a 6-bit
//! address (4Kbit/512B), 17/81 mean a 14-bit address (64Kbit/8KB); a
//! read-direction DMA is 68 units (4 dummy bits + 64 data bits). One 64-bit
//! (8-byte) word per address.

const MAX: usize = 0x2000; // 8KB, the larger variant

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Idle,
    Receiving, // accumulating a command bitstream (DMA -> EEPROM)
    Reading,   // emitting a read reply (DMA <- EEPROM)
}

pub struct Eeprom {
    data: Box<[u8; MAX]>,
    /// 6 (512B) or 14 (8KB). Defaults to 6 until a wider transfer is seen.
    addr_bits: u8,
    mode: Mode,
    /// Incoming command bits, MSB first (max 81).
    rx: Vec<u8>,
    expected: u32,
    /// 64-bit reply, MSB first, plus a 0..68 emit cursor.
    read_word: u64,
    read_pos: u32,
}

impl Eeprom {
    pub fn new() -> Self {
        Self {
            data: Box::new([0xFF; MAX]),
            addr_bits: 6,
            mode: Mode::Idle,
            rx: Vec::with_capacity(81),
            expected: 0,
            read_word: 0,
            read_pos: 0,
        }
    }

    fn words(&self) -> usize {
        if self.addr_bits == 6 { 64 } else { 1024 }
    }

    /// Detected save size in bytes (512 or 8192).
    fn size(&self) -> usize {
        self.words() * 8
    }

    /// A DMA touching the EEPROM is starting. `to_eeprom` = command/write
    /// direction (RAM -> chip); otherwise it's the read reply (chip -> RAM).
    pub fn begin(&mut self, count: u32, to_eeprom: bool) {
        if to_eeprom {
            // the write-direction length disambiguates the address width
            if count == 17 || count == 81 {
                self.addr_bits = 14;
            } else if count == 9 || count == 73 {
                self.addr_bits = 6;
            }
            self.rx.clear();
            self.expected = count;
            self.mode = Mode::Receiving;
        } else {
            self.read_pos = 0;
            self.mode = Mode::Reading;
        }
    }

    /// One bit from a DMA halfword written to the EEPROM.
    pub fn write_bit(&mut self, value: u16) {
        if self.mode != Mode::Receiving {
            return;
        }
        self.rx.push((value & 1) as u8);
        if self.rx.len() as u32 >= self.expected {
            self.execute();
            self.mode = Mode::Idle;
        }
    }

    /// One bit for a DMA halfword read from the EEPROM (in bit 0).
    pub fn read_bit(&mut self) -> u16 {
        let bit = if self.read_pos < 4 {
            0 // dummy lead-in
        } else {
            ((self.read_word >> (63 - (self.read_pos - 4).min(63))) & 1) as u16
        };
        self.read_pos += 1;
        bit
    }

    fn execute(&mut self) {
        let bits = &self.rx;
        if bits.len() < 2 {
            return;
        }
        let cmd = (bits[0] << 1) | bits[1];
        let a = self.addr_bits as usize;
        let mut addr = 0usize;
        for &b in &bits[2..2 + a] {
            addr = (addr << 1) | b as usize;
        }
        let word = addr & (self.words() - 1);
        match cmd {
            0b11 => {
                // read request: latch the 64-bit word for the reply
                let base = word * 8;
                let mut w = 0u64;
                for i in 0..8 {
                    w = (w << 8) | self.data[base + i] as u64;
                }
                self.read_word = w;
                self.read_pos = 0;
            }
            0b10 => {
                // write: the next 64 bits are the data word
                let mut w = 0u64;
                for &b in &bits[2 + a..2 + a + 64] {
                    w = (w << 1) | b as u64;
                }
                let base = word * 8;
                for i in 0..8 {
                    self.data[base + i] = (w >> (56 - i * 8)) as u8;
                }
            }
            _ => {}
        }
    }

    pub fn backup(&self) -> Vec<u8> {
        self.data[..self.size()].to_vec()
    }

    pub fn restore(&mut self, data: &[u8]) {
        // a saved file's length tells us the chip size
        self.addr_bits = if data.len() <= 512 { 6 } else { 14 };
        let n = data.len().min(MAX);
        self.data[..n].copy_from_slice(&data[..n]);
    }
}

impl Default for Eeprom {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed a write-direction transfer: `bits` MSB-first, length = count.
    fn send(e: &mut Eeprom, bits: &[u8]) {
        e.begin(bits.len() as u32, true);
        for &b in bits {
            e.write_bit(b as u16);
        }
    }

    /// Build a read-request bitstream for `addr` with `a`-bit addressing.
    fn read_req(addr: usize, a: usize) -> Vec<u8> {
        let mut v = vec![1, 1];
        for i in (0..a).rev() {
            v.push(((addr >> i) & 1) as u8);
        }
        v.push(0); // stop
        v
    }

    /// Build a write bitstream: addr + 64-bit `data`, `a`-bit addressing.
    fn write_req(addr: usize, a: usize, data: u64) -> Vec<u8> {
        let mut v = vec![1, 0];
        for i in (0..a).rev() {
            v.push(((addr >> i) & 1) as u8);
        }
        for i in (0..64).rev() {
            v.push(((data >> i) & 1) as u8);
        }
        v.push(0); // stop
        v
    }

    /// Drive a read reply DMA and reassemble the 64-bit word.
    fn read_word(e: &mut Eeprom) -> u64 {
        e.begin(68, false);
        let mut w = 0u64;
        for i in 0..68 {
            let b = e.read_bit();
            if i >= 4 {
                w = (w << 1) | b as u64;
            }
        }
        w
    }

    #[test]
    fn writes_and_reads_a_word_at_512b() {
        let mut e = Eeprom::new();
        send(&mut e, &write_req(5, 6, 0x0123_4567_89AB_CDEF)); // count 73 -> 6-bit
        assert_eq!(e.addr_bits, 6);
        send(&mut e, &read_req(5, 6)); // count 9
        assert_eq!(read_word(&mut e), 0x0123_4567_89AB_CDEF);
        assert_eq!(e.backup().len(), 512);
    }

    #[test]
    fn writes_and_reads_a_word_at_8k() {
        let mut e = Eeprom::new();
        send(&mut e, &write_req(1000, 14, 0xDEAD_BEEF_F00D_BABE)); // count 81 -> 14-bit
        assert_eq!(e.addr_bits, 14);
        send(&mut e, &read_req(1000, 14)); // count 17
        assert_eq!(read_word(&mut e), 0xDEAD_BEEF_F00D_BABE);
        assert_eq!(e.backup().len(), 8192);
    }

    #[test]
    fn distinct_addresses_are_independent() {
        let mut e = Eeprom::new();
        send(&mut e, &write_req(0, 6, 0x1111_1111_1111_1111));
        send(&mut e, &write_req(7, 6, 0x2222_2222_2222_2222));
        send(&mut e, &read_req(0, 6));
        assert_eq!(read_word(&mut e), 0x1111_1111_1111_1111);
        send(&mut e, &read_req(7, 6));
        assert_eq!(read_word(&mut e), 0x2222_2222_2222_2222);
    }

    #[test]
    fn restore_sets_size_and_contents() {
        let mut e = Eeprom::new();
        let mut saved = vec![0xFFu8; 8192];
        saved[7] = 0x5A; // word 0, least-significant data byte
        e.restore(&saved);
        assert_eq!(e.addr_bits, 14);
        send(&mut e, &read_req(0, 14));
        assert_eq!(read_word(&mut e) & 0xFF, 0x5A);
    }
}
