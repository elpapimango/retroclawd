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

/// Hatari's 50-frame log against ours over the same 50 frames: every code location's accesses agree
/// (`common::assert_same_streams`).
#[test]
fn emutos_io_log_matches_hatari() {
    let (Some(rom), Some(reference)) = (emutos(), hatari_io("1m")) else {
        return;
    };
    let ours = common::io_lines(&common::run_frames(rom, RamSize::M1, 50, None));
    common::assert_same_streams(&ours, &reference, "1m");
    eprintln!(
        "{} accesses over 50 frames agree with Hatari's, code location by code location",
        ours.len()
    );
}

/// Slice 4's end: with the ACIAs and the IKBD, EmuTOS's accesses up to its first DMA access equal Hatari's, as
/// `common::comparable` compares them, for every RAM size.
#[test]
fn emutos_matches_hatari_up_to_the_dma_chip() {
    let Some(rom) = emutos() else { return };
    let first_dma = "IO write.w $ffff8606 = $0080 pc=fc3734";
    for (name, ram) in common::RAM_SIZES {
        let ours = common::io_lines(&run_with(rom.clone(), ram));
        let end = ours
            .iter()
            .position(|l| l == first_dma)
            .expect("reaches the DMA chip")
            + 1;
        if let Some(reference) = hatari_io(name) {
            let ours = common::comparable(&ours[..end]);
            common::assert_prefix(&ours, &common::comparable(&reference), name);
            eprintln!(
                "--ram {name}: {} compared I/O accesses up to the DMA chip agree with Hatari",
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

/// The boot floppy is executable by the rule in *Internals* p.179; with `RC_DUMP_BOOT_FLOPPY` set it is written
/// there (for the Hatari golden: `capture_hatari_trace.py --disk-a`).
#[test]
fn boot_floppy_is_executable() {
    let img = common::boot_floppy();
    let boot = img.sector(0, 0, 1).unwrap();
    let sum = boot.chunks(2).fold(0u16, |s, w| {
        s.wrapping_add(u16::from_be_bytes([w[0], w[1]]))
    });
    assert_eq!(sum, 0x1234);
    if let Some(path) = std::env::var_os("RC_DUMP_BOOT_FLOPPY") {
        std::fs::write(path, img.to_bytes()).unwrap();
    }
}

/// Slice 5's end, part 1: 1500 frames with no disk, no stop, every code location's accesses the same as Hatari's
/// (`common::assert_same_sets`).
#[test]
fn emutos_1500_frames_without_a_disk_match_hatari() {
    let Some(rom) = emutos() else { return };
    let (m, ours) = common::run_frames_logged(rom, RamSize::M1, 1500, None);
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
    if let Some(reference) = common::golden("emutos14-uk-io-1m-1500.txt") {
        common::assert_same_sets(&ours, &reference, "1500 frames");
        eprintln!(
            "1500 frames: {} accesses, every code location's accesses the same as Hatari's",
            ours.len()
        );
    }
}

/// Slice 5's end, part 2: EmuTOS boots the floppy built here; the boot program runs (palette 0 `$0123`, `$CAFE` at
/// `$600`), and the accesses on the way agree with Hatari's booting the same image.
#[test]
fn emutos_boots_our_floppy() {
    let Some(rom) = emutos() else { return };
    // As long as Hatari's golden (1500 frames); the boot program loops once it has run.
    let (m, ours) = common::run_frames_logged(rom, RamSize::M1, 1500, Some(common::boot_floppy()));
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
    assert_eq!(
        (m.read_memory(0x600), m.read_memory(0x601)),
        (0xCA, 0xFE),
        "the boot program ran"
    );
    assert_eq!(m.bus().shifter().palette()[0], 0x0123);
    if let Some(reference) = common::golden("emutos14-uk-io-1m-boot.txt") {
        common::assert_same_sets(&ours, &reference, "boot");
        eprintln!("booting: accesses agree with Hatari's code location by code location");
    }
}

/// Slice 5's end, part 3: three runs of 1500 frames end on the same picture (the desktop). With `RC_SCREENSHOT_DIR`
/// set, the picture is written there as `emutos-1500.ppm`.
#[test]
fn emutos_desktop_frame_is_deterministic() {
    let Some(rom) = emutos() else { return };
    let frames: Vec<_> = (0..3)
        .map(|_| {
            common::run_frames(rom.clone(), RamSize::M1, 1500, None)
                .render_frame()
                .expect("a picture")
        })
        .collect();
    assert_eq!(frames[0], frames[1]);
    assert_eq!(frames[1], frames[2]);
    let (w, h, rgb) = &frames[0];
    assert!(rgb.iter().any(|&b| b != rgb[0]), "the picture is not blank");
    if let Some(dir) = std::env::var_os("RC_SCREENSHOT_DIR") {
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        ppm.extend_from_slice(rgb);
        std::fs::write(std::path::Path::new(&dir).join("emutos-1500.ppm"), ppm).unwrap();
    }
}

/// A key reaches EmuTOS: pressed at frame 1200 and released at 1205, its make and break codes (`$1E`, `$9E`) are
/// read from the keyboard ACIA's data register.
#[test]
fn emutos_sees_a_key() {
    let Some(rom) = emutos() else { return };
    let mut m = AtariSt::new(common::config(rom, RamSize::M1)).unwrap();
    let mut reads: Vec<u32> = Vec::new();
    let mut run_to = |m: &mut AtariSt, frame: u64| {
        while m.vbl_count() < frame {
            m.step();
            if m.now().is_multiple_of(64) {
                let log = m.take_io_log();
                reads.extend(
                    log.iter()
                        .filter(|a| a.addr == 0xFF_FC02 && !a.write)
                        .map(|a| a.value),
                );
            }
        }
        let log = m.take_io_log();
        reads.extend(
            log.iter()
                .filter(|a| a.addr == 0xFF_FC02 && !a.write)
                .map(|a| a.value),
        );
    };
    run_to(&mut m, 1200);
    m.key_event("KeyA", true).unwrap();
    run_to(&mut m, 1205);
    m.key_event("KeyA", false).unwrap();
    run_to(&mut m, 1300);
    assert!(m.io_stop().is_none());
    let make = reads
        .iter()
        .position(|&v| v == 0x1E)
        .expect("make code read");
    assert!(
        reads[make..].contains(&0x9E),
        "break code read after the make: {reads:02X?}"
    );
}
