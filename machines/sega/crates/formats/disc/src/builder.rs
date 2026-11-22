//! Builds small test discs (a cue sheet and BIN files) for tests and for probe tooling. Not a general disc authoring tool:
//! sectors carry a recognisable pattern, error-correction fields are zero.

use crate::cue::TrackKind;
use std::io;
use std::path::{Path, PathBuf};

/// How a built track's sectors are filled.
#[derive(Clone, Copy, Debug)]
pub enum Fill {
    /// Every user byte is this value.
    Byte(u8),
    /// User byte `i` of sector `n` is `(n * 7 + i) as u8`, so a misplaced sector or offset shows.
    Pattern,
}

#[derive(Clone, Copy, Debug)]
pub struct BuildTrack {
    pub kind: TrackKind,
    pub sectors: u32,
    pub fill: Fill,
}

fn user_bytes(kind: TrackKind) -> usize {
    match kind {
        TrackKind::Audio => 2352,
        _ => 2048,
    }
}

/// One sector of `track`, number `n` within the track, as it is stored in the BIN file.
pub fn sector_bytes(track: &BuildTrack, n: u32) -> Vec<u8> {
    let user = user_bytes(track.kind);
    let data: Vec<u8> = (0..user)
        .map(|i| match track.fill {
            Fill::Byte(b) => b,
            Fill::Pattern => (n as usize * 7 + i) as u8,
        })
        .collect();
    match track.kind {
        TrackKind::Audio | TrackKind::Mode1Cooked | TrackKind::Mode2Cooked => data,
        TrackKind::Mode1Raw | TrackKind::Mode2Raw => {
            let mut raw = vec![0u8; 2352];
            raw[1..11].fill(0xFF);
            raw[15] = if track.kind == TrackKind::Mode1Raw {
                1
            } else {
                2
            };
            raw[16..16 + 2048].copy_from_slice(&data);
            raw
        }
    }
}

fn mode_text(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Audio => "AUDIO",
        TrackKind::Mode1Raw => "MODE1/2352",
        TrackKind::Mode1Cooked => "MODE1/2048",
        TrackKind::Mode2Raw => "MODE2/2352",
        TrackKind::Mode2Cooked => "MODE2/2048",
    }
}

fn msf(sector: u32) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        sector / 4500,
        sector / 75 % 60,
        sector % 75
    )
}

/// Writes `<name>.cue` and one BIN holding all tracks, returns the cue path. All tracks must have the same sector size.
pub fn write_disc(dir: &Path, name: &str, tracks: &[BuildTrack]) -> io::Result<PathBuf> {
    let mut bin = Vec::new();
    let mut cue = format!("FILE \"{name}.bin\" BINARY\n");
    let mut at = 0u32;
    for (i, t) in tracks.iter().enumerate() {
        cue.push_str(&format!("  TRACK {:02} {}\n", i + 1, mode_text(t.kind)));
        cue.push_str(&format!("    INDEX 01 {}\n", msf(at)));
        for n in 0..t.sectors {
            bin.extend_from_slice(&sector_bytes(t, n));
        }
        at += t.sectors;
    }
    std::fs::write(dir.join(format!("{name}.bin")), bin)?;
    let path = dir.join(format!("{name}.cue"));
    std::fs::write(&path, cue)?;
    Ok(path)
}

/// Writes `<name>.cue` and one BIN per track.
pub fn write_split_disc(dir: &Path, name: &str, tracks: &[BuildTrack]) -> io::Result<PathBuf> {
    let mut cue = String::new();
    for (i, t) in tracks.iter().enumerate() {
        let file = format!("{name} (Track {}).bin", i + 1);
        cue.push_str(&format!("FILE \"{file}\" BINARY\n"));
        cue.push_str(&format!("  TRACK {:02} {}\n", i + 1, mode_text(t.kind)));
        cue.push_str("    INDEX 01 00:00:00\n");
        let mut bin = Vec::new();
        for n in 0..t.sectors {
            bin.extend_from_slice(&sector_bytes(t, n));
        }
        std::fs::write(dir.join(file), bin)?;
    }
    let path = dir.join(format!("{name}.cue"));
    std::fs::write(&path, cue)?;
    Ok(path)
}
