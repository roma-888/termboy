#!/usr/bin/env bash
# Builds the mGBA test suite (mgba-emu/suite, MIT) for LOCAL diagnostics only.
#
# The suite is distributed as SOURCE (no prebuilt release), so this clones it
# and builds suite.gba with devkitARM. It is large and its timing categories
# fail by design under termboy's coarse cycle model (those belong to the A2
# cycle-timing milestone), so it is NOT committed or CI-gated — output lands in
# a gitignored dir.
#
# Prerequisite: devkitARM (https://devkitpro.org). Without it, this prints the
# manual steps and exits 0 (the functional CI suite — jsmolka — does not depend
# on this).
set -euo pipefail

dest="crates/termboy-gba/tests/roms-local"
mkdir -p "$dest"

if ! command -v arm-none-eabi-gcc >/dev/null 2>&1; then
  cat <<'MSG'
mgba-suite: devkitARM not found — cannot build the suite here.
To run the mGBA diagnostic:
  1. Install devkitARM:  https://devkitpro.org/wiki/Getting_Started
  2. git clone https://github.com/mgba-emu/suite /tmp/mgba-suite
  3. cd /tmp/mgba-suite && make
  4. cp suite.gba crates/termboy-gba/tests/roms-local/
Then drive it against GbaCore (run_frame + screenshot the results screen).
MSG
  exit 0
fi

work="$(mktemp -d)"
git clone --depth 1 https://github.com/mgba-emu/suite "$work/suite"
make -C "$work/suite"
cp "$work/suite/suite.gba" "$dest/mgba-suite.gba"
rm -rf "$work"
echo "built mgba-suite.gba -> $dest"
