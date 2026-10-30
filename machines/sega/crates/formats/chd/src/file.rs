//! The file: header, map, track table and hunk reads.

use crate::cd::{self, Base};
use crate::map::{self, Entry, Kind, MAX_HUNKS};
use crate::meta::{self, TrackText};
use crate::{ChdError, crc16, format_err};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub use crate::cd::{FRAME_SIZE, MAIN_SIZE, SUBCODE_SIZE};

const TAG: &[u8; 8] = b"MComprHD";
const V5_HEADER_LEN: u32 = 124;
/// The most a hunk may hold: the format's own limit is 512 KB; this leaves room and bounds the memory of a decode.
const MAX_HUNK_BYTES: u32 = 1 << 21;
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
    file: File,
    file_len: u64,
    hunk_bytes: u32,
    codecs: [Option<Base>; 4],
    /// Whether any of the four codec slots names something this reader cannot decode (reported when such a hunk is read).
    codec_tags: [[u8; 4]; 4],
    map: Vec<Entry>,
    tracks: Vec<TrackInfo>,
    frames: u32,
    raw_sha1: [u8; 20],
    /// The last hunk decoded: its number and bytes.
    cached: Option<(u32, Vec<u8>)>,
}

fn io(e: std::io::Error) -> ChdError {
    ChdError::Io(format!("reading the CHD file: {e}"))
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes(b[..4].try_into().unwrap())
}

fn be64(b: &[u8]) -> u64 {
    u64::from_be_bytes(b[..8].try_into().unwrap())
}

impl Chd {
    pub fn open(path: &Path) -> Result<Chd, ChdError> {
        let name = path.display().to_string();
        let mut file = File::open(path).map_err(|e| ChdError::Io(format!("{name}: {e}")))?;
        let file_len = file
            .metadata()
            .map_err(|e| ChdError::Io(format!("{name}: {e}")))?
            .len();
        let mut head = [0u8; V5_HEADER_LEN as usize];
        let have = file.read(&mut head).map_err(io)?;
        if have < 16 || &head[..8] != TAG {
            return format_err("it does not start with the CHD tag");
        }
        if be32(&head[12..]) != 5 {
            return Err(ChdError::Unsupported(format!(
                "CHD version {} (only version 5 is read)",
                be32(&head[12..])
            )));
        }
        if have < V5_HEADER_LEN as usize || be32(&head[8..]) != V5_HEADER_LEN {
            return format_err("the version 5 header has the wrong length");
        }
        let tags: [[u8; 4]; 4] = std::array::from_fn(|i| {
            let at = 16 + 4 * i;
            [head[at], head[at + 1], head[at + 2], head[at + 3]]
        });
        let logical = be64(&head[32..]);
        let map_offset = be64(&head[40..]);
        let meta_offset = be64(&head[48..]);
        let hunk_bytes = be32(&head[56..]);
        let unit_bytes = be32(&head[60..]);
        if head[84 + 20..84 + 40].iter().any(|&b| b != 0) {
            return Err(ChdError::Unsupported(
                "a CHD with a parent file".to_string(),
            ));
        }
        if hunk_bytes == 0 || hunk_bytes > MAX_HUNK_BYTES || unit_bytes as usize != FRAME_SIZE {
            return format_err("the hunk and unit sizes are not those of a CD image");
        }
        if !hunk_bytes.is_multiple_of(unit_bytes) {
            return format_err("a hunk is not a whole number of frames");
        }
        let hunks = logical.div_ceil(u64::from(hunk_bytes));
        if hunks == 0 || hunks > u64::from(MAX_HUNKS) {
            return format_err("the file has no hunks or too many");
        }
        let hunks = hunks as u32;

        let compressed = tags[0] != [0; 4];
        let map = if compressed {
            let mut mhead = [0u8; 16];
            file.seek(SeekFrom::Start(map_offset)).map_err(io)?;
            file.read_exact(&mut mhead).map_err(io)?;
            let mlen = be32(&mhead[..4]);
            if u64::from(mlen) + map_offset + 16 > file_len {
                return format_err("the map leaves the file");
            }
            let mut raw = vec![0u8; mlen as usize];
            file.read_exact(&mut raw).map_err(io)?;
            map::compressed_map(&mhead, &raw, hunks, hunk_bytes, unit_bytes)?
        } else {
            let bytes = u64::from(hunks) * 4;
            if map_offset + bytes > file_len {
                return format_err("the map leaves the file");
            }
            file.seek(SeekFrom::Start(map_offset)).map_err(io)?;
            let mut raw = vec![0u8; bytes as usize];
            file.read_exact(&mut raw).map_err(io)?;
            map::plain_map(&raw, hunks, hunk_bytes)?
        };
        if map.iter().any(|e| e.kind == Kind::Parent) {
            return Err(ChdError::Unsupported(
                "a hunk held by a parent file".to_string(),
            ));
        }
        for e in &map {
            let size = match e.kind {
                Kind::Codec(_) | Kind::Plain => u64::from(e.length),
                _ => 0,
            };
            let in_file = match e.kind {
                Kind::Codec(_) | Kind::Plain => e
                    .offset
                    .checked_add(size)
                    .is_some_and(|end| end <= file_len),
                Kind::SelfRef => e.offset < u64::from(hunks),
                Kind::Parent => false,
            };
            if !in_file {
                return format_err("a hunk lies outside the file");
            }
        }

        let frames = u32::try_from(logical / u64::from(unit_bytes))
            .map_err(|_| ChdError::Format("the image has too many frames".to_string()))?;
        let texts = meta::read_chain(&mut file, meta_offset, file_len)?;
        let tracks = layout(&texts)?;
        Ok(Chd {
            file,
            file_len,
            hunk_bytes,
            codecs: tags.map(Base::from_tag),
            codec_tags: tags,
            map,
            tracks,
            frames,
            raw_sha1: head[64..84].try_into().unwrap(),
            cached: None,
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
        self.raw_sha1
    }

    /// How many hunks the file has.
    pub fn hunk_count(&self) -> u32 {
        self.map.len() as u32
    }

    /// One frame: 2352 bytes of sector (audio big-endian) then 96 of subcode.
    pub fn read_frame(&mut self, frame: u32, out: &mut [u8; FRAME_SIZE]) -> Result<(), ChdError> {
        if frame >= self.frames {
            return format_err("a frame past the end of the image was asked for");
        }
        let per_hunk = self.hunk_bytes as usize / FRAME_SIZE;
        let hunk = frame / per_hunk as u32;
        let at = (frame as usize % per_hunk) * FRAME_SIZE;
        if self.cached.as_ref().is_none_or(|(n, _)| *n != hunk) {
            let bytes = self.decode_hunk(hunk, 0)?;
            self.cached = Some((hunk, bytes));
        }
        let (_, bytes) = self.cached.as_ref().expect("filled above");
        out.copy_from_slice(&bytes[at..at + FRAME_SIZE]);
        Ok(())
    }

    /// The bytes of hunk `number`; a reference to another hunk is followed (at most a few steps).
    pub fn hunk(&mut self, number: u32) -> Result<Vec<u8>, ChdError> {
        if number as usize >= self.map.len() {
            return format_err("a hunk past the end was asked for");
        }
        self.decode_hunk(number, 0)
    }

    fn decode_hunk(&mut self, number: u32, depth: u32) -> Result<Vec<u8>, ChdError> {
        let entry = self.map[number as usize];
        let size = self.hunk_bytes as usize;
        let bytes = match entry.kind {
            Kind::SelfRef => {
                if depth >= 8 {
                    return format_err("hunk references nest too deep");
                }
                return self.decode_hunk(entry.offset as u32, depth + 1);
            }
            Kind::Parent => return Err(ChdError::Unsupported("a parent file".to_string())),
            Kind::Plain => self.read_at(entry.offset, size)?,
            Kind::Codec(slot) => {
                let base = self.codecs[usize::from(slot)].ok_or_else(|| {
                    ChdError::Unsupported(format!(
                        "the codec \"{}\" (only cdzl, cdlz and cdfl are read)",
                        String::from_utf8_lossy(&self.codec_tags[usize::from(slot)])
                    ))
                })?;
                let src = self.read_at(entry.offset, entry.length as usize)?;
                cd::decode_hunk(base, &src, size / FRAME_SIZE)?
            }
        };
        // the plain map carries no check values; the compressed one does, over the decoded hunk
        let checked = !matches!(entry.kind, Kind::Plain) || entry.crc != 0;
        if checked && crc16(&bytes) != entry.crc {
            return format_err("a hunk fails its check value");
        }
        Ok(bytes)
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, ChdError> {
        if offset.saturating_add(len as u64) > self.file_len {
            return format_err("a hunk lies outside the file");
        }
        let mut buf = vec![0u8; len];
        self.file.seek(SeekFrom::Start(offset)).map_err(io)?;
        self.file.read_exact(&mut buf).map_err(io)?;
        Ok(buf)
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
