# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this is

The Atari family of [retroclawd](https://github.com/elpapimango/retroclawd), a submodule of
the application at `machines/atari`. The ST design is `docs/superpowers/specs/2026-10-19-atari-st-design.md` and the
current plan `docs/superpowers/plans/2026-10-19-atari-st-slice1.md`, both in the application repository: read
them first.
The Atari ST family of [retroclawd](https://github.com/elpapimango/retroclawd) as a machine plugin, a submodule of
the application at `machines/atari`. The design is `docs/superpowers/specs/2026-10-19-atari-st-design.md` and the
current plan `docs/superpowers/plans/2026-10-21-atari-st-slice6.md`, both in the application repository: read
them first. Every hardware fact in code is listed in `docs/hardware-facts.md` with its source and a label
(`[D]` datasheet, `[H]` documentation, `[O]` observed in Hatari as a black box, `[H?]` unresolved); an I/O address
with no fact stops the machine (`io.rs`) instead of returning an invented value.

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
crates/chips/mc68901/                MC68901 MFP: GPIP, interrupt controller, timers, USART (timer D clocked)
crates/chips/st-shifter/             Shifter registers and the frame render (fetch.rs: picture window and borders; overscan.rs: the
                                     832 x 552 canvas)
crates/chips/ikbd/                   the keyboard controller, behavioural, from Atari's ikbd protocol document
crates/formats/st-floppy/            .st and .msa images: geometry, sectors, MFM tracks for the WD1772, written-track decode, boot.rs
                                     (a 720K floppy around a GEMDOS program)
crates/machines/atari-st/            the machine: config, bus, I/O decode (io.rs), events.rs (time and events: `accurate` mode's
                                     per-cycle clock), waits.rs (wait-state, E-clock and DMA constants with their probes), MMU, MFP clock,
                                     scheduler, PSG
                                     (psg.rs), ACIAs + serial peers (acia.rs), DMA + FDC (dma.rs), drives (drive.rs),
                                     I/O log (Hatari format), diagnostics, Machine impl, state.rs (RCST save states), serial.rs
                                     (file/echo peers for RS-232 and MIDI), acsi.rs (hard disks behind the DMA chip)
crates/plugins/atari-st-machine-plugin/  profile, options, commands (lib `atari_st_machine_plugin_impl`)
crates/plugin-dlls/atari-st/         the cdylib (`atari_st_machine_plugin`), only `export_plugin!`
timing/                              the probes (68000 programs run in Hatari and here), build.sh, README.md
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
Shared chips used from `retroclawd-crates`: `ay-3-8910` (YM2149 variant), `mc6850`, `wd1770`.

## Tests that need files

The EmuTOS tests (`crates/machines/atari-st/tests/emutos.rs`) need `RC_EMUTOS` (a path to `etos192uk.img`, or the
`--firmware auto` cache) and, for the Hatari comparisons, `RC_GOLDENS` with `atari-st/emutos14-uk-trace.txt` and
`atari-st/emutos14-uk-io-{256k,512k,1m,2m,2.5m,4m}.txt` from `retroclawd-tools/scripts/capture_hatari_trace.py`
(`--kind io --memsize 256|0|1|2|2560|4 --vbls 50`), plus `emutos14-uk-io-1m-1500.txt` (`--memsize 1 --vbls 1500`) and
`emutos14-uk-io-1m-boot.txt` (the same with `--disk-a` and the boot floppy the test `boot_floppy_is_executable`
writes to `RC_DUMP_BOOT_FLOPPY`). Without them they print where they looked and pass. The comparison rules (polling,
masks for timing-dependent and documented-conflict values, per-location streams, sets and ordered sequences) are
documented and unit-tested in `tests/common/mod.rs` and `tests/polls.rs`. `RC_SCREENSHOT_DIR` makes the desktop test
write its 1500-frame picture.
`tests/romm_games.rs` needs `RC_ROMM_GAMES`, a folder of the unzipped `.st` disk games from the owner's RomM (platform
`atari-st`; `retroclawd romm games atari-st`): each boots without a stop, gives the same frame on two runs, and is not
modified by being read. Never committed; without the variable the tests pass and say so.

## Phase 2 notes

- Hardware facts of Phase 2 (USART, ACSI, cartridge, raster, GEMDOS programs) are in `docs/hardware-facts.md` with their
  sources; the ACSI application notes (1985) and the Atari Compendium chapters 2 and 4 are the primary ones, by SHA-256.
- Scenarios: `scenarios/build.sh` needs `retroclawd-tools` built (`rcdisk`, `rcld`); `hello.s` and `hello.layout.toml` make
  `hello.prg`, `rcdisk hd` makes `hd.img`. `RC_SWEEP=<n>` multiplies the hostile-input loops of the tests (`msa`).
- `tests/prg.rs` runs a hand-made GEMDOS program from `AUTO\PROG.PRG` (EmuTOS needed; `RC_DUMP_AUTO_FLOPPY=<file>` writes the
  floppy it builds). A program from AUTO runs in user mode: it must not write below `$800`.
- The fuzz targets (`crates/machines/atari-st/fuzz`, `crates/formats/st-floppy/fuzz`) need `cargo +nightly fuzz`; not run here.

## Phase 3 notes (2026-10-23)

- `--accuracy accurate` follows every bus cycle (`StBus::begin_cycle/end_cycle`, `waits.rs`): a device sees a read 3 clocks and
  a write 0 clocks into its 4-clock cycle, the bus is aligned to 4 clocks, the MFP, PSG, FDC data register and `$FF8606` writes
  cost 4 more, the ACIAs wait for the E clock (+8), an interrupt acknowledge likewise, a 16-byte floppy DMA burst holds the
  CPU for 32 clocks. The VBL request comes 64 clocks into a frame. `fast` mode keeps its byte-exact states (`tests/fast_identity.rs`,
  `RC_RECORD_IDENTITY=1` re-records the table after an intended change); both modes share the device facts (floppy rotation, IKBD
  answer delays), so those changes re-record it.
- The Shifter records palette, resolution and sync writes with their clock (`PaletteChange.cycle`, `SyncWrite`); `Fetch`
  turns them into the picture window and the left/right/top/bottom borders; `render_exact` draws mid-line palette changes;
  `borders = on` (accurate only) draws the 832 x 552 canvas (`render_overscan`); the mono monitor is 501 lines of 224 clocks.
- Every timing fact has a probe in `timing/` (`timing/README.md`, `docs/timing-oracle.md`), a test in
  `tests/timing_probes.rs` and, where the dump is compared byte for byte, a manifest scenario; the ones that differ from Hatari
  are `known-gap`s with the reason. `RC_GOLDENS` holds Hatari's dumps (never committed). `docs/benchmarks.md`: `fast` 13.8x and
  `accurate` 12.8x real time on the idle desktop.
- States: `accuracy` in the header is a bit set (1 accurate, 2 borders, 4 mono); a state loads only into the same canvas and
  mode. `mfp_clock` is schema 2.
- Fuzz targets (`crates/*/fuzz`, `crates/machines/atari-st/fuzz/accurate_regs.rs`) compile but need `cargo +nightly fuzz`; the
  stable `tests/hostile_video.rs` covers the same ground (`RC_SWEEP=<n>`).


## Phase 4 notes (2026-10-23)

- Chipsets: `Chipset::{St, MegaSt, Ste}` (`AtariSt::with_chipset`; `new` is the ST); the plugin has three profiles. ST states stay byte-identical: the
  chipset part (`ChipsetExt`: STE video registers and `SteRegs`) is appended to the payload for the other two only, the header's accuracy byte
  has the chipset in bits 3-4.
- New code: `crates/chips/blitter` (register file and word engine over `BlitBus`), `src/ste.rs` (STE registers, pads, Blitter timing state),
  `src/dmasound.rs` (DMA sound, LMC1992), `src/rtc.rs` (RP5C15), `src/watch.rs`; the Blitter's turns with the CPU are `StBus::start_blit/blit_burst/
  blit_tick` (`bus.rs`); the DMA sound is brought up to date by `run_dma` (only while playing at MFP catch-ups, always at its registers and when
  audio is drained).
- Probes P27-P41 (`timing/README.md`); goldens in `RC_GOLDENS/atari-ste/timing/` and `atari-megast/timing/` (`capture_hatari_mem.py --machine ste|megast`).
  `timing/scan_p31.py` and `scan_p32.py` find which addresses of an area answer. Fuzz: `cargo +nightly fuzz run <target>` needs the MSVC ASan
  runtime directory on PATH on Windows; corpora, artifacts and `fuzz/target` are git-ignored.
- Tests: `tests/blitter.rs`, `blitter_timing.rs`, `ste_sound.rs`, `ste_ports.rs`, `rtc.rs`, `watch.rs`, `phase4_models.rs`, and the Hatari comparisons
  in `timing_probes.rs` (p27-p41; the DMA counter and P34 loops compare within a tolerance, their manifest rows are `known-gap`).
