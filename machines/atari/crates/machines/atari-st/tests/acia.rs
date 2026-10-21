//! The keyboard and MIDI ACIAs at `$FFFC00-$FFFC06` (facts: `docs/hardware-facts.md`, "ACIAs"): registers, frame
//! timing, the shared interrupt on GPIP I4 / MFP channel 6.

mod common;
use common::{BRA_SELF, config, move_b, rom};

use std::sync::{Arc, Mutex};

use atari_st::{AtariSt, RamSize, SerialPeer, StBus};
use machine::Machine;
use mc68000::{FunctionCode, IackResponse, M68kBus, Size};

const SUPER: FunctionCode = FunctionCode::SUPERVISOR_DATA;
/// ÷64: one bit is 64 × 16 CPU cycles (the ACIA clock is the CPU clock / 16); 8N1 is 10 bits.
const BIT: u64 = 64 * 16;
const FRAME: u64 = 10 * BIT;

fn bus() -> StBus {
    StBus::new(&config(rom(&[BRA_SELF], &[]), RamSize::M1)).unwrap()
}

/// A peer that records what it receives and sends a fixed list of bytes, each as soon as the line is free.
#[derive(Default)]
struct Recorder {
    got: Arc<Mutex<Vec<(u64, u8)>>>,
    to_send: Vec<u8>,
    sent: usize,
}

impl SerialPeer for Recorder {
    fn receive(&mut self, byte: u8, at: u64) {
        self.got.lock().unwrap().push((at, byte));
    }
    fn next_byte(&mut self, now: u64) -> Option<(u64, u8)> {
        let b = *self.to_send.get(self.sent)?;
        self.sent += 1;
        Some((now, b))
    }
    fn frame_cycles(&self) -> u64 {
        FRAME
    }
}

/// An endless sender.
struct Flood;
impl SerialPeer for Flood {
    fn receive(&mut self, _: u8, _: u64) {}
    fn next_byte(&mut self, now: u64) -> Option<(u64, u8)> {
        Some((now, 0x55))
    }
    fn frame_cycles(&self) -> u64 {
        FRAME
    }
}

#[test]
fn master_reset_then_96_reads_status_02() {
    let mut b = bus();
    b.write(0xFFFC00, Size::Byte, 0x03, SUPER).unwrap();
    b.write(0xFFFC00, Size::Byte, 0x96, SUPER).unwrap();
    assert_eq!(b.read(0xFFFC00, Size::Byte, SUPER), Ok(0x02));
    assert!(b.stop().is_none());
}

fn reset_and_send(byte: u8) -> Vec<u16> {
    let mut p = Vec::new();
    p.extend(move_b(0x03, 0xFC00));
    p.extend(move_b(0x96, 0xFC00));
    p.extend(move_b(byte, 0xFC02));
    p.push(BRA_SELF);
    p
}

/// The transmitter takes the written character at its next bit-clock edge (TDRE sets again then), and the peer has
/// it one frame later.
#[test]
fn transmit_takes_one_frame() {
    let mut m = AtariSt::new(config(rom(&reset_and_send(0x80), &[]), RamSize::M1)).unwrap();
    let got = Arc::new(Mutex::new(Vec::new()));
    m.attach_serial_peer(
        0,
        Box::new(Recorder {
            got: got.clone(),
            ..Recorder::default()
        }),
    );
    let mut written_at = None;
    let mut tdre_at = None;
    while m.now() < 40_000 {
        m.step();
        let status = m.bus().acia(0).peek(false);
        if written_at.is_none() && status & 0x02 == 0 && m.bus().acia(0).divide() == 64 {
            written_at = Some(m.now());
        }
        if written_at.is_some() && tdre_at.is_none() && status & 0x02 != 0 {
            tdre_at = Some(m.now());
        }
    }
    let (written, tdre) = (written_at.unwrap(), tdre_at.unwrap());
    assert!(
        tdre - written <= BIT + 24,
        "TDRE back {} cycles after the write",
        tdre - written
    );
    let got = got.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1, 0x80);
    let start = m.serial_tx_started_at(0).expect("started");
    assert_eq!(got[0].0, start + FRAME, "one frame after the start");
}

/// A received character sets RDRF; the ACIAs' shared IRQ pulls GPIP I4 low, MFP channel 6, vector VR + 6.
#[test]
fn received_byte_raises_gpip_i4_and_mfp_channel_6() {
    let mut p = Vec::new();
    p.extend(move_b(0x03, 0xFC00));
    p.extend(move_b(0x96, 0xFC00));
    p.extend(move_b(0x03, 0xFC04)); // the MIDI ACIA too: until its first reset it holds the shared line low
    p.extend(move_b(0x40, 0xFA17)); // VR $40
    p.extend(move_b(0x40, 0xFA09)); // IERB bit 6: channel 6
    p.extend(move_b(0x40, 0xFA15)); // IMRB
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    m.attach_serial_peer(
        0,
        Box::new(Recorder {
            to_send: vec![0xF1],
            ..Recorder::default()
        }),
    );
    while m.bus().acia(0).peek(false) & 0x01 == 0 && m.now() < 100_000 {
        m.step();
    }
    assert_eq!(m.bus().acia(0).peek(false), 0x83);
    assert_eq!(m.bus().mfp().read(0) & 0x10, 0, "I4 low");
    assert_eq!(m.bus().ipl(), 6);
    let mut b = m.into_bus();
    assert_eq!(b.iack(6), IackResponse::Vector(0x46));
    assert_eq!(b.read(0xFFFC02, Size::Byte, SUPER), Ok(0xF1));
    assert_eq!(b.mfp().read(0) & 0x10, 0x10, "I4 high again");
}

/// GPIP I4 reads low until the first master reset ([O] Hatari's `$A1`), high (no IRQ) after it.
#[test]
fn gpip_i4_low_until_the_first_master_reset() {
    let mut b = bus();
    assert_eq!(b.read(0xFFFA01, Size::Byte, SUPER), Ok(0xA1));
    b.write(0xFFFC00, Size::Byte, 0x03, SUPER).unwrap();
    b.write(0xFFFC04, Size::Byte, 0x03, SUPER).unwrap();
    assert_eq!(b.read(0xFFFA01, Size::Byte, SUPER), Ok(0xB1));
}

#[test]
fn midi_null_peer_never_receives_and_status_reads_02() {
    let mut p = Vec::new();
    p.extend(move_b(0x03, 0xFC04));
    p.extend(move_b(0x95, 0xFC04));
    p.extend(move_b(0x12, 0xFC06));
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    while m.now() < 100_000 {
        m.step();
    }
    assert_eq!(m.bus().acia(1).peek(false), 0x02);
    assert!(m.io_stop().is_none());
}

/// With nobody reading, the receiver overruns (data sheet sequence), and nothing piles up.
#[test]
fn unread_bytes_set_overrun_and_stay_bounded() {
    let mut p = Vec::new();
    p.extend(move_b(0x03, 0xFC00));
    p.extend(move_b(0x16, 0xFC00)); // no receive interrupt
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    m.attach_serial_peer(0, Box::new(Flood));
    while m.now() < 10_000 * FRAME / 100 {
        m.step();
    }
    let mut b = m.into_bus();
    assert_eq!(b.read(0xFFFC02, Size::Byte, SUPER), Ok(0x55));
    assert_eq!(b.read(0xFFFC00, Size::Byte, SUPER).unwrap() & 0x21, 0x21);
}

#[test]
fn odd_and_word_accesses_have_no_fact() {
    let mut b = bus();
    b.read(0xFFFC01, Size::Byte, SUPER).unwrap();
    assert_eq!(b.stop().unwrap().reason, atari_st::StopReason::NoFact);
    let mut b = bus();
    b.read(0xFFFC00, Size::Word, SUPER).unwrap();
    assert_eq!(b.stop().unwrap().reason, atari_st::StopReason::NoFact);
}
