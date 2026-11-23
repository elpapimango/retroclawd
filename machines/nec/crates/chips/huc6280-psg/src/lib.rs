//! The programmable sound generator of the HuC6280: six channels that each play a 32-sample, 5-bit waveform
//! (or a directly written sample), channels 5 and 6 can make noise, channel 2 can be frequency-modulated by
//! channel 1 (the LFO), and every channel and the master have left/right levels.
//!
//! The chip runs at the master clock divided by six (3.579545 MHz); this crate produces stereo samples at
//! `SAMPLE_RATE`. Facts: `machines/nec/docs/hardware-facts.md`, rows P1-P14.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const SAMPLE_RATE: u32 = 44_100;
const MASTER_HZ: u64 = 21_477_272;

#[derive(Clone, Copy, Serialize, Deserialize)]
struct Channel {
    freq: u16,
    control: u8,
    pan: u8,
    wave: [u8; 32],
    write_index: u8,
    index: u8,
    counter: u32,
    dda: u8,
    noise: u8,
    noise_counter: u32,
    noise_out: bool,
}

impl Channel {
    fn new() -> Self {
        Self {
            freq: 0,
            control: 0,
            pan: 0,
            wave: [0; 32],
            write_index: 0,
            index: 0,
            counter: 0,
            dda: 0,
            noise: 0,
            noise_counter: 0,
            noise_out: false,
        }
    }
    fn enabled(&self) -> bool {
        self.control & 0x80 != 0
    }
    fn dda_mode(&self) -> bool {
        self.control & 0x40 != 0
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Psg {
    ch: [Channel; 6],
    select: u8,
    balance: u8,
    lfo_freq: u8,
    lfo_ctrl: u8,
    lfsr: u32,
    /// Output-sample phase, in master clocks times the sample rate.
    phase: u64,
    /// PSG ticks owed (fractional carry, in 1/SAMPLE_RATE units).
    tick_carry: u64,
    out: Vec<f32>,
    /// The AC-coupling filter's memory (left and right: last input, last output).
    dc: [(f32, f32); 2],
}

impl Default for Psg {
    fn default() -> Self {
        Self::new()
    }
}

/// Linear gain for an attenuation in decibels.
fn gain(db: f32) -> f32 {
    10f32.powf(-db / 20.0)
}

impl Psg {
    #[must_use]
    pub fn new() -> Self {
        Self {
            ch: [Channel::new(); 6],
            select: 0,
            balance: 0,
            lfo_freq: 0,
            lfo_ctrl: 0,
            lfsr: 0x4000,
            phase: 0,
            tick_carry: 0,
            out: Vec::new(),
            dc: [(0.0, 0.0); 2],
        }
    }

    pub fn write(&mut self, port: u8, v: u8) {
        let sel = usize::from(self.select.min(5));
        match port & 0xF {
            0 => self.select = v & 7,
            1 => self.balance = v,
            2 => self.ch[sel].freq = (self.ch[sel].freq & 0xF00) | u16::from(v),
            3 => self.ch[sel].freq = (self.ch[sel].freq & 0x0FF) | (u16::from(v & 0x0F) << 8),
            4 => {
                let c = &mut self.ch[sel];
                let was_dda = c.dda_mode();
                c.control = v;
                if v & 0x40 != 0 && !was_dda && v & 0x80 == 0 {
                    c.index = 0;
                }
                if v & 0xC0 == 0x40 {
                    c.index = 0;
                }
                if v & 0x80 == 0 && v & 0x40 == 0 {
                    // the write pointer restarts when the channel is switched off
                    c.write_index = 0;
                }
            }
            5 => self.ch[sel].pan = v,
            6 => {
                let c = &mut self.ch[sel];
                if c.dda_mode() && !c.enabled() {
                    c.dda = v & 0x1F;
                } else if !c.enabled() {
                    c.wave[usize::from(c.write_index & 31)] = v & 0x1F;
                    c.write_index = (c.write_index + 1) & 31;
                }
            }
            7 => {
                if sel >= 4 {
                    self.ch[sel].noise = v;
                }
            }
            8 => self.lfo_freq = v,
            9 => {
                self.lfo_ctrl = v;
                if v & 0x80 != 0 {
                    self.ch[0].index = 0;
                    self.ch[1].index = 0;
                }
            }
            _ => {}
        }
    }

    #[must_use]
    pub fn read(&self, _port: u8) -> u8 {
        0xFF
    }

    fn sample_value(&self, i: usize) -> i32 {
        let c = &self.ch[i];
        if !c.enabled() && !c.dda_mode() {
            return 0;
        }
        if c.dda_mode() && !c.enabled() {
            return i32::from(c.dda) - 16;
        }
        if i >= 4 && c.noise & 0x80 != 0 {
            return if c.noise_out { 15 } else { -16 };
        }
        i32::from(c.wave[usize::from(c.index & 31)]) - 16
    }

    /// One tick of the 3.58 MHz sound clock.
    fn tick(&mut self) {
        let lfo_on = self.lfo_ctrl & 3 != 0;
        let modulation = if lfo_on {
            let m = i32::from(self.ch[0].wave[usize::from(self.ch[0].index & 31)]) - 16;
            let shift = [0, 0, 4, 8][usize::from(self.lfo_ctrl & 3)];
            if self.lfo_ctrl & 3 == 1 {
                m
            } else {
                m << shift
            }
        } else {
            0
        };
        for i in 0..6 {
            let period = if i == 1 && lfo_on {
                (i32::from(self.ch[1].freq) + modulation).clamp(1, 0xFFF) as u32
            } else if i == 0 && lfo_on {
                u32::from(self.ch[0].freq.max(1))
                    * u32::from(if self.lfo_freq == 0 {
                        256
                    } else {
                        u16::from(self.lfo_freq)
                    })
            } else if self.ch[i].freq == 0 {
                4096
            } else {
                u32::from(self.ch[i].freq)
            };
            let c = &mut self.ch[i];
            if c.counter <= 1 {
                c.counter = period;
                c.index = (c.index + 1) & 31;
            } else {
                c.counter -= 1;
            }
        }
        for i in 4..6 {
            if self.ch[i].noise & 0x80 != 0 {
                let c = &mut self.ch[i];
                if c.noise_counter <= 1 {
                    c.noise_counter = 64 * (32 - u32::from(c.noise & 0x1F));
                    let bit = (self.lfsr ^ (self.lfsr >> 1)) & 1;
                    self.lfsr = (self.lfsr >> 1) | (bit << 14);
                    c.noise_out = self.lfsr & 1 != 0;
                } else {
                    c.noise_counter -= 1;
                }
            }
        }
    }

    fn mix(&self) -> (f32, f32) {
        let mut l = 0.0f32;
        let mut r = 0.0f32;
        let master_l = f32::from(self.balance >> 4);
        let master_r = f32::from(self.balance & 15);
        for i in 0..6 {
            let c = &self.ch[i];
            if (!c.enabled() && !c.dda_mode()) || (i == 0 && self.lfo_ctrl & 3 != 0) {
                continue;
            }
            let vol = f32::from(c.control & 0x1F);
            let s = self.sample_value(i) as f32 / 16.0;
            let base = 1.5 * (31.0 - vol);
            let gl = gain(base + 3.0 * (15.0 - f32::from(c.pan >> 4)) + 3.0 * (15.0 - master_l));
            let gr = gain(base + 3.0 * (15.0 - f32::from(c.pan & 15)) + 3.0 * (15.0 - master_r));
            l += s * if c.pan >> 4 == 0 || master_l == 0.0 {
                0.0
            } else {
                gl
            };
            r += s * if c.pan & 15 == 0 || master_r == 0.0 {
                0.0
            } else {
                gr
            };
        }
        (l * 0.1655, r * 0.1655)
    }

    /// Advances by `master` master-clock cycles, appending the stereo samples finished meanwhile.
    pub fn run(&mut self, master: u64) {
        self.phase += master * u64::from(SAMPLE_RATE);
        // each output sample spans MASTER_HZ / SAMPLE_RATE master clocks = 1/6 of that in PSG ticks
        while self.phase >= MASTER_HZ {
            self.phase -= MASTER_HZ;
            self.tick_carry += MASTER_HZ / 6;
            let ticks = self.tick_carry / u64::from(SAMPLE_RATE);
            self.tick_carry %= u64::from(SAMPLE_RATE);
            for _ in 0..ticks {
                self.tick();
            }
            let (l, r) = self.mix();
            // the output is AC coupled: a channel parked at a non-zero level does not leave an offset
            for (i, v) in [l, r].into_iter().enumerate() {
                let (x1, y1) = self.dc[i];
                let y = v - x1 + 0.998 * y1;
                self.dc[i] = (v, y);
                self.out.push(y);
            }
        }
    }

    /// The stereo samples (interleaved left, right) since the last call.
    pub fn drain(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(psg: &mut Psg, ch: u8, freq: u16) {
        psg.write(0, ch);
        psg.write(1, 0xFF);
        psg.write(4, 0x00); // off, write pointer reset
        for i in 0..32 {
            psg.write(6, if i < 16 { 0x1F } else { 0x00 });
        }
        psg.write(2, freq as u8);
        psg.write(3, (freq >> 8) as u8);
        psg.write(5, 0xFF);
        psg.write(4, 0x9F);
    }

    #[test]
    fn a_square_wave_plays_at_the_programmed_pitch() {
        let mut psg = Psg::new();
        square(&mut psg, 0, 0x100);
        psg.run(MASTER_HZ); // one second
        let out = psg.drain();
        assert_eq!(out.len(), 2 * SAMPLE_RATE as usize);
        // crossings of zero: frequency = 3579545 / (32 * 256)
        let mut crossings = 0;
        let mut prev = out[0];
        for k in 1..SAMPLE_RATE as usize {
            let v = out[2 * k];
            if (v > 0.0) != (prev > 0.0) {
                crossings += 1;
            }
            prev = v;
        }
        let hz = crossings as f64 / 2.0;
        assert!((hz - 3_579_545.0 / (32.0 * 256.0)).abs() < 12.0, "{hz}");
    }

    #[test]
    fn a_silent_machine_makes_silence() {
        let mut psg = Psg::new();
        psg.run(MASTER_HZ / 10);
        assert!(psg.drain().iter().all(|s| *s == 0.0));
    }
}
