//! The frame: lines drawn as time passes, the interrupts and the HV counter (facts V18, V19, V25-V27).
//!
//! Lines here are V-counter lines: a line begins when the V counter changes, which is `PICTURE_OFFSET` master clocks before
//! the left edge of its picture. The positions of the events inside a line (picture start, the vertical interrupt, the
//! horizontal blanking flag) are approximations taken from what the reference emulators agree on; Phase 3 replaces them with
//! probe-measured pixel positions.

use crate::Vdp;
use crate::timing::LINE_CLOCKS;

/// A change of colour made while a line was being drawn.
#[derive(Clone, Copy, Debug)]
pub(crate) enum LineEvent {
    /// CRAM entry, new 9-bit colour.
    Cram(usize, u16),
    /// Register 7 (the backdrop colour).
    Backdrop(u8),
}

/// A line of the picture that has been worked out as palette indices and waits for its colours.
#[derive(Clone, Debug)]
pub(crate) struct PendingLine {
    vl: u32,
    width: usize,
    idx: [u8; 320],
    backdrop: [bool; 320],
    cram: [u16; 64],
    reg7: u8,
    events: Vec<(usize, LineEvent)>,
}

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
const MAX_HEIGHT: usize = 480;

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
        // nothing is due: the next line starts, or the vertical interrupt happens, later than `now`
        if let Some(line_start) = self.line_abs.checked_mul(LINE_CLOCKS)
            && now < line_start
            && self.vint_at.is_none_or(|t| now < t)
        {
            return;
        }
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
                self.gun_step(vl);
                self.hint_step(false);
            } else if vl == active {
                self.finish_frame();
                // the field flag alternates in the interlaced modes, starting with the even field when they are switched on
                let interlaced = self.regs[12] & 0x02 != 0;
                self.odd_frame = interlaced && self.was_interlaced && !self.odd_frame;
                self.was_interlaced = interlaced;
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

    /// Whether the picture is the two fields of interlace mode 2 woven into one (`set_weave`).
    fn weaving(&self) -> bool {
        self.weave && self.regs[12] & 0x06 == 0x06
    }

    /// The row of the output picture a line of the current field goes to.
    fn output_row(&self, line: usize) -> usize {
        if self.weaving() {
            2 * line + usize::from(self.odd_frame)
        } else {
            line
        }
    }

    /// Starts line `vl` of the picture: its pixels are worked out (as palette indices) with the registers and memories as they
    /// stand now, and turned into colours when the line is over, so that palette changes made during the line can show from the
    /// pixel they happen at (fact V34).
    fn draw_line(&mut self, vl: u32) {
        self.flush_line();
        let mut p = Box::new(PendingLine {
            vl,
            width: self.width() as usize,
            idx: [0; 320],
            backdrop: [false; 320],
            cram: [0; 64],
            reg7: self.regs[7] & 0x3F,
            events: Vec::new(),
        });
        self.render_line_indexed(vl, &mut p.idx, &mut p.backdrop);
        p.cram.copy_from_slice(&self.cram[..64]);
        self.line_pending = Some(p);
    }

    /// Turns the line that was started into colours, applying the palette writes that happened while it was drawn.
    fn flush_line(&mut self) {
        let Some(p) = self.line_pending.take() else {
            return;
        };
        let mut cram = p.cram;
        let mut backdrop = p.reg7;
        let mut row = [0u8; MAX_WIDTH * 3];
        let mut next = 0;
        for x in 0..p.width {
            while next < p.events.len() && p.events[next].0 <= x {
                match p.events[next].1 {
                    LineEvent::Cram(i, v) => cram[i & 0x3F] = v,
                    LineEvent::Backdrop(v) => backdrop = v & 0x3F,
                }
                next += 1;
            }
            let color = if p.backdrop[x] {
                backdrop
            } else {
                p.idx[x] & 0x3F
            };
            let rgb = crate::render::rgb_of(&cram, color, crate::render::brightness_of(p.idx[x]));
            row[3 * x..3 * x + 3].copy_from_slice(&rgb);
        }
        let at = self.output_row(p.vl as usize) * MAX_WIDTH * 3;
        if at + p.width * 3 <= self.frame_buf.len() {
            self.frame_buf[at..at + p.width * 3].copy_from_slice(&row[..p.width * 3]);
        }
    }

    /// Notes a palette or backdrop write at master clock `now` for the line being drawn, at the pixel the beam is at.
    pub(crate) fn log_line_event(&mut self, now: u64, event: LineEvent) {
        let phase = self.timing.phase(now);
        let offset = self.picture_offset();
        let per_pixel = if self.h40() { 8 } else { 10 };
        let Some(p) = &mut self.line_pending else {
            return;
        };
        if p.vl != phase.line {
            return;
        }
        let x = (phase.clock.saturating_sub(offset) / per_pixel) as usize;
        if x < p.width {
            p.events.push((x, event));
        }
    }

    /// The picture is complete: it becomes the one `frame` returns.
    fn finish_frame(&mut self) {
        self.flush_line();
        let (w, mut h) = (self.width() as usize, self.active_lines() as usize);
        if self.weaving() {
            h *= 2;
        }
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
        } else if self.ext_pending && self.regs[11] & 0x08 != 0 {
            2
        } else {
            0
        }
    }

    /// The 68000 acknowledged an interrupt: the vertical one has priority, as in `irq_level`.
    pub fn acknowledge(&mut self) {
        if self.vint_pending && self.regs[1] & 0x20 != 0 {
            self.vint_pending = false;
            self.vint_flag = false;
        } else if self.hint_pending && self.regs[0] & 0x10 != 0 {
            self.hint_pending = false;
        } else {
            self.ext_pending = false;
        }
    }

    /// A light gun that points at line `vl` raises the external interrupt and latches the counters at its column (fact I9).
    fn gun_step(&mut self, vl: u32) {
        let Some((x, y)) = self.gun else {
            return;
        };
        if self.gun_enabled && y == vl && y < self.active_lines() {
            self.ext_pending = true;
            let dot = x as usize / 2;
            // the H counter skips a stretch at the end of the line (fact V19)
            let h = if self.h40() {
                let i = dot % 210;
                if i <= 0xB6 { i } else { 0xE5 + (i - 0xB7) }
            } else {
                let i = dot % 171;
                if i <= 0x93 { i } else { 0xE9 + (i - 0x94) }
            };
            self.hv_latch = Some((((y as u16) << 8) | h as u16, self.regs[0] & 0x02 != 0, vl));
        } else if self.hv_latch.is_some_and(|(_, held, _)| !held) {
            self.hv_latch = None;
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
        if let Some((value, held, line)) = self.hv_latch {
            let current = self.timing.phase(now).line;
            if (held && self.regs[0] & 0x02 != 0) || (!held && current == line) {
                return value;
            }
        }
        let lines = u64::from(self.timing_lines());
        let in_frame = now % (lines * LINE_CLOCKS);
        let vl = (in_frame / LINE_CLOCKS) as u32;
        let clock = in_frame % LINE_CLOCKS;
        let mut v = vl;
        if v > vcounter_max(self.pal, self.v30()) {
            v = v.wrapping_sub(lines as u32);
        }
        if self.regs[12] & 0x02 != 0 {
            // interlaced: in mode 2 the counter counts double rows, and bit 0 shows bit 8 (fact V31)
            v <<= u32::from(self.regs[12] & 0x04 != 0);
            v = (v & !1) | ((v >> 8) & 1);
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
    fn a_palette_write_in_the_middle_of_a_line_shows_from_the_pixel_it_happens_at() {
        struct NoBus;
        impl crate::DmaBus for NoBus {
            fn read_word(&mut self, _a: u32) -> u16 {
                0
            }
        }
        let mut v = vdp();
        v.regs[12] = 0x81;
        v.advance(10 * LINE_CLOCKS + 100); // line 10 has been started
        v.write_control(0xC000, 10 * LINE_CLOCKS + 100, &mut NoBus);
        v.write_control(0x0000, 10 * LINE_CLOCKS + 100, &mut NoBus);
        v.fifo_stalls = false;
        // colour 0 turns white 1500 clocks into the line: pixel (1500 - 780) / 8 = 90
        v.write_data(0x0EEE, 10 * LINE_CLOCKS + 1500);
        v.advance(300 * LINE_CLOCKS);
        let (w, _, rgb) = v.frame().unwrap();
        let at = |x: usize| rgb[(10 * w as usize + x) * 3];
        assert_eq!((at(5), at(89)), (0, 0), "black before the write");
        assert_eq!((at(90), at(300)), (238, 238), "white from it");
        assert_eq!(
            rgb[(11 * w as usize + 5) * 3],
            238,
            "the next line starts with the new colour"
        );
    }

    #[test]
    fn a_light_gun_latches_the_counters_and_raises_the_external_interrupt_on_its_line() {
        let mut v = vdp();
        v.regs[0] = 0x06;
        v.regs[11] = 0x08;
        v.regs[12] = 0x81;
        v.set_gun(Some((90, 120)), true);
        v.advance(119 * LINE_CLOCKS + 1000);
        assert_eq!(v.irq_level(), 0, "not yet");
        v.advance(121 * LINE_CLOCKS);
        assert_eq!(v.irq_level(), 2);
        assert_eq!(v.read_hv(130 * LINE_CLOCKS), 0x782D, "the latch holds");
        v.acknowledge();
        assert_eq!(v.irq_level(), 0);
        v.regs[0] = 0x04;
        assert_ne!(
            v.read_hv(130 * LINE_CLOCKS),
            0x782D,
            "released with the latch bit"
        );
        // the port's HL pin off: nothing happens
        let mut v = vdp();
        v.regs[11] = 0x08;
        v.set_gun(Some((90, 120)), false);
        v.advance(130 * LINE_CLOCKS);
        assert_eq!(v.irq_level(), 0);
    }

    #[test]
    fn interlace_mode_2_can_show_both_fields_woven() {
        let mut v = vdp();
        v.regs[12] = 0x87;
        v.regs[16] = 0x01;
        v.advance(2 * 262 * LINE_CLOCKS);
        assert_eq!(v.frame().unwrap().1, 224, "one field by default");
        v.set_weave(true);
        v.advance(3 * 262 * LINE_CLOCKS);
        let (w, h, rgb) = v.frame().unwrap();
        assert_eq!((w, h, rgb.len()), (320, 448, 320 * 448 * 3));
    }

    #[test]
    fn a_disabled_picture_still_completes_frames() {
        let mut v = Vdp::new(false);
        v.advance(300 * LINE_CLOCKS);
        assert!(v.frame().is_some());
    }
}
