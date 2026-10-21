//! The YM2149 at `$FF8800-$FF8803` (`docs/hardware-facts.md`, "PSG").
//!
//! - [H] *Atari ST Internals* pp.59-60: a write to `$FF8800` selects a register, a write to `$FF8802` writes it, a
//!   read of `$FF8800` reads it, and a read of `$FF8802` gives `$FF`. [H?] `$FF8801`/`$FF8803` act as `$FF8800`/
//!   `$FF8802` (the chip sees A1 only).
//! - [H] the schematic (*Internals* figure 2.3-2) gives the YM's clock as "2 MHz"; [H?] it is the CPU clock / 4 and the
//!   SEL pin keeps the master clock equal to the input, so the YM runs at CPU clock / 4.
//! - [D] YM2149 catalog: a port set to output by R7 bits 6/7 reads back its latch; [H?] a port set to input reads
//!   `$FF` and drives nothing (the ST's port A lines are outputs; nothing reads them as inputs in this slice).
//!
//! Samples are made from CPU cycles: sample `n` (from 0) is the output at cycle `ceil((n + 1) × clock / 44 100)`, an exact integer
//! rule, so the count never drifts.

use std::collections::VecDeque;

use ay_3_8910::{Ay3_8910, Variant};

/// CPU clocks per YM clock.
const YM_DIVIDER: u64 = 4;
/// Undrained samples kept (two seconds): a run that never drains audio must not grow without bound.
const MAX_SAMPLES: usize = 2 * machine::AUDIO_SAMPLE_RATE as usize;

#[derive(Debug, Clone)]
pub struct Psg {
    ym: Ay3_8910,
    clock_hz: u64,
    /// The CPU cycle the chip has been advanced to.
    at: u64,
    /// Samples produced so far (the next one's index).
    sample_index: u64,
    samples: VecDeque<f32>,
}

impl Psg {
    pub fn new(clock_hz: u64) -> Self {
        Self {
            ym: Ay3_8910::with_variant(Variant::Ym2149),
            clock_hz,
            at: 0,
            sample_index: 0,
            samples: VecDeque::new(),
        }
    }

    /// Port A's output: the R14 latch when R7 sets it to output, else `$FF` (floating).
    pub fn port_a(&self) -> u8 {
        if self.ym.io_port_output(0) {
            self.ym.io_port_latch(0)
        } else {
            0xFF
        }
    }

    pub fn chip(&self) -> &Ay3_8910 {
        &self.ym
    }

    /// The cycle at which sample `n` (from 0) is taken: the end of its period, so `t` cycles hold exactly
    /// `floor(t × 44 100 / clock)` samples.
    fn sample_cycle(&self, n: u64) -> u64 {
        ((n + 1) as u128 * self.clock_hz as u128).div_ceil(machine::AUDIO_SAMPLE_RATE as u128)
            as u64
    }

    fn tick_to(&mut self, cycle: u64) {
        let ticks = cycle / YM_DIVIDER - self.at / YM_DIVIDER;
        if ticks > 0 {
            self.ym.tick(ticks as u32);
        }
        self.at = cycle;
    }

    /// Advances the chip to CPU cycle `cycle` (never backwards), taking every sample due on the way.
    pub fn catch_up(&mut self, cycle: u64) {
        if cycle <= self.at {
            return;
        }
        loop {
            let next = self.sample_cycle(self.sample_index);
            if next > cycle {
                break;
            }
            self.tick_to(next.max(self.at));
            if self.samples.len() == MAX_SAMPLES {
                self.samples.pop_front();
            }
            self.samples.push_back(self.ym.output());
            self.sample_index += 1;
        }
        self.tick_to(cycle);
    }

    /// The samples taken so far, removed.
    pub fn drain(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.samples).into()
    }

    /// A byte read at `addr` (`$FF8800-$FF8803`).
    pub fn read(&self, addr: u32) -> u8 {
        if addr & 2 != 0 {
            return 0xFF;
        }
        let reg = self.ym.selected_register();
        match reg {
            14 | 15 if !self.ym.io_port_output(reg as usize - 14) => 0xFF,
            _ => self.ym.read(),
        }
    }

    /// A byte written at `addr` (`$FF8800-$FF8803`).
    pub fn write(&mut self, addr: u32, value: u8) {
        if addr & 2 == 0 {
            self.ym.select_register(value);
        } else {
            self.ym.write(value);
        }
    }
}
