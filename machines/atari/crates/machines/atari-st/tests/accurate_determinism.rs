//! Determinism of the `accurate` mode: the same inputs give the same machine every time, whatever else the host does
//! between steps, and a saved state resumes into exactly the straight run.

mod common;
use common::{config, emutos};

use atari_st::{Accuracy, AtariSt, RamSize};
use machine::Machine;

fn machine(rom: &[u8]) -> AtariSt {
    let mut cfg = config(rom.to_vec(), RamSize::M1);
    cfg.accuracy = Accuracy::Accurate;
    AtariSt::new(cfg).unwrap()
}

fn run_to_vbl(m: &mut AtariSt, vbls: u64) {
    while m.io_stop().is_none() && m.vbl_count() < vbls {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
}

/// What the machine is: time, PC, registers, the picture, the audio so far and RAM, hashed with FNV-1a.
fn fingerprint(m: &mut AtariSt) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    };
    feed(&m.now().to_le_bytes());
    feed(&m.cpu().pc.to_le_bytes());
    feed(&m.render_frame().unwrap().2);
    let audio = m.drain_audio();
    feed(&(audio.len() as u64).to_le_bytes());
    for s in audio {
        feed(&s.to_le_bytes());
    }
    let ram: Vec<u8> = (0..0x10_0000u32).map(|a| m.bus().peek(a)).collect();
    feed(&ram);
    h
}

#[test]
fn three_runs_of_the_desktop_give_the_same_hashes() {
    let Some(rom) = emutos() else {
        return;
    };
    let hashes: Vec<u64> = (0..3)
        .map(|_| {
            let mut m = machine(&rom);
            run_to_vbl(&mut m, 250);
            fingerprint(&mut m)
        })
        .collect();
    assert_eq!(hashes[0], hashes[1]);
    assert_eq!(hashes[1], hashes[2]);
}

#[test]
fn a_state_saved_at_frame_n_matches_a_straight_run_at_frame_n_plus_k() {
    let Some(rom) = emutos() else {
        return;
    };
    for n in [1u64, 37, 120] {
        let mut a = machine(&rom);
        run_to_vbl(&mut a, n);
        // An instruction boundary inside a line, not on a frame edge.
        for _ in 0..777 {
            a.step();
        }
        let state = a.save_state_checked().unwrap();
        let mut b = machine(&rom);
        b.load_state_checked(&state).unwrap();
        run_to_vbl(&mut a, n + 15);
        run_to_vbl(&mut b, n + 15);
        assert_eq!(
            fingerprint(&mut a),
            fingerprint(&mut b),
            "saved at frame {n}"
        );
    }
}

#[test]
fn what_the_host_does_between_steps_changes_nothing() {
    let Some(rom) = emutos() else {
        return;
    };
    let mut straight = machine(&rom);
    run_to_vbl(&mut straight, 120);
    let mut poked = machine(&rom);
    // A small deterministic pseudo-random choice of what to do between steps: nothing, render, drain audio, snapshot.
    let mut seed = 0x1234_5678u32;
    while poked.io_stop().is_none() && poked.vbl_count() < 120 {
        poked.step();
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        match seed >> 28 {
            0 => {
                poked.render_frame();
            }
            1 => {
                poked.drain_audio();
            }
            2 => {
                poked.snapshot().unwrap();
            }
            _ => {}
        }
    }
    // The audio was drained on one side only: compare without it.
    let strip = |m: &mut AtariSt| {
        let (now, pc) = (m.now(), m.cpu().pc);
        let ram: Vec<u8> = (0..0x10_0000u32).map(|a| m.bus().peek(a)).collect();
        (now, pc, m.render_frame().unwrap().2, ram)
    };
    assert_eq!(strip(&mut straight), strip(&mut poked));
}
