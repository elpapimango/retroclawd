//! Writing to a floppy through the DMA chip and the WD1772: WRITE SECTOR, WRITE TRACK (format), write protect.
//! The data sheet's commands and formats are in `docs/hardware-facts.md` ("DMA and floppy controller").

mod common;
use common::{BRA_SELF, config, move_b, rom};

use atari_st::{AtariSt, RamSize};
use machine::Machine;
use st_floppy::Image;

fn move_w(value: u16, addr: u16) -> [u16; 3] {
    [0x31FC, value, addr]
}

/// BTST #5,$FFFA01.W; BNE.S back: wait for the FDC's interrupt (GPIP I5 low).
const WAIT_FDC: [u16; 4] = [0x0838, 0x0005, 0xFA01, 0x66F8];

fn select_a() -> Vec<u16> {
    [
        move_b(7, 0x8800),
        move_b(0xC0, 0x8802),
        move_b(14, 0x8800),
        move_b(0x05, 0x8802),
    ]
    .concat()
}

fn image() -> Image {
    let bytes: Vec<u8> = (0..737_280usize).map(|i| (i * 7 + i / 512) as u8).collect();
    Image::parse(&bytes).unwrap()
}

fn run(m: &mut AtariSt, cycles: u64) {
    while m.now() < cycles && m.io_stop().is_none() {
        m.step();
    }
}

/// Fills 512 bytes at `$10000` with `$DEADBEEF`, then writes them to sector `record` of cylinder 0, side 0 by DMA,
/// waits for the interrupt and stores the FDC status at `$1000`.
fn write_sector_program(record: u8) -> Vec<u16> {
    let mut p = vec![
        0x41F9, 0x0001, 0x0000, // LEA $10000.L,A0
        0x303C, 0x007F, // MOVE.W #127,D0
        0x20FC, 0xDEAD, 0xBEEF, // MOVE.L #$DEADBEEF,(A0)+
        0x51C8, 0xFFF8, // DBRA D0,-8
    ];
    p.extend(select_a());
    p.extend(move_b(0x00, 0x860D));
    p.extend(move_b(0x00, 0x860B));
    p.extend(move_b(0x01, 0x8609)); // DMA address $010000
    p.extend(move_w(0x090, 0x8606));
    p.extend(move_w(0x190, 0x8606)); // write direction, sector count
    p.extend(move_w(1, 0x8604)); // sector count 1
    p.extend(move_w(0x184, 0x8606));
    p.extend(move_w(record as u16, 0x8604)); // sector register
    p.extend(move_w(0x180, 0x8606));
    p.extend(move_w(0xA8, 0x8604)); // WRITE SECTOR, motor on
    p.extend(WAIT_FDC);
    p.extend(move_w(0x180, 0x8606));
    p.extend([0x31F8, 0x8604, 0x1000]); // MOVE.W $FF8604.W,$1000.W
    p.push(BRA_SELF);
    p
}

fn machine(program: &[u16]) -> AtariSt {
    let mut m = AtariSt::new(config(rom(program, &[]), RamSize::M1)).unwrap();
    m.insert_disk(0, image()).unwrap();
    m
}

#[test]
fn write_sector_changes_the_image() {
    let mut m = machine(&write_sector_program(3));
    run(&mut m, 12_000_000);
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
    assert_eq!(m.read_memory(0x1001) & 0x5C, 0, "status: no error bits");
    let sector = m.disk(0).unwrap().sector(0, 0, 3).unwrap();
    assert!(
        sector.chunks(4).all(|c| c == [0xDE, 0xAD, 0xBE, 0xEF]),
        "sector 3 holds the buffer"
    );
    let untouched = image();
    assert_eq!(
        m.disk(0).unwrap().sector(0, 0, 2),
        untouched.sector(0, 0, 2),
        "the neighbours are unchanged"
    );
    assert_eq!(
        m.disk(0).unwrap().sector(0, 0, 4),
        untouched.sector(0, 0, 4)
    );
    assert!(m.disk_modified(0));
}

#[test]
fn reading_a_disk_does_not_make_it_modified() {
    let mut m = machine(&[BRA_SELF]);
    run(&mut m, 3_000_000);
    assert!(!m.disk_modified(0));
}

#[test]
fn a_write_protected_disk_refuses_and_sets_the_status_bit() {
    let mut m = machine(&write_sector_program(3));
    m.set_write_protect(0, true);
    run(&mut m, 12_000_000);
    assert!(m.io_stop().is_none());
    assert_eq!(m.read_memory(0x1001) & 0x40, 0x40, "status: write protect");
    assert_eq!(
        m.disk(0).unwrap().sector(0, 0, 3),
        image().sector(0, 0, 3),
        "nothing was written"
    );
    assert!(!m.disk_modified(0));
}

/// A WRITE TRACK buffer in the data sheet's format (p.1-18): `F5` writes `A1` with a missing clock, `F7` the CRC.
fn format_buffer(cyl: u8, side: u8, sectors: u8, fill: u8) -> Vec<u8> {
    let mut b = vec![0x4E; 60];
    for r in 1..=sectors {
        b.extend([0x00; 12]);
        b.extend([0xF5, 0xF5, 0xF5, 0xFE, cyl, side, r, 2, 0xF7]);
        b.extend([0x4E; 22]);
        b.extend([0x00; 12]);
        b.extend([0xF5, 0xF5, 0xF5, 0xFB]);
        b.extend(std::iter::repeat_n(fill, 512));
        b.push(0xF7);
        b.extend([0x4E; 24]);
    }
    b.resize(13 * 512, 0x4E); // the DMA counts whole sectors
    b
}

/// Formats cylinder 0, side 0 from the buffer at `$10000` (WRITE TRACK, 13 DMA sectors) and stores the status.
fn write_track_program() -> Vec<u16> {
    let mut p = select_a();
    p.extend(move_b(0x00, 0x860D));
    p.extend(move_b(0x00, 0x860B));
    p.extend(move_b(0x01, 0x8609));
    p.extend(move_w(0x090, 0x8606));
    p.extend(move_w(0x190, 0x8606));
    p.extend(move_w(13, 0x8604));
    p.extend(move_w(0x180, 0x8606));
    p.extend(move_w(0xF8, 0x8604)); // WRITE TRACK, motor on
    p.extend(WAIT_FDC);
    p.extend(move_w(0x180, 0x8606));
    p.extend([0x31F8, 0x8604, 0x1000]);
    p.push(BRA_SELF);
    p
}

#[test]
fn write_track_formats_a_track() {
    let mut m = machine(&write_track_program());
    m.load(0x1_0000, &format_buffer(0, 0, 9, 0xE5));
    run(&mut m, 12_000_000);
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
    assert_eq!(m.read_memory(0x1001) & 0x5C, 0, "status: no error bits");
    let img = m.disk(0).unwrap();
    for r in 1..=9 {
        assert!(
            img.sector(0, 0, r).unwrap().iter().all(|&b| b == 0xE5),
            "sector {r}"
        );
    }
    assert_eq!(
        img.sector(0, 1, 1),
        image().sector(0, 1, 1),
        "the other side is untouched"
    );
    assert_eq!(img.sector(1, 0, 1), image().sector(1, 0, 1));
}
