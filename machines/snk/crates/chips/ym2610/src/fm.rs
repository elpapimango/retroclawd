//! The four FM channels of the YM2610: four sine operators each, with envelopes, joined by the eight algorithms (fact Y6).
//! The sine is computed (not read from the chip's tables). The envelope generator is the OPN family's integer one (fact Y13:
//! a 10-bit attenuation stepped by a 12-bit counter every three samples, with SSG-EG), as the family's YM2612 crate models it.
//! The third channel's separate frequencies are not done.

use std::f32::consts::TAU;

/// The detune of an operator in 20-bit phase increments for detune 1, 2 and 3 at key codes 0 to 31 (fact Y9).
const DETUNE: [[u8; 32]; 3] = [
    [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 8,
        8, 8,
    ],
    [
        1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 8, 9, 10, 11, 12, 13, 14,
        16, 16, 16, 16,
    ],
    [
        2, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19,
        20, 22, 22, 22, 22,
    ],
];

/// The LFO's frequencies for register `$22` bits 2-0, in Hz (fact Y12).
const LFO_HZ: [f32; 8] = [3.98, 5.56, 6.02, 6.37, 6.88, 9.63, 48.1, 72.2];
/// The amplitude modulation depth of AMS 0-3 (register `$B4` bits 5-4), in dB (fact Y12).
const AMS_DB: [f32; 4] = [0.0, 1.4, 5.9, 11.8];
/// The pitch modulation depth of PMS 0-7 (register `$B4` bits 2-0), in cents either way (fact Y12).
const PMS_CENTS: [f32; 8] = [0.0, 3.4, 6.7, 10.0, 14.0, 20.0, 40.0, 80.0];

/// The envelope's state (fact Y13).
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
enum State {
    Attack,
    Decay,
    Sustain,
    Release,
}

/// The step of the fastest rates (48 and up) by the low two bits of the rate and the low two bits of the envelope counter.
const EG_STEP_HIGH: [[u8; 4]; 4] = [[0, 0, 0, 0], [1, 0, 0, 0], [1, 0, 1, 0], [1, 1, 1, 0]];

/// What the envelope counter gives the operators on a sample: on one sample of three the counter has moved, and then the
/// number of its lowest set bit plus one (0 when it is 0) and its low two bits (fact Y13).
#[derive(Clone, Copy, Default)]
struct Tick {
    on: bool,
    shift: u8,
    low: u8,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Operator {
    mul: u8,
    /// The detune: bits 6-4 of register `$30`, a magnitude in bits 5-4 and the sign in bit 6 (fact Y9).
    dt: u8,
    tl: u8,
    ks: u8,
    ar: u8,
    d1r: u8,
    d2r: u8,
    sl: u8,
    rr: u8,
    /// SSG-EG: register `$90` bits 3-0 (bit 3 on, bit 2 attack, bit 1 alternate, bit 0 hold).
    ssg: u8,
    /// Amplitude modulation by the LFO (register `$60` bit 7).
    am: bool,
    /// The key as written (`$28`) and as the envelope saw it on the last sample.
    kon_latch: bool,
    kon: bool,
    state: State,
    /// Attenuation in the chip's units of 0.09375 dB: 0 is full volume, 1023 silent.
    level: u16,
    ssg_dir: bool,
    ssg_inv: bool,
    ssg_enable: bool,
    ssg_pgrst: bool,
    ssg_repeat: bool,
    ssg_hold: bool,
    /// The phase starts again on this sample (a key on, or an SSG-EG restart).
    reset: bool,
    /// The attenuation of this sample, from the envelope (worked out before the operators each sample).
    #[serde(skip)]
    att: u16,
    /// The position in the sine, in cycles.
    pos: f32,
    out: f32,
    prev: f32,
}

impl Default for Operator {
    fn default() -> Self {
        Operator {
            mul: 0,
            dt: 0,
            tl: 0,
            ks: 0,
            ar: 0,
            d1r: 0,
            d2r: 0,
            sl: 0,
            rr: 0,
            ssg: 0,
            am: false,
            kon_latch: false,
            kon: false,
            state: State::Release,
            level: 0x3FF,
            ssg_dir: false,
            ssg_inv: false,
            ssg_enable: false,
            ssg_pgrst: false,
            ssg_repeat: false,
            ssg_hold: false,
            reset: false,
            att: 0x3FF,
            pos: 0.0,
            out: 0.0,
            prev: 0.0,
        }
    }
}

#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct Channel {
    ops: [Operator; 4],
    fnum: u16,
    block: u8,
    feedback: u8,
    algorithm: u8,
    left: bool,
    right: bool,
    /// The LFO sensitivities: AMS 0-3 and PMS 0-7 (register `$B4`).
    ams: u8,
    pms: u8,
}

impl Channel {
    /// The key code: the block and two bits made of the top bits of the frequency number (fact Y9).
    fn keycode(&self) -> u32 {
        let f = u32::from(self.fnum);
        let (f11, f10, f9, f8) = ((f >> 10) & 1, (f >> 9) & 1, (f >> 8) & 1, (f >> 7) & 1);
        let n3 = if f11 == 1 {
            f10 | f9 | f8
        } else {
            f10 & f9 & f8
        };
        u32::from(self.block) << 2 | f11 << 1 | n3
    }

    fn key(&mut self, slot: usize, on: bool) {
        self.ops[slot].kon_latch = on;
    }

    /// The SSG-EG stage of operator `i`: direction, inversion and the restart, hold and phase reset latches (fact Y13).
    fn ssg_eg(&mut self, i: usize) {
        let op = &mut self.ops[i];
        let ssg = op.ssg;
        let mut direction = false;
        op.ssg_pgrst = false;
        op.ssg_repeat = false;
        op.ssg_hold = false;
        if ssg & 8 != 0 {
            direction = op.ssg_dir;
            if op.level & 0x200 != 0 {
                if ssg & 3 == 0 {
                    op.ssg_pgrst = true;
                }
                if ssg & 1 == 0 {
                    op.ssg_repeat = true;
                }
                if ssg & 3 == 2 {
                    direction = !direction;
                }
                if ssg & 3 == 3 {
                    direction = true;
                }
            }
            if op.kon_latch && (ssg & 7 == 5 || ssg & 7 == 3) {
                op.ssg_hold = true;
            }
            direction &= op.kon;
        }
        op.ssg_dir = direction;
        op.ssg_enable = ssg & 8 != 0;
        op.ssg_inv = (direction ^ (ssg & 0x0C == 0x0C)) && op.kon;
    }

    /// The step of operator `i`'s envelope on this sample: the rate from its state, the key scale and the counter (fact Y13).
    /// Returns the step (0-4) and whether the rate is the fastest (62 or 63), which reaches full volume at once.
    fn eg_step(&self, i: usize, tick: Tick) -> (u8, bool) {
        let op = &self.ops[i];
        let attack = (op.kon && op.ssg_repeat) || (!op.kon && op.kon_latch);
        let r = match if attack { State::Attack } else { op.state } {
            State::Attack => op.ar,
            State::Decay => op.d1r,
            State::Sustain => op.d2r,
            State::Release => op.rr << 1 | 1,
        };
        let ksv = (self.keycode() >> (op.ks ^ 3)) as u8;
        let rate = ((r << 1) + ksv).min(63);
        let mut inc = 0;
        if r != 0 && tick.on {
            if rate < 48 {
                inc = match ((rate >> 2) + tick.shift) & 15 {
                    12 => 1,
                    13 => (rate >> 1) & 1,
                    14 => rate & 1,
                    _ => 0,
                };
            } else {
                let step = EG_STEP_HIGH[usize::from(rate & 3)][usize::from(tick.low)];
                inc = (step + (rate >> 2)).saturating_sub(11).min(4);
            }
        }
        (inc, rate >> 1 == 31)
    }

    /// One sample of operator `i`'s envelope (fact Y13). Returns the attenuation the operator plays with on this sample: the
    /// level (inverted by SSG-EG) plus the total level, at most 1023.
    fn envelope(&mut self, i: usize, tick: Tick) -> u16 {
        self.ssg_eg(i);
        let (eg_inc, ratemax) = self.eg_step(i, tick);
        let op = &mut self.ops[i];
        let shown = if op.ssg_inv {
            512u16.wrapping_sub(op.level) & 0x3FF
        } else {
            op.level
        };
        let out = (shown + (u16::from(op.tl) << 3)).min(0x3FF);

        let (nkon, okon) = (op.kon_latch, op.kon);
        op.reset = (nkon && !okon) || op.ssg_pgrst;
        let kon_event = (nkon && !okon) || (okon && op.ssg_repeat);
        let koff_event = okon && !nkon;
        let mut level = i32::from(op.level);
        if koff_event {
            level = i32::from(shown);
        }
        let eg_off = if op.ssg_enable {
            level >> 9 != 0
        } else {
            level & 0x3F0 == 0x3F0
        };
        let mut next_state = op.state;
        let mut next = level;
        let mut inc = 0i32;
        // sustain level 15 means 31 of 32 steps of 32 units
        let sl = u16::from(op.sl) | (u16::from(op.sl) + 1) & 0x10;
        let decay_step = |ssg: bool| {
            let s = 1i32 << (eg_inc - 1);
            if ssg { s << 2 } else { s }
        };
        if kon_event {
            next_state = State::Attack;
            if ratemax {
                next = 0;
            } else if op.state == State::Attack && level != 0 && eg_inc != 0 && nkon {
                inc = (!level << eg_inc) >> 5;
            }
        } else {
            match op.state {
                State::Attack => {
                    if level == 0 {
                        next_state = State::Decay;
                    } else if eg_inc != 0 && !ratemax && nkon {
                        inc = (!level << eg_inc) >> 5;
                    }
                }
                State::Decay => {
                    if level >> 4 == i32::from(sl) << 1 {
                        next_state = State::Sustain;
                    } else if !eg_off && eg_inc != 0 {
                        inc = decay_step(op.ssg_enable);
                    }
                }
                State::Sustain | State::Release => {
                    if !eg_off && eg_inc != 0 {
                        inc = decay_step(op.ssg_enable);
                    }
                }
            }
            if !nkon {
                next_state = State::Release;
            }
        }
        if !kon_event && !op.ssg_hold && op.state != State::Attack && eg_off {
            next_state = State::Release;
            next = 0x3FF;
        }
        next += inc;
        op.kon = nkon;
        op.level = (next & 0x3FF) as u16;
        op.state = next_state;
        out
    }

    /// One operator's next output (-1..1) given the phase modulation in cycles.
    fn operator(&mut self, i: usize, modulation: f32, lfo: Lfo) -> f32 {
        let mul = if self.ops[i].mul == 0 {
            0.5
        } else {
            f32::from(self.ops[i].mul)
        };
        // cycles per sample: fnum x 2^block / 2^21 x multiple (fact Y6)
        // the detune moves the increment of the 20-bit phase by an amount that grows with the key code (fact Y9)
        let base = ((u32::from(self.fnum) << self.block) >> 1) as f32;
        let mag = usize::from(self.ops[i].dt & 3);
        let step = if mag == 0 {
            0
        } else {
            DETUNE[mag - 1][self.keycode().min(31) as usize]
        };
        let dt = if self.ops[i].dt & 4 != 0 {
            -f32::from(step)
        } else {
            f32::from(step)
        };
        let inc = (base + dt) / 1_048_576.0 * mul * lfo.pitch;
        let op = &mut self.ops[i];
        op.pos = if op.reset {
            0.0
        } else {
            (op.pos + inc).fract()
        };
        // the LFO's attenuation, in the same units of 0.09375 dB, on operators with AM on (fact Y12)
        let am = if op.am { lfo.am_db / 0.09375 } else { 0.0 };
        let total = f32::from(op.att) + am;
        let amp = 10f32.powf(-total.min(1023.0) * 0.09375 / 20.0);
        let v = ((op.pos + modulation) * TAU).sin() * amp;
        op.prev = op.out;
        op.out = v;
        v
    }

    /// One sample of the channel as (left, right), full scale about 1. `tri` is the LFO's triangle (-1 to 1) and `ramp` its
    /// unipolar form (0 to 1), both 0 when the LFO is off.
    fn sample(&mut self, tri: f32, ramp: f32, tick: Tick) -> (f32, f32) {
        for i in 0..4 {
            self.ops[i].att = self.envelope(i, tick);
        }
        let lfo = Lfo {
            pitch: 2f32.powf(PMS_CENTS[usize::from(self.pms & 7)] * tri / 1200.0),
            am_db: AMS_DB[usize::from(self.ams & 3)] * ramp,
        };
        let fb = if self.feedback == 0 {
            0.0
        } else {
            let o = &self.ops[0];
            (o.out + o.prev) * 8.0 / 2f32.powi(10 - i32::from(self.feedback))
        };
        let m = 4.0; // an output of 1 moves the phase of the next operator by four cycles
        let o1 = self.operator(0, fb, lfo);
        let out = match self.algorithm & 7 {
            0 => {
                let o2 = self.operator(1, o1 * m, lfo);
                let o3 = self.operator(2, o2 * m, lfo);
                self.operator(3, o3 * m, lfo)
            }
            1 => {
                let o2 = self.operator(1, 0.0, lfo);
                let o3 = self.operator(2, (o1 + o2) * m, lfo);
                self.operator(3, o3 * m, lfo)
            }
            2 => {
                let o2 = self.operator(1, 0.0, lfo);
                let o3 = self.operator(2, o2 * m, lfo);
                self.operator(3, (o1 + o3) * m, lfo)
            }
            3 => {
                let o2 = self.operator(1, o1 * m, lfo);
                let o3 = self.operator(2, 0.0, lfo);
                self.operator(3, (o2 + o3) * m, lfo)
            }
            4 => {
                let o2 = self.operator(1, o1 * m, lfo);
                let o3 = self.operator(2, 0.0, lfo);
                let o4 = self.operator(3, o3 * m, lfo);
                o2 + o4
            }
            5 => {
                let o2 = self.operator(1, o1 * m, lfo);
                let o3 = self.operator(2, o1 * m, lfo);
                let o4 = self.operator(3, o1 * m, lfo);
                o2 + o3 + o4
            }
            6 => {
                let o2 = self.operator(1, o1 * m, lfo);
                let o3 = self.operator(2, 0.0, lfo);
                let o4 = self.operator(3, 0.0, lfo);
                o2 + o3 + o4
            }
            _ => {
                let o2 = self.operator(1, 0.0, lfo);
                let o3 = self.operator(2, 0.0, lfo);
                let o4 = self.operator(3, 0.0, lfo);
                o1 + o2 + o3 + o4
            }
        };
        // the sum of the carriers saturates like the chip's 14-bit output (fact Y10)
        let out = out.clamp(-1.0, 1.0);
        (
            if self.left { out } else { 0.0 },
            if self.right { out } else { 0.0 },
        )
    }
}

/// What the LFO does to one sample of a channel: a pitch factor and an attenuation in dB.
#[derive(Clone, Copy)]
struct Lfo {
    pitch: f32,
    am_db: f32,
}

/// The four channels and the writes that reach them.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Fm {
    pub channels: [Channel; 4],
    /// The frequency high bits wait for the low byte (fact Y6).
    latch: [u8; 2],
    /// The LFO (register `$22`): on, its frequency (0-7) and where it is in its period (0 to 1).
    lfo_on: bool,
    lfo_rate: u8,
    lfo_pos: f32,
    /// The envelope counter (12 bits) and which of the three samples it moves on (fact Y13).
    eg_timer: u16,
    eg_quotient: u8,
}

impl Fm {
    /// For a state read from a file: the block (a shift) and the operator fields are in their register widths.
    pub fn check_state(&self) -> Result<(), String> {
        let ok = self.channels.iter().all(|c| {
            c.block <= 7
                && c.fnum <= 0x7FF
                && c.feedback <= 7
                && c.algorithm <= 7
                && c.ams <= 3
                && c.pms <= 7
                && c.ops.iter().all(|o| {
                    o.mul <= 15
                        && o.dt <= 7
                        && o.tl <= 0x7F
                        && o.ks <= 3
                        && o.ar <= 0x1F
                        && o.d1r <= 0x1F
                        && o.d2r <= 0x1F
                        && o.rr <= 15
                        && o.sl <= 15
                        && o.ssg <= 15
                        && o.level <= 0x3FF
                })
        });
        let lfo_ok = self.lfo_rate <= 7
            && (0.0..1.0).contains(&self.lfo_pos)
            && self.eg_timer <= 0xFFF
            && self.eg_quotient <= 2;
        if ok && lfo_ok {
            Ok(())
        } else {
            Err("an FM channel is out of range".to_string())
        }
    }

    pub fn new() -> Fm {
        let mut fm = Fm::default();
        for c in &mut fm.channels {
            c.left = true;
            c.right = true;
        }
        fm
    }

    /// The channel (0-3) a register of pair `pair` (0 or 1) names with its low two bits, `None` for the unused code 3.
    fn channel_of(pair: usize, reg: u8) -> Option<usize> {
        match reg & 3 {
            1 => Some(pair * 2),
            2 => Some(pair * 2 + 1),
            _ => None, // code 0 is channel 1 of the OPN family, which the YM2610 does not have
        }
    }

    /// A write of register `reg` of pair `pair`, for the channel registers `$30-$B7` and key on `$28` (pair A only).
    pub fn write(&mut self, pair: usize, reg: u8, value: u8) {
        if pair == 0 && reg == 0x22 {
            // the LFO: bit 3 on, bits 2-0 the frequency; switched off it stops at the start of its period (fact Y12)
            self.lfo_on = value & 8 != 0;
            self.lfo_rate = value & 7;
            if !self.lfo_on {
                self.lfo_pos = 0.0;
            }
            return;
        }
        if pair == 0 && reg == 0x28 {
            // key on: bits 7-4 the operators, bits 2-0 the channel (1, 2 then 5, 6)
            let ch = match value & 7 {
                1 => 0,
                2 => 1,
                5 => 2,
                6 => 3,
                _ => return,
            };
            // the operator bits in the order S1, S2, S3, S4 (fact Y6)
            for (n, bit) in [0x10u8, 0x20, 0x40, 0x80].iter().enumerate() {
                self.channels[ch].key(n, value & bit != 0);
            }
            return;
        }
        let Some(ch) = Self::channel_of(pair, reg) else {
            return;
        };
        let slot = [0usize, 2, 1, 3][usize::from((reg >> 2) & 3)];
        let c = &mut self.channels[ch];
        match reg {
            0x30..=0x3F => {
                c.ops[slot].mul = value & 0xF;
                c.ops[slot].dt = (value >> 4) & 7;
            }
            0x40..=0x4F => c.ops[slot].tl = value & 0x7F,
            0x50..=0x5F => {
                c.ops[slot].ks = value >> 6;
                c.ops[slot].ar = value & 0x1F;
            }
            0x60..=0x6F => {
                c.ops[slot].d1r = value & 0x1F;
                c.ops[slot].am = value & 0x80 != 0;
            }
            0x70..=0x7F => c.ops[slot].d2r = value & 0x1F,
            0x90..=0x9F => c.ops[slot].ssg = value & 0x0F,
            0x80..=0x8F => {
                c.ops[slot].rr = value & 0xF;
                c.ops[slot].sl = value >> 4;
            }
            0xA0..=0xA3 => {
                c.fnum = u16::from(self.latch[pair] & 7) << 8 | u16::from(value);
                c.block = (self.latch[pair] >> 3) & 7;
            }
            0xA4..=0xA7 => self.latch[pair] = value,
            0xB0..=0xB3 => {
                c.algorithm = value & 7;
                c.feedback = (value >> 3) & 7;
            }
            0xB4..=0xB7 => {
                c.left = value & 0x80 != 0;
                c.right = value & 0x40 != 0;
                c.ams = (value >> 4) & 3;
                c.pms = value & 7;
            }
            _ => {}
        }
    }

    /// One sample of all four channels as (left, right).
    pub fn sample(&mut self) -> (f32, f32) {
        // a triangle for the pitch and its unipolar form for the amplitude (the shapes are [H?], fact Y12)
        let (tri, ramp) = if self.lfo_on {
            self.lfo_pos = (self.lfo_pos
                + LFO_HZ[usize::from(self.lfo_rate & 7)] / crate::SAMPLE_RATE as f32)
                .fract();
            let p = self.lfo_pos;
            let tri = if p < 0.25 {
                4.0 * p
            } else if p < 0.75 {
                2.0 - 4.0 * p
            } else {
                4.0 * p - 4.0
            };
            let ramp = if p < 0.5 { 2.0 * p } else { 2.0 - 2.0 * p };
            (tri, ramp)
        } else {
            (0.0, 0.0)
        };
        // the envelope counter moves on one sample of three (fact Y13)
        self.eg_quotient = (self.eg_quotient + 1) % 3;
        let mut tick = Tick::default();
        if self.eg_quotient == 2 {
            let t = self.eg_timer;
            tick = Tick {
                on: true,
                shift: if t == 0 {
                    0
                } else {
                    t.trailing_zeros() as u8 + 1
                },
                low: (t & 3) as u8,
            };
            self.eg_timer = (t + 1) & 0xFFF;
        }
        let (mut l, mut r) = (0.0, 0.0);
        for c in &mut self.channels {
            let (cl, cr) = c.sample(tri, ramp, tick);
            l += cl;
            r += cr;
        }
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(fm: &mut Fm, samples: usize) -> Vec<f32> {
        (0..samples).map(|_| fm.sample().0).collect()
    }

    /// A single carrier (algorithm 7 with three operators at full attenuation) on channel 1 of pair A.
    fn tone(fnum: u16, block: u8) -> Fm {
        let mut fm = Fm::new();
        fm.write(0, 0xB1, 7);
        fm.write(0, 0xB5, 0xC0);
        for off in [0x01u8, 0x05, 0x09, 0x0D] {
            fm.write(0, 0x30 + off, 1);
            fm.write(0, 0x50 + off, 31);
            fm.write(0, 0x80 + off, 0x0F);
            fm.write(0, 0x40 + off, 127);
        }
        fm.write(0, 0x41, 0); // operator 1 audible
        fm.write(0, 0xA5, (block << 3) | (fnum >> 8) as u8);
        fm.write(0, 0xA1, fnum as u8);
        fm.write(0, 0x28, 0x11);
        fm
    }

    fn crossings(v: &[f32]) -> usize {
        v.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    /// PMS 7 (80 cents) with the slowest LFO (3.98 Hz): the first half period is higher, a whole period averages out (fact Y12).
    #[test]
    fn the_lfo_moves_the_pitch_by_the_channel_s_pms() {
        let half = (crate::SAMPLE_RATE as f32 / 3.98 / 2.0) as usize;
        let mut plain = tone(0x400, 7);
        let base = play(&mut plain, 2 * half);
        let mut vib = tone(0x400, 7);
        vib.write(0, 0xB5, 0xC7);
        vib.write(0, 0x22, 0x08);
        let moved = play(&mut vib, 2 * half);
        let (b, m) = (crossings(&base[..half]), crossings(&moved[..half]));
        // on average +40 cents (2.3 %) over the first half
        assert!(
            m as f32 > b as f32 * 1.015 && (m as f32) < b as f32 * 1.035,
            "{b} {m}"
        );
        let (b, m) = (crossings(&base), crossings(&moved));
        assert!(m.abs_diff(b) <= 2, "a whole period: {b} {m}");
        // off again: the LFO stops at the start of its period and the pitch is the plain one
        vib.write(0, 0x22, 0x00);
        assert_eq!(vib.lfo_pos, 0.0);
    }

    /// AMS 3 (11.8 dB) on an operator with AM on: at the top of the LFO's ramp the level is about a quarter (fact Y12).
    #[test]
    fn the_lfo_lowers_the_level_of_operators_with_am_on() {
        let mut fm = tone(0x400, 7);
        fm.write(0, 0x61, 0x80); // operator 1: AM on
        fm.write(0, 0xB5, 0xF0); // AMS 3
        fm.write(0, 0x22, 0x08); // 3.98 Hz
        let half = (crate::SAMPLE_RATE as f32 / 3.98 / 2.0) as usize;
        let v = play(&mut fm, 2 * half);
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let start = peak(&v[200..600]);
        let middle = peak(&v[half - 200..half + 200]);
        let ratio = middle / start;
        assert!((0.2..0.32).contains(&ratio), "{start} {middle} {ratio}");
        // an operator without AM is not touched
        let mut plain = tone(0x400, 7);
        plain.write(0, 0xB5, 0xF0);
        plain.write(0, 0x22, 0x08);
        let w = play(&mut plain, 2 * half);
        let r = peak(&w[half - 200..half + 200]) / peak(&w[200..600]);
        assert!(r > 0.95, "{r}");
    }

    #[test]
    fn a_key_on_makes_a_sine_of_the_frequency_the_registers_name() {
        // fnum 1024 would be f = 1024 x 2^block x 55555 / 2^21; fnum 0x400 with block 0: 27.1 Hz x 1... use block 4, fnum 0x2A0
        let mut fm = tone(0x2A0, 4);
        let s = play(&mut fm, 5555);
        let crossings = s.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        // 5555 samples are 0.1 s: the expected frequency is 0x2A0 x 16 x 55555 / 2^21 = 274.7 Hz, 27 cycles
        assert!((26..=29).contains(&crossings), "{crossings} cycles");
        assert!(s.iter().any(|v| v.abs() > 0.5), "the tone is loud");
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn the_keycode_comes_from_the_block_and_the_top_bits_of_the_frequency() {
        let mut c = Channel::default();
        c.block = 4;
        c.fnum = 0x2A0; // bit 10 clear, bits 9, 8, 7 = 1, 0, 1: N4 = 0, N3 = 0
        assert_eq!(c.keycode(), 16);
        c.fnum = 0x3A0; // bit 10 clear, bits 9, 8, 7 all set: N3 = 1
        assert_eq!(c.keycode(), 17);
        c.fnum = 0x480; // bit 10 set and bit 7 set: N4 = 1, N3 = 1
        assert_eq!(c.keycode(), 19);
    }

    #[test]
    fn detune_moves_the_pitch_up_or_down_by_a_little() {
        let count = |dt: u8| {
            let mut fm = tone(0x2A0, 4);
            fm.write(0, 0x31, 0x01 | (dt << 4));
            let s = play(&mut fm, 555_550);
            s.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
        };
        let (flat, up, down) = (count(0), count(3), count(7));
        assert!(up > flat, "{up} {flat}");
        assert!(down < flat, "{down} {flat}");
        assert!(up - flat < flat / 40, "a small amount: {up} {flat}");
    }

    /// Operator 1 of channel 1 keyed on at block 0 (key code 0, so no key scaling) with the given attack and first decay rate
    /// and sustain level 15; returns the levels after each of `samples` samples.
    fn levels(ar: u8, d1r: u8, samples: usize) -> Vec<u16> {
        let mut fm = Fm::new();
        fm.write(0, 0xA5, 0);
        fm.write(0, 0xA1, 0x10);
        fm.write(0, 0x51, ar);
        fm.write(0, 0x61, d1r);
        fm.write(0, 0x81, 0xF0);
        fm.write(0, 0x28, 0x11);
        (0..samples)
            .map(|_| {
                fm.sample();
                fm.channels[0].ops[0].level
            })
            .collect()
    }

    /// Rate 48 (a first decay of 24 at key code 0) adds one unit on every envelope tick, which is one sample of three; rate 52
    /// two, rate 60 eight (fact Y13).
    #[test]
    fn a_decay_rate_of_48_and_up_steps_on_every_third_sample() {
        let l = levels(31, 24, 601);
        assert_eq!(l[0], 0, "attack rate 31 reaches full volume at once");
        assert_eq!(l[600] - l[300], 100, "{:?}", &l[..12]);
        let l = levels(31, 26, 601);
        assert_eq!(l[600] - l[300], 200);
        let l = levels(31, 30, 61);
        assert_eq!(l[60] - l[30], 80);
    }

    /// Below rate 48 a step of one comes on a fraction of the ticks: rate 44 on every second (4 + 0 of 8 eighths), rate 4 once in
    /// 2048 ticks (fact Y13).
    #[test]
    fn a_slow_decay_rate_steps_on_a_fraction_of_the_ticks() {
        let l = levels(31, 22, 1201);
        assert_eq!(l[1200] - l[600], 100);
        let l = levels(31, 2, 3 * 4096 * 3 + 1);
        assert_eq!(l[3 * 4096 * 3] - l[3 * 4096], 4);
    }

    /// The attack closes a sixteenth of the remaining distance (more for higher rates) on each step and ends at 0, then the
    /// decay starts; the first steps are the biggest (fact Y13).
    #[test]
    fn the_attack_falls_exponentially_to_full_volume() {
        let l = levels(20, 0, 3000);
        assert!(l[60] < 0x3FF);
        let first = 0x3FF - l[60];
        assert!(first > 0);
        let zero = l.iter().position(|v| *v == 0).expect("reaches full volume");
        assert!(zero > 30, "{zero}");
        let mid = l[zero / 2];
        assert!(
            mid < 0x3FF / 4,
            "the exponential approach is quick at first: {mid}"
        );
        assert_eq!(l[zero + 10], 0, "a decay rate of 0 holds the level");
    }

    /// SSG-EG 8 (repeat a falling saw): the decay runs from 0 to the end, the envelope starts again from the attack; without it
    /// the level stays at the end (fact Y13).
    #[test]
    fn ssg_eg_repeats_the_envelope() {
        let mut fm = Fm::new();
        fm.write(0, 0x51, 31);
        fm.write(0, 0x61, 28); // fast first decay
        fm.write(0, 0x71, 28);
        fm.write(0, 0x81, 0xF0);
        fm.write(0, 0x91, 0x08);
        fm.write(0, 0x28, 0x11);
        let l: Vec<u16> = (0..2000)
            .map(|_| {
                fm.sample();
                fm.channels[0].ops[0].level
            })
            .collect();
        let restarts = l.windows(2).filter(|w| w[1] + 0x100 < w[0]).count();
        assert!(restarts >= 3, "{restarts}");
    }

    #[test]
    fn a_channel_with_four_loud_carriers_is_limited_to_one_carrier_at_full_scale() {
        let mut fm = tone(0x2A0, 4);
        for off in [0x01u8, 0x05, 0x09, 0x0D] {
            fm.write(0, 0x40 + off, 0);
            fm.write(0, 0x30 + off, 1);
        }
        fm.write(0, 0x28, 0xF1);
        let peak = play(&mut fm, 5555).iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.9 && peak <= 1.0, "{peak}");
    }

    #[test]
    fn key_off_releases_into_silence() {
        let mut fm = tone(0x2A0, 4);
        play(&mut fm, 2000);
        fm.write(0, 0x28, 0x01);
        let s = play(&mut fm, 200_000);
        assert!(s[s.len() - 100..].iter().all(|v| v.abs() < 0.001));
    }

    #[test]
    fn a_muted_pan_gives_nothing_on_that_side() {
        let mut fm = tone(0x2A0, 4);
        fm.write(0, 0xB5, 0x40); // right only
        let (l, r): (Vec<f32>, Vec<f32>) = (0..3000).map(|_| fm.sample()).unzip();
        assert!(l.iter().all(|v| *v == 0.0));
        assert!(r.iter().any(|v| v.abs() > 0.1));
    }
}
