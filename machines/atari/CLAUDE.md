# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this is

The Atari ST family of [retroclawd](https://github.com/elpapimango/retroclawd) as a machine plugin, a submodule of
the application at `machines/atari`. The design is `docs/superpowers/specs/2026-10-19-atari-st-design.md` and the
current plan `docs/superpowers/plans/2026-10-20-atari-st-slice2.md`, both in the application repository: read
them first. Every hardware fact in code is listed in `docs/hardware-facts.md` with its source and a label
(`[D]` datasheet, `[H]` documentation, `[O]` observed in Hatari as a black box, `[H?]` unresolved); an I/O address
with no fact stops the machine (`io.rs`) instead of returning an invented value.

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
crates/chips/mc68901/                MC68901 MFP: GPIP, interrupt controller, timers (USART registers only)
crates/chips/st-shifter/             Shifter registers and the frame render
crates/machines/atari-st/            the machine: config, bus, I/O decode (io.rs), MMU, MFP clock, scheduler,
                                     I/O log (Hatari format), diagnostics, Machine impl
crates/plugins/atari-st-machine-plugin/  profile, options, commands (lib `atari_st_machine_plugin_impl`)
crates/plugin-dlls/atari-st/         the cdylib (`atari_st_machine_plugin`), only `export_plugin!`
firmware.toml                        EmuTOS by URL and hash
```

## Tests that need files

The EmuTOS tests (`crates/machines/atari-st/tests/emutos.rs`) need `RC_EMUTOS` (a path to `etos192uk.img`, or the
`--firmware auto` cache) and, for the Hatari comparisons, `RC_GOLDENS` with `atari-st/emutos14-uk-trace.txt` and
`atari-st/emutos14-uk-io-{256k,512k,1m,2m,2.5m,4m}.txt` from `retroclawd-tools/scripts/capture_hatari_trace.py`
(`--kind io --memsize 256|0|1|2|2560|4 --vbls 50`). Without them they print where they looked and pass.
