//! CHD v5 (compressed hunks of data) files holding CD images: the header, the compressed map, the metadata that describes the
//! tracks, and the three CD codecs (`cdzl` deflate, `cdlz` LZMA, `cdfl` FLAC for audio), each with a deflate-compressed
//! subcode area and with the error-correction bytes of data sectors regenerated.
//!
//! Written from the documented format (the file layout in the header of MAME's `chd.h`, the map and Huffman coding, the codec
//! framing, the CD-ROM sector layout of ECMA-130). The input is untrusted: a damaged or hostile file returns a `ChdError`,
//! never a panic, and every size read from the file is bounded before memory is reserved for it.
#![forbid(unsafe_code)]

mod bits;
#[cfg(any(test, feature = "builder"))]
pub mod builder;
mod cd;
mod codec;
pub(crate) mod ecc;
mod file;
mod flac;
mod huff;
mod huffman;
mod image;
mod map;
mod mapenc;
mod meta;
#[cfg(test)]
mod synthetic;
mod writer;

pub use codec::{Codec, compress, decompress};
pub use file::{Chd, FRAME_SIZE, MAIN_SIZE, SUBCODE_SIZE, TrackInfo};
pub use image::{Header, HunkKind, Image, VerifyReport};
pub use meta::{FLAG_CHECKSUM, MetaEntry, entry_bytes, overall_sha1};
pub use writer::{CreateOptions, Summary, Writer};

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChdError {
    Io(String),
    /// The file is not a well-formed CHD.
    Format(String),
    /// A valid CHD that uses something this reader does not do (a parent file, a codec, a GD-ROM).
    Unsupported(String),
}

impl fmt::Display for ChdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChdError::Io(m) => write!(f, "{m}"),
            ChdError::Format(m) => write!(f, "not a usable CHD file: {m}"),
            ChdError::Unsupported(m) => write!(f, "CHD feature not supported: {m}"),
        }
    }
}

impl std::error::Error for ChdError {}

pub(crate) fn format_err<T>(why: &str) -> Result<T, ChdError> {
    Err(ChdError::Format(why.to_string()))
}

/// CRC-16 as the format uses it (CCITT: polynomial 0x1021, initial value 0xFFFF, no reflection, no final XOR).
pub(crate) fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crc_is_the_ccitt_one() {
        // the standard check value of CRC-16/CCITT-FALSE
        assert_eq!(crc16(b"123456789"), 0x29B1);
        assert_eq!(crc16(&[]), 0xFFFF);
    }
}
