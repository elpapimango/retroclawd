//! The IKBD's mouse command set against Atari's *Intelligent Keyboard (ikbd) Protocol* (revised 26 February 1985);
//! section numbers are the document's. One test per command, with the exact bytes the controller returns.

use ikbd::Ikbd;

fn drain(k: &mut Ikbd, now: u64) -> Vec<u8> {
    std::iter::from_fn(|| k.pop_output(now)).collect()
}

fn send(k: &mut Ikbd, bytes: &[u8], at: u64) {
    for &b in bytes {
        k.command_byte(b, at);
    }
}

const T: u64 = 1_000_000;

fn ready() -> Ikbd {
    let mut k = Ikbd::new();
    k.set_cycles_per_second(8_000_000);
    drain(&mut k, T);
    k
}

/// §9.2: `%00000mss`, "100 = mouse buttons act like keys": the buttons are the pseudo keys `$74` and `$75` (§4.3).
#[test]
fn button_action_keys_makes_the_buttons_keys() {
    let mut k = ready();
    send(&mut k, &[0x07, 0x04], T);
    k.mouse_button(true, true, T);
    k.mouse_button(true, false, T);
    k.mouse_button(false, true, T);
    assert_eq!(drain(&mut k, T), vec![0x74, 0xF4, 0x75]);
    k.mouse_move(3, 0, T);
    // §4.1: the button state bits are valid even when the buttons act like keys; the right button is still down.
    assert_eq!(drain(&mut k, T), vec![0xF9, 3, 0], "motion still reports");
}

/// §9.2: in absolute mode the `s` bits ask for a position report on a press (bit 0) or release (bit 1) of a button.
#[test]
fn button_action_reports_the_absolute_position_on_press_or_release() {
    let mut k = ready();
    send(&mut k, &[0x09, 0x01, 0x40, 0x00, 0xC8], T);
    k.mouse_move(5, 7, T);
    send(&mut k, &[0x07, 0x01], T);
    k.mouse_button(true, true, T);
    // Left button down since the last report is bit 2 of the buttons byte (§9.8).
    assert_eq!(drain(&mut k, T), vec![0xF7, 0x04, 0, 5, 0, 7]);
    k.mouse_button(true, false, T);
    assert_eq!(drain(&mut k, T), vec![], "only the press was asked for");
    send(&mut k, &[0x07, 0x02], T);
    k.mouse_button(false, true, T);
    assert_eq!(drain(&mut k, T), vec![], "a press is not a release");
    k.mouse_button(false, false, T);
    // Left up (bit 3) from before, right down (bit 0) and right up (bit 1) since the last report.
    assert_eq!(drain(&mut k, T), vec![0xF7, 0x0B, 0, 5, 0, 7]);
}

/// §9.3: back to relative reports; the buttons are part of the mouse record.
#[test]
fn set_relative_mouse_returns_to_relative_records() {
    let mut k = ready();
    send(&mut k, &[0x09, 0x01, 0x40, 0x00, 0xC8], T);
    k.mouse_move(5, 0, T);
    assert_eq!(
        drain(&mut k, T),
        vec![],
        "absolute mode reports nothing by itself"
    );
    send(&mut k, &[0x08], T);
    k.mouse_move(5, 0, T);
    assert_eq!(drain(&mut k, T), vec![0xF8, 5, 0]);
}

/// §9.4: "Motion below 0 is ignored ... Motion beyond that value is also ignored."
#[test]
fn absolute_mouse_clamps_at_the_maximum() {
    let mut k = ready();
    send(&mut k, &[0x09, 0x01, 0x40, 0x00, 0xC8], T); // 320 x 200
    k.mouse_move(400, 300, T);
    send(&mut k, &[0x0D], T);
    assert_eq!(drain(&mut k, T), vec![0xF7, 0, 0x01, 0x40, 0x00, 0xC8]);
    k.mouse_move(-1000, -1000, T);
    send(&mut k, &[0x0D], T);
    assert_eq!(drain(&mut k, T), vec![0xF7, 0, 0, 0, 0, 0]);
}

/// §9.4: the command resets the controller's coordinates.
#[test]
fn set_absolute_resets_the_position() {
    let mut k = ready();
    send(&mut k, &[0x09, 0x01, 0x40, 0x00, 0xC8], T);
    k.mouse_move(10, 10, T);
    send(&mut k, &[0x09, 0x01, 0x40, 0x00, 0xC8, 0x0D], T);
    assert_eq!(drain(&mut k, T), vec![0xF7, 0, 0, 0, 0, 0]);
}

/// §9.7: "the specified number of mouse phase changes must occur before the internally maintained coordinate is
/// changed by one (independently scaled for each axis)"; the rest carries over.
#[test]
fn mouse_scale_counts_clicks_per_unit() {
    let mut k = ready();
    send(&mut k, &[0x09, 0x01, 0x40, 0x00, 0xC8, 0x0C, 4, 2], T);
    k.mouse_move(7, 3, T);
    send(&mut k, &[0x0D], T);
    assert_eq!(drain(&mut k, T), vec![0xF7, 0, 0, 1, 0, 1]);
    k.mouse_move(1, 1, T); // 8 and 4 clicks in total: units 2 and 2
    send(&mut k, &[0x0D], T);
    assert_eq!(drain(&mut k, T), vec![0xF7, 0, 0, 2, 0, 2]);
}

/// §9.8: `$F7`, the buttons, X and Y as 16-bit values.
#[test]
fn interrogate_mouse_position_returns_the_record() {
    let mut k = ready();
    send(&mut k, &[0x09, 0x02, 0x00, 0x01, 0x00], T);
    k.mouse_move(0x0123, 0x0045, T);
    send(&mut k, &[0x0D], T);
    assert_eq!(drain(&mut k, T), vec![0xF7, 0, 0x01, 0x23, 0x00, 0x45]);
    // The buttons byte reports what happened since the last interrogation and then starts again.
    k.mouse_button(false, true, T);
    send(&mut k, &[0x0D, 0x0D], T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xF7, 0x01, 1, 0x23, 0, 0x45, 0xF7, 0, 1, 0x23, 0, 0x45]
    );
}

/// §9.9: `$0E`, a filler, X and Y.
#[test]
fn load_mouse_position_sets_the_coordinates() {
    let mut k = ready();
    send(&mut k, &[0x09, 0x02, 0x00, 0x01, 0x00], T);
    send(&mut k, &[0x0E, 0x00, 0x00, 0x64, 0x00, 0x32, 0x0D], T);
    assert_eq!(drain(&mut k, T), vec![0xF7, 0, 0, 0x64, 0, 0x32]);
}

/// §9.10, §9.11: the sign of Y, in relative and absolute mode.
#[test]
fn y_origin_flips_the_sign_of_y() {
    let mut k = ready();
    k.mouse_move(0, 4, T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xF8, 0, 4],
        "default: Y = 0 at the top"
    );
    send(&mut k, &[0x0F], T);
    k.mouse_move(0, 4, T);
    assert_eq!(drain(&mut k, T), vec![0xF8, 0, 0xFC]);
    send(&mut k, &[0x10], T);
    k.mouse_move(0, 4, T);
    assert_eq!(drain(&mut k, T), vec![0xF8, 0, 4]);
    send(&mut k, &[0x09, 0x01, 0x40, 0x00, 0xC8, 0x0F], T);
    k.mouse_move(0, 5, T);
    send(&mut k, &[0x0D], T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xF7, 0, 0, 0, 0, 0],
        "toward the user is negative: clamped at 0"
    );
    k.mouse_move(0, -9, T);
    send(&mut k, &[0x0D], T);
    assert_eq!(drain(&mut k, T), vec![0xF7, 0, 0, 0, 0, 9]);
}

/// §9.5: a cursor key pair (make, break) for every multiple of the distance, in each axis; the rest carries over.
#[test]
fn keycode_mode_sends_cursor_scancodes_per_delta() {
    let mut k = ready();
    send(&mut k, &[0x0A, 4, 3], T);
    k.mouse_move(10, 0, T);
    assert_eq!(drain(&mut k, T), vec![0x4D, 0xCD, 0x4D, 0xCD]);
    k.mouse_move(2, 0, T);
    assert_eq!(drain(&mut k, T), vec![0x4D, 0xCD], "2 left over + 2 = 4");
    k.mouse_move(-4, 0, T);
    assert_eq!(drain(&mut k, T), vec![0x4B, 0xCB]);
    k.mouse_move(0, -6, T);
    assert_eq!(drain(&mut k, T), vec![0x48, 0xC8, 0x48, 0xC8]);
    k.mouse_move(0, 3, T);
    assert_eq!(drain(&mut k, T), vec![0x50, 0xD0]);
}

/// §9.5: "this command is not affected by the mouse motion origin."
#[test]
fn keycode_mode_ignores_the_y_origin() {
    let mut k = ready();
    send(&mut k, &[0x0A, 1, 1, 0x0F], T);
    k.mouse_move(0, 1, T);
    assert_eq!(
        drain(&mut k, T),
        vec![0x50, 0xD0],
        "toward the user is still down"
    );
}

/// §4.3: in key code mode the buttons are `$74` (left) and `$75` (right).
#[test]
fn keycode_mode_buttons_are_keys() {
    let mut k = ready();
    send(&mut k, &[0x0A, 1, 1], T);
    k.mouse_button(true, true, T);
    k.mouse_button(true, false, T);
    k.mouse_button(false, true, T);
    assert_eq!(drain(&mut k, T), vec![0x74, 0xF4, 0x75]);
}

/// §9.6: no record until the motion reaches the threshold in an axis, and then all of it is reported.
#[test]
fn threshold_suppresses_small_moves() {
    let mut k = ready();
    send(&mut k, &[0x0B, 3, 5], T);
    k.mouse_move(2, 4, T);
    assert_eq!(drain(&mut k, T), vec![]);
    k.mouse_move(1, 0, T);
    assert_eq!(drain(&mut k, T), vec![0xF8, 3, 4], "x reached 3");
    k.mouse_move(0, 5, T);
    assert_eq!(drain(&mut k, T), vec![0xF8, 0, 5], "y reached 5");
}

/// §9.6: "This command is valid only in RELATIVE MOUSE POSITIONING mode."
#[test]
fn threshold_is_ignored_outside_relative_mode() {
    let mut k = ready();
    send(&mut k, &[0x09, 0x01, 0x40, 0x00, 0xC8, 0x0B, 9, 9, 0x8B], T);
    assert_eq!(drain(&mut k, T), vec![0xF6, 0x0B, 1, 1, 0, 0, 0, 0]);
}

/// §9.12-9.14: motion is accumulated while output is paused and goes out as the fewest packets when it resumes.
#[test]
fn motion_during_pause_goes_out_as_few_packets() {
    let mut k = ready();
    send(&mut k, &[0x13], T);
    k.mouse_move(100, 0, T);
    k.mouse_move(100, 0, T);
    k.mouse_move(100, 0, T);
    assert_eq!(drain(&mut k, T), vec![]);
    send(&mut k, &[0x11], T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xF8, 127, 0, 0xF8, 127, 0, 0xF8, 46, 0],
        "300 clicks is three packets, not one for each move"
    );
}

/// §9.14: "Pressing or releasing either mouse button causes any accumulated motion to be immediately queued."
#[test]
fn a_button_during_pause_queues_the_accumulated_motion() {
    let mut k = ready();
    send(&mut k, &[0x13], T);
    k.mouse_move(5, 0, T);
    k.mouse_move(5, 0, T);
    k.mouse_button(true, true, T);
    send(&mut k, &[0x11], T);
    assert_eq!(drain(&mut k, T), vec![0xFA, 10, 0]);
}

/// §9.13: all mouse reporting stops, and any mouse mode command resumes it.
#[test]
fn disable_mouse_stops_reports_until_a_mouse_mode_command() {
    let mut k = ready();
    send(&mut k, &[0x12], T);
    k.mouse_move(5, 5, T);
    k.mouse_button(true, true, T);
    assert_eq!(drain(&mut k, T), vec![]);
    send(&mut k, &[0x08], T);
    k.mouse_move(5, 0, T);
    // The left button went down while reporting was off and is still down (§4.1: valid button state bits).
    assert_eq!(drain(&mut k, T), vec![0xFA, 5, 0]);
}

/// §9.13: "If the mouse buttons have been commanded to act like keyboard keys, this command DOES affect their
/// actions."
#[test]
fn disable_mouse_stops_buttons_that_act_like_keys() {
    let mut k = ready();
    send(&mut k, &[0x07, 0x04, 0x12], T);
    k.mouse_button(true, true, T);
    assert_eq!(drain(&mut k, T), vec![]);
}

/// §9.27: the status of each mouse command is laid out like the command, padded to eight bytes after `$F6`.
#[test]
fn mouse_status_inquiries_answer_like_the_commands() {
    let mut k = ready();
    send(&mut k, &[0x07, 0x05, 0x87], T);
    assert_eq!(drain(&mut k, T), vec![0xF6, 0x07, 0x05, 0, 0, 0, 0, 0]);
    send(&mut k, &[0x88, 0x89, 0x8A], T);
    assert_eq!(
        drain(&mut k, T),
        [0xF6, 0x08, 0, 0, 0, 0, 0, 0].repeat(3),
        "any of the three asks for the mode"
    );
    send(&mut k, &[0x09, 0x01, 0x40, 0x00, 0xC8, 0x88], T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xF6, 0x09, 0x01, 0x40, 0x00, 0xC8, 0, 0]
    );
    send(&mut k, &[0x0A, 4, 3, 0x88], T);
    assert_eq!(drain(&mut k, T), vec![0xF6, 0x0A, 4, 3, 0, 0, 0, 0]);
    send(&mut k, &[0x08, 0x0B, 6, 7, 0x8B], T);
    assert_eq!(drain(&mut k, T), vec![0xF6, 0x0B, 6, 7, 0, 0, 0, 0]);
    send(&mut k, &[0x0C, 8, 9, 0x8C], T);
    assert_eq!(drain(&mut k, T), vec![0xF6, 0x0C, 8, 9, 0, 0, 0, 0]);
    send(&mut k, &[0x0F, 0x8F, 0x10, 0x90], T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xF6, 0x0F, 0, 0, 0, 0, 0, 0, 0xF6, 0x10, 0, 0, 0, 0, 0, 0]
    );
    send(&mut k, &[0x92, 0x12, 0x92], T);
    assert_eq!(
        drain(&mut k, T),
        vec![0xF6, 0x00, 0, 0, 0, 0, 0, 0, 0xF6, 0x12, 0, 0, 0, 0, 0, 0]
    );
}

/// §9.27: "The responses ... are designed so that the host may store them ... and later send them back as commands."
#[test]
fn a_mouse_status_sent_back_restores_the_mode() {
    let mut k = ready();
    send(&mut k, &[0x09, 0x01, 0x40, 0x00, 0xC8, 0x88], T);
    let status = drain(&mut k, T);
    send(&mut k, &[0x08], T);
    send(&mut k, &status[1..], T); // stripped of the $F6 header; the pad bytes are no-operations
    send(&mut k, &[0x88], T);
    assert_eq!(drain(&mut k, T), status);
}

/// [H?] §9.6, §9.7, §9.5: the document names "positive integers"; zero is read as one.
#[test]
fn zero_parameters_are_read_as_one() {
    let mut k = ready();
    send(&mut k, &[0x0B, 0, 0], T);
    k.mouse_move(1, 0, T);
    assert_eq!(drain(&mut k, T), vec![0xF8, 1, 0]);
    send(&mut k, &[0x0A, 0, 0], T);
    k.mouse_move(1, 0, T);
    assert_eq!(drain(&mut k, T), vec![0x4D, 0xCD]);
}
