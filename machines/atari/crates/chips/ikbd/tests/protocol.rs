//! The IKBD against Atari's *Intelligent Keyboard (ikbd) Protocol* (revised 26 February 1985); section numbers are the
//! document's.

use ikbd::{Ikbd, RESET_REPLY_DELAY, SCANCODES};

/// Every byte the IKBD has ready by `now`.
fn drain(k: &mut Ikbd, now: u64) -> Vec<u8> {
    std::iter::from_fn(|| k.pop_output(now)).collect()
}

fn send(k: &mut Ikbd, bytes: &[u8], at: u64) {
    for &b in bytes {
        k.command_byte(b, at);
    }
}

/// A controller past its power-up reply, at time 0.
fn ready() -> Ikbd {
    let mut k = Ikbd::new();
    drain(&mut k, u64::MAX / 2);
    k
}

const T: u64 = 1_000_000;

/// §9.1 and §8: RESET (`$80 $01`) answers with the version code after the self-test ([O] `$F1`, the document's
/// "second release"), after a delay ([O] Hatari 2.6.1: 502 002 CPU cycles).
#[test]
fn reset_replies_f1() {
    let mut k = ready();
    send(&mut k, &[0x80, 0x01], T);
    assert_eq!(k.next_output_at(), Some(T + RESET_REPLY_DELAY));
    assert_eq!(drain(&mut k, T + RESET_REPLY_DELAY - 1), vec![]);
    assert_eq!(drain(&mut k, T + RESET_REPLY_DELAY), vec![0xF1]);
}

/// §9.1: "Any byte following an 0x80 command byte other than 0x01 is ignored (and causes the 0x80 to be ignored)."
#[test]
fn reset_needs_01() {
    let mut k = ready();
    send(&mut k, &[0x80, 0x16], T);
    assert_eq!(
        drain(&mut k, u64::MAX / 2),
        vec![],
        "the $16 was eaten with the $80"
    );
}

/// §4.1: relative record `%111110xx` (bit 1 left, bit 0 right), dx, dy; threshold 1; Y=0 at top makes motion toward
/// the user positive.
#[test]
fn relative_mouse_packet() {
    let mut k = ready();
    k.mouse_move(5, -3, T);
    assert_eq!(drain(&mut k, T), vec![0xF8, 5, 0xFD]);
    k.mouse_button(true, true, T);
    assert_eq!(drain(&mut k, T), vec![0xFA, 0, 0]);
    k.mouse_button(false, true, T);
    assert_eq!(drain(&mut k, T), vec![0xFB, 0, 0]);
    send(&mut k, &[0x0F], T); // Y=0 at bottom
    k.mouse_move(0, 2, T);
    assert_eq!(drain(&mut k, T), vec![0xFB, 0, 0xFE]);
}

/// §4.1: motion beyond +127..-128 is broken into several packets.
#[test]
fn mouse_moves_larger_than_127_split_into_packets() {
    let mut k = ready();
    k.mouse_move(300, 0, T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xF8, 127, 0, 0xF8, 127, 0, 0xF8, 46, 0]
    );
}

/// §9.6: the threshold delays the report but loses no resolution.
#[test]
fn threshold_accumulates() {
    let mut k = ready();
    send(&mut k, &[0x0B, 4, 4], T);
    k.mouse_move(2, 0, T);
    assert_eq!(drain(&mut k, T), vec![]);
    k.mouse_move(3, 0, T);
    assert_eq!(drain(&mut k, T), vec![0xF8, 5, 0]);
}

/// §3: break = make | `$80`.
#[test]
fn key_make_and_break() {
    let mut k = ready();
    k.key(0x39, true, T);
    k.key(0x39, false, T);
    assert_eq!(drain(&mut k, T), vec![0x39, 0xB9]);
}

/// §5.1 and §8: joystick 1 reports events by default (`$FF`, then `%x000yyyy`); joystick 0 only after a joystick
/// command.
#[test]
fn joystick_1_event() {
    let mut k = ready();
    k.joystick(1, 0x81, T);
    assert_eq!(drain(&mut k, T), vec![0xFF, 0x81]);
    k.joystick(1, 0x81, T);
    assert_eq!(drain(&mut k, T), vec![], "no change, no event");
    k.joystick(0, 0x02, T);
    assert_eq!(drain(&mut k, T), vec![], "port 0 is the mouse");
    send(&mut k, &[0x14], T);
    k.joystick(0, 0x04, T);
    assert_eq!(drain(&mut k, T), vec![0xFE, 0x04]);
    send(&mut k, &[0x16], T);
    assert_eq!(drain(&mut k, T), vec![0xFD, 0x04, 0x81]);
}

/// §9.22-9.23: set and interrogate the clock (packed BCD); it advances by elapsed whole seconds.
#[test]
fn clock_set_and_interrogate() {
    let mut k = ready();
    let cps = 8_000_000;
    send(&mut k, &[0x1B, 0x25, 0x06, 0x07, 0x23, 0x59, 0x58], T);
    k.tick_clock(T + 3 * cps, cps);
    send(&mut k, &[0x1C], T + 3 * cps);
    assert_eq!(
        drain(&mut k, T + 3 * cps),
        vec![0xFC, 0x25, 0x06, 0x08, 0x00, 0x00, 0x01]
    );
    // An invalid BCD digit leaves its field alone.
    send(
        &mut k,
        &[0x1B, 0xFF, 0xFF, 0xFF, 0x10, 0xFF, 0xFF],
        T + 3 * cps,
    );
    send(&mut k, &[0x1C], T + 3 * cps);
    assert_eq!(
        drain(&mut k, T + 3 * cps),
        vec![0xFC, 0x25, 0x06, 0x08, 0x10, 0x00, 0x01]
    );
}

/// [O] Hatari: the clock starts at zero; interrogated at 1.2 s it reads 00-00-00 00:00:01.
#[test]
fn clock_starts_at_zero() {
    let mut k = Ikbd::new();
    let cps = 8_021_247;
    let at = cps * 6 / 5;
    k.tick_clock(at, cps);
    drain(&mut k, at);
    send(&mut k, &[0x1C], at);
    assert_eq!(drain(&mut k, at), vec![0xFC, 0, 0, 0, 0, 0, 1]);
}

/// §9: "Command codes … which are not specified should perform no operation."
#[test]
fn unknown_command_is_nop() {
    let mut k = ready();
    send(&mut k, &[0x00, 0x05, 0x30, 0x7F], T);
    k.key(0x1E, true, T);
    assert_eq!(drain(&mut k, T), vec![0x1E]);
}

/// §9.14: PAUSE holds output (key codes are still queued); any command resumes.
#[test]
fn paused_output_queues_until_resume() {
    let mut k = ready();
    send(&mut k, &[0x13], T);
    k.key(0x1E, true, T);
    assert_eq!(drain(&mut k, T), vec![]);
    send(&mut k, &[0x11], T);
    assert_eq!(drain(&mut k, T), vec![0x1E]);
}

/// §9.27: status inquiries answer `$F6` and 7 more bytes; `$88` reports the mouse mode, `$8F` the Y origin.
#[test]
fn status_inquiries() {
    let mut k = ready();
    send(&mut k, &[0x88], T);
    assert_eq!(drain(&mut k, T), vec![0xF6, 0x08, 0, 0, 0, 0, 0, 0]);
    send(&mut k, &[0x0F, 0x8F], T);
    assert_eq!(drain(&mut k, T), vec![0xF6, 0x0F, 0, 0, 0, 0, 0, 0]);
}

/// The output queue is bounded (the 6301 has 128 bytes of RAM, *Internals* p.69): a mouse flood with nobody reading
/// keeps the oldest bytes.
#[test]
fn output_queue_is_bounded() {
    let mut k = ready();
    k.key(0x1E, true, T);
    for _ in 0..100_000 {
        k.mouse_move(1, 0, T);
    }
    let out = drain(&mut k, T);
    assert!(out.len() <= 256, "{}", out.len());
    assert_eq!(out[0], 0x1E, "the oldest byte is kept");
}

/// Appendix A, spot checks, and no name or code twice.
#[test]
fn scancodes_are_unique_and_match_appendix_a() {
    let code = |n: &str| SCANCODES.iter().find(|(name, _)| *name == n).map(|e| e.1);
    assert_eq!(code("Escape"), Some(0x01));
    assert_eq!(code("Space"), Some(0x39));
    assert_eq!(code("Enter"), Some(0x1C));
    assert_eq!(code("Help"), Some(0x62));
    assert_eq!(code("NumpadEnter"), Some(0x72));
    assert_eq!(code("KeyA"), Some(0x1E));
    assert_eq!(code("IntlBackslash"), Some(0x60), "the ISO key");
    let mut names: Vec<_> = SCANCODES.iter().map(|e| e.0).collect();
    let mut codes: Vec<_> = SCANCODES.iter().map(|e| e.1).collect();
    names.sort();
    codes.sort();
    names.dedup();
    codes.dedup();
    assert_eq!(names.len(), SCANCODES.len());
    assert_eq!(codes.len(), SCANCODES.len());
    assert_eq!(SCANCODES.len(), 95);
}
