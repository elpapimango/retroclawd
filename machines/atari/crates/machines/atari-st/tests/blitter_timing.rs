//! What a running Blitter costs the CPU on the machine: HOG against a shared bus, the interrupt at the end, and the bound on
//! a hostile blit. (Hatari's loop counts are compared in `timing_probes.rs`, P34.)

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

fn mb(imm: u8, addr: u32) -> [u16; 4] {
    [0x13FC, u16::from(imm), (addr >> 16) as u16, addr as u16]
}

fn mw(imm: u16, addr: u32) -> [u16; 4] {
    [0x33FC, imm, (addr >> 16) as u16, addr as u16]
}

const REG: u32 = 0xFF_8A00;

/// Programs a copy of `words` words (HOP 2, OP 3: two NOPs a word, 8 clocks) and starts it with `control`.
fn blit_program(words: u16, control: u8, tail: &[u16]) -> Vec<u16> {
    blit_lines(words, 1, control, tail, 2)
}

/// The same with `lines` lines (0 means 65536) and the increment `inc` for both sides.
fn blit_lines(words: u16, lines: u16, control: u8, tail: &[u16], inc: u16) -> Vec<u16> {
    let mut p = Vec::new();
    p.extend(mw(inc, REG + 0x20));
    p.extend(mw(inc, REG + 0x22));
    p.extend(mw(0, REG + 0x24));
    p.extend(mw(0x1000, REG + 0x26));
    for m in [0x28, 0x2A, 0x2C] {
        p.extend(mw(0xFFFF, REG + m));
    }
    p.extend(mw(inc, REG + 0x2E));
    p.extend(mw(inc, REG + 0x30));
    p.extend(mw(0, REG + 0x32));
    p.extend(mw(0x8000, REG + 0x34));
    p.extend(mw(words, REG + 0x36));
    p.extend(mw(lines, REG + 0x38));
    p.extend(mb(2, REG + 0x3A));
    p.extend(mb(3, REG + 0x3B));
    p.extend(mb(0, REG + 0x3D));
    p.extend(tail);
    p.extend(mb(control, REG + 0x3C));
    p.push(0x60FE);
    p
}

fn machine(program: &[u16], accuracy: Accuracy) -> AtariSt {
    let mut cfg = config(ste_with(program), RamSize::M1);
    cfg.accuracy = accuracy;
    AtariSt::with_chipset(cfg, Chipset::Ste).unwrap()
}

/// Steps until the blit has started (the busy bit or the start write seen) and returns the clock at its start.
fn run_to_start(m: &mut AtariSt) -> u64 {
    let before = m.now();
    for _ in 0..400 {
        m.step();
        if m.blitter_busy() {
            return m.now();
        }
        if m.now() - before > 100_000 {
            break;
        }
    }
    m.now()
}

#[test]
fn a_blit_that_hogs_the_bus_stops_the_cpu_for_its_words_at_two_nops_each() {
    for accuracy in [Accuracy::Fast, Accuracy::Accurate] {
        let mut m = machine(&blit_program(1000, 0xC0, &[]), accuracy);
        let mut last = m.now();
        // find the instruction that starts the blit: the one that takes thousands of clocks
        let mut stall = 0;
        for _ in 0..200 {
            m.step();
            let took = m.now() - last;
            last = m.now();
            if took > 1000 {
                stall = took;
                break;
            }
        }
        // 1000 words of 8 clocks, and the write itself
        assert!((8_000..8_100).contains(&stall), "{accuracy:?}: {stall}");
        assert!(!m.blitter_busy());
    }
}

#[test]
fn a_blit_that_shares_the_bus_takes_162_clocks_for_each_64_it_works() {
    for accuracy in [Accuracy::Fast, Accuracy::Accurate] {
        let mut m = machine(&blit_program(1000, 0x80, &[]), accuracy);
        let start = run_to_start(&mut m);
        assert!(m.blitter_busy());
        while m.blitter_busy() && m.now() - start < 200_000 {
            m.step();
        }
        let took = m.now() - start;
        // 8000 clocks of words in bursts of 64 with 2 to hand over and 96 of CPU between: 125 bursts of 162
        assert!((19_500..21_500).contains(&took), "{accuracy:?}: {took}");
    }
}

#[test]
fn the_blitter_done_interrupt_comes_once_when_the_blit_ends() {
    let handler = 0x00E0_0100u32;
    let mut setup: Vec<u16> = vec![0x23FC, (handler >> 16) as u16, handler as u16, 0, 0x10C];
    setup.extend(mb(0x40, 0xFF_FA17));
    setup.extend(mb(0x08, 0xFF_FA09));
    setup.extend(mb(0x08, 0xFF_FA15));
    setup.extend([0x46FC, 0x2300]);
    let mut p = blit_program(300, 0x80, &setup);
    while p.len() < (0x100 - 0x10) / 2 {
        p.push(0x4E71);
    }
    // the handler counts and returns
    p.extend([0x5279, 0, 0x1100, 0x4E73]);
    let mut m = machine(&p, Accuracy::Accurate);
    for _ in 0..4_000 {
        m.step();
    }
    assert!(!m.blitter_busy());
    assert_eq!(m.read_memory(0x1101), 1, "the handler ran once");
}

#[test]
fn a_hostile_blit_costs_bounded_host_time_and_the_machine_keeps_running() {
    // An X count and a Y count of zero are 2^32 words. Over one word the whole time (increments 0) it never leaves memory:
    // a hogging blit and a sharing one each take what a frame allows and the machine runs on.
    for control in [0xC0, 0x80] {
        let p = blit_lines(0, 0, control, &[], 0);
        let mut m = machine(&p, Accuracy::Fast);
        let t = std::time::Instant::now();
        for _ in 0..3_000 {
            m.step();
        }
        assert!(t.elapsed().as_secs() < 10, "{:?}", t.elapsed());
        assert!(m.io_stop().is_none());
        assert!(m.blitter_busy());
    }
    // Walking on by two bytes a word it reaches the end of the address space it may touch and the machine stops there with a
    // report instead of running on.
    let mut m = machine(&blit_lines(0, 0, 0xC0, &[], 2), Accuracy::Fast);
    let t = std::time::Instant::now();
    for _ in 0..3_000 {
        m.step();
    }
    assert!(t.elapsed().as_secs() < 10, "{:?}", t.elapsed());
    assert!(m.io_stop().is_some());
}
