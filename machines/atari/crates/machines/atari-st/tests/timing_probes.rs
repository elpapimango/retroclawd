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
