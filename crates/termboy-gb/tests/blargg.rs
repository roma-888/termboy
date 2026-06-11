//! Integration harness: run a Blargg test ROM until it prints "Passed" or
//! "Failed" over the serial port (or 60 emulated seconds elapse).

use std::path::Path;

use termboy_core::{Buttons, Core};
use termboy_gb::GameBoy;

fn run_blargg(rel: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/roms").join(rel);
    let rom = std::fs::read(&path)
        .unwrap_or_else(|_| panic!("missing {rel} — run scripts/fetch-test-roms.sh"));
    let mut gb = GameBoy::new(rom).expect("test ROM should load");

    for _ in 0..60 * 60 {
        gb.run_frame(Buttons::default());
        let out = String::from_utf8_lossy(gb.serial_output()).into_owned();
        if out.contains("Passed") {
            return;
        }
        if out.contains("Failed") {
            panic!("blargg reported failure:\n{out}");
        }
    }
    panic!(
        "timed out after 60 emulated seconds; serial so far:\n{}",
        String::from_utf8_lossy(gb.serial_output())
    );
}

#[test]
fn cpu_instrs_01_special() {
    run_blargg("cpu_instrs/01-special.gb");
}
#[test]
fn cpu_instrs_02_interrupts() {
    run_blargg("cpu_instrs/02-interrupts.gb");
}
#[test]
fn cpu_instrs_03_op_sp_hl() {
    run_blargg("cpu_instrs/03-op sp,hl.gb");
}
#[test]
fn cpu_instrs_04_op_r_imm() {
    run_blargg("cpu_instrs/04-op r,imm.gb");
}
#[test]
fn cpu_instrs_05_op_rp() {
    run_blargg("cpu_instrs/05-op rp.gb");
}
#[test]
fn cpu_instrs_06_ld_r_r() {
    run_blargg("cpu_instrs/06-ld r,r.gb");
}
#[test]
fn cpu_instrs_07_jumps() {
    run_blargg("cpu_instrs/07-jr,jp,call,ret,rst.gb");
}
#[test]
fn cpu_instrs_08_misc() {
    run_blargg("cpu_instrs/08-misc instrs.gb");
}
#[test]
fn cpu_instrs_09_op_r_r() {
    run_blargg("cpu_instrs/09-op r,r.gb");
}
#[test]
fn cpu_instrs_10_bit_ops() {
    run_blargg("cpu_instrs/10-bit ops.gb");
}
#[test]
fn cpu_instrs_11_op_a_hl() {
    run_blargg("cpu_instrs/11-op a,(hl).gb");
}
#[test]
fn instr_timing() {
    run_blargg("instr_timing/instr_timing.gb");
}
