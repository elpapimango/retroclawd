//! The MFP's timer clock in terms of CPU clocks (spec section 8): an exact integer ratio, no accumulation.
//!
//! [H] The MFP timers count a 2.4576 MHz clock ("A millisecond timer", ST-Log 34, July 1989:
//! https://www.atarimagazines.com/st-log/issue34/60_1_A_MILLISECOND_TIMER.php), the rate at which TOS's Timer C
//! set-up (÷64, data 192) gives its documented 200 Hz. The CPU clock is the master crystal / 4.

use serde::{Deserialize, Serialize};

/// MFP timer clock (XTAL input), Hz.
pub const MFP_HZ: u64 = 2_457_600;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MfpClock {
    /// MFP ticks per CPU clock = `num / den` = 4 × MFP_HZ / crystal.
    num: u64,
    den: u64,
    /// Ticks are counted from CPU clock `origin`, `base` of them done by then (0 and 0 until a timer is started in
    /// `accurate` mode: a timer's prescaler then starts at the clock of the control-register write, as Hatari's does,
    /// not at the next tick of a clock that has run since power-on).
    origin: u64,
    base: u64,
}

impl MfpClock {
    pub fn new(crystal_hz: u32) -> Self {
        Self {
            num: 4 * MFP_HZ,
            den: crystal_hz as u64,
            origin: 0,
            base: 0,
        }
    }

    /// MFP ticks completed by CPU clock `cycle`.
    pub fn ticks_at(&self, cycle: u64) -> u64 {
        self.base
            + ((cycle.saturating_sub(self.origin)) as u128 * self.num as u128 / self.den as u128)
                as u64
    }

    /// The first CPU clock by which `tick` MFP ticks have completed.
    pub fn cycle_for(&self, tick: u64) -> u64 {
        self.origin
            + (tick.saturating_sub(self.base) as u128 * self.den as u128).div_ceil(self.num as u128)
                as u64
    }

    /// Counts ticks from CPU clock `cycle` on, `done` of them having been counted by then.
    pub fn restart_at(&mut self, cycle: u64, done: u64) {
        self.origin = cycle;
        self.base = done;
    }
}
