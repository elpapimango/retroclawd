//! The STE's enhanced joystick ports: what they read at rest and with pad buttons held, and the addresses that bus-error.
//! (Hatari's idle reads are compared in `timing_probes.rs`, P31.)

mod common;
use common::config;

use atari_st::ste::PadButton;
use atari_st::{AtariSt, Chipset, RamSize};
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

/// `MOVE.W abs.L,abs.L`
fn mw(src: u32, dst: u32) -> [u16; 5] {
    [
        0x33F9,
        (src >> 16) as u16,
        src as u16,
        (dst >> 16) as u16,
        dst as u16,
    ]
}

/// `MOVE.B abs.L,abs.L`
fn mb(src: u32, dst: u32) -> [u16; 5] {
    [
        0x13F9,
        (src >> 16) as u16,
        src as u16,
        (dst >> 16) as u16,
        dst as u16,
    ]
}

fn run(m: &mut AtariSt, steps: usize) {
    for _ in 0..steps {
        m.step();
    }
}

fn word(m: &AtariSt, addr: u64) -> u16 {
    u16::from_be_bytes([m.read_memory(addr), m.read_memory(addr + 1)])
}

fn reader() -> Vec<u16> {
    let mut p = Vec::new();
    p.extend(mw(0xFF_9200, 0x1000));
    p.extend(mw(0xFF_9202, 0x1002));
    p.extend(mb(0xFF_9201, 0x1004));
    p.extend(mb(0xFF_9211, 0x1005));
    p.extend(mw(0xFF_9210, 0x1006));
    p.extend(mw(0xFF_9220, 0x1008));
    p.push(0x60FE);
    p
}

fn machine(program: &[u16]) -> AtariSt {
    AtariSt::with_chipset(config(ste_with(program), RamSize::M1), Chipset::Ste).unwrap()
}

#[test]
fn at_rest_the_ports_read_as_hatari_shows() {
    let mut m = machine(&reader());
    run(&mut m, 100);
    assert_eq!(word(&m, 0x1000), 0xFFFF);
    assert_eq!(word(&m, 0x1002), 0xFFFF);
    assert_eq!(m.read_memory(0x1004), 0xFF);
    assert_eq!(m.read_memory(0x1005), 0x24);
    assert_eq!(word(&m, 0x1006), 0xFF24);
    assert_eq!(word(&m, 0x1008), 0);
}

#[test]
fn a_held_pad_button_clears_its_bit() {
    let mut m = machine(&reader());
    m.pad_button(0, PadButton::Up, true).unwrap();
    m.pad_button(0, PadButton::Right, true).unwrap();
    m.pad_button(1, PadButton::Left, true).unwrap();
    m.pad_button(0, PadButton::A, true).unwrap();
    m.pad_button(1, PadButton::Pause, true).unwrap();
    run(&mut m, 100);
    // up (bit 0) and right (bit 3) of pad 1, left (bit 2 of the upper nibble) of pad 2
    assert_eq!(word(&m, 0x1002), !(0x01u16 | 0x08 | 0x40));
    assert_eq!(m.read_memory(0x1004), !0x01u8, "pad 1 A");
    assert_eq!(word(&m, 0x1000) >> 8, !0x40u16 & 0xFF, "pad 2 pause");
    m.pad_button(0, PadButton::Up, false).unwrap();
    run(&mut m, 100);
}

#[test]
fn the_pad_keys_exist_only_on_an_ste() {
    let mut st_rom = vec![0u8; 196_608];
    st_rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    st_rom[4..8].copy_from_slice(&0x00FC_0010u32.to_be_bytes());
    st_rom[0x10..0x12].copy_from_slice(&0x60FEu16.to_be_bytes());
    let mut st = AtariSt::with_chipset(config(st_rom, RamSize::M1), Chipset::St).unwrap();
    assert!(st.pad_button(0, PadButton::A, true).is_err());
}

#[test]
fn the_gap_between_the_paddles_and_the_light_pen_bus_errors() {
    // A bus error handler at $E00080 notes it; the program reads $FF9218.
    let handler = 0x00E0_0080u32;
    let mut p: Vec<u16> = vec![0x23FC, (handler >> 16) as u16, handler as u16, 0, 8];
    p.extend([0x3039, 0x00FF, 0x9218]);
    p.push(0x60FE);
    while p.len() < (0x80 - 0x10) / 2 {
        p.push(0x4E71);
    }
    p.extend([0x33FC, 0xBEEF, 0, 0x1100, 0x60FE]);
    let mut m = machine(&p);
    run(&mut m, 300);
    assert_eq!(word(&m, 0x1100), 0xBEEF, "the handler ran");
    // a byte read of the light pen's odd address too
    let mut p: Vec<u16> = vec![0x23FC, (handler >> 16) as u16, handler as u16, 0, 8];
    p.extend([0x1039, 0x00FF, 0x9221]);
    p.push(0x60FE);
    while p.len() < (0x80 - 0x10) / 2 {
        p.push(0x4E71);
    }
    p.extend([0x33FC, 0xBEEF, 0, 0x1100, 0x60FE]);
    let mut m = machine(&p);
    run(&mut m, 300);
    assert_eq!(word(&m, 0x1100), 0xBEEF);
}
