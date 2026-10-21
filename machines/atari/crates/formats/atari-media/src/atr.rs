use crate::bounded;
use std::ops::Range;

/// Canonical ATR image with a retained, unmodified 16-byte header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atr {
    bytes: Vec<u8>,
    sectors: u16,
    sector_size: u16,
}

impl Atr {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        bounded(bytes.len())?;
        let header = bytes.get(..16).ok_or("truncated ATR header")?;
        if header[..2] != [0x96, 0x02] {
            return Err("ATR requires the 9602 signature".into());
        }
        if header[8..15].iter().any(|&b| b != 0) {
            return Err("extended/protected ATR header is unsupported (bytes 8..14)".into());
        }
        let sector_size = u16::from_le_bytes([header[4], header[5]]);
        valid_size(sector_size)?;
        let paragraphs = u32::from_le_bytes([header[2], header[3], header[6], header[7]]);
        let size = u64::from(paragraphs) * 16;
        if size != (bytes.len() - 16) as u64 {
            return Err("ATR paragraph count does not match the image length".into());
        }
        let size = bytes.len() - 16;
        let sectors = if sector_size == 128 || size <= 384 {
            if !size.is_multiple_of(128) {
                return Err("ATR has a partial 128-byte sector".into());
            }
            size / 128
        } else {
            if !(size - 384).is_multiple_of(256) {
                return Err(
                    "unsupported ATR geometry: expected three 128-byte boot sectors".into(),
                );
            }
            3 + (size - 384) / 256
        };
        let sectors = u16::try_from(sectors)
            .ok()
            .filter(|&n| n != 0)
            .ok_or("ATR sector count must be 1..65535")?;
        Ok(Self {
            bytes: bytes.to_vec(),
            sectors,
            sector_size,
        })
    }

    /// Makes an unformatted, zero-filled sector image, not an invented DOS filesystem.
    pub fn blank(sectors: u16, sector_size: u16) -> Result<Self, String> {
        valid_size(sector_size)?;
        if sectors == 0 {
            return Err("ATR sector count must be 1..65535".into());
        }
        let count = usize::from(sectors);
        let boot = count.min(3);
        let size = boot * 128 + (count - boot) * usize::from(sector_size);
        bounded(size + 16)?;
        let mut bytes = vec![0; size + 16];
        bytes[..2].copy_from_slice(&[0x96, 0x02]);
        let paragraphs = (size as u32 / 16).to_le_bytes();
        bytes[2..4].copy_from_slice(&paragraphs[..2]);
        bytes[4..6].copy_from_slice(&sector_size.to_le_bytes());
        bytes[6..8].copy_from_slice(&paragraphs[2..]);
        Ok(Self {
            bytes,
            sectors,
            sector_size,
        })
    }

    /// Reads a one-based sector; the first three sectors are always 128 bytes.
    pub fn sector(&self, number: u16) -> Result<&[u8], String> {
        Ok(&self.bytes[self.range(number)?])
    }

    pub fn sector_mut(&mut self, number: u16) -> Result<&mut [u8], String> {
        if self.write_protected() {
            return Err("ATR is write protected".into());
        }
        let range = self.range(number)?;
        Ok(&mut self.bytes[range])
    }

    pub fn sector_count(&self) -> u16 {
        self.sectors
    }
    pub fn sector_size(&self) -> u16 {
        self.sector_size
    }
    pub fn write_protected(&self) -> bool {
        self.bytes[15] != 0
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    fn range(&self, number: u16) -> Result<Range<usize>, String> {
        if number == 0 || number > self.sectors {
            return Err(format!(
                "ATR sector {number} is outside 1..{}",
                self.sectors
            ));
        }
        let index = usize::from(number - 1);
        let (offset, size) = if index < 3 {
            (index * 128, 128)
        } else {
            let size = usize::from(self.sector_size);
            (384 + (index - 3) * size, size)
        };
        Ok(16 + offset..16 + offset + size)
    }
}

fn valid_size(size: u16) -> Result<(), String> {
    if size == 128 || size == 256 {
        Ok(())
    } else {
        Err(format!(
            "unsupported ATR sector size {size}; expected 128 or 256"
        ))
    }
}
