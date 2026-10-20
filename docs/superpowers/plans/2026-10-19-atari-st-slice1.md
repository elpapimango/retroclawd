# Atari ST Slice 1 ("reset to first I/O") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the slice that proves the whole chain:
- a dynamically loaded `atari-st` plugin;
- an in-house MC68000 core validated by SingleStepTests;
- an explicit big-endian ST bus with the EmuTOS 1.4 ROM at `$FC0000`;
- a deterministic event scheduler;
- EmuTOS fetched and verified by hash.

A headless run executes EmuTOS deterministically until its first access to an unimplemented I/O register, and stops
there with a bounded diagnostic.

**Architecture:**
- **`mc68000` core:** a new crate in `retroclawd-crates` (`crates/core/68000`). It is generic over its own
  `M68kBus` trait, which carries function codes and has a `sync` hook. One declarative opcode table drives dispatch
  and disassembly.
- **`atari-st` machine:** lives in a new `retroclawd-atari` family repository (submodule `machines/atari`). It owns
  the bus, the scheduler and the diagnostics. The plugin and cdylib crates follow the existing families exactly.
- **Host changes:** the submodule, the family lists in `tools/`, and an Atari-agnostic URL firmware source in
  `firmware.rs`.

**Tech Stack:**
- Rust 2024 edition (rustc 1.98.1), `serde`, bincode v2 (family convention).
- Dev-dependencies: `serde_json` and `flate2` (SingleStepTests JSON .gz), and `m68k = "=0.14.6"` (MIT, the
  differential oracle).
- Host: `sha1` (already used), `zip` / `file_loader` (already used).

**Spec:** `docs/superpowers/specs/2026-10-19-atari-st-design.md`. Sections 3 (decisions), 7, 8, 9, 17, 18 and 27
govern this plan.

## Global Constraints

- No Atari code in the host; deleting `atari_st_machine_plugin.{dll,so,dylib}` removes the machine.
- No plugin ABI change: protocol 11, ABI 1.2, profile contract 2 stay as they are.
- `#![forbid(unsafe_code)]` in `mc68000` and every new Atari crate.
- Licence: `GPL-3.0-or-later` on every new crate. The new repo is public `elpapimango/retroclawd-atari`.
- Machine time comes only from the scheduler: no wall clock and no floats in authoritative timing; `u64` CPU cycles
  are the master unit.
- Every big-endian access is explicit (`u16::from_be_bytes` etc.); no pointer casts and no native-layout
  serialization.
- **Every [H?] hardware fact used in code** is checked against a source before it is coded. Its citation goes in a
  code comment: URL plus section. The datasheet or manual wins over emulator source.
- **Firmware:**

  | Item | Value |
  |---|---|
  | Archive URL | `https://downloads.sourceforge.net/project/emutos/emutos/1.4/emutos-192k-1.4.zip` |
  | Archive SHA-1 | `6f146a126065f0671a1df3d9a273c01f962e9d06` (SourceForge-published) |
  | PAL entry | `emutos-192k-1.4/etos192uk.img`, SHA-1 `904ea9dc09ff8286f6e0272c12255dfe85beb17a` |
  | NTSC entry | `emutos-192k-1.4/etos192us.img`, SHA-1 `c03ae6379c88b31303dd6ebe63087054f716cba0` |
  | Size | 196 608 bytes each (project-pinned) |

- **SingleStepTests:** `https://github.com/SingleStepTests/680x0`, commit
  `e0d5ece9670205cc84a0101081837deb446f86a3`, folder `68000/v1`. Fetched into a git-ignored folder and never
  committed; the tests skip with a printed reason when it is absent.
- **Defaults:** `--ram 1m`, `--monitor color`, `--model stfm`, `--accuracy fast`.
- **Commits:** each repository commits its own work; `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  Nothing is pushed until the final task.

## Review Focus

1. **A short, oversized or wrong ROM** (a 256K STE image, a truncated download) must give a clear error naming the
   expected size, never a panic or garbage execution → Task 13 test `a_rom_of_the_wrong_size_is_refused_with_the_sizes`.
2. **Addresses with bits 24-31 set** (an `A7` of `$FF00_0400`, a `JMP ($80FC0030)`) must wrap to 24 bits on the
   ST bus, not index out of bounds → Task 12 test `addresses_wrap_to_24_bits`.
3. **Firmware with no network but a valid cache** must boot; a tampered cache file must be refused; no network and no
   cache must give one error naming the URL and the hash → Task 15 tests.
4. **Word or long access at an odd address**, including the reset vectors of a malformed ROM, must raise an address
   error (group 0 frame), not read across → Task 9 test `odd_word_read_raises_address_error_with_group0_frame`.
5. **Running past the I/O stop:** a headless `--frames 50` run must not panic or loop forever after the stop. The
   machine stays `halted()` and steps cost 4 cycles, and the diagnostic is written exactly once → Task 14 test
   `the_io_stop_holds_the_machine_and_reports_once`.

---

## File structure

### retroclawd-crates

```
crates/core/68000/                 package `mc68000`
  Cargo.toml
  src/lib.rs                       pub use; crate docs (sources cited)
  src/bus.rs                       Size, FunctionCode, BusFault, IackResponse, M68kBus
  src/cpu.rs                       Cpu state, reset, step loop, prefetch, exceptions
  src/ea.rs                        effective-address calculation and operand read/write
  src/opcodes.rs                   OPCODES pattern table + dispatch table build (OnceLock)
  src/ops/{move_,arith,logic,shift,branch,system,bits,bcd,muldiv}.rs   instruction handlers by group
  src/disassemble.rs               disassemble(bytes, pc) -> (text, len)
  tests/singlestep.rs              SingleStepTests harness (one #[test] per group)
  tests/sst_baseline.txt           allowed failures by test name with a reason (starts empty)
  tests/exceptions.rs              own tests: STOP, TRACE, IRQ, RESET insn, illegal, privilege, address error
  tests/differential.rs            random streams vs `m68k` 0.14.6
scripts/fetch_singlesteptests.sh   downloads the pinned commit's 68000/v1 into crates/core/68000/tests/sst/ (git-ignored)
```

### retroclawd-atari (new)

```
Cargo.toml (workspace)  LICENSE  README.md  CLAUDE.md  CREDITS.md  .gitignore  rustfmt.toml  firmware.toml
crates/machines/atari-st/src/{lib.rs, config.rs, bus.rs, scheduler.rs, diag.rs, machine.rs}
crates/machines/atari-st/tests/{bus.rs, scheduler.rs, emutos.rs}
crates/plugins/atari-st-machine-plugin/src/lib.rs     lib `atari_st_machine_plugin_impl`, `entry_v1()`
crates/plugin-dlls/atari-st/{Cargo.toml, src/lib.rs}  lib `atari_st_machine_plugin`, cdylib
```

### retroclawd (host)

```
.gitmodules, machines/atari (submodule)
apps/retroclawd/src/firmware.rs          URL source, on-disk cache, region condition
apps/retroclawd/src/main.rs              pass region into firmware::auto_fill
tools/{build_all,check_all,smoke_test,bump_pin}.sh   family list gains `atari` (smoke test: not yet, see Task 16)
```

### retroclawd-tools

```
scripts/capture_hatari_trace.py          Hatari black-box CPU trace -> list of PCs
```

---

## Part A: retroclawd-crates, the `mc68000` core

### Task 1: Crate skeleton, bus trait, registers, reset

**Files:**
- Create: `crates/core/68000/{Cargo.toml, src/lib.rs, src/bus.rs, src/cpu.rs}`
- Modify: `Cargo.toml` (workspace `members` += `"crates/core/68000"`; `[profile.release.package.mc68000] opt-level = 3`)
- Modify: `.gitignore` (`crates/core/68000/tests/sst/`)
- Test: `crates/core/68000/src/cpu.rs` (`#[cfg(test)] mod tests`)

**Interfaces (Produces):**

```rust
pub enum Size { Byte, Word, Long }
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FunctionCode(pub u8); // FC2..FC0
impl FunctionCode { pub const USER_DATA: Self; USER_PROGRAM; SUPERVISOR_DATA; SUPERVISOR_PROGRAM; CPU_SPACE /* 1,2,5,6,7 */ }
pub struct BusFault;
pub enum IackResponse { Vector(u8), Autovector, Spurious }
pub trait M68kBus {
    fn read(&mut self, addr: u32, size: Size, fc: FunctionCode) -> Result<u32, BusFault>;   // addr is the 24-bit address the CPU drives
    fn write(&mut self, addr: u32, size: Size, value: u32, fc: FunctionCode) -> Result<(), BusFault>;
    fn iack(&mut self, level: u8) -> IackResponse;
    /// Internal (non-bus) CPU clocks since the previous call; each bus access itself is 4 clocks and is not included.
    fn sync(&mut self, _internal_clocks: u32) {}
    /// The RESET instruction's reset line.
    fn reset_devices(&mut self) {}
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Cpu { pub d: [u32; 8], pub a: [u32; 7], pub usp: u32, pub ssp: u32, pub pc: u32, /* private */ sr: u16, prefetch: [u16; 2], run: RunState, ipl: u8 }
pub enum RunState { Running, Stopped, Halted }
impl Cpu {
    pub fn new() -> Self;
    pub fn sr(&self) -> u16; pub fn set_sr(&mut self, sr: u16);   // set_sr swaps A7 between usp/ssp when S changes
    pub fn a7(&self) -> u32;                                       // active stack pointer
    pub fn prefetch(&self) -> [u16; 2]; pub fn set_prefetch(&mut self, q: [u16; 2]);
    pub fn run_state(&self) -> RunState;
    pub fn set_ipl(&mut self, level: u8);                          // 0..=7, sampled at instruction boundaries
    pub fn reset<B: M68kBus>(&mut self, bus: &mut B) -> u32;       // reset exception; returns clocks
    pub fn step<B: M68kBus>(&mut self, bus: &mut B) -> u32;        // one instruction or exception; returns clocks
}
```

- [ ] **Step 0: Source the reset facts.** From the M68000 Programmer's Reference Manual / User's Manual (Motorola
  M68000UM), read the reset exception:
  - the vector reads (SSP from long 0, PC from long 4, supervisor program space);
  - the SR after reset (S=1, T=0, mask 7);
  - the clock count.

  Put the citation (document, section) in `cpu.rs` above `reset`.
- [ ] **Step 1: Write the failing test** `reset_loads_ssp_and_pc_from_vectors_in_supervisor_mode`.
  - **Setup:** a `Vec<u8>` test bus with long `$0000_0400` at 0 and `$00FC_0030` at 4.
  - **Assert:**
    - `cpu.a7() == 0x400`;
    - `cpu.pc == 0xFC0030`;
    - `cpu.sr() & 0x2700 == 0x2700`;
    - `cpu.sr() & 0x8000 == 0`;
    - the bus saw two long (or four word) reads with `fc == SUPERVISOR_PROGRAM`.
- [ ] **Step 2:** `cargo test -p mc68000 reset_loads` → FAIL (unresolved items).
- [ ] **Step 3:** Implement the types and `Cpu::new`, `reset`, `sr`/`set_sr`, `a7`. `reset` also fills the
  prefetch queue from the new PC, so `step` can start.
- [ ] **Step 4:** `cargo test -p mc68000` → PASS. `cargo clippy -p mc68000 -- -D warnings` → clean.
- [ ] **Step 5:** Commit: `mc68000: crate skeleton, bus trait with function codes, reset exception`.

### Task 2: SingleStepTests fetch script and harness

**Files:**
- Create: `scripts/fetch_singlesteptests.sh`, `crates/core/68000/tests/singlestep.rs`,
  `crates/core/68000/tests/sst_baseline.txt`
- Modify: `crates/core/68000/Cargo.toml` (`[dev-dependencies] serde_json = "1"`, `flate2 = "1"`)

**Interfaces:**
- **Consumes:** Task 1 `Cpu`, `M68kBus`.
- **Produces:**
  - the `sst!(fn_name, "FILE")` macro (one `#[test]` per file);
  - environment variables:
    - `RC_SST_DIR` overrides the folder;
    - `RC_SST_BUS=1` also compares the bus transactions;
    - `RC_SST_PRINT=1` prints the failure names for the baseline.

- [ ] **Step 1: Write the script.** `fetch_singlesteptests.sh [--force]` downloads every `68000/v1/*.json.gz` of the
  pinned commit from `https://raw.githubusercontent.com/SingleStepTests/680x0/<commit>/68000/v1/<file>` into
  `crates/core/68000/tests/sst/`.
  - The file list comes from the GitHub contents API at that commit.
  - The script prints the count and exits 1 on any failed download.
  - Run it.
  - **Expected:** 125 files, including `README.md`.
- [ ] **Step 2: Write the harness.**
  - **`TestBus`:** a sparse `HashMap<u32, u8>` RAM; reads of absent bytes return 0. It records
    `(kind, fc, addr, size, value)` per access and accumulates `sync` clocks as `n` entries.
  - **Per test:**
    - load `initial` (d0-d7, a0-a6, usp, ssp, sr via `set_sr`, pc, prefetch, ram);
    - `cpu.step(&mut bus)`;
    - compare `final` registers, sr, pc, prefetch and each `final.ram` byte, plus `length == returned clocks`;
    - with `RC_SST_BUS=1`, also compare `transactions`. A TAS `"t"` entry equals a byte read then a byte write at
      the same address with `fc` equal.
  - **Failures:** any not listed in `sst_baseline.txt` (format: `<test name>\t<reason>`) fail the test, which prints
    the first 10 diffs.
  - **A missing folder** prints `SingleStepTests not fetched: run scripts/fetch_singlesteptests.sh` and the test
    returns (passes).
  - **Declare** `sst!(nop, "NOP")` only, for now.
- [ ] **Step 3:** `cargo test -p mc68000 --test singlestep nop` → FAIL (NOP is not implemented: step panics or
  diffs). Confirm that the failure output names the test and the field.
- [ ] **Step 4:** Implement the step loop up to dispatching `$4E71` NOP (prefetch advance: the formal PC moves by 2,
  the queue shifts and one word is fetched as `SUPERVISOR_PROGRAM`/`USER_PROGRAM` by S). Run until `nop` → PASS.
- [ ] **Step 5:** Commit: `mc68000: SingleStepTests harness (fetched at test time, never committed) and NOP`.

### Task 3: Opcode table, dispatch, effective addresses; MOVE family

**Files:**
- Create: `src/opcodes.rs`, `src/ea.rs`, `src/ops/move_.rs`
- Test: `tests/singlestep.rs` (add groups), `src/opcodes.rs` tests

**Interfaces (Produces):**

```rust
pub struct OpPattern { pub mask: u16, pub bits: u16, pub mnemonic: &'static str, pub size: SizeField, pub src: EaSet, pub dst: EaSet, pub handler: Handler }
pub static OPCODES: &[OpPattern];
pub(crate) fn dispatch() -> &'static [Handler; 65536]; // built once (OnceLock) from OPCODES; illegal words -> illegal handler
#[derive(Clone, Copy)] pub struct EaSet(pub u16); // bit per mode: Dn, An, (An), (An)+, -(An), (d16,An), (d8,An,Xn), abs.W, abs.L, (d16,PC), (d8,PC,Xn), #imm
```

- [ ] **Step 1: Write the failing tests.**
  - `opcode_patterns_never_overlap`: for every word 0..=0xFFFF, at most one pattern matches after EA legality.
  - Groups: `sst!` for `MOVE.b`, `MOVE.w`, `MOVE.l`, `MOVEA.w`, `MOVEA.l`, `MOVE.q`, `LEA`, `PEA`, `CLR.{b,w,l}`,
    `TST.{b,w,l}`, `EXG`, `SWAP`, `EXT.w`, `EXT.l`.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement `ea.rs`: all 12 modes, the extension-word fetch through the prefetch queue, byte `(An)+`
  / `-(An)` on A7 moving by 2, and the brief-extension-word index (`Xn.W`/`Xn.L`, sign-extended d8). Then implement
  the handlers. Odd-address word/long accesses are left to Task 9; until then they may produce baseline failures,
  which must be listed with the reason `address error: Task 9`.
- [ ] **Step 4:** Run the groups. Every failure is either fixed or listed in the baseline with `Task 9`; no other
  reason is allowed.
- [ ] **Step 5:** Commit: `mc68000: opcode table, effective addresses, MOVE/LEA/PEA/CLR/TST/EXG/SWAP/EXT`.

### Tasks 4-8: Remaining instruction groups

Each task follows the same five steps as Task 3: add the `sst!` lines, see them fail, implement the handlers in the
named file, reach a pass or a `Task 9` baseline entry only, and commit.

| Task | File | SingleStepTests groups |
|---|---|---|
| 4 | `ops/arith.rs` | `ADD.*`, `ADDA.*`, `ADDX.*`, `SUB.*`, `SUBA.*`, `SUBX.*`, `CMP.*`, `CMPA.*`, `NEG.*`, `NEGX.*` |
| 5 | `ops/logic.rs`, `ops/shift.rs` | `AND.*`, `OR.*`, `EOR.*`, `NOT.*`, `ASL/ASR/LSL/LSR/ROL/ROR/ROXL/ROXR.{b,w,l}` |
| 6 | `ops/branch.rs` | `Bcc`, `BSR`, `DBcc`, `Scc`, `JMP`, `JSR`, `RTS`, `RTR`, `RTE`, `LINK`, `UNLINK` |
| 7 | `ops/system.rs`, `ops/bits.rs` | `ANDItoCCR`, `ANDItoSR`, `ORItoCCR`, `ORItoSR`, `EORItoCCR`, `EORItoSR`, `MOVEtoCCR`, `MOVEtoSR`, `MOVEfromSR`, `MOVEtoUSP`, `MOVEfromUSP`, `MOVEM.{w,l}`, `MOVEP.{w,l}`, `BCHG`, `BCLR`, `BSET`, `BTST`, `TAS`, `NOP`, `RESET` |
| 8 | `ops/muldiv.rs`, `ops/bcd.rs` | `MULU`, `MULS`, `DIVU`, `DIVS`, `ABCD`, `SBCD`, `NBCD`, `CHK`, `TRAP`, `TRAPV` |

Pinned behaviour per task:
- **Task 7:** `RESET` calls `bus.reset_devices()`. Its clock count comes from the `RESET` group's `length`.
- **Task 8:** the BCD undefined V/N flags follow the test data; no own guess.

Each commit message names the groups.

### Task 9: Exceptions: address error, privilege, illegal, interrupts, STOP, trace

**Files:**
- Modify: `src/cpu.rs`
- Test: `tests/exceptions.rs`, `tests/singlestep.rs` (empty the `Task 9` baseline entries)

- [ ] **Step 0: Source the frame formats.** From M68000UM/PRM, read:
  - the group 0 frame (access info word: R/W, I/N, FC; access address; instruction register; SR; PC);
  - the group 1/2 frames;
  - the interrupt acknowledge cycle (autovector = vector 24 + level, spurious = 24);
  - the priority between trace, interrupt and illegal;
  - the STOP behaviour;
  - double bus fault → halt.

  Cite them in `cpu.rs`.
- [ ] **Step 1: Write the failing tests** in `tests/exceptions.rs`:
  - `odd_word_read_raises_address_error_with_group0_frame`: `MOVE.W (A0),D0` with `A0 = $1001`. Assert:
    - PC = long at vector 3 (`$0C`);
    - SSP decreased by 14;
    - the frame's access address = `$1001`;
    - the access word has R/W = read and FC = supervisor data.
  - `privileged_instruction_in_user_mode_takes_vector_8`.
  - `illegal_word_takes_vector_4`; `line_a_takes_vector_10`; `line_f_takes_vector_11`.
  - `interrupt_above_mask_is_taken_at_instruction_boundary_with_autovector` (level 4 → vector 28, new mask 4).
  - `vectored_interrupt_uses_the_iack_vector`.
  - `interrupt_at_or_below_mask_is_ignored_except_level_7`.
  - `stop_loads_sr_and_waits_until_an_interrupt`: while stopped, `step` returns 4 clocks and the PC does not move.
  - `trace_takes_vector_9_after_each_instruction_when_t_is_set`.
  - `double_bus_fault_halts`.
- [ ] **Step 2:** `cargo test -p mc68000 --test exceptions` → FAIL.
- [ ] **Step 3:** Implement. Odd word/long accesses are detected in the CPU before calling the bus.
- [ ] **Step 4:** Run the exceptions tests plus the full `singlestep` suite → PASS with an **empty**
  `sst_baseline.txt`. If a test datum contradicts the manual, keep the test datum. Add a baseline entry quoting both
  and stop to report it, because it needs owner review.
- [ ] **Step 5:** Commit: `mc68000: exceptions, interrupts, STOP, trace; SingleStepTests 68000/v1 at 100%`.

### Task 10: Disassembler and the differential oracle

**Files:**
- Create: `src/disassemble.rs`, `tests/differential.rs`
- Modify: `Cargo.toml` (`[dev-dependencies] m68k = "=0.14.6"`), `src/lib.rs`
- Modify: `CREDITS.md` (SingleStepTests: commit, licence none stated, fetched at test time; `m68k` 0.14.6 MIT,
  dev-only oracle; M68000UM)

**Interfaces (Produces):**
- `pub fn disassemble(bytes: &[u8], pc: u32) -> (String, usize)`: Motorola syntax, lower-case mnemonics with a size
  suffix, hex as `$`.
  - Truncated input → `("dc.w $xxxx", 2)`, or `("dc.b $xx", 1)` for a single byte.
  - Illegal → `dc.w`.

- [ ] **Step 1: Write the failing tests:**
  - `disassembles_reference_lines`, each line asserted exactly:
    - `4E71` → `nop`
    - `602E` at `$FC0000` → `bra.s $fc0030`
    - `41F9 00FC 0030` → `lea $fc0030,a0`
    - `4CDF 7FFF` → `movem.l (a7)+,d0-d7/a0-a6`
    - `0C40 1234` → `cmpi.w #$1234,d0`
  - `truncated_input_falls_back_to_data`.
  - `differential.rs::random_instruction_streams_match_m68k`:
    - 20 000 random legal words from a fixed-seed xorshift;
    - one step each on a fresh state for both cores;
    - compare D, A, SR, PC.
    - It is `#[ignore]` by default, because it is long; run it in the verification step.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement, driven by `OPCODES` mnemonics and `EaSet`.
- [ ] **Step 4:** `cargo test -p mc68000` → PASS. `cargo test -p mc68000 --test differential -- --ignored` → PASS,
  or each divergence investigated; an `m68k` bug is recorded in `CREDITS.md`, not copied.
- [ ] **Step 5:** `cargo bench` is not set up in this repository. Measure instead with
  `cargo test --release -p mc68000 --test singlestep -- --nocapture` (instructions/s printed by the harness) and
  record it in the commit message.
- [ ] **Step 6:** Commit: `mc68000: disassembler, differential test against m68k 0.14.6`. Push `retroclawd-crates`
  only at Task 17.

---

## Part B: retroclawd-atari

### Task 11: Repository and workspace

**Files:** the whole skeleton under `C:\Users\matti\Workspace\retroclawd-atari\`.

- [ ] **Step 1:** `gh repo create elpapimango/retroclawd-atari --public --description "Atari ST family for retroclawd (machine plugins)"`.
  Clone it next to the other repositories.
- [ ] **Step 2:** Copy the file shapes from `retroclawd-amstrad`, with the content adapted:
  - `Cargo.toml`: the workspace members of Tasks 12-14, the same release profiles, and
    `[profile.release.package.mc68000] opt-level = 3`;
  - `LICENSE` (GPL-3.0);
  - `rustfmt.toml`;
  - `.gitignore` (`/target`, `*.log`, `/roms/`, `/software/`);
  - `README.md`;
  - `CLAUDE.md`, stating: the purpose, the spec path in the host repository, the [H?] rule, the commands, and that
    EmuTOS is fetched by `--firmware auto`;
  - `CREDITS.md` (EmuTOS 1.4 GPL-2.0, fetched, never committed; the ST Internals transcription URL).

  Dependencies on `machine`, `plugin-api` and `mc68000` are by git revision of `retroclawd-crates`; before Task 17
  this is the local Part A commit, through a temporary `.cargo/config.toml` `[patch]`.
- [ ] **Step 3:** Write `firmware.toml`:

  ```toml
  [[rom]]
  machines = ["atari-st"]
  option = "rom"
  label = "EmuTOS 1.4 192K (English, PAL)"
  sha1 = ["904ea9dc09ff8286f6e0272c12255dfe85beb17a"]
  url = "https://downloads.sourceforge.net/project/emutos/emutos/1.4/emutos-192k-1.4.zip"
  archive_sha1 = "6f146a126065f0671a1df3d9a273c01f962e9d06"
  entry = "emutos-192k-1.4/etos192uk.img"
  when = { region = "pal" }

  [[rom]]   # same, NTSC
  machines = ["atari-st"]
  option = "rom"
  label = "EmuTOS 1.4 192K (English, NTSC)"
  sha1 = ["c03ae6379c88b31303dd6ebe63087054f716cba0"]
  url = "https://downloads.sourceforge.net/project/emutos/emutos/1.4/emutos-192k-1.4.zip"
  archive_sha1 = "6f146a126065f0671a1df3d9a273c01f962e9d06"
  entry = "emutos-192k-1.4/etos192us.img"
  when = { region = "ntsc" }
  ```

- [ ] **Step 4:** Commit: `Repository skeleton, firmware.toml for EmuTOS 1.4`.

### Task 12: ST bus and memory map

**Files:**
- Create: `crates/machines/atari-st/{Cargo.toml, src/lib.rs, src/config.rs, src/bus.rs, src/diag.rs}`
- Test: `crates/machines/atari-st/tests/bus.rs`

**Interfaces (Produces):**

```rust
pub enum RamSize { K256, K512, M1, M2, M2_5, M4 }       // bytes(): 262144 .. 4194304
pub enum Region { Pal, Ntsc }
pub enum Accuracy { Fast, Accurate }
pub struct Config { pub ram: RamSize, pub region: Region, pub accuracy: Accuracy, pub rom: Vec<u8>, pub trace_limit: u32 }
pub struct StBus { /* ram: Vec<u8>, rom: Vec<u8>, sched: Scheduler, diag: Diag, now: u64 */ }
impl StBus { pub fn new(cfg: &Config) -> Result<Self, String>; pub fn stop(&self) -> Option<&IoStop>; }
impl mc68000::M68kBus for StBus { ... }
pub struct IoStop { pub addr: u32, pub size: Size, pub write: bool, pub value: u32, pub fc: FunctionCode, pub at_cycle: u64 }
```

- [ ] **Step 0: Source the facts** for every row below. Use the Atari ST Hardware Specification (download it in a
  browser if Cloudflare blocks the script), the MMU/GLUE sections, and ST Internals as corroboration. Cite each in
  `bus.rs`.
  - Do reads of `$000000-$000007` always come from ROM, or only during reset?
  - Do writes there go to RAM or cause a bus error?
  - Supervisor-only areas (`< $800`, `>= $FF8000`): bus error in user mode?
  - ROM writes: ignored or bus error?
  - Unmapped RAM above the configured size: what does a read return?
  - The 24-bit wrap.

  Write down any point the sources leave open as `// unspecified: <choice>, see spec §9`, and make the choice
  deterministic.
- [ ] **Step 1: Write the failing tests:**
  - `rom_reads_big_endian_at_fc0000`: a synthetic 196 608-byte ROM starting `60 2E 01 04`; a word read at
    `$FC0000` = `$602E`; a long read at `$FC0000` = `$602E0104`.
  - `reset_vectors_come_from_rom_at_address_zero`: a long read at 0 = ROM long 0; at 4 = ROM long 4.
  - `ram_round_trips_byte_word_long_big_endian`: write long `$11223344` at `$1000`; byte reads give
    `11 22 33 44`.
  - `addresses_wrap_to_24_bits`: a read at `$FF00_1000` equals a read at `$00_1000`.
  - `user_mode_access_below_0x800_is_a_bus_error`: only if Step 0 confirms it; otherwise this test asserts the
    sourced behaviour.
  - `first_io_access_records_a_stop`: a byte read at `$FF8001` returns `Ok`, `stop()` is `Some` with addr
    `$FF8001`, and a second access does not replace the first stop.
- [ ] **Step 2:** `cargo test -p atari-st --test bus` → FAIL.
- [ ] **Step 3:** Implement. Decode with a `match` on `addr >> 16` into region handlers. RAM is a `Vec<u8>` of
  `ram.bytes()`, initialised to `$00` (spec §9: a deterministic fill, recorded as such in `diag`).
- [ ] **Step 4:** Run → PASS. Clippy clean.
- [ ] **Step 5:** Commit: `atari-st: big-endian ST bus, ROM at $FC0000, reset mirror, I/O stop`.

### Task 13: Scheduler, VBL, the `AtariSt` machine

**Files:**
- Create: `src/scheduler.rs`, `src/machine.rs`
- Test: `tests/scheduler.rs`, `src/machine.rs` tests

**Interfaces (Produces):**

```rust
pub enum Event { Vbl }
pub struct Scheduler { /* now: u64, heap: BinaryHeap<Reverse<(u64, u8, u64, Event)>>, seq: u64 */ }
impl Scheduler { pub fn new() -> Self; pub fn now(&self) -> u64; pub fn schedule(&mut self, at: u64, class: u8, ev: Event);
                 pub fn next_at(&self) -> Option<u64>; pub fn pop_due(&mut self, now: u64) -> Option<Event>; }
pub const CLASS_VIDEO: u8 = 0;   // order of simultaneous events: video, mfp, acia, fdc, dma, cpu-irq (spec §8)
pub struct AtariSt { /* cpu: mc68000::Cpu, bus: StBus, region, frame_cycles: u64 */ }
impl AtariSt { pub fn new(cfg: Config) -> Result<Self, String>; pub fn cycles_per_frame(&self) -> u64; pub fn io_stop(&self) -> Option<&IoStop>; }
impl machine::Machine for AtariSt { ... }
```

- [ ] **Step 0: Source the timing facts** and cite them in `scheduler.rs`:
  - the PAL and NTSC master crystal and CPU clock (the hardware spec);
  - cycles per line × lines per frame for the 50 Hz and 60 Hz colour modes;
  - the VBL interrupt level (ST Internals says level 4, `$070`; corroborate it).

  If PAL and NTSC crystal figures conflict between sources, show both in the comment and use the hardware spec.
  - `clock_hz()` is the rounded CPU clock.
  - `frame_rate()` is the exact rational `cpu_hz_num / (cpu_hz_den * cycles_per_frame)`, built from integer crystal
    values.
- [ ] **Step 1: Write the failing tests:**
  - `simultaneous_events_pop_in_class_then_insertion_order`.
  - `a_thousand_frames_take_exactly_a_thousand_frame_periods`:
    - a synthetic ROM whose reset PC loops on `BRA.S *` (`60FE`), with interrupts masked;
    - step until the CPU cycles ≥ 1000 × `cycles_per_frame`;
    - assert the VBL event count = 1000;
    - assert that the time of the 1000th VBL = 1000 × `cycles_per_frame` exactly.
  - `vbl_raises_ipl4_and_is_autovectored`:
    - a ROM with the vector `$70` → a handler that does `ADDQ.L #1,$1000` then `RTE`;
    - SR `$2300`;
    - after 3 frames, the long at `$1000` = 3.
  - `a_rom_of_the_wrong_size_is_refused_with_the_sizes`: a 262 144-byte ROM → `Err` containing `196608` and
    `262144`, and a hint that the 256K image is for the STE.
  - `step_is_deterministic`: two machines from the same config; 200 000 steps each; equal `cpu` registers and an
    equal RAM SHA-1.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement.
  - **`AtariSt::step`:**
    1. If `io_stop` is set: return 4 and keep `halted() == true`.
    2. Otherwise run one `cpu.step(&mut bus)`.
    3. Advance `now`.
    4. Pop due events: VBL sets the IPL to 4 until acknowledged. `iack(4)` returns `Autovector` and clears it.
    5. Return the clocks.
  - **The fast-mode `sync`** is a no-op. `Accuracy::Accurate` behaves the same in this slice; there are no
    accurate-only devices yet, so the option is accepted and recorded.
  - **`Machine` impl:**
    - `pc()` = `cpu.pc`; `registers()` = D0-D7, A0-A6, A7, USP, SSP, PC, SR;
    - `read_memory` / `load` / `set_pc` through the bus with `CPU_SPACE`-free debug access (no side effects, no
      stop recording);
    - `render_frame()` = a black 640×400 RGB frame;
    - `save_state()` = `Vec::new()`, `load_state()` = `Err("save states arrive in Phase 2")`;
    - `reset()` = `cpu.reset`, RAM kept.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit: `atari-st: deterministic event scheduler, VBL autovector, Machine impl`.

### Task 14: Plugin, cdylib, diagnostics, trace file

**Files:**
- Create: `crates/plugins/atari-st-machine-plugin/{Cargo.toml, src/lib.rs}`,
  `crates/plugin-dlls/atari-st/{Cargo.toml, src/lib.rs}`
- Modify: `crates/machines/atari-st/src/diag.rs`
- Test: `crates/plugins/atari-st-machine-plugin/src/lib.rs` tests

**Template:** `machines/sinclair/crates/plugins/zx81-machine-plugin/src/lib.rs`. Copy its structure: instance
registry, `guarded` wrappers, `*_inner` functions, raw `step` / `halted` / `pc` / `pc_wide`.

**Interfaces (Produces):**
- **Profile:**
  - `machine_id "atari-st"`, display name `"Atari ST / STF / STFM"`;
  - `cpu_family: CpuFamily::M68k`;
  - `cpus: [CpuInfo { id: "main", name: "MC68000", family: M68k, .. }]`;
  - capabilities `Video, Debug`; media kinds none in this slice; `key_names` empty.
- **Options:**

  | Key | Kind | Default |
  |---|---|---|
  | `rom` | File, required | — |
  | `ram` | Enum `256k,512k,1m,2m,2.5m,4m` | `1m` |
  | `monitor` | Enum `color,mono` | `color`; mono accepted and recorded, video arrives later |
  | `model` | Enum `st,stf,stfm` | `stfm` |
  | `accuracy` | Enum `fast,accurate` | `fast` |
  | `trace_file` | Text | — |
  | `trace_limit` | Int 1..=10 000 000 | 100 000 |
  | `diag_file` | Text | — |

  The region comes from the create request.
- **Debug:** registers (`RegisterField`, `width_bits` 32, PC 24); `GetDisassembly` through
  `mc68000::disassemble`; `ReadMemory` / `WriteMemory`; breakpoints on PC. These use the same command handlers as
  the ZX81 plugin, adapted.
- **Diagnostics (`diag.rs`):** `Diag { last_pcs: [u32; 256] ring, last_ops: [u16; 256], exceptions: ring of 64 }`.
  `Diag::report(&self, cpu, stop, cycles) -> String` writes these sections:
  - `stop:` address, size, direction, value, FC, emulated cycles and ms;
  - `cpu:` all registers, SR flags spelled out, privilege;
  - `instruction:` the disassembly at the stop PC;
  - `history:` the last 256 PCs, oldest first;
  - `exceptions:` the ring contents.

  It is written to `diag_file` (or stderr when that is unset) once, at the first stop. No memory or ROM bytes are
  written besides the one opcode word per history line.
- **Trace:** with `trace_file` set, each executed instruction appends `{pc:06X}\n` until `trace_limit` lines, then
  the file is closed.

- [ ] **Step 1: Write the failing tests:**
  - `the_profile_declares_the_options_and_one_m68k_cpu`.
  - `the_io_stop_holds_the_machine_and_reports_once`:
    - a synthetic ROM whose reset code does `MOVE.B $FF8001,D0`;
    - 50 frames of steps;
    - `halted()` is true;
    - the diagnostic output occurs once and contains `stop: read byte $FF8001`.
  - `trace_file_lists_pcs_up_to_the_limit`: a limit of 5 → exactly 5 lines, the first `FC0030`.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** `cargo test --workspace` → PASS.
  `cargo build --release --workspace` produces `target/release/atari_st_machine_plugin.dll`.
- [ ] **Step 5:** Commit: `atari-st plugin: profile, options, debug, bounded diagnostics, trace file`.

---

## Part C: retroclawd host

### Task 15: URL firmware source with on-disk cache and region condition

**Files:**
- Modify: `apps/retroclawd/src/firmware.rs` (`RawEntry`/`Entry` gain `source: Option<UrlSource>`; new `fetch.rs`
  seam), `apps/retroclawd/src/main.rs` (≈2364: pass the region), `machines/*/firmware.toml` header comment (format
  doc)
- Test: `apps/retroclawd/src/firmware.rs` tests

**Interfaces (Produces):**

```rust
pub(crate) struct UrlSource { pub url: String, pub archive_sha1: String, pub entry: String }
pub(crate) trait Fetcher { fn get(&self, url: &str) -> Result<Vec<u8>, String>; }   // real one: inputs/file_loader download
pub(crate) fn auto_fill(machine: &str, region: &str, declared: &[MachineOptionInfo], values: &HashMap<String, DebugValue>,
                        have: &HashSet<String>) -> Result<Vec<BinaryBlob>, String>;
fn cached(sha1: &str) -> Option<Vec<u8>>        // <cache_root>/firmware/<sha1>.bin, verified on read
```

**Rules:**
- **`url`, `archive_sha1` and `entry` go together:** all three or none. An entry with them still needs exactly one
  `sha1`.
- **The `when` key `region`** is matched against the `region` argument, so it sits beside the resolved options.
- **Resolution order per option:**
  1. the on-disk cache by entry SHA-1;
  2. the RomM server, when configured;
  3. the URL: download, check the archive SHA-1, extract `entry` with the existing zip reader, check the entry
     SHA-1, write the cache atomically (temp file + rename).
- **No RomM configuration** is no longer an error when a URL source applies.
- **Errors:**
  - a network failure with no cache → `firmware: EmuTOS 1.4 192K (English, PAL) unavailable: <url>: <error> (wanted
    SHA-1 904ea9dc…)`;
  - a cached file whose SHA-1 differs → deleted and refetched, with a warning line.

- [ ] **Step 1: Write the failing tests** (with a fake `Fetcher` that counts calls and serves a zip built in the
  test):
  - `a_url_entry_parses_and_needs_all_three_fields`.
  - `a_url_source_downloads_verifies_and_caches` (the second call makes no fetch).
  - `a_wrong_archive_hash_is_refused`.
  - `a_wrong_entry_hash_is_refused`.
  - `a_tampered_cache_file_is_refused_and_refetched`.
  - `offline_with_no_cache_names_the_url_and_hash`.
  - `region_selects_the_ntsc_image`.

  The cache root comes from an environment override `RETROCLAWD_CACHE_DIR` set to a temp directory, if `romm.rs`
  has none; add it there if missing.
- [ ] **Step 2:** `cargo test -p retroclawd firmware` → FAIL. **Step 3:** Implement. **Step 4:** PASS; all host
  tests pass; clippy clean.
- [ ] **Step 5:** Live check:

  ```
  target/release/retroclawd atari-st --firmware auto --headless --frames 1
  ```

  - Expect a download, then the run.
  - Disconnect the network or point to a bad URL, run again, and expect the cache hit.
  - Corrupt the cached file, and expect the refusal and refetch.

  Record the commands and outputs.
- [ ] **Step 6:** Commit: `firmware: URL source with archive/entry SHA-1 checks, on-disk cache, region condition`.

### Task 16: Submodule, scripts, EmuTOS run, Hatari comparison

**Files:**
- Modify: `.gitmodules`, `machines/atari`, `tools/build_all.sh`, `tools/check_all.sh`, `tools/bump_pin.sh`
  (`atari` added to the default family lists; `smoke_test.sh` unchanged until the desktop milestone D)
- Create: `machines/atari/crates/machines/atari-st/tests/emutos.rs`,
  `retroclawd-tools/scripts/capture_hatari_trace.py`

- [ ] **Step 1:** `git submodule add https://github.com/elpapimango/retroclawd-atari machines/atari`. Update the
  scripts. `tools/build_all.sh` → `atari_st_machine_plugin` is staged in `target/release/plugins`, with
  `atari.firmware.toml`. `target/release/retroclawd --list-plugins` lists `atari-st`. Delete the staged library →
  `retroclawd atari-st` fails with "unknown machine … discovered machines: …"; restore it.
- [ ] **Step 2: Write `emutos.rs`.**
  - **ROM:** found through `RC_EMUTOS` (a path), else the firmware cache by SHA-1. The test skips with a printed
    reason when neither exists.
  - **`emutos_runs_deterministically_to_its_first_io_access`:**
    - two machines;
    - stepped until `io_stop()` or 10 000 000 cycles;
    - assert both stopped at the same cycle;
    - assert an equal SHA-256 of the PC trace;
    - assert the first PC = `$FC0030`.
    - Print the stop and the instruction count, for the commit message.
- [ ] **Step 3: Install Hatari** (latest stable Windows build from the official site,
  https://www.hatari-emu.org/download.html) into `Workspace\Emulators\hatari-<version>`. Record the version.
  - **Read** the Hatari manual's trace and debugger options (`--trace`, `--parse` or a debugger breakpoint-script)
    to find a non-interactive way to log every executed PC from reset. Cite the manual section in the script header.
  - **Write `capture_hatari_trace.py ROM N OUT.txt`:** run Hatari with `--machine st --tos ROM --memsize 1024`
    (exact flags from the manual), headless, and write the first N PCs as `{pc:06X}` lines.
- [ ] **Step 4: Compare.** Add `emutos_trace_matches_hatari_up_to_our_stop`. It reads
  `RC_GOLDENS/atari-st/emutos14-uk-trace.txt` (made by the script with N = our instruction count; kept out of git
  like every golden) and compares line by line up to our stop.
  - A mismatch prints the first differing index with 8 lines of context from each side.
  - A missing golden → skip with the capture command printed.
  - If Hatari cannot produce a PC trace non-interactively, record it as a known gap in
    `machines/atari/docs/hatari-trace-gap.md` with what was tried, and keep the test skipped. That is a gap, not a
    failure of the slice.
- [ ] **Step 5:** Run `tools/check_all.sh`. All steps pass, including the `unit-atari` step.
- [ ] **Step 6:** Commit in each repository:
  - atari: `EmuTOS determinism and Hatari trace tests`;
  - tools: `capture_hatari_trace.py`;
  - host: `Add retroclawd-atari as machines/atari; scripts know the atari family`.

### Task 17: Pins, docs, push

- [ ] **Step 1:** Remove every temporary `.cargo/config.toml` patch, and restore any `Cargo.lock` the patch changed.
  Push `retroclawd-crates`. Run `tools/bump_pin.sh <crates rev>` (it now includes `atari`) so every family pins the
  same `retroclawd-crates` revision. Let it run `tools/check_all.sh` → all pass.
- [ ] **Step 2: Docs.**
  - Host `CLAUDE.md`: the repository table gains retroclawd-atari; a short "Atari ST (slice 1, 2026-10-xx)"
    paragraph covering what runs, where it stops, and the firmware source.
  - `docs/superpowers/specs/2026-10-19-atari-st-design.md`: a Status section listing what was done, which [H?] facts
    became [H] with their sources, and any gaps.
  - `shared/CLAUDE.md`: the `mc68000` bullet.
  - `retroclawd-atari/CLAUDE.md`: the current state.
  - `TODO.md`: the next slice (GLUE/MMU registers, then Shifter, per spec §23).
- [ ] **Step 3:** Commit and push all five repositories: crates, atari, the three families' pin bumps, tools and
  host.
- [ ] **Step 4: Report:**
  - the repositories and commits;
  - the SingleStepTests count (all groups, expected failures);
  - the differential result;
  - instructions/s;
  - the EmuTOS stop (PC, register, instruction count);
  - the Hatari comparison result or gap;
  - every [H?] fact verified, with its source;
  - deviations from this plan.
