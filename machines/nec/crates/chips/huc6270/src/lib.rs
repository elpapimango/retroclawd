//! The Hudson HuC6270 video display controller: 32K words of VRAM, a background of 8x8 tiles in a virtual screen
//! of up to 128x64 tiles, 64 sprites of 16x16 to 32x64 pixels, VRAM and sprite-table DMA, and the interrupts
//! (collision, overflow, raster compare, DMA ends, vertical blank).
//!
//! The chip draws a line at a time into palette indexes (0-255 background, 256-511 sprites) that the colour
//! encoder (`huc6260`) turns into colours. Facts: `machines/nec/docs/hardware-facts.md`, rows V1-V30.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

const VRAM_WORDS: usize = 0x8000;

/// Status bits.
pub const ST_CR: u8 = 0x01;
pub const ST_OR: u8 = 0x02;
pub const ST_RR: u8 = 0x04;
pub const ST_DS: u8 = 0x08;
pub const ST_DV: u8 = 0x10;
pub const ST_VD: u8 = 0x20;

#[derive(Clone, Serialize, Deserialize)]
pub struct Vdc {
    #[serde(with = "serde_words")]
    vram: Vec<u16>,
    #[serde(with = "serde_words")]
    satb: Vec<u16>,
    reg: [u16; 0x14],
    select: u8,
    status: u8,
    read_buf: u16,
    low_latch: u8,
    /// A write to the SATB register asks for a transfer at the next vertical blank.
    satb_pending: bool,
    /// The background's line counter (the row of the virtual screen drawn on the current line).
    bg_y: u16,
    /// BYR was written: the next line starts from it instead of counting on.
    byr_reload: bool,
    /// The first active line of the frame, and the line counter within the display.
    active_start: u16,
    pub frame: u32,
    frame_total: u16,
}

mod serde_words {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(v: &[u16], s: S) -> Result<S::Ok, S::Error> {
        v.serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u16>, D::Error> {
        Vec::<u16>::deserialize(d)
    }
}

impl Default for Vdc {
    fn default() -> Self {
        Self::new()
    }
}

impl Vdc {
    #[must_use]
    pub fn new() -> Self {
        Self {
            vram: vec![0; VRAM_WORDS],
            satb: vec![0; 256],
            reg: [0; 0x14],
            select: 0,
            status: 0,
            read_buf: 0,
            low_latch: 0,
            satb_pending: false,
            bg_y: 0,
            byr_reload: false,
            active_start: 0,
            frame: 0,
            frame_total: 263,
        }
    }

    /// The raw VRAM, for debuggers.
    #[must_use]
    pub fn vram(&self) -> &[u16] {
        &self.vram
    }

    /// The sprite attribute table copy (256 words), for debuggers.
    #[must_use]
    pub fn satb(&self) -> &[u16] {
        &self.satb
    }

    /// A register's value (for debuggers and tests).
    #[must_use]
    pub fn register(&self, index: usize) -> u16 {
        self.reg[index]
    }

    fn increment(&self) -> u16 {
        [1, 32, 64, 128][usize::from((self.reg[5] >> 11) & 3)]
    }

    /// A read of port 0 (status), 2 (data low) or 3 (data high). Reading the status clears the flags.
    pub fn read(&mut self, port: u8) -> u8 {
        match port & 3 {
            0 => {
                let s = self.status;
                self.status &= !(ST_CR | ST_OR | ST_RR | ST_DS | ST_DV | ST_VD);
                s
            }
            1 => 0,
            2 => {
                if self.select == 2 {
                    self.read_buf as u8
                } else {
                    0
                }
            }
            _ => {
                if self.select == 2 {
                    let v = (self.read_buf >> 8) as u8;
                    self.reg[1] = self.reg[1].wrapping_add(self.increment());
                    self.read_buf = self.vram[usize::from(self.reg[1]) & (VRAM_WORDS - 1)];
                    v
                } else {
                    0
                }
            }
        }
    }

    /// A write to port 0 (register select), 2 (data low) or 3 (data high).
    pub fn write(&mut self, port: u8, v: u8) {
        match port & 3 {
            0 => self.select = v & 0x1F,
            1 => {}
            2 => {
                let r = usize::from(self.select);
                if r < 0x14 {
                    if r == 2 {
                        self.low_latch = v;
                    } else {
                        self.reg[r] = (self.reg[r] & 0xFF00) | u16::from(v);
                    }
                }
            }
            _ => {
                let r = usize::from(self.select);
                if r >= 0x14 {
                    return;
                }
                if r != 2 {
                    self.reg[r] = (self.reg[r] & 0x00FF) | (u16::from(v) << 8);
                }
                self.after_high_write(r, v);
            }
        }
    }

    fn after_high_write(&mut self, r: usize, v: u8) {
        match r {
            1 => {
                self.read_buf = self.vram[usize::from(self.reg[1]) & (VRAM_WORDS - 1)];
            }
            2 => {
                let word = (u16::from(v) << 8) | u16::from(self.low_latch);
                self.vram[usize::from(self.reg[0]) & (VRAM_WORDS - 1)] = word;
                self.reg[0] = self.reg[0].wrapping_add(self.increment());
            }
            8 => self.byr_reload = true,
            0x12 => self.vram_dma(),
            0x13 => self.satb_pending = true,
            _ => {}
        }
    }

    fn vram_dma(&mut self) {
        let dcr = self.reg[0x0F];
        let mut src = self.reg[0x10];
        let mut dst = self.reg[0x11];
        let count = u32::from(self.reg[0x12]) + 1;
        for _ in 0..count {
            let w = self.vram[usize::from(src) & (VRAM_WORDS - 1)];
            self.vram[usize::from(dst) & (VRAM_WORDS - 1)] = w;
            src = if dcr & 4 != 0 {
                src.wrapping_sub(1)
            } else {
                src.wrapping_add(1)
            };
            dst = if dcr & 8 != 0 {
                dst.wrapping_sub(1)
            } else {
                dst.wrapping_add(1)
            };
        }
        self.reg[0x10] = src;
        self.reg[0x11] = dst;
        if dcr & 2 != 0 {
            self.status |= ST_DV;
        }
    }

    fn satb_dma(&mut self) {
        let base = usize::from(self.reg[0x13]);
        for i in 0..256 {
            self.satb[i] = self.vram[(base + i) & (VRAM_WORDS - 1)];
        }
        if self.reg[0x0F] & 1 != 0 {
            self.status |= ST_DS;
        }
    }

    /// Whether the interrupt line to the CPU (`IRQ1`) is asserted.
    #[must_use]
    pub fn irq(&self) -> bool {
        let cr = self.reg[5];
        let dcr = self.reg[0x0F];
        let s = self.status;
        (s & ST_CR != 0 && cr & 1 != 0)
            || (s & ST_OR != 0 && cr & 2 != 0)
            || (s & ST_RR != 0 && cr & 4 != 0)
            || (s & ST_VD != 0 && cr & 8 != 0)
            || (s & ST_DS != 0 && dcr & 1 != 0)
            || (s & ST_DV != 0 && dcr & 2 != 0)
    }

    /// The visible width in dots: `(HDW + 1) * 8`.
    #[must_use]
    pub fn display_width(&self) -> u16 {
        (self.reg[0x0B] & 0x7F) * 8 + 8
    }

    /// The number of active lines: `VDW + 1`.
    #[must_use]
    pub fn display_height(&self) -> u16 {
        (self.reg[0x0D] & 0x1FF) + 1
    }

    /// The number of lines in a frame: the lines before the picture, the picture and the `VCR` lines after it.
    /// Latched at the start of each frame, clamped to what a television can show.
    #[must_use]
    pub fn frame_lines(&self) -> u16 {
        self.frame_total
    }

    /// The first active line of the frame as latched when the frame began.
    #[must_use]
    pub fn frame_start(&self) -> u16 {
        self.active_start
    }

    /// The first active line of the frame (counted from the first line of the frame).
    #[must_use]
    pub fn active_start(&self) -> u16 {
        let vsw = self.reg[0x0C] & 0x1F;
        let vds = self.reg[0x0C] >> 8;
        vsw + vds + 2
    }

    /// Called at the start of every line of the frame. `line` counts from 0 at the first line of the frame.
    /// Sets the line-based status flags and, for an active line, draws it into `out` (palette indexes, one per
    /// dot, `display_width()` of them) and returns its row within the picture (0 for the first active line).
    pub fn start_line(&mut self, line: u16, out: &mut Vec<u16>) -> Option<u16> {
        if line == 0 {
            self.frame = self.frame.wrapping_add(1);
            self.active_start = self.active_start();
            let total = u32::from(self.active_start)
                + u32::from(self.display_height())
                + u32::from(self.reg[0x0E] & 0xFF);
            self.frame_total = total.clamp(262, 525) as u16;
        }
        let start = self.active_start;
        let height = self.display_height();
        let active = line >= start && line < start + height;
        // raster compare: the counter is 64 on the first active line
        if line >= start {
            let counter = 64 + (line - start);
            if counter == (self.reg[6] & 0x3FF) && self.reg[5] & 4 != 0 {
                self.status |= ST_RR;
            }
        }
        if line == start + height {
            if self.reg[5] & 8 != 0 {
                self.status |= ST_VD;
            }
            if self.satb_pending || self.reg[0x0F] & 0x10 != 0 {
                self.satb_pending = false;
                self.satb_dma();
            }
        }
        if !active {
            return None;
        }
        if line == start {
            self.bg_y = self.reg[8] & 0x1FF;
            self.byr_reload = false;
        } else if self.byr_reload {
            self.bg_y = self.reg[8] & 0x1FF;
            self.byr_reload = false;
        } else {
            self.bg_y = (self.bg_y + 1) & 0x1FF;
        }
        self.draw_line(line - start, out);
        Some(line - start)
    }

    fn draw_line(&mut self, row: u16, out: &mut Vec<u16>) {
        let width = usize::from(self.display_width());
        out.clear();
        out.resize(width, 0);
        let cr = self.reg[5];
        let mut bg_opaque = vec![false; width];
        if cr & 0x80 != 0 {
            self.draw_background(out, &mut bg_opaque);
        }
        if cr & 0x40 != 0 {
            self.draw_sprites(row, out, &bg_opaque);
        }
    }

    fn draw_background(&self, out: &mut [u16], opaque: &mut [bool]) {
        let mwr = self.reg[9];
        let vw_tiles = [32usize, 64, 128, 128][usize::from((mwr >> 4) & 3)];
        let vh_tiles = if mwr & 0x40 != 0 { 64usize } else { 32 };
        let vy = usize::from(self.bg_y) & (vh_tiles * 8 - 1);
        let bxr = usize::from(self.reg[7] & 0x3FF);
        let ty = vy >> 3;
        let fine_y = vy & 7;
        for (x, px) in out.iter_mut().enumerate() {
            let vx = (bxr + x) & (vw_tiles * 8 - 1);
            let entry = self.vram[(ty * vw_tiles + (vx >> 3)) & (VRAM_WORDS - 1)];
            let tile = usize::from(entry & 0x0FFF);
            let pal = entry >> 12;
            let addr = tile * 16 + fine_y;
            let p01 = self.vram[addr & (VRAM_WORDS - 1)];
            let p23 = self.vram[(addr + 8) & (VRAM_WORDS - 1)];
            let bit = 7 - (vx & 7);
            let pix = ((p01 >> bit) & 1)
                | (((p01 >> (8 + bit)) & 1) << 1)
                | (((p23 >> bit) & 1) << 2)
                | (((p23 >> (8 + bit)) & 1) << 3);
            if pix != 0 {
                *px = (pal << 4) | pix;
                opaque[x] = true;
            }
        }
    }

    fn draw_sprites(&mut self, row: u16, out: &mut [u16], bg_opaque: &[bool]) {
        // sprite coordinates: Y = line + 64 and X = dot + 32 on the sprite table's scale
        let line = i32::from(row);
        let width = out.len() as i32;
        let mut chosen: Vec<usize> = Vec::with_capacity(16);
        let mut cells_used = 0u32;
        let mut overflow = false;
        for i in 0..64 {
            let y = i32::from(self.satb[i * 4] & 0x3FF) - 64;
            let attr = self.satb[i * 4 + 3];
            let h = [16, 32, 64, 64][usize::from((attr >> 12) & 3)];
            if line < y || line >= y + h {
                continue;
            }
            let cells = if attr & 0x100 != 0 { 2 } else { 1 };
            if cells_used + cells > 16 {
                overflow = true;
                continue;
            }
            cells_used += cells;
            chosen.push(i);
        }
        if overflow && self.reg[5] & 2 != 0 {
            self.status |= ST_OR;
        }
        let bg_line = out.to_vec();
        let mut sprite0_mask = vec![false; out.len()];
        // draw in reverse so a lower index lands on top; `owner` records the winner for the priority test
        for &i in chosen.iter().rev() {
            let y = i32::from(self.satb[i * 4] & 0x3FF) - 64;
            let x0 = i32::from(self.satb[i * 4 + 1] & 0x3FF) - 32;
            let pat = self.satb[i * 4 + 2];
            let attr = self.satb[i * 4 + 3];
            let wide = attr & 0x100 != 0;
            let hmode = usize::from((attr >> 12) & 3);
            let h = [16, 32, 64, 64][hmode];
            let pal = 256 + ((attr & 0x0F) << 4);
            let hflip = attr & 0x800 != 0;
            let vflip = attr & 0x8000 != 0;
            let mut n = usize::from((pat >> 1) & 0x3FF);
            if wide {
                n &= !1;
            }
            match hmode {
                1 => n &= !2,
                2 | 3 => n &= !6,
                _ => {}
            }
            let mut r = (line - y) as usize;
            if vflip {
                r = (h as usize) - 1 - r;
            }
            let cell_row = r >> 4;
            let in_row = r & 15;
            let total_w = if wide { 32 } else { 16 };
            for sx in 0..total_w {
                let dx = x0 + sx;
                if dx < 0 || dx >= width {
                    continue;
                }
                let fx = if hflip { total_w - 1 - sx } else { sx } as usize;
                let cell_x = fx >> 4;
                let bit = 15 - (fx & 15);
                let cell = n + cell_x + cell_row * 2;
                let base = (cell << 6) + in_row;
                let p0 = (self.vram[base & (VRAM_WORDS - 1)] >> bit) & 1;
                let p1 = (self.vram[(base + 16) & (VRAM_WORDS - 1)] >> bit) & 1;
                let p2 = (self.vram[(base + 32) & (VRAM_WORDS - 1)] >> bit) & 1;
                let p3 = (self.vram[(base + 48) & (VRAM_WORDS - 1)] >> bit) & 1;
                let pix = p0 | (p1 << 1) | (p2 << 2) | (p3 << 3);
                if pix == 0 {
                    continue;
                }
                let d = dx as usize;
                if i == 0 {
                    sprite0_mask[d] = true;
                } else if sprite0_mask[d] {
                    if self.reg[5] & 1 != 0 {
                        self.status |= ST_CR;
                    }
                }
                let behind = attr & 0x80 == 0 && bg_opaque[d];
                // a lower-numbered sprite wins the dot even when it hides behind the background
                out[d] = if behind { bg_line[d] } else { pal + pix };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_reg(v: &mut Vdc, r: u8, value: u16) {
        v.write(0, r);
        v.write(2, value as u8);
        v.write(3, (value >> 8) as u8);
    }

    #[test]
    fn vram_write_and_read_back_with_increment() {
        let mut v = Vdc::new();
        setup_reg(&mut v, 0, 0x100); // MAWR
        v.write(0, 2);
        v.write(2, 0x34);
        v.write(3, 0x12);
        v.write(2, 0x78);
        v.write(3, 0x56);
        assert_eq!(v.vram()[0x100], 0x1234);
        assert_eq!(v.vram()[0x101], 0x5678);
        setup_reg(&mut v, 1, 0x100); // MARR
        v.write(0, 2);
        assert_eq!(v.read(2), 0x34);
        assert_eq!(v.read(3), 0x12);
        assert_eq!(v.read(2), 0x78);
    }

    #[test]
    fn background_tile_pixels() {
        let mut v = Vdc::new();
        // tile 1: row 0 plane0 = 0x80 (leftmost pixel colour 1)
        v.vram[16] = 0x0080;
        // BAT entry (0,0) = tile 1, palette 3
        v.vram[0] = 0x3001;
        setup_reg(&mut v, 5, 0x0080); // BG on
        setup_reg(&mut v, 0x0B, 0x001F); // 256 wide (HDW is the low byte of HDR)
        setup_reg(&mut v, 0x0D, 0x00EF);
        setup_reg(&mut v, 0x0C, 0x0F02);
        let mut line = Vec::new();
        let start = v.active_start();
        for l in 0..=start {
            let drew = v.start_line(l, &mut line);
            assert_eq!(drew.is_some(), l == start);
        }
        assert_eq!(line.len(), 256);
        assert_eq!(line[0], (3 << 4) | 1);
        assert_eq!(line[1], 0);
    }

    #[test]
    fn a_sprite_shows_its_rows() {
        let mut v = Vdc::new();
        // pattern 1 (cell at word 64): plane 0 rows: row 1 = 0xF000 (leftmost 4 dots), row 2 = 0x0F00
        v.vram[64 + 1] = 0xF000;
        v.vram[64 + 2] = 0x0F00;
        // sprite 0: y=64+10 (line 10), x=32+8 (dot 8), pattern number 1 (bits 1-10 of the word => 2), palette 5
        v.satb[0] = 64 + 10;
        v.satb[1] = 32 + 8;
        v.satb[2] = 1 << 1;
        v.satb[3] = 0x0080 | 5;
        setup_reg(&mut v, 5, 0x0040); // sprites on
        setup_reg(&mut v, 0x0B, 0x001F);
        setup_reg(&mut v, 0x0D, 0x00EF);
        setup_reg(&mut v, 0x0C, 0x0F02);
        let start = v.active_start();
        let mut line = Vec::new();
        for l in 0..start + 13 {
            let drew = v.start_line(l, &mut line);
            if l == start + 11 {
                assert!(drew.is_some());
                assert_eq!(line[8], 256 + (5 << 4) + 1);
                assert_eq!(line[11], 256 + (5 << 4) + 1);
                assert_eq!(line[12], 0);
            }
            if l == start + 12 {
                assert_eq!(line[8], 0);
                assert_eq!(line[12], 256 + (5 << 4) + 1);
            }
        }
    }

    #[test]
    fn vblank_flag_and_irq() {
        let mut v = Vdc::new();
        setup_reg(&mut v, 5, 0x0008); // vblank interrupt enable
        setup_reg(&mut v, 0x0D, 0x0003);
        setup_reg(&mut v, 0x0C, 0x0000);
        let mut line = Vec::new();
        let end = v.active_start() + v.display_height();
        for l in 0..end {
            v.start_line(l, &mut line);
            assert!(!v.irq(), "line {l}");
        }
        v.start_line(end, &mut line);
        assert!(v.irq());
        assert_eq!(v.read(0) & ST_VD, ST_VD);
        assert!(!v.irq());
    }
}
