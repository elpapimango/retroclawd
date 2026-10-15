# Machine-Owned Timing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move ownership of the run loop from a caller-constructed `Clock` to each `Machine` impl itself, and make both CPU clock rate and pacing/refresh rate region-aware (PAL/NTSC/Japan), so a future machine with a fundamentally different timing model (e.g. ZX81 video/NMI generation) can override its own loop without changing the `Machine` trait again.

**Architecture:** `machine` crate gains a `Region` enum, `Clock::new` takes its tick rate as a parameter (no more hardcoded 60), and `Machine` gains `frame_hz()` plus a required (not default) `run()` method — required rather than default because a default trait-method body cannot coerce an abstract `&mut Self` into `&mut dyn Machine` without a `Self: Sized` bound, and that bound would exclude the method from the vtable, making it uncallable on the `Box<dyn Machine>` that `emu` holds. Verified with a standalone `rustc` compile (see Task 3) — each machine impl instead writes its own one-line `run()` body (`Clock::new(self.frame_hz()).run(self, running)`), which compiles fine because `self` is concretely `Sized` inside an `impl` block, and remains fully overridable per machine later. `zx81` and `zx-spectrum` each take a `Region` at construction and map it to their own `clock_hz`/`frame_hz` values. `emu` drops `Clock` entirely and calls `machine.run(...)` directly on its `Box<dyn Machine>`.

**Tech Stack:** Rust 2024 edition, Cargo workspace, no external dependencies (stdlib only) — matches the rest of the workspace.

**Spec:** `docs/superpowers/specs/2026-09-05-machine-owned-timing-design.md`

## Global Constraints

- No external crate dependencies added anywhere in this plan — stdlib only.
- No ULA/video, NMI, or any actual ZX81 timing-model implementation this round — only the trait/ownership shape and region-aware Hz values change. `Zx81`/`ZxSpectrum`'s `run()` bodies stay identical one-liners delegating to `Clock`.
- `Region::Ntsc`/`Region::Japan` use the same numeric values as `Region::Pal` for `zx81`'s `clock_hz()` (placeholder — real NTSC/Timex Sinclair 1000 crystal not yet verified) and for all of `zx-spectrum`'s non-`Pal` arms (no real Spectrum variant existed for them). Both are explicitly flagged, not silently guessed.
- `zx81`/`zx-spectrum` frame_hz: `Region::Pal` -> 50, `Region::Ntsc` | `Region::Japan` -> 60.

---

### Task 1: `machine` crate — `Region` enum

**Files:**
- Create: `crates/machine/src/region.rs`
- Modify: `crates/machine/src/lib.rs`

**Interfaces:**
- Produces: `pub enum Region { Pal, Ntsc, Japan }` deriving `Debug, Clone, Copy, PartialEq, Eq`. Re-exported from the `machine` crate root as `machine::Region`.

- [ ] **Step 1: Write the failing test**

`crates/machine/src/region.rs`:
```rust
/// Broadcast/market variant a machine was built for. Affects both CPU clock
/// speed (crystal differs per market) and refresh rate (a machine's
/// `frame_hz`) -- each machine impl maps this to its own concrete Hz
/// values; `Region` itself carries no numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Pal,
    Ntsc,
    Japan,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_are_distinct_and_copy() {
        let a = Region::Pal;
        let b = a; // proves Copy -- if this moved, using `a` below would fail to compile
        assert_eq!(a, Region::Pal);
        assert_ne!(b, Region::Ntsc);
        assert_ne!(Region::Ntsc, Region::Japan);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p machine variants_are_distinct_and_copy`
Expected: FAIL to compile — `region` module not declared in `lib.rs`, so `crate::region::Region` (and the test's own `use super::*`) doesn't resolve.

- [ ] **Step 3: Wire the new module into `lib.rs`**

In `crates/machine/src/lib.rs`, change:
```rust
mod bus;
mod clock;
mod memory;

pub use bus::Bus;
pub use clock::{Clock, RunStats};
pub use memory::MemoryBus;
```
to:
```rust
mod bus;
mod clock;
mod memory;
mod region;

pub use bus::Bus;
pub use clock::{Clock, RunStats};
pub use memory::MemoryBus;
pub use region::Region;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p machine variants_are_distinct_and_copy`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/machine/src/region.rs crates/machine/src/lib.rs
git commit -m "feat(machine): add Region enum (Pal/Ntsc/Japan)"
```

---

### Task 2: `machine` crate — `Clock` takes its tick rate as a constructor argument

**Files:**
- Modify: `crates/machine/src/clock.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `Clock::new(ticks_per_sec: u64) -> Self` (was `Clock::new() -> Self`). `impl Default for Clock` is removed. `Clock::run` keeps its existing signature and body unchanged.

- [ ] **Step 1: Update the existing tests to the new constructor shape (this is the failing step — they currently call the zero-arg form)**

In `crates/machine/src/clock.rs`, change both test bodies:
```rust
    #[test]
    fn run_executes_until_halted_and_reports_totals() {
        let mut machine = FakeMachine {
            hz: 1_000_000,
            steps_remaining: 5,
            cost_per_step: 4,
        };
        let clock = Clock::new(60);
```
and:
```rust
    #[test]
    fn run_still_makes_progress_below_one_t_state_per_tick() {
        // hz < ticks_per_sec would make the per-tick budget 0, so `run` would
        // sleep forever without ever stepping. Two steps at one T-state of
        // budget per tick means this finishes in ~2 ticks (~33ms).
        let mut machine = FakeMachine {
            hz: 30,
            steps_remaining: 2,
            cost_per_step: 4,
        };
        let clock = Clock::new(60);
```
(only the `Clock::new()` -> `Clock::new(60)` lines change; nothing else in either test body.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p machine`
Expected: FAIL to compile — `this function takes 0 arguments but 1 argument was supplied` (the struct/impl still define the zero-arg `new`).

- [ ] **Step 3: Change `Clock::new` and remove `Default`**

In `crates/machine/src/clock.rs`, change:
```rust
impl Clock {
    pub fn new() -> Self {
        Self { ticks_per_sec: 60 }
    }
```
to:
```rust
impl Clock {
    pub fn new(ticks_per_sec: u64) -> Self {
        Self { ticks_per_sec }
    }
```
and delete the `impl Default for Clock { ... }` block entirely (there's no longer a sensible rate-independent default).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p machine`
Expected: PASS — 11 tests (10 baseline -- 8 `memory` + 2 `clock` -- plus `variants_are_distinct_and_copy` from Task 1; the 2 `clock` tests now pass with the new constructor).

- [ ] **Step 5: Write a failing test proving the tick rate is actually threaded through (not silently ignored)**

Add to the `tests` module in `crates/machine/src/clock.rs`:
```rust
    #[test]
    fn different_tick_rates_change_how_many_steps_run_per_tick() {
        use std::cell::Cell;

        // hz=100, cost_per_step=10: at ticks_per_sec=5, budget=100/5=20
        // T-states/tick -- fits two 10-T-state steps per tick, so all 4
        // steps finish within 2 ticks. At ticks_per_sec=100, budget=1
        // T-state/tick -- less than one step's cost, so only one step fits
        // per tick, needing 4 ticks. `running` is called once per step
        // attempt plus once per tick boundary, so counting its calls
        // reveals how many ticks each rate actually took.
        let calls = Cell::new(0u32);

        let mut slow = FakeMachine {
            hz: 100,
            steps_remaining: 4,
            cost_per_step: 10,
        };
        let slow_stats = Clock::new(5).run(&mut slow, |m| {
            calls.set(calls.get() + 1);
            !m.halted()
        });
        let calls_at_slow_rate = calls.get();

        calls.set(0);
        let mut fast = FakeMachine {
            hz: 100,
            steps_remaining: 4,
            cost_per_step: 10,
        };
        let fast_stats = Clock::new(100).run(&mut fast, |m| {
            calls.set(calls.get() + 1);
            !m.halted()
        });

        assert_eq!(slow_stats.instructions, 4);
        assert_eq!(fast_stats.instructions, 4);
        // Same total work, different tick-boundary structure -- proves the
        // rate passed to `Clock::new` drives the budget math.
        assert_eq!(calls_at_slow_rate, 7);
        assert_eq!(calls.get(), 9);
    }
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test -p machine different_tick_rates_change_how_many_steps_run_per_tick`
Expected: PASS. (Verified against a standalone `rustc` compile of the same `Clock`/`FakeMachine` shapes before writing this plan: slow rate = 7 calls, fast rate = 9 calls, both 4 instructions.)

- [ ] **Step 7: Run the full test suite**

Run: `cargo test -p machine`
Expected: PASS — 12 tests.

- [ ] **Step 8: Commit**

```bash
git add crates/machine/src/clock.rs
git commit -m "feat(machine): Clock::new takes its tick rate as a parameter"
```

---

### Task 3: `machine` crate — `Machine` trait gains `frame_hz()` and a required, dyn-dispatchable `run()`

**Files:**
- Modify: `crates/machine/src/lib.rs`
- Modify: `crates/machine/src/clock.rs` (the `FakeMachine` test helper)

**Interfaces:**
- Consumes: `Region` (Task 1, not used directly by the trait itself, but by implementors), `Clock::new(u64)` (Task 2).
- Produces: `Machine::frame_hz(&self) -> u64` (required). `Machine::run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> RunStats` (required — no default body). Every current and future `impl Machine` must supply both.

**Why `run` has no default body:** a default trait-method body that calls `Clock::new(self.frame_hz()).run(self, running)` needs to pass `self` (type `&mut Self`, where `Self` is abstract inside the trait definition) as `&mut dyn Machine`. That coercion requires `Self: Sized`, and Rust then excludes any method carrying a `Self: Sized` bound from the trait's vtable — so it could never be called as `boxed_machine.run(...)` where `boxed_machine: Box<dyn Machine>`, which is exactly how `emu` needs to call it (Task 6). Making `run` a required method with no default avoids the coercion entirely: inside each `impl Machine for ConcreteType`, `self` is concretely `Sized`, so the same one-line body compiles per-impl and is fully callable and overridable through `Box<dyn Machine>`. This was verified with a standalone `rustc --edition 2024` compile before writing this plan (three variants tried: default body with `Self: Sized` -- compiles but uncallable via `Box<dyn Machine>`; default body without `Self: Sized` -- fails with "the size for values of type `Self` cannot be known at compilation time"; required per-impl method -- compiles, dispatches virtually, and is overridable). Step 4 below reproduces the working, decisive case as a permanent regression test.

- [ ] **Step 1: Add `frame_hz` and `run` to the trait, and to `FakeMachine` (this step intentionally breaks the build -- the next step confirms exactly how)**

In `crates/machine/src/lib.rs`, change the trait from:
```rust
pub trait Machine {
    /// Fixed clock rate for this system, in Hz.
    fn clock_hz(&self) -> u64;

    /// Executes one CPU instruction. Returns its cost in T-states.
    fn step(&mut self) -> u32;
```
to:
```rust
pub trait Machine {
    /// Fixed clock rate for this system, in Hz.
    fn clock_hz(&self) -> u64;

    /// Pacing/refresh rate for this system, in Hz (e.g. 50 for PAL, 60 for
    /// NTSC/Japan). Read by each machine's own `run()` to build a `Clock`
    /// at the right tick rate -- distinct from `clock_hz` (CPU speed).
    fn frame_hz(&self) -> u64;

    /// Runs this machine until `running` returns false, checked between
    /// instructions. Every machine implements this itself (rather than a
    /// shared default) so a machine with a fundamentally different timing
    /// model can replace the loop entirely; the common case is one line:
    /// `Clock::new(self.frame_hz()).run(self, running)`.
    fn run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> RunStats;

    /// Executes one CPU instruction. Returns its cost in T-states.
    fn step(&mut self) -> u32;
```

In `crates/machine/src/clock.rs`, add both new methods to the `FakeMachine` test impl:
```rust
    impl Machine for FakeMachine {
        fn clock_hz(&self) -> u64 {
            self.hz
        }
        fn frame_hz(&self) -> u64 {
            60
        }
        fn run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> RunStats {
            Clock::new(self.frame_hz()).run(self, running)
        }
        fn step(&mut self) -> u32 {
            self.steps_remaining -= 1;
            self.cost_per_step
        }
```

- [ ] **Step 2: Run tests to verify the crate still compiles and passes**

Run: `cargo test -p machine`
Expected: PASS — 12 tests, unchanged from Task 2. (This step is verification, not a red step, since `FakeMachine` was updated in the same edit as the trait — there's no intermediate broken state to observe here; the decisive red/green pair is Step 3/4 below, which tests the actual design question.)

- [ ] **Step 3: Write a failing test proving `run` is callable and overridable through `Box<dyn Machine>`**

Add to the `tests` module in `crates/machine/src/clock.rs`:
```rust
    struct CountingOverride {
        hz: u64,
        steps_remaining: u32,
        override_calls: u32,
    }

    impl Machine for CountingOverride {
        fn clock_hz(&self) -> u64 {
            self.hz
        }
        fn frame_hz(&self) -> u64 {
            60
        }
        fn run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> RunStats {
            // A deliberately non-default body, standing in for a future
            // machine with its own timing model -- proves override, not
            // just default dispatch, works through a trait object.
            self.override_calls += 1;
            Clock::new(self.frame_hz()).run(self, running)
        }
        fn step(&mut self) -> u32 {
            self.steps_remaining -= 1;
            4
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
        fn load(&mut self, _addr: u16, bytes: &[u8]) -> usize {
            bytes.len()
        }
        fn set_pc(&mut self, _addr: u16) {}
    }

    #[test]
    fn run_dispatches_through_a_boxed_trait_object_and_can_be_overridden() {
        let mut boxed: Box<dyn Machine> = Box::new(CountingOverride {
            hz: 1_000_000,
            steps_remaining: 3,
            override_calls: 0,
        });

        let stats = boxed.run(&mut |m| !m.halted());

        assert_eq!(stats.instructions, 3);
        assert!(boxed.halted());
    }
```

- [ ] **Step 4: Run test to verify it fails first, for the right reason, if `run` were a default method instead**

This step is a design check, not a normal red/green cycle against missing code: `run` is already implemented on `CountingOverride` in Step 3, so this test compiles and passes as written. To confirm the design reasoning in this task's header is correct (not just asserted), temporarily change the trait's `run` to a default body instead of a required method:
```rust
    fn run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> RunStats
    where
        Self: Sized,
    {
        Clock::new(self.frame_hz()).run(self, running)
    }
```
and remove `CountingOverride`'s own `run` impl.

Run: `cargo test -p machine run_dispatches_through_a_boxed_trait_object_and_can_be_overridden`
Expected: FAIL to compile — `error: the \`run\` method cannot be invoked on a trait object` with a note pointing at `Self: Sized, -- this has a \`Sized\` requirement` (verified against a standalone `rustc --edition 2024` compile of this exact shape before writing this plan).

Revert both edits (restore `run` as a required trait method with no default, and restore `CountingOverride`'s own `run` impl) before continuing.

- [ ] **Step 5: Run the full test suite to confirm the reverted, correct state passes**

Run: `cargo test -p machine`
Expected: PASS — 13 tests (12 from before this task plus `run_dispatches_through_a_boxed_trait_object_and_can_be_overridden`).

- [ ] **Step 6: Commit**

```bash
git add crates/machine/src/lib.rs crates/machine/src/clock.rs
git commit -m "feat(machine): Machine gains frame_hz() and a required, dyn-dispatchable run()"
```

---

### Task 4: `zx81` crate — `Region`-aware clock/frame rate, own `run()`

**Files:**
- Modify: `crates/zx81/src/lib.rs`

**Interfaces:**
- Consumes: `machine::{Machine, MemoryBus, Clock, Region}` (Tasks 1-3).
- Produces: `Zx81::new(rom: Vec<u8>, region: Region) -> Self` (was `new(rom: Vec<u8>)`). `impl Machine for Zx81` gains `frame_hz` and `run`; `clock_hz` becomes region-dependent.

- [ ] **Step 1: Update the existing test to the new constructor and call shape (failing step)**

In `crates/zx81/src/lib.rs`, change the test:
```rust
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
```
to:
```rust
    #[test]
    fn runs_a_tiny_program_to_halt_and_reports_registers() {
        let mut zx81 = Zx81::new(vec![0; ROM_SIZE], Region::Pal);
        // LD A,0x2A ; HALT -- loaded into RAM since ROM writes are dropped.
        zx81.load(RAM_BASE, &[0x3E, 0x2A, 0x76]);
        zx81.set_pc(RAM_BASE);

        let stats = zx81.run(&mut |m| !m.halted());

        assert_eq!(stats.instructions, 2);
        assert!(zx81.halted());
        assert_eq!(zx81.registers()[0], ("AF", 0x2A00));
    }

    #[test]
    fn frame_hz_follows_region() {
        let pal = Zx81::new(vec![0; ROM_SIZE], Region::Pal);
        let ntsc = Zx81::new(vec![0; ROM_SIZE], Region::Ntsc);

        assert_eq!(pal.frame_hz(), 50);
        assert_eq!(ntsc.frame_hz(), 60);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p zx81`
Expected: FAIL to compile — `Zx81::new` takes 1 argument but 2 were supplied, `Region` not imported, `Machine` has no method `run` implemented for `Zx81` yet with the new signature, `frame_hz` doesn't exist.

- [ ] **Step 3: Implement the region field, constructor, and trait impl**

In `crates/zx81/src/lib.rs`, change:
```rust
use machine::{Machine, MemoryBus};
use z80::Cpu;
```
to:
```rust
use machine::{Clock, Machine, MemoryBus, Region};
use z80::Cpu;
```

Change the struct and constructor:
```rust
pub struct Zx81 {
    cpu: Cpu,
    bus: MemoryBus,
    region: Region,
}

impl Zx81 {
    /// `rom` is the real ZX81 ROM image (8K), supplied externally -- it's
    /// copyrighted and not bundled with this repo. `region` selects the
    /// market variant's clock/frame rate (see `Machine::clock_hz`/
    /// `frame_hz` below).
    pub fn new(rom: Vec<u8>, region: Region) -> Self {
        let mut bus = MemoryBus::new();
        bus.map_rom(0x0000, rom);
        bus.map_ram(RAM_BASE, RAM_SIZE);
        Self {
            cpu: Cpu::new(),
            bus,
            region,
        }
    }
}
```

Change the `Machine` impl's `clock_hz` and add `frame_hz`/`run`:
```rust
impl Machine for Zx81 {
    fn clock_hz(&self) -> u64 {
        match self.region {
            Region::Pal => 3_250_000,
            // Placeholder: real NTSC ZX81 / Timex Sinclair 1000 crystal not
            // yet verified against a hardware reference -- tracked as an
            // open question in the design spec.
            Region::Ntsc | Region::Japan => 3_250_000,
        }
    }

    fn frame_hz(&self) -> u64 {
        match self.region {
            Region::Pal => 50,
            Region::Ntsc | Region::Japan => 60,
        }
    }

    fn run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> RunStats {
        Clock::new(self.frame_hz()).run(self, running)
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

    fn load(&mut self, addr: u16, bytes: &[u8]) -> usize {
        self.bus.load(addr, bytes)
    }

    fn set_pc(&mut self, addr: u16) {
        self.cpu.pc = addr;
    }
}
```

`RunStats` is used in `run`'s return type but not yet imported — add it to the `use machine::{...}` line from this step: `use machine::{Clock, Machine, MemoryBus, Region, RunStats};`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p zx81`
Expected: PASS — 2 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/zx81/src/lib.rs
git commit -m "feat(zx81): region-aware clock/frame rate, own run()"
```

---

### Task 5: `zx-spectrum` crate — `Region`-aware clock/frame rate, own `run()`

**Files:**
- Modify: `crates/zx-spectrum/src/lib.rs`

**Interfaces:**
- Consumes: `machine::{Machine, MemoryBus, Clock, Region, RunStats}` (Tasks 1-3).
- Produces: `ZxSpectrum::new(rom: Vec<u8>, region: Region) -> Self` (was `new(rom: Vec<u8>)`). `impl Machine for ZxSpectrum` gains `frame_hz` and `run`.

- [ ] **Step 1: Update the existing test to the new constructor and call shape (failing step)**

In `crates/zx-spectrum/src/lib.rs`, change the test:
```rust
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
```
to:
```rust
    #[test]
    fn runs_a_tiny_program_to_halt_and_reports_registers() {
        let mut spectrum = ZxSpectrum::new(vec![0; ROM_SIZE], Region::Pal);
        // LD A,0x2A ; HALT -- loaded into RAM since ROM writes are dropped.
        spectrum.load(RAM_BASE, &[0x3E, 0x2A, 0x76]);
        spectrum.set_pc(RAM_BASE);

        let stats = spectrum.run(&mut |m| !m.halted());

        assert_eq!(stats.instructions, 2);
        assert!(spectrum.halted());
        assert_eq!(spectrum.registers()[0], ("AF", 0x2A00));
    }

    #[test]
    fn frame_hz_follows_region() {
        let pal = ZxSpectrum::new(vec![0; ROM_SIZE], Region::Pal);
        let ntsc = ZxSpectrum::new(vec![0; ROM_SIZE], Region::Ntsc);

        assert_eq!(pal.frame_hz(), 50);
        assert_eq!(ntsc.frame_hz(), 60);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p zx-spectrum`
Expected: FAIL to compile — same shape of errors as Task 4 Step 2.

- [ ] **Step 3: Implement the region field, constructor, and trait impl**

In `crates/zx-spectrum/src/lib.rs`, change:
```rust
use machine::{Machine, MemoryBus};
use z80::Cpu;
```
to:
```rust
use machine::{Clock, Machine, MemoryBus, Region, RunStats};
use z80::Cpu;
```

Change the struct and constructor:
```rust
pub struct ZxSpectrum {
    cpu: Cpu,
    bus: MemoryBus,
    region: Region,
}

impl ZxSpectrum {
    /// `rom` is the real 48K Spectrum ROM image, supplied externally -- it's
    /// copyrighted and not bundled with this repo. `region` selects the
    /// market variant's clock/frame rate (see `Machine::clock_hz`/
    /// `frame_hz` below).
    pub fn new(rom: Vec<u8>, region: Region) -> Self {
        let mut bus = MemoryBus::new();
        bus.map_rom(0x0000, rom);
        bus.map_ram(RAM_BASE, RAM_SIZE);
        Self {
            cpu: Cpu::new(),
            bus,
            region,
        }
    }
}
```

Change the `Machine` impl's `clock_hz` and add `frame_hz`/`run`:
```rust
impl Machine for ZxSpectrum {
    fn clock_hz(&self) -> u64 {
        match self.region {
            Region::Pal => 3_546_900,
            // Placeholder: no real Spectrum 48K variant shipped for these
            // markets -- kept equal to PAL until/unless one needs modeling.
            Region::Ntsc | Region::Japan => 3_546_900,
        }
    }

    fn frame_hz(&self) -> u64 {
        match self.region {
            Region::Pal => 50,
            Region::Ntsc | Region::Japan => 60,
        }
    }

    fn run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> RunStats {
        Clock::new(self.frame_hz()).run(self, running)
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

    fn load(&mut self, addr: u16, bytes: &[u8]) -> usize {
        self.bus.load(addr, bytes)
    }

    fn set_pc(&mut self, addr: u16) {
        self.cpu.pc = addr;
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p zx-spectrum`
Expected: PASS — 2 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/zx-spectrum/src/lib.rs
git commit -m "feat(zx-spectrum): region-aware clock/frame rate, own run()"
```

---

### Task 6: `emu` — `--region` flag, drop `Clock`, call `machine.run(...)` directly

**Files:**
- Modify: `crates/emu/src/main.rs`

**Interfaces:**
- Consumes: `machine::{Machine, Region}` (Tasks 1-3), `zx81::Zx81` (Task 4), `zx_spectrum::ZxSpectrum` (Task 5) — both now taking `(rom, Region)`.
- Produces: `emu` binary CLI: `emu <path-to-program> --machine <zx81|zx-spectrum> [--region <pal|ntsc|japan>] [--org 0xNNNN] [--rom <path>] [--dump-memory 0xSTART:LEN]`. Default region is `pal`.

This task has no automated test (it's a binary's CLI, same as before) — verified by manual smoke test, matching how `emu` has been verified throughout this project.

- [ ] **Step 1: Add `--region` to argument parsing**

In `crates/emu/src/main.rs`, change the import:
```rust
use machine::{Clock, Machine};
```
to:
```rust
use machine::{Machine, Region};
```

Add a field to `Args`:
```rust
struct Args {
    program_path: String,
    org: u16,
    rom_path: Option<String>,
    machine: MachineKind,
    region: Region,
    dump_memory: Option<(u16, u16)>,
}
```

In `parse_args`, add a local and a match arm, and thread it into the returned `Args`:
```rust
fn parse_args() -> Result<Args, String> {
    let mut program_path = None;
    let mut org = 0x4000u16;
    let mut rom_path = None;
    let mut machine = None;
    let mut region = Region::Pal;
    let mut dump_memory = None;

    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--org" => {
                let v = it.next().ok_or("--org needs a value")?;
                org = u16::from_str_radix(v.trim_start_matches("0x"), 16)
                    .map_err(|e| e.to_string())?;
            }
            "--rom" => {
                rom_path = Some(it.next().ok_or("--rom needs a value")?);
            }
            "--machine" => {
                let v = it.next().ok_or("--machine needs a value")?;
                machine = Some(match v.as_str() {
                    "zx81" => MachineKind::Zx81,
                    "zx-spectrum" => MachineKind::ZxSpectrum,
                    other => {
                        return Err(format!(
                            "unknown machine: {other} (expected zx81 or zx-spectrum)"
                        ));
                    }
                });
            }
            "--region" => {
                let v = it.next().ok_or("--region needs a value")?;
                region = match v.as_str() {
                    "pal" => Region::Pal,
                    "ntsc" => Region::Ntsc,
                    "japan" => Region::Japan,
                    other => {
                        return Err(format!(
                            "unknown region: {other} (expected pal, ntsc, or japan)"
                        ));
                    }
                };
            }
            "--dump-memory" => {
                let v = it
                    .next()
                    .ok_or("--dump-memory needs a value (e.g. 0x4000:16)")?;
                let (start, len) = v.split_once(':').ok_or("--dump-memory expects START:LEN")?;
                let start = u16::from_str_radix(start.trim_start_matches("0x"), 16)
                    .map_err(|e| e.to_string())?;
                let len: u16 = len
                    .parse()
                    .map_err(|e: std::num::ParseIntError| e.to_string())?;
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
        region,
        dump_memory,
    })
}
```

- [ ] **Step 2: Thread `Region` through `build_machine`, drop `Clock`**

Change:
```rust
fn build_machine(kind: MachineKind, rom: Vec<u8>) -> Box<dyn Machine> {
    match kind {
        MachineKind::Zx81 => Box::new(zx81::Zx81::new(rom)),
        MachineKind::ZxSpectrum => Box::new(zx_spectrum::ZxSpectrum::new(rom)),
    }
}
```
to:
```rust
fn build_machine(kind: MachineKind, rom: Vec<u8>, region: Region) -> Box<dyn Machine> {
    match kind {
        MachineKind::Zx81 => Box::new(zx81::Zx81::new(rom, region)),
        MachineKind::ZxSpectrum => Box::new(zx_spectrum::ZxSpectrum::new(rom, region)),
    }
}
```

In `main`, change the call site and the run call:
```rust
    let mut machine = build_machine(args.machine, rom);
```
to:
```rust
    let mut machine = build_machine(args.machine, rom, args.region);
```
and change:
```rust
    let clock = Clock::new();
    let stats = clock.run(machine.as_mut(), |m| !m.halted());
```
to:
```rust
    let stats = machine.run(&mut |m| !m.halted());
```

- [ ] **Step 3: Update the usage strings**

Both `eprintln!` usage lines change from:
```
"usage: emu <path-to-program> --machine <zx81|zx-spectrum> [--org 0xNNNN] [--rom <path>] [--dump-memory 0xSTART:LEN]"
```
to:
```
"usage: emu <path-to-program> --machine <zx81|zx-spectrum> [--region <pal|ntsc|japan>] [--org 0xNNNN] [--rom <path>] [--dump-memory 0xSTART:LEN]"
```
(there is exactly one such line, inside `parse_args`'s error branch in `main`).

- [ ] **Step 4: Build the whole workspace**

Run: `cargo build --workspace`
Expected: builds cleanly, no warnings.

- [ ] **Step 5: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS — z80 (184), machine (13), zx81 (2), zx-spectrum (2) all green.

- [ ] **Step 6: Manual smoke test — default region (PAL)**

```bash
printf '\x3e\x2a\x76' > /tmp/tiny.bin
cargo run -p emu -- /tmp/tiny.bin --machine zx81
```
Expected output includes `halted`, `2 instructions, 11 T-states`, and a `registers:` block whose first line is `AF   = 0x2A00` — same as before this plan (default behavior unchanged).

- [ ] **Step 7: Manual smoke test — `--region ntsc`**

```bash
cargo run -p emu -- /tmp/tiny.bin --machine zx81 --region ntsc
```
Expected: same `halted`/register output shape as Step 6 (this tiny 2-instruction program finishes well inside one tick regardless of region), confirming the flag is accepted and threaded through without error.

- [ ] **Step 8: Manual smoke test — unknown region is rejected**

```bash
cargo run -p emu -- /tmp/tiny.bin --machine zx81 --region pal-b
```
Expected: `error: unknown region: pal-b (expected pal, ntsc, or japan)` on stderr, exit code non-zero, usage string printed.

- [ ] **Step 9: Commit**

```bash
git add crates/emu/src/main.rs
git commit -m "feat(emu): add --region flag; call machine.run() directly, drop Clock"
```

---

## Post-plan state

- `machine::Region` is the shared vocabulary for market variants; any future machine crate takes one at construction.
- `Clock` is a reusable pacing helper (rate supplied by the caller) rather than an externally-owned driver — every `Machine` impl calls it from its own required `run()`.
- Adding a machine with a fundamentally different timing model (e.g. ZX81 video/NMI generation, a future non-Z80 system) means replacing that one machine's `run()` body; the trait, `Clock`, and every other machine are unaffected.
- `emu` (and any future UI) only ever calls `machine.run(...)` — it no longer knows `Clock` exists.
