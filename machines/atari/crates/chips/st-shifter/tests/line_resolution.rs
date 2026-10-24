//! A line is drawn whole in the resolution the register has `RES_SAMPLE_CLOCK` clocks into it ([O] P35 and P45: Hatari 2.6.1 does
//! not draw a part of a line in another resolution).

use st_shifter::{Monitor, RES_SAMPLE_CLOCK, Resolution, Shifter, WIDTH};

const FIRST_LINE: u16 = 63;
const PIXEL_START: i32 = 79;

/// A shifter at its first picture line over memory whose every 8 bytes are one low resolution group with plane 0 all ones and
/// the rest zero: in low resolution every pixel is colour 1 (white); in medium resolution the 4-byte groups alternate colour 1
/// (words `$FFFF 0000`) and colour 0 (black), 16 pixels each.
fn screen() -> (Shifter, Vec<u8>) {
    let mut s = Shifter::new(true);
    s.write(0xFF_8242, true, 0x777);
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

fn picture(s: &Shifter, ram: &[u8]) -> Vec<u8> {
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

/// Whether picture line `y` shows medium resolution: pixel 16 is black there and white in low resolution.
fn medium(out: &[u8], y: u32) -> bool {
    let i = ((2 * y * WIDTH + 16) * 3) as usize;
    out[i..i + 3] == [0, 0, 0]
}

/// Writes the resolution at `clock` of the current line, the way the machine does.
fn write_res(s: &mut Shifter, line: u16, clock: u64, value: u16) {
    s.set_line(line);
    s.set_line_start(0);
    s.write_at(0xFF_8260, false, value, clock);
}

#[test]
fn a_write_at_the_sample_clock_decides_the_line_and_one_after_it_does_not() {
    let (mut s, ram) = screen();
    // line 63 (picture line 0): medium written at the sample clock; line 64: written two clocks after it
    write_res(&mut s, 63, RES_SAMPLE_CLOCK, 1);
    s.line_boundary(64);
    assert_eq!(s.line_resolution(63), Resolution::Medium);
    write_res(&mut s, 64, RES_SAMPLE_CLOCK, 0);
    s.line_boundary(65);
    assert_eq!(s.line_resolution(64), Resolution::Low);
    write_res(&mut s, 65, RES_SAMPLE_CLOCK + 2, 1);
    s.line_boundary(66);
    assert_eq!(
        s.line_resolution(65),
        Resolution::Low,
        "written after the sample clock: the next line"
    );
    s.line_boundary(67);
    assert_eq!(
        s.line_resolution(66),
        Resolution::Medium,
        "the register stays medium"
    );
    let out = picture(&s, &ram);
    assert!(medium(&out, 0));
    assert!(!medium(&out, 1));
    assert!(!medium(&out, 2));
    assert!(medium(&out, 3));
}

#[test]
fn a_switch_back_after_the_sample_clock_does_not_undo_the_line() {
    let (mut s, ram) = screen();
    write_res(&mut s, 63, 100, 1);
    write_res(&mut s, 63, RES_SAMPLE_CLOCK + 6, 0);
    s.line_boundary(64);
    assert_eq!(s.line_resolution(63), Resolution::Medium);
    assert!(medium(&picture(&s, &ram), 0));
    // and the next line starts low again
    s.line_boundary(65);
    assert_eq!(s.line_resolution(64), Resolution::Low);
}

#[test]
fn a_switch_and_back_before_the_sample_clock_leaves_the_line_low() {
    let (mut s, ram) = screen();
    write_res(&mut s, 63, 100, 1);
    write_res(&mut s, 63, 300, 0);
    s.line_boundary(64);
    assert_eq!(s.line_resolution(63), Resolution::Low);
    assert!(!medium(&picture(&s, &ram), 0));
}
