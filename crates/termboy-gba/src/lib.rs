//! Game Boy Advance core. G1: ARM7TDMI CPU + coarse bus; the frontend-facing
//! GbaCore (Core impl, PPU, etc.) arrives in milestone G2.

pub mod bios;
pub mod bus;
pub mod cpu;
pub mod keypad;
pub mod ppu;
