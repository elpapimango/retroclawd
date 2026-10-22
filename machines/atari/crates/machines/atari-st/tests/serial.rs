//! The MFP's RS-232 line through the USART: a program writes characters at timer D's rate and a file peer receives
//! them, a loopback peer sends them back, and the output file is only ever the one the user named.

mod common;
use common::{BRA_SELF, config, move_b, move_b_abs, rom, wait_bit7};

use std::path::PathBuf;

use atari_st::serial::{FileSink, FileSource, LinePeer};
use atari_st::{AtariSt, RamSize};
use machine::Machine;

// MFP registers at $FFFA01 + 2n (*Internals* pp.60-61).
const TCDCR: u16 = 0xFA1D;
const TDDR: u16 = 0xFA25;
const UCR: u16 = 0xFA29;
const RSR: u16 = 0xFA2B;
const TSR: u16 = 0xFA2D;
const UDR: u16 = 0xFA2F;

/// A fresh folder for one test, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("atari-serial-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Timer D at a time-out every 4 timer clocks, the USART at 8N1 divide by 16, transmitter on.
fn setup() -> Vec<u16> {
    let mut p = Vec::new();
    p.extend(move_b(1, TDDR));
    p.extend(move_b(0x01, TCDCR));
    p.extend(move_b(0x88, UCR));
    p.extend(move_b(0x01, TSR));
    p
}

/// Writes `text` to the USART, waiting for the buffer to empty between characters.
fn send(p: &mut Vec<u16>, text: &[u8]) {
    for &c in text {
        p.extend(wait_bit7(TSR));
        p.extend(move_b(c, UDR));
    }
}

fn run(m: &mut AtariSt, frames: u64) {
    while m.io_stop().is_none() && m.vbl_count() < frames {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
}

#[test]
fn file_sink_writes_only_to_the_given_path() {
    let dir = Dir::new("sink");
    let out = dir.0.join("out.bin");
    let mut p = setup();
    send(&mut p, b"SERIAL");
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    m.attach_rs232_peer(Box::new(LinePeer::new(
        Some(FileSink::create(&out, false).unwrap()),
        None,
        false,
        0,
    )));
    run(&mut m, 4);
    assert_eq!(std::fs::read(&out).unwrap(), b"SERIAL");
    let names: Vec<_> = std::fs::read_dir(&dir.0)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("out.bin")]);
}

#[test]
fn an_existing_output_file_is_refused_without_overwrite() {
    let dir = Dir::new("refuse");
    let out = dir.0.join("keep.bin");
    std::fs::write(&out, b"precious").unwrap();
    let e = FileSink::create(&out, false).expect_err("refused");
    assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&out).unwrap(), b"precious", "left alone");
    drop(FileSink::create(&out, true).unwrap());
    assert_eq!(std::fs::read(&out).unwrap(), b"", "replaced when asked");
}

#[test]
fn a_program_reads_what_a_loopback_peer_sends_back() {
    let mut p = setup();
    p.extend(move_b(0x01, RSR));
    send(&mut p, b"A");
    p.extend(wait_bit7(RSR));
    p.extend(move_b_abs(UDR, 0x0600));
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    m.attach_rs232_peer(Box::new(LinePeer::new(None, None, true, 0)));
    run(&mut m, 6);
    assert_eq!(m.bus().peek(0x600), b'A');
}

#[test]
fn a_program_reads_the_input_file() {
    let dir = Dir::new("source");
    let input = dir.0.join("in.bin");
    std::fs::write(&input, b"xyz").unwrap();
    let mut p = setup();
    p.extend(move_b(0x01, RSR));
    for slot in 0..3u16 {
        p.extend(wait_bit7(RSR));
        p.extend(move_b_abs(UDR, 0x0600 + slot));
    }
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    m.attach_rs232_peer(Box::new(LinePeer::new(
        None,
        Some(FileSource::open(&input).unwrap()),
        false,
        0,
    )));
    run(&mut m, 8);
    let got: Vec<u8> = (0..3).map(|i| m.bus().peek(0x600 + i)).collect();
    assert_eq!(got, b"xyz");
}

#[test]
fn an_input_that_is_not_a_regular_file_or_too_big_is_refused() {
    let dir = Dir::new("big");
    assert!(FileSource::open(&dir.0).is_err(), "a folder");
    assert!(FileSource::open(&dir.0.join("missing")).is_err());
    let big = dir.0.join("big.bin");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(atari_st::serial::MAX_INPUT + 1).unwrap();
    assert!(FileSource::open(&big).is_err());
}
