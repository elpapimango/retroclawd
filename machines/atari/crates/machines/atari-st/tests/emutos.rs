//! EmuTOS 1.4 (192 KB, English PAL) on the machine: runs to its first access to a device that is not emulated yet,
//! the same way every time; and, when a reference trace from Hatari is present, takes the same path.
//!
//! The ROM is not committed. It comes from `RC_EMUTOS` (a path to `etos192uk.img`) or from the cache
//! `retroclawd --firmware auto` fills (`RETROCLAWD_CACHE_DIR`, else `%LOCALAPPDATA%/retroclawd/cache`,
//! `$XDG_CACHE_HOME/retroclawd`, `~/.cache/retroclawd`; file `firmware/<sha1>.bin`). Without it the tests print
//! where they looked and pass.

use std::path::PathBuf;

use atari_st::{Accuracy, AtariSt, Config, RamSize, Region};
use machine::Machine;

const ETOS192UK_SHA1: &str = "904ea9dc09ff8286f6e0272c12255dfe85beb17a";
/// Long enough for many frames of boot; the run ends earlier at the first unemulated device.
const MAX_CYCLES: u64 = 10_000_000;

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

fn emutos() -> Option<Vec<u8>> {
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

/// Runs to the stop (or `MAX_CYCLES`); returns the machine.
fn run(rom: Vec<u8>) -> AtariSt {
    run_with(rom, RamSize::M1)
}

fn run_with(rom: Vec<u8>, ram: RamSize) -> AtariSt {
    let mut m = AtariSt::new(Config {
        ram,
        region: Region::Pal,
        accuracy: Accuracy::Fast,
        rom,
        trace_limit: 1_000_000,
        io_log_limit: 1_000_000,
    })
    .unwrap();
    while m.io_stop().is_none() && m.now() < MAX_CYCLES {
        m.step();
    }
    m
}

#[test]
fn emutos_runs_deterministically_to_its_first_io_access() {
    let Some(rom) = emutos() else { return };
    let a = run(rom.clone());
    let b = run(rom);
    assert_eq!(
        a.trace().first(),
        Some(&0xFC0030),
        "reset PC from the ROM header"
    );
    assert_eq!(a.trace(), b.trace());
    assert_eq!(a.io_stop(), b.io_stop());
    assert_eq!(a.registers(), b.registers());
    match a.io_stop() {
        Some(stop) => eprintln!(
            "EmuTOS stops after {} instructions at ${:06X} ({} {:?}), cycle {}",
            a.trace().len(),
            stop.addr,
            if stop.write { "write" } else { "read" },
            stop.size,
            stop.at_cycle
        ),
        None => eprintln!("EmuTOS ran {MAX_CYCLES} cycles without a stop"),
    }
}

/// The reference trace: one PC per line (`{pc:06X}`), from `retroclawd-tools/scripts/capture_hatari_trace.py`,
/// kept with the other goldens (`RC_GOLDENS`), never committed.
fn hatari_trace() -> Option<Vec<u32>> {
    let path = std::env::var_os("RC_GOLDENS")
        .map(PathBuf::from)?
        .join("atari-st")
        .join("emutos14-uk-trace.txt");
    let text = std::fs::read_to_string(&path)
        .map_err(|_| {
            eprintln!(
                "no Hatari trace at {} (capture_hatari_trace.py makes it)",
                path.display()
            )
        })
        .ok()?;
    Some(
        text.lines()
            .filter_map(|l| u32::from_str_radix(l.trim(), 16).ok())
            .collect(),
    )
}

#[test]
fn emutos_trace_matches_hatari_up_to_our_stop() {
    let (Some(rom), Some(reference)) = (emutos(), hatari_trace()) else {
        return;
    };
    let ours = run(rom);
    let n = ours.trace().len().min(reference.len());
    assert!(n > 0, "empty trace");
    if let Some(i) = (0..n).find(|&i| ours.trace()[i] != reference[i]) {
        let from = i.saturating_sub(8);
        let show = |t: &[u32]| {
            t[from..(i + 8).min(t.len())]
                .iter()
                .map(|pc| format!("{pc:06X}"))
                .collect::<Vec<_>>()
        };
        panic!(
            "instruction {i} differs:\n  ours:   {:?}\n  hatari: {:?}",
            show(ours.trace()),
            show(&reference)
        );
    }
    eprintln!("the first {n} PCs agree with Hatari");
}

/// Hatari's I/O log for `--memsize <size>` (`capture_hatari_trace.py --kind io`), from `RC_GOLDENS`.
fn hatari_io(size: &str) -> Option<Vec<String>> {
    let path = std::env::var_os("RC_GOLDENS")
        .map(PathBuf::from)?
        .join("atari-st")
        .join(format!("emutos14-uk-io-{size}.txt"));
    match std::fs::read_to_string(&path) {
        Ok(text) => Some(text.lines().map(str::to_string).collect()),
        Err(_) => {
            eprintln!(
                "no Hatari I/O log at {} (capture_hatari_trace.py --kind io makes it)",
                path.display()
            );
            None
        }
    }
}

/// Every I/O access we make agrees with Hatari's, in order, up to our stop.
#[test]
fn emutos_io_log_matches_hatari() {
    let (Some(rom), Some(reference)) = (emutos(), hatari_io("1m")) else {
        return;
    };
    let ours: Vec<String> = run(rom).io_log().iter().map(|a| a.hatari_line()).collect();
    assert!(
        ours.len() <= reference.len(),
        "ours is longer than the reference"
    );
    if let Some(i) = (0..ours.len()).find(|&i| ours[i] != reference[i]) {
        let from = i.saturating_sub(4);
        panic!(
            "I/O access {i} differs:\n  ours:   {:#?}\n  hatari: {:#?}",
            &ours[from..(i + 4).min(ours.len())],
            &reference[from..(i + 4).min(reference.len())]
        );
    }
    eprintln!("the first {} I/O accesses agree with Hatari", ours.len());
}

/// EmuTOS's RAM sizing writes the same `$FF8001` values as in Hatari, for every `--ram`.
#[test]
fn emutos_sizes_ram_like_hatari() {
    let Some(rom) = emutos() else { return };
    for (name, ram) in [
        ("256k", RamSize::K256),
        ("512k", RamSize::K512),
        ("1m", RamSize::M1),
        ("2m", RamSize::M2),
        ("2.5m", RamSize::M2_5),
        ("4m", RamSize::M4),
    ] {
        let Some(reference) = hatari_io(name) else {
            continue;
        };
        let mmu = |lines: Vec<String>| -> Vec<String> {
            lines
                .into_iter()
                .filter(|l| l.contains("$ffff8001"))
                .collect()
        };
        let ours = mmu(run_with(rom.clone(), ram)
            .io_log()
            .iter()
            .map(|a| a.hatari_line())
            .collect());
        assert_eq!(ours, mmu(reference), "--ram {name}");
    }
}
