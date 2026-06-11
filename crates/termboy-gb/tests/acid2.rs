//! dmg-acid2 renders a face exercising every DMG PPU behavior; the image is
//! stable after a few frames. Golden = raw 160x144 shade indices, blessed
//! with UPDATE_GOLDEN=1 after VISUAL verification against the reference
//! image (https://github.com/mattcurrie/dmg-acid2 README, reference-dmg.png).

use std::path::Path;

use termboy_core::{Buttons, Core};
use termboy_gb::GameBoy;

#[test]
fn dmg_acid2_matches_golden() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let rom = std::fs::read(dir.join("roms/dmg-acid2/dmg-acid2.gb"))
        .expect("missing dmg-acid2.gb — run scripts/fetch-test-roms.sh");
    let mut gb = GameBoy::new(rom).expect("rom loads");
    for _ in 0..120 {
        gb.run_frame(Buttons::default()); // 2 seconds: image is long stable
    }
    let shades = gb.frame_shades();

    let golden_path = dir.join("golden/dmg-acid2.bin");
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
        std::fs::write(&golden_path, shades).unwrap();
        panic!("golden updated — verify VISUALLY against reference-dmg.png, then rerun without UPDATE_GOLDEN");
    }
    let golden = std::fs::read(&golden_path)
        .expect("missing golden — bless with UPDATE_GOLDEN=1 after visual verification");
    let diff = shades.iter().zip(&golden).filter(|(a, b)| a != b).count();
    assert_eq!(diff, 0, "{diff} of 23040 pixels differ from blessed golden");
}
