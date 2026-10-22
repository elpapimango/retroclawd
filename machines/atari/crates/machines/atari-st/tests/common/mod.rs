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

/// A 720K floppy built here, with an executable boot sector (*Atari ST Internals* pp.178-179: the BPB of an 80-track
/// double-sided disk, little-endian; the big-endian word sum of the sector is `$1234`; the program starts at 30).
/// The program sets palette 0 to `$0123`, writes `$CAFE` to `$600` and loops.
pub fn boot_floppy() -> st_floppy::Image {
    let mut disk = vec![0u8; 737_280];
    let boot = &mut disk[..512];
    boot[0..2].copy_from_slice(&0x601Cu16.to_be_bytes()); // BRA.S to offset 30
    let le = |b: &mut [u8], at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
    le(boot, 11, 512); // bytes per sector
    boot[13] = 2; // sectors per cluster
    le(boot, 14, 1); // reserved sectors
    boot[16] = 2; // FATs
    le(boot, 17, 112); // directory entries
    le(boot, 19, 1440); // sectors
    boot[21] = 0xF9; // media
    le(boot, 22, 5); // sectors per FAT
    le(boot, 24, 9); // sectors per track
    le(boot, 26, 2); // sides
    let program: [u16; 9] = [
        0x33FC, 0x0123, 0x00FF, 0x8240, // MOVE.W #$0123,$FF8240
        0x31FC, 0xCAFE, 0x0600, // MOVE.W #$CAFE,$600.W
        0x60FE, // BRA.S *
        0,
    ];
    for (i, w) in program.iter().enumerate() {
        boot[30 + 2 * i..32 + 2 * i].copy_from_slice(&w.to_be_bytes());
    }
    let sum = boot[..510].chunks(2).fold(0u16, |s, w| {
        s.wrapping_add(u16::from_be_bytes([w[0], w[1]]))
    });
    boot[510..512].copy_from_slice(&0x1234u16.wrapping_sub(sum).to_be_bytes());
    st_floppy::Image::parse(&disk).unwrap()
}

/// Runs `frames` frames (or to a stop) with `disk` in drive A and returns the machine and its whole I/O log, taken in
/// pieces with consecutive repeats dropped (the polling rule would drop them anyway) so a long run fits in memory.
pub fn run_frames_logged(
    rom: Vec<u8>,
    ram: RamSize,
    frames: u64,
    disk: Option<st_floppy::Image>,
) -> (AtariSt, Vec<String>) {
    let mut m = AtariSt::new(config(rom, ram)).unwrap();
    if let Some(d) = disk {
        m.insert_disk(0, d).unwrap();
    }
    let mut log: Vec<String> = Vec::new();
    let take = |m: &mut AtariSt, log: &mut Vec<String>| {
        for a in m.take_io_log() {
            let line = a.hatari_line();
            if log.last() != Some(&line) {
                log.push(line);
            }
        }
    };
    let mut steps = 0u64;
    while m.io_stop().is_none() && m.vbl_count() < frames {
        machine::Machine::step(&mut m);
        steps += 1;
        if steps.is_multiple_of(100_000) {
            take(&mut m, &mut log);
        }
    }
    take(&mut m, &mut log);
    (m, log)
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
const FF8606_READ: &str = "IO read.w $ffff8606";

/// Hatari prints the CPU's effective address (`$00ff8240` for an absolute long `$00FF8240`, `$ffff8240` for a
/// sign-extended short one); the 68000 puts only 24 bits on the bus, so the top byte is not compared.
fn bus_address(line: &str) -> String {
    match line.find(" $") {
        Some(i) if line.len() > i + 4 && line[i + 2..].starts_with("00ff") => {
            format!("{} $ffff{}", &line[..i], &line[i + 6..])
        }
        _ => line.to_string(),
    }
}

fn mask(line: &str) -> String {
    let line = &bus_address(line);
    match line.split_once(" = $") {
        Some((head, tail)) if TIMING_DEPENDENT_READS.contains(&head) => {
            format!("{head} = $?? {}", tail.split_once(' ').map_or("", |t| t.1))
        }
        Some((head, tail)) if head == FF8604_READ && tail.len() > 2 => {
            format!("{head} = $??{}", &tail[2..])
        }
        // `$FF8606` reads: only the three status bits are a fact; Hatari returns leftover bits above them.
        Some((head, tail)) if head == FF8606_READ => {
            let (value, rest) = tail.split_once(' ').unwrap_or((tail, ""));
            let bits = u16::from_str_radix(value, 16).map_or(0xFF, |v| v & 7);
            format!("{head} = $...{bits} {rest}")
        }
        _ => line.to_string(),
    }
}

/// A log ready for comparison: timing-dependent values masked, then the polling rule.
pub fn comparable(log: &[String]) -> Vec<String> {
    let masked: Vec<String> = log.iter().map(|l| mask(l)).collect();
    collapse_polls(&masked)
}

/// The accesses of each instruction address (the line's `pc=`), each stream masked and under the polling rule, with
/// each kept access's position in the log. An interrupt handler's accesses fall between the main program's at
/// points that depend on timing; each code location's own accesses, in their order, do not.
fn positioned_streams(log: &[String]) -> std::collections::BTreeMap<String, Vec<(usize, String)>> {
    let mut by_pc: std::collections::BTreeMap<String, Vec<(usize, String)>> = Default::default();
    for (n, line) in log.iter().enumerate() {
        let pc = line.rsplit_once("pc=").map_or("", |p| p.1).to_string();
        let stream = by_pc.entry(pc).or_default();
        let line = mask(line);
        let recent = &stream[stream.len().saturating_sub(POLL_WINDOW)..];
        if !recent.iter().any(|(_, l)| *l == line) {
            stream.push((n, line));
        }
    }
    by_pc
}

/// `positioned_streams` without the positions.
pub fn streams(log: &[String]) -> std::collections::BTreeMap<String, Vec<String>> {
    positioned_streams(log)
        .into_iter()
        .map(|(pc, s)| (pc, s.into_iter().map(|(_, l)| l).collect()))
        .collect()
}

/// Every code location in `reference` also runs in `ours`, and their streams (`streams`) agree as far as both go.
/// A failure names the difference that comes first in Hatari's log.
pub fn assert_same_streams(ours: &[String], reference: &[String], what: &str) {
    let (mine_all, theirs_all) = (positioned_streams(ours), positioned_streams(reference));
    let mut first: Option<(usize, String)> = None;
    for (pc, theirs) in &theirs_all {
        let Some(mine) = mine_all.get(pc) else {
            let at = theirs[0].0;
            let msg = format!(
                "pc={pc} runs in Hatari (its access {at}) but not here: {:?}",
                theirs[0].1
            );
            if first.as_ref().is_none_or(|(n, _)| at < *n) {
                first = Some((at, msg));
            }
            continue;
        };
        let n = mine.len().min(theirs.len());
        if let Some(i) = (0..n).find(|&i| mine[i].1 != theirs[i].1) {
            let at = theirs[i].0;
            let show = |s: &[(usize, String)]| {
                s[i..(i + 3).min(s.len())]
                    .iter()
                    .map(|e| e.1.clone())
                    .collect::<Vec<_>>()
            };
            let msg = format!(
                "pc={pc} access {i} differs (our access {at}, Hatari's {}):
  ours:   {:?}
  hatari: {:?}",
                theirs[i].0,
                show(mine),
                show(theirs)
            );
            if first.as_ref().is_none_or(|(n, _)| at < *n) {
                first = Some((at, msg));
            }
        }
    }
    for pc in mine_all.keys() {
        if !theirs_all.contains_key(pc) && first.is_none() {
            first = Some((
                usize::MAX,
                format!(
                    "pc={pc} runs here but not in Hatari: {:?}",
                    mine_all[pc][0].1
                ),
            ));
        }
    }
    if let Some((_, msg)) = first {
        panic!("{what}: {msg}");
    }
}

/// Levels of asynchronous signals that a long run samples at timing-dependent moments: GPIP I5 (the FDC's
/// interrupt, low from a command's end until its status is read) and the FDC status's index bit (set for the
/// index pulse's few byte times per revolution). Their behaviour is tested in `dma.rs` and `drive.rs`; the long-run
/// set comparison masks them.
fn transient(line: &str) -> String {
    let bits = |line: &str, head: &str, clear: u16, width: usize| -> Option<String> {
        let rest = line.strip_prefix(head)?.strip_prefix(" = $")?;
        let (value, tail) = rest.split_once(' ')?;
        let v = u16::from_str_radix(value.trim_start_matches('?'), 16).ok()? & !clear;
        let keep = value.len() - value.trim_start_matches('?').len();
        let hex = format!("{v:0width$x}");
        Some(format!(
            "{head} = ${}{} {tail}",
            &value[..keep],
            &hex[hex.len() - (width - keep)..]
        ))
    };
    bits(line, "IO read.b $fffffa01", 0x20, 2)
        .or_else(|| bits(line, "IO read.w $ffff8604", 0x02, 4))
        .unwrap_or_else(|| line.to_string())
}

/// Every code location produces the same distinct accesses (masked) in both logs, whatever their order and count.
/// For long runs, where background tasks driven by the VBL (EmuTOS's floppy media check alternates drive A and B
/// each frame) start on a frame that depends on timing: their phase changes the order at one location, not the
/// values. A failure names the first location in Hatari's log.
pub fn assert_same_sets(ours: &[String], reference: &[String], what: &str) {
    use std::collections::BTreeSet;
    let sets = |log: &[String]| {
        let mut by_pc: std::collections::BTreeMap<String, (usize, BTreeSet<String>)> =
            Default::default();
        for (n, line) in log.iter().enumerate() {
            let pc = line.rsplit_once("pc=").map_or("", |p| p.1).to_string();
            by_pc
                .entry(pc)
                .or_insert((n, BTreeSet::new()))
                .1
                .insert(transient(&mask(line)));
        }
        by_pc
    };
    let (mine, theirs) = (sets(ours), sets(reference));
    let mut problems: Vec<(usize, String)> = Vec::new();
    for (pc, (at, t)) in &theirs {
        match mine.get(pc) {
            None => problems.push((
                *at,
                format!("pc={pc} runs in Hatari but not here: {:?}", t.first()),
            )),
            Some((_, m)) if m != t => problems.push((
                *at,
                format!(
                    "pc={pc}: only here {:?}; only in Hatari {:?}",
                    m.difference(t).take(3).collect::<Vec<_>>(),
                    t.difference(m).take(3).collect::<Vec<_>>()
                ),
            )),
            Some(_) => {}
        }
    }
    for (pc, (_, m)) in &mine {
        if !theirs.contains_key(pc) {
            problems.push((
                usize::MAX,
                format!("pc={pc} runs here but not in Hatari: {:?}", m.first()),
            ));
        }
    }
    problems.sort();
    if let Some((_, first)) = problems.first() {
        let all: Vec<&str> = problems.iter().take(12).map(|p| p.1.as_str()).collect();
        panic!(
            "{what}: {} differences; first: {first}
all: {all:#?}",
            problems.len()
        );
    }
}

/// The accesses whose line starts with one of `prefixes`, in order, exactly as logged.
pub fn ordered(log: &[String], prefixes: &[&str]) -> Vec<String> {
    log.iter()
        .filter(|l| prefixes.iter().any(|p| l.starts_with(p)))
        .cloned()
        .collect()
}

/// Sequences that do not depend on timing, compared exactly and in order per code location over a whole run (in
/// number too, within 5% for a long periodic one): the bytes the IKBD sent
/// and the commands it got, and every DMA/FDC register write (the floppy commands and their set-up).
pub fn assert_same_ordered_sequences(ours: &[String], reference: &[String], what: &str) {
    for (name, prefixes) in [
        ("IKBD bytes read", &["IO read.b $fffffc02"][..]),
        ("IKBD commands", &["IO write.b $fffffc02"][..]),
        (
            "DMA/FDC writes",
            &["IO write.w $ffff8604", "IO write.w $ffff8606"][..],
        ),
    ] {
        // Per code location: an interrupt handler's writes (EmuTOS's VBL media check) fall between the main
        // routine's at timing-dependent points; each location's own sequence, its order and its count, do not.
        let by_pc = |log: Vec<String>| {
            let mut m: std::collections::BTreeMap<String, Vec<String>> = Default::default();
            for l in log {
                m.entry(l.rsplit_once("pc=").map_or("", |p| p.1).to_string())
                    .or_default()
                    .push(l);
            }
            m
        };
        let (mine_all, theirs_all) = (
            by_pc(ordered(ours, prefixes)),
            by_pc(ordered(reference, prefixes)),
        );
        let pcs: std::collections::BTreeSet<&String> =
            mine_all.keys().chain(theirs_all.keys()).collect();
        for pc in pcs {
            let (mine, theirs) = (
                mine_all.get(pc).cloned().unwrap_or_default(),
                theirs_all.get(pc).cloned().unwrap_or_default(),
            );
            // A periodic task's count depends on the frame it starts in: up to 5% (rounded down) more or fewer
            // entries pass, so a short sequence must match in number exactly.
            let common = mine.len().min(theirs.len());
            let slack = mine.len().max(theirs.len()) / 20;
            let limit = if mine.len().abs_diff(theirs.len()) <= slack {
                common
            } else {
                common + 1
            };
            if let Some(i) = (0..limit).find(|&i| mine.get(i) != theirs.get(i)) {
                panic!(
                    "{what}: {name} at pc={pc}: entry {i} differs ({} here, {} in Hatari):
  ours:   {:?}
  hatari: {:?}",
                    mine.len(),
                    theirs.len(),
                    &mine[i.min(mine.len())..(i + 3).min(mine.len())],
                    &theirs[i.min(theirs.len())..(i + 3).min(theirs.len())]
                );
            }
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

/// `BTST #7,$addr.W` then `BEQ` back to it: waits until bit 7 of the byte at `$FFxxxx` is set.
pub fn wait_bit7(addr: u16) -> [u16; 4] {
    [0x0838, 0x0007, addr, 0x67F8]
}

/// `MOVE.B $src.W,$dst.W`.
pub fn move_b_abs(src: u16, dst: u16) -> [u16; 3] {
    [0x11F8, src, dst]
}

/// `BTST #bit,$addr.W` then `BEQ` back to it: waits until that bit of the byte at `$FFxxxx` is set.
pub fn wait_bit(bit: u16, addr: u16) -> [u16; 4] {
    [0x0838, bit, addr, 0x67F8]
}
