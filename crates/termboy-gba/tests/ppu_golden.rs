//! jsmolka ppu/hello.gba: "Hello world!" drawn in mode 4 (verified in the
//! ROM source: text_init sets DISPCNT = 4 | BG2; glyphs land at x=72 y=76).
//! Structural asserts catch gross breakage; the byte-exact golden (blessed
//! visually via UPDATE_GOLDEN=1, see acid2.rs for the convention) catches
//! everything else.

use termboy_core::{Buttons, Core, Rgb};
use termboy_gba::GbaCore;

#[test]
fn hello_world_matches_golden() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let rom = std::fs::read(dir.join("roms/hello.gba"))
        .expect("missing hello.gba — run scripts/fetch-test-roms.sh");
    let mut core = GbaCore::new(rom).unwrap();
    for _ in 0..5 {
        core.run_frame(Buttons::default()); // ROM vsyncs once, then draws
    }
    let fb = core.run_frame(Buttons::default());

    assert_eq!(fb.pixels[0], Rgb(0, 0, 0), "backdrop must be black");
    let white = fb.pixels.iter().filter(|p| p.0 > 200).count();
    assert!((50..3000).contains(&white), "implausible white pixel count {white}");
    let text_band: usize = (76 * 240..84 * 240).filter(|&i| fb.pixels[i].0 > 200).count();
    assert_eq!(white, text_band, "white pixels exist outside the text rows");

    let bytes: Vec<u8> = fb.pixels.iter().flat_map(|p| [p.0, p.1, p.2]).collect();
    let golden_path = dir.join("golden/hello.bin");
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
        std::fs::write(&golden_path, &bytes).unwrap();
        panic!(
            "golden updated — verify VISUALLY (cargo run -- crates/termboy-gba/tests/roms/hello.gba), then rerun without UPDATE_GOLDEN"
        );
    }
    let golden = std::fs::read(&golden_path)
        .expect("missing golden — bless with UPDATE_GOLDEN=1 after visual verification");
    assert_eq!(bytes, golden, "frame differs from the blessed golden");
}
