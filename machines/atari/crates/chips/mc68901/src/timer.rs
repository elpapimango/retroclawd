//! One MFP timer: prescaler and 8-bit main counter (datasheet §5).

use serde::{Deserialize, Serialize};

/// Prescaler of each control code (Figure 5-3): 1-7 delay mode, 9-15 pulse width mode with the same divisors.
const PRESCALE: [u64; 8] = [0, 4, 10, 16, 50, 64, 100, 200];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Stopped,
    Delay(u64),
    EventCount,
    PulseWidth(u64),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Timer {
    /// Mode bits (3-0 for A/B; C/D use codes 0-7).
    control: u8,
    /// The timer data register: the reload value (0 counts 256).
    data: u8,
    /// The main counter (0 stands for 256).
    counter: u8,
    /// Timer clocks until the prescaler's next count pulse (1..=prescale while running).
    prescale_left: u64,
    output: bool,
}

impl Timer {
    pub(crate) fn mode(&self) -> Mode {
        match self.control & 0x0F {
            0 => Mode::Stopped,
            c @ 1..=7 => Mode::Delay(PRESCALE[c as usize]),
            8 => Mode::EventCount,
            c => Mode::PulseWidth(PRESCALE[(c - 8) as usize]),
        }
    }

    pub(crate) fn control(&self) -> u8 {
        self.control
    }

    /// A new mode. Starting the prescaler, or changing its divisor, restarts it: §5.1.1 says the first timeout after
    /// a change comes at an indeterminate time within 1-200 clocks; a full prescale period is the choice made
    /// ([H?]). Stopping loses the residual count (Figure 5-3 note).
    pub(crate) fn set_control(&mut self, control: u8) {
        let before = self.prescale();
        self.control = control & 0x0F;
        let after = self.prescale();
        if after != 0 && after != before {
            self.prescale_left = after;
        }
    }

    fn prescale(&self) -> u64 {
        match self.mode() {
            Mode::Delay(p) | Mode::PulseWidth(p) => p,
            Mode::Stopped | Mode::EventCount => 0,
        }
    }

    /// §5.2.1: a stopped timer loads the counter at once; a running one at its next timeout.
    pub(crate) fn write_data(&mut self, value: u8) {
        self.data = value;
        if self.mode() == Mode::Stopped {
            self.counter = value;
        }
    }

    /// §5.2.1: reading the data register gives the main counter.
    pub(crate) fn read_data(&self) -> u8 {
        self.counter
    }

    pub(crate) fn output(&self) -> bool {
        self.output
    }

    pub(crate) fn force_output_low(&mut self) {
        self.output = false;
    }

    /// Device reset (§2.2.3): stopped, output low; the data register and the counter stay.
    pub(crate) fn reset(&mut self) {
        self.control = 0;
        self.prescale_left = 0;
        self.output = false;
    }

    /// Applies `pulses` count pulses to the main counter; returns the number of timeouts (§5.1.1: counting through
    /// 01 reloads from the data register and toggles the output).
    pub(crate) fn count(&mut self, pulses: u64) -> u64 {
        if pulses == 0 {
            return 0;
        }
        let current = if self.counter == 0 {
            256
        } else {
            self.counter as u64
        };
        if pulses < current {
            self.counter = (current - pulses) as u8;
            return 0;
        }
        let reload = if self.data == 0 {
            256
        } else {
            self.data as u64
        };
        let rest = pulses - current;
        let timeouts = 1 + rest / reload;
        self.counter = (reload - rest % reload) as u8;
        if timeouts % 2 == 1 {
            self.output = !self.output;
        }
        timeouts
    }

    /// Lets `clocks` timer clocks pass with the prescaler running (delay mode, or pulse width mode while the input
    /// is active); returns the timeouts.
    pub(crate) fn advance(&mut self, clocks: u64, active: bool) -> u64 {
        let p = match self.mode() {
            Mode::Delay(p) => p,
            Mode::PulseWidth(p) if active => p,
            _ => return 0,
        };
        if clocks < self.prescale_left {
            self.prescale_left -= clocks;
            return 0;
        }
        let rest = clocks - self.prescale_left;
        let pulses = 1 + rest / p;
        self.prescale_left = p - rest % p;
        self.count(pulses)
    }

    /// Timer clocks until the next timeout, if the prescaler is running.
    pub(crate) fn next_timeout(&self, active: bool) -> Option<u64> {
        let p = match self.mode() {
            Mode::Delay(p) => p,
            Mode::PulseWidth(p) if active => p,
            _ => return None,
        };
        let current = if self.counter == 0 {
            256
        } else {
            self.counter as u64
        };
        Some(self.prescale_left + (current - 1) * p)
    }
}
