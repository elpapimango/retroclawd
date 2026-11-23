# CLAUDE.md

The NEC family of retroclawd (`machines/nec`): the PC Engine / TurboGrafx-16, HuCards only (id `pcengine`). The machines are linked into the program
(`apps/retroclawd/src/families.rs`, cargo feature `nec`, part of `console`).

## Status (2026-11-22)

HuCards boot to their title screens: 68 of 84 games sampled across the owner's 417 files agree with Beetle PCE Fast, Mednafen's PC Engine core
(libretro), within a mean luminance difference of 8, most within 1 (`scenarios/sweep.py`); the 16 that do not are listed in `docs/pcengine-gaps.md`. `pcengine.manifest.toml`: 10 scenarios pass, one known gap (Lady
Sword wants backup memory). Open: the CD-ROM unit and System Card (none on the server), SuperGrafx, backup memory, see `docs/pcengine-gaps.md`.
What a reviewer should check first: `docs/review-notes.md`.

Run: `rcd pcengine --rom romm:7161` (Air Zonk) or `--rom game.pce`; no firmware needed. Options `console` (pce/tg16) and `multitap`.

## Crates

- `shared/crates/core/huc6280`: the CPU (new: the shared 6502 core is NMOS). `Bus` is physical (21 bits); the core owns the eight mapping registers.
- `crates/chips/huc6270` (video: BG, sprites, DMA, interrupts, line at a time), `huc6260` (palette, dot clock), `huc6280-psg` (6 wave channels).
- `crates/formats/pce-card`: header, bit order, bank mapping (power-of-two, 384 KB two-chip, 2.5 MB Street Fighter II' window).
- `crates/machines/pcengine`: bus, timer, interrupt registers, I/O port, line scheduler (master clocks), 512 x 243 picture, save state (`RCPC` v1).
- `crates/plugins/pcengine-machine-plugin`: the host glue.

## Rules (same as the other families)

- Clean room: the reference is a black box (libretro: pictures, RAM, audio, input); no code, tables or constants copied. Every hardware fact is a row of
  `docs/hardware-facts.md` with its source tag (`[H]` documentation recalled, `[O]`/`[T]` observed).
- Never commit ROMs, goldens or oracle cores. The owner's RomM holds the games (platform 130); `rcd romm games 130` lists them.
- `#![forbid(unsafe_code)]` in all the crates here.
- Commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.

## Tools

`cargo run --release -p pcengine --example probe` (env `ROM`, `FRAMES`, `OUT` PPM, `PRESS=frame:Key,...`, `RC_TRACE`, `RC_TRACE_END`, `RAMOUT`,
`AUDIOOUT` WAV, `VRAMAT`); `scenarios/sweep.py` (games in probe and reference, `--side-by-side DIR`), `audio_compare.py`, `build.py` (test cards), `capture_pce_goldens.py`.
Debugging recipe that found most defects: run the game in both, dump work RAM at the same frame (`libretro_run.py --ram-at`, probe `RAMOUT`) and
see which zero-page variables differ.
