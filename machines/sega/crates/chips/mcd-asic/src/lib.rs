//! The Mega-CD's graphics unit (facts G1-G12 in `docs/hardware-facts.md`, source: Mega-CD Hardware Manual section 3-7 and 3-8).
//!
//! Two functions live in the sub-CPU registers `$FF804C-$FF8066`:
//!
//! * the font converter (`$FF804C-$FF8056`): sixteen font bits and two 4-bit colours become sixteen 4-bit dots;
//! * the rotation and scaling unit, which only works while word RAM is in 2M mode: a trace vector gives, for every line of the
//!   output, a start position and a step in a stamp map; every dot is looked up in the stamp map, taken from the stamp (flipped
//!   and rotated as its map entry says) and written into an image buffer made of 8x8 cells stacked vertically.
//!
//! The unit works on the 256 KB of word RAM as a plain byte slice (sub-CPU address `$080000` is index 0). The whole operation is
//! done at the moment the trace vector register is written; what takes time is only the busy flag and the level 1 interrupt.
#![forbid(unsafe_code)]

/// Size of word RAM in 2M mode.
use serde::{Deserialize, Serialize};

pub const RAM_SIZE: usize = 0x4_0000;

/// First and last register offset (from `$FF8000`) this unit answers.
pub const FIRST_REGISTER: u32 = 0x4C;
pub const LAST_REGISTER: u32 = 0x67;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Asic {
    font_color: u16,
    font_bits: u16,
    stamp_size: u16,
    map_base: u16,
    cells_v: u16,
    buffer_start: u16,
    offset: u16,
    h_dots: u16,
    v_dots: u16,
    tvba: u16,
    /// Sub-CPU clock at which the running operation ends.
    busy_until: Option<u64>,
    /// Debugging aid: the first trace vector lines of the last operation.
    last_trace: Vec<[u16; 4]>,
}

fn word(ram: &[u8], at: usize) -> u16 {
    let at = at & (RAM_SIZE - 1) & !1;
    u16::from(ram[at]) << 8 | u16::from(ram[at + 1])
}

impl Asic {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a state's values are ones the unit can have (a state is untrusted input).
    pub fn validate(&self) -> Result<(), String> {
        if self.last_trace.len() > 1 << 16 {
            return Err("the graphics unit's trace is longer than it can be".to_string());
        }
        Ok(())
    }

    /// Whether `off` (the address minus `$FF8000`) belongs to the unit.
    pub fn owns(off: u32) -> bool {
        (FIRST_REGISTER..=LAST_REGISTER).contains(&off)
    }

    /// The four words of converted font data (fact G2): each of the sixteen source bits gives one 4-bit dot, the colour of
    /// colour 1 when the bit is set and that of colour 0 when it is clear, the top bit first.
    fn font_word(&self, k: usize) -> u16 {
        let (c1, c0) = ((self.font_color >> 4) & 0xF, self.font_color & 0xF);
        (0..4).fold(0u16, |w, i| {
            let bit = 15 - (k * 4 + i);
            let c = if self.font_bits >> bit & 1 != 0 {
                c1
            } else {
                c0
            };
            w << 4 | c
        })
    }

    /// The first lines of the last trace vector (a debugging aid).
    pub fn last_trace(&self) -> &[[u16; 4]] {
        &self.last_trace
    }

    /// Whether the unit is working at sub-CPU clock `clock`.
    pub fn busy(&self, clock: u64) -> bool {
        self.busy_until.is_some_and(|t| clock < t)
    }

    /// `true` once, when the running operation has ended: the level 1 interrupt is due.
    pub fn take_finished(&mut self, clock: u64) -> bool {
        match self.busy_until {
            Some(t) if clock >= t => {
                self.busy_until = None;
                self.v_dots = 0; // the line counter runs down to zero (fact G7)
                true
            }
            _ => false,
        }
    }

    /// A word read; `None` for a register that cannot be read.
    pub fn read_word(&self, off: u32, clock: u64) -> Option<u16> {
        Some(match off & !1 {
            0x4C => self.font_color & 0x00FF,
            0x4E => self.font_bits,
            0x50 => self.font_word(0),
            0x52 => self.font_word(1),
            0x54 => self.font_word(2),
            0x56 => self.font_word(3),
            0x58 => (u16::from(self.busy(clock)) << 15) | (self.stamp_size & 7),
            0x5A => self.map_base,
            0x5C => self.cells_v & 0x1F,
            0x5E => self.buffer_start,
            0x60 => self.offset & 0x3F,
            0x62 => self.h_dots & 0x1FF,
            0x64 => self.v_dots & 0xFF,
            _ => return None, // 0x66, the trace vector register, cannot be read
        })
    }

    /// A word write. `ram` is word RAM, `priority` the two PM bits of the memory mode register (0 off, 1 underwrite,
    /// 2 overwrite), `clock` the sub-CPU clock. Writing the trace vector register starts the operation. `false`: the register
    /// cannot be written.
    pub fn write_word(
        &mut self,
        off: u32,
        v: u16,
        ram: &mut [u8],
        priority: u8,
        clock: u64,
    ) -> bool {
        match off & !1 {
            0x4C => self.font_color = v & 0x00FF,
            0x4E => self.font_bits = v,
            0x50..=0x56 => {} // font data is read only
            0x58 => self.stamp_size = v & 7,
            0x5A => self.map_base = v,
            0x5C => self.cells_v = v & 0x1F,
            0x5E => self.buffer_start = v,
            0x60 => self.offset = v & 0x3F,
            0x62 => self.h_dots = v & 0x1FF,
            0x64 => self.v_dots = v & 0xFF,
            0x66 => {
                self.tvba = v;
                self.run(ram, priority);
                let lines = u64::from(self.v_dots);
                self.busy_until = Some(clock + lines * (u64::from(self.h_dots) + 13));
            }
            _ => return false,
        }
        true
    }

    /// One dot of the stamp map at integer coordinates `(x, y)`.
    fn sample(&self, ram: &[u8], x: u32, y: u32) -> u8 {
        let sts = self.stamp_size & 2 != 0;
        let sms = self.stamp_size & 4 != 0;
        let size: u32 = if sts { 32 } else { 16 };
        let screen: u32 = if sms { 4096 } else { 256 };
        // the map base: bits 15 down are address bits A17 down, in the units each map size allows (fact G4)
        let base = (u32::from(self.map_base) << 2)
            & match (sms, sts) {
                (false, false) => 0x3_FE00,
                (false, true) => 0x3_FF80,
                (true, false) => 0x2_0000,
                (true, true) => 0x3_8000,
            };
        let per_row = screen / size;
        let index = (y / size) * per_row + x / size;
        let entry = word(ram, base as usize + index as usize * 2);
        let number = u32::from(entry & 0x7FF) & if sts { !3 } else { !0 };
        if number == 0 {
            return 0; // stamp 0 is blank (fact G5)
        }
        let (u, v) = (x % size, y % size);
        // undo the rotation, then the horizontal flip (fact G5: the order and the sense are [H?])
        let (mut sx, sy) = match (entry >> 13) & 3 {
            0 => (u, v),
            1 => (v, size - 1 - u),
            2 => (size - 1 - u, size - 1 - v),
            _ => (size - 1 - v, u),
        };
        if entry & 0x8000 != 0 {
            sx = size - 1 - sx;
        }
        let at = number * 128 + (sx >> 3) * size * 4 + sy * 4 + ((sx & 7) >> 1);
        let byte = ram[(at as usize) & (RAM_SIZE - 1)];
        if sx & 1 == 0 { byte >> 4 } else { byte & 0xF }
    }

    /// Runs the whole operation: `v_dots` lines of `h_dots` dots (fact G6).
    fn run(&mut self, ram: &mut [u8], priority: u8) {
        let sms = self.stamp_size & 4 != 0;
        let repeat = self.stamp_size & 1 != 0;
        let trace = (u32::from(self.tvba) << 2) & 0x3_FFF8;
        let buffer = (u32::from(self.buffer_start) << 2) & 0x3_FFE0;
        let cells = u32::from(self.cells_v & 0x1F) + 1;
        let (line_off, dot_off) = (u32::from(self.offset >> 3) & 7, u32::from(self.offset) & 7);
        self.last_trace.clear();
        for v in 0..u32::from(self.v_dots & 0xFF) {
            let at = (trace + v * 8) as usize;
            // positions are 24 bit: 13 integer bits then 11 fraction bits; the start has only three fraction bits
            let mut x = u32::from(word(ram, at)) << 8;
            let mut y = u32::from(word(ram, at + 2)) << 8;
            let dx = i32::from(word(ram, at + 4) as i16);
            let dy = i32::from(word(ram, at + 6) as i16);
            if v < 4 {
                self.last_trace.push([
                    word(ram, at),
                    word(ram, at + 2),
                    word(ram, at + 4),
                    word(ram, at + 6),
                ]);
            }
            for h in 0..u32::from(self.h_dots & 0x1FF) {
                let (xi, yi) = ((x >> 11) & 0x1FFF, (y >> 11) & 0x1FFF);
                let limit = if sms { 0x1000 } else { 0x100 };
                let inside = xi < limit && yi < limit;
                let dot = if inside || repeat {
                    self.sample(ram, xi & (limit - 1), yi & (limit - 1))
                } else {
                    0
                };
                let (bx, by) = (h + dot_off, v + line_off);
                let cell = (bx >> 3) * cells + (by >> 3);
                let addr = (buffer + cell * 32 + (by & 7) * 4 + ((bx & 7) >> 1)) as usize;
                if addr < RAM_SIZE {
                    let old = ram[addr];
                    let (old_dot, shift) = if bx & 1 == 0 {
                        (old >> 4, 4)
                    } else {
                        (old & 0xF, 0)
                    };
                    let write = match priority {
                        1 => old_dot == 0,
                        2 => dot != 0,
                        _ => true,
                    };
                    if write {
                        ram[addr] = (old & !(0xF << shift)) | dot << shift;
                    }
                }
                x = x.wrapping_add(dx as u32) & 0xFF_FFFF;
                y = y.wrapping_add(dy as u32) & 0xFF_FFFF;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_word(ram: &mut [u8], at: usize, v: u16) {
        ram[at] = (v >> 8) as u8;
        ram[at + 1] = v as u8;
    }

    /// 16x16 stamp number `n` whose dot (x, y) is `(x + y) % 16` unless `f` says otherwise.
    fn put_stamp16(ram: &mut [u8], n: usize, f: impl Fn(u32, u32) -> u8) {
        for y in 0..16u32 {
            for x in 0..16u32 {
                let at = n * 128 + (x as usize >> 3) * 64 + y as usize * 4 + (x as usize & 7) / 2;
                let d = f(x, y) & 0xF;
                if x & 1 == 0 {
                    ram[at] = (ram[at] & 0x0F) | d << 4;
                } else {
                    ram[at] = (ram[at] & 0xF0) | d;
                }
            }
        }
    }

    /// The dot at (bx, by) of an image buffer at `base` with `cells` cells per column.
    fn buffer_dot(ram: &[u8], base: usize, cells: usize, bx: usize, by: usize) -> u8 {
        let at = base + ((bx >> 3) * cells + (by >> 3)) * 32 + (by & 7) * 4 + (bx & 7) / 2;
        if bx & 1 == 0 {
            ram[at] >> 4
        } else {
            ram[at] & 0xF
        }
    }

    /// A 1x1 screen of 16x16 stamps: the map at `$1000`, the trace vector at `$2000`, the buffer at `$3000`.
    fn setup(ram: &mut [u8], a: &mut Asic, hdots: u16, vdots: u16) {
        a.write_word(0x58, 0, ram, 0, 0); // 16x16 stamps, 1x1 screen, no repeat
        a.write_word(0x5A, (0x1000 >> 2) as u16, ram, 0, 0);
        a.write_word(0x5C, 1, ram, 0, 0); // 2 cells tall
        a.write_word(0x5E, (0x3000 >> 2) as u16, ram, 0, 0);
        a.write_word(0x60, 0, ram, 0, 0);
        a.write_word(0x62, hdots, ram, 0, 0);
        a.write_word(0x64, vdots, ram, 0, 0);
    }

    fn start(ram: &mut [u8], a: &mut Asic, priority: u8) {
        a.write_word(0x66, (0x2000 >> 2) as u16, ram, priority, 100);
    }

    #[test]
    fn font_bits_become_dots_of_two_colours() {
        let mut a = Asic::new();
        let mut ram = vec![0u8; RAM_SIZE];
        a.write_word(0x4C, 0x00F1, &mut ram, 0, 0);
        a.write_word(0x4E, 0b1000_0000_0000_0001, &mut ram, 0, 0);
        assert_eq!(a.read_word(0x4C, 0), Some(0x00F1));
        assert_eq!(a.read_word(0x4E, 0), Some(0x8001));
        assert_eq!(a.read_word(0x50, 0), Some(0xF111));
        assert_eq!(a.read_word(0x52, 0), Some(0x1111));
        assert_eq!(a.read_word(0x54, 0), Some(0x1111));
        assert_eq!(a.read_word(0x56, 0), Some(0x111F));
        assert!(!a.write_word(0x50, 0, &mut ram, 0, 0) || a.read_word(0x50, 0) == Some(0xF111));
    }

    #[test]
    fn registers_keep_only_their_bits_and_the_trace_vector_register_cannot_be_read() {
        let mut a = Asic::new();
        let mut ram = vec![0u8; RAM_SIZE];
        a.write_word(0x58, 0xFFFF, &mut ram, 0, 0);
        assert_eq!(a.read_word(0x58, 0), Some(7));
        a.write_word(0x5C, 0xFFFF, &mut ram, 0, 0);
        assert_eq!(a.read_word(0x5C, 0), Some(0x1F));
        a.write_word(0x60, 0xFFFF, &mut ram, 0, 0);
        assert_eq!(a.read_word(0x60, 0), Some(0x3F));
        a.write_word(0x62, 0xFFFF, &mut ram, 0, 0);
        assert_eq!(a.read_word(0x62, 0), Some(0x1FF));
        assert_eq!(a.read_word(0x66, 0), None);
        assert!(!Asic::owns(0x4A) && Asic::owns(0x4C) && Asic::owns(0x67) && !Asic::owns(0x68));
    }

    #[test]
    fn an_unrotated_unscaled_line_copies_the_stamp_row_into_the_cell_layout() {
        let mut ram = vec![0u8; RAM_SIZE];
        let mut a = Asic::new();
        put_stamp16(&mut ram, 1, |x, y| (x + y) as u8);
        put_word(&mut ram, 0x1000, 0x0001); // stamp 1 at map entry (0, 0), no flip, no rotation
        // line 0: start (0, 0), step (1.0, 0): 16 dots along the top row of the stamp
        for (i, w) in [0u16, 0, 0x0800, 0].iter().enumerate() {
            put_word(&mut ram, 0x2000 + 2 * i, *w);
        }
        setup(&mut ram, &mut a, 16, 1);
        start(&mut ram, &mut a, 0);
        for x in 0..16usize {
            assert_eq!(buffer_dot(&ram, 0x3000, 2, x, 0), (x % 16) as u8, "dot {x}");
        }
        assert_eq!(
            buffer_dot(&ram, 0x3000, 2, 0, 1),
            0,
            "only line 0 was written"
        );
        // a vertical step of one dot per dot gives the diagonal
        for (i, w) in [0u16, 0, 0x0800, 0x0800].iter().enumerate() {
            put_word(&mut ram, 0x2000 + 2 * i, *w);
        }
        start(&mut ram, &mut a, 0);
        for i in 0..16usize {
            assert_eq!(
                buffer_dot(&ram, 0x3000, 2, i, 0),
                (2 * i % 16) as u8,
                "diagonal {i}"
            );
        }
    }

    #[test]
    fn lines_come_from_consecutive_trace_vector_entries_and_scaling_halves_the_step() {
        let mut ram = vec![0u8; RAM_SIZE];
        let mut a = Asic::new();
        put_stamp16(&mut ram, 1, |x, y| (x + y) as u8);
        put_word(&mut ram, 0x1000, 0x0001);
        // two lines: line 0 from y = 0, line 1 from y = 4; half-dot steps stretch the stamp to twice its width
        for (i, w) in [0u16, 0, 0x0400, 0, 0, 4 << 3, 0x0400, 0]
            .iter()
            .enumerate()
        {
            put_word(&mut ram, 0x2000 + 2 * i, *w);
        }
        setup(&mut ram, &mut a, 8, 2);
        start(&mut ram, &mut a, 0);
        for x in 0..8usize {
            assert_eq!(buffer_dot(&ram, 0x3000, 2, x, 0), ((x / 2) % 16) as u8);
            assert_eq!(buffer_dot(&ram, 0x3000, 2, x, 1), ((x / 2 + 4) % 16) as u8);
        }
    }

    #[test]
    fn flip_and_rotation_use_the_map_entry() {
        let mut ram = vec![0u8; RAM_SIZE];
        put_stamp16(&mut ram, 1, |x, y| {
            if x == 0 && y == 0 {
                9
            } else if x == 15 && y == 0 {
                5
            } else {
                0
            }
        });
        // four map entries at (0,0) (1,0) (2,0) (3,0): plain, flipped, 180 degrees, 90 degrees
        for (i, e) in [0x0001u16, 0x8001, 0x4001, 0x2001].iter().enumerate() {
            put_word(&mut ram, 0x1000 + 2 * i, *e);
        }
        let mut a = Asic::new();
        // one line along the top row of 4 stamps = 64 dots
        for (i, w) in [0u16, 0, 0x0800, 0].iter().enumerate() {
            put_word(&mut ram, 0x2000 + 2 * i, *w);
        }
        setup(&mut ram, &mut a, 64, 1);
        a.write_word(0x5C, 7, &mut ram, 0, 0);
        start(&mut ram, &mut a, 0);
        let dot = |x| buffer_dot(&ram, 0x3000, 8, x, 0);
        assert_eq!((dot(0), dot(15)), (9, 5), "plain");
        assert_eq!((dot(16), dot(31)), (5, 9), "flipped");
        assert_eq!(
            (dot(32), dot(47)),
            (0, 0),
            "turned half way the dots are on the bottom row"
        );
        assert_eq!(
            (dot(48), dot(63)),
            (0, 9),
            "turned a quarter way they are on clockwise the first dot is at the top right"
        );
    }

    #[test]
    fn outside_the_map_is_blank_unless_it_repeats_and_blank_stamps_stay_blank() {
        let mut ram = vec![0u8; RAM_SIZE];
        let mut a = Asic::new();
        put_stamp16(&mut ram, 1, |_, _| 7);
        put_word(&mut ram, 0x1000, 0x0001);
        // start at x = 250 and run 16 dots to the right: 6 dots inside the 256-dot screen, then outside
        for (i, w) in [250u16 << 3, 0, 0x0800, 0].iter().enumerate() {
            put_word(&mut ram, 0x2000 + 2 * i, *w);
        }
        setup(&mut ram, &mut a, 16, 1);
        start(&mut ram, &mut a, 0);
        // entry (15, 0) of the map is blank (stamp 0): everything there is 0 whether inside or not
        assert_eq!(buffer_dot(&ram, 0x3000, 2, 0, 0), 0);
        // repeat: the map wraps and stamp 1 at (0, 0) shows after the edge
        a.write_word(0x58, 1, &mut ram, 0, 0);
        start(&mut ram, &mut a, 0);
        assert_eq!(buffer_dot(&ram, 0x3000, 2, 6, 0), 7);
        assert_eq!(buffer_dot(&ram, 0x3000, 2, 5, 0), 0);
        // without repeat the wrapped part stays blank
        a.write_word(0x58, 0, &mut ram, 0, 0);
        for b in &mut ram[0x3000..0x3100] {
            *b = 0;
        }
        start(&mut ram, &mut a, 0);
        assert_eq!(buffer_dot(&ram, 0x3000, 2, 6, 0), 0);
    }

    #[test]
    fn overwrite_and_underwrite_decide_which_dots_win() {
        let mut ram = vec![0u8; RAM_SIZE];
        let mut a = Asic::new();
        put_stamp16(&mut ram, 1, |x, _| if x % 2 == 0 { 3 } else { 0 });
        put_word(&mut ram, 0x1000, 0x0001);
        for (i, w) in [0u16, 0, 0x0800, 0].iter().enumerate() {
            put_word(&mut ram, 0x2000 + 2 * i, *w);
        }
        setup(&mut ram, &mut a, 4, 1);
        // the buffer holds 9 in every dot of the line
        for x in 0..4usize {
            let at = 0x3000 + (x >> 3) * 64 + (x & 7) / 2;
            ram[at] = 0x99;
        }
        start(&mut ram, &mut a, 2); // overwrite: only non-zero stamp dots replace
        assert_eq!(
            (0..4)
                .map(|x| buffer_dot(&ram, 0x3000, 2, x, 0))
                .collect::<Vec<_>>(),
            [3, 9, 3, 9]
        );
        for x in 0..4usize {
            ram[0x3000 + (x & 7) / 2] = 0x00;
        }
        ram[0x3000] = 0x50; // dot 0 holds 5, dot 1 holds 0
        start(&mut ram, &mut a, 1); // underwrite: only empty buffer dots take the stamp
        assert_eq!(buffer_dot(&ram, 0x3000, 2, 0, 0), 5);
        assert_eq!(buffer_dot(&ram, 0x3000, 2, 2, 0), 3);
    }

    #[test]
    fn the_offset_moves_the_first_dot_inside_its_cell() {
        let mut ram = vec![0u8; RAM_SIZE];
        let mut a = Asic::new();
        put_stamp16(&mut ram, 1, |_, _| 6);
        put_word(&mut ram, 0x1000, 0x0001);
        for (i, w) in [0u16, 0, 0x0800, 0].iter().enumerate() {
            put_word(&mut ram, 0x2000 + 2 * i, *w);
        }
        setup(&mut ram, &mut a, 4, 1);
        a.write_word(0x60, 2 << 3 | 3, &mut ram, 0, 0); // line 2, dot 3
        start(&mut ram, &mut a, 0);
        assert_eq!(buffer_dot(&ram, 0x3000, 2, 3, 2), 6);
        assert_eq!(buffer_dot(&ram, 0x3000, 2, 6, 2), 6);
        assert_eq!(buffer_dot(&ram, 0x3000, 2, 2, 2), 0);
        assert_eq!(buffer_dot(&ram, 0x3000, 2, 3, 1), 0);
    }

    #[test]
    fn the_operation_is_busy_for_a_while_then_finishes_once_with_the_line_counter_at_zero() {
        let mut ram = vec![0u8; RAM_SIZE];
        let mut a = Asic::new();
        setup(&mut ram, &mut a, 16, 2);
        start(&mut ram, &mut a, 0);
        assert_eq!(a.read_word(0x58, 101).unwrap() & 0x8000, 0x8000);
        assert_eq!(a.read_word(0x64, 101), Some(2));
        assert!(!a.take_finished(101));
        let end = 100 + 2 * (16 + 13);
        assert!(a.take_finished(end));
        assert!(!a.take_finished(end + 1), "once");
        assert_eq!(a.read_word(0x58, end).unwrap() & 0x8000, 0);
        assert_eq!(a.read_word(0x64, end), Some(0));
    }

    #[test]
    fn a_32_dot_stamp_uses_four_cell_columns() {
        let mut ram = vec![0u8; RAM_SIZE];
        let mut a = Asic::new();
        // stamp number 4 (the first 32x32 stamp that is not stamp 0): dot (x, 0) = x / 4 for x < 32
        for x in 0..32usize {
            let at = 4 * 128 + (x >> 3) * 128 + (x & 7) / 2;
            let d = (x / 4) as u8 & 0xF;
            if x & 1 == 0 {
                ram[at] = (ram[at] & 0x0F) | d << 4;
            } else {
                ram[at] = (ram[at] & 0xF0) | d;
            }
        }
        put_word(&mut ram, 0x1000, 0x0004);
        for (i, w) in [0u16, 0, 0x0800, 0].iter().enumerate() {
            put_word(&mut ram, 0x2000 + 2 * i, *w);
        }
        setup(&mut ram, &mut a, 32, 1);
        a.write_word(0x58, 2, &mut ram, 0, 0); // 32x32 stamps, 1x1 screen
        a.write_word(0x5A, (0x1000 >> 2) as u16, &mut ram, 0, 0);
        a.write_word(0x5C, 3, &mut ram, 0, 0);
        start(&mut ram, &mut a, 0);
        for x in 0..32usize {
            assert_eq!(
                buffer_dot(&ram, 0x3000, 4, x, 0),
                (x / 4) as u8 & 0xF,
                "dot {x}"
            );
        }
    }
}
