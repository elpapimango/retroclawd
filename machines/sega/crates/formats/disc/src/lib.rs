//! CD images for the Sega CD: cue sheets, BIN and ISO files, sector reads and the table of contents.
//!
//! The input is untrusted: a hostile cue sheet or a truncated image returns a `DiscError`, never a panic, and a cue sheet
//! can only name files inside its own folder.
#![forbid(unsafe_code)]

pub mod builder;
pub mod cue;
mod disc;

pub use disc::{Disc, RAW_SECTOR, SECTORS_PER_SECOND, Track, USER_SECTOR};

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscError {
    /// The cue sheet is malformed; the line number (1-based) and the reason.
    Cue {
        line: usize,
        reason: String,
    },
    /// A limit of the format or of this reader was exceeded.
    TooLarge(String),
    /// A file name in a cue sheet that leaves the cue's folder.
    PathEscape(String),
    Io(String),
    Unsupported(String),
    /// A sector outside the disc.
    OutOfRange(u32),
}

impl fmt::Display for DiscError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiscError::Cue { line, reason } => write!(f, "cue sheet line {line}: {reason}"),
            DiscError::TooLarge(what) => write!(f, "disc image too large: {what}"),
            DiscError::PathEscape(name) => write!(
                f,
                "cue sheet file \"{name}\" is outside the cue sheet's folder"
            ),
            DiscError::Io(msg) => write!(f, "disc image: {msg}"),
            DiscError::Unsupported(what) => write!(f, "disc image format not supported: {what}"),
            DiscError::OutOfRange(lba) => write!(f, "sector {lba} is outside the disc"),
        }
    }
}

impl std::error::Error for DiscError {}
