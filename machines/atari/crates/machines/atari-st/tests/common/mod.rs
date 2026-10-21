//! Helpers shared by the integration tests: EmuTOS and the Hatari goldens (never committed), bare-metal ROMs, and
//! the polling rule for comparing I/O logs.
#![allow(dead_code)]

use std::path::PathBuf;

use atari_st::{Accuracy, AtariSt, Config, Monitor, RamSize, Region};

pub const ETOS192UK_SHA1: &str = "904ea9dc09ff8286f6e0272c12255dfe85beb17a";
/// Long enough for many frames of boot; the run ends earlier at the first unemulated device.
pub const MAX_CYCLES: u64 = 10_000_000;

/// The six RAM sizes with Hatari golden names.
pub const RAM_SIZES: [(&str, RamSize); 6] = [
    ("256k", RamSize::K256),
    ("512k", RamSize::K512),
    ("1m", RamSize::M1),
    ("2m", RamSize::M2),
    ("2.5m", RamSize::M2_5),
    ("4m", RamSize::M4),
];

fn cache_root() -> Option<PathBuf> {
    let env = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    env("RETROCLAWD_CACHE_DIR")
        .or_else(|| env("LOCALAPPDATA").map(|p| p.join("retroclawd").join("cache")))
        .or_else(|| env("XDG_CACHE_HOME").map(|p| p.join("retroclawd")))
        .or_else(|| env("HOME").map(|p| p.join(".cache").join("retroclawd")))
}

/// EmuTOS 1.4 (192 KB UK) from `RC_EMUTOS` or the `--firmware auto` cache; `None` (with a note) when absent.
pub fn emutos() -> Option<Vec<u8>> {
    let path = std::env::var_os("RC_EMUTOS")
        .map(PathBuf::from)
        .or_else(|| {
            cache_root().map(|c| c.join("firmware").join(format!("{ETOS192UK_SHA1}.bin")))
        })?;
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!(
                "EmuTOS not found at {} (set RC_EMUTOS, or run `retroclawd atari-st --firmware auto` once)",
                path.display()
            );
            None
        }
    }
}

/// A golden file under `RC_GOLDENS/atari-st/`, one line per entry.
pub fn golden(name: &str) -> Option<Vec<String>> {
    let path = std::env::var_os("RC_GOLDENS")
        .map(PathBuf::from)?
        .join("atari-st")
        .join(name);
    match std::fs::read_to_string(&path) {
        Ok(text) => Some(text.lines().map(str::to_string).collect()),
        Err(_) => {
            eprintln!(
                "no golden at {} (retroclawd-tools/scripts/capture_hatari_trace.py makes it)",
                path.display()
            );
            None
        }
    }
}

/// Hatari's I/O log for `--memsize <size>` (`capture_hatari_trace.py --kind io`).
pub fn hatari_io(size: &str) -> Option<Vec<String>> {
    golden(&format!("emutos14-uk-io-{size}.txt"))
}

pub fn config(rom: Vec<u8>, ram: RamSize) -> Config {
    Config {
        ram,
        region: Region::Pal,
        accuracy: Accuracy::Fast,
        monitor: Monitor::Colour,
        rom,
        trace_limit: 1_000_000,
        io_log_limit: 1_000_000,
    }
}

/// Runs to the first stop (or `MAX_CYCLES`).
pub fn run_until_stop(rom: Vec<u8>, ram: RamSize) -> AtariSt {
    let mut m = AtariSt::new(config(rom, ram)).unwrap();
    while m.io_stop().is_none() && m.now() < MAX_CYCLES {
        machine::Machine::step(&mut m);
    }
    m
}

/// Our I/O log in Hatari's format.
pub fn io_lines(m: &AtariSt) -> Vec<String> {
    m.io_log().iter().map(|a| a.hatari_line()).collect()
}

/// The polling rule: every run of consecutive identical accesses (address, size, direction, value and PC: the whole
/// line) becomes one entry. How often a loop polls depends on emulator timing, not on a hardware fact.
pub fn collapse_polls(log: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(log.len());
    for line in log {
        if out.last() != Some(line) {
            out.push(line.clone());
        }
    }
    out
}

/// Asserts `ours` equals the first `ours.len()` lines of `reference`, showing the first difference.
pub fn assert_prefix(ours: &[String], reference: &[String], what: &str) {
    assert!(
        ours.len() <= reference.len(),
        "{what}: ours ({}) is longer than the reference ({})",
        ours.len(),
        reference.len()
    );
    if let Some(i) = (0..ours.len()).find(|&i| ours[i] != reference[i]) {
        let from = i.saturating_sub(4);
        panic!(
            "{what}: I/O access {i} differs:\n  ours:   {:#?}\n  hatari: {:#?}",
            &ours[from..(i + 4).min(ours.len())],
            &reference[from..(i + 4).min(reference.len())]
        );
    }
}

/// A 192 KB ROM: SSP `$8000`, reset PC `$FC0010`, `program` from `$FC0010`, extra code blocks at their ROM offsets.
pub fn rom(program: &[u16], blocks: &[(usize, &[u16])]) -> Vec<u8> {
    let mut rom = vec![0u8; 196_608];
    rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00FC_0010u32.to_be_bytes());
    let mut put = |at: usize, words: &[u16]| {
        for (i, w) in words.iter().enumerate() {
            rom[at + 2 * i..at + 2 + 2 * i].copy_from_slice(&w.to_be_bytes());
        }
    };
    put(0x10, program);
    for (at, words) in blocks {
        put(*at, words);
    }
    rom
}

/// `MOVE.B #value,$addr.W` (the short address sign-extends to `$FFxxxx`).
pub fn move_b(value: u8, addr: u16) -> [u16; 3] {
    [0x11FC, value as u16, addr]
}

pub const BRA_SELF: u16 = 0x60FE;
