# Machine Abstraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract a CPU-agnostic `Machine` trait (+ movable `Clock`, reusable `MemoryBus`) into a new `machine` crate, implement it for thin ZX81 and ZX Spectrum skeleton crates with their own clock rate and memory map, and give `emu` CLI-based register/memory introspection that works the same way regardless of which CPU is inside.

**Architecture:** New `crates/machine` crate owns the `Machine` trait, `Clock` (paces `Machine::step()` calls to `Machine::clock_hz()`), and `MemoryBus` (region-mapped ROM/RAM, moved from `emu` unchanged plus a new read-only `peek`). Two new crates, `zx81` and `zx-spectrum`, each wrap a `z80::Cpu` + `machine::MemoryBus` and implement `Machine` with their own fixed clock rate and ROM/RAM layout. `emu` is rewired to hold only a `Box<dyn machine::Machine>`, selected via a `--machine` flag, and never touches `z80::Cpu` directly.

**Tech Stack:** Rust 2024 edition, Cargo workspace, no external dependencies (stdlib only).

**Spec:** `docs/superpowers/specs/2026-09-05-machine-abstraction-design.md`

## Global Constraints

- No external crate dependencies added anywhere in this plan — stdlib only, matching the rest of the workspace.
- No real ROM images are bundled in the repo (copyrighted) — machines take ROM bytes as a runtime/constructor argument, defaulting to a zero-filled placeholder of the correct size when none is supplied.
- No UI this round — all introspection is CLI text output.
- No ULA/video, keyboard matrix, or tape support this round — ZX81 and ZX Spectrum crates are clock+memory-map skeletons only.
- `Machine` trait register/memory access must not assume a specific CPU — the trait is implemented once per concrete machine, which owns the mapping from its CPU's actual registers to the trait's uniform shape.

---

### Task 1: `machine` crate — `MemoryBus` (moved + `peek`)

**Files:**
- Create: `crates/machine/Cargo.toml`
- Create: `crates/machine/src/lib.rs`
- Create: `crates/machine/src/memory.rs`
- Modify: `Cargo.toml:3` (workspace members)

**Interfaces:**
- Consumes: `z80::Bus` trait (`crates/z80/src/bus.rs`) — `fn read(&mut self, addr: u16) -> u8`, `fn write(&mut self, addr: u16, val: u8)`, `fn read_io(&mut self, port: u16) -> u8`, `fn write_io(&mut self, port: u16, val: u8)`.
- Produces: `pub struct MemoryBus` with `new() -> Self`, `map_ram(&mut self, base: u16, len: usize)`, `map_rom(&mut self, base: u16, data: Vec<u8>)`, `load(&mut self, base: u16, bytes: &[u8])`, `peek(&self, addr: u16) -> u8`, and `impl z80::Bus for MemoryBus`. Re-exported from `machine` crate root as `machine::MemoryBus`.

- [ ] **Step 1: Create the crate scaffold and move `MemoryBus` unchanged**

`crates/machine/Cargo.toml`:
```toml
[package]
name = "machine"
version = "0.1.0"
edition = "2024"

[dependencies]
z80 = { path = "../z80" }
```

`crates/machine/src/lib.rs`:
```rust
mod memory;

pub use memory::MemoryBus;
```

`crates/machine/src/memory.rs` — copy the full current contents of `crates/emu/src/memory.rs` (the `RegionKind` enum, `Region` struct, `MemoryBus` struct and its `new`/`map_ram`/`map_rom`/`load`/`region_for`, the `Default` impl, the `Bus` impl, and all five existing `#[cfg(test)] mod tests` tests) verbatim into this new file. Do not modify any of it in this step — this is a pure move.

Add `"crates/machine"` to the workspace members list in the root `Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/z80", "crates/emu", "crates/machine"]
```

- [ ] **Step 2: Verify the move**

Run: `cargo test -p machine`
Expected: PASS — all 5 tests (`ram_round_trips_reads_and_writes`, `rom_ignores_writes_but_keeps_initial_contents`, `unmapped_address_reads_fill_value_and_ignores_writes`, `load_writes_bytes_sequentially_into_ram`, `load_outside_any_region_is_silently_dropped`).

- [ ] **Step 3: Write a failing test for the new `peek` method**

Add to the `tests` module in `crates/machine/src/memory.rs`:
```rust
    #[test]
    fn peek_does_not_require_mutable_access() {
        let mut bus = MemoryBus::new();
        bus.map_ram(0x4000, 0x10);
        bus.write(0x4000, 0x77);

        let bus = bus; // shadow as immutable -- proves peek takes &self
        assert_eq!(bus.peek(0x4000), 0x77);
        assert_eq!(bus.peek(0x0000), 0xFF);
    }
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test -p machine peek_does_not_require_mutable_access`
Expected: FAIL with "no method named `peek` found"

- [ ] **Step 5: Implement `peek`**

Add to `impl MemoryBus` in `crates/machine/src/memory.rs`, after `region_for`:
```rust
    /// Read-only peek at one byte, for display/introspection -- unlike
    /// `Bus::read` this doesn't need `&mut self`.
    pub fn peek(&self, addr: u16) -> u8 {
        self.regions
            .iter()
            .find_map(|r| r.contains(addr).map(|off| r.data[off]))
            .unwrap_or(self.unmapped_fill)
    }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p machine`
Expected: PASS — 6 tests.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/machine
git commit -m "feat(machine): add machine crate with MemoryBus (moved) + peek"
```

---

### Task 2: `machine` crate — `Machine` trait + `Clock` (moved + reworked)

**Files:**
- Modify: `crates/machine/src/lib.rs`
- Create: `crates/machine/src/clock.rs`

**Interfaces:**
- Consumes: nothing outside this crate (the trait is the new boundary).
- Produces: `pub trait Machine` with `clock_hz(&self) -> u64`, `step(&mut self) -> u32`, `halted(&self) -> bool`, `registers(&self) -> Vec<(&'static str, u64)>`, `read_memory(&self, addr: u16) -> u8`, `load(&mut self, addr: u16, bytes: &[u8])`, `set_pc(&mut self, addr: u16)`. `pub struct Clock` with `new() -> Self` and `run(&self, machine: &mut dyn Machine, running: impl FnMut(&dyn Machine) -> bool) -> RunStats`. `pub struct RunStats { instructions: u64, t_states: u64, elapsed: Duration }`. Both re-exported from `machine` crate root.

- [ ] **Step 1: Define the `Machine` trait**

Replace the contents of `crates/machine/src/lib.rs` with:
```rust
mod clock;
mod memory;

pub use clock::{Clock, RunStats};
pub use memory::MemoryBus;

/// A CPU + memory system driven by a fixed clock rate. Implemented once per
/// concrete computer (ZX81, ZX Spectrum, ...); `emu` and any future UI only
/// ever hold a `Box<dyn Machine>`, so they work the same way regardless of
/// which CPU chip is inside.
pub trait Machine {
    /// Fixed clock rate for this system, in Hz.
    fn clock_hz(&self) -> u64;

    /// Executes one CPU instruction. Returns its cost in T-states.
    fn step(&mut self) -> u32;

    /// True once the CPU has halted.
    fn halted(&self) -> bool;

    /// Ordered register name/value pairs, e.g. `[("AF", 0x2A00), ...]`.
    /// Order and register set are chip-specific; the shape is uniform so a
    /// future UI can render any machine's registers without knowing which
    /// CPU is inside.
    fn registers(&self) -> Vec<(&'static str, u64)>;

    /// Pure read of one memory byte for display purposes -- does not go
    /// through the CPU's normal bus read path (which may have side effects
    /// for I/O-mapped reads).
    fn read_memory(&self, addr: u16) -> u8;

    /// Loads bytes into memory starting at `addr`, through normal write
    /// semantics (a write into ROM is dropped, matching real hardware).
    fn load(&mut self, addr: u16, bytes: &[u8]);

    /// Sets the program counter. Real hardware always resets to a fixed
    /// vector; this lets a caller (e.g. `emu`) point execution at a loaded
    /// test program instead, since there's no real ROM boot code yet.
    fn set_pc(&mut self, addr: u16);
}
```

This step has no test of its own — a trait with no implementor isn't runnably testable. It's verified in Step 3 below via `Clock`, and again in Tasks 4-5 via the concrete machines.

- [ ] **Step 2: Write `Clock` against the trait, with a fake `Machine` test**

`crates/machine/src/clock.rs`:
```rust
use std::time::{Duration, Instant};

use crate::Machine;

/// Paces `Machine::step` calls to the machine's own `clock_hz()` by running
/// in fixed-size T-state budgets ("ticks") and sleeping off whatever real
/// time each tick finished early by. The clock rate comes from the machine,
/// not a caller-supplied number -- so a ZX81 always runs at ZX81 speed and a
/// Spectrum always at Spectrum speed.
pub struct Clock {
    ticks_per_sec: u64,
}

pub struct RunStats {
    pub instructions: u64,
    pub t_states: u64,
    pub elapsed: Duration,
}

impl Clock {
    pub fn new() -> Self {
        Self { ticks_per_sec: 60 }
    }

    /// Runs until `running` returns false, checked between instructions.
    pub fn run(&self, machine: &mut dyn Machine, mut running: impl FnMut(&dyn Machine) -> bool) -> RunStats {
        let hz = machine.clock_hz();
        let t_states_per_tick = hz / self.ticks_per_sec;
        let tick_duration = Duration::from_secs_f64(1.0 / self.ticks_per_sec as f64);

        let mut instructions = 0u64;
        let mut t_states = 0u64;
        let start = Instant::now();

        while running(machine) {
            let tick_start = Instant::now();
            let mut budget = t_states_per_tick;

            while budget > 0 && running(machine) {
                let cost = machine.step() as u64;
                instructions += 1;
                t_states += cost;
                budget = budget.saturating_sub(cost);
            }

            let elapsed = tick_start.elapsed();
            if elapsed < tick_duration {
                std::thread::sleep(tick_duration - elapsed);
            }
        }

        RunStats { instructions, t_states, elapsed: start.elapsed() }
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeMachine {
        hz: u64,
        steps_remaining: u32,
        cost_per_step: u32,
    }

    impl Machine for FakeMachine {
        fn clock_hz(&self) -> u64 {
            self.hz
        }
        fn step(&mut self) -> u32 {
            self.steps_remaining -= 1;
            self.cost_per_step
        }
        fn halted(&self) -> bool {
            self.steps_remaining == 0
        }
        fn registers(&self) -> Vec<(&'static str, u64)> {
            Vec::new()
        }
        fn read_memory(&self, _addr: u16) -> u8 {
            0
        }
        fn load(&mut self, _addr: u16, _bytes: &[u8]) {}
        fn set_pc(&mut self, _addr: u16) {}
    }

    #[test]
    fn run_executes_until_halted_and_reports_totals() {
        let mut machine = FakeMachine { hz: 1_000_000, steps_remaining: 5, cost_per_step: 4 };
        let clock = Clock::new();

        let stats = clock.run(&mut machine, |m| !m.halted());

        assert_eq!(stats.instructions, 5);
        assert_eq!(stats.t_states, 20);
    }
}
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test -p machine`
Expected: PASS — 7 tests (6 from Task 1 plus `run_executes_until_halted_and_reports_totals`). Note this test takes ~1 tick (~16ms real time) to complete, since `Clock` sleeps off the remainder of the first tick before its outer loop re-checks `running` and exits — that's expected, not a hang.

- [ ] **Step 4: Commit**

```bash
git add crates/machine
git commit -m "feat(machine): add Machine trait and rework Clock to run against it"
```

---

### Task 3: `z80` crate — `Cpu::registers()`

**Files:**
- Modify: `crates/z80/src/cpu.rs:51-55` (insert after `pub fn new`)

**Interfaces:**
- Consumes: existing `Cpu` fields (`af`, `bc`, `de`, `hl`, `af_`, `bc_`, `de_`, `hl_`, `ix`, `iy`, `sp`, `pc`, `i`, `r`, `im`, `iff1`, `iff2`).
- Produces: `pub fn registers(&self) -> Vec<(&'static str, u64)>` on `Cpu`, ordered `AF, BC, DE, HL, AF', BC', DE', HL', IX, IY, SP, PC, I, R, IM, IFF1, IFF2`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/z80/src/cpu.rs` (near the top, after `use crate::test_support::FlatBus;`):
```rust
    #[test]
    fn registers_reports_ordered_name_value_pairs() {
        let mut cpu = Cpu::new();
        cpu.af = 0x2A00;
        cpu.bc = 0x1234;
        cpu.pc = 0x8000;

        let regs = cpu.registers();

        assert_eq!(regs[0], ("AF", 0x2A00));
        assert_eq!(regs[1], ("BC", 0x1234));
        assert_eq!(regs[11], ("PC", 0x8000));
        assert_eq!(regs.len(), 17);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p z80 registers_reports_ordered_name_value_pairs`
Expected: FAIL with "no method named `registers` found"

- [ ] **Step 3: Implement `registers`**

In `crates/z80/src/cpu.rs`, insert into `impl Cpu` right after `pub fn new` (which ends at line 54 `}`), before `pub fn step`:
```rust

    /// Ordered register name/value pairs, for debugging/introspection.
    /// Widened to `u64` so any future (possibly wider) CPU core can report
    /// through the same shape.
    pub fn registers(&self) -> Vec<(&'static str, u64)> {
        vec![
            ("AF", self.af as u64),
            ("BC", self.bc as u64),
            ("DE", self.de as u64),
            ("HL", self.hl as u64),
            ("AF'", self.af_ as u64),
            ("BC'", self.bc_ as u64),
            ("DE'", self.de_ as u64),
            ("HL'", self.hl_ as u64),
            ("IX", self.ix as u64),
            ("IY", self.iy as u64),
            ("SP", self.sp as u64),
            ("PC", self.pc as u64),
            ("I", self.i as u64),
            ("R", self.r as u64),
            ("IM", self.im as u64),
            ("IFF1", self.iff1 as u64),
            ("IFF2", self.iff2 as u64),
        ]
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p z80 registers_reports_ordered_name_value_pairs`
Expected: PASS

- [ ] **Step 5: Run the full z80 test suite to confirm no regressions**

Run: `cargo test -p z80`
Expected: PASS — 184 tests (183 existing + 1 new).

- [ ] **Step 6: Commit**

```bash
git add crates/z80/src/cpu.rs
git commit -m "feat(z80): add Cpu::registers() for CPU-agnostic introspection"
```

---

### Task 4: `zx81` crate

**Files:**
- Create: `crates/zx81/Cargo.toml`
- Create: `crates/zx81/src/lib.rs`
- Modify: `Cargo.toml:3` (workspace members)

**Interfaces:**
- Consumes: `machine::{Machine, MemoryBus, Clock}` (Tasks 1-2), `z80::Cpu` + `Cpu::registers()` (Task 3, unchanged `Cpu::step`/`pc`/`halted` fields).
- Produces: `pub const ROM_SIZE: usize`, `pub const RAM_BASE: u16`, `pub const RAM_SIZE: usize`, `pub struct Zx81`, `impl Zx81 { pub fn new(rom: Vec<u8>) -> Self }`, `impl Machine for Zx81`.

- [ ] **Step 1: Create the crate scaffold**

`crates/zx81/Cargo.toml`:
```toml
[package]
name = "zx81"
version = "0.1.0"
edition = "2024"

[dependencies]
z80 = { path = "../z80" }
machine = { path = "../machine" }
```

Add `"crates/zx81"` to the workspace members list in the root `Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/z80", "crates/emu", "crates/machine", "crates/zx81"]
```

- [ ] **Step 2: Write the failing smoke test**

`crates/zx81/src/lib.rs`:
```rust
use machine::{Machine, MemoryBus};
use z80::Cpu;

/// ZX81 ROM size (8K, the unexpanded machine's only ROM chip).
pub const ROM_SIZE: usize = 0x2000;
/// Fixed RAM base address on real hardware.
pub const RAM_BASE: u16 = 0x4000;
/// Unexpanded ZX81 RAM size (1K on-board, no 16K RAM pack).
pub const RAM_SIZE: usize = 0x0400;

/// A ZX81: a Z80 core, an 8K ROM, and 1K of RAM. No ULA/keyboard/tape yet --
/// this is the clock-and-memory-map skeleton other peripherals attach to
/// later.
pub struct Zx81 {
    cpu: Cpu,
    bus: MemoryBus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_a_tiny_program_to_halt_and_reports_registers() {
        let mut zx81 = Zx81::new(vec![0; ROM_SIZE]);
        // LD A,0x2A ; HALT -- loaded into RAM since ROM writes are dropped.
        zx81.load(RAM_BASE, &[0x3E, 0x2A, 0x76]);
        zx81.set_pc(RAM_BASE);

        let clock = machine::Clock::new();
        let stats = clock.run(&mut zx81, |m| !m.halted());

        assert_eq!(stats.instructions, 2);
        assert!(zx81.halted());
        assert_eq!(zx81.registers()[0], ("AF", 0x2A00));
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p zx81`
Expected: FAIL to compile — `Zx81::new` doesn't exist yet, and `Machine` isn't implemented for `Zx81`.

- [ ] **Step 4: Implement `Zx81::new` and `impl Machine for Zx81`**

Add to `crates/zx81/src/lib.rs`, after the `Zx81` struct definition and before the `#[cfg(test)]` module:
```rust
impl Zx81 {
    /// `rom` is the real ZX81 ROM image (8K), supplied externally -- it's
    /// copyrighted and not bundled with this repo.
    pub fn new(rom: Vec<u8>) -> Self {
        let mut bus = MemoryBus::new();
        bus.map_rom(0x0000, rom);
        bus.map_ram(RAM_BASE, RAM_SIZE);
        Self { cpu: Cpu::new(), bus }
    }
}

impl Machine for Zx81 {
    fn clock_hz(&self) -> u64 {
        3_250_000
    }

    fn step(&mut self) -> u32 {
        self.cpu.step(&mut self.bus) as u32
    }

    fn halted(&self) -> bool {
        self.cpu.halted
    }

    fn registers(&self) -> Vec<(&'static str, u64)> {
        self.cpu.registers()
    }

    fn read_memory(&self, addr: u16) -> u8 {
        self.bus.peek(addr)
    }

    fn load(&mut self, addr: u16, bytes: &[u8]) {
        self.bus.load(addr, bytes);
    }

    fn set_pc(&mut self, addr: u16) {
        self.cpu.pc = addr;
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p zx81`
Expected: PASS — 1 test.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/zx81
git commit -m "feat(zx81): add ZX81 machine skeleton (3.25MHz, 8K ROM, 1K RAM)"
```

---

### Task 5: `zx-spectrum` crate

**Files:**
- Create: `crates/zx-spectrum/Cargo.toml`
- Create: `crates/zx-spectrum/src/lib.rs`
- Modify: `Cargo.toml:3` (workspace members)

**Interfaces:**
- Consumes: same as Task 4 (`machine::{Machine, MemoryBus, Clock}`, `z80::Cpu` + `Cpu::registers()`).
- Produces: `pub const ROM_SIZE: usize`, `pub const RAM_BASE: u16`, `pub const RAM_SIZE: usize`, `pub struct ZxSpectrum`, `impl ZxSpectrum { pub fn new(rom: Vec<u8>) -> Self }`, `impl Machine for ZxSpectrum`. (Package name `zx-spectrum`; Cargo exposes it to dependents as the `zx_spectrum` crate.)

- [ ] **Step 1: Create the crate scaffold**

`crates/zx-spectrum/Cargo.toml`:
```toml
[package]
name = "zx-spectrum"
version = "0.1.0"
edition = "2024"

[dependencies]
z80 = { path = "../z80" }
machine = { path = "../machine" }
```

Add `"crates/zx-spectrum"` to the workspace members list in the root `Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/z80", "crates/emu", "crates/machine", "crates/zx81", "crates/zx-spectrum"]
```

- [ ] **Step 2: Write the failing smoke test**

`crates/zx-spectrum/src/lib.rs`:
```rust
use machine::{Machine, MemoryBus};
use z80::Cpu;

/// ZX Spectrum 48K ROM size (16K).
pub const ROM_SIZE: usize = 0x4000;
/// Fixed RAM base address on real hardware.
pub const RAM_BASE: u16 = 0x4000;
/// ZX Spectrum 48K RAM size.
pub const RAM_SIZE: usize = 0xC000;

/// A 48K ZX Spectrum: a Z80 core, a 16K ROM, and 48K of RAM. No ULA/keyboard
/// /tape yet -- this is the clock-and-memory-map skeleton other peripherals
/// attach to later.
pub struct ZxSpectrum {
    cpu: Cpu,
    bus: MemoryBus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_a_tiny_program_to_halt_and_reports_registers() {
        let mut spectrum = ZxSpectrum::new(vec![0; ROM_SIZE]);
        // LD A,0x2A ; HALT -- loaded into RAM since ROM writes are dropped.
        spectrum.load(RAM_BASE, &[0x3E, 0x2A, 0x76]);
        spectrum.set_pc(RAM_BASE);

        let clock = machine::Clock::new();
        let stats = clock.run(&mut spectrum, |m| !m.halted());

        assert_eq!(stats.instructions, 2);
        assert!(spectrum.halted());
        assert_eq!(spectrum.registers()[0], ("AF", 0x2A00));
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p zx-spectrum`
Expected: FAIL to compile — `ZxSpectrum::new` doesn't exist yet, and `Machine` isn't implemented for `ZxSpectrum`.

- [ ] **Step 4: Implement `ZxSpectrum::new` and `impl Machine for ZxSpectrum`**

Add to `crates/zx-spectrum/src/lib.rs`, after the `ZxSpectrum` struct definition and before the `#[cfg(test)]` module:
```rust
impl ZxSpectrum {
    /// `rom` is the real 48K Spectrum ROM image, supplied externally -- it's
    /// copyrighted and not bundled with this repo.
    pub fn new(rom: Vec<u8>) -> Self {
        let mut bus = MemoryBus::new();
        bus.map_rom(0x0000, rom);
        bus.map_ram(RAM_BASE, RAM_SIZE);
        Self { cpu: Cpu::new(), bus }
    }
}

impl Machine for ZxSpectrum {
    fn clock_hz(&self) -> u64 {
        3_546_900
    }

    fn step(&mut self) -> u32 {
        self.cpu.step(&mut self.bus) as u32
    }

    fn halted(&self) -> bool {
        self.cpu.halted
    }

    fn registers(&self) -> Vec<(&'static str, u64)> {
        self.cpu.registers()
    }

    fn read_memory(&self, addr: u16) -> u8 {
        self.bus.peek(addr)
    }

    fn load(&mut self, addr: u16, bytes: &[u8]) {
        self.bus.load(addr, bytes);
    }

    fn set_pc(&mut self, addr: u16) {
        self.cpu.pc = addr;
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p zx-spectrum`
Expected: PASS — 1 test.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/zx-spectrum
git commit -m "feat(zx-spectrum): add ZX Spectrum 48K machine skeleton (3.5469MHz, 16K ROM, 48K RAM)"
```

---

### Task 6: Rewire `emu` onto `Machine`, add `--machine`/`--dump-memory`, drop `--hz`

**Files:**
- Modify: `crates/emu/Cargo.toml`
- Modify: `crates/emu/src/main.rs` (full rewrite)
- Delete: `crates/emu/src/clock.rs`
- Delete: `crates/emu/src/machine.rs`
- Delete: `crates/emu/src/memory.rs`

**Interfaces:**
- Consumes: `machine::{Clock, Machine}` (Tasks 1-2), `zx81::{Zx81, ROM_SIZE}` (Task 4), `zx_spectrum::{ZxSpectrum, ROM_SIZE}` (Task 5).
- Produces: `emu` binary CLI: `emu <path-to-program> --machine <zx81|zx-spectrum> [--org 0xNNNN] [--rom <path>] [--dump-memory 0xSTART:LEN]`.

This task has no automated test (it's a binary's CLI, same as before) — verified by manual smoke test, matching how `emu` has been verified throughout this project so far.

- [ ] **Step 1: Update `emu`'s dependencies**

Replace `crates/emu/Cargo.toml` with:
```toml
[package]
name = "emu"
version = "0.1.0"
edition = "2024"

[dependencies]
machine = { path = "../machine" }
zx81 = { path = "../zx81" }
zx-spectrum = { path = "../zx-spectrum" }
```

- [ ] **Step 2: Delete the now-superseded local modules**

```bash
git rm crates/emu/src/clock.rs crates/emu/src/machine.rs crates/emu/src/memory.rs
```

- [ ] **Step 3: Rewrite `crates/emu/src/main.rs`**

Replace its full contents with:
```rust
use std::env;
use std::fs;
use std::process::ExitCode;

use machine::{Clock, Machine};

enum MachineKind {
    Zx81,
    ZxSpectrum,
}

struct Args {
    program_path: String,
    org: u16,
    rom_path: Option<String>,
    machine: MachineKind,
    dump_memory: Option<(u16, u16)>,
}

fn parse_args() -> Result<Args, String> {
    let mut program_path = None;
    let mut org = 0x4000u16;
    let mut rom_path = None;
    let mut machine = None;
    let mut dump_memory = None;

    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--org" => {
                let v = it.next().ok_or("--org needs a value")?;
                org = u16::from_str_radix(v.trim_start_matches("0x"), 16).map_err(|e| e.to_string())?;
            }
            "--rom" => {
                rom_path = Some(it.next().ok_or("--rom needs a value")?);
            }
            "--machine" => {
                let v = it.next().ok_or("--machine needs a value")?;
                machine = Some(match v.as_str() {
                    "zx81" => MachineKind::Zx81,
                    "zx-spectrum" => MachineKind::ZxSpectrum,
                    other => return Err(format!("unknown machine: {other} (expected zx81 or zx-spectrum)")),
                });
            }
            "--dump-memory" => {
                let v = it.next().ok_or("--dump-memory needs a value (e.g. 0x4000:16)")?;
                let (start, len) = v.split_once(':').ok_or("--dump-memory expects START:LEN")?;
                let start = u16::from_str_radix(start.trim_start_matches("0x"), 16).map_err(|e| e.to_string())?;
                let len: u16 = len.parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
                dump_memory = Some((start, len));
            }
            other if program_path.is_none() => program_path = Some(other.to_string()),
            other => return Err(format!("unexpected argument: {other}")),
        }
    }

    Ok(Args {
        program_path: program_path.ok_or("missing <path-to-program> argument")?,
        org,
        rom_path,
        machine: machine.ok_or("missing --machine <zx81|zx-spectrum>")?,
        dump_memory,
    })
}

fn rom_size(kind: &MachineKind) -> usize {
    match kind {
        MachineKind::Zx81 => zx81::ROM_SIZE,
        MachineKind::ZxSpectrum => zx_spectrum::ROM_SIZE,
    }
}

fn build_machine(kind: MachineKind, rom: Vec<u8>) -> Box<dyn Machine> {
    match kind {
        MachineKind::Zx81 => Box::new(zx81::Zx81::new(rom)),
        MachineKind::ZxSpectrum => Box::new(zx_spectrum::ZxSpectrum::new(rom)),
    }
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: emu <path-to-program> --machine <zx81|zx-spectrum> [--org 0xNNNN] [--rom <path>] [--dump-memory 0xSTART:LEN]"
            );
            return ExitCode::FAILURE;
        }
    };

    let program = match fs::read(&args.program_path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("failed to read {}: {e}", args.program_path);
            return ExitCode::FAILURE;
        }
    };

    let rom = match args.rom_path {
        Some(path) => match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("failed to read {path}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => vec![0; rom_size(&args.machine)],
    };

    let mut machine = build_machine(args.machine, rom);
    machine.load(args.org, &program);
    machine.set_pc(args.org);

    let clock = Clock::new();
    let stats = clock.run(machine.as_mut(), |m| !m.halted());

    let avg_hz = if stats.elapsed.as_secs_f64() > 0.0 {
        stats.t_states as f64 / stats.elapsed.as_secs_f64()
    } else {
        0.0
    };

    println!("halted");
    println!(
        "{} instructions, {} T-states, {:.3}s elapsed (~{:.0} Hz effective)",
        stats.instructions,
        stats.t_states,
        stats.elapsed.as_secs_f64(),
        avg_hz
    );

    println!("registers:");
    for (name, value) in machine.registers() {
        println!("  {name:<4} = 0x{value:04X}");
    }

    if let Some((start, len)) = args.dump_memory {
        println!("memory 0x{start:04X}..0x{:04X}:", start as u32 + len as u32);
        for offset in 0..len {
            let addr = start.wrapping_add(offset);
            if offset % 16 == 0 {
                print!("  0x{addr:04X}:");
            }
            print!(" {:02X}", machine.read_memory(addr));
            if offset % 16 == 15 {
                println!();
            }
        }
        if len % 16 != 0 {
            println!();
        }
    }

    ExitCode::SUCCESS
}
```

- [ ] **Step 4: Build the whole workspace**

Run: `cargo build --workspace`
Expected: builds cleanly, no warnings (in particular, no unused-dependency or dead-code warnings from the deleted modules).

- [ ] **Step 5: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS — z80 (184), machine (7), zx81 (1), zx-spectrum (1) all green.

- [ ] **Step 6: Manual smoke test — ZX81, default (no `--rom`)**

```bash
printf '\x3e\x2a\x76' > /tmp/tiny.bin
cargo run -p emu -- /tmp/tiny.bin --machine zx81
```
Expected output includes `halted`, `2 instructions, 11 T-states`, and a `registers:` block whose first line is `AF   = 0x2A00`.

- [ ] **Step 7: Manual smoke test — ZX Spectrum, default (no `--rom`)**

```bash
cargo run -p emu -- /tmp/tiny.bin --machine zx-spectrum
```
Expected output: same shape as Step 6 (`AF   = 0x2A00`), confirming both machines run through the same trait-driven path.

- [ ] **Step 8: Manual smoke test — `--dump-memory`**

```bash
cargo run -p emu -- /tmp/tiny.bin --machine zx81 --dump-memory 0x4000:4
```
Expected: a `memory 0x4000..0x4004:` line followed by one line of hex bytes starting `3E 2A 76 00` (the loaded program, plus one zero byte of untouched RAM).

- [ ] **Step 9: Commit**

```bash
git add crates/emu Cargo.toml
git commit -m "feat(emu): drive machines through the Machine trait; add --machine and --dump-memory, drop --hz"
```

---

## Post-plan state

- `crates/machine` is the shared, CPU-agnostic plumbing: `Machine` trait, `Clock`, `MemoryBus`.
- `crates/zx81` and `crates/zx-spectrum` are thin, correctly-clocked, correctly-memory-mapped skeletons — no peripherals yet.
- `crates/emu` only ever holds a `Box<dyn Machine>`; adding a third machine (or a non-Z80 CPU crate down the line) means a new machine crate plus one arm in `emu`'s `--machine` match, not a change to `emu`'s core logic.
- Register/memory introspection is uniform across machines via `Machine::registers()`/`read_memory()`, ready for a future UI to consume without knowing which CPU is inside.
