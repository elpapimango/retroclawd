//! The MIDI port: the second ACIA at `$FFFC04/06` at 31 250 baud, with a file or an echo on the other end.

mod common;
use common::{BRA_SELF, config, move_b, move_b_abs, rom, wait_bit};

use std::sync::{Arc, Mutex};

use atari_st::serial::{FileSink, FileSource, LinePeer, MIDI_FRAME_CYCLES};
use atari_st::{AtariSt, RamSize, SerialPeer};
use machine::Machine;

const MIDI_CTRL: u16 = 0xFC04;
const MIDI_DATA: u16 = 0xFC06;
const TDRE: u16 = 1;
const RDRF: u16 = 0;

/// Master reset, then divide by 16, 8 bits, no parity, 1 stop (the MC6850 data sheet's control register).
fn setup() -> Vec<u16> {
    let mut p = Vec::new();
    p.extend(move_b(0x03, MIDI_CTRL));
    p.extend(move_b(0x15, MIDI_CTRL));
    p
}

fn send(p: &mut Vec<u16>, text: &[u8]) {
    for &c in text {
        p.extend(wait_bit(TDRE, MIDI_CTRL));
        p.extend(move_b(c, MIDI_DATA));
    }
}

fn run(m: &mut AtariSt, frames: u64) {
    while m.io_stop().is_none() && m.vbl_count() < frames {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
}

struct Recorder(Arc<Mutex<Vec<(u64, u8)>>>);

impl SerialPeer for Recorder {
    fn receive(&mut self, byte: u8, at: u64) {
        self.0.lock().unwrap().push((at, byte));
    }
    fn next_byte(&mut self, _: u64) -> Option<(u64, u8)> {
        None
    }
    fn frame_cycles(&self) -> u64 {
        MIDI_FRAME_CYCLES
    }
}

/// 31 250 baud: the ACIA clock (CPU / 16) divided by 16 is 256 CPU cycles a bit, ten bits a character.
#[test]
fn a_byte_to_the_midi_acia_reaches_the_sink_at_31250_baud() {
    let mut p = setup();
    send(&mut p, b"MIDI");
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    let got = Arc::new(Mutex::new(Vec::new()));
    m.attach_serial_peer(1, Box::new(Recorder(got.clone())));
    run(&mut m, 3);
    let got = got.lock().unwrap();
    assert_eq!(got.iter().map(|g| g.1).collect::<Vec<_>>(), b"MIDI");
    assert_eq!(MIDI_FRAME_CYCLES, 2560);
    for pair in got.windows(2) {
        assert_eq!(pair[1].0 - pair[0].0, 2560, "one character apart");
    }
}

#[test]
fn the_midi_out_file_receives_the_bytes() {
    let dir = std::env::temp_dir().join(format!("atari-midi-out-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("midi.bin");
    let mut p = setup();
    send(&mut p, &[0x90, 0x3C, 0x7F]);
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    m.attach_serial_peer(
        1,
        Box::new(LinePeer::new(
            Some(FileSink::create(&out, false).unwrap()),
            None,
            false,
            MIDI_FRAME_CYCLES,
        )),
    );
    run(&mut m, 3);
    assert_eq!(std::fs::read(&out).unwrap(), [0x90, 0x3C, 0x7F]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn loopback_returns_the_byte() {
    let mut p = setup();
    send(&mut p, b"M");
    p.extend(wait_bit(RDRF, MIDI_CTRL));
    p.extend(move_b_abs(MIDI_DATA, 0x0600));
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    m.attach_serial_peer(
        1,
        Box::new(LinePeer::new(None, None, true, MIDI_FRAME_CYCLES)),
    );
    run(&mut m, 4);
    assert_eq!(m.bus().peek(0x600), b'M');
}

#[test]
fn the_midi_in_file_is_what_the_program_reads() {
    let mut p = setup();
    for slot in 0..2u16 {
        p.extend(wait_bit(RDRF, MIDI_CTRL));
        p.extend(move_b_abs(MIDI_DATA, 0x0600 + slot));
    }
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    m.attach_serial_peer(
        1,
        Box::new(LinePeer::new(
            None,
            Some(FileSource::from_bytes(vec![0xF8, 0xFA])),
            false,
            MIDI_FRAME_CYCLES,
        )),
    );
    run(&mut m, 4);
    assert_eq!([m.bus().peek(0x600), m.bus().peek(0x601)], [0xF8, 0xFA]);
}
