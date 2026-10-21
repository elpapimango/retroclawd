//! The Atari ST Shifter's registers and a frame render. Facts and their sources: `docs/hardware-facts.md` of
//! retroclawd-atari, "Shifter"; pages are those of *Atari ST Internals* (Abacus 1985).
//!
//! The picture is drawn once per frame from the registers as they are then (design Phase 1: no mid-frame effects)
//! onto a fixed 640 × 400 canvas: low resolution pixel-doubled, medium resolution line-doubled.
#![forbid(unsafe_code)]

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shifter {
    base_high: u8,
    base_mid: u8,
    sync: u8,
    palette: [u16; 16],
    resolution: u8,
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
        }
    }

    /// A register read; `None` when `addr` is not one of the Shifter's. `word` is a 16-bit access (palette only);
    /// otherwise the byte at `addr`. [H?] unused bits read 0.
    pub fn read(&self, addr: u32, word: bool) -> Option<u16> {
        Some(match (addr & 0xFF_FFFF, word) {
            (0xFF_8201, false) => self.base_high as u16,
            (0xFF_8203, false) => self.base_mid as u16,
            (0xFF_820A, false) => self.sync as u16,
            (0xFF_8260, false) => self.resolution as u16,
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
                *slot = merged & 0x0777;
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

    /// The whole frame from video RAM (`ram` reads one byte at a CPU address) as 640 × 400 RGB into `out`.
    ///
    /// Screen layout (pp.207-209): 16-pixel groups of N consecutive big-endian words, one per plane, the most
    /// significant bit the leftmost pixel; plane 0 (the first word) is bit 0 of the colour index ([H?] the book
    /// names the words but not the bit order of the index). High resolution: a set bit is dark unless palette 0
    /// bit 0 is set (p.58). A colour monitor shows nothing in high resolution, a mono monitor nothing in low or
    /// medium ([H?] they cannot sync to it, p.57).
    pub fn render(&self, monitor: Monitor, ram: &dyn Fn(u32) -> u8, out: &mut Vec<u8>) {
        out.clear();
        out.resize((WIDTH * HEIGHT * 3) as usize, 0);
        let (planes, width, lines) = match (self.resolution(), monitor) {
            (Resolution::Low, Monitor::Colour) => (4, 320, 200),
            (Resolution::Medium, Monitor::Colour) => (2, 640, 200),
            (Resolution::High, Monitor::Mono) => (1, 640, 400),
            _ => return,
        };
        let rgb: Vec<[u8; 3]> = if planes == 1 {
            let (dark, light) = ([0, 0, 0], [255, 255, 255]);
            if self.palette[0] & 1 == 0 {
                vec![light, dark]
            } else {
                vec![dark, light]
            }
        } else {
            self.palette.iter().map(|&c| to_rgb(c)).collect()
        };
        let x_scale = WIDTH / width;
        let y_scale = HEIGHT / lines;
        let bytes_per_line = width * planes / 8;
        let base = self.video_base();
        let mut words = [0u16; 4];
        for line in 0..lines {
            let row = base + line * bytes_per_line;
            for group in 0..width / 16 {
                let at = row + group * planes * 2;
                for (p, w) in words.iter_mut().enumerate().take(planes as usize) {
                    let a = at + p as u32 * 2;
                    *w = u16::from_be_bytes([ram(a), ram(a + 1)]);
                }
                for bit in 0..16u32 {
                    let mut index = 0usize;
                    for (p, w) in words.iter().enumerate().take(planes as usize) {
                        index |= (((w >> (15 - bit)) & 1) as usize) << p;
                    }
                    let colour = rgb[index];
                    let x = (group * 16 + bit) * x_scale;
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
