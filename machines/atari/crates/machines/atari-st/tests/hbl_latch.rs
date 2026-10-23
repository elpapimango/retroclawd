//! An HBL that came while the CPU's mask was too high stays pending and is taken the moment the mask comes down (P14's first interval).

mod common;
use common::config;

use atari_st::{Accuracy, AtariSt, Chipset, RamSize};
use machine::Machine;

fn ste_with(program: &[u16]) -> Vec<u8> {
    let mut rom = vec![0u8; 262_144];
    rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00E0_0010u32.to_be_bytes());
    for (i, w) in program.iter().enumerate() {
        rom[0x10 + 2 * i..0x12 + 2 * i].copy_from_slice(&w.to_be_bytes());
    }
    rom
}

#[test]
fn a_pending_hbl_is_taken_at_the_rte_that_lowers_the_mask() {
    // vectors, then SR $2300 and a loop; the VBL handler lowers the mask in the stacked SR; the HBL handler counts.
    let base = 0x00E0_0010u32;
    let hbl = base + 0x40;
    let vbl = base + 0x50;
    let mut p: Vec<u16> = vec![];
    let long = |a: u32| [(a >> 16) as u16, a as u16];
    p.extend([0x23FC]);
    p.extend(long(hbl));
    p.extend([0, 0x68]);
    p.extend([0x23FC]);
    p.extend(long(vbl));
    p.extend([0, 0x70]);
    p.extend([0x46FC, 0x2300]); // move.w #$2300,sr
    p.extend([0x60FE]); // bra.s *
    while p.len() < 0x20 {
        p.push(0x4E71);
    }
    // hbl at +0x40: addq.w #1,$1002 ; rte
    p.extend([0x5279, 0, 0x1002, 0x4E73]);
    while p.len() < 0x28 {
        p.push(0x4E71);
    }
    // vbl at +0x50: move.w #$2100,(sp) ; rte
    p.extend([0x3EBC, 0x2100, 0x4E73]);
    for accuracy in [Accuracy::Fast, Accuracy::Accurate] {
        let mut cfg = config(ste_with(&p), RamSize::M1);
        cfg.accuracy = accuracy;
        let mut m = AtariSt::with_chipset(cfg, Chipset::Ste).unwrap();
        while m.vbl_count() < 2 {
            m.step();
        }
        // run the VBL handler (taken within a few instructions) and a few more
        let before = u16::from_be_bytes([m.read_memory(0x1002), m.read_memory(0x1003)]);
        for _ in 0..40 {
            m.step();
        }
        let after = u16::from_be_bytes([m.read_memory(0x1002), m.read_memory(0x1003)]);
        assert!(
            after > before,
            "{accuracy:?}: the pending HBL was not taken after the mask came down ({before} to {after})"
        );
    }
}
