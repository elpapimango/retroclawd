//! The Z80 subsystem: 8 KB of RAM, the bus request and reset lines the 68000 drives, the bank register of the window onto the
//! 68000's address space. Facts Z1-Z12 in `docs/hardware-facts.md`; probe P05 (Genesis Plus GX, PicoDrive and BlastEm agree on
//! what is used here).
//!
//! Time: the Z80 runs in master clocks (15 per Z80 clock) and only while it is out of reset and the 68000 has not taken its bus.

use crate::config::Z80_DIVIDER;
use serde::{Deserialize, Serialize};

pub const Z80_RAM_SIZE: usize = 0x2000;

/// Master clocks the grant of a bus request takes after the Z80 finished the access it was in (a few Z80 clocks on average,
/// fact Z10: Kabuto measured 3.3 on average).
const GRANT_DELAY: u64 = 3 * Z80_DIVIDER;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Z80Sys {
    pub cpu: z80::Cpu,
    pub ram: Vec<u8>,
    /// Bits 23-15 of the 68000 address the window `$8000-$FFFF` shows.
    pub bank: u16,
    /// The 68000 asked for the Z80's bus (`$A11100`).
    requested: bool,
    /// The master clock at which a request was granted (the Z80 finished its access).
    grant_at: u64,
    /// The Z80 is held in reset (`$A11200` low); it starts so at power on.
    reset: bool,
    /// The master clock up to which the Z80 has run.
    pub time: u64,
    /// Z80 clocks spent inside the instruction being executed, for the time of an access in the middle of it.
    pub offset: u64,
    /// Z80 clocks the instruction being executed waited for the 68000's bus (included in `offset`).
    #[serde(skip)]
    pub wait: u64,
}

impl Z80Sys {
    pub fn new() -> Self {
        Self {
            cpu: z80::Cpu::new(),
            ram: vec![0; Z80_RAM_SIZE],
            bank: 0,
            requested: false,
            grant_at: 0,
            reset: true,
            time: 0,
            offset: 0,
            wait: 0,
        }
    }

    /// Whether a value read back from a saved state is usable.
    pub fn validate(&self) -> Result<(), String> {
        if self.ram.len() != Z80_RAM_SIZE || self.bank > 0x1FF {
            return Err("the Z80 RAM or bank register is out of range".to_string());
        }
        Ok(())
    }

    /// Whether the Z80's clocks are believable for a machine whose master clock is `now`: the Z80 runs a few cycles ahead of
    /// the 68000 at most (it finishes the instruction it is in), and a grant is never far in the future.
    pub fn validate_time(&self, now: u64) -> Result<(), String> {
        let slack = 64 * Z80_DIVIDER;
        if self.time > now.saturating_add(slack)
            || self.grant_at > now.saturating_add(slack)
            || self.offset > 64
        {
            return Err("the Z80 is ahead of the clock".to_string());
        }
        Ok(())
    }

    /// The 68000 wrote the bus request register.
    pub fn write_bus_request(&mut self, request: bool, now: u64) {
        if request && !self.requested {
            self.requested = true;
            // The Z80 gives the bus up when it has finished what it is doing; in reset it has nothing to finish.
            self.grant_at = if self.reset {
                now
            } else {
                self.time.max(now).saturating_add(GRANT_DELAY)
            };
        } else if !request && self.requested {
            self.requested = false;
            // The Z80 goes on from now on.
            self.time = self.time.max(now);
        }
    }

    /// The 68000 wrote the reset register (`released` = the line is high, the Z80 may run).
    pub fn write_reset(&mut self, released: bool, now: u64) {
        if !released {
            self.reset = true;
            self.cpu = z80::Cpu::new();
            if self.requested {
                self.grant_at = now;
            }
        } else if self.reset {
            self.reset = false;
            self.time = self.time.max(now);
        }
    }

    pub fn in_reset(&self) -> bool {
        self.reset
    }

    pub fn bus_requested(&self) -> bool {
        self.requested
    }

    /// Whether the 68000 owns the Z80's bus at `now`.
    pub fn granted(&self, now: u64) -> bool {
        self.requested && now >= self.grant_at
    }

    /// The value the 68000 reads at `$A11100`: bit 0 is 0 once the bus is granted.
    pub fn read_bus_request(&self, now: u64) -> u8 {
        u8::from(!self.granted(now))
    }

    /// `$A11200`: bit 0 high while the Z80 is running.
    pub fn read_reset(&self) -> u8 {
        u8::from(!self.reset)
    }

    /// Whether the Z80 executes instructions at the moment.
    pub fn running(&self, now: u64) -> bool {
        !self.reset && !(self.requested && now >= self.grant_at)
    }

    /// The RAM through the 68000's window: allowed while the 68000 owns the bus and the Z80 is not in reset.
    pub fn ram_open_to_68k(&self, now: u64) -> bool {
        self.granted(now) && !self.reset
    }

    /// A write to the bank register: the bit shifts in at the top, nine bits make the bank ([H?] fact Z5, probe P05).
    pub fn write_bank(&mut self, value: u8) {
        self.bank = (self.bank >> 1 | u16::from(value & 1) << 8) & 0x1FF;
    }
}

impl Default for Z80Sys {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_z80_is_in_reset_at_power_on_and_runs_after_the_reset_is_released() {
        let mut z = Z80Sys::new();
        assert!(z.in_reset());
        assert!(!z.running(0));
        z.write_reset(true, 100);
        assert!(z.running(100));
        assert_eq!(z.read_reset(), 1);
        z.write_reset(false, 200);
        assert!(!z.running(200));
        assert_eq!(z.read_reset(), 0);
    }

    #[test]
    fn a_bus_request_is_granted_after_a_short_delay_unless_the_z80_is_in_reset() {
        let mut z = Z80Sys::new();
        z.write_reset(true, 0);
        z.write_bus_request(true, 1000);
        assert_eq!(z.read_bus_request(1000), 1, "not yet granted");
        assert_eq!(z.read_bus_request(1000 + GRANT_DELAY), 0);
        assert!(!z.running(1000 + GRANT_DELAY));
        z.write_bus_request(false, 5000);
        assert!(z.running(5000));
        assert_eq!(z.read_bus_request(5000), 1);
        // in reset: granted at once
        let mut z = Z80Sys::new();
        z.write_bus_request(true, 7);
        assert_eq!(z.read_bus_request(7), 0);
    }

    #[test]
    fn the_ram_is_open_to_the_68000_only_with_the_bus_and_out_of_reset() {
        let mut z = Z80Sys::new();
        z.write_bus_request(true, 0);
        assert!(!z.ram_open_to_68k(100), "in reset: closed");
        z.write_reset(true, 100);
        assert!(z.ram_open_to_68k(100 + GRANT_DELAY));
        z.write_bus_request(false, 500);
        assert!(!z.ram_open_to_68k(600));
    }

    #[test]
    fn the_bank_register_shifts_in_nine_bits_lsb_first() {
        let mut z = Z80Sys::new();
        for _ in 0..9 {
            z.write_bank(1);
        }
        assert_eq!(z.bank, 0x1FF);
        for _ in 0..9 {
            z.write_bank(0);
        }
        assert_eq!(z.bank, 0);
        // address bits 15..23 of $380000 = %1_1100_0000: written from bit 15 up: 0,0,0,0,0,0,1,1,1
        for b in [0, 0, 0, 0, 0, 0, 1, 1, 1] {
            z.write_bank(b);
        }
        assert_eq!(u32::from(z.bank) << 15, 0x1C0 << 15);
        assert_eq!(0x1C0u32 << 15, 0x0E0_0000);
    }

    #[test]
    fn a_reset_clears_the_z80_registers() {
        let mut z = Z80Sys::new();
        z.write_reset(true, 0);
        z.cpu.pc = 0x1234;
        z.write_reset(false, 10);
        assert_eq!(z.cpu.pc, 0);
    }

    #[test]
    fn a_z80_ahead_of_the_clock_is_refused() {
        let mut z = Z80Sys::new();
        assert!(z.validate_time(0).is_ok());
        z.time = u64::MAX;
        assert!(z.validate_time(100).is_err());
        let mut z = Z80Sys::new();
        z.write_bus_request(true, u64::MAX - 1);
        z.write_reset(true, u64::MAX - 1);
    }
}
