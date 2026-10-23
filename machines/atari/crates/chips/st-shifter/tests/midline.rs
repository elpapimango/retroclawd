//! A palette change part way along a line reaches the pixels from the clock it was made at (`Shifter::render_exact`).

use st_shifter::{Monitor, Shifter, WIDTH};

const FIRST_LINE: u16 = 63;
const PIXEL_START: i32 = 79;

/// A shifter whose video memory is colour 1 everywhere, low resolution; colour 1 starts white.
fn screen() -> (Shifter, Vec<u8>) {
    let mut s = Shifter::new(true);
    s.write(0xFF_8242, true, 0x777);
    s.begin_frame();
    let mut ram = vec![0u8; 0x10000];
    for chunk in ram[..32000].as_chunks_mut::<8>().0 {
        chunk[0] = 0xFF;
        chunk[1] = 0xFF;
    }
    (s, ram)
}

fn render(s: &Shifter, ram: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    s.render_exact(
        Monitor::Colour,
        FIRST_LINE as u32,
        PIXEL_START,
        &|a| ram[a as usize],
        &mut out,
    );
    out
}

fn pixel(out: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * WIDTH + x) * 3) as usize;
    [out[i], out[i + 1], out[i + 2]]
}

/// Writes colour 1 at `clock` of line `line` the way the machine does.
fn write_at(s: &mut Shifter, line: u16, clock: u64, value: u16) {
    s.set_line(line);
    s.set_line_start(0);
    s.write_at(0xFF_8242, true, value, clock);
}

const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [255, 0, 0];

#[test]
fn a_write_reaches_the_pixel_of_its_clock() {
    let (mut s, ram) = screen();
    write_at(&mut s, FIRST_LINE + 10, (PIXEL_START + 100) as u64, 0x700);
    let out = render(&s, &ram);
    let y = 10 * 2;
    assert_eq!(pixel(&out, 2 * 100 - 1, y), WHITE, "just before");
    assert_eq!(pixel(&out, 2 * 100, y), RED, "from the write on");
    assert_eq!(pixel(&out, 639, y), RED);
}

#[test]
fn a_write_on_an_earlier_line_colours_the_whole_line() {
    let (mut s, ram) = screen();
    write_at(&mut s, FIRST_LINE + 3, 400, 0x700);
    let out = render(&s, &ram);
    assert_eq!(pixel(&out, 0, 4 * 2), RED);
    assert_eq!(
        pixel(&out, 0, 3 * 2),
        WHITE,
        "the line it was made on is white up to the write"
    );
}

#[test]
fn sixteen_changes_in_a_line_all_apply_in_order() {
    let (mut s, ram) = screen();
    // Red levels 0 to 7, twice: a change every 8 clocks, 16 pixels wide each.
    for i in 0..16u64 {
        write_at(
            &mut s,
            FIRST_LINE + 5,
            PIXEL_START as u64 + 8 * i,
            ((i & 7) as u16) << 8,
        );
    }
    let out = render(&s, &ram);
    let y = 5 * 2;
    for i in 0..16u32 {
        let want = ((i & 7) * 255 / 7) as u8;
        assert_eq!(pixel(&out, 16 * i + 3, y), [want, 0, 0], "segment {i}");
    }
}

#[test]
fn a_frame_without_changes_renders_as_the_line_renderer_does() {
    let (s, ram) = screen();
    let exact = render(&s, &ram);
    let mut plain = Vec::new();
    s.render_raster(
        Monitor::Colour,
        FIRST_LINE as u32,
        &|a| ram[a as usize],
        &mut plain,
    );
    assert_eq!(exact, plain);
}

#[test]
fn a_hostile_number_of_changes_is_bounded() {
    let (mut s, ram) = screen();
    for i in 0..100_000u64 {
        write_at(
            &mut s,
            FIRST_LINE + (i % 200) as u16,
            i % 512,
            (i % 8) as u16,
        );
    }
    let out = render(&s, &ram);
    assert_eq!(out.len(), (WIDTH * 400 * 3) as usize);
    assert!(s.palette_changes().len() <= st_shifter::MAX_PALETTE_CHANGES);
}
