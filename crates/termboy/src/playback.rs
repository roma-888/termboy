//! Transient playback controls: emulation speed and audio mute. None of this
//! touches the core's timing — speed scales the wall-clock frame interval the
//! frontend paces to, and mute just gates whether samples reach the device.

use std::time::Duration;

/// Selectable speed multipliers, slowest to fastest. `+`/`-` step through this
/// list and clamp at the ends (no wrap, so 4x doesn't jump back to 0.5x).
const SPEEDS: [f64; 4] = [0.5, 1.0, 2.0, 4.0];
/// Index of `1.0` in `SPEEDS`: the speed every game starts at.
const NORMAL: usize = 1;

pub struct Playback {
    speed: usize,
    muted: bool,
}

impl Default for Playback {
    fn default() -> Self {
        Self::new()
    }
}

impl Playback {
    pub fn new() -> Self {
        Self { speed: NORMAL, muted: false }
    }

    /// Step up one preset, clamping at the fastest.
    pub fn faster(&mut self) {
        self.speed = (self.speed + 1).min(SPEEDS.len() - 1);
    }

    /// Step down one preset, clamping at the slowest.
    pub fn slower(&mut self) {
        self.speed = self.speed.saturating_sub(1);
    }

    /// Current speed as a wall-clock multiplier (2.0 = twice as fast).
    pub fn multiplier(&self) -> f64 {
        SPEEDS[self.speed]
    }

    /// The wall-clock interval between frames at the current speed: the nominal
    /// `base` period divided by the multiplier (2x speed -> half the wait).
    pub fn frame_time(&self, base: Duration) -> Duration {
        base.div_f64(self.multiplier())
    }

    /// APU resample target (Hz) that keeps audio production matched to the
    /// device at the current speed. See the test for the reasoning.
    pub fn audio_rate(&self, host: u32) -> u32 {
        (host as f64 / self.multiplier()).round() as u32
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Terse overlay badge for the current speed, e.g. `speed 2x`, `speed 0.5x`.
    pub fn speed_label(&self) -> String {
        let m = self.multiplier();
        // Whole multipliers read better without the ".0" (2x, not 2.0x).
        if m.fract() == 0.0 {
            format!("speed {}x", m as u32)
        } else {
            format!("speed {m}x")
        }
    }

    /// Terse overlay badge for the mute toggle.
    pub fn mute_label(&self) -> &'static str {
        if self.muted { "muted" } else { "unmuted" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_normal_speed_unmuted() {
        let p = Playback::new();
        assert_eq!(p.multiplier(), 1.0);
        assert!(!p.is_muted());
    }

    #[test]
    fn faster_steps_up_then_clamps_at_max() {
        let mut p = Playback::new();
        p.faster();
        assert_eq!(p.multiplier(), 2.0);
        p.faster();
        assert_eq!(p.multiplier(), 4.0);
        p.faster(); // already at the top
        assert_eq!(p.multiplier(), 4.0);
    }

    #[test]
    fn slower_steps_down_then_clamps_at_min() {
        let mut p = Playback::new();
        p.slower();
        assert_eq!(p.multiplier(), 0.5);
        p.slower(); // already at the bottom
        assert_eq!(p.multiplier(), 0.5);
    }

    #[test]
    fn frame_time_scales_inversely_with_speed() {
        let base = Duration::from_millis(20);
        let mut p = Playback::new();
        assert_eq!(p.frame_time(base), base); // 1x
        p.faster();
        assert_eq!(p.frame_time(base), Duration::from_millis(10)); // 2x
        p.slower();
        p.slower();
        assert_eq!(p.frame_time(base), Duration::from_millis(40)); // 0.5x
    }

    #[test]
    fn audio_rate_scales_inversely_with_speed() {
        // The device drains `host` samples per wall-second; at speed S the core
        // advances S emulated-seconds per wall-second, so it must resample to
        // host/S to keep production matched to consumption (no underrun gaps).
        let mut p = Playback::new();
        assert_eq!(p.audio_rate(48_000), 48_000); // 1x
        p.faster();
        assert_eq!(p.audio_rate(48_000), 24_000); // 2x
        p.slower();
        p.slower();
        assert_eq!(p.audio_rate(48_000), 96_000); // 0.5x
    }

    #[test]
    fn mute_toggles() {
        let mut p = Playback::new();
        p.toggle_mute();
        assert!(p.is_muted());
        p.toggle_mute();
        assert!(!p.is_muted());
    }

    #[test]
    fn speed_label_omits_trailing_zero() {
        let mut p = Playback::new();
        assert_eq!(p.speed_label(), "speed 1x");
        p.slower();
        assert_eq!(p.speed_label(), "speed 0.5x");
        p.faster();
        p.faster();
        assert_eq!(p.speed_label(), "speed 2x");
    }

    #[test]
    fn mute_label_reflects_state() {
        let mut p = Playback::new();
        assert_eq!(p.mute_label(), "unmuted");
        p.toggle_mute();
        assert_eq!(p.mute_label(), "muted");
    }
}
