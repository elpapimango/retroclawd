//! Pure Atari media containers; no CPU, machine, filesystem or serial-device behavior.
//!
//! Format facts checked against Atari800 7.2.1, revision
//! `63a381884175e3cbfc745ff00dfb3d70d928eb50`:
//! - [`binload.c`](https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/src/binload.c),
//!   `BINLOAD_Loader` / `loader_cont`: XEX signature, little-endian inclusive ranges and separators.
//! - [`afile.h`](https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/src/afile.h),
//!   `AFILE_ATR_Header`: 16-byte header, magic, paragraph count and write-protection byte.
//! - [`sio.c`](https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/src/sio.c),
//!   `SIO_Mount` / `SIO_SizeOfSector`: 128/256-byte sectors and logical 128-byte boot sectors.
//!
//! This is an independent implementation of those format facts, not a translation.
//! Unlike that emulator's permissive loader, malformed/truncated input is rejected.
//! Only canonical ATR logical boot sectors are supported; physical/SIO2PC boot layouts,
//! nonzero extension bytes 8..14, and sector sizes other than 128/256 are refused.
//! The complete accepted header is retained, including the write-protection byte
//! (any nonzero value protects the image, as in Atari800). No DOS is assumed.
//! All inputs and serialized outputs are bounded to 16 MiB.

#![forbid(unsafe_code)]

mod atr;
mod xex;

pub use atr::Atr;
pub use xex::{Segment, Xex};

const MAX_BYTES: usize = 16 * 1024 * 1024;

fn bounded(size: usize) -> Result<(), String> {
    if size > MAX_BYTES {
        Err("Atari media exceeds the 16 MiB limit".into())
    } else {
        Ok(())
    }
}
