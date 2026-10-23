# Atari ST Phase 3 (slices 13-17) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the spec's Phase 3 column (§5): a 68000 whose bus cycles match the reference tests exactly, an
`accurate` mode in which every bus cycle advances machine time and devices are brought up to date at the cycle of the
access, bus wait states, a cycle-exact Shifter (video address counter, mid-line palette and resolution changes,
left/right/top/bottom border removal, Spec512), exact MFP/ACIA/IKBD/YM/DMA/FDC/ACSI timing, and the mono monitor.
Phase 3 ends when `--accuracy accurate` reproduces every timing probe of this plan exactly as the reference emulator
does, runs EmuTOS and the owner's games without a stop, and `check_all` runs both modes.

**Architecture:**
- Same repositories and layout as Phase 2. The CPU bus work is in `retroclawd-crates`; everything else is in
  `retroclawd-atari`; probe tooling in `retroclawd-tools`; the host changes only for the one new plugin option the
  frame canvas needs (Task 37) and for docs and pins.
- **The truth source is an oracle, not memory.** Atari's hardware specification is still unreachable and the timing
  facts of this phase (wait states, beam position, border-opening cycles, interrupt latency) are not in any datasheet.
  Every such fact comes from a **probe**: a small 68000 program, assembled by `rcasm`, started from a boot floppy,
  that writes its measurements into RAM. The same floppy runs in Hatari 2.6.1 (a black box; its memory is dumped
  with the debugger's `savebin` after N VBLs — verified 2026-10-22 with `--parse`) and in retroclawd; the dumps must
  be byte-identical. Facts are labelled `[O]` with the probe's name. Hatari's source is never read.
- **Fast mode must not change.** Every existing golden (S1-S11, the EmuTOS I/O traces) stays byte-identical in
  `fast` mode through the whole phase; that is the regression net for every refactor below.
- Five slices, each ending with a green `check_all` and probes that match: 13 oracle + CPU bus cycles, 14 the
  accurate time base and bus timing, 15 the cycle-exact Shifter, 16 peripherals' timing, 17 integration and exit.
  Slices run strictly in order (each consumes the previous slice's interfaces).
- **No plugin ABI bump.** The new options (`borders`, mono `monitor`) use the existing option kinds; the canvas size is
  reported through the existing `MachineMetadata`. Protocol stays 11.

**Tech Stack:** Rust 2024; Python standard library for scripts; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-19-atari-st-design.md`: §5 (hardware by phase), §7.3 (conformance gate),
§8 (clock, scheduler, fast/accurate), §9, §11 (fast/accurate design), §12 (video), §19 (save states), §21 (testing,
benchmarks), §24 (exit criteria), §26 (risks). Previous plan: `docs/superpowers/plans/2026-10-22-atari-st-phase2.md`.
Phase 4 (Blitter, STE, Mega ST RTC, the STE Shifter) is a separate plan and is out of scope here.

**Repositories and branches:**

| Repository | Branch | Tasks |
|---|---|---|
| retroclawd-crates | `atari-phase3` | 27-28 (`mc68000` bus cycles) |
| retroclawd-atari | `atari-st-slice13` ... `atari-st-slice17` (one per slice, merged to `main` locally before the next starts) | all others |
| retroclawd-tools | `atari-phase3-tools` | 26 (`capture_hatari_mem.py`), 47 (bench driver if needed) |
| retroclawd (host) | `master` | 37 (canvas), 50 (docs, pins) |

## Global Constraints

The Phase 2 plan's Global Constraints still apply verbatim:
- clean-room: no Hatari, ASE or EmuTOS code, tables, constants or comments copied; Hatari stays a black box;
- every hardware fact has a label (`[D]`, `[H]`, `[O]`, `[H?]`) and a source row in
  `machines/atari/docs/hardware-facts.md`; an unsourced I/O address stops the machine instead of returning an invented
  value;
- `#![forbid(unsafe_code)]`, GPL-3.0-or-later, big-endian access explicit, machine time only from the scheduler (no
  wall clock, no floats in timing);
- all firmware, media, states, symbols, executables and sources are untrusted: checked arithmetic, size caps, no
  `unwrap` on input, atomic writes;
- never commit goldens, firmware, datasheets or disk images, and never print their contents in logs;
- EmuTOS is the firmware for CI; commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.

Added for Phase 3:
- **Oracle gate.** A task that adds a timing behaviour starts with "write the probe, record Hatari's dump", then the
  failing retroclawd test (the dump as the expected value), then the implementation. A constant is never chosen by
  reasoning; it is fitted to the probe sweep (all offsets, not one) and the sweep is the test. A fact the probes
  cannot reach stays `[H?]` with the choice written down.
- **Probe contract.** A probe is `machines/atari/timing/pNN_name.s`: boot-sector program, supervisor mode, interrupts
  masked (`SR=$2700`) after setup, results at `$A0000` (user mode cannot write below `$800`; `$A0000` is above TOS's
  working area, as in `tests/prg.rs`), a final `bra.s *`. It states at its top what it measures and the byte length
  of its result. Built by `timing/build.sh` into the git-ignored `software/scenarios/timing/pNN.st`. Hatari's dump is
  a golden (never committed, `RC_GOLDENS`), retroclawd's is compared to it exactly.
- **Bounded cost.** A guest that writes a video or sync register on every cycle of a frame, or runs a stalled bus
  forever, costs bounded memory and bounded host time per frame (logs capped as in Phase 2; no allocation per cycle).
- **Fast-mode identity.** After every task of slices 14-16: `cargo test --workspace` and the manifest in `fast` mode
  give the same frame hashes as before the task.
- **Phase 3 exit criteria** (my proposal; spec §24 defines only Phase 1, so this is a ruling for you to confirm):
  1. `mc68000`: SingleStepTests pass with the **bus transaction comparison on** (every file, `EXPECTED` list for the
     documented disagreements only), where today 557 843 of 983 930 cases do;
  2. every probe of the table below except P18 (P01-P17, P19, P20) gives byte-identical results in Hatari and in retroclawd (`accurate`);
  3. mid-line palette (Spec512-style) and left/right/top/bottom border pictures match Hatari's `--borders on`
     screenshots under the same `similarity` rules as S1/S10, and are exact against retroclawd's own goldens;
  4. the mono monitor works (EmuTOS mono desktop, frame length measured);
  5. `accurate` mode: the EmuTOS desktop is reached with identical frame hashes on 3 runs, a state saved at frame N and
     loaded into a fresh machine matches a straight run, and the machine runs at least the Task 47 speed budget;
  6. the owner's RomM games (`tests/romm_games.rs`) boot without a stop in both modes;
  7. `tools/check_all.sh` includes the new scenarios in both modes and passes; every new parser or log has a bound and
     a hostile-input test.

## Probe table

| Probe | Measures | Owner task |
|---|---|---|
| P01 | harness smoke: constant bytes in RAM | 26 |
| P02 | instruction timing: loops of NOP, ADD, MOVE, MULU, DIVU, shifts, MOVEM, exceptions, with RAM, ROM and I/O operands | 31 |
| P03 | wait-state alignment: the same loop at every 2-cycle phase against the beam | 31 |
| P04 | video address counter `$FF8205/07/09` along a line and a frame | 30 |
| P05 | HBL and VBL assertion cycle, interrupt latency from a timer, IACK length, IPL sampling | 32 |
| P06 | MFP timer data-register reads, prescaler phase after a control write, timer-to-IRQ delay, event-count mode | 33 |
| P07 | mid-line palette write: which pixel changes, for every cycle offset 0-511 | 35 |
| P08 | mid-line resolution write: pixel effect for every offset | 35 |
| P09 | left border opening: hi/lo resolution toggle at every offset, the line's length and counter | 36 |
| P10 | right border opening, stabiliser (extra toggles), line length 230/224 bytes | 36 |
| P11 | top and bottom border: 50/60 Hz toggle at every line and offset | 36 |
| P12 | mono frame: lines per frame, cycles per line | 38 |
| P13 | DMA FIFO: CPU stall length and position during a floppy sector read | 40 |
| P14 | FDC rotation: index-to-index cycles, header search time, step and settle times | 41 |
| P15 | ACIA/IKBD: cycles from a command byte to its answer, bit-clock phase | 42 |
| P16 | YM2149: register write to first changed output sample, envelope/tone phase | 42 |
| P17 | ACSI: cycles from the last command byte to the interrupt, transfer cycles per block | 43 |
| P18 | BIOS-level: cycles for EmuTOS to reach its desktop (boot-time total); informational, a difference is a clue for the ledger, not a gate | 46 |
| P19 | printer and RS-232 byte times | 42 |
| P20 | combined: a game-style loop (raster palette, timer B, sound) over 200 VBLs | 46 |

## Decisions for the owner (my recommendations; approving the plan approves them)

1. **Accuracy defaults.** `fast` stays the default (speed, existing goldens); `accurate` is opt-in (`--accuracy
   accurate`). Recommend: keep.
2. **What "exact" means.** Agreement with Hatari 2.6.1 on the probes is the measure, because no cycle-level hardware
   document is reachable. Hatari is itself an emulator; where its behaviour is a documented approximation, the probe
   shows it and the fact is labelled `[O]`, not `[D]`. Recommend: accept; revisit when Atari's specification is
   obtained.
3. **Wake-up states.** Real STs differ by one or two cycles of Shifter/MMU phase between power-ups; Hatari offers
   several. Recommend: model the single state Hatari uses by default; list the others as deferred.
4. **Canvas.** Default stays the 640x400 picture (no border) so every existing golden stands; a new option
   `borders = on` draws the full overscan canvas (size measured in Task 35). Recommend: yes.
5. **Mono monitor.** Include (Task 38): the only missing fact is the frame length, which a probe measures.
6. **Pushing.** Recommend: one push at the end, in the Phase 2 order (crates, atari, tools, host), after `check_all`
   passes in both modes. Approving this plan authorises that push, so the run does not stop to ask. Nothing is pushed
   before the final task.

## Execution protocol (so the run does not stop)

Only four things stop the run: a destructive operation, a security-sensitive action, an outside side effect not yet
authorised (the push is authorised by decision 6), or a plan so broken every path is a guess. Everything else is a
ruling written to the ledger. In particular:
- **Hatari disagrees with a reading of a document** → Hatari wins, label `[O]`, note the disagreement in
  `hardware-facts.md`.
- **A probe cannot be made to match after two distinct explanations** → write the explanation tried, keep the closest
  model, add the probe to the manifest as `status = "known-gap"` with the measured difference, and continue. Exit
  criterion 2 then lists that probe; do not stall on it. (A gap in a P-number the exit criteria name is reported in the
  final message as "not met", never hidden.)
- **A shared-crate test (SingleStepTests) disagrees with the manual** → the manual wins and the case is listed in
  `EXPECTED` with its source, as in Phase 1.
- **Speed budget missed** → Task 47 owns the optimisation pass (event batching, idle-loop fast path); a missed budget
  after that is reported, not hidden.

## Review Focus

1. **Fast mode untouched.** The refactors of slice 14 move the scheduler into the bus and change how `now` advances.
   Fast mode's frame hashes, I/O traces, state bytes and instruction timing must be identical; Task 29's test compares
   all S1-S11 hashes before and after.
2. **Event storms.** A guest toggling resolution or sync on every cycle of a frame, or a palette write every 4 cycles
   for a whole frame, costs bounded memory and time: the per-line event list is capped and the extra events are
   dropped oldest-first with a counter in the diagnostics. Tests in Tasks 34-36.
3. **Time never goes backwards.** In `accurate` mode a device is caught up to the access's cycle exactly once;
   events are processed in `(time, class, seq)` order; a property test drives random access sequences and checks
   monotonic device clocks (Task 29).
4. **States across modes.** An `accurate` state does not load into a `fast` machine and the reverse (refused with a
   message naming both), the new fields are validated, and a hostile state cannot make the Shifter timeline or the
   DMA FIFO exceed its bounds (Task 33's schema bump, Task 49's mutation sweep).
5. **DMA and the bus.** A DMA transfer to the edge of RAM, an odd address, ROM or I/O never stalls the CPU forever,
   never writes outside RAM, and ends with the error bit set (Task 40).

---

## Slice 13: oracle and the CPU's bus cycles

### Task 26: the timing oracle (retroclawd-tools, retroclawd-atari)

**Files:**
- Create: `retroclawd-tools/scripts/capture_hatari_mem.py`, `machines/atari/timing/build.sh`,
  `machines/atari/timing/p01_smoke.s`, `machines/atari/timing/README.md`, `machines/atari/docs/timing-oracle.md`
- Modify: `machines/atari/atari-st.manifest.toml` (P01), `machines/atari/docs/hardware-facts.md` (a "Timing (Phase 3)"
  section header and the method)
- Test: `machines/atari/crates/machines/atari-st/tests/timing_probes.rs` (a loader that runs `software/scenarios/
  timing/pNN.st` headless and compares its RAM window to `RC_GOLDENS/atari-st/timing/pNN.bin`; passes with a note
  when either is absent, like the EmuTOS tests)

**Interfaces:**
- `capture_hatari_mem.py ROM FLOPPY VBLS ADDR LEN OUT.bin [--hatari PATH] [--memsize MIB] [--video-timing ...]`:
  runs Hatari with `--parse`, a breakpoint `b VBL = VBLS :trace :once :file FILE` whose FILE is `savebin OUT ADDR
  LEN` + `quit` (probe verified 2026-10-22), `--cpu-exact` as Hatari's default, `--fast-forward on`, no sound, and
  prints the options it used.
- Produces: `timing_probes::run_probe(name, vbls, addr, len) -> Vec<u8>` (retroclawd side) and
  `timing_probes::assert_matches_hatari(name)`; every later task adds `#[test]`s that call it.

**Steps:**
- [ ] **Step 1: Source the facts** about the oracle: Hatari's cycle-exact switch and defaults (`--cpu-exact`,
  video-timing / wake-up option and its default), what `hatari --trace help` lists for video and CPU, and whether the
  boot-sector floppy path matches how retroclawd boots a floppy. Write `timing-oracle.md` (method, options, the
  Hatari defaults used, how to regenerate a golden).
- [ ] **Step 2: Failing test:** `timing_probes::p01_smoke_matches_hatari` (P01 writes 16 known bytes at `$A0000`).
  With the golden present and the machine not yet building the floppy, it fails (floppy missing).
- [ ] **Step 3: Implement** the capture script, `timing/build.sh` (uses `rcasm --cpu 68000`, `rcld`/`rcdisk boot` as
  `scenarios/build.sh` does), P01, the test loader and the manifest entry (`kind = "memory"`, `reference =
  "hatari"`).
- [ ] **Step 4: Run** `timing/build.sh`, capture P01 into `../goldens/atari-st/timing/p01.bin`, run the test.
  Expected: PASS; the test also passes (with a note) when the golden is absent.
- [ ] **Step 5: Commit** both repositories ("Phase 3: the timing oracle harness").

### Task 27: `mc68000` bus transactions equal SingleStepTests, part 1 (retroclawd-crates)

**Files:** Modify `shared/crates/core/68000/src/{cpu,ea,ops/*,exception}.rs`, `tests/singlestep.rs` (bus comparison
on by default when data is present; `RC_SST_BUS=0` turns it off), `tests/singlestep-expected.md`.

Today (measured 2026-10-22, `RC_SST_BUS=1`): 557 843 of 983 930 cases pass; 27 files pass entirely, 19 pass none (all
shifts and rotates by register or memory count `ASL/ASR/LSL/LSR/ROL/ROR/ROXL/ROXR` `.b/.l`, `EXG`, `RTE`, `RTR`), the
others fail part of their cases (the position of internal clocks between accesses, or the order of two accesses).

**Interfaces:** `Cpu::step` keeps its signature. The sequence of `M68kBus` calls it makes (`read`, `write`, `sync`,
`iack`) becomes exactly the SST transaction list: same accesses in the same order with the same function codes,
and `sync(n)` calls whose sums between accesses equal the `n` entries. No `M68kBus` change.

**Steps:**
- [ ] **Step 1: Failing tests:** make bus comparison the default; run `cargo test --release -p mc68000 --test
  singlestep --no-fail-fast`; record the per-file pass counts in `singlestep-expected.md` as the starting point.
  Expected: FAIL on 95 files.
- [ ] **Step 2: Fix the 19 all-fail files first** (a shared helper explains most of them: the idle clocks of a
  register-count shift are `2n` and sit after the opcode fetch; `RTE`/`RTR` read order). One commit per family.
- [ ] **Step 3: Fix the partial files by failing count**, largest first; after each family run the whole suite (the
  functional comparison must not regress: the gate from Phase 1 stays 100%). `RC_SST_PRINT=1` prints the first
  difference per case; a pattern across many cases is a rule to find, not a case to special-case.
- [ ] **Step 4: Run** the full suite, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt`. The
  `EXPECTED` list may only gain entries with a source (a case where the test data disagrees with the Programmer's
  Reference Manual), never to hide an unexplained difference.
- [ ] **Step 5: Commit** per family on `atari-phase3`; ledger line with the pass count.

### Task 28: exception and interrupt bus order, IPL sampling point (retroclawd-crates)

**Files:** Modify `shared/crates/core/68000/src/exception.rs`, `cpu.rs`; Test: `tests/exceptions.rs`,
`tests/interrupt_timing.rs`

SingleStepTests have no files for interrupts, trace, STOP or the group 0 frames (spec §7.3), so these come from the
Programmer's Reference Manual's exception timing tables (source them: section and table in the test comments).

**Interfaces:** `Cpu::step` documents and implements: the interrupt level is sampled once per instruction at the
moment before the next instruction's first fetch, using the level most recently given to `set_ipl`; the interrupt
acknowledge sequence's bus order (internal clocks, the IACK read, the stacking writes, the vector read, the two
refill reads) and clock total are the manual's. A new `Cpu::ipl_sample_point()` is not needed: the machine decides
when to call `set_ipl` (Task 32).

**Steps:**
- [ ] **Step 1: Source** the manual's tables (interrupt processing, address/bus error, trace, TRAP, CHK, DIV0, STOP,
  RESET instruction clocks). **Step 2: Failing tests** `interrupt_timing::autovector_interrupt_takes_44_clocks_in_
  the_manual_order` (use the manual's figure, check it is 44 before asserting), `trap_#n`, `chk_exception`,
  `divide_by_zero`, `address_error_group_0_frame`, `stop_wakes_on_interrupt_and_costs_the_manual_clocks`.
  **Step 3: Implement.** **Step 4: Run** the crate's tests and the SST suite (no regression). **Step 5: Commit.**
  Merge `atari-phase3` into the crates branch the atari repository pins (local pin patch for development, as in
  Phase 2; the real pin bump is Task 50).

**Slice 13 gate:** the SST suite with bus comparison passes (criterion 1); `cargo test --workspace` of `atari`
(local patch) unchanged; merge `atari-st-slice13`.

---

## Slice 14: the accurate time base

### Task 29: scheduler into the bus; `Accurate` follows every cycle

**Files:** Modify `crates/machines/atari-st/src/{machine,bus,scheduler,config}.rs`; Test:
`crates/machines/atari-st/tests/accurate_time.rs`, `tests/fast_identity.rs`

**Interfaces:**
- `StBus` owns the `Scheduler` and `fn run_events_until(&mut self, t: u64)` (what `AtariSt::advance` does today for
  event handling); `AtariSt::step` only calls `cpu.step(&mut bus)` and `bus.finish_instruction(clocks)`.
- `M68kBus for StBus`: with `accurate == false` exactly today's behaviour (instruction-start timestamps,
  `sync` empty). With `accurate == true`:
  - `read`/`write`: `begin_access(addr)` charges the wait states (Task 31, zero until then), processes events up to
    `now`, performs the access with devices caught up to `now + ACCESS_POINT` (a constant fitted in Task 31, 0 at
    first), then `now += 4`;
  - `sync(n)`: `now += n` and events due are processed;
  - `iack(level)`: charges the acknowledge cycles Task 32 fits.
- `Accuracy` is a runtime flag on `StBus` (decision recorded in the ledger: a type parameter would duplicate the
  device code; the cost is one predictable branch per access; re-evaluate in Task 47).

**Steps:**
- [ ] **Step 1: Failing tests:** `fast_identity::all_manifest_scenarios_hash_the_same_as_the_recorded_table` (record
  frame/RAM/I/O-trace hashes of S1-S11 and the 1500-frame EmuTOS I/O trace *before* the refactor into
  `tests/fast_identity.table`, git-ignored? no: the table is hashes only and committed);
  `accurate_time::a_device_is_caught_up_to_the_cycle_of_the_access` (a test bus: write MFP timer, read its counter
  after 1, 2, 3 NOP-equivalents: the value changes at the exact cycle in `accurate`, only at instruction starts in
  `fast`); `accurate_time::events_run_in_time_class_seq_order_and_time_never_decreases` (property test, random
  accesses);
  `accurate_time::accurate_and_fast_agree_when_nothing_timing_sensitive_is_touched` (a RAM-only program: same RAM,
  same register state, same final `now`).
- [ ] **Step 2: Run** the identity test on the *current* code first and record the table (it must pass now); then
  write the other tests. Expected: new ones FAIL.
- [ ] **Step 3: Implement** the move; keep `fast` byte-identical. **Step 4: Run** `cargo test --workspace` (with
  `RC_EMUTOS`, `RC_GOLDENS`) and the manifest. **Step 5: Commit.**

### Task 30: the video address counter `$FF8205/07/09` and the line timeline (st-shifter)

**Files:** Create `crates/chips/st-shifter/src/timeline.rs`; Modify `lib.rs`, `io.rs`, `bus.rs`; Test:
`crates/chips/st-shifter/tests/timeline.rs`, probe P04 test

**Interfaces:**
- `LineTimeline { display_start: u16, display_end: u16, bytes: u16, … }` and the pure function
  `counter_at(&self, cycle_in_line: u16, frame_base: u32, line: u16) -> u32`: the Shifter's video counter (the
  address of the next word it will fetch) at a cycle of a raster line, from the line's timeline. For an ordinary
  line: `display_start`/`display_end` bound the fetch window (fitted from P04); the counter moves one word per 4
  cycles inside it (low res), and holds outside; the frame base is reloaded at the frame start.
- Registers: `$FF8205` high, `$FF8207` mid, `$FF8209` low, byte reads, read-only (a write is ignored; `[O]`
  behaviour fitted in P04). Reading them is an `accurate` feature; in `fast` they return the value for the
  instruction-start cycle, which is documented as less exact.

**Steps:**
- [ ] **Step 1: Source/probe.** Write P04: sync to the frame start through `$FF8209`, then read the three bytes at a
  sweep of delays (a NOP ladder, steps of 2 cycles over 600 cycles, over three lines and in all three resolutions);
  capture Hatari's dump. Record in `hardware-facts.md`: window start and end in cycles, the step per word, the
  per-resolution rates, the value outside the window.
- [ ] **Step 2: Failing tests:** `timeline::counter_follows_the_fitted_window_and_rate` (values from the dump),
  `timeline::outside_the_window_the_counter_holds`, `timing_probes::p04_matches_hatari`.
- [ ] **Step 3: Implement** until P04 matches in `accurate`. **Step 4: Run** crate tests, fast identity. **Step 5:
  Commit.** The EmuTOS I/O-trace tests must still pass: TOS never reads these registers, so a read must not appear
  in the Hatari I/O log it is compared with (check the comparison rules in `tests/common/mod.rs`).

### Task 31: bus wait states and `ACCESS_POINT`

**Files:** Modify `crates/machines/atari-st/src/bus.rs`, new `crates/machines/atari-st/src/waits.rs`; Test:
`tests/waits.rs`, probes P02, P03

**Interfaces:** `waits::extra_clocks(region: Region, addr: u32, size: Size, cycle_in_line: u16) -> u32`: the clocks
a bus cycle is lengthened by so that it ends on the boundary the MMU/Shifter slot imposes (RAM), the peripheral
(I/O) delay, and none for ROM per the probes; `ACCESS_POINT`: the clock within the 4-clock bus cycle at which a
device samples the access (fitted).

**Steps:**
- [ ] **Step 1: Probes.** P02: for a set of instructions and operand regions (RAM, ROM, MFP, Shifter, PSG, DMA),
  measure by the beam counter (Task 30) the clocks of a loop of N repeats for N = 1..32 at several starting phases;
  P03: the same loop with its start moved one cycle at a time over 16 cycles, to show the alignment rule. Capture
  Hatari's dumps. Record the rules found (a RAM word access is rounded up to a multiple of 4 from the line start; an
  I/O access waits until..., etc.) as `[O]` rows with the probe names.
- [ ] **Step 2: Failing tests:** `waits::ram_access_ends_on_a_four_clock_boundary`, `waits::io_access_cycle_counts_
  match_the_probe_table`, `timing_probes::p02_matches_hatari`, `timing_probes::p03_matches_hatari`.
- [ ] **Step 3: Implement** `extra_clocks` and apply it in `begin_access` (accurate only). Fit `ACCESS_POINT` with the
  MFP-timer and video-counter reads of P02. **Step 4: Run** crate tests; fast identity; EmuTOS boot in `accurate`
  reaches the desktop (a new test, headless 1500 frames). **Step 5: Commit.**

### Task 32: HBL/VBL cycle, IPL sampling, IACK cycles

**Files:** Modify `scheduler.rs`, `machine.rs`, `bus.rs`; Test: `tests/interrupt_timing.rs`, probe P05

**Interfaces:** the HBL request is raised at the probe-fitted cycle of the line (today at the line end), the VBL at
the fitted line and cycle; `StBus::ipl()` is passed to `cpu.set_ipl` at the sampling point Task 28 defined (before the
next instruction's first fetch, in `accurate`); `iack` charges the fitted cycles (autovector for GLUE, vector for the
MFP).

**Steps:**
- [ ] **Step 1: Probe P05:** an HBL/VBL handler that records the beam counter and a free-running MFP timer on entry;
  a timer-C interrupt handler that does the same; an instruction sweep that places the interrupt request at every
  clock of a long instruction (`MULU`, `DIVS`, `MOVEM`) and records the handler entry time. Capture Hatari's dump.
- [ ] **Step 2: Failing tests** from the dump: `interrupt_timing::hbl_handler_entry_cycle_matches_the_probe`,
  `…vbl…`, `…mfp_timer_irq_latency_for_every_offset_of_a_long_instruction…`, `timing_probes::p05_matches_hatari`.
- [ ] **Step 3: Implement. Step 4: Run** (fast identity, EmuTOS boot in both modes). **Step 5: Commit.**

### Task 33: MFP timer phase; accurate-mode state (schema bump)

**Files:** Modify `crates/chips/mc68901/src/{lib,timer}.rs`, `mfp_clock.rs`, `state.rs`; Test:
`crates/chips/mc68901/tests/phase.rs`, `tests/state_accurate.rs`, probe P06

**Interfaces:** the prescaler phase after a control-register write, the data-register read value as a function of the
exact access cycle, the delay from a timer reaching zero to the IRQ line, and event-count mode, all as fitted from P06;
`StatePayload` gains the accurate-mode fields (Shifter line timeline events so far, DMA FIFO fill, bus time inside the
current line) with a schema version bump and the decode checks of Phase 2's Task 6; `accuracy` stays part of the
envelope header, and loading across modes stays refused.

**Steps:**
- [ ] **Step 1: Probe P06**: write the control register at each of 13 phases of the prescaler, read the data register
  at 64 delays; record timeouts; timer-to-IRQ latency; Hatari dump.
- [ ] **Step 2: Failing tests:** `phase::data_register_read_matches_the_probe_for_every_prescaler`, `state_accurate::
  an_accurate_state_round_trips_to_the_same_frame_hash`, `…::a_fast_state_is_refused_by_an_accurate_machine_and_the_
  reverse`, `…::hostile_timeline_fields_are_refused` (counts above the cap, a time in the past, a line beyond the
  frame), `timing_probes::p06_matches_hatari`.
- [ ] **Step 3: Implement. Step 4: Run** everything; the Phase 2 state tests still pass for `fast` states of the old
  schema (a v1 state loads into a `fast` machine; document if the schema break forces refusal instead and say so in
  the ledger). **Step 5: Commit; merge slice 14.**

---

## Slice 15: the cycle-exact Shifter

### Task 34: line timeline events and the exact renderer

**Files:** Modify `crates/chips/st-shifter/src/{lib,timeline}.rs`, `machine.rs`; Test:
`crates/chips/st-shifter/tests/exact_render.rs`

**Interfaces:**
- `Shifter::write_at(addr, word, value, now)` (exists) records, per line, a `LineEvent { cycle: u16, kind }` with
  `kind` in `Palette{index,value} | Resolution(v) | Sync(v)`, capped at `MAX_LINE_EVENTS` (256) per line with a drop
  counter surfaced in diagnostics.
- `Shifter::render_exact(monitor, first_line, ram, out)` replaces the line-batch renderer when any line has events or
  a non-default timeline; lines without events render through the existing fast path (identical bytes).
- `LineTimeline::from_events(events: &[LineEvent], mode: Resolution, sync_hz: u8) -> LineTimeline` is the single
  function that turns the events of a line into its fetch window, line length in bytes and pixel-switch points
  (Tasks 35-36 add the rules; this task adds the structure and the unchanged ordinary line).

**Steps:**
- [ ] **Step 1: Failing tests:** `exact_render::a_frame_with_no_events_renders_exactly_as_the_line_renderer`,
  `exact_render::events_beyond_the_cap_are_dropped_and_counted`, `exact_render::events_apply_in_cycle_order_not_
  write_order_within_a_cycle_tie`.
- [ ] **Step 2-4: Run, implement, run** (fast identity; S1/S10 hashes). **Step 5: Commit.**

### Task 35: mid-line palette and resolution changes; canvas size

**Files:** Modify `st-shifter/src/{timeline,lib}.rs`; Test: `crates/chips/st-shifter/tests/midline.rs`, probes P07,
P08, scenario S12

**Interfaces:** a palette write at cycle `c` of a line changes the colour of the pixels fetched from the fitted
`c + latency` onward (Spec512 needs this at 4-cycle spacing); a resolution write at cycle `c` changes the mode from
the fitted pixel (a mid-line switch low→medium and back, giving the fitted shift). `latency` is a constant per
register class fitted from the sweeps. The canvas size for `borders = on` is read from Hatari's screenshot of the
border-on desktop (`--borders on` at the probe's resolution) and recorded as `[O]`.

**Steps:**
- [ ] **Step 1: Probes P07 and P08:** a program fills the screen with a known pattern, writes a palette entry (or the
  resolution) at every offset 0-511 of one line per frame (offset advances each VBL), over a 512-frame run; Hatari's
  screenshot at selected VBLs (the offsets of interest) plus a memory-kind dump of the beam-synchronised write
  offsets. Record the first changed pixel per offset.
- [ ] **Step 2: Failing tests:** `midline::palette_write_at_each_offset_changes_the_pixel_the_probe_shows` (table
  from the probe), `midline::a_resolution_switch_pair_shifts_the_line_by_the_probe_amount`,
  `midline::spec512_style_16_changes_in_a_line_all_apply`; scenario S12 (`similarity` against Hatari's screenshot,
  `max_mad` from measurement as in S10).
- [ ] **Step 3: Implement** until S12 and the tables pass. **Step 4: Run** all. **Step 5: Commit.**

### Task 36: left, right, top and bottom border removal; line length

**Files:** Modify `st-shifter/src/timeline.rs`, `machine.rs` (line length changes the next line's start of fetch),
`scheduler.rs`; Test: `tests/overscan.rs`, probes P09-P11, scenarios S13 (sides), S14 (top/bottom)

**Interfaces:** `LineTimeline::from_events` implements the rules the sweeps show: which resolution/sync write cycle
windows open the left border, the right border, the stabiliser's effect, the fetched byte count of the line (160,
230, 224, 204, …) and the resulting video counter at the next line; top and bottom border removal by the 60 Hz write
at the fitted line and cycle window and the number of lines drawn. Rules are tables keyed by the fitted windows, not
heuristics.

**Steps:**
- [ ] **Step 1: Probes P09-P11:** for each effect, a program that writes the switching register at every cycle offset
  in a window (one offset per frame or per line, whichever the effect needs), reads `$FF8209` at the start of the
  next line to record the line length, and leaves the picture for Hatari's `--borders on` screenshot. Record, per
  offset: opened or not, bytes fetched, counter. Build the tables in `hardware-facts.md`.
- [ ] **Step 2: Failing tests:** `overscan::left_border_opens_exactly_for_the_probe_offsets`,
  `…right_border…`, `…stabiliser_toggle_keeps_the_line_stable…`, `…line_length_in_bytes_matches_the_counter_probe…`,
  `…top_border…`, `…bottom_border…`, `…a_hostile_program_toggling_every_cycle_is_bounded…`; `timing_probes::
  p09..p11_match_hatari`; scenarios S13 and S14 against Hatari's border pictures.
- [ ] **Step 3: Implement** the rule tables; if one effect needs more than two attempts, follow the execution
  protocol (known-gap with the measured difference), not a stall. **Step 4: Run** all. **Step 5: Commit.**

### Task 37: the `borders` option and the overscan canvas (host and atari)

**Files:** Modify `crates/plugins/atari-st-machine-plugin/src/lib.rs` (option `borders = off|on`, default `off`;
`MachineMetadata` reports the canvas), `machine.rs` (`render` chooses canvas), host `apps/retroclawd` only if the
GUI cannot take a size change at reset (it can: CPC changes size at runtime, host CLAUDE.md "CPC Gate Array mode
changes"); Test: `tests/canvas.rs`, plugin test

**Interfaces:** `borders = on` renders the full raster window found in Task 35 (colour 0 or the palette's border
colour in the border; the border colour changes mid-frame render per line); `off` is byte-identical to today.
The option is part of the state header (a state saved with one setting does not load into the other).

**Steps:**
- [ ] **Step 1: Failing tests:** `canvas::borders_off_is_byte_identical_to_the_phase_2_picture`,
  `canvas::borders_on_has_the_measured_size`, `canvas::the_border_colour_follows_palette_entry_0_per_line`, plugin
  `the_borders_option_is_declared_and_defaults_to_off`, `state_envelope::borders_mismatch_is_refused`.
- [ ] **Step 2-4: implement, run, check** `retroclawd atari-st --borders on` headless dumps a picture of the
  recorded size. **Step 5: Commit; merge slice 15 after Task 38.**

### Task 38: the mono monitor

**Files:** Modify `scheduler.rs` (`geometry` for `Monitor::Mono`), `config.rs`, `bus.rs` (mono sync, mono frame),
plugin option text; Test: `tests/mono.rs`, probe P12, scenario S15

**Steps:**
- [ ] **Step 1: Probe P12:** under a mono monitor (Hatari `--monitor mono`, EmuTOS boots mono) measure the CPU clocks
  between two VBLs (a counted loop, bounded error) and the lines per frame (video counter progression per line, lines
  from the frame length and line length 224 clocks recorded in the spec's section 8). Record `[O]`.
- [ ] **Step 2: Failing tests:** `mono::frame_length_matches_the_probe`, `mono::emutos_mono_desktop_is_reached`,
  `mono::colour_resolution_on_mono_monitor_is_black` (facts `hardware-facts.md` row), scenario S15 (EmuTOS mono desktop
  against Hatari's screenshot).
- [ ] **Step 3: Implement** (remove the "mono is refused" paths and the manifest note). **Step 4: Run. Step 5:
  Commit; merge slice 15.**

---

## Slice 16: peripherals' timing

### Task 39: remaining Shifter edge cases found by the sweeps

Reserved slot for the effects Tasks 35-36 recorded as `known-gap` or as `[H?]` rows (for example a sync-mode write
that lands inside the HBL window). Steps: list the ledger's gaps; for each, one more probe-fit attempt with a new
explanation; close or keep with evidence. If the ledger has no gaps this task is a ledger line only.

### Task 40: DMA bus grants and the FIFO

**Files:** Modify `dma.rs`, `bus.rs`, `drive.rs`; Test: `tests/dma_timing.rs`, probe P13

**Interfaces:** `Dma::refill_burst(now) -> u32 stall_clocks` charged to the CPU by `StBus` at the fitted cycle of a
16-byte FIFO refill/drain, replacing the lump at transfer completion; transfers outside RAM end with the error bit and
no stall beyond the bounded attempt.

**Steps:**
- [ ] **Step 1: Probe P13:** a program reads sectors with a timer-measured loop on the CPU side, and counts how many
  loop iterations complete while the DMA runs; record the stall length per burst and its position. Hatari dump.
- [ ] **Step 2: Failing tests:** `dma_timing::a_sector_read_stalls_the_cpu_per_burst_as_the_probe_shows`,
  `…a_transfer_at_the_ram_edge_ends_with_the_error_bit_and_a_bounded_stall…`, `…odd_address…`, `…rom…`,
  `timing_probes::p13_matches_hatari`, plus the existing floppy tests unchanged in `fast`.
- [ ] **Step 3: Implement. Step 4: Run. Step 5: Commit.**

### Task 41: FDC and drive rotational timing

**Files:** Modify `drive.rs`, `dma.rs` (the shared `wd1770` is driven with the exact byte clock); Test:
`tests/rotation.rs`, probe P14

**Steps:**
- [ ] **Step 1: Probe P14:** index-to-index clocks (a loop on the index status bit), the search time to a sector
  header for each sector number from a known head position, step rates for each rate bit pair, head-settle and spin-up
  times; Hatari dump.
- [ ] **Step 2: Failing tests** from the dump: `rotation::index_to_index_is_the_probe_value`,
  `…sector_search_time_for_each_sector…`, `…step_rate_times…`, `…spin_up…`, `timing_probes::p14_matches_hatari`.
  The rotational position is a pure function of the machine time (a disk spinning since power-up), never of when the
  last command began.
- [ ] **Step 3: Implement** (if the shared `wd1770` needs a change, make it on the `atari-phase3` crates branch with
  a test in the chip). **Step 4: Run** (all floppy tests; `fast` stays as is unless a fact applies to both modes: the
  index pulse and rotation are device facts, so they change both modes; then the S3/S3W goldens are re-recorded once
  and the change is a ledger ruling with the reason). **Step 5: Commit.**

### Task 42: ACIA, IKBD, YM2149, printer and serial byte timing

**Files:** Modify `acia.rs`, `crates/chips/ikbd/src/lib.rs`, `psg.rs`, `serial.rs`; Test: `tests/peripheral_timing.rs`,
probes P15, P16, P19

**Steps:**
- [ ] **Step 1: Probes.** P15: send each IKBD command (`$80 $01` reset, `$0D` interrogate clock, a mouse mode, a read
  of memory) and record the clocks to the first answer byte and between bytes; the ACIA bit-clock phase; P16: a PSG
  register write then the first changed audio sample index (the machine's sample counter, via a chip snapshot), the
  envelope and tone counter phase; P19: bytes out of RS-232 and the printer at the programmed rates, with the
  fitted start-bit phase. Hatari dumps (Hatari's PSG output is read through its sound capture or the machine's
  registers; if no black-box reading of audio phase exists, P16 uses the PSG's timer-driven effects only and the
  audio phase stays `[H?]`, recorded).
- [ ] **Step 2: Failing tests:** `peripheral_timing::ikbd_answer_latency_for_each_command_matches_the_probe`,
  `…acia_bit_clock_phase…`, `…psg_write_to_sample_latency…`, `…serial_byte_start_phase…`, `timing_probes::p15`,
  `p16`, `p19` match Hatari.
- [ ] **Step 3: Implement** (the IKBD processing delays become `[O]` constants with their probe). **Step 4: Run. Step
  5: Commit.**

### Task 43: ACSI timing

**Files:** Modify `acsi.rs`, `bus.rs`; Test: `tests/acsi_timing.rs`, probe P17

**Steps:**
- [ ] **Step 1: Probe P17** with a hard disk image (made by `rcdisk hd`): clocks from the last command byte to the
  completion interrupt, per command, and per-block transfer clocks; Hatari dump.
- [ ] **Step 2: Failing tests:** `acsi_timing::completion_delay_per_command_matches_the_probe`,
  `…transfer_clocks_per_block…`, `timing_probes::p17_matches_hatari`. Replace "instant transfer" (Phase 2 ruling)
  with the fitted latency; record the ruling.
- [ ] **Step 3: Implement. Step 4: Run** (S11 hash re-recorded once with the ruling). **Step 5: Commit; merge slice
  16.**

---

## Slice 17: integration and exit

### Task 44: accurate-mode manifests

**Files:** Modify `atari-st.manifest.toml` (an `accuracy = "accurate"` variant of S1, S2, S2J, S2M, S3, S3W, S4,
S5, S6, S8P, S9S, S10, S11 and the new S12-S15; where a picture legitimately differs (timing-sensitive TOS
code) the variant has its own golden and a note), `tools` `rc-scenarios` only if it cannot pass an option per scenario
(it can: `args`).

**Steps:**
- [ ] **Step 1: Failing run:** `rc-scenarios run machines/atari/atari-st.manifest.toml` with the variants added and
  no goldens: each reports "golden missing" (expected). **Step 2:** three identical runs per variant; record the
  goldens made by retroclawd (never committed) and the Hatari ones where applicable. **Step 3:** all variants pass.
  **Step 4: Commit.**

### Task 45: determinism and state round trip in accurate mode

**Files:** Test: `tests/accurate_determinism.rs`

**Steps:**
- [ ] **Step 1: Failing tests:** `accurate_determinism::three_runs_of_the_desktop_give_the_same_hashes`,
  `…a_state_saved_at_frame_N_matches_a_straight_run_at_frame_N_plus_K` (for N in {1, 37, 200}, a state taken at an
  instruction boundary inside a line), `…two_machines_stepped_in_different_chunk_sizes_agree` (the harness calls
  `step` with arbitrary pauses). **Step 2-4: run, fix, run. Step 5: Commit.**

### Task 46: P18, P20 and the games

**Files:** Test: `tests/romm_games.rs` (accurate variant), probes P18, P20

**Steps:**
- [ ] **Step 1: Probes P18** (cycles for EmuTOS to reach the desktop: the VBL at which a known marker appears,
  Hatari vs retroclawd, difference recorded, not asserted equal unless every earlier probe matched — a difference is
  a clue for the ledger) **and P20** (a game-style loop; must match exactly).
- [ ] **Step 2: Failing tests:** `timing_probes::p20_matches_hatari`, `romm_games::every_game_boots_without_a_stop_in_
  accurate_mode` (needs `RC_ROMM_GAMES`; passes with a note without it), `…_gives_the_same_frame_on_two_runs`.
- [ ] **Step 3: Fix** what the games expose (each fix gets a test of its own). **Step 4: Run. Step 5: Commit.**

### Task 47: speed budget and the benchmark

**Files:** Create `crates/machines/atari-st/examples/bench.rs` (run with `cargo run --release --example bench`);
Modify hot paths if needed; Test: `tests/speed_smoke.rs` (a loose guard, `RC_BENCH=1` enables the timing assert)

**Interfaces:** the bench reports MIPS of a NOP/ALU mix, frames per second on the idle EmuTOS desktop, fast vs
accurate, host CPU and `rustc` version, median and variance over 10 runs (spec §21). **Budget (my ruling, adjustable):**
`fast` ≥ 10x real time and `accurate` ≥ 2x real time on the owner's machine in release; if `accurate` misses it,
optimise: an idle-loop fast path (`STOP`, `HALT`-like waits skip to the next event), batching consecutive RAM-only
instructions when no device is due before them, avoiding per-access event-queue peeks (cache `next_at`).

**Steps:**
- [ ] **Step 1: Write the bench; record the numbers** in `docs/benchmarks.md`. **Step 2: Failing test:**
  `speed_smoke::accurate_desktop_runs_at_the_budget` (only asserts with `RC_BENCH=1`). **Step 3: Optimise** until it
  holds, re-running fast identity and P-probes after each change. **Step 4: Commit.**

### Task 48: hostile and fuzz coverage

**Files:** Create `crates/machines/atari-st/fuzz/fuzz_targets/accurate_regs.rs` (random register writes at random
cycles), extend the `RC_SWEEP` mutation loops; Test: `tests/hostile_video.rs`

**Steps:**
- [ ] **Step 1: Failing tests:** `hostile_video::a_write_to_every_shifter_register_on_every_cycle_of_a_frame_is_bounded`
  (memory and time asserted), `…random_sync_and_resolution_sequences_never_panic_or_stop_the_machine` (seeded,
  `RC_SWEEP` scales), `…mutated_accurate_states_are_refused_or_load_cleanly`.
- [ ] **Step 2-4: run, fix, run** at `RC_SWEEP=40`. **Step 5: Commit.** (libFuzzer targets are written; running them
  needs nightly, as in Phase 2, and stays a deferred item if absent.)

### Task 49: documentation

**Files:** Modify `docs/hardware-facts.md` (every new row with its probe and label), `docs/timing-oracle.md`,
`CREDITS.md` (Hatari as the black-box oracle: version and options; any datasheet or manual section used for the
interrupt tables), `CLAUDE.md` of the atari repository (layout lines for `waits.rs`, `timeline.rs`, `timing/`),
`scenarios/README.md`, `timing/README.md`; host: spec "Status (…): Phase 3 done" with the exit-criteria table, `TODO.md`
("Phase 3 done, Phase 4 next" with the deferred items), host `CLAUDE.md` Phase 3 paragraph.

**Steps:** write them; `grep` that every `[O]` row names an existing probe; commit.

### Task 50: check_all, pins, push, final review

**Files:** Modify `Cargo.toml` pins in the atari and tools repositories; ledger.

**Steps:**
- [ ] **Step 1: Push the crates branch** (`atari-phase3` to `origin main`, fast-forward), take the local patch config
  aside, set the atari and tools pins to the pushed commit (`tools/bump_pin.sh` cannot be used while the atari branch
  is unpushed: edit the pins by hand and refresh `Cargo.lock`, as Phase 2 did), run the atari workspace tests with
  `RC_GOLDENS`, `RC_EMUTOS` and `RC_SST_BUS` default on.
- [ ] **Step 2: `tools/check_all.sh`** with the manifests in both modes; expected "all checks passed".
- [ ] **Step 3: Final review** as a self-review of the whole branch (no subagent tool): read `review-package` output
  against the plan's Review Focus; fix Critical/Important findings with RED→GREEN tests, ledger the minors.
- [ ] **Step 4: Push** in the Phase 2 order: crates, atari (`main`), tools (`main`), host `master` with the updated
  submodule pointers (re-check them with `git ls-tree HEAD`: `check_all` resets submodule checkouts, as it did in
  Phase 2, so check out `main`/the pinned commit again before committing the pointers). Never `git add -A`; the
  untracked agent-config files of the host are not committed.
- [ ] **Step 5: Final message** lists Rulings I made and Deferred minors from the ledger, whether each exit criterion
  is met (with evidence), the `known-gap` probes if any, and that the final review was a self-review.

---

## Self-review of this plan

- **Spec coverage (§5 Phase 3 column):** CPU prefetch-exact/per-access sync/wait states → 27, 29, 31; bus contention and
  4-cycle alignment → 31; Shifter cycle-exact fetches, borders, Spec512 → 30, 34-36; exact timer phase → 33; ACIA
  bit-time → 42; IKBD byte timing → 42; YM exact → 42; DMA/WD1772 rotational timing → 40-41; ACSI timing → 43;
  per-word DMA grants → 40; `accuracy` option and mode switching only at reset/load → 29, 33; benchmarks §21 → 47;
  mono monitor (a Phase 1 deferral) → 38. Spec §8 "DMA and Blitter ownership": the Blitter half is Phase 4.
- **Open sources:** Atari's hardware specification, the 68000 PRM timing tables (Task 28 fetches them), the
  MC68901 data sheet (fetched in Phase 2). Everything else is probe-fitted.
- **Type/name consistency:** `LineTimeline`/`LineEvent`/`counter_at` defined in Task 30, extended in 34-36;
  `waits::extra_clocks` and `ACCESS_POINT` in 31, used in 29's `begin_access`; `StBus::run_events_until` in 29, used by
  32-33, 40.
- **Placeholders:** none; where a value is fitted it says which probe fits it and how many offsets the sweep covers.
