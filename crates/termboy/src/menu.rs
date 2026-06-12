//! In-terminal ROM picker. Assumes raw mode + alternate screen are active.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Kind {
    Dmg,
    Color,
    Advance,
}

impl Kind {
    fn title(self) -> &'static str {
        match self {
            Kind::Dmg => "GAME BOY",
            Kind::Color => "GAME BOY COLOR",
            Kind::Advance => "GAME BOY ADVANCE",
        }
    }

    /// Section accent color (ANSI truecolor).
    fn color(self) -> &'static str {
        match self {
            Kind::Dmg => "\x1b[38;2;136;192;112m",    // DMG green
            Kind::Color => "\x1b[38;2;120;200;255m",  // GBC teal-blue
            Kind::Advance => "\x1b[38;2;180;130;255m", // GBA purple
        }
    }
}

pub struct Entry {
    pub path: PathBuf,
    pub kind: Kind,
}

/// Classify by extension (.gba) or by the cartridge header's CGB flag —
/// more reliable than the file extension for .gb/.gbc.
fn classify(path: &Path) -> Kind {
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gba"))
    {
        return Kind::Advance;
    }
    let mut header = [0u8; 0x150];
    match std::fs::File::open(path).and_then(|mut f| {
        use std::io::Read;
        f.read_exact(&mut header)
    }) {
        Ok(()) if header[0x143] & 0x80 != 0 => Kind::Color,
        _ => Kind::Dmg,
    }
}

/// All .gb/.gbc/.gba files in `dir`, grouped by kind, name-sorted within.
pub fn scan_roms(dir: &Path) -> Vec<Entry> {
    let mut roms: Vec<Entry> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| {
                            let e = e.to_ascii_lowercase();
                            e == "gb" || e == "gbc" || e == "gba"
                        })
                })
                .map(|path| Entry { kind: classify(&path), path })
                .collect()
        })
        .unwrap_or_default();
    roms.sort_by_key(|e| {
        (
            e.kind,
            e.path
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default(),
        )
    });
    roms
}

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";

fn draw(roms: &[Entry], selected: usize) {
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    let width = (cols as usize).clamp(30, 64);

    // compose all lines first, then window around the selection
    let mut lines: Vec<String> = Vec::new();
    let mut selected_line = 0;
    let mut last_kind: Option<Kind> = None;
    for (i, rom) in roms.iter().enumerate() {
        if last_kind != Some(rom.kind) {
            last_kind = Some(rom.kind);
            let title = rom.kind.title();
            let rule = "─".repeat(width.saturating_sub(title.len() + 5));
            lines.push(String::new());
            lines.push(format!("  {}{BOLD}{title}{RESET} {}{rule}{RESET}", rom.kind.color(), rom.kind.color()));
        }
        let name = rom.path.file_stem().map(|n| n.to_string_lossy()).unwrap_or_default();
        if i == selected {
            selected_line = lines.len();
            lines.push(format!("   {}\x1b[7m ▸ {name} {RESET}", rom.kind.color()));
        } else {
            lines.push(format!("     {name}"));
        }
    }

    let header_rows = 3;
    let footer_rows = 2;
    let visible = (rows as usize).saturating_sub(header_rows + footer_rows).max(1);
    let top = if selected_line >= visible {
        selected_line + 1 - visible
    } else {
        0
    };

    let green = Kind::Dmg.color();
    let mut out = String::from("\x1b[0m\x1b[2J\x1b[H");
    out.push_str(&format!(
        "  {green}▄▀▄▀▄{RESET} {BOLD}T E R M B O Y{RESET} {green}▄▀▄▀▄{RESET}\r\n"
    ));
    out.push_str(&format!("  {DIM}{} games{RESET}\r\n", roms.len()));
    for line in lines.iter().skip(top).take(visible) {
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str(&format!("\r\n  {DIM}↑/↓ move · Enter play · Esc quit{RESET}"));
    print!("{out}");
    std::io::stdout().flush().ok();
}

/// Interactive picker. Returns the chosen index, or None to quit.
pub fn pick(roms: &[Entry]) -> Option<usize> {
    if roms.is_empty() {
        return None;
    }
    let mut selected = 0usize;
    let mut dirty = true;
    loop {
        if dirty {
            draw(roms, selected);
            dirty = false;
        }
        if !event::poll(Duration::from_millis(100)).unwrap_or(false) {
            continue;
        }
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press || k.kind == KeyEventKind::Repeat => {
                match k.code {
                    KeyCode::Esc => return None,
                    KeyCode::Enter => return Some(selected),
                    KeyCode::Up | KeyCode::Char('k') => {
                        selected = selected.saturating_sub(1);
                        dirty = true;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        selected = (selected + 1).min(roms.len() - 1);
                        dirty = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::Resize(..)) => dirty = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom_bytes(cgb: u8) -> Vec<u8> {
        let mut rom = vec![0u8; 0x150];
        rom[0x143] = cgb;
        rom
    }

    #[test]
    fn scan_groups_by_kind_and_sorts() {
        let dir = std::env::temp_dir().join("termboy-test-menu2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("zelda.gb"), rom_bytes(0x00)).unwrap();
        std::fs::write(dir.join("crystal.gbc"), rom_bytes(0xC0)).unwrap();
        std::fs::write(dir.join("gold.gb"), rom_bytes(0x80)).unwrap(); // dual -> Color
        std::fs::write(dir.join("metroid.gba"), b"x".to_vec()).unwrap();
        std::fs::write(dir.join("notes.txt"), b"x".to_vec()).unwrap();
        std::fs::write(dir.join("apple.gb"), rom_bytes(0x00)).unwrap();
        let roms = scan_roms(&dir);
        let view: Vec<(Kind, String)> = roms
            .iter()
            .map(|e| {
                (
                    e.kind,
                    e.path.file_name().unwrap().to_string_lossy().into_owned(),
                )
            })
            .collect();
        assert_eq!(
            view,
            [
                (Kind::Dmg, "apple.gb".into()),
                (Kind::Dmg, "zelda.gb".into()),
                (Kind::Color, "crystal.gbc".into()),
                (Kind::Color, "gold.gb".into()),
                (Kind::Advance, "metroid.gba".into()),
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scan_missing_dir_is_empty() {
        assert!(scan_roms(Path::new("/nonexistent-termboy-dir")).is_empty());
    }
}
