//! The frame: lines drawn as time passes, the interrupts and the HV counter (facts V18, V19, V25-V27).
//!
//! Lines here are V-counter lines: a line begins when the V counter changes, which is `PICTURE_OFFSET` master clocks before
//! the left edge of its picture. The positions of the events inside a line (picture start, the vertical interrupt, the
//! horizontal blanking flag) are approximations taken from what the reference emulators agree on; Phase 3 replaces them with
//! probe-measured pixel positions.

use crate::Vdp;
use crate::timing::LINE_CLOCKS;

/// Master clocks from the start of a V-counter line to the left edge of its picture, H40 and H32 (fact V25).
const PICTURE_OFFSET_H40: u64 = 780;
const PICTURE_OFFSET_H32: u64 = 760;
/// Master clocks from the start of the first blank line to the vertical interrupt (fact V26).
const VINT_DELAY_H40: u64 = 788;
const VINT_DELAY_H32: u64 = 770;
/// The horizontal blanking flag is set between these clocks of a V-counter line (fact V27).
const HBLANK_H40: (u64, u64) = (228, 872);
const HBLANK_H32: (u64, u64) = (280, 860);

/// The widest picture and the tallest.
const MAX_WIDTH: usize = 320;
const MAX_HEIGHT: usize = 240;

/// The 8 (4) lowest bits of the V counter of the last lines jump to the top of the range (fact V19).
fn vcounter_max(pal: bool, v30: bool) -> u32 {
    match (pal, v30) {
        (false, false) => 0xEA,
        (true, false) => 0x102,
        (false, true) => 0x106,
        (true, true) => 0x10A,
    }
}

impl Vdp {
    fn picture_offset(&self) -> u64 {
        if self.h40() {
            PICTURE_OFFSET_H40
        } else {
            PICTURE_OFFSET_H32
        }
    }

    /// Brings the VDP up to master clock `now`: lines whose time has come are drawn and their events happen. Call it
    /// before anything reads or writes the VDP, so the registers a line is drawn with are the ones in force at its time.
    pub fn advance(&mut self, now: u64) {
        let lines = u64::from(self.timing_lines());
        // A line count that is far behind the clock (a damaged state) is caught up by skipping, not by looping.
        let behind = now / LINE_CLOCKS;
        if behind > self.line_abs.saturating_add(4 * lines) {
            self.line_abs = behind - lines;
            self.vint_at = None;
        }
        while let Some(line_start) = self.line_abs.checked_mul(LINE_CLOCKS) {
            let mut next = line_start;
            let mut vint = false;
            if let Some(t) = self.vint_at
                && t <= next
            {
                next = t;
                vint = true;
            }
            if next > now {
                break;
            }
            if vint {
                self.vint_at = None;
                self.vint_pending = true;
                self.vint_flag = true;
                self.z80_int = true;
                continue;
            }
            let vl = (self.line_abs % lines) as u32;
            self.z80_int = false;
            let active = self.active_lines();
            if vl < active {
                self.draw_line(vl);
                self.hint_step(false);
            } else if vl == active {
                self.finish_frame();
                self.hint_step(true);
                let delay = if self.h40() {
                    VINT_DELAY_H40
                } else {
                    VINT_DELAY_H32
                };
                self.vint_at = Some(line_start + delay);
            }
            if u64::from(vl) == lines - 1 {
                self.hint_counter = self.regs[10];
            }
            self.line_abs += 1;
        }
    }

    fn timing_lines(&self) -> u32 {
        if self.pal { 313 } else { 262 }
    }

    /// The horizontal interrupt counter, once per line: it runs down to zero, raises the interrupt and reloads (the line
    /// after the picture only checks it).
    fn hint_step(&mut self, only_check: bool) {
        if self.hint_counter == 0 {
            if !only_check {
                self.hint_counter = self.regs[10];
            }
            self.hint_pending = true;
        } else if !only_check {
            self.hint_counter -= 1;
        }
    }

    fn draw_line(&mut self, vl: u32) {
        let width = self.width() as usize;
        let mut row = [0u8; MAX_WIDTH * 3];
        self.render_line(vl, &mut row[..width * 3]);
        let at = vl as usize * MAX_WIDTH * 3;
        if at + width * 3 <= self.frame_buf.len() {
            self.frame_buf[at..at + width * 3].copy_from_slice(&row[..width * 3]);
        }
    }

    /// The picture is complete: it becomes the one `frame` returns.
    fn finish_frame(&mut self) {
        let (w, h) = (self.width() as usize, self.active_lines() as usize);
        let mut shown = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            let at = y * MAX_WIDTH * 3;
            shown.extend_from_slice(&self.frame_buf[at..at + w * 3]);
        }
        self.shown = shown;
        self.shown_size = (w as u32, h as u32);
        self.frames += 1;
    }

    /// The last complete picture: width, height and RGB bytes. `None` until the first frame is done.
    pub fn frame(&self) -> Option<(u32, u32, &[u8])> {
        (self.frames > 0).then_some((self.shown_size.0, self.shown_size.1, &self.shown[..]))
    }

    /// Complete frames drawn so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The interrupt level the VDP asks the 68000 for: 6 for the vertical interrupt, 4 for the horizontal one, else 0
    /// (fact V18); each only if its enable bit is set.
    pub fn irq_level(&self) -> u8 {
        if self.vint_pending && self.regs[1] & 0x20 != 0 {
            6
        } else if self.hint_pending && self.regs[0] & 0x10 != 0 {
            4
        } else {
            0
        }
    }

    /// The 68000 acknowledged an interrupt: the vertical one has priority, as in `irq_level`.
    pub fn acknowledge(&mut self) {
        if self.vint_pending && self.regs[1] & 0x20 != 0 {
            self.vint_pending = false;
            self.vint_flag = false;
        } else {
            self.hint_pending = false;
        }
    }

    /// The level of the Z80's interrupt line: high for the rest of the line the vertical interrupt happens in.
    pub fn z80_int(&self) -> bool {
        self.z80_int
    }

    /// The vertical interrupt flag of the status register.
    pub(crate) fn vint_flag(&self) -> bool {
        self.vint_flag
    }

    /// Whether the horizontal blanking flag is set at the given clock of a V-counter line.
    pub(crate) fn in_hblank(&self, clock: u64) -> bool {
        let (start, end) = if self.h40() { HBLANK_H40 } else { HBLANK_H32 };
        clock >= start && clock < end
    }

    /// The HV counter: vertical counter in the high byte, horizontal in the low byte (fact V19; positions are the
    /// approximation described at the top of this file).
    pub fn read_hv(&self, now: u64) -> u16 {
        let lines = u64::from(self.timing_lines());
        let in_frame = now % (lines * LINE_CLOCKS);
        let vl = (in_frame / LINE_CLOCKS) as u32;
        let clock = in_frame % LINE_CLOCKS;
        let mut v = vl;
        if v > vcounter_max(self.pal, self.v30()) {
            v = v.wrapping_sub(lines as u32);
        }
        let offset = self.picture_offset();
        let h = if self.h40() {
            if clock >= offset {
                (clock - offset) / 16
            } else {
                // the end of the previous line: counts 0xA5..0xB6, then 0xE4..0xFF
                let n = clock * 46 / offset;
                if n < 18 { 0xA5 + n } else { 0xE4 + (n - 18) }
            }
        } else if clock >= offset {
            (clock - offset) / 20
        } else {
            let n = clock * 38 / offset;
            if n < 15 { 0x85 + n } else { 0xE9 + (n - 15) }
        };
        ((v & 0xFF) as u16) << 8 | (h & 0xFF) as u16
    }
}

pub(crate) fn blank_frame() -> Vec<u8> {
    vec![0; MAX_WIDTH * MAX_HEIGHT * 3]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vdp() -> Vdp {
        let mut v = Vdp::new(false);
        v.regs[1] = 0x74;
        v.regs[10] = 0xFF;
        v
    }

    #[test]
    fn a_frame_is_complete_when_the_first_blank_line_begins() {
        let mut v = vdp();
        v.advance(223 * LINE_CLOCKS + 10);
        assert!(v.frame().is_none());
        v.advance(224 * LINE_CLOCKS);
        let (w, h, rgb) = v.frame().unwrap();
        assert_eq!((w, h), (256, 224));
        assert_eq!(rgb.len(), 256 * 224 * 3);
    }

    #[test]
    fn the_vertical_interrupt_comes_shortly_after_the_picture_and_needs_its_enable() {
        let mut v = vdp();
        v.advance(224 * LINE_CLOCKS + 100);
        assert_eq!(v.irq_level(), 0, "the interrupt is not due yet");
        v.advance(224 * LINE_CLOCKS + 800);
        assert!(v.vint_pending);
        assert_eq!(v.irq_level(), 6);
        assert!(v.z80_int());
        v.advance(225 * LINE_CLOCKS);
        assert!(
            !v.z80_int(),
            "the Z80 line is high for the rest of the line only"
        );
        v.acknowledge();
        assert_eq!(v.irq_level(), 0);
        assert!(!v.vint_flag());
        // enable bit off: pending but no request
        let mut v = vdp();
        v.regs[1] = 0x54;
        v.advance(224 * LINE_CLOCKS + 800);
        assert!(v.vint_pending);
        assert_eq!(v.irq_level(), 0);
    }

    #[test]
    fn the_horizontal_interrupt_counts_lines_by_register_10() {
        let mut v = vdp();
        v.regs[0] = 0x14;
        v.regs[10] = 3;
        v.hint_counter = 3;
        v.advance(0);
        assert!(!v.hint_pending);
        v.advance(3 * LINE_CLOCKS);
        assert!(v.hint_pending, "line 3 raises it after counting 3, 2, 1");
        assert_eq!(v.irq_level(), 4);
        v.acknowledge();
        v.advance(6 * LINE_CLOCKS);
        assert!(!v.hint_pending);
        v.advance(7 * LINE_CLOCKS);
        assert!(v.hint_pending, "and again after the reload: 4 lines");
    }

    #[test]
    fn register_10_at_zero_interrupts_on_every_line() {
        let mut v = vdp();
        v.regs[0] = 0x14;
        v.regs[10] = 0;
        v.advance(0);
        assert!(v.hint_pending);
        v.acknowledge();
        v.advance(LINE_CLOCKS);
        assert!(v.hint_pending);
    }

    #[test]
    fn the_vertical_interrupt_wins_over_the_horizontal_one() {
        let mut v = vdp();
        v.regs[0] = 0x14;
        v.hint_pending = true;
        v.vint_pending = true;
        assert_eq!(v.irq_level(), 6);
        v.acknowledge();
        assert_eq!(v.irq_level(), 4);
    }

    #[test]
    fn the_v_counter_jumps_at_the_end_of_the_ntsc_frame() {
        let v = Vdp::new(false);
        assert_eq!(v.read_hv(0) >> 8, 0);
        assert_eq!(v.read_hv(0xEA * LINE_CLOCKS) >> 8, 0xEA);
        assert_eq!(v.read_hv(0xEB * LINE_CLOCKS) >> 8, 0xE5);
        assert_eq!(v.read_hv(261 * LINE_CLOCKS) >> 8, 0xFF);
    }

    #[test]
    fn the_h_counter_runs_through_the_picture_in_steps_of_two_pixels() {
        let mut v = Vdp::new(false);
        v.regs[12] = 0x81;
        let at = |c: u64| (v.read_hv(c) & 0xFF) as u32;
        assert_eq!(at(780), 0);
        assert_eq!(at(780 + 16 * 159), 0x9F);
        assert!(at(0) >= 0xA5);
    }

    #[test]
    fn a_disabled_picture_still_completes_frames() {
        let mut v = Vdp::new(false);
        v.advance(300 * LINE_CLOCKS);
        assert!(v.frame().is_some());
    }
}
