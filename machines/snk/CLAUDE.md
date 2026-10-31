# CLAUDE.md

The SNK family repository of retroclawd (`machines/snk` in the host). Machine plugins are shared libraries loaded by the host
through the plugin ABI; this repository will hold the Neo Geo (MVS/AES cartridge, id `neogeo`).

## Status (2026-10-31)

P0 done (`neogeo.intake.toml`). `crates/formats/neo-set` reads a cartridge zip into its regions (tested on Cyber-Lip, Fatal Fury,
Aero Fighters 2). `crates/machines/neogeo` is the 68000's bus: BIOS (word-swapped dumps), program ROM with the 2 MiB half swap,
work/palette/backup RAM, the system register latch, the LSPC registers stored, the watchdog counted. An address no fact covers stops
the machine with a report; the probe `crates/machines/neogeo/tests/bios.rs` (ignored; env `RC_NEOGEO_ZIP`, `RC_BIOS_NAME`,
`RC_GAME`; also `RC_STEPS`, `RC_LIST=addr`, `RC_AT=addr`) runs the real BIOS until that happens. State of the probe: 5.7 million
instructions (about three seconds of machine time, the BIOS's calendar check passes with a one-second time pulse), then a byte
write to `$320000`: the sound command to the Z80 (REG_SOUND), which is where the Z80 and its BIOS program `sm1` come in. Every stop
so far was one fact row (`docs/hardware-facts.md` N1-N23). Next: the Z80 and its port map, the NMI and reply, the YM2610, then the
raster interrupts and the LSPC picture, controllers, save states, the plugin and its manifest.

## Rules (same as the other families)

- Clean room: no code, tables or constants copied from FBNeo or any emulator; reference sources are read for understanding only.
  Every hardware fact is a row in `docs/hardware-facts.md` with its source tag.
- Never commit ROMs, BIOS sets, goldens, datasheets or oracle cores. The owner's RomM holds the BIOS (firmware id 259) and the
  game sets (platform 79); `retroclawd romm games 79` lists them.
- `#![forbid(unsafe_code)]` in chip and format crates; machine time only from the scheduler.
- Oracle: `retroclawd-tools/scripts/libretro_run.py` with `RetroArch-Win64/cores/fbneo_libretro.dll` and
  `--system-dir RetroArch-Win64/system` (the BIOS is in its `fbneo/` folder).
- Commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
