//! GbaCore end-to-end: a hand-rolled mode-3 ROM paints one pixel; run_frame
//! must return it converted to RGB, at a stable ~280896-cycle frame cadence.

use termboy_core::{Buttons, Core, Rgb};
use termboy_gba::{CYCLES_PER_FRAME, GbaCore};

fn pixel_rom() -> Vec<u8> {
    let prog: [u32; 8] = [
        0xE3A0_0003, // MOV r0, #3        (mode 3)
        0xE380_0B01, // ORR r0, r0, #0x400 (BG2 enable)
        0xE3A0_1301, // MOV r1, #0x04000000
        0xE581_0000, // STR r0, [r1]      (DISPCNT)
        0xE3A0_2406, // MOV r2, #0x06000000
        0xE3A0_301F, // MOV r3, #0x1F     (red)
        0xE1C2_30B0, // STRH r3, [r2]     (pixel 0,0)
        0xEAFF_FFFE, // b $
    ];
    let mut rom = vec![0u8; 0x200];
    for (i, op) in prog.iter().enumerate() {
        rom[i * 4..i * 4 + 4].copy_from_slice(&op.to_le_bytes());
    }
    rom
}

#[test]
fn run_frame_renders_and_paces() {
    let mut core = GbaCore::new(pixel_rom()).unwrap();
    let fb = core.run_frame(Buttons::default());
    assert_eq!(fb.width, 240);
    assert_eq!(fb.height, 160);
    assert_eq!(fb.pixels[0], Rgb(255, 0, 0));
    assert_eq!(fb.pixels[1], Rgb(0, 0, 0));
    let c1 = core.cycles();
    core.run_frame(Buttons::default());
    let delta = core.cycles() - c1;
    assert!(
        (delta as i64 - CYCLES_PER_FRAME as i64).unsigned_abs() < 64,
        "frame cadence drifted: {delta}"
    );
}

#[test]
fn rejects_undersized_rom() {
    assert!(GbaCore::new(vec![0u8; 16]).is_err());
}

#[test]
fn no_save_ram_until_g5() {
    let core = GbaCore::new(pixel_rom()).unwrap();
    assert!(core.save_ram().is_none());
}
