//! Milestone-1 headless runner: executes a ROM and streams its serial output.
//! The terminal frontend replaces this in Milestone 2.

use std::io::Write;
use std::process::ExitCode;

use termboy_core::{Buttons, Core};
use termboy_gb::GameBoy;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: termboy <rom.gb>");
        return ExitCode::FAILURE;
    };
    let rom = match std::fs::read(&path) {
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
