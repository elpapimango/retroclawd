//! Where the VDP is in the frame. The frame is `lines x 3420` master clocks (facts C7-C9); the machine starts at the
//! beginning of a frame at master clock 0. Line numbers here are the display line counter: line 0 is the first line of
//! the picture. Exact flag positions come in Task 11 and the pixel-accurate model in Phase 3.

/// Master clocks per scanline (fact C7).
pub const LINE_CLOCKS: u64 = 3420;

/// What a point in time looks like to the VDP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phase {
    /// Whole frames since power on.
    pub frame: u64,
    /// The line within the frame, 0 = first picture line.
    pub line: u32,
    /// Master clocks since the line began.
    pub clock: u64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Timing {
    lines: u32,
}

impl Timing {
    pub(crate) fn new(pal: bool) -> Self {
        Self {
            lines: if pal { 313 } else { 262 },
        }
    }

    pub(crate) fn is_pal(&self) -> bool {
        self.lines == 313
    }

    pub(crate) fn frame_clocks(&self) -> u64 {
        u64::from(self.lines) * LINE_CLOCKS
    }

    pub(crate) fn phase(&self, now: u64) -> Phase {
        let frame_clocks = self.frame_clocks();
        let in_frame = now % frame_clocks;
        Phase {
            frame: now / frame_clocks,
            line: (in_frame / LINE_CLOCKS) as u32,
            clock: in_frame % LINE_CLOCKS,
        }
    }
}

impl crate::Vdp {
    /// The position in the frame at master clock `now`.
    pub fn phase(&self, now: u64) -> Phase {
        self.timing.phase(now)
    }

    /// Whether `line` is outside the picture (vertical blanking), or the display is off.
    pub(crate) fn line_is_blank(&self, line: u32) -> bool {
        line >= self.active_lines() || !self.display_enabled()
    }

    /// The master clock at which `units` DMA units, done at `active_rate` per line while the picture is drawn and
    /// `blank_rate` per line otherwise, are finished when they start at `start`.
    pub(crate) fn dma_end(&self, start: u64, units: u64, active_rate: u64, blank_rate: u64) -> u64 {
        let mut now = start;
        let mut left = units;
        // The loop is bounded: a full-length transfer needs a few thousand lines at the slowest rate.
        for _ in 0..200_000 {
            let p = self.timing.phase(now);
            let rate = if self.line_is_blank(p.line) {
                blank_rate
            } else {
                active_rate
            }
            .max(1);
            let to_end = LINE_CLOCKS - p.clock;
            let can = rate * to_end / LINE_CLOCKS;
            if can >= left {
                return now + (left * LINE_CLOCKS).div_ceil(rate);
            }
            left -= can;
            now += to_end;
        }
        now
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vdp;

    #[test]
    fn the_frame_has_the_documented_number_of_lines() {
        assert_eq!(Timing::new(false).frame_clocks(), 896_040);
        assert_eq!(Timing::new(true).frame_clocks(), 1_070_460);
    }

    #[test]
    fn phase_splits_the_master_clock_into_frame_line_and_clock() {
        let t = Timing::new(false);
        assert_eq!(
            t.phase(0),
            Phase {
                frame: 0,
                line: 0,
                clock: 0
            }
        );
        assert_eq!(
            t.phase(896_040 + 3420 * 5 + 7),
            Phase {
                frame: 1,
                line: 5,
                clock: 7
            }
        );
    }

    #[test]
    fn dma_takes_longer_while_the_picture_is_drawn() {
        let mut v = Vdp::new(false);
        v.regs[1] = 0x40;
        // 18 units a line while drawing: 36 units need exactly two lines
        assert_eq!(v.dma_end(0, 36, 18, 204), 2 * LINE_CLOCKS);
        // in vertical blanking (line 230) 204 a line: 102 units take half a line
        let start = 230 * LINE_CLOCKS;
        assert_eq!(v.dma_end(start, 102, 18, 204), start + LINE_CLOCKS / 2);
    }

    #[test]
    fn a_disabled_display_uses_the_blanking_rate_everywhere() {
        let v = Vdp::new(false);
        assert_eq!(v.dma_end(0, 204, 18, 204), LINE_CLOCKS);
    }
}
