# retroclawd-atari

Atari family emulation for [retroclawd](https://github.com/elpapimango/retroclawd): the machine logic and the
`atari_st_machine_plugin` and `atari_8bit_machine_plugin` shared libraries the application loads at run time. The application links none of it;
removing the library removes the machine.

State: slice 1 of the plan in the application repository
(`docs/superpowers/plans/2026-10-19-atari-st-slice1.md`, design in
`docs/superpowers/specs/2026-10-19-atari-st-design.md`): the MC68000 (from `retroclawd-crates`), the ST bus with
the ROM at `$FC0000`, a deterministic event scheduler, and EmuTOS 1.4 running until its first access to a device
that is not emulated yet, where it stops with a diagnostic.

**Atari 800 is experimental.** `atari-800` boots explicitly identified Altirra
replacement OS/BASIC ROMs in PAL/NTSC and fast/accurate modes. Initial ANTIC,
GTIA and POKEY reuse the shared NMOS 6502 and MOS 6520 PIA. Raw Programs,
keyboard, four digital joysticks, audio, State and the debug contract are wired.
XEX/ATR/CAS/CART loading, SIO, P/M graphics, scrolling and several timing
details remain unimplemented. S2 reference keyboard capture and S7 Acid800
are known gaps. See [acceptance and limits](docs/atari800-gaps.md) and
`atari-800.manifest.toml`; this is not a game-compatibility or Acid800 pass claim.

## Build

```
cargo build --release --workspace
```

Put `target/release/atari_st_machine_plugin.{dll,so,dylib}` in a `plugins/` folder next to `retroclawd`, or point
`RETROCLAWD_MACHINE_PLUGIN_DIRS` at it. Firmware: `retroclawd atari-st --firmware auto` fetches EmuTOS (see
`firmware.toml`); your own TOS dump goes in with `--rom`.

For Atari 800, stage `atari_8bit_machine_plugin` alongside the ST Plugin:

```text
retroclawd atari-800 --rom-os OS.rom --rom-basic BASIC.rom
retroclawd atari-800 --rom-os OS.rom --rom-basic BASIC.rom --execution-mode accurate --region ntsc
```

`--rom-os` requires 10240 bytes; optional `--rom-basic` requires 8192 bytes.
`--firmware auto` knows the verified replacement hashes, but has no direct
download URL for upstream C arrays; explicit files, cache or RomM must supply them.
Original Atari ROM compatibility has not been verified.

Build scenario Media with `scenarios/build.sh <retroclawd-tools executable folder>`.
Run `rc-scenarios run atari-800.manifest.toml` with `RETROCLAWD_BIN`,
`RETROCLAWD_MACHINE_PLUGIN_DIRS`, `RC_ROMS` (parent of this Family) and
`RC_GOLDENS` set. ROMs, generated Media and goldens stay out of git.

## Licence

GPL-3.0-or-later (see `LICENSE`). EmuTOS is GPL-2.0 and is never committed here.
