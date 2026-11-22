//! The STE DAC's output filters against the STe manual's description (`src/dac.rs`): a four-pole low-pass at 40 % of the sample
//! rate and a two-pole one at 16 kHz.

use atari_st::dac::{Dac, corner_hz, response_db};

#[test]
fn the_switched_filter_follows_the_sample_rate() {
    for (rate, hz) in [(0u8, 2503.2), (1, 5006.8), (2, 10_013.2), (3, 20_026.4)] {
        assert!((corner_hz(rate) - hz).abs() < 0.1, "{rate}");
    }
}

#[test]
fn the_analog_response_has_four_poles_below_the_corner_and_a_fixed_stage_at_16_khz() {
    // at 6258 Hz the corner is 2.5 kHz: well below 16 kHz the switched filter alone counts, -3 dB at the corner (Butterworth)
    assert!((response_db(0, corner_hz(0)) + 3.0).abs() < 0.1);
    // a decade above the corner a fourth-order filter is 80 dB down, and the fixed stage adds its own attenuation there
    assert!(response_db(0, 10.0 * corner_hz(0)) < -80.0);
    assert!(response_db(0, 10.0 * corner_hz(0)) > -95.0);
    // flat well below it
    assert!(response_db(0, 100.0).abs() < 0.01);
    // at 50 kHz the 20 kHz corner leaves the fixed filter to define the top: -3 dB at 16 kHz plus the switched stage's tail
    assert!(response_db(3, 16_000.0) < -3.0);
    assert!(response_db(3, 16_000.0) > -6.5);
}

fn measured_db(rate: u8, hz: f64) -> f64 {
    let sample_rate = f64::from(machine::AUDIO_SAMPLE_RATE);
    let n = 44_100usize;
    let input: Vec<f32> = (0..n)
        .map(|i| (2.0 * std::f64::consts::PI * hz * i as f64 / sample_rate).sin() as f32 * 0.5)
        .collect();
    let mut out = input.clone();
    Dac::default().apply(rate, 0, &mut out);
    let rms = |s: &[f32]| {
        (s[n / 2..]
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            / (n / 2) as f64)
            .sqrt()
    };
    20.0 * (rms(&out) / rms(&input)).log10()
}

#[test]
fn the_digital_filter_follows_the_analog_one_in_the_audible_band() {
    // the passband and the corner: within a decibel
    for (rate, hz) in [
        (0u8, 500.0),
        (0, 2500.0),
        (1, 5000.0),
        (2, 8000.0),
        (3, 1000.0),
    ] {
        let (want, got) = (response_db(rate, hz), measured_db(rate, hz));
        assert!(
            (want - got).abs() < 1.0,
            "rate {rate} at {hz} Hz: analog {want:.2} dB, measured {got:.2} dB"
        );
    }
    // the stop band of a steep filter near a third of the sampling rate: the bilinear transform bends the frequency axis a little
    for (rate, hz) in [(0u8, 4000.0), (1, 8000.0)] {
        let (want, got) = (response_db(rate, hz), measured_db(rate, hz));
        assert!(
            (want - got).abs() < 3.0,
            "rate {rate} at {hz} Hz: analog {want:.2} dB, measured {got:.2} dB"
        );
    }
}

#[test]
fn a_steady_level_passes_unchanged() {
    let mut out = vec![0.25f32; 4000];
    Dac::default().apply(0, 1, &mut out);
    assert!((out[3999] - 0.25).abs() < 1e-4);
}
