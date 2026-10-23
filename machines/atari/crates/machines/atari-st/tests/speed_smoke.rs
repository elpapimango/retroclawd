//! A loose guard on speed: `fast` runs the idle EmuTOS desktop at least 10 times real time and `accurate` at least twice (the
//! budget of the Phase 3 plan, in release on the owner's machine). The assertion only runs with `RC_BENCH=1`; otherwise the
//! test checks that both modes finish the same frames without a stop and says so.

mod common;
use common::{config, emutos};

use atari_st::{Accuracy, AtariSt, RamSize};
use machine::Machine;
use std::time::Instant;

/// Frames a second of the desktop between frames 600 and 800.
fn frames_per_second(rom: &[u8], accuracy: Accuracy) -> f64 {
    let mut cfg = config(rom.to_vec(), RamSize::M1);
    cfg.accuracy = accuracy;
    cfg.trace_limit = 0;
    cfg.io_log_limit = 0;
    let mut m = AtariSt::new(cfg).unwrap();
    while m.vbl_count() < 600 && m.io_stop().is_none() {
        m.step();
    }
    let started = Instant::now();
    while m.vbl_count() < 800 && m.io_stop().is_none() {
        m.step();
    }
    assert!(
        m.io_stop().is_none(),
        "{accuracy:?} stopped: {:?}",
        m.io_stop()
    );
    200.0 / started.elapsed().as_secs_f64()
}

#[test]
fn accurate_desktop_runs_at_the_budget() {
    let Some(rom) = emutos() else {
        return;
    };
    let fast = frames_per_second(&rom, Accuracy::Fast);
    let accurate = frames_per_second(&rom, Accuracy::Accurate);
    eprintln!(
        "fast {fast:.0} frames/s = {:.1}x real time, accurate {accurate:.0} frames/s = {:.1}x",
        fast / 50.0,
        accurate / 50.0
    );
    if std::env::var_os("RC_BENCH").is_some() {
        assert!(
            fast >= 500.0,
            "fast: {fast:.0} frames/s is under 10x real time"
        );
        assert!(
            accurate >= 100.0,
            "accurate: {accurate:.0} frames/s is under 2x real time"
        );
    } else {
        eprintln!("RC_BENCH is not set: the budget is not asserted");
    }
}
