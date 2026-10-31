//! The four FM channels of the YM2610: four sine operators each, with envelopes, joined by the eight algorithms (fact Y6).
//! A first implementation: the sine and the envelope are computed (not read from the chip's tables), detune, the LFO, the
//! SSG-EG and the third channel's separate frequencies are not done.

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Off,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Copy)]
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
    phase: Phase,
    /// Attenuation in the chip's units of 0.09375 dB: 0 is full volume, 1023 silent.
    att: f32,
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
            phase: Phase::Off,
            att: 1023.0,
            pos: 0.0,
            out: 0.0,
            prev: 0.0,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Channel {
    ops: [Operator; 4],
    fnum: u16,
    block: u8,
    feedback: u8,
    algorithm: u8,
    left: bool,
    right: bool,
}

/// The attenuation change an envelope rate (0-63) makes per sample (fact Y10): zero for rate 0, else on average (4 + rate mod 4)
/// times two to the power (rate / 4 - 14) for every envelope tick, which comes every third sample.
fn rate_step(rate: u32) -> f32 {
    if rate == 0 {
        return 0.0;
    }
    let r = rate.min(63);
    (4 + (r & 3)) as f32 * 2f32.powi((r >> 2) as i32 - 14) / 3.0
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

    fn effective_rate(&self, op: &Operator, r: u8, release: bool) -> u32 {
        if r == 0 && !release {
            return 0;
        }
        let base = if release {
            u32::from(r) * 2 + 1
        } else {
            u32::from(r) * 2
        };
        (base + (self.keycode() >> (3 - u32::from(op.ks)))).min(63)
    }

    fn key(&mut self, slot: usize, on: bool) {
        let op = &mut self.ops[slot];
        if on {
            if op.phase == Phase::Off || op.phase == Phase::Release {
                op.pos = 0.0;
                op.phase = Phase::Attack;
                if op.ar >= 31 {
                    op.att = 0.0;
                    op.phase = Phase::Decay;
                }
            }
        } else if op.phase != Phase::Off {
            op.phase = Phase::Release;
        }
    }

    fn step_envelope(&mut self) {
        for i in 0..4 {
            let op = self.ops[i];
            let sl_att = if op.sl >= 15 {
                992.0
            } else {
                f32::from(op.sl) * 32.0
            };
            let mut att = op.att;
            let mut phase = op.phase;
            match phase {
                Phase::Off => {}
                Phase::Attack => {
                    let r = self.effective_rate(&op, op.ar, false);
                    // the attack closes the gap to full volume faster the more is left
                    att -= att * rate_step(r) * 3.0 / 16.0 + 0.01;
                    if att <= 0.5 {
                        att = 0.0;
                        phase = Phase::Decay;
                    }
                }
                Phase::Decay => {
                    att += rate_step(self.effective_rate(&op, op.d1r, false));
                    if att >= sl_att {
                        att = att.max(sl_att);
                        phase = Phase::Sustain;
                    }
                }
                Phase::Sustain => {
                    att += rate_step(self.effective_rate(&op, op.d2r, false));
                }
                Phase::Release => {
                    att += rate_step(self.effective_rate(&op, op.rr, true));
                }
            }
            if att >= 1023.0 {
                att = 1023.0;
                if phase == Phase::Release || phase == Phase::Sustain {
                    phase = Phase::Off;
                }
            }
            self.ops[i].att = att;
            self.ops[i].phase = phase;
        }
    }

    /// One operator's next output (-1..1) given the phase modulation in cycles.
    fn operator(&mut self, i: usize, modulation: f32) -> f32 {
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
        let inc = (base + dt) / 1_048_576.0 * mul;
        let op = &mut self.ops[i];
        op.pos = (op.pos + inc).fract();
        if op.phase == Phase::Off {
            op.prev = op.out;
            op.out = 0.0;
            return 0.0;
        }
        let total = op.att + f32::from(op.tl) * 8.0;
        let amp = 10f32.powf(-total.min(1023.0) * 0.09375 / 20.0);
        let v = ((op.pos + modulation) * TAU).sin() * amp;
        op.prev = op.out;
        op.out = v;
        v
    }

    /// One sample of the channel as (left, right), full scale about 1.
    fn sample(&mut self) -> (f32, f32) {
        self.step_envelope();
        let fb = if self.feedback == 0 {
            0.0
        } else {
            let o = &self.ops[0];
            (o.out + o.prev) * 8.0 / 2f32.powi(10 - i32::from(self.feedback))
        };
        let m = 4.0; // an output of 1 moves the phase of the next operator by four cycles
        let o1 = self.operator(0, fb);
        let out = match self.algorithm & 7 {
            0 => {
                let o2 = self.operator(1, o1 * m);
                let o3 = self.operator(2, o2 * m);
                self.operator(3, o3 * m)
            }
            1 => {
                let o2 = self.operator(1, 0.0);
                let o3 = self.operator(2, (o1 + o2) * m);
                self.operator(3, o3 * m)
            }
            2 => {
                let o2 = self.operator(1, 0.0);
                let o3 = self.operator(2, o2 * m);
                self.operator(3, (o1 + o3) * m)
            }
            3 => {
                let o2 = self.operator(1, o1 * m);
                let o3 = self.operator(2, 0.0);
                self.operator(3, (o2 + o3) * m)
            }
            4 => {
                let o2 = self.operator(1, o1 * m);
                let o3 = self.operator(2, 0.0);
                let o4 = self.operator(3, o3 * m);
                o2 + o4
            }
            5 => {
                let o2 = self.operator(1, o1 * m);
                let o3 = self.operator(2, o1 * m);
                let o4 = self.operator(3, o1 * m);
                o2 + o3 + o4
            }
            6 => {
                let o2 = self.operator(1, o1 * m);
                let o3 = self.operator(2, 0.0);
                let o4 = self.operator(3, 0.0);
                o2 + o3 + o4
            }
            _ => {
                let o2 = self.operator(1, 0.0);
                let o3 = self.operator(2, 0.0);
                let o4 = self.operator(3, 0.0);
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

/// The four channels and the writes that reach them.
#[derive(Default)]
pub struct Fm {
    pub channels: [Channel; 4],
    /// The frequency high bits wait for the low byte (fact Y6).
    latch: [u8; 2],
}

impl Fm {
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
            0x60..=0x6F => c.ops[slot].d1r = value & 0x1F,
            0x70..=0x7F => c.ops[slot].d2r = value & 0x1F,
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
            }
            _ => {}
        }
    }

    /// One sample of all four channels as (left, right).
    pub fn sample(&mut self) -> (f32, f32) {
        let (mut l, mut r) = (0.0, 0.0);
        for c in &mut self.channels {
            let (cl, cr) = c.sample();
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

    #[test]
    fn an_envelope_rate_moves_the_attenuation_by_the_published_average_per_tick() {
        // one envelope tick is three samples; below rate 48 an update of 4 + (rate mod 4) eighths comes every 2^(11 - rate / 4) ticks
        let per_tick = |r: u32| rate_step(r) * 3.0;
        assert_eq!(rate_step(0), 0.0);
        assert!((per_tick(47) - 0.875).abs() < 1e-6);
        assert!((per_tick(48) - 1.0).abs() < 1e-6);
        assert!((per_tick(52) - 2.0).abs() < 1e-6);
        assert!((per_tick(60) - 8.0).abs() < 1e-6);
        assert!((per_tick(4) - 4.0 / 8192.0).abs() < 1e-9);
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
