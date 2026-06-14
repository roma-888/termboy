# termboy

A Game Boy, Game Boy Color, and Game Boy Advance emulator that runs in your terminal.

![dmg-acid2 rendered by termboy](assets/dmg-acid2.png)

Written in Rust. Three emulation cores share one frontend — a half-block terminal
renderer with auto-scaling, a ROM picker grouped by hardware, configurable input,
audio, and battery saves.

## Status

**Game Boy / Game Boy Color — complete.** Full audio (all four APU channels) plays
through your system output. Game Boy Color games run in full color (banked
VRAM/WRAM, color palettes, double-speed CPU, HDMA, MBC5) alongside the original DMG
library. MBC1/MBC3/MBC5 with battery saves (`<rom>.sav`, auto-flushed) and the MBC3
real-time clock. Blargg cpu_instrs + instr_timing, dmg-acid2 and cgb-acid2 (both
pixel-exact vs official references) all pass.

**Game Boy Advance — commercial games boot.** ARM7TDMI CPU (jsmolka's arm/thumb test
ROMs pass headlessly), the full scanline PPU (tiled and bitmap modes, affine
backgrounds, regular and affine sprites, windows, alpha/brightness blending,
mosaic), all four DMA channels, cascading timers, the IE/IF/IME interrupt system,
and an HLE BIOS (IntrWait, CpuSet, LZ77/Huffman decompression, affine helpers, …).
Pokémon boots through its intro to the title screen and in-game menus. Battery
saves persist for Flash and SRAM carts (auto-detected from the ROM, written to
`<rom>.sav`). Not yet implemented: EEPROM saves, GBA audio, and a
cycle-timing/performance pass.

- `cargo run --release -p termboy` — opens a game picker for `./roms` (GB/GBC/GBA, grouped by hardware; no argument needed)
- `cargo run --release -p termboy -- <rom>` — play a `.gb`/`.gbc`/`.gba` directly (pixel-perfect when it fits, auto-scaled to fit below that; `--exact` disables scaling)
- `--keys swap` (A/B swapped) or `--keys a=k,b=j,start=space` for custom bindings
- `--palette green|gray|pocket` or four hex colors (`--palette '#e0f8d0,#88c070,#346856,#081820'`) — Game Boy only
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
| A | L (GBA) |
| S | R (GBA) |
| Esc | Quit |

Input feels best in a terminal supporting the kitty keyboard protocol
(Ghostty, kitty, WezTerm, recent iTerm2/Alacritty) — real key-release events.
Elsewhere termboy falls back to timed release driven by OS key repeat; for a
snappier hold, reduce your OS key-repeat delay.
