//! In-terminal ROM picker. Assumes raw mode + alternate screen are active.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal;

/// All .gb/.gbc files in `dir`, sorted by file name (case-insensitive).
pub fn scan_roms(dir: &Path) -> Vec<PathBuf> {
    let mut roms: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| {
                            let e = e.to_ascii_lowercase();
                            e == "gb" || e == "gbc"
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    roms.sort_by_key(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    });
    roms
}

fn draw(roms: &[PathBuf], selected: usize) {
    let (_, rows) = terminal::size().unwrap_or((80, 24));
    let visible = (rows as usize).saturating_sub(4).max(1);
    let top = selected.saturating_sub(visible.saturating_sub(1).min(selected));
    let top = top.min(roms.len().saturating_sub(visible));

    let mut out = String::from("\x1b[2J\x1b[H\x1b[0m");
    out.push_str("  \x1b[1mtermboy\x1b[0m — select a game (Enter to play, Esc to quit)\r\n\r\n");
    for (i, rom) in roms.iter().enumerate().skip(top).take(visible) {
        let name = rom.file_stem().map(|n| n.to_string_lossy()).unwrap_or_default();
        if i == selected {
            out.push_str(&format!("  \x1b[7m ▸ {name} \x1b[0m\r\n"));
        } else {
            out.push_str(&format!("    {name}\r\n"));
        }
    }
    print!("{out}");
    std::io::stdout().flush().ok();
}

/// Interactive picker. Returns the chosen index, or None to quit.
pub fn pick(roms: &[PathBuf]) -> Option<usize> {
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

    #[test]
    fn scan_filters_and_sorts() {
        let dir = std::env::temp_dir().join("termboy-test-menu");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["zelda.gb", "Crystal.GBC", "notes.txt", "tetris.gb", "save.sav"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let roms = scan_roms(&dir);
        let names: Vec<String> = roms
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Crystal.GBC", "tetris.gb", "zelda.gb"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scan_missing_dir_is_empty() {
        assert!(scan_roms(Path::new("/nonexistent-termboy-dir")).is_empty());
    }
}
