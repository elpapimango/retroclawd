//! The map of a version 5 file: one entry per hunk telling where its bytes are and how they are coded.

use crate::bits::BitReader;
use crate::huffman::Decoder;
use crate::{ChdError, crc16, format_err};

/// The most hunks a file may have (a 4 GB image of 2 KB hunks is half a million; this is generous and bounds the map's memory).
pub(crate) const MAX_HUNKS: u32 = 1 << 22;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Compressed with the codec of this slot (0-3) of the header's list.
    Codec(u8),
    /// Stored as it is.
    Plain,
    /// The same bytes as another hunk of this file.
    SelfRef,
    /// A hunk of the parent file (not supported).
    Parent,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    pub kind: Kind,
    /// Compressed length in the file (0 for the kinds that carry none).
    pub length: u32,
    /// A file offset (codec, plain), or the number of the other hunk (self reference).
    pub offset: u64,
    pub crc: u16,
}

// the type codes of the compressed map's first stage
const TYPE_PLAIN: u8 = 4;
const TYPE_SELF: u8 = 5;
const TYPE_PARENT: u8 = 6;
const TYPE_RLE_SMALL: u8 = 7;
const TYPE_RLE_LARGE: u8 = 8;
const TYPE_SELF_0: u8 = 9;
const TYPE_SELF_1: u8 = 10;
const TYPE_PARENT_SELF: u8 = 11;
const TYPE_PARENT_0: u8 = 12;
const TYPE_PARENT_1: u8 = 13;

/// An uncompressed map: a 32-bit big-endian "offset divided by the hunk size" per hunk (0 means the parent has it).
pub(crate) fn plain_map(raw: &[u8], hunks: u32, hunk_bytes: u32) -> Result<Vec<Entry>, ChdError> {
    let mut out = Vec::with_capacity(hunks as usize);
    for i in 0..hunks as usize {
        let word = raw
            .get(i * 4..i * 4 + 4)
            .ok_or_else(|| ChdError::Format("the hunk map is cut short".to_string()))?;
        let at = u32::from_be_bytes(word.try_into().unwrap());
        out.push(if at == 0 {
            Entry {
                kind: Kind::Parent,
                length: 0,
                offset: 0,
                crc: 0,
            }
        } else {
            Entry {
                kind: Kind::Plain,
                length: hunk_bytes,
                offset: u64::from(at) * u64::from(hunk_bytes),
                crc: 0,
            }
        });
    }
    Ok(out)
}

/// The compressed map: `raw` is what follows the 16-byte map header in the file, `header` that header.
pub(crate) fn compressed_map(
    header: &[u8; 16],
    raw: &[u8],
    hunks: u32,
    hunk_bytes: u32,
    unit_bytes: u32,
) -> Result<Vec<Entry>, ChdError> {
    let first_offset = u64::from_be_bytes([
        0, 0, header[4], header[5], header[6], header[7], header[8], header[9],
    ]);
    let map_crc = u16::from_be_bytes([header[10], header[11]]);
    let (length_bits, self_bits, parent_bits) = (
        u32::from(header[12]),
        u32::from(header[13]),
        u32::from(header[14]),
    );
    if length_bits > 24 || self_bits > 32 || parent_bits > 32 {
        return format_err("the map's field widths are out of range");
    }
    let bad = || ChdError::Format("the compressed map is damaged".to_string());
    let mut bits = BitReader::new(raw);
    let decoder = Decoder::from_stream(&mut bits).ok_or_else(bad)?;

    // first the type of every hunk, with runs of the same type coded as repeat counts
    let mut types = Vec::with_capacity(hunks as usize);
    let (mut last, mut repeat) = (0u8, 0u32);
    while types.len() < hunks as usize {
        if repeat > 0 {
            types.push(last);
            repeat -= 1;
            continue;
        }
        let symbol = decoder.decode(&mut bits).ok_or_else(bad)?;
        match symbol {
            TYPE_RLE_SMALL => {
                types.push(last);
                repeat = 2 + u32::from(decoder.decode(&mut bits).ok_or_else(bad)?);
            }
            TYPE_RLE_LARGE => {
                types.push(last);
                let high = u32::from(decoder.decode(&mut bits).ok_or_else(bad)?);
                let low = u32::from(decoder.decode(&mut bits).ok_or_else(bad)?);
                repeat = 2 + 16 + (high << 4) + low;
            }
            other => {
                last = other;
                types.push(other);
            }
        }
    }

    // then, hunk by hunk, the lengths and offsets those types call for
    let units_per_hunk = u64::from(hunk_bytes / unit_bytes.max(1));
    let mut entries = Vec::with_capacity(hunks as usize);
    let mut rebuilt = Vec::with_capacity(hunks as usize * 12);
    let mut cursor = first_offset;
    let (mut last_self, mut last_parent) = (0u64, 0u64);
    for (number, &kind) in types.iter().enumerate() {
        let mut length = 0u32;
        let mut crc = 0u16;
        let mut offset = cursor;
        let kind = match kind {
            0..=3 => {
                length = bits.read(length_bits);
                crc = bits.read(16) as u16;
                cursor += u64::from(length);
                Kind::Codec(kind)
            }
            TYPE_PLAIN => {
                length = hunk_bytes;
                crc = bits.read(16) as u16;
                cursor += u64::from(hunk_bytes);
                Kind::Plain
            }
            TYPE_SELF => {
                offset = u64::from(bits.read(self_bits));
                last_self = offset;
                Kind::SelfRef
            }
            TYPE_PARENT => {
                offset = u64::from(bits.read(parent_bits));
                last_parent = offset;
                Kind::Parent
            }
            TYPE_SELF_0 | TYPE_SELF_1 => {
                if kind == TYPE_SELF_1 {
                    last_self += 1;
                }
                offset = last_self;
                Kind::SelfRef
            }
            TYPE_PARENT_SELF => {
                last_parent = number as u64 * units_per_hunk;
                offset = last_parent;
                Kind::Parent
            }
            TYPE_PARENT_0 | TYPE_PARENT_1 => {
                if kind == TYPE_PARENT_1 {
                    last_parent += units_per_hunk;
                }
                offset = last_parent;
                Kind::Parent
            }
            _ => return format_err("the map has an unknown hunk type"),
        };
        // the check value covers the entries in their expanded form
        rebuilt.push(match kind {
            Kind::Codec(slot) => slot,
            Kind::Plain => TYPE_PLAIN,
            Kind::SelfRef => TYPE_SELF,
            Kind::Parent => TYPE_PARENT,
        });
        rebuilt.extend_from_slice(&length.to_be_bytes()[1..]);
        rebuilt.extend_from_slice(&offset.to_be_bytes()[2..]);
        rebuilt.extend_from_slice(&crc.to_be_bytes());
        entries.push(Entry {
            kind,
            length,
            offset,
            crc,
        });
    }
    if bits.overflowed() {
        return Err(bad());
    }
    if crc16(&rebuilt) != map_crc {
        return format_err("the map fails its check value");
    }
    Ok(entries)
}
