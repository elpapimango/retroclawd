# CLAUDE.md

Guidance for Claude Code working in this repository.

This folder holds what runs the emulator headless: `crates/rc-suite` (`rc-scenarios` and the integration suites) and `scripts/` (the reference-emulator capture scripts). The command-line tools (`rcasm`, `rcdis`, `rcdisk`, `rctape`, `rcsnap`, `rcld`, `rcchd`) are a separate project, [retroclawd-tools](https://github.com/elpapimango/retroclawd-tools), installed by `tools/build_all.sh --tools`; `rc-suite` takes them from crates.io.

## Testing approach (decided 2026-10-17)

The point of these suites is **acid tests for an emulator**: programs and facts from outside (third-party diagnostics, published
documents, other emulators we read) that show the emulator behaves like the real machine *as far as it is worth checking* -- not proof of
real hardware, which we will never have and do not aim for. If games, demos and applications load and run, that is the goal. So:
a case earns its place by being judged from outside (a third-party program, a document's number, a behaviour a game can see); a case that
only checks our own plumbing, or repeats what another case already shows, does not. Pass 1 of the review (2026-10-17) cut the 101 cases to
69: merged cases keep every check of the cases they replace, removed ones are listed in `TODO.md` of the host repository. Acid-style suites
(CPC Acid Tests Suite, cpctech `test.zip`, Acid800 as a model for how an acid test is laid out) are added as cases with a known-gaps list. Pass 2 (2026-10-17) applied the same rules to every unit and integration test of every repository (this repository's own tools went 145 -> 90); see `TODO.md` in the host repository.

Acid tests fetched so far (copied by the host's `tools/fetch_acid.sh --from <folder>`, never committed): Patrik Rak's z80test on the 48K (`spectrum_z80test`; all variants pass). CPC: the self-checking programs of Kevin Thacker's suite (`cpc_acid_tests`; every program has a recorded state in `BASELINE`, regenerate with `RC_ACID_PRINT=1`; `RC_ACID_ONLY=name` runs one). Our own acid programs: `spectrum_timing` (T-state counts against the WOS48 contention rules), `spectrum_io`, `c64_hardware` (cases name their document in the file header; `.gap(..)` marks what the emulator does not do yet).

## Data-driven scenarios (2026-10-18)

The standard checks of the new-machine playbook (`docs/new-machine-playbook.md` in the application repository) are data, not Rust:
`rc_suite::scenario` reads a `<machine>.manifest.toml` (machine, files, reference emulator, scenarios S1-S8 as `screen`, `memory`,
`audio`, `state`, `timing`) and runs the real `retroclawd` headless. `rc-scenarios run <manifest>` runs one, `rc-scenarios hash <file>`
prints the SHA-256 to pin a golden, `rc-scenarios check <manifest>` validates without an emulator; `tests/manifests.rs` runs every
`*.manifest.toml` below `RC_MANIFESTS`. Environment: `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS`, `RC_ROMS`, goldens in
`RC_GOLDENS` (fetched from RomM by hash, never committed), diff images of failing screens in `RC_SCENARIO_OUT`.

Rules the code enforces: every golden is pinned by hash; a tolerance needs a written `reason`; a scenario that cannot be matched is
`status = "known-gap"` with a `gap` (reason and evidence) and the run fails when a known gap starts passing; unknown keys are errors.
A screen or memory known gap with no captured golden may omit both `golden` and `golden_sha256` (and `compare` for a screen).
Its reason and evidence must be nonempty. The runner reports `not run: reference golden not captured` without starting the emulator;
it never substitutes a hash, invents a comparison, or counts that scenario as passed. Memory ranges remain required.
Once a golden is supplied, the ordinary pin, comparison and unexpected-pass rules apply unchanged.
`status = "known-gap"` with a `gap` (reason and evidence) and the run fails when a known gap starts passing; unknown keys are errors. A scenario whose `--firmware auto` finds no firmware (no cache, RomM or network; retroclawd's "no firmware with a
matching SHA-1" line) is reported as `SKIP` with that line and does not fail the run; a file found with the wrong hash
(`has SHA-1 ... but ...`) still fails it.
**A picture from another emulator is compared with `mode = "similarity"`** (both reduced to a 64x48 luminance grid over an optional
`crop`, mean absolute difference at most `max_mad`): the reference draws its own border, scale and palette, so pixel equality is not
possible. `normalize = true` stretches each picture's pixel brightness to the full range (a different palette); `anchor_ground =
true` makes the most common brightness (the paper) full white (a different paper grey beside a brighter white); both are fitted on
pixels, not grid cells. Keep the crop inside the screen (a border colour is a third tone neither removes) and away from a blinking
cursor. `mode = "exact"` is for goldens this project made itself. A scenario's own `files` (option flag -> file below `RC_ROMS`)
and `actions` (`--at FRAME:ACTION`, e.g. `key_down=Enter`) are added to the manifest's; the same flag replaces the manifest's.
`audio` scenarios check a pitch: `tone_hz` within `tone_tolerance_pct` (the strongest sine in that band, at least `min_amplitude`) or
`silent_below_rms`; `rc-scenarios audio FILE.wav [--from --to --lo --hi]` measures a recording to write one.

**State (2026-10-18)**: every machine has a manifest in its family repository (`machines/<family>/<id>.manifest.toml`: zx81, the three
Spectrums, cpc464/664/6128, c64, c128), each with S1 boot, S2 typed input, S4 audio (a pitch, or silence on the ZX81), S5 state, S6
timing, and S3 media and S7 hardware test where a route exists; the CPC Plus and GX4000 have none (no reference emulator). The
references are **ZEsarUX 13.0** (ZX81, Spectrum family, CPC family) and **VICE 3.10** (C64, C128); `scripts/capture_zesarux_screen.py`,
`capture_zesarux_audio.py` (ZEsarUX over its remote protocol, `--realvideo`, a crop onto retroclawd's canvas; it refuses to start when a
ZEsarUX is already listening, which once gave a golden of the wrong machine), `capture_vice_screen.py` (a fixed cycle count with
`-limitcycles`, exit screenshot; VICE only records sound in real time) and `make_zx81_p.py` make the goldens. Each family's
`scenarios/build.sh` builds the test media with these tools into the git-ignored `software/scenarios/`. Goldens live in a folder of
your own (`RC_GOLDENS`), never in git. Known gaps found: S6 on the ZX81, CPC, C64 and C128 (each emulator paces at its nominal frame
rate; the references use the real raster), the C64's fast loader printing no SEARCHING/LOADING, the CPC diagnostics' CRTC/FDC lines
(reference looks wrong), the C64 dead test's clock; each has a note under the family's `docs/`. All are closed (2026-10-19); the CPC
diagnostics' lines are compared with Caprice32 (`scripts/capture_caprice_screen.py`, `golden_crop` when the reference frames
the screen differently). Not done: a `trace` kind (S8; the
plugin ABI has no instruction trace), memory windows above 64K, S2/S4/S7 on machines where the reference cannot be driven (the 128K
menu, the C128's S3), z80test and the CPC acid disks as manifests (the hand-written suites cover them).
The Atari ST's reference is Hatari 2.6.1, run as a black box: `scripts/capture_hatari_screen.py ROM VBLS OUT.png`
(a debugger breakpoint at that VBL takes `screenshot`; 640x400 with `--borders off --statusbar off`, byte-identical on
repeated runs) and `scripts/capture_hatari_trace.py` (`--kind pc|io`, `--disk-a`, `--trace-extra`). Hatari cannot be
given keys or mouse input headlessly, so the ST's input scenarios have `exact` goldens made by retroclawd.

## Emulator suites (`crates/rc-suite`)

A harness and per-machine suites that run the real `retroclawd` headless and check memory, registers and the picture against
**hardware facts that each case cites** (a datasheet, a manual, the ROM itself), never against the emulator's earlier output.
`tests/zx81.rs`, `tests/cpc.rs`, `tests/c128.rs`, `tests/spectrum.rs` and `tests/c64.rs` hold the cases; a case belongs to an `Area` so a failure names what is wrong. A feature the
emulator lacks is a *gap*: `Case::gap("why")` -- it must fail, and the run fails when it starts passing, so gaps only shrink on
purpose. Everything skips unless `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS` (a folder with
`sinclair/ amstrad/ commodore/`, each with its `roms/`) are set, so public CI stays green without firmware.

* Run: `RETROCLAWD_BIN=... RETROCLAWD_MACHINE_PLUGIN_DIRS=... RC_ROMS=.../machines cargo test -p rc-suite -- --nocapture`.
* `--frames N` runs whole frames like the window (a frame interrupt each, no stop at `HALT`); `--max-cycles` stops at the first
  `HALT`, which a ROM waiting for the next frame reaches during boot.
* `z80_program` / `z80_image` assemble a test program (`--program`). A `--program` run does not boot the firmware, so a case that
  needs video timing programs the CRTC itself.
* `Resident` runs code on a **firmware-booted** CPC: it boots, saves a snapshot, writes the code into the snapshot's RAM, points
  the program counter at it and resumes. The firmware's jump block and interrupts are live, so the code can call AMSDOS (`CAS IN
  OPEN` 0xBC77 / `CAS IN DIRECT` 0xBC83), which is how disks the tools made are checked headless (no keyboard needed).
* Writing a test program as a string: the program text goes in a plain string with real newlines.
* `--type TEXT` (with `--frames`, `--type-after N`) types on the emulated keyboard like the paste hotkey, so BASIC can be driven and
  the keyboard path is part of the test: `PRINT 12*34` is checked on the C128 screen, and a disk is loaded with `LOAD"NAME",8,1`.
  A C128 with a real drive (`--true-drive`) probes it for a boot sector first: type from frame 500, not 150.
  `--type-delay MS` sets the time between key events (default 60): the ZX81 ROM debounces its keys and loses them that fast, use 160.
  The ZX81 keyboard cases read the expected matrix positions from the ROM's own key tables (0x007E unshifted, 0x00A5 shifted).
  The ZX81 ROM scans the keyboard once a frame and debounces it: type with `--type-delay 160` (the default 60 ms loses keys). The ZX81
  suite checks every key against the ROM's own key tables (0x007E unshifted, 0x00A5 shifted) with a program that reads all eight
  half rows and keeps the AND of what it saw, so exactly the typed key's matrix line shows low.
* `c128_8502_program` runs 6502 code on a C128: it powers up on its Z80, so a Z80 stub sets the memory map, points the 8502 reset
  vector at the code and sets the processor-select bit of the MMU's mode register (a Z80 reaches the I/O chips with `IN`/`OUT`, not
  memory writes; in Z80 mode `$1000-$13FF` is colour RAM). The KERNAL is not booted in such a run; to use it, boot normally and type.
* `mos6502_program` / `Env::c64()` run 6502 code on a plain C64 (`--program` starts the CPU at the load address with nothing
  initialised; the KERNAL is not booted). To measure a frame on the C64 use a CIA timer between two raster line 0s -- the
  host's `--frames` batch (985248 / 50 cycles) is pacing, not the VIC's frame -- and read `$D012` *before* `$D011`, or the
  255 -> 256 line change shows up as a false line 0. The Spectrum cases need `--no-border` for a 256x192 picture.
* Read hardware values through the CPU, not the dump: `PRINT PEEK(1)` found that the C64 port read 247, which the dump
  path would also have shown but is easy to dismiss as a debug view.
* `tools/fetch_roms.sh` in the host repository fetches the firmware these suites read from your RomM (the files of every
  case, drive DOS ROMs included); the C64 KERNAL must be revision 3 for the boot cases to match their baseline.
* **Third-party diagnostics**: `tools/fetch_diagnostics.sh` (host repository) downloads pinned release images (SHA-256 checked) into
  `machines/<family>/software/diagnostics/` (git-ignored; they are other people's software, never committed). The Spectrum case runs
  brendanalford/zx-diagnostics v0.37's `testrom.bin` (LGPL-3.0) as the machine's ROM and reads the result from the screen colours its
  source documents (`PASS` in green ink, a bright-green "48K RAM Tests Passed" bar, nothing red) -- about 100 s of machine time, 5000 frames.
  llopis/amstrad-diagnostics (MIT, v1.3; `AmstradDiag.cpr` and `.dsk`, fetched the same way) has five cases in `tests/cpc.rs`
  (`cpc_diagnostics`, about 35 s): the cartridge boots on a GX4000 in colour and shows `LOWER RAM: PASSED` in its success colour (cyan, nothing
  in the error colour); its keyboard test paints every key it reads as pressed from the matrix, so two runs type the odd and the even
  half of the 48 unshifted keys and check that exactly those cells light up (a key wired to a neighbour's matrix position fails it --
  swapping `,` and `.` was mutation-checked); on a CPC6128 the disk (`RUN"DIAG`, then `q` padding typed while it loads, then the menu key)
  passes its upper RAM test and its ROM test names the lower ROM, BASIC and AMSDOS by checksum (read from how far each line's text reaches).
  The cartridge needed two fixes: its `.cpr` is malformed (`parse_lenient`) and a CPC+ must honour classic Gate Array ink writes (it showed
  black). The first screen of the keyboard test ignores the first two keys, hence the sacrificial `z`. Five more cases (2026-10-16, `cpc_diagnostics`
  now about 90 s in a debug build): the keyboard test's joystick cells (`--at N:key_down|key_up=Joy1*`, 10 frames per press, two
  complementary runs: exactly the pressed cells light), the menu's JOYSTICK result after leaving the test by holding fire 1 (cyan `PASSED`
  with up/down/left/right/fire 1 read; error-colour `FAILED` when right never was; the untouched keyboard is `FAILED`), and the 6128 disk's
  soak test (`SOAK ITERATION N` in the title: banner still there, no error colour and a changed counter digit at frame 2000 versus 3800, taken
  with `--at 2000:screenshot=FILE`; `rc_suite::Screen::read` is public for that). Mutation-checked in the amstrad plugin: swapped Up/Down
  bits, Left moved to bit 7, bit 7 stuck on writes to RAM banks 4+ (aliasing banks or byte pairs did NOT fail it: the program fills and verifies one
  pattern, so an alias is consistent to it). Not done: the sound test, the ROM test for a 464/664.
  The +3 case `a +3 reads a sector from each of its two drives` (`--disk` + `--disk-b`) drives the FDC ports directly with a hand-made
  one-sector DSK per drive (unit in the second command byte of Read Data); mutation-checked (drive B inserted into unit 0 fails it).
  The unified diagnostic cartridge (crystalct's board: the modified dead test in Ultimax mode plus a `$DE00` flip-flop that turns it into an ordinary 8K cartridge running the standard diagnostic rev 586220++) is the second case in `c64_diagnostics`, from the 32K EPROM image `tools/fetch_diagnostics.sh` fetches (`unified_32k.bin`, local use only). The `.crt` is built by the test (retroclawd's own hardware type 0xC640). By frame 8000 (30 s in a release build, about 5 min in a debug one) the dead test has finished and the cartridge switched (between frames 5000 and 6000), and the standard diagnostic shows its RAM/ROM tests OK, the ROM checksums (`B6C70A`, `043D56`, `F3F7F8`), TIMER1/2 A/B and INTERRUPT OK; the ports and cassette read BAD without the diagnostic's test plugs and are not asserted. An earlier note here that the dead test's RAM test takes half an hour was wrong (Ultimax leaves only the low 4K; a whole cycle is about 5000 frames).
  AmstradDiag's own sound test (repository head 7ac70fc, `[T]` in the menu; the v1.3 release has none) is `cpc_diagnostics_head` in `tests/cpc.rs`,
  skipped unless `AmstradDiagHead.dsk` exists (`tools/build_amstradiag_head.sh` in the host repository builds it with sjasmplus and puts the program
  on a blank AMSDOS disk with `rcdisk`). It runs `RUN"DIAG`, types `t`, and reads the `--dump-audio` capture: the first loud sound (the CPC's key click for
  the typed letters is quieter) starts the clock, then noise A, A+B, A+B+C (louder with each channel, no chord tone in it), tones 110/131/165 Hz
  from the step that turns each channel on, then muted. Mutation-checked (PSG clock divisor). The C64 unified cartridge's standard diagnostic reads
  the 6581 BAD without a paddle in the control ports (its test plug is what puts POTX/POTY in `$50-$8F`) and OK with `--mouse paddle-port2`: two cases.
  Later the same day: the disk on a CPC664 (two cases: system information with the FDC detected, ROM test by checksum), eight PSG cases
  (`--dump-audio` through `rc_suite::Audio`: each channel's tone with sub-harmonic checks -- a square wave a third too slow still shows its
  third harmonic at the right frequency, which a first version missed and a mutation caught --, chord, mixer mute, noise, volume, register 1)
  and one case reading `B:SAME.BIN` from drive B (`--disk-b`) through AMSDOS on a 6128. Note for programs run with `--program`: set SP below
  `$C000`, the upper ROM shows there and a stack in high RAM reads back as ROM.
  The C64 dead test (crystalct/C64UnifDiagCart's modified rev 781220 ROM, a raw 8K Ultimax image fetched from a pinned commit; CERN-OHL-W-2.0
  repository, Commodore's code, so local use only) has one case in `tests/c64.rs` (`c64_diagnostics`, about 130 s in a debug build): wrapped in an
  Ultimax `.crt` it boots with no KERNAL involved and, by frame 4000, prints OK next to ZERO PAGE, STACK PAGE, SCREEN RAM and COLOR RAM and
  starts the main RAM test (which is short: the whole dead test, sound test included, ends by about frame 5000). The Spectrum suite also checks that both
  16K halves of every 128K-family ROM variant appear at 0x0000 exactly as the file holds them.
* Joysticks and sound are testable headless now: `--at N:key_down=Joy1Up` and `--dump-audio` (see above).

## Acid-suite notes (2026-10-18)
- All suites pass against a full `tools/build_all.sh` build. Env: `RETROCLAWD_BIN`, `RC_ROMS` (= host `machines`), `RETROCLAWD_MACHINE_PLUGIN_DIRS`.
- `cpc_acid`: `RC_ACID_PRINT=1` prints `BASELINE|name|finishes|{failing}|passed|last line|frames`; `RC_ACID_ONLY=name`. Segmented runner: 700-frame first segment, then 200-frame segments via `--at N:save_state`/`--load-state`; 40 silent segments = hang. `UNRELIABLE` = programs the author flags (ppi vsync-on).
- Remaining CPC gaps (in `BASELINE`): cpctest, asicfloat, asicppi (hang), asicrom, asictest, asic_external_ram, dmatest, pritest, FDC programs (nec765 is instant, no overrun), fdc_wp (needs write-protect).
- `spectrum_timing`: pad filler must use `ld d,0` (never BC; port programs keep it). Delay = contended cycles - uncontended cycles.
