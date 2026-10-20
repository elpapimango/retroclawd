# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this is

The Atari ST family of [retroclawd](https://github.com/elpapimango/retroclawd) as a machine plugin, a submodule of
the application at `machines/atari`. The design is `docs/superpowers/specs/2026-10-19-atari-st-design.md` and the
current plan `docs/superpowers/plans/2026-10-19-atari-st-slice1.md`, both in the application repository: read
them first.

Rules from the design that apply to every change:
- **No hardware fact from memory.** A fact used in code is checked against a source (datasheet, Atari's hardware
  documentation, a reliable transcription) and cited in a comment (URL and section). Where sources disagree, both
  are cited and the choice explained. Hatari and other emulators are references, never copied.
- Machine time comes only from the scheduler (`u64` CPU cycles); no wall clock, no floats in timing.
- Big-endian access is always explicit; no pointer casts, no native-layout serialization.
- `#![forbid(unsafe_code)]` in every crate; all input (ROMs, disk images, states) is untrusted.

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
```
