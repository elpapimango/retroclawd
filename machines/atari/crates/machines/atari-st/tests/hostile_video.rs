//! A guest that hammers the video registers: every write is bounded (the logs have caps), the machine never panics or
//! stops, and a state mutated at random is refused or loads cleanly. `RC_SWEEP=<n>` multiplies the random loops.

mod common;
use common::{config, move_b, rom};

use atari_st::{Accuracy, AtariSt, RamSize};
use machine::Machine;
use std::time::Instant;

/// `MOVE.W #value,$addr.W`.
fn move_w(value: u16, addr: u16) -> [u16; 3] {
    [0x31FC, value, addr]
}

/// `JMP $FC0010.L`: back to the start of the program.
const JMP_START: [u16; 3] = [0x4EF9, 0x00FC, 0x0010];

fn machine(program: &[u16], accuracy: Accuracy) -> AtariSt {
    let mut cfg = config(rom(program, &[]), RamSize::M1);
    cfg.accuracy = accuracy;
    AtariSt::new(cfg).unwrap()
}

fn run_frames(m: &mut AtariSt, frames: u64) {
    let target = m.vbl_count() + frames;
    while m.io_stop().is_none() && m.vbl_count() < target {
        m.step();
    }
}

fn sweep() -> u64 {
    std::env::var("RC_SWEEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

/// A seeded generator: xorshift32.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
}

#[test]
fn a_write_to_every_shifter_register_on_every_cycle_of_a_frame_is_bounded() {
    // Palette (16 words), the sync mode, the resolution and the video base bytes, over and over without a pause.
    let mut program = Vec::new();
    for i in 0..16u16 {
        program.extend(move_w(0x0777 ^ (i * 0x111), 0x8240 + 2 * i));
    }
    for (value, addr) in [
        (0u8, 0x820A),
        (2, 0x820A),
        (2, 0x8260),
        (0, 0x8260),
        (1, 0x8260),
        (0, 0x8201),
        (0, 0x8203),
    ] {
        program.extend(move_b(value, addr));
    }
    program.extend(JMP_START);
    for accuracy in [Accuracy::Fast, Accuracy::Accurate] {
        let mut m = machine(&program, accuracy);
        let started = Instant::now();
        run_frames(&mut m, 5);
        let elapsed = started.elapsed();
        assert!(m.io_stop().is_none(), "{accuracy:?}: {:?}", m.io_stop());
        let shifter = m.bus().shifter();
        assert!(
            shifter.palette_changes().len() <= st_shifter::MAX_PALETTE_CHANGES,
            "{accuracy:?}: the palette log is capped"
        );
        assert!(
            shifter.sync_log().count() <= st_shifter::MAX_SYNC_LOG,
            "{accuracy:?}: the sync log is capped"
        );
        let canvas = m.render_frame().unwrap();
        assert_eq!(canvas.2.len(), (canvas.0 * canvas.1 * 3) as usize);
        assert!(
            elapsed.as_secs() < 60,
            "{accuracy:?}: five frames took {elapsed:?}"
        );
    }
}

#[test]
fn random_sync_and_resolution_sequences_never_panic_or_stop_the_machine() {
    for seed in 1..=4 * sweep() as u32 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9) | 1);
        let mut program = Vec::new();
        for _ in 0..300 {
            let r = rng.next();
            match r % 5 {
                0 => program.extend(move_b((r >> 8) as u8 & 3, 0x820A)),
                1 => program.extend(move_b((r >> 8) as u8 & 3, 0x8260)),
                2 => program.extend(move_w(
                    (r >> 8) as u16 & 0x777,
                    0x8240 + 2 * ((r >> 4) as u16 & 15),
                )),
                3 => program.extend(move_b((r >> 8) as u8, 0x8201 + 2 * ((r >> 4) as u16 & 1))),
                _ => program.extend([0x4E71, 0x4E71]),
            }
        }
        program.extend(JMP_START);
        for accuracy in [Accuracy::Fast, Accuracy::Accurate] {
            let mut m = machine(&program, accuracy);
            run_frames(&mut m, 4);
            assert!(
                m.io_stop().is_none(),
                "seed {seed} {accuracy:?}: {:?}",
                m.io_stop()
            );
            let (w, h, rgb) = m.render_frame().unwrap();
            assert_eq!(rgb.len(), (w * h * 3) as usize);
        }
    }
}

#[test]
fn mutated_accurate_states_are_refused_or_load_cleanly() {
    let program = [move_b(2, 0x820A), move_w(0x0700, 0x8240), [0x60FE, 0, 0]].concat();
    let mut a = machine(&program, Accuracy::Accurate);
    run_frames(&mut a, 3);
    let state = a.save_state_checked().unwrap();
    let mut rng = Rng(0xC0FF_EE11);
    let mut refused = 0;
    for _ in 0..200 * sweep() {
        let mut bad = state.clone();
        for _ in 0..1 + rng.next() % 4 {
            let at = rng.next() as usize % bad.len();
            bad[at] ^= 1 << (rng.next() % 8);
        }
        let mut b = machine(&program, Accuracy::Accurate);
        match b.load_state_checked(&bad) {
            Err(_) => refused += 1,
            Ok(()) => {
                run_frames(&mut b, 2);
                let _ = b.render_frame();
            }
        }
    }
    assert!(
        refused > 0,
        "a mutated state is refused now and then (the CRC)"
    );
}
