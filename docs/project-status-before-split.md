# Project status as of the per-family split (2026-10-12)

The long status paragraph from CLAUDE.md, kept for reference. The machine families it describes now live in their own repositories (see README.md).


Working code. Cargo workspace: 15 library crates + 1 binary app (see
"Crate layout" below). Complete Z80 core validated vs ZEXALL/ZEXDOC,
CPU-agnostic machine abstraction (now proven against a second CPU family
too -- see the new `mos6502` crate below), ZX81 and ZX Spectrum (48K, 128K/+2, and +2A/+3 memory
model, via `zx_spectrum::Model`) each w/ real keyboard matrix + real video
+ real audio (beeper all Spectrum models, AY-3-8910 128K/+2A/+3) wired,
ZX Spectrum also loads real `.tap`/`.tzx` tape files at real-time EAR-bit
pulse speed (turbo multiplier selectable) via the new `zx-tape` crate,
shared Sinclair keyboard-matrix crate, shared AY-3-8910 sound-chip crate,
2 snapshot-format crates (`.z80`, `.p`/`.p81`, load-only so far -- `.z80`
now parses +2A/+3-shaped snapshots too, see `Model::Spectrum2A3`'s own
notes below), CLI runner w/ Bevy audio output, dedicated emulation
thread decoupling CPU/audio/video from the GUI's own render loop (see
`emu_thread` below), the +3's floppy disk controller (new `nec765`/
`dsk` crates, `--disk <path>`, now writable with opt-in persistence via `--disk-save`/`--disk-writeback`), per-game TOML config
(new `game-config` crate, `--game-config <path>`, layered over the
existing CLI flags), internal save states (`--dump-state`/`--load-state`),
an optional input-latency "run-ahead" mode (`--run-ahead <N>`), and a
browser-based interactive debugger (`--debugger`, registers/memory/
disassembly/breakpoints/pause-step/assemble-and-poke, see `debugger`/
`debug_server` below). Full ZX Spectrum model coverage and per-game TOML
config are both done. **Current architecture:** machine and debugger
integration now runs through `plugin-api`, with bundled or dynamically loaded
plugins. `HostedMachine` in `plugin_bridge.rs` replaced the old `AnyMachine`
enum; the debugger UI, debug log, and ZRCP transports are plugins. Remaining
active work is machine parity and removal of superseded diagnostic paths (see
`TODO.md`).
Amstrad CPC support -- `crates/machines/cpc` now covers CPC464, CPC664, and CPC6128, each boot-verified against its
real combined firmware ROM. Shared hardware includes register-driven CRTC,
Gate Array with all three official video modes and one-in-four RAM arbitration,
i8255/AY keyboard/audio, and the 32-entry palette. `cpc::Model` selects 64K
464/664 or 128K banked 6128 memory; 664/6128 expose AMSDOS upper-ROM slot 7
and the reused `nec765`/`dsk` floppy path through real CPC port decode. Save
states preserve model-specific RAM paging, cassette motor/tape position, and
FDC command phase while disk/tape bytes remain external. `Cpc464Plus`/
`Cpc6128Plus`/`Gx4000` add the real CPC+ ASIC on top (4096-colour palette,
hardware sprites, screen split/soft scroll, PRI, vectored IM2 interrupts,
DMA sound, analogue joystick, GX4000's mandatory cartridge boot) -- see
`crates/machines/cpc`'s own bullet below. And a
Commodore 64 track (started specifically to prove the
machine-abstraction claim above against a real second CPU family) --
`crates/core/6502` (package `mos6502`) now has a complete 6502 core: full
documented instruction set, all addressing modes, real NMOS quirks
(page-cross extra cycle, `JMP` indirect page-wrap bug, `JSR`/`RTS`/`BRK`/
`RTI` stack conventions), and the illegal/undocumented opcode set
(in scope from day one per this track's own plan; every one of the 256
possible opcode bytes has defined behavior -- Rust's own non-exhaustive-
match error was the check that caught any gap). Proving `machine::Bus`
genuinely generalizes needed exactly one change: `Bus::read_io`/
`write_io` gained default bodies since the 6502 has no IO-port space at
all -- see `machine`'s own notes below. Validated by focused unit tests,
Klaus Dormann's documented-opcode functional test, and Tom Seddon's
exhaustive NMOS decimal/illegal-opcode suite. The latter found and fixed
two real bugs: decimal `ADC`'s N/V source and decimal `ARR`'s missing
nibble corrections; see `TODO.md` and `CREDITS.md`.
`crates/machines/c64` boots real KERNAL/BASIC/character
ROMs to an actual `READY.` prompt -- real PLA/6510-port memory banking,
`crates/chips/vic-ii` (raster IRQ, bad-line cycle stealing, all 5 real
display modes, and all 8 hardware sprites with sprite collisions), a full CIA1 +
CIA2 (Timer A/B, TOD clock, serial register; CIA1's Timer A wired into
`mos6502::Cpu::request_irq`, CIA2's own IRQ correctly routed to
`request_nmi` instead, and CIA2's Port A now drives real VIC bank
selection too -- see below), and
a real `crates/chips/sid` (MOS 6581/8580, oscillators/noise/envelope/
registers, and now a real analog state-variable filter too) wired into
its own `$D400`-`$D7FF`
I/O window and the audio/save-state paths, `--machine c64` +
`--rom-kernal`/`--rom-basic`/`--rom-char` (a real C64 needs three ROM
files, not one, unlike every Sinclair machine here) in `retroclawd`.
Real bug found and fixed getting there: `vic_ii::Vic`'s
`$D012`/`$D011` raster registers were pure storage with no live counter,
so real KERNAL boot code's own `LDA $D012 / BNE` busy-wait loop (used
during init, not just for raster IRQs) spun forever -- `Vic::tick`
now advances a live raster counter every `C64::step`; see `TODO.md`'s
"Commodore 64 support" section for the full diagnosis (disassembled the
real ROM to find it, then verified the fix by reverting it and watching
the new regression test fail, not guessed). Drive/media parity has
progressed a lot too: `crates/chips/via-6522` (package `via6522`, a
standalone MOS 6522 VIA) and `crates/machines/1541` (package `c1541`, the
1541 modeled as its own real `mos6502`-based computer) are both done,
wired to `c64::C64` over a real, software-visible IEC serial bus;
`crates/formats/d64` parses real 1541 disk images (track/sector/BAM/
directory); and `c1541::C1541`'s VIA2 is now wired to a real GCR (Group
Code Recording) disk mechanism -- head stepper motor, spindle motor,
density select, and the real bit-level SYNC-detection/byte-assembly
hardware DOS firmware bit-bangs through, with byte-ready wired to both
VIA2's own `CA1` and (a real, distinct hardware fact) the 6502's own SO
pin (`mos6502::Cpu::set_overflow`, new this pass); and `retroclawd`'s
existing, fully generic `--program`/`--org` path now auto-detects a
`.prg` extension, reading its own real 2-byte load address instead of
requiring `--org`; the new `c64-tape` crate adds real `.tap` (a bit-level
pulse capture, wired into CIA1's FLAG pin + the 6510 port's real
MOTOR/SENSE bits) and `.t64` (a directory container with no real
bitstream, poked into memory like `.prg` instead) cassette loading via
`--tape`. `--disk` now also accepts a real `.d64` image for `--machine
c64`, serviced by `c64::C64::try_fast_kernal_load_trap` -- a real
IEC-level fast loader that traps KERNAL LOAD's own entry point
(`$F49E`) and answers it directly from the attached image's logical
contents, no simulated 1541/IEC-bus bit-banging at all (deliberately
simpler than the real, separately-clocked-machine wiring `c1541`/
`tests/iec_1541.rs` already prove works, since nothing here needs the
real bit-level protocol to get the same logical result) -- plus `--run`
auto-run scaffolding (`.d64` autotypes `LOAD"*",8,1` then `RUN`; a raw
`.prg`/`.t64` poke synthesizes BASIC's own VARTAB/ARYTAB/STREND pointers
first, then types bare `RUN`), mirroring CPC's own `--run`/autotype
precedent. See `c1541`'s/`d64`'s own bullets, `c64`'s own tape/disk
paragraphs, and `retroclawd`'s own `--program`/`--tape`/`--disk`/`--run`
notes below, and `TODO.md`'s "drives and baseline media" section for
what's still explicitly out of scope (disk writing is `.d64`-only SAVE, and any
`retroclawd`-level dual-machine integration). ZX81 stays parked as-is
in the meantime (including
its own audio and tape support -- neither modeled there yet, see
`TODO.md`). Keep file current as structure changes.

