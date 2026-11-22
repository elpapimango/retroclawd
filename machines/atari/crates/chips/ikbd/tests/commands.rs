//! The IKBD's joystick, clock, memory and control commands against Atari's *Intelligent Keyboard (ikbd) Protocol*
//! (revised 26 February 1985); section numbers are the document's.

use ikbd::Ikbd;

const CPS: u64 = 8_000_000;
const T: u64 = 1_000_000;
/// A hundredth and a tenth of a second.
const HUNDREDTH: u64 = CPS / 100;
const TENTH: u64 = CPS / 10;

fn drain(k: &mut Ikbd, now: u64) -> Vec<u8> {
    std::iter::from_fn(|| k.pop_output(now)).collect()
}

fn send(k: &mut Ikbd, bytes: &[u8], at: u64) {
    for &b in bytes {
        k.command_byte(b, at);
    }
}

fn ready() -> Ikbd {
    let mut k = Ikbd::new();
    k.set_cycles_per_second(CPS);
    drain(&mut k, T);
    k
}

/// §9.15, §9.16: event reporting is the default; interrogation mode stops the events; `$16` still answers.
#[test]
fn joystick_event_and_interrogation_modes() {
    let mut k = ready();
    send(&mut k, &[0x14], T);
    k.joystick(1, 0x01, T);
    assert_eq!(drain(&mut k, T), vec![0xFF, 0x01]);
    send(&mut k, &[0x15], T);
    k.joystick(1, 0x81, T);
    assert_eq!(drain(&mut k, T), vec![], "no events in interrogation mode");
    send(&mut k, &[0x16], T);
    assert_eq!(drain(&mut k, T), vec![0xFD, 0x00, 0x81]);
}

/// §9.17: the interrogation answers in the event mode as well (`$FD`, joystick 0, joystick 1).
#[test]
fn joystick_interrogation_in_event_mode() {
    let mut k = ready();
    send(&mut k, &[0x14], T);
    k.joystick(0, 0x82, T);
    k.joystick(1, 0x04, T);
    drain(&mut k, T);
    send(&mut k, &[0x16], T);
    assert_eq!(drain(&mut k, T), vec![0xFD, 0x82, 0x04]);
}

/// §9.21: events stop; any joystick mode command resumes them.
#[test]
fn disable_joysticks_stops_events_until_a_joystick_command() {
    let mut k = ready();
    send(&mut k, &[0x1A], T);
    k.joystick(1, 0x01, T);
    assert_eq!(drain(&mut k, T), vec![]);
    send(&mut k, &[0x14], T);
    k.joystick(1, 0x02, T);
    assert_eq!(drain(&mut k, T), vec![0xFF, 0x02]);
}

/// §9.18: every `rate` hundredths of a second two bytes: `%000000ab` (a joystick 1's fire, b joystick 0's), then
/// `%nnnnmmmm` (the sticks, joystick 1 high).
#[test]
fn joystick_monitoring_reports_at_the_given_rate() {
    let mut k = ready();
    send(&mut k, &[0x17, 5], T);
    assert_eq!(k.next_wake(), Some(T + 5 * HUNDREDTH));
    k.joystick(1, 0x82, T + 100);
    k.joystick(0, 0x84, T + 200);
    assert_eq!(
        drain(&mut k, T + 5 * HUNDREDTH - 1),
        vec![],
        "before the first sample"
    );
    assert_eq!(drain(&mut k, T + 5 * HUNDREDTH), vec![0x03, 0x24]);
    assert_eq!(
        drain(&mut k, T + 15 * HUNDREDTH),
        vec![0x03, 0x24, 0x03, 0x24],
        "one pair for each period that passed"
    );
    k.joystick(1, 0x00, T + 15 * HUNDREDTH + 5);
    assert_eq!(drain(&mut k, T + 20 * HUNDREDTH), vec![0x01, 0x04]);
}

/// §9.18: "do nothing but monitor the serial command line, maintain the time-of-day clock, and monitor the
/// joystick": no key or mouse records.
#[test]
fn joystick_monitoring_reports_nothing_else() {
    let mut k = ready();
    send(&mut k, &[0x17, 50], T);
    k.key(0x1E, true, T);
    k.mouse_move(5, 5, T);
    k.joystick(1, 0x01, T);
    assert_eq!(
        drain(&mut k, T + 10),
        vec![],
        "no key, mouse or event record"
    );
}

/// §5.3: "The PAUSE command in this mode not only stops the output but also temporarily stops scanning the joysticks
/// (samples are not queued)."
#[test]
fn pause_stops_the_monitoring_samples() {
    let mut k = ready();
    send(&mut k, &[0x17, 10], T);
    send(&mut k, &[0x13], T);
    assert_eq!(k.next_wake(), None);
    assert_eq!(drain(&mut k, T + 100 * HUNDREDTH), vec![]);
    send(&mut k, &[0x11], T + 100 * HUNDREDTH);
    assert_eq!(
        drain(&mut k, T + 100 * HUNDREDTH),
        vec![],
        "nothing was queued meanwhile"
    );
    assert_eq!(drain(&mut k, T + 110 * HUNDREDTH), vec![0x00, 0x00]);
}

/// §9.19: the joystick 1 fire button, eight samples to a byte, the first the most significant bit, at the rate of
/// eight samples in the time a character takes.
#[test]
fn fire_button_monitoring_packs_eight_samples_a_byte() {
    let mut k = ready();
    send(&mut k, &[0x18], T);
    let sample = ikbd::BYTE_CYCLES / 8;
    assert_eq!(k.next_wake(), Some(T + sample));
    k.joystick(1, 0x80, T + 1);
    assert_eq!(drain(&mut k, T + 8 * sample), vec![0xFF]);
    // Released after the tenth sample: samples 9 and 10 were high, 11 to 16 low.
    k.joystick(1, 0x00, T + 10 * sample + 1);
    assert_eq!(drain(&mut k, T + 16 * sample), vec![0xC0]);
}

/// §9.27: "STATUS INQUIRY commands are not valid if the ikbd is in JOYSTICK MONITORING mode or FIRE BUTTON
/// MONITORING mode."
#[test]
fn status_inquiries_are_ignored_while_monitoring() {
    let mut k = ready();
    send(&mut k, &[0x17, 100, 0x88, 0x94], T);
    assert_eq!(drain(&mut k, T), vec![]);
}

/// §5.5, §9.20: on closure a make/break pair, then pairs every T tenths until R tenths have passed, then every V
/// tenths; the pseudo key codes are `$74` and `$75` for the fire buttons.
#[test]
fn joystick_keycode_mode_repeats_with_a_velocity_breakpoint() {
    let mut k = ready();
    // RX RY TX TY VX VY: horizontal breakpoint after 0.2 s, 0.1 s apart before it, 0.3 s apart after.
    send(&mut k, &[0x19, 2, 2, 1, 1, 3, 3], T);
    k.joystick(0, 0x08, T); // right
    assert_eq!(drain(&mut k, T), vec![0x4D, 0xCD], "at once");
    assert_eq!(k.next_wake(), Some(T + TENTH));
    assert_eq!(
        drain(&mut k, T + 2 * TENTH),
        vec![0x4D, 0xCD],
        "0.1 s: still before the 0.2 s breakpoint"
    );
    // [H?] the next T-spaced pair (0.2 s) would reach the breakpoint, so V (0.3 s) counts from the last pair.
    assert_eq!(k.next_wake(), Some(T + 4 * TENTH));
    assert_eq!(drain(&mut k, T + 4 * TENTH - 1), vec![]);
    assert_eq!(drain(&mut k, T + 4 * TENTH), vec![0x4D, 0xCD]);
    assert_eq!(
        drain(&mut k, T + 7 * TENTH),
        vec![0x4D, 0xCD],
        "then every 0.3 s"
    );
    k.joystick(0, 0x00, T + 7 * TENTH + 1);
    assert_eq!(k.next_wake(), None, "released: no more repeats");
    assert_eq!(drain(&mut k, T + 20 * TENTH), vec![]);
}

/// §9.20: "by setting RX and/or RY to zero, the velocity feature can be disabled. The values of TX and TY then
/// become meaningless, and the generation of cursor keystrokes is set by VX and VY."
#[test]
fn joystick_keycode_mode_with_r_zero_uses_v_only() {
    let mut k = ready();
    send(&mut k, &[0x19, 0, 0, 9, 9, 2, 2], T);
    k.joystick(0, 0x01, T); // up
    k.joystick(0, 0x03, T); // up and down together: neither
    assert_eq!(drain(&mut k, T), vec![0x48, 0xC8]);
    k.joystick(0, 0x02, T + 1); // down
    assert_eq!(drain(&mut k, T + 1), vec![0x50, 0xD0]);
    assert_eq!(
        drain(&mut k, T + 4 * TENTH + 1),
        vec![0x50, 0xD0, 0x50, 0xD0],
        "every 0.2 s"
    );
}

/// §5.5: the two axes repeat on their own timers.
#[test]
fn joystick_keycode_axes_are_independent() {
    let mut k = ready();
    send(&mut k, &[0x19, 0, 0, 0, 0, 1, 2], T);
    k.joystick(0, 0x0C, T); // left and right together: neither
    k.joystick(0, 0x05, T); // up and left
    assert_eq!(drain(&mut k, T), vec![0x4B, 0xCB, 0x48, 0xC8]);
    assert_eq!(
        drain(&mut k, T + 2 * TENTH),
        vec![0x4B, 0xCB, 0x4B, 0xCB, 0x48, 0xC8],
        "x every 0.1 s, y every 0.2 s, in time order (x before y at the same instant)"
    );
}

/// §5.5: the fire buttons are the pseudo keys `$74` (joystick 0) and `$75` (joystick 1).
#[test]
fn joystick_keycode_mode_fire_buttons_are_keys() {
    let mut k = ready();
    send(&mut k, &[0x19, 0, 0, 0, 0, 0, 0], T);
    k.joystick(0, 0x80, T);
    k.joystick(0, 0x00, T);
    k.joystick(1, 0x80, T);
    assert_eq!(drain(&mut k, T), vec![0x74, 0xF4, 0x75]);
}

/// §9.27: the joystick mode status lays out the keycode parameters like the command; `$14`/`$15` answer by code.
#[test]
fn joystick_status_inquiries() {
    let mut k = ready();
    send(&mut k, &[0x94], T);
    assert_eq!(drain(&mut k, T), vec![0xF6, 0x14, 0, 0, 0, 0, 0, 0]);
    send(&mut k, &[0x15, 0x95], T);
    assert_eq!(drain(&mut k, T), vec![0xF6, 0x15, 0, 0, 0, 0, 0, 0]);
    send(&mut k, &[0x19, 1, 2, 3, 4, 5, 6, 0x99], T);
    assert_eq!(drain(&mut k, T), vec![0xF6, 0x19, 1, 2, 3, 4, 5, 6]);
    send(&mut k, &[0x9A, 0x1A, 0x9A], T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xF6, 0x00, 0, 0, 0, 0, 0, 0, 0xF6, 0x1A, 0, 0, 0, 0, 0, 0]
    );
}

/// §9.22, §9.23: the clock takes its time from the controller's own cycle count, never from anywhere else: it stands
/// still while no time passes and runs one second per `cycles_per_second`.
#[test]
fn clock_advances_with_scheduler_time_only() {
    let mut k = ready();
    send(&mut k, &[0x1B, 0x99, 0x12, 0x31, 0x23, 0x59, 0x58], T);
    send(&mut k, &[0x1C], T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xFC, 0x99, 0x12, 0x31, 0x23, 0x59, 0x58]
    );
    // The same instant, asked twice, and a long time of other traffic that carries no time of its own.
    send(&mut k, &[0x1C], T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xFC, 0x99, 0x12, 0x31, 0x23, 0x59, 0x58]
    );
    k.tick_clock(T + CPS - 1, CPS);
    send(&mut k, &[0x1C], T + CPS - 1);
    assert_eq!(
        drain(&mut k, T + CPS - 1),
        vec![0xFC, 0x99, 0x12, 0x31, 0x23, 0x59, 0x58]
    );
    // Two seconds: 23:59:58 on 31 December 1999 becomes 00:00:00 on 1 January 2000 (year 99 wraps to 00).
    k.tick_clock(T + 2 * CPS, CPS);
    send(&mut k, &[0x1C], T + 2 * CPS);
    assert_eq!(
        drain(&mut k, T + 2 * CPS),
        vec![0xFC, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00]
    );
}

/// [H?] two-digit years: 2000 is a leap year (divisible by 4), so February has 29 days.
#[test]
fn the_clock_knows_leap_years() {
    let mut k = ready();
    send(&mut k, &[0x1B, 0x00, 0x02, 0x28, 0x23, 0x59, 0x59], T);
    k.tick_clock(T + CPS, CPS);
    send(&mut k, &[0x1C], T + CPS);
    assert_eq!(
        drain(&mut k, T + CPS),
        vec![0xFC, 0x00, 0x02, 0x29, 0x00, 0x00, 0x00]
    );
    send(&mut k, &[0x1B, 0x01, 0x02, 0x28, 0x23, 0x59, 0x59], T + CPS);
    k.tick_clock(T + 2 * CPS, CPS);
    send(&mut k, &[0x1C], T + 2 * CPS);
    assert_eq!(
        drain(&mut k, T + 2 * CPS),
        vec![0xFC, 0x01, 0x03, 0x01, 0x00, 0x00, 0x00]
    );
}

/// A host that did not look for a very long time (or a corrupt state with a tiny clock rate) must not make the
/// clock loop for ever.
#[test]
fn a_huge_time_step_is_cheap() {
    let mut k = ready();
    k.tick_clock(u64::MAX / 4, CPS);
    k.tick_clock(u64::MAX / 2, 1);
    send(&mut k, &[0x1C], u64::MAX / 2);
    assert_eq!(drain(&mut k, u64::MAX / 2).len(), 7);
}

/// §9.1: RESET keeps the clock.
#[test]
fn reset_keeps_the_clock() {
    let mut k = ready();
    send(&mut k, &[0x1B, 0x25, 0x06, 0x07, 0x08, 0x09, 0x10], T);
    send(&mut k, &[0x80, 0x01], T);
    let version = T + ikbd::RESET_REPLY_DELAY;
    assert_eq!(drain(&mut k, version), vec![0xF1]);
    send(&mut k, &[0x1C], version);
    assert_eq!(
        drain(&mut k, version),
        vec![0xFC, 0x25, 0x06, 0x07, 0x08, 0x09, 0x10]
    );
}

/// §9.1: RESET returns every mode to its default; [H?] a packet being sent is cut off and the queue is emptied.
#[test]
fn reset_restores_the_defaults_and_empties_the_queue() {
    let mut k = ready();
    send(&mut k, &[0x15, 0x09, 0x01, 0x40, 0x00, 0xC8, 0x16], T);
    assert_eq!(k.pop_output(T), Some(0xFD), "a packet is begun");
    send(&mut k, &[0x13], T);
    k.key(0x1E, true, T);
    send(&mut k, &[0x80, 0x01], T + 2);
    assert_eq!(
        drain(&mut k, T + 3),
        vec![],
        "the rest of the packet, the key and the pause are gone"
    );
    let version = T + 2 + ikbd::RESET_REPLY_DELAY;
    assert_eq!(drain(&mut k, version), vec![0xF1]);
    k.joystick(1, 0x01, version);
    k.mouse_move(2, 0, version);
    assert_eq!(drain(&mut k, version), vec![0xFF, 0x01, 0xF8, 2, 0]);
}

/// §9.24, §9.25: [H?] the controller's own memory is not emulated; what the host loads reads back, the rest is zero.
#[test]
fn memory_load_and_examine_round_trip() {
    let mut k = ready();
    send(&mut k, &[0x20, 0x00, 0x80, 4, 0xDE, 0xAD, 0xBE, 0xEF], T);
    send(&mut k, &[0x21, 0x00, 0x7F], T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xF6, 0x20, 0, 0xDE, 0xAD, 0xBE, 0xEF, 0]
    );
    send(&mut k, &[0x21, 0xFF, 0xFE], T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xF6, 0x20, 0, 0, 0, 0, 0, 0],
        "an address near the top wraps and reads what was never loaded"
    );
}

/// §9.24: "(0-128)": a count of zero loads nothing, and a count over 128 loads the first 128 bytes ([H?]).
#[test]
fn memory_load_counts() {
    let mut k = ready();
    send(&mut k, &[0x20, 0x10, 0x00, 0], T);
    let mut big = vec![0x20, 0x20, 0x00, 200];
    big.extend((0..200u8).map(|i| i | 1));
    send(&mut k, &big, T);
    send(&mut k, &[0x21, 0x10, 0x00], T);
    assert_eq!(drain(&mut k, T), vec![0xF6, 0x20, 0, 0, 0, 0, 0, 0]);
    send(&mut k, &[0x21, 0x20, 0x7E], T);
    // Bytes 126 and 127 were loaded, 128 and on were not.
    assert_eq!(drain(&mut k, T), vec![0xF6, 0x20, 127, 127, 0, 0, 0, 0]);
}

/// §9.26: [H?] the controller's firmware is not run; the address is noted.
#[test]
fn controller_execute_is_recorded_not_run() {
    let mut k = ready();
    send(&mut k, &[0x22, 0xF0, 0x00, 0x22, 0x12, 0x34], T);
    assert_eq!(k.executed(), &[0xF000, 0x1234]);
    assert_eq!(drain(&mut k, T), vec![]);
    k.key(0x1E, true, T);
    assert_eq!(
        drain(&mut k, T),
        vec![0x1E],
        "and the controller carries on"
    );
}

/// §9: "Command codes ... which are not specified should perform no operation."
#[test]
fn unknown_command_bytes_are_ignored_not_fatal() {
    let mut k = ready();
    // Unspecified codes between the commands, and the status codes the document does not list.
    send(
        &mut k,
        &[
            0x00, 0x01, 0x05, 0x06, 0x1D, 0x1E, 0x1F, 0x23, 0x30, 0x7F, 0x81, 0x86, 0x8D, 0x91,
            0x9B, 0xFF,
        ],
        T,
    );
    assert_eq!(drain(&mut k, T), vec![]);
    k.mouse_move(1, 0, T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xF8, 1, 0],
        "the modes are untouched"
    );
}

/// §9.12: RESUME is a no-operation when output is not paused; any command resumes a pause.
#[test]
fn resume_without_a_pause_does_nothing() {
    let mut k = ready();
    send(&mut k, &[0x11], T);
    k.key(0x1E, true, T);
    assert_eq!(drain(&mut k, T), vec![0x1E]);
}

/// A host may set a delay between a query and its answer (`set_reply_delays`); the answer is not ready before it.
#[test]
fn an_answer_waits_for_the_reply_delay_the_host_set() {
    let mut k = ready();
    k.set_reply_delays(6_500, 5_000);
    send(&mut k, &[0x16], T);
    assert_eq!(drain(&mut k, T + 6_499), vec![], "not yet");
    assert_eq!(drain(&mut k, T + 6_500), vec![0xFD, 0x00, 0x00]);
    send(&mut k, &[0x87], T);
    assert_eq!(drain(&mut k, T + 4_999), vec![]);
    assert_eq!(drain(&mut k, T + 5_000).len(), 8);
}

/// A reset keeps the host's reply delays (they are the host's, not a mode).
#[test]
fn reset_keeps_the_reply_delays() {
    let mut k = ready();
    k.set_reply_delays(6_500, 5_000);
    send(&mut k, &[0x80, 0x01], T);
    drain(&mut k, T + 1_000_000);
    send(&mut k, &[0x16], 2 * T);
    assert_eq!(drain(&mut k, 2 * T + 6_499), vec![]);
    assert_eq!(drain(&mut k, 2 * T + 6_500), vec![0xFD, 0x00, 0x00]);
}
