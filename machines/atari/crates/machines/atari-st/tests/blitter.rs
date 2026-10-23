//! The Blitter on the machine's bus: registers reached by the CPU, memory it may and may not touch, a long blit across
//! frames and its state. (The engine's rules are in `crates/chips/blitter/tests` and the comparison with Hatari's dumps in
//! `timing_probes.rs`.)

mod common;
use common::config;

use atari_st::{AtariSt, Chipset, RamSize};
use machine::Machine;

/// A 256 KB STE image whose reset code is `program` at `$E00010`.
fn ste_with(program: &[u16]) -> Vec<u8> {
    let mut rom = vec![0u8; 262_144];
    rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00E0_0010u32.to_be_bytes());
    for (i, w) in program.iter().enumerate() {
        rom[0x10 + 2 * i..0x12 + 2 * i].copy_from_slice(&w.to_be_bytes());
    }
    rom
}

/// `MOVE.W #imm,addr.L`
fn mw(imm: u16, addr: u32) -> [u16; 4] {
    [0x33FC, imm, (addr >> 16) as u16, addr as u16]
}

/// `MOVE.B #imm,addr.L`
fn mb(imm: u8, addr: u32) -> [u16; 4] {
    [0x13FC, u16::from(imm), (addr >> 16) as u16, addr as u16]
}

const REG: u32 = 0xFF_8A00;

/// Registers for a copy of `x` words, `y` lines from `src` to `dst` with HOP 2, OP 3, then the start and a loop.
fn copy_program(src: u32, dst: u32, x: u16, y: u16, data: &[u16]) -> Vec<u16> {
    copy_program_with(src, dst, x, y, data, [2, 2, 2, 2])
}

/// The same with the increments `[source x, source y, destination x, destination y]`.
fn copy_program_with(src: u32, dst: u32, x: u16, y: u16, data: &[u16], inc: [u16; 4]) -> Vec<u16> {
    let mut p = Vec::new();
    for (i, w) in data.iter().enumerate() {
        p.extend(mw(*w, src + 2 * i as u32));
    }
    p.extend(mw(inc[0], REG + 0x20));
    p.extend(mw(inc[1], REG + 0x22));
    p.extend(mw((src >> 16) as u16, REG + 0x24));
    p.extend(mw(src as u16, REG + 0x26));
    for m in [0x28, 0x2A, 0x2C] {
        p.extend(mw(0xFFFF, REG + m));
    }
    p.extend(mw(inc[2], REG + 0x2E));
    p.extend(mw(inc[3], REG + 0x30));
    p.extend(mw((dst >> 16) as u16, REG + 0x32));
    p.extend(mw(dst as u16, REG + 0x34));
    p.extend(mw(x, REG + 0x36));
    p.extend(mw(y, REG + 0x38));
    p.extend(mb(2, REG + 0x3A));
    p.extend(mb(3, REG + 0x3B));
    p.extend(mb(0, REG + 0x3D));
    p.extend(mb(0x80, REG + 0x3C));
    p.push(0x60FE);
    p
}

fn run(m: &mut AtariSt, steps: usize) {
    for _ in 0..steps {
        if m.io_stop().is_some() {
            break;
        }
        m.step();
    }
}

fn words(m: &AtariSt, addr: u64, n: u64) -> Vec<u16> {
    (0..n)
        .map(|i| u16::from_be_bytes([m.read_memory(addr + 2 * i), m.read_memory(addr + 2 * i + 1)]))
        .collect()
}

#[test]
fn the_cpu_starts_a_blit_and_finds_the_words_moved() {
    let prog = copy_program(0x1000, 0x2000, 3, 1, &[0x1111, 0x2222, 0x3333]);
    let mut m = AtariSt::with_chipset(config(ste_with(&prog), RamSize::M1), Chipset::Ste).unwrap();
    run(&mut m, 200);
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
    assert_eq!(words(&m, 0x2000, 3), [0x1111, 0x2222, 0x3333]);
}

#[test]
fn a_mega_st_has_the_blitter_and_a_plain_st_has_none() {
    let prog = copy_program(0x1000, 0x2000, 1, 1, &[0xBEEF]);
    let mut image = vec![0u8; 196_608];
    image[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    image[4..8].copy_from_slice(&0x00FC_0010u32.to_be_bytes());
    for (i, w) in prog.iter().enumerate() {
        image[0x10 + 2 * i..0x12 + 2 * i].copy_from_slice(&w.to_be_bytes());
    }
    let mut mega =
        AtariSt::with_chipset(config(image.clone(), RamSize::M1), Chipset::MegaSt).unwrap();
    run(&mut mega, 200);
    assert_eq!(words(&mega, 0x2000, 1), [0xBEEF]);
    // On a plain ST `$FF8A00` has no fact: the first register write stops the machine and nothing moves.
    let mut st = AtariSt::with_chipset(config(image, RamSize::M1), Chipset::St).unwrap();
    run(&mut st, 200);
    assert!(st.io_stop().is_some());
    assert_eq!(words(&st, 0x2000, 1), [0]);
}

#[test]
fn a_blit_into_the_io_area_stops_the_machine_like_a_dma_there() {
    let prog = copy_program(0x1000, 0xFF_8240, 1, 1, &[0x0123]);
    let mut m = AtariSt::with_chipset(config(ste_with(&prog), RamSize::M1), Chipset::Ste).unwrap();
    run(&mut m, 200);
    let stop = m.io_stop().expect("the Blitter wrote the palette");
    assert_eq!(stop.addr, 0xFF_8240);
}

#[test]
fn a_blit_from_unfitted_memory_reads_ff_and_one_past_the_decode_stops() {
    // 1 MB fitted: the area between 1 MB and 4 MB reads as open bus; $400000 and up has no fact.
    let prog = copy_program(0x20_0000, 0x2000, 1, 1, &[]);
    let mut m = AtariSt::with_chipset(config(ste_with(&prog), RamSize::M1), Chipset::Ste).unwrap();
    run(&mut m, 200);
    assert!(m.io_stop().is_none());
    assert_eq!(words(&m, 0x2000, 1), [0xFFFF]);
    let prog = copy_program(0x40_0000, 0x2000, 1, 1, &[]);
    let mut m = AtariSt::with_chipset(config(ste_with(&prog), RamSize::M1), Chipset::Ste).unwrap();
    run(&mut m, 200);
    assert!(m.io_stop().is_some());
}

#[test]
fn a_blit_longer_than_a_burst_goes_on_over_the_frames_and_its_state_round_trips() {
    // X and Y counts of zero: 2^32 words, written one burst at a time, once at the start and once a frame.
    // The increments are zero: the same word over and over, so no run of the blit leaves memory.
    let prog = copy_program_with(0x1000, 0x2000, 0, 0, &[0x7777], [0; 4]);
    let mut m = AtariSt::with_chipset(config(ste_with(&prog), RamSize::M1), Chipset::Ste).unwrap();
    run(&mut m, 200);
    assert!(m.io_stop().is_none());
    while m.vbl_count() < 3 {
        m.step();
    }
    assert!(m.io_stop().is_none());
    let state = m.save_state_checked().unwrap();
    let mut b = AtariSt::with_chipset(config(ste_with(&prog), RamSize::M1), Chipset::Ste).unwrap();
    b.load_state_checked(&state).unwrap();
    assert_eq!(b.save_state_checked().unwrap(), state);
    // and it is still going in the restored machine
    while b.vbl_count() < 5 && b.io_stop().is_none() {
        b.step();
    }
    assert!(b.io_stop().is_none());
}
