//! Phase 4 on the new models: runs are deterministic, a state taken mid-run continues exactly as the run would have, and
//! hostile register writes and states never panic (`RC_SWEEP=<n>` multiplies the loops). EmuTOS tests need `RC_EMUTOS`
//! (192K, the Mega ST) and the 256K image in the cache (the STE); without them they pass and say so.

mod common;
use common::{config, emutos, emutos_ste};

use atari_st::{Accuracy, AtariSt, Chipset, RamSize};
use machine::Machine;

fn sweep() -> u64 {
    std::env::var("RC_SWEEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

fn boot(chipset: Chipset, accuracy: Accuracy) -> Option<AtariSt> {
    let rom = if chipset == Chipset::Ste {
        emutos_ste()?
    } else {
        emutos()?
    };
    let mut cfg = config(rom, RamSize::M1);
    cfg.accuracy = accuracy;
    Some(AtariSt::with_chipset(cfg, chipset).unwrap())
}

fn run_to(m: &mut AtariSt, vbl: u64) {
    while m.vbl_count() < vbl && m.io_stop().is_none() {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
}

#[test]
fn a_run_is_deterministic_and_a_state_continues_it_exactly_on_each_model() {
    for chipset in [Chipset::MegaSt, Chipset::Ste] {
        for accuracy in [Accuracy::Fast, Accuracy::Accurate] {
            let (Some(mut a), Some(mut b)) = (boot(chipset, accuracy), boot(chipset, accuracy))
            else {
                return;
            };
            run_to(&mut a, 120);
            run_to(&mut b, 120);
            assert_eq!(
                a.save_state_checked().unwrap(),
                b.save_state_checked().unwrap(),
                "{chipset:?} {accuracy:?}: two runs"
            );
            let state = a.save_state_checked().unwrap();
            let mut c = boot(chipset, accuracy).unwrap();
            c.load_state_checked(&state).unwrap();
            run_to(&mut a, 160);
            run_to(&mut c, 160);
            assert_eq!(
                a.save_state_checked().unwrap(),
                c.save_state_checked().unwrap(),
                "{chipset:?} {accuracy:?}: continued"
            );
            assert_eq!(a.render_frame(), c.render_frame());
        }
    }
}

#[test]
fn flipped_bytes_in_a_state_are_refused_or_load_and_run_without_a_panic() {
    for chipset in [Chipset::MegaSt, Chipset::Ste] {
        let Some(mut m) = boot(chipset, Accuracy::Fast) else {
            return;
        };
        run_to(&mut m, 60);
        let state = m.save_state_checked().unwrap();
        let mut seed = 0x1234_5678u32;
        let n = 40 * sweep();
        for _ in 0..n {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let mut bad = state.clone();
            // bytes near the end are the chipset part; spread the rest over the whole state
            let at = if seed & 1 == 0 {
                bad.len() - 1 - (seed >> 8) as usize % 4000.min(bad.len() - 1)
            } else {
                (seed >> 8) as usize % bad.len()
            };
            bad[at] ^= 1 << (seed >> 24 & 7);
            let mut t = boot(chipset, Accuracy::Fast).unwrap();
            if t.load_state_checked(&bad).is_ok() {
                for _ in 0..2_000 {
                    t.step();
                }
            }
        }
    }
}

fn ste_with(program: &[u16]) -> Vec<u8> {
    let mut rom = vec![0u8; 262_144];
    rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00E0_0010u32.to_be_bytes());
    for (i, w) in program.iter().enumerate() {
        rom[0x10 + 2 * i..0x12 + 2 * i].copy_from_slice(&w.to_be_bytes());
    }
    rom
}

/// A program that writes pseudo-random words at pseudo-random even offsets (0 to $3E) from `base` for ever.
fn scribbler(base: u32) -> Vec<u16> {
    vec![
        0x207C,
        (base >> 16) as u16,
        base as u16, // movea.l #base,a0
        0x7001,      // moveq #1,d0
        0xC0FC,
        25173, // loop: mulu.w #25173,d0
        0x0640,
        13849,  // add.w #13849,d0
        0x3200, // move.w d0,d1
        0x0241,
        0x003E, // and.w #$3E,d1
        0x3180,
        0x1000, // move.w d0,0(a0,d1.w)
        0x60EC, // bra.s loop
    ]
}

#[test]
fn random_writes_to_the_blitter_and_the_dma_sound_never_panic_and_stay_bounded() {
    for base in [0xFF_8A00u32, 0xFF_8900] {
        for accuracy in [Accuracy::Fast, Accuracy::Accurate] {
            let mut cfg = config(ste_with(&scribbler(base)), RamSize::M1);
            cfg.accuracy = accuracy;
            let mut m = AtariSt::with_chipset(cfg, Chipset::Ste).unwrap();
            let t = std::time::Instant::now();
            for _ in 0..(30_000 * sweep()) {
                m.step();
            }
            assert!(
                t.elapsed().as_secs() < 60,
                "{base:x} {accuracy:?}: {:?}",
                t.elapsed()
            );
            let _ = m.drain_audio();
            // A machine that stopped (a blit into the I/O area) has no state to save; the others round-trip.
            if let Ok(state) = m.save_state_checked() {
                let mut n = AtariSt::with_chipset(
                    config(ste_with(&scribbler(base)), RamSize::M1),
                    Chipset::Ste,
                )
                .unwrap();
                n.load_state_checked(&state).unwrap();
            }
        }
    }
}
