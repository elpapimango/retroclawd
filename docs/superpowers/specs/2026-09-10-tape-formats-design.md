# `.tap`/`.tzx` tape format support -- design

## Problem

`retroclawd` can load a memory snapshot (`.z80`, `.p`/`.p81`) but has no way
to load a real ZX Spectrum cassette image (`.tap`, `.tzx`) -- the format
most real commercial software actually shipped on. Unlike a snapshot,
tape data isn't a point-in-time memory dump: real hardware loads it by
the ROM's own routine bit-banging through the EAR input line (port
`0xFE` bit 6), reading pulse timing off a physical cassette in real time.
`ZxSpectrumBus::read_io` doesn't model this bit at all today -- it always
reads high (matching "no cassette present"), consistent by coincidence
with no tape support existing yet, not a deliberate EAR-bit
implementation.

## Goals

- Load real `.tap` and `.tzx` files through the ROM's own real tape-loading
  routine, at real hardware pulse timing by default -- consistent with
  this project's hardware-accuracy stance elsewhere (NMI timing, interrupt
  timing, audio generation).
- A selectable turbo mode: the *same* real pulse-level protocol, sped up by
  an integer multiplier, not a different (ROM-routine-trapping)
  mechanism -- avoids per-ROM-version fragility and keeps the ROM's own
  loading-screen behavior (border stripes, "LOADING" caption) working
  correctly regardless of speed.
- Tape starts playing automatically the moment the ROM first reads the EAR
  bit; stops automatically at end-of-file (or at a `.tzx` "stop the tape"
  control block -- see below).
- Generic across every `Machine`/`Model` that has an EAR input in the same
  place (48K/128K/+2A/+3 all share the same port `0xFE` bit 6) -- no
  per-model special-casing beyond what already exists for the beeper/AY.

## Non-goals

- Tape **saving** (writing pulses out via the MIC output, port `0xFE` bit
  3) is out of scope -- same "load-only for now, `Snapshot`'s shape stays
  save-friendly for later" precedent as `z80-snapshot`/`zx81-snapshot`.
- Full `.tzx` block-type coverage. `.tzx` has 20+ block IDs; v1 implements
  the two that carry real pulse data for the vast majority of commercial
  releases (standard speed data blocks, turbo speed data blocks) plus the
  pause/stop-tape control block. Every other known block ID is skipped
  safely (using its own documented length prefix to know how many bytes to
  skip) since it carries no loadable pulse data (text descriptions,
  archive info, group markers, etc.). A block ID that *would* carry pulse
  data but isn't implemented is a real, clear parse error -- silently
  skipping it would corrupt loading, not just omit metadata.
- ZX81 tape support -- ZX81 stays parked per existing priority; this is
  Spectrum-only.
- The +3's floppy disk controller -- separate, already-tracked follow-on
  work, unrelated to tape.

## Architecture

### New crate `crates/zx-tape`

No dependency on `machine`, any CPU crate, or `zx-spectrum` -- same
reasoning as `z80-snapshot`/`zx81-snapshot`: parsing a tape file into a
pulse sequence is a fact about the file format, not about how a machine
consumes it. The standard pulse-timing constants baked into the `.tap`
synthesis below are inherently ZX-Spectrum-specific (that's what the
format means), same epistemic footing as `z80-snapshot` encoding
Spectrum-specific paging semantics while still living independent of the
`zx-spectrum` crate.

```rust
pub struct Pulse {
    pub cycles: u32, // T-states this pulse lasts
    pub level: bool, // EAR level during this pulse
}

pub struct Tape {
    pulses: Vec<Pulse>,
}

pub fn parse_tap(bytes: &[u8]) -> Result<Tape, Error>;
pub fn parse_tzx(bytes: &[u8]) -> Result<Tape, Error>;

pub struct TapePlayer {
    // owns a Tape, current pulse index, progress within that pulse,
    // playing/stopped state, and the turbo speed multiplier
}

impl TapePlayer {
    pub fn new(tape: Tape) -> Self;
    pub fn set_speed_multiplier(&mut self, n: u32); // 1 = real time
    pub fn play(&mut self);
    pub fn tick(&mut self, cycles: u32);
    pub fn ear_bit(&self) -> bool;
    pub fn finished(&self) -> bool;
}
```

### `.tap` -> pulse synthesis

`.tap` has no pulse timing at all -- each block (`[len_lo, len_hi, data...]`,
where `data` is `[flag_byte, ...payload, checksum]`) is defined to always
be played back through the ROM's own standard loading protocol. Verified
against the TZX format specification (see `CREDITS.md`) -- the same
constants a `.tzx` "standard speed data block" (ID `0x10`) uses, since
`.tap` is effectively that one block type with the pulse timing implied
rather than stated:

- Pilot tone: 2168 T-states/pulse, alternating level -- 8063 pulses if the
  block's flag byte is < 128 (a header block), 3223 if >= 128 (a data
  block).
- Sync: one 667 T-state pulse, one 735 T-state pulse.
- Each data bit (MSB first, across the flag byte + payload + checksum
  together): two pulses of 855 T-states for a `0` bit, two of 1710
  T-states for a `1` bit.
- A pause (idle level) before the next block -- exact default duration to
  confirm against the spec during implementation, not asserted here.

### `.tzx` block handling

- ID `0x10` (Standard Speed Data Block): same synthesis as `.tap`, with an
  explicit pause value read from the block itself instead of a default.
- ID `0x11` (Turbo Speed Data Block): same idea, but every timing
  parameter (pilot pulse length, pilot count, sync lengths, bit-0/bit-1
  lengths, used bits in the last byte, pause) is read from the block
  itself rather than the standard constants -- this is what most
  commercial turbo loaders used, so it's the second block type v1 must
  support, not optional polish.
- ID `0x20` (Pause/Stop the tape): a duration of `0` is the format's own
  "stop the tape" signal (used by multi-load games that need the player to
  press Play again). Maps naturally onto the already-chosen "auto-play on
  next EAR read" design -- `TapePlayer` just goes back to not-playing, and
  the next `read_io` that touches bit 6 restarts it, no separate manual
  control needed.
- Every other known block ID: skipped using its own documented length
  prefix. An unrecognized block ID with no way to determine its length is
  a parse error, not a silent skip.

### Bus/machine integration (`crates/zx-spectrum`)

- `ZxSpectrumBus` gains `tape: Option<zx_tape::TapePlayer>` (mirrors
  `ay: Option<Ay3_8910>`'s shape) plus `tape_mut()`/accessors for
  `ZxSpectrum` to install one.
- `ZxSpectrum::step()` calls `bus.tick_tape(cycles)` right after stepping
  the CPU (same pattern as `ay.tick(ay_cycles)`, but no clock-divider --
  tape pulses are already defined in real T-states, 1:1 with CPU cycles).
- `read_io`'s port `0xFE` handling composes bit 6 from
  `tape.ear_bit()` when a tape is installed (bits 5/7 stay hardcoded 1,
  bits 0-4 stay the keyboard read -- verified layout, see `CREDITS.md`);
  the first such read triggers `TapePlayer::play()` if not already
  playing.
- Model-agnostic: `Model::Spectrum48`/`Spectrum128`/`Spectrum2A3` all share
  the same port `0xFE` EAR bit, so this needs no per-model branching
  beyond what already exists.

### CLI (`apps/retroclawd`)

- `--tape <path>` -- new flag, separate from `--snapshot` (a snapshot
  replaces machine state instantly; a tape is installed and plays back
  over real time -- different enough semantics to not overload one flag).
  Format (`.tap` vs `.tzx`) picked by extension, same convention as
  `--snapshot`.
- `--tape-speed <N>` -- turbo multiplier, default `1` (real time).

## Testing approach

- `zx-tape`'s pulse synthesis: unit-tested directly against known-good
  pulse counts/durations for a small synthetic `.tap` block (e.g. a 1-byte
  data block), asserting the exact pilot/sync/bit pulse sequence the
  verified constants above predict -- not an end-to-end "does a real game
  load" test, which belongs at a higher level once real test files are on
  hand.
- `TapePlayer::tick`/`ear_bit`: unit-tested the same way `ay-3-8910`'s
  `tick`/`output` are -- feed known cycle counts, assert the resulting EAR
  level matches the expected pulse at that point in the sequence, including
  the turbo-speed-multiplier case.
- Bus integration: a `read_io` test mirroring the existing AY/keyboard
  tests -- install a tape, confirm the first bit-6 read starts playback and
  bit 6 changes value as cycles advance.
