//! Turns arbitrary bytes into a guest program of register writes (video, MFP, PSG, DMA, ACIA, MMU: each three input bytes
//! choose a register and a value) and runs an `accurate` machine on it for a few frames. Any program must run without a
//! panic; a stop at an unemulated device is an allowed outcome.

#![no_main]

use atari_st::{Accuracy, AtariSt, Config, Monitor, RamSize, Region};
use libfuzzer_sys::fuzz_target;
use machine::Machine;

/// The registers a write may go to (short absolute addresses, `$FFxxxx`).
const REGISTERS: [u16; 24] = [
    0x8201, 0x8203, 0x820A, 0x8240, 0x8242, 0x825E, 0x8260, 0x8604, 0x8606, 0x8609, 0x860B, 0x860D, 0x8800, 0x8802, 0xFA07,
    0xFA09, 0xFA13, 0xFA15, 0xFA17, 0xFA19, 0xFA1B, 0xFA1D, 0xFC00, 0xFC02,
];

fn rom(data: &[u8]) -> Vec<u8> {
    let mut rom = vec![0u8; atari_st::ROM_SIZE];
    rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00FC_0010u32.to_be_bytes());
    let mut at = 0x10;
    let mut put = |w: u16| {
        if at + 2 <= 0x2000 {
            rom[at..at + 2].copy_from_slice(&w.to_be_bytes());
            at += 2;
        }
    };
    for c in data.chunks_exact(3).take(600) {
        let reg = REGISTERS[c[0] as usize % REGISTERS.len()];
        if c[0] & 0x80 != 0 {
            // MOVE.W #value,$reg.W
            put(0x31FC);
            put(u16::from(c[1]) << 8 | u16::from(c[2]));
        } else {
            // MOVE.B #value,$reg.W
            put(0x11FC);
            put(u16::from(c[1]));
        }
        put(reg);
    }
    // JMP $FC0010.L
    put(0x4EF9);
    put(0x00FC);
    put(0x0010);
    rom
}

fuzz_target!(|data: &[u8]| {
    let mut m = AtariSt::new(Config {
        ram: RamSize::K256,
        region: Region::Pal,
        accuracy: Accuracy::Accurate,
        monitor: Monitor::Colour,
        rom: rom(data),
        trace_limit: 0,
        io_log_limit: 0,
    })
    .unwrap();
    let target = 3;
    while m.io_stop().is_none() && m.vbl_count() < target {
        m.step();
    }
    let _ = m.render_frame();
});
