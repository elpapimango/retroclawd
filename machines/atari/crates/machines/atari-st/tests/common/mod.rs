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

/// Runs `frames` frames (VBLs), or to a stop, with `disk` in drive A.
pub fn run_frames(
    rom: Vec<u8>,
    ram: RamSize,
    frames: u64,
    disk: Option<st_floppy::Image>,
) -> AtariSt {
    let mut m = AtariSt::new(config(rom, ram)).unwrap();
    if let Some(d) = disk {
        m.insert_disk(0, d).unwrap();
    }
    while m.io_stop().is_none() && m.vbl_count() < frames {
        machine::Machine::step(&mut m);
    }
    m
}

/// Our I/O log in Hatari's format.
pub fn io_lines(m: &AtariSt) -> Vec<String> {
    m.io_log().iter().map(|a| a.hatari_line()).collect()
}

/// Lines a repeat is looked for in (see `collapse_polls`).
pub const POLL_WINDOW: usize = 4;

/// The polling rule: a line identical (address, size, direction, value and PC: the whole line) to one of the last
/// `POLL_WINDOW` lines kept is dropped. How often a loop polls a status register, and how many timer interrupts fall
/// between its polls, depends on emulator timing, not on a hardware fact; a changed value, a new address or PC, or a
/// different order still shows. Applied the same way to both logs.
pub fn collapse_polls(log: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(log.len());
    for line in log {
        let recent = &out[out.len().saturating_sub(POLL_WINDOW)..];
        if !recent.contains(line) {
            out.push(line.clone());
        }
    }
    out
}

/// Reads whose value depends on the exact cycle of the read rather than on a hardware fact: the MFP timer data
/// registers (`$FFFA1F-$FFFA25`) count down while their timer runs, so a few cycles of timing difference change the
/// value. Their value is masked; the access itself still counts.
const TIMING_DEPENDENT_READS: [&str; 4] = [
    "IO read.b $fffffa1f",
    "IO read.b $fffffa21",
    "IO read.b $fffffa23",
    "IO read.b $fffffa25",
];

/// The documented conflict on `$FF8604` reads: *Internals* p.59 says the upper byte reads as 1s, Hatari shows `$00`
/// (`docs/hardware-facts.md`). The upper byte is masked; the FDC's low byte is compared.
const FF8604_READ: &str = "IO read.w $ffff8604";

fn mask(line: &str) -> String {
    match line.split_once(" = $") {
        Some((head, tail)) if TIMING_DEPENDENT_READS.contains(&head) => {
            format!("{head} = $?? {}", tail.split_once(' ').map_or("", |t| t.1))
        }
        Some((head, tail)) if head == FF8604_READ && tail.len() > 2 => {
            format!("{head} = $??{}", &tail[2..])
        }
        _ => line.to_string(),
    }
}

/// A log ready for comparison: timing-dependent values masked, then the polling rule.
pub fn comparable(log: &[String]) -> Vec<String> {
    let masked: Vec<String> = log.iter().map(|l| mask(l)).collect();
    collapse_polls(&masked)
}

/// The accesses of each instruction address (the line's `pc=`), each stream masked and under the polling rule. An
/// interrupt handler's accesses fall between the main program's at points that depend on timing; each code
/// location's own accesses, in their order, do not.
pub fn streams(log: &[String]) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut by_pc: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for line in log {
        let pc = line.rsplit_once("pc=").map_or("", |p| p.1).to_string();
        by_pc.entry(pc).or_default().push(mask(line));
    }
    by_pc
        .into_iter()
        .map(|(pc, s)| (pc, collapse_polls(&s)))
        .collect()
}

/// Every code location in `reference` also runs in `ours`, and their streams (`streams`) agree as far as both go.
pub fn assert_same_streams(ours: &[String], reference: &[String], what: &str) {
    let (ours, reference) = (streams(ours), streams(reference));
    for (pc, theirs) in &reference {
        let mine = ours.get(pc).unwrap_or_else(|| {
            panic!(
                "{what}: pc={pc} runs in Hatari but not here: {:?}",
                &theirs[..theirs.len().min(3)]
            )
        });
        let n = mine.len().min(theirs.len());
        if let Some(i) = (0..n).find(|&i| mine[i] != theirs[i]) {
            panic!(
                "{what}: pc={pc} access {i} differs:
  ours:   {:?}
  hatari: {:?}",
                &mine[i..(i + 3).min(mine.len())],
                &theirs[i..(i + 3).min(theirs.len())]
            );
        }
    }
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
