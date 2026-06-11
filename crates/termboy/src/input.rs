//! Keyboard -> Buttons. Two modes:
//! - enhanced: terminal reports real press/release (kitty keyboard protocol)
//! - fallback: terminals only report presses/repeats, so a key counts as held
//!   for HOLD after its last event (design spec §4)

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use termboy_core::Buttons;

/// Fallback hold window. Long enough to bridge the OS key-repeat delay on
/// most setups, short enough that taps don't feel sticky.
const HOLD: Duration = Duration::from_millis(200);

pub struct Input {
    enhanced: bool,
    held: Buttons,                // enhanced mode state
    timers: [Option<Instant>; 8], // fallback: last event per button bit
}

fn map(code: KeyCode) -> Option<Buttons> {
    Some(match code {
        KeyCode::Right => Buttons::RIGHT,
        KeyCode::Left => Buttons::LEFT,
        KeyCode::Up => Buttons::UP,
        KeyCode::Down => Buttons::DOWN,
        KeyCode::Char('x') | KeyCode::Char('X') => Buttons::A,
        KeyCode::Char('z') | KeyCode::Char('Z') => Buttons::B,
        KeyCode::Tab => Buttons::SELECT,
        KeyCode::Enter => Buttons::START,
        _ => return None,
    })
}

impl Input {
    pub fn new(enhanced: bool) -> Self {
        Self { enhanced, held: Buttons::default(), timers: [None; 8] }
    }

    pub fn handle(&mut self, key: &KeyEvent, now: Instant) {
        let Some(b) = map(key.code) else { return };
        if self.enhanced {
            match key.kind {
                KeyEventKind::Press | KeyEventKind::Repeat => self.held = self.held.with(b),
                KeyEventKind::Release => self.held = Buttons(self.held.0 & !b.0),
            }
        } else if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            self.timers[b.0.trailing_zeros() as usize] = Some(now);
        }
    }

    pub fn buttons(&self, now: Instant) -> Buttons {
        if self.enhanced {
            return self.held;
        }
        let mut out = Buttons::default();
        for (bit, timer) in self.timers.iter().enumerate() {
            if let Some(t) = timer {
                if now.duration_since(*t) < HOLD {
                    out = out.with(Buttons(1 << bit));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press)
    }

    fn release(code: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Release)
    }

    #[test]
    fn fallback_holds_for_window_then_releases() {
        let mut input = Input::new(false);
        let t0 = Instant::now();
        input.handle(&press(KeyCode::Char('x')), t0);
        assert!(input.buttons(t0 + Duration::from_millis(100)).contains(Buttons::A));
        assert!(!input.buttons(t0 + Duration::from_millis(250)).contains(Buttons::A));
    }

    #[test]
    fn fallback_repeat_extends_hold() {
        let mut input = Input::new(false);
        let t0 = Instant::now();
        input.handle(&press(KeyCode::Down), t0);
        input.handle(&press(KeyCode::Down), t0 + Duration::from_millis(150));
        assert!(input.buttons(t0 + Duration::from_millis(300)).contains(Buttons::DOWN));
    }

    #[test]
    fn fallback_ignores_release_events() {
        let mut input = Input::new(false);
        let t0 = Instant::now();
        input.handle(&press(KeyCode::Enter), t0);
        input.handle(&release(KeyCode::Enter), t0); // some terminals send these anyway
        assert!(input.buttons(t0 + Duration::from_millis(50)).contains(Buttons::START));
    }

    #[test]
    fn enhanced_tracks_press_and_release_exactly() {
        let mut input = Input::new(true);
        let t0 = Instant::now();
        input.handle(&press(KeyCode::Left), t0);
        input.handle(&press(KeyCode::Char('z')), t0);
        let b = input.buttons(t0 + Duration::from_secs(60)); // no time decay
        assert!(b.contains(Buttons::LEFT) && b.contains(Buttons::B));
        input.handle(&release(KeyCode::Left), t0);
        let b = input.buttons(t0);
        assert!(!b.contains(Buttons::LEFT) && b.contains(Buttons::B));
    }

    #[test]
    fn unmapped_keys_do_nothing() {
        let mut input = Input::new(false);
        let t0 = Instant::now();
        input.handle(&press(KeyCode::Char('q')), t0);
        assert_eq!(input.buttons(t0), Buttons::default());
    }
}
