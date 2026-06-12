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
