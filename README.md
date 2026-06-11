# termboy

A Game Boy emulator that runs in your terminal.

Written in Rust. Targets the original Game Boy (DMG) first, with an architecture ready for Game Boy Color support.

## Status

Milestone 2 complete: plays nothing yet (input lands in Milestone 3), but
renders for real — Blargg cpu_instrs + instr_timing and dmg-acid2 (pixel-exact
vs the official reference) all pass.

- `cargo run --release -p termboy -- <rom.gb>` — render a ROM in any truecolor terminal (pixel-perfect at 160x72+, auto-scaled to fit below that; `--exact` disables scaling; Esc quits)
- `cargo run --release -p termboy -- --headless <rom.gb>` — run headless, print serial output
- `cargo test --workspace` — full test suite including hardware test ROMs
