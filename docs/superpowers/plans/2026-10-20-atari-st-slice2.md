# Atari ST Slice 2 ("MMU, Shifter, MFP: EmuTOS to the sound chip") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Emulate the memory controller (`$FF8001`), the Shifter's registers and a frame render, and the MC68901 MFP,
with HBL/VBL/MFP interrupts from the scheduler. EmuTOS 1.4 then runs past its RAM sizing, its hardware probes, the MFP
set-up and the video set-up, and stops at its first access to the YM2149 (`$FF8800`, PC `$FC6D88`). Every I/O access
on the way is identical (address, size, value, PC) to Hatari 2.6.1's.

**Architecture:**
- **Decode first.** A new I/O decode table (`atari-st/src/io.rs`) says, per address, which device answers, which
  addresses are open (read `$FF`, no bus error), which raise a bus error, and which have **no sourced fact yet**: the
  machine stops at those with a "no hardware fact" diagnostic, never guessing.
- **Devices:**
  - The MMU is a module of the machine crate (`atari-st/src/mmu.rs`, as the design allows).
  - The MFP is a new chip crate, `crates/chips/mc68901`.
  - The Shifter is a new chip crate, `crates/chips/st-shifter`.

  Both chip crates live in retroclawd-atari (design §6 placement rule).
- **Time.** The scheduler gains per-line events (HBL), frame geometry chosen by the sync-mode register, and MFP timer
  timeouts converted exactly between the CPU clock and the MFP's own clock (Bresenham, design §8). In fast mode a
  device is brought up to date on every access to it (`catch_up`), so nothing the CPU reads is stale.
- **Oracle.** Hatari's `--trace io_all` output is the acceptance oracle, used as a black box (§Evidence).

**Tech Stack:**
- Rust 2024 (rustc 1.98.1) and `serde`. No new external dependencies.
- retroclawd-tools: a Python standard-library script.

**Spec:** `docs/superpowers/specs/2026-10-19-atari-st-design.md`, sections 5, 6, 8, 9, 10, 11, 12, 16, 21 and 23
(backlog items 3, 4 and 5). Slice 1's plan is `docs/superpowers/plans/2026-10-19-atari-st-slice1.md`.

**Repositories:**

| Repository | Work |
|---|---|
| retroclawd-atari | Nearly all of it, on branch `atari-st-slice2` |
| retroclawd-tools | Task 2's capture option, on branch `hatari-io-trace` |
| retroclawd (host) | Documentation and the submodule pointer only (Task 9) |
| retroclawd-crates | No change expected: no shared crate changes, so no pin bump. If the core needs a fix, it follows slice 1's flow (crates branch, `tools/bump_pin.sh`) |

## Global Constraints

- Clean-room:
  - Hatari, ASE and EmuTOS are never a source of code, tables, constants or comments.
  - Hatari runs only as a black box; EmuTOS only as the firmware under test.
- **Fact labels.** Every hardware fact used in code carries one of these labels in its doc comment, together with its
  source (document plus page or section):

  | Label | Meaning |
  |---|---|
  | `[D]` | datasheet |
  | `[H]` | hardware documentation (ST Internals, the Atari hardware specification, TOS documentation) |
  | `[O]` | Hatari-observed (black box) |
  | `[H?]` | unresolved, with the documented constant chosen |

  Facts are collected in `machines/atari/docs/hardware-facts.md`, one row each.
- **`[O]` facts.** An `[O]` fact may set the behaviour of a single observed address or value. Generalising it to a
  range needs `[H]` or `[D]`.
- **Unsourced timing.** Where the timing behind a register is unsourced (for example the video address counter), the
  register access stops the machine with "no hardware fact" instead of returning an invented value.
- `#![forbid(unsafe_code)]` in every crate.
- Licence `GPL-3.0-or-later`.
- Chip crates have no host, OS or Bevy I/O. Their time comes only from the clocks the machine passes in.
- State types derive `serde`.
- No floats in timing. `u64` CPU cycles are the master unit. The MFP clock conversion is integer and exact.
- Never commit:
  - goldens (`RC_GOLDENS/atari-st/`);
  - downloaded datasheets or books;
  - firmware.
- I/O logs contain register values only, never ROM or RAM contents.
- Defaults are unchanged: `--ram 1m`, `--monitor color`, `--model stfm`, `--accuracy fast`.
- The plugin ABI is unchanged.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Evidence gathered while planning (2026-10-20)

**Hatari 2.6.1 runs.** Black-box options for every run: `--machine st --tos etos192uk.img --sound off --fast-forward on
--trace io_all,cpu_exception`. The order in which EmuTOS first touches each device:

| Step | I/O access or event | Device |
|---|---|---|
| 1 | read `$FF8001` | MMU |
| 2 | bus error at `$FF8006` (PC `$FC007C`), then CPU probes (vectors 4, 11) | — |
| 3 | writes `$FF8001` = `$0A`, then the final configuration | MMU |
| 4 | read `$FF860F` = `$FF` (no bus error) | DMA page |
| 5 | reads and writes `$FFFC21`, `$FFFC25`, `$FFFC27`, `$FFFC3B` = `$FF` (no bus error) | Mega ST RTC probe |
| 6 | bus error at `$FF8A3C` | Blitter probe |
| 7 | clears `$FFFA01`-`$FFFA2D` (every odd byte); `VR` = `$48` | MFP |
| 8 | `$FF8260` = 0, `$FF820A` = 2, palette (16 words), GPIP read = `$A1`, video base `$FF8201`/`$03` = `$0F`/`$80` | Shifter |
| 9 | Timer C = ÷64 / 192 (`$FA1D` = `$50`, `$FA23` = `$C0`); Timer D; USART reads `RSR` = `$00`, `TSR` = `$80` | MFP |
| 10 | first PSG write `$FF8800` = `$07` (PC `$FC6D88`), the end of this slice | — |
| next | `$FFFC00` (keyboard ACIA), then DMA/FDC | — |

The 1 MiB log has 105 I/O accesses before the PSG write, which is the 106th. Hatari reaches it within 30 VBLs; 3 VBLs are not enough.

**Final MMU configuration per RAM size:**

| `--memsize` | 256 KiB | 512 KiB | 1 MiB | 2 MiB | 2.5 MiB | 4 MiB |
|---|---|---|---|---|---|---|
| Final `$FF8001` | `$00` | `$04` | `$05` | `$08` | `$09` | `$0A` |

The first write is `$0A` in every case. With 4 MiB, EmuTOS also reads `$4FFFFF`, which is a bus error.

**Documents (downloaded to the scratchpad, never committed):**

| Document | Size | SHA-256 | Covers |
|---|---|---|---|
| MC68901 datasheet, Motorola, Jan 1984 (bitsavers) | 52 pp. | `5441a092…` | Register map §1, reset §2.2.3, interrupts §3, timers §5 |
| *Atari ST Internals*, Abacus 1985 (atarimania PDF) | — | `961b981d…` | `$FF8001` bit layout p.55; Shifter registers pp.56-58; GLUE clocks p.~12; ACIA clocks |
| Atari *ST Hardware Specification* (1986, cpcwiki) | — | — | Still HTTP 403 from here: if the owner downloads it by hand, Task 1 uses it first |

**Gaps found:**
- *ST Internals* does not give the MFP timer crystal or the MMU's address multiplexing.
- *ST Internals* says GPIP I7 is the monochrome detect, low = mono (p.~95).
- *ST Internals* says `$FF820A` bit 1 is 1 = 50 Hz.

## Review Focus

1. **Reprogramming a running MFP timer.**
   - Writing TxDR while the timer counts must not reload the counter until the next timeout.
   - A prescaler change takes effect as datasheet §5.2 says.
   - Stopping (control = 0) and reading the data register returns the held count.
2. **Pending versus masked versus in-service.**
   - IMR gates only the IRQ output, not pending.
   - Software end-of-interrupt (VR bit S) blocks equal and lower channels until ISR is cleared.
   - A disabled channel (IER = 0) clears its pending bit.
   - IACK with nothing pending answers spurious, never a stale vector.
3. **Every `--ram` size boots EmuTOS's sizing to Hatari's `$FF8001` value.** That includes 256K (two 128K banks) and
   2.5M (2M + 512K), and accesses between the configured top and `$3FFFFF`.
4. **Accesses the table does not know stop cleanly.** That covers:
   - a word access to the MFP;
   - an even byte in the MFP or ACIA area;
   - `$FF8205`-`$FF8209`;
   - any address with no fact behind it.

   Each must stop with the "no hardware fact" diagnostic. None may return a made-up value, and none may panic.
5. **Long runs keep exact time.** After 10 000 frames:
   - the MFP's tick count equals `floor(cycles × 4 × 2 457 600 / crystal)` exactly;
   - HBL counts equal lines × frames, with 50/60 Hz switches in between.

---

### Task 1: Hardware facts and the I/O decode table

**Files:**
- Create: `machines/atari/docs/hardware-facts.md`
- Create: `crates/machines/atari-st/src/io.rs`
- Modify: `crates/machines/atari-st/src/bus.rs` (route `Region::Io` through `io::decode`)
- Modify: `crates/machines/atari-st/src/diag.rs` (headlines)
- Modify: `crates/machines/atari-st/src/lib.rs`
- Modify: `CREDITS.md`
- Test: `crates/machines/atari-st/tests/bus.rs`

**Interfaces:**
- Produces:
  - `pub enum Device { Mmu, Shifter, Dma, Psg, Mfp, Acia }`
  - `pub enum Decode { Device(Device), Open, BusError, NoFact }`
  - `pub fn decode(addr: u32, size: Size) -> Decode`
  - `IoStop` gains `pub reason: StopReason`, with `pub enum StopReason { NotEmulated(Device), NoFact }`.
  - `Diag::io_headline` names the device ("PSG (YM2149) is not emulated yet") or says "no hardware fact for
    `$FFxxxx` (byte read)".

**Steps:**

- [ ] **Step 1: Write the fact sheet.**
  - One table row per fact this slice uses, each with its label, source, page or section, and the value: every row
    named in Tasks 3-7.
  - Read the sources in this order: the owner's copy of the Atari *ST Hardware Specification* if present, the MC68901
    datasheet, *ST Internals*, TOS documentation (`freemint.github.io/tos.hyp`) and the Atari Compendium.
  - An `[O]` row cites the Hatari trace line.
  - Open questions (MMU multiplexing, MFP crystal, mono line count, video counter timing, palette unused bits) get
    `[H?]` rows with the constant chosen, or "stops".
  - The datasheet and *ST Internals* go in `CREDITS.md` with their URL and SHA-256.
- [ ] **Step 2: Write the failing decode tests** in `tests/bus.rs`. One row each:

  | Access | Expected decode |
  |---|---|
  | `$FF8001` byte | `Device(Mmu)` |
  | `$FF8000` byte | `NoFact` |
  | `$FF8006` word | `BusError` (`[O]` step 2) |
  | `$FF8201`, `$FF8203`, `$FF820A`, `$FF8240`-`$FF825E` words, `$FF8260` | `Device(Shifter)` |
  | `$FF8205` | `NoFact` (video counter, see Task 5) |
  | `$FF8604` word | `Device(Dma)` |
  | `$FF860F` byte | `Open` (`[O]` step 4) |
  | `$FF8800` | `Device(Psg)` |
  | `$FF8A3C` | `BusError` (`[O]` step 6) |
  | `$FFFA01`-`$FFFA2F` odd bytes | `Device(Mfp)` |
  | `$FFFA00` byte, `$FFFA01` word | `NoFact` |
  | `$FFFC00`, `$FFFC02`, `$FFFC04`, `$FFFC06` | `Device(Acia)` |
  | `$FFFC21`, `$FFFC25`, `$FFFC27`, `$FFFC3B` | `Open` (`[O]` step 5) |
  | `$FF9000` | `NoFact` |

  Also through the bus:
  - an `Open` read returns `$FF` and a write is ignored, with no stop;
  - a `BusError` row returns `Err(BusFault)`;
  - `NoFact` and `Device(Psg)` each set `io_stop()` with the right `StopReason`;
  - user-mode I/O is still a bus error.
- [ ] **Step 3: Run it.** `cargo test -p atari-st --test bus`. Expected: FAIL (`io` module missing).
- [ ] **Step 4: Implement `io::decode` and route the bus through it.**
  - The table is a `match` on address ranges. Each arm's comment carries its fact label.
  - Until Tasks 3-7, every `Device(_)` arm stops with `NotEmulated`.
- [ ] **Step 5: Run the tests.** `cargo test -p atari-st`. Expected: PASS. `tests/emutos.rs` still stops at `$FF8001`
  (now `NotEmulated(Mmu)`).
- [ ] **Step 6: Commit.** "atari-st: I/O decode table with sourced facts; stops name the device or the missing fact".

### Task 2: The I/O access log and the Hatari I/O golden

**Files:**
- Create: `crates/machines/atari-st/src/iolog.rs`
- Modify: `bus.rs` (record each completed I/O access)
- Modify: `machine.rs` (`pub fn io_log(&self) -> &[IoAccess]`)
- Modify: `config.rs` (`io_log_limit: u32`)
- Modify: `crates/plugins/atari-st-machine-plugin/src/lib.rs` (option `io_trace_file`, Text, written on drop like
  `trace_file`)
- Modify: `retroclawd-tools/scripts/capture_hatari_trace.py` (`--kind pc|io`)
- Test: `tests/emutos.rs`; a unit test in `iolog.rs`

**Interfaces:**
- Produces:
  - `pub struct IoAccess { pub pc: u32, pub addr: u32, pub size: Size, pub write: bool, pub value: u32 }`
  - `impl IoAccess { pub fn hatari_line(&self) -> String }`, which gives e.g.
    `IO write.b $ffff8001 = $0a pc=fc0188` and `IO write.w $ffff8240 = $0777 pc=fc66c6`: lower-case hex, the 24-bit
    address printed as `format!("$ff{:06x}", addr)` (Hatari's sign-extended form), the value 2 or 4 digits by size,
    the PC 6 digits.
  - The log records `Device` and `Open` accesses, including the stopping access, which is last. It does not record
    bus-error or `NoFact` accesses. `pc` is the address of the instruction making the access (`Diag::last_pc`).
- Consumes: `decode` (Task 1).

**Steps:**

- [ ] **Step 1: Write the failing tests.**
  - `iolog::tests::formats_like_hatari` asserts the two example lines above.
  - `tests/emutos.rs::emutos_io_log_matches_hatari` reads `RC_GOLDENS/atari-st/emutos14-uk-io-1m.txt`, skipping with a
    message when it is absent. It asserts that `ours.io_log()` formatted equals the golden's first `ours.len()` lines,
    and on failure prints the first differing line with 4 lines of context.
- [ ] **Step 2: Run them.** Expected: FAIL to compile.
- [ ] **Step 3: Implement the log.** Make it bounded by `io_log_limit` (the plugin sets 100 000 when `io_trace_file`
  is given).
- [ ] **Step 4: Add the capture option in tools.**
  - `--kind io` keeps the lines starting `IO ` verbatim (Hatari already prints the form `hatari_line` produces).
  - `--memsize` is passed through.
  - Write the six goldens `emutos14-uk-io-{256k,512k,1m,2m,2.5m,4m}.txt` (Hatari `--memsize` 256, 0, 1, 2, 2560, 4;
    `--run-vbls 50`).
- [ ] **Step 5: Run the tests.** `cargo test -p atari-st`. Expected: PASS. The log has 1 line, equal to the golden's
  first.
- [ ] **Step 6: Commit both repositories.**
  - atari: "atari-st: I/O access log in Hatari's trace format, `io_trace_file` option".
  - tools: "capture_hatari_trace.py: --kind io".

### Task 3: MMU: memory configuration and bank mapping

**Files:**
- Create: `crates/machines/atari-st/src/mmu.rs`
- Modify: `bus.rs` (RAM accesses go through `Mmu::translate`; `Device(Mmu)` handled)
- Modify: `config.rs`
- Test: `crates/machines/atari-st/tests/mmu.rs`

**Interfaces:**
- Produces:
  - `pub enum BankSize { None, K128, K512, M2 }`
  - `pub fn installed_banks(ram: RamSize) -> [BankSize; 2]`, as follows:

    | `--ram` | Banks |
    |---|---|
    | 256k | `[K128, K128]` |
    | 512k | `[K512, None]` |
    | 1m | `[K512, K512]` |
    | 2m | `[M2, None]` |
    | 2.5m | `[M2, K512]` |
    | 4m | `[M2, M2]` |

    This is `[O]`: the final configuration values in §Evidence, decoded by *ST Internals* p.55 (bits 3-2 bank 0, bits
    1-0 bank 1; `00` = 128K, `01` = 512K, `10` = 2M, `11` reserved).
  - `pub struct Mmu { config: u8, installed: [BankSize; 2] }`, with:
    - `read_config() -> u8`;
    - `write_config(u8)` (keeps bits 3-0; what the other bits read back is a fact row);
    - `translate(addr: u32) -> Option<usize>`, an index into the RAM `Vec`, or `None` for no RAM there.
  - The RAM `Vec` holds bank 0's chips first, then bank 1's.
  - Power-on config: `$00` (`[O]` step 1, the first read; marked as such).

**Steps:**

- [ ] **Step 1: Source the address multiplexing.**
  - What has to be found: for each configured bank size and each installed chip size, which CPU offset reaches which
    cell.
  - Sources, in order: the hardware specification; the MMU's documented row/column multiplexing; forum descriptions
    of TOS's sizing pattern (configure 2M+2M, write a pattern at `$8`/`$208`/`$408`... and see where it reads back).
  - Record the rule, or the observations it is built from, in the fact sheet.
  - If nothing beyond forum hearsay exists, make the rule `[H?]` and let the six-size oracle in Step 4 decide. Write a
    ledger ruling.
- [ ] **Step 2: Write the failing tests.**
  - `config_register_keeps_the_low_four_bits`.
  - `installed_banks_per_ram_option` (the table above).
  - `bank_1_starts_after_the_configured_bank_0`.
  - `no_ram_above_the_configured_total`: reads `$FF`, writes are ignored, no bus error, as in slice 1, still `[H?]`.
  - An aliasing test per (configured, installed) pair from the Step 1 rule.
  - `tests/emutos.rs::emutos_sizes_ram_like_hatari`: for each of the six sizes, the `$FF8001` writes in our I/O log
    equal those in that size's golden. With no golden the test skips that size.
- [ ] **Step 3: Run them.** Expected: FAIL.
- [ ] **Step 4: Implement.**
  - The fast path keeps the slice 1 shape: `translate` is a few shifts and masks per access, with no allocation.
  - The full 1m golden-prefix test (Task 2's) must now get past step 3 of §Evidence. It reaches the next stop, the
    first `Device(Mfp)` access.
- [ ] **Step 5: Run the tests.** `cargo test -p atari-st`. Expected: PASS for all sizes whose golden exists.
- [ ] **Step 6: Commit.** "atari-st: MMU memory configuration and bank mapping; EmuTOS sizes RAM like Hatari for every
  --ram".

### Task 4: `mc68901` MFP chip crate

**Files:**
- Create: `crates/chips/mc68901/{Cargo.toml, src/lib.rs, src/timer.rs, tests/datasheet.rs}`
- Modify: the root `Cargo.toml` (add `crates/chips/mc68901` to `members`)

**Interfaces:**
- Produces (package `mc68901`, lib `mc68901`):
  - `pub struct Mfp` (serde). Methods:

    | Method | Notes |
    |---|---|
    | `new() -> Mfp` | the power-on state, equal to `reset` plus the registers reset leaves alone, at their datasheet or `[H?]` value |
    | `reset(&mut self)` | datasheet §2.2.3: all registers cleared except TADR-TDDR, UDR, TSR; VR = `$0F`; timers stopped; GPIP lines inputs |
    | `read(reg: u8) -> u8` | `reg` = 0..=23, datasheet register order GPIP, AER, DDR, IERA, IERB, IPRA, IPRB, ISRA, ISRB, IMRA, IMRB, VR, TACR, TBCR, TCDCR, TADR, TBDR, TCDR, TDDR, SCR, UCR, RSR, TSR, UDR |
    | `write(reg: u8, value: u8)` | |
    | `set_input(bit: u8, level: bool)` | a GPIP line; an edge matching AER sets that channel's pending bit if it is enabled |
    | `timer_input(t: TimerAb, level: bool)` | TAI/TBI, for event count and pulse width |
    | `tick(clocks: u64)` | MFP timer clocks |
    | `next_timeout(&self) -> Option<u64>` | timer clocks until the earliest running delay-mode timer times out |
    | `irq(&self) -> bool` | |
    | `iack(&mut self) -> Option<u8>` | the vector `VR & $F0 \| channel`; sets in-service when VR.S is set; clears pending; `None` when nothing is pending and unmasked |
    | `timer_output(t) -> bool` | TAO-TDO, for later slices |

  - `pub enum TimerAb { A, B }`.
  - Channels are 0-15 in datasheet Table 3-x order:

    | Channel | Source |
    |---|---|
    | 0-3 | GPIP0-3 |
    | 4 | Timer D |
    | 5 | Timer C |
    | 6-7 | GPIP4-5 |
    | 8 | Timer B |
    | 9 | Tx error |
    | 10 | Tx empty |
    | 11 | Rx error |
    | 12 | Rx full |
    | 13 | Timer A |
    | 14-15 | GPIP6-7 |

    Verify the order against the datasheet before coding.
  - The USART is register storage only (Phase 2 adds the behaviour).

**Steps:**

- [ ] **Step 1: Write the failing datasheet tests,** each citing its section in a comment:
  - `reset_state` (§2.2.3; VR reads `$0F`).
  - `gpip_reads_inputs_and_outputs_by_ddr` (§4).
  - `edge_interrupt_follows_aer`.
  - `disabling_a_channel_clears_pending`.
  - `mask_gates_irq_not_pending`.
  - `iack_returns_highest_priority_vector`.
  - `software_eoi_blocks_lower_channels_until_isr_cleared`.
  - `automatic_eoi_never_sets_isr`.
  - `delay_mode_times_out_after_prescale_times_data` (each prescaler: 4, 10, 16, 50, 64, 100, 200).
  - `data_register_zero_counts_256`.
  - `writing_tdr_while_running_loads_at_next_timeout` (§5.2).
  - `reading_tdr_returns_the_counter`.
  - `event_count_mode_counts_active_edges`.
  - `pulse_width_mode_counts_while_input_active`.
  - `timer_output_toggles_on_timeout`.
  - `next_timeout_is_exact`.
  - `tsr_survives_reset`.
- [ ] **Step 2: Run them.** `cargo test -p mc68901`. Expected: FAIL to compile.
- [ ] **Step 3: Implement.**
  - `timer.rs` holds one `Timer` struct: mode, prescaler, prescale counter, main counter, reload.
  - `tick` advances in O(timers) using division, not a per-clock loop. It must be exact across arbitrary tick splits:
    add the test `tick_split_is_equivalent` (`tick(a+b)` == `tick(a); tick(b)` for random a, b, seeded).
- [ ] **Step 4: Run the tests.** Expected: PASS. `cargo clippy -p mc68901 -- -D warnings` is clean.
- [ ] **Step 5: Commit.** "mc68901: MFP registers, interrupt controller and timers from the Motorola datasheet".

### Task 5: `st-shifter` chip crate: registers and frame render

**Files:**
- Create: `crates/chips/st-shifter/{Cargo.toml, src/lib.rs, tests/render.rs}`
- Modify: the root `Cargo.toml`

**Interfaces:**
- Produces (package `st-shifter`):
  - `pub enum Resolution { Low, Medium, High }`
  - `pub enum Monitor { Colour, Mono }`
  - `pub struct Shifter` (serde), with:
    - `new()`;
    - `read(addr: u32, size: Size) -> Option<u16>`, where `None` means "not decoded here";
    - `write(addr: u32, size: Size, value: u16)`.

    It handles `$FF8201`, `$FF8203`, `$FF820A`, `$FF8240`-`$FF825E` (word and byte access) and `$FF8260`.
  - Accessors:
    - `video_base() -> u32`, equal to `$8201 << 16 | $8203 << 8` (*ST Internals* p.56, 256-byte steps);
    - `sync_50hz() -> bool`, true when `$820A` bit 1 is set;
    - `resolution()`;
    - `palette() -> [u16; 16]`.
  - `pub fn render(&self, monitor: Monitor, ram: &dyn Fn(u32) -> u8, out: &mut Vec<u8>)` writes a 640×400 RGB canvas
    (design §12):
    - low resolution: 320×200 pixels, each 2×2;
    - medium resolution: 640×200, each line doubled;
    - high resolution: 640×400.
  - Bitplane layout: each 16-pixel group is N big-endian words (4/2/1), plane 0 first, MSB = leftmost pixel. This is
    `[H]`, *ST Internals*, video section; cite the page.
  - Palette: 3 bits per gun, `$0RGB` with masks `$700`/`$070`/`$007`. To 8 bits: `level * 255 / 7` (a display choice,
    noted as not a hardware fact).
  - High resolution on mono uses palette 0 bit 0 to invert (*ST Internals* p.58).
  - Colour monitor in high resolution, or mono monitor in low/medium resolution: a black canvas, `[H?]` row "no
    picture".
  - Palette read-back of the unused bits follows the Task 1 fact row.

**Steps:**

- [ ] **Step 1: Write the failing tests.**
  - `video_base_from_high_and_mid_bytes` (`$0F`/`$80` gives `$0F8000`, as EmuTOS writes).
  - `palette_keeps_nine_bits`.
  - `low_res_plane_order_and_pixel_doubling`: a hand-built 16-pixel group with known plane words gives known colour
    indices, checked at canvas pixels (0,0), (1,1), (30,0).
  - `medium_res_two_planes`.
  - `high_res_mono_and_inversion`.
  - `colour_monitor_high_res_is_black`.
  - `render_is_deterministic` (hash twice).
- [ ] **Step 2: Run them.** Expected: FAIL to compile.
- [ ] **Step 3: Implement.** Render word by word; no per-pixel allocation.
- [ ] **Step 4: Run the tests.** Expected: PASS; clippy is clean.
- [ ] **Step 5: Commit.** "st-shifter: Shifter registers and low/medium/high frame render".

### Task 6: Scheduler lines, interrupts and the MFP clock

**Files:**
- Modify: `crates/machines/atari-st/src/scheduler.rs`
- Modify: `machine.rs`
- Modify: `bus.rs` (an interrupt latch struct, `iack` by level)
- Create: `crates/machines/atari-st/src/mfp_clock.rs`
- Test: `tests/scheduler.rs` and a new `tests/interrupts.rs`

**Interfaces:**
- Consumes: `Mfp::{tick, next_timeout, irq, iack, reset}` (Task 4); `Shifter::sync_50hz` (Task 5).
- Produces:
  - `Event::{LineEnd, Vbl, MfpTimeout}`.
  - `pub struct FrameGeometry { clocks_per_line: u32, lines: u32 }`, from
    `geometry(sync_50hz: bool, monitor: Monitor) -> Result<FrameGeometry, String>`:

    | Mode | Clocks per line | Lines | Source |
    |---|---|---|---|
    | 50 Hz | 512 | 313 | existing citation |
    | 60 Hz | 508 | 263 | existing citation |
    | Mono | 224 | the sourced count | if the count is unsourced, return `Err` naming the missing fact, and the plugin refuses `--monitor mono` |

    The geometry is chosen at each frame start (the VBL).
  - `MfpClock { crystal_hz: u32, mfp_hz: u32 }`:
    - `fn mfp_ticks_at(cpu_cycle: u64) -> u64`, which is `cpu_cycle * 4 * mfp_hz / crystal_hz` in `u128`, floored;
    - `fn cpu_cycle_for(mfp_tick: u64) -> u64`, the least cycle whose tick count is at least `mfp_tick`.

    `mfp_hz` is the sourced MFP crystal. The candidate is 2 457 600 Hz, cross-checked by TOS programming Timer C ÷64 ×
    192 for its documented 200 Hz `_hz_200` (§Evidence step 9).
  - Interrupt levels (*ST Internals* vector table; design §10):

    | Level | Source | Acknowledge |
    |---|---|---|
    | 2 | HBL | autovector |
    | 4 | VBL | autovector |
    | 6 | MFP | vectored; `iack` answers `Vector(v)`, or `Spurious` when `Mfp::iack` is `None` |

    IPL = the highest asserted level. The HBL and VBL latches clear on their acknowledge.
  - `reset_devices` (the RESET instruction) calls `Mfp::reset`. Nothing else is reset unless a fact row says so.
  - GPIP inputs at power-on: I7 = monitor (colour = high); the other lines are per fact rows such that the value read
    is `$A1` with a colour monitor (`[O]` §Evidence step 8). Each bit gets its own row; any bit only Hatari explains is
    `[O]`.
  - TBI (display enable) stays unconnected until DE timing is sourced: a fact row and a TODO line.

**Steps:**

- [ ] **Step 1: Write the failing tests.**
  - `hbl_every_line_and_vbl_every_frame` (1000 frames, exact counts).
  - `switching_to_60hz_changes_the_next_frame` (write `$820A` = 0 mid-frame; the next frame is 508 × 263).
  - `mfp_clock_round_trip_is_exact` (random cycles, seeded).
  - `mfp_ticks_never_drift` (10 000 frames).
  - `ipl_is_highest_pending_level`.
  - `mfp_iack_returns_vector_and_spurious_when_empty`.
  - `reset_instruction_resets_the_mfp`.
  - `timer_c_200hz_bare_metal`: a hand-assembled program sets VR = `$48`, Timer C ÷64 data 192, enables and unmasks
    channel 5, installs a handler at vector `$45` that increments a RAM counter and clears ISR (software EOI), and
    loops with `STOP #$2300`. After 50 frames, `counter == floor(elapsed_mfp_ticks / (64 × 192))`. The program bytes
    are written by hand from the PRM encodings, each with a comment.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
  - An MFP access first calls `catch_up(now)`, which ticks the MFP to `mfp_ticks_at(now)`.
  - After any MFP write, and after each `MfpTimeout`, reschedule the single next `MfpTimeout` at
    `cpu_cycle_for(now_ticks + next_timeout)`. Keep one outstanding event, with a generation counter to drop stale
    ones.
  - STOP jumps `now` to the next event, as before.
- [ ] **Step 4: Run the tests.** `cargo test -p atari-st`. Expected: PASS.
- [ ] **Step 5: Commit.** "atari-st: line events, HBL/VBL/MFP interrupts, exact MFP clock domain".

### Task 7: Wire the devices in; EmuTOS runs to the PSG

**Files:**
- Modify: `bus.rs` (`Device(Mfp)` gives `Mfp::read`/`write` with `reg = (addr - $FFFA01) / 2`; `Device(Shifter)`
  gives `Shifter`)
- Modify: `machine.rs` (`render_frame` comes from `Shifter::render` at the last VBL; `monitor` comes from `Config`)
- Modify: `config.rs` (`monitor: Monitor`)
- Test: `tests/emutos.rs` and `tests/bus.rs`

**Interfaces:**
- Consumes: Tasks 1-6.
- Produces:
  - `AtariSt::render_frame()`: the frame rendered at the most recent VBL, a 640×400 RGB canvas. It is black until the
    first VBL.
  - `Config::monitor`.

**Steps:**

- [ ] **Step 1: Write the failing tests.**
  - `bus.rs::mfp_registers_reachable_at_odd_addresses`.
  - `bus.rs::shifter_palette_word_write_and_render`.
  - `emutos.rs::emutos_runs_to_the_psg`: the stop is `NotEmulated(Psg)` at `$FF8800`, a write of `$07` from PC
    `$FC6D88`, identical over 2 runs (I/O log, registers, frame hash).
  - `emutos.rs::emutos_io_log_matches_hatari` (Task 2) now covers all 106 lines, ending with the stop access.
  - The PC-trace comparison against Hatari is narrowed to the PCs before the first interrupt we take. Fast mode has no
    bus wait states, so the exact instruction at which an interrupt arrives may differ from Hatari's. Record this as a
    known difference in the test's doc comment.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement the wiring.** Any divergence from the golden is fixed in the device model per its fact sheet
  row, never by special-casing an EmuTOS PC (design §16). A divergence the facts cannot explain is a ledger ruling,
  with the golden line cited.
- [ ] **Step 4: Run the tests.** `cargo test --workspace` in retroclawd-atari. Expected: PASS. The 1m I/O log has
  exactly 106 lines and equals the golden.
- [ ] **Step 5: Commit.** "atari-st: MMU, Shifter and MFP on the bus; EmuTOS runs to its first PSG access, I/O
  identical to Hatari".

### Task 8: Plugin: monitor, chip snapshots, release run

**Files:**
- Modify: `crates/plugins/atari-st-machine-plugin/src/lib.rs`
- Modify: `crates/machines/atari-st/src/machine.rs` (`pub fn chip_snapshots(&self) -> Vec<(String, Vec<(String,
  u64)>)>`, or `plugin_api` types in the plugin)
- Test: the plugin's tests (in-file `#[cfg(test)]`, as slice 1)

**Interfaces:**
- Produces:
  - `--monitor color|mono` reaches `Config::monitor`. `mono` is refused with Task 6's message while the mono line
    count is unsourced.
  - `GetChipSnapshots` returns chips with these fields:

    | Chip | Fields |
    |---|---|
    | `mmu` | `config` |
    | `shifter` | `video_base`, `resolution`, `sync`, `palette0..15` |
    | `mfp` | the 24 registers by name, `irq` |
    | `scheduler` | `now`, `frame`, `line`, `next_event_at` |

**Steps:**

- [ ] **Step 1: Write the failing tests.**
  - `chip_snapshots_name_every_device`.
  - `mono_monitor_is_refused_with_the_missing_fact`, or `mono_monitor_selects_high_res_timing` if the count was
    sourced.
  - `io_trace_file_is_written_on_drop`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests.** Then build the release and run it live:

  ```
  cargo build --release --workspace
  retroclawd atari-st --firmware auto --headless --frames 200 --plugin-option io_trace_file=io.txt --dump-screen s.png
  ```

  Adjust the option syntax to the host's real form; check `--help`. Expected:
  - the PSG stop report on stderr;
  - `io.txt` with 106 lines;
  - `s.png` as 640×400.
- [ ] **Step 5: Commit.** "atari-st plugin: monitor option, chip snapshots".

### Task 9: Documentation, integration, push

**Files:**
- Modify (retroclawd-atari): `CLAUDE.md`, `README.md`, `CREDITS.md`, `docs/hardware-facts.md` (final)
- Modify (host):
  - the `machines/atari` pointer;
  - `CLAUDE.md` (the Atari ST section: slice 2 status);
  - `TODO.md` ("Atari ST: next slices": the PSG and the ACIA/IKBD next; open `[H?]` rows);
  - the spec's Status section (slice 2 done; the `[H?]` facts that became `[H]`/`[D]`/`[O]`).

**Steps:**

- [ ] **Step 1: Update the docs as listed.** Every remaining `[H?]` row appears in `TODO.md`.
- [ ] **Step 2: Run the full check.** `tools/check_all.sh` from the host, with the atari branch checked out in
  `machines/atari`. Expected: every step passes, including unit-atari.
- [ ] **Step 3: Commit.**
  - atari: "Docs: slice 2".
  - host: "Docs: Atari ST slice 2; machines/atari to <rev>". Commit the host only after the atari branch is merged to
    `main` and pushed.
- [ ] **Step 4: Merge and push.** Ask the owner before pushing; this is a shared-branch push. Merge
  `atari-st-slice2` and `hatari-io-trace` to their `main`, push atari, then tools, then the host.
