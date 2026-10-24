# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this is

The Sega family of [retroclawd](https://github.com/elpapimango/retroclawd), a submodule of the application at
`machines/sega`: the Mega Drive / Genesis (`genesis`) as the machine plugin `sega_machine_plugin`. The design is
`docs/superpowers/specs/2026-10-24-sega-genesis-design.md` and the plan `docs/superpowers/plans/2026-10-24-sega-genesis.md`,
both in the application repository: read them first.

Rules that apply to every change:
- **No hardware fact from memory.** Every fact in code has a row in `docs/hardware-facts.md` labelled `[D]` datasheet, `[H]`
  documentation, `[O]` observed in reference emulators by a probe, `[H?]` unresolved, with its source. An I/O address
  without a fact stops the machine with a bounded report.
- Reference emulators (Genesis Plus GX, PicoDrive, BlastEm, clownmdemu; MAME) may be run and their source read for
  understanding. Copy no code, table, constant or comment; confirm each fact by a document or a probe on two oracles.
- Machine time comes only from the scheduler (`u64` master clocks); no wall clock, no floats in timing.
- `#![forbid(unsafe_code)]` in every Core, Chip and Machine crate; ROMs, states and firmware are untrusted input.
- Never commit ROMs, firmware (`bios_MD.bin`, `sk.bin`, ...), goldens, documents or oracle cores.

## Commands

- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all`
- Depends on `retroclawd-crates` by git revision (`tools/bump_pin.sh` in the application moves it). For testing against a
  local checkout, a git-ignored `.cargo/config.toml` with `[patch."https://github.com/elpapimango/retroclawd-crates"]`.

## Layout

```
crates/machines/genesis/            the machine (Machine impl, bus, scheduler)
crates/plugins/sega-machine-plugin/ profile, options, commands (lib `sega_machine_plugin_impl`)
crates/plugin-dlls/genesis/         the cdylib (`sega_machine_plugin`), only `export_plugin!`
docs/                               hardware-facts.md, sources.md
scenarios/                          probe ROM sources and the ROM builder
```
