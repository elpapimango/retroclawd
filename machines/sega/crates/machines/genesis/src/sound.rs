//! The sound of the machine: the YM2612 and the PSG run against the master clock and are mixed into one stereo stream with a
//! sample for every 24 chip cycles of the YM (1008 master clocks, 53 267 Hz on NTSC, fact C4).
//!
//! The YM2612 is clocked once per 42 master clocks (one chip cycle) and the PSG once per 240 (one step of 16 input clocks).
//! Writes are made at the time they happen: the chips are run up to that time first.

use serde::{Deserialize, Serialize};
use sn76489::{Psg, STEP_MASTER_CLOCKS};
use ym2612::{CYCLE_MASTER_CLOCKS, CYCLES_PER_SAMPLE, Chip, Ym2612};

/// Master clocks of one output sample.
pub const SAMPLE_MASTER_CLOCKS: u64 = CYCLE_MASTER_CLOCKS * CYCLES_PER_SAMPLE as u64;

/// The sum of the 24 cycle outputs of a sample, divided by this, is 1 when the FM is as loud as Genesis Plus GX makes a
/// full-scale FM channel mix (probe P07: its level is 1.5 times the die-accurate sum scaled to +-1; fact S10).
const YM_FULL_SCALE: f32 = 6.0 * 256.0 * 3.0 / 1.5;
/// How loud the PSG is next to the FM: its four channels at full volume together reach this level (fact S10: set from the
/// level the reference emulators give it, probe P-MIX).
const PSG_FULL_SCALE: f32 = 0.51;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sound {
    ym: Ym2612,
    psg: Psg,
    /// Master clock the YM has run to (a multiple of its cycle).
    ym_time: u64,
    /// Chip cycles of the current sample done so far and the sums of their outputs.
    ym_cycle: usize,
    sum_l: i32,
    sum_r: i32,
    /// The PSG: the master clock it has run to, the time of its next step, the level it outputs now and the integral of the
    /// level over the current sample.
    psg_time: u64,
    psg_next: u64,
    psg_level: f32,
    psg_integral: f64,
    /// The AC coupling of the output: a one-pole high-pass filter per channel (its last input and output).
    dc_in: [f32; 2],
    dc_out: [f32; 2],
    /// Finished samples, interleaved left and right.
    out: Vec<f32>,
}

/// The high-pass pole: a corner of about 5 Hz at the output rate (fact S11: the analogue output of the console is coupled
/// through capacitors; the exact corner depends on the model).
const DC_POLE: f32 = 0.9994;
/// The longest stretch of master clocks the chips are run through when behind (about 8 frames of sound).
const MAX_LAG: u64 = 8 * 1_070_460;

impl Sound {
    pub fn new(chip: Chip) -> Self {
        Self {
            ym: Ym2612::new(chip),
            psg: Psg::new(),
            ym_time: 0,
            ym_cycle: 0,
            sum_l: 0,
            sum_r: 0,
            psg_time: 0,
            psg_next: STEP_MASTER_CLOCKS,
            psg_level: 0.0,
            psg_integral: 0.0,
            dc_in: [0.0; 2],
            dc_out: [0.0; 2],
            out: Vec::new(),
        }
    }

    /// Whether a value read back from a saved state is usable.
    pub fn validate(&self) -> Result<(), String> {
        self.ym.validate()?;
        self.psg.validate()?;
        if self.ym_cycle >= CYCLES_PER_SAMPLE || self.out.len() > 1 << 20 {
            return Err("the sound state is out of range".to_string());
        }
        if self.psg_next <= self.psg_time || self.psg_next - self.psg_time > STEP_MASTER_CLOCKS {
            return Err("the PSG clock is out of range".to_string());
        }
        Ok(())
    }

    pub fn ym(&self) -> &Ym2612 {
        &self.ym
    }

    pub fn psg(&self) -> &Psg {
        &self.psg
    }

    /// Runs the PSG up to master clock `t`, adding to the integral of its output.
    fn psg_run_to(&mut self, t: u64) {
        while self.psg_next <= t {
            self.psg_integral += f64::from(self.psg_level) * (self.psg_next - self.psg_time) as f64;
            self.psg_time = self.psg_next;
            self.psg.tick();
            self.psg_level = self.psg.output();
            self.psg_next += STEP_MASTER_CLOCKS;
        }
        if t > self.psg_time {
            self.psg_integral += f64::from(self.psg_level) * (t - self.psg_time) as f64;
            self.psg_time = t;
        }
    }

    /// Runs both chips up to master clock `t`; finished samples go to the output queue.
    pub fn run_to(&mut self, t: u64) {
        // A chip far behind the clock (a machine that is never drained, or a damaged state) skips the time instead of
        // making every sample of it: sound that nobody collected is not worth computing.
        if t > self.ym_time + MAX_LAG {
            let skip = (t - MAX_LAG - self.ym_time) / SAMPLE_MASTER_CLOCKS * SAMPLE_MASTER_CLOCKS;
            self.ym_time += skip;
            self.psg_time = self.psg_time.max(self.ym_time);
            self.psg_next = self
                .psg_next
                .max(self.psg_time + 1)
                .min(self.psg_time + STEP_MASTER_CLOCKS);
            self.psg_integral = 0.0;
            self.ym_cycle = 0;
            self.sum_l = 0;
            self.sum_r = 0;
            self.out.clear();
        }
        while self.ym_time + CYCLE_MASTER_CLOCKS <= t {
            let (l, r) = self.ym.clock();
            self.sum_l += i32::from(l);
            self.sum_r += i32::from(r);
            self.ym_time += CYCLE_MASTER_CLOCKS;
            self.ym_cycle += 1;
            if self.ym_cycle == CYCLES_PER_SAMPLE {
                self.psg_run_to(self.ym_time);
                let psg = (self.psg_integral / SAMPLE_MASTER_CLOCKS as f64) as f32
                    * (PSG_FULL_SCALE / 4.0);
                self.psg_integral = 0.0;
                for (c, sum) in [self.sum_l, self.sum_r].into_iter().enumerate() {
                    let x = sum as f32 / YM_FULL_SCALE + psg;
                    let y = x - self.dc_in[c] + DC_POLE * self.dc_out[c];
                    self.dc_in[c] = x;
                    self.dc_out[c] = y;
                    self.out.push(y);
                }
                self.sum_l = 0;
                self.sum_r = 0;
                self.ym_cycle = 0;
            }
        }
    }

    /// A write to one of the YM2612's four ports at master clock `t`.
    pub fn ym_write(&mut self, t: u64, port: u32, value: u8) {
        self.run_to(t);
        self.ym.write(port, value);
    }

    pub fn ym_read(&mut self, t: u64, port: u32) -> u8 {
        self.run_to(t);
        self.ym.read(port)
    }

    /// A byte written to the PSG at master clock `t`.
    pub fn psg_write(&mut self, t: u64, value: u8) {
        self.run_to(t);
        self.psg_run_to(t);
        self.psg.write(value);
        self.psg_level = self.psg.output();
    }

    /// The samples finished so far (interleaved left and right), emptying the queue.
    pub fn drain(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ym_write_gap(s: &mut Sound, t: &mut u64, bank: u32, reg: u8, value: u8) {
        s.ym_write(*t, bank * 2, reg);
        *t += 12 * 42;
        s.ym_write(*t, bank * 2 + 1, value);
        *t += 24 * 42;
    }

    #[test]
    fn a_sample_comes_out_every_1008_master_clocks() {
        let mut s = Sound::new(Chip::Ym3438);
        s.run_to(896_040);
        let samples = s.drain();
        assert_eq!(
            samples.len(),
            2 * 888,
            "888 samples of two channels in an NTSC frame"
        );
        assert!(s.drain().is_empty());
    }

    #[test]
    fn the_psg_tone_comes_out_at_the_right_pitch() {
        let mut s = Sound::new(Chip::Ym3438);
        // channel 0: period 100, full volume: a square wave of clock / (32 * 100) = 1118.6 Hz at the NTSC Z80 clock
        s.psg_write(0, 0x80 | 100 & 0x0F);
        s.psg_write(0, (100 >> 4) as u8);
        s.psg_write(0, 0x90);
        s.psg_write(0, 0xBF);
        s.psg_write(0, 0xDF);
        s.psg_write(0, 0xFF);
        s.run_to(53_693_175 / 10);
        let samples = s.drain();
        let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
        let mean = left.iter().sum::<f32>() / left.len() as f32;
        let mut crossings = 0;
        for w in left.windows(2) {
            if (w[0] - mean) * (w[1] - mean) < 0.0 {
                crossings += 1;
            }
        }
        let seconds = 0.1;
        let hz = crossings as f64 / 2.0 / seconds;
        assert!((hz - 1118.6).abs() < 25.0, "{hz} Hz");
    }

    #[test]
    fn the_ym_makes_sound_when_a_note_is_keyed_on() {
        let mut s = Sound::new(Chip::Ym2612);
        let mut t = 0;
        for (reg, v) in [
            (0x30, 1),
            (0x40, 0),
            (0x50, 0x1F),
            (0x60, 0),
            (0x70, 0),
            (0x80, 0x0F),
        ] {
            ym_write_gap(&mut s, &mut t, 0, reg, v);
        }
        ym_write_gap(&mut s, &mut t, 0, 0xB0, 7);
        ym_write_gap(&mut s, &mut t, 0, 0xB4, 0xC0);
        ym_write_gap(&mut s, &mut t, 0, 0xA4, 0x22);
        ym_write_gap(&mut s, &mut t, 0, 0xA0, 0x69);
        ym_write_gap(&mut s, &mut t, 0, 0x28, 0xF0);
        s.run_to(t + 896_040 / 4);
        let samples = s.drain();
        let peak = samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.05, "{peak}");
    }

    #[test]
    fn the_ym_status_register_is_read_at_its_time() {
        let mut s = Sound::new(Chip::Ym2612);
        let mut t = 0;
        ym_write_gap(&mut s, &mut t, 0, 0x24, 0xFF);
        ym_write_gap(&mut s, &mut t, 0, 0x25, 0x03);
        ym_write_gap(&mut s, &mut t, 0, 0x27, 0x05);
        assert_eq!(s.ym_read(t + 10 * SAMPLE_MASTER_CLOCKS, 0) & 1, 1);
    }
}
