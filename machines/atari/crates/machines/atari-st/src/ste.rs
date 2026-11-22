//! The registers the STE and Mega ST add: the Blitter is a device (`blitter`), the others plain register files until their
//! devices exist (Tasks 53 to 55 replace each file by its device). The reads are what Hatari 2.6.1 returned for EmuTOS's accesses.

use serde::{Deserialize, Serialize};

/// Clocks the Microwire interface takes for a bit, and the clocks after the write before the first bit moves.
pub const MICROWIRE_BIT_CLOCKS: u64 = 8;
pub const MICROWIRE_START_DELAY: u64 = 8;

/// The Blitter's turn-taking: clock credit not yet spent on a word, CPU clocks run since its last burst, and the clock the
/// count began at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlitTiming {
    pub credit: u32,
    pub cpu: u64,
    pub mark: u64,
}

/// A button of an STE enhanced-port pad; the value is its bit in `SteRegs::pads`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadButton {
    Up = 0,
    Down = 1,
    Left = 2,
    Right = 3,
    A = 4,
    B = 5,
    C = 6,
    Option = 7,
    Pause = 8,
}

impl PadButton {
    /// The key-name suffix of the button (`Joy1PadUp`).
    pub const ALL: [(&'static str, PadButton); 9] = [
        ("Up", PadButton::Up),
        ("Down", PadButton::Down),
        ("Left", PadButton::Left),
        ("Right", PadButton::Right),
        ("A", PadButton::A),
        ("B", PadButton::B),
        ("C", PadButton::C),
        ("Option", PadButton::Option),
        ("Pause", PadButton::Pause),
    ];
}

/// The STE and Mega ST's added registers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SteRegs {
    pub sound: Vec<u8>,
    /// The clock at which the Microwire data register was last written (the shift runs from then).
    pub microwire_start: Option<u64>,
    /// The two enhanced-port pads as buttons held (see `PadButton`).
    pub pads: [u16; 2],
    pub blitter: blitter::Blitter,
    /// Where a Blitter that shares the bus is in its turn-taking with the CPU.
    pub blit_timing: BlitTiming,
    /// The DMA sound and the volume control behind the Microwire interface.
    pub dma: crate::dmasound::DmaSound,
    pub rtc: crate::rtc::Rtc,
}

impl Default for SteRegs {
    fn default() -> Self {
        Self {
            sound: vec![0; 0x26],
            microwire_start: None,
            pads: [0; 2],
            blitter: blitter::Blitter::new(),
            blit_timing: BlitTiming::default(),
            dma: crate::dmasound::DmaSound::default(),
            rtc: crate::rtc::Rtc::default(),
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
            && self.pads.iter().all(|p| p >> 9 == 0)
            && self.blitter.is_consistent()
            && self.dma.is_consistent()
            && self.blit_timing.credit <= 160_256
            && self.blit_timing.cpu <= 96 * 8
            && self.rtc.is_consistent();
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

    /// A pad button went down or up.
    pub fn set_pad(&mut self, pad: usize, button: PadButton, down: bool) {
        let bit = 1 << button as u16;
        if down {
            self.pads[pad & 1] |= bit;
        } else {
            self.pads[pad & 1] &= !bit;
        }
    }

    /// The enhanced joystick ports (`$FF9200-$FF9223`): `None` is a bus error. [O] Hatari 2.6.1, P31: at rest `$FF9200` and
    /// `$FF9202` read `$FFFF`, `$FF9201` and `$FF9203` `$FF`, the paddles `$FF9210-$FF9217` read `$FF` at even addresses and
    /// `$24` at odd ones, the light pen words `$FF9220/22` read 0; writes change nothing; `$FF9218-$FF921F`, and a byte read
    /// of `$FF9221` or `$FF9223`, are bus errors. [H?] which bit a held pad button clears (active low, as at rest): the
    /// directions are the four nibbles of `$FF9202` (up, down, left, right from bit 0; pad 1 the low nibble, pad 2 the
    /// next), the fire buttons the bits of `$FF9201` (Internals: Fire 0, 2, 1, 3 from bit 0, taken as pad 1 A, pad 2 A,
    /// pad 1 B, pad 2 B), C, Option and Pause bits 0-2 (pad 1) and 4-6 (pad 2) of `$FF9200`.
    pub fn joy(&mut self, addr: u32, word: bool, write_value: Option<u32>) -> Option<u32> {
        let at = addr - 0xFF_9200;
        if (0x18..0x20).contains(&at) || (!word && (at == 0x21 || at == 0x23)) {
            return None;
        }
        if let Some(v) = write_value {
            return Some(v);
        }
        let pad = |n: usize, bit: PadButton| u8::from(self.pads[n] >> bit as u16 & 1 != 0);
        let nibble = |n: usize| {
            pad(n, PadButton::Up)
                | pad(n, PadButton::Down) << 1
                | pad(n, PadButton::Left) << 2
                | pad(n, PadButton::Right) << 3
        };
        let misc = |n: usize| {
            pad(n, PadButton::C) | pad(n, PadButton::Option) << 1 | pad(n, PadButton::Pause) << 2
        };
        let byte = |o: u32| -> u8 {
            match o {
                0x00 => !(misc(0) | misc(1) << 4),
                0x01 => {
                    !(pad(0, PadButton::A)
                        | pad(1, PadButton::A) << 1
                        | pad(0, PadButton::B) << 2
                        | pad(1, PadButton::B) << 3)
                }
                0x03 => !(nibble(0) | nibble(1) << 4),
                0x11 | 0x13 | 0x15 | 0x17 => 0x24,
                0x20..=0x23 => 0,
                _ => 0xFF,
            }
        };
        Some(if word {
            u32::from(byte(at)) << 8 | u32::from(byte(at + 1))
        } else {
            u32::from(byte(at))
        })
    }
}
