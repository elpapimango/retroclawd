# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repo.

## Project status

The `retroclawd` application plus the integration tooling. **No machine and no shared crate lives in this
repository.** Machines are shared libraries (`*_machine_plugin.so/.dll/.dylib`) loaded by the app through the
C-ABI in `plugin-api` (policy: `shared/docs/plugin-abi.md`). Four other repositories hold everything else; all are
git submodules here, and the three machine repositories depend on the shared crates by git revision:

| Repository (submodule path) | Holds |
|---|---|
| `elpapimango/retroclawd-crates` (`shared/`) | `machine`, `plugin-api`, the `z80` and `mos6502` cores, `ay-3-8910`, `nec765`, `dsk`, `zx-tape`, the plugin ABI policy. No machine, no app code. |
| `elpapimango/retroclawd-sinclair` (`machines/sinclair`) | ZX81, ZX Spectrum 48K/128K/+2/+2A/+3, snapshot formats `.z80/.sna/.szx/.p`, Sinclair keyboard |
| `elpapimango/retroclawd-amstrad` (`machines/amstrad`) | CPC464/664/6128, CPC+ / GX4000, `.cpr` cartridges, SNA snapshots, i8255 |
| `elpapimango/retroclawd-commodore` (`machines/commodore`) | Experimental VIC-20/VIC-I; C64, C128, 1541/1570/1571/1581, VIC-II, SID, CIAs, VIA, WD1770, VDC, CBM disk formats |
| `elpapimango/retroclawd-atari` (`machines/atari`) | Atari ST/STF/STFM (`atari-st`, slice 1 of the 16-bit work: CPU, memory map, scheduler, EmuTOS to its first device) |

Stays here: the app (`apps/retroclawd`, including the debuggers `debug-ui`, `debug-zrcp`, `debug-log`: ordinary
modules behind cargo features, no longer plugins), the app-only crates (`app-config`, `game-config`, `file-loader`, `pok`), `crates/plugins/test-machine-plugin` (a
do-nothing machine used only by this repository's unit tests) and `tools/`. The app uses `shared/` by path; the
families pin a `retroclawd-crates` commit (`rev = ...`), and `tools/bump_pin.sh` moves all of them together.

The detailed per-crate notes are in each repository's `CLAUDE.md`; the pre-split status narrative is kept in
`docs/project-status-before-split.md`.

Everything at once: `tools/build_all.sh` / `tools\build_all.ps1` (checks out `shared/` and the families, builds the
app and every family, stages the libraries in `target/<profile>/plugins/`); `tools/smoke_test.sh` boots every machine
and compares the boot screens; `tools/check_all.sh` runs every check (build, unit tests of every repository, smoke test, every
scenario manifest, the rc-suite cases; `--acid` adds the CPC acid suite) and `tools/bump_pin.sh --push` runs it before pushing.
Using one machine on its own: build the family repository (`cargo build --release
--workspace`) and put the resulting libraries in a `plugins/` folder next to the `retroclawd` executable, or point
`RETROCLAWD_MACHINE_PLUGIN_DIRS` at them. `retroclawd --list-plugins` shows what was found.

**Common execution policy:** `plugin_api::ExecutionMode` declares and parses the
ordinary `execution_mode` Option (`--execution-mode fast|accurate`). A Plugin
must wire it to real fidelity differences before declaring it. C64/C128 map
fast (default) to the line engine and accurate to the PAL cycle engine;
media traps and true-drive remain independent. Both the common Option and the
legacy `vic_accuracy` Option omit a wire default so absence cannot override an
explicit selection through the other key. Contradictory selections are errors;
configuration precedence applies per key, not across aliases. Existing state
engine metadata preserves deterministic restoration. No ABI change is involved.

**Experimental VIC-20:** the Commodore submodule supplies `vic20_machine_plugin`,
with no Host machine-specific code. It uses the common execution Option:
instruction-batched or per-CPU-bus-cycle peripheral clocks, on PAL and NTSC.
Raw programs, PRG/P00 Host parsing and read-only VIC-20 TAP-v0/v1 playback work;
`--true-drive` adds the existing 1541/IEC implementation for writable D64 at
device 8, with `--rom-1541` or hash-based Firmware sourcing. Disk export,
writeback, swapping and complete Drive states work; Drive CPU steps remain
instruction-granular in both modes. Recording does not work.
Generic read-only cartridges use `--cartridge` with raw `.20/.40/.60/.a0/.b0`
or addressed `.prg/.bin/.rom`. The Family Format owns parsing; the Machine owns
read-only mapping, RAM-conflict rejection and attach/eject reset. The Host sends
explicit cartridge Media without machine-specific extension gating, preserving
PRG Program semantics. State v4 validates canonical mapped-ROM identity and reads
old v1/v2/v3 states with no cartridge. Banking/flash/RAM cartridges and VIC CRT are absent.
Hash-verified RomM firmware boots and types BASIC in both
modes and regions. `machines/commodore/vic20.manifest.toml` reports 62 passing
S1-S7 variants and no known scenario gaps. External GPL-3.0 Dead Test v1.1.01
matches all 1024 VICE memory bytes in both Regions/modes, as a cartridge and
full-expansion replacement KERNAL. `tools/fetch_diagnostics` fetches its three
hash-pinned release assets; peripheral/fidelity limits remain documented. Goldens pin
the installed VICE 3.9 executable and captures. The nonlinear mixer calibration
fixes the single-voice gain regression: RMS agrees within 1%, including PAL
volume 7. `docs/vic20-gaps.md` in that Family records commands and proof.
Uncaptured screen goldens are omitted, never invented; use the matching
`rc-scenarios` feature branch until its missing-capture support is merged.

**C16/Plus4 media prerequisite:** `c64-tape::ted` in the Commodore Family owns
the TAP-v2 parser/writer used by `rctape`; there is no duplicate tools parser.
It keeps exact half-waves in slow TED timer clocks and rejects unsupported or
malformed headers, inconsistent sizes and oversized payloads. No C16/Plus4
Machine, Host media route or hardware cassette playback is implemented yet.

## Keymaps and typed characters (2026-10-30)

`assets/keymaps/{uk,us,es,de,fr,sv,no,da}.toml` and `us-c64.toml` ship inside the program (`apps/retroclawd/src/builtin_keymaps.rs`, selected by
name with `--keymap de` or `[input] keymap`; a file of that name next to the config wins). Each selects the symbolic layout; the Atari ST family
files also carry explicit bindings (the ST keys are the US positions). The symbolic layout turns the *typed character* into the machine key
whatever Shift/AltGr or physical key produced it, so the AltGr layer and the ISO key left of Z work wherever the plugin declares the character
(`KC_NUBS` binds that key by name); `ß Æ Œ` type two letters one frame apart (`Held::follow` in `gui/keys.rs`). The ZX Spectrum plugin types
`\ | { } ~ [ ]` and `(c)` as the ROM's extended-mode sequence in machine time (`extended.rs`, other key events wait behind it in order); the CPC
types `~` as Control+2. A `.prg` goes to the plugin as a ready-made program only on a 68000 machine (`program_goes_to_plugin`): the VIC-20 also
declares `Program` but wants the host to read the load address, and used to fail with "requires exactly one unsigned org Option".

## Scenario manifests (2026-10-18)

Every machine has a `machines/<family>/<id>.manifest.toml` (zx81, zx-spectrum, zx-spectrum-128, zx-spectrum-2a3, cpc464/664/6128, c64,
c128) holding the playbook's standard scenarios as data, run by `rc-scenarios` of `retroclawd-tools` against the **release** `retroclawd`
and its plugins. References: ZEsarUX 13.0 and VICE 3.10 (the CPC Plus and GX4000 have none). Build the test media first
(`machines/<family>/scenarios/build.sh`, output in the git-ignored `software/scenarios/`), then:

```
RETROCLAWD_BIN=target/release/retroclawd RETROCLAWD_MACHINE_PLUGIN_DIRS=target/release/plugins RC_ROMS=machines   RC_GOLDENS=<folder with the goldens> rc-scenarios run machines/sinclair/zx81.manifest.toml
```

Goldens are never committed (a folder of your own, or RomM by hash); `retroclawd-tools/scripts/capture_*.py` regenerate them. Known gaps
(`status = "known-gap"`, evidence under `machines/<family>/docs/`): none open. Closed 2026-10-19: the CPC diagnostics' CRTC/FDC lines
(compared with Caprice32 instead of ZEsarUX, whose FDC detection fails there; `retroclawd-tools/scripts/capture_caprice_screen.py`), the C64's
fast loader now returns through the KERNAL's own SEARCHING/LOADING routines, and the C64 dead test's clock (the CIA TOD clock honours CRA
`TODIN`, so a PAL machine left at 60 Hz runs at 5/6 speed, as VICE and the hardware do). The S6 frame-length gaps were closed 2026-10-19: each machine reports its exact
frame through `Machine::frame_rate` / `MachineMetadata::frame_rate` (ZX81 312 x 207 T-states, its line corrected from 208 to 207; CPC
79872; C64/C128 one raster frame, 19656 PAL), and the host paces and budgets by it. Details and the compare options are in `retroclawd-tools/CLAUDE.md`.

## Atari 8-bit experimental support

`machines/atari` also builds `atari_8bit_machine_plugin`, exposing
`atari-800`/`atari-800xl`/`atari-65xe`/`atari-130xe`.
`rom_os` is 10240 bytes on 800 or 16384 on XL/XE; optional `rom_basic` is
8192 bytes. All profiles have PAL/NTSC and
fast/accurate execution. Shared NMOS 6502 and `pia-6520` are reused; ANTIC,
GTIA and POKEY live in the Atari Family. No Host machine-specific code or ABI
change is needed. Explicit Altirra replacements boot and execute BASIC;
original Atari firmware has not been verified.

Per-profile manifests cover reference boot, raw/XEX/ATR/XFD/CAR execution,
tone pitch, State and raster timing. Native OS tests also prove D2 writes
and CAS load/execution through CIO. The sector peripherals use actual
PIA/POKEY pins. State v5 preserves profile/MMU/Media/control phases and reads
v1-v4 only on 800; preserve original files for rollback.
Accurate Acid800 reaches 56/1/1 on 800 (XL-only banking fails) and 57/0/1
on XL/XE. Reference keyboard/CAS/RMS capture and diagnostic screen parity
remain explicit gaps (every pinned golden is reproducible: 2026-10-30, `retroclawd-tools/scripts/capture_atari8bit_goldens.py`, manifests 44/0/10 each); original Atari firmware and general games are not proven.
Read the Family intakes and
`machines/atari/docs/atari800-gaps.md` before extending it. C16/Plus4 stay parked.

## Atari ST (slice 1, 2026-10-20)
## Atari ST (Phase 1 done: slices 1-6, 2026-10-21)

Design `docs/superpowers/specs/2026-10-19-atari-st-design.md`, plans `docs/superpowers/plans/2026-10-19-atari-st-slice1.md`
`docs/superpowers/plans/2026-10-20-atari-st-slice2.md`, `docs/superpowers/plans/2026-10-21-atari-st-slices3-5.md` and
`docs/superpowers/plans/2026-10-21-atari-st-slice6.md`. Every hardware fact used is listed with its source in
`machines/atari/docs/hardware-facts.md` (`[D]` datasheet, `[H]` documentation, `[O]` observed in Hatari, `[H?]`
unresolved); an I/O address without a fact stops the machine instead of returning an invented value.
The 68000 is `mc68000` in `shared/` (SingleStepTests 68000/v1: 996 322 of 1 000 060 cases exact, the rest an ASR carry
disagreement with the PRM and two bad upstream cases, an explicit list in `tests/singlestep.rs` with its sources in
`tests/singlestep-expected.md`; `scripts/fetch_singlesteptests.sh` fetches the suite, never committed).
`atari-st` maps the 192 KB TOS at `$FC0000` (mirrored at `$0-$7`), RAM through the memory controller (`$FF8001`, DRAM
row/column aliasing so TOS sizes every `--ram` like Hatari), the Shifter (`st-shifter`: video base, sync mode, palette,
resolution, a frame-based 640x400 picture), the MC68901 MFP (`mc68901`: GPIP, interrupt controller, timers; USART
registers only), HBL/VBL/MFP interrupts (levels 2/4/6) and the MFP's 2.4576 MHz clock as an exact ratio of the CPU
clock. Slices 3-5 (2026-10-21) added the YM2149 (the shared `ay-3-8910`'s YM variant: sound, port A drive select), the
keyboard and MIDI ACIAs (shared `mc6850`), the IKBD (`ikbd`: keys by the host's key names, mouse, `Joy1*`/`Joy2*`,
clock), and floppies: the DMA chip, the shared `wd1770` as a WD1772, two drives (`--drives`), `.st` images (`st-floppy`;
the host routes `.st` to the disk slot); writing to a disk stops the machine until Phase 2. Every I/O device is now
emulated. EmuTOS 1.4 boots to its desktop, boots an executable floppy, and sees keys; over 1500 frames its I/O
accesses agree with Hatari 2.6.1's (`--io-trace-file`, Hatari's `--trace io_all` format; `capture_hatari_trace.py
--kind io`, `--disk-a`, `--trace-extra`) under the comparison rules in `machines/atari/crates/machines/atari-st/tests/
common/mod.rs`. A stop has a bounded report (`--diag-file`, `--trace-file`). `--monitor mono` is refused until the
mono frame's line count is sourced.
Firmware: `--firmware auto` now also downloads from a `firmware.toml` `url` (zip SHA-1 `archive_sha1`, file `entry`,
both hashes checked) and caches it under `<cache>/firmware/<sha1>.bin`; lookup order cache, RomM, URL; `when = { region =
... }` matches `--region`. Reference emulator: Hatari 2.6.1 (black box; `retroclawd-tools/scripts/capture_hatari_trace.py`).
Slice 6 (the Phase 1 exit): `machines/atari/atari-st.manifest.toml` (S1 desktop against Hatari's picture by
similarity, `retroclawd-tools/scripts/capture_hatari_screen.py`; S2/S2J/S2M/S3 keys, joystick, mouse and a floppy made
by `rcdisk` (`.st`, FAT12, `rcdisk boot`) as exact goldens made by retroclawd, since headless Hatari takes no input;
S4 YM2149 tone; S6 frame length), media from `machines/atari/scenarios/build.sh`. `--at mouse_move=DX,DY` (split into
steps of at most 127) and `--at mouse_down|mouse_up=left|right|middle` drive the mouse in batch runs (`scripted.rs`).
`rc-scenarios` reports a scenario as `SKIP` when `--firmware auto` finds no EmuTOS (no cache, RomM or network).

**Phase 2 (2026-10-22, slices 7-12; plan `docs/superpowers/plans/2026-10-22-atari-st-phase2.md`, spec Status 2026-10-22).**
Floppies can be written (WD1772 write sector / write track decode into the image; `.msa` images; `write_protect`; dirty
export and `--disk-save` through the plugin), the machine has save states (`RCST` envelope: id, schema, TOS and accuracy
checks, CRC, 64 MiB cap, bincode payload; clean disks stay the user's file), the MFP USART (timer D) with RS-232 peers,
MIDI and Centronics ports (options `serial_out/in/loopback`, `midi_*`, `centronics_out`, `overwrite_outputs`: a sink never
replaces a file unasked), the full IKBD command set, a read-only cartridge at `$FA0000` (`--cartridge x.stc`, a write is a
bus error), a raster palette by line and a log of sync/resolution writes, and ACSI hard disks (`--acsi0 ... --acsi7` raw
images, AHDI/FAT16 made by `rcdisk hd`, EmuTOS shows drive C). A `.prg` given to `--program` goes to the plugin as
`MediaKind::Program` when the machine declares it (`program_goes_to_plugin`): the plugin builds a 720K floppy holding
`AUTO\NAME.PRG` (`st_floppy::boot`) and EmuTOS runs it. `retroclawd-tools` gained the 68000 toolchain: `rcasm --cpu 68000`
(`-f gemdos` for a relocatable `.PRG`), `rcdis --cpu 68000`, `rcld` with `machine = "atari-st"`, `rc_common::prg` (the GEMDOS
executable format) and `rcdisk hd`. The manifest now has S3W, S5, S8P, S9S, S10 and S11 as well. Scenario media build: `machines/atari/scenarios/build.sh`.
**Phase 3 (2026-10-23, slices 13-17; plan `docs/superpowers/plans/2026-10-22-atari-st-phase3.md`, spec Status 2026-10-23).**
`--accuracy accurate` follows every bus cycle: the 68000 (`mc68000`, bus transactions now equal SingleStepTests', 996 322 of
1 000 060 cases, interrupt timing pinned to UM Table 8-14) takes 4-clock aligned cycles with wait states, devices are brought up
to date at the cycle of the access (`events.rs`, `waits.rs`), the Shifter keeps the clock of every palette/resolution/sync write
(`fetch.rs`: picture window and left/right/top/bottom borders; `render_exact`, `render_overscan`: the `borders = on` option's
832 x 552 canvas) and the mono monitor works (`--monitor mono`: 501 lines of 224 clocks). Every timing fact was observed with
a probe, a 68000 program in `machines/atari/timing/` that runs in Hatari 2.6.1 (a black box, memory dumped with the debugger's
`savebin` by `retroclawd-tools/scripts/capture_hatari_mem.py`) and here; `machines/atari/docs/timing-oracle.md`,
`timing/README.md`, `docs/hardware-facts.md` "Timing (Phase 3)". The probes found a real defect (the FDC was not ticked until it
set Busy, so completion came at frame boundaries and EmuTOS lost a revolution per sector: boot 137 frames slower than Hatari;
fixed in the shared `wd1770`), the floppy rotation facts (6267-byte revolution, 116-byte index pulse, 614-byte sector pitch,
exact six-revolution spin-up), the IKBD answer delays and the register access costs. Fourteen probes stay `known-gap` in the
manifest with the measured difference (interrupt placement jitter, Hatari's lost lines, its instant USART and unchanged ACSI DMA
address); the exit table is in the spec. `accurate` runs the idle desktop at 12.8x real time and `fast` at 13.8x
(`docs/benchmarks.md`); both modes boot the three RomM games to the same frames. Pins: crates `cdc4417`.
Unfinished and unsourced items are the `[H?]` rows of `machines/atari/docs/hardware-facts.md` and TODO.md "Atari ST: Phase 2 done, Phase 3 next".

**Phase 4 (2026-10-23, slices 18-20; plan `docs/superpowers/plans/2026-10-23-atari-st-phase4.md`, spec Status 2026-10-23).** Two more machine ids in the Atari plugin, `atari-mega-st` (192K ROM at `$FC0000`, the Blitter, the RP5C15 clock at `$FFFC20`, options `rtc_time`/`rtc_mode`) and `atari-ste` (256K ROM at `$E00000`, the STE Shifter with 4096 colours, scroll, line offset and writable counter, DMA sound with the LMC1992 volume control and stereo output, enhanced joystick ports with `Joy1Pad*`/`Joy2Pad*` keys, the Blitter); `--firmware auto` finds EmuTOS 256K for the STE. The Blitter (`crates/chips/blitter`) is checked word for word against Hatari's P33 and P37-P40 dumps, shares the bus as P34 shows (HOG stalls the CPU, else 64 clocks of words against 96 of CPU) and raises GPIP 3's edge. The option `watch` (`$1000-$10FF:w,$400`) stops the machine at the first data access with a report; the host has `--acsi-save N=FILE` (at exit), `--at FRAME:save_acsi=N:FILE` and the unbound hotkey `save_hd` (ACSI 0), which write a copy mid-run and count its changes as saved. Manifests `machines/atari/atari-ste.manifest.toml` and `atari-mega-st.manifest.toml` (S16 and P31/P33/P37-P40 exact; P29/P30/P32/P34/P41 `known-gap` with their reasons). The fuzz targets ran 60 s each with no finding. Left open: see TODO.md "Atari ST: Phase 4".

## Sega Mega Drive / Genesis (2026-10-24)

`machines/sega` (`elpapimango/retroclawd-sega`, public) builds `sega_machine_plugin` with one machine id, `genesis`
(NTSC/PAL/Japan from the ROM header, `--console-region` overrides). Design `docs/superpowers/specs/2026-10-24-sega-genesis-design.md`,
plan `docs/superpowers/plans/2026-10-24-sega-genesis.md`; every hardware fact is a row of `machines/sega/docs/hardware-facts.md`
(`[D]/[H]/[O]/[H?]`, sources in `docs/sources.md`). Two CPUs: the 68000 (`mc68000`, `step()` returns master clocks, 53.69 MHz NTSC /
53.20 MHz PAL) and the Z80 (`z80`, run lazily to the 68000's clock by `Bus::catch_up`, always before the VDP is advanced); the profile
lists both (`cpus = main, z80`) and the debugger follows either through `OnCpu`. Chips in the family: `vdp` (planes, scroll, window,
sprites, shadow/highlight, FIFO, DMA with per-line rates, HV counter, HINT/VINT), `ym2612` (a Rust re-implementation of the
die-level pipeline, cycle-identical to the Nuked OPN2 reference harness on 80 random register scripts, both chip variants),
`sn76489`; format `md-cart` (headers, `.smd`, SRAM info). Cartridge: power-of-two mirroring (Sonic & Knuckles leaves its upper 2 MB
open), SSF2 mapper (> 4 MB or header `SEGA SSF`), battery SRAM (`sram_file`, saved when the guest locks it and on drop), TMSS boot ROM
(`--tmss auto|off|on`, `--rom-tmss`). Timing: bus refresh (2 of every 130 CPU cycles), Z80 access to the 68000's bus (3 Z80 clocks
wait, 10 CPU cycles lost by the 68000, waits for DMA), VDP DMA holds the 68000. Save states are `RCST` v2 (ROM CRC, region, chip,
full validation, fuzzed by `RC_SWEEP`). Interlace modes 1 and 2 (single field), serial EEPROM boards (EA, Sega, Acclaim) and the Sonic & Knuckles lock-on (`--lock-on`)
are done, and since 2026-10-29 so are Codemasters J-Cart pads, cheat codes (option `cheats`: Game Genie `XXXX-YYYY`, `AAAAAA:DDDD`; no lock-on cartridge ROM), weave interlace (`interlace_output=weave`), the Sega Mouse, 4-Way Play and Menacer (options `port1`/`port2`), the Sonic 2 lock-on chip mechanism (`lock_on_chip`, untested against a real dump), `--execution-mode fast|accurate` (fast drops refresh, Z80 cost and FIFO stalls; `docs/benchmarks.md`) and mid-line palette/backdrop changes (fact V34). Not done: 32X, real Game Genie/Action Replay cartridge ROMs; see `TODO.md` "Sega Mega Drive". Oracles for comparison are the libretro cores in a RetroArch install
(`retroclawd-tools/scripts/libretro_run.py`; Genesis Plus GX is the main reference); `machines/sega/scenarios/` has the probes
(`probe.py`), the frame comparison against Genesis Plus GX (`compare_frames.py`, `batch_compare.py`) and `sweep.py` (every game of
`software/roms/` for N frames). Manifest: `machines/sega/genesis.manifest.toml` (`rc-scenarios`).

**Sega CD (2026-10-29, slices 1-2 of 9; spec `docs/superpowers/specs/2026-10-29-sega-cd-design.md`, plans `docs/superpowers/plans/2026-10-29-sega-cd.md` and `...-slice2.md`)**: machine id `segacd` in the same plugin library, a Genesis with an `Expansion` (family-internal hook on the cartridge port, `genesis::Expansion`) that owns the Mega-CD: `crates/machines/segacd` (second `mc68000` as the sub-CPU at 12.5 MHz from the master clock by an exact integer ratio, 512 KB program RAM with bank window and write protect, 256 KB word RAM in 2M mode with DMNA/RET hand-over, 8 KB backup RAM, the BIOS at `$000000` and the mode-1 windows `$400000/$420000/$600000`), `crates/chips/mcd-gate-array` (both register sets, mailboxes, stopwatch, timer, interrupt levels; forced-reset sequence), `crates/formats/disc` (cue/BIN/ISO reader, sector reads, TOC, test-disc builder). Facts C1-C16 and D1-D3 in `machines/sega/docs/hardware-facts.md` come from the scanned Mega-CD Hardware Manual (`MCD-HW`; Sega Retro's article pages need a browser, its image host does not). Options: `rom_bios` (131072 bytes, `--firmware auto` by console region through `firmware.toml`) and `disc` (a path to a `.cue`/`.iso`, opened at start-up; the drive is not emulated yet). The debugger has a third CPU `sub`. Slice 3a (2026-10-29, same day): the CD drive port (`mcd-gate-array/src/cdd.rs`: ten-nibble status and command packets, checksum, HOCK, a level 4 interrupt every 1/75 s, power-on all-zero packet then no-disc answers), the CD controller's register file (`cdc.rs`, LC8951 tables, no decoder yet) and the PCM chip's registers and wave RAM (`crates/chips/rf5c164`, nothing played) are in; the real US BIOS now runs its whole start-up through the drive handshake and stops at the graphics unit's registers (sub write `$FF8058`, slice 7), with a black picture (Genesis Plus GX shows the logo and PRESS THE START BUTTON at frame 3000). The graphics ASIC (`crates/chips/mcd-asic`: stamp map, trace vector, image buffer, font converter, priority modes; facts G1-G9; rotation sense unverified) is in ahead of its slice, and with the one-cycle-late VDP DMA from word RAM (fact C17) the real US BIOS draws its Sega CD screen (earth, logo, PRESS THE START BUTTON) like Genesis Plus GX (mean difference 0.5 per channel at frame 3000). Slice 3b (partly, 2026-10-29): the CD controller decodes blocks into its 16 KB buffer (headers, pointers, STAT3 clearing the decoder interrupt, level 5), transfers go to the host port or by DMA to program RAM, word RAM and the PCM chip (`mcd-gate-array/src/cdc.rs`), and `crates/machines/segacd/src/drive.rs` is a disc-backed drive (`--disc x.cue`, facts E11-E15: table of contents reports, seek, read, one block per 1/75 s, `Drive` trait in `cdd.rs`). **Disc boot works (2026-10-29, later):** the real BIOS now boots the Dragon's Lair disc to its title screen. What it needed besides the drive model: a head in the lead-in reports the first (data) track and RS8 bit 2, seeks last 12 periods, the controller sees the headers of the lead-in sectors the BIOS seeks into, the data/music bit 8 of `$FF8036`, a write to either byte address of the flag word sets that CPU's flags (facts C9, E15), the 1M/1M word RAM mode with its swap protocol and the cell image (`$220000`) and dot image (`$080000`) views (C8, C18, C19; the BIOS boots in 1M mode). Debug aids: `crates/machines/segacd/tests/bios.rs` (ignored; `RC_BIOS`, `RC_DISC`, `RC_CDDLOG=short`, `RC_IOLOG`/`RC_IOFILTER`/`RC_SUBWATCH`, `RC_MAINIO`, `RC_RAMWATCH`, `RC_DUMP_RAM`, `RC_TRACE`, `RC_LIST`/`RC_SUBLIST`), and `libretro_run.py` with Genesis Plus GX and `--ram-at` to compare the BIOS's RAM variables over time (RAM dumps are byte-swapped in pairs). Later the same day: sub-CPU addresses repeat every 1 MB and main word RAM every 256 KB (facts C20, C21), which let Silpheed reach its Sega logo and Shining Force CD and Lunar reach the BIOS prompt about unformatted backup RAM; the 8 KB backup RAM now persists in a file (option `backup_ram`, default `saves/segacd-backup.brm`, read at start, written at exit when changed). **Audio and backup RAM (2026-10-29, later):** the RF5C164 plays (`rf5c164`: 384 source clocks a sample, 5.11 address step, `$FF` loop byte, envelope and pan, facts P4-P6), the drive plays audio tracks (command 3 on an audio track, 588 stereo frames a period, fact E17) and `segacd/src/audio.rs` adds both to the Genesis output through the `Expansion::mix_audio` hook (C22); CD audio matches Genesis Plus GX (RMS 4469 against 4471 on a synthetic tone disc through the BIOS CD player). Lunar and Shining Force CD ask to format the backup RAM, then run (C23); the file is the `backup_ram` option. **Disc routing (2026-10-30):** a text machine option whose value is `romm:<id>` or a `.zip` (the `disc` option) is unpacked whole into `<cache>/unpacked/` by `apps/retroclawd/src/media_set.rs` (zip-slip safe, once per archive version, a marker file makes an interrupted unpacking redo, sets unused for 7 days are pruned) and replaced by the `.cue` (else `.gdi/.iso/.chd/.toc`) inside; a lone file from RomM passes through. Checked by booting Dragon's Lair from a zip. **CHD and save states (2026-10-30):** `--disc x.chd` works (CHD v5, `machines/sega/crates/formats/chd`, verified against the SHA-1 in the header of a 266 MB real file; GD-ROM and parent files are refused in words; a pregap type starting with `V` is stored in the track's frames, any other is silence, found against chdman 2026-10-30). That crate is a general CHD library now (all nine hunk codecs both ways, parents, a writer, in-place metadata edits) and `rcchd` in `retroclawd-tools` is `chdman`'s work on top of it: createcd (cue/bin, iso, gdi) / createhd / createdvd / createraw, the extracts, copy, addmeta/delmeta/dumpmeta, checked against chdman 0.289 (its files read here, ours pass its verify with the same SHA-1s, uncompressed files byte-identical), and the Mega-CD has save states (`Expansion::save_state`, RCST schema 3: sub-CPU, RAM, gate array, CDC/CDD, PCM, graphics unit, drive position; BIOS and disc identity are checked; real BIOS plus Dragon's Lair, Silpheed and Lunar resume bit-exactly). `machines/sega/segacd.manifest.toml` (S1, S3 on three discs, S5, S4 and S6 against Genesis Plus GX: 11 pass, no gap). Final Fight CD (black screen) was a dropped CD controller transfer to word RAM in 1M mode (fact E18, fixed 2026-10-30; found by tracing a private build of Genesis Plus GX, see `machines/sega/CLAUDE.md` "Final Fight CD, solved"). A boot sweep of ten discs (2026-10-30) found Snatcher and Eternal Champions black too: two more bugs, the word RAM mode switch losing its contents (fact E19) and the table of contents report's track digit (fact E20), both fixed the same day; Earthworm Jim stopped on a read of `$FF800A` (the CDC DMA address reads back, fact E21, fixed the same day); all ten discs now boot.