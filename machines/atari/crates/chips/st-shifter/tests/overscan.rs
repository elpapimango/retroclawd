//! The canvas drawn with the borders (`Shifter::render_overscan`): 832 x 552, the picture at (96, 58) as measured on
//! Hatari's `--borders on` screenshot, the border colour palette entry 0 as it is at each pixel.

use st_shifter::{Monitor, OVERSCAN_HEIGHT, OVERSCAN_WIDTH, Shifter};

const FIRST_LINE: u16 = 63;
const PIXEL_START: i32 = 79;

fn screen() -> (Shifter, Vec<u8>) {
    let mut s = Shifter::new(true);
    s.write(0xFF_8240, true, 0x007); // border: blue
    s.write(0xFF_8242, true, 0x777); // colour 1: white
    s.begin_frame();
    for line in 1..=FIRST_LINE {
        s.line_boundary(line);
    }
    let mut ram = vec![0u8; 0x10000];
    for chunk in ram[..32000].as_chunks_mut::<8>().0 {
        chunk[0] = 0xFF;
        chunk[1] = 0xFF;
    }
    (s, ram)
}

fn render(s: &Shifter, ram: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    s.render_overscan(Monitor::Colour, PIXEL_START, &|a| ram[a as usize], &mut out);
    out
}

fn pixel(out: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * OVERSCAN_WIDTH + x) * 3) as usize;
    [out[i], out[i + 1], out[i + 2]]
}

const WHITE: [u8; 3] = [255, 255, 255];
const BLUE: [u8; 3] = [0, 0, 255];
const RED: [u8; 3] = [255, 0, 0];

#[test]
fn the_canvas_has_the_measured_size() {
    let (s, ram) = screen();
    assert_eq!(render(&s, &ram).len(), (832 * 552 * 3) as usize);
    assert_eq!((OVERSCAN_WIDTH, OVERSCAN_HEIGHT), (832, 552));
}

#[test]
fn the_picture_sits_at_96_58_and_is_surrounded_by_the_border_colour() {
    let (s, ram) = screen();
    let out = render(&s, &ram);
    assert_eq!(pixel(&out, 95, 100), BLUE, "left of the picture");
    assert_eq!(pixel(&out, 96, 100), WHITE, "first picture column");
    assert_eq!(pixel(&out, 735, 100), WHITE, "last picture column");
    assert_eq!(pixel(&out, 736, 100), BLUE, "right of the picture");
    assert_eq!(pixel(&out, 300, 57), BLUE, "above the picture");
    assert_eq!(pixel(&out, 300, 58), WHITE, "first picture row");
    assert_eq!(pixel(&out, 300, 457), WHITE, "last picture row");
    assert_eq!(pixel(&out, 300, 458), BLUE, "below the picture");
    assert_eq!(pixel(&out, 0, 0), BLUE);
    assert_eq!(pixel(&out, 831, 551), BLUE);
}

#[test]
fn the_border_colour_follows_palette_entry_zero_along_a_line() {
    let (mut s, ram) = screen();
    s.set_line(10);
    s.set_line_start(0);
    // Written at the clock of canvas pixel 200 (clock 31 + 100) of raster line 44.
    s.set_line(44);
    s.write_at(0xFF_8240, true, 0x700, 131);
    let out = render(&s, &ram);
    let y = (44 - 34) * 2;
    assert_eq!(pixel(&out, 199, y), BLUE);
    assert_eq!(pixel(&out, 200, y), RED);
    assert_eq!(pixel(&out, 831, y), RED, "to the end of the line");
    assert_eq!(pixel(&out, 0, y + 2), RED, "and on the lines after it");
    assert_eq!(pixel(&out, 0, y - 1), BLUE, "not before");
}

#[test]
fn the_mono_monitor_has_no_colour_picture() {
    let (s, ram) = screen();
    let mut out = Vec::new();
    s.render_overscan(Monitor::Mono, PIXEL_START, &|a| ram[a as usize], &mut out);
    assert_eq!(pixel(&out, 300, 100), BLUE);
}
