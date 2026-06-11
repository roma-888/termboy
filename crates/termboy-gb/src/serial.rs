//! Serial port stub. Captures outgoing bytes; this is how Blargg test ROMs
//! report pass/fail, so the test harness reads `output()`.

#[derive(Default)]
pub struct Serial {
    sb: u8,
    out: Vec<u8>,
}

impl Serial {
    pub fn read_sb(&self) -> u8 { self.sb }
    pub fn write_sb(&mut self, value: u8) { self.sb = value; }
    /// SC reads back with unused bits high and the transfer already complete.
    pub fn read_sc(&self) -> u8 { 0x7E }
    pub fn write_sc(&mut self, value: u8) {
        if value & 0x80 != 0 {
            self.out.push(self.sb);
        }
    }
    pub fn output(&self) -> &[u8] { &self.out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_to_sc_with_start_bit_captures_sb() {
        let mut s = Serial::default();
        s.write_sb(b'H');
        s.write_sc(0x81);
        s.write_sb(b'i');
        s.write_sc(0x81);
        s.write_sc(0x01); // no start bit -> nothing captured
        assert_eq!(s.output(), b"Hi");
    }
}
