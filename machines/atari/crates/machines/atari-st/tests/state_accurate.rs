//! Save states of an `accurate` machine: time passes inside the bus, so the state carries the bus time, the pending
//! VBL request and the wait-state bookkeeping that follow from it; and a state belongs to the mode that saved it.

mod common;
use common::{config, emutos};

use atari_st::{Accuracy, AtariSt, RamSize};
use machine::Machine;

fn machine(rom: &[u8], accuracy: Accuracy) -> AtariSt {
    let mut cfg = config(rom.to_vec(), RamSize::M1);
    cfg.accuracy = accuracy;
    AtariSt::new(cfg).unwrap()
}

fn run_to_vbl(m: &mut AtariSt, vbls: u64) {
    while m.io_stop().is_none() && m.vbl_count() < vbls {
        m.step();
    }
}

fn fingerprint(m: &AtariSt) -> (u64, u32, Vec<u8>, Vec<u8>) {
    let ram: Vec<u8> = (0..0x10_0000u32).map(|a| m.bus().peek(a)).collect();
    (m.now(), m.cpu().pc, m.render_frame().unwrap().2, ram)
}

#[test]
fn an_accurate_state_saved_mid_frame_continues_like_a_straight_run() {
    let Some(rom) = emutos() else {
        return;
    };
    for stop_after in [37u64, 200] {
        let mut a = machine(&rom, Accuracy::Accurate);
        run_to_vbl(&mut a, stop_after);
        // Part way through a line: some instructions into the frame.
        for _ in 0..1234 {
            a.step();
        }
        let state = a.save_state_checked().unwrap();
        let mut straight = machine(&rom, Accuracy::Accurate);
        straight.load_state_checked(&state).unwrap();
        run_to_vbl(&mut a, stop_after + 20);
        run_to_vbl(&mut straight, stop_after + 20);
        assert_eq!(
            fingerprint(&a),
            fingerprint(&straight),
            "after {stop_after} frames"
        );
    }
}

#[test]
fn a_state_taken_between_the_end_of_a_frame_and_the_vbl_keeps_the_vbl_pending() {
    let Some(rom) = emutos() else {
        return;
    };
    let mut a = machine(&rom, Accuracy::Accurate);
    run_to_vbl(&mut a, 100);
    // Step until the frame has ended but the VBL request (64 clocks into the next frame) has not come yet.
    let frames = a.vbl_count();
    let mut state = None;
    for _ in 0..200_000 {
        a.step();
        if a.vbl_count() > frames && a.now() < a.last_vbl_at() + 60 {
            state = Some(a.save_state_checked().unwrap());
            break;
        }
    }
    let state =
        state.expect("the window between the frame end and the VBL request is 64 clocks wide");
    let mut b = machine(&rom, Accuracy::Accurate);
    b.load_state_checked(&state).unwrap();
    for _ in 0..30_000 {
        a.step();
        b.step();
    }
    assert_eq!(fingerprint(&a), fingerprint(&b));
}

#[test]
fn a_state_does_not_load_into_the_other_mode() {
    let Some(rom) = emutos() else {
        return;
    };
    let mut fast = machine(&rom, Accuracy::Fast);
    let mut accurate = machine(&rom, Accuracy::Accurate);
    run_to_vbl(&mut fast, 10);
    run_to_vbl(&mut accurate, 10);
    let from_fast = fast.save_state_checked().unwrap();
    let from_accurate = accurate.save_state_checked().unwrap();
    let e = accurate
        .load_state_checked(&from_fast)
        .unwrap_err()
        .to_string();
    assert!(e.contains("fast") && e.contains("accurate"), "{e}");
    let e = fast
        .load_state_checked(&from_accurate)
        .unwrap_err()
        .to_string();
    assert!(e.contains("fast") && e.contains("accurate"), "{e}");
}

#[test]
fn mutated_accurate_states_are_refused_or_load_without_a_panic() {
    let Some(rom) = emutos() else {
        return;
    };
    let mut a = machine(&rom, Accuracy::Accurate);
    run_to_vbl(&mut a, 5);
    let state = a.save_state_checked().unwrap();
    let sweep: usize = std::env::var("RC_SWEEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let mut seed = 0x1234_5678_9ABC_DEF0u64;
    for _ in 0..(60 * sweep) {
        let mut bad = state.clone();
        for _ in 0..3 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let at = (seed >> 33) as usize % bad.len();
            bad[at] ^= 1 << ((seed >> 20) & 7);
        }
        let mut b = machine(&rom, Accuracy::Accurate);
        if b.load_state_checked(&bad).is_ok() {
            for _ in 0..2_000 {
                b.step();
            }
        }
    }
}
