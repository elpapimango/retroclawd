//! The Sega 315-5313 VDP in its Mega Drive mode (mode 5): the control and data ports, VRAM, CRAM and VSRAM, DMA, the
//! frame timing and the picture. Every behaviour has a row in `docs/hardware-facts.md` of the Sega family repository;
//! where a row says `[O]` a probe on several reference emulators fixed it (`scenarios/probes/`).
//!
//! Time is the machine's master clock (`u64`, 53.69 MHz NTSC); the VDP derives its position in the frame from it.
#![forbid(unsafe_code)]

mod dma;
mod fifo;
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
    /// The master clock at which the VDP has finished with each FIFO entry.
    pub(crate) fifo_done: [u64; 4],
    pub(crate) dma: dma::DmaState,
    /// Master clock at which the busy flag of a fill or copy clears.
    pub(crate) dma_busy_until: u64,
    pub(crate) sprite_overflow: bool,
    pub(crate) sprite_collision: bool,
    pub(crate) odd_frame: bool,
    /// Interlace was on at the end of the previous frame.
    pub(crate) was_interlaced: bool,
    /// Interlace mode 2 shows both fields woven into a picture twice as tall instead of one field.
    #[serde(skip)]
    pub(crate) weave: bool,
    /// The FIFO holds the 68000 up (off: every write is accepted at once).
    #[serde(skip)]
    pub(crate) fifo_stalls: bool,
    /// A buffer the sprite pass of a line reuses.
    #[serde(skip)]
    pub(crate) sprite_scratch: Vec<render::SpritePx>,
    /// A light gun's screen position, and whether the controller port's HL pin lets it latch the counters (input state,
    /// not saved).
    #[serde(skip)]
    pub(crate) gun: Option<(u32, u32)>,
    #[serde(skip)]
    pub(crate) gun_enabled: bool,
    /// The external interrupt (level 2), raised by a light gun.
    pub(crate) ext_pending: bool,
    /// The HV counter a light gun latched: its value, whether the latch is held (register 0 bit 1) and the line it was
    /// taken on (a latch that is not held only shows on that line).
    pub(crate) hv_latch: Option<(u16, bool, u32)>,
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
            fifo_done: [0; 4],
            dma: dma::DmaState::default(),
            dma_busy_until: 0,
            sprite_overflow: false,
            sprite_collision: false,
            odd_frame: false,
            was_interlaced: false,
            weave: false,
            fifo_stalls: true,
            sprite_scratch: Vec::new(),
            gun: None,
            gun_enabled: false,
            ext_pending: false,
            hv_latch: None,
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

    /// Whether a value read back from a saved state can be used: every size and index the code relies on holds.
    pub fn validate(&self) -> Result<(), String> {
        if self.vram.len() != VRAM_BYTES
            || self.cram.len() != CRAM_WORDS
            || self.vsram.len() != VSRAM_WORDS
        {
            return Err("a video memory has the wrong size".to_string());
        }
        if self.frame_buf.len() != frame::blank_frame().len() {
            return Err("the picture buffer has the wrong size".to_string());
        }
        let (w, h) = self.shown_size;
        if w > 320 || h > 480 || self.shown.len() != (w * h * 3) as usize {
            return Err("the shown picture does not fit its size".to_string());
        }
        if self.timing.lines() != if self.pal { 313 } else { 262 } {
            return Err("the timing does not match the region".to_string());
        }
        if self.fifo_idx >= 4 {
            return Err("the FIFO position is out of range".to_string());
        }
        Ok(())
    }

    /// Whether the VDP's position in time is believable for a machine whose master clock is `now`: it can lag behind (it is
    /// brought up to date when it is used) but never be ahead of the clock by more than a line.
    pub fn validate_time(&self, now: u64) -> Result<(), String> {
        if self.line_abs > now / LINE_CLOCKS + 2 {
            return Err("the VDP is ahead of the clock".to_string());
        }
        if self
            .fifo_done
            .iter()
            .any(|&t| t > now.saturating_add(8 * LINE_CLOCKS))
        {
            return Err("the FIFO is busy for too long".to_string());
        }
        if self
            .vint_at
            .is_some_and(|t| t > now.saturating_add(2 * LINE_CLOCKS))
        {
            return Err("the vertical interrupt is scheduled too far ahead".to_string());
        }
        Ok(())
    }

    /// Chooses what interlace mode 2 shows: one field per frame (the default) or both fields woven into a 448-line picture.
    /// Where a light gun points (screen pixels) and whether the port's HL pin is enabled.
    pub fn set_gun(&mut self, position: Option<(u32, u32)>, enabled: bool) {
        self.gun = position;
        self.gun_enabled = enabled;
    }

    /// Chooses whether the data port's FIFO holds the 68000 up (the default) or accepts every write at once.
    pub fn set_fifo_stalls(&mut self, stalls: bool) {
        self.fifo_stalls = stalls;
    }

    pub fn set_weave(&mut self, weave: bool) {
        self.weave = weave;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_damaged_vdp_state_is_refused() {
        assert!(Vdp::new(false).validate().is_ok());
        let mut v = Vdp::new(false);
        v.fifo_idx = 9;
        assert!(v.validate().is_err());
        let mut v = Vdp::new(true);
        v.timing = timing::Timing::new(false);
        assert!(v.validate().is_err(), "a PAL VDP with NTSC timing");
        let mut v = Vdp::new(false);
        v.vram.pop();
        assert!(v.validate().is_err());
        let mut v = Vdp::new(false);
        v.shown_size = (320, 240);
        assert!(v.validate().is_err());
        let mut v = Vdp::new(false);
        v.line_abs = u64::MAX;
        assert!(v.validate_time(1_000_000).is_err());
        assert!(Vdp::new(false).validate_time(0).is_ok());
    }

    #[test]
    fn a_vdp_far_behind_the_clock_skips_instead_of_looping() {
        let mut v = Vdp::new(false);
        v.advance(u64::MAX / 4);
        v.advance(u64::MAX);
    }
}
