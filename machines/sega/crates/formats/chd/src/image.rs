//! A CHD version 5 file as bytes: the header, the map, the metadata, and the hunks decoded through whichever codecs the file names,
//! with a parent file when the file is a delta. Nothing here knows what the bytes mean (a CD, a disk); `Chd` and the tools build on it.

use crate::codec::{self, Codec};
use crate::map::{self, Entry, Kind, MAX_HUNKS};
use crate::meta::{self, MetaEntry};
use crate::{ChdError, crc16, format_err};
use sha1::{Digest, Sha1};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const TAG: &[u8; 8] = b"MComprHD";
pub const V5_HEADER_LEN: u32 = 124;
/// The most a hunk may hold: the format's own limit is 512 KB; this leaves room and bounds the memory of a decode.
pub const MAX_HUNK_BYTES: u32 = 1 << 21;

/// The version 5 header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// The four codec slots (all zero: the file is not compressed).
    pub compressors: [[u8; 4]; 4],
    pub logical_bytes: u64,
    pub map_offset: u64,
    pub meta_offset: u64,
    pub hunk_bytes: u32,
    pub unit_bytes: u32,
    /// SHA-1 of the data (all the hunks, decoded).
    pub raw_sha1: [u8; 20],
    /// SHA-1 of the data and the checksummed metadata.
    pub sha1: [u8; 20],
    /// The overall SHA-1 of the parent file (all zero: no parent).
    pub parent_sha1: [u8; 20],
}

impl Header {
    pub fn compressed(&self) -> bool {
        self.compressors[0] != [0; 4]
    }

    pub fn has_parent(&self) -> bool {
        self.parent_sha1 != [0; 20]
    }

    pub fn hunk_count(&self) -> u64 {
        self.logical_bytes.div_ceil(u64::from(self.hunk_bytes))
    }

    /// The 124 bytes as they go in the file.
    pub fn to_bytes(&self) -> [u8; V5_HEADER_LEN as usize] {
        let mut out = [0u8; V5_HEADER_LEN as usize];
        out[..8].copy_from_slice(TAG);
        out[8..12].copy_from_slice(&V5_HEADER_LEN.to_be_bytes());
        out[12..16].copy_from_slice(&5u32.to_be_bytes());
        for (i, c) in self.compressors.iter().enumerate() {
            out[16 + 4 * i..20 + 4 * i].copy_from_slice(c);
        }
        out[32..40].copy_from_slice(&self.logical_bytes.to_be_bytes());
        out[40..48].copy_from_slice(&self.map_offset.to_be_bytes());
        out[48..56].copy_from_slice(&self.meta_offset.to_be_bytes());
        out[56..60].copy_from_slice(&self.hunk_bytes.to_be_bytes());
        out[60..64].copy_from_slice(&self.unit_bytes.to_be_bytes());
        out[64..84].copy_from_slice(&self.raw_sha1);
        out[84..104].copy_from_slice(&self.sha1);
        out[104..124].copy_from_slice(&self.parent_sha1);
        out
    }

    /// Parses and checks a header (the bytes read from the start of a file).
    pub fn parse(head: &[u8]) -> Result<Header, ChdError> {
        if head.len() < 16 || &head[..8] != TAG {
            return format_err("it does not start with the CHD tag");
        }
        let be32 = |at: usize| u32::from_be_bytes(head[at..at + 4].try_into().unwrap());
        let be64 = |at: usize| u64::from_be_bytes(head[at..at + 8].try_into().unwrap());
        if be32(12) != 5 {
            return Err(ChdError::Unsupported(format!(
                "CHD version {} (only version 5 is read)",
                be32(12)
            )));
        }
        if head.len() < V5_HEADER_LEN as usize || be32(8) != V5_HEADER_LEN {
            return format_err("the version 5 header has the wrong length");
        }
        let h = Header {
            compressors: std::array::from_fn(|i| head[16 + 4 * i..20 + 4 * i].try_into().unwrap()),
            logical_bytes: be64(32),
            map_offset: be64(40),
            meta_offset: be64(48),
            hunk_bytes: be32(56),
            unit_bytes: be32(60),
            raw_sha1: head[64..84].try_into().unwrap(),
            sha1: head[84..104].try_into().unwrap(),
            parent_sha1: head[104..124].try_into().unwrap(),
        };
        if h.hunk_bytes == 0 || h.hunk_bytes > MAX_HUNK_BYTES || h.unit_bytes == 0 {
            return format_err("the hunk and unit sizes are not usable");
        }
        if !h.hunk_bytes.is_multiple_of(h.unit_bytes) {
            return format_err("a hunk is not a whole number of units");
        }
        let hunks = h.hunk_count();
        if hunks > u64::from(MAX_HUNKS) {
            return format_err("the file has too many hunks");
        }
        Ok(h)
    }
}

/// An open file.
pub struct Image {
    file: File,
    file_len: u64,
    header: Header,
    codecs: [Option<Codec>; 4],
    map: Vec<Entry>,
    meta: Vec<MetaEntry>,
    parent: Option<Box<Image>>,
    /// The last hunk decoded: its number and bytes.
    cached: Option<(u32, Vec<u8>)>,
}

fn io(e: std::io::Error) -> ChdError {
    ChdError::Io(format!("reading the CHD file: {e}"))
}

impl Image {
    /// Opens `path`. A file that records a parent needs `parent` (the path of that file), whose overall SHA-1 must be the recorded one.
    pub fn open(path: &Path, parent: Option<&Path>) -> Result<Image, ChdError> {
        let name = path.display().to_string();
        let mut file = File::open(path).map_err(|e| ChdError::Io(format!("{name}: {e}")))?;
        let file_len = file
            .metadata()
            .map_err(|e| ChdError::Io(format!("{name}: {e}")))?
            .len();
        let mut head = [0u8; V5_HEADER_LEN as usize];
        let have = file.read(&mut head).map_err(io)?;
        let header = Header::parse(&head[..have])?;
        let hunks = header.hunk_count() as u32;

        let map = if header.compressed() {
            let mut mhead = [0u8; 16];
            file.seek(SeekFrom::Start(header.map_offset)).map_err(io)?;
            file.read_exact(&mut mhead)
                .map_err(|_| ChdError::Format("the map leaves the file".to_string()))?;
            let mlen = u32::from_be_bytes(mhead[..4].try_into().unwrap());
            if u64::from(mlen) + header.map_offset + 16 > file_len {
                return format_err("the map leaves the file");
            }
            let mut raw = vec![0u8; mlen as usize];
            file.read_exact(&mut raw).map_err(io)?;
            map::compressed_map(&mhead, &raw, hunks, header.hunk_bytes, header.unit_bytes)?
        } else {
            let bytes = u64::from(hunks) * 4;
            if header.map_offset.saturating_add(bytes) > file_len {
                return format_err("the map leaves the file");
            }
            file.seek(SeekFrom::Start(header.map_offset)).map_err(io)?;
            let mut raw = vec![0u8; bytes as usize];
            file.read_exact(&mut raw).map_err(io)?;
            let mut map = map::plain_map(&raw, hunks, header.hunk_bytes)?;
            // an empty entry in an uncompressed file is the same hunk of the parent (or zeros without one)
            let per_hunk = u64::from(header.hunk_bytes / header.unit_bytes);
            for (n, e) in map.iter_mut().enumerate() {
                if e.kind == Kind::Parent {
                    e.offset = n as u64 * per_hunk;
                }
            }
            map
        };
        for e in &map {
            let in_file = match e.kind {
                Kind::Codec(_) | Kind::Plain => e
                    .offset
                    .checked_add(u64::from(e.length))
                    .is_some_and(|end| end <= file_len),
                Kind::SelfRef => e.offset < u64::from(hunks),
                Kind::Parent => true,
            };
            if !in_file {
                return format_err("a hunk lies outside the file");
            }
        }
        let meta = meta::read_chain(&mut file, header.meta_offset, file_len)?;

        let parent = match (header.has_parent(), parent) {
            (false, _) => None,
            (true, None) => {
                return Err(ChdError::Unsupported(
                    "a CHD with a parent file (name it)".to_string(),
                ));
            }
            (true, Some(p)) => {
                let parent = Image::open(p, None)?;
                if parent.header.sha1 != header.parent_sha1 {
                    return format_err("the parent file is not the one this file was made from");
                }
                if parent.header.unit_bytes != header.unit_bytes {
                    return format_err("the parent file's unit size differs");
                }
                Some(Box::new(parent))
            }
        };
        if parent.is_none() && map.iter().any(|e| e.kind == Kind::Parent) && header.compressed() {
            return format_err("a hunk is held by a parent but the file names none");
        }
        Ok(Image {
            file,
            file_len,
            codecs: header.compressors.map(Codec::from_tag),
            header,
            map,
            meta,
            parent,
            cached: None,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn metadata(&self) -> &[MetaEntry] {
        &self.meta
    }

    pub fn hunk_count(&self) -> u32 {
        self.map.len() as u32
    }

    pub fn file_len(&self) -> u64 {
        self.file_len
    }

    /// How hunk `n` is stored: for reports.
    pub fn hunk_kind(&self, n: u32) -> Option<HunkKind> {
        self.map.get(n as usize).map(|e| match e.kind {
            Kind::Codec(slot) => HunkKind::Codec(slot),
            Kind::Plain => HunkKind::Plain,
            Kind::SelfRef => HunkKind::SelfRef(e.offset as u32),
            Kind::Parent => HunkKind::Parent(e.offset),
        })
    }

    /// The bytes of hunk `number`, always `hunk_bytes` long (the last hunk of an image that ends inside it is padded with zeros).
    pub fn hunk(&mut self, number: u32) -> Result<Vec<u8>, ChdError> {
        if number as usize >= self.map.len() {
            return format_err("a hunk past the end was asked for");
        }
        self.decode_hunk(number, 0)
    }

    /// Like `hunk`, keeping the last one for the next call.
    pub fn hunk_cached(&mut self, number: u32) -> Result<&[u8], ChdError> {
        if self.cached.as_ref().is_none_or(|(n, _)| *n != number) {
            let bytes = self.hunk(number)?;
            self.cached = Some((number, bytes));
        }
        Ok(&self.cached.as_ref().expect("filled above").1)
    }

    /// `len` bytes of the image starting at byte `offset`; bytes past the logical end read as zero.
    pub fn read_bytes(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, ChdError> {
        let hb = u64::from(self.header.hunk_bytes);
        let mut out = Vec::with_capacity(len);
        let mut at = offset;
        while out.len() < len {
            let n = at / hb;
            let inside = (at % hb) as usize;
            let take = (hb as usize - inside).min(len - out.len());
            if n >= self.map.len() as u64 {
                out.resize(out.len() + take, 0);
            } else {
                let hunk = self.hunk_cached(n as u32)?;
                out.extend_from_slice(&hunk[inside..inside + take]);
            }
            at += take as u64;
        }
        Ok(out)
    }

    fn decode_hunk(&mut self, number: u32, depth: u32) -> Result<Vec<u8>, ChdError> {
        let entry = self.map[number as usize];
        let size = self.header.hunk_bytes as usize;
        let bytes = match entry.kind {
            Kind::SelfRef => {
                if depth >= 8 {
                    return format_err("hunk references nest too deep");
                }
                return self.decode_hunk(entry.offset as u32, depth + 1);
            }
            Kind::Parent => {
                let Some(parent) = self.parent.as_mut() else {
                    // an uncompressed file without a parent: an empty entry is a hunk of zeros
                    return Ok(vec![0u8; size]);
                };
                let at = entry
                    .offset
                    .checked_mul(u64::from(self.header.unit_bytes))
                    .ok_or_else(|| ChdError::Format("a parent offset is too large".to_string()))?;
                return parent.read_bytes(at, size);
            }
            Kind::Plain => self.read_at(entry.offset, size)?,
            Kind::Codec(slot) => {
                let codec = self.codecs[usize::from(slot)].ok_or_else(|| {
                    ChdError::Unsupported(format!(
                        "the codec \"{}\"",
                        String::from_utf8_lossy(&self.header.compressors[usize::from(slot)])
                    ))
                })?;
                let src = self.read_at(entry.offset, entry.length as usize)?;
                codec::decompress(codec, &src, size)?
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

    /// Reads every hunk and checks what the file promises: each hunk's check value (done by decoding), the SHA-1 of the data and the
    /// overall SHA-1 with the metadata. `progress` is told how many hunks are done.
    pub fn verify(&mut self, mut progress: impl FnMut(u32, u32)) -> Result<VerifyReport, ChdError> {
        let mut hasher = Sha1::new();
        let total = self.hunk_count();
        let logical = self.header.logical_bytes;
        let hb = u64::from(self.header.hunk_bytes);
        for n in 0..total {
            let hunk = self.hunk(n)?;
            // the data hash covers the logical bytes only: the last hunk is cut where the image ends
            let end = ((u64::from(n) + 1) * hb).min(logical) - u64::from(n) * hb;
            hasher.update(&hunk[..end as usize]);
            progress(n + 1, total);
        }
        let raw: [u8; 20] = hasher.finalize().into();
        let overall = meta::overall_sha1(&raw, &self.meta);
        // an uncompressed file does not carry the hashes
        let recorded = self.header.compressed();
        Ok(VerifyReport {
            raw_ok: !recorded || raw == self.header.raw_sha1,
            overall_ok: !recorded || overall == self.header.sha1,
            raw_sha1: raw,
            overall_sha1: overall,
            hashes_recorded: recorded,
        })
    }
}

/// How a hunk is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HunkKind {
    /// Compressed with the codec of this slot.
    Codec(u8),
    Plain,
    /// The same bytes as this other hunk.
    SelfRef(u32),
    /// The parent's bytes at this unit offset.
    Parent(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    pub raw_ok: bool,
    pub overall_ok: bool,
    pub raw_sha1: [u8; 20],
    pub overall_sha1: [u8; 20],
    /// False for an uncompressed file, which has no hashes to compare.
    pub hashes_recorded: bool,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.raw_ok && self.overall_ok
    }
}
