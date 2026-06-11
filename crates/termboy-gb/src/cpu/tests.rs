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
