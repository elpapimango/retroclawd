//! The raster palette through the whole machine: a program that changes palette entry 0 in an HBL handler draws a
//! different colour on every picture line.

mod common;
use common::{BRA_SELF, config, move_b, rom};

use atari_st::{AtariSt, RamSize};
use machine::Machine;

const HANDLER: u32 = 0xFC_0100;
const RTE_AT: u32 = 0xFC_0200;

/// Colour 0 steps by `$0111` on every HBL (a level 2 interrupt, vector `$68`), the picture is video RAM at `$10000`
/// (zeros: every pixel is colour 0) and the VBL (vector `$70`) only returns.
fn program() -> (Vec<u16>, Vec<u16>) {
    let long = |v: u32| [(v >> 16) as u16, v as u16];
    let mut p = Vec::new();
    p.extend([0x21FC]);
    p.extend(long(HANDLER));
    p.push(0x0068); // MOVE.L #HANDLER,$68.W
    p.extend([0x21FC]);
    p.extend(long(RTE_AT));
    p.push(0x0070); // MOVE.L #RTE,$70.W
    p.extend(move_b(0x01, 0x8201)); // video base $010000
    p.extend(move_b(0x00, 0x8203));
    p.extend([0x46FC, 0x2100]); // MOVE.W #$2100,SR: supervisor, interrupt mask 1
    p.push(BRA_SELF);
    let handler = vec![
        0x0640, 0x0111, // ADDI.W #$0111,D0
        0x0240, 0x0777, // ANDI.W #$0777,D0
        0x31C0, 0x8240, // MOVE.W D0,$FF8240.W
        0x4E73, // RTE
    ];
    (p, handler)
}

fn colour(frame: &[u8], picture_line: u32) -> [u8; 3] {
    let i = ((2 * picture_line * 640 + 5) * 3) as usize;
    [frame[i], frame[i + 1], frame[i + 2]]
}

#[test]
fn an_hbl_handler_draws_a_different_colour_on_every_line() {
    let (p, handler) = program();
    let mut m = AtariSt::new(config(
        rom(&p, &[(0x100, &handler), (0x200, &[0x4E73])]),
        RamSize::M1,
    ))
    .unwrap();
    while m.io_stop().is_none() && m.vbl_count() < 3 {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
    let (_, _, frame) = m.render_frame().unwrap();
    // Each line's colour is one step on from the line above; the steps wrap after eight (the colour mask drops
    // `$0888` to `$0000`), so a line equals the one eight lines on and differs from its neighbours.
    for y in 10..150 {
        assert_ne!(
            colour(&frame, y),
            colour(&frame, y + 1),
            "line {y} and the next"
        );
        assert_eq!(
            colour(&frame, y),
            colour(&frame, y + 8),
            "line {y} and eight on"
        );
    }
    let distinct: std::collections::BTreeSet<_> = (10..18).map(|y| colour(&frame, y)).collect();
    assert_eq!(distinct.len(), 8, "eight colours in a row of eight lines");
}

/// With nothing changing the palette the picture is one colour, as in Phase 1.
#[test]
fn a_still_palette_draws_one_colour() {
    let mut m = AtariSt::new(config(rom(&[BRA_SELF], &[]), RamSize::M1)).unwrap();
    while m.vbl_count() < 3 {
        m.step();
    }
    let (_, _, frame) = m.render_frame().unwrap();
    assert_eq!(colour(&frame, 0), colour(&frame, 199));
}

/// The sync-mode and resolution writes of the frame are in the stop report, with their line and cycle: a program that
/// plays the overscan trick shows there (and only there: no border is opened).
#[test]
fn the_stop_report_lists_the_sync_and_resolution_writes_of_the_frame() {
    let mut p = Vec::new();
    p.extend(move_b(0x00, 0x820A)); // sync mode 60 Hz
    p.extend(move_b(0x02, 0x820A)); // and back to 50 Hz
    p.extend(move_b(0x01, 0x8260)); // medium resolution
    p.extend(move_b(0x00, 0x8260));
    // MOVE.B $FF9000.L,$0600.L: no source covers $FF9000, so the machine stops there.
    p.extend([0x13F9, 0x00FF, 0x9000, 0x0000, 0x0600]);
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    while m.io_stop().is_none() && m.vbl_count() < 3 {
        m.step();
    }
    assert!(
        m.io_stop().is_some(),
        "the unsourced read stopped the machine"
    );
    let report = m.take_stop_report().expect("a report");
    assert!(
        report.contains("sync and resolution writes this frame, last 4"),
        "{report}"
    );
    assert!(
        report.contains("sync=0") && report.contains("resolution=1"),
        "{report}"
    );
}
