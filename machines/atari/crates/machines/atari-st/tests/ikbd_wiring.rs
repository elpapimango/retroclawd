//! The IKBD on the keyboard ACIA: bare-metal programs read its reset reply, key codes and mouse packets.

mod common;
use common::{config, move_b, rom};

use atari_st::{AtariSt, RamSize};
use machine::Machine;

/// Resets both ACIAs (keyboard ÷64 8N1), sends `$80 $01` to the IKBD, then stores every received byte from `$1000`
/// on.
fn reader() -> Vec<u16> {
    let wait_tdre = [0x0838, 0x0001, 0xFC00, 0x67F8]; // BTST #1,$FFFC00.W; BEQ.S back to the BTST
    let mut p = Vec::new();
    p.extend(move_b(0x03, 0xFC00));
    p.extend(move_b(0x03, 0xFC04));
    p.extend(move_b(0x96, 0xFC00));
    p.extend(wait_tdre);
    p.extend(move_b(0x80, 0xFC02));
    p.extend(wait_tdre);
    p.extend(move_b(0x01, 0xFC02));
    p.extend([0x41F8, 0x1000]); // LEA $1000.W,A0
    // loop: BTST #0,$FFFC00.W; BEQ.S loop; MOVE.B $FFFC02.W,(A0)+; BRA.S loop
    p.extend([0x0838, 0x0000, 0xFC00, 0x67F8, 0x10F8, 0xFC02, 0x60F2]);
    p
}

fn machine() -> AtariSt {
    AtariSt::new(config(rom(&reader(), &[]), RamSize::M1)).unwrap()
}

fn run_until(m: &mut AtariSt, cycle: u64) {
    while m.now() < cycle {
        m.step();
    }
}

fn received(m: &AtariSt, n: u64) -> Vec<u8> {
    (0..n).map(|i| m.read_memory(0x1000 + i)).collect()
}

/// After the reset delay ([O] 502 002 cycles) and one frame on the line, the program has `$F1`.
#[test]
fn bare_metal_reads_reset_reply() {
    let mut m = machine();
    run_until(&mut m, 2_000_000);
    assert_eq!(m.read_memory(0x1000), 0xF1);
    assert!(m.io_stop().is_none());
}

#[test]
fn key_press_reaches_the_cpu_as_make_code() {
    let mut m = machine();
    run_until(&mut m, 2_000_000);
    m.key_event("Space", true).unwrap();
    m.key_event("Space", false).unwrap();
    run_until(&mut m, 2_100_000);
    let got = received(&m, 4);
    let n = got.iter().position(|&b| b == 0x39).expect("make code");
    assert_eq!(got[n + 1], 0xB9);
}

#[test]
fn mouse_motion_arrives_as_a_relative_packet() {
    let mut m = machine();
    run_until(&mut m, 2_000_000);
    m.mouse_motion(3, -2);
    run_until(&mut m, 2_100_000);
    let got = received(&m, 6);
    let n = got.iter().position(|&b| b == 0xF8).expect("header");
    assert_eq!(&got[n..n + 3], &[0xF8, 3, 0xFE]);
}

#[test]
fn joystick_1_arrives_as_an_event() {
    let mut m = machine();
    run_until(&mut m, 2_000_000);
    m.joystick(1, 0x80);
    run_until(&mut m, 2_100_000);
    let got = received(&m, 4);
    let n = got.iter().position(|&b| b == 0xFF).expect("header");
    assert_eq!(got[n + 1], 0x80);
}

#[test]
fn unknown_key_name_is_an_error() {
    let mut m = machine();
    assert!(m.key_event("NoSuchKey", true).is_err());
}
