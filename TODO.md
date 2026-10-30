# TODO / Roadmap

Updated 2026-09-28.

This file is the current backlog, not a development diary. Completed implementation detail belongs
in `CLAUDE.md`, source comments, `CREDITS.md`, and git history.

Status labels:

- **NEXT** — actionable and prioritized.
- **BACKLOG** — valid work, but not currently prioritized.
- **BLOCKED** — requires verified external information or another prerequisite.
- **PARKED** — deliberately inactive; do not let it drive unrelated design.
- **DONE** — retained only when it defines the baseline for remaining work.

## Test review: acid tests, not exhaustive validation (2026-10-17)

Decision: the aim is a simple, capable emulator, tested the way an acid test would: third-party programs and published facts, not proof of
real hardware. Plan agreed: (1) review everything in two passes with a keep/trim/remove list for approval -- **pass 1, the 101 `rc-suite` cases,
is done (101 -> 69)**, pass 2 (the unit and integration tests of every repository) is next; (2) then add acid-style suites as cases with a known-gaps
list: CPC Acid Tests Suite (cpcwiki) and cpctech `test.zip` (fetched, hash-checked, never committed), Acid800 (`Emulators\Acid800-1.2`, Atari 800 is
parked, so as a model of an acid test's layout), and a few small acid-style programs of our own for the C64 and the Spectrum (documented behaviour,
expected results from documents; ChronoCoders' C64 tests and kondrak's rust64 are reference reading only, nothing from them is run or copied).
Pass 1 removed: c128 MMU version register, `$FF01` load; c64 CPU port values, raster 0-311, typed PRINT, the dead test alone (the unified case runs
it), the 6581-with-paddle case (and the BAD assertion in the unified case; the missing-plug explanation stays in the notes below); cpc PSG channel A/B/C
alone, mixer mute, noise, volume (AmstradDiag's sound test and the chord/register-1 cases cover them), joystick-FAILED, soak, 664 ROM checksums;
spectrum FRAMES counter, ROM-halves-equal-the-file; zx81 ROM ignores writes, 16K pack, last cell of each RAM size, saved-file round trip, SAVE format,
16K K prompt. Merged (all checks kept): cpc AMSDOS single/multi-sector, keyboard odd/even, joystick directions; spectrum frame lengths (3 -> 1), RAM pages and fixed banks,
ROM select bits; zx81 keyboard plain/shifted.

**Pass 2 of the test review is done (2026-10-17): 2046 `#[test]` functions became 1317** -- host 347 -> 249, shared 500 -> 372, sinclair 263 -> 149,
amstrad 222 -> 137, commodore 569 -> 320, tools 145 -> 90 -- by the rules agreed that day: keep third-party and document-based checks (CPU
core tests, ZEXALL/Klaus Dormann/Seddon, real-ROM acceptance tests), one regression test per real bug, data-loss and secrecy guards, one parser test per
format feature; remove tests that restate our own code, duplicates of an rc-suite case, trivia and exploration probes; merge tests that differ only in a
literal. A merged test **keeps every check**: the originals became plain helper functions that one `#[test]` calls in order (a merge never crosses a
module or a `#[cfg]`). The CPU cores' instruction tests were not merged (a merged test would hide all but its first failure). Also fixed on the way:
the tools tests that run real ROMs looked for `48.rom` (the file is `spec48.rom`), and the real-disks round trip now skips when no `.d64` is present.
Not touched: the ignored real-ROM tests (only the four exploration probes `dump_picture`, `probe_disk`, `play_disk_image`, `play_media` were removed).

**Acid suites (2026-10-17, step 2 of the review)**: `tools/fetch_acid.sh --from <Acid_Tests_Suite folder>` copies the third-party acid tests
from a folder you already have into the git-ignored `machines/<family>/software/acid/` folders, SHA-256 checked, never committed.
*Patrik Rak's z80test 1.0* (inside the cpcwiki suite) runs on the 48K as `spectrum_z80test` in rc-suite: six variants, 152 CRC-checked tests each,
compared with a real 48K with a Zilog Z80; the runner answers the `scroll?` prompt only while it shows (a key pressed during a test would change the
IN tests) by rerunning from reset with one more press per round, and reads the `Result:` line from the picture with the 48K ROM's own character set.
It found three real gaps, all fixed the same day: the ULA's idle EAR bit (port 0xFE reads 0xBF with no tape playing, was 0xFF), the N flag of
INI/IND/OUTI/OUTD (bit 7 of the byte, was always set), and SCF/CCF F3/F5 (`(Q ^ F) | A`, Q = F if the previous instruction wrote F). full, doc, flags,
docflags and ccf now pass; **z80memptr gap closed 2026-10-18**: MEMPTR/WZ is modelled in the Z80 core (all six variants pass).
**CPC acid tests (2026-10-17)**: Kevin Thacker's suite (cpcwiki) has about ten programs that print `name-PASS`/`name-FAIL` themselves and end with
"Done."; the rest (most of crtc, cpctests, plus pictures, devices) draw pictures or raw data to compare with a real CPC, so they are **not** cases (decided:
self-checking programs only). Run as `cpc_acid_tests` (rc-suite `tests/cpc_acid.rs`): `RUN"NAME` on the author's disk, the transcript read with the new
`--capture-at 0xBB5A:A=FILE` (host: logs register A each time the CPU reaches an address -- here the firmware's TXT OUTPUT -- so scrolled and paged text
is complete), segments of 200 frames through a save state, Space pressed only after a silent segment, menus answered by prompt text. Each program has a
recorded state (`BASELINE`: does it finish, which tests fail); a new failure, a test that now passes, or a program that stops finishing fails the suite.
Today: **romtest passes**; ppi 11 failing (8255 modes 1 and 2 are not modelled, port B latch, control-write reset), psg 4 (PSG port A output read-back,
register masks), cpctest 2 (interrupt position/width) and it returns to BASIC at the GA I/O decode test, type1ctrl 2 (light pen status), and on the CPC+
asicfloat, asiclock, asicppi (hangs), asicrom, asictest (hangs), asic_external_ram, dmatest (hangs), pritest (hangs). The failing lists are the known gaps, to be
fixed or explained one by one. **Bugs this found and fixed**: the DSK reader refused the acid disks' header (`MV - CPCEMU / 13 May 17`, now any header starting
`MV - CPC` / `EXTENDED`), and a CPC save state dropped the 8255, the keyboard row and the selected PSG register (a restored machine's PPI was in power-on mode; the
ROM test printed nothing after a restore). **Not done**: the disc FDC programs (`disc/fdc_*`, `fdctest3`; they print PASS/FAIL but need a chain of setup answers --
FDC model 2, /DACK 1, drive configuration 1, multi-drive N, drive 0, 40 tracks, slow ready -- and print thousands of detail lines page by page: a run is 200k frames
and unfinished; they need a faster way through paged output first), the CRTC programs (their files have `_` in the name, which AMSDOS answers `Bad command`
to when typed; they print raw data anyway), `keyb`/`keyb2` (need a person at the keyboard), `devices/`. Then: our own small C64 and Spectrum programs.

**Own acid programs, C64 and Spectrum (2026-10-17, last part of step 2)**: small programs of our own, each one documented fact, the document named in its case
(sources chosen by the user: Bauer's VIC-II article, the C64 Programmer's Reference Guide / c64-wiki pages, the MOS 6526 datasheet; the World of Spectrum 48K and 128K
references, the Sinclair wiki's floating-bus page, the Kempston interface manual, the Z80 timing tables). rc-suite files `spectrum_timing.rs`, `spectrum_io.rs`,
`c64_hardware.rs`; `Outcome.cycles` now also reads a run to HALT ("N instructions, M cycles"), which is how the Spectrum programs measure: a program starts at T-state 0
of the frame, fillers put one access at a chosen T-state, and the cycles with the access in contended and in uncontended RAM differ by the delay.
**Spectrum**: contended memory (6,5,4,3,2,1,0,0 from 14335, 192 lines of 224, none in the border, 14361 and 228-T lines on the 128K), port contention outside 0x40-0x7F, the
ULA answering every even port with 0xBF, Kempston (8/4/2/1, +16), AY read-back, 0x7FFD and 0x1FFD decode and the beeper (twenty 400-T periods = twenty edges 5.04 samples
apart) all agree with the documents. **Gaps found, all closed 2026-10-18**: the floating bus sampled one T-state late (fixed); the "I/O contention gap" for ports 0x40-0x7F was a bug in the test's filler (it trashed BC), not the emulator; C64 bad lines already work with `--vic-accuracy cycle` (the case passes it); unconnected VIC bits now read 1 and $D02F-$D03F read $FF. Also fixed via the CPC acid tests: 8255 modes 1/2, AY invalid register select, CRTC lightpen/status, ASIC unlock sequence, CPC save state now holds the PPI. **CPC gaps closed 2026-10-19** (details in `machines/amstrad/CLAUDE.md`, "CPC Plus acid fixes"): asicfloat, asicppi, asicrom, asic_external_ram and asictest now pass completely, cpctest's HSYNC position/width pass (VSYNC sampled at the I/O cycle), dmatest 21 of 25, pritest 13 of 20. **CPC known gaps left** (in `BASELINE`): dmatest (prescale changed mid-pause, CRTC R0 length, the channel 1 interrupt test), pritest (re-trigger/trigger-bug HSYNC cases, "PRI start (R2)" at the end of the line, "PRI can delay normal raster interrupt", "enable norm during pri int handler", "im2 vector (norm)" which expects one classic raster interrupt per frame), and the FDC programs. **FDC timing model (2026-10-19, `retroclawd-crates` `nec765`, `enable_timing` on the CPC)**: the disk turns
(300 rpm, 32 us per byte), Read/Write Data wait for their sector and move one byte every 32 us with overrun, Read ID waits for the next ID,
Format runs index to index, Seek/Recalibrate step at the Specify rate with a PCN per unit (units 2/3 are drives 0/1: only US0 is wired)
and keep the MSR busy bits until Sense Interrupt Status, a drive's ready change raises an interrupt per unit, and the Scan commands
exist. Before: fdc_gen 7 passing (hung), fdc_seek 5 (hung), fdc_read/fdc_write/fdc_time hung on their overrun tests, fdctest3 0. After:
fdc_gen 26, fdc_seek 29, fdc_read 10, fdc_write 12, fdc_time 2, fdctest3 18, all but fdctest3/fdc_wp finishing. Left (in `BASELINE`):
DMA mode, the timing measurements of fdc_time, ready changes during a command, the format result registers and FM, the read/write
"data error" and deleted-mark cases, overlapped seeks finishing on the same head, "read id not ready" (motor spin-down); several are
test bugs (noted in `BASELINE`). The +3 keeps the instant controller (no `enable_timing`) but gets the ready changes, aliasing,
per-unit PCN and Scan; its scenarios and `rc-suite` cases pass. **Later the same day**: the PRI compare moved into the CRTC's HSYNC edges
(latched at HSYNC start, fired at its end; an HSYNC running to the end of its line fires on the line before too), a PRI clears the Gate Array
counter's bit 5 before that counter takes the same HSYNC fall and taking the PRI's interrupt leaves the counter alone, and `nec765` gained DMA
mode (no `EXM`/`RQM`, reads end with data errors as on a 6128, Read ID still shows `EXM` while reading the ID). Eleven more tests pass:
pritest 17 of 20 (the trigger-bug, re-trigger and "can delay normal raster interrupt" cases), fdc_gen 27, fdc_read 14, fdc_write 13, fdc_seek
31 ("seek (wait for end without sense int), then read id", from Read ID's search/read split). Still open: pritest's "PRI start (R2)" (three
HSYNC positions, a VSYNC sample a few microseconds off), "enable norm during pri int handler" (reads an interrupt count left by the tests
before it) and "im2 vector (norm)" (one classic interrupt per frame in IM2 on a Plus, cause unknown); fdc_read's track-0 cases (the program
writes a deleted mark there, its tables expect a data mark); fdc_write's DMA write-deleted case (expects an overrun where write data expects
a normal end). **Gap sweep (2026-10-19, last)**: every remaining acid failure is now either fixed or explained in `BASELINE`'s notes
(`retroclawd-tools/crates/rc-suite/tests/cpc_acid.rs`). Fixed: a non-zero PRI write drops a pending raster request, a DMA prescale write
ends the pause unit in progress (dmatest 21 + 2 phase-dependent ones moved to `UNRELIABLE`, with the reason), the 765's first step
pulse goes out at once (fdc_seek 33), fdctest3 gets the frame budget its all-N loops need (it then ends in its own buffer overrun), and the
harness reads a doubled-letter verdict ("FAAIL"). Explained, not fixed (the evidence does not settle the mechanism; tried changes made
other cases worse): dmatest's R0 = 0 HSYNC case and channel 2 interrupt, pritest's three, fdc_seek's overlapped seeks on one drive and
lost steps, fdc_time's sub-millisecond timing, fdc_read/fdc_write's ready-change, data-error and FM cases, fdc_gen's spin-down case.
The last scenario known gap (CPC diagnostics' CRTC/FDC lines) is closed against Caprice32. `tools/check_all.sh` runs every check of every
repository; `tools/bump_pin.sh --push` runs it before pushing. Not emulator findings: cpctest's "GA I/O decode test" calls an uninitialised BC (its `ga_io_restore_extra` is never loaded) and resets the machine; asictest's "sprite ram mask" result buffer runs into the firmware's variables (in the harness's `UNRELIABLE`).

## Parked: checks that need real hardware (2026-10-17)

No real hardware is available, so these stay parked until someone has it; everything around them is unit-tested.
- **Dead keys and non-ASCII typing** (symbolic layout, `gui/fold.rs`, `LogicalKeys::pressed_dead`, paste folding): fed through
  `LogicalKeys` in tests only. To check: a layout with a dead key (Swedish, German, US-International), `--layout symbolic`, press the
  accent key then `e` (nothing from the dead key, then `e`), type `é` (types `e`), paste text with `é`/`ß` (`e`/`ss`).
- **Pad hotkey chords and pads in general** (`[hotkeys]` `pad1:select+start`, `[pads]`): tested with a Bevy `App` that spawns `Gamepad`
  entities; gilrs itself was never exercised. To check: a physical pad, bind `reset = ["f12", "pad:select+start"]`, hold Select then
  press Start, and confirm Select alone does not reach the machine as a joystick button once it is part of a chord.

## Sega Mega Drive / Genesis (2026-10-24): working, accuracy work continues

Done: the machine (68000 + Z80, VDP, YM2612, SN76489, I/O, pads), cartridges (mirroring, SSF2, battery SRAM, serial EEPROM boards,
Sonic & Knuckles lock-on, TMSS), interlace modes 1 and 2 (single field output), save states, the plugin and its debugger (two CPUs),
probes `p00`-`p50` (PAL V28/V30 DMA `p49*`, HBLANK flag `p50`) against Genesis Plus GX, PicoDrive, BlastEm and clownmdemu (and MAME as a tie-break for game logic), a manifest of
30 scenarios (one `known-gap`), a 30-game RomM sweep (no stops, no crashes) and a frame comparison against Genesis Plus GX
(`scenarios/batch_compare.py`: most games identical at most sampled frames, the rest differ in animation phase).

Frame offsets in title animations (Ecco, Aladdin, Sonic 3D Blast, Golden Axe at 600+ frames): analysed on Aladdin
(`scenarios/ramscan.py`, `mame_probe.py`). Our work RAM equals Genesis Plus GX's, one frame apart, for 60 frames; the offset then
changes by one at the end of the loading phase, where the game's wait for the vertical interrupt turns a lag of well under 1% into
a whole frame. The loops the loading phase spends its time in agree with Genesis Plus GX to 0.04% (`p44*`), refresh, FIFO and DMA
agree (`p22`, `p29`, `p34`); what is left is below what the probes resolve. MAME models none of the refresh, FIFO or Z80 contention
(its `p22`/`p29`/`p30` results show it), so it cannot arbitrate those; it agrees with Genesis Plus GX on the game logic.

Open, each with what it needs:
- **FIFO slots** are fitted to Genesis Plus GX (about 16 clocks). Mid-line CRAM/backdrop writes show from the pixel they happen at
  (fact V34, probe `p48`, compared with BlastEm; Genesis Plus GX does not do it). Other mid-line register effects, DMA slot
  interleave with the 68000 and pixel-level HBLANK flags are not modelled beyond the per-line rates.
- **Z80 access to the 68000's bus**: 10 CPU cycles lost by the 68000 (Genesis Plus GX 10.3 on average, BlastEm 8): `P30B` known gap.
- **Done 2026-10-29**: J-Cart, cheat codes (Game Genie decode, raw patches), mouse, 4-way play, Menacer, weave interlace, Sonic 2
  lock-on chip mechanism (no dump, untested), `--execution-mode fast|accurate`, tile-row picture cache (`docs/benchmarks.md`), fuzz
  tests for cheats and cartridge headers, PAL V30 DMA verification (V35), HBLANK flag check (V36). P30B: GPGX is the outlier (PicoDrive, BlastEm and here agree within 2 %), so it stays a gap. Sega CD: slices 1-3 done 2026-10-29 (disc crate, sub-CPU, gate array, CDD/CDC, graphics unit; real BIOS boots Dragon's Lair and Silpheed; backup RAM file persists), PCM and CD-DA audio and the backup RAM file work (Lunar and Shining Force CD run once formatted); RomM/zip discs unpack through `media_set.rs` (2026-10-30); open: Final Fight CD stays black (the sub-CPU waits at $857E for a flag only the sector routine at $4F80 sets), CHD, CD save states, see `docs/superpowers/specs/2026-10-29-sega-cd-design.md`. Not started: 32X, Pico, real Game Genie/Action Replay ROMs.
- **EEPROM**: tested against the data sheets' sequences and by booting NBA Jam (no oracle sets the lines; a game that saves to it
  was not played through).
- **Speed**: about 5.2x real time headless (`docs/benchmarks.md`); YM2612 is 12 % of it.

## Keymaps, typed characters and open reference files (2026-10-30)

**Done:** shipped keymaps (uk, us, es, de, fr, sv, no, da, us-c64) with Atari ST bindings; Spectrum `\ | { } ~ [ ] (c)` and corrected symbol
combos, CPC `~`; `ß Æ Œ` typed as two letters; the ISO key (`KC_NUBS`) bindable; the 8-bit Ataris need no per-language file (every ASCII
punctuation key is declared); zip/RomM discs (`media_set.rs`); `.prg` on a 6502 machine that declares `Program` (VIC-20) no longer goes to the
plugin as a program file; the VIC-20 manifest is closed (62/62) with goldens made by VICE 3.10 (hashes equal the pinned ones; recipe in
`retroclawd-tools/scripts/capture_vic20_goldens.py`, note `+virtualdev` is `+trapdevice` in 3.10).

**Open (needs third-party files, none of which is on this machine or on RomM):**
- `atari-800`, `atari-800xl`, `atari-65xe`, `atari-130xe` manifests: the Altirra replacement ROMs (`rom_os`, `rom_basic`; sources are the C arrays in the
  Atari800 7.2.1 sources, `machines/atari/firmware.toml` has the SHA-1s) and the Atari800 7.2.1 reference emulator for the S1/S2/S3/S7 goldens
  (`retroclawd-tools/scripts/capture_atari800_screen.py`, Windows only). The scenario media now build (`machines/atari/scenarios/build.sh`);
  the Acid800 ATR comes from `Emulators/Acid800-1.2`. Once the ROMs are in `machines/atari/roms/atarixl/`, upload them to RomM or copy them there.
- Not covered by the keymaps: dead keys and characters with no letter form (`€`, `¿`), non-US EmuTOS keyboard tables, accents typed as
  accents (a machine without them gets the plain letter).

## Atari ST: closed (2026-10-24); what is parked, and what each item needs

Phases 1 to 4 and the leftovers are done: ST, Mega ST and STE boot EmuTOS and the RomM games in both accuracy modes; every timing fact has a
probe or a source; the manifests are `atari-st` 32 passed / 14 known gaps, `atari-ste` 11 / 4, `atari-mega-st` 0 / 1 (all gaps are Hatari
differences with their evidence in the manifest rows). Since Phase 4: S17-S21, USART flags and break (MC68901 sheet), the RP5C15 and LMC1992
data sheets (clock, Microwire stream, silent power-up, bass/treble), mid-line resolution (P35, P45), the STE DAC filters, ACSI opcodes
`$05 $06 $0D $15`, the host ACSI save action, and the 1986 *Engineering Hardware Specification* (rows relabelled `[D]`). See
`machines/atari/CLAUDE.md` and `docs/hardware-facts.md`.

Parked (not planned; each says what would unblock it):
- **STE pad button bits and the STE video extras**: observed in Hatari only. Needs the Atari STE hardware register list (the STe service
  manual refers to it and does not contain it).
- **Empty cartridge slot level, and which unlisted I/O addresses bus-error** (`io.rs` stops the machine on them): the 1986 specification
  says only "bus error". Needs real-hardware knowledge or the GLUE's decode.
- **Sync mode reset value**: the specification says reset is all zeros (60 Hz); honouring it was tried and reverted on 2026-10-24 (breaks
  the Hatari boot-phase comparisons P04, P09/P10, the recorded fast-mode table and bare-metal tests, which assume a PAL machine starts at
  50 Hz). Revisit only with a real-hardware reference for a PAL power-on frame.
- **DMA status error bit versus the selected register** (specification note 841017G) and **the DMA FIFO's second 16 bytes**: no value or
  probe defines them.
- **Hatari's other wake-up states** (ws1, ws2, ws4; P04 differs): Hatari is the only oracle, no hardware source; each state needs its own fit.
- **The 14 known-gap probes** (P02 P04 P06 P09 P10 P12 P14 P16 P17 P18 P20 P21 P22 and P29/P30, P32/P34/P41 on the STE): Hatari's jitter,
  lost lines, its instant USART, its unchanged ACSI DMA address, its counter ramp and shift rate; P16/P17 would also need a motor-start hook
  in the shared `Floppy` trait. Looked at twice, none closable without a Hatari-specific fit.
- **A switch to high resolution inside the picture and lost lines** (P09 frames 16-18, 91-119): Hatari's own behaviour, not modelled.
- **ACSI details beyond the 1986 table** (INQUIRY/REQUEST SENSE/READ CAPACITY layouts, block count 0 = 256, error codes `$24`/`$25`): `[H?]`
  choices; needs the AHDI/ACSI specification pages that were poor scans.
- **LMC1992 mixer meaning** (the ST wiring of input select: YM at full level for 1, 12 dB down for 0) and **the board's tone capacitors**:
  `[H?]`; needs a schematic of the STE audio section (the Mega STE schematic in `docs/` is a TT-class board and was not mined further).
- **Not tried**: the STBook specification and the Mega STE schematic in `docs/`; `.stx` images; libFuzzer targets beyond their 60 s runs.

### Atari ST: Phase 3 done (kept for the record, 2026-10-23; superseded by the section above)

Phases 1 to 3 are done (spec section 24, Status 2026-10-21, 2026-10-22 and 2026-10-23). Phase 3 gave `--accuracy accurate` (every bus
cycle, wait states, cycle-exact Shifter with borders and mid-line palette, exact MFP/ACIA/IKBD/DMA/FDC timing, the mono monitor, the
`borders = on` canvas) and a timing oracle (probes run in Hatari 2.6.1 and here, `machines/atari/docs/timing-oracle.md`). Next: Phase 4
(spec section 5): the Blitter, the STE (its Shifter, DMA sound, joystick ports), the Mega ST clock, memory watchpoints (decision 1 of
Phase 2: wait for a need), `.stx` once a specification with a usable licence exists.
Deferred from Phase 3 (ledger, `Deferred minors`; the exit table with every `known-gap` probe is in the spec Status): the libFuzzer
targets were never run (nightly + cargo-fuzz: `msa`, `state_decode`, `prg`, `ahdi_fat16`, `rcasm68k`, `rcdis68k`, `accurate_regs`,
60 s each); Hatari's other wake-up states (only `ws3`); the YM2149's audio phase against the register write (no black-box reading);
picture-level comparisons of the border tricks and of mid-line resolution switches (P08 not probed on its own; Hatari's lost and
cut lines, P09/P10/P12); the 14 `known-gap` probes (interrupt placement to a few clocks, Hatari's own jitter and its instant USART and
unchanged ACSI DMA address); the host has no `--disk-save` for ACSI hard disks; the numbering of the probes follows the order written,
not the plan's table.
Open hardware facts (`machines/atari/docs/hardware-facts.md`, `[H?]` and `[O]` rows; Atari's hardware specification
is behind Cloudflare from here, download it by hand): the MMU's address multiplexing (modelled as DRAM row/column,
checked only through EmuTOS's sizing), RAM above the configured top, unused register bits, display-enable timing (Timer B input not connected; the video address counter stops the
machine), plane-to-colour bit order, the GPIP idle levels, the NTSC crystal (32.0424 vs 32.04245 MHz), the ACIA
clock. Known limit: fast mode advances the MFP to an instruction's start time on access.

## Current direction

**Per-family repositories and a shared-crates repository (2026-10-12, done).** Machine code left this repository:
`retroclawd-sinclair` (ZX81, ZX Spectrum), `retroclawd-amstrad` (CPC family) and `retroclawd-commodore` (C64, C128,
drives) each hold their machines, plugin libraries, chips/formats, ROMs and software with the full git history of
those paths. The crates more than one repository uses (`machine`, `plugin-api` + its ABI policy, `z80`, `mos6502`,
`ay-3-8910`, `nec765`, `dsk`, `zx-tape`) live together in `retroclawd-crates` (decision: keep them together rather
than scatter them across the families), checked out here as `shared/`; the app uses them by path, the families pin
a commit. The machine roadmaps moved with their repositories (their `TODO.md`). Follow-ups:
- **DONE (2026-10-12)**: cross-repository smoke test -- `tools/smoke_test.sh` builds everything (`tools/build_all.sh`),
  boots zx81, Spectrum 48/128, CPC464/6128, C64 and C128 headless with the real ROMs from the submodules and compares
  the boot-screen hashes with `tools/smoke/baseline.sha256` (`--update` rewrites the baseline on purpose); it also
  notes when a family pins a different `retroclawd-crates` commit than `shared/`.
  `.github/workflows/smoke.yml` runs it weekly/manually (secret `FAMILY_REPOS_TOKEN`). The baseline was recorded
  from a Linux debug build and passes unchanged on Linux release, Windows release and a GitHub runner, so the
  hashes do not depend on build profile or platform. Not covered: media loading, audio, GUI.
- **DONE (2026-10-12)**: pin-bump script -- `tools/bump_pin.sh` (push the new commit in `shared/`, run it; `--push`
  publishes the family commits): checks the commit is on GitHub, prints the plugin ABI/protocol constants and the
  shared-crate diff between the old and new rev, rewrites the `rev` in every family manifest and lockfile, runs each
  family's tests, commits, and records the new submodule commits here.
- **DONE (2026-10-12)**: PowerShell entry points (`tools\*.ps1`, run the bash scripts through Git for Windows);
  checked on Windows (build and smoke test).
- **DONE (2026-10-12)**: 6502 disassembly in the debugger. `mos6502::disassemble` (shared/) decodes all 256 opcodes and
  assembles one instruction; the C64/C128 plugins return it from `GetDisassembly` (the C128 follows its running CPU, 8502
  or Z80); the host's debugger had been showing `DB $xx` for *every* machine because `debugger.rs` disassembled with a
  stub instead of asking the machine plugin -- it now uses the plugin's disassembly, picks the assembler (Z80 or 6502)
  from the machine's CPU family, and steps over `JSR` as well as `CALL`/`RST`. Verified live through the GUI and the
  debug-ui HTTP API on a C64 (real KERNAL idle loop, assemble-and-poke).
- **DONE (2026-10-12)**: debugger follow-ups. (1) Z80 disassembly checked live on a ZX Spectrum (and ZX81/CPC). (2) Symbols:
  `machine::SymbolTable` annotates any CPU's disassembly text (`; NAME` after a `$1234`/`$12`/`0x1234` operand);
  `debug_symbols.rs` has built-in tables (C64/C128 KERNAL jump table, vectors, VIC/SID/CIA registers; Spectrum ROM entries
  and system variables) and `--symbols <file>` (repeatable; assembler listings, map files, VICE labels; path, URL or
  `zip#entry`) overrides them. (3) Z80 `JR`/`DJNZ` now show and accept the absolute target like 6502 branches
  (`z80::disassemble::disassemble_at`/`assemble_one_at`, applied by the host so the family plugins did not change). (4) The
  ZRCP `assemble` help is CPU-agnostic and its test host accepts 6502 as well as Z80. Verified live under Xvfb on a C64
  (`JSR $FFD2 ; CHROUT`, user label beating a built-in) and a Spectrum (`CALL 0x0556 ; LD_BYTES`, `LD A,(0x5C78) ; FRAMES`).
  Not done: symbols are not used on the *input* side (`assemble JSR CHROUT`), per-game symbol files in `--game-config`,
  and the Spectrum table is the 48K ROM's (on a 128K machine with ROM 0 paged in the names can be wrong).
- **DONE (2026-10-13)**: `retroclawd-tools` (<https://github.com/elpapimango/retroclawd-tools>, private) -- command-line
  tools on top of the crates. It is a **standalone workspace** (not a submodule here; this repository does not build,
  pin or smoke-test it) that pins `retroclawd-crates` by rev (and, with the media tools, the machine repositories'
  format crates); its CI needs the secret `FAMILY_REPOS_TOKEN` (read access to the four repositories). Done: `rcasm`
  (two-pass 6502/Z80 assembler, one syntax, flat/.prg/per-segment output, symbol file the debugger's `--symbols`
  reads, listing; the Z80 encoder is built by enumerating `z80::disassemble`, the 6502 reuses `assemble_one`, so there
  is no second opcode table -- every opcode of both CPUs round-trips) and `rcdis` (linear or flow disassembly, labels,
  symbol files, output that `rcasm` assembles back to the same bytes -- checked on random bytes and on the Spectrum,
  ZX81, CPC464, C64 KERNAL/BASIC and 1541 DOS ROMs). Programs built with `rcasm` were run in this emulator (C64 and
  Spectrum) and left the expected bytes in memory. `rcdisk` is done too (list, info, validate, extract, add, rm, rename,
  create, convert for `.d64/.d64-40/.x64/.d71/.d81`, CP/M `.dsk` in the CPCEMU layouts with AMSDOS and +3DOS headers,
  `.g64` read-only): it runs on the new generic CBM DOS operations in `cbm-dos` (commodore repo) and `dsk::cpm`
  (crates repo). Checked in the emulator: a real 1541 DOS loads files from disks `rcdisk` created, converted and
  edited, and a real AMSDOS on a CPC6128 lists and runs (`RUN"DISC"`) files from a `rcdisk`-made `.dsk`. `rctape` is
  done as well (list, info, validate, extract, cat with hex / BASIC listing, cut, rm, merge, split, cleanup, add,
  convert): ZX Spectrum `.tzx` is kept block by block (byte-exact round trip of every block kind; the playing time
  agrees with `zx-tape`'s player), `.tap` is converted to and from it, Commodore `.t64` is listed and extracted as
  `.prg` files and a Commodore `.tap` is described; tapes made by it load through the Spectrum ROM's own `LD-BYTES` in
  this emulator. `rcsnap` is done too (info, convert, dump, extract, create): ZX Spectrum `.z80`/`.sna`/`.szx` convert
  into each other (48K, 128K, +2A/+3) and say what the target format cannot keep, ZX81 `.p`/`.81`/`.p81` (with the
  name), CPC `.sna` (versions 1-3), C64 `.crt` and CPC Plus `.cpr` (list, extract chips / pages, plain cartridges
  from and to raw ROM images). Checked in this emulator: snapshots made by `rcsnap create` start in all three
  formats, 48K and 128K snapshots the emulator wrote convert to the other formats and load again, and an 8K `.crt`
  made from a raw ROM starts on the C64. `rcld` is done too (the last of the planned tools): a TOML layout names sources
  (assembled together), binaries, `.prg` files, symbol files, regions and reserved ranges; it checks overlaps, regions
  and holes by name and writes C64 `.prg` (with a `10 SYS` line), `.d64`, `.d81`, `.crt`; Spectrum `.tap`/`.tzx` (with
  a BASIC loader and an optional loading screen), `.sna`/`.z80`/`.szx`, +3 `.dsk`; CPC AMSDOS `.dsk`; flat `.bin`.
  Checked in this emulator: the Spectrum ROM's own `LD-BYTES` loads a tape `rcld` made, the C64 KERNAL loads its `.prg`
  from a `.d64`, a `.crt` and the snapshots start, and in the GUI the real BASIC loaders ran -- a C64 `.prg` with
  `--run` (`SYS` into the program), a Spectrum tape through `LOAD ""` / `CLEAR` / `LOAD ""CODE` / `RANDOMIZE USR`, a CPC
  disk with `RUN"HELLO`. Finding (fixed 2026-10-14, see below): the emulator started a Spectrum tape at power-on, so
  `LOAD ""` typed later missed the first block. Remaining ideas, none
  planned: `rcdis` reading a file out of a disk or tape image; in `rcdisk` writing `.g64` from a `.d64`, copying REL
  files, a BASIC tokenizer, zip/URL inputs; in `rctape` decoding a Commodore `.tap` (KERNAL / turbo loaders),
  `.wav/.csw/.pzx/.voc`, a BASIC tokenizer; in `rcsnap` 128K `create`, patching registers, ZX81 BASIC listing; in
  `rcld` CPC tape / ZX81 / `.cpr` output, 128K snapshots, separately assembled modules. The brief with the decisions is
  `docs/prompts/retroclawd-tools.md`; the tools' CI runs on every push (secret `FAMILY_REPOS_TOKEN`).
- **DONE (2026-10-14)**: `z80` executor/disassembler for `DD CB d op` with `op & 7` = 4 or 5 copied the result into
  IXH/IXL (IYH/IYL); the undocumented-Z80 literature (The Undocumented Z80 Documented 4.1) and FUSE copy into the real H / L.
  Fixed in both (`retroclawd-crates` `75cd75e`) with tests for the executor and the printed text; ZEXALL still passes.
  Found by `retroclawd-tools` while enumerating the disassembler.
- **DONE (2026-10-14)**: a Spectrum tape no longer starts at power-on. The ROM's keyboard scan reads port 0xFE from boot,
  which started every loaded tape at once, so a typed `LOAD ""` found its first blocks gone (found while checking
  `rcld`'s loaders live). Play is now pressed when execution reaches `LD-BYTES` (0x0556); `--tape-start any-read` brings
  the old rule back for a custom loader started from a snapshot. Checked live: a tape with no lead-in pause loaded after
  `LOAD ""` was typed 40 s after start. (`retroclawd-sinclair` `c31a779`, pins moved by `tools/bump_pin.sh`.)
- **DONE (2026-10-15), step 3 of the RomM plan**: `--firmware auto` / `[romm] firmware = "auto"` fills the ROM options nothing
  else names from the server's firmware by SHA-1, from a `firmware.toml` per family (`retroclawd-sinclair`/`-amstrad`/
  `-commodore`, staged as `<family>.firmware.toml` next to the libraries), with `retroclawd romm match [machine]` to see
  the result; downloads are hash-checked; entries may carry `when` conditions (drive ROM per `--drive-type`) and `parts`
  (the C128's BASIC halves). Not covered yet (the +2A/+3 64K ROM was added to RomM and the manifest on 2026-10-16): CPC+ firmware of its own,
  firmware that RomM stores as an ordinary ROM entry rather than firmware.
- **DONE (2026-10-14/15), steps 2 of the same plan**: `retroclawd romm ...`, `romm:<id>` sources, cache commands.
- **DONE (2026-10-14..16)**: RomM integration -- `retroclawd romm platforms|games|firmware|match`, `romm:<id>` /
  `romm:fw/<id>` sources (also in game-config `media`), `cache list|clean|remove`, `[romm] url, token` (env `ROMM_URL` /
  `ROMM_TOKEN` win), `--firmware auto`. Rules kept: the host contains no ROM or game data; the token is never in the
  repository, never printed or logged (read-only client API token; a separate token for CI); machines stay separate
  repositories because of possible takedowns. Details and what is still open: the cleanup entry below.
- **DONE (2026-10-15): cleanup before going public.** No ROM image or commercial software is in any repository
  or history any more. What was done, so the next session starts from facts:
  - Tests that read `roms/` / `software/` skip when the file is absent (the CPC tape tests use `software_tape()`);
    each family git-ignores `roms/` and `software/` for local copies and documents the files in `docs/firmware.md`.
    `retroclawd --firmware auto` fetches firmware from a RomM server by SHA-1 (`firmware.toml` per family).
  - Licence **GPL-3.0-or-later** (`LICENSE` in every repository, `license` in every manifest); the CRT shaders and the
    6502 / ZEXALL suites keep their own notices (`CREDITS.md`, fixture READMEs).
  - Removed: `docs/Altirra-4.40-src.7z` (the Atari 8-bit references are links below), the generated prompts, the old
    split scripts, the pre-split plans, `smoke.yml` (needs ROMs), the token-based CI clone steps, `zesinfo.json`.
    `retroclawd.toml` became `retroclawd.example.toml` (the real one is git-ignored).
  - **History: every repository was deleted and recreated from one fresh commit** (author: the GitHub noreply address).
    The old private histories (host, crates, the three families, tools) are kept as bare mirrors outside the repositories
    (`retroclawd-archive/`) and are not on GitHub. The families pin the new `retroclawd-crates` commit.
  - **Public since 2026-10-15** (order: crates, sinclair, amstrad, commodore, tools, host; each switched after its CI was
    green). Issues, wiki, projects and discussions are off; secret scanning and push protection are on (they are only
    available on public repositories). The CI workflows use no secrets. Actions are on Node 24 (`checkout@v6`,
    `upload-artifact@v6`) and run on `ubuntu-24.04` instead of `ubuntu-latest`. `tools/smoke_test.sh` with real ROMs
    matched the baseline hashes for all seven machines on the fresh build.
  - **Verified against the real RomM server (2026-10-16)**: `retroclawd romm match` resolves every `firmware.toml` entry
    (the c128 `parts` join and the drive-ROM `when` rules included; only the 32K C128 BASIC image is absent, the two
    halves are used), and `--firmware auto` boots zx81, zx-spectrum, zx-spectrum-128, cpc464, cpc6128, c64 and c128.
    `tools/smoke_test.sh --debug` passes for all seven (C64 with the real rev 3 KERNAL, C128 with the 318020-05 set);
    rerun on a fresh debug build of `025a2e6` (option check included): 7 of 7 ok, none skipped.
    Found on the way: `commodore/firmware.toml` named the options `rom_kernal`/`rom_basic`/`rom_char`/`rom_c64_*`, but
    the plugins declare the keys `kernal`/`basic`/`char`/`c64_basic`/`c64_kernal` (only `rom_1541` has the prefix), so
    `--firmware auto` filled nothing on the C64/C128 although `romm match` looked fine. Fixed in
    `retroclawd-commodore` `74f5b73` (pinned here). Also: the same dump can be listed twice on the server with one entry's file gone
    (`missing_from_fs`, content 404); `firmware_index` now skips such entries (it picked firmware 34 for the C128 KERNAL).
    `romm match` and `--firmware auto` now flag an entry whose `option` the plugin does not declare as a file option
    (`OPTION NOT DECLARED by the plugin (file options: ...)`, a stderr warning; unit test `an_entry_must_name_a_declared_file_option`).
    Not done: a check in each family's own CI against its plugins' profiles. Hashes not in any family: CPC+ own firmware.
  - **Work-laptop networking (2026-10-16)**: both HTTP clients (`romm`, `file-loader`) honour `HTTP_PROXY`/`HTTPS_PROXY`/
    `NO_PROXY` and now use the **system certificate store** (`rustls-tls-native-roots`), because Zscaler re-signs TLS and
    the bundled Mozilla roots failed with `invalid peer certificate: UnknownIssuer`. HTTP errors show their cause chain.
    Not tried behind a real Zscaler yet.
  - `tools/fetch_roms.sh` (`.ps1` wrapper): fills the git-ignored `machines/<family>/roms/` files the smoke test reads from
    the RomM firmware by SHA-1 (tries every listed entry of a hash, verifies before keeping). `git` history of the
    host was rewritten on 2026-10-15; an old clone needs `git fetch && git reset --hard origin/master`.
  - Next, in the owner's order: (1) the per-machine test suites incl. the ZX81 one; (2) the Atari 8-bit family.
    Everything needed is described below; nothing of them is started.
  - Environment notes: a cloud session's disk is small (the build needs ~17 GB); local sessions have `ROMM_URL` /
    `ROMM_TOKEN` in the Windows environment.
- **TODO (separate note, 2026-10-14; references added 2026-10-15)**: a full test suite per machine built with the
  `retroclawd-tools` programs: programs from `rcasm` / `rcld` in every output (`.prg`, `.d64`, `.d81`, `.crt`, `.tap`, `.tzx`,
  `.sna`, `.z80`, `.szx`, `.dsk`) started headlessly on each machine, disks / tapes / snapshots / cartridges made with
  `rcdisk` / `rctape` / `rcsnap` / `rcld` loaded and compared, boot screens per ROM revision, and the known CPU / chip test
  programs; firmware fetched from RomM by hash (`--firmware auto`). Runs where the firmware is available (locally, and in CI
  through the RomM token), skips otherwise. Existing diagnostic suites to use as models and, where their licence allows,
  as test inputs (assembled with our own tools, or run from their released images fetched from RomM):
  - ZX Spectrum: <https://github.com/brendanalford/zx-diagnostics> (ROM, RAM, ULA, contention, keyboard, tape, AY, 128K
    paging and +2A/+3 tests) -- one result screen per model as the expected output.
  - Amstrad CPC: <https://github.com/llopis/amstrad-diagnostics> (Gate Array, CRTC, PPI, PSG, FDC, keyboard).
  - Commodore 64: <https://github.com/crystalct/C64UnifDiagCart> (a diagnostic cartridge: RAM, ROM, VIC-II, SID, CIA, ports);
    also usable as a `.crt` test of our cartridge loader.
  Each suite is a list of (program, machine, option set, frames to run, expected screen hash or memory check); a failure
  names the area (CPU, video, sound, timers, I/O, media). The expected results come from the real hardware behaviour the
  diagnostic documents, not from our own output.
- **STARTED (2026-10-15): the suites live in `retroclawd-tools`, crate `rc-suite`** (`crates/rc-suite`): a harness that runs the
  real `retroclawd` headless (`--frames N` runs whole frames like the window) and checks memory, registers and the picture
  against cited hardware facts. A case for a missing feature is a *gap*: it must fail, and the run fails when it starts passing, so
  the list only shrinks on purpose. Run: `RETROCLAWD_BIN=... RETROCLAWD_MACHINE_PLUGIN_DIRS=... RC_ROMS=.../machines cargo test -p
  rc-suite -- --nocapture` (skips without them). **ZX81: 9 cases, all passing**: boot to the `K` prompt (sysvars and the glyph
  from the ROM's own character set), FAST mode at 3.25 MHz, SLOW mode `FRAMES` at 50 a second, ROM shadow and write protection,
  RAM mirrors, the echo area above `0x8000`, `--ram-kb`. Writing them found that the ZX81 had no memory mirrors, no RAM packs, no
  opcode-fetch echo and no `R`-driven interrupt (the ROM's display routine never ran; the old smoke baseline was a blank
  screen): all fixed, see the sinclair TODO. Still to do in the ZX81 suite: keyboard (needs a way to press keys headless), tape,
  `.p`/`.81` round trips, SLOW-mode effective speed (0.8046 MHz), RAM-pack boot per size.
  **CPC: 7 cases, all passing** (`tests/cpc.rs`): instructions in whole microseconds, the Gate Array interrupt every 52 lines
  (13312 T), RAM configurations C0-C7 on the 6128, lower ROM and the RAM under it, a disk `rcld` made and a 3000-byte file `rcdisk`
  added both read back by the firmware's AMSDOS (through `Resident`: boot, snapshot, patch code in, resume), and `.sna` versions
  1-3. No emulator bug was found. Not yet: CRTC types 0-4, video modes from screen RAM, `.cdt` (the tools make none for the CPC),
  upper-ROM selection (the ROM numbers could not be sourced), sound (not reachable headless).
  **C128: 12 cases, all passing** (`tests/c128.rs`): the BASIC 7.0 banner, MMU version register (`$20`), ROMs shown with the RAM under
  them, `$FF01` loading CR from PCRA, RAM banks 0 and 1, the 19656-cycle frame, 2 MHz mode (58 batches a frame at 1 MHz, 116 at 2), BASIC
  evaluating a typed `PRINT 12*34`, the C64-mode banner, and a file `rcdisk` put on a `.d64`/`.d71`/`.d81` loaded by a real emulated
  1541/1571/1581 through `LOAD"NAME",8,1`. No emulator bug was found. Not yet: the VDC (80 columns), the Z80 side, the 1571's MFM mode.
  Headless typing is new (`--type`, README), so the keyboard is now reachable: a keyboard-matrix case for the ZX81 and a
  `RUN"` case for the CPC can be written. **ZX81 keyboard: 3 more cases, all passing (12 in all)**: every one of the 39 typable keys
  (letters, digits, Space, `.`, Enter) and the 15 shifted symbols pulls exactly its matrix line(s) low (the positions are read from
  the ROM's own key tables at 0x007E and 0x00A5), and typed `PRINT 12`, `12*34`, `1.5+1` and `7-2*3` run in the ROM. This found that the
  ZX81's `.` key had no name and the shifted symbols (`+ - * / = ( ) : ; ? , " $ < >`) were not reachable, so BASIC could not be
  typed or pasted; both are fixed in the plugin (`Period` and the combo keys, see the sinclair CLAUDE.md), and `--type-delay` was added
  because the ROM drops keys typed faster than about 160 ms.
  **ZX81 (2026-10-16): 24 cases passing, no gaps**, after the keyboard ones: RAMTOP for `--ram-kb` 1/2/16/32/56 (4400h, 4800h, then
  8000h: ZXDOCS says the ROM's detection ends at 8000h), the first and last cell of the 16K/32K/56K machines are separate RAM (the
  56K one from 2000h), a hand-built `.p` (`1 PRINT "A"`, NXTLIN on the line) loads at 4009h and autostarts -- the `A` appears in
  the top-left cell, drawn from the ROM's own character set --, **saving a `.p` / `.81`** (found missing, built the same day:
  the file is memory 4009h..E_LINE-1, `.81` equals `.p`, a reload runs again) and all three ROM revisions in RomM boot to the
  same `K` prompt. `tools/fetch_roms.sh` fetches `zx81.rom` and `zx81b.rom` for it. **ZX81 tape** (built the same day, see below): `LOAD ""`, `SAVE "A"` and save-then-load through the real ROM. **Not done, deliberately:** SLOW-mode effective speed
  (ZXDOCS: 0.8046 MHz = 108 free lines x 149 cycles a frame) -- that figure is for BASIC running under the ROM's display
  routine; machine code run with `USR` in SLOW mode loses the display and only pays for the NMIs, so a machine-code loop is the
  wrong probe and a BASIC loop needs an oracle for how many cycles an interpreted loop costs. And the third-party diagnostics:
  none was sourced and licence-checked yet.
- **DONE (2026-10-16): ZX81 `.p` / `.81` save** (`retroclawd-sinclair`): `--dump-snapshot x.p` / `.81` and the save hotkey
  write memory from 4009h up to E_LINE-1; the suite checks the file against memory, `.81` against `.p`, and a reload. `.p81`
  is not saved (no program name; `rcsnap convert --name`). The host's save dialog already proposed `.p`.
- **DONE (2026-10-16): ZX81 tape** (`retroclawd-sinclair`, host: `--tape-save` accepts `.p` / `.81` / `.p81`, the tape dialog offers
  them): `--tape x.p` plays a `.p` / `.81` / `.p81` as the cassette signal (ZXDOCS: 150 us pulses, 4 = `0`, 9 = `1`, 1300 us
  between bits), `LOAD ""` reads it, `SAVE "A"` is recorded from the MIC output (the retrace latch) and decoded back into the
  file. Suite: 3 cases (load, save as `.p` and `.p81`, save-then-load through the ROM), all passing on the first run against the
  real ROM. Side effects worth knowing: an idle port `0xFE` read is now `0x7F` (bit 7 = tape input, 0 = normal; it was `0xFF`),
  and `OUT (0xFF)` no longer starts the NMI generator (`SAVE` writes it for every pulse). Open: a fast-load trap (a 16K program
  takes minutes at the real 307 bit/s), tape position in `.state` files, `.wav` / `.tzx` input, ZX80.
- **DONE (2026-10-16): flaky host tests fixed.** `inputs::tests::cleanup_removes_the_unzipped_files` asserted that the process-wide
  input loader holds no unpacked files right after `cleanup()`, which any parallel test unpacking a zip broke: the zip tests of
  `inputs::tests` now take one lock (`serial()`). `plugin_bridge`'s bogus-library test set `RETROCLAWD_MACHINE_PLUGIN_DIRS` while
  other tests could initialise the shared machine catalog (and another test removed the same variable): discovery now has
  `discover_runtime_catalog_from(candidates, explicit, searched_dirs)` and the test calls it with the file, no environment
  variable at all. 30 parallel runs of the 279 tests: 0 failures (about one in three failed before).
- **DONE (2026-10-16): suites for the ZX Spectrum and the C64, ZX81 follow-ups, firmware.toml check.**
  - `retroclawd-tools` `rc-suite`: **Spectrum 12 cases + 2 gaps** (`tests/spectrum.rs`: the ROM boots to the copyright text drawn
    from the ROM's own font, P_RAMT, FRAMES, ROM read-only / RAM from 0x4000, every key on its half row of port 0xFE, the border
    bits, the interleaved display file, contention slowing a loop, and on the 128K the eight RAM pages, the fixed pages 5 and 2,
    the ROM select bit and the paging lock) and **C64 9 cases** (`tests/c64.rs`: the banner, `PEEK(0)`/`PEEK(1)`, the whole
    LORAM/HIRAM/CHAREN bank-switching table, a PAL frame of 19656 cycles measured with a CIA timer between two raster line 0s,
    312 raster lines, the 60 Hz jiffy clock, typed BASIC, voice 3's oscillator read-back, a CIA timer counting cycles). Sources
    are the World of Spectrum 48K/128K references and c64-wiki pages, named in each case; the C128 suite now runs again because
    `tools/fetch_roms.sh` also fetches the 1541/1570/1571/1581 DOS ROMs.
  - Found by them: **Spectrum frame length is 70 938 T for every model** (48K should be 69 888, 128K 70 908): two gap cases and a
    TODO in the sinclair repository. **C64 `PEEK(1)` gave 247** (the 6510's missing bits 6-7 floated high), fixed in
    `retroclawd-commodore`. **A 16K ZX81 drew a blank screen** (`decode_rows` skipped every other row of an expanded display
    file), fixed in `retroclawd-sinclair`. Not bugs, noted so they are not rediscovered: the ROM sets `ERR_NR` itself, so it is not
    asserted; a 16K ZX81 needs ~2.4 s to reach its prompt; the C64 `--frames` batch is 19705 cycles but the VIC's own frame is
    19656 (checked with a timer), so only the host's pacing is 50.00 Hz.
  - ZX81 follow-ups: an opt-in **fast-load trap** (`--fast-tape-load`), the **tape position in `.state` files**, and **bit 6 of
    port 0xFE** as the 50/60 Hz flag (`--region ntsc` gets the ROM's 31-line margin). `.tzx`/`.pzx`/`.csw`/`.voc`/`.wav` input
    for the ZX81 followed (see the next entry).
  - **`firmware.toml` option check in every family's CI**: each plugin crate has the unit test
    `firmware_toml_names_only_file_options_the_plugin_declares` (it reads the family's `firmware.toml` and checks every `option`
    against the plugin's declared file options; mutation-checked with the old `rom_kernal` bug). A machine id in the manifest
    that no plugin serves is still not caught.
- **DONE (2026-10-16): the remaining queue -- 128K variants, the ZX80 layout, the C64 dead test, host gaps.**
  - Suite: every 128K-family ROM variant (128K, +2, 4.02, Derby 1.4) pages both 16K halves at 0x0000 exactly as the file holds them (the
    boot/frame-interrupt case already covered each); the C64 dead test (crystalct/C64UnifDiagCart's modified 781220 ROM, fetched by
    `tools/fetch_diagnostics.sh` from a pinned commit into the git-ignored folder -- the repository is CERN-OHL-W-2.0, the code inside
    Commodore's, so it is never committed or redistributed) boots from an Ultimax `.crt` and prints OK for the zero page and the stack
    page (about 50 s in a debug build). Its full cycle and the unified cartridge's mode switch were done on 2026-10-17 (below).
  - ZX80: `zx81-snapshot::parse_o`/`write_o` (`.o` = memory from 0x4000 up to `E_LINE` at offset 0x0A, no name; a file that disagrees
    with its own `E_LINE` is refused) and the ZX81 plugin now names a ZX80 program as such and refuses it ("no ZX80 is emulated"). A
    ZX80 machine needs its own 4K ROM (none is in RomM) and is a separate project.
  - Host (see CLAUDE.md "Per-run settings, per-game file names..."): screenshots/quick-save states named after the game, `--hotkey
    ACTION=CHORD`, `[keys]`/`layout`/`symbols` in `--game-config`, symbol names in the debugger's `assemble`, and a toast + hold when a
    plugin crashes. **Since (same day):** `--headless --frames N --at FRAME:ACTION[=FILE]` runs the hotkey/dialog actions headless
    (`scripted.rs`, reusing `HostControl::apply`; failures exit 1), and `SymbolTable::substitute` folds `NAME+1`/`NAME-$10` and
    `#<NAME`/`#>NAME`. **Still open:** hotkeys that need a window (`--at pause` etc. are refused), drag and drop, recent files, a second drive, pad buttons
    as hotkey triggers, dead keys/non-ASCII. (The live crash test was added afterwards: the stub panics on a `crash-now` key, run in a child copy of the test binary.)
- **DONE (2026-10-17): version info, the 6581 question and AmstradDiag's sound test.**
  - **Version info**: `retroclawd --version` (`-V`), the top of `--help`, `--list-plugins` and the overlay's `version` panel (hotkey `version`,
    default `shift+f9`; `--show-overlay ...:version`) show the version, **build number** (commits in the repository up to the one built from),
    git **commit** and its date, and `dirty` for uncommitted changes -- for the host and for each machine plugin, each from the repository it was
    built in. New shared crate `build-info` (stamps at build time, `version_tag!()`/`build_info!()`); a plugin carries the stamp in its
    descriptor's existing `version` string, so there is no ABI change and an older plugin shows its plain version. See the "Version info"
    paragraph of `CLAUDE.md`.
  - **6581 BAD in the standard diagnostic is the missing test plug, not the SID.** Its SID test (`$8CB0`) reads POTX and POTY five times and
    wants both in `$50-$8F`, which its test plug (paddle resistors) provides; open control ports read `$FF`, here and on a real C64. With a
    paddle on port 2 (`--mouse paddle-port2`, POT `$7F`) the chip table shows the 6581 OK; the 4066 analog switch that picks the port then
    reads BAD because port 1 still has no pot. Suite: two cases (BAD without a paddle, OK with one). Not done: an emulated diagnostic harness
    (loopback plugs for the cassette, serial, user and control ports) that would let the whole standard diagnostic pass.
  - **AmstradDiag's sound test** (repository head, `[T]`; not in the v1.3 release): `tools/build_amstradiag_head.sh` builds the disk from the pinned
    commit 7ac70fc with sjasmplus (the official Windows build is fetched and hash-checked when none is on PATH). On the CPC6128 it detects an
    "AY-3-891x" (register 1 keeps four bits, as an AY-3-8912), steps noise A, A+B, A+B+C and then the 110/131/165 Hz tones the same way
    and ends muted; the suite case measures every step in the `--dump-audio` capture (noise grows with channels and holds no tone, each tone
    appears exactly from its step) and is mutation-checked (PSG clock divisor).
- **DONE (2026-10-17): the unified diagnostic cartridge's mode switch, and the dead test's real length.** The board (crystalct's C64 Unified Diagnostic Cartridge) is the modified dead test in Ultimax mode plus a flip-flop: after its first cycle the dead test copies a routine to `$0334` that writes `$01` to `$DE00` and jumps to the KERNAL's reset routine, the flip-flop turns the cartridge into an ordinary 8K one, the KERNAL finds `CBM80` and the *standard* diagnostic (rev 586220++, first 8K of the 32K EPROM image) starts. Without that switch the emulator's `JMP $FCE2` landed inside the dead test's own ROM and ran garbage. New mapper `Mapper::UnifiedDiag` (`crates/machines/c64/src/cartridge.rs`), CRT hardware type `0xC640` (our own number: the board has no `.crt` type; chips: 8K at `$8000` = standard diagnostic, 8K at `$E000` = dead test; the header's EXROM/GAME levels = the JPE/JPG start mode), an I/O1 write sets the mode from data bit 0 (1 = 8K, 0 = Ultimax; read off the dead test's routine, there is no schematic here), a reset returns to the start mode, the mode rides in the save state. Tests: two unit tests, a real-KERNAL test with a synthetic Ultimax half and CBM80 half (mutation-checked: removing the `$DE00` handler fails it), and the suite case that runs the real 32K image (`tools/fetch_diagnostics.sh` now fetches `unified_32k.bin`). **What the real run shows** (30 s in a release build): the dead test finishes zero page, stack, screen RAM, colour RAM, the low-RAM test and the sound test by about frame 5000, the switch comes between frames 5000 and 6000, and by frame 8000 the standard diagnostic shows every RAM/ROM test OK, the three ROM checksums (KERNAL `B6C70A`, BASIC `043D56`, CHARAC `F3F7F8`), TIMER1/TIMER2 A and B OK and INTERRUPT OK; cassette, control port, serial port and user port read BAD/OPEN and the chip table flags the two 6526 and the 6581 BAD -- those tests want the diagnostic's test plugs (loopback connectors), so they are not asserted; the 6581 BAD turned out to be the missing plug too (2026-10-17, see the next entry). **Correction**: the earlier notes here, in `retroclawd-tools/CLAUDE.md` and in the commit messages said the dead test's main RAM test needs half an hour of machine time. That was wrong: Ultimax leaves only the low 4K to test, so a whole cycle is about 5000 frames. The long run I took of the dead test alone (a screen "stuck" at SCREEN RAM, blank screens, colour bars) was the CPU executing the dead test's ROM from `$FCE2` after the missing switch, not a RAM pass.
- **DONE (2026-10-17): the +3's second drive.** `ZxSpectrum::load_disk_in`/`eject_disk_from`/`disk_in`/`take_modified_disk_image_of`/
  `mark_disk_saved_in`, the save state carries drive B's modified image, and the plugin takes `MediaSlot::Disk { drive: 0 | 1 }` (two
  `DiskStatus` entries; a third drive is refused). Same shape as the CPC's, on the shared two-unit `nec765`. Tests: bus (reads A and B apart,
  independent modified/saved/restored), plugin (attach/export/detach/mark per drive, no drive 2, 48K refuses), and a host suite case that
  reads a sector from each drive through the FDC ports. The +3DOS ROM's own `DD_READ_SECTOR` was tried and not used: run without a
  boot it sits in its spin-up/seek delay loops.
- **DONE (2026-10-16, later): the open-items queue (1-6).**
  - AmstradDiag on a CPC664 (the 664 ROM is now in `fetch_roms.sh`): system information (model, FDC detected), lower RAM, ROM test (OS 664
    FD6F, 664 BASIC 1440, AMSDOS 0F91). A 464 cannot run the disk (no FDC); the cartridge needs a GX4000. The diagnostics' own sound test
    exists in the repo only after v1.3 (the release has no `[T]` entry), so instead the PSG is tested from the other side: eight CPC suite
    cases program the AY through the PPI exactly as that test's source does (periods `0238/01DD/017B`) and `--dump-audio` measures the result
    (each channel's frequency with sub-harmonic checks, chord, mixer mute, noise, volume, register 1 four bits like an AY-3-8912).
  - C64 dead test: now runs to frame 4000 and checks the screen RAM and colour RAM tests too, and that the main RAM test starts. (The
    "half an hour for a full RAM pass" written here was wrong, see the 2026-10-17 entry on the unified cartridge.)
    Mutation-checked (colour RAM bit stuck).
  - Host: pad chords in `[hotkeys]` (`pad1:select+start`), accent folding and dead keys in the symbolic layout and in paste, and a second
    drive: `--disk-b`/`--disk-b-save`, `load_disk_b`/`eject_disk_b`/`save_disk_b`, `nec765` with two units (shared), CPC plugin drive 1.
    Verified with the real 6128 firmware reading `B:` files. Per-game `[hotkeys]`/`[pads]` and the live plugin-crash test were done earlier.
  - **Still open:** the sound test of AmstradDiag HEAD (needs sjasmplus to
    build), drag and drop, recent files, the ZX80 machine (needs a ROM), Interface 1/zxpand ROMs (skipped).
- **DONE (2026-10-16): GX4000 diagnostics cartridge and the CPC diagnostics suite.**
  - Real bug: a CPC+/GX4000 ignored classic Gate Array ink writes (colours come from the ASIC palette only), so any program setting
    colours the old way showed black; `Asic::set_legacy_ink` fixes it. The earlier "cartridge parks in a VSYNC wait" diagnosis was wrong
    (it runs and programs the CRTC itself; the screen was black). Unit-tested and mutation-checked.
  - `tools/fetch_diagnostics.sh` also fetches amstrad-diagnostics v1.3's `.cpr` and `.dsk`. Five suite cases (`cpc_diagnostics`): the cartridge's
    menu and LOWER RAM PASSED on a GX4000, its keyboard test lighting exactly the typed keys (odd and even halves of the 48 unshifted
    keys, an independent check of the whole matrix), and on a CPC6128 disk its upper RAM test and its ROM test (lower ROM, BASIC and
    AMSDOS recognised by checksum). The menu works once the program has loaded; the earlier miss was typing too early.
  - **Then (same day): its soak and joystick tests.** Joystick (GX4000 cartridge, keyboard test): `--at FRAME:key_down|key_up=Joy1*` presses
    the stick and the program's own display paints what it read from matrix line 9 (two complementary runs light exactly the pressed
    cells of up/down/left/right/fire 1-3), and leaving the test holding fire 1 shows `JOYSTICK : PASSED` (cyan) once up, down, left, right and
    fire 1 were read, `FAILED` (error colour) when a direction never was. Soak (6128 disk): the title counts `SOAK ITERATION N`; two pictures
    of one run (`--at 2000:screenshot=FILE` and the final one) still show the banner, no error colour, and the counter moved on.
    Mutation-checked: swapped Up/Down bits and a moved Left bit fail the joystick cases, a stuck bit 7 in banks 4+ fails the soak (and the
    upper RAM case). Open: its sound test.
- **DONE (2026-10-16): CPC punctuation and typing, `.wav` input, the malformed `.cpr`.**
  - CPC: the keyboard matrix is complete (read from the firmware's key tables at ROM 0x1D69/0x1DB9, and tested against them): the
    punctuation keys, F0-F9, and 21 shifted-symbol combo keys, so `--type` and paste can type `"` and every other symbol into BASIC. Real
    bugs found: `,` and `.` were swapped, and Space also pressed joystick fire, which the firmware turns into an `X` (every typed space was
    "X "). A suite case types all the symbols into the real firmware and reads the string back.
  - `retroclawd-crates` `zx-tape::parse_wav` (PCM 8/16/24/32-bit and float, any rate, first channel, Schmitt trigger at 10% of the peak)
    and `.wav` tapes on the ZX81 (rescaled), ZX Spectrum and CPC plugins and in the host's tape dialogs; a ZX81 suite case records a
    program at 192 kHz and loads it through the ROM.
  - `cpc-cartridge::parse_lenient` repairs a wrong RIFF size field and a short last page (the released `AmstradDiag.cpr` has both) and
    reports it; `parse` stays strict. The CPC diagnostics disk boots and shows its menu (see the next entry for the suite cases).
- **DONE (2026-10-16): more 128K ROMs, ZX81 pulse tapes, Spectrum diagnostics.**
  - The `firmware.toml` 128K entry also accepts `plus2uk.rom`, `spec128uk_4.02.rom` and `spec128uk_derby1.4.rom` (after the preferred
    `128.rom`); a suite case checks that every cached 128K-family ROM boots with its frame interrupt running. `tools/fetch_roms.sh`
    fetches them.
  - `retroclawd-crates` `zx-tape`: `Tape::rescaled(from_hz, to_hz)`; the ZX81 plugin plays `.tzx` / `.pzx` / `.csw` / `.voc` rescaled to
    3.25 MHz (unit-tested per format; a suite `.tzx` case loads through the real ROM). The families' pins moved to include it.
  - Third-party diagnostics: `tools/fetch_diagnostics.sh` (pinned, SHA-256 checked, git-ignored cache) and a Spectrum suite case that runs
    zx-diagnostics v0.37's ROM and reads its documented pass colours (every test PASS, bright-green "48K RAM Tests Passed" bar). **Not
    done:** the C64 cartridge (the CPC diagnostics followed, see the entries above); see the tools `CLAUDE.md`.
- **DONE (2026-10-16): Spectrum frame length** -- every model ran a 70 938 T frame; now 69 888 (48K) and 70 908 (128K family) through
  per-model effective clocks (3 494 400 / 3 545 400 Hz at the host's 50 Hz tick), no trait or ABI change; the three gap cases closed.
  Details and the 0.16% / 0.04% wall-clock trade in the sinclair TODO.
- **DONE (2026-10-16): +2A/+3 and more ROMs.** RomM gained the +2A/+3 64K ROMs (`plus3.rom`, `plus2a.rom`), a +2 UK ROM, two more 128K
  variants, Interface 1 v1/v2 and the `zx81-550-kludge` ZX81 ROM. `retroclawd-sinclair` `firmware.toml` now lists the +2A/+3 ROM
  (`romm match zx-spectrum-2a3` and `--firmware auto` work; the +3 boots) and the kludge ZX81 ROM, which joins the suite's revision
  case (it boots to the same prompt). New suite cases for the +2A/+3 (`spectrum_plus3`, skipped without `plus3.rom`): the four
  special-paging all-RAM maps and the 2-bit ROM number, both matching the World of Spectrum 128K reference on the first run, and
  the shared 70908 T frame as a third gap (same cause as the 48K/128K ones). `tools/fetch_roms.sh` fetches `plus3.rom` and the
  kludge ROM. Not used yet: Interface 1 ROMs (no Interface 1 is emulated), the other 128K/+2 UK ROMs (could join the 128K manifest
  entry), `zxpand_zx81.rom`.
- **TODO (2026-10-15): ZX81 test suite**, separate from the diagnostics above because the ZX81 is the odd machine (no ULA
  interrupt, the CPU makes the picture): a program set that exercises every area of the emulator and checks it works --
  ROM boot to the stable loop with NMIs waking `HALT`; FAST and SLOW mode timing (frame length, NMI generator on and off,
  the display-file generation through the `R`/`I` trick and the 64K wrap of the video fetch); keyboard matrix (every key,
  shifted combinations, the `NEW`/`CLS` echo); `.p` and `.81` loading and saving (`rcsnap` / `rcld` made), tape
  in/out through the EAR/MIC bit; 1K/2K/16K/32K/56K RAM sizes and the memory map mirrors; RAM-pack and ROM-shadow
  behaviour; character set and inverse video; ZXPAND-style extras the emulator claims to support. Assemble the programs with
  `rcasm`/`rcld`, run headless, compare screens (`--dump-screen`) and memory (`--dump-memory`). Real-hardware facts go
  in as expected values with their source; our own output is never the oracle.
- **DONE (2026-10-14)**: the debugger plugins (`debug-ui`, `debug-zrcp`, `debug-log`) are ordinary modules of the
  application behind cargo features (`apps/retroclawd/src/debuggers/`); the debugger half of `plugin-api` is deleted and the
  machine plugin ABI (what separates the machines) is untouched. See CLAUDE.md "Debuggers are modules, not plugins".
- **BACKLOG**: this repository's history still contains the moved files (they were stripped from the tip only).

Pluginization baseline, diagnostic cleanup, and the CLI rework are complete -- see
"Plugin host and debugger plugin architecture" below for the mechanism.

1. Machines are shared libraries only (2026-10-12): the shipped `retroclawd` links no machine code
   and loads every machine from a plugin library -- see "DONE: machines are shared libraries".
2. Debugger plugins stay compiled into the app (with real library builds alongside).
3. Preserve plugin-level boot, media, state, debug, and input smoke coverage.
4. Resume machine-specific parity backlog (CPC/C64/ZX) -- see per-machine roadmap sections below.
5. Any new machine (Enterprise 64/128, MSX, Atari 400/800, or anything under "Future machine
   roadmaps" below) integrates through the plugin/debug/fast-path contract as-is -- see "Checklist:
   new machine plugin integration" under "Plugin host and debugger plugin architecture" below before
   starting one.

## Plugin host and debugger plugin architecture

### DONE: pluginization baseline

`retroclawd` is the host/orchestrator; machine and debugger capabilities are discovered through a
versioned C-ABI plugin boundary (`crates/plugin-api`); machines load as shared libraries, debugger
plugins are compiled in (and also built as libraries).
`AnyMachine` is gone -- CLI machine selection and lifecycle route entirely through plugin
discovery. Every active machine plugin (ZX81, ZX Spectrum, CPC, C64) exposes the full required
debug contract (registers/flags, memory, execution control, disassembly, media state, chip-level
snapshots, save-state parity) and declares its own creation-time options
(`plugin_api::MachineOptionInfo`, see `CLAUDE.md`'s `retroclawd` bullet). Browser and TCP/ZRCP
debuggers both run as `DebuggerPlugin`s over the same host debug API and
`DebuggerCommand`/`DebuggerState` vocabulary. `debug-log` owns all tracing output; no second
app-side host log sink exists (2026-10-01: `run_gui` still filtered `debug-log` out of the launch list
on the assumption a host-side sink existed, so `--plugin debug-log:...` never started -- no file, no
stdout; fixed, with a bridge-level test, plus an `engine_level` cap that keeps third-party shader/engine
chatter out of `level=debug` logs). The actual protocol/ABI contract lives in
`crates/plugin-api/src/{protocol.rs,abi.rs}` -- that source, not a draft, is now authoritative.

### DONE: debug-contract genericity cleanup (memory/architecture reassessment)

Prompted by a direct architecture reassessment: memory usage was unaffected by the plugin split
(bundled mode -- the default -- statically links every machine plugin into one binary regardless
of `--machine` selection, so a single-machine run carries every other machine's code either way;
only dynamic mode, an opt-in dev/deployment path, could reduce this), and debugging had become
harder because some flows bypassed the generic plugin contract with hardcoded, per-machine
special cases instead of extending it. Two concrete bypasses were found and fixed -- see
`CLAUDE.md`'s own "Debug-contract genericity cleanup" paragraph (under `retroclawd`'s ZRCP
section) for the full account:

1. CPC exposed a second, CPC-only `ChipSnapshot` (`"cpc.overlay"`) alongside the shared generic
   one (`"overlay"`), read by a host function requiring 5 specific field names all present (or it
   silently returned nothing) and gated to `machine_id.starts_with("cpc")` -- which meant
   `get-paging-state` always replied "unavailable" for ZX Spectrum 128K too, a real, live bug,
   despite its own help text and `machine_id` gate both claiming Spectrum support. Fixed by
   merging CPC's fields into its own `"overlay"` chip and making every host-side reader
   (`--debug`'s diagnostic log, ZRCP's `get-paging-state`/`get-io-ports`/`get-cpc-io`) format
   whatever fields are actually present generically, no fixed field-name list or `machine_id`
   gate.
2. The debugger's `PokeBank` command (banked/paged poke, the `.POK` trainer importer's main use)
   was pre-rejected by the host for any `machine_id` not starting with `"zx-spectrum"`, before
   ever asking the plugin -- while the other three plugins' own `WriteMemory` handlers silently
   discarded an unrecognized `space` and wrote flat memory anyway, meaning simply deleting the
   host gate would have made a banked poke on C64/ZX81/CPC write to the wrong place instead of
   erroring. Fixed both ends: those three plugins now explicitly reject a banked-write request
   they don't support, and the host gate is gone entirely -- the plugin's own answer is now the
   sole authority, delegated exactly like every other machine-specific `MachineCommand` behavior
   already is.

Explicitly not attempted in this pass (real, separate future work, not part of this cleanup):
real CPC banked-write support (`WriteMemory` with a `space`) for its own 128K/expansion RAM, and
building a general "closed contract" declared-capability mechanism for the *rest* of the
codebase's remaining machine-specific special cases (e.g. `debug-zrcp-plugin`'s own CPU-family/
clock-rate/keyboard-quoting `machine_id` branches) -- those are real, narrow facts about specific
machines (which CPU family, what a keyboard layout can type), not instances of a debug contract
that could have been generic but wasn't, so they were deliberately left as-is rather than
generalized for its own sake.

### DONE: machines are shared libraries (2026-10-12, supersedes the feature-gated bundling below)

Found when the Windows `*_machine_plugin.dll` files turned out to be ~10 KB: they were empty. The
plugin crates exported their entry point only when built `dynamic` and not `bundled`, and
`retroclawd` depended on them as `bundled`; cargo unifies features across a workspace build, so a
normal `cargo build --release` produced libraries with no exported entry (the linker then dropped all
the code), while the real emulation sat inside `retroclawd.exe`. Fixed structurally rather than by
flags: each plugin is now an ordinary library crate (`crates/plugins/<name>-plugin`, lib name
`<name>_plugin_impl`, no exported symbols, `entry_v1()` returns the descriptor) plus a thin
`crates/plugin-dlls/<id>` crate (`<name>-plugin-dll`, `crate-type = ["cdylib"]`, lib name
`<name>_plugin`) whose whole source is `plugin_api::export_plugin!(<impl>::entry_v1)`. The
`dynamic`/`bundled` features are gone. `retroclawd` no longer depends on any machine plugin (the
shipped binary contains no machine code: `nm` finds no `zx81::`/`c64::`/`cpc::`/`vic_ii` symbols) and
finds them at startup in `RETROCLAWD_MACHINE_PLUGIN_DIRS`, else `<exe dir>/plugins`, `<exe dir>` (where
`cargo build --workspace` leaves them) and `./plugins`; a missing machine is "unknown machine ...;
discovered machines: ...", and no libraries at all is an error naming the searched folders. Removing a
machine from a build or a release is deleting its library. (The debuggers were later made plain modules of the
app, 2026-10-14, and no longer have libraries.)
Unit tests of the app link the plugin implementation libraries directly (dev-dependencies; unit
tests never scan folders) and `a_built_machine_library_loads_on_its_own_and_lists_its_machines`
loads the real built `.so`/`.dll` when `cargo build --workspace` has produced it. The plugin screenshot
script builds the `-dll` packages and runs the four machines from them (`--verify` still needs the
missing `plugin-screen-baseline.sha256`). **Not done / honest limits**: the machine source still
lives in this repository (and its history) and the app crate still lists the machine plugins as
dev-dependencies for tests, so a takedown request would still need the source moved to its own
repository -- the next step is splitting each machine (plugin, machine crate, its formats, ROMs,
software) into a separate repo that builds only its library, with this repository keeping the host,
`plugin-api` and the shared cores/chips.

### SUPERSEDED: feature-gated bundled machine plugins (memory/architecture reassessment)

Companion fix from the same reassessment, addressing the memory half of the concern rather than
the debugging half above. Bundled mode (the default, and what actually ships -- dynamic loading is
opt-in, dev/deployment-only) previously linked all four machine plugins into one binary
unconditionally regardless of `--machine` selection, so a single-machine build was never actually
possible despite the bundled/dynamic split nominally existing to make that kind of choice
available. `apps/retroclawd/Cargo.toml` now gates each `*-machine-plugin` dependency behind its own
cargo feature (`machine-zx81`/`machine-zx-spectrum`/`machine-cpc`/`machine-c64`), all four still on
by default (a plain `cargo build` is unchanged); `plugin_bridge.rs`'s `bundled_machine_plugin_entries`
builds its `Vec` from four independently `#[cfg]`-gated pushes instead of a fixed 4-element array
unconditionally naming all four crates. Verified live, not just type-checked: `cargo build -p
retroclawd --no-default-features --features machine-c64` links and its `--list-machine`/`--help`
correctly show only `c64`; `cargo clippy`/`cargo test --workspace` both stay clean across the
default and several reduced feature combinations. See `CLAUDE.md`'s own "Feature-gated bundled
machine plugins" paragraph for the full account, including the honest caveat: this fixes binary
size/link time/"can I build just one machine" (previously not possible at all), not necessarily
single-machine runtime memory (RSS) -- unreached code mostly never gets paged in either way, and
Bevy's own footprint dominates regardless. Debugger plugins were left bundled unconditionally --
only the four machine plugins were the ones actually flagged as always-linked-regardless-of-
selection.

### DONE: fast-path step/halted/pc ABI bypass (speed/architecture reassessment)

Third fix from the same reassessment, addressing speed rather than memory or debuggability --
prompted directly by "I don't care about a huge binary in the end, I care that it's accurate,
fast, easy to debug and expandable." Every emulated CPU instruction was paying a full
`bincode`-encode + FFI call + `bincode`-decode round trip (`HostedMachine::step()` through
`instance_call`), even in bundled mode where no real process/address-space boundary exists to
justify one -- a real, measured cost, not a hypothetical. `MachinePluginVTableV1` gained 3 new
raw scalar-in/scalar-out fields (`step`/`halted`/`pc`, no serialization at all), gated safely by
an `ABI_MINOR_V1` bump (0 -> 1) since the host's major/minor check runs strictly before any
vtable memory access. Every machine plugin implements the 3 new functions against its own
existing instance registry; `plugin_bridge.rs` threads them straight through
`HostedMachine`/`HostedBackend`/`PluginV1MachineBackend`/`PluginHost` to the raw vtable call.
Deliberately scoped to only these 3 methods -- `clock_hz`/`frame_hz` are called once per tick,
`render_frame`/`drain_audio` are already `Vec<u8>`-returning calls at 50-60Hz -- see `CLAUDE.md`'s
own "Fast-path `step`/`halted`/`pc` ABI bypass" paragraph for the full account. The slower
`MachineCommand::Step`/`IsHalted`/`GetPc` protocol variants remain in place for `DebugCommand`-
driven paths (e.g. debugger step-over); this is an additional fast lane, not a replacement.

### DONE: plugin ABI hardening and compatibility policy (2026-10-12)

The written policy is `docs/plugin-abi.md` (four versions and what each means: ABI major/minor,
`PROTOCOL_VERSION`, profile contract; what is frozen; what a change requires; failure containment;
release layout and naming per platform). Enforced in code and tests, not just prose:
* `plugin_api::compat` -- the one place the host's accept/refuse rules live, used by the machine and
  debugger loaders: the descriptor's `abi_major`/`abi_minor` are read first (frozen offsets) so a
  descriptor of another major is refused without being interpreted; same major, plugin minor >= host
  minor, kind, non-null vtable, vtable `size` >= the host's.
* `codec::decode_message` checks the protocol version **before** decoding the payload (a skewed plugin
  is reported as "protocol version mismatch", never decoded against the wrong schema), and decoding is
  capped at 256 MiB so a mismatched length prefix cannot allocate the machine away. Plugins now answer
  codec errors with the readable text (`CodecError::into_call_result`).
* `plugin_api::guard` -- every exported function of all eight plugins runs under `catch_unwind`: a panic
  no longer aborts the process (verified live with a deliberate panic in a real `.so`: the run exited 0);
  the first panic text is kept, later calls return `InternalError`, and a crashed machine's `step`
  idles (4 cycles) / `halted` reads 1 so the host cannot spin. A crashed plugin stays dead until restart.
* Tests: `abi.rs` pins every offset/size on 64-bit; `tests/protocol_schema.rs` fingerprints
  `protocol.rs` against `PROTOCOL_VERSION` (mutation-checked: adding a constant fails it); host-side
  skew tests with stub plugins (newer minor + larger vtable loads, minor-0 refused without reading its
  short vtable, protocol skew reported, contract skew both ways).
* `--list-plugins` prints the host's ABI/protocol/contract numbers and each plugin's ABI.
* Release artifacts: `.github/scripts/package_release.sh [--debug]` builds the workspace and stages
  `retroclawd-<version>-<target>/{retroclawd[.exe], plugins/, PLUGINS.txt, SHA256SUMS}` (verifies the
  staged layout loads every library); `.github/workflows/release-artifacts.yml` runs it on the three
  platforms by hand. **Only the Linux path has been run**; the Windows/macOS branches are unverified.
* Not done: a host-side toast/log when a plugin crashes (today the panic message on stderr and the
  frozen machine are the only signs; `instance_call` errors do carry the text), and real-library skew
  tests (a fixture plugin built with a different ABI/protocol) -- the stub-vtable tests cover the
  host's decisions but not an actual second build.

### Checklist: new machine plugin integration (read first for Enterprise/MSX/Atari/any future machine)

Every future machine in this file (Enterprise 64/128, MSX family, Atari 400/800, and anything added
after them) integrates through the same plugin/debug contract the four current machines
(zx81/zx-spectrum/cpc/c64) already prove out, not through a bespoke path invented per machine. Read
this checklist before scoping a new machine's own roadmap section below, and treat each future
roadmap's "app integration"/"CLI" step as shorthand for the items here, not a separate mechanism:

1. **Own machine-plugin crate** under `crates/plugins/<name>-machine-plugin`, implementing
   `plugin_api::PluginDescriptorV1`/`MachinePluginVTableV1` directly -- copy the shape of any
   existing `*-machine-plugin/src/lib.rs`: a `state()`-backed `Mutex<PluginState>` instance
   registry (`next_instance`/`instances: HashMap<MachineInstanceHandle, InstanceRecord>`), the 5
   base vtable functions (`plugin_list_profiles_v1`/`plugin_create_instance_v1`/
   `plugin_destroy_instance_v1`/`plugin_instance_call_v1`/`plugin_free_buffer_v1`), and both
   `entry_v1` (the library exports no symbols; the `plugin-dlls` crate adds the C entry points,
   see item 6). Keep the actual machine (CPU/bus/chips)
   in its own `crates/machines/<name>` library crate that the plugin only wraps -- same
   "plugin crate is a thin host-facing adapter, the machine crate is the real implementation"
   split every current machine already has; do not fold machine logic into the plugin crate itself.
2. **The 3 fast-path vtable fields are mandatory, not optional**: `MachinePluginVTableV1` (ABI
   minor 1) requires `step`/`halted`/`pc` raw `extern "C" fn(instance) -> {u32,u8,u16}` fields with
   zero serialization -- the vtable struct literal will not compile without them, so this can't
   silently be skipped. Implement them exactly like every existing plugin does (`plugin_step_v1`/
   `plugin_halted_v1`/`plugin_pc_v1`, looking the instance up in the same registry
   `instance_call` already uses, returning `0`/`1`/`0` for a handle that somehow doesn't resolve).
   Never route a new machine's hot per-instruction loop back through `MachineCommand::Step` instead
   of these -- that per-instruction `bincode` round trip is exactly what the 2026-09-28 fast-path
   fix removed; see `CLAUDE.md`'s "Fast-path `step`/`halted`/`pc` ABI bypass" paragraph.
3. **Full required debug contract from day one**: `MachineProfile::debug_capabilities` must include
   every entry in `plugin_api::REQUIRED_MACHINE_DEBUG_CAPABILITIES` (registers/flags, memory,
   execution control, disassembly, media state, chip-level snapshots, save-state parity) -- a
   partial contract silently discovered later is exactly what the 2026-09-28 debug-contract
   genericity cleanup (see `CLAUDE.md`) had to fix for two existing machines; don't reintroduce
   the same gap for a new one.
4. **One generic `"overlay"` `ChipSnapshot`, never a second machine-specific chip id**: whatever
   diagnostic fields a new machine wants surfaced in `--show-overlay`/`--debug`/ZRCP's
   `get-paging-state` go on the shared `chip_id: "overlay"` snapshot as plain `NamedValue`
   key/value pairs (see `cpc-machine-plugin::overlay_chip_snapshot` for the richest current
   example, folding PPI/keyboard/PSG/tape/disk telemetry into that one chip). A second, bespoke
   `chip_id` the host needs its own reader for is the exact mistake found and fixed for CPC's old
   `"cpc.overlay"` chip (see `CLAUDE.md`) -- don't reintroduce it for Enterprise's `nick`/`dave`
   state, MSX's VDP/PSG/slot state, or Atari's ANTIC/GTIA/POKEY state.
5. **`WriteMemory`'s `space` field**: if the new machine has no real debugger-addressable banked-
   memory concept yet, its handler must explicitly reject a `Some(space)` request with a clear
   error, the same way `zx81`/`c64`-machine-plugin already do (CPC and ZX Spectrum implement
   `ram-bank:N` now) -- never silently fall through
   to a flat write, which would poke the wrong bank/segment/slot. Only add real banked-write
   support (Enterprise paging segments, MSX slot/subslot addressing, Atari PORTB/cartridge banking)
   once an actual debugger use case needs it, mirroring ZX Spectrum 128K's own `PokeBank` support --
   the host-side gate for this is generic already (see the same cleanup), so a new plugin needs no
   host change to add real support later, only its own handler.
6. **A shared library of its own**: add a `crates/plugin-dlls/<id>` crate (copy any existing one: a
   `cdylib` whose only source is `plugin_api::export_plugin!(<name>_machine_plugin_impl::entry_v1)`,
   lib name `<name>_machine_plugin` so the file name matches the host's `*machine*plugin*` scan) and
   list it in the workspace `members`. Do **not** add the machine to `apps/retroclawd/Cargo.toml`
   (only as a dev-dependency for unit tests, if wanted): the app finds it as a library at startup.
7. **Options via `MachineOptionInfo`, not new CLI flags or `MachineCommand` variants**: every
   ROM/RAM/BIOS input is a declared `File`/`FileSet` option (read from disk by the host into a
   `BinaryBlob`); every scalar knob (region, RAM size, VDP/paging variant, etc.) is
   `Bool`/`Int`/`Text`/`Enum`, sent once in `MachineCreateRequest::options` at creation time --
   never a new post-creation `MachineCommand::Set*` variant (that whole pattern was deleted from
   `zx-spectrum`/`cpc` in favor of this one, see `CLAUDE.md`'s "Machine-owned options" paragraph).
8. **Own format crate per real file format, no shared `Loader`/`Snapshot` trait** -- an Enterprise
   `.dtf`, MSX `.cas`, or Atari `.atr`/`.xfd`/`.cas` parser is its own crate under
   `crates/formats/`, same as `d64`/`d71`/`d81`/`dsk`/`zx-tape` already are; do not invent a shared
   trait across them from only one or two future data points.
9. **Save states**: bincode-derive a private `SaveState` the same shape `c64`/`cpc` already use --
   a direct serde derive for anything with no external-media entanglement, a small mirror struct
   plus `dump_state`/`restore_state` for anything wrapping external media (a loaded tape/disk) --
   excluding ROM/disk/tape bytes exactly like every existing machine's save state already does.
10. **Verify live through the real binary before marking any milestone done**, not just unit
    tests: `--headless --dump-screen`/`--dump-memory` boot proof at minimum, the same bar every
    current machine met, plus whatever media/input proof that machine's own roadmap step calls
    for.

## Input, hotkeys and file dialogs roadmap (design agreed 2026-10-10; all five phases done)

Replaces the thin `[keymap.default]`/`[keymap.machines.<id>]` host-key-to-machine-key map and the
hard-coded F11/F12 with a layered input configuration in `retroclawd.toml`. References: Caprice32
(`kbd_layout=keymap_us.map`, lines `CPC_A<tab>SDLK_a<tab>[MOD_PC_SHIFT]`, global only), RetroArch
(hotkeys separate from core input, optional hotkey-enable button, `.rmp` remaps scoped core/folder/game),
VICE (`.vkm` `keysym row col shiftflag`, positional vs symbolic variants, a map patches the current
one, `!CLEAR`). Three separate concerns, each layered CLI > `--game-config` > `[machines.<id>.*]` >
global > built-in defaults, per-binding override, `"none"` unbinds:

- **Decisions made:** per-machine bindings are sub-tables (`[machines.<id>.keys]`,
  `[machines.<id>.hotkeys]`) beside the flat `[machines.<id>]`; hotkeys are plain keys by default with an
  optional `hotkeys.modifier` (hold-to-enable, for machines that use every F-key); key maps are inline
  in `retroclawd.toml` only (no external keymap files yet); gamepads come after phases 1-2.
- Today's gaps: unknown machine key names pass through silently (`canonical_machine_key_name`'s
  `other => other`), the `[keymap]` comment in the committed `retroclawd.toml` still says "reserved",
  no chords/modifiers, no unbind, joystick aliasing (arrows+Space) is hard-coded per machine plugin.

1. **Hotkeys and actions (phase 1) -- DONE (2026-10-10).** `[hotkeys]` / `[machines.<id>.hotkeys]`,
   chords, hold `modifier`, `--list-hotkeys [machine]`; actions `reset`, `fullscreen`, `mute`,
   `volume_up`/`volume_down`, `overlay` (runtime toggle), `pause`, `screenshot`, `quick_save`/`quick_load`
   with `state_slot_next`/`state_slot_prev`, `fast_forward` (hold, 8x, renders every 8th tick),
   `shader_next`/`shader_prev` (live WGSL swap), `quit`, plus one-line toast messages. Details and the
   defaults rationale in `CLAUDE.md` ("Hotkeys"). Verified live through the real GUI under Xvfb (overlay,
   mute toast, pause, screenshot file, state save/slots/load, shader swap, reset, quit, hold modifier).
   Not done: hotkeys under `--headless`, a CLI override of the table, per-game state file names, volume
   shown anywhere but the toast, "turbo tape" as a separate runtime tape-speed action (fast-forward covers
   loading time).
2. **Keyboard remap (phase 2) -- DONE (2026-10-10).** `[keys]` / `[machines.<id>.keys]` replace `[keymap.*]`
   (clean cutover, old section is a clear error), host-side chords with modifier consumption, lists and
   `"none"`, startup validation against plugin-reported key names (`MachineProfile::key_names`, protocol
   v7; the plugins build the list from the tables their handlers use), `--list-keys <machine>`; the host
   lost its hard-coded per-machine name table. Details in `CLAUDE.md` ("Key bindings"). Not done: modifier
   keys cannot be the target of chord bindings, no per-game `[keys]` in `--game-config`, no CLI override.
3. **File dialogs with `rfd` (phase 4) -- DONE (2026-10-10).** Hotkey actions `load_tape`, `load_disk`,
   `eject_tape`, `eject_disk`, `load_snapshot`, `save_snapshot`, `load_state`, `save_state`, `save_tape`,
   `save_disk`; the machine pauses (silent) while a dialog is open; last folder per file type in
   `retroclawd.state.toml` next to the config; XDG portal backend on Linux with in-app confirmation (the
   portal has no message boxes) -- ejecting or swapping a modified disk asks Save As / overwrite original /
   discard / cancel, a swapped-in disk keeps the never-silent exit policy. Eject uses the existing
   `MachineCommand::DetachMedia` (no protocol bump), implemented in the CPC, ZX Spectrum and C64 plugins.
   Details in `CLAUDE.md` ("File dialogs"). Verified live up to the dialog itself (the prompt, cancel,
   discard, no hang without a portal); **an actual native dialog window was never seen** (no desktop portal
   in the sandbox), nor macOS/Windows. Not done: dialogs under `--headless`, drag and drop, recent-files
   list, a second drive (only drive 0 exists), per-game folders.
4. **Joystick and gamepad layer (phase 3) -- DONE (2026-10-10).** `[pads]` / `[machines.<id>.pads]`
   (`[pads.N]` per pad slot, `deadzone`), pad buttons -> machine key names through the same refcounted router
   as the keyboard, defaults pad N -> the machine's `Joy{N}*` keys, `Joy1*` names added to the Spectrum plugin
   and a real joystick on both C64 control ports, `--list-keys` prints the buttons and bindings. Differs from
   the original sketch: the arrow-key/Space joystick aliases in the Spectrum and CPC plugins were *kept*
   (replacing them with host-side defaults would have needed per-machine cursor-key knowledge in the host) and
   there was no plugin-api bump (pads reuse `KeyEvent`). Verified by unit tests including a Bevy `App` test
   spawning `Gamepad` entities; **never run with a physical pad** (no `/dev/uinput`/evdev in the sandbox).
   Not done: rumble. (Pad hotkey chords since 2026-10-16; sticks and analog triggers drive machine axes since
   2026-10-19, `[pads.N] left_x = "Joy1X"` -- no current machine reports an axis yet.)
5. **Layouts and keymap files (phase 5) -- DONE (2026-10-10).** `[input] layout = "positional" | "symbolic"`
   (also `[machines.<id>.input]` and `--layout`), keymap files (`[input] keymap`, repeatable `--keymap`;
   `[keys]`/`[machines.<id>.keys]` plus an optional `layout`), layered below/above the inline `[keys]`.
   Symbolic works from the characters each machine plugin declares its keys type (protocol v8,
   `MachineKeyInfo::chars`) plus letters/digits by identity; the C64 plugin gained the shifted-legend keys
   (verified against the KERNAL's own shift decode table). Details in `CLAUDE.md` ("Keyboard layouts and
   keymap files"). **Verified live** under Xvfb by re-mapping the X keymap so the QWERTY-`Y` position types
   `z` (a German keyboard): positional sent `KeyY` (differs), symbolic sent `KeyZ` and the screen was
   pixel-identical to a US-layout run typing `z`. Not done: dead keys and non-ASCII characters, the CPC
   punctuation row (the plugin maps only `, . /` so symbolic helps letters/digits there), ZX81 punctuation,
   per-game layout in `--game-config`, VICE-style matrix-position keymap files, a real non-US keyboard.

## Validation priority

### DONE: external 6502 validation

`mos6502` passes Klaus Dormann's documented-opcode functional test and Tom Seddon's exhaustive
NMOS decimal/illegal-opcode suite. Fixtures include matching source, licenses, pinned revisions,
checksums, and execution instructions under `crates/core/6502/tests/fixtures/`.

The external suite found and fixed two real bugs: decimal `ADC` derived N/V from the wrong
intermediate result, and decimal-mode `ARR` omitted NMOS nibble corrections. Focused regression
tests preserve both cases. The chip-dependent unstable opcodes still use deterministic common-case
formulas; no single result can represent every physical chip and operating condition.

### BACKLOG: machine boot integration checks

Add deterministic machine-level checks only when they defend a real boot invariant rather than
retesting plumbing. Execute in this order:

1. ZX81 ROM reaches its stable interactive loop while NMIs continue waking `HALT`.
2. Each ZX Spectrum model reaches its expected ROM screen.
3. CPC 464/664/6128 reach their model-specific firmware `Ready` screens.
4. C64 reaches the BASIC `READY.` prompt.

Keep ROM licensing constraints in mind; tests requiring private ROMs cannot be mandatory for public
CI.

## Debugger and application UX

### DONE: debugger capabilities

Completed debugger features:

1. **DONE** Reverse debugging and instruction-history backstep navigation.
2. **DONE** Watch expressions and conditional breakpoints.
3. **DONE** Step-over using temporary breakpoints and decoded instruction flow.
4. **DONE** `.POK` import through existing address-space and banked poke commands.
5. **DONE** Typed register editing for Z80 and 6502 plugin machines.
6. **DONE** Bitmap Explorer machine palettes and per-machine defaults from plugin metadata.
7. **DONE** ZRCP transport for DeZog-oriented workflows.

### DONE: keyboard mapping and input coverage

Completed:

1. **DONE** Typed stable host key names with per-machine keymap overrides.
2. **DONE** ZX Spectrum punctuation, Caps Shift, and Symbol Shift mappings.
3. **DONE** C64 punctuation, semantic keys, and shifted function keys.
4. **DONE** Preserve F11 fullscreen and F12 reset as host controls.

### BACKLOG: host features (added 2026-10-12)

1. **DONE (2026-10-12) Clipboard paste into the emulator window** -- `paste` hotkey (default
   `shift+insert`), `apps/retroclawd/src/gui/paste.rs`, see `CLAUDE.md`. Verified live under Xvfb
   (a real X11 clipboard, Shift+Insert injected through XTest) typing a three-line BASIC program
   into the C64 that then ran. Still open: the Windows/macOS clipboard paths and Wayland were not
   run (arboard supports them; only the text -> key conversion is unit-tested for those), the
   in-app prompt line does not accept paste, there is no size cap beyond cancelling with the hotkey
   again, characters a machine cannot type are counted and shown in the toast but not listed, and
   cancel-on-real-key was not added (only the second hotkey press cancels).
2. **ROMM integration** (<https://github.com/rommapp/romm>, a self-hosted ROM manager with a REST
   API). Direction not decided -- ask the user before designing. Candidate shapes to put to them:
   (a) a read-only client: configure `[romm] url/token`, `retroclawd --romm-list <platform>` and a
   `--romm <id>` launch that downloads the ROM/media through the API into a cache and starts the
   matching machine; (b) a platform-slug map (ROMM platform slugs -> `machine_id` + media kind, e.g.
   `zxs`, `acpc`, `c64`) so ROMM's library knows which retroclawd profile plays a title; (c) hash
   matching (ROMM stores MD5/SHA1/CRC) against the media retroclawd was given; (d) sync of save states,
   screenshots and playtime back to ROMM saves/states endpoints; (e) ROMM launching retroclawd
   through a custom-protocol/CLI link. Whatever is chosen: tokens come from the config/env, never
   the command line; downloads go through the same media loading as local files; nothing in the
   emulator core depends on the network.

### BACKLOG: headless interrupt-driven execution

Preserve the current default (`HALT` as completion) for standalone test programs. If this backlog is
implemented, execute in this order:

1. Add an explicit opt-in mode that continues through `HALT` until `--max-cycles`.
2. Keep existing batch semantics unchanged when that mode is not enabled.

## Cross-platform targets

### PARKED: native CI

Parked. Resume when release automation, target toolchains, and platform runners are available. Do not
claim cross-platform runtime support from local Windows verification alone.

### BACKLOG: browser/WASM frontend

Do not compile the native CLI unchanged for WASM. Execute in this order:

1. Add a separate app under `apps/` that reuses the machine crates.
2. Provide:
   - `requestAnimationFrame`-driven pacing instead of blocking sleep.
   - Browser file selection/fetch for ROMs and media instead of `std::fs` and CLI arguments.
   - Web-compatible audio, input, debugger access, and save downloads.
   - A `wasm32-unknown-unknown` build check.
3. Factor a shared runner library only when both native and browser frontends prove the common API.

## Future machine roadmaps — PARKED

Do not generalize current APIs in preparation for these systems -- the plugin/debug/options
contract these roadmaps integrate through already exists and is already generic (see "Checklist:
new machine plugin integration" under "Plugin host and debugger plugin architecture" above); a
future machine's job is to *use* that contract correctly, not to reshape it further in advance.

## Enterprise 64/128 roadmap — PARKED

### DONE: baseline

No Enterprise machine is implemented in retroclawd yet.

### BACKLOG: external parity execution sequence (PARKED; gap vs ep128emu/libretro)

Primary source material:

- https://docs.libretro.com/library/ep128emu/
- https://github.com/libretro/ep128emu-core
- https://github.com/istvan-v/ep128emu
- https://github.com/libretro/ep128emu-core/blob/master/core/sample.ep128cfg

### BACKLOG: core parity track — Enterprise 64/128 one-go implementation

Strict sequence for a later one-pass implementation (do not reorder; read together with "Checklist:
new machine plugin integration" above -- items below that say "app integration"/"CLI" mean that
checklist's items 6-7, not a separate mechanism):

1. Scope lock: start with Enterprise 128 only. Treat TVC/CPC/ZX modes in ep128emu as reference
   sources, not part of this implementation pass.
2. Crate layout: add `crates/machines/enterprise` for machine wiring (checklist item 1) and keep new
   chip behavior in dedicated crates only where there is clear chip-level reuse (`nick` video,
   `dave` system/audio/IO); add `crates/plugins/enterprise-machine-plugin` as the thin C-ABI
   adapter over it (checklist item 1), including its own `step`/`halted`/`pc` fast-path vtable
   functions from the start (checklist item 2) and its `machine-enterprise` Cargo feature
   (checklist item 6).
3. Machine construction and memory map: implement Enterprise 128 paging model, default RAM/ROM
   wiring, and deterministic reset/boot path from externally supplied ROM(s).
4. Bus/timing foundation: implement Z80 wait-state and memory-contention behavior needed to keep CPU
   execution synchronized with Enterprise video timing.
5. Video baseline (`nick`): implement the minimum documented modes and timing needed for real ROM boot
   screens and stable frame output.
6. System/audio baseline (`dave`): implement timer/interrupt plumbing, keyboard+joystick port wiring,
   and baseline audio output path compatible with existing GUI audio delivery.
7. Input mapping: add Enterprise keyboard matrix mapping and joystick mappings (internal + external)
   plugin host dispatch without regressing existing machine hotkeys.
8. Tape/media phase A: add Enterprise tape loading path (`.tap`, optional WAV-as-tape semantics,
   own `crates/formats/` crate per checklist item 8) with real-time behavior and debug diagnostics
   matching existing tape observability style (the generic `"overlay"` chip, checklist item 4).
9. Disk/media phase B: evaluate and implement Enterprise disk/hard-image attach workflow (`.img`,
   `.dsk`, `.vhd`, own format crate(s) per checklist item 8) with explicit read/write policy and
   persistence semantics.
10. Enterprise-native convenience media: evaluate `.dtf` and direct executable file startup
    (`.bas/.com/.trn/.128`) only after base boot/media paths are stable.
11. Save states: add machine-specific internal save-state coverage for Enterprise paging, timers,
    video/audio state, and attached-media position metadata (media bytes remain external; shape
    per checklist item 9).
12. App integration: declare Enterprise's `MachineProfile`/`MachineOptionInfo` options (ROM
    file(s), any Enterprise-specific scalar knobs) and full `REQUIRED_MACHINE_DEBUG_CAPABILITIES`
    contract per checklist items 3/7 -- `retroclawd enterprise128` needs no bespoke CLI code, only
    a correctly declared profile.
13. Verification matrix: require boot-to-ROM prompt proof, keyboard input proof, tape load proof,
    disk attach proof, and save-state round-trip proof before marking support done, all via the
    real binary (checklist item 10).
14. Cross-machine follow-through: when Enterprise-driven work touches CPC or ZX logic, update the CPC
    and ZX emulator-comparison checklists in this TODO during the same change.
15. Documentation/citations: add exact references used for each accepted behavior to `CREDITS.md` and
    keep TODO sequence numbering aligned with delivered milestones.

### BACKLOG: parity extensions (PARKED)

After Enterprise 128 baseline is complete, evaluate follow-up scope in this order: TVC support,
Enterprise expansions, and only then overlap features for CPC/ZX modes that belong in their own
machine roadmaps.

### BACKLOG: cross-emulator comparison addendum (PARKED; ep128emu as additional source)

When this roadmap is resumed, keep an explicit feature-delta checklist against ep128emu/libretro and
require hardware-backed behavior or executable tests whenever sources disagree.

## MSX family roadmap — PARKED

### DONE: baseline

No MSX-family machine is implemented in retroclawd yet.

### BACKLOG: external parity execution sequence (PARKED; gap vs blueMSX/libretro)

Primary source material:

- https://docs.libretro.com/library/bluemsx/
- https://github.com/libretro/blueMSX-libretro
- https://sourceforge.net/projects/bluemsx/files/
- https://sourceforge.net/projects/bluemsx/

### BACKLOG: core parity track — MSX family one-go implementation

Strict sequence for a later one-pass implementation (do not reorder; read together with "Checklist:
new machine plugin integration" above -- items below that say "app integration"/"CLI" mean that
checklist's items 6-7, not a separate mechanism):

1. Scope lock: start with MSX1 and MSX2 baseline compatibility on Z80. Keep non-MSX platforms in
   blueMSX docs (SVI/Coleco/SG-1000) out of this pass.
2. Crate layout: add `crates/machines/msx` for machine wiring and keep reusable chips in dedicated
   crates when there is clear second-consumer value (`tms99xx` VDP family, PSG reuse where
   possible); add `crates/plugins/msx-machine-plugin` as the thin C-ABI adapter over it (checklist
   item 1), including its own `step`/`halted`/`pc` fast-path vtable functions from the start
   (checklist item 2) and its `machine-msx` Cargo feature (checklist item 6).
3. Machine profile model: define explicit machine profiles (MSX1, MSX2, MSX2+) with ROM slot map,
   RAM sizing, VDP selection, and regional timing -- as real `MachineProfile`/`MachineOptionInfo`
   declarations per checklist item 7, not a bespoke config path.
4. Slot/bus foundation: implement primary/secondary slot decoding and cartridge slot mapping before
   adding media loaders.
5. Video baseline: implement TMS9918A-class behavior first, then MSX2-class VDP deltas required for
   real software boot and stable rendering.
6. Audio baseline: wire PSG and mandatory machine sound paths with the existing audio queue/resampling
   pipeline.
7. Input baseline: add MSX keyboard matrix + joystick mapping through plugin host dispatch,
   preserving host controls (F11/F12 and debugger hotkeys).
8. Cartridge/media phase A: add ROM loader with mapper detection and an explicit override path for
   unknown mappers; keep mapper implementations local to MSX machine code. If a debugger use case
   later needs banked-poke support for slot-mapped RAM, follow checklist item 5's real-support path
   rather than silently accepting an unrecognized `space`.
9. Disk/media phase B: add `.dsk` attach path (own format crate per checklist item 8, reusing `dsk`
   only if MSX's own on-disk layout is genuinely byte-identical, never by loosening `dsk`'s own
   CPC-specific assumptions) plus multi-disk workflow (`.m3u`) and deterministic disk index
   switching semantics.
10. Cassette phase C: add `.cas` loading flow (own format crate per checklist item 8) with
    auto-rewind behavior policy and observability logs via the generic `"overlay"` chip (checklist
    item 4).
11. Save states: add machine-specific internal save-state coverage for slot state, VDP/PSG/interrupt
    state, and media position metadata (external media bytes remain external; shape per checklist
    item 9).
12. App integration: declare MSX's `MachineProfile`/`MachineOptionInfo` options (BIOS/machine-type
    selection, any MSX-specific scalar knobs) and full `REQUIRED_MACHINE_DEBUG_CAPABILITIES`
    contract per checklist items 3/7 -- `retroclawd msx*` profiles need no bespoke CLI code, only
    correctly declared profiles.
13. Verification matrix: require ROM boot proof, keyboard input proof, cartridge mapper proof,
    multi-disk swap proof, cassette load proof, and save-state round-trip proof, all via the real
    binary (checklist item 10).
14. Documentation/citations: record accepted behavior references in `CREDITS.md` and keep roadmap
    step numbering synchronized with delivered milestones.

### BACKLOG: parity extensions (PARKED)

After MSX1/MSX2/MSX2+ baseline, evaluate in order: turboR/R800 path, advanced cartridge ecosystems,
and non-MSX blueMSX targets (SVI/Coleco/SG-1000) as separate roadmap sections.

### BACKLOG: cross-emulator comparison addendum (PARKED; blueMSX as additional source)

Keep an explicit feature-delta checklist against blueMSX/libretro and hardware references. If source
sets disagree, require executable tests or primary hardware documentation before selecting behavior.

## Atari 400/800 roadmap — 800/XL/XE accepted with recorded gaps

Resumed in the dependency-first additional-Machine request. C16/Plus4 are parked
by the owner; start with Atari 800, then 800XL/65XE/130XE. Reuse the established
Atari Family repository rather than creating a second Atari repository.

### TODO (2026-10-15): the Atari 8-bit family as one new family repository

Scope, decided by the owner: **Atari 400, 800, 1200XL, 600XL, 800XL, 130XE, XEGS and the 5200** as one family
(`retroclawd-atari`, a fourth submodule beside sinclair / amstrad / commodore, same layout: `crates/machines`, chips,
formats, `crates/plugins`, `crates/plugin-dlls`, a `firmware.toml`, no ROMs in the tree). The 5200 is the same chip set
(ANTIC, GTIA, POKEY, a 6502C at 1.79 MHz) with a different memory map, joystick/paddle controllers and cartridge-only
software, so it belongs here rather than in a separate family; the XL/XE models add banked RAM, the PORTB register, the
built-in BASIC and self-test switches, and the XEGS adds a detachable keyboard and its built-in game cartridge.

References, in order of authority:

1. **Altirra** (Avery Lee) <https://virtualdub.org/altirra.html> -- the main reference for cycle-exact ANTIC, GTIA and POKEY
   behaviour, DMA and CPU stealing, and the hardware manual it ships; read its source when work starts.
2. **atari800** <https://github.com/atari800/atari800> -- the second reference: models, memory maps, cartridge and disk
   formats, the 5200 specifics, behaviour of real software.

Validation: **Altirra's Acid800** (<https://virtualdub.org/downloads/Acid800-1.2.7z>, v1.2.7) is the hardware-compliance
suite for ANTIC / GTIA / POKEY / the CPU / the SIO and the XL/XE banking; every test must pass, and a failure is a bug in
the emulator, not in the suite -- same stance as ZEXALL/ZEXDOC and the 6502 suites. Wire it into the machine test suite
above (a `.xex` / cartridge image fetched from RomM by hash, because we must not commit it: check its licence before
vendoring any part), one expected result per model and per video standard (PAL and NTSC) and per memory size.

Shared chips follow the repository rule: ANTIC, GTIA and POKEY as their own crates under the family repository (not in
`retroclawd-crates`, no other family uses them), the `mos6502` core from `retroclawd-crates` reused as is (check the
6502C halt line / DMA stalling hook that `Bus::dummy_cycle` already gives the C64 cycle engine). Media: `.atr` / `.xfd`
disks, `.xex` executables, `.car` / `.rom` cartridges, `.cas` tapes (`rctape`, `rcdisk` and `rcld` get Atari outputs
for the test suite), `.atari800` state is not needed.


### DONE: original baseline (historical)

Experimental `atari-800` now boots explicit Altirra replacement OS/BASIC ROMs
through a dynamic Plugin in PAL/NTSC and fast/accurate modes. Initial ANTIC,
GTIA and POKEY reuse shared NMOS 6502 and MOS 6520 PIA. Keyboard/BASIC execution,
raw Programs, digital joysticks, tone pitch, State, warm reset, frame budgets and
the debug contract are wired. This is a working slice, not the complete baseline.

`machines/atari/atari-800.manifest.toml`: **36 passed, 0 failed, 8 known gaps**.
Raw/XEX Programs and read-only ATR boot have independently captured Atari800
7.2.1 reference memory. XEX executes ordered INITAD/RUNAD; D1: status/read uses
actual PIA/POKEY serial pins, not ROM traps. Version 4 State preserves in-flight
SIO, pending XEX, the IRQ-mask sample and explicit Chip/bus/interrupt pipelines;
versions 1-3 remain readable
(eject attached disks before loading version 1).
S2 reference printable-input automation remains blocked. S7 executes all tests:
fast 36/21/1, accurate 46/11/1 passed/failed/skipped in both Regions, versus
reference 25/32/1. No earlier passing case regresses; case-level evidence is
in the Family gap document.
POKEY async receive timer hold and shared SHY/SHX/SHA/TAS high-byte stores are
fixed; the illegal-instruction test now passes. CLI/SEI/PLP mask sampling also
passes, with explicit State migrations across all affected Machines and Drives.
Old-file readers remain; keep original files for rollback to older Plugins.
Accurate DMA HALT now stops writes independently of WSYNC RDY. The apparent
seven-cycle SLO was lost refresh stalls, not a missing Core cycle or POKEY timer
bug. Both CPU timing tests and serial-port timing pass in both Regions/modes.
Accurate BRK now passes: the Core permits mid-sequence NMI vector hijacking,
and ANTIC raises its nominal edge at cycle 8 rather than 12. Stack frames,
seven bus cycles, CPU serde, State layouts and the ABI are unchanged.
The ten-area fidelity increment adds NMIST/NMIRES/NMIEN phases, VCOUNT,
WSYNC, DLI and blocked-NMI timing, normal DMA/refresh and scrolling, shared
POKEY polynomial/divider/borrow/two-tone pipelines, and live GTIA PMG,
priorities and collisions. Accurate-mode IRQ delivery uses the preceding
clock's pin sample. Fast-mode BRK, abnormal DMA, special graphics modes and
remaining pipeline edge cases are not closed. Diagnostic totals are not
acceptance passes.
S4 checks documented pitch only, not reference RMS calibration.
Original Atari firmware, general Media, games, GTIA special modes and exact
timing remain unverified/unimplemented. Evidence and reproduction:
`machines/atari/docs/atari800-gaps.md`. No XL/XE profile exists yet.

### DONE: XL/XE and Media close-out

The Plugin now exposes 800/800XL/65XE/130XE with profile-specific RAM/OS,
PORTB/self-test/independent 130XE CPU/ANTIC banks, Help and correct joystick
counts. Native OS tests cover all profiles, Regions and execution modes for
BASIC/input/reset/ATR/State; added tests prove D2 writes through SIOV, five CAR
mappers through OS boot and CAS load/execution through CIO.
Writable ATR/XFD D1:/D2:, standard CAS playback and supported CAR/raw dumps are
wired through Plugin and Host Media routing.

Accurate Acid800 scores 56/1/1 on 800 (only XL banking fails) and 57/0/1 on
XL/XE; fast scores 43/14/1 and 44/13/1. PAL/NTSC agree. State v5 retains
readers for v1-v4 on 800 and preserves their Core/Chip field layouts.
Per-profile manifests pin real reference boot and raw/XEX/ATR/XFD/CAR captures.
Final release acceptance: 44 passed / 0 failed / 10 known gaps per profile
(176 passes, 40 repeated gap variants total). Tools reproduces CAR and XFD
fixtures byte for byte; CAS creation remains a tooling gap.
Reference input/CAS/RMS, diagnostic-screen mismatch, original Atari firmware,
games, cassette boot and remaining abnormal/analogue hardware remain explicit
limits. See `machines/atari/docs/atari800-gaps.md#xlxe-and-media-close-out`.
The playbook permits these recorded gaps; this is not full hardware parity.

### BACKLOG: external parity execution sequence (PARKED; gap vs Atari800/Altirra/libretro)

Primary source material:

- https://docs.libretro.com/library/atari800/
- https://github.com/libretro/atari800
- https://github.com/atari800/atari800
- https://sourceforge.net/projects/atari800/
- https://www.virtualdub.org/altirra.html -- Avery Lee's Altirra, the reference-tier Atari 8-bit
  emulator for cycle-exact ANTIC/GTIA/POKEY behavior specifically (its own documentation is widely
  cited as more precise on DMA/timing edge cases than Atari800's); read its real source directly
  when this roadmap resumes, same "shallow-clone and read, not a doc summary" standard this
  project already holds VICE/ep128emu/blueMSX to.
- acid800 v1.2 -- the Atari 8-bit hardware-compliance test suite (ANTIC/GTIA/POKEY timing and DMA
  edge cases, analogous in role to this project's own ZEXALL/ZEXDOC and Klaus Dormann/Tom Seddon
  6502 suites); required, not optional, for this roadmap's own verification matrix (step 13 below)
  once real ANTIC/GTIA/POKEY work begins -- treat a suite failure as a real bug to fix, not a
  suite quirk to work around, same stance this project already takes toward its own external Z80/
  6502 test suites.

### BACKLOG: core parity track — Atari 400/800 family one-go implementation

Strict sequence for a later one-pass implementation (do not reorder; read together with "Checklist:
new machine plugin integration" above -- items below that say "app integration"/"CLI" mean that
checklist's items 6-7, not a separate mechanism):

1. Scope lock: start with Atari 800 (48K, NTSC or PAL selectable) on the existing `mos6502` core --
   real 8-bit Atari hardware is 6502-family (6502C "Sally" variant, no documented behavioral
   difference from stock NMOS 6502 that this project's own core doesn't already model), the actual
   second real-world proof point this project's `mos6502` track was already built to find. Treat
   400/1200XL/XL/XE variants as later scope, not part of this first pass.
2. Crate layout: add `crates/machines/atari800` for machine wiring and dedicated chip crates for
   genuine second-consumer value (`antic` display-list/DMA video chip, `gtia` color/player-missile
   graphics, `pokey` sound+serial+keyboard chip -- `pokey` in particular has no existing analog in
   this codebase, unlike `antic`/`gtia`'s rough kinship to `vic-ii`'s own raster-DMA shape); add
   `crates/plugins/atari800-machine-plugin` as the thin C-ABI adapter over it (checklist item 1),
   including its own `step`/`halted`/`pc` fast-path vtable functions from the start (checklist item
   2) and its `machine-atari800` Cargo feature (checklist item 6).
3. Machine construction and memory map: implement the real Atari 800 memory map (OS ROM at
   `$D800`-`$FFFF`, hardware register windows for ANTIC/GTIA/PIA/POKEY at `$D000`-`$D7FF`, cartridge
   window at `$A000`-`$BFFF`, BASIC ROM optionally mapped in when no cartridge is present) and a
   deterministic reset/boot path from externally supplied OS ROM (+ optional BASIC ROM) -- both are
   real, separate ROM images on real hardware, same "several ROM files, not one" shape `c64` already
   established for its own KERNAL/BASIC/char-ROM split.
4. Bus/timing foundation: implement ANTIC DMA cycle-stealing from the CPU (a real, well-documented
   hardware fact -- ANTIC halts the 6502 during active display-list fetch/refresh cycles) using the
   same "the caller's outer real-time pacing loop trusts `step`'s own returned cycle count"
   substitution shape `vic-ii`'s own bad-line cycle stealing already established for a different CPU
   family, not a new mechanism.
5. Video baseline (`antic`+`gtia`): implement the minimum documented display-list instruction set and
   GTIA color/priority modes needed for real OS ROM boot screens and stable frame output; defer
   player/missile (sprite) graphics and any raster-interrupt-driven display-list tricks until a real
   ROM/game boot needs them, mirroring `vic-ii`'s own "base mode first, sprites/raster IRQ later"
   sequencing.
6. System/audio baseline (`pokey`): implement the 4-channel sound generation, the serial I/O
   controller (SIO -- the real protocol every Atari disk/cassette device speaks), keyboard scanning,
   and POKEY's own interrupt sources (IRQ on keyboard/serial/timer events) wired into `mos6502::Cpu::
   request_irq`; wire baseline audio output through the existing audio queue/resampling pipeline.
7. Input mapping: add Atari keyboard matrix mapping plus digital joystick (PIA-connected, not a
   separate port chip the way C64's CIA-scanned keyboard differs from its own joystick wiring)
   through plugin host dispatch, without regressing existing machine hotkeys.
8. Cartridge/media phase A: add `.rom`/`.car`/`.bin` cartridge loading (own format crate per
   checklist item 8) with mapper/bank-switching detection and an explicit override path for unknown
   mappers, mirroring MSX's own cartridge-phase caution about not silently guessing a mapper.
9. Disk/media phase B: add `.atr`/`.xfd` disk image attach (own format crate per checklist item 8,
   real Atari sector layout -- 128-byte single-density or 256-byte double-density sectors, a 16-byte
   `.atr` header -- not reused from `d64`/`dsk`, a genuinely different disk format family) through
   the real SIO protocol POKEY's own baseline already models, plus multi-disk workflow parity with
   MSX's own `.m3u` approach.
10. Cassette phase C: add `.cas` loading (own format crate per checklist item 8) with real SIO-level
    timing semantics and observability logs via the generic `"overlay"` chip (checklist item 4).
11. Save states: add machine-specific internal save-state coverage for ANTIC/GTIA/POKEY state,
    cartridge bank-switch state, and attached-media position metadata (external media bytes remain
    external; shape per checklist item 9).
12. App integration: declare Atari's `MachineProfile`/`MachineOptionInfo` options (OS ROM, optional
    BASIC ROM, model/region selection, any Atari-specific scalar knobs) and full
    `REQUIRED_MACHINE_DEBUG_CAPABILITIES` contract per checklist items 3/7 -- `retroclawd atari800`
    needs no bespoke CLI code, only a correctly declared profile.
13. Verification matrix: require boot-to-ROM-prompt/BASIC-ready proof, keyboard input proof,
    cartridge boot proof, disk load proof, cassette load proof, and save-state round-trip proof
    before marking support done, all via the real binary (checklist item 10) -- plus a clean run of
    acid800 v1.2 against `antic`/`gtia`/`pokey`, the accuracy baseline for this machine the same way
    ZEXALL/ZEXDOC and Klaus Dormann/Tom Seddon's suites already are for `z80`/`mos6502`; a failure
    is diagnosed and fixed, not silently accepted.
14. Documentation/citations: add exact references used for each accepted behavior to `CREDITS.md`
    (Altirra's own source/docs and acid800's own test descriptions in particular, wherever they
    were the deciding reference for a specific timing/behavior choice) and keep TODO sequence
    numbering aligned with delivered milestones.

### BACKLOG: parity extensions (PARKED)

After Atari 800 baseline is complete, evaluate follow-up scope in this order: Atari 400 (same
hardware, different case/keyboard -- likely memory-identical the same way Spectrum's +2 was to
128K), XL/XE models (real, different memory-map/banking facts -- extra RAM under the OS ROM, a new
PORTB-driven banking register, a real BASIC-disable pin), and only then peripheral extras (real
printer/850 interface emulation, hard-disk-style SIO devices).

### BACKLOG: cross-emulator comparison addendum (PARKED; Atari800/Altirra/libretro as additional source)

When this roadmap is resumed, keep an explicit feature-delta checklist against Atari800/Altirra/
libretro and require hardware-backed behavior, acid800 v1.2 results, or executable tests whenever
sources disagree -- Altirra is the preferred tie-breaker specifically for ANTIC/GTIA/POKEY cycle
timing, matching its own documented emphasis on that accuracy tier.

## Next-wave machine roadmaps (added 2026-10-12) — PARKED

Requested systems, all to the same spec as the existing machines: a machine crate under
`crates/machines/`, shared chip crates under `crates/chips/` when a second user exists, format
crates under `crates/formats/`, a `*-machine-plugin` implementing the `plugin-api` contract
(options, `key_names`, media kinds, save states, debugger state, overlay chip), a `machine-*` cargo
feature in `apps/retroclawd`, ROM-gated boot proof tests, per-machine docs in `CLAUDE.md`/`README.md`/
`CREDITS.md`, and the same close-out discipline as CPC/C64 (a documented feature-delta checklist
against the reference emulator, every deliberate gap named). Read "Checklist: new machine plugin
integration" first. Everything below is a *scope note*: each item must be re-researched from primary
sources (data sheets, reference emulator source) before its first line of code, and promoted to its own
`## <name> roadmap` section with a DONE/BACKLOG split.

Suggested order, by reuse (not a commitment): BBC Micro -> Acorn Electron (shares 6502, ROM paging,
tape/disk formats), Apple II family (6502/65C02, new disk formats), Oric (6502 + AY + VIA reuse),
ZX Spectrum Next (extends the Spectrum), Commodore 128 (extends the C64 crates), Dragon 32/64 (new
6809 core), TI-99/4A (new TMS9900 core), Sinclair QL (68008; shares the 68000 core decision with
Genesis), RP6502 (W65C02S + RP2040-side devices), Commander X16 (65C02 + VERA + YM2151).

- **ZX Spectrum Next** (full emulation): Z80N (the Z80 plus extra opcodes -- add to `z80` behind a
  CPU-variant flag, validated by its own exhaustive opcode test), NextReg-driven hardware, 8K-page
  MMU over up to 2 MB RAM, ULA/Timex/LoRes/Layer 2/tilemap modes, 128 hardware sprites, Copper,
  zxnDMA, three AY (TurboSound Next) + DAC audio, 3.5/7/14/28 MHz speeds, SD card (esxDOS/NextZXOS
  file API), RTC, `.nex` loading plus `.sna`/`.z80`/`.tap`/`.tzx`, joysticks/mouse, Next keyboard.
  Reuses `zx-spectrum` video contention knowledge, `ay-3-8910`, `zx-tape`, snapshot crates.
- **Apple II series** (II, II+, IIe, enhanced IIe, IIc; IIgs is a separate later phase needing a
  65C816, the Mega II and the Ensoniq 5503): 6502/65C02 (`mos6502` gains a 65C02 variant), soft-switch
  memory/video map (text/lo-res/hi-res/double hi-res, 40/80 columns, Language Card, aux memory),
  Disk II controller with GCR (`.dsk/.do/.po`, `.nib`, `.woz`), cassette, speaker, paddles/joystick,
  slot cards (Mockingboard, mouse, Thunderclock, Super Serial), keyboard variants.
- **BBC Micro Model B** (then B+, Master 128 as variants): 6502 at 2 MHz, 6845 CRTC + Video ULA
  (modes 0-7) + SAA5050 teletext, two 6522 VIAs (system/user), 8271 FDC (B) / WD1770 (B+/Master),
  SN76489 sound, 6850 serial/cassette, paged sideways ROM/RAM, 1 MHz bus, Tube later, `.ssd/.dsd`,
  `.uef`/tape, keyboard matrix + joystick/ADC, OS 1.2 + BASIC.
- **Commodore 128**: DONE and closed (2026-10-12), see `CLAUDE.md`'s `c128`/`vdc8563`/`drive-link`
  notes: 8502 + Z80 on one bus behind the MMU 8722, Z80 boot hand-off, VIC-IIe + VDC 80 columns, SID,
  two CIAs, C128 and C64 mode, save states, real 1571/1581/1541 DOS over IEC (`--true-drive`), plugin
  `c128-machine-plugin`, `--run`/`--mode c64`/BASIC `.prg` injection, `vic_accuracy = cycle`, and
  CP/M Plus booting to `A>` from the CBM-format system disk (`BOOT`, 80 columns). Proven with the real
  ROMs (`roms/c128`): BASIC 7.0 `READY.` in 40 and 80 columns, `GO64`, typed keys, directories from real
  drive DOS ROMs. **Left open, deliberately**: burst/fast serial (CIA1 SDR + the 1571/1581 sides), the
  1571's WD1770 (MFM) mode that CP/M data disks need, external function ROM slots, REU/cartridge/printer/
  datasette parity with the C64 plugin, `.lnx`/`.tap` media, the 128D/128DCR variants and VDC light
  pen/interlace.
- **Acorn Electron**: 6502A (2 MHz bus-throttled to 1 MHz RAM access), single ULA doing video, sound,
  tape and ROM paging, 32 KB RAM, BASIC + sideways ROMs, `.uef`/cassette, Plus 1/Plus 3 (ADFS/DFS
  via 1770) later. Shares almost everything with the BBC Micro crates.
- **Oric-1 / Oric Atmos**: 6502 at 1 MHz, 6522 VIA, AY-3-8912, the ULA's text/hires attribute
  video, cassette (`.tap`, `.wav`), Microdisc/Jasmin disk interfaces (`.dsk`), printer port, ROM
  variants (1.0/1.1, Atmos), keyboard via VIA + AY.
- **TI-99/4A**: a new TMS9900 core (16-bit, workspace-register architecture, memory-mapped registers
  in RAM, 3 MHz with the 8-bit multiplexed bus wait states), TMS9918A VDP (graphics I/II, text,
  multicolour, sprites), TMS9919 sound, GROMs with auto-incrementing address pointer, 256 bytes
  scratchpad, console + cartridge ROM/GROM banking (`.rpk`/`.bin`), cassette, TI Disk Controller
  card (`.dsk`), speech synthesizer (TMS5220) and peripheral expansion box later.
- **Dragon 32/64**: a new MC6809E core (full instruction set, cycle timing, interrupts, validated
  with a published 6809 test suite), MC6847 VDG, MC6883 SAM, two 6821 PIAs, cassette, cartridge
  (`.rom`), DragonDOS/Delta disk with WD2793 (`.vdk/.dmk/.dsk`), 64's 64 KB RAM map and serial
  port; CoCo-compatible variants are cheap once the 6809 core and SAM/VDG exist.
- **Sinclair QL**: a 68008 core (reuse the 68000 core if Genesis lands first; one core with a
  data-bus-width option rather than two), ZX8301 (video/display modes 4 and 8, 512x256/256x256) and
  ZX8302 (RTC, microdrive, network, serial), the 8049 IPC (keyboard, sound, joysticks) with its
  serial protocol to the ZX8302, Microdrives (`.mdv`), QDOS/SMSQ ROM, 128 KB RAM and expansion,
  `.win`/`.qlf` disk and file formats.
- **RP6502 (Picocomputer)** (<https://picocomputer.github.io/emu.html>, reference emulator in the
  browser): W65C02S CPU, the RP2040-side RIA (registers at `$FFC0-$FFFF`, 64 KB XRAM, USB
  keyboard/mouse/gamepad, file API, `.rp6502` ROM packaging) and VGA video processor
  (modes 1-4, character/tile/bitmap, sprites, 3D-ish affine tiles), YM2151-style/PSG sound per the
  firmware docs. Read the firmware documentation and the emulator source before scoping; the
  platform changes between firmware releases, so pin a version and record it in `CREDITS.md`.
- **Commander X16** (full emulation): W65C02S at up to 8 MHz (`mos6502` 65C02 variant, shared with
  the Apple II work), 512 KB banked RAM (`$A000-$BFFF`, 8 KB windows) and up to 2 MB banked ROM
  (KERNAL, BASIC, DOS, GEOS, CodeX, charsets), two 65C22 VIAs (keyboard/mouse via PS/2-style bit-banged
  or the SMC/I2C system management controller, SNES pads, IEC), the **VERA** video chip (own 128 KB VRAM,
  320x240/640x480 VGA, layer 0/1 in tile and bitmap modes, 128 sprites, palette of 256 12-bit entries,
  composer/scaling, FX/affine helper, SPI/SD card access, PSG with 16 voices + PCM FIFO, UART-style
  access via the data ports with auto-increment), YM2151 FM (OPM) audio, the `.prg`/`.bas`/`.crt`-style
  program loading from an FAT32 SD image (`.img`) or a host directory (hostfs, as the reference
  emulator does), 65C02 `STP`/`WAI` handling, the RTC and NVRAM on the SMC, and the emulator debug
  ports. Reference: the official `x16-emulator` source and ROM repo; pin a ROM release (R4x) and record
  it in `CREDITS.md`. Needs new chip crates (`vera`, `ym2151`), a FAT-image/hostfs format layer, and the
  keyboard layouts the ROM expects.

## Other future systems (brief scope notes) — PARKED

Not full roadmaps -- placeholders for systems mentioned but not yet sequenced in detail. When one of
these is promoted to an active roadmap, give it its own `## <name> roadmap — PARKED` section
following the same DONE/BACKLOG template as Enterprise/MSX/Atari above, and read "Checklist: new
machine plugin integration" first, same as every roadmap in this file.

- NES: reuse `mos6502` with 2A03 decimal behavior supplied by the machine, then add PPU, APU,
  controller protocol, iNES parsing, and mapper implementations local to the NES crate.
- Sega Master System: reuse the Z80; add its VDP, PSG, cartridge, and controller hardware.
- Sega Genesis: add a 68000 core, YM2612, PSG, VDP, and an internally scheduled secondary Z80.

Decision for a future Genesis implementation: keep `Machine::step` unchanged initially. The machine
coordinates both CPUs internally, as it already coordinates CPUs and peripherals. Add a separate
multi-core debugger surface only if real debugger requirements demand it; do not widen every
single-CPU `Machine` implementation preemptively.

## Shared peripheral chips

Implemented shared-chip crates are `ay-3-8910`, `nec765`, `sinclair-keyboard`, `vic-ii`, `i8255`,
and `sid`. Keep their interfaces chip-specific; machine crates own address decode and host wiring.

Potential chips such as SN76489, YM2612, Z80 PIO, and Z80 CTC get separate crates when a real
machine starts using them. Do not create speculative chip crates without a consumer.

## Interrupts

### DONE: baseline

- The Z80 core has distinct maskable IRQ and NMI paths, with NMI priority and correct IFF behavior.
- ZX Spectrum raises its maskable interrupt once per video frame.
- ZX81 raises NMIs at the modeled scanline cadence while its NMI generator is enabled.
- C64 CIA1 (Timer A) and VIC-II (raster-compare only) both can request their CPU interrupts; the
  CPC Gate Array can too.

The remaining headless `HALT` execution concern is tracked under Debugger and application UX. New
machine timing work must preserve these established interrupt-delivery contracts.

## Display shaders

### DONE: selected single-pass CRT presets

Bundled WGSL presets now include `crt-geom`, `zfast-crt`, and `crt-potato`, alongside the earlier
`crt`, `scanlines`, and `sharp-bilinear` choices. These use one `Material2d` fragment pass and the
shared `shader_intensity`/`scanline_intensity` controls. See `CREDITS.md` for upstream sources and
the deliberate procedural-mask/progressive-rendering simplifications.

### PARKED: full crt-royale and mame_hlsl ports

Libretro's `crt-royale` preset is a 12-pass pipeline with bloom/halation intermediates and six mask
textures. `mame_hlsl` is an 11-pass pipeline with NTSC encode/decode, phosphor/chroma stages, and a
mask texture. Full ports need render-to-texture pass orchestration and multi-texture materials;
single-pass approximations are intentionally not substituted without a user decision.

## Architecture guardrails

- `machine` remains CPU-agnostic and must not depend on concrete CPUs or machines.
- CPU-visible addresses are each Core's own (`machine::Bus` is 16-bit; a 68000 Core defines its own bus); the `Machine` trait and the plugin protocol use `machine::Addr` (`u64`); banked physical memory belongs inside each machine bus (ADR 0003).
- CPU cores depend only on `machine::Bus`.
- Format crates parse and write their own concrete formats; do not invent a shared
  `Loader`/`Snapshot` trait across snapshots, tapes, and disks.
- Peripheral crates expose chip-specific interfaces, not `machine::Bus` implementations.
- Machine crates own address decode and chip-to-chip wiring.
- Do not extract a shared component from one consumer or from two structurally different consumers.
- Clean cutovers only: migrate callers and remove obsolete paths rather than keeping aliases.

## ROM and legal handling

The private development repository currently contains copyrighted ROM files for personal backup and
verification. It must not be made public while those files remain anywhere in git history.

Any public distribution must:

- Remove copyrighted ROMs from the complete published history, not only the working tree.
- Require users to supply legally obtained ROMs.
- Keep user-facing ROM size/layout instructions current for every machine.
- Review bundled shader and third-party fixture licenses before declaring a project license.

## Verification standard

- Bug fix: reproduce the failure, fix the source, and confirm the reproduction no longer fails.
- Hardware behavior: cite an authoritative source in `CREDITS.md` and verify against real software
  or a focused behavioral test.
- UI: exercise the real surface and visually inspect it.
- Performance: profile before changing hot code and benchmark the exact path afterward.
- Permanent tests must defend observable behavior or a plausible regression; use throwaway probes
  for one-time evidence.
- Run focused checks during development, then workspace formatting, linting, and relevant tests once
  integration is complete.

## Performance

Recorded baselines that still guide future work:

- Generic `Cpu::step<B: Bus>` removed dynamic dispatch from the Z80 hot path and reduced the
  measured ZEXDOC runtime from about 59.4s to 27.7s before later interrupt costs. Do not reintroduce
  trait-object bus access on the instruction path.
- `vic_ii::render_text` gained about 1.40x from precomputed SIMD mask/palette tables; constructing
  SIMD lanes per cell was slower than scalar code. Preserve the precomputation strategy.
- Use `cargo build --profile profiling` for optimized binaries with symbols.
- Release profile size optimization is intentional; hot emulator crates have package-level
  `opt-level = 3` overrides in the workspace manifest.

No optimization task is active without a current profile showing a real bottleneck.

## Completed baseline

These are complete and should not reappear as open work:

- Z80 core validated by ZEXALL/ZEXDOC, including documented and undocumented behavior.
- ZX Spectrum 48K, 128K/+2, and +2A/+3 memory models.
- Spectrum memory/I/O contention and floating bus.
- `.z80`, `.sna`, and `.szx` snapshot loading/saving; ZX81 `.p`/`.p81` loading.
- `.tap`, `.tzx`, `.pzx`, and `.csw` tape playback; fast-load trap.
- Standard and Extended DSK parsing/writing; +3 and CPC NEC765 controller with write/format support.
- Beeper, AY-3-8910, Turbosound, CPC AY audio, and Bevy audio output/resampling.
- Kempston, Sinclair 1/2, Cursor, Fuller, and Kempston mouse input.
- Internal save states, reset, run-ahead, shaders, aspect-preserving scaling, and persistent app
  settings.
- Browser debugger with disassembly, memory view, Bitmap Explorer, breakpoints, pause/step,
  assemble-and-poke, and save/load operations.
- CPC 464 firmware boot and C64 KERNAL/BASIC boot.
