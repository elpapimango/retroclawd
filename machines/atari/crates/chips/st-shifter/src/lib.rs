//! The Atari ST Shifter's registers and a frame render. Facts and their sources: `docs/hardware-facts.md` of
//! retroclawd-atari, "Shifter"; pages are those of *Atari ST Internals* (Abacus 1985).
//!
//! The picture is drawn once per frame onto a fixed 640 × 400 canvas: low resolution pixel-doubled, medium
//! resolution line-doubled. The palette is followed line by line: every palette write is logged with the raster line
//! it happened on, and [`Shifter::render_raster`] applies each change from its line down (one line is the
//! granularity; a change inside a line is Phase 3). Everything else is taken as the registers are at the end of the
//! frame.
#![forbid(unsafe_code)]

mod fetch;
mod overscan;
pub use fetch::{
    Fetch, LEFT_BYTES, LINE_BYTES, MONO_LINE_BYTES, MONO_LINES, MONO_START, RIGHT_BYTES,
};
pub use overscan::{OVERSCAN_FIRST_LINE, OVERSCAN_HEIGHT, OVERSCAN_WIDTH};

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 400;

/// `$FF8260` bits 1-0 (p.58).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    /// 320 × 200, four planes.
    Low,
    /// 640 × 200, two planes.
    Medium,
    /// 640 × 400, one plane.
    High,
    /// `11`, reserved: no picture ([H?]).
    Reserved,
}

impl Resolution {
    /// The resolution `$FF8260` bits 1-0 select.
    fn of(register: u8) -> Self {
        match register & 3 {
            0 => Resolution::Low,
            1 => Resolution::Medium,
            2 => Resolution::High,
            _ => Resolution::Reserved,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Monitor {
    Colour,
    Mono,
}

/// The most palette changes a frame's log keeps: a program that writes more (a raster routine changing all sixteen
/// colours on every line needs 3 200) loses the picture's later changes, not the palette itself.
pub const MAX_PALETTE_CHANGES: usize = 16_384;

/// The most sync-mode and resolution writes a frame's log keeps; the oldest are dropped first.
pub const MAX_SYNC_LOG: usize = 256;

/// Which of the two registers a [`SyncWrite`] is a write to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncRegister {
    /// `$FF820A`, the sync mode (50/60 Hz, external sync).
    Sync,
    /// `$FF8260`, the resolution.
    Resolution,
}

/// One write to the sync mode or the resolution register, with where in the frame it happened: the raster line and
/// the CPU cycles since that line began. This is a log only. The picture does not use it and no border is opened by
/// it: a program that switches the sync mode or the resolution at the right cycle of a line to open the overscan
/// borders shows up here and nowhere else (that is Phase 3, spec section 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncWrite {
    pub line: u16,
    pub cycle: u32,
    pub register: SyncRegister,
    pub value: u8,
}

/// One palette write, on raster line `line` of the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteChange {
    pub line: u16,
    /// CPU clocks into the line when the device saw the write (0 for a write made outside a running machine).
    pub cycle: u16,
    pub index: u8,
    pub value: u16,
}

/// The registers only the STE's Shifter has ([O] Hatari 2.6.1 `--machine ste`, probe P27: the values read back as written, but
/// the video base high byte keeps its low six bits). The video counter's three bytes are writable on the STE: the written
/// counter runs on from there until the next frame loads it from the base.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteVideo {
    /// `$FF820D`: the video base's low byte.
    pub base_low: u8,
    /// `$FF820F`: words skipped at the end of every line.
    pub line_offset: u8,
    /// `$FF8265` and `$FF8264`: the pixel scroll (the second one without the extra fetch).
    pub hscroll: u8,
    pub hscroll_np: u8,
    /// What the counter has been moved by (bytes, wrapping at 24 bits).
    pub counter_adjust: i32,
    /// For each picture line of the frame so far, in order: the bytes its fetch adds to the 160 (the line offset, and the
    /// extra 16-pixel group the scroll prefetches) and the pixel scroll it was drawn with ([O] P28).
    pub line_extra: Vec<(u16, u8)>,
}

/// What each picture line of a frame is drawn from, by picture line `y`: where its data starts in memory (`None`: nothing is
/// fetched and the line stays black), the pixel scroll, and the resolution it is drawn in.
struct Lines<'a> {
    row_at: &'a dyn Fn(u32) -> Option<u32>,
    scroll_at: &'a dyn Fn(u32) -> u32,
    res_at: &'a dyn Fn(u32) -> Resolution,
}

/// A picture line's palette at its start and the changes made on it: the canvas pixel each reaches, the colour index
/// and the value, in time order.
type LinePalette = ([u16; 16], Vec<(i32, usize, u16)>);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shifter {
    base_high: u8,
    base_mid: u8,
    sync: u8,
    palette: [u16; 16],
    resolution: u8,
    /// The raster line the machine is on (set by it at each line end).
    line: u16,
    /// The palette at the start of the frame, and the writes since, in time order.
    frame_palette: [u16; 16],
    changes: Vec<PaletteChange>,
    /// The CPU cycle the current raster line began on, and the sync-mode and resolution writes of the frame.
    line_start: u64,
    sync_log: VecDeque<SyncWrite>,
    /// The clocks into the line of the write being made (set by `write_at`, stored with the palette change).
    #[serde(skip)]
    cycle: u16,
    /// The picture window of the frame and the borders of its lines (`fetch.rs`).
    fetch: Fetch,
    /// The STE's registers (not part of the shared state: see `ste_state`).
    #[serde(skip)]
    ste: Option<SteVideo>,
    /// The resolution and sync mode registers as the current raster line began.
    line_res: u8,
    line_sync: u8,
    /// The resolution each raster line of the frame is drawn in, by line (`UNKNOWN_RES` for a line not seen): the register as
    /// it stood `RES_SAMPLE_CLOCK` clocks into the line ([O] P35, `line_resolution`). Not part of the saved state: a state
    /// loaded in the middle of a frame draws the lines before it in the register's resolution.
    #[serde(skip)]
    line_res_log: Vec<u8>,
    /// The resolution of the current line as sampled so far.
    #[serde(skip)]
    res_sample: u8,
}

/// A raster line with no recorded resolution in `Shifter::line_res_log`.
const UNKNOWN_RES: u8 = 0xFF;

/// The clock of a line at which the resolution register decides how the line is drawn: a line is drawn in the resolution the
/// register has at this clock, whole ([O] P35 and P45: Hatari 2.6.1, which draws no part of a line in another resolution; the
/// lines P35 draws in medium are the same for 464 to 467 and change at 460 and 468).
pub const RES_SAMPLE_CLOCK: u64 = 464;

impl Shifter {
    /// Power-on: every register 0 except sync mode, 50 Hz on a PAL machine and 60 Hz on an NTSC one ([H?]: TOS
    /// sets it before showing anything).
    pub fn new(pal: bool) -> Self {
        Self {
            base_high: 0,
            base_mid: 0,
            sync: if pal { 0x02 } else { 0x00 },
            palette: [0; 16],
            resolution: 0,
            line: 0,
            frame_palette: [0; 16],
            changes: Vec::new(),
            line_start: 0,
            sync_log: VecDeque::new(),
            cycle: 0,
            fetch: Fetch::default(),
            ste: None,
            line_res: 0,
            line_sync: if pal { 0x02 } else { 0x00 },
            line_res_log: Vec::new(),
            res_sample: 0,
        }
    }

    /// The CPU cycle the current raster line began on (the machine sets it with [`Shifter::set_line`]).
    pub fn set_line_start(&mut self, cycle: u64) {
        self.line_start = cycle;
    }

    /// A register write at CPU cycle `now`: like [`Shifter::write`], and a write to the sync mode or the resolution
    /// goes into the log with its line and cycle.
    pub fn write_at(&mut self, addr: u32, word: bool, value: u16, now: u64) {
        self.cycle = now.saturating_sub(self.line_start).min(u16::MAX as u64) as u16;
        let register = match (addr & 0xFF_FFFF, word) {
            (0xFF_820A, false) => Some(SyncRegister::Sync),
            (0xFF_8260, false) => Some(SyncRegister::Resolution),
            _ => None,
        };
        if let Some(register) = register {
            if self.sync_log.len() == MAX_SYNC_LOG {
                self.sync_log.pop_front();
            }
            self.sync_log.push_back(SyncWrite {
                line: self.line,
                cycle: now.saturating_sub(self.line_start).min(u32::MAX as u64) as u32,
                register,
                value: value as u8 & 0x03,
            });
            if register == SyncRegister::Resolution
                && now.saturating_sub(self.line_start) <= RES_SAMPLE_CLOCK
            {
                self.res_sample = value as u8 & 0x03;
            }
        }
        self.write(addr, word, value);
    }

    /// The sync-mode and resolution writes of the frame so far, oldest first.
    pub fn sync_log(&self) -> impl Iterator<Item = &SyncWrite> {
        self.sync_log.iter()
    }

    /// The raster line the next writes belong to.
    pub fn set_line(&mut self, line: u16) {
        self.line = line;
    }

    /// The current raster line ends and `new_line` begins: the finished line's borders are decided from its register
    /// writes, the registers as they now stand are the new line's, and the new line may start or end the picture.
    pub fn line_boundary(&mut self, new_line: u16) {
        self.record_ste_line(self.line);
        self.fetch.end_line(
            self.line,
            self.line_res,
            self.line_sync,
            self.sync_log.iter(),
        );
        let finished = self.line as usize;
        if self.line_res_log.len() <= finished {
            self.line_res_log.resize(finished + 1, UNKNOWN_RES);
        }
        self.line_res_log[finished] = self.res_sample;
        self.line_res = self.resolution;
        self.res_sample = self.resolution;
        self.line_sync = self.sync;
        self.line = new_line;
        self.fetch.start_line(new_line, self.sync & 2 == 0);
    }

    /// This is an STE's Shifter: 4096 colours and the extra registers.
    pub fn set_ste(&mut self, ste: bool) {
        self.ste = ste.then(SteVideo::default);
    }

    pub fn is_ste(&self) -> bool {
        self.ste.is_some()
    }

    /// The STE registers as data, for a state (`None` on an ST).
    pub fn ste_state(&self) -> Option<&SteVideo> {
        self.ste.as_ref()
    }

    pub fn restore_ste_state(&mut self, state: SteVideo) {
        if self.ste.is_some() {
            self.ste = Some(state);
        }
    }

    /// The video counter's movement since the frame began (STE).
    pub fn counter_adjust(&self) -> i32 {
        self.ste.as_ref().map_or(0, |s| s.counter_adjust)
    }

    pub fn set_counter_adjust(&mut self, by: i32) {
        if let Some(s) = self.ste.as_mut() {
            s.counter_adjust = by;
        }
    }

    /// The monitor is monochrome (at power-on).
    pub fn set_mono(&mut self, mono: bool) {
        self.fetch.set_mono(mono);
    }

    /// The picture window and the borders of the lines of this frame.
    pub fn fetch(&self) -> &Fetch {
        &self.fetch
    }

    /// The borders the current line has opened by now, from the register writes made on it so far.
    pub fn current_borders(&self) -> (bool, bool) {
        if !self.fetch.fetching(self.line) {
            return (false, false);
        }
        Fetch::split(Fetch::flags(
            self.line,
            self.line_res,
            self.line_sync,
            self.sync_log.iter(),
        ))
    }

    /// Where the data of raster line `line` starts, as bytes past the video base, and whether its left border opened
    /// (the visible picture then starts 26 bytes into the data); `None` for a line that fetches nothing.
    pub fn picture_line(&self, line: u16) -> Option<(u32, bool)> {
        if !self.fetch.fetching(line) {
            return None;
        }
        let (left, _) = self.fetch.borders(line);
        Some((self.bytes_before(line), left))
    }

    /// The bytes fetched on the picture lines before `line`: the 160 of each, the border bytes and, on an STE, the line
    /// offset and the scroll's extra group.
    pub fn bytes_before(&self, line: u16) -> u32 {
        let mut bytes = self.fetch.bytes_before(line);
        if let Some(ste) = &self.ste {
            let first = self.fetch.lines_before(line);
            bytes += ste
                .line_extra
                .iter()
                .take(first.len())
                .map(|&(extra, _)| u32::from(extra))
                .sum::<u32>();
        }
        bytes
    }

    /// The pixel scroll picture raster line `line` was drawn with (STE; 0 on an ST).
    pub fn line_scroll(&self, line: u16) -> u32 {
        let (Some(ste), true) = (&self.ste, self.fetch.fetching(line)) else {
            return 0;
        };
        let index = self
            .fetch
            .lines_before(line.saturating_add(1))
            .len()
            .saturating_sub(1);
        match ste.line_extra.get(index) {
            Some(&(_, scroll)) => u32::from(scroll),
            // The line being drawn now: the registers as they are.
            None => u32::from(self.ste_scroll()),
        }
    }

    /// The scroll the registers give: the no-prefetch register when it is set, else the other.
    fn ste_scroll(&self) -> u8 {
        self.ste.as_ref().map_or(0, |s| {
            if s.hscroll_np & 15 != 0 {
                s.hscroll_np & 15
            } else {
                s.hscroll & 15
            }
        })
    }

    /// The resolution raster line `line` of this frame is drawn in: the register's value `RES_SAMPLE_CLOCK` clocks into it,
    /// the whole line ([O] P35: Hatari does not draw a part of a line in another resolution). A line not seen (a state
    /// loaded part way through the frame) uses the register now.
    pub fn line_resolution(&self, line: u16) -> Resolution {
        match self.line_res_log.get(line as usize).copied() {
            Some(UNKNOWN_RES) | None => self.resolution(),
            Some(r) => Resolution::of(r),
        }
    }

    /// Bytes of one 16-pixel group in the resolution now selected.
    fn group_bytes(&self) -> u16 {
        match self.resolution() {
            Resolution::Low => 8,
            Resolution::Medium => 4,
            Resolution::High => 2,
            Resolution::Reserved => 0,
        }
    }

    /// What the finished picture line `line` added to its fetch and how it was scrolled, from the STE registers.
    fn record_ste_line(&mut self, line: u16) {
        if self.ste.is_none() || !self.fetch.fetching(line) {
            return;
        }
        let Some(first) = self.fetch.first_line() else {
            return;
        };
        let index = (line - first) as usize;
        let scroll = self.ste_scroll();
        let group = self.group_bytes();
        let Some(ste) = self.ste.as_mut() else {
            return;
        };
        // [O] P28: 2 bytes a word of line offset, and one more group when the scroll register with prefetch is non-zero
        // and the no-prefetch one is not.
        let prefetch = ste.hscroll & 15 != 0 && ste.hscroll_np & 15 == 0;
        let extra = u16::from(ste.line_offset) * 2 + if prefetch { group } else { 0 };
        if ste.line_extra.len() > index {
            return;
        }
        ste.line_extra.resize(index + 1, (0, 0));
        ste.line_extra[index] = (extra, scroll);
    }

    /// A new frame starts: the palette as it now is becomes the frame's first, and the change log empties.
    pub fn begin_frame(&mut self) {
        self.frame_palette = self.palette;
        self.changes.clear();
        self.sync_log.clear();
        self.line_res_log.clear();
        self.fetch.reset();
        self.set_counter_adjust(0);
        if let Some(s) = self.ste.as_mut() {
            s.line_extra.clear();
        }
    }

    /// The palette writes of the frame so far, in time order.
    pub fn palette_changes(&self) -> &[PaletteChange] {
        &self.changes
    }

    /// A register read; `None` when `addr` is not one of the Shifter's. `word` is a 16-bit access (palette only);
    /// otherwise the byte at `addr`. [H?] unused bits read 0.
    pub fn read(&self, addr: u32, word: bool) -> Option<u16> {
        if let Some(ste) = &self.ste {
            match (addr & 0xFF_FFFF, word) {
                (0xFF_820D, false) => return Some(ste.base_low as u16),
                (0xFF_820F, false) => return Some(ste.line_offset as u16),
                (0xFF_8264, false) => return Some(ste.hscroll_np as u16),
                (0xFF_8265, false) => return Some(ste.hscroll as u16),
                // [O] P27: the sync mode reads with its unused bits set, the resolution without.
                (0xFF_820A, false) => return Some(0xFC | self.sync as u16),
                (0xFF_8260, false) => return Some(self.resolution as u16),
                _ => {}
            }
        }
        Some(match (addr & 0xFF_FFFF, word) {
            (0xFF_8201, false) => self.base_high as u16,
            (0xFF_8203, false) => self.base_mid as u16,
            (0xFF_820A, false) => self.sync as u16,
            // [O] bits 7-2 read as 1: Hatari reads $FC with resolution 0 (EmuTOS at $FC69C2).
            (0xFF_8260, false) => 0xFC | self.resolution as u16,
            (a @ 0xFF_8240..=0xFF_825F, w) => {
                let colour = self.palette[((a - 0xFF_8240) / 2) as usize];
                match (w, a & 1) {
                    (true, _) => colour,
                    (false, 0) => colour >> 8,
                    (false, _) => colour & 0xFF,
                }
            }
            _ => return None,
        })
    }

    /// A register write; ignored when `addr` is not one of the Shifter's.
    pub fn write(&mut self, addr: u32, word: bool, value: u16) {
        if let Some(ste) = self.ste.as_mut() {
            match (addr & 0xFF_FFFF, word) {
                (0xFF_820D, false) => {
                    ste.base_low = value as u8;
                    return;
                }
                (0xFF_820F, false) => {
                    ste.line_offset = value as u8;
                    return;
                }
                (0xFF_8264, false) => {
                    ste.hscroll_np = value as u8;
                    return;
                }
                (0xFF_8265, false) => {
                    ste.hscroll = value as u8;
                    return;
                }
                // [O] P27: the video base's high byte keeps six bits (a 4 MB address space).
                (0xFF_8201, false) => {
                    self.base_high = value as u8 & 0x3F;
                    return;
                }
                _ => {}
            }
        }
        match (addr & 0xFF_FFFF, word) {
            // p.56: the video base, high and mid bytes, read/write.
            (0xFF_8201, false) => self.base_high = value as u8,
            (0xFF_8203, false) => self.base_mid = value as u8,
            // p.57: bit 1 50/60 Hz, bit 0 external sync.
            (0xFF_820A, false) => self.sync = value as u8 & 0x03,
            // p.58: bits 1-0.
            (0xFF_8260, false) => self.resolution = value as u8 & 0x03,
            // p.57: 3 bits per gun, `$0RGB`.
            (a @ 0xFF_8240..=0xFF_825F, w) => {
                let ste = self.ste.is_some();
                let slot = &mut self.palette[((a - 0xFF_8240) / 2) as usize];
                let merged = match (w, a & 1, ste) {
                    (true, _, _) => value,
                    // [O] P27: on the STE a byte write lands in both halves of the register.
                    (false, _, true) => (value & 0xFF) | (value & 0xFF) << 8,
                    (false, 0, false) => (*slot & 0x00FF) | (value << 8),
                    (false, _, false) => (*slot & 0xFF00) | (value & 0xFF),
                };
                let value = merged & if ste { 0x0FFF } else { 0x0777 };
                if *slot != value {
                    *slot = value;
                    if self.changes.len() < MAX_PALETTE_CHANGES {
                        self.changes.push(PaletteChange {
                            line: self.line,
                            cycle: self.cycle,
                            index: ((a - 0xFF_8240) / 2) as u8,
                            value,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    /// The screen's start address: `$FF8201` bits 23-16, `$FF8203` bits 15-8 (p.56, 256-byte steps).
    pub fn video_base(&self) -> u32 {
        (self.base_high as u32) << 16
            | (self.base_mid as u32) << 8
            | self.ste.as_ref().map_or(0, |s| (s.base_low & 0xFE) as u32)
    }

    /// `$FF820A` bit 1: 1 = 50 Hz (p.57).
    pub fn sync_50hz(&self) -> bool {
        self.sync & 0x02 != 0
    }

    pub fn resolution(&self) -> Resolution {
        Resolution::of(self.resolution)
    }

    pub fn palette(&self) -> [u16; 16] {
        self.palette
    }

    /// The whole frame from video RAM (`ram` reads one byte at a CPU address) as 640 × 400 RGB into `out`, with the
    /// palette as it is now on every line.
    ///
    /// Screen layout (pp.207-209): 16-pixel groups of N consecutive big-endian words, one per plane, the most
    /// significant bit the leftmost pixel; plane 0 (the first word) is bit 0 of the colour index ([H?] the book
    /// names the words but not the bit order of the index). High resolution: a set bit is dark while palette 0
    /// bit 0 is set (observed in Hatari; the book, p.58, says the opposite). A colour monitor shows nothing in high resolution, a mono monitor nothing in low or
    /// medium ([H?] they cannot sync to it, p.57).
    pub fn render(&self, monitor: Monitor, ram: &dyn Fn(u32) -> u8, out: &mut Vec<u8>) {
        let palette = self.palette;
        let base = self.video_base();
        let (extra, scroll) = self.ste_line_extra();
        let bytes_per_line = self.bytes_per_line(monitor) + extra;
        self.draw(
            monitor,
            ram,
            out,
            &mut |_| (palette, Vec::new()),
            &Lines {
                row_at: &|line| Some(base + line * bytes_per_line),
                scroll_at: &|_| scroll,
                res_at: &|_| self.resolution(),
            },
        );
    }

    /// The frame with the palette followed down the screen: picture line `y` (0 at the top, 0-199 in low and medium
    /// resolution) is raster line `first_line + y`, and shows the frame's first palette with every change logged on
    /// that line or an earlier one applied (the last write to a colour on a line wins).
    pub fn render_raster(
        &self,
        monitor: Monitor,
        first_line: u32,
        ram: &dyn Fn(u32) -> u8,
        out: &mut Vec<u8>,
    ) {
        let mut palette = self.frame_palette;
        let mut next = 0;
        let base = self.video_base();
        let (extra, scroll) = self.ste_line_extra();
        let bytes_per_line = self.bytes_per_line(monitor) + extra;
        self.draw(
            monitor,
            ram,
            out,
            &mut |y| {
                while let Some(c) = self.changes.get(next)
                    && c.line as u32 <= first_line + y
                {
                    palette[(c.index & 15) as usize] = c.value;
                    next += 1;
                }
                (palette, Vec::new())
            },
            &Lines {
                row_at: &|line| Some(base + line * bytes_per_line),
                scroll_at: &|_| scroll,
                res_at: &|_| self.resolution(),
            },
        );
    }

    /// Like [`Shifter::render_raster`], but a palette change on a line reaches the pixels from the clock it was made at:
    /// the first pixel of the picture is drawn at clock `pixel_start` of the line, and the beam draws one low resolution
    /// pixel (two medium resolution pixels) a clock. A change before the line's start applies to the whole line.
    pub fn render_exact(
        &self,
        monitor: Monitor,
        first_line: u32,
        pixel_start: i32,
        ram: &dyn Fn(u32) -> u8,
        out: &mut Vec<u8>,
    ) {
        let mut palette = self.frame_palette;
        let mut next = 0;
        let base = self.video_base();
        self.draw(
            monitor,
            ram,
            out,
            &mut |y| {
                // Changes of earlier lines are done; those of this line are applied pixel by pixel.
                let line = first_line + y;
                while let Some(c) = self.changes.get(next)
                    && (c.line as u32) < line
                {
                    palette[(c.index & 15) as usize] = c.value;
                    next += 1;
                }
                let start = palette;
                let mut in_line = Vec::new();
                while let Some(c) = self.changes.get(next)
                    && c.line as u32 == line
                {
                    let clock = c.cycle as i32 - pixel_start;
                    // Two canvas pixels a clock in both resolutions.
                    in_line.push((2 * clock, (c.index & 15) as usize, c.value));
                    // The line after this one starts with them all done.
                    palette[(c.index & 15) as usize] = c.value;
                    next += 1;
                }
                (start, in_line)
            },
            &Lines {
                // The data of the raster line shown as picture line `y`: the picture may start before or after it.
                row_at: &|y| {
                    self.picture_line((first_line + y) as u16)
                        .map(|(offset, left)| base + offset + if left { LEFT_BYTES } else { 0 })
                },
                scroll_at: &|y| self.line_scroll((first_line + y) as u16),
                res_at: &|y| self.line_resolution((first_line + y) as u16),
            },
        );
    }

    /// The STE's registers as the `fast` render sees them (whole frame, as they are at its end): the bytes a line fetches
    /// beyond its own (the line offset in words, and a group more with the scroll register that prefetches) and the pixel
    /// scroll. An ST has neither.
    fn ste_line_extra(&self) -> (u32, u32) {
        let Some(ste) = &self.ste else {
            return (0, 0);
        };
        let prefetch = ste.hscroll & 15 != 0 && ste.hscroll_np & 15 == 0;
        let extra = u32::from(ste.line_offset) * 2
            + if prefetch {
                u32::from(self.group_bytes())
            } else {
                0
            };
        (extra, u32::from(self.ste_scroll()))
    }

    /// Bytes of picture data on a line in the resolution now selected.
    fn bytes_per_line(&self, monitor: Monitor) -> u32 {
        match (self.resolution(), monitor) {
            (Resolution::Low, Monitor::Colour) => 160,
            (Resolution::Medium, Monitor::Colour) => 160,
            (Resolution::High, Monitor::Mono) => 80,
            _ => 0,
        }
    }

    /// Draws the frame. `palette_at(y)` gives picture line `y`'s palette at its start and the palette changes made on the
    /// line as (canvas pixel they reach, colour index, value), in time order.
    fn draw(
        &self,
        monitor: Monitor,
        ram: &dyn Fn(u32) -> u8,
        out: &mut Vec<u8>,
        palette_at: &mut dyn FnMut(u32) -> LinePalette,
        picture: &Lines,
    ) {
        let Lines {
            row_at,
            scroll_at,
            res_at,
        } = *picture;
        out.clear();
        out.resize((WIDTH * HEIGHT * 3) as usize, 0);
        // A colour monitor draws each line in its own resolution; the mono monitor has one.
        let lines = match (self.resolution(), monitor) {
            (Resolution::Low | Resolution::Medium, Monitor::Colour) => 200,
            (Resolution::High, Monitor::Mono) => 400,
            _ => return,
        };
        let y_scale = HEIGHT / lines;
        let ste = self.ste.is_some();
        let rgb_of = |palette: &[u16; 16], planes: u32| -> Vec<[u8; 3]> {
            if planes == 1 {
                let (dark, light) = ([0, 0, 0], [255, 255, 255]);
                // [O] Hatari's mono desktop with EmuTOS' palette 0 = $777: a set bit is dark while bit 0 of palette 0 is set.
                if palette[0] & 1 != 0 {
                    vec![light, dark]
                } else {
                    vec![dark, light]
                }
            } else {
                palette.iter().map(|&c| to_rgb(c, ste)).collect()
            }
        };
        let mut words = [0u16; 4];
        for line in 0..lines {
            let (mut palette, changes) = palette_at(line);
            let (planes, width) = match (monitor, res_at(line)) {
                (Monitor::Colour, Resolution::Low) => (4, 320),
                (Monitor::Colour, Resolution::Medium) => (2, 640),
                (Monitor::Mono, _) => (1, 640),
                _ => {
                    // no picture in this resolution on this monitor: the line stays black, its palette still advances
                    continue;
                }
            };
            let x_scale = WIDTH / width;
            let mut rgb = rgb_of(&palette, planes);
            let mut change_at = 0;
            // A line that fetches nothing stays black.
            let Some(row) = row_at(line) else {
                continue;
            };
            let scroll = scroll_at(line);
            let mut loaded = u32::MAX;
            for group in 0..width / 16 {
                for bit in 0..16u32 {
                    // The STE's pixel scroll moves the picture left: pixel `x` shows the data `scroll` pixels on.
                    let source = group * 16 + bit + scroll;
                    let (src_group, bit_in) = (source / 16, source % 16);
                    if src_group != loaded {
                        loaded = src_group;
                        let at = row + src_group * planes * 2;
                        for (p, w) in words.iter_mut().enumerate().take(planes as usize) {
                            let a = at + p as u32 * 2;
                            *w = u16::from_be_bytes([ram(a), ram(a + 1)]);
                        }
                    }
                    let x = (group * 16 + bit) * x_scale;
                    // The palette changes that have reached the beam by this pixel.
                    let mut changed = false;
                    while let Some(&(at_x, index, value)) = changes.get(change_at)
                        && at_x <= x as i32
                    {
                        palette[index] = value;
                        change_at += 1;
                        changed = true;
                    }
                    if changed {
                        rgb = rgb_of(&palette, planes);
                    }
                    let mut index = 0usize;
                    for (p, w) in words.iter().enumerate().take(planes as usize) {
                        index |= (((w >> (15 - bit_in)) & 1) as usize) << p;
                    }
                    let colour = rgb[index];
                    for dy in 0..y_scale {
                        let y = line * y_scale + dy;
                        for dx in 0..x_scale {
                            let i = ((y * WIDTH + x + dx) * 3) as usize;
                            out[i..i + 3].copy_from_slice(&colour);
                        }
                    }
                }
            }
        }
    }
}

/// `$0RGB`, 3 bits per gun, to 8 bits per channel: `level * 255 / 7` (a display choice, not a hardware fact).
fn to_rgb(colour: u16, ste: bool) -> [u8; 3] {
    if ste {
        // Four bits a gun: bits 2-0 are the high bits and bit 3 the low bit ([H] Internals, STE palette).
        let level = |shift: u16| {
            let v = (colour >> shift) & 0xF;
            ((((v & 7) << 1) | (v >> 3)) as u32 * 17) as u8
        };
        return [level(8), level(4), level(0)];
    }
    let level = |shift: u16| (((colour >> shift) & 7) as u32 * 255 / 7) as u8;
    [level(8), level(4), level(0)]
}
