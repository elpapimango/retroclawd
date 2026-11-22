//! The Centronics port: PSG port B is the data lines, PSG port A bit 5 the strobe (a low pulse), GPIP I0 BUSY
//! (*Internals* p.59, pp.88-89 and p.61).

mod common;
use common::{BRA_SELF, config, move_b, rom};

use atari_st::serial::FileSink;
use atari_st::{AtariSt, RamSize};
use machine::Machine;

const SELECT: u16 = 0x8800;
const WRITE: u16 = 0x8802;

fn psg(p: &mut Vec<u16>, reg: u8, value: u8) {
    p.extend(move_b(reg, SELECT));
    p.extend(move_b(value, WRITE));
}

fn run(m: &mut AtariSt, frames: u64) {
    while m.io_stop().is_none() && m.vbl_count() < frames {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("atari-printer-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Both ports as outputs (R7 bits 7 and 6), the strobe high, the data on port B, then the strobe low and high.
fn print_program(byte: u8) -> Vec<u16> {
    let mut p = Vec::new();
    psg(&mut p, 14, 0xFF);
    psg(&mut p, 7, 0xFF);
    psg(&mut p, 15, byte);
    psg(&mut p, 14, 0xDF);
    psg(&mut p, 14, 0xFF);
    p.push(BRA_SELF);
    p
}

#[test]
fn a_strobe_writes_the_data_byte() {
    let dir = temp("strobe");
    let out = dir.join("print.txt");
    let mut m = AtariSt::new(config(rom(&print_program(b'P'), &[]), RamSize::M1)).unwrap();
    m.attach_printer(FileSink::create(&out, false).unwrap());
    run(&mut m, 2);
    assert_eq!(std::fs::read(&out).unwrap(), b"P");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_the_falling_edge_of_the_strobe_prints() {
    let dir = temp("edge");
    let out = dir.join("print.txt");
    let mut p = Vec::new();
    psg(&mut p, 14, 0xFF);
    psg(&mut p, 7, 0xFF);
    psg(&mut p, 15, b'A');
    psg(&mut p, 14, 0xDF);
    // The data change and the rest of port A while the strobe stays low print nothing.
    psg(&mut p, 15, b'B');
    psg(&mut p, 14, 0x9F);
    psg(&mut p, 14, 0xFF);
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    m.attach_printer(FileSink::create(&out, false).unwrap());
    run(&mut m, 2);
    assert_eq!(std::fs::read(&out).unwrap(), b"A");
    let _ = std::fs::remove_dir_all(&dir);
}

/// [H?] Port A becoming an output with the strobe line low (its latch resets to 0) is not a pulse the printer sees.
#[test]
fn a_port_a_that_only_now_becomes_an_output_does_not_strobe() {
    let dir = temp("floating");
    let out = dir.join("print.txt");
    let mut p = Vec::new();
    psg(&mut p, 15, b'X');
    psg(&mut p, 7, 0xFF);
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    m.attach_printer(FileSink::create(&out, false).unwrap());
    run(&mut m, 2);
    assert_eq!(std::fs::read(&out).unwrap(), b"");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Without an output file the printer is off line: BUSY (GPIP I0) stays high and nothing is written anywhere.
#[test]
fn no_printer_out_file_means_busy_stays_set() {
    let mut m = AtariSt::new(config(rom(&print_program(b'P'), &[]), RamSize::M1)).unwrap();
    assert_eq!(m.bus().mfp().peek(0) & 1, 1, "BUSY at power-up");
    run(&mut m, 2);
    assert_eq!(m.bus().mfp().peek(0) & 1, 1, "BUSY after the strobe");
}

#[test]
fn a_connected_printer_is_not_busy() {
    let dir = temp("busy");
    let mut m = AtariSt::new(config(rom(&[BRA_SELF], &[]), RamSize::M1)).unwrap();
    m.attach_printer(FileSink::create(&dir.join("p.txt"), false).unwrap());
    run(&mut m, 2);
    assert_eq!(m.bus().mfp().peek(0) & 1, 0);
    let _ = std::fs::remove_dir_all(&dir);
}
