//! Speed of the machine: instructions a second of a NOP/ALU mix, and frames a second of the idle EmuTOS desktop, `fast` and
//! `accurate`. `cargo run --release --example bench -p atari-st` (EmuTOS from `RC_EMUTOS` or its first argument).
//! A frame is 1/50 s of machine time, so the real-time multiple is the frames a second over 50.

use std::time::Instant;

use atari_st::{Accuracy, AtariSt, Config, RamSize, Region};
use machine::Machine;

fn config(rom: Vec<u8>, accuracy: Accuracy) -> Config {
    Config {
        ram: RamSize::M1,
        region: Region::Pal,
        accuracy,
        monitor: st_shifter::Monitor::Colour,
        rom,
        trace_limit: 0,
        io_log_limit: 0,
    }
}

/// A 192 KB ROM: SSP `$8000`, reset PC `$FC0010`, an endless loop of register ALU instructions.
fn alu_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 196_608];
    rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00FC_0010u32.to_be_bytes());
    // ADD.L D0,D1; SUB.L D1,D2; EOR.L D2,D3; ADD.W D3,D0; NOP; MOVE.L D0,D4; BRA.S to the start (-14)
    let words: [u16; 7] = [0xD280, 0x9481, 0xB583, 0xD043, 0x4E71, 0x2800, 0x60F4];
    for (i, w) in words.iter().enumerate() {
        rom[0x10 + 2 * i..0x12 + 2 * i].copy_from_slice(&w.to_be_bytes());
    }
    rom
}

fn mips(accuracy: Accuracy) -> f64 {
    let mut m = AtariSt::new(config(alu_rom(), accuracy)).unwrap();
    let n = 20_000_000u64;
    let t = Instant::now();
    for _ in 0..n {
        m.step();
    }
    n as f64 / t.elapsed().as_secs_f64() / 1e6
}

fn desktop_fps(rom: &[u8], accuracy: Accuracy) -> f64 {
    let mut m = AtariSt::new(config(rom.to_vec(), accuracy)).unwrap();
    while m.vbl_count() < 900 && m.io_stop().is_none() {
        m.step();
    }
    let (start, t) = (m.vbl_count(), Instant::now());
    while m.vbl_count() < start + 300 && m.io_stop().is_none() {
        m.step();
    }
    (m.vbl_count() - start) as f64 / t.elapsed().as_secs_f64()
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("RC_EMUTOS").ok());
    for accuracy in [Accuracy::Fast, Accuracy::Accurate] {
        print!("{accuracy:?}: {:.1} MIPS (NOP/ALU mix)", mips(accuracy));
        match path.as_ref().and_then(|p| std::fs::read(p).ok()) {
            Some(rom) => {
                let fps = desktop_fps(&rom, accuracy);
                println!(
                    ", idle desktop {fps:.0} frames/s = {:.1}x real time",
                    fps / 50.0
                );
            }
            None => println!(", no EmuTOS (RC_EMUTOS or an argument) for the desktop figure"),
        }
    }
}
