//! What a machine is built from.

use serde::{Deserialize, Serialize};

/// The RAM sizes the ST memory controller can be configured for (`--ram`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RamSize {
    K256,
    K512,
    M1,
    M2,
    M2_5,
    M4,
}

impl RamSize {
    pub fn bytes(self) -> usize {
        match self {
            RamSize::K256 => 256 << 10,
            RamSize::K512 => 512 << 10,
            RamSize::M1 => 1 << 20,
            RamSize::M2 => 2 << 20,
            RamSize::M2_5 => (2 << 20) + (512 << 10),
            RamSize::M4 => 4 << 20,
        }
    }

    /// The option value (`256k`, `512k`, `1m`, `2m`, `2.5m`, `4m`).
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "256k" => RamSize::K256,
            "512k" => RamSize::K512,
            "1m" => RamSize::M1,
            "2m" => RamSize::M2,
            "2.5m" => RamSize::M2_5,
            "4m" => RamSize::M4,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Region {
    Pal,
    Ntsc,
}

/// `--accuracy`: the same devices either way; `Accurate` lets the bus see every cycle (spec section 11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accuracy {
    Fast,
    Accurate,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub ram: RamSize,
    pub region: Region,
    pub accuracy: Accuracy,
    pub monitor: st_shifter::Monitor,
    /// The TOS image as stored (big-endian, as the CPU reads it).
    pub rom: Vec<u8>,
    /// Instructions the trace file records (0: no trace).
    pub trace_limit: u32,
    /// I/O accesses the I/O log records (0: none).
    pub io_log_limit: u32,
}
