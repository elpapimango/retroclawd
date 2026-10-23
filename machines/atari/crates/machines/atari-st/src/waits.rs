//! Wait states of the bus cycles (`accurate` mode). Every rule here is an observation of Hatari 2.6.1 made with the
//! timing probes (`docs/hardware-facts.md`, "Timing (Phase 3)"), not a datasheet fact.

use crate::bus::{Region, StBus};
use crate::io::{self, Decode, Device};
use mc68000::Size;

/// A bus cycle starts on a multiple of 4 clocks: the MMU and the Shifter share the RAM in 4-clock slots, so a cycle that
/// would start between two of them waits for the next (probe P02: every instruction takes a multiple of 4 clocks).
pub(crate) fn align4(t: u64) -> u32 {
    ((4 - (t & 3)) & 3) as u32
}

/// Extra clocks of a cycle to the MFP, the YM2149 or an ACIA: they sit on the slow bus (probe P02, variants 13, 16, 17:
/// an access to them costs 4 clocks more than one to RAM).
pub(crate) const PERIPHERAL_EXTRA: u32 = 4;

/// The 68000's E clock is a tenth of the CPU clock; the acknowledge of an autovectored interrupt waits for it.
pub(crate) const E_PERIOD: u64 = 10;

/// Clocks the acknowledge of an interrupt takes beyond the 4 of the cycle itself, once it has found the E clock (probe
/// P02: the mean number of loop runs a line fits Hatari's to 0.01 with this value).
pub(crate) const IACK_EXTRA: u32 = 8;

impl StBus {
    /// The clocks a bus cycle that would start now at `addr` waits first.
    pub(crate) fn wait_clocks(&self, addr: u32, size: Size) -> u32 {
        let t = self.now;
        match self.region(addr) {
            Region::Io => match io::decode(addr & 0xFF_FFFF, size) {
                Decode::Device(Device::Mfp | Device::Psg | Device::Acia) => {
                    align4(t) + PERIPHERAL_EXTRA
                }
                _ => align4(t),
            },
            _ => align4(t),
        }
    }

    /// The clocks the interrupt acknowledge that would start now waits: for the next E clock edge, counted from the
    /// frame start, and then `IACK_EXTRA`.
    pub(crate) fn iack_wait_clocks(&self) -> u32 {
        let phase = (self.now - self.last_vbl_at) % E_PERIOD;
        ((E_PERIOD - phase) % E_PERIOD) as u32 + IACK_EXTRA
    }
}
