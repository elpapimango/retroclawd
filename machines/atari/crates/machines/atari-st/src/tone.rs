//! The LMC1992/3 tone controls: bass and treble shelves, 2 dB a step, +-12 dB at 50 Hz and at 15 kHz ([D] data sheet,
//! `docs/LMC1993.pdf`: Table I notes 4 and 5, the electrical characteristics' bass and treble ranges, and the analog
//! application hints for the standard circuit with 0.0047 uF tone capacitors).
//!
//! Each control is a first-order shelf, `(1 + s/wz) / (1 + s/wp)` (a gain `wz/wp` on the bass so that it is 1 at high
//! frequencies). At full boost the sheet gives, for the standard circuit, the turnover frequencies `1 / (2 pi C (116.4 k))` = 291
//! Hz (bass) and `1 / (2 pi C (13.8 k))` = 2454 Hz (treble) and the inflection frequencies, where the response is within 3 dB of
//! its final value, `1 / (2 pi C (625.6 k))` = 54 Hz and `1 / (2 pi C (2.6 k))` = 13.0 kHz. Taking the turnover as the zero and
//! choosing the pole so that the gain at 50 Hz (bass) or 15 kHz (treble) is the programmed step gives, at 12 dB, poles at 54 Hz
//! and 12.7 kHz: the sheet's own inflection frequencies, so the three figures agree. A cut is the inverse of the boost of the
//! same size. [H?] The capacitor values of the ST's board are not known (the sheet's standard circuit is assumed) and the
//! intermediate steps follow the sheet only at 50 Hz and 15 kHz: between them the shape is this model's.

/// The bass turnover, Hz.
const BASS_ZERO_HZ: f64 = 291.0;
/// The treble turnover, Hz.
const TREBLE_ZERO_HZ: f64 = 2454.0;
/// Where the steps are measured, Hz.
const BASS_REF_HZ: f64 = 50.0;
const TREBLE_REF_HZ: f64 = 15_000.0;
/// The setting that is flat (0 dB).
pub const FLAT: u8 = 6;

/// A first-order section: `y = b0 x + b1 x1 - a1 y1`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Coefficients {
    b0: f32,
    b1: f32,
    a1: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Section {
    x1: f32,
    y1: f32,
}

impl Section {
    fn run(&mut self, c: Coefficients, x: f32) -> f32 {
        let y = c.b0 * x + c.b1 * self.x1 - c.a1 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// `(zero_hz, pole_hz, scale)` of the shelf of a bass or treble setting (0 to 12, 6 flat); `None` when flat.
fn shelf(setting: u8, bass: bool) -> Option<(f64, f64, f64)> {
    let steps = i32::from(setting.min(12)) - i32::from(FLAT);
    if steps == 0 {
        return None;
    }
    let gain = 10f64.powf(f64::from(2 * steps.abs()) / 20.0);
    let (zero, reference) = if bass {
        (BASS_ZERO_HZ, BASS_REF_HZ)
    } else {
        (TREBLE_ZERO_HZ, TREBLE_REF_HZ)
    };
    // The pole that gives `gain` at the reference frequency.
    let pole = if bass {
        ((zero * zero + reference * reference) / (gain * gain) - reference * reference).sqrt()
    } else {
        reference / ((1.0 + (reference / zero).powi(2)) / (gain * gain) - 1.0).sqrt()
    };
    // Boost: zero at the turnover. The bass is unity at high frequencies, the treble at low ones.
    let boost_scale = if bass { zero / pole } else { 1.0 };
    if steps > 0 {
        Some((zero, pole, boost_scale))
    } else {
        // The inverse: the pole and zero swap, the scale inverts.
        Some((pole, zero, 1.0 / boost_scale))
    }
}

/// The analog magnitude of a shelf at `hz`, in dB.
fn magnitude_db(zero: f64, pole: f64, scale: f64, hz: f64) -> f64 {
    let m = scale * ((1.0 + (hz / zero).powi(2)) / (1.0 + (hz / pole).powi(2))).sqrt();
    20.0 * m.log10()
}

/// The response of the bass control at setting `setting` at `hz`, dB (the analog filter the sheet describes).
pub fn bass_response_db(setting: u8, hz: f64) -> f64 {
    shelf(setting, true).map_or(0.0, |(z, p, s)| magnitude_db(z, p, s, hz))
}

/// The response of the treble control at setting `setting` at `hz`, dB.
pub fn treble_response_db(setting: u8, hz: f64) -> f64 {
    shelf(setting, false).map_or(0.0, |(z, p, s)| magnitude_db(z, p, s, hz))
}

/// The bilinear transform of a shelf at `rate` Hz, the corner frequencies prewarped.
fn digital(shelf: (f64, f64, f64), rate: f64) -> Coefficients {
    let (zero, pole, scale) = shelf;
    let a = (std::f64::consts::PI * zero / rate).tan();
    let b = (std::f64::consts::PI * pole / rate).tan();
    let denominator = 1.0 + 1.0 / b;
    Coefficients {
        b0: (scale * (1.0 + 1.0 / a) / denominator) as f32,
        b1: (scale * (1.0 - 1.0 / a) / denominator) as f32,
        a1: ((1.0 - 1.0 / b) / denominator) as f32,
    }
}

/// The magnitude of a section at `hz` at the sampling `rate`.
fn digital_gain(c: Coefficients, hz: f64, rate: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * hz / rate;
    let (cos, sin) = (w.cos(), w.sin());
    let (b0, b1, a1) = (f64::from(c.b0), f64::from(c.b1), f64::from(c.a1));
    let num = ((b0 + b1 * cos).powi(2) + (b1 * sin).powi(2)).sqrt();
    let den = ((1.0 + a1 * cos).powi(2) + (a1 * sin).powi(2)).sqrt();
    num / den
}

/// The treble shelf as a digital section whose gain at 15 kHz is the programmed step: the corner frequencies sit a third of the
/// way to the Nyquist frequency, where the bilinear transform bends the response, so the pole is found again against the
/// digital response (bisection; the gain rises with the pole).
fn treble_section(setting: u8, rate: f64) -> Option<Coefficients> {
    let (zero, pole, _) = shelf(setting, false)?;
    let boost = setting > FLAT;
    let (zero, pole) = if boost { (zero, pole) } else { (pole, zero) };
    // `zero` and `pole` of the boost: the lower one is the turnover.
    let turnover = zero.min(pole);
    let gain = 10f64.powf(f64::from(2 * (i32::from(setting) - i32::from(FLAT)).abs()) / 20.0);
    let (mut low, mut high) = (turnover, 0.499 * rate);
    for _ in 0..60 {
        let mid = 0.5 * (low + high);
        let c = digital((turnover, mid, 1.0), rate);
        if digital_gain(c, TREBLE_REF_HZ, rate) < gain {
            low = mid;
        } else {
            high = mid;
        }
    }
    let pole = 0.5 * (low + high);
    Some(if boost {
        digital((turnover, pole, 1.0), rate)
    } else {
        digital((pole, turnover, 1.0), rate)
    })
}

/// The tone controls' filters for the two output channels. Not part of a save state: it is the last few samples only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tone {
    /// `[bass, treble][left, right]`.
    sections: [[Section; 2]; 2],
}

impl Tone {
    /// Filters interleaved stereo `samples` (left, right, ...) at 44 100 Hz with the bass and treble `settings` (0 to 12, 6 flat).
    pub fn apply(&mut self, bass: u8, treble: u8, samples: &mut [f32]) {
        let rate = f64::from(machine::AUDIO_SAMPLE_RATE);
        for (which, setting) in [bass, treble].into_iter().enumerate() {
            let section = if which == 0 {
                shelf(setting, true).map(|f| digital(f, rate))
            } else {
                treble_section(setting, rate)
            };
            let Some(c) = section else {
                // flat: the section forgets its samples
                self.sections[which] = [Section::default(); 2];
                continue;
            };
            for frame in samples.as_chunks_mut::<2>().0 {
                for (channel, sample) in frame.iter_mut().enumerate() {
                    *sample = self.sections[which][channel].run(c, *sample);
                }
            }
        }
    }
}
