# Sega CD slice 2: the sub-CPU system

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans (inline). Checkbox steps.

**Goal:** `retroclawd segacd --rom-bios <file>` runs the Mega-CD BIOS on a second 68000 until it needs the CD drive: gate array registers, program RAM, word RAM, BIOS overlay, interrupts, mailboxes, timer and stopwatch work, an unknown register stops the machine with a report.

**Architecture:** a pure `mcd-gate-array` chip crate holds all register state and the word RAM ownership; the Genesis `Bus` gets a family-internal `Expansion` hook for `$000000-$3FFFFF` and `$A12000-$A120FF`; a new `segacd` machine crate owns the sub bus, the sub `mc68000` and the scheduling of both CPUs; the plugin gains profile `segacd`.

**Spec:** `docs/superpowers/specs/2026-10-29-sega-cd-design.md`. **Facts:** `machines/sega/docs/hardware-facts.md` section 12 (C1-C15, from the Mega-CD Hardware Manual `MCD-HW`). The unresolved ones (C8 word RAM 1M swap bit sense, C12 H-INT vector fetch, C15 LED/version bits) are settled by probes in Task 5 before they are coded; until then the code returns/does the documented 2M behaviour and stops the machine in 1M mode with a report.

## Global Constraints

Same as `2026-10-29-sega-cd.md`. Also: the sub-CPU clock is 12.5 MHz and not an integer fraction of the master (53 693 175 / 53 203 424 Hz): the sub-CPU is advanced from master time with an exact integer ratio accumulator (no floats); the stopwatch and timer count 384 sub clocks per tick (C10, C11).

## Review Focus

- A write of any size to `$A12002`/`$A12001` that is not the exact reset sequence must not reset the gate array (C5).
- Main-CPU access to program RAM while the sub-CPU runs (SBRQ = 0): defined (open bus read, ignored write), never a panic or a state change.
- Sub-CPU access to word RAM it does not own in 2M mode (C7: hang): the sub-CPU is held until forced reset; the main CPU and the frame keep running.
- Odd-address and byte access to the registers that are word-only (stopwatch, CDC host data): defined, no panic.
- Timer value 0 and 255; stopwatch wrap at 4095; counters across a save-state boundary.
- Unknown sub or main I/O address: a bounded stop report naming the CPU, PC and address (project rule).

### Task 1: `mcd-gate-array` crate

**Files:** Create `machines/sega/crates/chips/mcd-gate-array/{Cargo.toml,src/lib.rs,src/regs.rs,src/wordram.rs,src/timers.rs}`; Modify workspace members.

**Interfaces (produces):**
- `GateArray::new() -> GateArray`; `reset_forced(&mut self)`.
- Main side: `main_read_word(&mut self, off: u32) -> Option<u16>`, `main_write_byte(&mut self, off: u32, v: u8)`, `main_write_word(&mut self, off: u32, v: u16)` (`off` = address - `$A12000`, `None` = unmapped); detects the C5 sequence across calls.
- Sub side: `sub_read_word(&mut self, off: u32) -> Option<u16>`, `sub_write_word`, `sub_write_byte`.
- State queries for the buses: `sub_halted() -> bool`, `sub_bus_granted() -> bool`, `prg_bank() -> u8`, `prg_write_protect(addr: u32) -> bool`, `word_ram_owner() -> WordRamOwner { Main, Sub }` (2M) and `WordRam1M { bank0_to_main: bool }` (1M), `mode_1m() -> bool`.
- Time: `advance_sub_clocks(&mut self, clocks: u64)` ticks stopwatch/timer; `take_sub_irq() -> Option<u8>` highest pending level enabled by the mask; `ack_sub_irq(level)`.
- Event inputs for later slices: `raise_level(level: u8)` (3 timer is internal; 1, 4, 5, 6 from graphics/CDD/CDC/subcode).

- [ ] Tests, one per fact: `reset_sequence_must_be_exact` (C5: right sequence clears, wrong size/order/value does not), `main_a12000_bits` (C4: SRES/SBRQ read-back, IFL2 raised only with IEN2, cleared on ack), `power_on_values` (C6, C9, C12 table values), `flag_word_halves` (C9: main writes high byte only, sub low byte only), `command_and_status_words` (C9 directions), `stopwatch_counts_and_wraps` (C10: 384 clocks per count, 4095 -> 0, write 0 clears), `timer_counts_down_and_interrupts_level_3` (C11: n=0 none, n=1 after 2 ticks, reload not repeated unless written again [H?] mark ignore), `interrupt_mask_and_priority` (C3: highest level first, masked level held pending), `word_ram_2m_handover` (C7: DMNA/RET sequence and read-back), `prg_bank_and_write_protect` (C1, C2: bank bits, protect ranges in $200 steps).
- [ ] Run `cargo test -p mcd-gate-array` — Expected FAIL (no crate), then PASS.
- [ ] Implement per signatures; `#![forbid(unsafe_code)]`; no floats. Commit "mcd-gate-array: registers, word RAM ownership, timers".

### Task 2: Genesis bus expansion hook

**Files:** Modify `crates/machines/genesis/src/{bus.rs,lib.rs,config.rs}`.

**Interfaces (produces):** `pub trait Expansion { fn read_byte(&mut self, addr: u32, now: u64) -> Option<u8>; fn write_byte(&mut self, addr: u32, v: u8, now: u64) -> bool; fn read_word(...)/write_word(...) defaults from bytes; fn dma_read_word(&mut self, addr: u32) -> Option<u16>; fn advance(&mut self, until: u64); fn irq_main(&mut self) -> bool }` and `Bus::attach_expansion(Box<dyn Expansion>)`. When attached, `$000000-$3FFFFF` (cartridge area) and `$A12000-$A120FF` go to it first; `None`/`false` falls through to the existing behaviour (stop report for unknown). VDP DMA from the cartridge area uses `dma_read_word`.

- [ ] Test `an_expansion_sees_cartridge_area_and_gate_array_addresses` (a test expansion returning fixed bytes) and `without_an_expansion_nothing_changes` (existing suite passes untouched).
- [ ] Run `cargo test -p genesis` — Expected: the whole existing suite still PASSES, new tests PASS after implementation; also run the Genesis manifest (30 scenarios) to prove no regression.
- [ ] Commit.

### Task 3: `segacd` machine crate

**Files:** Create `crates/machines/segacd/{Cargo.toml,src/lib.rs,src/sub_bus.rs,src/expansion.rs,src/machine.rs}`.

**Interfaces:** `SegaCd::new(config: &SegaCdConfig) -> Result<SegaCd, String>` (BIOS 131072 bytes, region); implements `machine::Machine`; `SubBus` implements the `mc68000` bus trait: PRG RAM 512 KB, word RAM (2M mode only, 1M stops with report until C8 is settled), BIOS ROM mirror (the sub CPU boots from the program RAM: the main CPU copies code in; the BIOS runs on the main CPU first), backup RAM stub (reads `$FF`... only odd bytes, 8 KB, returns erased until slice 4), PCM window stub (stop), `$FF8000` registers via `GateArray`. Scheduler: `step()` runs the main 68000 one instruction, then advances the sub-CPU by the exact ratio `sub_clocks = master_delta * 12_500_000 / master_hz` kept as a remainder-carrying integer accumulator; the sub is not run while `sub_halted()`; sub IRQs delivered through `take_sub_irq`.

- [ ] Tests: `bios_loader_rejects_wrong_size`, `sub_cpu_runs_a_program_copied_by_main` (a hand-assembled `rcasm` program: main copies 16 bytes of sub code to PRG RAM bank 0 through `$020000`, releases reset; sub increments a status word; main reads it), `sub_clock_ratio_is_exact` (after 1 NTSC second of master clocks the sub has 12 500 000 +- 1 clocks), `unmapped_sub_address_stops_with_report`, `main_cannot_touch_prg_ram_while_sub_runs`, `sub_hangs_on_word_ram_it_does_not_own`, `sub_irq2_from_main_ifl2` (main raises level 2, the sub's handler runs).
- [ ] Run tests (FAIL then PASS), clippy, fmt. Commit.

### Task 4: plugin profile, options, host routing

**Files:** Modify `crates/plugins/sega-machine-plugin/src/lib.rs`, `firmware.toml`, `crates/plugin-dlls/genesis/` (id stays); host `apps/retroclawd/src/{inputs.rs,main.rs}` for `--disc` (generic: a media option kind `disc`).

**Interfaces:** profile `segacd` (cpus `main`, `sub`, `z80`; options `rom_bios` File 131072 bytes, `console_region`; `media_kinds` gains `Other("disc-path")`), firmware entries for the preferred BIOS per region (spec section 2 hashes) with `when = { console_region = ... }`; `OnCpu` accepts `sub`.

- [ ] Tests: plugin profile test (ids, option keys, cpus), firmware.toml parse test, `OnCpu` sub registers/disassembly, host `inputs` test that a zip with `a.cue`+`a.bin` unpacks to one folder and the cue path is returned (generic, no Sega logic).
- [ ] `retroclawd segacd --list-keys` / `--help` smoke. Commit family and host separately.

### Task 5: probes and the BIOS boot

**Files:** Create `scenarios/probes/segacd/*.s`, extend `scenarios/probe.py`/`mkrom.py` for a tiny disc, `segacd.manifest.toml` (skeleton).

- [ ] Probes (run on Genesis Plus GX and PicoDrive/clownmdemu, results in work RAM/PRG RAM): `pcd01_memmode_power_on` (C6), `pcd02_word_ram_2m_handover` (C7), `pcd03_word_ram_1m_swap` (C8: settles the DMNA/RET sense), `pcd04_hint_vector` (C12), `pcd05_comm_flags` (C9), `pcd06_stopwatch_timer` (C10, C11), `pcd07_wp_and_bank` (C1, C2). Each result becomes `[O]` in the facts doc or a `known-gap` row.
- [ ] Boot test: with a RomM BIOS (`romm:fw/<id>`, preferred hash per region) the machine runs for 600 frames, the sub CPU's PC stays in program RAM/BIOS after init, and the first stop (expected: an unmapped CDD/CDC register, slice 3) is reported with CPU, PC and address. Compare how far GPGX's BIOS gets (picture) with ours for the record.
- [ ] Update `CLAUDE.md` (host, Sega paragraph), `TODO.md`, memory file; commit family + pin in host.

**Exit criteria for the slice:** tests green; Genesis manifest unchanged (30 scenarios); probes `pcd01`-`pcd07` exact against two oracles or recorded; BIOS runs to its first CD register and stops with a bounded report.
