# Atari ST Slice 6 ("Phase 1 exit") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Meet the design's Phase 1 exit criteria (spec §24). An `atari-st.manifest.toml` runs in `tools/check_all.sh`,
with these scenarios:
- the EmuTOS desktop against a Hatari 2.6.1 screenshot;
- keyboard, mouse and joystick input through `--at`;
- a floppy made by our own `rcdisk` that opens on the desktop;
- a YM2149 tone checked against the datasheet's formula;
- the frame length.

Two further items close the list:
- an injected-fault test of the diagnostics;
- written justifications for the 68000 SingleStepTests cases that do not pass.

**Architecture:**
- **No new emulation.** Slice 6 is evidence and tooling:
  - `rcdisk` learns Atari FAT12 `.st` disks;
  - the host's `--at` learns the mouse;
  - a Hatari screenshot script supplies the reference picture;
  - small scenario programs (hand-assembled, built by a script) make keyboard, joystick and sound visible.
- **Goldens:**
  - A Hatari picture is compared with `mode = "similarity"`.
  - A scenario Hatari cannot drive is an `exact` golden this project makes. Hatari has no headless input injection.
    This covers the keys, the mouse, the joystick and the opened floppy window. Each such golden is checked by eye
    once and recorded with its SHA-256, as the rc-scenarios rules allow ("`mode = "exact"` is for goldens this
    project made itself").
- **Tone check.** The `audio` scenario checks the pitch the YM2149 datasheet's formula gives
  (`fT = fMaster / (16 TP)`), not a recording from another emulator.

**Tech Stack:**
- Rust 2024 for `rcdisk`, the host and the atari tests;
- Python standard library for the scripts;
- no new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-19-atari-st-design.md`, §21 (testing), §23 item 9, §24 (Phase 1 exit criteria).
Previous plan: `docs/superpowers/plans/2026-10-21-atari-st-slices3-5.md`.

**Repositories and branches:**

| Repository | Branch | Work |
|---|---|---|
| retroclawd-tools | `atari-phase1-exit` | Task 1 (`rcdisk` st), Task 3 (Hatari screenshot script) |
| retroclawd (host) | `master` | Task 2 (`--at` mouse), Task 7 (`check_all`), docs |
| retroclawd-atari | `atari-st-slice6` | Task 4 (scenario programs and build), Task 5 (manifest), Task 6 (injected fault) |
| retroclawd-crates | `singlestep-expected` | Task 6 (SingleStepTests expected-failure list and its justification) |

## Global Constraints

Everything in the slices 3-5 plan's Global Constraints still applies:
- clean-room;
- fact labels, and no hardware fact from memory;
- `#![forbid(unsafe_code)]` and GPL-3.0-or-later;
- never commit goldens, firmware, datasheets or disk images (the build script makes scenario media into the
  git-ignored `software/scenarios/`);
- commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`;
- pushing needs the owner's go-ahead.

Also:
- **EmuTOS for the scenarios** comes from `--firmware auto`, which uses the cache and downloads by hash when it is
  missing. Nothing proprietary, and no TOS, is needed.
- **New FAT12 facts** come from Microsoft's *FAT: General Overview of On-Disk Format* (v1.03, 2000) `[H]`,
  directory entries and FAT12 packing, and from *Atari ST Internals* pp.178-179 (the ST's BPB values) `[H]`. They
  go in `retroclawd-tools/CREDITS.md` with URL and SHA-256.
- **Scenario programs** are 68000 machine code written as hex words with a comment per instruction (no 68000
  assembler exists yet; spec §15 is later). They are checked by running them in tests, not by eye alone.

## Review Focus

1. **`rcdisk st` on hostile input.** These must be errors, never a panic or an endless loop:
   - a FAT12 image with a cyclic cluster chain;
   - a chain out of range;
   - a directory entry with a size larger than its chain;
   - a disk full when adding a file.

   Tests in Task 1.
2. **`--at mouse_move` with large deltas.** Motion beyond ±127 is split into several `MouseMotion` events, with no
   overflow. Unknown buttons are an error naming the valid ones. Tests in Task 2.
3. **Scenario determinism.** Each `exact` golden must come out the same on three runs before its hash is recorded
   (Task 5 step). A scenario that is not deterministic is a finding, not a golden.
4. **`check_all` without goldens or network.** The Atari manifest must skip cleanly with a message, as the other
   families' manifests do, when `RC_GOLDENS` is unset or EmuTOS can be neither found nor fetched. Test in Task 7.
5. **The injected fault reaches the user.** The test checks the report itself: the failing instruction, the address,
   "no hardware fact", and the bounded trace. Task 6.

---

### Task 1: `rcdisk st`: Atari FAT12 `.st` disks (retroclawd-tools)

**Files:**
- Create: `crates/rcdisk/src/st.rs` (adapter on the crate's `Disk` trait), `crates/rcdisk/tests/st.rs`
- Modify: `crates/rcdisk/src/lib.rs`, `src/main.rs` (CLI), `docs/rcdisk.md`, `CREDITS.md`

**Interfaces:**
- Produces:
  - CLI `rcdisk st new OUT.st [--sides 1|2] [--sectors 9|10]` (default 2 sides, 9 sectors: 720K);
  - CLI `rcdisk st add DISK.st FILE [--name NAME.EXT]`, `rcdisk st list DISK.st`, `rcdisk st extract DISK.st NAME OUT`;
  - CLI `rcdisk st boot DISK.st CODE.bin` (puts code at offset 30, `BRA.S` at 0, fixes the checksum word to `$1234`);
  - lib `st::StDisk` implementing the crate's `Disk` trait (read the trait first and follow it).
- Facts:
  - BPB values for 80 tracks, 1 or 2 sides, as *Internals* p.178's table: 512 bytes/sector, 2 sectors/cluster,
    1 reserved, 2 FATs, 112 directory entries, media `$F8`/`$F9`, 5 sectors/FAT, 9 sectors/track;
  - FAT12 entry packing, the end-of-chain marker (`$FF8-$FFF`), and directory entry layout (8.3 name, attribute,
    start cluster, size; little-endian) from the Microsoft document;
  - the boot checksum rule from *Internals* p.179.

**Steps:**

- [ ] **Step 1: Failing tests** (`tests/st.rs`):
  - `new_720k_has_the_internals_bpb` (every field above);
  - `add_then_list_and_extract_round_trip` (3 files, one of 3000 bytes spanning clusters);
  - `boot_code_makes_the_sector_executable` (the word sum is `$1234`, the code is at 30);
  - `disk_full_is_an_error`;
  - `cyclic_chain_is_an_error`;
  - `out_of_range_chain_is_an_error`;
  - `size_beyond_chain_is_an_error`;
  - `bad_names_are_refused` (more than 8.3, lower case folded to upper case per the Microsoft document).
- [ ] **Step 2: Run.** `cargo test -p rcdisk --test st`. Expected: FAIL.
- [ ] **Step 3: Implement.** Checked arithmetic. The chain walk is bounded by the cluster count.
- [ ] **Step 4: Run** `cargo test -p rcdisk`, clippy and fmt. Expected: PASS. Then by hand: an image made by
  `rcdisk st new` + `add` boots in Hatari (`--disk-a`), and its desktop drive A window lists the files. Check by eye;
  the screenshot is taken with Task 3's script.
- [ ] **Step 5: Commit** (tools): "rcdisk: Atari FAT12 .st disks".

### Task 2: `--at` mouse actions (host)

**Files:**
- Modify: `apps/retroclawd/src/scripted.rs` (actions), the `--at` help text, `CLAUDE.md` (`--at` paragraph)
- Test: `scripted.rs` unit tests with the stub machine

**Interfaces:**
- Produces:
  - `--at FRAME:mouse_move=DX,DY`: host pixels, right and down positive. It becomes one or more
    `MachineCommand::MouseMotion { dx, dy }`, each within -128..127; a larger move is split into equal steps.
  - `--at FRAME:mouse_down=left|right|middle`, `mouse_up=...`: `MachineCommand::MouseButton`.
  - An unknown button or a malformed delta is a parse error naming the valid forms.

**Steps:**

- [ ] **Step 1: Failing tests:**
  - `parse_mouse_actions`;
  - `a_large_move_is_split_into_steps_that_sum_to_it` (`300,-200` gives steps of at most 127/128 that sum exactly);
  - `unknown_button_is_an_error`;
  - `mouse_actions_reach_the_machine` (the stub plugin records the `MouseMotion`/`MouseButton` it receives; add that
    recording to `test-machine-plugin` if it is missing).
- [ ] **Step 2: Run** `cargo test -p retroclawd scripted`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the host tests, clippy and fmt. Expected: PASS. Live: `retroclawd atari-st --firmware auto
  --headless --frames 1600 --at 1500:mouse_move=-200,-100 --dump-screen x.png` shows the pointer moved (by eye).
- [ ] **Step 5: Commit** (host): "--at: mouse_move, mouse_down, mouse_up".

### Task 3: Hatari screenshot capture (retroclawd-tools)

**Files:**
- Create: `scripts/capture_hatari_screen.py`
- Modify: `CLAUDE.md` (the capture scripts section)

**Interfaces:**
- Produces: `capture_hatari_screen.py ROM VBLS OUT.png [--memsize N] [--disk-a IMG] [--hatari PATH]`. It writes a
  debugger command file:
  - a breakpoint on `VBL` reaching VBLS, which runs a second file with `screenshot OUT.png` then `quit`;
  - Hatari runs with `--parse`.

  Read the exact breakpoint and `:file` syntax in Hatari's `doc/debugger.html`, not from memory. Black box, standard
  library only.

**Steps:**

- [ ] **Step 1:** Read `debugger.html` (breakpoints with conditions, `:file`, `screenshot`, `quit`). Write the script.
- [ ] **Step 2: Check by hand.**
  - `capture_hatari_screen.py etos192uk.img 1500 S1.png` writes a PNG of the EmuTOS desktop. Look at it.
  - Run it twice; record whether the two PNGs are byte-identical. That decides how strict S1's comparison can be.
- [ ] **Step 3: Commit** (tools): "capture_hatari_screen: a Hatari screenshot at a given VBL".

### Task 4: Scenario programs and media (retroclawd-atari)

**Files:**
- Create: `scenarios/build.sh` (calls `rcdisk` and the script, writes `software/scenarios/`)
- Create: `scenarios/make_scenarios.py` (writes the code blobs from documented hex words)
- Create: `scenarios/README.md`, which describes each program instruction by instruction
- Modify: `.gitignore` (`/software/` is already ignored; check it)

**Interfaces:**
- Produces, in `software/scenarios/`:
  - `tone.bin`: for `--program` at `$10000`. It sets YM R7 = `$3E`, R8 = `$0F`, R0/R1 = 284, then loops `BRA.S *`.
    The YM register writes go through `$FF8800`/`$FF8802` with `MOVE.B` absolute short, as in `tests/psg.rs`. Check
    that `--program` on the atari-st starts at the given PC in supervisor mode; if it does not, rule and adapt.
  - `keys.st`: a 720K disk (`rcdisk st new`) whose boot program does two things:
    - it installs a joystick handler through XBIOS `Kbdvbase` (trap #14, function 34), which writes the joystick
      packet's state byte into palette 0;
    - it loops on BIOS `Bconin(2)` and `Bconout(2, c)` (trap #13, functions 2 and 3), echoing typed characters to the
      console.

    The TOS calls are `[H]` from *Internals* (BIOS/XBIOS chapters, function numbers and arguments); cite the pages
    in `README.md`.
  - `files.st`: a 720K disk with `HELLO.TXT` (text) and `README.TXT` (`rcdisk st add`), not bootable.

**Steps:**

- [ ] **Step 1: Failing tests.** A new `crates/machines/atari-st/tests/scenarios.rs`, skipped with a note when
  `software/scenarios/` is missing, runs each program on the machine:
  - `tone_bin_plays_the_formula_pitch` (zero crossings, as in `tests/psg.rs`);
  - `keys_st_echoes_a_typed_key` (EmuTOS boots `keys.st`; `key_event` H then I; the I/O log shows `$23`/`$17` read,
    and the console's video RAM changes);
  - `keys_st_joystick_sets_palette_0` (Joy1 fire gives palette 0 = the event byte `$80`, through the installed
    handler).
- [ ] **Step 2: Run.** Expected: FAIL (no media).
- [ ] **Step 3: Write** `make_scenarios.py`, `build.sh` and `README.md`. Run `scenarios/build.sh` (with `RCDISK`
  pointing at the tools build).
- [ ] **Step 4: Run** the scenario tests. Expected: PASS.
- [ ] **Step 5: Commit** (atari): "Scenario programs: tone, keyboard/joystick floppy, file floppy".

### Task 5: `atari-st.manifest.toml` (retroclawd-atari)

**Files:**
- Create: `machines/atari/atari-st.manifest.toml`
- Goldens (not committed): `RC_GOLDENS/atari-st/S1-desktop.png` (Hatari, Task 3) and the exact goldens made here

**Scenarios:**

| Id | Kind | What |
|---|---|---|
| S1 | screen, `similarity` | 1500 frames, the EmuTOS desktop against Hatari's screenshot. The crop avoids the pointer; state the reason |
| S2 | screen, `exact` | `keys.st`; `--at` types `HI` (key down/up pairs); the console shows the echoed text |
| S2-joy | screen, `exact` | `keys.st`; `--at` `Joy1Fire1` down; palette 0 changes the background |
| S2-mouse | screen, `exact` | the desktop; `--at mouse_move` takes the pointer to a set place |
| S3 | screen, `exact` | `files.st` in drive A; `--at mouse_move` to the drive A icon, then a double click (`mouse_down`/`mouse_up` twice); the window lists `HELLO.TXT` and `README.TXT` |
| S4 | audio | `--program tone.bin`; `tone_hz` = CPU clock / 4 / (16 × 284), computed in a comment from the PAL CPU clock 8 021 247 Hz `[H]`, tolerance 1% |
| S6 | timing | `cycles_per_frame = 160256` (512 × 313, PAL `[H]`); the tolerance is the longest 68000 instruction the machine can overrun by, stated with its source (the 68000 user's manual timing tables) |

The manifest names `[reference] emulator = "hatari"`, `version = "2.6.1"`, and the capture command. In comments it
says:
- S5 is absent: save states are Phase 2;
- S7 is absent: no ST hardware test program is shipped;
- why each `exact` scenario has no Hatari reference.

**Steps:**

- [ ] **Step 1:** Capture S1's golden with Task 3's script into `RC_GOLDENS/atari-st/`.
- [ ] **Step 2:** Write the manifest without hashes. Generate each `exact` golden three times with `rc-scenarios`
  (its golden-writing mode; read `retroclawd-tools/CLAUDE.md` for the command) and check the three are identical.
  Look at each picture and say in the manifest comment what it shows. Then record the hashes.
- [ ] **Step 3: Run** `rc-scenarios run machines/atari/atari-st.manifest.toml` with the release binary and plugins.
  Expected: every scenario passes.
- [ ] **Step 4: Mutation check.** Run S3 with `files.st` swapped for `keys.st`, and S4 with TP = 300. Each must fail.
  Record the failure messages in the ledger.
- [ ] **Step 5: Commit** (atari): "atari-st.manifest.toml: Phase 1 scenarios".

### Task 6: Injected fault and the SingleStepTests expected failures

**Files:**
- Modify: `machines/atari/crates/machines/atari-st/tests/diagnostics.rs` (EmuTOS fault test)
- Modify (crates, branch `singlestep-expected`): `crates/core/68000/tests/singlestep.rs` and new
  `crates/core/68000/tests/singlestep-expected.md`

**Steps:**

- [ ] **Step 1: Failing test.** `emutos_with_an_injected_fault_reports_it`:
  - copy EmuTOS in memory, and replace one instruction on its boot path (found by the PC trace, before the MFP
    set-up) with `MOVE.B $FF9000,D0`;
  - the run stops;
  - the report (`take_stop_report`) contains "no hardware fact", `$FF9000`, the disassembled instruction, the
    registers and a bounded trace;
  - two runs give the same report.

  Skipped without EmuTOS.
- [ ] **Step 2: Run, implement any missing report part, run.** Expected: PASS.
- [ ] **Step 3: SingleStepTests.** Make `singlestep.rs` hold an explicit expected-failure list (opcode group,
  count, reason):
  - ASR carry: 3 736 cases, where the test data disagrees with the PRM's ASR flag table;
  - two bad upstream cases: issue #4.

  The test then fails if any other case fails, or if a listed case starts passing. `singlestep-expected.md` gives
  each reason with its source: the PRM page and table, and the upstream issue link. Run it (the suite is fetched by
  `scripts/fetch_singlesteptests.sh`; if the suite is absent here, fetch it into the git-ignored folder first).
  Expected: PASS with the list.
- [ ] **Step 4: Commit** (atari and crates).

### Task 7: `check_all` with the Atari family

**Files:**
- Modify: `tools/check_all.sh` (only if needed), `tools/check_all.ps1` if it has its own list

**Steps:**

- [ ] **Step 1:** Check how `check_all.sh` finds manifests (`machines/*/*.manifest.toml`), firmware and goldens. The
  Atari manifest needs `--firmware auto` with a cache.
  - Without `RC_GOLDENS`, the manifests are skipped with a message.
  - Without network or cache, the Atari scenarios must skip with a clear message, not fail. Add that to
    `rc-scenarios` (tools) if it does not handle a `--firmware auto` failure. Test it there.
- [ ] **Step 2:** The slices 3-5 integration run lost `target/release/retroclawd.exe` during the rc-suite steps
  (`cannot start ... file not found` for the Spectrum and ZX81 suites). If it happens again, find the cause:
  - a build step replacing the binary;
  - an antivirus quarantine;
  - a cleanup in a test.

  Fix it, or ledger it with the evidence.
- [ ] **Step 3: Run** `tools/check_all.sh`. Expected: "all checks passed", including `scenarios-atari-st`.
- [ ] **Step 4: Commit** (host) if anything changed.

### Task 8: Final review, docs, push (owner-gated)

- [ ] Whole-branch review over the four repositories (executing-plans "Final Review"), with this plan's Review
  Focus. One fix pass.
- [ ] Docs:
  - spec Status "Phase 1 done";
  - host CLAUDE.md Atari section;
  - TODO.md: Phase 2 next, with the items deferred from slices 3-5;
  - atari README;
  - tools CLAUDE.md for `rcdisk st` and the screenshot script.
- [ ] Ask the owner, then:
  - push crates, then `tools/bump_pin.sh` if crates changed;
  - push the tools, atari and host branches;
  - run `check_all`;
  - delete the branches and the plan's workspace.
