//! What answers at each I/O address (`$FF8000-$FFFFFF`). Every row has a fact behind it, listed with its label in
//! `docs/hardware-facts.md`; an address with none is `NoFact`, and the machine stops there instead of guessing.
//!
//! Device pages: *Atari ST Internals* (Abacus 1985) p.55. Register addresses per device: the same book, pp.55-60.
//! Single addresses marked `[O]` were observed in Hatari 2.6.1 (black box) while EmuTOS 1.4 probed them.

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

/// The decode of one access of `size` at the 24-bit address `addr`.
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
