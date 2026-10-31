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
    let backdrop = palette_colour(src.palette, 255, 15);
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
    // a sprite's height is kept by the first sprite of a sticky chain (fact N32)
    let mut height = 0usize;
    let mut sx = 0i32;
    let mut sy = 0i32;
    for n in 0..381usize {
        let scb3 = src.vram[0x8200 + n];
        let scb4 = src.vram[0x8400 + n];
        if scb3 & 0x40 == 0 {
            height = usize::from(scb3 & 0x3F);
            sy = i32::from(scb3 >> 7);
            sx = i32::from(scb4 >> 7);
        } else {
            sx += 16; // sticky: the next column of the chain, one tile to the right
        }
        if height == 0 {
            continue;
        }
        let top = (496 - sy) & 0x1FF; // the sprite's top line on the screen (fact N32)
        let rel = (y as i32 - top) & 0x1FF;
        if rel as usize >= height.min(32) * 16 {
            continue;
        }
        let tile_row = rel as usize / 16;
        let py = rel as usize % 16;
        let entry = n * 64 + tile_row * 2;
        let (lo, attr) = (usize::from(src.vram[entry]), src.vram[entry + 1]);
        let tile = lo | usize::from((attr >> 4) & 0xF) << 16;
        let palette = usize::from(attr >> 8);
        let (hflip, vflip) = (attr & 1 != 0, attr & 2 != 0);
        let py = if vflip { 15 - py } else { py };
        for x in 0..16 {
            let screen = (sx + x as i32) & 0x1FF;
            let screen = if screen >= 0x1F0 {
                screen - 0x200
            } else {
                screen
            };
            if !(0..WIDTH as i32).contains(&screen) {
                continue;
            }
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
