//! Standard CAS chunks: https://a8cas.sourceforge.net/format-cas.html
//! (`FUJI`, `baud`, `data`). Byte framing and motor: Altirra Hardware
//! Reference 2026-01-02, 9.1 and appendix C.1. Output is the deck's decoded SIO
//! DATA IN level, not an invented audio waveform or a ROM loader trap.
use crate::bounded;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Record {
    bytes: Range<usize>,
    baud: u16,
    gap_ms: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cas {
    bytes: Vec<u8>,
    records: Vec<Record>,
}

impl Cas {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        bounded(bytes.len())?;
        if bytes.get(..4) != Some(b"FUJI") {
            return Err("CAS must begin with a FUJI chunk".into());
        }
        let mut offset = 0;
        let mut baud = 600;
        let mut records = Vec::new();
        while offset < bytes.len() {
            let header = bytes
                .get(offset..offset + 8)
                .ok_or("truncated CAS chunk header")?;
            let length = usize::from(u16::from_le_bytes([header[4], header[5]]));
            let aux = u16::from_le_bytes([header[6], header[7]]);
            let start = offset + 8;
            let end = start + length;
            let data = bytes.get(start..end).ok_or("truncated CAS chunk data")?;
            match &header[..4] {
                b"FUJI" => {
                    std::str::from_utf8(data).map_err(|_| "CAS FUJI description is not UTF-8")?;
                }
                b"baud" => {
                    if length != 0 || aux == 0 {
                        return Err(
                            "CAS baud chunk requires no data and a nonzero baud rate".into()
                        );
                    }
                    baud = aux;
                }
                b"data" => records.push(Record {
                    bytes: start..end,
                    baud,
                    gap_ms: aux,
                }),
                tag => {
                    return Err(format!(
                        "unsupported CAS chunk {:?}; supported: FUJI, baud, data",
                        String::from_utf8_lossy(tag)
                    ));
                }
            }
            offset = end;
        }
        Ok(Self {
            bytes: bytes.to_vec(),
            records,
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }
    pub fn record_count(&self) -> usize {
        self.records.len()
    }
}

/// Explicit extension state; persist both fields with the CAS identity and clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CasPosition {
    pub record: u32,
    pub cycle: u64,
}

pub struct CasPlayer {
    cas: Cas,
    clock: u64,
    position: CasPosition,
}

impl CasPlayer {
    pub fn new(cas: Cas, clock: u64) -> Result<Self, String> {
        if !(1..=100_000_000).contains(&clock) {
            return Err("invalid CAS playback clock".into());
        }
        let mut player = Self {
            cas,
            clock,
            position: CasPosition::default(),
        };
        player.skip_empty();
        Ok(player)
    }
    fn gap(&self, record: &Record) -> u64 {
        (self.clock * u64::from(record.gap_ms)).div_ceil(1000)
    }
    fn duration(&self, record: &Record) -> u64 {
        self.gap(record)
            + (record.bytes.len() as u64 * 10 * self.clock).div_ceil(u64::from(record.baud))
    }
    fn skip_empty(&mut self) {
        while let Some(record) = self.cas.records.get(self.position.record as usize) {
            if self.duration(record) != 0 {
                break;
            }
            self.position.record += 1;
        }
    }
    pub fn position(&self) -> CasPosition {
        self.position
    }
    pub fn restore_position(&mut self, position: CasPosition) -> Result<(), String> {
        let valid = match self.cas.records.get(position.record as usize) {
            Some(record) => position.cycle < self.duration(record),
            None => position.record as usize == self.cas.records.len() && position.cycle == 0,
        };
        if !valid {
            return Err("invalid CAS playback position".into());
        }
        self.position = position;
        Ok(())
    }
    pub fn finished(&self) -> bool {
        self.position.record as usize == self.cas.records.len()
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        self.cas.to_bytes()
    }
    /// Gaps and record ends round up to CPU clocks. Bits use rational timing
    /// within each record, so nonintegral baud divisors do not accumulate drift.
    /// A stopped motor freezes the complete phase and releases DATA IN high.
    pub fn tick(&mut self, mut cycles: u64, motor: bool) -> bool {
        if !motor {
            return true;
        }
        while let Some(record) = self.cas.records.get(self.position.record as usize) {
            let remaining = self.duration(record) - self.position.cycle;
            if cycles < remaining {
                self.position.cycle += cycles;
                break;
            }
            cycles -= remaining;
            self.position.record += 1;
            self.position.cycle = 0;
            self.skip_empty();
        }
        self.input()
    }
    pub fn input(&self) -> bool {
        let Some(record) = self.cas.records.get(self.position.record as usize) else {
            return true;
        };
        let gap = self.gap(record);
        if self.position.cycle < gap {
            return true;
        }
        let bit = (self.position.cycle - gap) * u64::from(record.baud) / self.clock;
        match bit % 10 {
            0 => false,
            9 => true,
            index => {
                self.cas.bytes[record.bytes.start + (bit / 10) as usize] & (1 << (index - 1)) != 0
            }
        }
    }
}
