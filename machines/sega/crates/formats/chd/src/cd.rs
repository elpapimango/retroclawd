//! The CD hunk codecs. A hunk holds `frames` frames of 2448 bytes (a 2352-byte sector and 96 bytes of subcode). The codec splits
//! it: the main data of every frame is compressed as one run by the base codec (`cdzl` deflate, `cdzs` Zstandard, `cdlz` LZMA, `cdfl`
//! FLAC for audio) and the subcode of every frame as one run by deflate (Zstandard for `cdzs`). A `cdzl`, `cdzs` or `cdlz` hunk
//! starts with one bit per frame saying that the frame's sync pattern and parity were removed, and the length of the base run; a
//! `cdfl` hunk is audio only and carries neither.
//!
//! Audio comes out in the byte order the format stores it, big-endian 16-bit samples; the disc layer turns it round.

use crate::ChdError;
use crate::codec::{self, Codec, bad};
use crate::ecc;

pub const MAIN_SIZE: usize = 2352;
pub const SUBCODE_SIZE: usize = 96;
pub const FRAME_SIZE: usize = MAIN_SIZE + SUBCODE_SIZE;

/// Where the parity of a data sector starts; the sync pattern and everything from here are what a flagged frame leaves out.
const PARITY_START: usize = 0x81C;

fn unpack_sub(codec: Codec, src: &[u8], size: usize) -> Result<Vec<u8>, ChdError> {
    if codec == Codec::CdZstd {
        codec::zstd_unpack(src, size)
    } else {
        codec::inflate(src, size)
    }
}

/// Decodes one hunk of `frames` frames into `frames * FRAME_SIZE` bytes.
pub(crate) fn decode_hunk(codec: Codec, src: &[u8], frames: usize) -> Result<Vec<u8>, ChdError> {
    let main_size = frames * MAIN_SIZE;
    let sub_size = frames * SUBCODE_SIZE;
    let (main, flags, sub_src): (Vec<u8>, &[u8], &[u8]) = match codec {
        Codec::CdFlac => {
            let (main, used) = codec::flac_unpack(src, main_size, true)?;
            (main, &[], src.get(used..).unwrap_or(&[]))
        }
        Codec::CdZlib | Codec::CdZstd | Codec::CdLzma => {
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
            let main = match codec {
                Codec::CdZlib => codec::inflate(base_src, main_size)?,
                Codec::CdZstd => codec::zstd_unpack(base_src, main_size)?,
                _ => codec::lzma_unpack(base_src, main_size)?,
            };
            (main, &head[..flag_bytes], &src[header + length..])
        }
        _ => return Err(bad("not a CD codec")),
    };
    let sub = unpack_sub(codec, sub_src, sub_size)?;
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

/// Encodes one hunk of whole frames; `None` when a codec step fails (the caller then tries another codec or stores the hunk).
pub(crate) fn encode_hunk(codec: Codec, hunk: &[u8]) -> Option<Vec<u8>> {
    if hunk.is_empty() || !hunk.len().is_multiple_of(FRAME_SIZE) {
        return None;
    }
    let frames = hunk.len() / FRAME_SIZE;
    let sub: Vec<u8> = hunk
        .as_chunks::<FRAME_SIZE>()
        .0
        .iter()
        .flat_map(|f| f[MAIN_SIZE..].iter().copied())
        .collect();
    let packed_sub = if codec == Codec::CdZstd {
        codec::zstd_pack(&sub)
    } else {
        codec::deflate(&sub)
    };
    if codec == Codec::CdFlac {
        // audio only: a frame that is a data sector (it starts with the sync pattern) is not audio
        if hunk
            .as_chunks::<FRAME_SIZE>()
            .0
            .iter()
            .any(|f| f[..12] == ecc::SYNC)
        {
            return None;
        }
        let main: Vec<u8> = hunk
            .as_chunks::<FRAME_SIZE>()
            .0
            .iter()
            .flat_map(|f| f[..MAIN_SIZE].iter().copied())
            .collect();
        let block = codec::flac_block(main.len(), 2352);
        let mut out = codec::flac_pack(&main, true, block);
        out.extend(packed_sub);
        return Some(out);
    }
    let mut flags = vec![0u8; frames.div_ceil(8)];
    let mut main = Vec::with_capacity(frames * MAIN_SIZE);
    for (i, f) in hunk.as_chunks::<FRAME_SIZE>().0.iter().enumerate() {
        let mut sector = [0u8; MAIN_SIZE];
        sector.copy_from_slice(&f[..MAIN_SIZE]);
        if ecc::is_regenerable(&sector) {
            flags[i / 8] |= 1 << (i % 8);
            sector[..12].fill(0);
            sector[PARITY_START..].fill(0);
        }
        main.extend_from_slice(&sector);
    }
    let base = match codec {
        Codec::CdZlib => codec::deflate(&main),
        Codec::CdZstd => codec::zstd_pack(&main),
        _ => codec::lzma_pack(&main)?,
    };
    let length_bytes = if hunk.len() < 65536 { 2 } else { 3 };
    let length = base.len();
    if length >= 1 << (8 * length_bytes) {
        return None;
    }
    let mut out = flags;
    out.extend_from_slice(&(length as u32).to_be_bytes()[4 - length_bytes..]);
    out.extend(base);
    out.extend(packed_sub);
    Some(out)
}
