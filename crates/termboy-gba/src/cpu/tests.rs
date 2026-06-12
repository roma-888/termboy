use super::Cpu;
use super::psr::{C, I, Mode, N, T, V, Z};
use crate::bus::Bus;

/// Cpu with ARM opcodes placed at the cartridge entry point 0x08000000.
pub fn cpu_with(prog: &[u32]) -> Cpu {
    let mut rom = vec![0u8; 0x4000];
    for (i, op) in prog.iter().enumerate() {
        rom[i * 4..i * 4 + 4].copy_from_slice(&op.to_le_bytes());
    }
    Cpu::new(Bus::new(rom))
}

/// Cpu started in Thumb state with Thumb opcodes at 0x08000000.
pub fn thumb_cpu_with(prog: &[u16]) -> Cpu {
    let mut rom = vec![0u8; 0x4000];
    for (i, op) in prog.iter().enumerate() {
        rom[i * 2..i * 2 + 2].copy_from_slice(&op.to_le_bytes());
    }
    let mut cpu = Cpu::new(Bus::new(rom));
    cpu.regs.cpsr.set_flag(T, true);
    cpu.regs.set(15, 0x0800_0000);
    cpu.flush();
    cpu
}

#[test]
fn boots_at_cartridge_entry_in_system_mode() {
    let cpu = cpu_with(&[0xE1A0_0000]); // MOV r0, r0 (canonical NOP)
    assert_eq!(cpu.exec_addr(), 0x0800_0000);
    assert_eq!(cpu.regs.cpsr.mode(), Mode::System);
}

#[test]
fn pc_reads_as_plus_8_in_arm() {
    let mut cpu = cpu_with(&[0xE1A0_000F]); // MOV r0, r15
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x0800_0008);
    assert_eq!(cpu.exec_addr(), 0x0800_0004); // advanced to the next instr
}

#[test]
fn condition_codes_gate_execution() {
    // MOVEQ r0,#1 ; MOVNE r1,#1  (Z is clear after reset)
    let mut cpu = cpu_with(&[0x03A0_0001, 0x13A0_1001]);
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0); // EQ failed
    assert_eq!(cpu.regs.get(1), 1); // NE passed
}

#[test]
fn all_sixteen_conditions() {
    let mut cpu = cpu_with(&[0xE1A0_0000]);
    // (cond, cpsr-flag-bits, expected)
    let cases: &[(u8, u32, bool)] = &[
        (0x0, Z, true),
        (0x0, 0, false), // EQ
        (0x1, 0, true),
        (0x1, Z, false), // NE
        (0x2, C, true),
        (0x3, C, false), // CS / CC
        (0x4, N, true),
        (0x5, N, false), // MI / PL
        (0x6, V, true),
        (0x7, V, false), // VS / VC
        (0x8, C, true),
        (0x8, C | Z, false), // HI = C && !Z
        (0x9, Z, true),
        (0x9, C, false), // LS = !C || Z
        (0xA, N | V, true),
        (0xA, N, false), // GE = N == V
        (0xB, V, true),
        (0xB, N | V, false), // LT
        (0xC, 0, true),
        (0xC, Z, false), // GT = !Z && N == V
        (0xD, Z, true),
        (0xD, 0, false), // LE
        (0xE, 0, true),              // AL
        (0xF, N | Z | C | V, false), // NV: never on ARMv4
    ];
    for &(cond, flags, expected) in cases {
        cpu.regs.cpsr.0 = (cpu.regs.cpsr.0 & 0x0FFF_FFFF) | flags;
        assert_eq!(cpu.check_cond(cond), expected, "cond {cond:#X} flags {flags:#010X}");
    }
}

#[test]
fn branch_with_offset() {
    let mut cpu = cpu_with(&[0xEA00_0001]); // B +1 -> target = here+8 + 4
    cpu.step();
    assert_eq!(cpu.exec_addr(), 0x0800_000C);
}

#[test]
fn branch_and_link_sets_lr() {
    let mut cpu = cpu_with(&[0xEB00_0010]); // BL
    cpu.step();
    assert_eq!(cpu.regs.get(14), 0x0800_0004);
    assert_eq!(cpu.exec_addr(), 0x0800_0048);
}

#[test]
fn backward_branch_to_self_detected_via_exec_addr() {
    let mut cpu = cpu_with(&[0xEAFF_FFFE]); // b $ (the test ROMs' idle loop)
    let before = cpu.exec_addr();
    cpu.step();
    assert_eq!(cpu.exec_addr(), before);
}

#[test]
fn bx_switches_to_thumb_and_back() {
    let mut cpu = cpu_with(&[0xE12F_FF10]); // BX r0
    cpu.regs.set(0, 0x0800_0101); // odd -> Thumb at 0x08000100
    cpu.step();
    assert!(cpu.regs.cpsr.thumb());
    assert_eq!(cpu.exec_addr(), 0x0800_0100);
}

#[test]
fn pc_reads_as_plus_4_in_thumb() {
    let mut cpu = thumb_cpu_with(&[0x4678]); // MOV r0, pc (hi-reg MOV)
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x0800_0004);
}

#[test]
fn thumb_shift_immediate_sets_flags() {
    let mut cpu = thumb_cpu_with(&[0x0108]); // LSL r0, r1, #4
    cpu.regs.set(1, 0x1800_0001);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x8000_0010);
    assert!(cpu.regs.cpsr.flag(N) && cpu.regs.cpsr.flag(C)); // bit 28 out last
}

#[test]
fn thumb_add_sub_register_and_imm3() {
    // ADD r0, r1, r2 ; SUB r0, r1, #2
    let mut cpu = thumb_cpu_with(&[0x1888, 0x1E88]);
    cpu.regs.set(1, 10);
    cpu.regs.set(2, 5);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 15);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 8);
    assert!(cpu.regs.cpsr.flag(C)); // 10-2: no borrow
}

#[test]
fn thumb_mov_cmp_add_sub_imm8() {
    // MOV r0,#42 ; CMP r0,#42 ; ADD r0,#1 ; SUB r0,#1
    let mut cpu = thumb_cpu_with(&[0x202A, 0x282A, 0x3001, 0x3801]);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 42);
    cpu.step();
    assert!(cpu.regs.cpsr.flag(Z) && cpu.regs.cpsr.flag(C));
    cpu.step();
    assert_eq!(cpu.regs.get(0), 43);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 42);
}

#[test]
fn thumb_alu_register_ops() {
    // ADC r0,r1 ; NEG r0,r1 ; MUL r0,r1 ; ROR r0,r1
    let mut cpu = thumb_cpu_with(&[0x4148, 0x4248, 0x4348, 0x41C8]);
    cpu.regs.cpsr.set_flag(C, true);
    cpu.regs.set(0, 1);
    cpu.regs.set(1, 2);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 4); // 1 + 2 + C
    cpu.step();
    assert_eq!(cpu.regs.get(0) as i32, -2); // NEG = 0 - r1
    cpu.step();
    assert_eq!(cpu.regs.get(0) as i32, -4); // -2 * 2
    cpu.regs.set(0, 1);
    cpu.regs.set(1, 1);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x8000_0000); // ROR by 1
    assert!(cpu.regs.cpsr.flag(C) && cpu.regs.cpsr.flag(N));
}

#[test]
fn thumb_hi_register_ops_dont_set_flags() {
    // ADD r0, r8 ; MOV r8, r0
    let mut cpu = thumb_cpu_with(&[0x4440, 0x4680]);
    cpu.regs.cpsr.set_flag(Z, true);
    cpu.regs.set(0, 0xFFFF_FFFF);
    cpu.regs.set(8, 1);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0);
    assert!(cpu.regs.cpsr.flag(Z)); // unchanged, not recomputed
    cpu.step();
    assert_eq!(cpu.regs.get(8), 0);
}

#[test]
fn thumb_bx_back_to_arm() {
    let mut cpu = thumb_cpu_with(&[0x4708]); // BX r1
    cpu.regs.set(1, 0x0800_0040); // even -> ARM
    cpu.step();
    assert!(!cpu.regs.cpsr.thumb());
    assert_eq!(cpu.exec_addr(), 0x0800_0040);
}

#[test]
fn thumb_load_store_register_offset() {
    // STR r0, [r1, r2] ; LDR r3, [r1, r2] ; STRB r0, [r1, r2] ; LDRB r3, [r1, r2]
    let mut cpu = thumb_cpu_with(&[0x5088, 0x588B, 0x5488, 0x5C8B]);
    cpu.regs.set(0, 0xAABB_CCDD);
    cpu.regs.set(1, 0x0200_0000);
    cpu.regs.set(2, 8);
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.get(3), 0xAABB_CCDD);
    cpu.regs.set(2, 16);
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.get(3), 0xDD); // byte store/load
}

#[test]
fn thumb_load_store_signed_halfword() {
    // STRH r0, [r1, r2] ; LDRH r3, [r1, r2] ; LDRSB r4, [r1, r2] ; LDRSH r5, [r1, r2]
    let mut cpu = thumb_cpu_with(&[0x5288, 0x5A8B, 0x568C, 0x5E8D]);
    cpu.regs.set(0, 0x9_8765);
    cpu.regs.set(1, 0x0200_0000);
    cpu.regs.set(2, 4);
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.get(3), 0x8765);
    cpu.step();
    assert_eq!(cpu.regs.get(4), 0x0000_0065);
    cpu.step();
    assert_eq!(cpu.regs.get(5), 0xFFFF_8765); // sign-extended
}

#[test]
fn thumb_load_store_immediate_offsets() {
    // STR r0, [r1, #4] ; LDR r2, [r1, #4] ; STRB r0, [r1, #1] ; LDRB r2, [r1, #1]
    let mut cpu = thumb_cpu_with(&[0x6048, 0x684A, 0x7048, 0x784A]);
    cpu.regs.set(0, 0x1234_5678);
    cpu.regs.set(1, 0x0200_0000);
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.get(2), 0x1234_5678);
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.get(2), 0x78);
}

#[test]
fn thumb_halfword_immediate_and_sp_relative() {
    // STRH r0, [r1, #2] ; LDRH r2, [r1, #2] ; STR r0, [sp, #4] ; LDR r3, [sp, #4]
    let mut cpu = thumb_cpu_with(&[0x8048, 0x884A, 0x9001, 0x9B01]);
    cpu.regs.set(0, 0xF00D_BEEF);
    cpu.regs.set(1, 0x0200_0000);
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.get(2), 0xBEEF);
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.get(3), 0xF00D_BEEF);
}

#[test]
fn thumb_load_address_pc_and_sp() {
    // ADD r0, pc, #4 ; ADD r1, sp, #4
    let mut cpu = thumb_cpu_with(&[0xA001, 0xA901]);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x0800_0008); // (pc & !2) + 4
    let sp = cpu.regs.get(13);
    cpu.step();
    assert_eq!(cpu.regs.get(1), sp + 4);
}

#[test]
fn thumb_adjust_sp() {
    // SUB sp, #16 ; ADD sp, #16
    let mut cpu = thumb_cpu_with(&[0xB084, 0xB004]);
    let sp = cpu.regs.get(13);
    cpu.step();
    assert_eq!(cpu.regs.get(13), sp - 16);
    cpu.step();
    assert_eq!(cpu.regs.get(13), sp);
}

#[test]
fn thumb_push_pop_with_lr_pc() {
    // PUSH {r0, lr} ; POP {r1, pc}
    let mut cpu = thumb_cpu_with(&[0xB501, 0xBD02]);
    let sp0 = cpu.regs.get(13);
    cpu.regs.set(0, 0x77);
    cpu.regs.set(14, 0x0800_0021); // odd: POP pc ignores bit 0 on ARMv4
    cpu.step();
    assert_eq!(cpu.regs.get(13), sp0 - 8);
    cpu.step();
    assert_eq!(cpu.regs.get(13), sp0);
    assert_eq!(cpu.regs.get(1), 0x77);
    assert_eq!(cpu.exec_addr(), 0x0800_0020);
    assert!(cpu.regs.cpsr.thumb()); // ARMv4: POP pc never changes state
}

#[test]
fn thumb_stmia_ldmia_with_writeback() {
    // STMIA r0!, {r1, r2} ; SUB r0, #8 ; LDMIA r0!, {r3, r4}
    let mut cpu = thumb_cpu_with(&[0xC006, 0x3808, 0xC818]);
    cpu.regs.set(0, 0x0200_0000);
    cpu.regs.set(1, 0xAA);
    cpu.regs.set(2, 0xBB);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x0200_0008);
    cpu.step();
    cpu.step();
    assert_eq!((cpu.regs.get(3), cpu.regs.get(4)), (0xAA, 0xBB));
    assert_eq!(cpu.regs.get(0), 0x0200_0008);
}

#[test]
fn thumb_conditional_branch() {
    // CMP r0,#0 ; BEQ +1 ; MOV r1,#1 ; MOV r2,#2
    let mut cpu = thumb_cpu_with(&[0x2800, 0xD000, 0x2101, 0x2202]);
    cpu.step();
    cpu.step(); // taken: skips MOV r1,#1
    cpu.step();
    assert_eq!(cpu.regs.get(1), 0);
    assert_eq!(cpu.regs.get(2), 2);
}

#[test]
fn thumb_unconditional_branch_to_self() {
    let mut cpu = thumb_cpu_with(&[0xE7FE]); // b $
    let before = cpu.exec_addr();
    cpu.step();
    assert_eq!(cpu.exec_addr(), before);
}

#[test]
fn thumb_long_branch_with_link() {
    // BL +4: F000 F802 -> target 0x08000008, lr = return | 1
    let mut cpu = thumb_cpu_with(&[0xF000, 0xF802]);
    cpu.step();
    cpu.step();
    assert_eq!(cpu.exec_addr(), 0x0800_0008);
    assert_eq!(cpu.regs.get(14), 0x0800_0005); // after-BL address, thumb bit set
}

#[test]
fn thumb_swi_hle_div() {
    let mut cpu = thumb_cpu_with(&[0xDF06]); // SWI 6
    cpu.regs.set(0, 100);
    cpu.regs.set(1, 7);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 14);
    assert_eq!(cpu.regs.get(1), 2);
}

#[test]
fn thumb_swi_without_hle_enters_arm_supervisor() {
    let mut cpu = thumb_cpu_with(&[0xDF06]);
    cpu.hle_bios = false;
    cpu.step();
    assert_eq!(cpu.regs.cpsr.mode(), Mode::Supervisor);
    assert!(!cpu.regs.cpsr.thumb()); // exceptions always enter ARM state
    assert_eq!(cpu.regs.get(14), 0x0800_0002);
    assert_eq!(cpu.exec_addr(), 0x0000_0008);
}

#[test]
fn thumb_pc_relative_load_word_aligns() {
    // at 0x08000000: LDR r0, [pc, #4] -> (0x08000004 & !2) + 4 = 0x08000008
    let mut cpu = thumb_cpu_with(&[0x4801, 0, 0, 0, 0x5544, 0x7766]);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x7766_5544);
}

#[test]
fn adds_sets_carry_and_overflow() {
    let mut cpu = cpu_with(&[0xE091_0002]); // ADDS r0, r1, r2
    cpu.regs.set(1, 0x7FFF_FFFF);
    cpu.regs.set(2, 1);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x8000_0000);
    assert!(cpu.regs.cpsr.flag(N) && cpu.regs.cpsr.flag(V));
    assert!(!cpu.regs.cpsr.flag(C) && !cpu.regs.cpsr.flag(Z));
}

#[test]
fn subs_carry_means_no_borrow() {
    let mut cpu = cpu_with(&[0xE051_0002, 0xE051_0002]); // SUBS r0, r1, r2 (x2)
    cpu.regs.set(1, 5);
    cpu.regs.set(2, 5);
    cpu.step();
    assert!(cpu.regs.cpsr.flag(Z) && cpu.regs.cpsr.flag(C)); // 5-5: no borrow
    cpu.regs.set(1, 0);
    cpu.regs.set(2, 1);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0xFFFF_FFFF);
    assert!(!cpu.regs.cpsr.flag(C)); // borrow -> C clear
}

#[test]
fn adc_sbc_use_carry() {
    // ADCS r0, r1, r2 ; SBCS r3, r4, r5
    let mut cpu = cpu_with(&[0xE0B1_0002, 0xE0D4_3005]);
    cpu.regs.cpsr.set_flag(C, true);
    cpu.regs.set(1, 1);
    cpu.regs.set(2, 2);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 4); // 1 + 2 + C
    // after ADCS of small numbers C is clear -> SBC subtracts an extra 1
    cpu.regs.set(4, 10);
    cpu.regs.set(5, 3);
    cpu.step();
    assert_eq!(cpu.regs.get(3), 6); // 10 - 3 - 1
}

#[test]
fn logical_ops_take_carry_from_shifter() {
    let mut cpu = cpu_with(&[0xE1B0_0081]); // MOVS r0, r1, LSL #1
    cpu.regs.set(1, 0x8000_0001);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 2);
    assert!(cpu.regs.cpsr.flag(C)); // bit 31 shifted out
    assert!(!cpu.regs.cpsr.flag(Z) && !cpu.regs.cpsr.flag(N));
}

#[test]
fn lsr_zero_immediate_means_lsr_32() {
    let mut cpu = cpu_with(&[0xE1B0_0021]); // MOVS r0, r1, LSR #0
    cpu.regs.set(1, 0x8000_0000);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0);
    assert!(cpu.regs.cpsr.flag(C) && cpu.regs.cpsr.flag(Z));
}

#[test]
fn ror_zero_immediate_is_rrx() {
    let mut cpu = cpu_with(&[0xE1B0_0061]); // MOVS r0, r1, ROR #0
    cpu.regs.cpsr.set_flag(C, true);
    cpu.regs.set(1, 1);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x8000_0000); // carry rotated into bit 31
    assert!(cpu.regs.cpsr.flag(C)); // old bit 0 out
}

#[test]
fn register_shift_by_32_and_more() {
    // MOVS r0, r1, LSL r2 ; MOVS r0, r1, LSL r2
    let mut cpu = cpu_with(&[0xE1B0_0211, 0xE1B0_0211]);
    cpu.regs.set(1, 1);
    cpu.regs.set(2, 32);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0);
    assert!(cpu.regs.cpsr.flag(C)); // LSL #32: C = old bit 0
    cpu.regs.set(2, 33);
    cpu.step();
    assert!(!cpu.regs.cpsr.flag(C)); // LSL >32: result 0, C 0
}

#[test]
fn register_shift_reads_pc_plus_12() {
    let mut cpu = cpu_with(&[0xE1A0_021F]); // MOV r0, r15, LSL r2
    cpu.regs.set(2, 0);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x0800_000C); // PC+12, not PC+8
}

#[test]
fn immediate_operand_with_rotation() {
    let mut cpu = cpu_with(&[0xE3A0_04FF]); // MOV r0, #0xFF000000 (0xFF ror 8)
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0xFF00_0000);
}

#[test]
fn cmp_tst_only_set_flags() {
    // CMP r1, r2 ; TST r1, r3
    let mut cpu = cpu_with(&[0xE151_0002, 0xE111_0003]);
    cpu.regs.set(1, 7);
    cpu.regs.set(2, 7);
    cpu.regs.set(3, 8);
    cpu.step();
    assert!(cpu.regs.cpsr.flag(Z) && cpu.regs.cpsr.flag(C));
    cpu.step();
    assert!(cpu.regs.cpsr.flag(Z)); // 7 & 8 == 0
    assert_eq!(cpu.regs.get(0), 0); // nothing written
}

#[test]
fn mrs_reads_cpsr() {
    let mut cpu = cpu_with(&[0xE10F_0000]); // MRS r0, CPSR
    cpu.step();
    assert_eq!(cpu.regs.get(0) & 0x1F, 0x1F); // System mode bits
}

#[test]
fn msr_switches_mode_and_banks() {
    let mut cpu = cpu_with(&[0xE121_F000]); // MSR CPSR_c, r0
    cpu.regs.set(0, 0x12); // IRQ mode
    cpu.step();
    assert_eq!(cpu.regs.cpsr.mode(), Mode::Irq);
    assert_eq!(cpu.regs.get(13), 0x0300_7FA0); // IRQ stack appeared
}

#[test]
fn msr_flag_immediate() {
    let mut cpu = cpu_with(&[0xE328_F4F0]); // MSR CPSR_f, #0xF0000000
    cpu.step();
    assert!(cpu.regs.cpsr.flag(N) && cpu.regs.cpsr.flag(Z));
    assert!(cpu.regs.cpsr.flag(C) && cpu.regs.cpsr.flag(V));
    assert_eq!(cpu.regs.cpsr.mode(), Mode::System); // control byte untouched
}

#[test]
fn msr_cannot_set_thumb_bit() {
    let mut cpu = cpu_with(&[0xE121_F000]); // MSR CPSR_c, r0
    cpu.regs.set(0, 0x3F); // System mode + T bit
    cpu.step();
    assert!(!cpu.regs.cpsr.thumb());
    assert_eq!(cpu.regs.cpsr.mode(), Mode::System);
}

#[test]
fn msr_mrs_spsr_roundtrip() {
    // MSR CPSR_c, r0 ; MSR SPSR_fc, r1 ; MRS r2, SPSR
    let mut cpu = cpu_with(&[0xE121_F000, 0xE169_F001, 0xE14F_2000]);
    cpu.regs.set(0, 0x12); // to IRQ mode (it has an SPSR)
    cpu.regs.set(1, 0xF000_0013);
    cpu.step();
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.get(2), 0xF000_0013);
}

#[test]
fn mul_and_mla() {
    // MUL r0, r1, r2 ; MLA r3, r1, r2, r0
    let mut cpu = cpu_with(&[0xE000_0291, 0xE023_0291]);
    cpu.regs.set(1, 7);
    cpu.regs.set(2, 6);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 42);
    cpu.step();
    assert_eq!(cpu.regs.get(3), 84); // 7*6 + r0(42)
}

#[test]
fn muls_sets_n_and_z() {
    let mut cpu = cpu_with(&[0xE010_0291, 0xE010_0291]); // MULS r0, r1, r2
    cpu.regs.set(1, 0xFFFF_FFFF); // -1
    cpu.regs.set(2, 1);
    cpu.step();
    assert!(cpu.regs.cpsr.flag(N) && !cpu.regs.cpsr.flag(Z));
    cpu.regs.set(2, 0);
    cpu.step();
    assert!(cpu.regs.cpsr.flag(Z) && !cpu.regs.cpsr.flag(N));
}

#[test]
fn umull_unsigned_64() {
    let mut cpu = cpu_with(&[0xE081_0392]); // UMULL r0, r1, r2, r3
    cpu.regs.set(2, 0xFFFF_FFFF);
    cpu.regs.set(3, 2);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0xFFFF_FFFE); // low
    assert_eq!(cpu.regs.get(1), 1); // high
}

#[test]
fn smull_signed_64() {
    let mut cpu = cpu_with(&[0xE0C1_0392]); // SMULL r0, r1, r2, r3
    cpu.regs.set(2, 0xFFFF_FFFF); // -1
    cpu.regs.set(3, 2);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0xFFFF_FFFE); // -2 low
    assert_eq!(cpu.regs.get(1), 0xFFFF_FFFF); // -2 high
}

#[test]
fn umlal_accumulates() {
    let mut cpu = cpu_with(&[0xE0A1_0392]); // UMLAL r0, r1, r2, r3
    cpu.regs.set(0, 5); // existing low
    cpu.regs.set(1, 1); // existing high
    cpu.regs.set(2, 2);
    cpu.regs.set(3, 3);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 11); // (1<<32 | 5) + 6
    assert_eq!(cpu.regs.get(1), 1);
}

#[test]
fn ldr_str_basic_and_writeback() {
    // STR r0, [r1, #4]! ; LDR r2, [r1], #4
    let mut cpu = cpu_with(&[0xE5A1_0004, 0xE491_2004]);
    cpu.regs.set(0, 0xCAFE_F00D);
    cpu.regs.set(1, 0x0200_0000);
    cpu.step();
    assert_eq!(cpu.bus.read32(0x0200_0004), 0xCAFE_F00D);
    assert_eq!(cpu.regs.get(1), 0x0200_0004); // pre-index writeback
    cpu.step();
    assert_eq!(cpu.regs.get(2), 0xCAFE_F00D);
    assert_eq!(cpu.regs.get(1), 0x0200_0008); // post-index writeback
}

#[test]
fn ldr_misaligned_rotates() {
    let mut cpu = cpu_with(&[0xE591_0000]); // LDR r0, [r1]
    cpu.bus.write32(0x0200_0000, 0x1122_3344);
    cpu.regs.set(1, 0x0200_0001);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x4411_2233);
}

#[test]
fn ldrb_strb() {
    // STRB r0, [r1] ; LDRB r2, [r1]
    let mut cpu = cpu_with(&[0xE5C1_0000, 0xE5D1_2000]);
    cpu.regs.set(0, 0x1234_56AB);
    cpu.regs.set(1, 0x0200_0003);
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.get(2), 0xAB);
}

#[test]
fn ldr_with_shifted_register_offset() {
    let mut cpu = cpu_with(&[0xE791_0102]); // LDR r0, [r1, r2, LSL #2]
    cpu.bus.write32(0x0200_0010, 0x5555_AAAA);
    cpu.regs.set(1, 0x0200_0000);
    cpu.regs.set(2, 4);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x5555_AAAA);
}

#[test]
fn str_pc_stores_plus_12() {
    let mut cpu = cpu_with(&[0xE581_F000]); // STR pc, [r1]
    cpu.regs.set(1, 0x0200_0000);
    cpu.step();
    assert_eq!(cpu.bus.read32(0x0200_0000), 0x0800_000C);
}

#[test]
fn ldr_into_pc_branches() {
    let mut cpu = cpu_with(&[0xE591_F000]); // LDR pc, [r1]
    cpu.bus.write32(0x0200_0000, 0x0800_0100);
    cpu.regs.set(1, 0x0200_0000);
    cpu.step();
    assert_eq!(cpu.exec_addr(), 0x0800_0100);
}

#[test]
fn ldrh_and_misalignment() {
    // LDRH r0, [r1] ; LDRH r0, [r2]
    let mut cpu = cpu_with(&[0xE1D1_00B0, 0xE1D2_00B0]);
    cpu.bus.write32(0x0200_0000, 0x8000_FFEE);
    cpu.regs.set(1, 0x0200_0000);
    cpu.regs.set(2, 0x0200_0001); // misaligned: halfword rotated by 8
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0xFFEE);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0xEE00_00FF);
}

#[test]
fn ldrsb_and_ldrsh_sign_extend() {
    // LDRSB r0, [r1] ; LDRSH r2, [r1] ; LDRSH r3, [r4]  (r4 misaligned)
    let mut cpu = cpu_with(&[0xE1D1_00D0, 0xE1D1_20F0, 0xE1D4_30F0]);
    cpu.bus.write16(0x0200_0000, 0x80FE);
    cpu.regs.set(1, 0x0200_0000);
    cpu.regs.set(4, 0x0200_0001);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0xFFFF_FFFE); // byte 0xFE sign-extended
    cpu.step();
    assert_eq!(cpu.regs.get(2), 0xFFFF_80FE); // halfword sign-extended
    cpu.step();
    assert_eq!(cpu.regs.get(3), 0xFFFF_FF80); // misaligned LDRSH acts as LDRSB
}

#[test]
fn strh_with_immediate_offset() {
    let mut cpu = cpu_with(&[0xE1C1_00B2]); // STRH r0, [r1, #2]
    cpu.regs.set(0, 0x1234_BEEF);
    cpu.regs.set(1, 0x0200_0000);
    cpu.step();
    assert_eq!(cpu.bus.read16(0x0200_0002), 0xBEEF);
}

#[test]
fn swp_word_and_byte() {
    // SWP r0, r1, [r2] ; SWPB r3, r4, [r2]
    let mut cpu = cpu_with(&[0xE102_0091, 0xE142_3094]);
    cpu.bus.write32(0x0200_0000, 0x0BAD_F00D);
    cpu.regs.set(1, 0x1111_1111);
    cpu.regs.set(2, 0x0200_0000);
    cpu.regs.set(4, 0xEE);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x0BAD_F00D);
    assert_eq!(cpu.bus.read32(0x0200_0000), 0x1111_1111);
    cpu.step();
    assert_eq!(cpu.regs.get(3), 0x11);
    assert_eq!(cpu.bus.read32(0x0200_0000), 0x1111_11EE);
}

#[test]
fn stm_ldm_roundtrip_with_writeback() {
    // STMIA r0!, {r1-r3} ; LDMDB r0!, {r4-r6}
    let mut cpu = cpu_with(&[0xE8A0_000E, 0xE930_0070]);
    cpu.regs.set(0, 0x0200_0000);
    cpu.regs.set(1, 0x11);
    cpu.regs.set(2, 0x22);
    cpu.regs.set(3, 0x33);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x0200_000C);
    assert_eq!(cpu.bus.read32(0x0200_0000), 0x11);
    assert_eq!(cpu.bus.read32(0x0200_0008), 0x33);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x0200_0000); // DB walked back down
    assert_eq!((cpu.regs.get(4), cpu.regs.get(5), cpu.regs.get(6)), (0x11, 0x22, 0x33));
}

#[test]
fn push_pop_idiom() {
    // STMDB sp!, {r0, r1, lr} ; LDMIA sp!, {r2, r3, pc}
    let mut cpu = cpu_with(&[0xE92D_4003, 0xE8BD_800C]);
    let sp0 = cpu.regs.get(13);
    cpu.regs.set(0, 0xA);
    cpu.regs.set(1, 0xB);
    cpu.regs.set(14, 0x0800_0200);
    cpu.step();
    assert_eq!(cpu.regs.get(13), sp0 - 12);
    cpu.step();
    assert_eq!(cpu.regs.get(13), sp0);
    assert_eq!((cpu.regs.get(2), cpu.regs.get(3)), (0xA, 0xB));
    assert_eq!(cpu.exec_addr(), 0x0800_0200); // pc popped -> branch
}

#[test]
fn stm_base_first_in_list_stores_original_base() {
    let mut cpu = cpu_with(&[0xE8A0_0003]); // STMIA r0!, {r0, r1}
    cpu.regs.set(0, 0x0200_0000);
    cpu.regs.set(1, 7);
    cpu.step();
    assert_eq!(cpu.bus.read32(0x0200_0000), 0x0200_0000); // original base
    assert_eq!(cpu.regs.get(0), 0x0200_0008);
}

#[test]
fn stm_base_not_first_stores_written_back_base() {
    let mut cpu = cpu_with(&[0xE8A1_0003]); // STMIA r1!, {r0, r1}
    cpu.regs.set(0, 7);
    cpu.regs.set(1, 0x0200_0000);
    cpu.step();
    assert_eq!(cpu.bus.read32(0x0200_0004), 0x0200_0008); // new base stored
}

#[test]
fn ldm_base_in_list_loaded_value_wins() {
    let mut cpu = cpu_with(&[0xE8B0_0003]); // LDMIA r0!, {r0, r1}
    cpu.bus.write32(0x0200_0000, 0x0BAD_CAFE);
    cpu.bus.write32(0x0200_0004, 0x55);
    cpu.regs.set(0, 0x0200_0000);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x0BAD_CAFE); // not the written-back base
    assert_eq!(cpu.regs.get(1), 0x55);
}

#[test]
fn empty_rlist_transfers_r15_and_moves_base_by_0x40() {
    let mut cpu = cpu_with(&[0xE8A0_0000]); // STMIA r0!, {}
    cpu.regs.set(0, 0x0200_0000);
    cpu.step();
    assert_eq!(cpu.bus.read32(0x0200_0000), 0x0800_000C); // r15 = PC+12
    assert_eq!(cpu.regs.get(0), 0x0200_0040);
}

#[test]
fn stm_user_bank_from_exception_mode() {
    // MSR CPSR_c, r0 (-> IRQ) ; STMIA r1, {r13, r14}^
    let mut cpu = cpu_with(&[0xE121_F000, 0xE8C1_6000]);
    cpu.regs.set(0, 0x12);
    cpu.regs.set(1, 0x0200_0000);
    let user_sp = cpu.regs.get(13); // captured while still in System
    cpu.regs.set(14, 0x4444_4444);
    cpu.step(); // now in IRQ mode; r13/r14 are the IRQ bank
    cpu.step();
    assert_eq!(cpu.bus.read32(0x0200_0000), user_sp); // user bank, not IRQ's
    assert_eq!(cpu.bus.read32(0x0200_0004), 0x4444_4444);
}

#[test]
fn ldm_with_pc_and_s_bit_restores_cpsr() {
    // MSR CPSR_c, r0 (-> IRQ) ; MSR SPSR_fc, r2 ; LDMIA r1, {pc}^
    let mut cpu = cpu_with(&[0xE121_F000, 0xE169_F002, 0xE8D1_8000]);
    cpu.regs.set(0, 0x12);
    cpu.regs.set(2, 0x6000_001F); // System mode, C+Z set
    cpu.bus.write32(0x0200_0000, 0x0800_0100);
    cpu.regs.set(1, 0x0200_0000);
    cpu.step();
    cpu.step();
    cpu.step();
    assert_eq!(cpu.exec_addr(), 0x0800_0100);
    assert_eq!(cpu.regs.cpsr.mode(), Mode::System);
    assert!(cpu.regs.cpsr.flag(Z) && cpu.regs.cpsr.flag(C));
}

#[test]
fn swi_hle_div() {
    let mut cpu = cpu_with(&[0xEF06_0000]); // SWI 0x60000 -- the ROMs' Div idiom
    cpu.regs.set(0, 1234);
    cpu.regs.set(1, 100);
    cpu.step();
    assert_eq!(cpu.regs.get(0), 12); // quotient
    assert_eq!(cpu.regs.get(1), 34); // remainder
    assert_eq!(cpu.regs.get(3), 12); // |quotient|
    assert_eq!(cpu.regs.cpsr.mode(), Mode::System); // HLE: no mode switch
}

#[test]
fn swi_hle_div_negative() {
    let mut cpu = cpu_with(&[0xEF06_0000]);
    cpu.regs.set(0, (-7i32) as u32);
    cpu.regs.set(1, 2);
    cpu.step();
    assert_eq!(cpu.regs.get(0) as i32, -3);
    assert_eq!(cpu.regs.get(1) as i32, -1);
    assert_eq!(cpu.regs.get(3), 3);
}

#[test]
fn swi_without_hle_takes_the_vector() {
    let mut cpu = cpu_with(&[0xEF06_0000]);
    cpu.hle_bios = false;
    let old_cpsr = cpu.regs.cpsr;
    cpu.step();
    assert_eq!(cpu.regs.cpsr.mode(), Mode::Supervisor);
    assert_eq!(cpu.regs.get(14), 0x0800_0004); // return past the SWI
    assert_eq!(cpu.exec_addr(), 0x0000_0008); // SWI vector
    assert!(cpu.regs.cpsr.flag(I)); // IRQs masked
    assert_eq!(cpu.regs.spsr(), old_cpsr);
}

#[test]
fn undefined_instruction_takes_the_vector() {
    let mut cpu = cpu_with(&[0xE7F0_00F0]); // permanently-undefined encoding
    cpu.step();
    assert_eq!(cpu.regs.cpsr.mode(), Mode::Undefined);
    assert_eq!(cpu.regs.get(14), 0x0800_0004);
    assert_eq!(cpu.exec_addr(), 0x0000_0004);
}

#[test]
fn irq_entry_and_masking() {
    let mut cpu = cpu_with(&[0xE1A0_0000, 0xE1A0_0000]); // NOPs
    cpu.hle_bios = false; // raw hardware vector path
    cpu.step();
    cpu.irq();
    assert_eq!(cpu.regs.cpsr.mode(), Mode::Irq);
    // lr_irq = next instruction + 4 (handlers return with SUBS pc, lr, #4)
    assert_eq!(cpu.regs.get(14), 0x0800_0008);
    assert_eq!(cpu.exec_addr(), 0x0000_0018);
    let mode_before = cpu.regs.cpsr.mode();
    cpu.irq(); // I is now set -> ignored
    assert_eq!(cpu.regs.cpsr.mode(), mode_before);
    assert_eq!(cpu.exec_addr(), 0x0000_0018);
}

#[test]
#[should_panic(expected = "HLE BIOS function")]
fn unimplemented_bios_function_panics_loudly() {
    let mut cpu = cpu_with(&[0xEF0B_0000]); // SWI 0x0B (CpuSet) -- G4
    cpu.step();
}

#[test]
fn test_op_with_rd_15_restores_cpsr_from_spsr() {
    // MSR CPSR_c, r0 (-> IRQ) ; MSR SPSR_fc, r2 ; CMPP pc, r0 (0xE15FF000)
    // The undocumented "P" variants restore CPSR like exception returns do.
    let mut cpu = cpu_with(&[0xE121_F000, 0xE169_F002, 0xE15F_F000]);
    cpu.regs.set(0, 0x12);
    cpu.regs.set(2, 0x4000_001F); // System mode with Z set
    cpu.step();
    cpu.step();
    cpu.step();
    assert_eq!(cpu.regs.cpsr.mode(), Mode::System);
    assert!(cpu.regs.cpsr.flag(Z)); // flags came from SPSR, not the compare
    assert_eq!(cpu.exec_addr(), 0x0800_000C); // fell through, no branch
}

#[test]
fn mov_to_pc_branches() {
    let mut cpu = cpu_with(&[0xE3A0_F00C]); // MOV pc, #12
    cpu.step();
    assert_eq!(cpu.exec_addr(), 0x0000_000C); // absolute (BIOS region)
}

#[test]
fn hle_irq_dispatches_through_ram_vector_and_returns() {
    let mut cpu = cpu_with(&[
        0xE1A0_0000, // 0x08000000: nop
        0xE1A0_0000, // 0x08000004: resume point after the ISR
        0xE1A0_0000,
        0xE1A0_0000,
        // 0x08000010: user IRQ handler
        0xE3A0_1301, // MOV r1, #0x04000000
        0xE281_1C02, // ADD r1, r1, #0x200
        0xE3A0_2001, // MOV r2, #1
        0xE1C1_20B2, // STRH r2, [r1, #2]   ; IF = 1 (acknowledge vblank)
        0xE3A0_4063, // MOV r4, #0x63       ; marker (r4 is not stub-saved)
        0xE12F_FF1E, // BX lr               ; -> BIOS return shim at 0x138
    ]);
    cpu.bus.write32(0x0300_7FFC, 0x0800_0010);
    cpu.step(); // execute the first nop
    let resume = cpu.exec_addr(); // 0x08000004
    let sp_before = cpu.regs.get(13);
    cpu.regs.set(0, 0xAAAA); // clobber-check: stub must save/restore r0-r3,r12
    cpu.bus.raise_irq(1);
    cpu.irq();
    assert_eq!(cpu.regs.cpsr.mode(), Mode::Irq);
    assert_eq!(cpu.exec_addr(), 0x0800_0010); // straight into the handler
    assert_eq!(cpu.regs.get(0), 0x0400_0000); // the stub's MOV r0
    assert!(cpu.regs.cpsr.flag(I));
    for _ in 0..6 {
        cpu.step(); // run the handler through BX lr
    }
    assert_eq!(cpu.exec_addr(), 0x138); // parked on the return shim
    cpu.step(); // HLE ldmfd + subs pc, lr, #4
    assert_eq!(cpu.exec_addr(), resume);
    assert_eq!(cpu.regs.cpsr.mode(), Mode::System);
    assert!(!cpu.regs.cpsr.flag(I));
    assert_eq!(cpu.regs.get(0), 0xAAAA); // restored
    assert_eq!(cpu.regs.get(4), 0x63); // handler really ran
    assert_eq!(cpu.regs.get(13), sp_before);
    assert_eq!(cpu.bus.read16(0x0400_0202), 0); // acknowledged
}

#[test]
fn irq_pending_needs_ime_ie_and_if() {
    let mut cpu = cpu_with(&[0xE1A0_0000]);
    cpu.bus.write16(0x0400_0200, 1); // IE
    cpu.bus.raise_irq(1); // IF
    assert!(!cpu.bus.irq_pending()); // IME off
    cpu.bus.write16(0x0400_0208, 1);
    assert!(cpu.bus.irq_pending());
    cpu.bus.write16(0x0400_0202, 1); // acknowledge
    assert!(!cpu.bus.irq_pending());
}

#[test]
fn halt_idles_until_ie_and_if_intersect_even_with_ime_off() {
    let mut cpu = cpu_with(&[0xE1A0_0000, 0xE1A0_0000, 0xE1A0_0000]);
    cpu.bus.write16(0x0400_0200, 1); // IE: vblank
    cpu.bus.write8(0x0400_0301, 0); // HALTCNT
    assert!(cpu.bus.halted);
    let pc = cpu.exec_addr();
    for _ in 0..8 {
        cpu.step_system();
    }
    assert_eq!(cpu.exec_addr(), pc); // nothing executed
    cpu.bus.raise_irq(1); // IE & IF intersect; IME still 0
    cpu.step_system(); // wakes at the end of this tick
    assert!(!cpu.bus.halted);
    cpu.step_system(); // IME=0: no dispatch, execution just continues
    assert_ne!(cpu.exec_addr(), pc);
    assert_eq!(cpu.regs.cpsr.mode(), Mode::System);
}

#[test]
fn swi_halt_sets_the_bus_flag() {
    let mut cpu = cpu_with(&[0xEF02_0000]); // SWI 0x02 = Halt
    cpu.step();
    assert!(cpu.bus.halted);
}

#[test]
fn vblank_intr_wait_sleeps_until_the_isr_records_vblank() {
    let mut cpu = cpu_with(&[
        0xEF05_0000, // 0x08000000: SWI VBlankIntrWait
        0xE3A0_4001, // 0x08000004: MOV r4, #1 (woke marker)
        0xEAFF_FFFE, // 0x08000008: b $
        0xE1A0_0000,
        // 0x08000010: ISR — acknowledge IF and record in BIOS_IF
        0xE3A0_1301, // MOV r1, #0x04000000
        0xE281_1C02, // ADD r1, r1, #0x200
        0xE3A0_2001, // MOV r2, #1
        0xE1C1_20B2, // STRH r2, [r1, #2]   ; IF = 1
        0xE3A0_3403, // MOV r3, #0x03000000
        0xE283_3C7F, // ADD r3, r3, #0x7F00
        0xE283_30F8, // ADD r3, r3, #0xF8   ; r3 = 0x03007FF8
        0xE1D3_20B0, // LDRH r2, [r3]
        0xE382_2001, // ORR r2, r2, #1
        0xE1C3_20B0, // STRH r2, [r3]       ; BIOS_IF |= vblank
        0xE12F_FF1E, // BX lr
    ]);
    cpu.bus.write32(0x0300_7FFC, 0x0800_0010);
    cpu.bus.write16(0x0400_0200, 1); // IE: vblank (IME comes from IntrWait)
    cpu.bus.write16(0x0400_0004, 1 << 3); // DISPSTAT: vblank IRQ enable
    for _ in 0..500_000u32 {
        if cpu.regs.get(4) == 1 {
            break;
        }
        cpu.step_system();
    }
    assert_eq!(cpu.regs.get(4), 1, "VBlankIntrWait never woke");
    assert!(cpu.bus.cycles >= 160 * 1232, "woke before vblank");
    assert_eq!(cpu.regs.cpsr.mode(), Mode::System);
    assert_eq!(cpu.bus.read16(0x0300_7FF8) & 1, 0); // flag consumed
    assert!(!cpu.bus.halted);
}
