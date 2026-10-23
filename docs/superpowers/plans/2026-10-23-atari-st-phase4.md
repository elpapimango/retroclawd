# Atari ST Phase 4 and the Phase 3 carry-overs (slices 18-20) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the spec's Phase 4 column (§5) and clear the Phase 3 deferrals: the machine ids `atari-mega-st` and
`atari-ste`, the Blitter (functional and bus timing), the STE Shifter (4096 colours, scrolling, writable video counter),
STE DMA sound with the Microwire/LMC1992 and stereo output, the STE enhanced joystick ports, the Mega ST real-time clock,
memory watchpoints; and from Phase 3 the fuzz runs, the ACSI `--disk-save`, IKBD/USART leftovers, a second attempt at every
`known-gap` probe, the mid-line resolution probe.

**Architecture:**
- Same repositories as Phase 3: CPU/shared chips in `retroclawd-crates` (branch `atari-phase4`), everything else in
  `retroclawd-atari` (branch `atari-st-slice18`, one branch for the whole phase), probe tooling in `retroclawd-tools`
  (`atari-phase4-tools`), docs/pins/`--disk-save` in the host (`master`).
- **The truth source is the oracle again.** Hatari 2.6.1 (`--machine megast|ste`, EmuTOS 256K for the STE) is run as a black
  box with probes (68000 programs) exactly as in Phase 3 (`docs/timing-oracle.md`); functional results (blitter output,
  register read-back, palette bits) must be byte-identical, timing facts are fitted to loop counts. Facts are labelled `[O]`
  with their probe in `hardware-facts.md`; where a document is reachable (Atari Compendium chapter 3 for the Blitter, the
  RP5C15 data sheet, the LMC1992 data sheet) the fact is `[H]`/`[D]` with its SHA-256. Hatari's source is never read.
- **No plugin ABI change.** Mega ST and STE are two more machine ids served by the same plugin library (it already
  declares per-profile options); stereo audio uses the existing `AudioPcm { channels }`; watchpoints are a machine option
  with a report like the I/O stop, not a protocol command. Protocol stays 11.
- **ST/STF/STFM stay byte-identical.** `atari-st` has no Blitter unless asked (`blitter = on`) and no STE registers: every
  existing golden, the fast-mode identity table and the 44 manifest scenarios keep their results.
- One push at the end (Task 71), crates then atari then tools then host.

**Tech Stack:** Rust 2024; Python standard library for scripts; `cargo +nightly fuzz` (nightly toolchain installed once, user
level, authorised).

**Spec:** `docs/superpowers/specs/2026-10-19-atari-st-design.md` §4 (model matrix), §5 (hardware by phase), §12 (video), §13
(audio), §24 (exit criteria), §26 (risks). Previous plan: `docs/superpowers/plans/2026-10-22-atari-st-phase3.md`.

## Global Constraints

The Phase 2 and 3 constraints apply verbatim: clean-room (no Hatari, ASE or EmuTOS code, tables, constants or comments);
every hardware fact labelled `[D] [H] [O] [H?]` with a source row in `machines/atari/docs/hardware-facts.md`, an unsourced I/O
address stops the machine; `#![forbid(unsafe_code)]`, GPL-3.0-or-later, explicit big-endian access, machine time only from
the scheduler (no wall clock, no floats in timing); firmware, media, states, symbols, executables and sources are
untrusted (checked arithmetic, caps, no `unwrap` on input); never commit goldens, firmware, datasheets or disk images;
commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`; probes are `timing/pNN_*.s` (numbers continue
at P27), results at `$C000`, a golden is Hatari's dump (never committed).

Added for Phase 4:
- **Determinism.** The RTC never reads the host clock: `rtc_time` (option, default `2026-01-01 00:00:00`) is the time at
  power-on and the clock then follows machine time. Hatari's RTC comes from the host, so the probes compare register
  behaviour and rates, not the time value.
- **Models are data.** `Model { St, MegaSt, Ste }` selects ROM size and base, the register set that decodes, the Blitter
  default and the RTC. A register of another model is "no fact": it stops the machine as before.
- **Phase 4 exit criteria** (my proposal, as for Phase 3):
  1. EmuTOS 256K boots to its desktop on `atari-ste` and 192K on `atari-mega-st`, in both accuracy modes, against Hatari
     (`--machine ste`, `--machine megast`) pictures under the S1 similarity rule;
  2. the Blitter gives byte-identical memory to Hatari's for a sweep of all 16 HOP x 16 OP combinations with skew, endmasks,
     directions, smudge and line number, and its bus sharing costs the CPU the clocks Hatari's does (within the Phase 3
     tolerances);
  3. the STE Shifter: 12-bit palette read-back, scroll registers, line offset and video counter writes equal Hatari's
     probe dumps; a 4096-colour picture and a fine-scrolled picture match Hatari's screenshots (similarity);
  4. STE DMA sound: registers, frame end and repeat, the MFP wiring and the rate equal Hatari's probes; the output is stereo
     with the tone and level the registers ask for (audio expectation); the LMC1992 volume/balance are applied;
  5. the Mega ST RTC: register read/write and the clock rate equal Hatari's probes, EmuTOS reads and sets it;
  6. watchpoints stop the machine on the named access with a report, bounded cost when none is set;
  7. carry-overs: every fuzz target ran 60 s without a finding (or its finding is fixed with a test); `--disk-save` works for
     ACSI disks; every `known-gap` of Phase 3 has had a second attempt and is closed or re-documented with the new evidence;
  8. `tools/check_all.sh` passes with the new scenarios in both modes; the speed budget of Task 47 still holds
     (`fast` >= 10x, `accurate` >= 2x real time on the idle desktop, also on the STE).

## Probe table (continues the Phase 3 numbering)

| Probe | Measures | Owner task |
|---|---|---|
| P27 | STE Shifter registers: read-back and write masks of `$FF8205-$FF8265`, 12-bit palette bits | 52 |
| P28 | STE scroll and line offset: video counter progression with `$FF820F`/`$FF8265` | 52 |
| P29 | STE DMA sound registers: read-back, counter progression at each rate, frame end/repeat, MFP Timer A/GPIP7 | 53 |
| P30 | Microwire/LMC1992 write protocol: data/mask registers and the shift-out time | 53 |
| P31 | STE joystick ports: `$FF9200-$FF9222` read-back with the pads idle | 54 |
| P32 | RTC (RP5C15) registers: read-back, bank select, 12/24 h, leap year, the one-second rate | 55 |
| P33 | Blitter functional sweep: memory after each HOP/OP/skew/mask/direction combination | 56 |
| P34 | Blitter timing: CPU loop count while a blit runs (HOG on/off, burst length), clocks per word | 57 |
| P35 | Mid-line resolution switch picture (carry-over of P08): pixels after a write at every offset | 62 |

Gaps retried in Task 62 keep their Phase 3 probe numbers.

## Decisions taken (the owner answered them)

1. Models: STE and Mega ST; Mega STE, TT and Falcon stay out.
2. Memory watchpoints in, `.stx` out (no specification with a usable licence; clean-room rule).
3. A nightly toolchain and `cargo-fuzz` are installed (user level) to run the fuzz targets.
4. Known gaps: one more attempt each, then document.

My rulings that need no answer: one push at the end; the ids are `atari-mega-st` and `atari-ste` (spec O7); `atari-st` can
take the Blitter as an option (`blitter = on`), default off; watchpoints are an option, not an ABI change; EmuTOS 256K
(`etos256uk`/`us`, SHA-1 in `firmware.toml`, cached by `--firmware auto`) is the STE firmware.

## Execution protocol (so the run does not stop)

Only four things stop the run: a destructive operation, a security-sensitive action, an outside side effect not yet
authorised (the push and the nightly install are authorised), a plan so broken every path is a guess. Everything else is a
ruling in the ledger (`.superpowers/sdd/2026-10-23-atari-st-phase4/progress.md`). A probe that cannot be matched after two
distinct explanations becomes `status = "known-gap"` with evidence and the run continues; the final message says "not met".

## Review Focus

1. **ST untouched.** The new decode tables, model plumbing and Shifter changes must leave `atari-st` states, frames, I/O traces
   and probes as they are (identity table, S1-S15, the Hatari I/O comparisons).
2. **Blitter bounds.** A blit that runs over RAM, ROM, I/O or the cartridge never panics, writes only RAM, stops the machine
   at an unemulated device with a bounded report, and costs bounded host time per frame (an X count of 65535 x Y count 65535
   with HOG set is capped per frame and resumes).
3. **STE DMA sound bounds.** Frame base/end in ROM, I/O or beyond RAM, odd addresses, end before base, zero-length frames
   never loop forever or write; the audio buffer is capped.
4. **States.** The model, the blitter state, the sound DMA, the RTC and the watch list are in the state header or payload; a
   state loads only into the same model; hostile mutations are refused or load cleanly.
5. **Watch overhead.** With no watch set the bus pays one branch; with a watch the report names the access and stops.

---

## Slice 18: models, the STE video and sound, the RTC

### Task 51: model plumbing, ids, firmware

**Files:** Modify `config.rs` (`Model`), `bus.rs`/`mmu.rs`/`io.rs` (ROM size and base per model, decode per model), `machine.rs`,
the plugin (`machine_profile()` for three ids, options per profile), `firmware.toml` (256K entries), `docs/hardware-facts.md`,
`tests/models.rs`.

**Interfaces:** `Config.model: Model` (default `St`); `AtariSt::new` accepts it; the plugin creates `atari-mega-st` (192K ROM at
`$FC0000`, RTC, Blitter default on) and `atari-ste` (256K ROM at `$E00000` mirrored at 0..7, STE registers, Blitter always).

- [ ] **Step 1: Failing tests** `models::ste_rom_is_mapped_at_e00000_with_the_reset_vector_mirrored`,
  `models::an_st_does_not_decode_ste_registers`, `models::the_plugin_declares_three_machine_ids_with_their_options`,
  `models::a_state_loads_only_into_the_same_model`; the 256K image boots in a bare-metal sense (EmuTOS runs until it reads the
  first STE register: stop, or desktop once Tasks 52-54 are in).
- [ ] **Step 2: Implement; Step 3: run (all atari tests unchanged); Step 4: commit.**

### Task 52: the STE Shifter (probes P27, P28)

**Files:** `crates/chips/st-shifter` (STE registers, 12-bit palette, `render` for 4096 colours), `bus.rs` decode for
`$FF8205-$FF8265`, `waits.rs` (counter with line offset and scroll), `tests/ste_video.rs`, probes `p27_ste_video_regs.s`,
`p28_ste_scroll.s`, scenario S16 (EmuTOS STE desktop), S17 (a 4096-colour strip picture made by a probe, similarity).

**Steps:**
- [ ] **Step 1: Probes P27/P28:** run in Hatari `--machine ste --tos etos256uk.img`; goldens in `RC_GOLDENS/atari-ste/`.
  The probe harness gets `--machine` and `--tos` options in `capture_hatari_mem.py` (tools branch) and a `machine` field in
  the manifest's scenario (`args = ["--model", ...]`; the host option is the machine id).
- [ ] **Step 2: Failing tests** from the dumps: palette bit 3 as the least significant bit of each gun, write masks, video base
  low byte `$FF820D`, line offset `$FF820F`, pixel scroll `$FF8265`, video counter writable (`$FF8205/07/09` on the STE), the
  counter with scroll and offset.
- [ ] **Step 3: Implement**, fast and accurate; the 4096-colour render keeps the 512-colour path byte-identical for the ST.
- [ ] **Step 4: Run; commit.**

### Task 53: STE DMA sound and the Microwire (probes P29, P30)

**Files:** new `crates/chips/ste-sound` (DMA frame registers, rate divider, FIFO, Microwire shifter, LMC1992 mixer state),
`bus.rs`, `events.rs` (a sound event class), plugin audio (stereo), `tests/ste_sound.rs`, probes `p29_ste_dma_sound.s`,
`p30_microwire.s`, scenario S18 (a DMA sample played, tone expectation on the stereo channels).

**Interfaces:** registers `$FF8900` control (play/repeat), `$FF8903/05/07` frame base, `$FF8909/0B/0D` counter, `$FF890F/11/13`
frame end, `$FF8921` mode (rate 6258/12517/25033/50066 Hz, mono/stereo), `$FF8922/24` Microwire data/mask; `Machine::drain_audio`
returns interleaved stereo for the STE (`MachineMetadata` audio format 2 channels); the YM is mixed per the LMC1992 mix setting.

- [ ] **Step 1: Probes** (Hatari `--machine ste`): register read-back, counter at each rate over a measured number of clocks,
  frame end then repeat/stop, the MFP effects (Timer A event count, GPIP7), the Microwire shift (time until the mask register
  returns to 0). **Step 2: Failing tests** from the dumps and `ste_sound::stereo_tone_matches_the_registers` (rms, tone, left
  and right). **Step 3: Implement** (deterministic sample accumulator like the PSG, frame counter driven by scheduler events,
  caps on the sample queue). **Step 4: Run; commit.**

### Task 54: STE enhanced joystick ports and pad names (probe P31)

**Files:** `io.rs`/`bus.rs` (`$FF9200-$FF9222`), `acia.rs`/plugin key table (`Joy1PadA/B/C/Option/Pause/Up/Down/Left/Right`,
paddles), `tests/ste_ports.rs`, probe `p31_ste_ports.s`.

- [ ] **Step 1: Probe** idle read-back. **Step 2: Failing tests** (read-back equals Hatari's, a key name sets the bit the matrix
  selects, the key list contains the pad names only for the STE). **Step 3: Implement. Step 4: commit.**

### Task 55: Mega ST real-time clock (probe P32)

**Files:** new `crates/chips/rp5c15`, `bus.rs` decode `$FFFC20-$FFFC3F` (Mega ST only), the plugin options `rtc_time`, `rtc_mode`
(`fixed`: the clock runs from `rtc_time`; `stopped`), state, `tests/rtc.rs`, probe `p32_rtc.s`.

- [ ] **Step 1: Probe** (Hatari `--machine megast`, `--rtc-year` pinned): register read-back of every nibble, bank select
  (mode register bits), 12/24 h, the leap year register, alarm registers; the one-second rate against the MFP timer.
  **Step 2: Failing tests; Step 3: Implement** (the chip from the RP5C15 data sheet: fetch its SHA-256 and cite sections; the
  time follows machine time); EmuTOS reads and sets the date (`tests/rtc.rs::emutos_reads_the_clock_it_was_given`).
  **Step 4: commit.**

### Task 56: the Blitter, functional (probe P33)

**Files:** new `crates/chips/blitter` (registers `$FF8A00-$FF8A3E`: halftone RAM, source and destination increments and
addresses, endmasks, counts, HOP/OP, line number, skew; the engine over a bus trait), `bus.rs` (decode, RAM access with a cap),
`tests/blitter.rs`, probe `p33_blitter_sweep.s`, scenario S19 (a blitted picture, exact against retroclawd's golden and
similarity against Hatari's).

**Interfaces:** `Blitter::start(&mut self, bus: &mut impl BlitBus) -> BlitProgress`; `BlitBus { read_word(addr), write_word(addr, v) }`.

- [ ] **Step 1: Document fetch:** Atari Compendium chapter 3 "Blitter" (SHA-256 recorded in `CREDITS.md`); transcribe the
  register table into `hardware-facts.md` as `[H]`. **Step 2: Probe P33:** the program blits a fixed source pattern for the
  full HOP x OP x skew x endmask x direction sweep (results hashed per case: a CRC-16 per case, 256+ cases) plus smudge and the
  line-draw mode; Hatari's dump is the golden. **Step 3: Failing tests** per area from the dump (HOP, OP, skew FXSR/NFSR,
  endmasks, X/Y increments including negative, line number/smudge). **Step 4: Implement the engine; Step 5: run; commit.**

### Task 57: the Blitter on the bus (probe P34)

**Files:** `events.rs`/`bus.rs` (bus request: the CPU is stalled by the granted cycles; HOG vs the 64-cycle share), `waits.rs`
constants, `tests/blitter_timing.rs`, probe `p34_blitter_timing.s`.

- [ ] **Step 1: Probe** loop counts per frame while blits of known size run, HOG set and clear, in Hatari. **Step 2: Failing
  tests** (clocks per word per operation class; non-HOG interleave; the interrupt-at-end behaviour: MFP GPIP3). **Step 3:
  Implement** (accurate: the blit advances with machine time in bursts; fast: the blit runs at once and charges the clocks in
  one lump), per-frame cap. **Step 4: run; commit.**

### Task 58: memory watchpoints

**Files:** `bus.rs` (`WatchList` checked in `read_cycle`/`write_cycle` only when non-empty), plugin option `watch`
(`"$1000-$10FF:w,$400:rw"`, up to 16), `diag.rs` (the report), `tests/watch.rs`.

- [ ] **Step 1: Failing tests:** a write into a watched range stops the machine with a report naming address, size, value, PC
  and cycle; a read watch; ranges that overlap; at most 16, bad syntax is an error; the stop is in the state header? (a state
  carries the list). **Step 2: Implement. Step 3: run; commit.**

---

## Slice 19: carry-overs

### Task 59: run the fuzz targets

Install the nightly toolchain and `cargo-fuzz` (authorised). Run `msa`, `state_decode`, `prg`, `ahdi_fat16`, `rcasm68k`,
`rcdis68k`, `accurate_regs` for 60 s each (`cargo +nightly fuzz run <t> -- -max_total_time=60`); a finding gets a RED test, then
the fix; the corpus stays out of git. Ledger: targets, executions, findings.

### Task 60: `--disk-save` for ACSI disks

**Files (host):** `apps/retroclawd/src/main.rs` (flag `--acsi-save <target>=<file>` or the existing `--disk-save` extended), the
plugin export (`Export::DiskImage` drive 8 + target already exported), tests in `apps/retroclawd`; docs.

- [ ] Failing test (stub plugin exports the hard disk image; the host writes it atomically to the file on exit and on the save
  action), implement, run, commit.

### Task 61: IKBD joystick key code mode for stick 1, USART break and parity

**Files:** `crates/chips/ikbd`, `crates/chips/mc68901`, `serial.rs`, tests next to the Phase 2 ones.

- [ ] Failing tests from the protocol document (key code mode applies to joystick 0 only per the document; verify and record,
  implement what the document gives), USART break detect and parity error flags with a file peer injecting them; commit.

### Task 62: a second attempt at every known gap, and P35

One attempt each with a *new* explanation, each recorded in the ledger as closed or kept with evidence:

- P02/P04: interrupt placement (acknowledge length by instruction class, the VBL phase of the line fetch);
- P06: first count after the control write; P09/P10/P12: Hatari's cut and lost lines (model them if a rule fits the sweep);
- P14: the HBL pending at mask removal; P16/P17: the revolution origin (motor start); P18: nothing (loop phase), document;
- P19: the settle and command set-up delays; P20: per-command means by class from more cases; P21 and P22: stay (Hatari
  simplifications), re-document; P35 (mid-line resolution picture) probed and, if it matches, closed as a Phase 3 limit.

Each closed gap changes its manifest entry to exact and the fast identity table is re-recorded only if state bytes change.

---

## Slice 20: integration and exit

### Task 63: scenarios and manifests for Phase 4

`atari-st.manifest.toml` (and `machines/atari/atari-ste.manifest.toml` if the runner needs one per machine id): S16 STE desktop
(fast and accurate), S17 4096 colours, S18 DMA sound (audio), S19 blitter picture, S20 Mega ST desktop, S21 RTC (EmuTOS clock
set and read back), the probes P27-P35 as memory scenarios, accurate variants where the picture can differ.

### Task 64: determinism, states and games on the new models

`tests/ste_determinism.rs` (three runs, state round trip mid-frame with the blitter and DMA sound running, host interleavings);
the three RomM games boot on `atari-ste` and `atari-mega-st` without a stop in both modes (they run on ST hardware: no stop
and a frame each; no assertion that they look right); hostile tests: `hostile_blitter.rs`, `hostile_sound.rs`
(`RC_SWEEP=40`).

### Task 65: speed

`bench` gains `--model`; the budget holds for the STE desktop (fast >= 10x, accurate >= 2x); optimise the blitter loop or the
sound event rate if it misses.

### Task 66: documentation

`hardware-facts.md` (each new row labelled with its probe/document), `timing-oracle.md` (STE and Mega ST capture options),
`timing/README.md` (P27-P35), `CREDITS.md` (Atari Compendium chapter 3, RP5C15, LMC1992, EmuTOS 256K), atari `CLAUDE.md`,
`scenarios/README.md`; host: spec Status "Phase 4 done" with the exit table, `TODO.md` (what is next: nothing planned; list the
unreachable), host `CLAUDE.md` paragraph.

### Task 67: check_all, pins, self-review, push

Push `atari-phase4` to crates `main`; set the atari and tools pins (local patch config aside, refresh locks); `tools/check_all.sh`;
self-review of the whole branch against the Review Focus; push in the order crates, atari (`main`), tools (`main`), host
`master` (re-check `git ls-tree HEAD` for the pointers; never `git add -A`; the host's untracked agent-config files are not
committed). The final message lists Rulings, Deferred minors, the exit criteria with evidence, the known gaps still open and
that the review was a self-review.

---

## Self-review of this plan

- **Spec coverage (§5 Phase 4 column):** Mega/STE bus differences -> 51; STE Shifter -> 52; STE DMA sound and Microwire -> 53;
  enhanced ports and Jaguar pad key names (§5 input table) -> 54; RTC -> 55; Blitter -> 56-57; watchpoints (Phase 2 decision
  1) -> 58; the Phase 3 deferrals -> 59-62.
- **Open sources:** Atari's hardware specification stays unreachable; Blitter, RTC and LMC1992 facts come from the Compendium,
  the data sheets and Hatari probes. Anything a probe cannot reach stays `[H?]` with the choice written down.
- **Type/name consistency:** `Model` in 51 is used by 52-57; `BlitBus` in 56 by 57; the sound event class in 53 by 57's
  scheduler ordering.
- **Placeholders:** none; where a value is fitted it names the probe and the sweep.
