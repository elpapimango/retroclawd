//! What answers at each I/O address (`$FF8000-$FFFFFF`). Every row has a fact behind it, listed with its label in
//! `docs/hardware-facts.md`; an address with none is `NoFact`, and the machine stops there instead of guessing.
//!
//! Device pages: *Atari ST Internals* (Abacus 1985) p.55. Register addresses per device: the same book, pp.55-60.
//! Single addresses marked `[O]` were observed in Hatari 2.6.1 (black box) while EmuTOS 1.4 probed them.

use crate::config::Chipset;
use mc68000::Size;

/// A device behind the I/O area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    /// Memory configuration, `$FF8001`.
    Mmu,
    /// Video base, sync mode, palette, resolution.
    Shifter,
    /// DMA and floppy controller access.
    Dma,
    /// YM2149 sound chip.
    Psg,
    /// MC68901 multi-function peripheral.
    Mfp,
    /// The keyboard and MIDI MC6850s.
    Acia,
    /// The STE's DMA sound and Microwire, `$FF8900-$FF8925`.
    SteSound,
    /// The STE's enhanced joystick ports, `$FF9200-$FF9223`.
    SteJoy,
    /// The Blitter, `$FF8A00-$FF8A3F` (Mega ST and STE).
    Blitter,
    /// The Mega ST's RP5C15 clock, `$FFFC20-$FFFC3F`.
    Rtc,
}

impl Device {
    pub fn name(self) -> &'static str {
        match self {
            Device::Mmu => "MMU (memory configuration)",
            Device::Shifter => "Shifter (video)",
            Device::Dma => "DMA / floppy controller",
            Device::Psg => "PSG (YM2149)",
            Device::Mfp => "MFP (MC68901)",
            Device::Acia => "ACIA (MC6850)",
            Device::SteSound => "STE DMA sound and Microwire",
            Device::SteJoy => "STE joystick ports",
            Device::Blitter => "Blitter",
            Device::Rtc => "RTC (RP5C15)",
        }
    }
}

/// How an I/O access ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decode {
    Device(Device),
    /// No device, but the cycle completes: reads `$FF` per byte, writes are ignored.
    Open,
    /// The cycle ends with BERR.
    BusError,
    /// No hardware fact for this access yet.
    NoFact,
}

/// The decode for a chipset: the registers it adds (Mega ST: the clock and the Blitter; STE: the STE Shifter's, DMA sound,
/// joystick ports and the Blitter) first, then the common ST table. Every row of the added ones is `[O]`: the accesses
/// Hatari 2.6.1 (`--machine megast|ste`) answers while EmuTOS probes and uses them (`docs/hardware-facts.md`, "STE and
/// Mega ST").
pub fn decode_for(chipset: Chipset, addr: u32, size: Size) -> Decode {
    let byte = size == Size::Byte;
    let a = addr & 0xFF_FFFF;
    match chipset {
        Chipset::St => {}
        Chipset::MegaSt => match a {
            0xFF_8A00..=0xFF_8A3F => return Decode::Device(Device::Blitter),
            0xFF_FC20..=0xFF_FC3F => return Decode::Device(Device::Rtc),
            _ => {}
        },
        Chipset::Ste => match a {
            0xFF_820D | 0xFF_820F | 0xFF_8264 | 0xFF_8265 if byte => {
                return Decode::Device(Device::Shifter);
            }
            // The video counter's bytes are read/write on the STE.
            0xFF_8900..=0xFF_8925 => return Decode::Device(Device::SteSound),
            0xFF_9200..=0xFF_9223 => return Decode::Device(Device::SteJoy),
            0xFF_8A00..=0xFF_8A3F => return Decode::Device(Device::Blitter),
            // [O] the coprocessor probe of EmuTOS (`tst.w $FA40`): a bus error, and EmuTOS goes on.
            0xFF_FA40..=0xFF_FA5F => return Decode::BusError,
            // [O] the SCC, VME, Falcon sound and `$FFFE00` probes: no bus error trace line in Hatari, EmuTOS goes on.
            0xFF_8C80..=0xFF_8C87
            | 0xFF_8E00..=0xFF_8E0F
            | 0xFF_8926..=0xFF_89FF
            | 0xFF_FE00..=0xFF_FE3F => {
                return Decode::BusError;
            }
            _ => {}
        },
    }
    decode(addr, size)
}

/// The decode of one access of `size` at the 24-bit address `addr` on an ST.
pub fn decode(addr: u32, size: Size) -> Decode {
    use Device as Dev;
    let byte = size == Size::Byte;
    match addr & 0xFF_FFFF {
        // [H] Internals p.55: the single memory configuration register.
        0xFF_8001 if byte => Decode::Device(Dev::Mmu),
        // [O] bus error.
        0xFF_8006 | 0xFF_8A3C => Decode::BusError,
        // [H] Internals pp.56-58: video base high/mid, sync mode, resolution (bytes); palette (bytes or words).
        0xFF_8201 | 0xFF_8203 | 0xFF_820A | 0xFF_8260 if byte => Decode::Device(Dev::Shifter),
        // [O] the video address counter, read only (docs/hardware-facts.md, "Timing (Phase 3)").
        0xFF_8205 | 0xFF_8207 | 0xFF_8209 if byte => Decode::Device(Dev::Shifter),
        0xFF_8240..=0xFF_825F => Decode::Device(Dev::Shifter),
        // [H] Internals pp.58-59: FDC access/sector count and mode (words), DMA address (bytes).
        0xFF_8604 | 0xFF_8606 if !byte => Decode::Device(Dev::Dma),
        0xFF_8609 | 0xFF_860B | 0xFF_860D if byte => Decode::Device(Dev::Dma),
        // [O] open.
        0xFF_860F if byte => Decode::Open,
        // [H] Internals pp.59-60: register select / read, write data.
        0xFF_8800..=0xFF_8803 if byte => Decode::Device(Dev::Psg),
        // [H] Internals pp.60-61: the 24 MFP registers on the odd bytes.
        a @ 0xFF_FA01..=0xFF_FA2F if byte && a & 1 == 1 => Decode::Device(Dev::Mfp),
        // [H] Internals p.62: keyboard control/data, MIDI control/data.
        0xFF_FC00 | 0xFF_FC02 | 0xFF_FC04 | 0xFF_FC06 if byte => Decode::Device(Dev::Acia),
        // [O] open (EmuTOS's Mega ST clock probe on an ST).
        0xFF_FC21 | 0xFF_FC25 | 0xFF_FC27 | 0xFF_FC3B if byte => Decode::Open,
        _ => Decode::NoFact,
    }
}
