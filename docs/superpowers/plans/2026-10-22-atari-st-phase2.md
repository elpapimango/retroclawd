# Atari ST Phase 2 (slices 7-12) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the spec's Phase 2 column (§5): floppy writing, formatting and `.msa`; save states (S5); the MFP
USART, MIDI, printer and RS-232; the full IKBD command set; cartridge and unmapped-bus behaviour; a line-based raster
palette and the sync/resolution border foundations; the 68000 assembler and disassembler with the `.PRG` toolchain;
ACSI hard disks with AHDI partitions and FAT16. Phase 2 ends with a hard-disk scenario and a `.PRG` that runs from a
generated floppy.

**Architecture:**
- Same shape as Phase 1: emulation in `retroclawd-atari`, shared chips in `retroclawd-crates`, tools in
  `retroclawd-tools`, host changes only where a protocol or routing gap forces them.
- Six slices, each ending with something visible (a scenario, a round trip, a screenshot) and a green `check_all`:
  7 floppy write, 8 save states, 9 peripherals and the IKBD, 10 GLUE/cartridge/raster, 11 68000 toolchain,
  12 ACSI. Slices 7-10 touch the machine and are ordered by dependency (8 needs the state of everything 7 adds; 9 and
  10 add state that 8's envelope already carries). Slice 11 touches only tools and can run in parallel after slice 7.
- **No ABI bump is planned.** Dirty-disk export reuses `ExportDisk`/`MarkDiskSaved`/`DiskImage` (protocol 3),
  cartridges and hard disks reuse `MediaKind::Cartridge`/`Other("acsi")`. The one protocol question (memory
  watchpoints, spec §5 and §25) is settled by Decision 1 below, not by this plan.

**Tech Stack:** Rust 2024; Python standard library for scripts; `crc32fast` and `bincode` v2 only where the other
families already use them (check each family's `Cargo.toml`; no new dependency otherwise).

**Spec:** `docs/superpowers/specs/2026-10-19-atari-st-design.md`: §5 (hardware by phase), §10 (ACIAs), §13 (floppy
formats), §14 (ACSI), §15 (tools), §17 (plugin/ABI), §19 (save states), §20 (security), §23 item 10-11, §25
(deferrals). Previous plan: `docs/superpowers/plans/2026-10-21-atari-st-slice6.md`.

**Repositories and branches:**

| Repository | Branch | Slices |
|---|---|---|
| retroclawd-atari | `atari-st-slice7` ... `atari-st-slice12` (one per slice, merged to `main` before the next starts) | 7-10, 12; Task 20's floppy builder |
| retroclawd-crates | `atari-phase2` | `wd1770` write/format support if the shared chip lacks it (Task 1), `mc6850` MIDI clock if needed (Task 9) |
| retroclawd-tools | `atari-phase2-tools` | 11 (rcasm, rcdis, rcld, PRG), 12 (`rcdisk` partitions and FAT16), `rc-scenarios` additions |
| retroclawd (host) | `master` | `.prg` routing (Task 20), docs, pins (`--at save_disk` already exists) |

## Global Constraints

Everything in the slice 6 plan's Global Constraints still applies, verbatim:
- clean-room: no Hatari, ASE or EmuTOS code, tables, constants or comments copied; Hatari stays a black box;
- every hardware fact has a label (`[D]` datasheet, `[H]` documentation, `[O]` observed in Hatari, `[H?]` unresolved)
  and a source in `machines/atari/docs/hardware-facts.md`; an unsourced I/O address or behaviour stops the machine
  with the existing report instead of returning an invented value;
- `#![forbid(unsafe_code)]`, GPL-3.0-or-later, big-endian access explicit, machine time only from the scheduler;
- all firmware, media, states, symbols, executables and sources are untrusted: checked arithmetic, size caps, no
  `unwrap` on input, atomic writes (temp + rename);
- never commit goldens, firmware, datasheets or disk images, and never print their contents in logs;
- EmuTOS is the firmware for CI (`--firmware auto`); no proprietary TOS is bundled, downloaded or required;
- commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`; pushing needs the owner's go-ahead.

Added for Phase 2:
- **Sourcing gate.** A task that adds hardware or format behaviour starts with a step "source the facts". If a primary
  source cannot be fetched (Atari's hardware specification is behind Cloudflare from here), the task records the fact
  as `[H?]`, implements only what the sourced facts cover, and the executor stops and asks the owner to download the
  document rather than filling the gap. Candidate facts are never taken from memory.
- **Phase 2 exit criteria** (my proposal; spec §24 only defines Phase 1, so this is a ruling for you to confirm):
  1. a program writes a sector through TOS and the exported image lists the new data in `rcdisk st list`;
  2. a state saved at frame N and loaded into a fresh machine matches a straight run (frame hash, RAM hash, I/O
     trace) for 3 machine runs;
  3. the IKBD answers every command of Atari's protocol document that the spec's matrix lists, each tested;
  4. a `.PRG` made by `rcasm` + `rcld` runs from a generated floppy and prints a known string on screen;
  5. EmuTOS sees a hard disk image with an AHDI partition as drive C: and lists a file made by `rcdisk`;
  6. `tools/check_all.sh` includes every new scenario (S3W, S5, S9, S10) and passes; every new parser has a fuzz
     target and hostile-input tests.

## Decisions for the owner

These change what gets built. Each has a recommendation; the plan follows it unless you say otherwise.

1. **Memory watchpoints (spec §5, §25).** Recommend: do not add them in Phase 2; no ST debugging need has appeared,
   and the protocol bump touches every family. Revisit with the trace file in hand.
2. **`.stx` (Pasti).** Recommend: Phase 2 does `.msa` only; `.stx` waits for a source with
   a clear licence (spec risk 4). Task 24 produces the licence/provenance note, not code.
3. **`.dim`.** Recommend: skip until someone has a file to test with. The spec says "late Phase 2" and the format's
   header layout is unresearched.
4. **PRG on the ST without GEMDOS emulation.** Recommend the spec's floppy injection (Task 20): the plugin builds an
   in-memory `.st` with `AUTO\PROG.PRG`. This duplicates a small FAT12 writer that `rcdisk st` already has (tools
   cannot be a dependency of the atari repository); the duplicate is a known cost, kept to the boot-only subset.

## Review Focus

1. **Hostile images and states.** Every new parser (`.msa` RLE, the RCST envelope, AHDI partition chains, FAT16
   chains, `.PRG` fixup tables) refuses: truncated input, a size or count field beyond its cap, a decompression
   expansion beyond the track size, a cyclic chain, an extended-partition loop. Tests in the owning task, a fuzz
   target per parser (Task 25 writes any still missing and runs them all).
2. **A write must never lose or corrupt user media.** A disk written by the guest changes only the in-memory image;
   the file changes only by `--disk-writeback`/`--disk-save`, atomically; a write-protected image (option) refuses
   writes the way the hardware does. Tests Tasks 1 and 3.
3. **A state from another machine, firmware or version.** Refused with a message naming both sides; a media hash
   mismatch loads with a warning. Task 6.
4. **Determinism.** Every new device (USART, MIDI, printer, IKBD clock, raster palette) takes its time from the
   scheduler only; the three-run identical-hash check from Phase 1 is repeated for each new scenario before its
   golden is recorded.
5. **Output files.** The MIDI/printer/serial sinks never overwrite an existing file without being asked and never
   follow the guest into a path outside the one the user gave. Tasks 8-9.

---

## Slice 7: floppy write

### Task 1: WD1772 write path (retroclawd-crates, retroclawd-atari)

**Files:**
- Modify: `shared/crates/chips/wd1770/src/` if the chip lacks WRITE SECTOR/WRITE TRACK data flow;
  `crates/machines/atari-st/src/drive.rs`, `dma.rs`, `bus.rs`; `docs/hardware-facts.md`
- Create: `crates/formats/st-floppy/src/decode.rs`
- Test: `crates/formats/st-floppy/tests/decode.rs`, `crates/machines/atari-st/tests/floppy_write.rs`

**Interfaces:**
- Produces:
  - `st_floppy::decode::sectors_from_mfm(track_bytes: &[u8]) -> Result<Vec<Sector>, DecodeError>`: finds ID and data
    address marks, checks both CRCs, returns sectors in found order; a bad CRC is an error naming track, sector.
  - `Drive::write(byte)` records MFM bytes at the head position; `Drive::commit_track()` decodes the written track
    into the image's sectors; `Drive::is_dirty() -> bool`.
  - Removes `take_write_attempt` and the write-stops-the-machine path in `bus.rs:249`.

**Steps:**
- [ ] **Step 1: Source the facts** (sourcing gate): WD1772 datasheet sections for WRITE SECTOR, WRITE TRACK (the
  special meaning of `F5`, `F6`, `F7`, `F8-FB`, `FC`, `FE`), the write gate timing, and the write-protect abort. Add
  rows to `hardware-facts.md`.
- [ ] **Step 2: Failing tests:**
  - `decode::mfm_of_a_synthesised_track_decodes_to_the_same_sectors` (round trip with the existing synthesiser);
  - `decode::a_bad_data_crc_is_an_error_naming_the_sector`;
  - `decode::a_track_with_no_address_marks_decodes_to_no_sectors`;
  - `floppy_write::write_sector_changes_the_image` (WRITE SECTOR `$A0` through the DMA registers, then READ SECTOR
    returns the written bytes);
  - `floppy_write::write_track_formats_a_blank_disk` (a track built from `F5/F6/F7` bytes yields 9 sectors);
  - `floppy_write::a_write_protected_disk_refuses_and_sets_the_status_bit`.
- [ ] **Step 3: Run.** `cargo test -p st-floppy -p atari-st`. Expected: FAIL.
- [ ] **Step 4: Implement** per the signatures. Writes reach the image only at `commit_track`, so a half-written track
  never leaves the image inconsistent.
- [ ] **Step 5: Run** the two crates' tests, clippy, fmt. Expected: PASS. Run the Phase 1 suite
  (`cargo test --workspace`): EmuTOS boot and the S3 floppy scenario still identical.
- [ ] **Step 6: Commit** (crates, then atari): "WD1772: write sector and write track".

### Task 2: `.msa` images (retroclawd-atari)

**Files:**
- Create: `crates/formats/st-floppy/src/msa.rs`, `crates/formats/st-floppy/tests/msa.rs`
- Modify: `crates/formats/st-floppy/src/lib.rs` (`Image::from_bytes` recognises `$0E0F`),
  `docs/hardware-facts.md`

**Interfaces:**
- Produces: `msa::parse(bytes: &[u8]) -> Result<Image, MsaError>`, `msa::write(image: &Image) -> Vec<u8>` (RLE
  where it shrinks a track, raw otherwise); caps: input and output ≤ 2 MiB, each track's expansion ≤ sectors-per-track
  × 512.

**Steps:**
- [ ] **Step 1: Source the format** (sourcing gate): a published description of the header (ID, sectors per track,
  sides, first and last track) and the RLE marker. Record URL and SHA-256 in `CREDITS.md`; mark `[H]` or `[H?]`.
- [ ] **Step 2: Failing tests:** `msa::round_trips_every_geometry` (720K, 360K, 800K), `msa::rle_run_longer_than_the_track_is_an_error`,
  `msa::truncated_header_is_an_error`, `msa::a_track_count_beyond_the_cap_is_an_error`,
  `msa::incompressible_track_is_stored_raw`, `msa::parse_of_random_bytes_never_panics` (1000 seeded inputs).
- [ ] **Step 3: Run.** Expected: FAIL. **Step 4: Implement.** **Step 5: Run** crate tests, clippy, fmt.
- [ ] **Step 6: Commit:** "st-floppy: .msa read and write".

### Task 3: plugin write-back and write protection (retroclawd-atari, host)

**Files:**
- Modify: `crates/plugins/atari-st-machine-plugin/src/lib.rs` (`ExportDisk`, `MarkDiskSaved`, options),
  `crates/machines/atari-st/src/config.rs`; host `apps/retroclawd/src/plugin_bridge.rs` only if `.msa` is not yet
  routed to the disk slot
- Test: `crates/plugins/atari-st-machine-plugin/tests/disk_export.rs`

**Interfaces:**
- Produces: `MachineCommand::ExportDisk { drive }` returns `DiskImage { bytes, dirty }` in the image's own format
  (`.st` as `.st`, `.msa` as `.msa`); `MarkDiskSaved { drive }` clears the dirty flag; option `write_protect`
  (`Bool`, per `--drives` count both drives, default false).
- Consumes: Task 1's `Drive::is_dirty`, Task 2's `msa::write`.

**Steps:**
- [ ] **Step 1: Failing tests:** `export_returns_the_written_sector_and_dirty`, `mark_saved_clears_dirty`,
  `an_msa_disk_exports_as_msa`, `write_protect_option_refuses_writes`, `drive_b_export_is_independent_of_drive_a`.
  Model them on `machines/commodore/.../c64-machine-plugin` export tests (read them first).
- [ ] **Step 2-4: Run, implement, run.** Expected after: PASS plus the host unit tests.
- [ ] **Step 5: Commit:** "atari-st plugin: disk export, write protect".

### Task 4: floppy-write scenario S3W

**Files:**
- Create: `machines/atari/scenarios/write.st` source (a boot program in 68000 words, comments per instruction; build
  script entry), `machines/atari/atari-st.manifest.toml` entry
- Modify: `scenarios/build.sh`, `scenarios/README.md`

**Interfaces:** the boot program calls the BIOS `Rwabs` (trap 13, read/write flag 1, one sector, a known buffer of
the text "WRITTEN BY GUEST"), then prints "DONE" through `Bconout`. The scenario runs it, then `--at 400:save_disk=OUT`
exports the disk; a check reads sector 8 of `OUT` with `rcdisk` and compares the text.

**Steps:**
- [ ] **Step 1:** the manifest entry with no program yet. Run `rc-scenarios`: fails on the missing file (RED).
- [ ] **Step 2:** write the program, build it, check the screen shows "DONE", then the sector check. Three runs give the
  same exported image hash. Record it as an `exact` golden hash; note: the hash is of the exported file, never the file
  itself committed.
- [ ] **Step 3: Commit** (atari): "S3W: floppy write scenario". Merge slice 7.

---

## Slice 8: save states

### Task 5: serde on the device state (retroclawd-atari)

**Files:**
- Modify: `crates/machines/atari-st/src/{psg,acia,dma,drive,bus,machine}.rs`, `crates/chips/*/src/lib.rs` where a
  derive is missing, `shared/crates/chips/{ay-3-8910,mc6850,wd1770}` (crates branch) if their state is not
  `Serialize`
- Test: `crates/machines/atari-st/tests/state_roundtrip.rs`

**Interfaces:**
- Produces: `AtariSt::snapshot(&self) -> StatePayload` and `AtariSt::restore(&mut self, StatePayload) -> Result<(),
  StateError>`; `StatePayload` is `Serialize + Deserialize`, holds CPU, RAM, MMU, Shifter, MFP, both ACIAs, IKBD, PSG,
  DMA, FDC, drives (dirty sector data included), scheduler counters, I/O log position. Anything not meant to persist
  (the stop report, host-side sinks) is listed in a doc comment, not silently omitted.

**Steps:**
- [ ] **Step 1: Failing test:** `restore_of_a_snapshot_continues_identically`: boot 50 frames with a test ROM that
  exercises timer, keyboard and floppy, snapshot, run 20 more frames, hash RAM + registers + palette; restore into a
  new machine, run 20 frames, same hash. Fails to compile first (no `snapshot`).
- [ ] **Step 2: Run.** Expected: FAIL. **Step 3: Implement** (derives, then the two functions). Cover the deferred
  item "serde on the PSG/ACIA/DMA/drive state" from the slices 3-5 ledger. **Step 4: Run** crate tests, clippy.
- [ ] **Step 5: Commit** (crates then atari): "Serde for the ST device state; snapshot and restore".

### Task 6: the RCST envelope and `save_state`/`load_state`

**Files:**
- Create: `crates/machines/atari-st/src/state.rs`
- Modify: `machine.rs` (`save_state`, `load_state`), plugin crate for the firmware and media hashes
- Test: `crates/machines/atari-st/tests/state_envelope.rs`

**Interfaces:**
- Produces: `state::encode(payload, meta: &StateMeta) -> Vec<u8>` and `state::decode(bytes, expect: &StateExpect) ->
  Result<(StatePayload, Vec<StateWarning>), StateError>`. Envelope exactly as spec §19: big-endian magic `RCST`,
  schema version, machine id, plugin `version_tag`, `mc68000` state version, accuracy mode, firmware SHA-1, per-drive
  media SHA-1 + dirty flag, payload length, CRC32, then the bincode v2 payload. `StateError` variants: `WrongMachine`,
  `NewerSchema`, `TooLarge` (64 MiB cap), `BadCrc`, `FirmwareMismatch { saved, loaded }`, `Truncated`.
  A media hash mismatch is a `StateWarning`, not an error.

**Steps:**
- [ ] **Step 1: Failing tests:** `round_trip_through_save_and_load_state`, `another_machine_id_is_refused`,
  `a_newer_schema_is_refused`, `a_length_field_beyond_the_cap_is_refused_before_allocating` (tracking allocator, as
  `rcdisk`'s test does), `a_flipped_payload_bit_is_a_crc_error`, `a_different_firmware_names_both_hashes`,
  `a_media_mismatch_loads_with_a_warning`, `truncation_at_every_offset_is_an_error_not_a_panic`.
- [ ] **Step 2-4: Run, implement, run.** **Step 5: Commit:** "atari-st: save states (RCST envelope)".

### Task 7: S5 scenario and fuzz target

**Files:** `atari-st.manifest.toml` (S5, `kind = "state"`, modelled on `c64.manifest.toml` S5),
`crates/machines/atari-st/fuzz/` (a `cargo fuzz` target `state_decode`)

**Steps:**
- [ ] **Step 1:** manifest entry: `frames = 1400`, `more_frames = 100`, EmuTOS desktop with a mouse move at frame 1300
  so the pointer is part of the state. Run: expect the state kind to report equal frames.
- [ ] **Step 2:** the fuzz target feeds arbitrary bytes to `state::decode`; run 60 s locally, no panic. Add the target
  to the list Task 25 runs.
- [ ] **Step 3: Commit** and merge slice 8; bump `retroclawd-crates` pin through `tools/bump_pin.sh` if Task 5 changed it.

---

## Slice 9: serial, MIDI, printer and the IKBD

### Task 8: MFP USART with an RS-232 peer

**Files:** Modify `crates/chips/mc68901/src/` (USART: RSR, TSR, UDR, SCR, UCR, the receive/transmit interrupts),
`crates/machines/atari-st/src/acia.rs` or a new `serial.rs` (peer trait), `io.rs`; Test: `crates/chips/mc68901/tests/usart.rs`,
`crates/machines/atari-st/tests/serial.rs`

**Interfaces:**
- Produces: `trait SerialPeer { fn tx(&mut self, byte: u8); fn rx(&mut self) -> Option<u8>; }` with `Loopback` and
  `FileSink { path }`/`FileSource { path }` implementations; options `serial_out` (path), `serial_in` (path),
  `serial_loopback` (`Bool`). Baud timing from the MFP timer D, [D] MC68901 §USART.

**Steps:**
- [ ] **Step 1: Source the facts** (MC68901 datasheet USART sections: register layouts, the divide-by-16 clock,
  start/stop bits, the interrupt vectors); add rows to `hardware-facts.md`.
- [ ] **Step 2: Failing tests:** `usart::a_byte_written_to_udr_is_transmitted_after_one_character_time`,
  `usart::loopback_delivers_the_byte_and_raises_the_receive_interrupt`, `usart::overrun_sets_the_flag`,
  `serial::file_sink_writes_only_to_the_given_path`, `serial::an_existing_output_file_is_refused_without_overwrite`.
- [ ] **Step 3-5: Run, implement, run. Commit:** "MFP USART and the RS-232 peer".

### Task 9: MIDI ACIA and the Centronics printer port

**Files:** Modify `crates/machines/atari-st/src/{acia,psg,io}.rs`, plugin options; Test: `tests/midi.rs`, `tests/printer.rs`

**Interfaces:** options `midi_out`, `midi_in`, `midi_loopback`, `printer_out` (paths; the same overwrite rule as
Task 8). The MIDI ACIA's TX/RX move through the Task 8 `SerialPeer` at 31250 baud (÷16 of the 500 kHz clock, [D]
spec §10). Printer: PSG port B is the data lines, port A bit 5 the strobe; a strobe falling edge writes the byte to
`printer_out`, BUSY is read from GPIP bit 0 (`[H]`, source in step 1).

**Steps:**
- [ ] **Step 1: Source the facts** for the printer handshake (strobe, busy GPIP bit). **Step 2: Failing tests:**
  `midi::a_byte_to_the_midi_acia_reaches_the_sink_at_31250_baud`, `midi::loopback_returns_the_byte`,
  `printer::a_strobe_writes_the_data_byte`, `printer::no_printer_out_file_means_busy_stays_set`.
- [ ] **Step 3-5: Run, implement, run. Commit.**

### Task 10: IKBD mouse modes

**Files:** Modify `crates/chips/ikbd/src/lib.rs`; Test: `crates/chips/ikbd/tests/mouse.rs`

**Interfaces:** commands (opcodes from Atari's IKBD document, sourced in step 1): set relative mouse, set absolute
mouse (with maximum X/Y), set mouse keycode mode (with deltas), set mouse button action, set mouse threshold, set
mouse scale, interrogate mouse position, load mouse position, set Y origin at bottom/top, mouse enable/disable.
`Ikbd::mouse_move(dx, dy)` and the button inputs are unchanged; the mode decides the report format (relative packet,
absolute packet, cursor-key scancodes).

**Steps:**
- [ ] **Step 1: Source** the command list and packet formats again from the IKBD document used in slice 3 (cite the
  page); no command from memory. **Step 2: Failing tests**, one per command: for each, the exact bytes the IKBD
  returns for a scripted motion, e.g. `absolute_mouse_clamps_at_the_maximum`, `keycode_mode_sends_cursor_scancodes_per_delta`,
  `threshold_suppresses_small_moves`, `bad_parameters_are_ignored_like_the_document_says` (or an `[H?]` row where the
  document is silent).
- [ ] **Step 3-5: Run, implement, run. Commit:** "ikbd: mouse command set".

### Task 11: IKBD joystick, clock and control commands

**Files:** Modify `crates/chips/ikbd/src/lib.rs`; Test: `crates/chips/ikbd/tests/commands.rs`

**Interfaces:** joystick event reporting, interrogation, monitoring (with rate), fire-button monitoring, keycode mode;
time-of-day set/get (BCD, the clock driven from the scheduler, never the wall clock); status inquiries; pause/resume
output; memory load/examine and controller execute (the controller's own firmware is not emulated: these return what
the document defines for an empty controller memory, or an `[H?]` stop, never an invented result); reset command
`$80 $01`.

**Steps:**
- [ ] **Step 1: Source. Step 2: Failing tests**, one per command, plus `clock_advances_with_scheduler_time_only`
  and `unknown_command_bytes_are_ignored_not_fatal`. **Step 3-5: Run, implement, run. Commit.**
- [ ] **Step 6:** deferred slices 3-5 items closed here: IKBD reset cancelling a byte in flight and its queue size
  (decide from the document or record `[H?]`).

### Task 12: slice 9 scenario S9S

**Files:** `scenarios/serial.st` (boot program, 68000 words), manifest

A boot program writes "SERIAL" through the MFP USART at timer D rate, and "MIDI" through the MIDI ACIA; the scenario
runs with `serial_loopback` and a `midi_out` file; the check reads the file with the `file` scenario compare (add the
comparer to `rc-scenarios` if none exists: `golden` as exact bytes; never committed). Three identical runs.
**Commit** and merge slice 9.

---

## Slice 10: GLUE, cartridge, raster palette

### Task 13: cartridge

**Files:** Modify `crates/machines/atari-st/src/{bus,mmu,config}.rs`, plugin media/options; host routing for
`.stc`/`.img` as `MediaKind::Cartridge` for this machine; Test: `tests/cartridge.rs`

**Interfaces:** `MediaKind::Cartridge` with a 128 KiB window at `$FA0000-$FBFFFF` (spec §9, sourced in step 1 with the
`$FA52235F` application-ROM magic TOS looks for); read-only; absent cartridge reads per the sourced bus behaviour.

**Steps:**
- [ ] **Step 1: Source** the cartridge window address, size, magic, and what an absent cartridge returns.
- [ ] **Step 2: Failing tests:** `cartridge_is_visible_at_fa0000`, `a_write_to_the_cartridge_window_is_refused_as_documented`,
  `a_cartridge_over_128k_is_an_error`, `an_empty_slot_reads_as_sourced`. **Step 3-5: Run, implement, run. Commit.**

### Task 14: unmapped regions and GLUE bus errors

**Files:** Modify `crates/machines/atari-st/src/{bus,io,mmu}.rs`, `docs/hardware-facts.md`; Test: `tests/bus_errors.rs`

**Interfaces:** each region of the memory map gets one of: mapped, bus error (the 68000 bus-error exception with the
documented stack frame, which `mc68000` already produces), or "no fact", which keeps today's stop. Only regions with a
source move from "stop" to "bus error".

**Steps:**
- [ ] **Step 1: Source** (hardware specification, the Cloudflare document: ask the owner to download it if it is
  still blocked). **Step 2: Failing tests:** one per sourced region, `access_to_<region>_raises_a_bus_error`, plus
  `an_unsourced_region_still_stops_with_the_report`. **Step 3-5: Run, implement, run. Commit.**

### Task 15: line-based raster palette

**Files:** Modify `crates/chips/st-shifter/src/lib.rs` (palette writes timestamped by line), `crates/machines/atari-st/src/machine.rs`
(render per line); Test: `crates/chips/st-shifter/tests/raster.rs`, `crates/machines/atari-st/tests/raster.rs`

**Interfaces:** `Shifter::write_palette(index, value, line: u16)`; the frame render applies each palette change from its
line down. Granularity is one line (cycle-exact is Phase 3, spec §5).

**Steps:**
- [ ] **Step 1: Failing tests:** `a_palette_change_on_line_100_colours_only_lines_100_and_below`,
  `two_changes_on_one_line_the_last_one_wins`, `a_frame_with_no_changes_renders_exactly_as_before` (the S1 hash).
- [ ] **Step 2-4: Run, implement, run.** Check S1/S2/S3 manifests still match their goldens.
- [ ] **Step 5: Commit:** "st-shifter: raster palette by line".

### Task 16: sync and resolution border foundations

**Files:** Modify `crates/chips/st-shifter/src/lib.rs`, `diag.rs`; Test: `crates/chips/st-shifter/tests/sync.rs`

**Interfaces:** the Shifter records each sync-mode and resolution register write with its line and cycle in a
bounded per-frame log (cap 256 entries, oldest dropped); exposed in the diagnostics report and as a `ChipSnapshot`
field. The picture is unchanged (no border opening: Phase 3). A software "overscan" write therefore shows in the log
only. Say so in the doc comment so nobody mistakes the log for emulated overscan.

**Steps:**
- [ ] **Step 1: Failing tests:** `sync_writes_are_logged_with_line_and_cycle`, `the_log_is_bounded`,
  `the_picture_ignores_the_log`. **Step 2-4. Step 5: Commit.**

### Task 17: scenario S10 (raster) against Hatari

**Files:** `scenarios/raster.st`, manifest, `retroclawd-tools/scripts/capture_hatari_screen.py` only if it needs an
option

A boot program writes a different palette entry 0 on every 20th line from an HBL handler (so the picture is coloured
bands). Hatari draws the same bands: `similarity` against its screenshot, crop away the mouse pointer, normalise
colours as S1 does. If Hatari's band edges differ by a line, say so in the manifest reason with the measured number
and set `max_mad` from measurement, not a guess. Three identical runs; merge slice 10.

---

## Slice 11: 68000 toolchain (retroclawd-tools, can run in parallel after Task 4)

### Task 18: `.PRG` format and `rcld` output

**Files:**
- Create: `crates/rc-common/src/prg.rs` (header, relocation, symbols) in retroclawd-tools; `crates/rcld/src/` Atari
  template; Test: `crates/rc-common/tests/prg.rs`, `crates/rcld/tests/prg.rs`

**Interfaces:**
- Produces: `prg::parse(bytes) -> Result<Prg, PrgError>` and `Prg::to_bytes()`; fields per spec §15 (28-byte header,
  fixup table with `1` = +254, DRI symbols of 14 bytes); caps on every size and on the fixup walk.
- `rcld --target atari-prg` emits a relocatable `.PRG` from one assembled program; the assembler marks which longs are
  section-relative.

**Steps:**
- [ ] **Step 1: Source** the header and fixup format (GEMDOS reference or the Atari Compendium; record SHA-256 in
  `CREDITS.md`). **Step 2: Failing tests:** `prg::round_trip`, `prg::fixup_table_with_1_bytes_adds_254`,
  `prg::a_fixup_beyond_the_text_is_an_error`, `prg::symbol_table_beyond_the_file_is_an_error`,
  `prg::parse_of_random_bytes_never_panics`. **Step 3-5: Run, implement, run. Commit.**

### Task 19: `rcasm --cpu 68000` and `rcdis`

**Files:** Modify `crates/rcasm/src/{cpu,asm,expr}.rs` (new `Cpu::M68000`), `crates/rcdis/src/lib.rs`;
Test: `crates/rcasm/tests/m68000.rs`, `crates/rcdis/tests/m68000.rs`

**Interfaces:** Motorola syntax per spec §15 (sizes, all twelve addressing modes, `bra.s` ambiguity error unless a
size is given or `.opt autobranch`); `u32` location counter and origin with `.org` limited to 24 bits; encoder driven
by `mc68000::OPCODES` (the way the Z80 back end enumerates its disassembler) so the debugger, rcdis and rcasm cannot
drift. `rcdis --cpu 68000` prints canonical rcasm syntax, labels for branch targets, `dc.w` for an illegal or
truncated word.

**Steps:**
- [ ] **Step 1: Failing tests:** `every_opcode_word_assembles_back_to_itself` (each word the disassembler accepts,
  over all 65536 first words with fixed extension words; the test names any mismatch), `ambiguous_branch_is_an_error`,
  `org_beyond_24_bits_is_an_error`, `byte_sized_address_register_operand_is_an_error`,
  `rcdis_of_the_emutos_image_assembles_to_identical_bytes` (the image from `--firmware auto`'s cache; skipped when
  absent), `all_twelve_addressing_modes_encode`.
- [ ] **Step 2-4: Run, implement, run.** **Step 5: Commit** (tools).

### Task 20: run a `.PRG`: host routing and the generated floppy

**Files:** host `apps/retroclawd/src/main.rs` (`resolve_program_load`); atari
`crates/formats/st-floppy/src/boot.rs`, plugin `Program` media; Test: host unit test, `crates/formats/st-floppy/tests/boot.rs`,
`crates/machines/atari-st/tests/prg.rs`

**Interfaces:**
- Produces: `st_floppy::boot::with_auto_program(name: &str, prg: &[u8]) -> Result<Image, BootError>`: a 720K image with
  an `AUTO` folder holding the program; the plugin declares `MediaKind::Program` and builds this in memory.
- The host passes `.prg` as `MediaKind::Program` when the profile declares it; the C64 header path stays for profiles
  that do not.

**Steps:**
- [ ] **Step 1: Failing tests:** host: `prg_goes_to_the_plugin_when_the_profile_declares_program`,
  `prg_keeps_the_c64_path_when_it_does_not`; floppy: `with_auto_program_lists_the_file_under_auto`,
  `a_prg_larger_than_the_disk_is_an_error`; machine: `a_prg_prints_its_string_on_the_desktop` (EmuTOS; skip without
  firmware).
- [ ] **Step 2-4: Run, implement, run. Commit** in each repository.

### Task 21: slice 11 scenario S8P

Assemble `hello.s` with `rcasm`/`rcld` in `scenarios/build.sh`, run it with `--program hello.prg`, compare the screen
with an exact golden after three identical runs. Merge slice 11.

---

## Slice 12: ACSI and hard disks

### Task 22: ACSI target through the DMA chip

**Files:** Create `crates/machines/atari-st/src/acsi.rs`; Modify `dma.rs`, `io.rs`; Test: `tests/acsi.rs`

**Interfaces:** `trait BlockDevice { fn len(&self) -> u64; fn read(&mut self, lba: u64, out: &mut [u8]) -> Result<(), BlockError>; fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError>; }`;
an ACSI target answers TEST UNIT READY, REQUEST SENSE, READ(6), WRITE(6), MODE SENSE, INQUIRY, FORMAT; the command
bytes come through `$FF8604` with A1 low then high, data through the DMA. Target ID 0-7; today's "HDC access with
nothing connected" behaviour stays for absent IDs ([O], dma.rs).

**Steps:**
- [ ] **Step 1: Source** the ACSI command protocol (Atari's ACSI/DMA specification) and the DMA mode bits for the HDC;
  settle the deferred "DMA HDC/sector-count bit order" item from the slices 3-5 ledger. If the document cannot be
  fetched, stop and ask the owner for it.
- [ ] **Step 2: Failing tests:** `test_unit_ready_good_status`, `read6_transfers_the_blocks_to_ram`,
  `write6_changes_the_backing_image_only_in_memory`, `read_beyond_the_end_is_check_condition_with_sense`,
  `an_absent_target_id_behaves_as_before`, `a_transfer_across_the_top_of_ram_is_refused` (the deferred item).
- [ ] **Step 3-5: Run, implement, run. Commit.**

### Task 23: hard-disk media and options

**Files:** Modify the plugin (`MediaKind::Other("acsi")`, `MediaSlot::Other("acsi0")`, option `acsi0`..`acsi7` as `File`
options), `config.rs`; the image is a raw `.img`/`.hd` file of at most 512 MiB (cap checked arithmetic); write-back
through `ExportDisk`-style export, atomic; Test: `tests/hard_disk.rs`

**Steps:**
- [ ] **Step 1: Failing tests:** `an_image_larger_than_the_cap_is_refused`, `an_image_not_a_multiple_of_512_is_an_error`,
  `a_guest_write_is_exported_dirty`, `two_targets_are_independent`. **Step 2-5. Commit.**

### Task 24: `rcdisk` partitions and FAT16; the `.stx` provenance note

**Files:** retroclawd-tools `crates/rcdisk/src/{ahdi,fat16}.rs`, tests, `docs/rcdisk.md`; a note
`machines/atari/docs/stx-and-dim-provenance.md`

**Interfaces:** `rcdisk hd new OUT.img --size SIZE --partition SIZE` (AHDI root sector with one `GEM`/`BGM`
partition), `rcdisk hd add/list/extract` on partition 0 for FAT16 with logical sector sizes above 512 on `BGM`;
loop and depth caps on `XGM` chains; extract refuses `..` and absolute names. The note records what is known about
the `.stx` and `.dim` formats, their sources' licences, and recommends go or no-go to the owner (Decision 2 and 3);
no `.stx`/`.dim` code is written.

**Steps:**
- [ ] **Step 1: Source** the AHDI root sector layout and the Atari BPB conventions (`[H?]` until read; cite URL and
  SHA-256). **Step 2: Failing tests:** `a_new_disk_has_a_valid_ahdi_root`, `add_list_extract_round_trip_fat16`,
  `an_xgm_chain_loop_is_an_error`, `an_extract_name_with_dotdot_is_refused`, `a_partition_beyond_the_disk_is_an_error`,
  `a_cyclic_fat16_chain_is_an_error`. **Step 3-5: Run, implement, run. Commit.**

### Task 25: scenario S11 (EmuTOS sees drive C:), fuzz sweep, Phase 2 exit

**Files:** `scenarios/hd.img` build entry (`rcdisk hd`), manifest S11, docs (`CLAUDE.md` of the host and atari, the
spec's new Status section, `TODO.md`)

**Steps:**
- [ ] **Step 1:** S11: the desktop with a hard disk image holding `HELLO.TXT`; EmuTOS shows drive C; a double click on
  the C icon (the pointer positions from a measured run, as S3 did) lists the file. Exact golden, three identical runs,
  checked by eye once.
- [ ] **Step 2: Fuzz sweep.** Run every fuzz target added in this plan (`state_decode`, `msa`, `prg`, `ahdi`,
  `fat16`, `rcasm`, `rcdis`) for 60 s each; fix any crash with a regression test first.
- [ ] **Step 3: Phase 2 exit check** against the six criteria in Global Constraints; the final whole-repository
  review of the plan's executor covers the rest.
- [ ] **Step 4:** full `tools/check_all.sh` (with the build), then docs and `bump_pin.sh`. Pushing waits for the owner.
