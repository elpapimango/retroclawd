//! Which lines of a frame the Shifter fetches picture data for, and how much it fetches on each: the top and bottom border,
//! and the left and right border of a line. Every rule here is an observation of Hatari 2.6.1 made with the border sweeps
//! P09, P10 and P12 (`docs/hardware-facts.md`, "Timing (Phase 3)"), in the time base of this emulator: the clocks are those
//! at which the Shifter saw the register write.
//!
//! - **Top and bottom.** At the start of every line the sync mode decides: the picture starts at line 34 when it says 60 Hz
//!   and at line 63 when it says 50 Hz, and ends (after 200 lines in a normal frame) at line 234 or 263. A line that starts
//!   with the other frequency misses the start (no picture at all in that frame) or the end (the picture goes on).
//! - **Right.** A line fetches 44 bytes more, to the right of the picture, when the resolution is high at clock 370 and
//!   low again by clock 390, or when the sync mode is 50 Hz at clock 362, 60 Hz at 364 and 50 Hz again at 372.
//! - **Left.** A line fetches 26 bytes more, before the picture, when it begins in high resolution (the register was
//!   switched before the end of the line before) and goes back to low within 16 clocks.
//!
//! A line is drawn in one resolution: the register's value about 464 clocks into it (`Shifter::line_resolution`, probes P35
//! and P45); Hatari draws no part of a line in another resolution.
//!
//! What is not modelled: a switch to high resolution inside the picture, which Hatari answers by cutting the line short
//! (probe P09, frames 16 to 18), a line lost altogether (frames 91 to 119), and the 60 Hz line near the bottom of the frame
//! (P12, line 257).

use serde::{Deserialize, Serialize};

use crate::{SyncRegister, SyncWrite};

/// Bytes a normal picture line holds (320 low resolution pixels in four planes).
pub const LINE_BYTES: u32 = 160;
/// Extra bytes fetched on the left of a line whose left border opened, and on the right of one whose right border opened.
pub const LEFT_BYTES: u32 = 26;
pub const RIGHT_BYTES: u32 = 44;

/// The first picture line of a 50 Hz and of a 60 Hz frame, and the line after the last.
pub const START_50: u16 = 63;
pub const START_60: u16 = 34;
pub const END_50: u16 = 263;
pub const END_60: u16 = 234;

/// The monochrome monitor: the first picture line of its 501-line frame, the picture's lines and the bytes of a line
/// (640 pixels in one plane). It has no borders to open and no 50/60 Hz choice.
pub const MONO_START: u16 = 34;
pub const MONO_LINES: u16 = 400;
pub const MONO_LINE_BYTES: u32 = 80;

const LEFT: u8 = 1;
const RIGHT: u8 = 2;

/// Clocks the right-border rules look at (see the module comment).
const RIGHT_RES_CHECK: u32 = 370;
const RIGHT_RES_LOW_BY: u32 = 390;
const RIGHT_SYNC_BEFORE: u32 = 362;
const RIGHT_SYNC_DURING: u32 = 364;
const RIGHT_SYNC_AFTER: u32 = 372;
/// The left border needs the resolution back to low within this many clocks of the line's start.
const LEFT_LOW_BY: u32 = 16;

/// The picture window of a frame and the borders of each line of it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetch {
    started: bool,
    ended: bool,
    first: u16,
    last: u16,
    /// Flags of the picture lines, in order from the first one: `LEFT`, `RIGHT`.
    lines: Vec<u8>,
    /// A monochrome monitor (set once at power-on from the configuration, not part of a state).
    #[serde(skip)]
    mono: bool,
}

/// The value of a register at `clock` of a line that began with `start`, after the writes of the line (`SyncRegister`
/// `reg`) made at or before that clock.
fn value_at<'a>(
    writes: impl Iterator<Item = &'a SyncWrite>,
    reg: SyncRegister,
    start: u8,
    line: u16,
    clock: u32,
) -> u8 {
    let mut v = start;
    for w in writes {
        if w.line == line && w.register == reg && w.cycle <= clock {
            v = w.value;
        }
    }
    v
}

impl Fetch {
    /// A new frame: nothing has started.
    pub fn reset(&mut self) {
        *self = Self {
            mono: self.mono,
            ..Self::default()
        };
    }

    /// The monitor is monochrome: the picture is a fixed window of 400 lines of 80 bytes.
    pub fn set_mono(&mut self, mono: bool) {
        self.mono = mono;
    }

    /// Bytes of a picture line without borders.
    pub fn line_bytes(&self) -> u32 {
        if self.mono {
            MONO_LINE_BYTES
        } else {
            LINE_BYTES
        }
    }

    /// Whether picture data is being fetched on `line`.
    pub fn fetching(&self, line: u16) -> bool {
        self.started && line >= self.first && (!self.ended || line < self.last)
    }

    /// The frame's first picture line, once it has begun.
    pub fn first_line(&self) -> Option<u16> {
        self.started.then_some(self.first)
    }

    /// The vertical rules at the start of `line`, with the sync mode in force then (`hz60`).
    pub fn start_line(&mut self, line: u16, hz60: bool) {
        if self.mono {
            if line == MONO_START && !self.started {
                self.started = true;
                self.first = line;
            } else if self.started && !self.ended && line == MONO_START + MONO_LINES {
                self.ended = true;
                self.last = line;
            }
            return;
        }
        if !self.started && !self.ended {
            if line == if hz60 { START_60 } else { START_50 } {
                self.started = true;
                self.first = line;
            }
        } else if self.started && !self.ended && line == if hz60 { END_60 } else { END_50 } {
            self.ended = true;
            self.last = line;
        }
    }

    /// The horizontal rules for `line`, which has just ended: `res_start` and `sync_start` are the registers as the line
    /// began, `log` the writes of the frame. The line must be a picture line.
    pub fn end_line<'a>(
        &mut self,
        line: u16,
        res_start: u8,
        sync_start: u8,
        log: impl Iterator<Item = &'a SyncWrite> + Clone,
    ) {
        if !self.fetching(line) {
            return;
        }
        let index = (line - self.first) as usize;
        if self.lines.len() > index {
            return;
        }
        self.lines.resize(index + 1, 0);
        self.lines[index] = if self.mono {
            0
        } else {
            Self::flags(line, res_start, sync_start, log)
        };
    }

    /// The borders (`LEFT`, `RIGHT`) a line opens with the register writes made on it so far.
    pub fn flags<'a>(
        line: u16,
        res_start: u8,
        sync_start: u8,
        log: impl Iterator<Item = &'a SyncWrite> + Clone,
    ) -> u8 {
        let res = |clock| {
            value_at(
                log.clone(),
                SyncRegister::Resolution,
                res_start,
                line,
                clock,
            )
        };
        let sync = |clock| value_at(log.clone(), SyncRegister::Sync, sync_start, line, clock);
        let mut flags = 0;
        // Left: the line began in high resolution and low is back within a few clocks.
        if res_start == 2 && res(LEFT_LOW_BY) != 2 {
            flags |= LEFT;
        }
        // Right: high resolution at 370 and low by 390, or the 50, 60, 50 Hz sequence.
        let right_res = res(RIGHT_RES_CHECK) == 2 && res(RIGHT_RES_LOW_BY) != 2;
        let right_sync = sync(RIGHT_SYNC_BEFORE) == 2
            && sync(RIGHT_SYNC_DURING) == 0
            && sync(RIGHT_SYNC_AFTER) == 2;
        if right_res || right_sync {
            flags |= RIGHT;
        }
        flags
    }

    /// `flags` as (left, right).
    pub fn split(flags: u8) -> (bool, bool) {
        (flags & LEFT != 0, flags & RIGHT != 0)
    }

    /// Whether `line`'s left and right border opened.
    pub fn borders(&self, line: u16) -> (bool, bool) {
        if !self.started || line < self.first {
            return (false, false);
        }
        let f = self
            .lines
            .get((line - self.first) as usize)
            .copied()
            .unwrap_or(0);
        (f & LEFT != 0, f & RIGHT != 0)
    }

    /// The picture lines before `line` (empty before the picture starts).
    pub fn lines_before(&self, line: u16) -> std::ops::Range<u16> {
        if !self.started || line <= self.first {
            return 0..0;
        }
        let end = if self.ended {
            self.last.min(line)
        } else {
            line
        };
        self.first..end
    }

    /// The bytes fetched on the picture lines before `line` (zero before the picture starts).
    pub fn bytes_before(&self, line: u16) -> u32 {
        if !self.started || line <= self.first {
            return 0;
        }
        let end = if self.ended {
            self.last.min(line)
        } else {
            line
        };
        (self.first..end)
            .map(|l| {
                let (left, right) = self.borders(l);
                self.line_bytes()
                    + if left { LEFT_BYTES } else { 0 }
                    + if right { RIGHT_BYTES } else { 0 }
            })
            .sum()
    }
}
