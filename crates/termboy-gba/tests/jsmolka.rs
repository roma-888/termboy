//! jsmolka's gba-tests, run headlessly. Verified against the ROM source:
//! every test macro records the first failing test number — arm.gba keeps
//! it in r12, thumb.gba in r7 (0 = all passed) — then the ROM draws the
//! verdict and parks in a one-instruction `b idle` loop. We run until the
//! same address executes twice in a row, then read the register.
//!
//! We drive `step_system` (not bare `step`) so the harness matches the real
//! GbaCore: DMA, timers, and interrupts all run. nes.gba's DMA tests need it.
//! A DMA tick doesn't advance the PC, so the idle check skips those ticks
//! (otherwise a pending DMA would look like the `b idle` park).

use termboy_gba::bus::Bus;
use termboy_gba::cpu::Cpu;

fn run_to_idle(name: &str) -> Cpu {
    let path = format!("{}/tests/roms/{name}", env!("CARGO_MANIFEST_DIR"));
    let rom = std::fs::read(&path)
        .unwrap_or_else(|_| panic!("{path} missing — run scripts/fetch-test-roms.sh"));
    let mut cpu = Cpu::new(Bus::new(rom));
    for _ in 0..50_000_000u64 {
        let before = cpu.exec_addr();
        let ran_dma = cpu.bus.dma_ready().is_some();
        cpu.step_system();
        if !ran_dma && cpu.exec_addr() == before {
            return cpu;
        }
    }
    panic!("{name}: no idle loop after 50M steps (PC near {:#010X})", cpu.exec_addr());
}

#[test]
fn jsmolka_arm() {
    let cpu = run_to_idle("arm.gba");
    assert_eq!(cpu.regs.get(12), 0, "arm.gba failed at test {}", cpu.regs.get(12));
}

#[test]
fn jsmolka_thumb() {
    let cpu = run_to_idle("thumb.gba");
    assert_eq!(cpu.regs.get(7), 0, "thumb.gba failed at test {}", cpu.regs.get(7));
}

#[test]
fn jsmolka_memory() {
    let cpu = run_to_idle("memory.gba");
    assert_eq!(cpu.regs.get(12), 0, "memory.gba failed at test {}", cpu.regs.get(12));
}

#[test]
fn jsmolka_nes() {
    let cpu = run_to_idle("nes.gba");
    assert_eq!(cpu.regs.get(12), 0, "nes.gba failed at test {}", cpu.regs.get(12));
}

#[test]
fn jsmolka_save_none() {
    let cpu = run_to_idle("save_none.gba");
    assert_eq!(cpu.regs.get(12), 0, "save/none.gba failed at test {}", cpu.regs.get(12));
}

#[test]
fn jsmolka_save_sram() {
    let cpu = run_to_idle("save_sram.gba");
    assert_eq!(cpu.regs.get(12), 0, "save/sram.gba failed at test {}", cpu.regs.get(12));
}

#[test]
fn jsmolka_save_flash64() {
    let cpu = run_to_idle("save_flash64.gba");
    assert_eq!(cpu.regs.get(12), 0, "save/flash64.gba failed at test {}", cpu.regs.get(12));
}

#[test]
fn jsmolka_save_flash128() {
    let cpu = run_to_idle("save_flash128.gba");
    assert_eq!(cpu.regs.get(12), 0, "save/flash128.gba failed at test {}", cpu.regs.get(12));
}
