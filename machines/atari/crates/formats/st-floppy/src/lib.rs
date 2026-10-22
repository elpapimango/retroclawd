//! Atari ST `.st` floppy images: a raw dump of 512-byte sectors, cylinder by cylinder, side by side.
//!
//! - **Geometry** ([H] *Atari ST Internals* pp.178-179, the boot sector; its 16-bit fields are little-endian, "8086
//!   format"): bytes per sector at 11, total sectors at 19, sectors per track at 24, sides at 26. A plausible BPB
//!   (512-byte sectors, 1-2 sides, at least one sector per track, a whole number of cylinders) that agrees with the
//!   file size gives the geometry; a plausible one that disagrees is an error naming both, never a guess; an
//!   implausible one (an unformatted or non-DOS boot sector) leaves the size table to decide.
//! - **Size table** ([H?] the common ST formats): 360K 80×1×9, 400K 80×1×10, 720K 80×2×9, 800K 80×2×10, 820K
//!   82×2×10.
//! - **Tracks** ([D] WD177X-00 data sheet, *1986 Storage Management Products Handbook* p.1-18, "recommended
//!   dual-density format"): gap I 60 × `$4E`; per sector 12 × `$00`, 3 × `$A1` (sync), `$FE`, C H R N, CRC, gap II
//!   22 × `$4E`, 12 × `$00`, 3 × `$A1`, `$FB`, data, CRC, gap III 24 × `$4E`; gap IV `$4E` to the end of the track.
//!   Gaps I and III may shrink to 2 bytes ("as short as 2 bytes for WD177X-00 operation"); gap II may not. CRC-CCITT
//!   from `$FFFF` over the sync bytes, the mark and the field, high byte first.
//! - **Track length** [H?]: 6250 bytes, 250 kbit/s MFM for one revolution at 300 rpm.
//!
//! Every input is untrusted: sizes are capped, arithmetic is checked, nothing indexes past the data.
#![forbid(unsafe_code)]

pub mod decode;
pub mod msa;

use serde::{Deserialize, Serialize};

/// Bytes on one track (one revolution).
pub const TRACK_BYTES: usize = 6250;
/// Images larger than this are refused.
pub const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
const SECTOR: usize = 512;
/// The ID field's N for 512 bytes (`128 << N`).
const SIZE_CODE: u8 = 2;
const MAX_CYLINDERS: u32 = 86;
/// A sync byte (`A1` with a missing clock) as the `wd1770::Floppy` trait marks it.
const SYNC: u16 = 0x1A1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Geometry {
    pub cylinders: u8,
    pub sides: u8,
    pub sectors: u8,
}

impl Geometry {
    pub(crate) fn bytes(&self) -> usize {
        self.cylinders as usize * self.sides as usize * self.sectors as usize * SECTOR
    }
}

impl std::fmt::Display for Geometry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} cylinders × {} sides × {} sectors ({} sectors)",
            self.cylinders,
            self.sides,
            self.sectors,
            self.cylinders as u32 * self.sides as u32 * self.sectors as u32
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Empty, too large, or not a whole number of sectors.
    Size(usize),
    /// The boot sector's BPB and the file size disagree.
    Geometry { bpb: String, size: usize },
    /// No BPB and no known size.
    Unknown(usize),
    /// The format does not fit on a track.
    TrackTooLong(u8),
    /// A sector address or length outside the image.
    BadSector,
    /// A track written by the controller does not decode.
    Decode(decode::DecodeError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Size(n) => write!(
                f,
                "a .st image is a whole number of 512-byte sectors, at most {MAX_IMAGE_BYTES} bytes; this one has {n} bytes"
            ),
            Error::Geometry { bpb, size } => write!(
                f,
                "the boot sector says {bpb}, but the file holds {} sectors ({size} bytes)",
                size / SECTOR
            ),
            Error::Unknown(n) => write!(
                f,
                "no usable boot sector and no known format of {n} bytes ({} sectors)",
                n / SECTOR
            ),
            Error::TrackTooLong(spt) => write!(
                f,
                "{spt} sectors of 512 bytes do not fit on a {TRACK_BYTES}-byte track in the WD1772's format"
            ),
            Error::BadSector => write!(f, "no such sector, or the data is not 512 bytes"),
            Error::Decode(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawImage")]
pub struct Image {
    pub geometry: Geometry,
    data: Vec<u8>,
}

/// An image as it arrives from a serialised state: the same checks as a file's, so a hostile state cannot hold a
/// geometry that does not match its data.
#[derive(Deserialize)]
struct RawImage {
    geometry: Geometry,
    data: Vec<u8>,
}

impl TryFrom<RawImage> for Image {
    type Error = String;

    fn try_from(raw: RawImage) -> Result<Image, String> {
        let g = raw.geometry;
        if g.cylinders == 0
            || u32::from(g.cylinders) > MAX_CYLINDERS
            || !(1..=2).contains(&g.sides)
            || g.sectors == 0
        {
            return Err(format!("not a disk geometry: {g}"));
        }
        Image::from_geometry(g, raw.data).map_err(|e| e.to_string())
    }
}

fn le16(bytes: &[u8], at: usize) -> u32 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]]) as u32
}

/// The BPB's geometry when the fields are plausible on their own.
fn bpb_geometry(boot: &[u8]) -> Option<(Geometry, u32)> {
    let (eps, total, spt, sides) = (
        le16(boot, 11),
        le16(boot, 19),
        le16(boot, 24),
        le16(boot, 26),
    );
    if eps != SECTOR as u32 || spt == 0 || spt > 255 || !(1..=2).contains(&sides) || total == 0 {
        return None;
    }
    let per_cyl = spt * sides;
    if !total.is_multiple_of(per_cyl) || total / per_cyl > MAX_CYLINDERS {
        return None;
    }
    Some((
        Geometry {
            cylinders: (total / per_cyl) as u8,
            sides: sides as u8,
            sectors: spt as u8,
        },
        total,
    ))
}

fn table_geometry(size: usize) -> Option<Geometry> {
    let g = |cylinders, sides, sectors| Geometry {
        cylinders,
        sides,
        sectors,
    };
    match size / SECTOR {
        720 => Some(g(80, 1, 9)),
        800 => Some(g(80, 1, 10)),
        1440 => Some(g(80, 2, 9)),
        1600 => Some(g(80, 2, 10)),
        1640 => Some(g(82, 2, 10)),
        _ => None,
    }
}

/// Gap I, gap III and the bytes the sectors take with them; `None` when even the shortest gaps do not fit.
fn layout(sectors: u8) -> Option<(usize, usize)> {
    let fixed_per_sector = 12 + 3 + 1 + 4 + 2 + 22 + 12 + 3 + 1 + SECTOR + 2;
    let n = sectors as usize;
    for (gap1, gap3) in [(60, 24), (2, 24), (2, 2)] {
        if gap1 + n * (fixed_per_sector + gap3) <= TRACK_BYTES {
            return Some((gap1, gap3));
        }
    }
    None
}

impl Image {
    pub fn parse(bytes: &[u8]) -> Result<Image, Error> {
        let size = bytes.len();
        if size == 0 || size > MAX_IMAGE_BYTES || !size.is_multiple_of(SECTOR) {
            return Err(Error::Size(size));
        }
        let geometry = match bpb_geometry(&bytes[..SECTOR]) {
            Some((g, _)) if g.bytes() == size => g,
            Some((g, _)) => {
                return Err(Error::Geometry {
                    bpb: g.to_string(),
                    size,
                });
            }
            None => table_geometry(size).ok_or(Error::Unknown(size))?,
        };
        if layout(geometry.sectors).is_none() {
            return Err(Error::TrackTooLong(geometry.sectors));
        }
        Ok(Image {
            geometry,
            data: bytes.to_vec(),
        })
    }

    /// An image of `geometry` holding `data`; the geometry is trusted (the format said so), the size is checked.
    pub(crate) fn from_geometry(geometry: Geometry, data: Vec<u8>) -> Result<Image, Error> {
        if data.len() != geometry.bytes() || data.is_empty() || data.len() > MAX_IMAGE_BYTES {
            return Err(Error::Size(data.len()));
        }
        if layout(geometry.sectors).is_none() {
            return Err(Error::TrackTooLong(geometry.sectors));
        }
        Ok(Image { geometry, data })
    }

    fn offset(&self, cyl: u8, side: u8, sector: u8) -> Option<usize> {
        let g = self.geometry;
        if cyl >= g.cylinders || side >= g.sides || sector == 0 || sector > g.sectors {
            return None;
        }
        let index = (cyl as usize * g.sides as usize + side as usize) * g.sectors as usize
            + sector as usize
            - 1;
        Some(index * SECTOR)
    }

    /// Sector `sector` (from 1) of `cyl`/`side`.
    pub fn sector(&self, cyl: u8, side: u8, sector: u8) -> Option<&[u8]> {
        let at = self.offset(cyl, side, sector)?;
        self.data.get(at..at + SECTOR)
    }

    pub fn write_sector(
        &mut self,
        cyl: u8,
        side: u8,
        sector: u8,
        data: &[u8],
    ) -> Result<(), Error> {
        let at = self.offset(cyl, side, sector).ok_or(Error::BadSector)?;
        if data.len() != SECTOR {
            return Err(Error::BadSector);
        }
        self.data[at..at + SECTOR].copy_from_slice(data);
        Ok(())
    }

    /// Takes the sectors found on a track the controller wrote (`decode::sectors_from_mfm`) into the image and returns
    /// how many sectors changed. A sector whose ID names another cylinder or side, a size other than 512 bytes, or a
    /// record the geometry lacks has nowhere to go and is ignored (a `.st` image cannot change its geometry); a track
    /// that does not decode changes nothing.
    pub fn apply_track(&mut self, cyl: u8, side: u8, track: &[u16]) -> Result<usize, Error> {
        let sectors = decode::sectors_from_mfm(track).map_err(Error::Decode)?;
        let mut changed = 0;
        for s in sectors {
            if s.cylinder != cyl || s.side != side || s.size_code != SIZE_CODE {
                continue;
            }
            let Some(old) = self.sector(cyl, side, s.record) else {
                continue;
            };
            if old != s.data.as_slice() {
                self.write_sector(cyl, side, s.record, &s.data)?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.data.clone()
    }

    /// The bytes under the head on one revolution of `cyl`/`side`, for `wd1770::Floppy::read` (bit 8 marks a sync
    /// byte). A cylinder or side the image lacks is an unformatted track (all `$4E`).
    pub fn track(&self, cyl: u8, side: u8) -> Vec<u16> {
        let mut t: Vec<u16> = Vec::with_capacity(TRACK_BYTES);
        let g = self.geometry;
        let Some((gap1, gap3)) = layout(g.sectors) else {
            return vec![0x4E; TRACK_BYTES];
        };
        if cyl >= g.cylinders || side >= g.sides {
            return vec![0x4E; TRACK_BYTES];
        }
        let fill = |t: &mut Vec<u16>, n: usize, b: u16| t.extend(std::iter::repeat_n(b, n));
        let field = |t: &mut Vec<u16>, mark: u8, body: &[u8]| {
            t.extend([SYNC; 3]);
            t.push(mark as u16);
            t.extend(body.iter().map(|&b| b as u16));
            let crc = [0xA1, 0xA1, 0xA1, mark]
                .iter()
                .chain(body)
                .fold(0xFFFF, |c, &b| wd1770::crc_byte(c, b));
            t.extend([crc >> 8, crc & 0xFF]);
        };
        fill(&mut t, gap1, 0x4E);
        for r in 1..=g.sectors {
            fill(&mut t, 12, 0x00);
            field(&mut t, 0xFE, &[cyl, side, r, SIZE_CODE]);
            fill(&mut t, 22, 0x4E);
            fill(&mut t, 12, 0x00);
            field(
                &mut t,
                0xFB,
                self.sector(cyl, side, r).unwrap_or(&[0; SECTOR]),
            );
            fill(&mut t, gap3, 0x4E);
        }
        t.resize(TRACK_BYTES, 0x4E);
        t
    }
}
