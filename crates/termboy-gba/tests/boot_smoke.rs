//! Commercial-boot smoke test. Ignored by default: needs a real ROM at
//! roms/ in the repo root and takes a while in debug builds. Run with:
//!   cargo test -p termboy-gba --release --test boot_smoke -- --ignored

use termboy_core::{Buttons, Core};
use termboy_gba::GbaCore;

#[test]
#[ignore]
fn commercial_rom_draws_a_real_frame() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../roms/Pokemon Ultra Violet.gba");
    let rom = std::fs::read(path).expect("commercial ROM not present");
    let mut core = GbaCore::new(rom).unwrap();
    let mut pressed = Buttons::default();
    for frame in 0..900 {
        // tap A/Start periodically to get past intro screens
        pressed = if frame % 90 < 4 {
            if frame % 180 < 90 { Buttons::A } else { Buttons::START }
        } else {
            Buttons::default()
        };
        core.run_frame(pressed);
    }
    let fb = core.run_frame(pressed);
    let first = fb.pixels[0];
    assert!(
        fb.pixels.iter().any(|p| *p != first),
        "screen is a solid color 15 seconds in — boot failed"
    );
}
