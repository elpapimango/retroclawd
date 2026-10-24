//! The picture with its borders: the canvas Hatari draws with `--borders on` (832 x 552 for a colour monitor), measured on
//! its screenshot of a full-screen picture: the 640 x 400 picture at x 96, y 58, so 48 low resolution pixels of border to the
//! left and right, 29 lines above and 47 lines below.
//!
//! The Shifter draws one low resolution pixel (two medium resolution pixels) a clock, starting `pixel_start` clocks into a
//! line. The canvas shows the clocks `pixel_start - 48` to `pixel_start + 368` of the lines 34 to 310. Data a line fetched
//! to the left of the picture (its left border opened) or to the right of it (the right border) is drawn there; the rest of
//! the line is the border colour, palette entry 0 at that pixel.

use crate::{LEFT_BYTES, LINE_BYTES, Monitor, RIGHT_BYTES, Resolution, Shifter, to_rgb};

pub const OVERSCAN_WIDTH: u32 = 832;
pub const OVERSCAN_HEIGHT: u32 = 552;
/// The first raster line of the canvas and the number of lines.
pub const OVERSCAN_FIRST_LINE: u32 = 34;
const LINES: u32 = 276;
/// Clocks of border before and after the 320 clocks of picture.
const BORDER_CLOCKS: i32 = 48;

impl Shifter {
    /// The canvas with the borders (see the module comment) in `out`, `OVERSCAN_WIDTH` x `OVERSCAN_HEIGHT` RGB. Only the
    /// colour monitor in low or medium resolution has a picture; anything else gives the border colour.
    pub fn render_overscan(
        &self,
        monitor: Monitor,
        pixel_start: i32,
        ram: &dyn Fn(u32) -> u8,
        out: &mut Vec<u8>,
    ) {
        out.clear();
        out.resize((OVERSCAN_WIDTH * OVERSCAN_HEIGHT * 3) as usize, 0);
        let base = self.video_base();
        let x0 = pixel_start - BORDER_CLOCKS;
        let mut palette = self.frame_palette;
        let mut next = 0;
        for line in 0..LINES {
            let raster = (OVERSCAN_FIRST_LINE + line) as u16;
            // Each line is drawn in its own resolution (`Shifter::line_resolution`).
            let planes = match (self.line_resolution(raster), monitor) {
                (Resolution::Low, Monitor::Colour) => 4u32,
                (Resolution::Medium, Monitor::Colour) => 2,
                _ => 0,
            };
            // Changes of earlier lines are folded in; those of this line are applied as the beam reaches them.
            while let Some(c) = self.changes.get(next)
                && c.line < raster
            {
                palette[(c.index & 15) as usize] = c.value;
                next += 1;
            }
            let start = palette;
            let mut changes: Vec<(i32, usize, u16)> = Vec::new();
            while let Some(c) = self.changes.get(next)
                && c.line == raster
            {
                changes.push((2 * (c.cycle as i32 - x0), (c.index & 15) as usize, c.value));
                palette[(c.index & 15) as usize] = c.value;
                next += 1;
            }
            let mut colours = start;
            let mut rgb: Vec<[u8; 3]> = colours.iter().map(|&c| to_rgb(c, self.is_ste())).collect();
            let mut change_at = 0;
            // The data of this line: where it starts, whether it has a left and a right part.
            let data = if planes == 0 {
                None
            } else {
                self.picture_line(raster)
            };
            let (left, right) = self.fetch.borders(raster);
            let (bytes, stream_start) = match data {
                Some(_) => (
                    LINE_BYTES
                        + if left { LEFT_BYTES } else { 0 }
                        + if right { RIGHT_BYTES } else { 0 },
                    pixel_start - if left { 2 * LEFT_BYTES as i32 } else { 0 },
                ),
                None => (0, 0),
            };
            let row_address = data.map(|(offset, _)| base + offset);
            for x in 0..OVERSCAN_WIDTH {
                while let Some(&(at, index, value)) = changes.get(change_at)
                    && at <= x as i32
                {
                    colours[index] = value;
                    rgb[index] = to_rgb(value, self.is_ste());
                    change_at += 1;
                }
                let clock = x0 + (x / 2) as i32;
                let mut colour = rgb[0];
                if let Some(row) = row_address {
                    let rel = clock - stream_start;
                    if rel >= 0 && (rel as u32) < 2 * bytes {
                        let rel = rel as u32;
                        // Low resolution: 16 pixels in 16 clocks, 8 bytes; medium: 16 pixels in 8 clocks, 4 bytes.
                        let (group, bit) = if planes == 4 {
                            (rel / 16, rel % 16)
                        } else {
                            (rel / 8, 2 * (rel % 8) + (x & 1))
                        };
                        let at = row + group * planes * 2;
                        let mut index = 0usize;
                        for p in 0..planes {
                            let a = at + p * 2;
                            let w = u16::from_be_bytes([ram(a), ram(a + 1)]);
                            index |= (((w >> (15 - bit)) & 1) as usize) << p;
                        }
                        colour = rgb[index];
                    }
                }
                for dy in 0..2 {
                    let y = line * 2 + dy;
                    let i = ((y * OVERSCAN_WIDTH + x) * 3) as usize;
                    out[i..i + 3].copy_from_slice(&colour);
                }
            }
        }
    }
}
