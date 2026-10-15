# ZX81 video output: `Machine::render_frame`, real ROM font, PNG dump

Date: 2026-09-06
Status: Approved, pending implementation plan

## Problem

`retroclawd` has no way to see what a machine's video output actually
looks like. `crates/zx81` can boot a real ROM and run a program, but the
only introspection available is `--dump-memory` (raw hex) and
`--dump-screen` doesn't exist yet. There's also no GUI (Bevy work is
separate, sequenced later per `TODO.md`), so this needs to work headless,
from the CLI, today.

This follows the ZX81 keyboard-matrix work (`Zx81Key`,
`Zx81Bus`) as the second half of an earlier split; keyboard is done,
this spec covers video.

## Why not a character-grid abstraction

An earlier iteration of this design put a ZX81-specific `DisplayCell {
code, inverse }` grid on the table, decoded from `D_FILE`, with ASCII/ANSI
rendering for a terminal dump. That was rejected: the ZX Spectrum's video
hardware has no character-code concept at all -- it's a raw bitmap
(`0x4000`-`0x57FF`, a non-linear pixel layout) plus a separate attribute
byte per 8x8 cell (`0x5800`-`0x5AFF`) for color. A "character grid" trait
method would never fit Spectrum, let alone the wider set of machines this
needs to scale to (MSX/Spectravideo's TMS9918 VDP -- tile+sprite
compositing against its own VRAM; Amstrad CPC's CRTC + gate array --
arbitrary bitmap modes; Elan Enterprise's "Nick" chip; ABC80's own text
controller). Each of those is genuinely different at the video-generation
level, and none of that should be unified -- forcing it would mean
building speculative abstractions for machines that don't exist yet in
this codebase, which this project's own `TODO.md` explicitly guards
against elsewhere (e.g. the NES-mapper guidance: don't generalize until a
second real case exists).

What *is* universal, no matter how different the internal video hardware
is: every one of these machines ultimately produces a grid of colored
pixels for one frame. That's the seam this spec builds on -- unify the
*output contract* (pixels), leave the *generation mechanism* (VDP
compositing, CRTC bitmap fetch, ZX81's CPU-driven font lookup, whatever)
entirely to each machine crate.

## Non-goals

- MSX, Spectravideo, Amstrad CPC, Elan Enterprise, or ABC80 support. This
  spec adds the `Machine::render_frame` contract and ZX81's own
  implementation of it; every other machine implements it (or doesn't)
  whenever it's actually built, with its own design pass.
- ZX Spectrum video. `ZxSpectrum::render_frame` keeps the trait's default
  (`None`) this round -- Spectrum's bitmap+attribute decode is a separate
  task.
- Live/continuous rendering, animation, or any GUI. `render_frame` returns
  one static frame snapshot of current machine state, called on demand
  (e.g. after a run stops), not on any timing loop. Real ZX81 hardware
  couples video generation to CPU execution during retrace (NMI-driven,
  HALT-based); this spec does not implement or simulate that coupling --
  it just decodes whatever is currently in `D_FILE` and reads the
  already-loaded ROM's font table, exactly once, whenever called.
- Colour, sound, tape. Unaffected/unrelated to this work.

## `Machine` trait change

One new default method, in `crates/machine/src/lib.rs`:

```rust
/// Renders the current video frame as RGB pixels, for machines with video
/// output implemented. `(width, height, rgb)` where
/// `rgb.len() == width as usize * height as usize * 3`, row-major,
/// top-to-bottom, left-to-right. `None` for machines without video output
/// implemented yet.
fn render_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
    None
}
```

Object-safety: unlike `run()`, this needs no `Self: Sized` bound and no
special handling -- it's a plain `&self` method returning an owned value,
with no internal need to coerce `self` into `&mut dyn Machine`. A default
body works through `Box<dyn Machine>` with no caveats.

`zx-spectrum` requires no code change (keeps the inherited default).

## ZX81's `render_frame` implementation

New code in `crates/zx81`, likely a new module (`video.rs`, mirroring
`bus.rs`/`keyboard.rs`'s one-concern-per-file layout):

1. Read the `D_FILE` system-variable pointer at `0x400C`/`0x400D`
   (16-bit, little-endian) via the existing `read_memory`/`peek` path --
   no new bus methods needed, this is pure memory reads.
2. Decode the display file into up to 24 rows of up to 32 character codes
   each, splitting on the `0x76` row-terminator byte (same decode shape
   as the earlier character-grid design, just consumed internally now
   instead of exposed as a public type) -- capped at 24 rows / 32
   bytes-per-row so an uninitialized or garbage `D_FILE` pointer can't
   hang the decode (mirrors `--max-cycles`'s "bound the loop, don't trust
   unvalidated state" philosophy already used elsewhere in this crate).
3. For each character cell, look up its real 8x8 pixel bitmap from the
   already-loaded ROM: verified against an actual ZX81 ROM disassembly
   (the `INITIAL` routine sets `I = 0x1E` via `LD A,$1E` / `LD I,A`; the
   `COPY` display routine computes `HL = 0x1E00 + (char_code * 8) +
   scanline_row` for `scanline_row` 0-7) -- this is the real ZX81
   hardware mechanism (the Z80's own instruction-refresh addressing,
   repurposed by the ROM as a font lookup during video generation), not
   an approximation. One byte per scanline row, bits MSB-first (bit 7 =
   leftmost pixel of that row), `1` = ink. Character codes 128-191
   (inverse video, bit 7 of the *character* code set) use the same
   `code - 128` bitmap with black/white swapped; codes 64-127 (BASIC
   keyword tokens) have no real bitmap in this table and render as blank
   (all-white) cells -- they shouldn't appear in a real display file
   during normal `PRINT` output, but a raw `POKE` could put one there.
   Codes 192-255 (`code - 128` falls in 64-127) render as blank the same
   way -- an inverse-video blank is still blank, so no separate swap
   logic is needed for this range.
4. Composite into a 256x192 RGB buffer (32 cols x 8px = 256, 24 rows x
   8px = 192 -- ZX81's real PAL resolution). Monochrome: ink pixels are
   `(0, 0, 0)`, paper pixels are `(255, 255, 255)`. Always returns
   `Some(...)` (never `None` -- the buffer is well-defined even for an
   uninitialized/garbage `D_FILE`, same "trust what's there" stance as
   `--dump-memory`).

No new public types are needed in `crates/zx81` for this -- the row/cell
decoding is an internal implementation detail of `render_frame`, not a
public API (unlike the keyboard matrix, nothing outside this method
needs the intermediate character-code representation).

## `retroclawd` CLI change

- `--dump-screen <path>` (was going to be a boolean flag in an earlier
  iteration of this design; now takes a file path, e.g. `--dump-screen
  screen.png`, since PNG output needs a destination).
- After the run stops (halt or `--max-cycles` cutoff), if `--dump-screen`
  was given: call `machine.render_frame()`. If `Some((w, h, rgb))`,
  PNG-encode and write to the given path. If `None` (e.g. `--machine
  zx-spectrum`), print `"no video output implemented for {machine} yet"`
  to stderr and exit with the same failure code style already used for
  other flag/machine mismatches elsewhere in `main.rs` -- not a crash,
  a clear message.
- New dependency: the `png` crate (encoding only -- not the broader
  `image` crate, since nothing here needs format conversion or decoding,
  just writing one RGB buffer to a file. Narrower dependency, lower
  lifecycle cost).

## Testing

- `crates/zx81`: unit tests on `render_frame()` directly (no ROM-loading
  ceremony beyond a small crafted ROM fixture covering a couple of known
  character codes' font bytes) --
  - a blank/collapsed `D_FILE` (all rows just `0x76`) renders an
    all-white 256x192 buffer,
  - a known character code at a known screen cell produces the expected
    black/white pixel pattern at the expected pixel offsets (verifying
    the `0x1E00 + code*8 + row` addressing and MSB-first bit order),
  - the same character in its inverse-video form (code + 128) swaps
    black and white at those same pixel offsets,
  - a row exceeding 32 bytes without a terminator, and more than 24
    rows, are both bounded (regression tests for the safety caps),
  - a code in the 64-127 (token) range renders as a blank cell.
- `retroclawd`: no new automated test beyond the existing build --
  `--dump-screen` on `zx-spectrum` should print the "not implemented"
  message and fail cleanly; manual verification that a real ZX81 ROM
  boot produces a plausible-looking PNG (e.g. opening it and eyeballing
  that it looks like a ZX81 boot screen) is the practical acceptance
  check here, same as the project's existing "start the dev server and
  use it" guidance for anything that isn't cheaply assertable.

## Open questions

None blocking. Two things worth a note for whoever revisits video work
later:

- This spec renders one static frame on demand, not on any timing loop.
  Real ZX81 video generation is coupled to CPU execution (NMI-driven,
  HALT-based, and behaves differently in FAST vs. SLOW mode -- SLOW mode
  pauses computation during retrace, FAST mode blanks the display during
  computation, the well-known ZX81 "flicker"). None of that is modeled
  here. A future live-rendering GUI (Bevy) will need to decide how much
  of that timing coupling to actually simulate -- this spec doesn't
  answer that, it only makes sure `render_frame`'s contract (pixels in,
  pixels out) doesn't block whichever answer that ends up being.
- The MSX/SVI/CPC/Enterprise/ABC80 machines this is meant to scale to are
  not designed here at all -- only the trait contract they'd eventually
  implement. Each will need its own brainstorm/design pass when actually
  started, per this project's existing "don't generalize speculatively"
  practice.
