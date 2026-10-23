//! Wait states of the bus cycles (`accurate` mode). Every rule here is an observation of Hatari 2.6.1 made with the
//! timing probes (`docs/hardware-facts.md`, "Timing (Phase 3)"), not a datasheet fact.

use crate::bus::{Region, StBus};
use crate::io::{self, Decode, Device};
use mc68000::Size;

/// A bus cycle starts on a multiple of 4 clocks: the MMU and the Shifter share the RAM in 4-clock slots, so a cycle that
/// would start between two of them waits for the next (probe P02: every instruction takes a multiple of 4 clocks).
pub(crate) fn align4(t: u64) -> u32 {
    ((4 - (t & 3)) & 3) as u32
}

/// Extra clocks of a cycle to the MFP, the YM2149 or an ACIA: they sit on the slow bus (probe P02, variants 13, 16, 17:
/// an access to them costs 4 clocks more than one to RAM).
pub(crate) const PERIPHERAL_EXTRA: u32 = 4;

/// The 68000's E clock is a tenth of the CPU clock; the acknowledge of an autovectored interrupt waits for it.
pub(crate) const E_PERIOD: u64 = 10;

/// Clocks the acknowledge of an interrupt takes beyond the 4 of the cycle itself, once it has found the E clock (probe
/// P02: the mean number of loop runs a line fits Hatari's to 0.01 with this value).
pub(crate) const IACK_EXTRA: u32 = 8;

/// The E clock's phase against the start of the frame: its edges fall at this many clocks past a multiple of 10 (probe
/// P02, fitted together with the VBL delay below).
pub(crate) const E_PHASE: u64 = 6;

/// Clocks between the end of a frame (the line-0 HBL) and the VBL request: the GLUE raises the VBL a little into the
/// first line, so the HBL of the line boundary is served first and an HBL counted from the VBL numbers the lines as
/// Hatari does (probes P02 and P04: with no delay HBL number n came one line early).
pub(crate) const VBL_DELAY: u64 = 64;

/// The clocks into a line at which the Shifter starts fetching the picture: a fit to probe P04 (Hatari), within
/// one word.
pub(crate) const WINDOW_START: u64 = 56;

/// The Shifter fetches one word of the picture every 4 clocks inside its window (160 bytes a line in the colour
/// resolutions).
pub(crate) const CLOCKS_PER_WORD: u64 = 4;
pub(crate) const WORDS_PER_LINE: u64 = 80;
/// The picture's lines (colour monitor).
pub(crate) const PICTURE_LINES: u32 = 200;

impl StBus {
    /// The video address counter at the current time: the address of the next word the Shifter reads. Before the first
    /// line of the picture it holds the frame's base address, inside a line it moves one word every 4 clocks from the
    /// start of the fetch window, after the last line it holds the end of the picture.
    pub(crate) fn video_counter(&self) -> u32 {
        let first = crate::scheduler::first_display_line(self.geometry.lines);
        let base = self.shifter.video_base();
        if self.line < first {
            return base;
        }
        let n = self.line - first;
        if n >= PICTURE_LINES {
            return base.wrapping_add(PICTURE_LINES * 160);
        }
        let line_start = self.last_vbl_at + self.line as u64 * self.geometry.clocks_per_line as u64;
        let into = self.now.saturating_sub(line_start);
        let words = (into.saturating_sub(WINDOW_START) / CLOCKS_PER_WORD).min(WORDS_PER_LINE);
        base.wrapping_add(n * 160 + 2 * words as u32)
    }

    /// The clocks a bus cycle that would start now at `addr` waits first.
    pub(crate) fn wait_clocks(&self, addr: u32, size: Size) -> u32 {
        let t = self.now;
        match self.region(addr) {
            Region::Io => match io::decode(addr & 0xFF_FFFF, size) {
                Decode::Device(Device::Mfp | Device::Psg | Device::Acia) => {
                    align4(t) + PERIPHERAL_EXTRA
                }
                _ => align4(t),
            },
            _ => align4(t),
        }
    }

    /// The clocks the interrupt acknowledge that would start now waits: for the next E clock edge, counted from the
    /// frame start, and then `IACK_EXTRA`.
    pub(crate) fn iack_wait_clocks(&self) -> u32 {
        let phase = (self.now + E_PERIOD - E_PHASE - self.last_vbl_at % E_PERIOD) % E_PERIOD;
        ((E_PERIOD - phase) % E_PERIOD) as u32 + IACK_EXTRA
    }
}
