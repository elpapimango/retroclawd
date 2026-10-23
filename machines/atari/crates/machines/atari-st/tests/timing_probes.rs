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
    run_probe_on(name, vbls, len, accuracy, st_shifter::Monitor::Colour)
}

/// `run_probe` on the given monitor.
pub fn run_probe_on(
    name: &str,
    vbls: u64,
    len: usize,
    accuracy: Accuracy,
    monitor: st_shifter::Monitor,
) -> Option<Vec<u8>> {
    let rom = emutos()?;
    let image = floppy(name)?;
    let mut cfg = config(rom, RamSize::M1);
    cfg.accuracy = accuracy;
    cfg.monitor = monitor;
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

/// P06: the MFP timer A data register against the time since the control register started the timer, for four
/// prescalers (/4, /10, /50, /200) and 64 starting points each.
/// All 256 reads of the timer are within one count of Hatari's and all but a few are equal (manifest P06, known gap).
#[test]
fn p06_mfp_timer_agrees_with_hatari() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p06", 700, 4 + 4 * 64, Accuracy::Accurate),
        golden_bytes("p06"),
    ) else {
        return;
    };
    let diffs: Vec<i32> = ours[4..]
        .iter()
        .zip(&theirs[4..])
        .map(|(a, b)| *a as i32 - *b as i32)
        .collect();
    assert!(diffs.iter().all(|d| d.abs() <= 1), "{diffs:?}");
    assert!(diffs.iter().filter(|d| **d != 0).count() <= 4, "{diffs:?}");
}

#[test]
#[ignore]
fn p06_report() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p06", 700, 4 + 4 * 64, Accuracy::Accurate),
        golden_bytes("p06"),
    ) else {
        return;
    };
    for m in 0..4 {
        let a = &ours[4 + m * 64..4 + (m + 1) * 64];
        let b = &theirs[4 + m * 64..4 + (m + 1) * 64];
        println!("mode {m} ours   {a:?}");
        println!("mode {m} theirs {b:?}");
    }
}

#[test]
#[ignore]
fn p06_score() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p06", 700, 4 + 4 * 64, Accuracy::Accurate),
        golden_bytes("p06"),
    ) else {
        return;
    };
    let d: u64 = ours[4..]
        .iter()
        .zip(&theirs[4..])
        .map(|(a, b)| (*a as i64 - *b as i64).unsigned_abs())
        .sum();
    let eq = (0..4)
        .filter(|m| ours[4 + m * 64..4 + (m + 1) * 64] == theirs[4 + m * 64..4 + (m + 1) * 64])
        .count();
    println!("SCORE equal {eq} abs_diff {d}");
}

/// `cargo test --release -p atari-st --test timing_probes border_log -- --ignored --nocapture` with `RC_PROBE=p09`: for each
/// frame of a border sweep, the clocks into line 100 at which the device saw the two register writes.
#[test]
#[ignore]
fn border_log() {
    let name = std::env::var("RC_PROBE").unwrap_or_else(|_| "p09".to_string());
    let (Some(rom), Some(image)) = (emutos(), floppy(&name)) else {
        return;
    };
    let mut cfg = config(rom, RamSize::M1);
    cfg.accuracy = Accuracy::Accurate;
    let mut m = AtariSt::new(cfg).unwrap();
    m.insert_disk(0, image).unwrap();
    let mut frame = 0u32;
    while m.vbl_count() < 700 {
        m.step();
        if m.line() == 140 {
            let writes: Vec<_> = m
                .bus()
                .shifter()
                .sync_log()
                .filter(|w| w.line == 100)
                .map(|w| w.cycle)
                .collect();
            if !writes.is_empty() {
                frame += 1;
                println!("frame {} writes at {:?}", frame - 1, writes);
                while m.line() == 140 {
                    m.step();
                }
            }
        }
    }
}

/// The sweep probes P09 to P11 record four bytes a frame (the frame, then the counter high, mid, low).
fn sweep_counters(bytes: &[u8]) -> Vec<(u8, u32)> {
    bytes[4..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| (c[0], (c[1] as u32) << 16 | (c[2] as u32) << 8 | c[3] as u32))
        .collect()
}

/// The frames of a sweep where the counter differs from the control's by more than noise, with the difference.
fn sweep_effects(
    name: &str,
    control: &str,
    accuracy: Accuracy,
    source: Option<&[u8]>,
) -> Vec<(usize, i64)> {
    let bytes = |n: &str| -> Option<Vec<u8>> {
        match (source, n == name) {
            (Some(b), true) => Some(b.to_vec()),
            _ => run_probe(n, 700, 4 + 4 * 128, accuracy),
        }
    };
    let (Some(t), Some(c)) = (bytes(name), bytes(control)) else {
        return Vec::new();
    };
    sweep_counters(&t)
        .iter()
        .zip(sweep_counters(&c).iter())
        .enumerate()
        .filter(|(_, (a, _))| a.0 != 0)
        .filter_map(|(i, (a, b))| {
            let d = a.1 as i64 - b.1 as i64;
            (d.abs() > 8).then_some((i, d))
        })
        .collect()
}

#[test]
#[ignore]
fn border_report() {
    let name = std::env::var("RC_PROBE").unwrap_or_else(|_| "p09".to_string());
    let ours = sweep_effects(&name, "p11", Accuracy::Accurate, None);
    println!("ours   {ours:?}");
    let (Some(t), Some(c)) = (golden_bytes(&name), golden_bytes("p11")) else {
        return;
    };
    let hatari = sweep_counters(&t)
        .iter()
        .zip(sweep_counters(&c).iter())
        .enumerate()
        .filter(|(_, (a, _))| a.0 != 0)
        .filter_map(|(i, (a, b))| {
            let d = a.1 as i64 - b.1 as i64;
            (d.abs() > 8).then_some((i, d))
        })
        .collect::<Vec<_>>();
    println!("hatari {hatari:?}");
}

/// P12 and P13 record seven bytes a frame: the frame, the counter at line 120, the counter at line 300.
fn vertical_counters(bytes: &[u8]) -> Vec<(i64, i64)> {
    bytes[4..]
        .as_chunks::<7>()
        .0
        .iter()
        .map(|c| {
            (
                ((c[1] as i64) << 16 | (c[2] as i64) << 8 | c[3] as i64),
                ((c[4] as i64) << 16 | (c[5] as i64) << 8 | c[6] as i64),
            )
        })
        .collect()
}

fn vertical_effects(test: &[u8], control: &[u8]) -> Vec<(usize, i64, i64)> {
    vertical_counters(test)
        .iter()
        .zip(vertical_counters(control).iter())
        .enumerate()
        .filter_map(|(i, (a, b))| {
            let (d1, d2) = (a.0 - b.0, a.1 - b.1);
            (d1.abs() > 8 || d2.abs() > 8).then_some((10 + i, d1, d2))
        })
        .collect()
}

#[test]
#[ignore]
fn vertical_report() {
    let (Some(t), Some(c)) = (
        run_probe("p12", 900, 4 + 7 * 256, Accuracy::Accurate),
        run_probe("p13", 900, 4 + 7 * 256, Accuracy::Accurate),
    ) else {
        return;
    };
    println!("ours   {:?}", vertical_effects(&t, &c));
    if let (Some(ht), Some(hc)) = (golden_bytes("p12"), golden_bytes("p13")) {
        println!("hatari {:?}", vertical_effects(&ht, &hc));
    }
}

/// The border effects the model reproduces: a line that opened its right border fetches 44 bytes more, one that opened its
/// left border 26 more. The frames where that happens are the same as Hatari's (P09: the resolution switched to high and
/// back, P10: the sync mode to 60 Hz and back), to the frame. What Hatari also does and the model does not (cuts a line
/// short, loses a line: effects of -106 and -160 bytes) is not compared; manifest P09 says so.
#[test]
fn p09_and_p10_open_the_borders_on_the_frames_hatari_does() {
    let kinds = |effects: Vec<(usize, i64)>| -> Vec<(usize, i64)> {
        effects
            .into_iter()
            .filter_map(|(i, d)| {
                if (d - 44).abs() <= 4 {
                    Some((i, 44))
                } else if (d - 26).abs() <= 4 {
                    Some((i, 26))
                } else {
                    None
                }
            })
            .collect()
    };
    for name in ["p09", "p10"] {
        let Some(t) = golden_bytes(name) else {
            return;
        };
        let Some(c) = golden_bytes("p11") else {
            return;
        };
        let hatari = kinds(
            sweep_counters(&t)
                .iter()
                .zip(sweep_counters(&c).iter())
                .enumerate()
                .filter(|(_, (a, _))| a.0 != 0)
                .filter_map(|(i, (a, b))| {
                    let d = a.1 as i64 - b.1 as i64;
                    (d.abs() > 8).then_some((i, d))
                })
                .collect(),
        );
        let ours = kinds(sweep_effects(name, "p11", Accuracy::Accurate, None));
        assert!(!hatari.is_empty(), "{name}: Hatari's sweep shows no border");
        assert_eq!(ours, hatari, "{name}");
    }
}

/// The picture window: the top border opens when the line before the picture starts in 60 Hz, the picture is missed when
/// the line it should start on starts in 60 Hz after that, the bottom ends at line 234 for a 60 Hz line there and goes on
/// past line 263 when that line starts in 60 Hz (P12; line 257 in Hatari's sweep is not modelled).
#[test]
fn p12_moves_the_picture_window_on_the_lines_hatari_does() {
    let (Some(ht), Some(hc)) = (golden_bytes("p12"), golden_bytes("p13")) else {
        return;
    };
    let (Some(t), Some(c)) = (
        run_probe("p12", 900, 4 + 7 * 256, Accuracy::Accurate),
        run_probe("p13", 900, 4 + 7 * 256, Accuracy::Accurate),
    ) else {
        return;
    };
    let modelled = [33usize, 62, 233, 262];
    let pick = |e: Vec<(usize, i64, i64)>| -> Vec<(usize, i64)> {
        e.into_iter()
            .filter(|(x, _, _)| modelled.contains(x))
            .map(|(x, _, d2)| (x, d2 / 100))
            .collect()
    };
    let (ours, theirs) = (
        pick(vertical_effects(&t, &c)),
        pick(vertical_effects(&ht, &hc)),
    );
    assert_eq!(theirs.len(), 4, "{theirs:?}");
    assert_eq!(ours, theirs);
}

/// P14 (mono frame length): ten loop counts of frames with the HBL masked, then the HBLs of eleven frames.
fn p14(accuracy: Accuracy) -> Option<(Vec<u32>, Vec<u16>)> {
    let b = run_probe_on("p14", 700, 66, accuracy, st_shifter::Monitor::Mono)?;
    assert_eq!(&b[..4], b"P14 ");
    let loops = b[4..44]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect();
    let hbls = b[44..66]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_be_bytes(*c))
        .collect();
    Some((loops, hbls))
}

#[test]
fn p14_the_mono_frame_has_the_lines_and_length_hatari_has() {
    let Some(theirs) = golden_bytes("p14") else {
        return;
    };
    let (loops, hbls) = p14(Accuracy::Accurate).unwrap();
    // The first interval holds the HBL that was pending when the mask came off: Hatari counts one more there (ledger).
    assert_eq!(hbls[1..], [501; 10], "lines in a frame");
    // A frame is 501 x 224 clocks: the counted loop (20 clocks an iteration) fills it but the VBL handler's few hundred.
    let their_loops: Vec<u32> = theirs[4..44]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect();
    let mean = |v: &[u32]| v.iter().sum::<u32>() as f64 / v.len() as f64;
    assert!(
        (mean(&loops) - mean(&their_loops)).abs() < 1.0,
        "loop counts ours {loops:?} theirs {their_loops:?}"
    );
}

/// P15 as the 3-byte counters of the frame's HBLs, as addresses.
fn counters(b: &[u8]) -> Vec<u32> {
    b[4..]
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes([0, c[0], c[1], c[2]]))
        .collect()
}

#[test]
#[ignore = "report"]
fn p15_report() {
    let (Some(ours), Some(theirs)) = (
        run_probe_on(
            "p15",
            700,
            1508,
            Accuracy::Accurate,
            st_shifter::Monitor::Mono,
        ),
        golden_bytes("p15"),
    ) else {
        return;
    };
    let (o, t) = (counters(&ours), counters(&theirs));
    let first = |v: &[u32]| v.windows(2).position(|w| w[0] != w[1]);
    println!("first change ours {:?} theirs {:?}", first(&o), first(&t));
    let last = |v: &[u32]| v.windows(2).rposition(|w| w[0] != w[1]);
    println!("last change ours {:?} theirs {:?}", last(&o), last(&t));
    let diff: i64 = o
        .iter()
        .zip(&t)
        .map(|(a, b)| (*a as i64 - *b as i64).abs())
        .sum();
    println!("sum |ours - theirs| = {diff}");
    for i in (0..501).filter(|&i| o[i].abs_diff(t[i]) > 24) {
        println!("{i}: ours {:X} theirs {:X}", o[i], t[i]);
    }
}

#[test]
fn p15_the_mono_picture_window_and_fetch_rate_match_hatari() {
    let (Some(ours), Some(theirs)) = (
        run_probe_on(
            "p15",
            700,
            1508,
            Accuracy::Accurate,
            st_shifter::Monitor::Mono,
        ),
        golden_bytes("p15"),
    ) else {
        return;
    };
    let (o, t) = (counters(&ours), counters(&theirs));
    let edges = |v: &[u32]| {
        (
            v.windows(2).position(|w| w[0] != w[1]),
            v.windows(2).rposition(|w| w[0] != w[1]),
            v[v.len() - 1] - v[0],
        )
    };
    // Where the picture starts and stops being fetched and how much it holds (400 lines of 80 bytes) are equal; inside a
    // line the counter differs by the few clocks the interrupt that reads it is placed differently.
    assert_eq!(edges(&o), edges(&t));
    // The three counter bytes are read one after the other, so a read that straddles a carry of the middle byte tears (a
    // difference of about 256, on one sample in 80 here and there); the rest agree to a few clocks of interrupt placement.
    let off = o
        .iter()
        .zip(&t)
        .filter(|(a, b)| a.abs_diff(**b) > 24)
        .count();
    assert!(
        off <= 8,
        "{off} of {} samples differ by more than 24 bytes",
        o.len()
    );
}

/// P16: 30 records of 8 bytes: the frame's loop count, the GPIP byte, the DMA status, the DMA address (mid, low).
fn p16_records(b: &[u8]) -> Vec<(u32, u8, u16)> {
    b[4..]
        .as_chunks::<8>()
        .0
        .iter()
        .map(|r| {
            (
                u32::from_be_bytes([r[0], r[1], r[2], r[3]]),
                r[5],
                u16::from_be_bytes([r[6], r[7]]),
            )
        })
        .collect()
}

#[test]
#[ignore = "report"]
fn p16_report() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p16", 700, 244, Accuracy::Accurate),
        golden_bytes("p16"),
    ) else {
        return;
    };
    for (i, (o, t)) in p16_records(&ours)
        .iter()
        .zip(p16_records(&theirs))
        .enumerate()
    {
        println!("frame {}: ours {:?}  theirs {:?}", i + 2, o, t);
    }
}

/// The clocks a counted loop (20 clocks an iteration, 8002 an undisturbed frame) loses in a frame per 16-byte burst the DMA
/// moved in it, over the frames that are inside a transfer (the address moved before and after them).
fn p16_clocks_per_burst(records: &[(u32, u8, u16)]) -> Vec<u32> {
    let mut out = Vec::new();
    for w in records.windows(3) {
        let moved = |a: u16, b: u16| b.wrapping_sub(a) as u32;
        let (before, now) = (moved(w[0].2, w[1].2), moved(w[1].2, w[2].2));
        if before > 0 && now > 0 && w[1].2 != 0x1200 {
            let bursts = now / 16;
            out.push((8002u32.saturating_sub(w[2].0)) * 20 / bursts.max(1));
        }
    }
    out
}

#[test]
fn p16_a_dma_burst_holds_the_cpu_for_the_clocks_hatari_shows() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p16", 700, 244, Accuracy::Accurate),
        golden_bytes("p16"),
    ) else {
        return;
    };
    let (o, t) = (
        p16_clocks_per_burst(&p16_records(&ours)),
        p16_clocks_per_burst(&p16_records(&theirs)),
    );
    assert!(
        o.len() >= 5 && t.len() >= 5,
        "frames inside a transfer: {o:?} {t:?}"
    );
    let mean = |v: &[u32]| v.iter().sum::<u32>() as f64 / v.len() as f64;
    assert!(
        (mean(&o) - mean(&t)).abs() <= 3.0,
        "clocks per burst ours {o:?} theirs {t:?}"
    );
}

#[test]
fn p16_in_fast_mode_the_dma_does_not_slow_the_loop() {
    let Some(ours) = run_probe("p16", 700, 244, Accuracy::Fast) else {
        return;
    };
    let loops: Vec<u32> = p16_records(&ours).iter().skip(3).map(|r| r.0).collect();
    assert!(loops.iter().all(|&l| l >= 7995), "{loops:?}");
}

/// P17: 16 stretches of the index bit (longs), then eight (loops to the interrupt request, sector) pairs of longs.
fn p17_values(b: &[u8]) -> (Vec<u32>, Vec<(u32, u32)>) {
    let longs: Vec<u32> = b[4..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect();
    let pairs = longs[16..].chunks(2).map(|c| (c[0], c[1])).collect();
    (longs[..16].to_vec(), pairs)
}

#[test]
#[ignore = "report"]
fn p17_report() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p17", 700, 196, Accuracy::Accurate),
        golden_bytes("p17"),
    ) else {
        return;
    };
    println!("ours   {:?}", p17_values(&ours));
    println!("theirs {:?}", p17_values(&theirs));
}

const P18_NAMES: [&str; 16] = [
    "nop",
    "fdc_read",
    "fdc_write",
    "dma_mode_write",
    "dma_status_read",
    "dma_addr_read",
    "dma_addr_write",
    "mfp_read",
    "psg_read",
    "acia_read",
    "shifter_read",
    "rom_read",
    "acia_read_nop1",
    "acia_read_nop2",
    "acia_read_nop3",
    "acia_read_nop5",
];

/// P18: per variant three loop counts of a frame, as the clocks an iteration takes.
fn p18_clocks(b: &[u8]) -> Vec<u32> {
    b[4..]
        .as_chunks::<12>()
        .0
        .iter()
        .map(|c| 160_256 / u32::from_be_bytes([c[4], c[5], c[6], c[7]]))
        .collect()
}

#[test]
fn p18_an_access_to_each_register_costs_the_clocks_hatari_shows() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p18", 800, 196, Accuracy::Accurate),
        golden_bytes("p18"),
    ) else {
        return;
    };
    let (o, t) = (p18_clocks(&ours), p18_clocks(&theirs));
    let table: Vec<_> = P18_NAMES.iter().zip(o.iter().zip(&t)).collect();
    assert_eq!(o, t, "clocks an iteration, ours and theirs: {table:?}");
}

/// P19: five longs, the loop counts of a 20-track seek at the four step rates and of a restore from track 20.
fn p19_values(b: &[u8]) -> Vec<u32> {
    b[4..24]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect()
}

#[test]
fn p19_the_step_rates_take_the_times_hatari_shows() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p19", 800, 24, Accuracy::Accurate),
        golden_bytes("p19"),
    ) else {
        return;
    };
    let (o, t) = (p19_values(&ours), p19_values(&theirs));
    // 20 steps at 6, 12, 2 and 3 ms: the data sheet's times at the FDC's 8 MHz; the two emulators place the controller's
    // internal delays (settle, verify setup) a few hundred clocks apart, 0.25% at most.
    for (i, (a, b)) in o.iter().zip(&t).enumerate() {
        assert!(
            a.abs_diff(*b) <= 3 + b / 400,
            "case {i}: ours {o:?} theirs {t:?}"
        );
    }
}

#[test]
fn p17_the_index_pulse_and_the_sector_pitch_are_hatari_s() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p17", 800, 196, Accuracy::Accurate),
        golden_bytes("p17"),
    ) else {
        return;
    };
    let ((os, op), (ts, tp)) = (p17_values(&ours), p17_values(&theirs));
    // Stretches of the index bit after the first (which starts at an arbitrary point of the revolution): low then high.
    for (i, (a, b)) in os.iter().zip(&ts).enumerate().skip(2) {
        let limit = if i % 2 == 0 { 5 } else { 3 };
        assert!(
            a.abs_diff(*b) <= limit,
            "stretch {i}: ours {os:?} theirs {ts:?}"
        );
    }
    // The loop count from a Read Address command to the next header, once the previous one has just been read.
    for (i, (a, b)) in op.iter().zip(&tp).enumerate().skip(2).take(6) {
        assert!(
            a.0.abs_diff(b.0) <= 8,
            "header {i}: ours {op:?} theirs {tp:?}"
        );
    }
}

/// P20: per case (12) eight loop counts (longs) and eight bytes.
fn p20_cases(b: &[u8]) -> Vec<(Vec<u32>, Vec<u8>)> {
    b[4..]
        .chunks(40)
        .map(|c| {
            let counts = c[..32]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|x| u32::from_be_bytes(*x))
                .collect();
            (counts, c[32..40].to_vec())
        })
        .collect()
}

#[test]
#[ignore = "report"]
fn p20_report() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p20", 3000, 484, Accuracy::Accurate),
        golden_bytes("p20"),
    ) else {
        return;
    };
    for (i, (o, t)) in p20_cases(&ours).iter().zip(p20_cases(&theirs)).enumerate() {
        println!("case {i}: ours   {:?} {:02X?}", o.0, o.1);
        println!("case {i}: theirs {:?} {:02X?}", t.0, t.1);
    }
}

#[test]
fn p20_the_keyboard_answers_with_the_latencies_and_byte_spacing_hatari_shows() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p20", 3000, 484, Accuracy::Accurate),
        golden_bytes("p20"),
    ) else {
        return;
    };
    let (o, t) = (p20_cases(&ours), p20_cases(&theirs));
    assert_eq!(o.len(), 12);
    for (i, (a, b)) in o.iter().zip(&t).enumerate() {
        // The answers' bytes are the same except the clock's seconds (the clock runs from another start).
        assert_eq!(a.1[0], b.1[0], "case {i}: first answer byte");
        let answered =
            |c: &(Vec<u32>, Vec<u8>)| c.0.iter().take_while(|&&n| n < 100_000 && n != 0).count();
        assert_eq!(answered(a), answered(b), "case {i}: number of answer bytes");
        // The first answer: Hatari's own figure varies by about 30 loop runs between starts; the rest is a byte time apart.
        let limit = if i == 0 { 40 } else { 70 };
        assert!(
            a.0[0].abs_diff(b.0[0]) <= limit,
            "case {i}: first answer ours {:?} theirs {:?}",
            a.0,
            b.0
        );
        for k in 1..answered(a) {
            assert!(
                a.0[k].abs_diff(b.0[k]) <= 2,
                "case {i}: byte {k} ours {:?} theirs {:?}",
                a.0,
                b.0
            );
        }
    }
}

/// P21: eight loop counts of a byte at 9600 baud, then eight at 1200.
fn p21_values(b: &[u8]) -> Vec<u32> {
    b[4..68]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect()
}

#[test]
#[ignore = "report"]
fn p21_report() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p21", 1500, 68, Accuracy::Accurate),
        golden_bytes("p21"),
    ) else {
        return;
    };
    println!("ours   {:?}", p21_values(&ours));
    println!("theirs {:?}", p21_values(&theirs));
}

/// Hatari 2.6.1 transmits at once when no RS-232 device is attached (every wait above is one loop run), so there is no
/// timing to copy; ours follows the MC68901 data sheet: a frame is 10 bits of 16 timer D periods x 2, so 1200 baud takes
/// eight times as long as 9600.
#[test]
fn p21_the_transmitter_follows_the_baud_rate_the_data_sheet_gives() {
    let Some(ours) = run_probe("p21", 1500, 68, Accuracy::Accurate) else {
        return;
    };
    let v = p21_values(&ours);
    let (fast, slow) = (&v[2..8], &v[10..16]);
    let (f, s) = (
        fast.iter().sum::<u32>() as f64 / 6.0,
        slow.iter().sum::<u32>() as f64 / 6.0,
    );
    assert!((s / f - 8.0).abs() < 0.05, "{fast:?} {slow:?}");
    // 9600 baud is 8 355 clocks a byte; a loop run of the poll is about 40 clocks.
    assert!((f * 40.0 - 8_355.0).abs() < 250.0, "{fast:?}");
}

/// Runs the floppy probe `name` with a blank 1 MB hard disk on ACSI target 0 (no partitions, so EmuTOS still boots the
/// floppy); the disk's data does not matter to the timing.
fn run_probe_hd(name: &str, vbls: u64, len: usize, accuracy: Accuracy) -> Option<Vec<u8>> {
    let rom = emutos()?;
    let image = floppy(name)?;
    let mut cfg = config(rom, RamSize::M1);
    cfg.accuracy = accuracy;
    let mut m = AtariSt::new(cfg).unwrap();
    m.insert_disk(0, image).unwrap();
    m.attach_hard_disk(0, vec![0; 1 << 20]).unwrap();
    while m.io_stop().is_none() && m.vbl_count() < vbls {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{name} stopped: {:?}", m.io_stop());
    Some((0..len as u32).map(|i| m.bus().peek(RESULT + i)).collect())
}

#[test]
#[ignore = "report"]
fn p22_report() {
    let Some(ours) = run_probe_hd("p22", 1500, 36, Accuracy::Accurate) else {
        return;
    };
    println!("ours {:02X?}", ours);
}

/// P22: four (loops to the completion request, DMA address after) pairs of longs: TEST UNIT READY, READ(6) of 1, 8 and 32 blocks.
fn p22_pairs(b: &[u8]) -> Vec<(u32, u32)> {
    let longs: Vec<u32> = b[4..36]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect();
    longs.chunks(2).map(|c| (c[0], c[1])).collect()
}

/// Hatari 2.6.1 finishes an ACSI command at once (the completion request is there at the first look), which is what the
/// Phase 2 model does; it also leaves the DMA address counter where it was, where the ACSI notes have the transfer move it
/// on: ours follows the notes (the difference is the second column of the probe).
#[test]
fn p22_an_acsi_command_is_done_at_once_as_in_hatari() {
    let (Some(ours), Some(theirs)) = (
        run_probe_hd("p22", 1500, 36, Accuracy::Accurate),
        golden_bytes("p22"),
    ) else {
        return;
    };
    let (o, t) = (p22_pairs(&ours), p22_pairs(&theirs));
    assert_eq!(
        o.iter().map(|p| p.0).collect::<Vec<_>>(),
        t.iter().map(|p| p.0).collect::<Vec<_>>(),
        "loops to completion: ours {o:?} theirs {t:?}"
    );
    // Ours moves the address by the blocks read: 0, 1, 8, 32 blocks of 512 bytes.
    assert_eq!(
        o.iter().map(|p| p.1).collect::<Vec<_>>(),
        vec![0x20000, 0x20200, 0x21000, 0x24000]
    );
}

/// P23: twelve longs: TOS's 200 Hz counter and frame counter at the body's start and after a counted loop, then eight more
/// 200 Hz counters.
fn p23_values(b: &[u8]) -> Vec<u32> {
    b[4..52]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect()
}

#[test]
#[ignore = "report"]
fn p23_report() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p23", 600, 52, Accuracy::Accurate),
        golden_bytes("p23"),
    ) else {
        return;
    };
    println!("ours   {:?}", p23_values(&ours));
    println!("theirs {:?}", p23_values(&theirs));
}

/// P24: twelve longs, the summed waits of eight single-sector reads for twelve pauses between them.
fn p24_values(b: &[u8]) -> Vec<u32> {
    b[4..52]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect()
}

#[test]
#[ignore = "report"]
fn p24_report() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p24", 2500, 52, Accuracy::Accurate),
        golden_bytes("p24"),
    ) else {
        return;
    };
    println!("ours   {:?}", p24_values(&ours));
    println!("theirs {:?}", p24_values(&theirs));
}

#[test]
#[ignore = "report"]
fn p25_report() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p25", 1500, 132, Accuracy::Accurate),
        golden_bytes("p25"),
    ) else {
        return;
    };
    let pairs = |b: &[u8]| -> Vec<(u32, u32)> {
        b[4..]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| {
                (
                    u32::from_be_bytes([c[0], c[1], c[2], c[3]]),
                    u32::from_be_bytes([c[4], c[5], c[6], c[7]]),
                )
            })
            .collect()
    };
    println!("ours   {:X?}", pairs(&ours));
    println!("theirs {:X?}", pairs(&theirs));
}

/// P26: five (loops to the interrupt request of a Read Address after an idle time, status) pairs of longs: idle times 0.25,
/// 1.25, 2.5, 2.5 and 5 seconds.
fn p26_pairs(b: &[u8]) -> Vec<(u32, u32)> {
    b[4..44]
        .chunks(8)
        .map(|c| {
            (
                u32::from_be_bytes([c[0], c[1], c[2], c[3]]),
                u32::from_be_bytes([c[4], c[5], c[6], c[7]]),
            )
        })
        .collect()
}

#[test]
#[ignore = "report"]
fn p26_report() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p26", 3000, 44, Accuracy::Accurate),
        golden_bytes("p26"),
    ) else {
        return;
    };
    println!("ours   {:?}", p26_pairs(&ours));
    println!("theirs {:?}", p26_pairs(&theirs));
}

#[test]
fn p24_consecutive_sector_reads_catch_the_next_header_as_in_hatari() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p24", 2500, 52, Accuracy::Accurate),
        golden_bytes("p24"),
    ) else {
        return;
    };
    let (o, t) = (p24_values(&ours), p24_values(&theirs));
    // The summed waits of eight reads, about 33 000 runs, for twelve pauses: a header missed would add a revolution
    // (36 000 runs); the interrupt request has to come when the sector ends, not when the frame does.
    for (i, (a, b)) in o.iter().zip(&t).enumerate() {
        assert!(a.abs_diff(*b) <= 150, "pause {i}: ours {o:?} theirs {t:?}");
    }
}

#[test]
fn p23_the_floppy_boot_takes_as_long_as_in_hatari() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p23", 600, 52, Accuracy::Accurate),
        golden_bytes("p23"),
    ) else {
        return;
    };
    let (o, t) = (p23_values(&ours), p23_values(&theirs));
    // TOS's 200 Hz counter when the body starts: within 60 ticks (0.3 s: the revolution phase of the sector reads).
    assert!(
        o[0].abs_diff(t[0]) <= 60,
        "body start: ours {o:?} theirs {t:?}"
    );
    // The counted loops after it take the same ticks: 100 000 runs, then 10 000 runs eight times.
    assert!(
        (o[2] - o[0]).abs_diff(t[2] - t[0]) <= 2,
        "loop: ours {o:?} theirs {t:?}"
    );
    for i in 5..12 {
        assert!(
            (o[i] - o[i - 1]).abs_diff(t[i] - t[i - 1]) <= 1,
            "step {i}: ours {o:?} theirs {t:?}"
        );
    }
}

#[test]
fn p26_the_motor_spins_up_for_six_revolutions_after_a_long_idle_as_in_hatari() {
    let (Some(ours), Some(theirs)) = (
        run_probe("p26", 3000, 44, Accuracy::Accurate),
        golden_bytes("p26"),
    ) else {
        return;
    };
    let (o, t) = (p26_pairs(&ours), p26_pairs(&theirs));
    for i in 0..2 {
        assert!(o[i].0 < 40_000, "no spin-up after a short idle: {o:?}");
    }
    for i in 2..5 {
        assert!(
            o[i].0.abs_diff(t[i].0) <= t[i].0 / 60,
            "idle case {i}: ours {o:?} theirs {t:?}"
        );
    }
}

#[test]
#[ignore = "report"]
fn boot_transitions_report() {
    let Some(rom) = emutos() else {
        return;
    };
    let mut cfg = config(rom, RamSize::M1);
    cfg.accuracy = Accuracy::Accurate;
    let mut m = AtariSt::new(cfg).unwrap();
    let mut last = 0u64;
    let mut frame = 0;
    while m.io_stop().is_none() && m.vbl_count() < 1000 {
        m.step();
        if m.vbl_count() != frame {
            frame = m.vbl_count();
            if frame % 5 == 0 {
                let rgb = m.render_frame().unwrap().2;
                let h = rgb.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
                    (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
                });
                if h != last {
                    println!("frame {frame}: picture changed ({h:016x})");
                    last = h;
                }
            }
        }
    }
}

/// Runs STE probe `name` (Hatari `--machine ste`, EmuTOS 256K from the cache or `RC_EMUTOS_STE`) and returns `len` bytes.
pub fn run_probe_ste(name: &str, vbls: u64, len: usize, accuracy: Accuracy) -> Option<Vec<u8>> {
    let rom = common::emutos_ste()?;
    let image = floppy(name)?;
    let mut cfg = config(rom, RamSize::M1);
    cfg.accuracy = accuracy;
    let mut m = AtariSt::with_chipset(cfg, atari_st::Chipset::Ste).unwrap();
    m.insert_disk(0, image).unwrap();
    while m.io_stop().is_none() && m.vbl_count() < vbls {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{name} stopped: {:?}", m.io_stop());
    Some((0..len as u32).map(|i| m.bus().peek(RESULT + i)).collect())
}

fn golden_ste(name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(std::env::var_os("RC_GOLDENS")?)
        .join("atari-ste/timing")
        .join(format!("{name}.bin"));
    std::fs::read(&path).ok()
}

#[test]
fn p30_the_microwire_data_register_shifts_out_as_in_hatari() {
    let (Some(ours), Some(theirs)) = (
        run_probe_ste("p30", 800, 340, Accuracy::Accurate),
        golden_ste("p30"),
    ) else {
        return;
    };
    let words = |b: &[u8]| -> Vec<u16> {
        b[4..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect()
    };
    let (o, t) = (words(&ours), words(&theirs));
    assert_eq!(o.len(), t.len());
    // Six experiments of 20 data words and 8 mask words. The mask register always reads what was written; the data
    // register's first two reads and its end (zero) are Hatari's; between them Hatari's shift per read varies (4, 3, 3, 3 bits
    // in the tightest loop) where ours is one bit per 8 clocks (a known gap, the manifest says so).
    for e in 0..6 {
        let (od, td) = (&o[28 * e..28 * e + 20], &t[28 * e..28 * e + 20]);
        let (om, tm) = (&o[28 * e + 20..28 * e + 28], &t[28 * e + 20..28 * e + 28]);
        assert_eq!(om, tm, "experiment {e}: the mask register");
        assert_eq!(od[..2], td[..2], "experiment {e}: the first two reads");
        assert_eq!(od[19], 0, "experiment {e}: shifted out");
        assert_eq!(td[19], 0);
    }
}

#[test]
fn p27_the_ste_video_registers_read_back_as_in_hatari() {
    let (Some(ours), Some(theirs)) = (
        run_probe_ste("p27", 700, 122, Accuracy::Accurate),
        golden_ste("p27"),
    ) else {
        return;
    };
    assert_eq!(&ours[..4], b"P27\0");
    // 11 byte registers of 8 values each, then 10 palette words and 5 byte writes read as words. The video counter's low
    // byte (the last register) moves with the beam between the write and the read: it is only checked to be within 16.
    let counter_low = 4 + 10 * 8..4 + 11 * 8;
    for i in 4..122 {
        if counter_low.contains(&i) {
            let (a, b) = (ours[i] as i32, theirs[i] as i32);
            assert!(
                (a - b).rem_euclid(256) <= 16 || (b - a).rem_euclid(256) <= 16,
                "byte {i}: ours {a:02X} theirs {b:02X}"
            );
        } else {
            assert_eq!(
                ours[i], theirs[i],
                "byte {i}: ours {:02X} theirs {:02X}",
                ours[i], theirs[i]
            );
        }
    }
}

/// P28: per setting three records of 9 bytes (the registers read back, the counter at line 100 and at line 110): the
/// median of the three strides, in bytes over the ten lines.
fn p28_strides(b: &[u8]) -> Vec<(Vec<u8>, i32)> {
    let rec = |i: usize| &b[4 + 9 * i..4 + 9 * i + 9];
    (0..21)
        .map(|k| {
            let mut s: Vec<i32> = (0..3)
                .map(|j| {
                    let r = rec(3 * k + j);
                    let at = |o: usize| {
                        (i32::from(r[o]) << 16) | (i32::from(r[o + 1]) << 8) | i32::from(r[o + 2])
                    };
                    at(6) - at(3)
                })
                .collect();
            s.sort_unstable();
            (rec(3 * k + 2)[..3].to_vec(), s[1])
        })
        .collect()
}

#[test]
fn p28_the_line_offset_and_scroll_change_the_fetch_as_in_hatari() {
    let (Some(ours), Some(theirs)) = (
        run_probe_ste("p28", 1100, 580, Accuracy::Accurate),
        golden_ste("p28"),
    ) else {
        return;
    };
    let (o, t) = (p28_strides(&ours), p28_strides(&theirs));
    for (k, (a, b)) in o.iter().zip(&t).enumerate() {
        assert_eq!(a.0, b.0, "setting {k}: the registers");
        // The three counter bytes are read one after the other: a read that straddles a carry of the middle byte is off by
        // 256 (in either emulator, at its own phase), which is not a difference of the fetch.
        let d = (a.1 - b.1).rem_euclid(256);
        assert!(
            d <= 24 || d >= 232,
            "setting {k} {:?}: ours {} theirs {}",
            a.0,
            a.1,
            b.1
        );
    }
}

#[test]
#[ignore = "report"]
fn p28_report() {
    let Some(ours) = run_probe_ste("p28", 1100, 580, Accuracy::Accurate) else {
        return;
    };
    for k in 0..63 {
        let r = &ours[4 + 9 * k..13 + 9 * k];
        let at =
            |o: usize| (u32::from(r[o]) << 16) | (u32::from(r[o + 1]) << 8) | u32::from(r[o + 2]);
        println!(
            "{k}: regs {:?} line100 {:06X} line110 {:06X} stride {}",
            &r[..3],
            at(3),
            at(6),
            at(6) as i64 - at(3) as i64
        );
    }
}

const P36_NAMES: [&str; 17] = [
    "nop",
    "shifter_palette_read",
    "ste_base_low_read",
    "ste_hscroll_write",
    "sound_ctrl_read",
    "sound_ctrl_write",
    "microwire_read",
    "microwire_mask_read",
    "joy_read",
    "joy_byte_read",
    "blitter_read",
    "blitter_halftone_read",
    "blitter_halftone_write",
    "rom_read",
    "mw_read_to_mem",
    "mem_read_to_mem",
    "mw_read_postinc",
];

#[test]
fn p36_an_access_to_each_ste_register_costs_the_clocks_hatari_shows() {
    let (Some(ours), Some(theirs)) = (
        run_probe_ste("p36", 1000, 208, Accuracy::Accurate),
        golden_ste("p36"),
    ) else {
        return;
    };
    let clocks = |b: &[u8]| -> Vec<u32> {
        b[4..]
            .as_chunks::<12>()
            .0
            .iter()
            .map(|c| 160_256 / u32::from_be_bytes([c[4], c[5], c[6], c[7]]))
            .collect()
    };
    let (o, t) = (clocks(&ours), clocks(&theirs));
    let table: Vec<_> = P36_NAMES.iter().zip(o.iter().zip(&t)).collect();
    assert_eq!(o, t, "clocks an iteration, ours and theirs: {table:?}");
}

/// The Blitter's dumps (P33 a checksum and the registers per setting, P37 to P39 raw destination words) must be Hatari's,
/// word for word: the engine's rules are all [O] (`docs/hardware-facts.md`, "Blitter").
fn blitter_dump_matches(name: &str, len: usize, what: &str) {
    let (Some(ours), Some(theirs)) = (
        run_probe_ste(name, 1500, len, Accuracy::Fast),
        golden_ste(name),
    ) else {
        return;
    };
    assert_eq!(theirs.len(), len, "{name} golden length");
    assert_eq!(&ours[..4], &theirs[..4], "{name} header");
    let record = (len - 4) / what.parse::<usize>().unwrap();
    let wrong: Vec<usize> = (0..what.parse().unwrap())
        .filter(|&i| {
            ours[4 + i * record..4 + (i + 1) * record]
                != theirs[4 + i * record..4 + (i + 1) * record]
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "{name}: settings that differ from Hatari: {wrong:?}"
    );
}

#[test]
fn p33_the_blitter_leaves_the_memory_and_registers_hatari_does_for_174_settings() {
    blitter_dump_matches("p33", 4 + 174 * 20, "174");
}

#[test]
fn p37_the_skew_and_its_flags_feed_the_destination_as_in_hatari() {
    blitter_dump_matches("p37", 4 + 64 * 80, "64");
}

#[test]
fn p38_end_masks_with_the_skew_flags_match_hatari() {
    blitter_dump_matches("p38", 4 + 27 * 80, "27");
}

#[test]
fn p39_a_blit_right_to_left_matches_hatari() {
    blitter_dump_matches("p39", 4 + 18 * 80, "18");
}

#[test]
fn p40_a_restart_with_the_y_count_at_zero_moves_nothing_as_in_hatari() {
    blitter_dump_matches("p40", 4 + 5 * 10, "5");
}

/// P29's records, decoded: (counter, control, gpip) per sample, eleven sets of 32 after the 48 bytes of read-back.
fn p29_records(b: &[u8]) -> Vec<Vec<(u32, u8, u8, u8)>> {
    (0..11)
        .map(|set| {
            (0..32)
                .map(|i| {
                    let o = 52 + set * 192 + i * 6;
                    let c = u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
                    (c >> 8, c as u8, b[o + 4], b[o + 5])
                })
                .collect()
        })
        .collect()
}

#[test]
fn p29_the_dma_sound_registers_and_counter_move_as_in_hatari() {
    let (Some(ours), Some(theirs)) = (
        run_probe_ste("p29", 1500, 2165, Accuracy::Accurate),
        golden_ste("p29"),
    ) else {
        return;
    };
    assert_eq!(&ours[..52], &theirs[..52], "register read-back");
    let (o, t) = (p29_records(&ours), p29_records(&theirs));
    for set in 0..11 {
        if std::env::var_os("RC_P29_REPORT").is_some() && o[set] != t[set] {
            eprintln!(
                "set {set}\n ours   {:x?}\n theirs {:x?}",
                &o[set][..14],
                &t[set][..14]
            );
        }
    }
    // Exact: the control register and GPIP line 7 at every record (when play stops, and that it does) and the frame ends
    // timer A counted. Known gap (`docs/hardware-facts.md`): Hatari moves the counter in lumps at scan lines with a start-up
    // ramp that differs per rate; ours is the bytes fetched ahead of the sample played, so the counter is compared within one
    // FIFO (16 bytes), circularly in the frame.
    let frame_len = [
        0x10000, 0x10000, 0x10000, 0x10000, 0x10000, 0x10000, 0x10000, 0x10000, 0x40, 0x40, 0x10,
    ];
    for set in 0..11 {
        for i in 0..32 {
            let (a, b) = (o[set][i], t[set][i]);
            assert_eq!(
                (a.2, a.3),
                (b.2, b.3),
                "set {set} record {i}: control and GPIP"
            );
            let d = a.0.abs_diff(b.0);
            let d = d.min(frame_len[set] - d.min(frame_len[set]));
            assert!(
                d <= 16,
                "set {set} record {i}: counter {:x} against Hatari's {:x}",
                a.0,
                b.0
            );
        }
    }
    assert_eq!(ours[2164], theirs[2164], "timer A data after the loop");
}

#[test]
fn p41_the_dma_sound_plays_bytes_at_hatari_s_rate_over_a_long_run() {
    let (Some(ours), Some(theirs)) = (
        run_probe_ste("p41", 600, 24, Accuracy::Accurate),
        golden_ste("p41"),
    ) else {
        return;
    };
    for mode in 0..5 {
        let at = |b: &[u8]| {
            i64::from(u32::from_be_bytes([
                0,
                b[4 + 4 * mode],
                b[5 + 4 * mode],
                b[6 + 4 * mode],
            ]))
        };
        let (a, b) = (at(&ours), at(&theirs));
        let d = (a - b).rem_euclid(0x10000);
        let d = d.min(0x10000 - d);
        assert!(d <= 16, "mode {mode}: counter {a:x} against Hatari's {b:x}");
    }
}
