use super::*;
use crate::bus::Bus;
use crate::cartridge::Cartridge;

/// CPU with `prog` placed at the post-boot entry point 0x0100.
pub fn cpu_with(prog: &[u8]) -> Cpu {
    let mut rom = vec![0u8; 0x8000];
    rom[0x147] = 0x00;
    rom[0x100..0x100 + prog.len()].copy_from_slice(prog);
    Cpu::new(Bus::new(Cartridge::new(rom).unwrap()))
}

#[test]
fn nop_takes_one_mcycle() {
    let mut cpu = cpu_with(&[0x00]);
    cpu.step();
    assert_eq!(cpu.regs.pc, 0x101);
    assert_eq!(cpu.bus.cycles, 4);
}

#[test]
fn ld_r_r_copies_register() {
    let mut cpu = cpu_with(&[0x78]); // LD A,B
    cpu.regs.b = 0x42;
    cpu.step();
    assert_eq!(cpu.regs.a, 0x42);
    assert_eq!(cpu.bus.cycles, 4);
}

#[test]
fn ld_r_n_loads_immediate() {
    let mut cpu = cpu_with(&[0x06, 0x42]); // LD B,0x42
    cpu.step();
    assert_eq!(cpu.regs.b, 0x42);
    assert_eq!(cpu.bus.cycles, 8);
}

#[test]
fn ld_hl_indirect_reads_and_writes_memory() {
    let mut cpu = cpu_with(&[0x36, 0xAB, 0x7E]); // LD (HL),0xAB ; LD A,(HL)
    cpu.regs.set_hl(0xC000);
    cpu.step();
    assert_eq!(cpu.bus.cycles, 12); // fetch + imm + write
    cpu.step();
    assert_eq!(cpu.regs.a, 0xAB);
    assert_eq!(cpu.bus.cycles, 12 + 8);
}

#[test]
fn ld_a_hli_reads_and_increments() {
    let mut cpu = cpu_with(&[0x2A]); // LD A,(HL+)
    cpu.regs.set_hl(0xC000);
    cpu.bus.write(0xC000, 0x77);
    cpu.step();
    assert_eq!(cpu.regs.a, 0x77);
    assert_eq!(cpu.regs.hl(), 0xC001);
    assert_eq!(cpu.bus.cycles, 8);
}

#[test]
fn ldh_n_a_writes_high_page() {
    let mut cpu = cpu_with(&[0xE0, 0x80]); // LDH (0x80),A -> 0xFF80 (HRAM)
    cpu.regs.a = 0x5A;
    cpu.step();
    assert_eq!(cpu.bus.read(0xFF80), 0x5A);
    assert_eq!(cpu.bus.cycles, 12);
}

#[test]
fn ld_a_nn_absolute() {
    let mut cpu = cpu_with(&[0xFA, 0x00, 0xC0]); // LD A,(0xC000)
    cpu.bus.write(0xC000, 0x99);
    cpu.step();
    assert_eq!(cpu.regs.a, 0x99);
    assert_eq!(cpu.bus.cycles, 16);
}

use registers::{C, H, N, Z};

#[test]
fn ld_rr_nn_and_push_pop_roundtrip() {
    // LD BC,0xBEEF ; PUSH BC ; POP DE
    let mut cpu = cpu_with(&[0x01, 0xEF, 0xBE, 0xC5, 0xD1]);
    cpu.step();
    assert_eq!(cpu.regs.bc(), 0xBEEF);
    assert_eq!(cpu.bus.cycles, 12);
    cpu.step(); // PUSH BC: 16 T
    assert_eq!(cpu.bus.cycles, 12 + 16);
    cpu.step(); // POP DE: 12 T
    assert_eq!(cpu.regs.de(), 0xBEEF);
    assert_eq!(cpu.bus.cycles, 12 + 16 + 12);
}

#[test]
fn pop_af_masks_flag_low_nibble() {
    let mut cpu = cpu_with(&[0xF1]); // POP AF
    cpu.regs.sp = 0xC000;
    cpu.bus.write(0xC000, 0xFF); // low byte -> F
    cpu.bus.write(0xC001, 0x12); // high byte -> A
    cpu.step();
    assert_eq!(cpu.regs.af(), 0x12F0);
}

#[test]
fn ld_nn_sp_stores_both_bytes() {
    let mut cpu = cpu_with(&[0x08, 0x00, 0xC0]); // LD (0xC000),SP
    cpu.regs.sp = 0xABCD;
    cpu.step();
    assert_eq!(cpu.bus.read(0xC000), 0xCD);
    assert_eq!(cpu.bus.read(0xC001), 0xAB);
    assert_eq!(cpu.bus.cycles, 20);
}

#[test]
fn ld_hl_sp_plus_e_sets_carries_from_low_byte() {
    let mut cpu = cpu_with(&[0xF8, 0x02]); // LD HL,SP+2
    cpu.regs.sp = 0xFFFE;
    cpu.step();
    assert_eq!(cpu.regs.hl(), 0x0000);
    assert!(cpu.regs.flag(H) && cpu.regs.flag(C));
    assert!(!cpu.regs.flag(Z) && !cpu.regs.flag(N));
    assert_eq!(cpu.bus.cycles, 12);
}
