//! The SN76489 as built into the Sega VDP (the 315-5313's integrated PSG): three square wave channels and a noise channel,
//! 4-bit attenuation in 2 dB steps. Facts S4-S9 in `docs/hardware-facts.md`.
//!
//! The chip counts in steps of 16 input clocks (the Z80 clock, master / 15, so one step is 240 master clocks); `tick` is one
//! step. A tone channel's output flips each time its counter, loaded with the 10-bit period, runs out; the noise channel
//! shifts a 16-bit register every `16 << rate` steps (or at the rate of tone channel 2) and outputs its low bit.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Master clocks of one PSG step: 15 per input clock, 16 input clocks.
pub const STEP_MASTER_CLOCKS: u64 = 15 * 16;

/// Attenuation steps 0-14 are 2 dB each, 15 is off.
const VOLUME: [f32; 16] = [
    1.0,
    0.794_328_2,
    0.630_957_34,
    0.501_187_2,
    0.398_107_17,
    0.316_227_77,
    0.251_188_64,
    0.199_526_23,
    0.158_489_32,
    0.125_892_54,
    0.1,
    0.079_432_82,
    0.063_095_73,
    0.050_118_72,
    0.039_810_72,
    0.0,
];

/// The noise shift register starts, and starts again after every write to the noise register, with only its top bit set.
const NOISE_SEED: u16 = 0x8000;
/// White noise feeds back bits 0 and 3 (the integrated chip's taps).
const NOISE_TAPS: u16 = 0x0009;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Psg {
    /// Tone periods (10 bits) of channels 0-2 and the noise control (3 bits) in `noise`.
    tone: [u16; 3],
    noise: u8,
    /// Attenuations 0-15 of the three tones and the noise.
    attenuation: [u8; 4],
    /// The register the next data byte goes to: bits 6-4 of the last latch byte.
    latch: u8,
    counter: [u16; 4],
    /// The tone flip-flops: high outputs the channel's volume, low outputs nothing.
    high: [bool; 3],
    lfsr: u16,
}

impl Psg {
    pub fn new() -> Self {
        Self {
            tone: [0; 3],
            noise: 0,
            // Silent at power on: the reference emulators and the console's own start-up leave the PSG quiet (fact S6).
            attenuation: [0xF; 4],
            // The power-on latch is tone 2's attenuation (verified only on the integrated chip: fact S6).
            latch: 3,
            counter: [0; 4],
            high: [false; 3],
            lfsr: NOISE_SEED,
        }
    }

    /// Writes a byte to the data port. A byte with bit 7 set latches a register and writes its low 4 bits; any other byte
    /// writes the high 6 bits of the latched tone period (or the low bits of the latched register).
    pub fn write(&mut self, byte: u8) {
        if byte & 0x80 != 0 {
            self.latch = (byte >> 4) & 7;
            match self.latch {
                0 | 2 | 4 => {
                    let c = usize::from(self.latch >> 1);
                    self.tone[c] = (self.tone[c] & 0x3F0) | u16::from(byte & 0x0F);
                }
                6 => self.write_noise(byte & 7),
                l => self.attenuation[usize::from(l >> 1)] = byte & 0x0F,
            }
        } else {
            match self.latch {
                0 | 2 | 4 => {
                    let c = usize::from(self.latch >> 1);
                    self.tone[c] = (self.tone[c] & 0x00F) | u16::from(byte & 0x3F) << 4;
                }
                6 => self.write_noise(byte & 7),
                l => self.attenuation[usize::from(l >> 1)] = byte & 0x0F,
            }
        }
    }

    fn write_noise(&mut self, value: u8) {
        self.noise = value;
        // The shift register restarts whenever the noise register is written (fact S7).
        self.lfsr = NOISE_SEED;
        if value & 3 == 3 {
            self.counter[3] = self.counter[2];
        } else {
            self.counter[3] = 0;
        }
    }

    /// The period a tone channel reloads: zero acts as one on the integrated chip (fact S6).
    fn period(&self, channel: usize) -> u16 {
        self.tone[channel].max(1)
    }

    /// The period of the noise shift register in steps.
    fn noise_period(&self) -> u16 {
        match self.noise & 3 {
            3 => self.period(2),
            n => 0x10 << n,
        }
    }

    /// One step of the chip (240 master clocks).
    pub fn tick(&mut self) {
        for c in 0..3 {
            if self.counter[c] <= 1 {
                self.counter[c] = self.period(c);
                self.high[c] = !self.high[c];
            } else {
                self.counter[c] -= 1;
            }
        }
        if self.counter[3] <= 1 {
            self.counter[3] = self.noise_period();
            self.shift_noise();
        } else {
            self.counter[3] -= 1;
        }
    }

    fn shift_noise(&mut self) {
        let feedback = if self.noise & 4 != 0 {
            u16::from((self.lfsr & NOISE_TAPS).count_ones() & 1 == 1)
        } else {
            self.lfsr & 1
        };
        self.lfsr = self.lfsr >> 1 | feedback << 15;
    }

    /// The mixed output right now: the sum of the channels' levels, each between 0 and 1 (nothing when a channel is low).
    pub fn output(&self) -> f32 {
        let mut sum = 0.0;
        for c in 0..3 {
            if self.high[c] {
                sum += VOLUME[usize::from(self.attenuation[c])];
            }
        }
        if self.lfsr & 1 != 0 {
            sum += VOLUME[usize::from(self.attenuation[3])];
        }
        sum
    }

    /// Whether a value read back from a saved state is usable.
    pub fn validate(&self) -> Result<(), String> {
        if self.latch > 7
            || self.noise > 7
            || self.attenuation.iter().any(|a| *a > 15)
            || self.tone.iter().any(|t| *t > 0x3FF)
        {
            return Err("a PSG register is out of range".to_string());
        }
        Ok(())
    }

    pub fn tone_period(&self, channel: usize) -> u16 {
        self.tone[channel]
    }

    pub fn attenuation(&self, channel: usize) -> u8 {
        self.attenuation[channel]
    }

    pub fn noise_register(&self) -> u8 {
        self.noise
    }

    pub fn noise_register_value(&self) -> u16 {
        self.lfsr
    }
}

impl Default for Psg {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(psg: &mut Psg, channel: u8, period: u16) {
        psg.write(0x80 | channel << 5 | (period & 0x0F) as u8);
        psg.write((period >> 4) as u8 & 0x3F);
    }

    fn volume(psg: &mut Psg, channel: u8, attenuation: u8) {
        psg.write(0x90 | channel << 5 | attenuation);
    }

    #[test]
    fn a_tone_period_is_written_in_two_bytes() {
        let mut p = Psg::new();
        tone(&mut p, 1, 0x2A5);
        assert_eq!(p.tone_period(1), 0x2A5);
        // the second byte alone changes the high bits of the latched register
        p.write(0x03);
        assert_eq!(p.tone_period(1), 0x035);
        p.write(0xA2);
        assert_eq!(p.tone_period(1), 0x032);
    }

    #[test]
    fn volume_is_attenuation_in_2_db_steps_with_15_off() {
        let mut levels = Vec::new();
        for a in [0u8, 1, 5, 14, 15] {
            let mut q = Psg::new();
            volume(&mut q, 0, a);
            q.high[0] = true;
            levels.push(q.output());
        }
        assert!((levels[0] - 1.0).abs() < 1e-6);
        assert!((levels[1] - 0.794_328).abs() < 1e-4, "-2 dB");
        assert!((levels[2] - 0.316_228).abs() < 1e-4, "-10 dB");
        assert!((levels[3] - 0.039_811).abs() < 1e-4, "-28 dB");
        assert_eq!(levels[4], 0.0);
    }

    #[test]
    fn a_tone_flips_every_period_steps() {
        let mut p = Psg::new();
        tone(&mut p, 0, 4);
        volume(&mut p, 0, 0);
        let mut flips = Vec::new();
        let mut last = p.high[0];
        for step in 1..=24 {
            p.tick();
            if p.high[0] != last {
                flips.push(step);
                last = p.high[0];
            }
        }
        // the counter starts at zero, so the first step reloads and flips; then every 4 steps
        assert_eq!(flips, vec![1, 5, 9, 13, 17, 21]);
    }

    #[test]
    fn period_zero_acts_as_one() {
        let mut p = Psg::new();
        tone(&mut p, 2, 0);
        let mut last = p.high[2];
        let mut flips = 0;
        for _ in 0..10 {
            p.tick();
            if p.high[2] != last {
                flips += 1;
                last = p.high[2];
            }
        }
        assert_eq!(flips, 10);
    }

    #[test]
    fn white_noise_follows_the_taps_of_bits_0_and_3() {
        let mut p = Psg::new();
        p.write(0xE4); // white noise, rate 0 (16 steps)
        assert_eq!(p.noise_register_value(), 0x8000);
        let mut seen = vec![p.noise_register_value()];
        for _ in 0..4 {
            for _ in 0..16 {
                p.tick();
            }
            seen.push(p.noise_register_value());
        }
        assert_eq!(seen[0], 0x8000);
        assert_eq!(
            seen[1], 0x4000,
            "bits 0 and 3 are zero: a 0 enters at the top"
        );
        // run a full cycle: 15-bit... the register must not stick
        let mut distinct = std::collections::HashSet::new();
        for _ in 0..2000 {
            for _ in 0..16 {
                p.tick();
            }
            distinct.insert(p.noise_register_value());
        }
        assert!(
            distinct.len() > 1000,
            "the sequence is long: {}",
            distinct.len()
        );
    }

    #[test]
    fn periodic_noise_rotates_the_register() {
        let mut p = Psg::new();
        p.write(0xE3); // periodic, tied to tone 2
        tone(&mut p, 2, 1);
        let mut values = std::collections::HashSet::new();
        for _ in 0..100 {
            p.tick();
            values.insert(p.noise_register_value());
        }
        assert_eq!(values.len(), 16, "a single bit rotating through 16 places");
    }

    #[test]
    fn writing_the_noise_register_restarts_the_shift_register() {
        let mut p = Psg::new();
        p.write(0xE4);
        for _ in 0..200 {
            p.tick();
        }
        assert_ne!(p.noise_register_value(), 0x8000);
        p.write(0xE5);
        assert_eq!(p.noise_register_value(), 0x8000);
    }

    #[test]
    fn noise_rates_are_16_32_64_steps_or_tone_2() {
        let mut p = Psg::new();
        for (reg, steps) in [(0xE0u8, 16u16), (0xE1, 32), (0xE2, 64)] {
            p.write(reg);
            assert_eq!(p.noise_period(), steps);
        }
        tone(&mut p, 2, 100);
        p.write(0xE3);
        assert_eq!(p.noise_period(), 100);
    }

    #[test]
    fn the_noise_attenuation_register_is_the_latch_7_write() {
        let mut p = Psg::new();
        p.write(0xFA);
        assert_eq!(p.attenuation(3), 0x0A);
        p.write(0xDF);
        assert_eq!(p.attenuation(2), 0x0F);
    }

    #[test]
    fn the_power_on_latch_is_tone_2_attenuation() {
        let mut p = Psg::new();
        p.write(0x05);
        assert_eq!(
            p.attenuation(1),
            0x05,
            "a data byte right after power on goes to register 3"
        );
    }
}
