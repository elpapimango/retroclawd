//! Blitter behaviour on a plain word memory. Each case is worked out by hand from the register map and, for the skew, its
//! two flags and the right-to-left run, from what Hatari's probes P33/P37/P38/P39 showed (the machine's own tests compare
//! the whole dumps).
use blitter::{BlitBus, Blitter};

#[derive(Clone)]
struct Mem(Vec<u16>);

impl Mem {
    fn new() -> Self {
        Mem(vec![0; 1 << 15])
    }
}

impl BlitBus for Mem {
    fn read_word(&mut self, addr: u32) -> u16 {
        self.0[(addr as usize >> 1) % self.0.len()]
    }
    fn write_word(&mut self, addr: u32, value: u16) {
        let n = self.0.len();
        self.0[(addr as usize >> 1) % n] = value;
    }
}

fn set_word(b: &mut Blitter, off: usize, v: u16) {
    b.write(off, true, v);
}

fn set_long(b: &mut Blitter, off: usize, v: u32) {
    b.write(off, true, (v >> 16) as u16);
    b.write(off + 2, true, v as u16);
}

#[derive(Clone, Copy)]
struct Setup {
    hop: u8,
    op: u8,
    skew: u8,
    src_x: i16,
    src_y: i16,
    dst_x: i16,
    dst_y: i16,
    masks: [u16; 3],
    x: u16,
    y: u16,
    src: u32,
    dst: u32,
    control: u8,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            hop: 2,
            op: 3,
            skew: 0,
            src_x: 2,
            src_y: 2,
            dst_x: 2,
            dst_y: 2,
            masks: [0xFFFF; 3],
            x: 1,
            y: 1,
            src: 0x1000,
            dst: 0x2000,
            control: 0,
        }
    }
}

fn program(b: &mut Blitter, s: Setup) -> bool {
    set_word(b, 0x20, s.src_x as u16);
    set_word(b, 0x22, s.src_y as u16);
    set_long(b, 0x24, s.src);
    set_word(b, 0x28, s.masks[0]);
    set_word(b, 0x2A, s.masks[1]);
    set_word(b, 0x2C, s.masks[2]);
    set_word(b, 0x2E, s.dst_x as u16);
    set_word(b, 0x30, s.dst_y as u16);
    set_long(b, 0x32, s.dst);
    set_word(b, 0x36, s.x);
    set_word(b, 0x38, s.y);
    b.write(0x3A, false, u16::from(s.hop));
    b.write(0x3B, false, u16::from(s.op));
    b.write(0x3D, false, u16::from(s.skew));
    b.write(0x3C, false, u16::from(s.control | 0x80))
}

/// Programs the registers, starts the blit and runs it to its end.
fn blit(b: &mut Blitter, mem: &mut Mem, s: Setup) {
    assert!(program(b, s), "the busy bit starts the blit");
    assert!(b.busy());
    b.run(mem, 1 << 20);
    assert!(!b.busy());
}

fn put(m: &mut Mem, addr: u32, words: &[u16]) {
    for (i, w) in words.iter().enumerate() {
        m.write_word(addr + 2 * i as u32, *w);
    }
}

fn get(m: &mut Mem, addr: u32, n: usize) -> Vec<u16> {
    (0..n).map(|i| m.read_word(addr + 2 * i as u32)).collect()
}

#[test]
fn the_counts_run_down_and_the_control_register_keeps_the_line() {
    let mut b = Blitter::new();
    let mut m = Mem::new();
    put(&mut m, 0x1000, &[0xAAAA, 0xBBBB]);
    blit(
        &mut b,
        &mut m,
        Setup {
            x: 2,
            y: 3,
            ..Setup::default()
        },
    );
    assert_eq!(b.read(0x36, true), 2, "the x count reads back as written");
    assert_eq!(b.read(0x38, true), 0, "the y count runs down to zero");
    // busy and HOG are clear, the halftone line advanced once a line
    assert_eq!(b.read(0x3C, false), 3);
}

#[test]
fn a_copy_moves_the_words_and_leaves_the_addresses_where_the_rules_put_them() {
    let mut b = Blitter::new();
    let mut m = Mem::new();
    put(&mut m, 0x1000, &[1, 2, 3, 0, 0, 4, 5, 6]);
    blit(
        &mut b,
        &mut m,
        Setup {
            x: 3,
            y: 2,
            src_y: 6,
            dst_y: 6,
            ..Setup::default()
        },
    );
    assert_eq!(get(&mut m, 0x2000, 3), [1, 2, 3]);
    // the next line starts one y increment after the last word of the line, with no x increment
    assert_eq!(get(&mut m, 0x200A, 3), [4, 5, 6]);
    assert_eq!(b.read(0x24, true), 0);
    assert_eq!(b.read(0x26, true), 0x1014);
    assert_eq!(b.read(0x32, true), 0);
    assert_eq!(b.read(0x34, true), 0x2014);
}

#[test]
fn every_logical_operation_combines_source_and_destination() {
    type Op = fn(u16, u16) -> u16;
    let table: [Op; 16] = [
        |_, _| 0,
        |s, d| s & d,
        |s, d| s & !d,
        |s, _| s,
        |s, d| !s & d,
        |_, d| d,
        |s, d| s ^ d,
        |s, d| s | d,
        |s, d| !(s | d),
        |s, d| !(s ^ d),
        |_, d| !d,
        |s, d| s | !d,
        |s, _| !s,
        |s, d| !s | d,
        |s, d| !(s & d),
        |_, _| 0xFFFF,
    ];
    for (op, f) in table.iter().enumerate() {
        let mut b = Blitter::new();
        let mut m = Mem::new();
        put(&mut m, 0x1000, &[0x0FF0]);
        put(&mut m, 0x2000, &[0x3C3C]);
        blit(
            &mut b,
            &mut m,
            Setup {
                op: op as u8,
                ..Setup::default()
            },
        );
        assert_eq!(get(&mut m, 0x2000, 1)[0], f(0x0FF0, 0x3C3C), "op {op}");
    }
}

#[test]
fn hop_picks_the_halftone_the_source_or_both() {
    for (hop, want) in [(0, 0xFFFF), (1, 0x8001), (2, 0x0FF0), (3, 0x0FF0 & 0x8001)] {
        let mut b = Blitter::new();
        let mut m = Mem::new();
        put(&mut m, 0x1000, &[0x0FF0]);
        b.write(0x00, true, 0x8001);
        blit(
            &mut b,
            &mut m,
            Setup {
                hop,
                ..Setup::default()
            },
        );
        assert_eq!(get(&mut m, 0x2000, 1)[0], want, "hop {hop}");
    }
}

#[test]
fn the_halftone_line_steps_with_the_destination_y_direction() {
    let mut b = Blitter::new();
    let mut m = Mem::new();
    for i in 0..16u16 {
        b.write(2 * usize::from(i), true, 0x1000 + i);
    }
    blit(
        &mut b,
        &mut m,
        Setup {
            hop: 1,
            y: 3,
            control: 14,
            ..Setup::default()
        },
    );
    assert_eq!(get(&mut m, 0x2000, 3), [0x100E, 0x100F, 0x1000]);
    assert_eq!(b.read(0x3C, false) & 15, 1);
    // a negative y increment steps the line down
    let mut m = Mem::new();
    blit(
        &mut b,
        &mut m,
        Setup {
            hop: 1,
            y: 3,
            control: 1,
            dst: 0x2010,
            dst_y: -2,
            ..Setup::default()
        },
    );
    assert_eq!(get(&mut m, 0x200C, 3), [0x100F, 0x1000, 0x1001]);
}

#[test]
fn smudge_takes_the_halftone_line_from_the_source() {
    let mut b = Blitter::new();
    let mut m = Mem::new();
    for i in 0..16u16 {
        b.write(2 * usize::from(i), true, 0x2000 + i);
    }
    put(&mut m, 0x1000, &[0x1237]);
    blit(
        &mut b,
        &mut m,
        Setup {
            hop: 1,
            control: 0x20,
            ..Setup::default()
        },
    );
    assert_eq!(get(&mut m, 0x2000, 1)[0], 0x2007);
    assert_eq!(b.read(0x3C, false), 0x21, "smudge stays, the line advanced");
}

#[test]
fn end_masks_keep_the_destination_outside_them() {
    let mut b = Blitter::new();
    let mut m = Mem::new();
    put(&mut m, 0x1000, &[0xFFFF, 0xFFFF, 0xFFFF]);
    blit(
        &mut b,
        &mut m,
        Setup {
            x: 3,
            masks: [0x00FF, 0xAAAA, 0xFF00],
            ..Setup::default()
        },
    );
    assert_eq!(get(&mut m, 0x2000, 3), [0x00FF, 0xAAAA, 0xFF00]);
    // a line of one word takes the first mask only
    let mut m = Mem::new();
    put(&mut m, 0x1000, &[0xFFFF]);
    blit(
        &mut b,
        &mut m,
        Setup {
            masks: [0x0F0F, 0, 0x00F0],
            ..Setup::default()
        },
    );
    assert_eq!(get(&mut m, 0x2000, 1)[0], 0x0F0F);
}

#[test]
fn the_skew_shifts_the_source_across_the_word_boundary() {
    let mut b = Blitter::new();
    let mut m = Mem::new();
    put(&mut m, 0x1000, &[0x1234, 0x5678]);
    // the shifter starts holding zero: the first word is 0x00001234 >> 4, the second 0x12345678 >> 4
    blit(
        &mut b,
        &mut m,
        Setup {
            skew: 4,
            x: 2,
            ..Setup::default()
        },
    );
    assert_eq!(get(&mut m, 0x2000, 2), [0x0123, 0x4567]);
}

#[test]
fn the_shifter_keeps_its_words_from_one_blit_to_the_next() {
    let mut b = Blitter::new();
    let mut m = Mem::new();
    put(&mut m, 0x1000, &[0x1234]);
    put(&mut m, 0x1100, &[0x5678]);
    blit(&mut b, &mut m, Setup::default());
    blit(
        &mut b,
        &mut m,
        Setup {
            src: 0x1100,
            dst: 0x2100,
            skew: 4,
            ..Setup::default()
        },
    );
    assert_eq!(get(&mut m, 0x2100, 1)[0], 0x4567);
}

#[test]
fn fxsr_reads_one_word_before_the_line_and_nfsr_skips_the_last_read() {
    // FXSR: the word read first is the older one for the first destination word; skew 0 gives the second source word.
    let mut b = Blitter::new();
    let mut m = Mem::new();
    put(&mut m, 0x1000, &[10, 20, 30, 40]);
    blit(
        &mut b,
        &mut m,
        Setup {
            skew: 0x80,
            x: 3,
            ..Setup::default()
        },
    );
    assert_eq!(get(&mut m, 0x2000, 3), [20, 30, 40]);
    // NFSR: two reads for three words; the last word repeats the one before.
    let mut b = Blitter::new();
    let mut m = Mem::new();
    put(&mut m, 0x1000, &[10, 20, 30, 40]);
    blit(
        &mut b,
        &mut m,
        Setup {
            skew: 0x40,
            x: 3,
            ..Setup::default()
        },
    );
    assert_eq!(get(&mut m, 0x2000, 3), [10, 20, 20]);
}

#[test]
fn a_negative_x_increment_runs_right_to_left_with_the_newest_word_above() {
    let mut b = Blitter::new();
    let mut m = Mem::new();
    put(&mut m, 0x1000, &[1, 2, 3]);
    blit(
        &mut b,
        &mut m,
        Setup {
            src: 0x1004,
            dst: 0x2004,
            src_x: -2,
            dst_x: -2,
            x: 3,
            ..Setup::default()
        },
    );
    // skew 0 right to left: each destination word gets the word read before it, the first the shifter's leftover (zero)
    assert_eq!(get(&mut m, 0x2000, 3), [2, 3, 0]);
}

#[test]
fn a_count_of_zero_is_65536_and_a_run_can_be_split() {
    let mut b = Blitter::new();
    let mut m = Mem::new();
    set_word(&mut b, 0x2E, 0);
    set_word(&mut b, 0x30, 0);
    set_long(&mut b, 0x32, 0x2000);
    set_word(&mut b, 0x36, 0);
    set_word(&mut b, 0x38, 2);
    b.write(0x3A, false, 0);
    b.write(0x3B, false, 15);
    assert!(b.write(0x3C, false, 0x80));
    assert_eq!(b.run(&mut m, 1000), 1000);
    assert!(b.busy());
    assert_eq!(b.run(&mut m, 1 << 20), 2 * 65536 - 1000);
    assert!(!b.busy());
    assert_eq!(b.read(0x38, true), 0);
}

#[test]
fn a_blit_split_anywhere_gives_the_same_memory() {
    let source: Vec<u16> = (1..=12).collect();
    let setup = Setup {
        x: 3,
        y: 4,
        skew: 0xC5,
        src_y: 4,
        dst_y: 4,
        masks: [0x7FFF, 0xFFFF, 0xFFFE],
        ..Setup::default()
    };
    let mut whole = Blitter::new();
    let mut mem_whole = Mem::new();
    put(&mut mem_whole, 0x1000, &source);
    blit(&mut whole, &mut mem_whole, setup);
    for cut in 1..12 {
        let mut b = Blitter::new();
        let mut m = Mem::new();
        put(&mut m, 0x1000, &source);
        assert!(program(&mut b, setup));
        b.run(&mut m, cut);
        let mut resumed = b.clone();
        let mut m2 = m.clone();
        resumed.run(&mut m2, 1 << 20);
        b.run(&mut m, 1 << 20);
        assert_eq!(b, resumed, "cut {cut}");
        assert_eq!(m.0, m2.0, "cut {cut}");
        assert_eq!(m.0, mem_whole.0, "cut {cut}");
        assert_eq!(b, whole, "cut {cut}");
    }
}

#[test]
fn a_write_to_the_control_register_while_busy_changes_nothing() {
    let mut b = Blitter::new();
    let mut m = Mem::new();
    assert!(program(
        &mut b,
        Setup {
            x: 4,
            ..Setup::default()
        }
    ));
    b.run(&mut m, 1);
    assert!(!b.write(0x3C, false, 0x8F));
    assert_eq!(b.read(0x3C, false), 0x80);
}

#[test]
fn a_restart_with_the_y_count_left_at_zero_moves_nothing_but_a_written_zero_means_65536() {
    let mut b = Blitter::new();
    let mut m = Mem::new();
    put(&mut m, 0x2000, &[0x1234]);
    blit(
        &mut b,
        &mut m,
        Setup {
            op: 0,
            ..Setup::default()
        },
    );
    assert_eq!(get(&mut m, 0x2000, 1)[0], 0);
    // EmuTOS restarts a blit with a TAS once it has ended: the registers are as the blit left them
    b.write(0x3B, false, 15);
    assert!(!b.write(0x3C, false, 0x80));
    assert!(!b.busy());
    assert_eq!(get(&mut m, 0x2000, 1)[0], 0, "nothing moved");
    assert_eq!(
        b.read(0x3C, false),
        0,
        "the control register holds what was written, busy clear"
    );
    // a Y count of zero that the program writes is 65536 lines
    set_word(&mut b, 0x38, 0);
    set_word(&mut b, 0x36, 1);
    assert!(b.write(0x3C, false, 0x80));
    assert_eq!(b.run(&mut m, 1 << 20), 65536);
    assert_eq!(get(&mut m, 0x2000, 1)[0], 0xFFFF);
}
