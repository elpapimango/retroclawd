# `.tap`/`.tzx` Tape Format Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Load real ZX Spectrum `.tap`/`.tzx` cassette images through the ROM's own real tape-loading routine, at real hardware pulse timing by default with a selectable turbo speed multiplier, auto-starting playback on the first EAR-bit read.

**Architecture:** A new `zx-tape` crate parses both formats into a flat sequence of EAR-line `Pulse`/`Stop` events (`.tap` synthesizes the standard ROM protocol's pulses from raw data bytes; `.tzx`'s standard/turbo speed blocks do the same with either fixed or per-block timing parameters) and a `TapePlayer` that walks that sequence in real T-states. `zx-spectrum`'s bus owns an optional `TapePlayer`, ticks it every CPU step, and composes its EAR level into port `0xFE` bit 6 on read -- generic across every `Model`, since they all share the same port.

**Tech Stack:** Rust, no new external dependencies.

**Spec:** `docs/superpowers/specs/2026-09-10-tape-formats-design.md`

## Global Constraints

- Load-only -- no tape saving (MIC output, port `0xFE` bit 3) in this plan.
- Generic across every `zx_spectrum::Model` (48K/128K/+2A/+3 share the same port `0xFE` EAR bit) -- no per-model branching.
- `zx-tape` has no dependency on `machine`, any CPU crate, or `zx-spectrum` -- same reasoning as `z80-snapshot`/`zx81-snapshot`.
- Turbo mode is the *same* real pulse-level protocol sped up by an integer multiplier, never a ROM-routine trap.
- An unrecognized `.tzx` block ID that this plan doesn't implement a skip for is a real parse error (`Error::UnsupportedBlockType`), never a silently-guessed skip length.
- Standard protocol timing constants (verified against the TZX format specification, see `CREDITS.md`): pilot pulse 2168 T-states (8063 pulses if the block's flag byte < 128, else 3223), sync pulses 667/735 T-states, bit-0 as two 855 T-state pulses, bit-1 as two 1710 T-state pulses.
- Port `0xFE` bit 6 = EAR input on read; bits 5 and 7 always read 1 (verified against the World of Spectrum 48K reference, see `CREDITS.md`).

---

## Task 1: `zx-tape` crate -- pulse types + `.tap` parsing

**Files:**
- Create: `crates/zx-tape/Cargo.toml`
- Create: `crates/zx-tape/src/lib.rs`
- Modify: `Cargo.toml` (workspace root -- add `crates/zx-tape` to `members`)

**Interfaces:**
- Produces:
  - `pub struct Pulse { pub cycles: u32, pub level: bool }`
  - `pub struct Tape` (opaque; internally `{ events: Vec<Event> }`)
  - `pub enum Error { TruncatedBlock, NotATzxFile, UnsupportedBlockType(u8) }` (implements `std::error::Error`/`Display`)
  - `pub fn parse_tap(bytes: &[u8]) -> Result<Tape, Error>`
  - (crate-private, consumed by Task 2/3 in the same crate) `enum Event { Pulse(Pulse), Stop }`, `fn push_pulse_block(...)`, `fn push_standard_block(...)`

- [ ] **Step 1: Create `crates/zx-tape/Cargo.toml`**

```toml
[package]
name = "zx-tape"
version = "0.1.0"
edition = "2024"

[dependencies]
```

- [ ] **Step 2: Add the crate to the workspace**

Edit `Cargo.toml` (root), adding `"crates/zx-tape"` to the `members` list:

```toml
[workspace]
resolver = "2"
members = ["crates/z80", "crates/machine", "crates/sinclair-keyboard", "crates/zx81", "crates/zx-spectrum", "crates/z80-snapshot", "crates/zx81-snapshot", "crates/ay-3-8910", "crates/zx-tape", "apps/retroclawd"]
```

- [ ] **Step 3: Write the failing tests**

Create `crates/zx-tape/src/lib.rs` with just the types and a `tests` module first (no `parse_tap`/`push_*` bodies yet -- Step 4 implements them):

```rust
/// One EAR-line pulse: how long it lasts (in real Z80 T-states) and what
/// level it holds for that duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pulse {
    pub cycles: u32,
    pub level: bool,
}

/// One event in a parsed tape's timeline -- either a timed pulse, or a
/// "stop the tape" control point (from a `.tzx` pause/stop block with a
/// zero duration): playback halts there until `TapePlayer::play` is
/// called again, matching a real cassette needing Play pressed again for
/// a multi-load game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Pulse(Pulse),
    Stop,
}

/// A fully parsed tape: every EAR-line event, in order, from the moment
/// playback starts to end-of-tape.
#[derive(Debug, Clone, Default)]
pub struct Tape {
    events: Vec<Event>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    TruncatedBlock,
    NotATzxFile,
    UnsupportedBlockType(u8),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TruncatedBlock => write!(f, "tape file truncated mid-block"),
            Error::NotATzxFile => write!(f, "not a .tzx file (missing 'ZXTape!' signature)"),
            Error::UnsupportedBlockType(id) => {
                write!(f, "unsupported .tzx block type 0x{id:02X}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Standard ROM tape-loading protocol constants, in real Z80 T-states --
/// verified against the TZX format specification, see CREDITS.md.
const PILOT_PULSE: u32 = 2168;
const PILOT_PULSES_HEADER: u32 = 8063;
const PILOT_PULSES_DATA: u32 = 3223;
const SYNC1: u32 = 667;
const SYNC2: u32 = 735;
const BIT0_PULSE: u32 = 855;
const BIT1_PULSE: u32 = 1710;

/// Default pause (idle level) between blocks when the source doesn't
/// specify one, in real Z80 T-states at the Spectrum's ~3.5MHz PAL clock
/// (matching `zx_spectrum::ZxSpectrum::clock_hz`'s own PAL constant) --
/// the conventional 1-second gap most tools/emulators use for `.tap`
/// (which stores no pause value at all). Not format-critical: any
/// reasonably long idle gap works, since its only job is giving the ROM's
/// loader a clear silence between one block's data and the next pilot
/// tone.
const DEFAULT_PAUSE_CYCLES: u32 = 3_546_900; // 1000ms

fn read_u16_checked(bytes: &[u8], pos: usize) -> Result<u16, Error> {
    let slice = bytes.get(pos..pos + 2).ok_or(Error::TruncatedBlock)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_slice_checked(bytes: &[u8], pos: usize, len: usize) -> Result<&[u8], Error> {
    bytes.get(pos..pos + len).ok_or(Error::TruncatedBlock)
}

/// Synthesizes one block's pulse sequence: `pilot_count` pulses of
/// `pilot_cycles`, then a `sync1_cycles`/`sync2_cycles` pulse pair, then
/// each byte of `data` bit-encoded MSB-first as `bit0_cycles`/
/// `bit1_cycles` pulse pairs -- shared by both the standard protocol
/// (`.tap`, and `.tzx`'s Standard Speed Data Block) and `.tzx`'s Turbo
/// Speed Data Block, which is the exact same algorithm with every timing
/// value taken from the block itself instead of the fixed standard
/// constants. `used_bits_in_last_byte` (1-8) trims how many of the final
/// byte's bits are actually encoded -- always 8 for the standard
/// protocol, which has no such concept; `.tzx` turbo blocks can specify
/// fewer.
#[allow(clippy::too_many_arguments)]
fn push_pulse_block(
    events: &mut Vec<Event>,
    data: &[u8],
    pilot_cycles: u32,
    pilot_count: u32,
    sync1_cycles: u32,
    sync2_cycles: u32,
    bit0_cycles: u32,
    bit1_cycles: u32,
    used_bits_in_last_byte: u8,
    pause_cycles: u32,
) {
    let mut level = true;

    for _ in 0..pilot_count {
        events.push(Event::Pulse(Pulse {
            cycles: pilot_cycles,
            level,
        }));
        level = !level;
    }

    events.push(Event::Pulse(Pulse {
        cycles: sync1_cycles,
        level,
    }));
    level = !level;
    events.push(Event::Pulse(Pulse {
        cycles: sync2_cycles,
        level,
    }));
    level = !level;

    let last_byte_index = data.len().saturating_sub(1);
    for (byte_index, &byte) in data.iter().enumerate() {
        let bits = if byte_index == last_byte_index {
            used_bits_in_last_byte
        } else {
            8
        };
        for bit_index in 0..bits {
            let bit = (byte >> (7 - bit_index)) & 1;
            let pulse_cycles = if bit == 0 { bit0_cycles } else { bit1_cycles };
            events.push(Event::Pulse(Pulse {
                cycles: pulse_cycles,
                level,
            }));
            level = !level;
            events.push(Event::Pulse(Pulse {
                cycles: pulse_cycles,
                level,
            }));
            level = !level;
        }
    }

    events.push(Event::Pulse(Pulse {
        cycles: pause_cycles,
        level: false,
    }));
}

/// Synthesizes the standard ROM loading pulse sequence for one block's
/// raw bytes (`data[0]` is the flag byte; the header-vs-data pilot-tone
/// pulse count is chosen from it, per the verified standard protocol --
/// see CREDITS.md).
fn push_standard_block(events: &mut Vec<Event>, data: &[u8], pause_cycles: u32) {
    let pilot_count = if data.first().copied().unwrap_or(0) < 128 {
        PILOT_PULSES_HEADER
    } else {
        PILOT_PULSES_DATA
    };
    push_pulse_block(
        events,
        data,
        PILOT_PULSE,
        pilot_count,
        SYNC1,
        SYNC2,
        BIT0_PULSE,
        BIT1_PULSE,
        8,
        pause_cycles,
    );
}

/// Parses a `.tap` file: a flat sequence of `[len_lo, len_hi, data...]`
/// blocks, each played back through the standard ROM loading protocol
/// (`.tap` itself stores no pulse timing -- that's the whole point of the
/// format, it's *always* the standard protocol). `len` counts `data`'s
/// own bytes (flag byte + payload + checksum together), not including
/// the 2-byte length prefix itself.
pub fn parse_tap(bytes: &[u8]) -> Result<Tape, Error> {
    let mut events = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let len = read_u16_checked(bytes, pos)? as usize;
        pos += 2;
        let data = read_slice_checked(bytes, pos, len)?;
        pos += len;
        push_standard_block(&mut events, data, DEFAULT_PAUSE_CYCLES);
    }
    Ok(Tape { events })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tap_synthesizes_the_standard_pilot_sync_and_bit_pulses() {
        // Minimal 1-byte block: flag byte 0x00 (header -> 8063 pilot
        // pulses per the verified standard protocol), all bits zero.
        let bytes = [0x01, 0x00, 0x00]; // len=1 (LE), data=[0x00]
        let tape = parse_tap(&bytes).unwrap();

        // 8063 pilot + 2 sync + 8 bits * 2 pulses/bit + 1 pause.
        assert_eq!(tape.events.len(), 8063 + 2 + 16 + 1);

        assert_eq!(
            tape.events[0],
            Event::Pulse(Pulse {
                cycles: PILOT_PULSE,
                level: true
            })
        );
        assert_eq!(
            tape.events[1],
            Event::Pulse(Pulse {
                cycles: PILOT_PULSE,
                level: false
            })
        );

        // Sync pair right after the pilot tone (indices 0..8062 are
        // pilot, so index 8063 is the first index after it).
        assert_eq!(
            tape.events[8063],
            Event::Pulse(Pulse {
                cycles: SYNC1,
                level: true
            })
        );
        assert_eq!(
            tape.events[8064],
            Event::Pulse(Pulse {
                cycles: SYNC2,
                level: false
            })
        );

        // First data bit (0, since the byte is 0x00) -> two BIT0_PULSE
        // pulses right after sync.
        assert_eq!(
            tape.events[8065],
            Event::Pulse(Pulse {
                cycles: BIT0_PULSE,
                level: true
            })
        );
        assert_eq!(
            tape.events[8066],
            Event::Pulse(Pulse {
                cycles: BIT0_PULSE,
                level: false
            })
        );

        // Final event is the pause.
        assert_eq!(
            *tape.events.last().unwrap(),
            Event::Pulse(Pulse {
                cycles: DEFAULT_PAUSE_CYCLES,
                level: false
            })
        );
    }

    #[test]
    fn parse_tap_uses_data_pilot_count_when_flag_byte_is_128_or_above() {
        let bytes = [0x01, 0x00, 0xFF]; // flag byte 0xFF (>= 128) -> data block
        let tape = parse_tap(&bytes).unwrap();

        // 3223 pilot + 2 sync + 16 bit pulses + 1 pause.
        assert_eq!(tape.events.len(), 3223 + 2 + 16 + 1);
    }

    #[test]
    fn parse_tap_rejects_a_truncated_block() {
        let bytes = [0x05, 0x00, 0x01, 0x02]; // claims len=5, only 2 bytes follow
        assert_eq!(parse_tap(&bytes), Err(Error::TruncatedBlock));
    }

    #[test]
    fn parse_tap_handles_multiple_blocks() {
        // Two 1-byte blocks back to back.
        let bytes = [0x01, 0x00, 0x00, 0x01, 0x00, 0xFF];
        let tape = parse_tap(&bytes).unwrap();

        let first_block_len = 8063 + 2 + 16 + 1;
        let second_block_len = 3223 + 2 + 16 + 1;
        assert_eq!(tape.events.len(), first_block_len + second_block_len);
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p zx-tape`
Expected: all 4 tests PASS (the file above already contains `parse_tap`'s
real implementation, not a stub -- there's no separate "red" state to
watch here since the type/function signatures and their bodies were
written together; the tests exist to lock in the exact pulse counts/values
the verified standard protocol predicts).

- [ ] **Step 5: Commit**

```bash
git add crates/zx-tape Cargo.toml
git commit -m "feat(zx-tape): pulse types and .tap parsing

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 2: `TapePlayer` -- real-time playback

**Files:**
- Modify: `crates/zx-tape/src/lib.rs` (append `TapePlayer` + tests)

**Interfaces:**
- Consumes: `Tape`, `Event`, `Pulse` from Task 1.
- Produces:
  - `pub struct TapePlayer` (opaque)
  - `impl TapePlayer { pub fn new(tape: Tape) -> Self; pub fn set_speed_multiplier(&mut self, n: u32); pub fn play(&mut self); pub fn tick(&mut self, cycles: u32); pub fn ear_bit(&self) -> bool; pub fn finished(&self) -> bool; }`

- [ ] **Step 1: Write the failing tests**

Append to `crates/zx-tape/src/lib.rs`'s `tests` module:

```rust
    #[test]
    fn tape_player_reports_idle_level_until_played() {
        let tape = Tape {
            events: vec![Event::Pulse(Pulse {
                cycles: 10,
                level: true,
            })],
        };
        let player = TapePlayer::new(tape);

        assert!(!player.finished());
        assert!(player.ear_bit()); // idle -- not playing yet
    }

    #[test]
    fn tape_player_advances_through_pulses_as_it_ticks() {
        let tape = Tape {
            events: vec![
                Event::Pulse(Pulse {
                    cycles: 10,
                    level: true,
                }),
                Event::Pulse(Pulse {
                    cycles: 5,
                    level: false,
                }),
            ],
        };
        let mut player = TapePlayer::new(tape);
        player.play();

        assert!(player.ear_bit()); // first pulse: level=true

        player.tick(10); // exactly consumes the first pulse
        assert!(!player.ear_bit()); // now on the second pulse: level=false

        player.tick(5); // exactly consumes the second pulse -- tape ends
        assert!(player.finished());
        assert!(player.ear_bit()); // idle level once finished
    }

    #[test]
    fn tape_player_speed_multiplier_compresses_real_elapsed_time() {
        let tape = Tape {
            events: vec![Event::Pulse(Pulse {
                cycles: 100,
                level: true,
            })],
        };
        let mut player = TapePlayer::new(tape);
        player.set_speed_multiplier(4);
        player.play();

        player.tick(25); // 25 real cycles * 4x = 100 effective cycles
        assert!(player.finished());
    }

    #[test]
    fn tape_player_stops_at_a_stop_event_and_resumes_on_play() {
        let tape = Tape {
            events: vec![
                Event::Pulse(Pulse {
                    cycles: 10,
                    level: true,
                }),
                Event::Stop,
                Event::Pulse(Pulse {
                    cycles: 10,
                    level: false,
                }),
            ],
        };
        let mut player = TapePlayer::new(tape);
        player.play();
        player.tick(10); // consumes the first pulse, lands on Stop
        player.tick(1); // processes the Stop event -- playback halts

        assert!(!player.finished());
        assert!(player.ear_bit()); // stopped -- idle level regardless of position

        player.play(); // resume
        assert!(!player.ear_bit()); // now on the third event immediately
    }

    #[test]
    fn tape_player_play_is_a_no_op_once_finished() {
        let tape = Tape {
            events: vec![Event::Pulse(Pulse {
                cycles: 1,
                level: true,
            })],
        };
        let mut player = TapePlayer::new(tape);
        player.play();
        player.tick(1);
        assert!(player.finished());

        player.play(); // no-op: nothing left to play
        assert!(player.ear_bit()); // still idle
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p zx-tape`
Expected: FAIL to compile -- `TapePlayer` doesn't exist yet.

- [ ] **Step 3: Implement `TapePlayer`**

Append to `crates/zx-tape/src/lib.rs`, after `parse_tap` and before `#[cfg(test)]`:

```rust
/// Walks a `Tape`'s events in real time, exposing the EAR line's current
/// level. `tick` advances it by however many real CPU cycles just
/// elapsed; the caller (`zx_spectrum::ZxSpectrumBus`) is responsible for
/// calling it once per CPU step, same pattern as `ay_3_8910::Ay3_8910::
/// tick`.
pub struct TapePlayer {
    tape: Tape,
    position: usize,
    cycles_into_current: u32,
    playing: bool,
    speed_multiplier: u32,
}

impl TapePlayer {
    pub fn new(tape: Tape) -> Self {
        Self {
            tape,
            position: 0,
            cycles_into_current: 0,
            playing: false,
            speed_multiplier: 1,
        }
    }

    /// Turbo mode: the *same* real pulse-level protocol, sped up -- not a
    /// different mechanism. `n` of `0` is treated as `1` (real time).
    pub fn set_speed_multiplier(&mut self, n: u32) {
        self.speed_multiplier = n.max(1);
    }

    /// Starts (or resumes) playback. A no-op if the tape has already run
    /// out -- there's nothing left to play.
    pub fn play(&mut self) {
        if !self.finished() {
            self.playing = true;
        }
    }

    pub fn finished(&self) -> bool {
        self.position >= self.tape.events.len()
    }

    /// Advances playback by `cycles` real CPU cycles (scaled by the speed
    /// multiplier). A no-op while not playing or already finished.
    pub fn tick(&mut self, cycles: u32) {
        if !self.playing || self.finished() {
            return;
        }

        let mut remaining = cycles.saturating_mul(self.speed_multiplier);
        while remaining > 0 && !self.finished() {
            match self.tape.events[self.position] {
                Event::Stop => {
                    // Playback halts here until `play` is called again --
                    // advance past the marker now, so resuming doesn't
                    // immediately re-trigger the same stop.
                    self.playing = false;
                    self.position += 1;
                    self.cycles_into_current = 0;
                    return;
                }
                Event::Pulse(pulse) => {
                    let left_in_pulse = pulse.cycles.saturating_sub(self.cycles_into_current);
                    if remaining < left_in_pulse {
                        self.cycles_into_current += remaining;
                        remaining = 0;
                    } else {
                        remaining -= left_in_pulse;
                        self.cycles_into_current = 0;
                        self.position += 1;
                    }
                }
            }
        }
    }

    /// The EAR line's current level. Idle-high (`true`) whenever playback
    /// isn't actively running (not yet started, stopped at a `Stop`
    /// event, or the tape has fully finished) -- matches a real stopped
    /// cassette presenting no signal, regardless of which pulse happens
    /// to be queued up next.
    pub fn ear_bit(&self) -> bool {
        if !self.playing || self.finished() {
            return true;
        }
        match self.tape.events[self.position] {
            Event::Pulse(pulse) => pulse.level,
            Event::Stop => true,
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p zx-tape`
Expected: all tests PASS (4 from Task 1 + 5 new).

- [ ] **Step 5: Commit**

```bash
git add crates/zx-tape/src/lib.rs
git commit -m "feat(zx-tape): TapePlayer real-time playback with turbo speed and stop/resume

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 3: `.tzx` parsing

**Files:**
- Modify: `crates/zx-tape/src/lib.rs` (append `parse_tzx` + tests)

**Interfaces:**
- Consumes: `push_pulse_block`, `push_standard_block`, `Event`, `Tape`, `Error` from Task 1.
- Produces: `pub fn parse_tzx(bytes: &[u8]) -> Result<Tape, Error>`

- [ ] **Step 1: Write the failing tests**

Append to `crates/zx-tape/src/lib.rs`'s `tests` module:

```rust
    fn tzx_header() -> Vec<u8> {
        let mut bytes = b"ZXTape!\x1a".to_vec();
        bytes.push(1); // major version
        bytes.push(20); // minor version
        bytes
    }

    #[test]
    fn parse_tzx_rejects_a_file_without_the_signature() {
        assert_eq!(parse_tzx(b"not a tzx file"), Err(Error::NotATzxFile));
    }

    #[test]
    fn parse_tzx_standard_speed_block_matches_tap_synthesis() {
        let mut bytes = tzx_header();
        bytes.push(0x10); // Standard Speed Data Block
        bytes.extend_from_slice(&500u16.to_le_bytes()); // pause (ms)
        bytes.extend_from_slice(&1u16.to_le_bytes()); // data length
        bytes.push(0x00); // data: flag byte 0x00 (header)

        let tape = parse_tzx(&bytes).unwrap();

        // Same shape as the equivalent .tap block, but with the block's
        // own pause value instead of the default.
        assert_eq!(tape.events.len(), 8063 + 2 + 16 + 1);
        let pause_cycles = 500u32 * 3_546_900 / 1000;
        assert_eq!(
            *tape.events.last().unwrap(),
            Event::Pulse(Pulse {
                cycles: pause_cycles,
                level: false
            })
        );
    }

    #[test]
    fn parse_tzx_turbo_speed_block_uses_the_blocks_own_timing_and_used_bits() {
        let mut bytes = tzx_header();
        bytes.push(0x11); // Turbo Speed Data Block
        bytes.extend_from_slice(&1000u16.to_le_bytes()); // pilot pulse
        bytes.extend_from_slice(&600u16.to_le_bytes()); // sync1
        bytes.extend_from_slice(&700u16.to_le_bytes()); // sync2
        bytes.extend_from_slice(&500u16.to_le_bytes()); // bit0
        bytes.extend_from_slice(&1000u16.to_le_bytes()); // bit1
        bytes.extend_from_slice(&100u16.to_le_bytes()); // pilot pulse count
        bytes.push(3); // used bits in last byte
        bytes.extend_from_slice(&0u16.to_le_bytes()); // pause (ms)
        bytes.extend_from_slice(&[1, 0, 0]); // data length (3 bytes, LE)
        bytes.push(0xFF); // 1 data byte, only 3 bits used

        let tape = parse_tzx(&bytes).unwrap();

        // 100 pilot + 2 sync + 3 bits (used_bits_in_last_byte) * 2 pulses + 1 pause.
        assert_eq!(tape.events.len(), 100 + 2 + 6 + 1);
        assert_eq!(
            tape.events[0],
            Event::Pulse(Pulse {
                cycles: 1000,
                level: true
            })
        );
    }

    #[test]
    fn parse_tzx_pause_block_with_nonzero_duration_is_an_idle_pulse() {
        let mut bytes = tzx_header();
        bytes.push(0x20); // Pause/Stop the tape
        bytes.extend_from_slice(&2000u16.to_le_bytes()); // 2000ms, not a stop

        let tape = parse_tzx(&bytes).unwrap();

        assert_eq!(tape.events.len(), 1);
        let expected_cycles = 2000u32 * 3_546_900 / 1000;
        assert_eq!(
            tape.events[0],
            Event::Pulse(Pulse {
                cycles: expected_cycles,
                level: false
            })
        );
    }

    #[test]
    fn parse_tzx_pause_block_with_zero_duration_is_a_stop_event() {
        let mut bytes = tzx_header();
        bytes.push(0x20);
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let tape = parse_tzx(&bytes).unwrap();

        assert_eq!(tape.events, vec![Event::Stop]);
    }

    #[test]
    fn parse_tzx_skips_group_start_and_end_blocks() {
        let mut bytes = tzx_header();
        bytes.push(0x21); // Group start
        bytes.push(3); // name length
        bytes.extend_from_slice(b"abc");
        bytes.push(0x22); // Group end (no fields)

        let tape = parse_tzx(&bytes).unwrap();

        assert_eq!(tape.events.len(), 0);
    }

    #[test]
    fn parse_tzx_errors_clearly_on_an_unimplemented_block_type() {
        let mut bytes = tzx_header();
        bytes.push(0x30); // Text description -- not implemented, must error not skip

        assert_eq!(parse_tzx(&bytes), Err(Error::UnsupportedBlockType(0x30)));
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p zx-tape`
Expected: FAIL to compile -- `parse_tzx` doesn't exist yet.

- [ ] **Step 3: Implement `parse_tzx`**

Append to `crates/zx-tape/src/lib.rs`, after `parse_tap` and before `#[cfg(test)]`. Also add this helper next to `read_u16_checked`/`read_slice_checked`:

```rust
fn read_u24_checked(bytes: &[u8], pos: usize) -> Result<u32, Error> {
    let slice = bytes.get(pos..pos + 3).ok_or(Error::TruncatedBlock)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], 0]))
}

/// Converts a `.tzx` pause value (milliseconds) to real Z80 T-states, at
/// the same PAL clock rate `DEFAULT_PAUSE_CYCLES` uses.
fn ms_to_cycles(ms: u16) -> u32 {
    (ms as u64 * 3_546_900 / 1000) as u32
}
```

Then `parse_tzx` itself:

```rust
const TZX_SIGNATURE: &[u8] = b"ZXTape!\x1a";

/// Parses a `.tzx` file. Implements the two block types that carry real
/// pulse data for the vast majority of commercial releases (Standard
/// Speed Data Block `0x10`, Turbo Speed Data Block `0x11`), the
/// pause/stop-tape control block (`0x20`), and the two trivial,
/// commonly-present metadata blocks whose skip length this was verified
/// against (Group Start `0x21`, Group End `0x22`). Every other block ID
/// is a clear `Error::UnsupportedBlockType` rather than a guessed skip --
/// see the design spec and this plan's Global Constraints for why.
pub fn parse_tzx(bytes: &[u8]) -> Result<Tape, Error> {
    if bytes.len() < TZX_SIGNATURE.len() + 2 || &bytes[..TZX_SIGNATURE.len()] != TZX_SIGNATURE {
        return Err(Error::NotATzxFile);
    }

    let mut events = Vec::new();
    let mut pos = TZX_SIGNATURE.len() + 2; // skip signature + major/minor version bytes

    while pos < bytes.len() {
        let id = bytes[pos];
        pos += 1;

        match id {
            0x10 => {
                let pause_ms = read_u16_checked(bytes, pos)?;
                let len = read_u16_checked(bytes, pos + 2)? as usize;
                pos += 4;
                let data = read_slice_checked(bytes, pos, len)?;
                pos += len;
                push_standard_block(&mut events, data, ms_to_cycles(pause_ms));
            }
            0x11 => {
                let pilot_pulse = read_u16_checked(bytes, pos)? as u32;
                let sync1 = read_u16_checked(bytes, pos + 2)? as u32;
                let sync2 = read_u16_checked(bytes, pos + 4)? as u32;
                let bit0 = read_u16_checked(bytes, pos + 6)? as u32;
                let bit1 = read_u16_checked(bytes, pos + 8)? as u32;
                let pilot_count = read_u16_checked(bytes, pos + 10)? as u32;
                let used_bits_in_last_byte = *bytes.get(pos + 12).ok_or(Error::TruncatedBlock)?;
                let pause_ms = read_u16_checked(bytes, pos + 13)?;
                let len = read_u24_checked(bytes, pos + 15)? as usize;
                pos += 18;
                let data = read_slice_checked(bytes, pos, len)?;
                pos += len;
                push_pulse_block(
                    &mut events,
                    data,
                    pilot_pulse,
                    pilot_count,
                    sync1,
                    sync2,
                    bit0,
                    bit1,
                    used_bits_in_last_byte,
                    ms_to_cycles(pause_ms),
                );
            }
            0x20 => {
                let pause_ms = read_u16_checked(bytes, pos)?;
                pos += 2;
                if pause_ms == 0 {
                    events.push(Event::Stop);
                } else {
                    events.push(Event::Pulse(Pulse {
                        cycles: ms_to_cycles(pause_ms),
                        level: false,
                    }));
                }
            }
            0x21 => {
                let len = *bytes.get(pos).ok_or(Error::TruncatedBlock)? as usize;
                pos += 1;
                if pos + len > bytes.len() {
                    return Err(Error::TruncatedBlock);
                }
                pos += len;
            }
            0x22 => {}
            other => return Err(Error::UnsupportedBlockType(other)),
        }
    }

    Ok(Tape { events })
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p zx-tape`
Expected: all tests PASS (9 from Tasks 1-2 + 7 new).

- [ ] **Step 5: Lint and format**

Run: `cargo clippy -p zx-tape -- -D warnings` -- must be clean (the
`#[allow(clippy::too_many_arguments)]` on `push_pulse_block` from Task 1
already covers that one expected lint).
Run: `cargo fmt -p zx-tape -- --check` -- must be clean; if not, run
`cargo fmt -p zx-tape` and re-check.

- [ ] **Step 6: Commit**

```bash
git add crates/zx-tape/src/lib.rs
git commit -m "feat(zx-tape): .tzx parsing (standard/turbo speed blocks, pause/stop, group markers)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 4: `zx-spectrum` bus/machine integration

**Files:**
- Modify: `crates/zx-spectrum/Cargo.toml` (add `zx-tape` dependency)
- Modify: `crates/zx-spectrum/src/bus.rs` (tape field, `load_tape`/`set_tape_speed`/`tick_tape`, `read_io` EAR-bit composition)
- Modify: `crates/zx-spectrum/src/lib.rs` (`ZxSpectrum::load_tape`/`set_tape_speed`, `step()` calls `tick_tape`)

**Interfaces:**
- Consumes: `zx_tape::{Tape, TapePlayer}` from Tasks 1-2.
- Produces:
  - `ZxSpectrumBus::load_tape(&mut self, tape: zx_tape::Tape)`, `::set_tape_speed(&mut self, multiplier: u32)`, `::tick_tape(&mut self, cycles: u32)` (all `pub(crate)`)
  - `ZxSpectrum::load_tape(&mut self, tape: zx_tape::Tape)`, `::set_tape_speed(&mut self, multiplier: u32)` (both `pub`)

- [ ] **Step 1: Add the dependency**

Edit `crates/zx-spectrum/Cargo.toml`:

```toml
[dependencies]
z80 = { path = "../z80" }
machine = { path = "../machine" }
sinclair-keyboard = { path = "../sinclair-keyboard" }
z80-snapshot = { path = "../z80-snapshot" }
ay-3-8910 = { path = "../ay-3-8910" }
zx-tape = { path = "../zx-tape" }
```

- [ ] **Step 2: Write the failing tests**

Read `crates/zx-spectrum/src/bus.rs`'s `tests` module first to see the
exact existing test style (`ZxSpectrumBus::new(Model::Spectrum48, ...)`
etc.) -- these new tests must live in that same module. Append:

```rust
    #[test]
    fn tape_ear_bit_is_idle_high_with_no_tape_installed() {
        let mut bus = ZxSpectrumBus::new(Model::Spectrum48, vec![0u8; crate::ROM_SIZE]);
        // Bit 6 set (idle) even before any tape is ever installed.
        assert_eq!(bus.read_io(0x00FE) & 0b0100_0000, 0b0100_0000);
    }

    #[test]
    fn tape_first_read_auto_starts_playback_and_bit6_reflects_the_first_pulse() {
        let mut bus = ZxSpectrumBus::new(Model::Spectrum48, vec![0u8; crate::ROM_SIZE]);
        let tape = zx_tape::parse_tap(&[0x01, 0x00, 0x00]).unwrap(); // starts level=true
        bus.load_tape(tape);

        // Not playing yet -- idle high.
        // (No direct "is playing" accessor; verified indirectly via
        // tick_tape + a second read below, matching how AY state is
        // tested elsewhere in this file.)
        let first_read = bus.read_io(0x00FE);
        assert_eq!(first_read & 0b0100_0000, 0b0100_0000); // first pulse is level=true

        bus.tick_tape(2168); // exactly one pilot pulse -- now level=false
        let second_read = bus.read_io(0x00FE);
        assert_eq!(second_read & 0b0100_0000, 0);
    }

    #[test]
    fn tape_speed_multiplier_is_applied() {
        let mut bus = ZxSpectrumBus::new(Model::Spectrum48, vec![0u8; crate::ROM_SIZE]);
        let tape = zx_tape::parse_tap(&[0x01, 0x00, 0x00]).unwrap();
        bus.load_tape(tape);
        bus.set_tape_speed(4);

        bus.read_io(0x00FE); // auto-starts playback
        bus.tick_tape(2168 / 4); // 1/4 the real cycles, but 4x speed -> one full pulse
        let read = bus.read_io(0x00FE);
        assert_eq!(read & 0b0100_0000, 0); // already on the second pulse (level=false)
    }

    #[test]
    fn tape_does_not_affect_keyboard_bits_of_the_same_read() {
        let mut bus = ZxSpectrumBus::new(Model::Spectrum48, vec![0u8; crate::ROM_SIZE]);
        bus.press_key(ZxSpectrumKey::Q); // row 2 (A10), bit 0
        let tape = zx_tape::parse_tap(&[0x01, 0x00, 0xFF]).unwrap(); // level=true first
        bus.load_tape(tape);

        // Row-2 select mask, same convention as this file's other keyboard tests.
        let value = bus.read_io(0b1111_1011_1111_1110);
        assert_eq!(value & 0b0000_0001, 0); // Q still reads pressed
        assert_eq!(value & 0b0100_0000, 0b0100_0000); // tape bit still correct
    }
```

- [ ] **Step 3: Run the tests to verify they fail to compile**

Run: `cargo test -p zx-spectrum tape`
Expected: FAIL to compile -- `load_tape`/`set_tape_speed`/`tick_tape` don't exist on `ZxSpectrumBus` yet.

- [ ] **Step 4: Implement the bus changes**

In `crates/zx-spectrum/src/bus.rs`:

Add `zx_tape` to the `use` block at the top (alongside the existing
`ay_3_8910` usage, wherever that's referenced -- likely a bare
`ay_3_8910::Ay3_8910` path reference rather than a `use` statement, in
which case no import line is needed here either, just reference
`zx_tape::TapePlayer` by its full path the same way).

Add a `tape` field to `ZxSpectrumBus`'s struct definition, next to `ay`:

```rust
    /// The currently loaded tape, if any -- `None` until `load_tape` is
    /// called. See `read_io`/`tick_tape` for how it drives port `0xFE`
    /// bit 6 (EAR input).
    tape: Option<zx_tape::TapePlayer>,
```

Initialize it in `ZxSpectrumBus::new`'s `Self { ... }` construction, next
to the existing `ay: ...` line:

```rust
            tape: None,
```

Add these methods to `impl ZxSpectrumBus` (near `ay`/`ay_mut`):

```rust
    /// Installs a tape for loading, replacing any tape already installed.
    /// Playback starts automatically the next time port `0xFE` bit 6 is
    /// read (see `read_io`), not immediately -- matches a real cassette
    /// sitting loaded-but-not-playing until Play is pressed.
    pub(crate) fn load_tape(&mut self, tape: zx_tape::Tape) {
        self.tape = Some(zx_tape::TapePlayer::new(tape));
    }

    /// Sets the currently loaded tape's turbo speed multiplier (1 = real
    /// time). A no-op if no tape is installed yet.
    pub(crate) fn set_tape_speed(&mut self, multiplier: u32) {
        if let Some(tape) = &mut self.tape {
            tape.set_speed_multiplier(multiplier);
        }
    }

    /// Advances the currently loaded tape's playback by `cycles` real CPU
    /// cycles. A no-op if no tape is installed. Called once per CPU step
    /// from `ZxSpectrum::step`, same pattern as the AY chip's `tick`.
    pub(crate) fn tick_tape(&mut self, cycles: u32) {
        if let Some(tape) = &mut self.tape {
            tape.tick(cycles);
        }
    }
```

Modify `read_io`'s keyboard branch to compose the EAR bit and auto-start
playback. Replace:

```rust
        if port & 0x0001 == 0 {
            self.keyboard.read(port)
        } else {
            0xFF
        }
```

with:

```rust
        if port & 0x0001 == 0 {
            let mut value = self.keyboard.read(port);
            // Real port 0xFE bit 6 = EAR input (verified against the
            // World of Spectrum 48K reference, see CREDITS.md);
            // `keyboard.read` already leaves bit 6 set (idle-high) by
            // default, so only clear it when a tape's current pulse is
            // low. Reading this bit at all is also what starts playback
            // (see TapePlayer::play's own doc comment for why that's a
            // safe no-op once already playing or finished).
            if let Some(tape) = &mut self.tape {
                tape.play();
                if !tape.ear_bit() {
                    value &= 0b1011_1111;
                }
            }
            value
        } else {
            0xFF
        }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p zx-spectrum tape`
Expected: all 4 new tests PASS.

- [ ] **Step 6: Wire `ZxSpectrum::step` and add its own `load_tape`/`set_tape_speed`**

Read `crates/zx-spectrum/src/lib.rs`'s `step` method and the area around
`load_snapshot`/`set_border` first, to place these consistently with the
existing style.

In `ZxSpectrum::step`, right after the existing AY-tick block
(`if let Some(ay) = self.bus.ay_mut() { ay.tick(ay_cycles); }`) and before
the audio-accumulator block, add:

```rust
        self.bus.tick_tape(cycles);
```

Add these two public methods on `ZxSpectrum` (near `set_border`):

```rust
    /// Installs a tape for loading -- see `crates/zx-tape`. Replaces any
    /// tape already installed. Playback starts automatically the next
    /// time the ROM reads the EAR bit (port 0xFE bit 6), not immediately
    /// on install.
    pub fn load_tape(&mut self, tape: zx_tape::Tape) {
        self.bus.load_tape(tape);
    }

    /// Sets the currently loaded tape's turbo speed multiplier (1 = real
    /// time). Call after `load_tape`; a no-op if no tape is installed.
    pub fn set_tape_speed(&mut self, multiplier: u32) {
        self.bus.set_tape_speed(multiplier);
    }
```

- [ ] **Step 7: Run the full crate test suite**

Run: `cargo test -p zx-spectrum`
Expected: all tests pass (existing tests + the 4 new ones).

- [ ] **Step 8: Lint and format**

Run: `cargo clippy -p zx-spectrum -- -D warnings` -- must be clean.
Run: `cargo fmt -p zx-spectrum -- --check` -- must be clean; if not, run
`cargo fmt -p zx-spectrum` and re-check.

- [ ] **Step 9: Commit**

```bash
git add crates/zx-spectrum
git commit -m "feat(zx-spectrum): tape playback wired into the bus and CPU step loop

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 5: CLI wiring (`--tape`/`--tape-speed`)

**Files:**
- Modify: `apps/retroclawd/Cargo.toml` (add `zx-tape` dependency)
- Modify: `apps/retroclawd/src/any_machine.rs` (`apply_tape` dispatch)
- Modify: `apps/retroclawd/src/main.rs` (`--tape`/`--tape-speed` flags, loading logic)

**Interfaces:**
- Consumes: `zx_tape::{parse_tap, parse_tzx}` (Tasks 1/3), `AnyMachine::ZxSpectrum(ZxSpectrum)`'s `load_tape`/`set_tape_speed` (Task 4).
- Produces: `AnyMachine::apply_tape(&mut self, extension: &str, bytes: &[u8], speed: u32) -> Result<(), String>`

- [ ] **Step 1: Add the dependency**

Edit `apps/retroclawd/Cargo.toml`, adding to `[dependencies]` (alongside
the existing `z80-snapshot`/`zx81-snapshot` lines):

```toml
zx-tape = { path = "../../crates/zx-tape" }
```

- [ ] **Step 2: Write the failing tests**

Read `apps/retroclawd/src/any_machine.rs`'s existing
`apply_snapshot_rejects_wrong_extension_for_zx81`/
`apply_snapshot_loads_a_z80_file_into_zx_spectrum` tests first, to match
their exact construction style. Append to that file's `tests` module:

```rust
    #[test]
    fn apply_tape_rejects_zx81() {
        let mut machine = AnyMachine::Zx81(zx81::Zx81::new(vec![0; zx81::ROM_SIZE], Region::Pal));

        let result = machine.apply_tape("tap", &[0x01, 0x00, 0x00], 1);

        assert!(result.is_err());
    }

    #[test]
    fn apply_tape_rejects_an_unknown_extension() {
        let mut machine = AnyMachine::ZxSpectrum(zx_spectrum::ZxSpectrum::new(
            vec![0; zx_spectrum::ROM_SIZE],
            Region::Pal,
            zx_spectrum::Model::Spectrum48,
        ));

        let result = machine.apply_tape("wav", &[0u8; 4], 1);

        assert!(result.is_err());
    }

    #[test]
    fn apply_tape_loads_a_tap_file_into_zx_spectrum() {
        let mut machine = AnyMachine::ZxSpectrum(zx_spectrum::ZxSpectrum::new(
            vec![0; zx_spectrum::ROM_SIZE],
            Region::Pal,
            zx_spectrum::Model::Spectrum48,
        ));

        let result = machine.apply_tape("tap", &[0x01, 0x00, 0x00], 1);

        assert!(result.is_ok());
    }
```

- [ ] **Step 3: Run the tests to verify they fail to compile**

Run: `cargo test -p retroclawd apply_tape`
Expected: FAIL to compile -- `apply_tape` doesn't exist yet.

- [ ] **Step 4: Implement `AnyMachine::apply_tape`**

In `apps/retroclawd/src/any_machine.rs`, add this method to `impl
AnyMachine`, near `apply_snapshot`:

```rust
    /// Parses `bytes` as a tape in the format `extension` names and
    /// installs it, playing at `speed`x real time (1 = real speed). Same
    /// reasoning as `apply_snapshot`/`set_border`: tape playback is a
    /// concrete per-machine concern (ZX81 has no tape support modeled at
    /// all, parked -- see TODO.md), so the dispatch lives here rather
    /// than on the `Machine` trait.
    pub fn apply_tape(&mut self, extension: &str, bytes: &[u8], speed: u32) -> Result<(), String> {
        match self {
            AnyMachine::Zx81(_) => {
                Err("--tape isn't supported for --machine zx81 yet".to_string())
            }
            AnyMachine::ZxSpectrum(m) => {
                let tape = match extension {
                    "tap" => zx_tape::parse_tap(bytes).map_err(|e| e.to_string())?,
                    "tzx" => zx_tape::parse_tzx(bytes).map_err(|e| e.to_string())?,
                    other => {
                        return Err(format!(
                            "--tape: unsupported extension \".{other}\" (expected .tap or .tzx)"
                        ));
                    }
                };
                m.load_tape(tape);
                m.set_tape_speed(speed);
                Ok(())
            }
        }
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p retroclawd apply_tape`
Expected: all 3 tests PASS.

- [ ] **Step 6: Wire the CLI flags**

In `apps/retroclawd/src/main.rs`'s `Args` struct, add these two fields
right after the existing `snapshot: Option<String>` field:

```rust
    /// Load a tape file, playing it back in real time starting the moment
    /// the ROM first reads the EAR bit (port 0xFE bit 6). Format is picked
    /// by file extension: `.tap` or `.tzx`. Only supported on
    /// --machine zx-spectrum/zx-spectrum-128/zx-spectrum-2a3 (not zx81).
    /// Mutually exclusive with --snapshot (a snapshot replaces machine
    /// state instantly; a tape plays back over real time -- different
    /// enough semantics to not overload one flag).
    #[arg(long, conflicts_with = "snapshot")]
    tape: Option<String>,

    /// Tape turbo speed multiplier (1 = real time, the default). Plays
    /// back the exact same real pulse protocol, just faster -- not a
    /// different loading mechanism. Only meaningful with --tape.
    #[arg(long, default_value_t = 1, requires = "tape")]
    tape_speed: u32,
```

Also change `program_path`'s existing attribute from
`#[arg(conflicts_with = "snapshot")]` to
`#[arg(conflicts_with_all = ["snapshot", "tape"])]` (a tape, like a
snapshot, fully takes over how the machine starts up instead of a
standalone program being poked into a running boot).

Right after the existing `if let Some(path) = &args.snapshot { ... }`
block (the one calling `machine.apply_snapshot`), add:

```rust
    if let Some(path) = &args.tape {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("failed to read {path}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let extension = Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if let Err(e) = machine.apply_tape(&extension, &bytes, args.tape_speed) {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }
```

- [ ] **Step 7: Write CLI arg-parsing tests**

Read the existing `snapshot_with_program_path_is_rejected`/
`snapshot_alone_is_accepted` tests in `main.rs`'s `tests` module first,
then append matching ones:

```rust
    #[test]
    fn tape_with_program_path_is_rejected() {
        let result = Args::try_parse_from([
            "retroclawd",
            "--machine",
            "zx-spectrum",
            "--rom",
            "rom.bin",
            "--tape",
            "game.tap",
            "program.bin",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn tape_with_snapshot_is_rejected() {
        let result = Args::try_parse_from([
            "retroclawd",
            "--machine",
            "zx-spectrum",
            "--rom",
            "rom.bin",
            "--tape",
            "game.tap",
            "--snapshot",
            "game.z80",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn tape_alone_is_accepted() {
        let result = Args::try_parse_from([
            "retroclawd",
            "--machine",
            "zx-spectrum",
            "--rom",
            "rom.bin",
            "--tape",
            "game.tap",
        ]);

        assert!(result.is_ok());
    }

    #[test]
    fn tape_speed_without_tape_is_rejected() {
        let result = Args::try_parse_from([
            "retroclawd",
            "--machine",
            "zx-spectrum",
            "--rom",
            "rom.bin",
            "--tape-speed",
            "10",
        ]);

        assert!(result.is_err());
    }
```

- [ ] **Step 8: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: all tests pass.

- [ ] **Step 9: Lint and format**

Run: `cargo clippy --workspace -- -D warnings` -- must be clean.
Run: `cargo fmt --all -- --check` -- must be clean; if not, run
`cargo fmt --all` and re-check.

- [ ] **Step 10: Build and smoke-test the help text**

Run: `cargo build -p retroclawd` -- must succeed.
Run: `cargo run -p retroclawd -- --help` and confirm `--tape`/
`--tape-speed` appear with readable descriptions.

- [ ] **Step 11: Commit**

```bash
git add apps/retroclawd
git commit -m "feat(retroclawd): --tape/--tape-speed CLI flags

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Post-plan cleanup

- [ ] Update `CLAUDE.md`'s crate-layout section (add `zx-tape`, matching
      the existing `z80-snapshot`/`zx81-snapshot` entries' style) and the
      `zx-spectrum`/`retroclawd` sections (tape playback in `step`/
      `read_io`, the `--tape`/`--tape-speed` flags) once all 5 tasks are
      done and verified. Not its own task above since it's pure
      documentation with no test cycle of its own -- fold it into
      whichever commit is most convenient, or do it as one final commit
      after Task 5.
- [ ] Update `TODO.md`'s "Snapshot / tape / floppy formats as their own
      crates" section to mark `.tap`/`.tzx` RESOLVED once done, mirroring
      how the `.z80`/`.p81` crates' entries read.
- [ ] Manual verification with a real `.tap`/`.tzx` file (not available to
      any automated step in this plan) -- confirm a real game actually
      loads via `LOAD ""` at real speed, and again with `--tape-speed`
      turbo'd up. This needs a human with a real file and a running GUI
      window; no task above can verify it.
