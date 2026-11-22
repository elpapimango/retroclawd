//! Cue sheets.

use crate::DiscError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Mode1Raw,
    Mode1Cooked,
    Mode2Raw,
    Mode2Cooked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CueTrack {
    pub number: u8,
    pub kind: TrackKind,
    pub pregap: u32,
    pub postgap: u32,
    /// `(index number, sector from the start of the file)`.
    pub indexes: Vec<(u8, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CueFile {
    pub name: String,
    pub tracks: Vec<CueTrack>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CueSheet {
    pub files: Vec<CueFile>,
}

/// The longest cue sheet read (a real one is a few hundred bytes).
pub const MAX_CUE_BYTES: usize = 64 * 1024;
/// A compact disc holds at most 99 tracks.
pub const MAX_TRACKS: usize = 99;
pub const MAX_FILES: usize = 100;

impl TrackKind {
    /// Bytes per sector in the BIN file.
    pub fn sector_size(self) -> usize {
        match self {
            TrackKind::Mode1Cooked | TrackKind::Mode2Cooked => 2048,
            _ => 2352,
        }
    }
}

fn err(line: usize, reason: impl Into<String>) -> DiscError {
    DiscError::Cue {
        line,
        reason: reason.into(),
    }
}

/// Splits a line into words; a word may be quoted with `"`.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

/// `mm:ss:ff` (two digits each) to sectors; 75 frames a second.
fn msf(text: &str, line: usize) -> Result<u32, DiscError> {
    let parts: Vec<&str> = text.split(':').collect();
    let ok = parts.len() == 3
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit()));
    if !ok {
        return Err(err(line, format!("bad time \"{text}\", expected mm:ss:ff")));
    }
    let n: Vec<u32> = parts.iter().map(|p| p.parse().unwrap_or(0)).collect();
    if n[1] > 59 || n[2] > 74 {
        return Err(err(line, format!("time \"{text}\" out of range")));
    }
    Ok((n[0] * 60 + n[1]) * 75 + n[2])
}

fn check_path(name: &str) -> Result<(), DiscError> {
    let bad = name.is_empty()
        || name.starts_with(['/', '\\'])
        || name.contains(':')
        || name.split(['/', '\\']).any(|part| part == "..");
    if bad {
        Err(DiscError::PathEscape(name.to_string()))
    } else {
        Ok(())
    }
}

/// Parses a cue sheet. Only `FILE`, `TRACK`, `PREGAP`, `POSTGAP` and `INDEX` matter; every other command is ignored.
pub fn parse(text: &str) -> Result<CueSheet, DiscError> {
    if text.len() > MAX_CUE_BYTES {
        return Err(DiscError::TooLarge(format!(
            "the cue sheet is {} bytes; at most {MAX_CUE_BYTES} are read",
            text.len()
        )));
    }
    let mut sheet = CueSheet::default();
    let mut tracks = 0usize;
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let w = words(raw);
        let Some(cmd) = w.first() else { continue };
        match cmd.to_ascii_uppercase().as_str() {
            "FILE" => {
                if w.len() < 2 {
                    return Err(err(line, "FILE needs a file name"));
                }
                check_path(&w[1])?;
                if sheet.files.len() >= MAX_FILES {
                    return Err(DiscError::TooLarge(format!("more than {MAX_FILES} files")));
                }
                sheet.files.push(CueFile {
                    name: w[1].clone(),
                    tracks: Vec::new(),
                });
            }
            "TRACK" => {
                let file = sheet
                    .files
                    .last_mut()
                    .ok_or_else(|| err(line, "TRACK before any FILE"))?;
                if w.len() < 3 {
                    return Err(err(line, "TRACK needs a number and a mode"));
                }
                let number: u8 = w[1]
                    .parse()
                    .ok()
                    .filter(|n| (1..=99).contains(n))
                    .ok_or_else(|| err(line, "track number must be 1 to 99"))?;
                let kind = match w[2].to_ascii_uppercase().as_str() {
                    "AUDIO" => TrackKind::Audio,
                    "MODE1/2352" => TrackKind::Mode1Raw,
                    "MODE1/2048" => TrackKind::Mode1Cooked,
                    "MODE2/2352" => TrackKind::Mode2Raw,
                    "MODE2/2048" => TrackKind::Mode2Cooked,
                    other => return Err(err(line, format!("unsupported track mode {other}"))),
                };
                tracks += 1;
                if tracks > MAX_TRACKS {
                    return Err(DiscError::TooLarge(format!(
                        "more than {MAX_TRACKS} tracks"
                    )));
                }
                file.tracks.push(CueTrack {
                    number,
                    kind,
                    pregap: 0,
                    postgap: 0,
                    indexes: Vec::new(),
                });
            }
            cmd @ ("PREGAP" | "POSTGAP" | "INDEX") => {
                let track = sheet
                    .files
                    .last_mut()
                    .and_then(|f| f.tracks.last_mut())
                    .ok_or_else(|| err(line, format!("{cmd} before any TRACK")))?;
                if cmd == "INDEX" {
                    if w.len() < 3 {
                        return Err(err(line, "INDEX needs a number and a time"));
                    }
                    let n: u8 = w[1]
                        .parse()
                        .ok()
                        .filter(|n| *n <= 99)
                        .ok_or_else(|| err(line, "index number must be 0 to 99"))?;
                    track.indexes.push((n, msf(&w[2], line)?));
                } else {
                    let t = w
                        .get(1)
                        .ok_or_else(|| err(line, format!("{cmd} needs a time")))?;
                    let v = msf(t, line)?;
                    if cmd == "PREGAP" {
                        track.pregap = v;
                    } else {
                        track.postgap = v;
                    }
                }
            }
            _ => {}
        }
    }
    if tracks == 0 {
        return Err(err(1, "the cue sheet has no track"));
    }
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINGLE: &str = "FILE \"game.bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 00 10:00:00\n    INDEX 01 10:02:00\n";

    #[test]
    fn parses_a_single_bin_cue() {
        let sheet = parse(SINGLE).unwrap();
        assert_eq!(sheet.files.len(), 1);
        let file = &sheet.files[0];
        assert_eq!(file.name, "game.bin");
        assert_eq!(file.tracks.len(), 2);
        assert_eq!(file.tracks[0].kind, TrackKind::Mode1Raw);
        assert_eq!(file.tracks[0].indexes, vec![(1, 0)]);
        assert_eq!(file.tracks[1].kind, TrackKind::Audio);
        assert_eq!(file.tracks[1].indexes, vec![(0, 45_000), (1, 45_150)]);
    }

    #[test]
    fn parses_one_bin_per_track() {
        let text = "FILE a.bin BINARY\nTRACK 01 MODE1/2048\nINDEX 01 00:00:00\nFILE b.bin BINARY\nTRACK 02 AUDIO\nPREGAP 00:02:00\nINDEX 01 00:00:00\n";
        let sheet = parse(text).unwrap();
        assert_eq!(sheet.files.len(), 2);
        assert_eq!(sheet.files[0].tracks[0].kind, TrackKind::Mode1Cooked);
        assert_eq!(sheet.files[1].tracks[0].pregap, 150);
    }

    #[test]
    fn msf_converts_to_sectors() {
        let text = "FILE a.bin BINARY\nTRACK 01 AUDIO\nINDEX 01 00:02:00\nINDEX 02 74:59:74\n";
        let sheet = parse(text).unwrap();
        assert_eq!(
            sheet.files[0].tracks[0].indexes,
            vec![(1, 150), (2, 337_499)]
        );
    }

    #[test]
    fn rejects_bad_msf() {
        for bad in ["00:60:00", "00:00:75", "aa:bb:cc", "1:2", "100:00:00"] {
            let text = format!("FILE a.bin BINARY\nTRACK 01 AUDIO\nINDEX 01 {bad}\n");
            assert!(parse(&text).is_err(), "{bad}");
        }
    }

    #[test]
    fn rejects_path_escape() {
        for bad in [
            "../x.bin",
            "/abs.bin",
            r"C:\x.bin",
            "a/../../b.bin",
            r"..\x.bin",
        ] {
            let text = format!("FILE \"{bad}\" BINARY\nTRACK 01 AUDIO\nINDEX 01 00:00:00\n");
            assert!(
                matches!(parse(&text), Err(DiscError::PathEscape(_))),
                "{bad}"
            );
        }
        // a subfolder is fine
        let ok = "FILE \"sub/x.bin\" BINARY\nTRACK 01 AUDIO\nINDEX 01 00:00:00\n";
        assert!(parse(ok).is_ok());
    }

    #[test]
    fn rejects_unknown_mode_and_misplaced_lines() {
        assert!(parse("FILE a.bin BINARY\nTRACK 01 CDG/2448\nINDEX 01 00:00:00\n").is_err());
        assert!(parse("TRACK 01 AUDIO\n").is_err());
        assert!(parse("FILE a.bin BINARY\nINDEX 01 00:00:00\n").is_err());
        assert!(parse("FILE a.bin BINARY\nTRACK 00 AUDIO\n").is_err());
        assert!(parse("FILE a.bin BINARY\nTRACK 100 AUDIO\n").is_err());
        assert!(parse("").is_err(), "a sheet with no track is not a disc");
    }

    #[test]
    fn bounds_hostile_input() {
        let mut many = String::from("FILE a.bin BINARY\n");
        for i in 0..200_000 {
            many.push_str(&format!("REM {i}\n"));
        }
        assert!(matches!(parse(&many), Err(DiscError::TooLarge(_))));
        let mut tracks = String::from("FILE a.bin BINARY\n");
        for i in 0..1000 {
            tracks.push_str(&format!("TRACK {:02} AUDIO\nINDEX 01 00:00:00\n", i % 100));
        }
        assert!(parse(&tracks).is_err());
        assert!(parse(&"\u{0}\u{1}\u{ff}garbage".repeat(50)).is_err());
    }

    #[test]
    fn accepts_quotes_rem_lines_and_crlf() {
        let text = "REM GENRE Test\r\nPERFORMER \"x\"\r\nFILE \"my game.bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    FLAGS DCP\r\n    INDEX 01 00:00:00\r\n";
        let sheet = parse(text).unwrap();
        assert_eq!(sheet.files[0].name, "my game.bin");
        assert_eq!(sheet.files[0].tracks[0].kind, TrackKind::Mode2Raw);
    }
}
