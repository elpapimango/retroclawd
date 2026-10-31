# CLAUDE.md

The SNK family repository of retroclawd (`machines/snk` in the host). Machine plugins are shared libraries loaded by the host
through the plugin ABI; this repository will hold the Neo Geo (MVS/AES cartridge, id `neogeo`).

## Status (2026-10-31)

P0 done (`neogeo.intake.toml`). `crates/formats/neo-set` reads a cartridge zip into its regions (tested on Cyber-Lip, Fatal Fury,
Aero Fighters 2). Nothing is emulated yet. Next slices: the 68000 memory map with BIOS and cartridge to the first fetches,
LSPC video, Z80 and YM2610, controllers, save states.

## Rules (same as the other families)

- Clean room: no code, tables or constants copied from FBNeo or any emulator; reference sources are read for understanding only.
  Every hardware fact is a row in `docs/hardware-facts.md` with its source tag.
- Never commit ROMs, BIOS sets, goldens, datasheets or oracle cores. The owner's RomM holds the BIOS (firmware id 259) and the
  game sets (platform 79); `retroclawd romm games 79` lists them.
- `#![forbid(unsafe_code)]` in chip and format crates; machine time only from the scheduler.
- Oracle: `retroclawd-tools/scripts/libretro_run.py` with `RetroArch-Win64/cores/fbneo_libretro.dll` and
  `--system-dir RetroArch-Win64/system` (the BIOS is in its `fbneo/` folder).
- Commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
