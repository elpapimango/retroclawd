# Machine-owned timing: per-machine `run()`, region-aware clock/frame rate

Date: 2026-09-05
Status: Approved, pending implementation plan

## Problem

Today `machine::Clock` owns the entire run loop for every machine: it's
constructed by the caller (`emu`) with a hardcoded 60 ticks/sec pacing rate,
and drives `Machine::step()` in fixed T-state budgets regardless of which
machine it's driving.

This doesn't scale to real hardware variation:

- Different machines have genuinely different execution/timing models. The
  ZX81 in particular has no ULA — video comes from the CPU itself executing
  the display file during HSYNC/NMI, which couples video timing to CPU
  execution in a way a generic external budget-loop can't express. (Not
  implemented this round — see Non-goals — but the trait must not need to
  change again when it is.)
- Real machines shipped in region variants with different crystal
  frequencies and refresh rates: PAL (Europe, 50Hz), NTSC (US, 60Hz), and
  Japan (60Hz, sometimes a distinct crystal from US). A single hardcoded
  `clock_hz()` and a single hardcoded 60Hz pacing rate can't represent this.

## Non-goals (this round)

- ZX81 video/NMI generation. This round only changes who owns the timing
  *loop* and how region affects clock/frame rate — it does not implement
  any actual per-frame or per-scanline hook. `Zx81`/`ZxSpectrum` keep using
  the default `run()` behavior (delegating to `Clock`) after this round.
- Real NTSC ZX81/Timex Sinclair 1000 crystal value research. `Region::Ntsc`
  and `Region::Japan` use placeholder Hz values (see Open questions) until
  a reference is checked.
- Any UI. Still CLI-only, `emu` text output.

## The `Region` enum (new, in `machine`)

```rust
/// Broadcast/market variant a machine was built for. Affects both CPU clock
/// speed (crystal differs per market) and refresh rate (frame_hz) -- each
/// machine impl maps this to its own concrete Hz values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Pal,
    Ntsc,
    Japan,
}
```

`Japan` is kept distinct from `Ntsc` even though both are 60Hz, since some
machines (not ZX81/Spectrum specifically) used a genuinely different
crystal for the Japanese market — the enum shouldn't need to change if a
future machine crate needs that distinction.

## `Machine` trait changes

Add `frame_hz()` and a default `run()`:

```rust
pub trait Machine {
    fn clock_hz(&self) -> u64;
    fn frame_hz(&self) -> u64; // NEW -- pacing/refresh rate: 50 (Pal), 60 (Ntsc/Japan)

    fn run(&mut self, running: impl FnMut(&dyn Machine) -> bool) -> RunStats
    where
        Self: Sized,
    {
        Clock::new(self.frame_hz()).run(self, running)
    }

    fn step(&mut self) -> u32;
    fn halted(&self) -> bool;
    fn registers(&self) -> Vec<(&'static str, u64)>;
    fn read_memory(&self, addr: u16) -> u8;
    fn load(&mut self, addr: u16, bytes: &[u8]) -> usize;
    fn set_pc(&mut self, addr: u16);
}
```

A machine gets correct region-aware pacing for free by implementing
`frame_hz()` — no loop code duplicated. A future machine with a
fundamentally different execution model (ZX81 video/NMI) overrides `run()`
without the trait needing another change.

**Open implementation question:** `run()`'s default body takes `self` by
value into `Clock::run(self, ...)`, which requires `Self: Sized`. `emu`
holds `Box<dyn Machine>` and needs to call this through the trait object.
If default-method dispatch on `dyn Machine` doesn't compile as written, the
plan should fall back to a free function `machine::run(m: &mut dyn Machine,
running) -> RunStats { Clock::new(m.frame_hz()).run(m, running) }` with
identical behavior — `emu` would call `machine::run(machine.as_mut(),
running)` instead of `machine.run(running)`. Either shape satisfies this
spec; the plan should verify which compiles and use it.

## `Clock` changes

Drop the hardcoded rate; take it as a constructor argument:

```rust
impl Clock {
    pub fn new(ticks_per_sec: u64) -> Self {
        Self { ticks_per_sec }
    }
    // run() body unchanged -- already generic over self.ticks_per_sec
}
```

`impl Default for Clock` is removed — there's no longer a sensible default
rate independent of a machine's `frame_hz()`. The existing `.max(1)` guard
on `t_states_per_tick` in `run()` is unchanged (still needed for a
low-`clock_hz` machine).

## `Region` in `zx81` / `zx-spectrum`

Both structs gain a `region: Region` field; constructors become
`new(rom: Vec<u8>, region: Region)`.

```rust
impl Machine for Zx81 {
    fn clock_hz(&self) -> u64 {
        match self.region {
            Region::Pal => 3_250_000,
            Region::Ntsc | Region::Japan => 3_250_000, // placeholder, see Open questions
        }
    }
    fn frame_hz(&self) -> u64 {
        match self.region {
            Region::Pal => 50,
            Region::Ntsc | Region::Japan => 60,
        }
    }
    // ... unchanged
}
```

`ZxSpectrum` takes `Region` too, for constructor-shape consistency across
machine crates, even though only PAL is historically real for it —
`Ntsc`/`Japan` arms return the same numbers as `Pal` as a placeholder (no
real Spectrum variant existed for them; a future correction is cheap since
nothing depends on the placeholder being accurate yet).

## `emu` changes

- New `--region <pal|ntsc|japan>` flag, default `pal` (matches today's
  implicit-only-PAL behavior). Hand-rolled `&str` match, consistent with
  how `--machine` is already parsed.
- `build_machine` passes the parsed `Region` through to `Zx81::new`/
  `ZxSpectrum::new`.
- `emu` no longer constructs a `Clock` itself. The run call becomes
  `machine.run(running)` (or `machine::run(machine.as_mut(), running)` —
  see the trait's open question) instead of `clock.run(machine.as_mut(),
  running)`.

## Testing

- `machine`: existing `Clock` tests (`FakeMachine`) add a `frame_hz()`
  stub and switch from `Clock::new()` to `Clock::new(60)` (or whatever rate
  the test wants). Add one new test asserting `Clock::new(50)` produces a
  different tick duration / budget than `Clock::new(60)` for the same
  `hz`, confirming the rate is actually threaded through rather than
  ignored.
- `zx81` / `zx-spectrum`: existing smoke test switches to
  `Zx81::new(vec![...], Region::Pal)`. Add one cheap unit test per crate
  asserting `frame_hz()` returns 50 for `Region::Pal` and 60 for
  `Region::Ntsc` (no CPU run needed).
- `emu`: manual smoke test — both machines with no `--region` (unchanged
  behavior), plus one run with `--region ntsc` confirming it doesn't error.

## Open questions

- Real NTSC ZX81 / Timex Sinclair 1000 crystal frequency is not yet
  verified against a hardware reference — `Region::Ntsc`/`Region::Japan`
  use the same `clock_hz()` as `Region::Pal` as a placeholder for `zx81`.
  Should be corrected once a reference value is checked, independent of
  this round's trait/timing-ownership work.
