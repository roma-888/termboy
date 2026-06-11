//! Terminal frontend: half-block rendering at the Game Boy's 59.73 Hz.
//! Pixel-perfect when the terminal is big enough, auto-downscaled to fit
//! when it isn't (`--exact` disables downscaling).
//! `--headless` runs without UI and streams serial output (debug tool).

mod input;
mod screen;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::{cursor, execute, terminal};
use termboy_core::{Buttons, Core};
use termboy_gb::GameBoy;

fn sav_path(rom_path: &str) -> PathBuf {
    Path::new(rom_path).with_extension("sav")
}

/// Atomic write: a crash mid-write must never corrupt an existing save.
fn write_save(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("sav.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

/// Write the battery save if it changed since the last flush.
fn flush_save(gb: &GameBoy, sav: &Path, last: &mut Option<Vec<u8>>) {
    if let Some(data) = gb.save_ram() {
        if last.as_deref() != Some(&data[..]) && write_save(sav, &data).is_ok() {
            *last = Some(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sav_path_replaces_extension() {
        assert_eq!(sav_path("roms/tetris.gb"), PathBuf::from("roms/tetris.sav"));
        assert_eq!(sav_path("pokemon.red.gb"), PathBuf::from("pokemon.red.sav"));
    }

    #[test]
    fn write_save_is_atomic_and_readable() {
        let dir = std::env::temp_dir().join("termboy-test-sav");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game.sav");
        write_save(&path, b"hello").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        assert!(!path.with_extension("sav.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// 4_194_304 Hz / 70_224 cycles ≈ 59.73 fps.
const FRAME_TIME: Duration = Duration::from_nanos(70_224 * 1_000_000_000 / 4_194_304);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let headless = args.iter().any(|a| a == "--headless");
    let exact = args.iter().any(|a| a == "--exact");
    let Some(path) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: termboy [--headless] [--exact] <rom.gb>");
        return ExitCode::FAILURE;
    };
    let rom = match std::fs::read(path) {
        Ok(rom) => rom,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut gb = match GameBoy::new(rom) {
        Ok(gb) => gb,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let sav = sav_path(path);
    if let Ok(data) = std::fs::read(&sav) {
        gb.load_ram(&data);
    }
    if headless {
        run_headless(gb, &sav)
    } else {
        run_terminal(gb, exact, &sav)
    }
}

fn run_headless(mut gb: GameBoy, sav: &Path) -> ExitCode {
    let mut printed = 0;
    for _ in 0..60 * 60 {
        // run up to 60 emulated seconds
        gb.run_frame(Buttons::default());
        let out = gb.serial_output();
        if out.len() > printed {
            std::io::stdout().write_all(&out[printed..]).unwrap();
            std::io::stdout().flush().unwrap();
            printed = out.len();
        }
    }
    println!();
    let mut last = None;
    flush_save(&gb, sav, &mut last);
    ExitCode::SUCCESS
}

/// Restores the terminal even on panic (design spec §4).
struct TerminalGuard {
    /// Kitty keyboard protocol active: real press/release events.
    enhanced: bool,
}

impl TerminalGuard {
    fn enter() -> std::io::Result<Self> {
        terminal::enable_raw_mode()?;
        // Must be queried in raw mode, before the alternate screen.
        let enhanced = terminal::supports_keyboard_enhancement().unwrap_or(false);
        execute!(
            std::io::stdout(),
            terminal::EnterAlternateScreen,
            cursor::Hide,
            terminal::Clear(terminal::ClearType::All)
        )?;
        if enhanced {
            execute!(
                std::io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
            )?;
        }
        Ok(Self { enhanced })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.enhanced {
            let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        }
        let _ = execute!(
            std::io::stdout(),
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

fn run_terminal(mut gb: GameBoy, exact: bool, sav: &Path) -> ExitCode {
    let mut screen = screen::Screen::new(160, 144);
    let (need_cols, need_rows) = screen.required_size();
    let mut last_size = (0u16, 0u16);

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(
            std::io::stdout(),
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
        default_hook(info);
    }));

    let guard = match TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: cannot enter raw mode: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut input = input::Input::new(guard.enhanced);

    let mut out = String::new();
    let mut next_frame = Instant::now();
    let mut last_saved: Option<Vec<u8>> = gb.save_ram();
    let mut frames: u32 = 0;
    let code = 'game: loop {
        // Input: Esc quits, everything else goes to the tracker.
        let now = Instant::now();
        while event::poll(Duration::ZERO).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(k)) if k.code == KeyCode::Esc && k.kind == KeyEventKind::Press => {
                    break 'game ExitCode::SUCCESS;
                }
                Ok(Event::Key(k)) => input.handle(&k, now),
                Ok(Event::Resize(..)) => screen.invalidate(),
                _ => {}
            }
        }

        let (cols, rows) = terminal::size().unwrap_or((0, 0));
        let too_small = if exact {
            (cols as usize) < need_cols || (rows as usize) < need_rows
        } else {
            cols < 16 || rows < 8 // below this nothing recognizable fits
        };
        if too_small {
            out.clear();
            out.push_str("\x1b[2J\x1b[H\x1b[0m");
            let (nc, nr) = if exact { (need_cols, need_rows) } else { (16, 8) };
            out.push_str(&format!(
                "termboy needs a {nc}x{nr} terminal (yours: {cols}x{rows}). Resize, or press Esc to quit."
            ));
            print!("{out}");
            std::io::stdout().flush().ok();
            screen.invalidate();
            last_size = (0, 0);
            std::thread::sleep(Duration::from_millis(100));
            next_frame = Instant::now();
            continue;
        }
        if (cols, rows) != last_size {
            last_size = (cols, rows);
            screen.set_viewport(cols as usize, rows as usize);
            screen.invalidate();
            print!("\x1b[2J"); // clear leftovers outside the (re)centered image
            std::io::stdout().flush().ok();
        }

        let fb = gb.run_frame(input.buttons(Instant::now()));
        out.clear();
        screen.render(fb, &mut out);
        if !out.is_empty() {
            print!("{out}");
            std::io::stdout().flush().ok();
        }

        frames += 1;
        if frames % 300 == 0 {
            flush_save(&gb, sav, &mut last_saved); // every ~5 seconds
        }

        next_frame += FRAME_TIME;
        let now = Instant::now();
        if next_frame > now {
            std::thread::sleep(next_frame - now);
        } else {
            next_frame = now; // fell behind: don't try to catch up in a burst
        }
    };
    flush_save(&gb, sav, &mut last_saved);
    code
}
