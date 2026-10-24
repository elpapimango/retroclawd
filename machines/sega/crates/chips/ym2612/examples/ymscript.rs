//! Runs a script of register writes and chip cycles and writes every cycle's output, the format of the reference harness
//! (`software/tools/nuked/oracle.c`): `W port data`, `T cycles`, `R port` on stdin; `ymscript 2612|3438 out.raw`.

use std::io::{BufRead, Write};
use ym2612::{Chip, Ym2612};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let kind = if args[1] == "2612" {
        Chip::Ym2612
    } else {
        Chip::Ym3438
    };
    let mut chip = Ym2612::new(kind);
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[2]).unwrap());
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("W") => {
                let port: u32 = parts.next().unwrap().parse().unwrap();
                let data: u32 = parts.next().unwrap().parse().unwrap();
                chip.write(port, data as u8);
            }
            Some("T") => {
                let n: u64 = parts.next().unwrap().parse().unwrap();
                for _ in 0..n {
                    let (l, r) = chip.clock();
                    out.write_all(&l.to_le_bytes()).unwrap();
                    out.write_all(&r.to_le_bytes()).unwrap();
                }
            }
            Some("R") => {
                let port: u32 = parts.next().unwrap().parse().unwrap();
                println!("R {port} -> {}", chip.read(port));
            }
            _ => {}
        }
    }
}
