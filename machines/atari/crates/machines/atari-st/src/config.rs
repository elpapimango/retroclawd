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

/// The chipset: what is fitted besides the common ST parts (spec section 4). The `st`, `stf` and `stfm` of the plugin's
/// `model` option are one chipset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Chipset {
    /// ST, STF, STFM: 192 KB TOS at `$FC0000`, no Blitter unless one is fitted.
    #[default]
    St,
    /// Mega ST: as the ST, with the Blitter and the RP5C15 real-time clock.
    MegaSt,
    /// STE: 256 KB TOS at `$E00000`, the Blitter, the STE Shifter, DMA sound and enhanced joystick ports.
    Ste,
}

impl Chipset {
    /// The machine id of the plugin.
    pub fn id(self) -> &'static str {
        match self {
            Chipset::St => "atari-st",
            Chipset::MegaSt => "atari-mega-st",
            Chipset::Ste => "atari-ste",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [Chipset::St, Chipset::MegaSt, Chipset::Ste]
            .into_iter()
            .find(|c| c.id().eq_ignore_ascii_case(id))
    }

    /// Where the TOS image is mapped and how big it is. [H] *Internals* "below 1.4" at `$FC0000`, 192 KB; the STE's TOS
    /// 1.6x/2.0x and EmuTOS 256K at `$E00000`.
    pub fn rom_base(self) -> u32 {
        match self {
            Chipset::Ste => 0xE0_0000,
            _ => 0xFC_0000,
        }
    }

    pub fn rom_size(self) -> usize {
        match self {
            Chipset::Ste => 262_144,
            _ => 196_608,
        }
    }

    /// The Blitter is standard on the Mega ST and the STE.
    pub fn standard_blitter(self) -> bool {
        self != Chipset::St
    }

    pub fn has_rtc(self) -> bool {
        self == Chipset::MegaSt
    }

    pub fn is_ste(self) -> bool {
        self == Chipset::Ste
    }

    /// The two bits the state header keeps.
    pub fn code(self) -> u8 {
        self as u8
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
