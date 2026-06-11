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

#[test]
fn add_sets_half_and_full_carry() {
    let mut cpu = cpu_with(&[0x80]); // ADD A,B
    cpu.regs.a = 0x3C;
    cpu.regs.b = 0xC4;
    cpu.step();
    assert_eq!(cpu.regs.a, 0x00);
    assert!(cpu.regs.flag(Z) && cpu.regs.flag(H) && cpu.regs.flag(C));
    assert!(!cpu.regs.flag(N));
}

#[test]
fn adc_includes_carry_in_half_carry() {
    let mut cpu = cpu_with(&[0x88]); // ADC A,B
    cpu.regs.a = 0x0F;
    cpu.regs.b = 0x00;
    cpu.regs.set_flag(C, true);
    cpu.step();
    assert_eq!(cpu.regs.a, 0x10);
    assert!(cpu.regs.flag(H) && !cpu.regs.flag(C));
}

#[test]
fn sbc_borrows_through_carry() {
    let mut cpu = cpu_with(&[0x98]); // SBC A,B
    cpu.regs.a = 0x00;
    cpu.regs.b = 0x00;
    cpu.regs.set_flag(C, true);
    cpu.step();
    assert_eq!(cpu.regs.a, 0xFF);
    assert!(cpu.regs.flag(N) && cpu.regs.flag(H) && cpu.regs.flag(C));
}

#[test]
fn cp_sets_flags_without_storing() {
    let mut cpu = cpu_with(&[0xFE, 0x42]); // CP 0x42
    cpu.regs.a = 0x42;
    cpu.step();
    assert_eq!(cpu.regs.a, 0x42);
    assert!(cpu.regs.flag(Z) && cpu.regs.flag(N));
}

#[test]
fn inc_preserves_carry_dec_sets_n() {
    let mut cpu = cpu_with(&[0x3C, 0x05]); // INC A ; DEC B
    cpu.regs.a = 0x0F;
    cpu.regs.b = 0x10;
    cpu.regs.set_flag(C, true);
    cpu.step();
    assert_eq!(cpu.regs.a, 0x10);
    assert!(cpu.regs.flag(H) && cpu.regs.flag(C) && !cpu.regs.flag(N));
    cpu.step();
    assert_eq!(cpu.regs.b, 0x0F);
    assert!(cpu.regs.flag(N) && cpu.regs.flag(H));
}

#[test]
fn daa_after_bcd_add() {
    let mut cpu = cpu_with(&[0x80, 0x27]); // ADD A,B ; DAA — 0x15 + 0x27 = BCD 42
    cpu.regs.a = 0x15;
    cpu.regs.b = 0x27;
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.a, 0x42);
}

#[test]
fn scf_ccf_cpl() {
    let mut cpu = cpu_with(&[0x37, 0x3F, 0x2F]); // SCF ; CCF ; CPL
    cpu.regs.a = 0xAA;
    cpu.step();
    assert!(cpu.regs.flag(C));
    cpu.step();
    assert!(!cpu.regs.flag(C));
    cpu.step();
    assert_eq!(cpu.regs.a, 0x55);
    assert!(cpu.regs.flag(N) && cpu.regs.flag(H));
}

#[test]
fn add_hl_rr_sets_carries_keeps_z() {
    let mut cpu = cpu_with(&[0x09]); // ADD HL,BC
    cpu.regs.set_hl(0x8FFF);
    cpu.regs.set_bc(0x7001);
    cpu.regs.set_flag(Z, true);
    cpu.step();
    assert_eq!(cpu.regs.hl(), 0x0000);
    assert!(cpu.regs.flag(H) && cpu.regs.flag(C) && cpu.regs.flag(Z));
    assert!(!cpu.regs.flag(N));
    assert_eq!(cpu.bus.cycles, 8);
}

#[test]
fn inc_rr_no_flags_dec_rr_wraps() {
    let mut cpu = cpu_with(&[0x03, 0x0B, 0x0B]); // INC BC ; DEC BC ; DEC BC
    cpu.step();
    assert_eq!(cpu.regs.bc(), 0x0014); // post-boot BC = 0x0013
    assert_eq!(cpu.regs.f & 0xF0, 0xB0); // flags untouched
    assert_eq!(cpu.bus.cycles, 8);
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.bc(), 0x0012);
}

#[test]
fn add_sp_e_negative_offset() {
    let mut cpu = cpu_with(&[0xE8, 0xFE]); // ADD SP,-2
    cpu.regs.sp = 0xFFFE;
    cpu.step();
    assert_eq!(cpu.regs.sp, 0xFFFC);
    assert_eq!(cpu.bus.cycles, 16);
}

#[test]
fn rla_rotates_through_carry_and_clears_z() {
    let mut cpu = cpu_with(&[0x17]); // RLA
    cpu.regs.a = 0x80;
    cpu.regs.set_flag(C, true);
    cpu.step();
    assert_eq!(cpu.regs.a, 0x01);
    assert!(cpu.regs.flag(C));
    assert!(!cpu.regs.flag(Z)); // Z must be 0 even when the result is 0
    assert_eq!(cpu.bus.cycles, 4);
}

#[test]
fn cb_rlc_b_sets_z_when_zero() {
    let mut cpu = cpu_with(&[0xCB, 0x00]); // RLC B
    cpu.regs.b = 0x00;
    cpu.step();
    assert!(cpu.regs.flag(Z));
    assert_eq!(cpu.bus.cycles, 8);
}

#[test]
fn cb_sra_keeps_sign_srl_does_not() {
    let mut cpu = cpu_with(&[0xCB, 0x28, 0xCB, 0x38]); // SRA B ; SRL B
    cpu.regs.b = 0x81;
    cpu.step();
    assert_eq!(cpu.regs.b, 0xC0);
    assert!(cpu.regs.flag(C));
    cpu.step();
    assert_eq!(cpu.regs.b, 0x60);
}

#[test]
fn cb_swap_nibbles() {
    let mut cpu = cpu_with(&[0xCB, 0x37]); // SWAP A
    cpu.regs.a = 0xF1;
    cpu.step();
    assert_eq!(cpu.regs.a, 0x1F);
    assert!(!cpu.regs.flag(C));
}

#[test]
fn cb_bit_res_set_on_hl() {
    // BIT 7,(HL) ; SET 7,(HL) ; RES 0,(HL)
    let mut cpu = cpu_with(&[0xCB, 0x7E, 0xCB, 0xFE, 0xCB, 0x86]);
    cpu.regs.set_hl(0xC000);
    cpu.bus.write(0xC000, 0x01);
    cpu.step();
    assert!(cpu.regs.flag(Z)); // bit 7 is 0
    assert!(cpu.regs.flag(H) && !cpu.regs.flag(N));
    assert_eq!(cpu.bus.cycles, 12); // BIT (HL) = 3 M-cycles
    cpu.step();
    assert_eq!(cpu.bus.read(0xC000), 0x81);
    assert_eq!(cpu.bus.cycles, 12 + 16); // SET (HL) = 4 M-cycles
    cpu.step();
    assert_eq!(cpu.bus.read(0xC000), 0x80);
}

#[test]
fn jr_taken_timing() {
    let mut cpu = cpu_with(&[0x20, 0x05, 0x20, 0x05]); // JR NZ,+5
    cpu.regs.set_flag(Z, false);
    cpu.step(); // taken: lands on 0x107
    assert_eq!(cpu.regs.pc, 0x107);
    assert_eq!(cpu.bus.cycles, 12);
}

#[test]
fn jr_untaken_timing() {
    let mut cpu = cpu_with(&[0x20, 0x05]); // JR NZ,+5 with Z set
    cpu.regs.set_flag(Z, true);
    cpu.step();
    assert_eq!(cpu.regs.pc, 0x102);
    assert_eq!(cpu.bus.cycles, 8);
}

#[test]
fn jr_negative_offset() {
    let mut cpu = cpu_with(&[0x18, 0xFE]); // JR -2: jump to itself
    cpu.step();
    assert_eq!(cpu.regs.pc, 0x100);
}

#[test]
fn call_and_ret_roundtrip() {
    // 0x100: CALL 0x110 ; 0x103: NOP ... 0x110: RET
    let mut prog = vec![0xCD, 0x10, 0x01, 0x00];
    prog.resize(0x11, 0x00);
    prog[0x10] = 0xC9;
    let mut cpu = cpu_with(&prog);
    let sp0 = cpu.regs.sp;
    cpu.step(); // CALL: 24 T
    assert_eq!(cpu.regs.pc, 0x110);
    assert_eq!(cpu.regs.sp, sp0 - 2);
    assert_eq!(cpu.bus.cycles, 24);
    cpu.step(); // RET: 16 T
    assert_eq!(cpu.regs.pc, 0x103);
    assert_eq!(cpu.regs.sp, sp0);
    assert_eq!(cpu.bus.cycles, 24 + 16);
}

#[test]
fn ret_cc_untaken_timing() {
    let mut cpu = cpu_with(&[0xC8]); // RET Z, untaken
    cpu.regs.set_flag(Z, false);
    cpu.step();
    assert_eq!(cpu.bus.cycles, 8); // 2 M-cycles untaken
}

#[test]
fn rst_pushes_and_vectors() {
    let mut cpu = cpu_with(&[0xEF]); // RST 0x28
    let sp0 = cpu.regs.sp;
    cpu.step();
    assert_eq!(cpu.regs.pc, 0x28);
    assert_eq!(cpu.regs.sp, sp0 - 2);
    assert_eq!(cpu.bus.cycles, 16);
}

#[test]
fn jp_hl_is_one_mcycle() {
    let mut cpu = cpu_with(&[0xE9]); // JP (HL)
    cpu.regs.set_hl(0x1234);
    cpu.step();
    assert_eq!(cpu.regs.pc, 0x1234);
    assert_eq!(cpu.bus.cycles, 4);
}
