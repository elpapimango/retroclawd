//! The file: header, map, track table and hunk reads.

use crate::codec::Codec;
use crate::image::Image;
use crate::meta::{self, TrackText};
use crate::{ChdError, format_err};
use std::path::Path;

pub use crate::cd::{FRAME_SIZE, MAIN_SIZE, SUBCODE_SIZE};

/// A track is padded in the file to a multiple of this many frames.
const TRACK_PADDING: u32 = 4;

/// One track of the disc as the CHD describes it, placed in the file's frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackInfo {
    pub number: u32,
    /// `AUDIO`, `MODE1_RAW`, `MODE1`, `MODE2_RAW`, ...
    pub kind: String,
    /// Frames the track takes in the file, a stored pregap included (the padding after it is not counted).
    pub frames: u32,
    pub pregap: u32,
    /// Whether the pregap's sectors are in the file (at the start of the track's frames) or are to be taken as silence.
    pub pregap_in_file: bool,
    /// The frame of the file where the track's data starts.
    pub first_frame: u32,
}

pub struct Chd {
    image: Image,
    tracks: Vec<TrackInfo>,
    frames: u32,
}

impl Chd {
    pub fn open(path: &Path) -> Result<Chd, ChdError> {
        let image = Image::open(path, None)?;
        let h = image.header().clone();
        if h.unit_bytes as usize != FRAME_SIZE {
            return format_err("the hunk and unit sizes are not those of a CD image");
        }
        for tag in h.compressors.iter().filter(|t| **t != [0; 4]) {
            if !Codec::from_tag(*tag).is_some_and(Codec::is_cd) {
                return Err(ChdError::Unsupported(format!(
                    "the codec \"{}\" (only cdzl, cdzs, cdlz and cdfl are read)",
                    String::from_utf8_lossy(tag)
                )));
            }
        }
        let frames = u32::try_from(h.logical_bytes / u64::from(h.unit_bytes))
            .map_err(|_| ChdError::Format("the image has too many frames".to_string()))?;
        let tracks = layout(image.metadata())?;
        Ok(Chd {
            image,
            tracks,
            frames,
        })
    }

    pub fn tracks(&self) -> &[TrackInfo] {
        &self.tracks
    }

    /// Frames in the file (the tracks' frames and their padding).
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// The SHA-1 of all the hunks' decoded bytes, as the header records it.
    pub fn raw_sha1(&self) -> [u8; 20] {
        self.image.header().raw_sha1
    }

    /// How many hunks the file has.
    pub fn hunk_count(&self) -> u32 {
        self.image.hunk_count()
    }

    /// One frame: 2352 bytes of sector (audio big-endian) then 96 of subcode.
    pub fn read_frame(&mut self, frame: u32, out: &mut [u8; FRAME_SIZE]) -> Result<(), ChdError> {
        if frame >= self.frames {
            return format_err("a frame past the end of the image was asked for");
        }
        let per_hunk = self.image.header().hunk_bytes as usize / FRAME_SIZE;
        let hunk = frame / per_hunk as u32;
        let at = (frame as usize % per_hunk) * FRAME_SIZE;
        let bytes = self.image.hunk_cached(hunk)?;
        out.copy_from_slice(&bytes[at..at + FRAME_SIZE]);
        Ok(())
    }

    /// The bytes of hunk `number`; a reference to another hunk is followed (at most a few steps).
    pub fn hunk(&mut self, number: u32) -> Result<Vec<u8>, ChdError> {
        self.image.hunk(number)
    }
}

/// Places the described tracks in the file's frames: each starts where the previous ended, rounded up to the padding.
fn layout(texts: &[meta::Entry]) -> Result<Vec<TrackInfo>, ChdError> {
    let mut described: Vec<TrackText> = Vec::new();
    for entry in texts {
        match &entry.tag {
            b"CHT2" => described.push(meta::parse_track(&entry.data, true)?),
            b"CHTR" => described.push(meta::parse_track(&entry.data, false)?),
            b"CHGD" | b"CHGT" => {
                return Err(ChdError::Unsupported("a GD-ROM image".to_string()));
            }
            _ => {}
        }
    }
    if described.is_empty() {
        return format_err("the file describes no CD tracks");
    }
    let mut tracks = Vec::with_capacity(described.len());
    let mut at = 0u32;
    for t in described {
        tracks.push(TrackInfo {
            number: t.number,
            kind: t.kind,
            frames: t.frames,
            pregap: t.pregap,
            pregap_in_file: t.pregap_in_file,
            first_frame: at,
        });
        let padded = t
            .frames
            .div_ceil(TRACK_PADDING)
            .checked_mul(TRACK_PADDING)
            .and_then(|f| at.checked_add(f))
            .ok_or_else(|| ChdError::Format("the tracks have too many frames".to_string()))?;
        at = padded;
    }
    Ok(tracks)
}
