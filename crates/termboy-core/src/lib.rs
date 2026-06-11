//! System-agnostic interface between emulator cores and the frontend.

/// 24-bit RGB pixel. Cores map their native shades/palettes to RGB themselves,
/// so the frontend never knows which system it is running.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub struct FrameBuffer {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgb>, // row-major, width * height
}

impl FrameBuffer {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height, pixels: vec![Rgb(0, 0, 0); width * height] }
    }
}

/// Button state as a bitset. Extensible (GBA shoulder buttons get free bits).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Buttons(pub u8);

impl Buttons {
    pub const RIGHT: Buttons = Buttons(0x01);
    pub const LEFT: Buttons = Buttons(0x02);
    pub const UP: Buttons = Buttons(0x04);
    pub const DOWN: Buttons = Buttons(0x08);
    pub const A: Buttons = Buttons(0x10);
    pub const B: Buttons = Buttons(0x20);
    pub const SELECT: Buttons = Buttons(0x40);
    pub const START: Buttons = Buttons(0x80);

    pub fn contains(self, other: Buttons) -> bool { self.0 & other.0 == other.0 }
    pub fn with(self, other: Buttons) -> Buttons { Buttons(self.0 | other.0) }
}

pub trait Core {
    /// Run exactly one video frame's worth of emulated time and return the frame.
    fn run_frame(&mut self, buttons: Buttons) -> &FrameBuffer;
    /// Battery-backed save state if the loaded cartridge has any
    /// (owned: cores may append metadata such as RTC time at save time).
    fn save_ram(&self) -> Option<Vec<u8>>;
    fn load_ram(&mut self, data: &[u8]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_compose_and_query() {
        let b = Buttons::default().with(Buttons::A).with(Buttons::START);
        assert!(b.contains(Buttons::A));
        assert!(b.contains(Buttons::START));
        assert!(!b.contains(Buttons::LEFT));
    }

    #[test]
    fn framebuffer_dimensions() {
        let fb = FrameBuffer::new(160, 144);
        assert_eq!(fb.pixels.len(), 160 * 144);
    }
}
