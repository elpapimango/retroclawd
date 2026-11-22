//! The Mega-CD's two sound sources, the PCM chip (32 552 Hz) and the CD audio (44 100 Hz), brought to the Genesis' own output
//! rate and added to its sound (fact C22). Each source queues the samples its clock produced; the mixer takes them at the pace
//! of the output samples with linear interpolation between neighbours.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// A queue of stereo samples at `in_hz` read at the output rate.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resampler {
    in_hz: u64,
    queue: VecDeque<(f32, f32)>,
    prev: (f32, f32),
    cur: (f32, f32),
    /// Input samples times the master clock rate not yet taken: a sample is due when this reaches `master_hz`.
    acc: u64,
}

/// The queue holds at most this much audio (half a second); a source nobody drains does not grow without bound.
const MAX_SECONDS_DIV: u64 = 2;

impl Resampler {
    pub fn new(in_hz: u64) -> Self {
        Self {
            in_hz,
            queue: VecDeque::new(),
            prev: (0.0, 0.0),
            cur: (0.0, 0.0),
            acc: 0,
        }
    }

    pub fn push(&mut self, l: f32, r: f32) {
        self.queue.push_back((l, r));
        let max = (self.in_hz / MAX_SECONDS_DIV) as usize;
        if self.queue.len() > max {
            self.queue.pop_front();
        }
    }

    /// The next output sample, `frame_clocks` master clocks after the last, for a master clock of `master_hz`.
    pub fn next(&mut self, frame_clocks: u64, master_hz: u64) -> (f32, f32) {
        self.acc += self.in_hz * frame_clocks;
        while self.acc >= master_hz {
            self.acc -= master_hz;
            self.prev = self.cur;
            // when the source runs dry the last value is held rather than dropping to zero (no click)
            self.cur = self.queue.pop_front().unwrap_or(self.cur);
        }
        let t = self.acc as f32 / master_hz as f32;
        (
            self.prev.0 + (self.cur.0 - self.prev.0) * t,
            self.prev.1 + (self.cur.1 - self.prev.1) * t,
        )
    }

    /// Whether a state's values are ones this resampler can have (a state is untrusted input).
    pub fn validate(&self, in_hz: u64) -> Result<(), String> {
        if self.in_hz != in_hz || self.queue.len() as u64 > in_hz / MAX_SECONDS_DIV {
            return Err("an audio resampler of the state does not fit this machine".to_string());
        }
        Ok(())
    }

    pub fn queued(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_at_the_output_rate_comes_out_one_sample_late() {
        let mut r = Resampler::new(1000);
        for i in 1..=4 {
            r.push(i as f32, -(i as f32));
        }
        // output rate 1000 Hz: master 1_000_000 Hz, 1000 clocks a sample
        let out: Vec<f32> = (0..4).map(|_| r.next(1000, 1_000_000).0).collect();
        assert_eq!(out, [0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_slower_source_is_interpolated_and_a_dry_one_holds_its_last_value() {
        let mut r = Resampler::new(500); // half the output rate
        r.push(0.0, 0.0);
        r.push(1.0, 1.0);
        // two output samples per input sample; the pair of inputs (0, 1) reaches the output after the first pair has moved through
        let out: Vec<f32> = (0..6).map(|_| r.next(1000, 1_000_000).0).collect();
        assert_eq!(out, [0.0, 0.0, 0.0, 0.0, 0.5, 1.0]);
    }

    #[test]
    fn a_source_nobody_drains_stays_bounded() {
        let mut r = Resampler::new(100);
        for i in 0..1000 {
            r.push(i as f32, 0.0);
        }
        assert_eq!(r.queued(), 50);
    }
}
