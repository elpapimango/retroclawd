# retroclawd-atari

Atari ST family emulation for [retroclawd](https://github.com/elpapimango/retroclawd): the machine logic and the
`atari_st_machine_plugin` shared library the application loads at run time. The application links none of it;
removing the library removes the machine.

State: Phase 1 done, slices 1-6 of the plans in the application repository
(`docs/superpowers/plans/2026-10-21-atari-st-slice6.md` and earlier; design in `docs/superpowers/specs/2026-10-19-atari-st-design.md`). Emulated:
- the MC68000 (from `retroclawd-crates`) and the ST bus, ROM at `$FC0000`, the memory controller (every `--ram` size);
- the Shifter (registers, a frame-based picture), the MC68901 MFP, HBL/VBL/MFP interrupts;
- the YM2149 (sound, port A drive select), the keyboard and MIDI ACIAs, the IKBD (keyboard, mouse, joysticks, clock);
- the DMA chip, the WD1772 and two floppy drives with `.st` images (reading; writing stops the machine until Phase 2).

EmuTOS 1.4 boots to its desktop, boots an executable floppy, and takes keys, the mouse and joysticks. Its I/O
accesses are compared with Hatari 2.6.1's in the tests (`crates/machines/atari-st/tests/common/mod.rs` documents the
rules). Every hardware fact used is listed with its source in `docs/hardware-facts.md`.

Scenarios: `atari-st.manifest.toml` (run by `rc-scenarios` of retroclawd-tools, see the application's CLAUDE.md) checks
the desktop against Hatari 2.6.1, keys, joystick and mouse through `--at`, a floppy made by `rcdisk` listed on the
desktop, a YM2149 tone and the frame length. Its media come from `scenarios/build.sh` (`scenarios/README.md`), built
into the git-ignored `software/scenarios/`.

Options: `--ram`, `--monitor` (colour; mono is refused until its frame is sourced), `--drives 1|2`; `--disk x.st`
(drive A), `--disk-b`; key names are the host's (`KeyA`, `Escape`, `Help`, `Undo`, ...), joysticks `Joy1*` (the
joystick port) and `Joy2*` (the mouse port).

## Build

```
cargo build --release --workspace
```

Put `target/release/atari_st_machine_plugin.{dll,so,dylib}` in a `plugins/` folder next to `retroclawd`, or point
`RETROCLAWD_MACHINE_PLUGIN_DIRS` at it. Firmware: `retroclawd atari-st --firmware auto` fetches EmuTOS (see
`firmware.toml`); your own TOS dump goes in with `--rom`.

## Licence

GPL-3.0-or-later (see `LICENSE`). EmuTOS is GPL-2.0 and is never committed here.
