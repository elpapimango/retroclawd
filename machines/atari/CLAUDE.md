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
crates/machines/atari-8bit/          experimental 800/800XL/65XE/130XE, MMU, shared NMOS 6502 and pia-6520
crates/plugins/atari-8bit-machine-plugin/  four profiles, input, Media, debug contract, guarded ABI
crates/plugin-dlls/atari-8bit/       thin export only
```

## Atari 8-bit

Read `atari-800.intake.toml` and `docs/atari800-gaps.md` before changing the
8-bit Machine; do not infer its hardware from the ST. Sources: Altirra Hardware
Reference 2026-01-02 chapters 2/4/5/6/14 and pinned Atari800 7.2.1.
Each `atari-*.manifest.toml` records both Regions and modes; reference input,
cassette, RMS and diagnostic mismatches remain evidence-backed gaps.
`atari-media` owns strict ATR/XFD, CAS, CAR/raw cartridge and XEX formats without
a Machine dependency. The Machine owns ordered INITAD/RUNAD, two writable sector
peripherals and motor-gated cassette playback on actual PIA/POKEY pins.

The 800 has 48 KiB RAM and a 10 KiB OS at `$D800`; XL/XE have 64/128 KiB RAM
and a 16 KiB OS. `mmu` resolves CPU and ANTIC separately from driven PORTB
pins; writes into mapped ROM do not reach underlying RAM. Accurate
mode clocks every Core bus cycle and stalls only reads; fast mode batches.
ANTIC owns VBI/DLI/reset NMI; the Host's frame interrupt request is a no-op.
Atari 800 warm reset is RNMI; XL/XE reset CPU, PIA and ANTIC's resettable latches
without erasing RAM, raster or GTIA/POKEY state. Save state validates ROM,
Region, mode and Chip state before mutation. Joystick input levels are held
separately from PIA output/readback so DDR writes cannot lose another port.
Version 5 adds explicit profile/MMU, D2/write transactions, cartridge banks,
tape phase, DMA/GTIA video and PIA control extensions. Preserve original
serialized Core/Chip and v1-v4 layouts. Legacy States load only on 800;
v1 requires both disks ejected. Preserve old files for rollback.

Private replacement-ROM test:
`ATARI800_OS`, `ATARIXL_OS`, `ATARI800_BASIC`, `ATARI800_DISK` (Acid800 ATR), `ATARI800_OUTPUT` then
`cargo test --release -p atari-8bit --test firmware -- --ignored`.
It boots/types/resets all profiles in both Regions/modes, checks future and
in-flight State, D2 writes through OS SIOV, five CAR mappers through OS boot,
and CAS load/execution through OS CIO. Refer to the gap document for current
diagnostic results and limitations; fixture tests are not reference goldens.
Build fixtures with existing `rcasm`, `rcld` and `rcdisk`; never commit ROMs,
third-party diagnostics, generated Media, goldens or reference binaries.
