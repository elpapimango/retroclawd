//! The metadata chain after the hunks: each entry is a 4-byte tag, a flags byte, a 24-bit length and the file offset of the next
//! entry, followed by its data. The CD track descriptions are text.

use crate::{ChdError, format_err};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// How many entries of the chain are read and how long one may be.
const MAX_ENTRIES: usize = 4096;
const MAX_ENTRY_BYTES: u32 = 1 << 20;

pub(crate) struct Entry {
    pub tag: [u8; 4],
    pub data: Vec<u8>,
}

pub(crate) fn read_chain(
    file: &mut File,
    mut offset: u64,
    file_len: u64,
) -> Result<Vec<Entry>, ChdError> {
    let io = |e: std::io::Error| ChdError::Io(format!("reading the metadata: {e}"));
    let mut entries = Vec::new();
    while offset != 0 {
        if entries.len() >= MAX_ENTRIES {
            return format_err("the metadata chain is too long");
        }
        if offset.checked_add(16).is_none_or(|end| end > file_len) {
            return format_err("the metadata chain leaves the file");
        }
        file.seek(SeekFrom::Start(offset)).map_err(io)?;
        let mut head = [0u8; 16];
        file.read_exact(&mut head).map_err(io)?;
        let tag = [head[0], head[1], head[2], head[3]];
        let length = u32::from_be_bytes([0, head[5], head[6], head[7]]);
        let next = u64::from_be_bytes(head[8..16].try_into().unwrap());
        if length > MAX_ENTRY_BYTES || offset + 16 + u64::from(length) > file_len {
            return format_err("a metadata entry is longer than the file");
        }
        let mut data = vec![0u8; length as usize];
        file.read_exact(&mut data).map_err(io)?;
        entries.push(Entry { tag, data });
        if next != 0 && next <= offset {
            return format_err("the metadata chain loops");
        }
        offset = next;
    }
    Ok(entries)
}

/// A track as the metadata describes it (`CHT2`, or the older `CHTR` that lacks the pregap fields).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrackText {
    pub number: u32,
    pub kind: String,
    pub frames: u32,
    pub pregap: u32,
    /// A pregap type starting with `V` means the pregap sectors are not in the file.
    pub pregap_in_file: bool,
    pub postgap: u32,
}

pub(crate) fn parse_track(text: &[u8], has_pregap: bool) -> Result<TrackText, ChdError> {
    let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
    let text = String::from_utf8_lossy(&text[..end]);
    let field = |name: &str| -> Option<&str> {
        text.split_ascii_whitespace()
            .find_map(|part| part.strip_prefix(name)?.strip_prefix(':'))
    };
    let number = |name: &str| -> Result<u32, ChdError> {
        field(name)
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| ChdError::Format(format!("a track description lacks {name}")))
    };
    let kind = field("TYPE")
        .ok_or_else(|| ChdError::Format("a track description lacks TYPE".to_string()))?
        .to_string();
    let (pregap, pregap_in_file, postgap) = if has_pregap {
        (
            number("PREGAP")?,
            !field("PGTYPE").unwrap_or("").starts_with('V'),
            number("POSTGAP")?,
        )
    } else {
        (0, false, 0)
    };
    Ok(TrackText {
        number: number("TRACK")?,
        kind,
        frames: number("FRAMES")?,
        pregap,
        pregap_in_file,
        postgap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_description_is_read_by_its_keywords() {
        let t = parse_track(
            b"TRACK:2 TYPE:AUDIO SUBTYPE:NONE FRAMES:1500 PREGAP:150 PGTYPE:VAUDIO PGSUB:NONE POSTGAP:0\0",
            true,
        )
        .unwrap();
        assert_eq!(
            t,
            TrackText {
                number: 2,
                kind: "AUDIO".into(),
                frames: 1500,
                pregap: 150,
                pregap_in_file: false,
                postgap: 0
            }
        );
        let stored = parse_track(
            b"TRACK:1 TYPE:MODE1_RAW SUBTYPE:NONE FRAMES:9 PREGAP:2 PGTYPE:MODE1 PGSUB:NONE POSTGAP:0",
            true,
        )
        .unwrap();
        assert!(stored.pregap_in_file);
        let old = parse_track(b"TRACK:1 TYPE:MODE1 SUBTYPE:NONE FRAMES:5", false).unwrap();
        assert_eq!((old.pregap, old.frames), (0, 5));
        assert!(parse_track(b"TRACK:1 SUBTYPE:NONE FRAMES:5", false).is_err());
        assert!(parse_track(b"TYPE:AUDIO FRAMES:5", false).is_err());
    }
}
