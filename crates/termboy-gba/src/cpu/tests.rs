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
#[ignore = "data processing lands in task 6"]
fn pc_reads_as_plus_8_in_arm() {
    let mut cpu = cpu_with(&[0xE1A0_000F]); // MOV r0, r15
    cpu.step();
    assert_eq!(cpu.regs.get(0), 0x0800_0008);
    assert_eq!(cpu.exec_addr(), 0x0800_0004); // advanced to the next instr
}

#[test]
#[ignore = "data processing lands in task 6"]
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
