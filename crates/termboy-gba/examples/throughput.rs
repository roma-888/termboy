//! Measure GBA emulation throughput (no terminal rendering): real frames the
//! core emulates per second, and the multiple of real time.
//!
//!   cargo run --release -p termboy-gba --example throughput -- <rom.gba>
//!
//! Real-time target is 59.7275 Hz (16.7427 ms/frame); a result above 1.00x
//! means full speed with headroom. Requires a local ROM — commercial ROMs are
//! never committed, so this lives as an example rather than a test.
use std::time::Instant;

use termboy_core::{Buttons, Core};
use termboy_gba::{CLOCK_HZ, CYCLES_PER_FRAME, GbaCore};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: cargo run --release -p termboy-gba --example throughput -- <rom.gba>");
        std::process::exit(2);
    };
    let rom = match std::fs::read(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let mut core = match GbaCore::new(rom) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("not a GBA ROM: {e}");
            std::process::exit(1);
        }
    };
    core.set_audio_rate(48_000);

    // One reused buffer: nothing in the timed loop should allocate per frame.
    let mut audio: Vec<(f32, f32)> = Vec::new();

    // Warm up past the BIOS/intro so we time steady-state load, not boot.
    for f in 0..240u32 {
        let b = if f % 40 < 4 { Buttons::A } else { Buttons::default() };
        core.run_frame(b);
        core.drain_audio(&mut audio);
        audio.clear();
    }

    let n = 600u32;
    let start = Instant::now();
    for f in 0..n {
        let b = if f % 40 < 4 { Buttons::START } else { Buttons::default() };
        core.run_frame(b);
        core.drain_audio(&mut audio);
        audio.clear();
    }
    let total = start.elapsed().as_secs_f64();
    let per = total / n as f64;
    let target = CYCLES_PER_FRAME as f64 / CLOCK_HZ as f64; // one real GBA frame, in seconds

    println!("frames:         {n}");
    println!("total:          {total:.3} s");
    println!("per frame:      {:.3} ms", per * 1e3);
    println!("throughput:     {:.1} fps", 1.0 / per);
    println!("realtime speed: {:.2}x  (target 1.00x = {:.4} Hz)", target / per, 1.0 / target);
}
