# retroclawd-atari

Atari family emulation for [retroclawd](https://github.com/elpapimango/retroclawd): the machine logic and the
`atari_st_machine_plugin` and `atari_8bit_machine_plugin` shared libraries the application loads at run time. The application links none of it;
removing the library removes the machine.

State: slice 1 of the plan in the application repository
(`docs/superpowers/plans/2026-10-19-atari-st-slice1.md`, design in
`docs/superpowers/specs/2026-10-19-atari-st-design.md`): the MC68000 (from `retroclawd-crates`), the ST bus with
the ROM at `$FC0000`, a deterministic event scheduler, and EmuTOS 1.4 running until its first access to a device
that is not emulated yet, where it stops with a diagnostic.

**Atari 8-bit support is experimental.** `atari-800`, `atari-800xl`, `atari-65xe`
and `atari-130xe` boot explicitly identified Altirra replacement OS/BASIC ROMs
in PAL/NTSC and fast/accurate modes. ANTIC,
GTIA and POKEY reuse the shared NMOS 6502 and MOS 6520 PIA. Raw/XEX Programs,
keyboard, digital joysticks (four on 800, two on XL/XE), audio, State and the debug
contract are wired. Writable ATR/XFD on D1:/D2: uses actual PIA/POKEY serial pins,
without ROM traps. CAS playback uses the real motor and serial input; all four
replacement OS profiles load and execute a cassette Program through CIO.
Standard 8K/16K and XEGS 32K/64K/128K CAR cartridges boot through the OS.
XEX uses ordered INITAD/RUNAD; general OS-dependent applications are not proven.
Normal PM graphics, scrolling, interrupt/DMA phases and POKEY pipelines are
implemented, including GTIA 9/10/11 and pseudo-E, HSCROL feedback and phantom DMA.
State v5 retains profile, MMU, Media and control history and reads versions 1-4
on the original 800; preserve original files for rollback to older Plugins.
Accurate Acid800 completes at 56/1/1 on 800 (only the XL-only banking check fails)
and 57/0/1 on XL/XE. Fast mode scores 43/14/1 and 44/13/1 respectively.
PAL/NTSC agree. These totals are diagnostic observations, not full hardware fidelity.
Each Machine has its own acceptance manifest. Reference keyboard and cassette
capture, reference audio RMS and the reference emulator's failing Acid800 output
remain explicit gaps. Abnormal character/overlapping DMA and analogue output
are not fully modeled.
See [acceptance and limits](docs/atari800-gaps.md) and
`atari-*.manifest.toml`; this is not a general game-compatibility claim.

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
retroclawd atari-800 --rom-os OS.rom --program standalone.xex
retroclawd atari-800 --rom-os OS.rom --disk boot.atr
retroclawd atari-130xe --rom-os XL.rom --rom-basic BASIC.rom --execution-mode accurate
retroclawd atari-800xl --rom-os XL.rom --disk boot.xfd --disk-b data.atr
retroclawd atari-65xe --rom-os XL.rom --cartridge game.car
retroclawd atari-800xl --rom-os XL.rom --rom-basic BASIC.rom --tape program.cas
```

`--rom-os` requires 10240 bytes on 800 or 16384 bytes on XL/XE;
optional `--rom-basic` requires 8192 bytes.
`--firmware auto` knows the verified replacement hashes, but has no direct
download URL for upstream C arrays; explicit files, cache or RomM must supply them.
Original Atari ROM compatibility has not been verified.
Raw cartridge dumps accept only unambiguous 8K/16K layouts; banked dumps require
a supported CAR header. Protected disks, other cartridge mappers, cassette
recording/turbo and `--tape-speed` other than 1 are refused explicitly.
Disk exports preserve ATR/XFD format; the Host's normal writeback/save policies
protect modified Media. A sector peripheral is not an emulated 810 CPU/FDC.

Build scenario Media with `scenarios/build.sh <retroclawd-tools executable folder>`.
Run `rc-scenarios run atari-800.manifest.toml` with `RETROCLAWD_BIN`,
`RETROCLAWD_MACHINE_PLUGIN_DIRS`, `RC_ROMS` (parent of this Family) and
`RC_GOLDENS` set. ROMs, generated Media and goldens stay out of git.
All four manifests report **44 passed, 0 failed, 10 known gaps, 0 newly
closed gaps** each. The gaps cover reference input/CAS/audio capture and the
reference's failing diagnostic screen; see `docs/atari800-gaps.md`.

## Licence

GPL-3.0-or-later (see `LICENSE`). EmuTOS is GPL-2.0 and is never committed here.
