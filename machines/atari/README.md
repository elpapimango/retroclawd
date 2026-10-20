# retroclawd-atari

Atari ST family emulation for [retroclawd](https://github.com/elpapimango/retroclawd): the machine logic and the
`atari_st_machine_plugin` shared library the application loads at run time. The application links none of it;
removing the library removes the machine.

State: slice 1 of the plan in the application repository
(`docs/superpowers/plans/2026-10-19-atari-st-slice1.md`, design in
`docs/superpowers/specs/2026-10-19-atari-st-design.md`): the MC68000 (from `retroclawd-crates`), the ST bus with
the ROM at `$FC0000`, a deterministic event scheduler, and EmuTOS 1.4 running until its first access to a device
that is not emulated yet, where it stops with a diagnostic.

## Build

```
cargo build --release --workspace
```

Put `target/release/atari_st_machine_plugin.{dll,so,dylib}` in a `plugins/` folder next to `retroclawd`, or point
`RETROCLAWD_MACHINE_PLUGIN_DIRS` at it. Firmware: `retroclawd atari-st --firmware auto` fetches EmuTOS (see
`firmware.toml`); your own TOS dump goes in with `--rom`.

## Licence

GPL-3.0-or-later (see `LICENSE`). EmuTOS is GPL-2.0 and is never committed here.
