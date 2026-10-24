//! The data port's FIFO: four entries that the VDP empties in its external access slots. While the picture is drawn there
//! are only a few slots a line (the rest go to the picture), so a 68000 that writes faster waits for a free entry. Fact V28,
//! probes `p29_fifo_rate`, `p31_fifo_*`, `p33_slots_*`.

use crate::Vdp;
use crate::timing::LINE_CLOCKS;

/// Access slots of a line while the picture is drawn, as master clocks since the start of the V-counter line, H40.
const SLOTS_H40: [u16; 18] = [
    312, 792, 896, 1060, 1276, 1428, 1556, 1780, 1924, 2052, 2312, 2420, 2592, 2812, 2948, 3124,
    3332, 3380,
];
/// The same in H32.
const SLOTS_H32: [u16; 16] = [
    204, 472, 828, 940, 1120, 1436, 1580, 1748, 2008, 2220, 2364, 2704, 2836, 3036, 3332, 3348,
];

impl Vdp {
    fn slots(&self) -> &'static [u16] {
        if self.h40() { &SLOTS_H40 } else { &SLOTS_H32 }
    }

    /// The first access slot after `t`, `skip` slots further on (the table repeats every line).
    fn slot_after(&self, t: u64, skip: usize) -> u64 {
        let table = self.slots();
        let base = t - t % LINE_CLOCKS;
        let rel = t - base;
        let first = table
            .iter()
            .position(|&s| u64::from(s) > rel)
            .unwrap_or(table.len());
        let i = first + skip;
        base + (i / table.len()) as u64 * LINE_CLOCKS + u64::from(table[i % table.len()])
    }

    /// A word written to the data port by the 68000 at `now`: the master clock at which it is accepted, later than `now`
    /// when the FIFO is full. Writes while the picture is not drawn are not held up.
    pub(crate) fn fifo_accept(&mut self, now: u64) -> u64 {
        if self.line_is_blank(self.timing.phase(now).line) {
            return now;
        }
        let idx = self.fifo_idx & 3;
        let newest = self.fifo_done[(idx + 3) & 3];
        let oldest = self.fifo_done[idx];
        let mut accepted = now;
        let mut reference = now;
        if now < newest {
            if now < oldest {
                accepted = oldest;
            }
            reference = newest;
        }
        // A word of VRAM takes two slots (the memory is byte-wide), a word of CRAM or VSRAM one.
        let skip = usize::from(self.code & 0x06 == 0);
        self.fifo_done[idx] = self.slot_after(reference, skip);
        accepted
    }

    /// Bits 9 (FIFO empty) and 8 (FIFO full) of the status register at master clock `now`.
    pub(crate) fn fifo_status(&self, now: u64) -> u16 {
        let idx = self.fifo_idx & 3;
        let mut bits = 0;
        if now >= self.fifo_done[(idx + 3) & 3] {
            bits |= 0x200;
        } else if now < self.fifo_done[idx] {
            bits |= 0x100;
        }
        bits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A display-on VDP in H40 with the data port writing VRAM (code 1) or CRAM (code 3).
    fn vdp(code: u8) -> Vdp {
        let mut v = Vdp::new(false);
        v.regs[1] = 0x44;
        v.regs[12] = 0x81;
        v.regs[15] = 2;
        v.code = code;
        v
    }

    /// 20 lines into the picture.
    const T0: u64 = 20 * LINE_CLOCKS + 100;

    #[test]
    fn four_writes_fit_in_the_fifo_and_the_fifth_waits_for_a_slot() {
        let mut v = vdp(3);
        for i in 0..4 {
            assert_eq!(
                v.write_data(0x222, T0 + i),
                T0 + i,
                "write {i} is taken at once"
            );
        }
        let fifth = v.write_data(0x222, T0 + 4);
        assert!(fifth > T0 + 4, "the fifth write waits");
        assert!(
            fifth < T0 + LINE_CLOCKS,
            "for a slot of the same or the next line"
        );
    }

    #[test]
    fn writes_while_the_picture_is_not_drawn_are_not_held_up() {
        let mut v = vdp(3);
        let blank = 230 * LINE_CLOCKS;
        for i in 0..40 {
            assert_eq!(v.write_data(0x222, blank + i), blank + i);
        }
        v.regs[1] = 0x04; // display off
        for i in 0..40 {
            assert_eq!(v.write_data(0x222, T0 + i), T0 + i);
        }
    }

    #[test]
    fn a_word_of_vram_takes_twice_the_slots_of_a_word_of_cram() {
        let finish = |code: u8| {
            let mut v = vdp(code);
            let mut t = T0;
            for _ in 0..100 {
                t = v.write_data(0x222, t);
            }
            t
        };
        let (vram, cram) = (finish(1), finish(3));
        assert!(
            vram - T0 > (cram - T0) * 3 / 2,
            "100 VRAM words took {} clocks, CRAM words {}",
            vram - T0,
            cram - T0
        );
    }

    #[test]
    fn the_status_register_shows_a_full_and_then_an_empty_fifo() {
        let mut v = vdp(3);
        assert_eq!(v.fifo_status(T0), 0x200, "empty at first");
        for i in 0..4 {
            v.write_data(0x222, T0 + i);
        }
        assert_eq!(v.fifo_status(T0 + 4), 0x100, "full after four writes");
        let drained = v.fifo_done[3];
        assert_eq!(
            v.fifo_status(drained),
            0x200,
            "empty once the last entry is processed"
        );
    }
}
