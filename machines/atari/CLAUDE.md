# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this is

The Atari family of [retroclawd](https://github.com/elpapimango/retroclawd), a submodule of
the application at `machines/atari`. The ST design is `docs/superpowers/specs/2026-10-19-atari-st-design.md` and the
current plan `docs/superpowers/plans/2026-10-19-atari-st-slice1.md`, both in the application repository: read
them first.

Rules from the design that apply to every change:
- **No hardware fact from memory.** A fact used in code is checked against a source (datasheet, Atari's hardware
  documentation, a reliable transcription) and cited in a comment (URL and section). Where sources disagree, both
  are cited and the choice explained. Hatari and other emulators are references, never copied.
- Machine time comes only from the scheduler (`u64` CPU cycles); no wall clock, no floats in timing.
- Big-endian access is always explicit; no pointer casts, no native-layout serialization.
- `#![forbid(unsafe_code)]` in every Core, Chip and Machine crate; all input (ROMs, disk images, states) is untrusted.
  Plugin crates use only the established guarded C ABI boundary for unsafe code.

## Commands

- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all`
- Depends on `retroclawd-crates` by git revision (`tools/bump_pin.sh` in the application moves it). For testing
  against a local checkout, a git-ignored `.cargo/config.toml` with
  `[patch."https://github.com/elpapimango/retroclawd-crates"]` path entries; never commit it.

## Layout

```
crates/machines/atari-st/            the machine: config, bus (memory map), scheduler, diagnostics, Machine impl
crates/plugins/atari-st-machine-plugin/  profile, options, commands (lib `atari_st_machine_plugin_impl`)
crates/plugin-dlls/atari-st/         the cdylib (`atari_st_machine_plugin`), only `export_plugin!`
firmware.toml                        EmuTOS by URL and hash
crates/chips/{antic,gtia,pokey}/     initial Atari 8-bit Chips, no Host or Core dependency
crates/machines/atari-8bit/          experimental Atari800, shared NMOS 6502 and pia-6520
crates/plugins/atari-8bit-machine-plugin/  Atari 800 profile, input, debug contract, guarded ABI
crates/plugin-dlls/atari-8bit/       thin export only
```

## Atari 800

Read `atari-800.intake.toml` and `docs/atari800-gaps.md` before changing the
8-bit Machine; do not infer its hardware from the ST. Sources: Altirra Hardware
Reference 2026-01-02 chapters 2/4/5/6/14 and pinned Atari800 7.2.1.
`atari-800.manifest.toml` records actual acceptance and known gaps; no XL/XE
profile or general Atari Media loader exists yet.

The OS starts at `$D800`, optional BASIC at `$A000`; RAM is 48 KiB. Accurate
mode clocks every Core bus cycle and stalls only reads; fast mode batches.
ANTIC owns VBI/DLI/reset NMI; the Host's frame interrupt request is a no-op.
Atari 800 warm reset is an RNMI, not a CPU reset. Save state validates ROM,
Region, mode and Chip state before mutation. Joystick input levels are held
separately from PIA output/readback so DDR writes cannot lose another port.

Private replacement-ROM test:
`ATARI800_OS`, `ATARI800_BASIC`, `ATARI800_OUTPUT` then
`cargo test --release -p atari-8bit --test firmware -- --ignored`.
It boots and types through POKEY in both Regions/modes, checks deterministic
future State and warm reset. It does not establish reference S2 or Acid800
acceptance. Assemble raw fixtures with existing `rcasm`; never commit ROMs,
third-party diagnostics, generated Media, goldens or reference binaries.
