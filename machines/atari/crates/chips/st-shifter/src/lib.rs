//! The Atari ST Shifter's registers and a frame render. Facts and their sources: `docs/hardware-facts.md` of
//! retroclawd-atari, "Shifter"; pages are those of *Atari ST Internals* (Abacus 1985).
//!
//! The picture is drawn once per frame onto a fixed 640 × 400 canvas: low resolution pixel-doubled, medium
//! resolution line-doubled. The palette is followed line by line: every palette write is logged with the raster line
//! it happened on, and [`Shifter::render_raster`] applies each change from its line down (one line is the
//! granularity; a change inside a line is Phase 3). Everything else is taken as the registers are at the end of the
//! frame.
#![forbid(unsafe_code)]

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
}

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

    /// A new frame starts: the palette as it now is becomes the frame's first, and the change log empties.
    pub fn begin_frame(&mut self) {
        self.frame_palette = self.palette;
        self.changes.clear();
        self.sync_log.clear();
    }

    /// The palette writes of the frame so far, in time order.
    pub fn palette_changes(&self) -> &[PaletteChange] {
        &self.changes
    }

    /// A register read; `None` when `addr` is not one of the Shifter's. `word` is a 16-bit access (palette only);
    /// otherwise the byte at `addr`. [H?] unused bits read 0.
    pub fn read(&self, addr: u32, word: bool) -> Option<u16> {
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
                let slot = &mut self.palette[((a - 0xFF_8240) / 2) as usize];
                let merged = match (w, a & 1) {
                    (true, _) => value,
                    (false, 0) => (*slot & 0x00FF) | (value << 8),
                    (false, _) => (*slot & 0xFF00) | (value & 0xFF),
                };
                let value = merged & 0x0777;
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
        (self.base_high as u32) << 16 | (self.base_mid as u32) << 8
    }

    /// `$FF820A` bit 1: 1 = 50 Hz (p.57).
    pub fn sync_50hz(&self) -> bool {
        self.sync & 0x02 != 0
    }

    pub fn resolution(&self) -> Resolution {
        match self.resolution {
            0 => Resolution::Low,
            1 => Resolution::Medium,
            2 => Resolution::High,
            _ => Resolution::Reserved,
        }
    }

    pub fn palette(&self) -> [u16; 16] {
        self.palette
    }

    /// The whole frame from video RAM (`ram` reads one byte at a CPU address) as 640 × 400 RGB into `out`, with the
    /// palette as it is now on every line.
    ///
    /// Screen layout (pp.207-209): 16-pixel groups of N consecutive big-endian words, one per plane, the most
    /// significant bit the leftmost pixel; plane 0 (the first word) is bit 0 of the colour index ([H?] the book
    /// names the words but not the bit order of the index). High resolution: a set bit is dark unless palette 0
    /// bit 0 is set (p.58). A colour monitor shows nothing in high resolution, a mono monitor nothing in low or
    /// medium ([H?] they cannot sync to it, p.57).
    pub fn render(&self, monitor: Monitor, ram: &dyn Fn(u32) -> u8, out: &mut Vec<u8>) {
        let palette = self.palette;
        self.draw(monitor, ram, out, &mut |_| (palette, Vec::new()));
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
        self.draw(monitor, ram, out, &mut |y| {
            while let Some(c) = self.changes.get(next)
                && c.line as u32 <= first_line + y
            {
                palette[(c.index & 15) as usize] = c.value;
                next += 1;
            }
            (palette, Vec::new())
        });
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
        self.draw(monitor, ram, out, &mut |y| {
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
        });
    }

    /// Draws the frame. `palette_at(y)` gives picture line `y`'s palette at its start and the palette changes made on the
    /// line as (canvas pixel they reach, colour index, value), in time order.
    fn draw(
        &self,
        monitor: Monitor,
        ram: &dyn Fn(u32) -> u8,
        out: &mut Vec<u8>,
        palette_at: &mut dyn FnMut(u32) -> LinePalette,
    ) {
        out.clear();
        out.resize((WIDTH * HEIGHT * 3) as usize, 0);
        let (planes, width, lines) = match (self.resolution(), monitor) {
            (Resolution::Low, Monitor::Colour) => (4, 320, 200),
            (Resolution::Medium, Monitor::Colour) => (2, 640, 200),
            (Resolution::High, Monitor::Mono) => (1, 640, 400),
            _ => return,
        };
        let x_scale = WIDTH / width;
        let y_scale = HEIGHT / lines;
        let bytes_per_line = width * planes / 8;
        let base = self.video_base();
        let rgb_of = |palette: &[u16; 16]| -> Vec<[u8; 3]> {
            if planes == 1 {
                let (dark, light) = ([0, 0, 0], [255, 255, 255]);
                if palette[0] & 1 == 0 {
                    vec![light, dark]
                } else {
                    vec![dark, light]
                }
            } else {
                palette.iter().map(|&c| to_rgb(c)).collect()
            }
        };
        let mut words = [0u16; 4];
        for line in 0..lines {
            let (mut palette, changes) = palette_at(line);
            let mut rgb = rgb_of(&palette);
            let mut change_at = 0;
            let row = base + line * bytes_per_line;
            for group in 0..width / 16 {
                let at = row + group * planes * 2;
                for (p, w) in words.iter_mut().enumerate().take(planes as usize) {
                    let a = at + p as u32 * 2;
                    *w = u16::from_be_bytes([ram(a), ram(a + 1)]);
                }
                for bit in 0..16u32 {
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
                        rgb = rgb_of(&palette);
                    }
                    let mut index = 0usize;
                    for (p, w) in words.iter().enumerate().take(planes as usize) {
                        index |= (((w >> (15 - bit)) & 1) as usize) << p;
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
fn to_rgb(colour: u16) -> [u8; 3] {
    let level = |shift: u16| (((colour >> shift) & 7) as u32 * 255 / 7) as u8;
    [level(8), level(4), level(0)]
}
