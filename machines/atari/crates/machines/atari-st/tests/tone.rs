//! The LMC1992/3 bass and treble controls against the data sheet's figures (`docs/LMC1993.pdf`, `src/tone.rs`): 2 dB a step, plus
//! and minus 12 dB at 50 Hz (bass) and at 15 kHz (treble), the inflection within 3 dB of the final value, a cut the inverse of
//! the boost.

use atari_st::tone::{FLAT, Tone, bass_response_db, treble_response_db};

#[test]
fn the_controls_move_two_decibels_a_step_at_their_reference_frequencies() {
    for setting in 0..=12u8 {
        let want = 2.0 * (f64::from(setting) - f64::from(FLAT));
        assert!(
            (bass_response_db(setting, 50.0) - want).abs() < 0.01,
            "bass {setting}"
        );
        assert!(
            (treble_response_db(setting, 15_000.0) - want).abs() < 0.01,
            "treble {setting}"
        );
    }
}

#[test]
fn the_inflection_frequencies_are_within_three_decibels_of_the_final_gain() {
    // [D] "the inflection frequencies (at which maximum boost and cut are within 3 dB of their final values)": 54 Hz and 13 kHz
    let bass_final = bass_response_db(12, 0.01);
    assert!(bass_final - bass_response_db(12, 54.0) < 3.0);
    assert!(bass_final - bass_response_db(12, 54.0) > 0.0);
    let treble_final = treble_response_db(12, 1.0e6);
    assert!(treble_final - treble_response_db(12, 13_000.0) < 3.0);
    // a boost of 12 dB at the reference frequency is a little more than 12 dB in the end
    assert!((14.0..15.5).contains(&bass_final), "{bass_final}");
    assert!((14.0..15.5).contains(&treble_final), "{treble_final}");
}

#[test]
fn a_cut_is_the_inverse_of_the_boost_of_the_same_size() {
    for hz in [20.0, 50.0, 100.0, 291.0, 1000.0, 2454.0, 8000.0, 15_000.0] {
        for steps in 1..=6u8 {
            let (up, down) = (FLAT + steps, FLAT - steps);
            assert!((bass_response_db(up, hz) + bass_response_db(down, hz)).abs() < 1e-9);
            assert!((treble_response_db(up, hz) + treble_response_db(down, hz)).abs() < 1e-9);
        }
    }
}

#[test]
fn the_flat_setting_changes_nothing() {
    let mut tone = Tone::default();
    let original: Vec<f32> = (0..2000)
        .map(|i| ((i * 37) % 200) as f32 / 100.0 - 1.0)
        .collect();
    let mut samples = original.clone();
    tone.apply(FLAT, FLAT, &mut samples);
    assert_eq!(samples, original);
}

/// The steady-state gain of the filter on a sine of `hz` in the left channel (the right one silent), dB.
fn measured_db(bass: u8, treble: u8, hz: f64) -> f64 {
    let rate = f64::from(machine::AUDIO_SAMPLE_RATE);
    let n = 44_100usize;
    let input: Vec<f32> = (0..n)
        .flat_map(|i| {
            [
                (2.0 * std::f64::consts::PI * hz * i as f64 / rate).sin() as f32 * 0.1,
                0.0,
            ]
        })
        .collect();
    let mut out = input.clone();
    Tone::default().apply(bass, treble, &mut out);
    // the last 22 050 frames, long after the start-up
    let rms = |s: &[f32]| {
        (s.chunks(2)
            .skip(n / 2)
            .map(|f| f64::from(f[0]).powi(2))
            .sum::<f64>()
            / (n / 2) as f64)
            .sqrt()
    };
    20.0 * (rms(&out) / rms(&input)).log10()
}

#[test]
fn the_filters_give_the_sheets_boost_and_cut_on_a_sine() {
    assert!(
        (measured_db(12, FLAT, 50.0) - 12.0).abs() < 0.3,
        "{}",
        measured_db(12, FLAT, 50.0)
    );
    assert!((measured_db(0, FLAT, 50.0) + 12.0).abs() < 0.3);
    // the digital filter follows the analog one at 15 kHz to within a decibel or so (44.1 kHz sampling)
    assert!(
        (measured_db(FLAT, 12, 15_000.0) - 12.0).abs() < 1.5,
        "{}",
        measured_db(FLAT, 12, 15_000.0)
    );
    assert!((measured_db(FLAT, 0, 15_000.0) + 12.0).abs() < 1.5);
    // the mid band is left alone by either control
    assert!(measured_db(12, FLAT, 1000.0).abs() < 1.5);
    assert!(measured_db(FLAT, 12, 1000.0).abs() < 1.5);
    assert!(measured_db(12, 12, 1000.0).abs() < 3.0);
}
