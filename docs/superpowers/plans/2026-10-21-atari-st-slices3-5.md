# Atari ST Slices 3-5 ("Sound, keyboard, floppy: EmuTOS to an idle desktop") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Emulate the YM2149 (slice 3), the two MC6850 ACIAs and the IKBD (slice 4), and the DMA chip, WD1772 and a
floppy drive with `.st` images (slice 5). Afterwards EmuTOS 1.4 runs for 1500 frames without stopping. Its I/O
accesses equal Hatari 2.6.1's, under the polling rule below. It boots a test floppy built by our own code, and keys,
mouse and joystick reach it through the IKBD.

**Architecture:**
- **Three slices, one plan.** Each slice ends with an EmuTOS milestone task and a green workspace. The slices share
  one branch per repository and one final review.
- **Shared chips go to `retroclawd-crates`** (design §6 placement rule):
  - the YM2149 is a variant of the existing `ay-3-8910`;
  - `mc6850` is new;
  - `wd1770` moves there from retroclawd-commodore.

  The Atari-only chips (`ikbd`) and the format crate (`st-floppy`) stay in retroclawd-atari.
- **One pin bump at the end.** The atari and commodore repositories build against the local `shared/` checkout through
  a git-ignored `[patch]` during the work. The crates commit is pushed and every family's pin moved only in the last
  task, with the owner's approval.
- **Time.** New scheduler event classes for ACIA byte completion and FDC byte times, in the design's order: video →
  MFP → ACIA → FDC. Every device runs from integer CPU cycles; nothing reads the wall clock.
- **Oracle.** Hatari's `--trace io_all` output, run as a black box. From slice 4 on, the comparison collapses status
  polling (below), because how often a loop polls depends on emulator timing, not on a hardware fact.

**Tech Stack:**
- Rust 2024 and `serde`. No new external dependencies.
- retroclawd-tools: the Python standard library.

**Spec:** `docs/superpowers/specs/2026-10-19-atari-st-design.md`:
- sections 5, 6, 8, 9, 10, 12, 13 and 21;
- section 23, backlog items 6, 7 and 8;
- section 24 for the Phase 1 criteria that this plan meets in part.

Slice 2's plan is `docs/superpowers/plans/2026-10-20-atari-st-slice2.md`.

**Repositories and branches:**

| Repository | Branch | Work |
|---|---|---|
| retroclawd-crates (`shared/`) | `atari-slices-3-5` | Tasks 1-3: YM2149 variant, `mc6850`, `wd1770` moved |
| retroclawd-atari (`machines/atari`) | `atari-st-slices3-5` | Tasks 4-14 |
| retroclawd-commodore (`machines/commodore`) | `wd1770-from-crates` | Task 3: use the shared `wd1770`, delete the local copy |
| retroclawd-tools | `hatari-disk-trace` | Task 5: `--disk-a` and an extra-trace option for the capture script |
| retroclawd (host) | `master` | Task 13: `.st` media extension. Task 15: docs and submodule pointers |

## Global Constraints

Everything in slice 2's Global Constraints still applies:
- Hatari, ASE and EmuTOS are never a source of code, tables, constants or comments.
- Fact labels are `[D]` datasheet, `[H]` documentation, `[O]` Hatari-observed for a single address or value, and
  `[H?]` unresolved with the constant chosen. Each fact gets one row in `machines/atari/docs/hardware-facts.md` and a
  label in its doc comment.
- An access with no fact stops the machine ("no hardware fact"). An access to a device that is not emulated yet
  stops the machine with "not emulated".
- `#![forbid(unsafe_code)]`, licence `GPL-3.0-or-later`, and `serde` on state.
- No floats in timing. `u64` CPU cycles are the master unit, and clock ratios are exact integers or carried
  remainders. Audio samples are `f32`, and only the sample values are floats.
- Never commit:
  - goldens;
  - datasheets or books;
  - firmware;
  - disk images, except ones the tests build in memory.
- I/O logs and diagnostics carry register values only, never ROM, RAM or disk contents.
- Defaults are unchanged: `--ram 1m`, `--monitor color`, `--model stfm`, `--accuracy fast`.
- The plugin ABI and `PROTOCOL_VERSION` are unchanged.
- The `ay-3-8910` change is additive. The AY variant's behaviour and its serialized state stay as they are, so the
  Spectrum and CPC scenarios and the smoke-test hashes do not move.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Pushing anything, and moving any family's `retroclawd-crates` pin, needs the owner's go-ahead (Task 15).

## The polling rule (slices 4-5)

`collapse_polls(log)` replaces every run of consecutive identical accesses (same address, size, direction, value and
PC) with a single entry. Hatari's log and ours are compared after collapsing both.

It is a test helper in `crates/machines/atari-st/tests/common/mod.rs`, unit-tested on hand-written logs.

| Case | Comparison result |
|---|---|
| A run of `read.b $FFFC00 = $02 pc=fc4856` of a different length | Equal |
| A value changing inside a run (`$02 $02 $83`) | Kept: `$02 $83` |
| Order | Never changed |
| Different PCs | Not merged |

Slice 3's milestone needs no collapsing: no device with timing comes into play before `$FFFC00`.

## Evidence gathered while planning (2026-10-21)

**Hatari 2.6.1, 1500 frames, 1 MiB, no disk** (`--kind io --memsize 1 --vbls 1500`, 11 107 accesses). Every address
EmuTOS touches is already in `io.rs`'s decode table. Slices 3-5 make every `Device` row real.

| Line | First access | Device |
|---|---|---|
| 106 | `write.b $FF8800 = $07 pc=fc6d88`, then `$FF8802 = $C0` (R7: both ports output) | PSG (slice 3) |
| 110 | `write.b $FFFC00 = $03` (master reset), then `$96` | keyboard ACIA (slice 4) |
| 113, 116 | `$FFFC02 = $80`, `$01`: IKBD RESET | IKBD |
| 147-148 | status `$83`, data `$F1`: the IKBD's reset reply, after 15 polls reading `$02` | IKBD |
| 156 | `write.b $FFFC04 = $03`, then `$95` | MIDI ACIA |
| 748-783 | `$1C` (interrogate clock), reply `$FC 00 00 00 00 00 01`; later `$1B` and 6 bytes (set clock) | IKBD |
| 282 | `write.b $FF860F = $00` (an `[O]` open address; the write is ignored) | — |
| 284-286 | PSG R14 read, then written (drive select); `$FF8606 = $0080`, `$FF8604 = $00D0` (FDC force interrupt) | DMA/FDC (slice 5) |
| 293 | `read.w $FF8604 = $00E4` (FDC status, no disk) | FDC |
| 322-324 | DMA address low, mid, high | DMA |
| ~330 | `$FF8606 = $0088`, `$0190`, `$0090`: the ACSI probe and the sector count | DMA/ACSI |
| 853 on | `read.b $FF8260` (×1438); line 5856 on: palette words read back | Shifter (VDI) |
| end | every ~32 accesses: PSG R14 deselect/select, FDC status `$00E4`, GPIP `$B1`/`$91` | idle loop |

**Documents** (in the scratchpad `docs-dl/`, never committed):

| Document | SHA-256 | Covers |
|---|---|---|
| Yamaha YM2149 catalog, Sept 1992 (bitsavers `components/yamaha/YM2149_199209.pdf`) | `a4b946bf…7d20` | Registers (Table 3); tone `fT = fM/(16·TP)`; noise `fM/(16·NP)`; envelope `fE = fM/(256·EP)` with 32 steps, a 5-bit level (`E4-E0`) and a 5-bit DAC; the fixed level is 4 bits; SEL halves the clock; the CONT/ATT/ALT/HOLD shapes (Table 4) |
| Motorola MC6850 ACIA (docs.ampnuts.ru `MC6850.pdf`) | `294a8b9f…4655` | Registers, ÷1/16/64, word select, status bits, master reset |
| *Intelligent Keyboard (ikbd) Protocol*, Atari, 26 Feb 1985 (bitsavers `Atari_ST_GEM_Programming_1986/GEM_0883.pdf`) | `9f216bf7…9d82` | Command set §9, power-up defaults, RESET reply "`0xF0` … within 300 ms; a second release `0xF1`", scan codes (Appendix A) |
| Western Digital *1986 Storage Management Products Handbook* (bitsavers), WD177X-00 pp.1-1..1-26 | `72382885…895f` | WD1772 commands, status, timing at 8 MHz, IBM-format track layout |
| *Atari ST Internals* (slice 2 copy) | `961b981d…` | Read the PDF text with `pdftotext -raw`: the `-layout` text shifts the bit tables by one row. Covers: DMA mode bits p.59 (1 A0, 2 A1, 3 FDC/HDC, 4 sector-count select, 6 DMA off, 7 HDC/FDC, 8 write); DMA status bits 0-2; `$FF8604` upper byte "reads 1"; DMA address low → mid → high p.59; PSG port A p.60 (0 side, 1 drive 0, 2 drive 1, 3 RTS, 4 DTR, 5 strobe, 6 GPO, 7 reserved); `$FF8802` reads `$FF`; the ACIAs (p.47): 500 kHz, CTS/DCD/RTS not connected, keyboard ÷64, MIDI ÷16; the schematic labels the YM clock 2 MHz |

**Conflicts found already. Each one is a fact row with both sources and the choice:**

| Item | `[H]`/`[D]` says | Hatari shows | Choice |
|---|---|---|---|
| `$FF8604` upper byte on reads | "consistently read 1" (*Internals* p.59) | `$00E4` | `[H?]`: Task 10 tries the hardware specification. If it is unavailable, follow *Internals* (`$FF`) and record the difference as a known oracle mismatch: the test masks the high byte of `$FF8604` reads only |
| IKBD reset reply | `$F0` for the first release, `$F1` for a second (protocol doc §8) | `$F1` | `[O]` `$F1`: the document allows it, and it is a single value |
| DMA mode bits 3 and 7 | both described as FDC/HDC select (*Internals*) | EmuTOS writes `$80` for the FDC and `$88` for ACSI | `[H?]`: bit 3 set = HDC (both sources agree on it); bit 7 is not decoded |

## Review Focus

1. **Polling collapse must not hide a real divergence.** Only identical consecutive accesses merge. A unit test proves
   that a changed value, address or PC inside a run survives (Task 4, `common` tests).
2. **Malformed `.st` images.** These include sizes that fit no geometry, a BPB that contradicts the file size, zero
   sectors per track, and files over 2 MiB. Each is an `Err` naming both geometries, and none panics. Tests are in
   Task 9, including a bounded loop over truncated and garbled images.
3. **The IKBD output and ACIA receive path when the CPU never reads.** Bytes queue in the IKBD (bounded, oldest kept,
   protocol-doc buffer behaviour); the ACIA sets overrun per the datasheet; nothing grows without bound under a mouse
   flood (Task 6 `unread_bytes_set_overrun_and_stay_bounded`, Task 7 `output_queue_is_bounded`).
4. **DMA transfers that run past RAM.** A transfer that crosses the top of RAM, or points at ROM or I/O, follows the
   bus's own rules: writes to ROM are ignored, and a transfer into the I/O area stops with "no hardware fact". It never
   indexes out of bounds (Task 10).
5. **Media changes in mid-command.** These include ejecting during a sector read, deselecting the drive or changing
   side while the FDC is busy, and a disk in a drive that does not exist (`--drives 1`, drive B selected). The FDC
   sees what the hardware would (no index pulses, no data) and the machine never panics (Task 10 `drive.rs` tests
   `eject_during_read_sector_never_panics`, `side_change_mid_command`, `drive_b_with_drives_1_is_empty`).

---

# Slice 3: YM2149 (and the shared-crate groundwork)

### Task 1: `ay-3-8910`: the YM2149 variant (retroclawd-crates)

**Files:**
- Modify: `shared/crates/chips/ay-3-8910/src/lib.rs`
- Test: `shared/crates/chips/ay-3-8910/tests/ym2149.rs`
- Modify: `shared/CREDITS.md` (YM2149 catalog, URL and SHA-256)

**Interfaces:**
- Produces:
  - `pub enum Variant { Ay38910, Ym2149 }` (serde, default `Ay38910`).
  - `Ay3_8910::with_variant(v: Variant) -> Self`.
  - `Ay3_8910::new()` is unchanged (AY).
  - `pub fn envelope_level(&self) -> u8`: 0-31 on the YM, 0-15 on the AY.
  - `output()` uses a 32-entry level table on the YM. For a fixed level `L` (4 bits) on the YM, the DAC input is
    `2L + 1` for `L > 0` and 0 for `L = 0`. This is `[H?]`: the datasheet says only "one of 16" and "5 bit D/A".
    Record the choice in a doc comment.
  - The 32-step table is `[H?]`: the catalog gives no values. Use a documented constant with geometric steps between
    the existing AY table's entries (`table32[2k+1] = AY[k]`, and the even entries the geometric mean of their
    neighbours, `table32[0] = 0`).

**Steps:**

- [ ] **Step 1: Write the failing tests** in `tests/ym2149.rs`.
  - `ym_envelope_counts_32_steps_per_period`:
    - shape `$0C` (CONT, ATT), EP = 1;
    - after every `8` master clocks (the datasheet's `fE = fM/(256·EP)` with 32 steps), `envelope_level()` reads
      0, 1, …, 31, then 0 again.
  - `ay_envelope_is_unchanged`: the same sequence on `Ay3_8910::new()` gives the AY's existing 16-level values. Write
    this from the current code's behaviour before touching anything.
  - `ym_tone_period_matches_datasheet`: TP = 100. The channel A output toggles every `8 × 100` master clocks
    (`fT = fM/(16·TP)`: a full period is 1600).
  - `ym_shapes_follow_table_4`: for each of the 16 shape values, the level sequence over two periods matches Table 4
    (hold, alternate, attack, continue). Shapes `$00-$07` behave as `$09` or `$0F` per the table.
  - `variant_survives_serde`: serialize and deserialize a YM, and the variant is kept. Deserializing an old AY state
    (no `variant` field) gives AY.
- [ ] **Step 2: Run them.** `cargo test -p ay-3-8910 --test ym2149` in `shared/`. Expected: FAIL (no `Variant`).
- [ ] **Step 3: Implement.** `variant` is a field with `#[serde(default)]`. The envelope keeps its 5-bit position, and
  `level()` returns it unhalved on the YM. The tone and noise generators do not change.
- [ ] **Step 4: Run the crate and its users.**
  - `cargo test -p ay-3-8910`, then `cargo test --workspace` in `shared/`. Expected: PASS.
  - `cargo test --workspace` in `machines/sinclair` and `machines/amstrad` against the local `shared/`
    (Task 3, Step 1 sets up the patch). Expected: PASS, unchanged.
- [ ] **Step 5: Commit** on `shared/` branch `atari-slices-3-5`: "ay-3-8910: YM2149 variant (32-step envelope, 5-bit
  DAC)".

### Task 2: `mc6850` ACIA chip crate (retroclawd-crates)

**Files:**
- Create: `shared/crates/chips/mc6850/{Cargo.toml,src/lib.rs,tests/datasheet.rs}`
- Modify: `shared/Cargo.toml` (workspace member), `shared/CREDITS.md`

**Interfaces:**
- Produces:

  ```rust
  pub struct Acia { /* serde */ }
  impl Acia {
      pub fn new() -> Self;                  // the state after power-up (datasheet: master reset needed before use)
      pub fn read(&mut self, rs: bool) -> u8;   // rs false = status, true = receive data (clears RDRF, overrun per datasheet)
      pub fn peek(&self, rs: bool) -> u8;       // no side effects (debugger, snapshots)
      pub fn write(&mut self, rs: bool, v: u8); // control or transmit data
      pub fn irq(&self) -> bool;                // IRQ output asserted
      pub fn divide(&self) -> u32;              // 1, 16 or 64 from CR1-CR0; 0 after master reset
      pub fn frame_bits(&self) -> u32;          // start + data + parity + stop bits from CR4-CR2
      pub fn take_tx(&mut self) -> Option<u8>;  // the shift register's byte, once its frame time has elapsed (the machine schedules it)
      pub fn tx_pending(&self) -> bool;
      pub fn rx_byte(&mut self, b: u8);         // a complete received frame: RDRF, or overrun if RDRF still set
      pub fn set_cts(&mut self, high: bool);
      pub fn set_dcd(&mut self, high: bool);
  }
  ```

- Timing stays outside the crate. The machine computes a frame time as `frame_bits() × divide() × clock_ratio` CPU
  cycles and calls `take_tx`/`rx_byte` at those times. This keeps the crate free of any clock.

**Steps:**

- [ ] **Step 1: Write the failing datasheet tests**, one per datasheet behaviour, each citing its section:
  - master reset (CR1-CR0 = 11) clears the status except CTS/DCD, and `irq()` is low;
  - `$96` sets: ÷64, 8 data + 1 stop, RTS low with the transmit interrupt disabled, receive interrupt enabled. `$95`
    is ÷16 with the same frame. The tests use these exact bytes (EmuTOS's, from the Evidence table);
  - writing TDR clears TDRE, and TDRE sets again when the byte moves to the shift register;
  - `rx_byte` sets RDRF, and IRQ when CR7 is set; reading RDR clears both;
  - a second `rx_byte` before the read sets OVRN; status bit 5 reads per the datasheet's read sequence;
  - CTS high inhibits TDRE; DCD high sets status bit 2 and IRQ (per the datasheet);
  - `frame_bits()` for all 8 word-select codes.
- [ ] **Step 2: Run.** `cargo test -p mc6850`. Expected: FAIL (crate missing).
- [ ] **Step 3: Implement** from the datasheet only. Each register bit has a doc comment with its datasheet table.
- [ ] **Step 4: Run.** `cargo test -p mc6850` and `cargo clippy -p mc6850 --all-targets -- -D warnings`. Expected:
  PASS.
- [ ] **Step 5: Commit**: "mc6850: Motorola ACIA from the datasheet".

### Task 3: Move `wd1770` to retroclawd-crates; local patches for development

**Files:**
- Create: `shared/crates/chips/wd1770/` (moved with `git` from the commodore repository, history kept through
  `git format-patch`/`am` or a plain copy with the source commit named in the commit message)
- Modify: `shared/Cargo.toml`, `shared/CREDITS.md` (VICE reference note moves with it)
- Modify: `machines/commodore/Cargo.toml`, `machines/commodore/crates/machines/1541/Cargo.toml` (`wd1770` from
  `retroclawd-crates`)
- Delete: `machines/commodore/crates/chips/wd1770/`
- Create (git-ignored, never committed): `.cargo/config.toml` in `machines/atari`, `machines/commodore`,
  `machines/sinclair` and `machines/amstrad`, each with a `[patch."https://github.com/elpapimango/retroclawd-crates"]`
  path entry for every crate this plan touches

**Interfaces:**
- Produces: `wd1770::{Wd1770, Floppy, Wd1770State, crc_byte}` unchanged, at `retroclawd-crates`.

**Steps:**

- [ ] **Step 1: Set up and check the patches.** Write the `.cargo/config.toml` files. Then run
  `cargo test --workspace` in `machines/commodore` with commodore still on its local `wd1770`. Expected: PASS.
  - **Check:** a `[patch]` entry may name a crate (`mc6850`) that the pinned git revision does not contain yet. Test
    this with `cargo tree -p atari-st` after Task 6 adds the dependency.
  - If cargo refuses, **stop and ask the owner** whether the `atari-slices-3-5` branch of retroclawd-crates may be
    pushed (a non-default branch) so the families can depend on `branch = "atari-slices-3-5"` during the work.
- [ ] **Step 2: Move the crate** into `shared/`. `cargo test -p wd1770` in `shared/`. Expected: PASS (its existing
  tests).
- [ ] **Step 3: Switch commodore** to the shared crate. Use a git dependency at the current pin; the patch resolves it
  locally. Delete the local copy. `cargo test --workspace` in `machines/commodore`. Expected: PASS, the same test
  count as Step 1 minus nothing.
- [ ] **Step 4: Commit** in `shared/` ("wd1770: moved from retroclawd-commodore") and in `machines/commodore` on
  branch `wd1770-from-crates` ("Use wd1770 from retroclawd-crates").

### Task 4: PSG wiring, audio, test helpers; EmuTOS to the keyboard ACIA (slice 3 milestone)

**Files:**
- Modify: `crates/machines/atari-st/Cargo.toml` (`ay-3-8910` from retroclawd-crates)
- Create: `crates/machines/atari-st/src/psg.rs`. It holds the YM, the CPU/4 clock with a carried remainder, and the
  44 100 Hz sample accumulator (the CPC/Spectrum pattern: samples from cycles, integer rational).
- Modify: `src/bus.rs` (route `Device::Psg`), `src/machine.rs` (`drain_audio`, catch-up at VBL and on access),
  `src/lib.rs`
- Create: `crates/machines/atari-st/tests/common/mod.rs` (`collapse_polls`, shared EmuTOS loaders moved from
  `emutos.rs`)
- Test: `tests/psg.rs`, `tests/emutos.rs`, and a `common` unit test file `tests/polls.rs`
- Modify: `docs/hardware-facts.md` (PSG section), `CREDITS.md`

**Interfaces:**
- Produces:
  - `StBus::psg(&self) -> &Psg`;
  - `Psg::port_a(&self) -> u8` (output latch when R7 bit 6 is set, else `$FF` [H?]: inputs float high on the ST;
    nothing reads them in this plan);
  - `Machine::drain_audio` returns mono `f32` at 44 100 Hz.
  - `pub fn collapse_polls(log: &[String]) -> Vec<String>` (test helper).
- Decode, from *Internals* p.59-60:
  - `$FF8800` write = register select;
  - `$FF8800` read = selected register (port A or B returns its latch when set to output, per the datasheet);
  - `$FF8802` write = data;
  - `$FF8802` read = `$FF` `[H]`;
  - `$FF8801`/`$FF8803` keep their slice 2 decode (`Device(Psg)`) and act as `$8800`/`$8802` `[H?]`: the PSG sees
    only A1. If a source turns up showing otherwise, they move to `NoFact`.
- Clock: the YM's input clock is `[H]` "2 MHz" (*Internals* schematic). The divider, CPU clock / 4, and SEL tied so
  the master clock equals the input are `[H?]`. Record both, and the tone test below pins the arithmetic, not the
  hardware.
- RESET instruction: the YM is not reset (`[H?]`, slice 2's rule: only sourced resets).

**Steps:**

- [ ] **Step 1: Failing tests.**
  - `tests/psg.rs`:
    - `select_write_read_back` (R0 = `$34` reads back `$34`);
    - `r1_reads_four_bits`;
    - `port_a_latch_and_direction` (`$C0` to R7, `$07` to R14, then `port_a() == $07` and `$FF8800` reads `$07`);
    - `ff8802_reads_ff`;
    - `user_mode_is_bus_error`;
    - `tone_a_440ish_from_bare_metal` (S4 shape): a ROM program sets TP = 284, level 15, mixer tone A only, then
      `STOP`. 50 frames of `drain_audio()`: the zero-crossing count of the DC-removed signal over the run equals
      `2 × frames × cycles_per_frame / (4 × 16 × 284)` within ±2;
    - `audio_sample_count_is_exact`: after `N` frames, samples drained = `floor(N × cycles_per_frame × 44 100 /
      clock_hz)` exactly.
  - `tests/polls.rs`: the four rows of "The polling rule".
  - `tests/emutos.rs`: `emutos_runs_to_the_keyboard_acia` stops at `write.b $FFFC00 = $03 pc=fc5160` with
    `NotEmulated(Acia)`. The log equals Hatari's first 109 lines exactly, for all six `--ram` goldens.
    `emutos_runs_to_the_psg` is removed.
- [ ] **Step 2: Run.** `cargo test -p atari-st`. Expected: FAIL (PSG not emulated, helpers missing).
- [ ] **Step 3: Implement** `psg.rs` and wire it in. A PSG access calls `catch_up` first, as the MFP does.
- [ ] **Step 4: Run.** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt
  --all --check` in `machines/atari` with `RC_EMUTOS` and `RC_GOLDENS` set. Expected: PASS. The EmuTOS tests must
  print that they compared against the goldens, not that they skipped.
- [ ] **Step 5: Commit** (atari): "atari-st: YM2149 (registers, port A, audio); EmuTOS runs to the keyboard ACIA".

---

# Slice 4: ACIAs and the IKBD

### Task 5: Hatari capture options (retroclawd-tools)

**Files:**
- Modify: `retroclawd-tools/scripts/capture_hatari_trace.py`, and the CLAUDE.md capture notes

**Interfaces:**
- Produces:
  - `--disk-a PATH`: Hatari's `--disk-a`. Tasks 12-13 use it.
  - `--trace-extra FLAGS`: comma-separated Hatari trace flags appended to the run, with their lines passed through to
    a second output file `OUT.extra.txt`.
  - The `--kind io` output itself is unchanged.

**Steps:**

- [ ] **Step 1:** Run `hatari --trace help` and record which flags exist for the IKBD and the FDC (for example
  `ikbd_cmds`, `ikbd_acia`, `fdc`). Note in the ledger whether their lines carry a time (VBL/HBL/cycle).
- [ ] **Step 2:** Implement both options. Check by hand:
  - with `--disk-a` on a zero-filled 737 280-byte file built in the scratchpad, the I/O log differs from the no-disk
    run (the boot sector is read);
  - `--trace-extra ikbd_cmds` writes the extra file.
- [ ] **Step 3: Measure the IKBD reset delay.**
  - If Step 1 found a timed IKBD trace: from command byte `$01` received to `$F1` sent, in CPU cycles at 8 MHz. That
    becomes the `[O]` row "IKBD RESET reply delay".
  - Otherwise use `[H?]` "reply queued as soon as the second byte is received". This is a documented constant from
    the protocol doc's upper bound being 300 ms, with no lower bound given. Ledger which one.
- [ ] **Step 4: Commit** (tools, branch `hatari-disk-trace`).

### Task 6: ACIA wiring and serial timing

**Files:**
- Create: `crates/machines/atari-st/src/acia.rs` (two `mc6850::Acia`, byte-time events, the IRQ OR, the MIDI port)
- Modify: `src/bus.rs`, `src/machine.rs`, `src/scheduler.rs` (`Event::AciaTx(n)`, `Event::AciaRx(n)`,
  `CLASS_ACIA = 2`), `Cargo.toml` (`mc6850`)
- Test: `tests/acia.rs`
- Modify: `docs/hardware-facts.md`

**Interfaces:**
- Consumes: `mc6850::Acia` (Task 2).
- Produces:
  - `pub trait SerialPeer { fn receive(&mut self, byte: u8, at: u64); fn next_byte(&mut self, now: u64) -> Option<(u64, u8)>; }`
    The ACIA module drives a peer: the IKBD on the keyboard ACIA, `NullPeer` on MIDI (Phase 2: loopback/file).
  - `StBus::acia(&self, n: usize) -> &mc6850::Acia`.
- Facts:
  - addresses `$FFFC00/02/04/06` (bytes) `[H]` p.62;
  - ACIA clock 500 kHz `[H]` p.47, as CPU clock / 16 `[H?]`. One bit = `divide × 16` CPU cycles; a frame =
    `frame_bits × divide × 16`;
  - both IRQs, ORed and active low, drive GPIP I4, which is MFP channel 6 `[H]` (*Internals* p.61, *TOS hyp* `$118`).
    The line level is high when idle `[O]` (GPIP `$B1` after the ACIA reset);
  - CTS and DCD are low `[H]` p.47 ("not connected") with `[O]` status `$02`;
  - odd and word accesses to `$FFFC0x`: `NoFact`, unchanged.
- RESET instruction: the ACIAs are not reset `[H?]`. The 6850 has no reset pin (datasheet: master reset is by
  software).

**Steps:**

- [ ] **Step 1: Failing tests** (`tests/acia.rs`):
  - `master_reset_then_96_reads_status_02` (TDRE set, matching Hatari line 112);
  - `transmit_takes_one_frame`: write `$80` to `$FFFC02`. Status reads `$00` until exactly `10 × 64 × 16` cycles
    later, then `$02`. Check one cycle before and at;
  - `peer_receives_the_byte_at_frame_end` (a recording peer gets `$80` at that cycle);
  - `received_byte_raises_gpip_i4_and_mfp_channel_6` (with IERB/IMRB set, IPL 6 and vector `VR + 6`);
  - `midi_null_peer_never_receives_and_status_reads_02`;
  - `unread_bytes_set_overrun_and_stay_bounded`: a peer offering 10 000 bytes with no reads. OVRN is set, and memory
    is unchanged in size (the peer keeps its own bounded queue).
- [ ] **Step 2: Run.** `cargo test -p atari-st --test acia`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - Transmit: a write to TDR schedules `AciaTx` at `now + frame` when the shift register is free.
  - Receive: on each frame boundary the peer is asked for its next byte, through an `AciaRx` event at the peer's
    time, or one frame after the previous byte, whichever is later.
  - The GPIP I4 level is recomputed after every ACIA change and MFP catch-up.
- [ ] **Step 4: Run** `cargo test -p atari-st`. Expected: PASS. EmuTOS now stops at its first IKBD dependency (no
  peer yet). The `emutos.rs` milestone test is updated in Task 8.
- [ ] **Step 5: Commit**: "atari-st: keyboard and MIDI ACIAs (MC6850) with frame timing".

### Task 7: `ikbd` chip crate (behavioural HD6301 keyboard controller)

**Files:**
- Create: `crates/chips/ikbd/{Cargo.toml,src/lib.rs,src/scancodes.rs,tests/protocol.rs}`
- Modify: workspace `Cargo.toml`, `CREDITS.md`, `docs/hardware-facts.md` (IKBD section)

**Interfaces:**
- Produces:

  ```rust
  pub struct Ikbd { /* serde */ }
  impl Ikbd {
      pub fn new() -> Self;                       // power-up: defaults of protocol doc §8, clock 00-00-00 00:00:00 [O]
      pub fn command_byte(&mut self, b: u8, at: u64);   // a byte from the ST
      pub fn pop_output(&mut self, now: u64) -> Option<u8>; // next byte to send, if its time has come
      pub fn next_output_at(&self) -> Option<u64>;
      pub fn key(&mut self, scan: u8, down: bool, at: u64);
      pub fn mouse_move(&mut self, dx: i32, dy: i32, at: u64);
      pub fn mouse_button(&mut self, left: bool, down: bool, at: u64);
      pub fn joystick(&mut self, port: u8, bits: u8, at: u64); // bits: up, down, left, right, fire
      pub fn tick_clock(&mut self, now: u64, cycles_per_second: u64); // the time-of-day clock
  }
  pub const SCANCODES: &[(&str, u8)];               // Appendix A, name -> make code
  ```

- The output queue is bounded at the protocol doc's buffer size if one is stated. Otherwise it holds 256 bytes `[H?]`,
  and the oldest bytes are kept (newer input is dropped).
- The clock is BCD, starts at all zeros (`[O]` Hatari's `$1C` reply), and advances once per second of machine time.
- Commands implemented: every one in protocol doc §9 that changes reported data:

  | Code | Command |
  |---|---|
  | `$80 $01` | RESET (replies `$F1` `[O]` after the Task 5 delay) |
  | `$07` | set mouse button action |
  | `$08` | relative mouse |
  | `$09` | absolute mouse |
  | `$0A` | mouse keycode mode |
  | `$0B` | set mouse threshold |
  | `$0C` | set mouse scale |
  | `$0D` | interrogate mouse position |
  | `$0E` | load mouse position |
  | `$0F` | Y=0 at bottom |
  | `$10` | Y=0 at top |
  | `$11` | resume |
  | `$12` | disable mouse |
  | `$13` | pause output |
  | `$14` | joystick event reporting |
  | `$15` | joystick interrogation mode |
  | `$16` | joystick interrogate |
  | `$1A` | disable joysticks |
  | `$1B` | set time-of-day |
  | `$1C` | interrogate time-of-day |
  | `$20` | memory load: the bytes are consumed and ignored, `[H?]` |
  | `$21` | memory read: replies zeros, `[H?]` |
  | `$22` | controller execute: ignored, `[H?]` |
  | `$80+` | status inquiries per §9 |

  Joystick monitoring (`$17`), fire button monitoring (`$18`) and keycode mode (`$19`) take their parameter bytes and
  are ignored, `[H?]` (Phase 2). Codes not listed are NOPs (§9: "should perform no operation").

**Steps:**

- [ ] **Step 1: Failing tests** (`tests/protocol.rs`). Each cites its protocol-doc section:
  - `reset_replies_f1` (with the delay from Task 5);
  - `reset_needs_01` (`$80 $02` is ignored, and the `$80` with it);
  - `relative_mouse_packet` (header `$F8 | buttons`, dx, dy as signed bytes; threshold 1; Y down positive with Y=0 at
    top);
  - `mouse_moves_larger_than_127_split_into_packets`;
  - `key_make_and_break` (break = make | `$80`);
  - `joystick_1_event` (`$FF` header, then the state byte);
  - `clock_set_and_interrogate` (`$1B` with BCD, then `$1C` returns it, advanced by elapsed whole seconds);
  - `clock_starts_at_zero` (`$1C` at 1.2 s gives `$FC 00 00 00 00 00 01`, the Hatari evidence);
  - `unknown_command_is_nop`;
  - `paused_output_queues_until_resume`;
  - `output_queue_is_bounded` (100 000 mouse moves, queue length ≤ the bound);
  - `scancodes_are_unique_and_match_appendix_a` (spot-check: `Esc` `$01`, `Space` `$39`, `Return` `$1C`, `Help` `$62`,
    `KeypadEnter` `$72`).
- [ ] **Step 2: Run.** `cargo test -p ikbd`. Expected: FAIL (crate missing).
- [ ] **Step 3: Implement** from the protocol document. The scan-code names are the document's key names in
  `UpperCamelCase`, so the plugin can report them.
- [ ] **Step 4: Run.** `cargo test -p ikbd`, plus clippy. Expected: PASS.
- [ ] **Step 5: Commit**: "ikbd: keyboard controller from Atari's ikbd protocol document".

### Task 8: IKBD on the keyboard ACIA, host input; EmuTOS to the floppy (slice 4 milestone)

**Files:**
- Modify: `src/acia.rs` (the IKBD as the keyboard peer), `src/machine.rs` (`key_event`, `mouse_motion`,
  `mouse_button`, `joystick`), `Cargo.toml` (`ikbd`)
- Modify: `crates/plugins/atari-st-machine-plugin/src/lib.rs`:
  - `key_names` from `ikbd::SCANCODES`, plus `Joy1Up/Down/Left/Right/Fire1`;
  - `KeyEvent`, `MouseMotion` and `MouseButton` handlers;
  - `chars` for the symbolic layout from the UK keyboard legends in Appendix A where the document gives them.
    Otherwise none (the positional layout still works).
- Test: `tests/ikbd_wiring.rs`, `tests/emutos.rs`, plugin unit tests

**Interfaces:**
- Consumes: `ikbd::Ikbd` (Task 7), `SerialPeer` (Task 6).
- Produces: `AtariSt::key_event(name: &str, down: bool) -> Result<(), String>`, `AtariSt::mouse_motion(dx, dy)`,
  `AtariSt::mouse_button(left, down)`, `AtariSt::joystick(port, bits)`.

**Steps:**

- [ ] **Step 1: Failing tests.**
  - `tests/ikbd_wiring.rs`:
    - `bare_metal_reads_reset_reply`: a ROM program resets the ACIA (`$03`, `$96`), sends `$80 $01`, and polls status
      bit 0. The byte read is `$F1`, stored in RAM;
    - `key_press_reaches_the_cpu_as_make_code` (`key_event("Space", true)` → `$39` read by the program);
    - `mouse_motion_arrives_as_a_relative_packet`.
  - Plugin: `profile_lists_atari_key_names`, `unknown_key_name_is_an_error`.
  - `tests/emutos.rs`: `emutos_runs_to_the_dma_chip` stops at `write.w $FF8606 = $0080 pc=fc3734` with
    `NotEmulated(Dma)`. The log equals Hatari's first 285 lines after `collapse_polls` on both sides, for all six RAM
    sizes.
    - If the comparison diverges at the IKBD clock reply because of whole-second rounding, the ledger records the
      cycle count of the `$1C` command in both emulators (from Task 5's extra trace). The test then compares that
      reply with a mask on the seconds byte only. That is a ruling, not a silent mask.
- [ ] **Step 2: Run.** `cargo test -p atari-st` and the plugin tests. Expected: FAIL.
- [ ] **Step 3: Implement the wiring.**
  - `next_byte` uses `Ikbd::next_output_at`.
  - The IKBD clock ticks at each VBL and on access.
  - Plugin input goes through the machine's methods.
  - Mouse motion uses the host's relative deltas. It is not scaled `[H?]`: the host already reports pixels.
- [ ] **Step 4: Run** the whole workspace with goldens. Expected: PASS, and the EmuTOS tests report "compared".
- [ ] **Step 5: Commit**: "atari-st: IKBD on the keyboard ACIA, keyboard/mouse/joystick input; EmuTOS runs to the
  DMA chip".

---

# Slice 5: DMA, WD1772, floppy drive, `.st`

### Task 9: `st-floppy` format crate: `.st` images and MFM tracks

**Files:**
- Create: `crates/formats/st-floppy/{Cargo.toml,src/lib.rs,src/mfm.rs,tests/st.rs}`
- Modify: workspace `Cargo.toml`, `docs/hardware-facts.md` (floppy section), `CREDITS.md`

**Interfaces:**
- Produces:

  ```rust
  pub struct Geometry { pub cylinders: u8, pub sides: u8, pub sectors: u8 } // 512-byte sectors
  pub struct Image { pub geometry: Geometry, /* sector data */ }
  impl Image {
      pub fn parse(bytes: &[u8]) -> Result<Image, Error>;
      pub fn sector(&self, cyl: u8, side: u8, sector: u8) -> Option<&[u8]>;   // sector 1-based
      pub fn write_sector(&mut self, cyl: u8, side: u8, sector: u8, data: &[u8]) -> Result<(), Error>; // Phase 2 uses it; tested now
      pub fn to_bytes(&self) -> Vec<u8>;
      pub fn track(&self, cyl: u8, side: u8) -> Vec<u16>; // MFM track bytes for wd1770::Floppy; bit 8 = sync mark
  }
  pub const TRACK_BYTES: usize = 6250; // 250 kbit/s × 200 ms (300 rpm), [H?] rotation speed
  ```

- Geometry rule (design §13):
  1. Use the boot sector BPB if it is self-consistent with the file size: sectors per track 1-11, sides 1-2,
     cylinders ≤ 86, total sectors × 512 equal to the size. The BPB is little-endian `[H]` (FAT documentation).
  2. Otherwise use the size table: 360K = 80×1×9, 400K = 80×1×10, 720K = 80×2×9, 800K = 80×2×10, 820K = 82×2×10
     (`[H?]` for the last).
  3. If both fail, or they disagree, return `Err` naming both. Never guess.
  4. The size cap is 2 MiB.
- Track layout: the WD177X-00 datasheet's IBM-format FORMAT TRACK table `[D]` (gap 4a, sync, index mark …
  ID/data marks with their `$A1` sync bytes flagged in bit 8, CRC from `wd1770::crc_byte`, gap 4b fill to
  `TRACK_BYTES`). If a layout does not fit in 6250 bytes (11 sectors), gap 3 shrinks to the datasheet minimum.
  Beyond that, `parse` refuses the geometry.

**Steps:**

- [ ] **Step 1: Failing tests** (`tests/st.rs`):
  - `parses_720k_by_bpb` (a BPB built in the test);
  - `falls_back_to_size_table`;
  - `bpb_disagreeing_with_size_is_an_error_naming_both`;
  - `rejects_empty_oversize_and_odd_sizes`;
  - `garbled_and_truncated_images_never_panic` (every prefix length of a 720K image in 4 KiB steps, plus 1000
    pseudo-random BPBs from a fixed seed);
  - `track_has_9_ids_and_data_with_good_crcs` (decode the synthesized track back with a test-side reader using
    `crc_byte`);
  - `track_length_is_6250`;
  - `round_trip_bytes`.
- [ ] **Step 2: Run.** `cargo test -p st-floppy`. Expected: FAIL.
- [ ] **Step 3: Implement.** Use checked arithmetic and no indexing that can panic on input.
- [ ] **Step 4: Run.** Expected: PASS, and clippy is clean.
- [ ] **Step 5: Commit**: "st-floppy: .st images, geometry from the BPB or the size, MFM tracks".

### Task 10: Drive model, DMA chip, FDC wiring

**Files:**
- Create: `crates/machines/atari-st/src/drive.rs` (an `StDrive` implementing `wd1770::Floppy`: rotation in byte
  times, index pulse, head position and step, track 0, write protect, motor, no-disk behaviour, an optional `Image`)
- Create: `crates/machines/atari-st/src/dma.rs` (mode register, sector count, the 24-bit address counter with
  byte-wise register access, 16-byte FIFO, status, the ACSI side with no device)
- Modify: `src/bus.rs` (`Device::Dma`), `src/machine.rs`, `src/scheduler.rs` (`Event::FdcTick`, `CLASS_FDC = 3`),
  `src/psg.rs` (port A → drive select and side), `Cargo.toml` (`wd1770`, `st-floppy`)
- Test: `tests/dma.rs`, `tests/drive.rs`
- Modify: `docs/hardware-facts.md`

**Interfaces:**
- Consumes: `wd1770::{Wd1770, Floppy}` (Task 3), `st_floppy::Image` (Task 9), `Psg::port_a` (Task 4).
- Produces:
  - `AtariSt::insert_disk(drive: u8, image: st_floppy::Image) -> Result<(), String>`;
  - `AtariSt::eject_disk(drive: u8) -> Option<st_floppy::Image>`;
  - `AtariSt::disk_modified(drive) -> bool` (always false in Phase 1: write support is Phase 2. A write command
    replies write-protected `[H?]` until then. EmuTOS does not write during boot);
  - `Config.drives: u8` (1 or 2; the plugin's existing `drives` option if present, else default 2 `[H?]`).
- Facts (each a row):

  | Fact | Value | Label |
  |---|---|---|
  | DMA mode bits | 1 A0, 2 A1, 3 HDC, 4 sector count, 6 DMA off, 8 write | `[H]` *Internals* p.59; bit 7 not decoded `[H?]` |
  | DMA status | bit 0 error (0 = OK), bit 1 sector count not zero, bit 2 FDC DRQ | `[H]` |
  | `$FF8604` read, FDC selected | FDC register in the low byte; high byte per the conflict table | `[H?]` |
  | Address counter | `$FF8609/0B/0D`, read/write; DMA advances it | `[H]` |
  | Writing the write bit (bit 8) both ways | clears the FIFO and status | `[H?]`, EmuTOS's `$0190`/`$0090` pattern |
  | FIFO | 16 bytes; one burst per 16 bytes, no bus stall in Phase 1 | `[H?]`, design §8 |
  | FDC clock | the CPU clock (8 MHz) `[H?]`; `Wd1770::new(drive, 8, true)` | WD1772 |
  | INTRQ | GPIP I5, active low | `[H]` *Internals* p.61 |
  | Idle level | high | `[O]` `$A1` |
  | PSG port A | bit 0 side (0 = side 1 `[H?]`), bits 1/2 select drive 0/1, active low | `[H]` p.60; polarity `[H?]` with `[O]` (`$07` = none selected) |
  | No disk in the selected drive | no index pulses; write protect reads set; FDC status `$E4` after the force-interrupt sequence | `[O]` value, the mechanism `[H?]` |
  | ACSI | HDC accesses with no device: writes are dropped; I5 never asserted; a read stops with "no hardware fact" | `[O]` for writes (EmuTOS's probe) |
  | Rotation | 300 rpm = `TRACK_BYTES` byte times per revolution | `[H?]` |

- RESET instruction: the FDC and DMA are not reset `[H?]`.

**Steps:**

- [ ] **Step 1: Failing tests.**
  - `tests/drive.rs`:
    - `index_once_per_6250_byte_times`;
    - `no_disk_no_index`;
    - `step_and_track0`;
    - `side_select_reads_the_other_side`;
    - `deselected_drive_does_not_rotate_data_into_the_fdc`;
    - `eject_during_read_sector_never_panics`: the command ends with the datasheet's record-not-found or timeout
      status, and DMA stops;
    - `side_change_mid_command`: the head reads the other side's bytes from that point;
    - `drive_b_with_drives_1_is_empty`: selecting drive 1 with `--drives 1` behaves like an empty drive.
  - `tests/dma.rs`. Each is a bare-metal program or direct bus writes:
    - `mode_register_selects_fdc_registers` (`$0080` status, `$0082` track, `$0084` sector, `$0086` data);
    - `sector_count_register`;
    - `address_counter_bytes_read_back`;
    - `read_sector_1_into_ram`: a 720K image built in the test with a known pattern. The program sets the address,
      sector count 1, `$0090`/`$0190` toggles, then FDC READ SECTOR. The 512 bytes land in RAM; DMA status bit 0
      is set (OK) and bit 1 clear; GPIP I5 goes low at the end;
    - `dma_into_rom_is_ignored_and_into_io_stops`;
    - `acsi_write_dropped_and_i5_stays_high`;
    - `fdc_force_interrupt_with_no_disk_status_e4`.
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement.**
  - `FdcTick` events run `Wd1770::tick` in byte-time steps while a command is active.
  - Status reads catch up first.
  - DRQ moves a byte into the FIFO; a full FIFO is written to RAM through `StBus::poke` semantics (RAM only, MMU
    translated).
- [ ] **Step 4: Run** the whole workspace with goldens. Expected: PASS. EmuTOS now runs past the floppy probe; the
  next task sets its milestone.
- [ ] **Step 5: Commit**: "atari-st: DMA chip, WD1772 and floppy drives (read), PSG drive select".

### Task 11: Plugin and host: `.st` media

**Files:**
- Modify: `crates/plugins/atari-st-machine-plugin/src/lib.rs`:
  - `AttachMedia { slot: Disk { drive }, .. }` with a `.st` name: parse and insert;
  - `DetachMedia`;
  - `drives` option (1|2, default 2);
  - the chip snapshots gain `dma` and `fdc` rows.
- Modify (host, `master`):
  - `apps/retroclawd/src/plugin_bridge.rs` `apply_media`: `"st"` → `apply_disk`;
  - `apps/retroclawd/src/dialogs.rs`: `st` in the disk filter for `atari-*` machines and in the extension map.
- Test: plugin unit tests; host `plugin_bridge` unit test with the stub plugin (the extension routes to `AttachMedia`
  Disk)

**Steps:**

- [ ] **Step 1: Failing tests.**
  - Plugin: `attach_st_inserts_into_drive_a`, `attach_garbage_is_an_error_with_the_geometry_message`,
    `drive_b_refused_with_drives_1`.
  - Host: `st_extension_is_a_disk`.
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the plugin tests and `cargo test -p retroclawd plugin_bridge`. Expected: PASS.
- [ ] **Step 5: Commit** in both repositories ("atari-st plugin: .st disks" / "Host: .st is a disk image").

### Task 12: EmuTOS milestones: 1500 frames, a boot floppy, the desktop (slice 5 milestone)

**Files:**
- Modify: `tests/emutos.rs`, `tests/common/mod.rs` (`run_frames`, `boot_floppy`)
- Golden (not committed): `RC_GOLDENS/atari-st/emutos14-uk-io-1m-1500.txt` and
  `emutos14-uk-io-1m-boot.txt`, captured with the Task 5 script

**Interfaces:**
- Produces:
  - `fn run_frames(rom, ram, frames, disk: Option<Image>) -> AtariSt` (test helper);
  - `fn boot_floppy() -> Image`: a 720K image built in memory. Its boot sector is executable: the big-endian word sum
    of the 256 words is `$1234` `[H]` (*TOS hyp*, "Boot sector"); a checksum-fixing word is used. Its code sets
    palette 0 to `$0123`, writes `$CAFE` to `$600`, then loops with `BRA.S *`. The bytes come from our own
    hand-assembled 68000 code (as `interrupts.rs` does).

**Steps:**

- [ ] **Step 1: Capture the goldens** (scratchpad, then `RC_GOLDENS`):
  - `--kind io --memsize 1 --vbls 1500` → `emutos14-uk-io-1m-1500.txt`;
  - `--disk-a <boot_floppy written by a small test that dumps it>` → `emutos14-uk-io-1m-boot.txt`.

  Ledger the line counts. The boot floppy dump goes only to the scratchpad.
- [ ] **Step 2: Failing tests.**
  - `emutos_1500_frames_without_a_disk_match_hatari`: no stop; `collapse_polls` logs equal over the whole golden.
    Any `$FF8604` high-byte ruling (Task 10) applies through a named mask function.
  - `emutos_boots_our_floppy`: `$600` holds `$CAFE` and palette 0 is `$0123` within 1500 frames. The I/O log
    equals Hatari's boot golden up to the boot program's first instruction.
  - `emutos_desktop_frame_is_deterministic`: three runs of 1500 frames give identical `render_frame()` bytes. Write
    that frame as a PNG to `RC_SCREENSHOT_DIR` when it is set, for the owner to look at.
- [ ] **Step 3: Run.** Expected: FAIL. Fix the device code until they pass. Every fix is a device behaviour with a
  fact row, never a special case for EmuTOS. Each divergence found is ledgered with its line number and cause.
- [ ] **Step 4: Run** the whole atari workspace, clippy and fmt. Expected: PASS. Then a release run:
  `retroclawd atari-st --firmware auto --headless --frames 1500 --dump-screen <scratchpad>/desktop.png` with the
  freshly built plugin. The PNG shows the EmuTOS desktop (checked by eye; the PNG path goes in the final report).
- [ ] **Step 5: Commit**: "atari-st: EmuTOS runs 1500 frames like Hatari, boots a floppy, shows the desktop".

### Task 13: Input checked through EmuTOS; plugin release run

**Files:**
- Test: `tests/emutos.rs` (`emutos_sees_a_key`)
- Modify: `machines/atari/README.md`

**Steps:**

- [ ] **Step 1: Failing test.** `emutos_sees_a_key`: at frame 1200, `key_event("KeyA", true)`, then false at 1205.
  EmuTOS's keyboard buffer has received scan code `$1E`. Assert it through the bytes EmuTOS read from `$FFFC02`
  (the I/O log), not through EmuTOS internals.
- [ ] **Step 2: Run, implement any missing wiring, run.** Expected: PASS.
- [ ] **Step 3: Live check** with the release app and plugin:
  `retroclawd atari-st --firmware auto --headless --frames 1600 --at 1200:key_down=KeyA --at 1205:key_up=KeyA`
  exits 0. Then a GUI run, by eye: the desktop draws, the mouse pointer moves, and a double-click on drive A with the
  boot floppy inserted gives EmuTOS's reaction. A failure here is a finding for the ledger, not something to hide.
- [ ] **Step 4: Commit**: "atari-st: keyboard input checked through EmuTOS".

### Task 14: Final review and fix pass

- [ ] Run the whole-branch review (executing-plans "Final Review") over all five branches. The reviewer gets this
  plan's Review Focus verbatim and the ledger's rulings.
- [ ] Fix pass per the skill: each Critical or Important finding gets a test that fails first, then the fix.

### Task 15: Docs, pins, integration (owner-gated push)

**Files:**
- Modify: `machines/atari/{CLAUDE.md,README.md,docs/hardware-facts.md}`; `shared/CLAUDE.md` (`mc6850`, `wd1770`, YM
  variant notes)
- Modify (host): `CLAUDE.md` "Atari ST" section (slices 3-5), `TODO.md` (next: slice 6, the Phase 1 exit; see
  below), the spec's Status section

**Steps:**

- [ ] **Step 1:** Write the docs.
- [ ] **Step 2: Ask the owner** to approve pushing. The order:
  1. retroclawd-crates `atari-slices-3-5` → `main`;
  2. `tools/bump_pin.sh <new rev>` (moves every family's pin, runs their tests, commits);
  3. retroclawd-commodore `wd1770-from-crates` → `main` (after the bump, so its manifest points at the new rev);
  4. retroclawd-atari and retroclawd-tools branches → `main`;
  5. the host commit with all submodule pointers.

  Push only after the answer.
- [ ] **Step 3:** After the bump, remove the `.cargo/config.toml` patches. Then run `tools/check_all.sh` with every
  submodule on its pushed commit. Expected: "all checks passed".
- [ ] **Step 4:** Delete the merged branches and this plan's workspace.

## After this plan: slice 6 (Phase 1 exit, not planned here)

- An `atari-st.manifest.toml` with S1/S2/S4/S5/S6 scenarios against Hatari, including the PSG tone and keyboard,
  mouse and joystick through `--at`.
- A purpose-built FAT12 floppy (our tools) listed on the desktop.
- The injected-fault diagnostics test.
- The 68000 SingleStepTests expected-failure list justified.
- `check_all.sh` running the Atari manifest.
