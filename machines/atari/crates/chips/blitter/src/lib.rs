//! The Atari Blitter: halftone RAM, source and destination counters, the end masks, HOP, OP, skew and the engine that moves
//! one destination word at a time over a [`BlitBus`] (`docs/hardware-facts.md`, "Blitter").
//!
//! The register map is [H] *Atari ST Internals* (the Blitter chapter): halftone RAM `$FF8A00-1F`, source x/y increments
//! `$20/$22`, source address `$24`, end masks `$28/$2A/$2C`, destination x/y increments `$2E/$30`, destination address
//! `$32`, counts `$36/$38`, HOP `$3A`, OP `$3B`, control `$3C` (bits 3-0 the halftone line, 5 smudge, 6 HOG, 7 busy) and
//! skew `$3D` (bits 3-0 the skew, 6 NFSR, 7 FXSR). What the engine does with them is [O], measured against Hatari 2.6.1
//! by the probes P33 (174 settings), P37 (the skew and its two flags, raw), P38 (end masks with them) and P39 (right to
//! left): the source shifter is 32 bits wide and persists between lines and between blits; FXSR reads one word
//! before a line, NFSR skips the last read of a line and the shifter then takes the last word that crossed the data bus
//! (the destination word read for a partial mask, else the one just written); a negative destination x increment runs the
//! line right to left with the newest word in the shifter's upper half. Time is not modelled here: the machine decides how
//! many words [`Blitter::run`] may move at once.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Memory as the Blitter sees it: whole words at even 24-bit addresses.
pub trait BlitBus {
    fn read_word(&mut self, addr: u32) -> u16;
    fn write_word(&mut self, addr: u32, value: u16);
}

/// What a count of 0 means.
const COUNT_ZERO: u32 = 65536;

/// The Blitter as data (also its save state).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blitter {
    halftone: [u16; 16],
    src_x_inc: u16,
    src_y_inc: u16,
    src_addr: u32,
    end_mask: [u16; 3],
    dst_x_inc: u16,
    dst_y_inc: u16,
    dst_addr: u32,
    x_count: u16,
    y_count: u16,
    hop: u8,
    op: u8,
    /// Bits 3-0 the halftone line, 5 smudge, 6 HOG.
    control: u8,
    /// Bits 3-0 the skew, 6 NFSR, 7 FXSR.
    skew: u8,
    busy: bool,
    /// The Y count ran down to zero in a blit and has not been written since: [O] P40 a start then moves nothing.
    y_spent: bool,
    /// The source shifter: the older and the newer word.
    older: u16,
    newer: u16,
    /// The last word that crossed the data bus.
    latch: u16,
    /// The line being run (`None` between lines).
    line: Option<LineProgress>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LineProgress {
    /// Words done, the line's length, and the source reads made.
    x: u32,
    len: u32,
    reads: u32,
}

impl Default for Blitter {
    fn default() -> Self {
        Self {
            halftone: [0; 16],
            src_x_inc: 0,
            src_y_inc: 0,
            src_addr: 0,
            end_mask: [0xFFFF; 3],
            dst_x_inc: 0,
            dst_y_inc: 0,
            dst_addr: 0,
            x_count: 0,
            y_count: 0,
            hop: 0,
            op: 0,
            control: 0,
            skew: 0,
            busy: false,
            y_spent: false,
            older: 0,
            newer: 0,
            latch: 0,
            line: None,
        }
    }
}

/// The logical operation `op`: the sixteen functions of source `s` and destination `d`.
fn logic(op: u8, s: u16, d: u16) -> u16 {
    match op & 15 {
        0 => 0,
        1 => s & d,
        2 => s & !d,
        3 => s,
        4 => !s & d,
        5 => d,
        6 => s ^ d,
        7 => s | d,
        8 => !(s | d),
        9 => !(s ^ d),
        10 => !d,
        11 => s | !d,
        12 => !s,
        13 => !s | d,
        14 => !(s & d),
        _ => 0xFFFF,
    }
}

/// Whether the logical operation looks at the destination.
fn op_uses_destination(op: u8) -> bool {
    !matches!(op & 15, 0 | 3 | 12 | 15)
}

fn step(addr: u32, inc: u16) -> u32 {
    addr.wrapping_add(i32::from(inc as i16) as u32) & 0xFF_FFFF
}

impl Blitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    /// Whether a state (untrusted) can have come from this engine: a line in progress belongs to a busy blit and its
    /// position lies inside it.
    pub fn is_consistent(&self) -> bool {
        match self.line {
            None => true,
            Some(p) => {
                self.busy && p.len >= 1 && p.len <= COUNT_ZERO && p.x < p.len && p.reads <= p.x + 1
            }
        }
    }

    /// Whether the blit that is busy runs with HOG set (the CPU gets no bus cycles until it ends).
    pub fn hog(&self) -> bool {
        self.control & 0x40 != 0
    }

    /// The register byte at offset `off` from `$FF8A00` (0 to 63).
    pub fn read_byte(&self, off: usize) -> u8 {
        let high = |w: u16| (w >> 8) as u8;
        let long = |l: u32, i: usize| (l >> (8 * (3 - i))) as u8;
        match off {
            0x00..=0x1F => {
                let w = self.halftone[off / 2];
                if off.is_multiple_of(2) {
                    high(w)
                } else {
                    w as u8
                }
            }
            0x20 => high(self.src_x_inc),
            0x21 => self.src_x_inc as u8,
            0x22 => high(self.src_y_inc),
            0x23 => self.src_y_inc as u8,
            0x24..=0x27 => long(self.src_addr, off - 0x24),
            0x28 => high(self.end_mask[0]),
            0x29 => self.end_mask[0] as u8,
            0x2A => high(self.end_mask[1]),
            0x2B => self.end_mask[1] as u8,
            0x2C => high(self.end_mask[2]),
            0x2D => self.end_mask[2] as u8,
            0x2E => high(self.dst_x_inc),
            0x2F => self.dst_x_inc as u8,
            0x30 => high(self.dst_y_inc),
            0x31 => self.dst_y_inc as u8,
            0x32..=0x35 => long(self.dst_addr, off - 0x32),
            0x36 => high(self.x_count),
            0x37 => self.x_count as u8,
            0x38 => high(self.y_count),
            0x39 => self.y_count as u8,
            0x3A => self.hop,
            0x3B => self.op,
            0x3C => self.control | if self.busy { 0x80 } else { 0 },
            0x3D => self.skew,
            _ => 0,
        }
    }

    /// A register byte written at `off`. Returns true when the write starts a blit (the busy bit set in the control
    /// register while idle): the caller then runs it with [`Blitter::run`].
    pub fn write_byte(&mut self, off: usize, v: u8) -> bool {
        fn put(w: &mut u16, high: bool, v: u8) {
            *w = if high {
                (*w & 0x00FF) | u16::from(v) << 8
            } else {
                (*w & 0xFF00) | u16::from(v)
            };
        }
        fn put_long(l: &mut u32, i: usize, v: u8) {
            let shift = 8 * (3 - i);
            // [H?] the addresses are even: bit 0 is not stored.
            *l = ((*l & !(0xFF << shift)) | u32::from(v) << shift) & 0xFF_FFFE;
        }
        match off {
            0x00..=0x1F => put(&mut self.halftone[off / 2], off.is_multiple_of(2), v),
            0x20 | 0x21 => put(&mut self.src_x_inc, off == 0x20, v),
            0x22 | 0x23 => put(&mut self.src_y_inc, off == 0x22, v),
            0x24..=0x27 => put_long(&mut self.src_addr, off - 0x24, v),
            0x28 | 0x29 => put(&mut self.end_mask[0], off == 0x28, v),
            0x2A | 0x2B => put(&mut self.end_mask[1], off == 0x2A, v),
            0x2C | 0x2D => put(&mut self.end_mask[2], off == 0x2C, v),
            0x2E | 0x2F => put(&mut self.dst_x_inc, off == 0x2E, v),
            0x30 | 0x31 => put(&mut self.dst_y_inc, off == 0x30, v),
            0x32..=0x35 => put_long(&mut self.dst_addr, off - 0x32, v),
            0x36 | 0x37 => put(&mut self.x_count, off == 0x36, v),
            0x38 | 0x39 => {
                put(&mut self.y_count, off == 0x38, v);
                self.y_spent = false;
            }
            0x3A => self.hop = v & 3,
            0x3B => self.op = v & 15,
            0x3C => {
                if self.busy {
                    // [H?] a write while a blit runs changes nothing.
                    return false;
                }
                self.control = v & 0x6F;
                // [O] P40: a start with the Y count left at zero by the last blit moves nothing; a zero written means 65536.
                if v & 0x80 != 0 && !self.y_spent {
                    self.busy = true;
                    self.line = None;
                    return true;
                }
            }
            0x3D => self.skew = v & 0xCF,
            _ => {}
        }
        false
    }

    /// A register written as a byte or a word (a long is two words). The skew byte goes in before the control byte, so a
    /// word written to `$FF8A3C` starts the blit with its skew.
    pub fn write(&mut self, off: usize, word: bool, value: u16) -> bool {
        if !word {
            return self.write_byte(off, value as u8);
        }
        if off == 0x3C {
            self.write_byte(0x3D, value as u8);
            return self.write_byte(0x3C, (value >> 8) as u8);
        }
        let a = self.write_byte(off, (value >> 8) as u8);
        self.write_byte(off + 1, value as u8) || a
    }

    /// A register read as a byte or a word.
    pub fn read(&self, off: usize, word: bool) -> u16 {
        if word {
            u16::from(self.read_byte(off)) << 8 | u16::from(self.read_byte(off + 1))
        } else {
            u16::from(self.read_byte(off))
        }
    }

    /// Whether the destination word is fed by the source: HOP 2 and 3 with an operation that uses it, or smudge.
    fn uses_source(&self) -> bool {
        (self.hop >= 2 && !matches!(self.op, 0 | 5 | 10 | 15)) || self.control & 0x20 != 0
    }

    fn read_source(&mut self, bus: &mut impl BlitBus, reads: &mut u32) -> u16 {
        if *reads > 0 {
            self.src_addr = step(self.src_addr, self.src_x_inc);
        }
        *reads += 1;
        let w = bus.read_word(self.src_addr & 0xFF_FFFE);
        self.latch = w;
        w
    }

    /// Moves up to `words` destination words of the blit that is busy; returns how many it moved. The blit ends (the busy
    /// bit clears, HOG with it) after its last line.
    pub fn run(&mut self, bus: &mut impl BlitBus, words: u32) -> u32 {
        let mut moved = 0;
        while self.busy && moved < words {
            self.word(bus);
            moved += 1;
        }
        moved
    }

    fn word(&mut self, bus: &mut impl BlitBus) {
        let uses_source = self.uses_source();
        let skew = u32::from(self.skew & 15);
        let fxsr = self.skew & 0x80 != 0;
        let nfsr = self.skew & 0x40 != 0;
        let reverse = (self.dst_x_inc as i16) < 0;
        let mut p = match self.line {
            Some(p) => p,
            None => {
                let len = if self.x_count == 0 {
                    COUNT_ZERO
                } else {
                    u32::from(self.x_count)
                };
                let mut p = LineProgress {
                    x: 0,
                    len,
                    reads: 0,
                };
                if uses_source && fxsr {
                    self.newer = self.read_source(bus, &mut p.reads);
                }
                p
            }
        };
        let last = p.x == p.len - 1;
        let mask = if p.x == 0 {
            self.end_mask[0]
        } else if last {
            self.end_mask[2]
        } else {
            self.end_mask[1]
        };
        let needs_dest = mask != 0xFFFF || op_uses_destination(self.op);
        let dest = if needs_dest {
            bus.read_word(self.dst_addr & 0xFF_FFFE)
        } else {
            0
        };
        let skipped_read = uses_source && nfsr && last;
        let source = if uses_source {
            if skipped_read {
                if needs_dest {
                    self.latch = dest;
                }
                self.older = self.newer;
                self.newer = self.latch;
            } else {
                let w = self.read_source(bus, &mut p.reads);
                self.older = self.newer;
                self.newer = w;
            }
            let pair = if reverse {
                u32::from(self.newer) << 16 | u32::from(self.older)
            } else {
                u32::from(self.older) << 16 | u32::from(self.newer)
            };
            (pair >> skew) as u16
        } else {
            0
        };
        if needs_dest {
            self.latch = dest;
        }
        let row = if self.control & 0x20 != 0 {
            usize::from(source & 15)
        } else {
            usize::from(self.control & 15)
        };
        let half = self.halftone[row];
        let fed = match self.hop {
            0 => 0xFFFF,
            1 => half,
            2 => source,
            _ => source & half,
        };
        let written = (dest & !mask) | (logic(self.op, fed, dest) & mask);
        bus.write_word(self.dst_addr & 0xFF_FFFE, written);
        self.latch = written;
        if skipped_read {
            self.newer = written;
        }
        p.x += 1;
        if p.x < p.len {
            self.dst_addr = step(self.dst_addr, self.dst_x_inc);
            self.line = Some(p);
            return;
        }
        // The end of the line: no x increment after the last word.
        if uses_source && p.reads > 0 {
            self.src_addr = step(self.src_addr, self.src_y_inc);
        }
        self.dst_addr = step(self.dst_addr, self.dst_y_inc);
        let down = (self.dst_y_inc as i16) < 0;
        let line = (self.control & 15).wrapping_add(if down { 15 } else { 1 }) & 15;
        self.control = self.control & 0xF0 | line;
        self.y_count = self.y_count.wrapping_sub(1);
        self.line = None;
        if self.y_count == 0 {
            self.busy = false;
            self.y_spent = true;
            self.control &= 0x3F;
        }
    }
}
