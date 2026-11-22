//! The picture, one line at a time from the state of the VDP when the line is drawn (a mid-line change shows from the next
//! line; the pixel-accurate model is Phase 3). Layout facts: V7-V13 and V20 (`docs/hardware-facts.md`), layer order from the
//! Plutiedev "Layer priority" and "Shadow/highlight" pages.

use crate::Vdp;

/// A pixel of one layer: palette and colour index (`pppp cccc` in the low 6 bits), whether it is drawn, and its priority bit.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct Px {
    color: u8,
    opaque: bool,
    priority: bool,
}

/// What a sprite pixel does to the layers under it in shadow/highlight mode.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum Operator {
    #[default]
    None,
    Highlight,
    Shadow,
}

#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct SpritePx {
    px: Px,
    operator: Operator,
    /// Colour 14 of any palette: never darkened.
    colour14: bool,
}

/// The cells of a plane per register 16 field value (the value 2 is invalid and acts like 32).
fn plane_cells(field: u8) -> usize {
    match field & 3 {
        1 => 64,
        3 => 128,
        _ => 32,
    }
}

pub(crate) fn brightness_of(index: u8) -> Brightness {
    match index >> 6 {
        1 => Brightness::Shadow,
        2 => Brightness::Highlight,
        _ => Brightness::Normal,
    }
}

/// A colour of a palette (9-bit values `BBBGGGRRR`) as 8-bit RGB.
pub(crate) fn rgb_of(cram: &[u16], color: u8, brightness: Brightness) -> [u8; 3] {
    let c = cram[usize::from(color & 0x3F)];
    [
        level(c, brightness),
        level(c >> 3, brightness),
        level(c >> 6, brightness),
    ]
}

/// 3-bit colour components to 8-bit output levels for the three brightness levels (fact V24: the analogue levels are `2n`
/// for normal pixels, `n` for shadow and `7 + n` for highlight on a 0-15 scale).
fn level(n: u16, brightness: Brightness) -> u8 {
    // (2n, n, 7 + n) on a 0-15 scale, as 8-bit levels
    const LEVELS: [[u8; 8]; 3] = [
        [0, 34, 68, 102, 136, 170, 204, 238],
        [0, 17, 34, 51, 68, 85, 102, 119],
        [119, 136, 153, 170, 187, 204, 221, 238],
    ];
    LEVELS[brightness as usize][usize::from(n & 7)]
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Brightness {
    Normal,
    Shadow,
    Highlight,
}

/// The eight pixels of one row of a plane's tile, as drawn (flips applied), and which tile that was.
#[derive(Clone, Copy)]
struct TileRow {
    /// (column of the tile in plane pixels >> 3, row of the plane in pixels, plane base) the pixels belong to
    key: (usize, usize, usize),
    px: [Px; 8],
}

impl TileRow {
    fn empty() -> Self {
        TileRow {
            key: (usize::MAX, 0, 0),
            px: [Px::default(); 8],
        }
    }
}

impl Vdp {
    /// Interlace mode 2: tiles are 8x16 (64 bytes) and a field shows every other row of a picture twice as tall (fact V31).
    fn im2(&self) -> bool {
        self.regs[12] & 0x06 == 0x06
    }

    /// The row of the full-height picture the output line `line` shows: its own, or in interlace mode 2 the row of the
    /// current field.
    fn picture_row(&self, line: usize) -> usize {
        if self.im2() {
            line * 2 + usize::from(self.odd_frame)
        } else {
            line
        }
    }

    fn pattern_row(&self, pattern: usize, row: usize) -> [u8; 8] {
        let size = if self.im2() { 64 } else { 32 };
        let base = (pattern * size + row * 4) & 0xFFFF;
        let mut out = [0u8; 8];
        for i in 0..4 {
            let b = self.vram[(base + i) & 0xFFFF];
            out[2 * i] = b >> 4;
            out[2 * i + 1] = b & 15;
        }
        out
    }

    fn name_entry(&self, addr: usize) -> u16 {
        let a = addr & 0xFFFE;
        u16::from(self.vram[a]) << 8 | u16::from(self.vram[a + 1])
    }

    /// The pixel of a plane at plane position (`px`, `py`), through a cache of the tile row it is in: consecutive pixels of
    /// a tile share a name table entry and a pattern row.
    fn plane_px_cached(
        &self,
        cache: &mut TileRow,
        base: usize,
        wc: usize,
        hc: usize,
        px: usize,
        py: usize,
    ) -> Px {
        let key = (px >> 3, py, base);
        if cache.key != key {
            self.plane_tile_row(cache, base, wc, hc, px & !7, py);
            cache.key = key;
        }
        cache.px[px & 7]
    }

    /// Fills `cache` with the eight pixels of the tile row of the plane that starts at plane position (`px`, `py`).
    fn plane_tile_row(
        &self,
        cache: &mut TileRow,
        base: usize,
        wc: usize,
        hc: usize,
        px: usize,
        py: usize,
    ) {
        let (shift, last) = if self.im2() { (4, 15) } else { (3, 7) };
        let col = (px >> 3) % wc;
        let row = (py >> shift) % hc;
        let entry = self.name_entry(base + 2 * (row * wc + col));
        let mut y = py & last;
        if entry & 0x1000 != 0 {
            y = last - y;
        }
        let pattern = self.pattern_row(usize::from(entry & 0x7FF), y);
        let hi = (((entry >> 13) & 3) as u8) << 4;
        let priority = entry & 0x8000 != 0;
        for (i, out) in cache.px.iter_mut().enumerate() {
            let idx = pattern[if entry & 0x0800 != 0 { 7 - i } else { i }];
            *out = Px {
                color: hi | idx,
                opaque: idx != 0,
                priority,
            };
        }
    }

    fn hscroll(&self, line: usize) -> (usize, usize) {
        let base = usize::from(self.regs[13] & 0x3F) << 10;
        let offset = match self.regs[11] & 3 {
            0 => 0,
            1 => (line & 7) * 4,
            2 => (line & !7) * 4,
            _ => line * 4,
        };
        let a = usize::from(self.name_entry(base + offset) & 0x3FF);
        let b = usize::from(self.name_entry(base + offset + 2) & 0x3FF);
        (a, b)
    }

    fn vscroll(&self, x: usize) -> (usize, usize) {
        if self.regs[11] & 4 == 0 {
            (
                usize::from(self.vsram[0] & 0x3FF),
                usize::from(self.vsram[1] & 0x3FF),
            )
        } else {
            let column = ((x >> 4) * 2) & 0x3F;
            (
                usize::from(self.vsram[column] & 0x3FF),
                usize::from(self.vsram[column + 1] & 0x3FF),
            )
        }
    }

    /// Whether the window plane covers the cell at (`cx`, `cy`) (registers 17 and 18).
    fn in_window(&self, cx: usize, cy: usize) -> bool {
        let hsplit = usize::from(self.regs[17] & 0x1F) * 2;
        let right = self.regs[17] & 0x80 != 0;
        let vsplit = usize::from(self.regs[18] & 0x1F);
        let down = self.regs[18] & 0x80 != 0;
        let horizontal = if right { cx >= hsplit } else { cx < hsplit };
        let vertical = if down { cy >= vsplit } else { cy < vsplit };
        horizontal || vertical
    }

    /// The sprites on `line`, in drawing order, as `SpritePx` per screen column. Sets the overflow and collision flags.
    fn sprite_line(&mut self, line: usize, width: usize) -> Vec<SpritePx> {
        let h40 = self.h40();
        let table_mask = if h40 { 0x7E } else { 0x7F };
        let base = usize::from(self.regs[5] & table_mask) << 9;
        let (max_sprites, max_per_line, max_pixels) =
            if h40 { (80, 20, 320) } else { (64, 16, 256) };
        let mut visible: Vec<usize> = Vec::with_capacity(max_per_line);
        // sprite coordinates count from 128 above the picture, or in interlace mode 2 from 256 in rows twice as many
        let (cell, ymask, ybias) = if self.im2() {
            (16, 0x3FF, 0x100)
        } else {
            (8, 0x1FF, 0x80)
        };
        let picture_row = self.picture_row(line);
        let mut index = 0usize;
        for _ in 0..max_sprites {
            let entry = base + index * 8;
            let y = usize::from(self.name_entry(entry) & ymask);
            let size = self.name_entry(entry + 2);
            let height = usize::from((size >> 8) & 3) + 1;
            let row = (picture_row + ybias) as isize - y as isize;
            if (0..(height * cell) as isize).contains(&row) {
                if visible.len() == max_per_line {
                    self.sprite_overflow = true;
                    break;
                }
                visible.push(index);
            }
            index = usize::from(size & 0x7F);
            if index == 0 || index >= max_sprites {
                break;
            }
        }
        let mut out = std::mem::take(&mut self.sprite_scratch);
        out.clear();
        out.resize(width, SpritePx::default());
        let mut pixels_left = max_pixels as isize;
        let mut first = true;
        for &idx in &visible {
            let entry = base + idx * 8;
            let y = usize::from(self.name_entry(entry) & ymask) as isize - ybias as isize;
            let size = self.name_entry(entry + 2);
            let attr = self.name_entry(entry + 4);
            let raw_x = usize::from(self.name_entry(entry + 6) & 0x1FF);
            if raw_x == 0 && !first {
                break;
            }
            first = false;
            let wt = usize::from((size >> 10) & 3) + 1;
            let ht = usize::from((size >> 8) & 3) + 1;
            let x0 = raw_x as isize - 128;
            let mut row = (picture_row as isize - y) as usize;
            if attr & 0x1000 != 0 {
                row = ht * cell - 1 - row;
            }
            let hflip = attr & 0x0800 != 0;
            let palette = ((attr >> 13) & 3) as u8;
            for col in 0..wt {
                pixels_left -= 8;
                if pixels_left < 0 {
                    self.sprite_overflow = true;
                    break;
                }
                let tile_col = if hflip { wt - 1 - col } else { col };
                let pattern = usize::from(attr & 0x7FF) + tile_col * ht + row / cell;
                let mut pixels = self.pattern_row(pattern, row % cell);
                if hflip {
                    pixels.reverse();
                }
                for (i, &idx) in pixels.iter().enumerate() {
                    let sx = x0 + (col * 8 + i) as isize;
                    if idx == 0 || sx < 0 || sx >= width as isize {
                        continue;
                    }
                    let slot = &mut out[sx as usize];
                    if slot.px.opaque || slot.operator != Operator::None {
                        self.sprite_collision = true;
                        continue;
                    }
                    let operator = if self.regs[12] & 0x08 != 0 && palette == 3 && idx >= 14 {
                        if idx == 14 {
                            Operator::Highlight
                        } else {
                            Operator::Shadow
                        }
                    } else {
                        Operator::None
                    };
                    if operator != Operator::None {
                        slot.operator = operator;
                    } else {
                        slot.px = Px {
                            color: palette << 4 | idx,
                            opaque: true,
                            priority: attr & 0x8000 != 0,
                        };
                        slot.colour14 = idx == 14;
                    }
                }
            }
        }
        out
    }

    #[cfg(test)]
    pub(crate) fn rgb(&self, color: u8, brightness: Brightness) -> [u8; 3] {
        rgb_of(&self.cram, color, brightness)
    }

    /// Draws picture line `line` (0 is the first) into `out` as RGB, `width()` pixels, with the palette as it is now.
    pub fn render_line(&mut self, line: u32, out: &mut [u8]) {
        let width = self.width() as usize;
        let mut idx = [0u8; 320];
        let mut backdrop = [false; 320];
        self.render_line_indexed(line, &mut idx, &mut backdrop);
        let cram = self.cram.clone();
        let regs7 = self.regs[7] & 0x3F;
        for x in 0..width {
            let color = if backdrop[x] { regs7 } else { idx[x] & 0x3F };
            let rgb = rgb_of(&cram, color, brightness_of(idx[x]));
            out[3 * x..3 * x + 3].copy_from_slice(&rgb);
        }
    }

    /// Draws picture line `line` as palette indices: per pixel the colour (6 bits) and the brightness (the two bits above),
    /// and whether it is the backdrop colour (which follows register 7, so a change of it in the middle of the line can
    /// be applied afterwards).
    pub(crate) fn render_line_indexed(
        &mut self,
        line: u32,
        out: &mut [u8; 320],
        is_backdrop: &mut [bool; 320],
    ) {
        let width = self.width() as usize;
        let backdrop = self.regs[7] & 0x3F;
        let shadow_highlight = self.regs[12] & 0x08 != 0;
        if !self.display_enabled() {
            out[..width].fill(backdrop);
            is_backdrop[..width].fill(true);
            return;
        }
        let line = line as usize;
        let sprites = self.sprite_line(line, width);
        let row = self.picture_row(line);
        let cell_rows = if self.im2() { 16 } else { 8 };
        let wc = if self.h40() { 64 } else { 32 };
        let (hs_a, hs_b) = self.hscroll(line);
        let base_a = usize::from(self.regs[2] & 0x38) << 10;
        let base_b = usize::from(self.regs[4] & 0x07) << 13;
        let base_w = usize::from(self.regs[3] & if self.h40() { 0x3C } else { 0x3E }) << 10;
        let (wa, ha) = (plane_cells(self.regs[16]), plane_cells(self.regs[16] >> 4));
        let blank_left = self.regs[0] & 0x20 != 0;
        let (mut cache_a, mut cache_b, mut cache_w) =
            (TileRow::empty(), TileRow::empty(), TileRow::empty());
        let column_scroll = self.regs[11] & 4 != 0;
        let (mut vs_a, mut vs_b) = self.vscroll(0);
        let (wb, hb) = (plane_cells(self.regs[16]), plane_cells(self.regs[16] >> 4));
        let mut in_window = false;
        for x in 0..width {
            if x & 15 == 0 && column_scroll {
                (vs_a, vs_b) = self.vscroll(x);
            }
            if x & 7 == 0 {
                in_window = self.in_window(x >> 3, line >> 3);
            }
            let a = if in_window {
                self.plane_px_cached(&mut cache_w, base_w, wc, 32, x, row)
            } else {
                self.plane_px_cached(
                    &mut cache_a,
                    base_a,
                    wa,
                    ha,
                    (x + 1024 - hs_a) & (wa * 8 - 1),
                    (row + vs_a) & (ha * cell_rows - 1),
                )
            };
            let b = self.plane_px_cached(
                &mut cache_b,
                base_b,
                wb,
                hb,
                (x + 1024 - hs_b) & (wb * 8 - 1),
                (row + vs_b) & (hb * cell_rows - 1),
            );
            let s = sprites[x];
            let color = if blank_left && x < 8 {
                backdrop
            } else if s.px.opaque && s.px.priority {
                s.px.color
            } else if a.opaque && a.priority {
                a.color
            } else if b.opaque && b.priority {
                b.color
            } else if s.px.opaque {
                s.px.color
            } else if a.opaque {
                a.color
            } else if b.opaque {
                b.color
            } else {
                backdrop
            };
            let brightness = if shadow_highlight {
                // Darker unless some layer has its priority bit set (planes count even where transparent, sprites
                // only where drawn) or the pixel is colour 14 of a sprite.
                let mut shadow =
                    !(a.priority || b.priority || (s.px.opaque && s.px.priority) || s.colour14);
                let mut highlight = false;
                match s.operator {
                    Operator::Highlight => {
                        if shadow {
                            shadow = false;
                        } else {
                            highlight = true;
                        }
                    }
                    Operator::Shadow => shadow = true,
                    Operator::None => {}
                }
                if highlight {
                    Brightness::Highlight
                } else if shadow {
                    Brightness::Shadow
                } else {
                    Brightness::Normal
                }
            } else {
                Brightness::Normal
            };
            out[x] = color | (brightness as u8) << 6;
            is_backdrop[x] = color == backdrop && !(s.px.opaque || a.opaque || b.opaque)
                || (blank_left && x < 8);
        }
        self.sprite_scratch = sprites;
    }
}
