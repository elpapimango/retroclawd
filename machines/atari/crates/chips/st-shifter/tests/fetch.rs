//! The picture window and the borders of a line (src/fetch.rs): rules fitted to Hatari's border sweeps.

use st_shifter::{LEFT_BYTES, LINE_BYTES, RIGHT_BYTES, Shifter};

/// A shifter at the start of a 50 Hz frame, with the sync mode at 50 Hz.
fn frame() -> Shifter {
    let mut s = Shifter::new(true);
    s.begin_frame();
    s
}

fn lines_to(s: &mut Shifter, last: u16) {
    for l in 1..=last {
        s.line_boundary(l);
    }
}

fn write(s: &mut Shifter, line: u16, clock: u64, reg: u32, value: u8) {
    s.set_line_start(0);
    s.set_line(line);
    s.write_at(reg, false, value as u16, clock);
}

const SYNC: u32 = 0xFF_820A;
const RES: u32 = 0xFF_8260;

#[test]
fn the_picture_of_a_50_hz_frame_starts_at_line_63_and_ends_at_263() {
    let mut s = frame();
    lines_to(&mut s, 62);
    assert!(!s.fetch().fetching(62));
    s.line_boundary(63);
    assert!(s.fetch().fetching(63));
    lines_to_from(&mut s, 64, 262);
    assert!(s.fetch().fetching(262));
    s.line_boundary(263);
    assert!(!s.fetch().fetching(263));
    assert_eq!(s.fetch().bytes_before(263), 200 * LINE_BYTES);
}

fn lines_to_from(s: &mut Shifter, from: u16, last: u16) {
    for l in from..=last {
        s.line_boundary(l);
    }
}

#[test]
fn a_60_hz_line_before_the_picture_opens_the_top_border() {
    let mut s = frame();
    lines_to(&mut s, 32);
    // 60 Hz is written during line 33 and 50 Hz is back during line 34.
    s.line_boundary(33);
    write(&mut s, 33, 100, SYNC, 0);
    s.line_boundary(34);
    write(&mut s, 34, 100, SYNC, 2);
    assert!(
        s.fetch().fetching(34),
        "the picture starts at 34 when line 34 begins in 60 Hz"
    );
    lines_to_from(&mut s, 35, 262);
    assert!(s.fetch().fetching(262));
    // 229 lines in all: 34 to 262.
    s.line_boundary(263);
    assert_eq!(s.fetch().bytes_before(263), 229 * LINE_BYTES);
}

#[test]
fn a_60_hz_line_just_before_line_63_misses_the_picture() {
    let mut s = frame();
    lines_to(&mut s, 61);
    s.line_boundary(62);
    write(&mut s, 62, 100, SYNC, 0);
    s.line_boundary(63);
    write(&mut s, 63, 100, SYNC, 2);
    lines_to_from(&mut s, 64, 300);
    assert!(!s.fetch().fetching(100));
    assert_eq!(s.fetch().bytes_before(300), 0);
}

#[test]
fn a_60_hz_line_at_the_end_of_the_picture_opens_the_bottom_border() {
    let mut s = frame();
    lines_to(&mut s, 262);
    // Line 262 begins and writes 60 Hz; line 263 begins in 60 Hz: not the end of a 50 Hz picture.
    write(&mut s, 262, 100, SYNC, 0);
    s.line_boundary(263);
    write(&mut s, 263, 100, SYNC, 2);
    assert!(s.fetch().fetching(263), "the 50 Hz end check saw 60 Hz");
    lines_to_from(&mut s, 264, 300);
    assert!(s.fetch().fetching(300));
}

#[test]
fn high_resolution_at_clock_370_and_low_again_opens_the_right_border() {
    let mut s = frame();
    lines_to(&mut s, 100);
    write(&mut s, 100, 364, RES, 2);
    write(&mut s, 100, 372, RES, 0);
    s.line_boundary(101);
    assert_eq!(s.fetch().borders(100), (false, true));
    assert_eq!(
        s.fetch().bytes_before(101),
        (100 - 63) * LINE_BYTES + LINE_BYTES + RIGHT_BYTES
    );
}

#[test]
fn the_sync_sequence_50_60_50_around_clock_364_opens_the_right_border() {
    let mut s = frame();
    lines_to(&mut s, 100);
    write(&mut s, 100, 364, SYNC, 0);
    write(&mut s, 100, 372, SYNC, 2);
    s.line_boundary(101);
    assert_eq!(s.fetch().borders(100), (false, true));
    // Four clocks earlier or later it does not.
    for first in [356u64, 360, 368] {
        let mut s = frame();
        lines_to(&mut s, 100);
        write(&mut s, 100, first, SYNC, 0);
        write(&mut s, 100, first + 8, SYNC, 2);
        s.line_boundary(101);
        assert_eq!(
            s.fetch().borders(100),
            (false, false),
            "60 Hz from clock {first}"
        );
    }
}

#[test]
fn high_resolution_over_the_line_boundary_opens_the_left_border() {
    let mut s = frame();
    lines_to(&mut s, 100);
    write(&mut s, 100, 508, RES, 2);
    s.line_boundary(101);
    write(&mut s, 101, 4, RES, 0);
    s.line_boundary(102);
    assert_eq!(s.fetch().borders(101), (true, false));
    assert_eq!(
        s.picture_line(101),
        Some(((101 - 63) * LINE_BYTES, true)),
        "the visible picture starts {LEFT_BYTES} bytes into the line's data"
    );
    assert_eq!(
        s.fetch().bytes_before(102),
        (101 - 63) * LINE_BYTES + LINE_BYTES + LEFT_BYTES
    );
}

#[test]
fn a_hostile_register_storm_stays_bounded() {
    let mut s = frame();
    for l in 1..=312u16 {
        s.line_boundary(l);
        for c in 0..600u64 {
            write(
                &mut s,
                l,
                c,
                if c % 2 == 0 { SYNC } else { RES },
                (c % 4) as u8,
            );
        }
    }
    assert!(s.fetch().bytes_before(312) < 313 * 400);
}
