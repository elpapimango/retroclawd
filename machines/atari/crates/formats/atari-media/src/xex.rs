use crate::bounded;

/// A load segment. Order and overlaps are significant and are never normalized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: u16,
    pub bytes: Vec<u8>,
}

/// XEX load segments. INITAD/RUNAD are ordinary bytes here, interpreted by the machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xex {
    pub segments: Vec<Segment>,
}

impl Xex {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        bounded(bytes.len())?;
        if !bytes.starts_with(&[0xff, 0xff]) {
            return Err("XEX requires the FFFF header".into());
        }
        let mut at = 2;
        let mut segments = Vec::new();
        while at < bytes.len() {
            let mut start = word(bytes, &mut at)?;
            while start == 0xffff {
                start = word(bytes, &mut at)?;
            }
            let end = word(bytes, &mut at)?;
            if end < start {
                return Err("XEX segment wraps the 16-bit address space".into());
            }
            let len = usize::from(end) - usize::from(start) + 1;
            let data = bytes
                .get(at..at + len)
                .ok_or("truncated XEX segment data")?;
            segments.push(Segment {
                start,
                bytes: data.to_vec(),
            });
            at += len;
        }
        if segments.is_empty() {
            return Err("XEX contains no segments".into());
        }
        Ok(Self { segments })
    }

    /// Emits one FFFF header and ordered segments without optional separators.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        if self.segments.is_empty() {
            return Err("XEX contains no segments".into());
        }
        let mut size = 2usize;
        for segment in &self.segments {
            segment_end(segment)?;
            size = size
                .checked_add(4)
                .and_then(|n| n.checked_add(segment.bytes.len()))
                .ok_or("XEX size overflow")?;
            bounded(size)?;
        }
        let mut bytes = Vec::with_capacity(size);
        bytes.extend_from_slice(&[0xff, 0xff]);
        for segment in &self.segments {
            bytes.extend_from_slice(&segment.start.to_le_bytes());
            bytes.extend_from_slice(&segment_end(segment)?.to_le_bytes());
            bytes.extend_from_slice(&segment.bytes);
        }
        Ok(bytes)
    }
}

fn segment_end(segment: &Segment) -> Result<u16, String> {
    if segment.start == 0xffff {
        return Err("XEX segment start FFFF is reserved for separators".into());
    }
    let len = segment.bytes.len();
    if len == 0 || len > 0x10000 - usize::from(segment.start) {
        return Err("XEX segment is empty or wraps the 16-bit address space".into());
    }
    Ok((usize::from(segment.start) + len - 1) as u16)
}

fn word(bytes: &[u8], at: &mut usize) -> Result<u16, String> {
    let data = bytes
        .get(*at..*at + 2)
        .ok_or("truncated XEX segment header")?;
    *at += 2;
    Ok(u16::from_le_bytes([data[0], data[1]]))
}
