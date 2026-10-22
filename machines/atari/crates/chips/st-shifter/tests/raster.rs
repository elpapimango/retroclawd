//! The palette followed line by line: every palette write carries the raster line it happened on, and the frame shows
//! each change from its line down (one line is the granularity).

use st_shifter::{MAX_PALETTE_CHANGES, Monitor, Shifter, WIDTH};

fn palette(s: &mut Shifter, index: u32, value: u16) {
    s.write(0xFF8240 + 2 * index, true, value);
}

fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * WIDTH + x) * 3) as usize;
    [frame[i], frame[i + 1], frame[i + 2]]
}

/// Video RAM of zeros: every pixel is colour 0 in low resolution, so each picture line shows palette 0.
fn zeros(_: u32) -> u8 {
    0
}

const FIRST: u32 = 63;
const BLACK: [u8; 3] = [0, 0, 0];
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];

fn render(s: &Shifter) -> Vec<u8> {
    let mut out = Vec::new();
    s.render_raster(Monitor::Colour, FIRST, &zeros, &mut out);
    out
}

/// Picture line `y` is rows `2y` and `2y + 1` of the 640 × 400 canvas in low resolution.
fn row(frame: &[u8], y: u32) -> [[u8; 3]; 2] {
    [pixel(frame, 5, 2 * y), pixel(frame, 5, 2 * y + 1)]
}

#[test]
fn a_palette_change_on_line_100_colours_only_lines_100_and_below() {
    let mut s = Shifter::new(true);
    s.begin_frame();
    s.set_line(100);
    palette(&mut s, 0, 0x0700);
    let frame = render(&s);
    // Raster line 100 is picture line 100 - 63 = 37.
    assert_eq!(row(&frame, 36), [BLACK; 2], "the line above is untouched");
    assert_eq!(
        row(&frame, 37),
        [RED; 2],
        "the line of the write has the new colour"
    );
    assert_eq!(row(&frame, 199), [RED; 2], "and so has every line below");
    assert_eq!(pixel(&frame, 639, 0), BLACK);
}

#[test]
fn two_changes_on_one_line_the_last_one_wins() {
    let mut s = Shifter::new(true);
    s.begin_frame();
    s.set_line(80);
    palette(&mut s, 0, 0x0700);
    palette(&mut s, 0, 0x0070);
    let frame = render(&s);
    assert_eq!(row(&frame, 16), [BLACK; 2]);
    assert_eq!(row(&frame, 17), [GREEN; 2]);
    assert_eq!(s.palette_changes().len(), 2);
}

#[test]
fn successive_lines_follow_their_own_changes() {
    let mut s = Shifter::new(true);
    s.begin_frame();
    for (line, value) in [(70, 0x0700u16), (90, 0x0070), (110, 0x0000)] {
        s.set_line(line);
        palette(&mut s, 0, value);
    }
    let frame = render(&s);
    assert_eq!(row(&frame, 6), [BLACK; 2]);
    assert_eq!(row(&frame, 7), [RED; 2]);
    assert_eq!(row(&frame, 26), [RED; 2]);
    assert_eq!(row(&frame, 27), [GREEN; 2]);
    assert_eq!(row(&frame, 46), [GREEN; 2]);
    assert_eq!(row(&frame, 47), [BLACK; 2]);
}

/// Only the colour that was written changes; the others keep theirs.
#[test]
fn a_change_touches_one_colour_only() {
    let mut s = Shifter::new(true);
    palette(&mut s, 1, 0x0007);
    s.begin_frame();
    s.set_line(100);
    palette(&mut s, 0, 0x0700);
    let mut out = Vec::new();
    // Video RAM that is plane 0 only: colour 1 everywhere.
    s.render_raster(
        Monitor::Colour,
        FIRST,
        &|a| if a % 8 < 2 { 0xFF } else { 0 },
        &mut out,
    );
    assert_eq!(pixel(&out, 0, 0), [0, 0, 255], "colour 1 on line 0");
    assert_eq!(pixel(&out, 0, 400 - 2), [0, 0, 255], "colour 1 on line 199");
}

/// The frame render of Phase 1 is unchanged when no palette write came in the frame.
#[test]
fn a_frame_with_no_changes_renders_exactly_as_before() {
    let mut s = Shifter::new(true);
    palette(&mut s, 0, 0x0123);
    palette(&mut s, 3, 0x0456);
    s.begin_frame();
    let ram = |a: u32| (a.wrapping_mul(37) >> 3) as u8;
    let (mut raster, mut plain) = (Vec::new(), Vec::new());
    s.render_raster(Monitor::Colour, FIRST, &ram, &mut raster);
    s.render(Monitor::Colour, &ram, &mut plain);
    assert_eq!(raster, plain);
    assert!(s.palette_changes().is_empty());
}

/// A write of the value a colour already has changes nothing and is not logged.
#[test]
fn a_write_that_changes_nothing_is_not_logged() {
    let mut s = Shifter::new(true);
    s.begin_frame();
    palette(&mut s, 0, 0);
    assert!(s.palette_changes().is_empty());
    palette(&mut s, 0, 0x0777);
    palette(&mut s, 0, 0x0777);
    assert_eq!(s.palette_changes().len(), 1);
}

/// Writes made before the first display line (a VBL handler) colour the picture from its first line.
#[test]
fn a_change_before_the_first_display_line_applies_from_the_top() {
    let mut s = Shifter::new(true);
    s.begin_frame();
    s.set_line(10);
    palette(&mut s, 0, 0x0700);
    let frame = render(&s);
    assert_eq!(row(&frame, 0), [RED; 2]);
}

/// A new frame starts with the palette the last one ended with, and an empty log.
#[test]
fn a_new_frame_starts_with_the_last_palette() {
    let mut s = Shifter::new(true);
    s.begin_frame();
    s.set_line(100);
    palette(&mut s, 0, 0x0700);
    s.begin_frame();
    assert!(s.palette_changes().is_empty());
    let frame = render(&s);
    assert_eq!(row(&frame, 0), [RED; 2], "red from the first line now");
}

/// The log is bounded: past the cap the palette still takes the writes, only the picture's later changes are lost.
#[test]
fn the_log_is_bounded_and_the_palette_is_not() {
    let mut s = Shifter::new(true);
    s.begin_frame();
    for i in 0..(MAX_PALETTE_CHANGES + 100) {
        palette(&mut s, 0, if i % 2 == 0 { 0x0001 } else { 0x0002 });
    }
    assert_eq!(s.palette_changes().len(), MAX_PALETTE_CHANGES);
    assert_eq!(
        s.read(0xFF8240, true),
        Some(0x0002),
        "the palette has the last write"
    );
}

/// In high resolution the one palette entry still decides which of the two colours is dark, line by line.
#[test]
fn high_resolution_follows_palette_zero_too() {
    let mut s = Shifter::new(true);
    s.write(0xFF8260, false, 2);
    s.begin_frame();
    s.set_line(FIRST as u16 + 100);
    palette(&mut s, 0, 1);
    let mut out = Vec::new();
    s.render_raster(Monitor::Mono, FIRST, &zeros, &mut out);
    // Bit clear is light while palette 0 bit 0 is clear and dark after it is set.
    assert_eq!(pixel(&out, 3, 99), [255, 255, 255]);
    assert_eq!(pixel(&out, 3, 100), BLACK);
}
