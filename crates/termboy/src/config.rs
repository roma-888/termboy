//! Persistent user settings. A tiny flat `key = value` file sets the starting
//! defaults; `apply_setting` is the single validation/dispatch point shared by
//! both the file loader and the CLI argument parser, so config and flags can
//! never drift. CLI flags override the file (last writer wins).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// Read a flat `key = value` config file into `s`. Missing or unreadable file is
/// silent (no config). A `#` at the START of a (trimmed) line is a comment — never
/// mid-line, because hex palette values begin with `#`. A bad/unknown line warns to
/// stderr and is skipped; a config typo must never stop a game from launching.
pub fn load(path: &Path, s: &mut Settings) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            eprintln!("config:{}: ignoring line without '=': {raw:?}", i + 1);
            continue;
        };
        if let Err(e) = apply_setting(s, key.trim(), value.trim()) {
            eprintln!("config:{}: {e}", i + 1);
        }
    }
}

/// Pure path resolver (testable without touching process env): `$XDG_CONFIG_HOME`,
/// else `$HOME/.config`, joined with `termboy/config`.
fn path_from(xdg: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let base = xdg
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| home.map(|h| h.join(".config")))?;
    Some(base.join("termboy").join("config"))
}

/// Default config path from the real environment, or `None` if neither
/// `XDG_CONFIG_HOME` nor `HOME` is set.
pub fn default_path() -> Option<PathBuf> {
    path_from(
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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

    fn write_tmp(name: &str, body: &str) -> PathBuf {
        let p = std::env::temp_dir().join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn load_applies_values_and_skips_comments_and_blanks() {
        let p = write_tmp(
            "termboy-cfg-basic",
            "# my config\n\npalette = gray\n  speed = 2  \nmute = true\n",
        );
        let mut s = Settings::default();
        load(&p, &mut s);
        fs::remove_file(&p).ok();
        assert_ne!(s.palette, DMG_GREEN);
        assert_eq!(s.speed, 2.0);
        assert!(s.muted);
    }

    #[test]
    fn load_keeps_hash_inside_a_hex_palette() {
        // '#' is a comment ONLY at line start; a hex palette value must survive.
        let p = write_tmp("termboy-cfg-hex", "palette = #e0f8d0,#88c070,#346856,#081820\n");
        let mut s = Settings::default();
        load(&p, &mut s);
        fs::remove_file(&p).ok();
        assert_eq!(s.palette[0], Rgb(0xe0, 0xf8, 0xd0));
    }

    #[test]
    fn load_warns_and_skips_bad_lines_without_aborting() {
        let p = write_tmp(
            "termboy-cfg-bad",
            "palette = nope\nnosep line\nunknownkey = 5\nspeed = 4\n",
        );
        let mut s = Settings::default();
        load(&p, &mut s);
        fs::remove_file(&p).ok();
        assert_eq!(s.palette, DMG_GREEN);
        assert_eq!(s.speed, 4.0);
    }

    #[test]
    fn load_is_silent_when_file_missing() {
        let mut s = Settings::default();
        load(Path::new("/no/such/termboy/config"), &mut s);
        assert_eq!(s.speed, 1.0);
    }

    #[test]
    fn path_from_prefers_xdg_then_home() {
        assert_eq!(
            path_from(Some(PathBuf::from("/x")), Some(PathBuf::from("/home/u"))),
            Some(PathBuf::from("/x/termboy/config"))
        );
        assert_eq!(
            path_from(None, Some(PathBuf::from("/home/u"))),
            Some(PathBuf::from("/home/u/.config/termboy/config"))
        );
        assert_eq!(path_from(Some(PathBuf::new()), None), None);
    }
}
