//! Terminal frontend: half-block rendering at the Game Boy's 59.73 Hz.
//! Pixel-perfect when the terminal is big enough, auto-downscaled to fit
//! when it isn't (`--exact` disables downscaling).
//! `--headless` runs without UI and streams serial output (debug tool).

mod audio;
mod input;
mod menu;
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
use termboy_core::{Buttons, Core, Rgb};
use termboy_gb::{DMG_GREEN, GameBoy};
use termboy_gba::GbaCore;

const USAGE: &str = "\
usage: termboy [options] [rom.gb|rom.gba]

with no rom argument, termboy opens a game picker for ./roms (or .)

options:
  --palette <name>   green (default), gray, pocket, or four hex colors
                     lightest-to-darkest: '#e0f8d0,#88c070,#346856,#081820'
  --exact            require a 160x72 terminal instead of auto-scaling
  --keys <spec>      'swap' (A/B swapped) or per-button: 'a=k,b=j,start=space'
  --headless         run without UI, print serial output (debug tool)
  -h, --help         show this help

controls: arrows = D-pad, X = A, Z = B, A/S = L/R, Enter = Start, Tab = Select, Esc = quit";

fn parse_palette(name: &str) -> Option<[Rgb; 4]> {
    match name {
        "green" => Some(DMG_GREEN),
        "gray" | "grey" => Some([
            Rgb(0xFF, 0xFF, 0xFF),
            Rgb(0xAA, 0xAA, 0xAA),
            Rgb(0x55, 0x55, 0x55),
            Rgb(0x00, 0x00, 0x00),
        ]),
        "pocket" => Some([
            Rgb(0xE0, 0xDB, 0xCD),
            Rgb(0xA8, 0x9F, 0x94),
            Rgb(0x70, 0x6B, 0x66),
            Rgb(0x2B, 0x28, 0x26),
        ]),
        custom => {
            let mut out = [Rgb(0, 0, 0); 4];
            let mut n = 0;
            for part in custom.split(',') {
                let hex = part.trim().strip_prefix('#')?;
                if hex.len() != 6 || n == 4 {
                    return None;
                }
                let v = u32::from_str_radix(hex, 16).ok()?;
                out[n] = Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
                n += 1;
            }
            (n == 4).then_some(out)
        }
    }
}

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
fn flush_save(core: &impl Core, sav: &Path, last: &mut Option<Vec<u8>>) {
    if let Some(data) = core.save_ram() {
        if last.as_deref() != Some(&data[..]) && write_save(sav, &data).is_ok() {
            *last = Some(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_palettes_parse() {
        assert_eq!(parse_palette("green"), Some(DMG_GREEN));
        assert!(parse_palette("gray").is_some());
        assert!(parse_palette("pocket").is_some());
        assert!(parse_palette("sepia").is_none());
    }

    #[test]
    fn custom_hex_palette_parses() {
        let p = parse_palette("#e0f8d0,#88c070,#346856,#081820").unwrap();
        assert_eq!(p[0], Rgb(0xE0, 0xF8, 0xD0));
        assert_eq!(p[3], Rgb(0x08, 0x18, 0x20));
        assert!(parse_palette("#e0f8d0,#88c070").is_none()); // needs 4
        assert!(parse_palette("#xyz,#88c070,#346856,#081820").is_none());
    }

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
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let headless = args.iter().any(|a| a == "--headless");
    let exact = args.iter().any(|a| a == "--exact");
    let mut palette = DMG_GREEN;
    let mut keymap = input::default_keymap();
    let mut rom_arg: Option<&str> = None;
    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        let (flag, value) = if let Some(n) = arg.strip_prefix("--palette=") {
            ("palette", Some(n.to_string()))
        } else if arg == "--palette" {
            ("palette", it.next().cloned())
        } else if let Some(n) = arg.strip_prefix("--keys=") {
            ("keys", Some(n.to_string()))
        } else if arg == "--keys" {
            ("keys", it.next().cloned())
        } else {
            if !arg.starts_with('-') {
                rom_arg = Some(arg);
            }
            ("", None)
        };
        match (flag, value) {
            ("palette", Some(name)) => match parse_palette(&name) {
                Some(p) => palette = p,
                None => {
                    eprintln!(
                        "error: unknown palette {name:?} — try green, gray, pocket, \
                         or four hex colors like '#e0f8d0,#88c070,#346856,#081820'"
                    );
                    return ExitCode::FAILURE;
                }
            },
            ("keys", Some(spec)) => match input::parse_keys(&spec) {
                Some(m) => keymap = m,
                None => {
                    eprintln!("error: bad --keys spec {spec:?} — try 'swap' or 'a=k,b=j,start=space'");
                    return ExitCode::FAILURE;
                }
            },
            _ => {}
        }
    }
    match rom_arg {
        Some(path) if is_gba(path) => {
            if headless {
                eprintln!("error: --headless is for GB serial test ROMs; GBA has no serial console");
                return ExitCode::FAILURE;
            }
            match load_gba(path) {
                Ok(core) => run_terminal(core, 240, 160, exact, &sav_path(path), keymap),
                Err(msg) => {
                    eprintln!("error: {msg}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(path) => {
            let (gb, sav) = match load_game(path, palette) {
                Ok(pair) => pair,
                Err(msg) => {
                    eprintln!("error: {msg}");
                    return ExitCode::FAILURE;
                }
            };
            if headless {
                run_headless(gb, &sav)
            } else {
                run_terminal(gb, 160, 144, exact, &sav, keymap)
            }
        }
        None if headless => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
        None => run_menu(palette, exact, keymap),
    }
}

fn is_gba(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gba"))
}

fn load_gba(path: &str) -> Result<GbaCore, String> {
    let rom = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let mut core = GbaCore::new(rom)?;
    if let Ok(data) = std::fs::read(sav_path(path)) {
        core.load_ram(&data);
    }
    Ok(core)
}

fn load_game(path: &str, palette: [Rgb; 4]) -> Result<(GameBoy, PathBuf), String> {
    let rom = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let mut gb = GameBoy::new(rom).map_err(|e| e.to_string())?;
    gb.set_palette(palette);
    let sav = sav_path(path);
    if let Ok(data) = std::fs::read(&sav) {
        gb.load_ram(&data);
    }
    Ok((gb, sav))
}

/// No-arg mode: one terminal session hosting picker -> game -> picker.
fn run_menu(
    palette: [Rgb; 4],
    exact: bool,
    keymap: std::collections::HashMap<crossterm::event::KeyCode, termboy_core::Buttons>,
) -> ExitCode {
    let rom_dir = if Path::new("roms").is_dir() { Path::new("roms") } else { Path::new(".") };
    let roms = menu::scan_roms(rom_dir);
    if roms.is_empty() {
        eprintln!("no .gb/.gbc/.gba files found in {}", rom_dir.display());
        return ExitCode::FAILURE;
    }
    install_panic_hook();
    let guard = match TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: cannot enter raw mode: {e}");
            return ExitCode::FAILURE;
        }
    };
    let audio = audio::Audio::new();
    loop {
        let Some(i) = menu::pick(&roms) else {
            return ExitCode::SUCCESS;
        };
        let path = roms[i].path.to_string_lossy().into_owned();
        let launch_err = if roms[i].kind == menu::Kind::Advance {
            match load_gba(&path) {
                Ok(core) => {
                    let mut input = input::Input::new(guard.enhanced, keymap.clone());
                    let mut screen = screen::Screen::new(240, 160);
                    print!("\x1b[0m\x1b[2J");
                    let sav = sav_path(&path);
                    let audio = &audio; // keep the move closure from consuming it
                    catch_game_panic(move || {
                        run_game(core, exact, &sav, &mut input, &mut screen, audio);
                        // Esc in-game returns here: back to the picker.
                    })
                }
                Err(msg) => Some(msg),
            }
        } else {
            match load_game(&path, palette) {
                Ok((gb, sav)) => {
                    let mut input = input::Input::new(guard.enhanced, keymap.clone());
                    let mut screen = screen::Screen::new(160, 144);
                    print!("\x1b[0m\x1b[2J");
                    let audio = &audio;
                    catch_game_panic(move || {
                        run_game(gb, exact, &sav, &mut input, &mut screen, audio);
                    })
                }
                Err(msg) => Some(msg),
            }
        };
        if let Some(msg) = launch_err {
            // the panic hook may have left raw mode + the alternate screen;
            // restore both before showing the error in the picker session
            let _ = terminal::enable_raw_mode();
            print!("\x1b[?1049h\x1b[0m\x1b[2J\x1b[Herror: {msg}\r\n\r\npress any key");
            use std::io::Write as _;
            std::io::stdout().flush().ok();
            let _ = event::read();
        }
    }
}

/// Run a game, converting a core panic into a picker-visible message
/// instead of killing the whole app (GBA cores hit loud unimplemented!
/// seams until G4 fills them in).
fn catch_game_panic(f: impl FnOnce()) -> Option<String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => None,
        Err(e) => {
            let detail = e
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| e.downcast_ref::<&str>().copied())
                .unwrap_or("unknown panic");
            Some(format!("this game needs a later milestone: {detail}"))
        }
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
            // REPORT_EVENT_TYPES alone gives press/release for keys already sent
            // as escape sequences (arrows) and printable keys, but legacy keys
            // (Enter, Tab, Esc, Backspace) keep their byte encoding and never emit
            // release events — so START/SELECT never clear in the held model.
            // REPORT_ALL_KEYS_AS_ESCAPE_CODES forces every key into CSI-u so they
            // all report press/repeat/release uniformly.
            execute!(
                std::io::stdout(),
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                )
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

fn install_panic_hook() {
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
}

fn run_terminal<C: Core>(
    core: C,
    width: usize,
    height: usize,
    exact: bool,
    sav: &Path,
    keymap: std::collections::HashMap<crossterm::event::KeyCode, termboy_core::Buttons>,
) -> ExitCode {
    install_panic_hook();
    let guard = match TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: cannot enter raw mode: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut input = input::Input::new(guard.enhanced, keymap);
    let mut screen = screen::Screen::new(width, height);
    let audio = audio::Audio::new();
    run_game(core, exact, sav, &mut input, &mut screen, &audio)
}

/// Run one game until Esc. Assumes raw mode + alternate screen are active.
fn run_game<C: Core>(
    mut core: C,
    exact: bool,
    sav: &Path,
    input: &mut input::Input,
    screen: &mut screen::Screen,
    audio: &audio::Audio,
) -> ExitCode {
    core.set_audio_rate(audio.sample_rate);
    let mut audio_buf: Vec<(f32, f32)> = Vec::new();
    let (need_cols, need_rows) = screen.required_size();
    let mut last_size = (0u16, 0u16);
    screen.invalidate();

    let mut out = String::new();
    let mut next_frame = Instant::now();
    let mut last_saved: Option<Vec<u8>> = core.save_ram();
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
            out.push_str("\x1b[0m\x1b[2J\x1b[H");
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
            print!("\x1b[0m\x1b[2J"); // clear leftovers outside the (re)centered image
            std::io::stdout().flush().ok();
        }

        let fb = core.run_frame(input.buttons(Instant::now()));
        out.clear();
        screen.render(fb, &mut out);
        core.drain_audio(&mut audio_buf);
        audio.push(&mut audio_buf);
        if !out.is_empty() {
            print!("{out}");
            std::io::stdout().flush().ok();
        }

        frames += 1;
        if frames % 300 == 0 {
            flush_save(&core, sav, &mut last_saved); // every ~5 seconds
        }

        next_frame += FRAME_TIME;
        let now = Instant::now();
        if next_frame > now {
            std::thread::sleep(next_frame - now);
        } else {
            next_frame = now; // fell behind: don't try to catch up in a burst
        }
    };
    flush_save(&core, sav, &mut last_saved);
    code
}
