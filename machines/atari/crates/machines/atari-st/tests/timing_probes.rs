//! The timing probes (`machines/atari/timing/pNN_*.s`): 68000 programs started from a boot floppy that write their
//! measurements into RAM at `$C000`. The same floppy runs in Hatari 2.6.1 (a black box, `retroclawd-tools`
//! `scripts/capture_hatari_mem.py`), whose memory dump is the golden `RC_GOLDENS/atari-st/timing/pNN.bin`; ours must be
//! byte for byte the same (docs/timing-oracle.md). Without the floppy (`timing/build.sh`), EmuTOS or the golden a test
//! says so and passes.

mod common;
use common::{config, emutos};

use atari_st::{Accuracy, AtariSt, RamSize};
use machine::Machine;
use std::path::PathBuf;

/// Where a probe leaves its result.
pub const RESULT: u32 = 0xC000;

fn floppy(name: &str) -> Option<st_floppy::Image> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../software/scenarios/timing")
        .join(format!("{name}.st"));
    match std::fs::read(&path) {
        Ok(bytes) => st_floppy::Image::parse(&bytes).ok(),
        Err(_) => {
            eprintln!(
                "probe floppy {} missing: run timing/build.sh",
                path.display()
            );
            None
        }
    }
}

fn golden_bytes(name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(std::env::var_os("RC_GOLDENS")?)
        .join("atari-st/timing")
        .join(format!("{name}.bin"));
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!(
                "golden {} missing: run capture_hatari_mem.py",
                path.display()
            );
            None
        }
    }
}

/// Runs probe `name` on EmuTOS until VBL number `vbls` and returns `len` bytes from `RESULT`.
pub fn run_probe(name: &str, vbls: u64, len: usize, accuracy: Accuracy) -> Option<Vec<u8>> {
    let rom = emutos()?;
    let image = floppy(name)?;
    let mut cfg = config(rom, RamSize::M1);
    cfg.accuracy = accuracy;
    let mut m = AtariSt::new(cfg).unwrap();
    m.insert_disk(0, image).unwrap();
    while m.io_stop().is_none() && m.vbl_count() < vbls {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{name} stopped: {:?}", m.io_stop());
    Some((0..len as u32).map(|i| m.bus().peek(RESULT + i)).collect())
}

/// The probe's result equals Hatari's dump.
pub fn assert_matches_hatari(name: &str, vbls: u64, len: usize, accuracy: Accuracy) {
    let (Some(ours), Some(theirs)) = (run_probe(name, vbls, len, accuracy), golden_bytes(name))
    else {
        return;
    };
    assert_eq!(theirs.len(), len, "{name}: the golden has another length");
    if ours != theirs {
        let first = ours
            .iter()
            .zip(&theirs)
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "{name} differs from Hatari at byte {first} (of {len}): ours {:02X?} theirs {:02X?}",
            &ours[first..(first + 16).min(len)],
            &theirs[first..(first + 16).min(len)]
        );
    }
}

#[test]
fn p01_smoke_matches_hatari() {
    assert_matches_hatari("p01", 600, 16, Accuracy::Fast);
}

#[test]
fn p01_smoke_matches_hatari_in_accurate_mode() {
    assert_matches_hatari("p01", 600, 16, Accuracy::Accurate);
}

#[test]
fn p01_smoke_writes_its_marker() {
    // Independent of Hatari: the harness itself (floppy, boot sector, stub, load address) works.
    if let Some(got) = run_probe("p01", 600, 16, Accuracy::Fast) {
        assert_eq!(&got[..8], b"P1_SMOKE");
    }
}

/// P02: 48 variants of 64 lines (the first 64 lines after the VBL), one word a line, after the 4-byte header.
const P02_LINES: usize = 64;
const P02_VARIANTS: usize = 48;
const P02_BYTES: usize = 4 + P02_VARIANTS * P02_LINES * 2;

fn words(bytes: &[u8]) -> Vec<u16> {
    bytes[4..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|w| u16::from_be_bytes(*w))
        .collect()
}

/// The mean number of loop runs a line over the lines 6 to 64 of each variant: what the wait states add up to.
fn p02_mean(block: &[u16]) -> f64 {
    block[6..].iter().map(|&x| x as f64).sum::<f64>() / (block.len() - 6) as f64
}

/// Every variant's loop runs as often a line as in Hatari, to within two runs over the 58 lines compared. The exact
/// sequences of counts differ from line to line (manifest P02, known gap): they depend on where inside an instruction
/// the interrupt is taken, which the model of the acknowledge does not reproduce to the clock.
#[test]
fn p02_instruction_timing_agrees_with_hatari_on_the_mean() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p02", 1200, P02_BYTES, Accuracy::Accurate),
        golden_bytes("p02"),
    ) else {
        return;
    };
    let (ours, theirs) = (words(&ours), words(&theirs));
    for v in 0..P02_VARIANTS {
        let (a, b) = (
            &ours[v * P02_LINES..(v + 1) * P02_LINES],
            &theirs[v * P02_LINES..(v + 1) * P02_LINES],
        );
        let (ma, mb) = (p02_mean(a), p02_mean(b));
        assert!(
            (ma - mb).abs() <= 2.0 / (P02_LINES - 6) as f64 + 1e-9,
            "variant {v}: {ma:.3} runs a line here, {mb:.3} in Hatari"
        );
    }
}

/// `RC_GOLDENS=... cargo test -p atari-st --test timing_probes p02_report -- --ignored --nocapture`: per variant, the mean
/// count of loop runs a line here and in Hatari's dump.
#[test]
#[ignore]
fn p02_report() {
    let mode = if std::env::var_os("RC_FAST").is_some() {
        Accuracy::Fast
    } else {
        Accuracy::Accurate
    };
    let (Some(ours), Some(theirs)) = (run_probe("p02", 1200, P02_BYTES, mode), golden_bytes("p02"))
    else {
        return;
    };
    let (ours, theirs) = (words(&ours), words(&theirs));
    let (mut equal, mut total, mut mean_score) = (0, 0u64, 0.0f64);
    for v in 0..P02_VARIANTS {
        let a = &ours[v * P02_LINES..(v + 1) * P02_LINES];
        let b = &theirs[v * P02_LINES..(v + 1) * P02_LINES];
        let mean = |s: &[u16]| s[6..].iter().map(|&x| x as f64).sum::<f64>() / 57.0;
        mean_score += (mean(a) - mean(b)).abs() * 1000.0;
        let same = a == b;
        equal += same as u32;
        total += a[1..]
            .iter()
            .zip(&b[1..])
            .map(|(x, y)| (*x as i64 - *y as i64).unsigned_abs())
            .sum::<u64>();
        let d: u64 = a[1..]
            .iter()
            .zip(&b[1..])
            .map(|(x, y)| (*x as i64 - *y as i64).unsigned_abs())
            .sum();
        println!(
            "{v:2} ours {:8.3} theirs {:8.3} {} d={d}",
            mean(a),
            mean(b),
            if same { "=" } else { "DIFFERENT" }
        );
    }
    if let Ok(v) = std::env::var("RC_SHOW") {
        let v: usize = v.parse().unwrap();
        println!("ours   {:?}", &ours[v * P02_LINES..(v + 1) * P02_LINES]);
        println!("theirs {:?}", &theirs[v * P02_LINES..(v + 1) * P02_LINES]);
    }
    let first: u64 = (0..P02_VARIANTS)
        .map(|v| {
            (0..3)
                .map(|i| {
                    (ours[v * P02_LINES + i] as i64 - theirs[v * P02_LINES + i] as i64)
                        .unsigned_abs()
                })
                .sum::<u64>()
        })
        .sum();
    println!("SCORE equal {equal} abs_diff {total} mean_score {mean_score:.0} first3 {first}");
}

/// P04: five-byte samples (frame, line, counter high, mid, low) after the 4-byte header, up to a line of 0.
const P04_BYTES: usize = 4 + 5 * 800;

fn p04_samples(bytes: &[u8]) -> Vec<(u8, u8, u32)> {
    bytes[4..]
        .as_chunks::<5>()
        .0
        .iter()
        .take_while(|s| s[1] != 0)
        .map(|s| {
            (
                s[0],
                s[1],
                (s[2] as u32) << 16 | (s[3] as u32) << 8 | s[4] as u32,
            )
        })
        .collect()
}

/// The video address counter against Hatari's, sampled at four target lines along the frame (probe P04): above and below
/// the picture and on a line before its first row it holds the same address exactly; inside the picture it is within
/// one word (2 bytes) of Hatari's, for every one of the 128 starting points in the line.
#[test]
fn p04_video_counter_agrees_with_hatari() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p04", 700, P04_BYTES, Accuracy::Accurate),
        golden_bytes("p04"),
    ) else {
        return;
    };
    let (ours, theirs) = (p04_samples(&ours), p04_samples(&theirs));
    for line in [10u8, 34, 40, 100, 250] {
        let pick = |v: &[(u8, u8, u32)]| -> Vec<(u8, u32)> {
            v.iter()
                .filter(|s| s.1 == line && (1..=128).contains(&s.0))
                .map(|s| (s.0, s.2))
                .collect()
        };
        let (a, b) = (pick(&ours), pick(&theirs));
        assert_eq!(a.len(), 128, "line {line}: our samples");
        assert_eq!(b.len(), 128, "line {line}: Hatari's samples");
        let tolerance = if line >= 100 { 2 } else { 0 };
        for (x, y) in a.iter().zip(&b) {
            let d = (x.1 as i64 - y.1 as i64).abs();
            assert!(
                d <= tolerance,
                "line {line}, frame {}: counter {:06X} here, {:06X} in Hatari",
                x.0,
                x.1,
                y.1
            );
        }
    }
}

/// `cargo test ... p04_report -- --ignored --nocapture`: per sampled line, how the counter we read differs from Hatari's.
#[test]
#[ignore]
fn p04_report() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p04", 700, P04_BYTES, Accuracy::Accurate),
        golden_bytes("p04"),
    ) else {
        return;
    };
    let (ours, theirs) = (p04_samples(&ours), p04_samples(&theirs));
    let mut total = 0u64;
    for line in [10u8, 40, 100, 250, 34] {
        let pick = |v: &[(u8, u8, u32)]| -> Vec<(u8, u32)> {
            v.iter()
                .filter(|s| s.1 == line && (1..=128).contains(&s.0))
                .map(|s| (s.0, s.2))
                .collect()
        };
        let (a, b) = (pick(&ours), pick(&theirs));
        let n = a.len().min(b.len());
        let diffs: Vec<i64> = (0..n).map(|i| a[i].1 as i64 - b[i].1 as i64).collect();
        let abs: i64 = diffs.iter().map(|d| d.abs()).sum();
        total += abs as u64;
        if std::env::var_os("RC_SHOW").is_some() {
            println!("ours   {line}: {:X?}", a.iter().take(8).collect::<Vec<_>>());
            println!("theirs {line}: {:X?}", b.iter().take(8).collect::<Vec<_>>());
        }
        println!(
            "line {line}: {} of {} samples, sum |d| {abs}, first diffs {:?}",
            n,
            b.len(),
            &diffs[..diffs.len().min(12)]
        );
    }
    println!("SCORE equal 0 abs_diff {total} mean_score {total}");
}
