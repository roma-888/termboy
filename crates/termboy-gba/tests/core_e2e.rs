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

#[test]
fn eeprom_save_round_trips_through_save_and_load_ram() {
    let mut rom = pixel_rom();
    rom.resize(0x400, 0);
    rom[0x200..0x208].copy_from_slice(b"EEPROM_V");

    let write_cmd = |core: &mut GbaCore, addr: usize, data: u64| {
        let mut bits = vec![1u8, 0];
        for i in (0..6).rev() {
            bits.push(((addr >> i) & 1) as u8);
        }
        for i in (0..64).rev() {
            bits.push(((data >> i) & 1) as u8);
        }
        bits.push(0);
        for (i, &b) in bits.iter().enumerate() {
            core.debug_write16(0x0200_0000 + i as u32 * 2, b as u16);
        }
        core.debug_dma3(0x0200_0000, 0x0D00_0000, 73);
    };

    let mut core = GbaCore::new(rom.clone()).unwrap();
    write_cmd(&mut core, 2, 0xCAFE_F00D_1234_5678);
    let saved = core.save_ram().expect("eeprom cart has a save");
    assert_eq!(saved.len(), 512);

    // restore into a fresh core and read addr 2 back via DMA
    let mut fresh = GbaCore::new(rom).unwrap();
    fresh.load_ram(&saved);
    let mut req = vec![1u8, 1];
    for i in (0..6).rev() {
        req.push(((2usize >> i) & 1) as u8);
    }
    req.push(0);
    for (i, &b) in req.iter().enumerate() {
        fresh.debug_write16(0x0200_0000 + i as u32 * 2, b as u16);
    }
    fresh.debug_dma3(0x0200_0000, 0x0D00_0000, 9);
    fresh.debug_dma3(0x0D00_0000, 0x0200_0100, 68);
    let mut got = 0u64;
    for i in 4..68u32 {
        got = (got << 1) | (fresh.debug_read16(0x0200_0100 + i * 2) & 1) as u64;
    }
    assert_eq!(got, 0xCAFE_F00D_1234_5678);
}
