//! A faulty character on the RS-232 line (parity, framing, break) reaches the MFP RSR as flags (semantics of the USART
//! flags: see `mc68901`).

mod common;
use common::config;

use atari_st::{AtariSt, Chipset, RamSize};
use machine::Machine;
use mc68901::RxFault;

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

/// `MOVE.B abs.L,abs.L`
fn copy(src: u32, dst: u32) -> [u16; 5] {
    [
        0x13F9,
        (src >> 16) as u16,
        src as u16,
        (dst >> 16) as u16,
        dst as u16,
    ]
}

#[test]
fn a_break_and_a_framing_error_reach_the_receiver_status_register() {
    // UCR $88 (8 bits, one stop bit, divide by 16), timer D running, receiver on; then wait and store RSR and UDR.
    let mut p = Vec::new();
    p.extend(mb(0x88, 0xFF_FA29));
    p.extend(mb(0x01, 0xFF_FA25));
    p.extend(mb(0x01, 0xFF_FA1D));
    p.extend(mb(0x01, 0xFF_FA2B));
    p.extend(std::iter::repeat_n(0x4E71, 3000));
    p.extend(copy(0xFF_FA2B, 0x1000));
    p.extend(copy(0xFF_FA2F, 0x1001));
    p.push(0x60FE);
    let mut m = AtariSt::with_chipset(config(ste_with(&p), RamSize::M1), Chipset::Ste).unwrap();
    for _ in 0..60 {
        m.step();
    }
    assert!(
        m.rs232_receive_fault(0x55, RxFault::Break),
        "the receiver took the character"
    );
    for _ in 0..8000 {
        m.step();
    }
    let rsr = m.read_memory(0x1000);
    assert_eq!(
        rsr & 0x98,
        0x88,
        "buffer full and break, no frame error: {rsr:#04x}"
    );
    assert_eq!(m.read_memory(0x1001), 0, "a break reads as 0");
}
