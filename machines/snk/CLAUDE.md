# CLAUDE.md

The SNK family repository of retroclawd (`machines/snk` in the host). Machine plugins are shared libraries loaded by the host
through the plugin ABI; this repository will hold the Neo Geo (MVS/AES cartridge, id `neogeo`).

## Status (2026-10-31)

**The games boot and show their screens.** `neogeo-machine-plugin` (library `neogeo_machine_plugin`, machine id `neogeo`) runs the real
BIOS (`neogeo.zip`, firmware 259 on RomM) and a cartridge set: the NEO-GEO logo screen, then Cyber-Lip's title and attract mode, Fatal Fury's
title and Aero Fighters 2's staff roll, all like FinalBurn Neo's pictures apart from shrunk sprites (`docs/neogeo-gaps.md`).
Run it: `retroclawd neogeo --game cyberlip.zip --bios neogeo.zip` (both a path or `romm:<id>` / `romm:fw/<id>`).

Crates: `formats/neo-set` (a game zip into its P, S, C, M, V regions), `machines/neogeo` (`Bus`: the 68000's map and registers; `Sound`:
the Z80's map, ports, command/reply latches, YM2610 registers; `video`: the fix layer and sprites; `system::NeoGeo`: both CPUs in master
clocks, set loading, keys, picture), `plugins/neogeo-machine-plugin` and `plugin-dlls/neogeo` (the C ABI). Facts: `docs/hardware-facts.md`
N1-N35. `neogeo.manifest.toml`: S1 (Cyber-Lip title) and S6 (frame 405 504 clocks) pass, S3 (Fatal Fury, Aero Fighters 2) are known
gaps (sprite shrinking). Goldens are FinalBurn Neo's, made by `scenarios/capture_neogeo_goldens.py` (needs `roms/neogeo.zip` and the sets in
`software/sets/`, both git-ignored).

Probe: `crates/machines/neogeo/tests/bios.rs` (ignored; env `RC_NEOGEO_ZIP`, `RC_GAME`, `RC_FRAMES`, `RC_OUT` (a PPM), `RC_LIST`, `RC_AT`).
Next: sprite shrink (the L0 ROM and the horizontal pattern), the timer interrupt, the YM2610 (timers first: sound programs wait on them,
then FM, SSG and ADPCM), save states, backup RAM and memory card files, more games; see `docs/neogeo-gaps.md`.

## Rules (same as the other families)

- Clean room: no code, tables or constants copied from FBNeo or any emulator; reference sources are read for understanding only.
  Every hardware fact is a row in `docs/hardware-facts.md` with its source tag.
- Never commit ROMs, BIOS sets, goldens, datasheets or oracle cores. The owner's RomM holds the BIOS (firmware id 259) and the
  game sets (platform 79); `retroclawd romm games 79` lists them.
- `#![forbid(unsafe_code)]` in chip and format crates; machine time only from the scheduler.
- Oracle: `retroclawd-tools/scripts/libretro_run.py` with `RetroArch-Win64/cores/fbneo_libretro.dll` and
  `--system-dir RetroArch-Win64/system` (the BIOS is in its `fbneo/` folder).
- Commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
