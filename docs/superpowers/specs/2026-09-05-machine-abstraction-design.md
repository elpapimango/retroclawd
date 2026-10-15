# Machine abstraction: per-system crates, movable clock, CPU-agnostic introspection

Date: 2026-09-05
Status: Approved, pending implementation plan

## Problem

The current `emu` crate hardcodes a single generic `Machine` struct with a
CLI-supplied `--hz` clock and a flat/ROM+RAM memory layout. This doesn't scale
to adding real systems (ZX81, ZX Spectrum, and later ABC80, Amstrad CPC),
each of which has:

- a different, fixed clock rate (ZX81 ~3.25MHz, Spectrum 48K ~3.5469MHz),
- a different ROM/RAM memory map,
- (later) different peripherals (ULA/video, keyboard matrix, tape).

We also want a way to inspect a running machine's CPU registers and memory
contents. That inspection must work the same way regardless of which CPU
chip a given machine uses, since we intend to add non-Z80 cores later (e.g.
ABC80 is 6502-based).

No UI in this round — text/CLI output only.

## Non-goals (this round)

- Real ZX81/Spectrum peripherals: ULA/video timing, keyboard matrix, tape
  loading, sound. The two new machine crates are skeletons: correct clock
  rate and memory map, driven purely by the existing `z80` core, loading an
  externally supplied ROM image.
- ABC80 and Amstrad CPC crates. Mentioned only as evidence that clock rate
  must live per-machine, not hardcoded once.
- Any graphical or interactive UI. Introspection is CLI text output.
- A live/interactive debugger (breakpoints, step-by-step REPL). Only a
  post-halt (or on-demand) dump of registers and a memory range.
- Runtime machine plugin discovery. Two machines is a static `match` on a
  CLI flag, not a registry.

## Crate layout

```
crates/
  z80/          # CPU core -- unchanged
  machine/      # NEW: Machine trait, Clock, MemoryBus (generic, CPU-agnostic)
  zx81/         # NEW: Machine impl -- z80 core, 3.25MHz, 8K ROM / 1K RAM skeleton
  zx-spectrum/  # NEW: Machine impl -- z80 core, 3.5469MHz, 16K ROM / 48K RAM skeleton
  emu/          # runner: --machine <name>, --rom, --org, register/memory dump
```

`machine` becomes a new workspace member depended on by `zx81`, `zx-spectrum`,
and `emu`. `zx81` and `zx-spectrum` depend on `z80` and `machine`. `emu`
depends on `machine`, `zx81`, and `zx-spectrum` (to construct the chosen one),
but talks to whichever is selected only through the `Machine` trait.

## The `Machine` trait

Lives in `crates/machine`. Every concrete system implements it; `emu` (and
any future UI) only ever holds a `Box<dyn Machine>`.

```rust
pub trait Machine {
    /// Fixed clock rate for this system, in Hz. Read by Clock -- not a CLI
    /// override -- so a ZX81 always runs at ZX81 speed and a Spectrum
    /// always at Spectrum speed.
    fn clock_hz(&self) -> u64;

    /// Executes one CPU instruction. Returns its cost in T-states.
    fn step(&mut self) -> u32;

    /// True once the CPU has halted.
    fn halted(&self) -> bool;

    /// Ordered register name/value pairs, e.g. [("AF", 0x2A00), ("BC", 0), ...].
    /// Order and register set are chip-specific but the shape is uniform,
    /// so a future UI can render any machine's registers as a table without
    /// knowing which CPU is inside.
    fn registers(&self) -> Vec<(&'static str, u64)>;

    /// Pure read of one memory byte for display purposes. Does not go
    /// through the CPU's normal bus read path (which is `&mut self` and may
    /// have side effects for I/O-mapped reads) -- this is a passive peek.
    fn read_memory(&self, addr: u16) -> u8;

    /// Loads bytes (a ROM image or a program) starting at `addr`, through
    /// the machine's normal memory-write semantics (so writes to a ROM
    /// region are dropped, matching real hardware).
    fn load(&mut self, addr: u16, bytes: &[u8]);
}
```

`registers()` uses `u64` as a common width so any future CPU (8-bit, 16-bit,
wider) can report its registers without the trait needing per-CPU variants.

## `Clock` (moves to `crates/machine`)

Same pacing logic as today (fixed-size T-state budget per 1/60s tick, sleep
off whatever's left), but reworked to depend only on the trait:

```rust
pub struct Clock { ticks_per_sec: u64 }

impl Clock {
    pub fn new() -> Self { Self { ticks_per_sec: 60 } }

    pub fn run(&self, machine: &mut dyn Machine, mut running: impl FnMut(&dyn Machine) -> bool) -> RunStats {
        let hz = machine.clock_hz();
        // ... same budget/sleep loop as today, calling machine.step() ...
    }
}
```

This is the actual fix for "clock should be possible to change to suit
various machines": the rate lives on the `Machine` impl, not passed in from
the CLI. `emu` no longer takes a `--hz` flag.

## `MemoryBus` (moves to `crates/machine`, unchanged behavior + one addition)

Same region-mapped ROM/RAM implementation as already built (unmapped reads
return `0xFF`, writes to ROM or unmapped space are dropped). Adds:

```rust
impl MemoryBus {
    /// Read-only peek, no `&mut self` needed -- used by Machine::read_memory
    /// impls for the debug/introspection path.
    pub fn peek(&self, addr: u16) -> u8 { ... }
}
```

`Bus::read`/`Bus::write` (the trait the `z80` core drives) stay `&mut self`
as today, since a real bus read can have side effects (e.g. I/O port reads).
`peek` is a separate, deliberately inert accessor only for display.

## `zx81` and `zx-spectrum` crates

Each is a thin struct:

```rust
pub struct Zx81 { cpu: Cpu, bus: MemoryBus }

impl Zx81 {
    pub fn new() -> Self {
        let mut bus = MemoryBus::new();
        bus.map_rom(0x0000, vec![0; 0x2000]);  // 8K ROM, placeholder until --rom loads real image
        bus.map_ram(0x4000, 0x400);            // 1K RAM (unexpanded ZX81)
        Self { cpu: Cpu::new(), bus }
    }
}

impl Machine for Zx81 {
    fn clock_hz(&self) -> u64 { 3_250_000 }
    fn step(&mut self) -> u32 { self.cpu.step(&mut self.bus) as u32 }
    fn halted(&self) -> bool { self.cpu.halted }
    fn registers(&self) -> Vec<(&'static str, u64)> { /* AF, BC, DE, HL, AF', BC', DE', HL', IX, IY, SP, PC, I, R, IM */ }
    fn read_memory(&self, addr: u16) -> u8 { self.bus.peek(addr) }
    fn load(&mut self, addr: u16, bytes: &[u8]) { self.bus.load(addr, bytes); }
}
```

`ZxSpectrum` is the same shape: `clock_hz() = 3_546_900`, ROM 16K at
`0x0000`, RAM 48K at `0x4000`.

Both take their ROM image from disk via the CLI's `--rom` flag (same
mechanism `emu` already has) rather than embedding a copyrighted ROM binary
in the repo.

## `emu` changes

- New `--machine <zx81|zx-spectrum>` flag selects which concrete `Machine`
  to construct (a static `match`, not a registry).
- Drops `--hz` (superseded by `Machine::clock_hz()`).
- Keeps `--org` (load address for the program argument) and `--rom` (ROM
  image path).
- After halt, prints:
  - the register table from `Machine::registers()` (generic, works for any
    future CPU),
  - optionally a memory range via a new `--dump-memory START:LEN` flag
    (hex bytes), using `Machine::read_memory()`.

## Testing

- `machine` crate: unit tests for `MemoryBus` move over unchanged (already
  passing); add a `peek` test (read-only, doesn't require `&mut`).
- `zx81`/`zx-spectrum`: a smoke test per crate -- construct, load a tiny
  hand-built program (like the existing `LD A,0x2A / HALT` used earlier in
  `emu`), run to halt via `Clock`, assert `registers()` reflects it.
- `emu`: manual smoke test only (already how the CLI has been verified so
  far in this project) -- run each machine with a tiny program, confirm
  register table prints and `--dump-memory` prints the right bytes.

## Migration notes

- `crates/emu/src/machine.rs` and `crates/emu/src/memory.rs` move to
  `crates/machine/src/` (mostly cut-paste + trait-ify the existing
  `Machine` struct into the new trait).
- `crates/emu/src/clock.rs` moves to `crates/machine/src/clock.rs`,
  reworked to take `&mut dyn Machine` instead of `&mut Cpu, &mut dyn Bus`.
- The old generic RAM/ROM `Machine` struct in `emu` is deleted -- its
  behavior is superseded by `zx81`/`zx-spectrum` (whichever is picked via
  `--machine`).
