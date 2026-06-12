//! jsmolka ppu/hello.gba: "Hello world!" drawn in mode 4 (verified in the
//! ROM source: text_init sets DISPCNT = 4 | BG2; glyphs land at x=72 y=76).
//! Structural asserts catch gross breakage; the byte-exact golden (blessed
//! visually via UPDATE_GOLDEN=1, see acid2.rs for the convention) catches
//! everything else.

use termboy_core::{Buttons, Core, Rgb};
use termboy_gba::GbaCore;

/// Bless-or-compare a frame against tests/golden/<name>.bin (see acid2.rs
/// for the convention: UPDATE_GOLDEN=1 writes the blob; verify visually).
fn check_golden(name: &str, fb: &termboy_core::FrameBuffer) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let bytes: Vec<u8> = fb.pixels.iter().flat_map(|p| [p.0, p.1, p.2]).collect();
    let golden_path = dir.join(format!("golden/{name}.bin"));
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
        std::fs::write(&golden_path, &bytes).unwrap();
        panic!("{name} golden updated — verify VISUALLY, then rerun without UPDATE_GOLDEN");
    }
    let golden = std::fs::read(&golden_path)
        .expect("missing golden — bless with UPDATE_GOLDEN=1 after visual verification");
    assert_eq!(bytes, golden, "{name} differs from the blessed golden");
}

fn run_rom_frames(name: &str, frames: u32) -> GbaCore {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let rom = std::fs::read(dir.join(format!("roms/{name}")))
        .expect("missing ROM — run scripts/fetch-test-roms.sh");
    let mut core = GbaCore::new(rom).unwrap();
    for _ in 0..frames {
        core.run_frame(Buttons::default());
    }
    core
}

#[test]
fn hello_world_matches_golden() {
    let mut core = run_rom_frames("hello.gba", 5);
    let fb = core.run_frame(Buttons::default());

    assert_eq!(fb.pixels[0], Rgb(0, 0, 0), "backdrop must be black");
    let white = fb.pixels.iter().filter(|p| p.0 > 200).count();
    assert!((50..3000).contains(&white), "implausible white pixel count {white}");
    let text_band: usize = (76 * 240..84 * 240).filter(|&i| fb.pixels[i].0 > 200).count();
    assert_eq!(white, text_band, "white pixels exist outside the text rows");
    check_golden("hello", fb);
}

#[test]
fn stripes_renders_alternating_columns() {
    let mut core = run_rom_frames("stripes.gba", 3);
    let fb = core.run_frame(Buttons::default());
    // 8px columns alternating palette[1]=0x6290 (tile) / palette[0]=0x560B
    assert_ne!(fb.pixels[0], fb.pixels[8], "columns must alternate");
    assert_eq!(fb.pixels[0], fb.pixels[16], "period must be 16px");
    assert_eq!(fb.pixels[0], fb.pixels[120 * 240], "rows identical");
    check_golden("stripes", fb);
}

#[test]
fn shades_renders_blue_gradient() {
    let mut core = run_rom_frames("shades.gba", 3);
    let fb = core.run_frame(Buttons::default());
    // palette = i*2 << 10: pure blues, red/green channels exactly 0
    assert!(fb.pixels.iter().all(|p| p.0 == 0 && p.1 == 0), "non-blue pixel found");
    let distinct: std::collections::HashSet<u8> = fb.pixels[..240].iter().map(|p| p.2).collect();
    assert!(distinct.len() >= 8, "expected a gradient, got {} blues", distinct.len());
    check_golden("shades", fb);
}
