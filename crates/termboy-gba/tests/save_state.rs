use termboy_core::{Buttons, Core};
use termboy_gba::GbaCore;

fn boot(rom_file: &str, frames: u32) -> GbaCore {
    let path = format!("{}/tests/roms/{rom_file}", env!("CARGO_MANIFEST_DIR"));
    let rom = std::fs::read(&path).expect("test ROM — run scripts/fetch-test-roms.sh");
    let mut c = GbaCore::new(rom).unwrap();
    for _ in 0..frames {
        c.run_frame(Buttons::default());
    }
    c
}

#[test]
fn save_load_resumes_identically() {
    let mut c = boot("nes.gba", 30);
    let snap = c.save_state();
    let want: Vec<_> = {
        let mut g = boot("nes.gba", 0);
        g.load_state(&snap).unwrap();
        for _ in 0..20 {
            g.run_frame(Buttons::default());
        }
        g.run_frame(Buttons::default()).pixels.clone()
    };
    c.load_state(&snap).unwrap();
    for _ in 0..20 {
        c.run_frame(Buttons::default());
    }
    assert_eq!(c.run_frame(Buttons::default()).pixels, want);
}

#[test]
fn save_load_save_is_idempotent_nes() {
    let c = boot("nes.gba", 30);
    let snap = c.save_state();
    let mut g2 = boot("nes.gba", 0);
    g2.load_state(&snap).unwrap();
    assert_eq!(g2.save_state(), snap, "a kept field was not restored by load");
}

#[test]
fn save_load_save_is_idempotent_flash() {
    // Exercises the Flash save backend's serialization (save_flash128.gba detects 128K Flash).
    let c = boot("save_flash128.gba", 60);
    let snap = c.save_state();
    let mut g2 = boot("save_flash128.gba", 0);
    g2.load_state(&snap).unwrap();
    assert_eq!(g2.save_state(), snap);
}

#[test]
fn save_load_save_is_idempotent_sram() {
    let c = boot("save_sram.gba", 60);
    let snap = c.save_state();
    let mut g2 = boot("save_sram.gba", 0);
    g2.load_state(&snap).unwrap();
    assert_eq!(g2.save_state(), snap);
}

#[test]
fn header_rejects_wrong_console() {
    let mut c = boot("nes.gba", 0);
    let mut s = c.save_state();
    s[6] = 0; // console byte -> GB
    assert!(c.load_state(&s).is_err());
}
