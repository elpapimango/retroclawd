//! Machine time and the event queue (spec section 8).
//!
//! The master unit is one CPU clock, counted in a `u64` (about 73 000 years at 8 MHz). Every event has a time, a
//! class that orders events falling on the same clock, and an insertion number that orders the rest, so the order
//! is fully determined. Nothing reads the host clock.
//!
//! Frame timing (colour monitor). Sources are community measurements, two independent threads agreeing for PAL:
//! "Main HW / Emulator constants" (https://www.atari-forum.com/viewtopic.php?t=24853): the PAL crystal is
//! 32.084988 MHz, the CPU clock a quarter of it (8.021247 MHz), and a PAL frame 160256 CPU clocks; "Monochrome screen
//! timings" (https://www.atari-forum.com/viewtopic.php?t=12243): 512 clocks per line and 313 lines at 50 Hz, 508 and
//! 263 at 60 Hz. The NTSC crystal is given as 32.0424 and 32.04245 MHz by different posts
//! (https://forums.atariage.com/topic/353761-mystery-of-the-sts-weird-clock-oscillator-frequency-choices); 32.0424
//! is used until Atari's hardware specification settles it. To verify against that specification.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::config::Region;

pub const PAL_CRYSTAL_HZ: u32 = 32_084_988;
pub const NTSC_CRYSTAL_HZ: u32 = 32_042_400;
/// CPU clock = crystal / 4.
pub const CPU_DIVIDER: u32 = 4;

/// CPU clocks per frame: 512 x 313 at 50 Hz, 508 x 263 at 60 Hz.
pub fn cycles_per_frame(region: Region) -> u64 {
    match region {
        Region::Pal => 512 * 313,
        Region::Ntsc => 508 * 263,
    }
}

pub fn crystal_hz(region: Region) -> u32 {
    match region {
        Region::Pal => PAL_CRYSTAL_HZ,
        Region::Ntsc => NTSC_CRYSTAL_HZ,
    }
}

/// Classes order events on the same clock: video, then the MFP, ACIA, FDC, DMA, then CPU interrupt sampling.
pub const CLASS_VIDEO: u8 = 0;
pub const CLASS_MFP: u8 = 1;
pub const CLASS_ACIA: u8 = 2;
pub const CLASS_FDC: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Event {
    /// The end of a line: the GLUE requests the HBL; after the frame's last line also the VBL.
    LineEnd,
    /// The MFP's next timer timeout; the number tells a stale event (one scheduled before a later MFP write) apart.
    MfpTimeout(u64),
    /// ACIA `port` has a character starting or completing; the generation tells a stale event apart.
    AciaService { port: u8, generation: u64 },
    /// The floppy controller's next byte time while it works; the generation tells a stale event apart.
    FdcTick(u64),
}

/// The shape of one frame in CPU clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameGeometry {
    pub clocks_per_line: u32,
    pub lines: u32,
}

impl FrameGeometry {
    pub fn clocks(&self) -> u64 {
        self.clocks_per_line as u64 * self.lines as u64
    }
}

/// The frame for the sync mode (`$FF820A` bit 1) and the monitor. [H] colour 512 x 313 at 50 Hz, 508 x 263 at 60 Hz
/// (sources above). A mono monitor's line is 224 clocks (same thread), but its line count has no source yet, so it is
/// refused rather than guessed.
pub fn geometry(sync_50hz: bool, monitor: st_shifter::Monitor) -> Result<FrameGeometry, String> {
    match monitor {
        st_shifter::Monitor::Colour if sync_50hz => Ok(FrameGeometry {
            clocks_per_line: 512,
            lines: 313,
        }),
        st_shifter::Monitor::Colour => Ok(FrameGeometry {
            clocks_per_line: 508,
            lines: 263,
        }),
        st_shifter::Monitor::Mono => Err(
            "the mono monitor is not supported yet: the number of lines in its frame has no hardware source (docs/hardware-facts.md)"
                .to_string(),
        ),
    }
}

#[derive(Debug, Default)]
pub struct Scheduler {
    heap: BinaryHeap<Reverse<(u64, u8, u64, Event)>>,
    seq: u64,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn schedule(&mut self, at: u64, class: u8, event: Event) {
        self.heap.push(Reverse((at, class, self.seq, event)));
        self.seq += 1;
    }

    /// The time of the earliest event.
    pub fn next_at(&self) -> Option<u64> {
        self.heap.peek().map(|Reverse((at, ..))| *at)
    }

    /// The earliest event due at or before `now`, with its time and class.
    pub fn pop_due(&mut self, now: u64) -> Option<(u64, u8, Event)> {
        if self.next_at()? > now {
            return None;
        }
        self.heap
            .pop()
            .map(|Reverse((at, class, _, event))| (at, class, event))
    }
}
