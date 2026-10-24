//! Renderer tests: hand-made VRAM, expected pixels from the layout facts and the Plutiedev layer/sprite/shadow pages.

use crate::Vdp;
use crate::render::Brightness;

/// A VDP with the display on in H32: plane A at $C000, plane B at $E000, window at $8000, sprites at $D800, hscroll at
/// $F000; CRAM 0 grey, 1-3 blue, green, red.
fn vdp() -> Vdp {
    let mut v = Vdp::new(false);
    v.regs[1] = 0x74;
    v.regs[2] = 0x30;
    v.regs[4] = 0x07;
    v.regs[3] = 0x20;
    v.regs[5] = 0x6C;
    v.regs[13] = 0x3C;
    v.cram[0] = 0o111;
    v.cram[1] = 0o700;
    v.cram[2] = 0o070;
    v.cram[3] = 0o007;
    v
}

fn line(v: &mut Vdp, y: u32) -> Vec<[u8; 3]> {
    let mut out = vec![0u8; 3 * v.width() as usize];
    v.render_line(y, &mut out);
    out.chunks(3).map(|c| [c[0], c[1], c[2]]).collect()
}

fn colour(v: &Vdp, index: u8) -> [u8; 3] {
    v.rgb(index, Brightness::Normal)
}

/// Pattern `n` with every pixel colour `idx`.
fn solid_tile(v: &mut Vdp, n: usize, idx: u8) {
    for i in 0..32 {
        v.vram[n * 32 + i] = idx << 4 | idx;
    }
}

fn put_name(v: &mut Vdp, base: usize, wc: usize, col: usize, row: usize, entry: u16) {
    let a = base + 2 * (row * wc + col);
    v.vram[a] = (entry >> 8) as u8;
    v.vram[a + 1] = entry as u8;
}

fn put_sprite(v: &mut Vdp, n: usize, y: u16, size: u16, link: u16, attr: u16, x: u16) {
    let a = 0xD800 + 8 * n;
    for (i, w) in [y, size << 8 | link, attr, x].iter().enumerate() {
        v.vram[a + 2 * i] = (w >> 8) as u8;
        v.vram[a + 2 * i + 1] = *w as u8;
    }
}

#[test]
fn a_blank_display_shows_the_backdrop() {
    let mut v = vdp();
    v.regs[1] = 0x04;
    v.regs[7] = 0x00;
    let l = line(&mut v, 10);
    assert!(l.iter().all(|p| *p == colour(&v, 0)));
}

#[test]
fn one_tile_of_plane_a_shows_where_the_name_table_puts_it() {
    let mut v = vdp();
    solid_tile(&mut v, 1, 3);
    put_name(&mut v, 0xC000, 64, 2, 0, 1);
    v.regs[16] = 0x01; // 64 x 32 cells
    let l = line(&mut v, 3);
    assert_eq!(l[15], colour(&v, 0));
    assert_eq!(l[16], colour(&v, 3));
    assert_eq!(l[23], colour(&v, 3));
    assert_eq!(l[24], colour(&v, 0));
}

#[test]
fn flips_and_palettes_come_from_the_name_entry() {
    let mut v = vdp();
    // a tile with only its top-left pixel set
    v.vram[32] = 0x10;
    put_name(&mut v, 0xC000, 32, 0, 0, 1);
    put_name(&mut v, 0xC000, 32, 1, 0, 1 | 0x0800);
    put_name(&mut v, 0xC000, 32, 2, 0, 1 | 0x1000);
    put_name(&mut v, 0xC000, 32, 3, 0, 1 | 0x2000);
    v.cram[0x11] = 0o444;
    assert_eq!(line(&mut v, 0)[0], colour(&v, 1));
    assert_eq!(line(&mut v, 0)[8 + 7], colour(&v, 1), "h flip");
    assert_eq!(
        line(&mut v, 0)[16],
        colour(&v, 0),
        "v flip: the set row is the bottom one"
    );
    assert_eq!(line(&mut v, 7)[16], colour(&v, 1));
    assert_eq!(line(&mut v, 0)[24], colour(&v, 0x11), "palette 1");
}

#[test]
fn horizontal_scroll_moves_the_plane_to_the_right() {
    let mut v = vdp();
    solid_tile(&mut v, 1, 3);
    put_name(&mut v, 0xC000, 32, 0, 0, 1);
    v.vram[0xF000] = 0;
    v.vram[0xF001] = 4; // plane A scrolled by 4
    let l = line(&mut v, 0);
    assert_eq!(l[3], colour(&v, 0));
    assert_eq!(l[4], colour(&v, 3));
    assert_eq!(l[11], colour(&v, 3));
    assert_eq!(l[12], colour(&v, 0));
}

#[test]
fn per_line_horizontal_scroll_uses_one_entry_per_line() {
    let mut v = vdp();
    v.regs[11] = 3;
    solid_tile(&mut v, 1, 3);
    put_name(&mut v, 0xC000, 32, 0, 0, 1);
    v.vram[0xF000 + 4 + 1] = 8; // line 1 only
    assert_eq!(line(&mut v, 0)[0], colour(&v, 3));
    assert_eq!(line(&mut v, 1)[0], colour(&v, 0));
    assert_eq!(line(&mut v, 1)[8], colour(&v, 3));
}

#[test]
fn vertical_scroll_full_screen_and_per_two_cells() {
    let mut v = vdp();
    solid_tile(&mut v, 1, 3);
    put_name(&mut v, 0xC000, 32, 0, 1, 1);
    put_name(&mut v, 0xC000, 32, 2, 1, 1);
    v.vsram[0] = 8; // plane A scrolled up one cell: row 1 shows at line 0
    assert_eq!(line(&mut v, 0)[0], colour(&v, 3));
    assert_eq!(line(&mut v, 0)[16], colour(&v, 3));
    v.regs[11] = 4;
    v.vsram[0] = 8;
    v.vsram[2] = 0; // second two-cell column: no scroll
    assert_eq!(line(&mut v, 0)[0], colour(&v, 3));
    assert_eq!(line(&mut v, 0)[16], colour(&v, 0));
}

#[test]
fn the_window_replaces_plane_a_inside_its_area() {
    let mut v = vdp();
    solid_tile(&mut v, 1, 3);
    solid_tile(&mut v, 2, 2);
    for c in 0..32 {
        put_name(&mut v, 0xC000, 32, c, 0, 1);
        put_name(&mut v, 0x8000, 32, c, 0, 2);
    }
    v.regs[17] = 0x01; // window left of cell 2
    let l = line(&mut v, 0);
    assert_eq!(l[0], colour(&v, 2));
    assert_eq!(l[15], colour(&v, 2));
    assert_eq!(l[16], colour(&v, 3));
    v.regs[17] = 0x81; // right of cell 2
    let l = line(&mut v, 0);
    assert_eq!(l[0], colour(&v, 3));
    assert_eq!(l[16], colour(&v, 2));
    v.regs[17] = 0;
    v.regs[18] = 0x01; // above row 1
    assert_eq!(line(&mut v, 7)[100], colour(&v, 2));
    assert_eq!(line(&mut v, 8)[100], colour(&v, 0));
}

#[test]
fn a_sprite_is_drawn_at_its_position_with_the_priority_rules() {
    let mut v = vdp();
    solid_tile(&mut v, 1, 3);
    solid_tile(&mut v, 2, 2);
    for c in 0..32 {
        put_name(&mut v, 0xE000, 32, c, 0, 2);
    }
    // one 1x1 sprite at screen (16, 0), low priority: over plane B
    put_sprite(&mut v, 0, 128, 0, 0, 1, 128 + 16);
    let l = line(&mut v, 0);
    assert_eq!(l[15], colour(&v, 2));
    assert_eq!(l[16], colour(&v, 3));
    assert_eq!(l[23], colour(&v, 3));
    assert_eq!(l[24], colour(&v, 2));
    // plane B with its priority bit beats a low priority sprite
    for c in 0..32 {
        put_name(&mut v, 0xE000, 32, c, 0, 2 | 0x8000);
    }
    assert_eq!(line(&mut v, 0)[16], colour(&v, 2));
    // a high priority sprite beats it
    put_sprite(&mut v, 0, 128, 0, 0, 1 | 0x8000, 128 + 16);
    assert_eq!(line(&mut v, 0)[16], colour(&v, 3));
}

#[test]
fn sprites_are_column_major_and_flip_as_a_whole() {
    let mut v = vdp();
    solid_tile(&mut v, 1, 1);
    solid_tile(&mut v, 2, 2);
    solid_tile(&mut v, 3, 3);
    // a 2 wide x 1 high sprite: tiles 1 then 2 (columns)
    put_sprite(&mut v, 0, 128, 0x04, 0, 1, 128);
    let l = line(&mut v, 0);
    assert_eq!(l[0], colour(&v, 1));
    assert_eq!(l[8], colour(&v, 2));
    put_sprite(&mut v, 0, 128, 0x04, 0, 1 | 0x0800, 128);
    let l = line(&mut v, 0);
    assert_eq!(l[0], colour(&v, 2));
    assert_eq!(l[8], colour(&v, 1));
    // 1 wide x 2 high: tiles 1 (top) then 2 (bottom)
    put_sprite(&mut v, 0, 128, 0x01, 0, 1, 128);
    assert_eq!(line(&mut v, 0)[0], colour(&v, 1));
    assert_eq!(line(&mut v, 8)[0], colour(&v, 2));
}

#[test]
fn the_first_sprite_in_the_list_is_on_top() {
    let mut v = vdp();
    solid_tile(&mut v, 1, 1);
    solid_tile(&mut v, 2, 2);
    put_sprite(&mut v, 0, 128, 0, 1, 1, 128);
    put_sprite(&mut v, 1, 128, 0, 0, 2, 128);
    assert_eq!(line(&mut v, 0)[0], colour(&v, 1));
    assert!(v.sprite_collision, "the overlap sets the collision flag");
}

#[test]
fn a_sprite_link_loop_ends_and_sprites_beyond_the_line_limit_are_dropped() {
    let mut v = vdp();
    solid_tile(&mut v, 1, 3);
    // sprite 0 links to 1 and 1 links back to 0, which ends the list
    put_sprite(&mut v, 0, 128, 0, 1, 1, 128);
    put_sprite(&mut v, 1, 128, 0, 0, 1, 136);
    assert_eq!(line(&mut v, 0)[0], colour(&v, 3));
    // 17 sprites on one line in H32 (the limit is 16)
    let mut v = vdp();
    solid_tile(&mut v, 1, 3);
    for n in 0..17 {
        put_sprite(
            &mut v,
            n,
            128,
            0,
            (n + 1) as u16,
            1,
            128 + 8 * (n as u16 % 31),
        );
    }
    let _ = line(&mut v, 0);
    assert!(v.sprite_overflow);
}

#[test]
fn a_sprite_at_x_zero_hides_the_sprites_after_it() {
    let mut v = vdp();
    v.regs[12] = 0x81;
    solid_tile(&mut v, 1, 3);
    put_sprite(&mut v, 0, 128, 0, 1, 1, 128 + 40);
    put_sprite(&mut v, 1, 128, 0, 2, 1, 0);
    put_sprite(&mut v, 2, 128, 0, 0, 1, 128 + 80);
    let l = line(&mut v, 0);
    assert_eq!(l[40], colour(&v, 3));
    assert_eq!(l[80], colour(&v, 0), "masked by the sprite at x = 0");
}

#[test]
fn the_left_column_can_be_blanked() {
    let mut v = vdp();
    solid_tile(&mut v, 1, 3);
    put_name(&mut v, 0xC000, 32, 0, 0, 1);
    put_name(&mut v, 0xC000, 32, 1, 0, 1);
    v.regs[0] = 0x24;
    let l = line(&mut v, 0);
    assert_eq!(l[0], colour(&v, 0));
    assert_eq!(l[8], colour(&v, 3));
}

#[test]
fn shadow_highlight_darkens_unless_a_layer_has_priority_and_sprites_can_highlight() {
    let mut v = vdp();
    v.regs[12] = 0x08;
    solid_tile(&mut v, 1, 3);
    put_name(&mut v, 0xE000, 32, 0, 0, 1); // plane B, low priority: shadowed
    put_name(&mut v, 0xE000, 32, 1, 0, 1 | 0x8000); // high priority: normal
    let l = line(&mut v, 0);
    assert_eq!(l[0], v.rgb(3, Brightness::Shadow));
    assert_eq!(l[8], v.rgb(3, Brightness::Normal));
    // palette 3 colour 14 (highlight) and 15 (shadow) sprites change the pixel under them
    for i in 0..32 {
        v.vram[2 * 32 + i] = 0xEE;
        v.vram[3 * 32 + i] = 0xFF;
    }
    put_sprite(&mut v, 0, 128, 0, 1, 2 | 0x6000, 128);
    put_sprite(&mut v, 1, 128, 0, 0, 3 | 0x6000, 128 + 8);
    let l = line(&mut v, 0);
    // highlight over a shadowed pixel gives a normal pixel
    assert_eq!(l[0], v.rgb(3, Brightness::Normal));
    // shadow over a pixel with priority: darker
    assert_eq!(l[8], v.rgb(3, Brightness::Shadow));
}
