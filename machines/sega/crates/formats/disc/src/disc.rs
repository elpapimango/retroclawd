//! A disc: the cue sheet's files laid out as one run of sectors.
//!
//! Sector addresses are LBAs where LBA 0 is the first sector of the program area (absolute time 00:02:00, the 150 sectors of
//! lead-in pregap are not counted) [H: ECMA-130 / Red Book time addressing, fact D1 in `docs/hardware-facts.md`].

use crate::DiscError;
use crate::cue::{self, TrackKind};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Sectors per second of a single-speed drive.
pub const SECTORS_PER_SECOND: u32 = 75;
/// A raw sector: sync (12), header (4), 2336 bytes of mode data.
pub const RAW_SECTOR: usize = 2352;
pub const USER_SECTOR: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Track {
    pub number: u8,
    pub kind: TrackKind,
    /// The sector of index 01.
    pub start_lba: u32,
    /// The first sector of the track including its pregap (index 00 or a `PREGAP` command).
    pub pregap_lba: u32,
    /// Sectors from `start_lba` to the start of the next track (or the lead-out).
    pub sectors: u32,
}

enum Data {
    /// A `PREGAP` command: silent sectors, no data in any file.
    Silence,
    /// Sectors from `first` of file `file`.
    File { file: usize, first: u32 },
}

struct Segment {
    lba: u32,
    count: u32,
    kind: TrackKind,
    data: Data,
}

struct Source {
    file: File,
    name: String,
    sectors: u32,
}

pub struct Disc {
    tracks: Vec<Track>,
    segments: Vec<Segment>,
    sources: Vec<Source>,
    lead_out: u32,
}

fn io_err(name: &str, e: std::io::Error) -> DiscError {
    DiscError::Io(format!("{name}: {e}"))
}

impl Disc {
    /// Opens a `.cue` sheet or an `.iso` file.
    pub fn open(path: &Path) -> Result<Disc, DiscError> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        match ext.as_str() {
            "cue" => {
                let name = path.to_string_lossy();
                let file = File::open(path).map_err(|e| io_err(&name, e))?;
                let mut bytes = Vec::new();
                file.take(cue::MAX_CUE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| io_err(&name, e))?;
                let text = String::from_utf8_lossy(&bytes);
                Disc::from_cue(&cue::parse(&text)?, dir)
            }
            "iso" => {
                let sheet = cue::CueSheet {
                    files: vec![cue::CueFile {
                        name: path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string(),
                        tracks: vec![cue::CueTrack {
                            number: 1,
                            kind: TrackKind::Mode1Cooked,
                            pregap: 0,
                            postgap: 0,
                            indexes: vec![(1, 0)],
                        }],
                    }],
                };
                Disc::from_cue(&sheet, dir)
            }
            "chd" => Err(DiscError::Unsupported("chd".into())),
            other => Err(DiscError::Unsupported(format!(
                "\".{other}\" (expected .cue or .iso)"
            ))),
        }
    }

    fn from_cue(sheet: &cue::CueSheet, dir: &Path) -> Result<Disc, DiscError> {
        let mut disc = Disc {
            tracks: Vec::new(),
            segments: Vec::new(),
            sources: Vec::new(),
            lead_out: 0,
        };
        let too_big = || DiscError::TooLarge("the disc".into());
        let mut cursor = 0u32;
        for (file_index, cf) in sheet.files.iter().enumerate() {
            let size = cf
                .tracks
                .first()
                .map_or(RAW_SECTOR, |t| t.kind.sector_size());
            if cf.tracks.iter().any(|t| t.kind.sector_size() != size) {
                return Err(DiscError::Unsupported(format!(
                    "{}: tracks with different sector sizes in one file",
                    cf.name
                )));
            }
            let file = File::open(dir.join(&cf.name)).map_err(|e| io_err(&cf.name, e))?;
            let len = file.metadata().map_err(|e| io_err(&cf.name, e))?.len();
            if len % size as u64 != 0 {
                return Err(DiscError::Io(format!(
                    "{}: {len} bytes is not a multiple of the {size}-byte sector",
                    cf.name
                )));
            }
            let sectors = u32::try_from(len / size as u64)
                .map_err(|_| DiscError::TooLarge(cf.name.clone()))?;
            disc.sources.push(Source {
                file,
                name: cf.name.clone(),
                sectors,
            });
            for (ti, ct) in cf.tracks.iter().enumerate() {
                let index = |n: u8| ct.indexes.iter().find(|(i, _)| *i == n).map(|(_, p)| *p);
                let idx1 = index(1).ok_or_else(|| DiscError::Cue {
                    line: 0,
                    reason: format!("track {} has no INDEX 01", ct.number),
                })?;
                let first = index(0).unwrap_or(idx1).min(idx1);
                let next_first = cf
                    .tracks
                    .get(ti + 1)
                    .and_then(|n| n.indexes.iter().map(|(_, p)| *p).min());
                // only data that is in the file counts: a short file is clamped
                let first_c = first.min(sectors);
                let end = next_first.unwrap_or(sectors).min(sectors).max(first_c);
                let pregap_lba = cursor;
                if ct.pregap > 0 {
                    disc.segments.push(Segment {
                        lba: cursor,
                        count: ct.pregap,
                        kind: ct.kind,
                        data: Data::Silence,
                    });
                    cursor = cursor.checked_add(ct.pregap).ok_or_else(too_big)?;
                }
                let count = end - first_c;
                disc.segments.push(Segment {
                    lba: cursor,
                    count,
                    kind: ct.kind,
                    data: Data::File {
                        file: file_index,
                        first: first_c,
                    },
                });
                let start_lba = cursor + idx1.saturating_sub(first).min(count);
                cursor = cursor.checked_add(count).ok_or_else(too_big)?;
                disc.tracks.push(Track {
                    number: ct.number,
                    kind: ct.kind,
                    start_lba,
                    pregap_lba,
                    sectors: cursor - start_lba,
                });
            }
        }
        disc.lead_out = cursor;
        Ok(disc)
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// The first sector after the last track.
    pub fn lead_out(&self) -> u32 {
        self.lead_out
    }

    /// Minutes, seconds, frames of the absolute time of `lba` (LBA 0 is 00:02:00).
    pub fn msf(lba: u32) -> (u8, u8, u8) {
        let t = u64::from(lba) + 2 * u64::from(SECTORS_PER_SECOND);
        let per_min = 60 * u64::from(SECTORS_PER_SECOND);
        (
            (t / per_min).min(255) as u8,
            (t / u64::from(SECTORS_PER_SECOND) % 60) as u8,
            (t % u64::from(SECTORS_PER_SECOND)) as u8,
        )
    }

    /// Reads a sector in its raw 2352-byte form. A cooked (2048-byte) source is expanded with sync, header and a mode byte; its
    /// EDC and ECC fields are left zero (no reader of ours checks them).
    pub fn read_raw(&mut self, lba: u32, out: &mut [u8; RAW_SECTOR]) -> Result<(), DiscError> {
        let seg = self
            .segments
            .iter()
            .find(|s| lba >= s.lba && lba - s.lba < s.count)
            .ok_or(DiscError::OutOfRange(lba))?;
        let (kind, lba_in) = (seg.kind, lba - seg.lba);
        match seg.data {
            Data::Silence => {
                out.fill(0);
                Ok(())
            }
            Data::File { file, first } => {
                let src = &mut self.sources[file];
                let n = first + lba_in;
                if n >= src.sectors {
                    return Err(DiscError::OutOfRange(lba));
                }
                let size = kind.sector_size();
                src.file
                    .seek(SeekFrom::Start(u64::from(n) * size as u64))
                    .map_err(|e| io_err(&src.name, e))?;
                if size == RAW_SECTOR {
                    src.file.read_exact(out).map_err(|e| io_err(&src.name, e))?;
                } else {
                    let mut user = [0u8; USER_SECTOR];
                    src.file
                        .read_exact(&mut user)
                        .map_err(|e| io_err(&src.name, e))?;
                    out.fill(0);
                    out[1..11].fill(0xFF);
                    let (m, s, f) = Disc::msf(lba);
                    let bcd = |v: u8| ((v / 10) << 4) | (v % 10);
                    out[12] = bcd(m);
                    out[13] = bcd(s);
                    out[14] = bcd(f);
                    out[15] = if kind == TrackKind::Mode2Cooked { 2 } else { 1 };
                    out[16..16 + USER_SECTOR].copy_from_slice(&user);
                }
                Ok(())
            }
        }
    }

    /// Reads the 2048 user bytes of a mode 1 sector.
    pub fn read_user(&mut self, lba: u32, out: &mut [u8; USER_SECTOR]) -> Result<(), DiscError> {
        let mut raw = [0u8; RAW_SECTOR];
        self.read_raw(lba, &mut raw)?;
        out.copy_from_slice(&raw[16..16 + USER_SECTOR]);
        Ok(())
    }
}
