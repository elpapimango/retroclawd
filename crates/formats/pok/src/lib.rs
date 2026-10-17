//! Parser for Spectrum `.POK` trainer files.
//!
//! The text layout follows ZEsarUX's `util_parse_pok_mem` implementation and
//! the format description embedded in that source. This is a fresh parser;
//! no ZEsarUX code is copied. See `CREDITS.md`.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PokFile {
    pub trainers: Vec<PokTrainer>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PokTrainer {
    pub name: String,
    pub entries: Vec<PokEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PokEntry {
    /// Raw POK bank field. Bit 3 means "ignore bank"; values below 8 target
    /// a specific 16K bank on 128K Spectrum hardware.
    pub bank: u8,
    pub address: u16,
    pub value: u8,
    pub original: Option<u8>,
}

impl PokEntry {
    #[must_use]
    pub const fn ignores_bank(&self) -> bool {
        self.bank & 0b1000 != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidUtf8,
    InvalidLine { line: usize, message: String },
    EmptyFile,
    NoTrainers,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8 => write!(f, "POK file is not valid UTF-8 text"),
            Self::InvalidLine { line, message } => write!(f, "invalid POK line {line}: {message}"),
            Self::EmptyFile => write!(f, "POK file is empty"),
            Self::NoTrainers => write!(f, "POK file contains no trainer entries"),
        }
    }
}

impl std::error::Error for Error {}

/// Parses a Spectrum `.POK` trainer file.
///
/// `N` starts a trainer, `M` adds a non-final poke, `Z` adds the final poke of
/// that trainer, and `Y` terminates the file. A missing final `Y` is accepted,
/// matching established loaders that stop at end of text.
pub fn parse(bytes: &[u8]) -> Result<PokFile, Error> {
    if bytes.is_empty() {
        return Err(Error::EmptyFile);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)?;
    let mut trainers = Vec::new();
    let mut current: Option<PokTrainer> = None;
    let mut saw_terminator = false;

    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let Some(kind) = line.as_bytes().first().copied() else {
            continue;
        };
        match kind {
            b'N' => {
                if let Some(trainer) = current.take()
                    && !trainer.entries.is_empty()
                {
                    trainers.push(trainer);
                }
                current = Some(PokTrainer {
                    name: line[1..].trim().to_string(),
                    entries: Vec::new(),
                });
            }
            b'M' | b'Z' => {
                let trainer = current.as_mut().ok_or_else(|| Error::InvalidLine {
                    line: line_number,
                    message: "poke entry appears before trainer name".to_string(),
                })?;
                let fields: Vec<&str> = line[1..].split_whitespace().collect();
                if !(3..=4).contains(&fields.len()) {
                    return Err(Error::InvalidLine {
                        line: line_number,
                        message: "expected bank address value [original]".to_string(),
                    });
                }
                let entry = PokEntry {
                    bank: parse_decimal(fields[0], line_number, "bank")?,
                    address: parse_decimal(fields[1], line_number, "address")?,
                    value: parse_decimal(fields[2], line_number, "value")?,
                    original: fields
                        .get(3)
                        .map(|field| parse_decimal(field, line_number, "original"))
                        .transpose()?,
                };
                trainer.entries.push(entry);
                if kind == b'Z' {
                    let trainer = current.take().expect("current trainer exists");
                    trainers.push(trainer);
                }
            }
            b'Y' => {
                if line[1..].trim().is_empty() {
                    if let Some(trainer) = current.take()
                        && !trainer.entries.is_empty()
                    {
                        trainers.push(trainer);
                    }
                    saw_terminator = true;
                    break;
                }
                return Err(Error::InvalidLine {
                    line: line_number,
                    message: "Y terminator must not have fields".to_string(),
                });
            }
            _ => {
                return Err(Error::InvalidLine {
                    line: line_number,
                    message: "expected N, M, Z, or Y record".to_string(),
                });
            }
        }
    }

    if !saw_terminator
        && let Some(trainer) = current.take()
        && !trainer.entries.is_empty()
    {
        trainers.push(trainer);
    }
    if trainers.is_empty() {
        return Err(Error::NoTrainers);
    }
    Ok(PokFile { trainers })
}

fn parse_decimal<T>(field: &str, line: usize, name: &str) -> Result<T, Error>
where
    T: TryFrom<u64>,
{
    let value = field.parse::<u64>().map_err(|_| Error::InvalidLine {
        line,
        message: format!("{name} must be decimal"),
    })?;
    T::try_from(value).map_err(|_| Error::InvalidLine {
        line,
        message: format!("{name} is out of range"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_trainers_and_poke_groups() {
        let file = parse(
            b"NImmunity\r\nM 8 44278 58 0\r\nZ 8 44285 58 0\r\nNInfinite lives\nZ 8 50000 1\nY\n",
        )
        .expect("parse");
        assert_eq!(file.trainers.len(), 2);
        assert_eq!(file.trainers[0].name, "Immunity");
        assert_eq!(file.trainers[0].entries.len(), 2);
        assert!(file.trainers[0].entries[0].ignores_bank());
        assert_eq!(file.trainers[0].entries[1].original, Some(0));
        assert_eq!(file.trainers[1].name, "Infinite lives");
    }

    #[test]
    fn keeps_bank_specific_entries() {
        let file = parse(b"NBanked\nZ 3 1234 99\nY\n").expect("parse");
        let entry = &file.trainers[0].entries[0];
        assert_eq!(entry.bank, 3);
        assert!(!entry.ignores_bank());
    }

    fn rejects_malformed_records() {
        let error = parse(b"NBroken\nZ 8 nope 1\nY\n").expect_err("must reject");
        assert!(matches!(error, Error::InvalidLine { line: 2, .. }));
    }

    fn rejects_entries_without_trainer() {
        let error = parse(b"Z 8 100 1\n").expect_err("must reject");
        assert!(matches!(error, Error::InvalidLine { line: 1, .. }));
    }
    /// Merged (every check kept): `rejects_malformed_records`, `rejects_entries_without_trainer`.
    #[test]
    fn rejects_bad_files() {
        rejects_malformed_records();
        rejects_entries_without_trainer();
    }
}
