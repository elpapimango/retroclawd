# retroclawd-atari

Atari ST family emulation for [retroclawd](https://github.com/elpapimango/retroclawd): the machine logic and the
`atari_st_machine_plugin` shared library the application loads at run time. The application links none of it;
removing the library removes the machine.

State: slice 2 of the plan in the application repository
(`docs/superpowers/plans/2026-10-20-atari-st-slice2.md`, design in
`docs/superpowers/specs/2026-10-19-atari-st-design.md`): the MC68000 (from `retroclawd-crates`), the ST bus with
the ROM at `$FC0000`, the memory controller (`$FF8001`, every `--ram` size), the Shifter (registers and a
frame-based picture), the MC68901 MFP (interrupts and timers) and HBL/VBL/MFP interrupts from a deterministic event
scheduler. EmuTOS 1.4 runs until its first access to the YM2149 (`$FF8800`), with every I/O access on the way equal
to Hatari 2.6.1's, and stops there with a diagnostic. Every hardware fact used is listed with its source in
`docs/hardware-facts.md`.

## Build

```
cargo build --release --workspace
```

Put `target/release/atari_st_machine_plugin.{dll,so,dylib}` in a `plugins/` folder next to `retroclawd`, or point
`RETROCLAWD_MACHINE_PLUGIN_DIRS` at it. Firmware: `retroclawd atari-st --firmware auto` fetches EmuTOS (see
`firmware.toml`); your own TOS dump goes in with `--rom`.

## Licence

GPL-3.0-or-later (see `LICENSE`). EmuTOS is GPL-2.0 and is never committed here.
