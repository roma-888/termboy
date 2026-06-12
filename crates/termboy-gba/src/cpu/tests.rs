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
#[ignore = "thumb lands in task 12"]
fn pc_reads_as_plus_4_in_thumb() {
    let mut cpu = thumb_cpu_with(&[0x4678]); // MOV r0, pc (hi-reg MOV)
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x0800_0004);
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
fn mov_to_pc_branches() {
    let mut cpu = cpu_with(&[0xE3A0_F00C]); // MOV pc, #12
    cpu.step();
    assert_eq!(cpu.exec_addr(), 0x0000_000C); // absolute (BIOS region)
}
