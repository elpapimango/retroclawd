# CLAUDE.md

The NEC family of retroclawd (`machines/nec`): the PC Engine / TurboGrafx-16, HuCards only (id `pcengine`). The machines are linked into the program
(`apps/retroclawd/src/families.rs`, cargo feature `nec`, part of `console`).

## Status (2026-11-22)

HuCards boot to their title screens: 82 of 84 games sampled across the owner's 417 files agree with Beetle PCE Fast, Mednafen's PC Engine core
(libretro), within a mean luminance difference of 8, most within 1 (`scenarios/sweep.py`; 68 before 2026-11-27); the 2 that do not are listed in `docs/pcengine-gaps.md`. `pcengine.manifest.toml`: 17 scenarios pass, no gap. Since 2026-11-27: the backup unit (2 KB at bank `$F7`, option `backup_ram`) and the
SuperGrafx (option `supergrafx`, checked with the `sgx` test card against Beetle PCE, the accurate core). Open: the CD-ROM unit (no CD games on the
server; System Cards are in the RetroArch system folder), see `docs/pcengine-gaps.md`.
What a reviewer should check first: `docs/review-notes.md`.

Run: `rcd pcengine --rom romm:7161` (Air Zonk) or `--rom game.pce`; no firmware needed. Options `console` (pce/tg16), `multitap`, `supergrafx` and `backup_ram` (FILE, default `saves/pcengine-backup.bin`; `none`, `off`).

## Crates

- `shared/crates/core/huc6280`: the CPU (new: the shared 6502 core is NMOS). `Bus` is physical (21 bits); the core owns the eight mapping registers.
- `crates/chips/huc6270` (video: BG, sprites, DMA, interrupts, line at a time), `huc6260` (palette, dot clock), `huc6280-psg` (6 wave channels).
- `crates/formats/pce-card`: header, bit order, bank mapping (power-of-two, 384 KB two-chip, 2.5 MB Street Fighter II' window).
- `crates/machines/pcengine`: bus, timer, interrupt registers, I/O port, line scheduler (master clocks), 512 x 243 picture, save state (`RCPC` 2), the backup unit, the SuperGrafx (`Sgx`).
- `crates/plugins/pcengine-machine-plugin`: the host glue.

## Rules (same as the other families)

- Clean room: the reference is a black box (libretro: pictures, RAM, audio, input); no code, tables or constants copied. Every hardware fact is a row of
  `docs/hardware-facts.md` with its source tag (`[H]` documentation recalled, `[O]`/`[T]` observed).
- Never commit ROMs, goldens or oracle cores. The owner's RomM holds the games (platform 130); `rcd romm games 130` lists them.
- `#![forbid(unsafe_code)]` in all the crates here.
- Commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.

## Tools

`cargo run --release -p pcengine --example probe` (env `ROM`, `FRAMES`, `OUT` PPM, `PRESS=frame:Key,...`, `SUPERGRAFX`, `RC_TRACE`, `RC_TRACE_END`, `RAMOUT`,
`BRAMOUT`, `VRAMOUT` (compare with `libretro_run.py --vram-at`), `AUDIOOUT` WAV, `VRAMAT`); `scenarios/sweep.py` (games in probe and reference, `--side-by-side DIR`), `audio_compare.py`, `build.py` (the test cards: HuC6280 sources in `scenarios/cards/` built with `rcld` from retroclawd-tools, `RCLD=...`; `--compare` checks them byte for byte against the old Python assembler in the same file), `capture_pce_goldens.py`.
Debugging recipe that found most defects: run the game in both, dump work RAM at the same frame (`libretro_run.py --ram-at`, probe `RAMOUT`) and
see which zero-page variables differ.
