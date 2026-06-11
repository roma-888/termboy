//! Terminal frontend: half-block rendering at the Game Boy's 59.73 Hz.
//! `--headless` runs without UI and streams serial output (debug tool).

mod screen;

use std::io::Write as _;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode};
use crossterm::{cursor, execute, terminal};
use termboy_core::{Buttons, Core};
use termboy_gb::GameBoy;

/// 4_194_304 Hz / 70_224 cycles ≈ 59.73 fps.
const FRAME_TIME: Duration = Duration::from_nanos(70_224 * 1_000_000_000 / 4_194_304);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let headless = args.iter().any(|a| a == "--headless");
    let Some(path) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: termboy [--headless] <rom.gb>");
        return ExitCode::FAILURE;
    };
    let rom = match std::fs::read(path) {
        Ok(rom) => rom,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let gb = match GameBoy::new(rom) {
        Ok(gb) => gb,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if headless {
        run_headless(gb)
    } else {
        run_terminal(gb)
    }
}

fn run_headless(mut gb: GameBoy) -> ExitCode {
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
    ExitCode::SUCCESS
}

/// Restores the terminal even on panic (design spec §4).
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> std::io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(
            std::io::stdout(),
            terminal::EnterAlternateScreen,
            cursor::Hide,
            terminal::Clear(terminal::ClearType::All)
        )?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            std::io::stdout(),
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

fn run_terminal(mut gb: GameBoy) -> ExitCode {
    let mut screen = screen::Screen::new(160, 144);
    let (need_cols, need_rows) = screen.required_size();

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

    let _guard = match TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: cannot enter raw mode: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut out = String::new();
    let mut next_frame = Instant::now();
    loop {
        // Input: Esc quits. (Game buttons land in Milestone 3.)
        while event::poll(Duration::ZERO).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(k)) if k.code == KeyCode::Esc => return ExitCode::SUCCESS,
                Ok(Event::Resize(..)) => screen.invalidate(),
                _ => {}
            }
        }

        let (cols, rows) = terminal::size().unwrap_or((0, 0));
        if (cols as usize) < need_cols || (rows as usize) < need_rows {
            out.clear();
            out.push_str("\x1b[2J\x1b[H\x1b[0m");
            out.push_str(&format!(
                "termboy needs a {need_cols}x{need_rows} terminal (yours: {cols}x{rows}). Resize, or press Esc to quit."
            ));
            print!("{out}");
            std::io::stdout().flush().ok();
            screen.invalidate();
            std::thread::sleep(Duration::from_millis(100));
            next_frame = Instant::now();
            continue;
        }

        let fb = gb.run_frame(Buttons::default());
        out.clear();
        screen.render(fb, &mut out);
        if !out.is_empty() {
            print!("{out}");
            std::io::stdout().flush().ok();
        }

        next_frame += FRAME_TIME;
        let now = Instant::now();
        if next_frame > now {
            std::thread::sleep(next_frame - now);
        } else {
            next_frame = now; // fell behind: don't try to catch up in a burst
        }
    }
}
