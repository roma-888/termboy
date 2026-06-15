use termboy_core::{Buttons, Core};
use termboy_gb::GameBoy;

fn boot(frames: u32) -> GameBoy {
    let rom = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/roms/cpu_instrs/cpu_instrs.gb"))
        .expect("test ROM — run scripts/fetch-test-roms.sh");
    let mut gb = GameBoy::new(rom).unwrap();
    for _ in 0..frames { gb.run_frame(Buttons::default()); }
    gb
}

#[test]
fn save_load_resumes_identically() {
    let mut gb = boot(120);
    let snap = gb.save_state();
    let want: Vec<_> = {
        let mut g = boot(0);
        g.load_state(&snap).unwrap();
        for _ in 0..60 { g.run_frame(Buttons::default()); }
        g.run_frame(Buttons::default()).pixels.clone()
    };
    gb.load_state(&snap).unwrap();
    for _ in 0..60 { gb.run_frame(Buttons::default()); }
    assert_eq!(gb.run_frame(Buttons::default()).pixels, want);
}

#[test]
fn save_load_save_is_idempotent() {
    let gb = boot(120);
    let snap = gb.save_state();
    let mut g2 = boot(0);
    g2.load_state(&snap).unwrap();
    assert_eq!(g2.save_state(), snap, "a kept field was not restored by load");
}

#[test]
fn header_mismatches_are_rejected() {
    let mut gb = boot(0);
    assert!(gb.load_state(b"XXXXshort").is_err());
    let mut bad = gb.save_state();
    bad[4] = 0xFF; // corrupt version byte
    assert!(gb.load_state(&bad).is_err());
}
