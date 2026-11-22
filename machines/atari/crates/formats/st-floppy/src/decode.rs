//! Reading a track's bytes back into sectors, the way the WD1772 finds them: three sync bytes and an ID mark (`$FE`),
//! the ID field (cylinder, side, record, size code) and its CRC; then, within a gap's reach, three sync bytes and a
//! data mark (`$FB`, or `$F8-$FA` for a deleted or special mark) with the data and its CRC. The layout and the CRC
//! (CRC-CCITT from `$FFFF` over the sync bytes, the mark and the field) are the WD177X data sheet's, p.1-18.
//!
//! The input is whatever a guest program wrote to the disk, so it is untrusted: a field cut off by the end of the
//! track ends the scan, sizes are capped, and nothing indexes past the data.

use crate::SYNC;

/// How far past an ID field's CRC the data mark may start, in bytes. [H?] The data sheet's gap II is 22 + 12 bytes
/// and the controller waits for the data mark for 43 bytes after the CRC; 64 leaves room for a long gap.
const DATA_MARK_REACH: usize = 64;
/// The largest size code read (`128 << 3` = 1024 bytes); a larger one is not a sector this machine can hold.
const MAX_SIZE_CODE: u8 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedSector {
    pub cylinder: u8,
    pub side: u8,
    pub record: u8,
    pub size_code: u8,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// An ID field's CRC does not match; `at` is the byte index of its mark.
    IdCrc { at: usize },
    /// A data field's CRC does not match.
    DataCrc { cylinder: u8, side: u8, record: u8 },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::IdCrc { at } => write!(f, "bad CRC in the sector ID field at byte {at}"),
            DecodeError::DataCrc {
                cylinder,
                side,
                record,
            } => write!(
                f,
                "bad CRC in the data of sector {record} (cylinder {cylinder}, side {side})"
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

fn crc(mark: u8, body: &[u16]) -> u16 {
    [0xA1u8, 0xA1, 0xA1, mark]
        .into_iter()
        .chain(body.iter().map(|&b| b as u8))
        .fold(0xFFFF, wd1770::crc_byte)
}

/// The mark byte after three sync bytes at `i`, if there are three.
fn mark_at(track: &[u16], i: usize) -> Option<u8> {
    let sync = track.get(i..i + 3)?;
    if sync.iter().all(|&b| b == SYNC) {
        track.get(i + 3).map(|&m| m as u8)
    } else {
        None
    }
}

/// Every sector on `track` (bit 8 of a byte marks a sync byte), in the order found.
pub fn sectors_from_mfm(track: &[u16]) -> Result<Vec<DecodedSector>, DecodeError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < track.len() {
        if mark_at(track, i) != Some(0xFE) {
            i += 1;
            continue;
        }
        let Some(id) = track.get(i + 4..i + 8) else {
            break;
        };
        let Some(crc_bytes) = track.get(i + 8..i + 10) else {
            break;
        };
        let stored = (crc_bytes[0] << 8) | (crc_bytes[1] & 0xFF);
        if crc(0xFE, id) != stored {
            return Err(DecodeError::IdCrc { at: i + 3 });
        }
        let (cylinder, side, record, size_code) =
            (id[0] as u8, id[1] as u8, id[2] as u8, id[3] as u8);
        i += 10;
        let Some(data_at) = (i..(i + DATA_MARK_REACH).min(track.len()))
            .find(|&j| matches!(mark_at(track, j), Some(0xF8..=0xFB)))
        else {
            continue;
        };
        if size_code > MAX_SIZE_CODE {
            i = data_at + 4;
            continue;
        }
        let len = 128usize << size_code;
        let start = data_at + 4;
        let (Some(data), Some(crc_bytes)) = (
            track.get(start..start + len),
            track.get(start + len..start + len + 2),
        ) else {
            break;
        };
        let mark = track[data_at + 3] as u8;
        let stored = (crc_bytes[0] << 8) | (crc_bytes[1] & 0xFF);
        if crc(mark, data) != stored {
            return Err(DecodeError::DataCrc {
                cylinder,
                side,
                record,
            });
        }
        out.push(DecodedSector {
            cylinder,
            side,
            record,
            size_code,
            data: data.iter().map(|&b| b as u8).collect(),
        });
        i = start + len + 2;
    }
    Ok(out)
}
