//! The Hudson HuC6260 video colour encoder: a 512-entry palette of 9-bit colours (3 bits each of green, red and
//! blue), the dot clock selection and the number of lines per frame. Facts: `machines/nec/docs/hardware-facts.md`,
//! rows C1-C8.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Vce {
    palette: Vec<u16>,
    control: u8,
    address: u16,
}

impl Default for Vce {
    fn default() -> Self {
        Self::new()
    }
}

impl Vce {
    #[must_use]
    pub fn new() -> Self {
        Self {
            palette: vec![0; 512],
            control: 0,
            address: 0,
        }
    }

    /// A write to the encoder's registers (`port` is the address within `$0400-$0407`).
    pub fn write(&mut self, port: u8, v: u8) {
        match port & 7 {
            0 => self.control = v,
            2 => self.address = (self.address & 0x100) | u16::from(v),
            3 => self.address = (self.address & 0x0FF) | (u16::from(v & 1) << 8),
            4 => {
                let a = usize::from(self.address & 0x1FF);
                self.palette[a] = (self.palette[a] & 0x100) | u16::from(v);
            }
            5 => {
                let a = usize::from(self.address & 0x1FF);
                self.palette[a] = (self.palette[a] & 0x0FF) | (u16::from(v & 1) << 8);
                self.address = (self.address + 1) & 0x1FF;
            }
            _ => {}
        }
    }

    pub fn read(&mut self, port: u8) -> u8 {
        match port & 7 {
            4 => self.palette[usize::from(self.address & 0x1FF)] as u8,
            5 => {
                let a = usize::from(self.address & 0x1FF);
                let v = 0xFE | ((self.palette[a] >> 8) as u8 & 1);
                self.address = (self.address + 1) & 0x1FF;
                v
            }
            _ => 0xFF,
        }
    }

    /// The dot clock: 0 = 5.37 MHz (256 dots), 1 = 7.16 MHz (336-352 dots), 2/3 = 10.74 MHz (512 dots).
    #[must_use]
    pub fn dot_clock(&self) -> u8 {
        self.control & 3
    }

    /// Lines per frame: 262, or 263 when control bit 2 is set.
    #[must_use]
    pub fn lines(&self) -> u16 {
        if self.control & 4 != 0 { 263 } else { 262 }
    }

    /// The colour of palette entry `index` as 8-bit RGB.
    #[must_use]
    pub fn rgb(&self, index: u16) -> [u8; 3] {
        let c = self.palette[usize::from(index & 0x1FF)];
        let scale = |v: u16| (u32::from(v & 7) * 255 / 7) as u8;
        if self.control & 0x80 != 0 {
            // black and white output: the luminance of the colour
            let (g, r, b) = (
                u32::from((c >> 6) & 7),
                u32::from((c >> 3) & 7),
                u32::from(c & 7),
            );
            let y = ((r * 299 + g * 587 + b * 114) * 255 / (7 * 1000)) as u8;
            return [y, y, y];
        }
        [scale(c >> 3), scale(c >> 6), scale(c)]
    }

    /// The raw 9-bit colour of an entry.
    #[must_use]
    pub fn entry(&self, index: u16) -> u16 {
        self.palette[usize::from(index & 0x1FF)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_writes_and_auto_increment() {
        let mut v = Vce::new();
        v.write(2, 0x10);
        v.write(3, 0x00);
        v.write(4, 0x38); // low byte: R=7 B=0
        v.write(5, 0x01); // G bit 8
        v.write(4, 0x07);
        v.write(5, 0x00);
        assert_eq!(v.entry(0x10), 0x138);
        assert_eq!(v.entry(0x11), 0x007);
        assert_eq!(v.rgb(0x10), [255, 145, 0]);
        assert_eq!(v.rgb(0x11), [0, 0, 255]);
    }

    #[test]
    fn dot_clock_and_line_count() {
        let mut v = Vce::new();
        v.write(0, 0x05);
        assert_eq!(v.dot_clock(), 1);
        assert_eq!(v.lines(), 263);
    }
}
