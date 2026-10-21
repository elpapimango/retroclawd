//! GTIA normal playfield colors, console switches, speaker and trigger latches.
//! Source: Altirra Hardware Reference (2026-01-02), chapter 6 and section 14.3:
//! <https://www.virtualdub.org/downloads/Altirra%20Hardware%20Reference%20Manual.pdf>.
//! Initial slice: no P/M graphics/collisions, GTIA special modes or composite
//! artifacting. RGB is an analytical YIQ approximation, not a measured palette.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Gtia {
    registers: [u8; 32],
    triggers: [bool; 4],
    latched: [bool; 4],
    console: u8,
    pal: bool,
}

impl Gtia {
    pub fn new(pal: bool) -> Self {
        Self {
            registers: [0; 32],
            triggers: [true; 4],
            latched: [true; 4],
            console: 7,
            pal,
        }
    }
    pub fn read(&self, offset: u8) -> u8 {
        match offset & 31 {
            0..=15 => 0,
            16..=19 => u8::from(self.latched[usize::from(offset & 3)]),
            20 => {
                if self.pal {
                    1
                } else {
                    15
                }
            }
            31 => self.console & (!self.registers[31] & 7),
            _ => 15,
        }
    }
    pub fn write(&mut self, offset: u8, value: u8) {
        self.registers[usize::from(offset & 31)] = value;
        self.update_triggers();
    }
    pub fn register(&self, index: usize) -> u8 {
        self.registers[index & 31]
    }
    pub fn set_trigger(&mut self, port: usize, pressed: bool) -> Result<(), String> {
        let trigger = self
            .triggers
            .get_mut(port)
            .ok_or("GTIA trigger index exceeds 3")?;
        *trigger = !pressed;
        self.update_triggers();
        Ok(())
    }
    pub fn set_console(&mut self, bit: u8, pressed: bool) -> Result<(), String> {
        if bit > 2 {
            return Err("GTIA console bit exceeds 2".to_string());
        }
        if pressed {
            self.console &= !(1 << bit);
        } else {
            self.console |= 1 << bit;
        }
        Ok(())
    }
    fn update_triggers(&mut self) {
        for (latched, input) in self.latched.iter_mut().zip(self.triggers) {
            if self.registers[29] & 4 != 0 {
                *latched &= input;
            } else {
                *latched = input;
            }
        }
    }
    pub fn speaker(&self) -> bool {
        self.registers[31] & 8 == 0
    }
    pub fn color(&self, code: u8) -> u8 {
        match code {
            1..=4 => self.registers[21 + usize::from(code)] & 0xfe,
            5 => self.registers[24] & 0xfe,
            6 => (self.registers[24] & 0xf0) | (self.registers[23] & 0x0e),
            _ => self.registers[26] & 0xfe,
        }
    }
    pub fn palette() -> [[u8; 3]; 256] {
        std::array::from_fn(|index| {
            let hue = index >> 4;
            let y = (index & 14) as f64 / 14.0;
            let phase = (hue as f64 - 1.0) * std::f64::consts::TAU / 15.0 - 0.9;
            let saturation = if hue == 0 { 0.0 } else { 0.22 };
            let i = phase.cos() * saturation;
            let q = phase.sin() * saturation;
            [
                y + 0.956 * i + 0.621 * q,
                y - 0.272 * i - 0.647 * q,
                y - 1.106 * i + 1.703 * q,
            ]
            .map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.console > 7 {
            return Err("invalid GTIA state".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mirrored_registers_hires_luminance_and_trigger_latching() {
        let mut g = Gtia::new(true);
        g.write(0x38, 0x94);
        g.write(23, 0x0e);
        assert_eq!(g.color(5), 0x94);
        assert_eq!(g.color(6), 0x9e);
        g.write(29, 4);
        g.set_trigger(0, true).unwrap();
        g.set_trigger(0, false).unwrap();
        assert_eq!(g.read(16), 0);
        g.write(29, 0);
        assert_eq!(g.read(16), 1);
        g.set_console(0, true).unwrap();
        assert_eq!(g.read(31), 6);
        g.write(31, 2);
        assert_eq!(g.read(31), 4);
    }
}
