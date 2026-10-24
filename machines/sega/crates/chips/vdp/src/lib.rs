//! The Sega 315-5313 VDP in its Mega Drive mode (mode 5): the control and data ports, VRAM, CRAM and VSRAM, DMA, the
//! frame timing and the picture. Every behaviour has a row in `docs/hardware-facts.md` of the Sega family repository;
//! where a row says `[O]` a probe on several reference emulators fixed it (`scenarios/probes/`).
//!
//! Time is the machine's master clock (`u64`, 53.69 MHz NTSC); the VDP derives its position in the frame from it.
#![forbid(unsafe_code)]

mod dma;
mod frame;
mod memory;
mod ports;
mod render;
#[cfg(test)]
mod render_tests;
mod timing;

pub use dma::DmaBus;
pub use memory::{CRAM_WORDS, VRAM_BYTES, VSRAM_WORDS};
pub use timing::{LINE_CLOCKS, Phase};

use serde::{Deserialize, Serialize};

/// The number of registers (`$00-$17`).
pub const REGISTERS: usize = 24;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vdp {
    pub(crate) pal: bool,
    pub(crate) regs: [u8; REGISTERS],
    pub(crate) vram: Vec<u8>,
    /// Colour RAM as 9-bit values `BBBGGGRRR`.
    pub(crate) cram: Vec<u16>,
    /// Vertical scroll RAM as 11-bit values.
    pub(crate) vsram: Vec<u16>,
    /// The second command word is due.
    pub(crate) pending: bool,
    /// CD5-CD0 of the command.
    pub(crate) code: u8,
    pub(crate) addr: u16,
    /// Address bits 15 and 14 from the second command word.
    pub(crate) addr_latch: u16,
    /// The last four words written to the memories: what the unused bits of CRAM and VSRAM reads show.
    pub(crate) fifo: [u16; 4],
    pub(crate) fifo_idx: usize,
    pub(crate) dma: dma::DmaState,
    /// Master clock at which the busy flag of a fill or copy clears.
    pub(crate) dma_busy_until: u64,
    pub(crate) sprite_overflow: bool,
    pub(crate) sprite_collision: bool,
    pub(crate) odd_frame: bool,
    pub(crate) timing: timing::Timing,
    /// The next absolute line (V-counter line since power on) whose start has not happened yet.
    pub(crate) line_abs: u64,
    pub(crate) hint_counter: u8,
    pub(crate) hint_pending: bool,
    pub(crate) vint_pending: bool,
    /// The vertical interrupt flag of the status register.
    pub(crate) vint_flag: bool,
    /// The master clock of a vertical interrupt that is due inside the current line.
    pub(crate) vint_at: Option<u64>,
    pub(crate) z80_int: bool,
    /// The picture being drawn (320 pixels pitch) and the last complete one.
    pub(crate) frame_buf: Vec<u8>,
    pub(crate) shown: Vec<u8>,
    pub(crate) shown_size: (u32, u32),
    pub(crate) frames: u64,
}

impl Vdp {
    pub fn new(pal: bool) -> Self {
        Self {
            pal,
            regs: [0; REGISTERS],
            vram: vec![0; VRAM_BYTES],
            cram: vec![0; CRAM_WORDS],
            vsram: vec![0; VSRAM_WORDS],
            pending: false,
            code: 0,
            addr: 0,
            addr_latch: 0,
            fifo: [0; 4],
            fifo_idx: 0,
            dma: dma::DmaState::default(),
            dma_busy_until: 0,
            sprite_overflow: false,
            sprite_collision: false,
            odd_frame: false,
            timing: timing::Timing::new(pal),
            line_abs: 0,
            hint_counter: 0,
            hint_pending: false,
            vint_pending: false,
            vint_flag: false,
            vint_at: None,
            z80_int: false,
            frame_buf: frame::blank_frame(),
            shown: Vec::new(),
            shown_size: (0, 0),
            frames: 0,
        }
    }

    pub fn is_pal(&self) -> bool {
        self.pal
    }

    pub fn reg(&self, n: usize) -> u8 {
        self.regs.get(n).copied().unwrap_or(0)
    }

    pub fn vram(&self) -> &[u8] {
        &self.vram
    }

    pub fn cram(&self) -> &[u16] {
        &self.cram
    }

    pub fn vsram(&self) -> &[u16] {
        &self.vsram
    }

    /// The display is on (register 1, bit 6).
    pub fn display_enabled(&self) -> bool {
        self.regs[1] & 0x40 != 0
    }

    /// 40-cell mode (register 12, bit 0; bit 7 as well on the real chip, see fact V11).
    pub fn h40(&self) -> bool {
        self.regs[12] & 0x01 != 0
    }

    /// 30-row mode (register 1, bit 3).
    pub fn v30(&self) -> bool {
        self.regs[1] & 0x08 != 0
    }

    pub fn dma_enabled(&self) -> bool {
        self.regs[1] & 0x10 != 0
    }

    /// Lines of picture: 224, or 240 in 30-row mode.
    pub fn active_lines(&self) -> u32 {
        if self.v30() { 240 } else { 224 }
    }

    /// The width of the picture in pixels.
    pub fn width(&self) -> u32 {
        if self.h40() { 320 } else { 256 }
    }
}
