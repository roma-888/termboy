# termboy

A Game Boy emulator that runs in your terminal.

Written in Rust. Targets the original Game Boy (DMG) first, with an architecture ready for Game Boy Color support.

## Status

Milestone 4 complete: MBC1 + MBC3 cartridges with battery saves (`<rom>.sav`,
auto-flushed) and the MBC3 real-time clock. Tetris, Zelda, Mario Land and
Pokémon Red are playable with persistent saves. Blargg cpu_instrs (individual
+ combined) + instr_timing and dmg-acid2 (pixel-exact) all pass.

- `cargo run --release -p termboy -- <rom.gb>` — play a ROM in any truecolor terminal (pixel-perfect at 160x72+, auto-scaled to fit below that; `--exact` disables scaling)
- `cargo run --release -p termboy -- --headless <rom.gb>` — run headless, print serial output
- `cargo test --workspace` — full test suite including hardware test ROMs

## Controls

| Key | Button |
|-----|--------|
| Arrow keys | D-pad |
| X | A |
| Z | B |
| Enter | Start |
| Tab | Select |
| Esc | Quit |

Input feels best in a terminal supporting the kitty keyboard protocol
(Ghostty, kitty, WezTerm, recent iTerm2/Alacritty) — real key-release events.
Elsewhere termboy falls back to timed release driven by OS key repeat; for a
snappier hold, reduce your OS key-repeat delay.
