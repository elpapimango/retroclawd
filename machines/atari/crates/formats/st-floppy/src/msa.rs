//! `.msa` (Magic Shadow Archiver) images: a header and the tracks, each stored raw or run-length encoded.
//!
//! [H] DrCoolZic, *Atari Image File Formats* 1.0 (2014), "MSA file format", quoting Damien Burke's description of
//! 31 August 1997 (`http://info-coach.fr/atari/documents/_mydoc/FD_Image_File_Format.pdf`, SHA-256 in
//! `CREDITS.md`). Words are 16 bits, big-endian (the ID `$0E0F` is the two bytes `0E 0F`, PRONOM fmt/1472):
//! - header: ID `$0E0F`, sectors per track, sides minus one (0 or 1), first track, last track (0-based);
//! - then, for every track from the first to the last, sides alternating: a data length word, then the data;
//! - a length of 512 x sectors per track is the raw track; a shorter one is run-length encoded: a byte other than
//!   `$E5` is copied, `$E5` starts a run of marker, data byte, run length (a word), and a real `$E5` is
//!   `$E5 $E5 $0001`;
//! - [H] the archiver compresses only runs of at least 4 and stores a track raw when that is smaller.
//!
//! [H?] A file whose first track is not 0 leaves the earlier tracks zero-filled. Every input is untrusted: the
//! header's numbers are range-checked, a track never decodes to more than its size, and nothing indexes past the data.

use crate::{Error, Geometry, Image, MAX_CYLINDERS, MAX_IMAGE_BYTES, SECTOR};

const ID: u16 = 0x0E0F;
const MARKER: u8 = 0xE5;
const HEADER_BYTES: usize = 10;
/// The shortest run worth encoding (an encoded run takes 4 bytes).
const MIN_RUN: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsaError {
    /// Fewer bytes than the header.
    TooShort,
    /// The ID word is not `$0E0F`.
    BadId(u16),
    /// The header's geometry is outside what this machine can hold.
    Geometry {
        sectors: u16,
        sides: u16,
        first: u16,
        last: u16,
    },
    /// A track's data length is longer than the track, or the data ends early.
    Track {
        track: u16,
        side: u8,
        why: &'static str,
    },
    /// The image does not fit.
    Image(Error),
}

impl std::fmt::Display for MsaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MsaError::TooShort => write!(f, "an .msa file starts with a 10-byte header"),
            MsaError::BadId(id) => {
                write!(f, "not an .msa file: the ID word is ${id:04X}, not $0E0F")
            }
            MsaError::Geometry {
                sectors,
                sides,
                first,
                last,
            } => write!(
                f,
                "the .msa header asks for {sectors} sectors per track, sides field {sides}, tracks {first} to {last}: not a disk this machine can hold"
            ),
            MsaError::Track { track, side, why } => {
                write!(f, ".msa track {track} side {side}: {why}")
            }
            MsaError::Image(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MsaError {}

fn word(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
}

/// Decodes one run-length encoded track into exactly `size` bytes.
fn expand(data: &[u8], size: usize) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::with_capacity(size);
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if b != MARKER {
            out.push(b);
            i += 1;
        } else {
            let (Some(&value), Some(run)) = (data.get(i + 1), word(data, i + 2)) else {
                return Err("a run is cut off");
            };
            if out.len() + run as usize > size {
                return Err("the data expands beyond the track");
            }
            out.extend(std::iter::repeat_n(value, run as usize));
            i += 4;
        }
        if out.len() > size {
            return Err("the data expands beyond the track");
        }
    }
    if out.len() != size {
        return Err("the data expands to less than a track");
    }
    Ok(out)
}

pub fn parse(bytes: &[u8]) -> Result<Image, MsaError> {
    if bytes.len() < HEADER_BYTES {
        return Err(MsaError::TooShort);
    }
    let field = |n: usize| word(bytes, 2 * n).unwrap_or(0);
    let (id, sectors, sides, first, last) = (field(0), field(1), field(2), field(3), field(4));
    if id != ID {
        return Err(MsaError::BadId(id));
    }
    let bad = MsaError::Geometry {
        sectors,
        sides,
        first,
        last,
    };
    if sectors == 0
        || sectors > 255
        || sides > 1
        || first > last
        || u32::from(last) >= MAX_CYLINDERS
    {
        return Err(bad);
    }
    let geometry = Geometry {
        cylinders: last as u8 + 1,
        sides: sides as u8 + 1,
        sectors: sectors as u8,
    };
    let track_bytes = sectors as usize * SECTOR;
    if geometry.bytes() > MAX_IMAGE_BYTES {
        return Err(bad);
    }
    let mut data = vec![0u8; geometry.bytes()];
    let mut at = HEADER_BYTES;
    for cyl in first..=last {
        for side in 0..geometry.sides {
            let place = |why| MsaError::Track {
                track: cyl,
                side,
                why,
            };
            let len = word(bytes, at).ok_or_else(|| place("the track header is missing"))? as usize;
            at += 2;
            let body = bytes
                .get(at..at + len)
                .ok_or_else(|| place("the file ends inside the track"))?;
            at += len;
            if len > track_bytes {
                return Err(place("the data is longer than the track"));
            }
            let track = if len == track_bytes {
                body.to_vec()
            } else {
                expand(body, track_bytes).map_err(place)?
            };
            let to = (cyl as usize * geometry.sides as usize + side as usize) * track_bytes;
            data[to..to + track_bytes].copy_from_slice(&track);
        }
    }
    Image::from_geometry(geometry, data).map_err(MsaError::Image)
}

/// Encodes one track: runs of at least [`MIN_RUN`] and every `$E5` as runs; `None` when that is no smaller.
fn compress(track: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < track.len() {
        let b = track[i];
        let mut run = 1;
        while i + run < track.len() && track[i + run] == b && run < u16::MAX as usize {
            run += 1;
        }
        if run >= MIN_RUN || b == MARKER {
            out.extend([MARKER, b]);
            out.extend((run as u16).to_be_bytes());
        } else {
            out.extend(&track[i..i + run]);
        }
        i += run;
    }
    (out.len() < track.len()).then_some(out)
}

pub fn write(image: &Image) -> Vec<u8> {
    let g = image.geometry;
    let mut out = Vec::new();
    for v in [
        ID,
        g.sectors as u16,
        g.sides as u16 - 1,
        0,
        g.cylinders as u16 - 1,
    ] {
        out.extend(v.to_be_bytes());
    }
    let track_bytes = g.sectors as usize * SECTOR;
    for track in image.to_bytes().chunks(track_bytes) {
        match compress(track) {
            Some(c) => {
                out.extend((c.len() as u16).to_be_bytes());
                out.extend(c);
            }
            None => {
                out.extend((track.len() as u16).to_be_bytes());
                out.extend(track);
            }
        }
    }
    out
}
