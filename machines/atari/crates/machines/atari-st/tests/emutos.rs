//! EmuTOS 1.4 (192 KB, English PAL) on the machine: runs to its first access to a device that is not emulated yet,
//! the same way every time; and, when a reference trace from Hatari is present, takes the same path.
//!
//! The ROM is not committed. It comes from `RC_EMUTOS` (a path to `etos192uk.img`) or from the cache
//! `retroclawd --firmware auto` fills (`RETROCLAWD_CACHE_DIR`, else `%LOCALAPPDATA%/retroclawd/cache`,
//! `$XDG_CACHE_HOME/retroclawd`, `~/.cache/retroclawd`; file `firmware/<sha1>.bin`). Without it the tests print
//! where they looked and pass.

mod common;
use common::{MAX_CYCLES, emutos, hatari_io};

use std::path::PathBuf;

use atari_st::{AtariSt, RamSize};
use machine::Machine;

fn run(rom: Vec<u8>) -> AtariSt {
    common::run_until_stop(rom, RamSize::M1)
}

fn run_with(rom: Vec<u8>, ram: RamSize) -> AtariSt {
    common::run_until_stop(rom, ram)
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

/// Every I/O access we make agrees with Hatari's, in order, as `common::comparable` compares them.
#[test]
fn emutos_io_log_matches_hatari() {
    let (Some(rom), Some(reference)) = (emutos(), hatari_io("1m")) else {
        return;
    };
    let ours = common::io_lines(&run(rom));
    common::assert_prefix(
        &common::comparable(&ours),
        &common::comparable(&reference),
        "1m",
    );
    eprintln!("the first {} I/O accesses agree with Hatari", ours.len());
}

/// Slice 4's end: with the ACIAs and the IKBD, EmuTOS runs to its first DMA access, every access before it equal to
/// Hatari's under the polling rule, for every RAM size.
#[test]
fn emutos_runs_to_the_dma_chip() {
    let Some(rom) = emutos() else { return };
    for (name, ram) in common::RAM_SIZES {
        let a = run_with(rom.clone(), ram);
        let stop = a.io_stop().expect("stopped");
        assert_eq!(
            stop.reason,
            atari_st::StopReason::NotEmulated(atari_st::Device::Dma),
            "--ram {name}"
        );
        let ours = common::io_lines(&a);
        assert_eq!(
            ours.last().unwrap(),
            "IO write.w $ffff8606 = $0080 pc=fc3734",
            "--ram {name}"
        );
        if let Some(reference) = hatari_io(name) {
            let (ours, reference) = (common::comparable(&ours), common::comparable(&reference));
            common::assert_prefix(&ours, &reference, name);
            eprintln!(
                "--ram {name}: {} collapsed I/O accesses compared with Hatari",
                ours.len()
            );
        }
    }
}

/// EmuTOS's RAM sizing writes the same `$FF8001` values as in Hatari, for every `--ram`.
#[test]
fn emutos_sizes_ram_like_hatari() {
    let Some(rom) = emutos() else { return };
    for (name, ram) in common::RAM_SIZES {
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
