//! The hunk codecs by name: the generic ones (`zlib`, `zstd`, `lzma`, `huff`, `flac`) here, the CD ones (`cdzl`, `cdzs`, `cdlz`, `cdfl`) in
//! `cd.rs` on top of the same primitives.
//!
//! Framing (what a hunk's compressed bytes are): `zlib` a raw deflate stream (no zlib header), `zstd` one Zstandard frame, `lzma` a raw
//! LZMA stream with lc=3 lp=0 pb=2 and no end marker (the length is the hunk size), `huff` a tree then codes (`huff.rs`), `flac` a
//! byte (`L` or `B`, the byte order of the 16-bit stereo samples) then bare FLAC frames. A compressor gives up (`None`) when the result
//! is not smaller than the hunk, and the file then stores the hunk as it is.

use crate::{ChdError, flac, huff};
use flate2::Compression;
use flate2::write::DeflateEncoder;
use std::io::{Cursor, Read, Write};

/// lc = 3, lp = 0, pb = 2 as the properties byte: (pb * 5 + lp) * 9 + lc.
const LZMA_PROPS: u8 = 93;

/// A hunk codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    Zlib,
    Zstd,
    Lzma,
    Huff,
    Flac,
    CdZlib,
    CdZstd,
    CdLzma,
    CdFlac,
}

impl Codec {
    pub const ALL: [Codec; 9] = [
        Codec::Zlib,
        Codec::Zstd,
        Codec::Lzma,
        Codec::Huff,
        Codec::Flac,
        Codec::CdZlib,
        Codec::CdZstd,
        Codec::CdLzma,
        Codec::CdFlac,
    ];

    pub fn tag(self) -> [u8; 4] {
        *match self {
            Codec::Zlib => b"zlib",
            Codec::Zstd => b"zstd",
            Codec::Lzma => b"lzma",
            Codec::Huff => b"huff",
            Codec::Flac => b"flac",
            Codec::CdZlib => b"cdzl",
            Codec::CdZstd => b"cdzs",
            Codec::CdLzma => b"cdlz",
            Codec::CdFlac => b"cdfl",
        }
    }

    pub fn from_tag(tag: [u8; 4]) -> Option<Codec> {
        Codec::ALL.into_iter().find(|c| c.tag() == tag)
    }

    /// Whether the codec is made for CD frames (hunks of 2448-byte frames).
    pub fn is_cd(self) -> bool {
        matches!(
            self,
            Codec::CdZlib | Codec::CdZstd | Codec::CdLzma | Codec::CdFlac
        )
    }

    pub fn name(self) -> &'static str {
        match self {
            Codec::Zlib => "zlib (Deflate)",
            Codec::Zstd => "zstd (Zstandard)",
            Codec::Lzma => "lzma (LZMA)",
            Codec::Huff => "huff (Huffman)",
            Codec::Flac => "flac (FLAC)",
            Codec::CdZlib => "cdzl (CD Deflate)",
            Codec::CdZstd => "cdzs (CD Zstandard)",
            Codec::CdLzma => "cdlz (CD LZMA)",
            Codec::CdFlac => "cdfl (CD FLAC)",
        }
    }
}

pub(crate) fn bad(why: &str) -> ChdError {
    ChdError::Format(format!("a hunk fails to decode: {why}"))
}

fn corrupt<T>(why: &str) -> Result<T, ChdError> {
    Err(bad(why))
}

/// A raw deflate stream, best compression.
pub(crate) fn deflate(data: &[u8]) -> Vec<u8> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::best());
    enc.write_all(data).expect("writing to a Vec");
    enc.finish().expect("writing to a Vec")
}

/// A raw deflate stream that must inflate to exactly `size` bytes.
pub(crate) fn inflate(src: &[u8], size: usize) -> Result<Vec<u8>, ChdError> {
    let mut out = vec![0u8; size];
    let mut decoder = flate2::read::DeflateDecoder::new(src);
    decoder
        .read_exact(&mut out)
        .or_else(|_| corrupt("a deflate stream is short or damaged"))?;
    Ok(out)
}

pub(crate) fn zstd_pack(data: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest)
}

pub(crate) fn zstd_unpack(src: &[u8], size: usize) -> Result<Vec<u8>, ChdError> {
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(src)
        .or_else(|e| corrupt(&format!("a Zstandard frame cannot start ({e})")))?;
    let mut out = vec![0u8; size];
    decoder
        .read_exact(&mut out)
        .or_else(|_| corrupt("a Zstandard frame is short or damaged"))?;
    Ok(out)
}

/// The dictionary only has to reach as far back as one hunk's data goes.
fn lzma_dictionary(size: usize) -> u32 {
    size.next_power_of_two().max(4096) as u32
}

pub(crate) fn lzma_pack(data: &[u8]) -> Option<Vec<u8>> {
    let mut options = lzma_rust2::LzmaOptions::with_preset(6);
    options.dict_size = lzma_dictionary(data.len());
    options.lc = 3;
    options.lp = 0;
    options.pb = 2;
    let mut writer = lzma_rust2::LzmaWriter::new_no_header(Vec::new(), &options, false).ok()?;
    writer.write_all(data).ok()?;
    writer.finish().ok()
}

/// A raw LZMA stream (no header: the properties are the encoder's fixed lc=3 lp=0 pb=2) of exactly `size` bytes.
pub(crate) fn lzma_unpack(src: &[u8], size: usize) -> Result<Vec<u8>, ChdError> {
    let mut reader = lzma_rust2::LzmaReader::new_with_props(
        Cursor::new(src),
        size as u64,
        LZMA_PROPS,
        lzma_dictionary(size),
        None,
    )
    .or_else(|e| corrupt(&format!("an LZMA stream cannot start ({e})")))?;
    let mut out = vec![0u8; size];
    reader
        .read_exact(&mut out)
        .or_else(|e| corrupt(&format!("an LZMA stream is damaged ({e})")))?;
    Ok(out)
}

/// FLAC frames without a stream header (16 bits, two channels) holding `size` bytes of samples; returns the samples as bytes in the
/// order `big_endian` names, and how many source bytes the frames took.
pub(crate) fn flac_unpack(
    src: &[u8],
    size: usize,
    big_endian: bool,
) -> Result<(Vec<u8>, usize), ChdError> {
    let mut cursor = Cursor::new(src);
    let mut frames = claxon::frame::FrameReader::new(&mut cursor);
    let mut out = Vec::with_capacity(size);
    let mut buffer = Vec::new();
    while out.len() < size {
        let block = frames
            .read_next_or_eof(std::mem::take(&mut buffer))
            .map_err(|e| bad(&format!("a FLAC frame is damaged ({e})")))?
            .ok_or_else(|| bad("a FLAC stream ends early"))?;
        if block.channels() != 2 {
            return corrupt("a FLAC frame is not stereo");
        }
        for i in 0..block.duration() {
            if out.len() >= size {
                break;
            }
            for channel in 0..2 {
                let sample = block.sample(channel, i) as i16;
                let bytes = if big_endian {
                    sample.to_be_bytes()
                } else {
                    sample.to_le_bytes()
                };
                out.extend_from_slice(&bytes);
            }
        }
        buffer = block.into_buffer();
    }
    Ok((out, cursor.position() as usize))
}

/// FLAC frames for `data` read as 16-bit stereo in the given byte order; `block` is the block size in samples.
pub(crate) fn flac_pack(data: &[u8], big_endian: bool, block: usize) -> Vec<u8> {
    let samples: Vec<(i16, i16)> = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| {
            let pair = |a: u8, b: u8| {
                if big_endian {
                    i16::from_be_bytes([a, b])
                } else {
                    i16::from_le_bytes([a, b])
                }
            };
            (pair(c[0], c[1]), pair(c[2], c[3]))
        })
        .collect();
    flac::encode(&samples, block)
}

/// The block size of a hunk's FLAC frames: the hunk's samples, halved until at most `limit`.
pub(crate) fn flac_block(bytes: usize, limit: usize) -> usize {
    let mut block = bytes / 4;
    while block > limit {
        block /= 2;
    }
    block.max(1)
}

fn flac_generic_pack(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() || !data.len().is_multiple_of(4) {
        return None;
    }
    let block = flac_block(data.len(), 2048);
    let little = flac_pack(data, false, block);
    let big = flac_pack(data, true, block);
    let (marker, frames) = if little.len() <= big.len() {
        (b'L', little)
    } else {
        (b'B', big)
    };
    let mut out = Vec::with_capacity(frames.len() + 1);
    out.push(marker);
    out.extend(frames);
    Some(out)
}

fn flac_generic_unpack(src: &[u8], size: usize) -> Result<Vec<u8>, ChdError> {
    let (&marker, frames) = src.split_first().ok_or_else(|| bad("an empty FLAC hunk"))?;
    let big = match marker {
        b'L' => false,
        b'B' => true,
        _ => return corrupt("a FLAC hunk starts with neither L nor B"),
    };
    Ok(flac_unpack(frames, size, big)?.0)
}

/// Compresses one hunk with `codec`; `None` when the codec cannot or the result is not smaller than the hunk.
pub fn compress(codec: Codec, hunk: &[u8]) -> Option<Vec<u8>> {
    let packed = match codec {
        Codec::Zlib => Some(deflate(hunk)),
        Codec::Zstd => Some(zstd_pack(hunk)),
        Codec::Lzma => lzma_pack(hunk),
        Codec::Huff => huff::compress(hunk),
        Codec::Flac => flac_generic_pack(hunk),
        cd => crate::cd::encode_hunk(cd, hunk),
    }?;
    (packed.len() < hunk.len()).then_some(packed)
}

/// Decompresses one hunk of `size` bytes.
pub fn decompress(codec: Codec, src: &[u8], size: usize) -> Result<Vec<u8>, ChdError> {
    match codec {
        Codec::Zlib => inflate(src, size),
        Codec::Zstd => zstd_unpack(src, size),
        Codec::Lzma => lzma_unpack(src, size),
        Codec::Huff => huff::decompress(src, size),
        Codec::Flac => flac_generic_unpack(src, size),
        cd => {
            if !size.is_multiple_of(crate::cd::FRAME_SIZE) {
                return corrupt("a CD hunk is not a whole number of frames");
            }
            crate::cd::decode_hunk(cd, src, size / crate::cd::FRAME_SIZE)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hunk(n: usize) -> Vec<u8> {
        // compressible, with structure of several kinds: runs, a ramp, a repeating phrase
        (0..n)
            .map(|i| match (i / 64) % 3 {
                0 => 0,
                1 => (i % 251) as u8,
                _ => b"retroclawd "[i % 11],
            })
            .collect()
    }

    #[test]
    fn every_generic_codec_round_trips() {
        for size in [4096usize, 1024, 20] {
            let hunk = sample_hunk(size);
            for codec in [Codec::Zlib, Codec::Zstd, Codec::Lzma, Codec::Huff] {
                if let Some(packed) = compress(codec, &hunk) {
                    assert!(packed.len() < hunk.len());
                    assert_eq!(
                        decompress(codec, &packed, size).unwrap(),
                        hunk,
                        "{codec:?} {size}"
                    );
                } else {
                    assert!(size < 100, "{codec:?} should pack {size} bytes");
                }
            }
        }
    }

    #[test]
    fn flac_hunks_keep_their_byte_order() {
        // stereo 16-bit audio as little-endian bytes, and the same audio as big-endian bytes
        let mut le = Vec::new();
        let mut be = Vec::new();
        for i in 0..1024i32 {
            let l = ((i * 37) % 2000 - 1000) as i16;
            let r = ((i * 11) % 500) as i16;
            le.extend_from_slice(&l.to_le_bytes());
            le.extend_from_slice(&r.to_le_bytes());
            be.extend_from_slice(&l.to_be_bytes());
            be.extend_from_slice(&r.to_be_bytes());
        }
        for hunk in [&le, &be] {
            let packed = compress(Codec::Flac, hunk).expect("audio packs");
            assert!(packed[0] == b'L' || packed[0] == b'B');
            assert_eq!(&decompress(Codec::Flac, &packed, hunk.len()).unwrap(), hunk);
        }
        // not a whole number of samples, or not audio: no FLAC
        assert!(compress(Codec::Flac, &le[..le.len() - 1]).is_none());
        // a bad marker is an error
        let mut packed = compress(Codec::Flac, &le).unwrap();
        packed[0] = b'X';
        assert!(decompress(Codec::Flac, &packed, le.len()).is_err());
    }

    #[test]
    fn tags_name_the_codecs_and_back() {
        for c in Codec::ALL {
            assert_eq!(Codec::from_tag(c.tag()), Some(c));
        }
        assert_eq!(Codec::from_tag(*b"avhu"), None);
        assert!(Codec::CdLzma.is_cd() && !Codec::Lzma.is_cd());
    }

    #[test]
    fn damaged_input_is_an_error_not_a_panic() {
        let hunk = sample_hunk(4096);
        for codec in [
            Codec::Zlib,
            Codec::Zstd,
            Codec::Lzma,
            Codec::Huff,
            Codec::Flac,
        ] {
            let hunk = if codec == Codec::Flac {
                sample_hunk(4096)
            } else {
                hunk.clone()
            };
            let packed = compress(codec, &hunk).unwrap_or_else(|| vec![0; 10]);
            for cut in [0, 1, packed.len() / 2] {
                let _ = decompress(codec, &packed[..cut.min(packed.len())], 4096);
            }
            let mut flipped = packed.clone();
            for i in 0..flipped.len().min(64) {
                flipped[i] ^= 0xA5;
                let _ = decompress(codec, &flipped, 4096);
            }
        }
    }
}
