# NEC765/uPD765A floppy disk controller + `.dsk` support -- design

## Problem

`Model::Spectrum2A3`'s memory model is done, but the +3's actual floppy
drive is unmodeled -- `Paging2A3::disk_motor` (`0x1ffd` bit 3) is stored
and never read, and ports `0x2ffd`/`0x3ffd` (the FDC's main status
register and data register) don't exist on the bus at all. No +3 software
that loads from disk (the vast majority of real +3 releases -- the +3 was
sold with a disk drive, not a tape deck) can load. Explicitly requested by
the user as "full +3 including the floppy controller" (see `TODO.md`), not
deferred indefinitely -- sequenced as its own spec since it's a
genuinely new, bigger undertaking than the +2A/+3 memory model was.

## Goals

- Real NEC765/uPD765A command protocol on ports `0x2ffd`/`0x3ffd` --
  Command/Execution/Result phase structure, MSR bit semantics, byte-at-a-
  time host handshake -- matching how the real ROM's disk routines
  actually talk to the chip, not a shortcut API.
- Load a standard CPCEMU `.dsk` image and read real sector data off it
  through that protocol.
- `Model::Spectrum2A3` only -- the FDC is part of this model's hardware
  regardless of whether a disk is inserted (`+2A` just never gets one in
  practice; this project doesn't model the `+2A`/`+3` case split, see
  `zx-spectrum`'s existing `Model` notes).

## Non-goals (explicit, approved scope)

- **Write/format commands** -- Write Data, Format Track, and every other
  NEC765 command outside the read-only set below are out of scope. An
  unimplemented command byte gets the chip's own real "invalid command"
  result-phase response (status byte `0x80`), not a panic or silent
  no-op -- matches real hardware's behavior for a command it doesn't
  recognize, not a guess.
- **Real seek/rotational timing** -- Execution phase completes instantly
  (same tick) rather than modeling head-seek time, disk rotation latency,
  or DMA transfer rate. The protocol shape (phases, MSR polling, byte
  counts) is real; the *duration* of Execution isn't.
- Extended DSK (variable-size tracks, weak sectors) -- standard CPCEMU
  `.dsk` only. Covers the overwhelming majority of +3 software.
- Multiple disk formats (`.trd`, `.scl`, etc.) -- `.dsk` only, same
  "one format crate per real file format" precedent as `zx-tape`/
  `z80-snapshot`.
- Disk swapping mid-session, write-back saving -- load-only, same
  precedent as every other format crate here.

## Architecture

### New crate `crates/nec765`

Chip emulation only -- zero path dependencies (same reasoning as
`ay-3-8910`: this is a real chip's own well-documented internal protocol,
usable by any host machine that wires up the right ports, not something
that should know `zx-spectrum` or `.dsk` exist).

```rust
/// What the chip needs from a disk to serve Read Data / Read ID --
/// implemented by `dsk::Disk` in `zx-spectrum`'s bus wiring, but this
/// crate has no idea `dsk` exists. Sector data only (no write path) per
/// this spec's approved scope.
/// A sector's real NEC765 ID field (C/H/R/N) -- `size_code` (N) is the
/// chip's own `128 << N` sector-size encoding, matching the `.dsk` file
/// format's own per-sector size byte 1:1 (see `dsk` below), so no
/// conversion is needed either direction.
pub struct SectorId {
    pub cylinder: u8,
    pub head: u8,
    pub sector: u8,
    pub size_code: u8,
}

pub trait DiskMedium {
    /// The first sector this medium has for `(cylinder, head)` -- Read
    /// ID's real semantics (no rotational position tracked, see the
    /// command table's Read ID row).
    fn first_sector(&self, cylinder: u8, head: u8) -> Option<(SectorId, &[u8])>;
    /// One specific sector by its real ID fields -- Read Data's own
    /// params name exactly these.
    fn read_sector(&self, cylinder: u8, head: u8, sector: u8) -> Option<(SectorId, &[u8])>;
}

pub struct Fdc<D: DiskMedium> {
    disk: Option<D>,
    /// Real cylinder register the head is "positioned" over -- Seek/
    /// Recalibrate set it instantly (no real seek time modeled, per
    /// approved scope); Sense Interrupt Status reads it back as PCN.
    cylinder: u8,
    // phase state: Idle | Command{opcode, params_needed, params: Vec<u8>}
    //            | Result{bytes: Vec<u8>, pos: usize} -- no separate
    // Execution variant: Execution is instant (approved scope), so
    // write_data computes straight through Command -> Result at the last
    // param byte, same-call.
}

impl<D: DiskMedium> Fdc<D> {
    pub fn new() -> Self;
    pub fn insert_disk(&mut self, disk: D);
    pub fn read_msr(&self) -> u8;       // port 0x2ffd
    pub fn read_data(&mut self) -> u8;  // port 0x3ffd read
    pub fn write_data(&mut self, val: u8); // port 0x3ffd write
}
```

Generic over `D: DiskMedium` rather than boxing/trait-object -- one
concrete disk type per machine instance, no need for dynamic dispatch,
and keeps the crate testable with a trivial in-memory fake `DiskMedium`
without pulling in `dsk` as a dev-dependency.

### Phase state machine

Per the real uPD765A datasheet (see `CREDITS.md` once added): every
command goes through up to 3 phases.

1. **Command** -- host writes the command byte, then however many
   parameter bytes that command takes, one `write_data` call each. The
   chip only knows how many params to expect once it's seen the command
   byte (each command's own fixed count, see table below).
2. **Execution** -- the chip does the work. Instant here (approved
   scope): the moment the last param byte lands, results are computed
   immediately and the chip moves straight to Result phase.
3. **Result** -- host reads result bytes one `read_data` call each, same
   fixed count per command.

`read_msr` (`0x2ffd`, read-only) exposes handshake state the ROM polls
before every `read_data`/`write_data`:

- Bit 7 (RQM, Request for Master) -- `1` = chip ready for the next byte
  transfer. Always `1` here (instant execution -- no phase where the
  host must wait for the chip).
- Bit 6 (DIO, Data Input/Output) -- `1` = next transfer is chip-to-host
  (Result phase read), `0` = host-to-chip (Command phase write).
- Bits 0-3 (drive busy flags) -- always `0`; this project doesn't model
  a busy/seeking drive (approved scope).

Port decode: `0x2ffd`/`0x3ffd` assumed `port & 0xF002 == 0x2000` /
`== 0x3000` respectively, by analogy with `0x1ffd`'s already-verified
`0xF002`-family mask -- **not independently verified** against the +3
technical manual or a hardware test suite the way `0x1ffd`/`0x7ffd` were;
flag this clearly in the implementing code's own doc comment, same
epistemic-honesty convention as everywhere else in this codebase.

### Command set (read-only scope, verified against the real uPD765
datasheet's own command-phase/result-phase byte tables, not recollection
-- see `CREDITS.md` once this lands)

| Command | Byte | Params | Results | Notes |
|---|---|---|---|---|
| Specify | `0x03` | 2 (SRT/HLT, HUT/ND) | 0 | Step rate/head timings -- stored nowhere, unused (no seek timing modeled) |
| Recalibrate | `0x07` | 1 (US1/US0 drive select) | 0 | Seeks to cylinder 0 -- instant; internal cylinder register set to `0` |
| Seek | `0x0F` | 2 (HD/US1/US0, NCN) | 0 | Sets internal cylinder register straight to `NCN` -- instant |
| Sense Interrupt Status | `0x08` | 0 | 2 (ST0, PCN) | Real ROMs poll this after Seek/Recalibrate/Read Data, since those 3 have no result phase of their own (Read Data's own 7-byte result is separate, see below) or (Seek/Recalibrate) none at all |
| Read Data | `0x06` | 8 (HD/US1/US0, C, H, R, N, EOT, GPL, DTL) | sector data + 7 (ST0, ST1, ST2, C, H, R, N) | Real hardware transfers the sector's raw data AND the 7 status bytes through the same data register, back-to-back, both chip-to-host (DIO=1) -- this project's approved scope reads exactly the one sector named by `R` (real hardware can chain through `EOT` to read several consecutive sectors in one command; `EOT`/`GPL`/`DTL` are accepted as real params but not used to extend the transfer past that one sector -- an explicit scope reduction, not a hardware-verified claim about real EOT chaining). Result phase's C/H/R/N echo the same sector just read (real hardware sometimes advances R here for chaining; not relevant since chaining isn't implemented) |
| Read ID | `0x0A` | 1 (HD/US1/US0) | 7 (ST0, ST1, ST2, C, H, R, N) | Returns the ID field of whatever sector the disk's current head position would read next -- this project returns the first sector of the current cylinder/head (no real rotational position modeled); ST0 bit 6 set (abnormal termination) if no disk is inserted |

Any other command byte: Command phase still consumes whatever bytes
would normally be params (best-effort -- real hardware's behavior for a
totally unrecognized opcode varies; this project goes straight to a
1-byte Result phase with status `0x80`, the datasheet's own generic
"invalid command" indication).

### New crate `crates/dsk`

Parses a standard CPCEMU `.dsk` image, no machine/CPU dependency -- same
"format crate parses bytes into a plain struct" precedent as
`z80-snapshot`/`zx81-snapshot`/`zx-tape`.

```rust
pub struct Disk {
    tracks: Vec<Track>, // indexed by (cylinder * heads + head)
    // geometry derived from the disc-info header
}

struct Track {
    sectors: Vec<Sector>,
}

struct Sector {
    id: SectorId, // cylinder/head/sector/size byte, from the sector-info entry
    data: Vec<u8>,
}

pub fn parse(bytes: &[u8]) -> Result<Disk, Error>;
```

Format (verified against the CPCEMU `.dsk` specification, see
`CREDITS.md`): a 0x100-byte Disc Information block (identified by its
`"MV - CPCEMU..."` signature, distinguishing it from the "Extended DSK"
variant's different signature -- an Extended DSK file is a clear parse
error here, not silently misread) giving track count/side count/track
size, followed by one 0x100-byte Track Information block per track (own
signature, sector count, sector size), each holding a table of 8-byte
Sector Information entries (cylinder/head/sector/size-code/status bytes)
immediately followed by every sector's raw data back-to-back in table
order.

`zx-spectrum` implements `nec765::DiskMedium` for `dsk::Disk` in its own
bus-wiring module (not in `dsk` itself, keeping `dsk` free of any
`nec765` dependency -- the same "decoupled via a trait the format crate
never sees" split this spec's whole architecture is built around).

### Bus/machine integration (`crates/zx-spectrum`)

- `ZxSpectrumBus` gains `fdc: Option<nec765::Fdc<DskMedium>>`, real only
  when `Model::Spectrum2A3` (mirrors `ay`/`tape`'s existing `Option`
  shape) -- `None` on 48K/128K, so `read_io`/`write_io` skip straight to
  existing behavior there with no per-model branch beyond the existing
  `Option` check.
- `read_io`/`write_io` gain the `0x2ffd`/`0x3ffd` branches, checked
  alongside `0x1ffd`/`0x7ffd`'s existing most-specific-pattern-first
  resolution.
- `ZxSpectrum::load_disk(dsk::Disk)` mirrors `load_tape` -- installs a
  disk into the bus's `Fdc`, model-checked the same way
  `load_snapshot`/`load_tape` reject a mismatched `Model`/hardware shape.
- `0x1ffd` bit 3 (disk motor) stays stored-but-cosmetic -- instant
  Execution phase means motor spin-up time isn't modeled, so nothing
  reads it. Noted explicitly in the field's own doc comment once this
  lands, replacing today's "nothing acts on it yet" wording.

### CLI (`apps/retroclawd`)

- `--disk <path>` -- new flag, `.dsk` extension only,
  `requires`/model-checked against `--machine zx-spectrum-2a3` the same
  way `--tape`/`--snapshot` are checked against their own compatible
  machines (`AnyMachine::apply_disk`, same dispatch shape as
  `apply_tape`/`apply_snapshot`). `conflicts_with` nothing new --
  independent of `--snapshot`/`--tape`, a real +3 can have both a
  snapshot state and a disk in the drive simultaneously.

## Testing approach

- `nec765`: unit-tested against a trivial in-memory `DiskMedium` fake
  (no `dsk` dependency) -- feed each command's real byte sequence via
  `write_data`, assert MSR bit 6/7 transitions match the real phase
  sequence, assert Result-phase bytes match the datasheet's documented
  values for Recalibrate/Seek/Sense Interrupt Status, and that Read Data
  streams back exactly the fake medium's sector bytes. An unrecognized
  command byte asserted to produce the `0x80` invalid-command result.
- `dsk`: unit-tested against a small hand-built synthetic `.dsk` byte
  buffer (disc-info header + one track + one sector), asserting the
  parsed `Disk`'s geometry and sector data match exactly what was
  encoded -- same "construct the minimal real file shape by hand" style
  `z80-snapshot`'s existing tests use, not a reliance on a real
  commercial `.dsk` file being on hand for unit-level coverage (an actual
  +3 game disk is still worth trying manually once this lands, same as
  `USPIRITS.Z80` was for the snapshot/tape path).
- Bus integration: a `read_io`/`write_io` test mirroring the existing
  AY/tape bus tests -- install a disk, run a real Read Data command
  through the actual port sequence, confirm the bytes that come back
  match the installed disk's real sector data.
