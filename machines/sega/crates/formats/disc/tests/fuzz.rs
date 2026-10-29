//! Deterministic fuzz-style tests: hostile cue text and truncated images must give errors or clamped reads, never panics.

use disc::builder::{self, BuildTrack, Fill};
use disc::cue::{self, TrackKind};
use disc::{Disc, RAW_SECTOR};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

const VALID: &str = "FILE \"a.bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    PREGAP 00:02:00\n    INDEX 00 00:10:00\n    INDEX 01 00:12:00\n";

#[test]
fn random_cue_text_never_panics() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for _ in 0..5000 {
        let len = (rng.next() % 300) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let _ = cue::parse(&String::from_utf8_lossy(&bytes));
    }
}

#[test]
fn mutated_valid_cues_never_panic() {
    let mut rng = Rng(0xdead_beef_0bad_f00d);
    for _ in 0..5000 {
        let mut bytes = VALID.as_bytes().to_vec();
        for _ in 0..(1 + rng.next() % 6) {
            let at = (rng.next() as usize) % bytes.len();
            match rng.next() % 3 {
                0 => bytes[at] = rng.next() as u8,
                1 => {
                    bytes.remove(at);
                }
                _ => bytes.insert(at, rng.next() as u8),
            }
        }
        let _ = cue::parse(&String::from_utf8_lossy(&bytes));
    }
}

#[test]
fn every_truncation_of_an_image_is_an_error_or_a_clamped_read() {
    let dir = std::env::temp_dir().join(format!("disc-fuzz-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let tracks = [
        BuildTrack {
            kind: TrackKind::Mode1Raw,
            sectors: 3,
            fill: Fill::Pattern,
        },
        BuildTrack {
            kind: TrackKind::Audio,
            sectors: 2,
            fill: Fill::Pattern,
        },
    ];
    let cue = builder::write_disc(&dir, "t", &tracks).unwrap();
    let full = std::fs::read(dir.join("t.bin")).unwrap();
    for cut in (0..=full.len()).step_by(61).chain([full.len()]) {
        std::fs::write(dir.join("t.bin"), &full[..cut]).unwrap();
        if let Ok(mut d) = Disc::open(&cue) {
            let mut raw = [0u8; RAW_SECTOR];
            for lba in 0..8 {
                let _ = d.read_raw(lba, &mut raw);
            }
            assert!(d.lead_out() <= 5);
        }
    }
}
