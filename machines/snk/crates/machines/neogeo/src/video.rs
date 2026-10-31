//! Drawing a frame from the video RAM, the palette and the graphics ROMs (facts N29-N33). A first renderer: the fix layer and
//! the sprites, one scanline at a time; the hardware's shrinking is approximated and the sprite limits are not modelled.

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 224;

/// How a 16x16 sprite tile is stored in the interleaved pair (fact N31): which stored 8x8 block holds each quadrant, which
/// of the four bytes of a line holds each bitplane, and whether bit 7 is the leftmost pixel.
#[derive(Clone, Copy, Debug)]
pub struct SpriteLayout {
    /// For the quadrants top-left, top-right, bottom-left, bottom-right: the block's number in the tile.
    pub blocks: [u8; 4],
    /// For bitplanes 0 to 3: the byte (0-3) of a stored line that holds it.
    pub planes: [u8; 4],
    pub msb_left: bool,
}

impl Default for SpriteLayout {
    fn default() -> Self {
        SpriteLayout {
            blocks: [2, 0, 3, 1],
            planes: [0, 2, 1, 3],
            msb_left: false,
        }
    }
}

/// What a frame is drawn from.
pub struct Sources<'a> {
    pub vram: &'a [u16],
    /// The visible palette bank, 8 KiB (4096 colours as 16-bit big-endian words).
    pub palette: &'a [u8],
    /// The fix layer's tiles (the board's `sfix` or the cartridge's S ROM).
    pub fix: &'a [u8],
    /// The sprite data, the C ROM pairs already interleaved (`neo_set::NeoSet::c`).
    pub sprites: &'a [u8],
    pub layout: SpriteLayout,
    /// The vertical shrink table (`000-lo.lo`: 256 rows of 256 source lines, `$FF` for none).
    pub lo: &'a [u8],
    /// The auto-animation counter (fact N34).
    pub anim: u8,
}

/// A palette word as 0x00RRGGBB (fact N29).
pub fn colour(word: u16) -> u32 {
    let dark = u32::from(word >> 15);
    // the lowest bit of each channel is stored apart (bits 14-12), the other four in each nibble
    let channel = |low: u16, high: u16| -> u32 {
        let v = u32::from(high << 1 | low) & 0x1F; // 5 bits
        let v = (v << 3) | (v >> 2); // to 8 bits
        if dark == 1 { v * 3 / 4 } else { v }
    };
    let r = channel((word >> 14) & 1, (word >> 8) & 0xF);
    let g = channel((word >> 13) & 1, (word >> 4) & 0xF);
    let b = channel((word >> 12) & 1, word & 0xF);
    r << 16 | g << 8 | b
}

fn palette_colour(p: &[u8], palette: usize, index: usize) -> u32 {
    let at = (palette * 16 + index) * 2;
    colour(u16::from(p[at]) << 8 | u16::from(p[at + 1]))
}

/// The 4-bit pixel (x, y) of an 8x8 fix tile (fact N30).
fn fix_pixel(fix: &[u8], tile: usize, x: usize, y: usize) -> u8 {
    let at = tile * 32 + column_byte(x) * 8 + y;
    let byte = fix.get(at).copied().unwrap_or(0);
    if x & 1 == 0 { byte & 0xF } else { byte >> 4 }
}

/// Which of the four stored columns holds pixel `x` of a fix tile.
fn column_byte(x: usize) -> usize {
    [2, 2, 3, 3, 0, 0, 1, 1][x]
}

/// The 4-bit pixel (x, y) of a 16x16 sprite tile (fact N31).
fn sprite_pixel(sprites: &[u8], layout: &SpriteLayout, tile: usize, x: usize, y: usize) -> u8 {
    let block = usize::from(layout.blocks[(y / 8) * 2 + x / 8]);
    let base = tile * 128 + block * 32 + (y % 8) * 4;
    let bit = if layout.msb_left { 7 - (x % 8) } else { x % 8 };
    let mut v = 0u8;
    for (plane, byte) in layout.planes.iter().enumerate() {
        let b = sprites.get(base + usize::from(*byte)).copied().unwrap_or(0);
        v |= ((b >> bit) & 1) << plane;
    }
    v
}

/// Draws one frame into `out` (WIDTH x HEIGHT, 0x00RRGGBB).
pub fn render(src: &Sources, out: &mut [u32]) {
    let backdrop = palette_colour(src.palette, 255, 0);
    for y in 0..HEIGHT {
        let line = &mut out[y * WIDTH..(y + 1) * WIDTH];
        line.fill(backdrop);
        draw_sprites(src, y, line);
        draw_fix(src, y, line);
    }
}

fn draw_fix(src: &Sources, y: usize, line: &mut [u32]) {
    let row = y / 8 + 2; // the visible rows start at 2 (fact N30)
    for col in 0..40 {
        let entry = src.vram[0x7000 + col * 32 + row];
        let (palette, tile) = (usize::from(entry >> 12), usize::from(entry & 0xFFF));
        for x in 0..8 {
            let px = fix_pixel(src.fix, tile, x, y % 8);
            if px != 0 {
                line[col * 8 + x] = palette_colour(src.palette, palette, usize::from(px));
            }
        }
    }
}

fn draw_sprites(src: &Sources, y: usize, line: &mut [u32]) {
    // a chain of sticky sprites keeps the first one's height, Y and vertical shrink (fact N32); each continues to the right of the
    // previous one by that one's shrunk width (fact N36)
    let mut height = 0usize;
    let mut sx = 0i32;
    let mut sy = 0i32;
    let mut vshrink = 0xFFusize;
    let mut prev_width = 16i32;
    for n in 0..381usize {
        let scb2 = src.vram[0x8000 + n];
        let scb3 = src.vram[0x8200 + n];
        let scb4 = src.vram[0x8400 + n];
        let width = usize::from((scb2 >> 8) & 0xF) + 1;
        if scb3 & 0x40 == 0 {
            height = usize::from(scb3 & 0x3F);
            sy = i32::from(scb3 >> 7);
            sx = i32::from(scb4 >> 7);
            vshrink = usize::from(scb2 & 0xFF);
        } else {
            sx += prev_width;
        }
        prev_width = width as i32;
        if height == 0 {
            continue;
        }
        let top = (496 - sy) & 0x1FF; // the sprite's top line on the screen (fact N32)
        let rel = (y as i32 - top) & 0x1FF;
        // the source line of this output line: all of them at full height, else the vertical shrink table's (fact N36)
        let source = if vshrink == 0xFF {
            rel as usize
        } else if rel < 256 {
            match src.lo.get(vshrink * 256 + rel as usize).copied() {
                Some(0xFF) | None => continue,
                Some(b) => usize::from(b),
            }
        } else {
            continue;
        };
        if source >= height.min(32) * 16 {
            continue;
        }
        let tile_row = source / 16;
        let py = source % 16;
        let entry = n * 64 + tile_row * 2;
        let (lo, attr) = (usize::from(src.vram[entry]), src.vram[entry + 1]);
        let mut tile = lo | usize::from((attr >> 4) & 0xF) << 16;
        // auto-animation: the low two or three bits of the tile come from the counter (fact N34)
        if attr & 8 != 0 {
            tile = (tile & !3) | usize::from(src.anim & 3);
        } else if attr & 4 != 0 {
            tile = (tile & !7) | usize::from(src.anim & 7);
        }
        let palette = usize::from(attr >> 8);
        let (hflip, vflip) = (attr & 1 != 0, attr & 2 != 0);
        let py = if vflip { 15 - py } else { py };
        for dx in 0..width {
            let screen = (sx + dx as i32) & 0x1FF;
            let screen = if screen >= 0x1F0 {
                screen - 0x200
            } else {
                screen
            };
            if !(0..WIDTH as i32).contains(&screen) {
                continue;
            }
            // the columns kept of the 16 (fact N36: spread evenly, the hardware's pattern is not known)
            let x = dx * 16 / width;
            let px = sprite_pixel(
                src.sprites,
                &src.layout,
                tile,
                if hflip { 15 - x } else { x },
                py,
            );
            if px != 0 {
                line[screen as usize] = palette_colour(src.palette, palette, usize::from(px));
            }
        }
    }
}
