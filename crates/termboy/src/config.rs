//! Persistent user settings. A tiny flat `key = value` file sets the starting
//! defaults; `apply_setting` is the single validation/dispatch point shared by
//! both the file loader and the CLI argument parser, so config and flags can
//! never drift. CLI flags override the file (last writer wins).

use std::collections::HashMap;

use crossterm::event::KeyCode;
use termboy_core::{Buttons, Rgb};
use termboy_gb::DMG_GREEN;

use crate::input::{default_keymap, parse_keys};
use crate::{GraphicsPref, parse_palette};

pub struct Settings {
    pub palette: [Rgb; 4],
    pub keymap: HashMap<KeyCode, Buttons>,
    pub graphics: GraphicsPref,
    pub exact: bool,
    pub speed: f64,
    pub muted: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            palette: DMG_GREEN,
            keymap: default_keymap(),
            graphics: GraphicsPref::Auto,
            exact: false,
            speed: 1.0,
            muted: false,
        }
    }
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("expected true or false, got {other:?}")),
    }
}

/// The single validation/dispatch point for a `(key, value)` setting, shared by
/// the config-file loader and the CLI argument parser. Returns a human-readable
/// error string on a bad key or value; the caller decides whether to warn-and-skip
/// (config file) or hard-fail (CLI).
pub fn apply_setting(s: &mut Settings, key: &str, value: &str) -> Result<(), String> {
    match key {
        "palette" => {
            s.palette = parse_palette(value).ok_or_else(|| {
                format!(
                    "unknown palette {value:?} — try green, gray, pocket, \
                     or four hex colors like '#e0f8d0,#88c070,#346856,#081820'"
                )
            })?;
        }
        "keys" => {
            s.keymap = parse_keys(value)
                .ok_or_else(|| format!("bad keys spec {value:?} — try 'swap' or 'a=k,b=j,start=space'"))?;
        }
        "graphics" => {
            s.graphics = match value {
                "auto" => GraphicsPref::Auto,
                "kitty" => GraphicsPref::Kitty,
                "half" => GraphicsPref::Half,
                other => return Err(format!("graphics must be auto, kitty, or half (got {other:?})")),
            };
        }
        "exact" => s.exact = parse_bool(value)?,
        "mute" => s.muted = parse_bool(value)?,
        "speed" => {
            s.speed = match value {
                "0.5" => 0.5,
                "1" => 1.0,
                "2" => 2.0,
                "4" => 4.0,
                other => return Err(format!("speed must be 0.5, 1, 2, or 4 (got {other:?})")),
            };
        }
        other => return Err(format!("unknown setting {other:?}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_todays_hardwired_values() {
        let s = Settings::default();
        assert_eq!(s.palette, DMG_GREEN);
        assert!(!s.exact);
        assert_eq!(s.speed, 1.0);
        assert!(!s.muted);
        assert!(matches!(s.graphics, GraphicsPref::Auto));
    }

    #[test]
    fn apply_each_known_setting() {
        let mut s = Settings::default();
        apply_setting(&mut s, "palette", "gray").unwrap();
        assert_ne!(s.palette, DMG_GREEN);
        apply_setting(&mut s, "graphics", "kitty").unwrap();
        assert!(matches!(s.graphics, GraphicsPref::Kitty));
        apply_setting(&mut s, "exact", "true").unwrap();
        assert!(s.exact);
        apply_setting(&mut s, "speed", "2").unwrap();
        assert_eq!(s.speed, 2.0);
        apply_setting(&mut s, "mute", "true").unwrap();
        assert!(s.muted);
        apply_setting(&mut s, "keys", "a=k,start=space").unwrap();
        assert_eq!(s.keymap[&KeyCode::Char('k')], Buttons::A);
    }

    #[test]
    fn bad_values_and_unknown_keys_error() {
        let mut s = Settings::default();
        assert!(apply_setting(&mut s, "palette", "mauve").is_err());
        assert!(apply_setting(&mut s, "graphics", "fancy").is_err());
        assert!(apply_setting(&mut s, "exact", "yes").is_err());
        assert!(apply_setting(&mut s, "speed", "3").is_err());
        assert!(apply_setting(&mut s, "keys", "warp=k").is_err());
        assert!(apply_setting(&mut s, "nonsense", "x").is_err());
    }

    #[test]
    fn last_writer_wins_models_cli_override() {
        let mut s = Settings::default();
        apply_setting(&mut s, "palette", "gray").unwrap();
        let gray = s.palette;
        apply_setting(&mut s, "palette", "green").unwrap();
        assert_eq!(s.palette, DMG_GREEN);
        assert_ne!(s.palette, gray);
    }
}
