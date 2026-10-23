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
    for i in (30..40).chain(430..436) {
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
    let worst = o.iter().zip(&t).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 24, "the counters differ by up to {worst} bytes");
}
