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
//!
//! XFD accepts standard 720x128, 1040x128, 720x256 and 1440x256 images.
//! DD logical/physical boot layouts normalize to ATR; physical padding must
//! be zero. `Atr::to_xfd` exports either layout without guessing a filesystem.
//!
//! CAR parsing verifies its big-endian header, byte-sum checksum, reserved
//! bytes and mapper-specific size. Supported types are 1/2 (standard 8K/16K)
//! and 12/13/14 (XEGS 32K/64K/128K), per the pinned Atari800 `DOC/cart.txt`.
//! Raw images default only to standard 8K/16K; banked dumps require an explicit
//! `Mapper`. The cartridge exposes read mapping and D5 write banking separately
//! so the Machine can prioritize it over BASIC/RAM for both CPU and ANTIC.
//! Persist ROM identity, mapper and bank in a separate Machine state extension.
//!
//! CAS supports `FUJI`, `baud` and `data` per the
//! [A8CAS specification](https://a8cas.sourceforge.net/format-cas.html).
//! `CasPlayer` emits decoded 8N1 DATA IN in integer CPU clocks, gated by
//! the cassette motor. Persist the source/identity, clock and both fields of
//! `CasPosition`; no hidden byte shifter state exists outside that position.
//! Recording, WAV/audio demodulation, FSK/PWM turbo chunks, protected disk
//! formats, flash, cartridge RAM, OSS and other mappers are not implemented.

#![forbid(unsafe_code)]

mod atr;
mod cartridge;
mod cas;
mod xex;

pub use atr::Atr;
pub use cartridge::{Cartridge, Mapper};
pub use cas::{Cas, CasPlayer, CasPosition};
pub use xex::{Segment, Xex};

const MAX_BYTES: usize = 16 * 1024 * 1024;

fn bounded(size: usize) -> Result<(), String> {
    if size > MAX_BYTES {
        Err("Atari media exceeds the 16 MiB limit".into())
    } else {
        Ok(())
    }
}
