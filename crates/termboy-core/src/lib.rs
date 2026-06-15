//! System-agnostic interface between emulator cores and the frontend.

pub mod state;

use state::StateError;

/// Save-state format version. Bump on ANY layout change — old states then reject.
pub const STATE_VERSION: u16 = 1;

/// 24-bit RGB pixel. Cores map their native shades/palettes to RGB themselves,
/// so the frontend never knows which system it is running.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Clone)]
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

/// Button state as a bitset. Bits 0-7 match the GB layout; bits 8-9 are the
/// GBA shoulder buttons (cores map these to their own hardware order).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Buttons(pub u16);

impl Buttons {
    pub const RIGHT: Buttons = Buttons(0x0001);
    pub const LEFT: Buttons = Buttons(0x0002);
    pub const UP: Buttons = Buttons(0x0004);
    pub const DOWN: Buttons = Buttons(0x0008);
    pub const A: Buttons = Buttons(0x0010);
    pub const B: Buttons = Buttons(0x0020);
    pub const SELECT: Buttons = Buttons(0x0040);
    pub const START: Buttons = Buttons(0x0080);
    pub const L: Buttons = Buttons(0x0100);
    pub const R: Buttons = Buttons(0x0200);

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
    /// Move any pending stereo audio samples into `out`. Cores without
    /// audio (or with it disabled) may leave this as the default no-op.
    fn drain_audio(&mut self, out: &mut Vec<(f32, f32)>) {
        let _ = out;
    }
    /// Host audio sample rate (Hz). Cores without audio ignore it.
    fn set_audio_rate(&mut self, hz: u32) {
        let _ = hz;
    }
    /// Serialize full machine state (see [`state`]). Default: unsupported.
    fn save_state(&self) -> Vec<u8> {
        Vec::new()
    }
    /// Restore machine state from [`save_state`](Core::save_state) bytes.
    /// Default: unsupported.
    fn load_state(&mut self, data: &[u8]) -> Result<(), StateError> {
        let _ = data;
        Err(StateError::BadVersion)
    }
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
    fn buttons_widen_to_u16_with_shoulder_buttons() {
        let b = Buttons::default().with(Buttons::L).with(Buttons::R);
        assert!(b.contains(Buttons::L) && b.contains(Buttons::R));
        assert!(!b.contains(Buttons::A));
        assert_eq!(b.0, 0x0300);
    }

    #[test]
    fn framebuffer_dimensions() {
        let fb = FrameBuffer::new(160, 144);
        assert_eq!(fb.pixels.len(), 160 * 144);
    }
}
