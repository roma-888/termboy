//! Keyboard -> Buttons. Two modes:
//! - enhanced: terminal reports real press/release (kitty keyboard protocol)
//! - fallback: terminals only report presses/repeats, so a key counts as held
//!   for HOLD after its last event (design spec §4)

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use termboy_core::Buttons;

/// Fallback hold window. Long enough to bridge the OS key-repeat delay on
/// most setups, short enough that taps don't feel sticky.
const HOLD: Duration = Duration::from_millis(200);

pub struct Input {
    enhanced: bool,
    keymap: HashMap<KeyCode, Buttons>,
    held: Buttons,                // enhanced mode state
    timers: [Option<Instant>; 8], // fallback: last event per button bit
}

pub fn default_keymap() -> HashMap<KeyCode, Buttons> {
    let mut m = HashMap::new();
    m.insert(KeyCode::Right, Buttons::RIGHT);
    m.insert(KeyCode::Left, Buttons::LEFT);
    m.insert(KeyCode::Up, Buttons::UP);
    m.insert(KeyCode::Down, Buttons::DOWN);
    m.insert(KeyCode::Char('x'), Buttons::A);
    m.insert(KeyCode::Char('z'), Buttons::B);
    m.insert(KeyCode::Tab, Buttons::SELECT);
    m.insert(KeyCode::Enter, Buttons::START);
    m
}

fn key_from_name(name: &str) -> Option<KeyCode> {
    Some(match name {
        "enter" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        s if s.chars().count() == 1 => KeyCode::Char(s.chars().next()?.to_ascii_lowercase()),
        _ => return None,
    })
}

/// `swap` (A/B swapped) or `button=key,...` overriding individual buttons.
pub fn parse_keys(spec: &str) -> Option<HashMap<KeyCode, Buttons>> {
    let mut map = default_keymap();
    if spec == "swap" {
        map.insert(KeyCode::Char('z'), Buttons::A);
        map.insert(KeyCode::Char('x'), Buttons::B);
        return Some(map);
    }
    for part in spec.split(',') {
        let (button, key) = part.split_once('=')?;
        let b = match button.trim() {
            "a" => Buttons::A,
            "b" => Buttons::B,
            "start" => Buttons::START,
            "select" => Buttons::SELECT,
            "up" => Buttons::UP,
            "down" => Buttons::DOWN,
            "left" => Buttons::LEFT,
            "right" => Buttons::RIGHT,
            _ => return None,
        };
        let code = key_from_name(key.trim())?;
        // remove any key previously bound to this button, then bind
        map.retain(|_, v| *v != b);
        map.insert(code, b);
    }
    Some(map)
}

impl Input {
    pub fn new(enhanced: bool, keymap: HashMap<KeyCode, Buttons>) -> Self {
        Self { enhanced, keymap, held: Buttons::default(), timers: [None; 8] }
    }

    fn map(&self, code: KeyCode) -> Option<Buttons> {
        // case-insensitive chars: look up lowercased
        let code = match code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        };
        self.keymap.get(&code).copied()
    }

    pub fn handle(&mut self, key: &KeyEvent, now: Instant) {
        let Some(b) = self.map(key.code) else { return };
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
        let mut input = Input::new(false, default_keymap());
        let t0 = Instant::now();
        input.handle(&press(KeyCode::Char('x')), t0);
        assert!(input.buttons(t0 + Duration::from_millis(100)).contains(Buttons::A));
        assert!(!input.buttons(t0 + Duration::from_millis(250)).contains(Buttons::A));
    }

    #[test]
    fn fallback_repeat_extends_hold() {
        let mut input = Input::new(false, default_keymap());
        let t0 = Instant::now();
        input.handle(&press(KeyCode::Down), t0);
        input.handle(&press(KeyCode::Down), t0 + Duration::from_millis(150));
        assert!(input.buttons(t0 + Duration::from_millis(300)).contains(Buttons::DOWN));
    }

    #[test]
    fn fallback_ignores_release_events() {
        let mut input = Input::new(false, default_keymap());
        let t0 = Instant::now();
        input.handle(&press(KeyCode::Enter), t0);
        input.handle(&release(KeyCode::Enter), t0); // some terminals send these anyway
        assert!(input.buttons(t0 + Duration::from_millis(50)).contains(Buttons::START));
    }

    #[test]
    fn enhanced_tracks_press_and_release_exactly() {
        let mut input = Input::new(true, default_keymap());
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
    fn keys_swap_and_custom_specs() {
        let m = parse_keys("swap").unwrap();
        assert_eq!(m[&KeyCode::Char('z')], Buttons::A);
        assert_eq!(m[&KeyCode::Char('x')], Buttons::B);
        let m = parse_keys("a=k,start=space").unwrap();
        assert_eq!(m[&KeyCode::Char('k')], Buttons::A);
        assert_eq!(m[&KeyCode::Char(' ')], Buttons::START);
        assert!(!m.contains_key(&KeyCode::Char('x'))); // old A binding removed
        assert!(!m.contains_key(&KeyCode::Enter)); // old Start binding removed
        assert_eq!(m[&KeyCode::Char('z')], Buttons::B); // untouched default
        assert!(parse_keys("a=").is_none());
        assert!(parse_keys("warp=k").is_none());
        assert!(parse_keys("a=notakey").is_none());
    }

    #[test]
    fn unmapped_keys_do_nothing() {
        let mut input = Input::new(false, default_keymap());
        let t0 = Instant::now();
        input.handle(&press(KeyCode::Char('q')), t0);
        assert_eq!(input.buttons(t0), Buttons::default());
    }
}
