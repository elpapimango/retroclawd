//! Memory watchpoints: a range stops the machine at the first data access to it, with the access in the report.

mod common;
use common::config;

use atari_st::watch::WatchList;
use atari_st::{Accuracy, AtariSt, Chipset, RamSize, StopReason};
use machine::Machine;
use mc68000::Size;

fn ste_with(program: &[u16]) -> Vec<u8> {
    let mut rom = vec![0u8; 262_144];
    rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00E0_0010u32.to_be_bytes());
    for (i, w) in program.iter().enumerate() {
        rom[0x10 + 2 * i..0x12 + 2 * i].copy_from_slice(&w.to_be_bytes());
    }
    rom
}

/// `MOVE.W #imm,addr.L`, `MOVE.W addr.L,D0`, `MOVE.B #imm,addr.L`
fn write_word(imm: u16, addr: u32) -> [u16; 4] {
    [0x33FC, imm, (addr >> 16) as u16, addr as u16]
}
fn read_word(addr: u32) -> [u16; 3] {
    [0x3039, (addr >> 16) as u16, addr as u16]
}
fn write_byte(imm: u8, addr: u32) -> [u16; 4] {
    [0x13FC, u16::from(imm), (addr >> 16) as u16, addr as u16]
}

fn machine(program: &[u16], watch: &str, accuracy: Accuracy) -> AtariSt {
    let mut cfg = config(ste_with(program), RamSize::M1);
    cfg.accuracy = accuracy;
    let mut m = AtariSt::with_chipset(cfg, Chipset::Ste).unwrap();
    m.set_watch(watch).unwrap();
    m
}

fn run(m: &mut AtariSt) {
    for _ in 0..200 {
        m.step();
    }
}

#[test]
fn a_write_into_a_watched_range_stops_with_the_access_in_the_report() {
    for accuracy in [Accuracy::Fast, Accuracy::Accurate] {
        let mut p = write_word(0x1111, 0x500).to_vec();
        p.extend(write_word(0x1234, 0x1040));
        p.extend(write_word(0x9999, 0x600));
        p.push(0x60FE);
        let mut m = machine(&p, "$1000-$10FF:w", accuracy);
        run(&mut m);
        let stop = m.io_stop().expect("the watch tripped").clone();
        assert_eq!(stop.reason, StopReason::Watch);
        assert_eq!(
            (stop.addr, stop.size, stop.write, stop.value),
            (0x1040, Size::Word, true, 0x1234)
        );
        assert!(stop.at_cycle > 0);
        // the access was made, and nothing after it
        assert_eq!(m.bus().peek(0x1040), 0x12);
        assert_eq!(m.bus().peek(0x600), 0, "the next write did not happen");
        let report = m.take_stop_report().expect("a report");
        assert!(
            report.contains("watchpoint") && report.contains("$001040") && report.contains("$1234"),
            "{report}"
        );
        assert!(
            report.contains("PC") || report.contains("$E000"),
            "{report}"
        );
    }
}

#[test]
fn a_read_watch_trips_on_reads_only_and_a_write_watch_on_writes_only() {
    let mut p = write_word(0x4321, 0x2000).to_vec();
    p.extend(read_word(0x2000));
    p.push(0x60FE);
    let mut m = machine(&p, "$2000:r", Accuracy::Fast);
    run(&mut m);
    let stop = m.io_stop().expect("the read tripped");
    assert!(!stop.write && stop.value == 0x4321, "{stop:?}");
    let mut m = machine(&p, "$2000:w", Accuracy::Fast);
    run(&mut m);
    let stop = m.io_stop().expect("the write tripped");
    assert!(stop.write);
}

#[test]
fn any_byte_of_a_word_access_trips_a_watch_and_neighbours_do_not() {
    let mut p = write_word(0xAAAA, 0x3000).to_vec();
    p.extend(write_byte(1, 0x3003));
    p.push(0x60FE);
    let mut m = machine(&p, "$3001", Accuracy::Fast);
    run(&mut m);
    assert_eq!(
        m.io_stop().expect("byte 3001 is in the word at 3000").addr,
        0x3000
    );
    let mut m = machine(&p, "$3002", Accuracy::Fast);
    run(&mut m);
    assert!(m.io_stop().is_none(), "$3002 is touched by neither access");
    let mut m = machine(&p, "$3003", Accuracy::Fast);
    run(&mut m);
    let stop = m.io_stop().expect("the byte write at 3003");
    assert_eq!((stop.addr, stop.size), (0x3003, Size::Byte));
}

#[test]
fn running_code_is_not_watching_it() {
    let p = [0x60FE_u16];
    let mut m = machine(&p, "$E00000-$E3FFFF:rw", Accuracy::Fast);
    run(&mut m);
    assert!(m.io_stop().is_none());
}

#[test]
fn ranges_may_overlap_and_the_list_is_replaced_by_the_next_set() {
    let mut p = write_word(1, 0x1800).to_vec();
    p.push(0x60FE);
    let mut m = machine(&p, "$1000-$2000:w,$1800:rw,$1700-$1900", Accuracy::Fast);
    run(&mut m);
    assert!(m.io_stop().is_some());
    let mut m = machine(&p, "$1000-$2000:w", Accuracy::Fast);
    m.set_watch("").unwrap();
    run(&mut m);
    assert!(m.io_stop().is_none(), "an empty text clears the list");
}

#[test]
fn bad_lists_are_errors_with_the_reason() {
    let seventeen = (0..17)
        .map(|i| format!("${:X}", 0x1000 + i))
        .collect::<Vec<_>>()
        .join(",");
    let sixteen = (0..16)
        .map(|i| format!("${:X}", 0x1000 + i))
        .collect::<Vec<_>>()
        .join(",");
    assert!(WatchList::parse(&sixteen).is_ok());
    assert!(WatchList::parse(&seventeen).unwrap_err().contains("16"));
    for (bad, why) in [
        ("zzz", "not an address"),
        ("$1000-$0F00", "ends before"),
        ("$1000000", "24-bit"),
        ("$1000:x", "mode"),
        ("$1000-", "not an address"),
    ] {
        let e = WatchList::parse(bad).unwrap_err();
        assert!(e.contains(why), "{bad}: {e}");
    }
    let ok = WatchList::parse("0x400:rw, 4096-8191:w ,$2000-$20FF:R").unwrap();
    assert_eq!(ok.ranges().len(), 3);
}

#[test]
fn a_machine_with_no_watch_pays_nothing_visible() {
    // the same program with and without an empty list stops nowhere and ends in the same place
    let mut p = write_word(1, 0x1800).to_vec();
    p.push(0x60FE);
    let mut a = machine(&p, "", Accuracy::Fast);
    let mut b = AtariSt::with_chipset(config(ste_with(&p), RamSize::M1), Chipset::Ste).unwrap();
    run(&mut a);
    run(&mut b);
    assert!(a.io_stop().is_none() && b.io_stop().is_none());
    assert_eq!(a.bus().peek(0x1801), b.bus().peek(0x1801));
}
