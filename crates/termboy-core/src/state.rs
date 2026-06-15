//! Manual, dependency-free serialization for save states. A `Writer` appends
//! little-endian bytes; a `Reader` pops them with bounds checks (a truncated
//! or malformed file yields a `StateError`, never a panic).

#[derive(Debug, PartialEq, Eq)]
pub enum StateError {
    Truncated,
    BadMagic,
    BadVersion,
    WrongConsole,
    WrongRom,
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            StateError::Truncated => "save state is truncated or corrupt",
            StateError::BadMagic => "not a termboy save state",
            StateError::BadVersion => "save state from an incompatible version",
            StateError::WrongConsole => "save state is for a different console",
            StateError::WrongRom => "save state is for a different game",
        };
        f.write_str(s)
    }
}

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }
    pub fn put_u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn put_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn put_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn put_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn put_bool(&mut self, v: bool) {
        self.put_u8(v as u8);
    }
    pub fn put_bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }
}

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], StateError> {
        let end = self.pos.checked_add(n).ok_or(StateError::Truncated)?;
        let s = self.data.get(self.pos..end).ok_or(StateError::Truncated)?;
        self.pos = end;
        Ok(s)
    }
    pub fn get_u8(&mut self) -> Result<u8, StateError> {
        Ok(self.take(1)?[0])
    }
    pub fn get_u16(&mut self) -> Result<u16, StateError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn get_u32(&mut self) -> Result<u32, StateError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn get_u64(&mut self) -> Result<u64, StateError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn get_bool(&mut self) -> Result<bool, StateError> {
        Ok(self.get_u8()? != 0)
    }
    pub fn get_bytes(&mut self, n: usize) -> Result<&'a [u8], StateError> {
        self.take(n)
    }
    /// Read N bytes into a fixed array.
    pub fn get_array<const N: usize>(&mut self) -> Result<[u8; N], StateError> {
        Ok(self.take(N)?.try_into().unwrap())
    }
}

/// 64-bit FNV-1a hash, used as a ROM identity tag in save-state headers.
pub fn rom_id(rom: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in rom {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_scalars_and_bytes() {
        let mut w = Writer::new();
        w.put_u8(0x12);
        w.put_u16(0x3456);
        w.put_u32(0x789A_BCDE);
        w.put_u64(0x0102_0304_0506_0708);
        w.put_bool(true);
        w.put_bytes(&[9, 8, 7]);
        let mut r = Reader::new(&w.buf);
        assert_eq!(r.get_u8().unwrap(), 0x12);
        assert_eq!(r.get_u16().unwrap(), 0x3456);
        assert_eq!(r.get_u32().unwrap(), 0x789A_BCDE);
        assert_eq!(r.get_u64().unwrap(), 0x0102_0304_0506_0708);
        assert!(r.get_bool().unwrap());
        assert_eq!(r.get_bytes(3).unwrap(), &[9, 8, 7]);
    }

    #[test]
    fn underrun_is_an_error_not_a_panic() {
        let mut r = Reader::new(&[1, 2]);
        assert_eq!(r.get_u32(), Err(StateError::Truncated));
    }

    #[test]
    fn rom_id_is_stable_and_distinguishes() {
        assert_eq!(rom_id(b"abc"), rom_id(b"abc"));
        assert_ne!(rom_id(b"abc"), rom_id(b"abd"));
    }
}
