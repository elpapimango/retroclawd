//! The IKBD on the keyboard ACIA: bare-metal programs read its reset reply, key codes and mouse packets.

mod common;
use common::{config, move_b, rom};

use atari_st::{AtariSt, RamSize};
use machine::Machine;

/// Resets both ACIAs (keyboard ÷64 8N1), sends `$80 $01` to the IKBD, then stores every received byte from `$1000`
/// on.
fn reader() -> Vec<u16> {
    reader_after(&[])
}

/// The same, sending `commands` to the IKBD after the reset.
fn reader_after(commands: &[u8]) -> Vec<u16> {
    let wait_tdre = [0x0838, 0x0001, 0xFC00, 0x67F8]; // BTST #1,$FFFC00.W; BEQ.S back to the BTST
    let mut p = Vec::new();
    p.extend(move_b(0x03, 0xFC00));
    p.extend(move_b(0x03, 0xFC04));
    p.extend(move_b(0x96, 0xFC00));
    p.extend(wait_tdre);
    p.extend(move_b(0x80, 0xFC02));
    p.extend(wait_tdre);
    p.extend(move_b(0x01, 0xFC02));
    for &c in commands {
        p.extend(wait_tdre);
        p.extend(move_b(c, 0xFC02));
    }
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

/// Joystick monitoring (§9.18) produces output by itself: the keyboard ACIA is woken for each sample, with no key,
/// mouse or other event to prompt it. Rate 5 is 50 ms; the program stores the pairs it receives.
#[test]
fn joystick_monitoring_samples_reach_the_cpu_on_their_own() {
    let mut m = AtariSt::new(config(rom(&reader_after(&[0x17, 0x05]), &[]), RamSize::M1)).unwrap();
    run_until(&mut m, 4_000_000);
    m.joystick(1, 0x82);
    run_until(&mut m, 12_000_000);
    let got = received(&m, 40);
    let start = got.iter().rposition(|&b| b == 0xF1).expect("reset reply") + 1;
    let samples = &got[start..];
    // 50 ms is 400 000 cycles: about 20 pairs by now, all `00 00` until the joystick moved and `02 20` after.
    assert!(samples.len() >= 24, "{} bytes", samples.len());
    let pairs: Vec<_> = samples.chunks(2).filter(|c| c.len() == 2).collect();
    assert!(
        pairs
            .iter()
            .all(|p| p[..] == [0, 0] || p[..] == [0x02, 0x20]),
        "{pairs:?}"
    );
    assert_eq!(pairs[0], [0, 0]);
    assert_eq!(pairs.last().unwrap()[..], [0x02, 0x20]);
}

/// Cursor key repeats (§9.20) wake the port as well.
#[test]
fn joystick_keycode_repeats_reach_the_cpu_on_their_own() {
    let mut m = AtariSt::new(config(
        rom(&reader_after(&[0x19, 0, 0, 0, 0, 5, 5]), &[]),
        RamSize::M1,
    ))
    .unwrap();
    run_until(&mut m, 4_000_000);
    m.joystick(0, 0x08);
    run_until(&mut m, 12_000_000);
    let got = received(&m, 40);
    let start = got.iter().rposition(|&b| b == 0xF1).expect("reset reply") + 1;
    // The pair at once, then one every half second (4 000 000 cycles).
    let keys: Vec<_> = got[start..].chunks(2).take_while(|c| c[0] != 0).collect();
    assert!(keys.len() >= 2, "{keys:?}");
    assert!(keys.iter().all(|k| k[..] == [0x4D, 0xCD]), "{keys:?}");
}
