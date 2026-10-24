//! The STE's DMA sound output filters ([D] *STe manual* section 2.10.1): the DAC output goes through a four-pole switched low-pass
//! filter at 40 % of the sample frequency ("anti-aliasing of the sound data in a sample-frequency-sensitive way") and then a
//! two-pole fixed 16 kHz low-pass filter, before it reaches the LMC1992.
//!
//! [H?] The manual gives the pole counts and the corner frequencies and not the filter family or the damping: this model uses
//! Butterworth responses (two biquads of Q 0.5412 and 1.3066 for the four poles, one of Q 0.7071 for the two), the flattest
//! choice, applied to the 44 100 Hz output stream. At the 50 kHz rate the corner is 20 kHz, close to that stream's Nyquist
//! frequency, so only the 16 kHz section does much there.

/// The four sample rates, `$FF8921` bits 1-0: 6258, 12517, 25033 and 50066 Hz.
const RATES_HZ: [f64; 4] = [6258.0, 12517.0, 25033.0, 50066.0];
/// The fixed filter's corner, Hz.
const FIXED_HZ: f64 = 16_000.0;
/// The switched filter's corner as a fraction of the sample rate.
const CORNER_FRACTION: f64 = 0.4;
/// Q of the two sections of a fourth-order Butterworth filter, and of a second-order one.
const Q4: [f64; 2] = [0.541_196_1, 1.306_563];
const Q2: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// The switched filter's corner for a rate code (0 to 3), Hz.
pub fn corner_hz(rate: u8) -> f64 {
    CORNER_FRACTION * RATES_HZ[usize::from(rate & 3)]
}

/// One second-order section, direct form I.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Biquad {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

/// The low-pass section at `corner` Hz with quality `q`, sampled at `rate` Hz (RBJ cookbook).
fn lowpass(corner: f64, q: f64, rate: f64) -> Coefficients {
    let w = 2.0 * std::f64::consts::PI * corner / rate;
    let (sin, cos) = (w.sin(), w.cos());
    let alpha = sin / (2.0 * q);
    let a0 = 1.0 + alpha;
    Coefficients {
        b0: ((1.0 - cos) / 2.0 / a0) as f32,
        b1: ((1.0 - cos) / a0) as f32,
        b2: ((1.0 - cos) / 2.0 / a0) as f32,
        a1: (-2.0 * cos / a0) as f32,
        a2: ((1.0 - alpha) / a0) as f32,
    }
}

impl Biquad {
    fn run(&mut self, c: Coefficients, x: f32) -> f32 {
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// The magnitude of a section at `hz`, dB, from its analog prototype `1 / (s^2 + s/q + 1)` scaled to `corner`.
fn section_db(corner: f64, q: f64, hz: f64) -> f64 {
    let x = hz / corner;
    let re = 1.0 - x * x;
    let im = x / q;
    -10.0 * (re * re + im * im).log10()
}

/// The response of both filters together at `hz` for a rate code, dB (the analog filters the manual describes).
pub fn response_db(rate: u8, hz: f64) -> f64 {
    let switched = corner_hz(rate);
    Q4.iter().map(|&q| section_db(switched, q, hz)).sum::<f64>() + section_db(FIXED_HZ, Q2, hz)
}

/// The filters for the left and right channels: `[left, right]`, each the two sections of the switched filter and the fixed one.
/// Not part of a save state: it is the last few samples only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dac {
    sections: [[Biquad; 3]; 2],
}

impl Dac {
    /// Filters one channel's `samples` in place (`channel` 0 or 1) for the sample-rate code `rate`.
    pub fn apply(&mut self, rate: u8, channel: usize, samples: &mut [f32]) {
        let out_rate = f64::from(machine::AUDIO_SAMPLE_RATE);
        let switched = corner_hz(rate);
        let coefficients = [
            lowpass(switched, Q4[0], out_rate),
            lowpass(switched, Q4[1], out_rate),
            lowpass(FIXED_HZ, Q2, out_rate),
        ];
        let state = &mut self.sections[channel & 1];
        for sample in samples {
            let mut x = *sample;
            for (section, c) in state.iter_mut().zip(coefficients) {
                x = section.run(c, x);
            }
            *sample = x;
        }
    }
}
