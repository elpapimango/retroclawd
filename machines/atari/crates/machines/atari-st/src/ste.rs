//! The registers the STE and Mega ST add: the Blitter is a device (`blitter`), the others plain register files until their
//! devices exist (Tasks 53 to 55 replace each file by its device). The reads are what Hatari 2.6.1 returned for EmuTOS's accesses.

use serde::{Deserialize, Serialize};

/// Clocks the Microwire interface takes for a bit, and the clocks after the write before the first bit moves.
pub const MICROWIRE_BIT_CLOCKS: u64 = 8;
pub const MICROWIRE_START_DELAY: u64 = 8;

/// The STE and Mega ST's added registers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SteRegs {
    pub sound: Vec<u8>,
    /// The clock at which the Microwire data register was last written (the shift runs from then).
    pub microwire_start: Option<u64>,
    pub joy: Vec<u8>,
    pub blitter: blitter::Blitter,
    /// The DMA sound and the volume control behind the Microwire interface.
    pub dma: crate::dmasound::DmaSound,
    pub rtc: Vec<u8>,
}

impl Default for SteRegs {
    fn default() -> Self {
        Self {
            sound: vec![0; 0x26],
            microwire_start: None,
            joy: vec![0xFF; 0x24],
            blitter: blitter::Blitter::new(),
            dma: crate::dmasound::DmaSound::default(),
            rtc: vec![0; 0x20],
        }
    }
}

fn read(file: &[u8], at: usize, word: bool) -> u32 {
    if word {
        (u32::from(file[at]) << 8) | u32::from(file.get(at + 1).copied().unwrap_or(0xFF))
    } else {
        u32::from(file[at])
    }
}

fn write(file: &mut [u8], at: usize, word: bool, value: u32) {
    if word {
        file[at] = (value >> 8) as u8;
        if let Some(b) = file.get_mut(at + 1) {
            *b = value as u8;
        }
    } else {
        file[at] = value as u8;
    }
}

impl SteRegs {
    /// Whether a state's register files have the lengths of the real ones (a state is untrusted).
    pub fn check(&self) -> Result<(), crate::state::StateError> {
        let ok = self.sound.len() == 0x26
            && self.joy.len() == 0x24
            && self.blitter.is_consistent()
            && self.dma.is_consistent()
            && self.rtc.len() == 0x20;
        if ok {
            Ok(())
        } else {
            Err(crate::state::StateError::Payload(
                "a register file of the state has the wrong size".into(),
            ))
        }
    }

    pub fn sound(&mut self, addr: u32, word: bool, write_value: Option<u32>, now: u64) -> u32 {
        let at = (addr - 0xFF_8900) as usize;
        if at < 0x22 {
            return self.dma_register(at, word, write_value, now);
        }
        match write_value {
            Some(v) => {
                write(&mut self.sound, at, word, v);
                if (0x22..0x24).contains(&at) {
                    self.microwire_start = Some(now);
                    // The volume control takes the command when the data is written.
                    let data = read(&self.sound, 0x22, true) as u16;
                    let mask = read(&self.sound, 0x24, true) as u16;
                    self.dma.lmc.microwire(data, mask);
                }
                v
            }
            None if word && at == 0x22 => self.microwire_data(now),
            None => read(&self.sound, at, word),
        }
    }

    /// A DMA sound register (`$FF8900-$FF8921`) as a byte or a word.
    fn dma_register(&mut self, at: usize, word: bool, write_value: Option<u32>, now: u64) -> u32 {
        match write_value {
            Some(v) => {
                if word {
                    self.dma.write_byte(at, (v >> 8) as u8, now);
                    self.dma.write_byte(at + 1, v as u8, now);
                } else {
                    self.dma.write_byte(at, v as u8, now);
                }
                v
            }
            None => {
                let byte = |o: usize| u32::from(self.dma.read_byte(o).unwrap_or(0xFF));
                if word {
                    byte(at) << 8 | byte(at + 1)
                } else {
                    byte(at)
                }
            }
        }
    }

    /// The Microwire data register: the word written, shifted out to the left one bit every `MICROWIRE_BIT_CLOCKS` clocks
    /// from the write ([O] P30: Hatari 2.6.1 reads it shifted by about one bit per 8 clocks, zero once it is out).
    fn microwire_data(&self, now: u64) -> u32 {
        let data = read(&self.sound, 0x22, true);
        let Some(start) = self.microwire_start else {
            return data;
        };
        let shifts = now.saturating_sub(start + MICROWIRE_START_DELAY) / MICROWIRE_BIT_CLOCKS;
        if shifts >= 16 {
            0
        } else {
            (data << shifts) & 0xFFFF
        }
    }

    pub fn joy(&mut self, addr: u32, word: bool, write_value: Option<u32>) -> u32 {
        let at = (addr - 0xFF_9200) as usize;
        match write_value {
            Some(v) => {
                write(&mut self.joy, at, word, v);
                v
            }
            None => read(&self.joy, at, word),
        }
    }

    pub fn rtc(&mut self, addr: u32, write_value: Option<u32>) -> u32 {
        let at = (addr - 0xFF_FC20) as usize;
        match write_value {
            Some(v) => {
                self.rtc[at] = v as u8;
                v
            }
            None => u32::from(self.rtc[at]),
        }
    }
}
