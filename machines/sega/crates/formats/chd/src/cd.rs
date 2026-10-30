//! The CD hunk codecs. A hunk holds `frames` frames of 2448 bytes (a 2352-byte sector and 96 bytes of subcode). The codec splits
//! it: the main data of every frame is compressed as one run by the base codec (`cdzl` deflate, `cdlz` LZMA, `cdfl` FLAC) and the
//! subcode of every frame as one run by deflate. A `cdzl` or `cdlz` hunk starts with one bit per frame saying that the frame's sync
//! pattern and parity were removed, and the length of the base run; a `cdfl` hunk is audio only and carries neither.
//!
//! Audio comes out in the byte order the format stores it, big-endian 16-bit samples; the disc layer turns it round.

use crate::ChdError;
use crate::ecc;
use std::io::{Cursor, Read};

/// lc = 3, lp = 0, pb = 2 as the properties byte: (pb * 5 + lp) * 9 + lc.
const LZMA_PROPS: u8 = 93;

pub const MAIN_SIZE: usize = 2352;
pub const SUBCODE_SIZE: usize = 96;
pub const FRAME_SIZE: usize = MAIN_SIZE + SUBCODE_SIZE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Base {
    Deflate,
    Lzma,
    Flac,
}

impl Base {
    pub(crate) fn from_tag(tag: [u8; 4]) -> Option<Base> {
        match &tag {
            b"cdzl" => Some(Base::Deflate),
            b"cdlz" => Some(Base::Lzma),
            b"cdfl" => Some(Base::Flac),
            _ => None,
        }
    }
}

fn bad(why: &str) -> ChdError {
    ChdError::Format(format!("a hunk fails to decode: {why}"))
}

fn corrupt<T>(why: &str) -> Result<T, ChdError> {
    Err(bad(why))
}

/// A raw deflate stream that must inflate to exactly `size` bytes.
fn inflate(src: &[u8], size: usize) -> Result<Vec<u8>, ChdError> {
    let mut out = vec![0u8; size];
    let mut decoder = flate2::read::DeflateDecoder::new(src);
    decoder
        .read_exact(&mut out)
        .or_else(|_| corrupt("a deflate stream is short or damaged"))?;
    Ok(out)
}

/// A raw LZMA stream (no header: the properties are the encoder's fixed lc=3 lp=0 pb=2) of exactly `size` bytes.
fn unlzma(src: &[u8], size: usize) -> Result<Vec<u8>, ChdError> {
    // the dictionary only has to reach as far back as one hunk's data goes
    let dict = size.next_power_of_two().max(4096) as u32;
    let mut reader = lzma_rust2::LzmaReader::new_with_props(
        Cursor::new(src),
        size as u64,
        LZMA_PROPS,
        dict,
        None,
    )
    .or_else(|e| corrupt(&format!("an LZMA stream cannot start ({e})")))?;
    let mut out = vec![0u8; size];
    reader
        .read_exact(&mut out)
        .or_else(|e| corrupt(&format!("an LZMA stream is damaged ({e})")))?;
    Ok(out)
}

/// FLAC frames without a stream header (CD audio: 16 bits, two channels) holding `size` bytes of samples; returns the samples as
/// big-endian bytes and how many source bytes the frames took.
fn unflac(src: &[u8], size: usize) -> Result<(Vec<u8>, usize), ChdError> {
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
            out.extend_from_slice(&(block.sample(0, i) as i16).to_be_bytes());
            out.extend_from_slice(&(block.sample(1, i) as i16).to_be_bytes());
        }
        buffer = block.into_buffer();
    }
    Ok((out, cursor.position() as usize))
}

/// Decodes one hunk of `frames` frames into `frames * FRAME_SIZE` bytes.
pub(crate) fn decode_hunk(base: Base, src: &[u8], frames: usize) -> Result<Vec<u8>, ChdError> {
    let main_size = frames * MAIN_SIZE;
    let sub_size = frames * SUBCODE_SIZE;
    let (main, flags, sub_src): (Vec<u8>, &[u8], &[u8]) = match base {
        Base::Flac => {
            let (main, used) = unflac(src, main_size)?;
            (main, &[], src.get(used..).unwrap_or(&[]))
        }
        Base::Deflate | Base::Lzma => {
            let length_bytes = if frames * FRAME_SIZE < 65536 { 2 } else { 3 };
            let flag_bytes = frames.div_ceil(8);
            let header = flag_bytes + length_bytes;
            let head = src
                .get(..header)
                .ok_or_else(|| bad("a hunk is shorter than its header"))?;
            let length = head[flag_bytes..]
                .iter()
                .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
            let base_src = src
                .get(header..header + length)
                .ok_or_else(|| bad("a hunk's base data is cut short"))?;
            let main = if base == Base::Deflate {
                inflate(base_src, main_size)?
            } else {
                unlzma(base_src, main_size)?
            };
            (main, &head[..flag_bytes], &src[header + length..])
        }
    };
    let sub = inflate(sub_src, sub_size)?;
    let mut out = vec![0u8; frames * FRAME_SIZE];
    for frame in 0..frames {
        let dest = &mut out[frame * FRAME_SIZE..(frame + 1) * FRAME_SIZE];
        dest[..MAIN_SIZE].copy_from_slice(&main[frame * MAIN_SIZE..(frame + 1) * MAIN_SIZE]);
        dest[MAIN_SIZE..].copy_from_slice(&sub[frame * SUBCODE_SIZE..(frame + 1) * SUBCODE_SIZE]);
        if flags
            .get(frame / 8)
            .is_some_and(|f| f >> (frame % 8) & 1 != 0)
        {
            let sector: &mut [u8; MAIN_SIZE] = (&mut dest[..MAIN_SIZE]).try_into().unwrap();
            sector[..12].copy_from_slice(&ecc::SYNC);
            ecc::generate(sector);
        }
    }
    Ok(out)
}
