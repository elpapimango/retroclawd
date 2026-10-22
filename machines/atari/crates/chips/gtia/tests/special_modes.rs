use gtia::Gtia;

// AHR 2026-01-02 §§4.8, 6.6, 6.8-6.10 and Table 17.
// https://www.virtualdub.org/downloads/Altirra%20Hardware%20Reference%20Manual.pdf
// These tests feed AN samples, not predecoded GTIA pixels. No diagnostic
// binary or emulator implementation is used as a rendering oracle.
fn setup(mode: u8) -> Gtia {
    let mut g = Gtia::new(false);
    for i in 0..4 {
        g.write(18 + i, 0x12 + 0x20 * i);
        g.write(22 + i, 0x84 + 0x10 * i);
    }
    g.write(26, 0x02);
    g.write(27, mode);
    for _ in 0..228 {
        g.tick_color_clock([0; 2], false);
    }
    g.begin_scanline();
    g
}

fn pair(bits: u8) -> [u8; 2] {
    [5 + ((bits >> 1) & 1), 5 + (bits & 1)]
}

fn advance(g: &mut Gtia, hpos: u16) {
    for _ in 0..hpos + 2 {
        g.tick_color_clock([0; 2], true);
    }
}

#[test]
fn mode10_decodes_all_sixteen_nibbles_without_virtual_player_collisions() {
    for pixel in 0..16 {
        let mut g = setup(0x81);
        for i in 0..4 {
            g.write(i, 0x81);
            g.write(13 + i, 0x80);
        }
        g.write(4, 0x81);
        g.write(17, 2);
        advance(&mut g, 0x80);
        g.tick_color_clock_with_next(pair(pixel >> 2), pair(pixel & 3), true);
        g.tick_color_clock_with_next(pair(pixel & 3), pair(0), true);
        let expected = if pixel & 4 != 0 { 1 << (pixel & 3) } else { 0 };
        for i in 0..4 {
            assert_eq!(g.read(4 + i), expected, "pixel {pixel}, P{i}PF");
            assert_eq!(g.read(12 + i), 15 & !(1 << i), "real P/M only");
        }
        assert_eq!(g.read(0), expected, "pixel {pixel}, M0PF");
        assert_eq!(g.read(8), 15);

        let mut g = setup(0x80);
        g.write(4, 0x81);
        g.write(17, 2);
        advance(&mut g, 0x80);
        g.tick_color_clock_with_next(pair(pixel >> 2), pair(pixel & 3), true);
        g.tick_color_clock_with_next(pair(pixel & 3), pair(0), true);
        assert_eq!(g.read(8), 0, "pixel {pixel} is not a real player");
    }
}

#[test]
fn nibble_phase_fixed_to_blank_mode10_delays_exactly_one_clock() {
    for mode in [0x40, 0x80, 0xc0] {
        let mut g = setup(mode);
        advance(&mut g, 0x70);
        // AN pairs 01,10 form $6; 11,01 form $D. An odd HSCROL changes
        // which pairs meet here, not the nibble clock or PM coordinate.
        let samples = [pair(1), pair(2), pair(3), pair(1), pair(0)];
        let out: Vec<_> = samples
            .windows(2)
            .map(|s| g.tick_color_clock_with_next(s[0], s[1], true))
            .collect();
        let expected = match mode {
            0x40 => [0x06, 0x06, 0x0f, 0x0f],
            0x80 => [0x12, 0xa4, 0xa4, 0x94],
            _ => [0x62, 0x62, 0xd2, 0xd2],
        };
        assert_eq!(out, expected.map(|c| [c; 2]));
    }
}

#[test]
fn special_colors_preserve_mode9_low_luminance_and_mode11_zero_rule() {
    for (mode, fifth_color, expected) in [
        (0x40, None, 0xa7),
        (0x40, Some(0x54), 0x57),
        (0xc0, None, 0xb6),
        (0xc0, Some(0x54), 0x74),
    ] {
        let mut g = setup(mode | if fifth_color.is_some() { 0x10 } else { 0 });
        g.write(26, 0xa6);
        if let Some(color) = fifth_color {
            g.write(25, color);
            g.write(4, 0x80);
            g.write(17, 2);
        }
        advance(&mut g, 0x80);
        assert_eq!(
            g.tick_color_clock_with_next(pair(0), pair(3), true),
            [expected; 2]
        );
    }
    let mut g = setup(0xd0);
    g.write(4, 0x80);
    g.write(17, 2);
    g.write(25, 0x54);
    advance(&mut g, 0x80);
    assert_eq!(
        g.tick_color_clock_with_next(pair(0), pair(0), true),
        [0x50; 2]
    );
}

#[test]
fn mode10_lowres_background_second_half_mutes_pf_not_player_codes() {
    for (next, expected) in [([0; 2], 0x02), ([1; 2], 0x84)] {
        let mut g = setup(0x80);
        advance(&mut g, 0x80);
        g.tick_color_clock_with_next([2; 2], next, true);
        assert_eq!(
            g.tick_color_clock_with_next(next, [0; 2], true),
            [expected; 2]
        );
    }
    let mut g = setup(0x80);
    advance(&mut g, 0x80);
    g.tick_color_clock_with_next([0; 2], [0; 2], true);
    assert_eq!(g.tick_color_clock([0; 2], true), [0x12; 2]);
}

#[test]
fn special_pixels_enter_priority_without_becoming_real_collision_objects() {
    // Mode 10 border acts as P0: real P1 is hidden unless multicolor
    // combines it. Modes 9/11 drop their nibble when a real P/M is present.
    for (mode, expected) in [(0x80, 0x12), (0xa0, 0x32), (0x40, 0x32), (0xc0, 0x32)] {
        let mut g = setup(mode);
        g.write(1, 0x81);
        g.write(14, 0x80);
        advance(&mut g, 0x80);
        g.tick_color_clock_with_next(pair(0), pair(0), true);
        assert_eq!(
            g.tick_color_clock_with_next(pair(0), pair(0), true),
            [expected; 2]
        );
        assert_eq!(g.read(13), 0);
    }
}

#[test]
fn special_write_pipeline_and_hblank_latch_produce_pseudo_e_until_next_command() {
    // AHR §6.10 uses a write at HPOS $80; normal PRIOR bits change at
    // $82, mode 10 at $83 and mode 9/11 at $84.
    for (mode, delay, expected) in [(0x40, 4, 0x0a), (0x80, 3, 0x02), (0xc0, 4, 0xa2)] {
        let mut g = setup(0);
        advance(&mut g, 0x80);
        g.write(27, mode);
        for _ in 0..delay {
            assert_eq!(g.tick_color_clock(pair(2), true), [0xa4, 0xa4]);
        }
        assert_eq!(g.tick_color_clock(pair(2), true), [expected; 2]);
    }
    let mut g = setup(0x80);
    advance(&mut g, 0x70);
    g.write(27, 0);
    for _ in 0..4 {
        g.tick_color_clock(pair(0), true);
    }
    // Actual hires AN encodings %100-%111 now select PF0-PF3, not
    // PF2 + luminance. A new scanline alone is not an AN mode command.
    for bits in 0..4 {
        assert_eq!(
            g.tick_color_clock(pair(bits), true),
            [0x84 + bits * 0x10; 2]
        );
    }
    g.begin_scanline();
    assert_eq!(g.tick_color_clock(pair(0), true), [0x84; 2]);
    g.set_hires(true);
    assert_eq!(g.tick_color_clock(pair(0), true), [0xa4; 2]);

    for mode in [0x40, 0xc0] {
        let mut g = setup(mode);
        advance(&mut g, 0x80);
        g.write(27, 0);
        for _ in 0..3 {
            g.tick_color_clock(pair(0), true);
        }
        assert_eq!(g.tick_color_clock(pair(0), true), [0x02; 2]);
        assert_eq!(g.tick_color_clock(pair(0), true), [0x84; 2]);
    }

    // A command still presented when PRIOR becomes effective restores hires.
    // A command ending one clock earlier cannot restore it later by itself.
    for (command_clock, expected) in [(3, 0x84), (4, 0xa4)] {
        let mut g = setup(0x80);
        advance(&mut g, 0x70);
        g.write(27, 0);
        for clock in 0..4 {
            if clock == command_clock {
                g.set_hires(true);
            }
            g.tick_color_clock(pair(0), true);
        }
        if command_clock == 4 {
            g.set_hires(true);
        }
        assert_eq!(g.tick_color_clock(pair(0), true), [expected; 2]);
    }
}
