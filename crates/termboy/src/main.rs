//! Terminal frontend: half-block rendering at the Game Boy's 59.73 Hz.
//! Pixel-perfect when the terminal is big enough, auto-downscaled to fit
//! when it isn't (`--exact` disables downscaling).
//! `--headless` runs without UI and streams serial output (debug tool).

mod audio;
mod capture;
mod config;
mod input;
mod kitty;
mod menu;
mod pause;
mod playback;
mod rewind;
mod screen;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{cursor, execute, terminal};
use termboy_core::{Buttons, Core, Rgb};
use termboy_gb::{DMG_GREEN, GameBoy};
use termboy_gba::GbaCore;

use pause::Action;

const USAGE: &str = "\
usage: termboy [options] [rom.gb|rom.gba]

with no rom argument, termboy opens a game picker for ./roms (or .)

options:
  --palette <name>   green (default), gray, pocket, or four hex colors
                     lightest-to-darkest: '#e0f8d0,#88c070,#346856,#081820'
  --exact            require a 160x72 terminal instead of auto-scaling
  --graphics <mode>  auto (default), kitty (force image protocol), or half (force
                     half-blocks). auto uses kitty graphics on Ghostty/kitty/WezTerm
  --keys <spec>      'swap' (A/B swapped) or per-button: 'a=k,b=j,start=space'
  --config <path>    use this file instead of ~/.config/termboy/config
  --color-correct    GBA only: hardware color correction (off by default)
  --headless         run without UI, print serial output (debug tool)
  -h, --help         show this help

controls: arrows = D-pad, X = A, Z = B, A/S = L/R, Enter = Start, Tab = Select
          +/- = speed (0.5x-4x), M = mute, Backspace = rewind, Esc = quit";

pub(crate) fn parse_palette(name: &str) -> Option<[Rgb; 4]> {
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

/// Save-state slot file: `<rom>.ss0` … `<rom>.ss9`, derived from the `.sav` path.
fn ss_path(sav: &Path, slot: u8) -> PathBuf {
    sav.with_extension(format!("ss{slot}"))
}

/// Which save-state slots already have a file on disk (for the browser markers).
fn slots_filled(sav: &Path) -> [bool; pause::SLOTS] {
    let mut s = [false; pause::SLOTS];
    for (i, filled) in s.iter_mut().enumerate() {
        *filled = ss_path(sav, i as u8).exists();
    }
    s
}

/// If `k` is a save-state slot key, perform the action and return the overlay
/// message. Bare digit `N` saves to slot N; the shifted digit (kitty terminals
/// report digit+SHIFT, legacy ones report the symbol) loads it.
fn handle_slot_key<C: Core>(k: &KeyEvent, core: &mut C, sav: &Path) -> Option<String> {
    let KeyCode::Char(c) = k.code else { return None };
    let digit = |c: char| c.to_digit(10).map(|d| d as u8);
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    // Bare digit -> save. Messages stay terse so the overlay badge stays a
    // small corner notification rather than a wide banner over the game.
    if c.is_ascii_digit() && !shift {
        let slot = digit(c).unwrap();
        return Some(match write_save(&ss_path(sav, slot), &core.save_state()) {
            Ok(()) => format!("saved {slot}"),
            Err(_) => format!("failed {slot}"),
        });
    }
    // Shifted digit (kitty) or its shifted symbol (legacy) -> load.
    const SYMBOLS: [char; 10] = [')', '!', '@', '#', '$', '%', '^', '&', '*', '('];
    let load_slot = if c.is_ascii_digit() && shift {
        digit(c)
    } else {
        SYMBOLS.iter().position(|&s| s == c).map(|i| i as u8)
    };
    let slot = load_slot?;
    Some(match std::fs::read(ss_path(sav, slot)) {
        Ok(d) => match core.load_state(&d) {
            Ok(()) => format!("loaded {slot}"),
            Err(_) => format!("failed {slot}"),
        },
        Err(_) => format!("empty {slot}"),
    })
}

/// If `k` is a playback control, apply it and return the overlay badge to
/// flash. `+`/`=` speed up, `-`/`_` slow down, `m` toggles audio mute. Like the
/// save-state digits, these keys are reserved — they never reach the game.
/// `<dir>/<rom-stem>-<unix_millis>.png` in the launch directory, creating `<dir>`
/// (`images` for screenshots, `clips` for recordings) if needed. Best-effort.
fn capture_path(sav: &Path, dir: &str) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let stem = sav.file_stem().unwrap_or_default().to_string_lossy();
    let folder = PathBuf::from(dir);
    let _ = std::fs::create_dir_all(&folder);
    folder.join(format!("{stem}-{ts}.png"))
}

/// `P` flags a screenshot (taken on the next frame); `R` toggles recording and
/// writes the clip on stop. Returns the overlay to show, an empty string for
/// "consumed silently", or `None` if it's not a capture key.
fn handle_capture_key(
    k: &KeyEvent,
    screenshot_pending: &mut bool,
    recorder: &mut Option<capture::Recorder>,
    sav: &Path,
) -> Option<String> {
    let KeyCode::Char(c) = k.code else { return None };
    match c.to_ascii_lowercase() {
        'p' => {
            *screenshot_pending = true;
            Some(String::new())
        }
        'r' => Some(match recorder.take() {
            Some(rec) => {
                let path = capture_path(sav, "clips");
                let label = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                match std::fs::write(&path, rec.finish()) {
                    Ok(()) => format!("saved clip {label}"),
                    Err(e) => format!("clip failed: {e}"),
                }
            }
            None => {
                *recorder = Some(capture::Recorder::new(3));
                "\u{25CF} REC".to_string() // ● REC
            }
        }),
        _ => None,
    }
}

fn handle_playback_key(k: &KeyEvent, pb: &mut playback::Playback) -> Option<String> {
    let KeyCode::Char(c) = k.code else { return None };
    match c {
        '+' | '=' => pb.faster(),
        '-' | '_' => pb.slower(),
        'm' => {
            pb.toggle_mute();
            return Some(pb.mute_label().to_string());
        }
        _ => return None,
    }
    Some(pb.speed_label())
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

    #[test]
    fn frame_periods_agree_across_cores() {
        // Each core's frame period in ns, computed identically from its own
        // documented cycles-per-frame and clock. The GBA has no double-speed
        // mode, so both must land on exactly the same period.
        let gb_ns = termboy_gb::CYCLES_PER_FRAME * 1_000_000_000 / termboy_gb::CLOCK_HZ;
        let gba_ns = termboy_gba::CYCLES_PER_FRAME * 1_000_000_000 / termboy_gba::CLOCK_HZ;
        assert_eq!(gb_ns, gba_ns, "GB and GBA frame periods must match");
        assert_eq!(FRAME_TIME, Duration::from_nanos(gb_ns), "FRAME_TIME must be the shared period");

        let hz = 1e9 / gb_ns as f64;
        assert!((hz - 59.7275).abs() < 0.001, "frame rate {hz} Hz, expected ~59.7275");
    }
}

/// The shared 59.7275 Hz frame period. Both cores emit one frame per this span
/// of wall-clock time — GB: 70_224 cycles @ 4.194304 MHz; GBA: 280_896 cycles
/// @ 16.777216 MHz — so the frontend paces every core identically and the GBA
/// needs no double-speed branch. `frame_periods_agree_across_cores` guards it.
const FRAME_TIME: Duration = Duration::from_nanos(
    termboy_gb::CYCLES_PER_FRAME * 1_000_000_000 / termboy_gb::CLOCK_HZ,
);

/// During rewind, advance one snapshot every this many displayed frames.
/// Snapshots are `rewind::INTERVAL` (6) game-frames apart, so stepping every 3
/// displayed frames replays ~2 game-frames per displayed frame → ~2x reverse —
/// controllable, instead of the 6x that overshot a short tap to the buffer floor.
const REWIND_STEP: u32 = 3;

/// Most emulated frames to run in one loop pass before pacing/dropping the
/// backlog. Kept small so a slow scene (or an unreachable target speed) can't
/// accumulate frames and replay them in one jerky burst between redraws — the
/// render gate, not a big batch, is what amortizes redraw cost now. Also bounds
/// catch-up after a stall (e.g. the laptop slept) so it never avalanches.
const MAX_CATCHUP: u32 = 4;

/// Which renderer to use. `Auto` picks kitty graphics on terminals that support
/// it (detected from the environment), half-blocks elsewhere.
#[derive(Clone, Copy)]
pub(crate) enum GraphicsPref {
    Auto,
    Kitty,
    Half,
}

impl GraphicsPref {
    fn use_kitty(self) -> bool {
        match self {
            GraphicsPref::Kitty => true,
            GraphicsPref::Half => false,
            GraphicsPref::Auto => kitty::detect_kitty(|k| std::env::var(k).ok()),
        }
    }
}

/// Active renderer: half-block ANSI or kitty graphics. The run loop drives
/// whichever through this one interface, so it stays renderer-agnostic.
enum Display {
    Half(screen::Screen),
    Kitty(kitty::KittyScreen),
}

impl Display {
    fn new(use_kitty: bool, width: usize, height: usize) -> Self {
        if use_kitty {
            Display::Kitty(kitty::KittyScreen::new(width, height))
        } else {
            Display::Half(screen::Screen::new(width, height))
        }
    }

    fn required_size(&self) -> (usize, usize) {
        match self {
            Display::Half(s) => s.required_size(),
            Display::Kitty(k) => k.required_size(),
        }
    }

    fn set_viewport(&mut self, cols: usize, rows: usize, cell_px: Option<(usize, usize)>) {
        match self {
            Display::Half(s) => s.set_viewport(cols, rows),
            Display::Kitty(k) => k.set_viewport(cols, rows, cell_px),
        }
    }

    fn invalidate(&mut self) {
        match self {
            Display::Half(s) => s.invalidate(),
            Display::Kitty(k) => k.invalidate(),
        }
    }

    fn set_overlay(&mut self, msg: &str) {
        match self {
            Display::Half(s) => s.set_overlay(msg),
            Display::Kitty(k) => k.set_overlay(msg),
        }
    }

    fn clear_overlay(&mut self) {
        match self {
            Display::Half(s) => s.clear_overlay(),
            Display::Kitty(k) => k.clear_overlay(),
        }
    }

    fn render(&mut self, fb: &termboy_core::FrameBuffer, out: &mut String) {
        match self {
            Display::Half(s) => s.render(fb, out),
            Display::Kitty(k) => k.render(fb, out),
        }
    }

    /// On exit, free any transmitted image so it doesn't linger in the terminal.
    fn teardown(&self, out: &mut String) {
        if let Display::Kitty(_) = self {
            out.push_str("\x1b_Ga=d,d=A\x1b\\");
        }
    }
}

/// Work sent from the emulation thread to the render thread. Frames carry the
/// current overlay so the worker needs no separate overlay channel; only the
/// newest queued frame is drawn (older ones are dropped), so a slow renderer
/// never backs up or stalls emulation.
enum RenderMsg {
    Frame { fb: termboy_core::FrameBuffer, overlay: Option<String> },
    Viewport { cols: usize, rows: usize, cell_px: Option<(usize, usize)> },
    Invalidate,
    /// Bytes to write verbatim (resize clear, too-small notice).
    Raw(String),
}

/// Handle to the render worker. The worker owns the `Display` and is the only
/// thread that writes to stdout, so emulation never spends time rendering or
/// blocking on terminal I/O. Dropping the handle closes the channel, which makes
/// the worker tear down its image and exit — this runs on normal return *and*
/// during a panic unwind, so the terminal is always left clean.
struct Renderer {
    tx: Option<std::sync::mpsc::Sender<RenderMsg>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Renderer {
    fn spawn(display: Display) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<RenderMsg>();
        let handle = std::thread::spawn(move || render_loop(display, rx));
        Self { tx: Some(tx), handle: Some(handle) }
    }

    fn send(&self, msg: RenderMsg) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(msg); // worker gone -> nothing to draw to; ignore
        }
    }

    fn frame(&self, fb: termboy_core::FrameBuffer, overlay: Option<String>) {
        self.send(RenderMsg::Frame { fb, overlay });
    }
    fn viewport(&self, cols: usize, rows: usize, cell_px: Option<(usize, usize)>) {
        self.send(RenderMsg::Viewport { cols, rows, cell_px });
    }
    fn invalidate(&self) {
        self.send(RenderMsg::Invalidate);
    }
    fn raw(&self, s: String) {
        self.send(RenderMsg::Raw(s));
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.tx.take(); // close the channel so the worker finishes and exits
        if let Some(h) = self.handle.take() {
            let _ = h.join(); // wait for its teardown before the terminal is reused
        }
    }
}

/// Render worker loop: drains the channel each wake-up, applying control
/// messages in order but keeping only the most recent frame, then draws that
/// one. A redraw never blocks emulation (a different thread) and is capped near
/// the display rate so fast-forward can't flood the terminal.
fn render_loop(mut display: Display, rx: std::sync::mpsc::Receiver<RenderMsg>) {
    use std::io::Write as _;
    use std::sync::mpsc::TryRecvError;
    let write = |s: &str| {
        let mut so = std::io::stdout();
        let _ = so.write_all(s.as_bytes());
        let _ = so.flush();
    };
    let mut last_overlay: Option<String> = None;
    let mut out = String::new();
    let mut last_draw = Instant::now();
    loop {
        // Block for the next message, then drain whatever else is queued so we
        // render only the freshest frame (control messages still apply in order).
        let Ok(mut msg) = rx.recv() else { break }; // handle dropped -> quit
        let mut latest: Option<(termboy_core::FrameBuffer, Option<String>)> = None;
        let mut disconnected = false;
        loop {
            match msg {
                RenderMsg::Frame { fb, overlay } => latest = Some((fb, overlay)),
                RenderMsg::Viewport { cols, rows, cell_px } => {
                    display.set_viewport(cols, rows, cell_px)
                }
                RenderMsg::Invalidate => display.invalidate(),
                RenderMsg::Raw(s) => write(&s),
            }
            match rx.try_recv() {
                Ok(m) => msg = m,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if let Some((fb, overlay)) = latest {
            // Cap the redraw rate so fast-forward doesn't stream frames faster
            // than the terminal can usefully show them.
            let since = last_draw.elapsed();
            if since < FRAME_TIME {
                std::thread::sleep(FRAME_TIME - since);
            }
            if overlay != last_overlay {
                match &overlay {
                    Some(m) => display.set_overlay(m),
                    None => display.clear_overlay(),
                }
                last_overlay = overlay;
            }
            out.clear();
            display.render(&fb, &mut out);
            if !out.is_empty() {
                write(&out);
            }
            last_draw = Instant::now();
        }
        if disconnected {
            break;
        }
    }
    // Free any transmitted image so it doesn't linger over the picker / on exit.
    out.clear();
    display.teardown(&mut out);
    if !out.is_empty() {
        write(&out);
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let headless = args.iter().any(|a| a == "--headless");
    let exact = args.iter().any(|a| a == "--exact");
    let color_correct = args.iter().any(|a| a == "--color-correct");
    // Defaults < config file < CLI flags.
    let mut settings = config::Settings::default();
    match config_path_arg(&args) {
        Some(path) => config::load(&path, &mut settings),
        None => {
            if let Some(path) = config::default_path() {
                config::ensure(&path); // first run: drop a commented starter config
                config::load(&path, &mut settings);
            }
        }
    }

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
        } else if let Some(n) = arg.strip_prefix("--graphics=") {
            ("graphics", Some(n.to_string()))
        } else if arg == "--graphics" {
            ("graphics", it.next().cloned())
        } else if arg == "--config" {
            it.next(); // path already consumed by config_path_arg
            ("", None)
        } else if arg.strip_prefix("--config=").is_some() {
            ("", None)
        } else {
            if !arg.starts_with('-') {
                rom_arg = Some(arg);
            }
            ("", None)
        };
        if !flag.is_empty() {
            if let Some(v) = value {
                if let Err(e) = config::apply_setting(&mut settings, flag, &v) {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    }
    if exact {
        settings.exact = true;
    }
    if color_correct {
        settings.color_correct = true;
    }
    match rom_arg {
        Some(path) if is_gba(path) => {
            if headless {
                eprintln!("error: --headless is for GB serial test ROMs; GBA has no serial console");
                return ExitCode::FAILURE;
            }
            match load_gba(path) {
                Ok(core) => run_terminal(core, 240, 160, &sav_path(path), &settings),
                Err(msg) => {
                    eprintln!("error: {msg}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(path) => {
            let (gb, sav) = match load_game(path, settings.palette) {
                Ok(pair) => pair,
                Err(msg) => {
                    eprintln!("error: {msg}");
                    return ExitCode::FAILURE;
                }
            };
            if headless {
                run_headless(gb, &sav)
            } else {
                run_terminal(gb, 160, 144, &sav, &settings)
            }
        }
        None if headless => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
        None => run_menu(&settings),
    }
}

/// Terminal pixels-per-cell `(width, height)`, used to upscale graphics frames
/// to the exact on-screen size. `None` on terminals that don't report a pixel
/// size — the kitty renderer then sends native frames and lets them scale.
fn cell_pixels() -> Option<(usize, usize)> {
    let ws = terminal::window_size().ok()?;
    if ws.width == 0 || ws.height == 0 || ws.columns == 0 || ws.rows == 0 {
        return None;
    }
    Some((ws.width as usize / ws.columns as usize, ws.height as usize / ws.rows as usize))
}

/// First `--config <path>` / `--config=<path>` on the command line, if any.
fn config_path_arg(args: &[String]) -> Option<PathBuf> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(p) = a.strip_prefix("--config=") {
            return Some(PathBuf::from(p));
        }
        if a == "--config" {
            return it.next().map(PathBuf::from);
        }
    }
    None
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
/// How a game session ended: back to the picker, or quit the app.
enum GameExit {
    Library,
    Quit,
}

fn run_menu(settings: &config::Settings) -> ExitCode {
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
    let use_kitty = settings.graphics.use_kitty();
    loop {
        let Some(i) = menu::pick(&roms) else {
            return ExitCode::SUCCESS;
        };
        let path = roms[i].path.to_string_lossy().into_owned();
        let outcome: Result<GameExit, String> = if roms[i].kind == menu::Kind::Advance {
            match load_gba(&path) {
                Ok(mut core) => {
                    core.set_color_correction(settings.color_correct);
                    let mut input = input::Input::new(guard.enhanced, settings.keymap.clone());
                    let screen = Display::new(use_kitty, 240, 160);
                    print!("\x1b[0m\x1b[2J");
                    let sav = sav_path(&path);
                    let audio = &audio; // keep the move closure from consuming it
                    let exact = settings.exact;
                    let playback = playback::Playback::with(settings.speed, settings.muted);
                    catch_game_panic(move || {
                        run_game(core, exact, &sav, &mut input, screen, audio, playback, true)
                    })
                }
                Err(msg) => Err(msg),
            }
        } else {
            match load_game(&path, settings.palette) {
                Ok((mut gb, sav)) => {
                    gb.set_color_correction(settings.color_correct);
                    let mut input = input::Input::new(guard.enhanced, settings.keymap.clone());
                    let screen = Display::new(use_kitty, 160, 144);
                    print!("\x1b[0m\x1b[2J");
                    let audio = &audio;
                    let exact = settings.exact;
                    let playback = playback::Playback::with(settings.speed, settings.muted);
                    catch_game_panic(move || {
                        run_game(gb, exact, &sav, &mut input, screen, audio, playback, true)
                    })
                }
                Err(msg) => Err(msg),
            }
        };
        match outcome {
            Ok(GameExit::Library) => {} // loop re-shows the picker
            Ok(GameExit::Quit) => return ExitCode::SUCCESS,
            Err(msg) => {
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
}

/// Run a game, converting a core panic into a picker-visible message
/// instead of killing the whole app (GBA cores hit loud unimplemented!
/// seams until G4 fills them in).
fn catch_game_panic<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(v) => Ok(v),
        Err(e) => {
            let detail = e
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| e.downcast_ref::<&str>().copied())
                .unwrap_or("unknown panic");
            Err(format!("this game needs a later milestone: {detail}"))
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
        // Disable "alternate scroll" (DEC private mode 1007). On the alternate
        // screen, terminals translate mouse-wheel/trackpad scrolling into arrow-
        // key presses (so less/vim scroll with the wheel) — here that injects
        // phantom D-pad Up/Down into the game. termboy takes no mouse input, so
        // turn it off; Drop restores it. We never enable mouse capture, so this
        // is the whole of the mouse story.
        let mut o = std::io::stdout();
        write!(o, "\x1b[?1007l")?;
        o.flush()?;
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
        // Restore the alternate-scroll mode disabled in enter().
        let _ = write!(std::io::stdout(), "\x1b[?1007h");
        let _ = std::io::stdout().flush();
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
    mut core: C,
    width: usize,
    height: usize,
    sav: &Path,
    settings: &config::Settings,
) -> ExitCode {
    install_panic_hook();
    let guard = match TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: cannot enter raw mode: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut input = input::Input::new(guard.enhanced, settings.keymap.clone());
    let screen = Display::new(settings.graphics.use_kitty(), width, height);
    let audio = audio::Audio::new();
    core.set_color_correction(settings.color_correct);
    let playback = playback::Playback::with(settings.speed, settings.muted);
    match run_game(core, settings.exact, sav, &mut input, screen, &audio, playback, false) {
        GameExit::Library | GameExit::Quit => ExitCode::SUCCESS,
    }
}

/// Run one game until Esc. Assumes raw mode + alternate screen are active.
/// Emulation runs on this thread; rendering and *all* terminal writes happen on
/// a worker (`Renderer`), so a redraw never steals CPU from the core or blocks
/// it on terminal I/O. We hand the worker the latest frame each pass and it
/// draws the newest, dropping the rest — emulation is never gated by render
/// cost, so fast-forward runs as fast as the core itself allows.
fn run_game<C: Core>(
    mut core: C,
    exact: bool,
    sav: &Path,
    input: &mut input::Input,
    screen: Display,
    audio: &audio::Audio,
    mut playback: playback::Playback,
    has_library: bool,
) -> GameExit {
    core.set_audio_rate(audio.sample_rate);
    let mut audio_buf: Vec<(f32, f32)> = Vec::new();
    let (need_cols, need_rows) = screen.required_size();
    let renderer = Renderer::spawn(screen);
    renderer.invalidate();
    let mut last_size = (0u16, 0u16);

    // Wall-clock time the next emulated frame is due; emulation is paced to this
    // (so the selected speed is exact when the core can reach it).
    let mut next_emu = Instant::now();
    let mut last_saved: Option<Vec<u8>> = core.save_ram();
    let mut frames: u32 = 0;
    // Transient overlay text (save/speed badge, or "rewind"), handed to the
    // worker with each frame until it expires — the worker owns the Display, so
    // the overlay travels with the frame rather than being set on it directly.
    let mut overlay: Option<String> = None;
    let mut overlay_until = Instant::now();
    let mut rewind = rewind::Rewind::new();
    let mut rewind_key = input::HoldKey::new(input.enhanced());
    let mut rewind_tick: u32 = 0;
    let mut recorder: Option<capture::Recorder> = None;
    let mut screenshot_pending = false;
    let mut capture_tick: u32 = 0;
    let mut last_fb: Option<termboy_core::FrameBuffer> = None;
    let mut paused: Option<pause::Menu> = None;
    let mut menu_dirty = false;
    let code = 'game: loop {
        // Input: Esc opens the pause menu, number keys drive save-state slots,
        // the rest goes to the button tracker.
        let now = Instant::now();

        if paused.is_some() {
            // --- PAUSED: drive the menu; no emulation runs. ---
            // Keep the panel centered if the terminal was resized.
            let (cols, rows) = terminal::size().unwrap_or((0, 0));
            if (cols, rows) != last_size {
                last_size = (cols, rows);
                renderer.viewport(cols as usize, rows as usize, cell_pixels());
                renderer.invalidate();
                renderer.raw("\x1b[0m\x1b[2J".to_string());
                menu_dirty = true;
            }
            if menu_dirty {
                if let Some(fb) = &last_fb {
                    renderer.frame(pause::compose_menu(fb, &paused.as_ref().unwrap().view()), None);
                }
                menu_dirty = false;
            }
            // Block briefly for input so we don't busy-spin while paused.
            if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                if let Ok(Event::Key(k)) = event::read() {
                    if matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                        let action = {
                            let menu = paused.as_mut().unwrap();
                            match k.code {
                                KeyCode::Up | KeyCode::Char('k') => {
                                    menu.up();
                                    Action::None
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    menu.down();
                                    Action::None
                                }
                                KeyCode::Left => menu.left(),
                                KeyCode::Right => menu.right(),
                                KeyCode::Enter => menu.select(),
                                KeyCode::Esc => menu.back(),
                                _ => Action::None,
                            }
                        };
                        menu_dirty = true;
                        match action {
                            Action::None => {}
                            Action::SpeedFaster => {
                                playback.faster();
                                core.set_audio_rate(playback.audio_rate(audio.sample_rate));
                                paused.as_mut().unwrap().set_speed_label(playback.multiplier_label());
                            }
                            Action::SpeedSlower => {
                                playback.slower();
                                core.set_audio_rate(playback.audio_rate(audio.sample_rate));
                                paused.as_mut().unwrap().set_speed_label(playback.multiplier_label());
                            }
                            Action::ToggleMute => {
                                playback.toggle_mute();
                                paused.as_mut().unwrap().set_muted(playback.is_muted());
                            }
                            Action::Resume => {
                                paused = None;
                                input.release_all();
                                renderer.invalidate();
                                last_size = (0, 0);
                                next_emu = Instant::now();
                            }
                            Action::Save(slot) => {
                                overlay = Some(
                                    match write_save(&ss_path(sav, slot as u8), &core.save_state()) {
                                        Ok(()) => format!("saved {slot}"),
                                        Err(_) => format!("failed {slot}"),
                                    },
                                );
                                overlay_until = Instant::now() + Duration::from_millis(1500);
                                paused = None;
                                input.release_all();
                                renderer.invalidate();
                                last_size = (0, 0);
                                next_emu = Instant::now();
                            }
                            Action::Load(slot) => {
                                overlay = Some(match std::fs::read(ss_path(sav, slot as u8)) {
                                    Ok(d) => match core.load_state(&d) {
                                        Ok(()) => format!("loaded {slot}"),
                                        Err(_) => format!("failed {slot}"),
                                    },
                                    Err(_) => format!("empty {slot}"),
                                });
                                overlay_until = Instant::now() + Duration::from_millis(1500);
                                paused = None;
                                input.release_all();
                                renderer.invalidate();
                                last_size = (0, 0);
                                next_emu = Instant::now();
                            }
                            Action::Library => break 'game GameExit::Library,
                            Action::Quit => break 'game GameExit::Quit,
                        }
                    }
                }
            }
            continue;
        }

        while event::poll(Duration::ZERO).unwrap_or(false) {
            match event::read() {
                // Backspace (any kind) drives hold-to-rewind, so it needs the
                // release events too — handle it before the press-only arm.
                Ok(Event::Key(k)) if k.code == KeyCode::Backspace => {
                    rewind_key.handle(k.kind, now);
                }
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                    if k.code == KeyCode::Esc {
                        // Open the pause menu (a frame must exist to dim behind it).
                        if last_fb.is_some() {
                            paused = Some(pause::Menu::new(
                                has_library,
                                playback.multiplier_label(),
                                playback.is_muted(),
                                slots_filled(sav),
                            ));
                            menu_dirty = true;
                            continue 'game;
                        }
                    }
                    if let Some(msg) = handle_capture_key(&k, &mut screenshot_pending, &mut recorder, sav) {
                        if !msg.is_empty() {
                            overlay = Some(msg);
                            overlay_until = Instant::now() + Duration::from_millis(1500);
                        }
                    } else if let Some(msg) = handle_slot_key(&k, &mut core, sav) {
                        overlay = Some(msg);
                        overlay_until = Instant::now() + Duration::from_millis(1500);
                    } else if let Some(msg) = handle_playback_key(&k, &mut playback) {
                        // A speed change moves the wall-clock frame rate, so
                        // retarget the APU's resample rate to keep sample
                        // production matched to the device — otherwise 0.5x
                        // underruns into silence-laced crackle. (Mute leaves the
                        // multiplier unchanged, so this is a no-op for it.)
                        core.set_audio_rate(playback.audio_rate(audio.sample_rate));
                        overlay = Some(msg);
                        overlay_until = Instant::now() + Duration::from_millis(1500);
                    } else {
                        input.handle(&k, now);
                    }
                }
                Ok(Event::Key(k)) => input.handle(&k, now),
                Ok(Event::Resize(..)) => renderer.invalidate(),
                _ => {}
            }
        }
        if overlay_until <= now {
            overlay = None;
        }

        let (cols, rows) = terminal::size().unwrap_or((0, 0));
        let too_small = if exact {
            (cols as usize) < need_cols || (rows as usize) < need_rows
        } else {
            cols < 16 || rows < 8 // below this nothing recognizable fits
        };
        if too_small {
            let (nc, nr) = if exact { (need_cols, need_rows) } else { (16, 8) };
            renderer.raw(format!(
                "\x1b[0m\x1b[2J\x1b[Htermboy needs a {nc}x{nr} terminal (yours: {cols}x{rows}). Resize, or press Esc to quit."
            ));
            renderer.invalidate();
            last_size = (0, 0);
            std::thread::sleep(Duration::from_millis(100));
            next_emu = Instant::now();
            continue;
        }
        if (cols, rows) != last_size {
            last_size = (cols, rows);
            renderer.viewport(cols as usize, rows as usize, cell_pixels());
            renderer.invalidate();
            renderer.raw("\x1b[0m\x1b[2J".to_string()); // clear leftovers outside the (re)centered image
        }

        let frame_dt = playback.frame_dt(FRAME_TIME);
        if rewind_key.is_held(now) {
            // Rewind: step back through snapshots, newest first, but only every
            // REWIND_STEP displayed frames so it reverses at ~2x (not 6x). Audio
            // is silenced (reverse audio is just noise). On non-step frames we
            // hand the worker nothing, so the screen holds; when the ring runs
            // dry it freezes on the oldest available state.
            if rewind_tick.is_multiple_of(REWIND_STEP) && let Some(snap) = rewind.pop() {
                let _ = core.load_state(&snap);
                let fb = core.run_frame(Buttons::default());
                renderer.frame(fb.clone(), Some("rewind".to_string()));
                core.drain_audio(&mut audio_buf);
                audio_buf.clear();
            }
            rewind_tick = rewind_tick.wrapping_add(1);
            overlay_until = now + Duration::from_millis(400);
            next_emu = now + FRAME_TIME; // rewind isn't speed-scaled: pace at 60 Hz
        } else {
            rewind_tick = 0; // fresh hold steps immediately
            // Catch emulation up to the wall clock — a few frames at most, so a
            // slow scene can't build a backlog and replay it in a jerky burst. We
            // hand the worker only the last frame produced; it draws the newest
            // and drops the rest, so emulation is never gated by render cost.
            // Audio drains every emulated frame, so production stays matched.
            let due = playback::frames_due(now.saturating_duration_since(next_emu), frame_dt, MAX_CATCHUP);
            let buttons = input.buttons(now);
            for i in 0..due {
                let fb = core.run_frame(buttons);
                if i + 1 == due {
                    renderer.frame(fb.clone(), overlay.clone());
                    last_fb = Some(fb.clone());
                    if screenshot_pending {
                        screenshot_pending = false;
                        let path = capture_path(sav, "images");
                        let label = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        overlay = Some(match std::fs::write(&path, capture::encode_png(fb, 4)) {
                            Ok(()) => format!("saved {label}"),
                            Err(e) => format!("screenshot failed: {e}"),
                        });
                        overlay_until = Instant::now() + Duration::from_millis(1500);
                    }
                    let mut auto_save = false;
                    if let Some(rec) = recorder.as_mut() {
                        capture_tick = capture_tick.wrapping_add(1);
                        if capture_tick % 2 == 0 {
                            rec.push(fb);
                        }
                        auto_save = rec.len() >= 600; // ~20s safety cap
                    }
                    if auto_save && let Some(rec) = recorder.take() {
                        let path = capture_path(sav, "clips");
                        let label = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        let _ = std::fs::write(&path, rec.finish());
                        overlay = Some(format!("saved clip {label}"));
                        overlay_until = Instant::now() + Duration::from_millis(1500);
                    }
                }
                core.drain_audio(&mut audio_buf);
                if playback.is_muted() {
                    audio_buf.clear();
                } else {
                    audio.push(&mut audio_buf);
                }
                // Snapshot for rewind only on normal frames (the closure runs
                // ~1/INTERVAL of the time, so save_state's cost is amortized).
                rewind.record(|| core.save_state());
            }
            next_emu += frame_dt * due;
        }

        frames += 1;
        if frames % 300 == 0 {
            flush_save(&core, sav, &mut last_saved); // every ~5 seconds
        }

        // Pace to the next due frame. If we've fallen more than a full catch-up
        // window behind (the app was suspended, or the core can't reach this
        // speed), drop the backlog instead of replaying it in a burst.
        let now = Instant::now();
        if now.saturating_duration_since(next_emu) > frame_dt * MAX_CATCHUP {
            next_emu = now;
        } else if next_emu > now {
            std::thread::sleep(next_emu - now);
        }
    };
    flush_save(&core, sav, &mut last_saved);
    code // `renderer` drops here: it closes the channel, the worker tears down
         // its image, and we join — so the terminal is clean before we return.
}
