#!/usr/bin/env bash
# Downloads Blargg's test ROMs (freely redistributable) from retrio/gb-test-roms.
# Run once from the repo root; the ROMs are committed to the repo.
set -euo pipefail

base="https://github.com/retrio/gb-test-roms/raw/master"
dest="crates/termboy-gb/tests/roms"
mkdir -p "$dest/cpu_instrs" "$dest/instr_timing"

roms=(
  "01-special" "02-interrupts" "03-op sp,hl" "04-op r,imm" "05-op rp"
  "06-ld r,r" "07-jr,jp,call,ret,rst" "08-misc instrs" "09-op r,r"
  "10-bit ops" "11-op a,(hl)"
)
for r in "${roms[@]}"; do
  encoded="${r// /%20}"
  curl -sfL "$base/cpu_instrs/individual/$encoded.gb" -o "$dest/cpu_instrs/$r.gb"
  echo "fetched cpu_instrs/$r.gb"
done
curl -sfL "$base/instr_timing/instr_timing.gb" -o "$dest/instr_timing/instr_timing.gb"
echo "fetched instr_timing/instr_timing.gb"

mkdir -p "$dest/dmg-acid2"
curl -sfL "https://github.com/mattcurrie/dmg-acid2/releases/download/v1.0/dmg-acid2.gb" \
  -o "$dest/dmg-acid2/dmg-acid2.gb"
echo "fetched dmg-acid2/dmg-acid2.gb"

curl -sfL "$base/cpu_instrs/cpu_instrs.gb" -o "$dest/cpu_instrs/cpu_instrs.gb"
echo "fetched cpu_instrs/cpu_instrs.gb"

mkdir -p "$dest/cgb-acid2"
curl -sfL "https://github.com/mattcurrie/cgb-acid2/releases/download/v1.1/cgb-acid2.gbc" \
  -o "$dest/cgb-acid2/cgb-acid2.gbc"
echo "fetched cgb-acid2/cgb-acid2.gbc"

# --- GBA: jsmolka's CPU test ROMs (MIT, prebuilt in the repo) ---
gba_base="https://github.com/jsmolka/gba-tests/raw/master"
gba_dest="crates/termboy-gba/tests/roms"
mkdir -p "$gba_dest"
curl -sfL "$gba_base/arm/arm.gba" -o "$gba_dest/arm.gba"
curl -sfL "$gba_base/thumb/thumb.gba" -o "$gba_dest/thumb.gba"
curl -sfL "$gba_base/ppu/hello.gba" -o "$gba_dest/hello.gba"
curl -sfL "$gba_base/ppu/stripes.gba" -o "$gba_dest/stripes.gba"
curl -sfL "$gba_base/ppu/shades.gba" -o "$gba_dest/shades.gba"
echo "fetched gba-tests arm.gba + thumb.gba + hello.gba"

# A1 functional-accuracy ROMs (jsmolka, MIT). Save ROMs are flattened to
# save_<kind>.gba so the whole suite stays in one flat tests/roms/ dir.
curl -sfL "$gba_base/memory/memory.gba"   -o "$gba_dest/memory.gba"
curl -sfL "$gba_base/nes/nes.gba"         -o "$gba_dest/nes.gba"
curl -sfL "$gba_base/save/none.gba"       -o "$gba_dest/save_none.gba"
curl -sfL "$gba_base/save/sram.gba"       -o "$gba_dest/save_sram.gba"
curl -sfL "$gba_base/save/flash64.gba"    -o "$gba_dest/save_flash64.gba"
curl -sfL "$gba_base/save/flash128.gba"   -o "$gba_dest/save_flash128.gba"
echo "fetched gba-tests memory/nes/save"
