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
fn flash_save_round_trips_through_save_and_load_ram() {
    // a ROM that declares 128KB flash so detection picks Flash(M1)
    let mut rom = pixel_rom();
    rom.resize(0x400, 0);
    rom[0x200..0x209].copy_from_slice(b"FLASH1M_V");

    let mut core = GbaCore::new(rom.clone()).unwrap();
    // JEDEC program of 0x5A at flash offset 0x40: AA/55/A0 then the byte
    core.debug_write8(0x0E00_5555, 0xAA);
    core.debug_write8(0x0E00_2AAA, 0x55);
    core.debug_write8(0x0E00_5555, 0xA0);
    core.debug_write8(0x0E00_0040, 0x5A);
    let saved = core.save_ram().expect("flash cart has a save");
    assert_eq!(saved.len(), 0x2_0000);
    assert_eq!(saved[0x40], 0x5A);

    let mut fresh = GbaCore::new(rom).unwrap();
    fresh.load_ram(&saved);
    assert_eq!(fresh.debug_read8(0x0E00_0040), 0x5A);
}

#[test]
fn cart_without_save_signature_has_no_save_ram() {
    let core = GbaCore::new(pixel_rom()).unwrap();
    assert!(core.save_ram().is_none());
}
