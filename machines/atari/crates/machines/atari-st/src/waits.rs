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

/// Clocks an ACIA access takes beyond the E clock edge it waits for. [O] P18: values 6 to 8 fit Hatari's loop lengths for five
/// alignments of the access (an ACIA read in a counted loop takes 50 clocks, 60 with one NOP more); 8, the same as the
/// interrupt acknowledge's, the other access that waits for the E clock.
pub(crate) const ACIA_EXTRA: u32 = 8;

/// The E clock's phase against the start of the frame: its edges fall at this many clocks past a multiple of 10 (probe
/// P02, fitted together with the VBL delay below).
pub(crate) const E_PHASE: u64 = 6;

/// Clocks between the end of a frame (the line-0 HBL) and the VBL request: the GLUE raises the VBL a little into the
/// first line, so the HBL of the line boundary is served first and an HBL counted from the VBL numbers the lines as
/// Hatari does (probes P02 and P04: with no delay HBL number n came one line early).
pub(crate) const VBL_DELAY: u64 = 64;

/// Clocks a 16-byte floppy DMA burst holds the CPU off the bus: 8 words at 4 clocks. [O] Hatari 2.6.1, probe P16: a counted
/// loop loses 1060 clocks in a frame in which the DMA moved 528 bytes (33 bursts), 32 clocks a burst.
pub(crate) const DMA_BURST_STALL: u64 = 32;

/// The clocks into a line at which the Shifter starts fetching the picture: a fit to probe P04 (Hatari), within
/// one word.
pub(crate) const WINDOW_START: u64 = 56;

/// The clock of a line at which the first pixel of the picture is on the screen: the Shifter takes 23 clocks from
/// fetching a word to drawing it. A palette write at clock `c` reaches the pixel `c - PIXEL_START` (probe P07: all 122
/// rows of Hatari's strips start and end on the same pixel).
pub(crate) const PIXEL_START: i32 = WINDOW_START as i32 + 23;

/// The Shifter fetches one word of the picture every 4 clocks inside its window (160 bytes a line in the colour
/// resolutions).
pub(crate) const CLOCKS_PER_WORD: u64 = 4;
pub(crate) const WORDS_PER_LINE: u64 = 80;
/// The monochrome monitor's line: its fetch starts at this clock ([O] P15, fitted) and holds 40 words.
pub(crate) const MONO_WINDOW_START: u64 = 8;
pub(crate) const MONO_WORDS_PER_LINE: u64 = 40;

impl StBus {
    /// The video address counter at the current time: the address of the next word the Shifter reads. Before the first
    /// line of the picture it holds the frame's base address, inside a line it moves one word every 4 clocks from the
    /// start of the fetch window, after the last line it holds the end of the picture.
    pub(crate) fn video_counter(&self) -> u32 {
        self.video_counter_unadjusted()
            .wrapping_add(self.shifter.counter_adjust() as u32)
            & 0xFF_FFFF
    }

    /// A byte written to the video counter (STE): the counter takes it now and runs on from there until the frame loads it
    /// from the base. [O] P27: the high byte keeps six bits, the low byte's bit 0 reads 0.
    pub(crate) fn write_video_counter(&mut self, addr: u32, value: u8) {
        let now = self.video_counter();
        let shift = 8 * ((0xFF_8209 - addr) / 2);
        let mut new = (now & !(0xFF << shift)) | (u32::from(value) << shift);
        new &= 0x3F_FFFE;
        let run = self.video_counter_unadjusted();
        let adjust = (new.wrapping_sub(run) & 0xFF_FFFF) as i32;
        self.shifter.set_counter_adjust(adjust);
    }

    fn video_counter_unadjusted(&self) -> u32 {
        let base = self.shifter.video_base();
        let fetch = self.shifter.fetch();
        let line = self.line as u16;
        if !fetch.fetching(line) {
            // Before the picture: the base; after it: past its last line.
            return base.wrapping_add(self.shifter.bytes_before(u16::MAX));
        }
        let mono = self.monitor == st_shifter::Monitor::Mono;
        let (left, right) = self.shifter.current_borders();
        let line_start = self.last_vbl_at + self.line as u64 * self.geometry.clocks_per_line as u64;
        let into = self.now.saturating_sub(line_start);
        // A left border opened starts the fetch 52 clocks sooner, a right border runs it 88 clocks longer.
        let start = if mono {
            MONO_WINDOW_START
        } else {
            WINDOW_START
        } - if left { 52 } else { 0 };
        let line_words = if mono {
            MONO_WORDS_PER_LINE
        } else {
            WORDS_PER_LINE
        };
        let words = (into.saturating_sub(start) / CLOCKS_PER_WORD)
            .min(line_words + if left { 13 } else { 0 } + if right { 22 } else { 0 });
        base.wrapping_add(self.shifter.bytes_before(line) + 2 * words as u32)
    }

    /// The clocks a bus cycle that would start now at `addr` waits first (`write`: the CPU writes).
    pub(crate) fn wait_clocks(&self, addr: u32, size: Size, write: bool) -> u32 {
        let t = self.now;
        match self.region(addr) {
            Region::Io => match io::decode_for(self.chipset, addr & 0xFF_FFFF, size) {
                Decode::Device(Device::Mfp | Device::Psg) => align4(t) + PERIPHERAL_EXTRA,
                Decode::Device(Device::Acia) => {
                    let a = align4(t);
                    a + self.e_clock_wait(t + a as u64) + ACIA_EXTRA
                }
                // [O] P18: the FDC/sector count data register (`$FF8604`) and a write of the mode register (`$FF8606`)
                // take the extra 4 clocks; the status read, the address counter and the other registers do not.
                Decode::Device(Device::Dma)
                    if (addr & 0xFF) == 0x04 || (write && (addr & 0xFF) == 0x06) =>
                {
                    align4(t) + PERIPHERAL_EXTRA
                }
                _ => align4(t),
            },
            _ => align4(t),
        }
    }

    /// The clocks from `t` to the next edge of the E clock (0 when `t` is on one).
    fn e_clock_wait(&self, t: u64) -> u32 {
        let phase = (t + E_PERIOD - E_PHASE - self.last_vbl_at % E_PERIOD) % E_PERIOD;
        ((E_PERIOD - phase) % E_PERIOD) as u32
    }

    /// The clocks the interrupt acknowledge that would start now waits: for the next E clock edge, counted from the
    /// frame start, and then `IACK_EXTRA`.
    pub(crate) fn iack_wait_clocks(&self) -> u32 {
        let phase = (self.now + E_PERIOD - E_PHASE - self.last_vbl_at % E_PERIOD) % E_PERIOD;
        ((E_PERIOD - phase) % E_PERIOD) as u32 + IACK_EXTRA
    }
}
