use gtia::Gtia;

// Altirra Hardware Reference 2026-01-02, §§6.5-6.8, 6.10, 14.3.
// The independent Acid800 gtia_vdelay/pmoverlap/pmresize/collision tests
// exercise these same mechanisms through real CPU writes and collision reads.
fn setup() -> Gtia {
    let mut g = Gtia::new(false);
    for i in 0..4 {
        g.write(18 + i, 0x12 + i * 0x20);
        g.write(22 + i, 0x86 + i * 0x10);
    }
    g.write(26, 0x04);
    g
}

fn line(g: &mut Gtia, pf: [u8; 2], visible: bool) -> Vec<[u8; 2]> {
    g.begin_scanline();
    (0..228).map(|_| g.tick_color_clock(pf, visible)).collect()
}

fn hpos(h: usize) -> usize {
    h + 2
}

#[test]
fn dma_gates_vertical_delay_and_cpu_graphics_are_independent() {
    let mut g = setup();
    g.write(0, 0x80);
    g.write(4, 0x90);
    g.dma_player(0, 0xff, true).unwrap();
    g.dma_missiles(3, true);
    assert_eq!(g.register(13), 0);
    assert_eq!(g.register(17), 0);
    g.write(29, 3);
    g.write(28, 0x11);
    g.dma_player(0, 0xff, false).unwrap();
    g.dma_missiles(0xff, false);
    assert_eq!(g.register(13), 0);
    assert_eq!(g.register(17), 0xfc);
    g.dma_player(0, 0x80, true).unwrap();
    g.dma_missiles(2, true);
    let image = line(&mut g, [0; 2], true);
    assert_eq!(image[hpos(0x80)], [0x12; 2]);
    assert_eq!(image[hpos(0x81)], [4; 2]);
    assert_eq!(image[hpos(0x90)], [0x12; 2]);
    assert_eq!(image[hpos(0x91)], [4; 2]);
    g.write(13, 0x40);
    let image = line(&mut g, [0; 2], true);
    assert_eq!(image[hpos(0x80)], [4; 2]);
    assert_eq!(image[hpos(0x81)], [0x12; 2]);
    assert!(g.dma_player(4, 0, false).is_err());
}

#[test]
fn every_player_and_missile_size_expands_right_msb_first() {
    for (size, width) in [(0, 1), (1, 2), (2, 1), (3, 4)] {
        let mut g = setup();
        g.write(0, 0x80);
        g.write(4, 0x90);
        g.write(8, size);
        g.write(12, size);
        g.write(13, 0x81);
        g.write(17, 2);
        let image = line(&mut g, [0; 2], true);
        for x in 0..8 * width {
            let expected = if x < width || x >= 7 * width { 0x12 } else { 4 };
            // Quad-width player also overlaps the missile, with the same color.
            let expected = if (16..16 + width).contains(&x) {
                0x12
            } else {
                expected
            };
            assert_eq!(image[hpos(0x80 + x)], [expected; 2], "size {size}, x {x}");
        }
        assert_eq!(image[hpos(0x90)], [0x12; 2]);
    }
}

#[test]
fn collisions_latch_all_hidden_pairs_and_clear_only_on_hitclr() {
    let mut g = setup();
    for i in 0..3 {
        g.write(i, 0x80);
        g.write(13 + i, 0x80);
    }
    g.write(4, 0x80);
    g.write(17, 2);
    line(&mut g, [2; 2], true);
    assert_eq!(g.read(0), 2);
    assert_eq!(g.read(4), 2);
    assert_eq!(g.read(8), 7);
    assert_eq!(g.read(12), 6);
    assert_eq!(g.read(13), 5);
    assert_eq!(g.read(14), 3);
    line(&mut g, [0; 2], false);
    assert_eq!(g.read(12), 6);
    g.write(30, 0xff);
    assert_eq!((0..16).map(|r| g.read(r)).sum::<u8>(), 0);
    line(&mut g, [2; 2], false);
    assert_eq!(g.read(4), 0);
}

#[test]
fn hires_pair_collisions_and_luminance_apply_even_over_players() {
    let mut g = setup();
    g.write(0, 0x80);
    g.write(13, 0x80);
    g.write(27, 1);
    g.write(23, 0x0e);
    let image = line(&mut g, [5, 6], true);
    assert_eq!(image[hpos(0x80)], [0x12, 0x1e]);
    assert_eq!(g.read(4), 4);
    g.write(30, 0);
    line(&mut g, [5; 2], true);
    assert_eq!(g.read(4), 0);
}

#[test]
fn normal_priorities_multicolor_fifth_player_and_black_conflicts() {
    for (prior, pf, expected) in [
        (1, 1, 0x12),
        (2, 1, 0x12),
        (4, 1, 0x86),
        (8, 1, 0x86),
        (0, 1, 0x96),
        (5, 1, 0),
    ] {
        let mut g = setup();
        g.write(0, 0x80);
        g.write(13, 0x80);
        g.write(27, prior);
        assert_eq!(line(&mut g, [pf; 2], true)[hpos(0x80)], [expected; 2]);
    }
    let mut g = setup();
    g.write(0, 0x80);
    g.write(1, 0x80);
    g.write(13, 0x80);
    g.write(14, 0x80);
    g.write(27, 0x21);
    assert_eq!(line(&mut g, [0; 2], true)[hpos(0x80)], [0x32; 2]);
    g.write(4, 0x90);
    g.write(17, 2);
    g.write(27, 0x18);
    assert_eq!(line(&mut g, [1; 2], true)[hpos(0x90)], [0xb6; 2]);
    assert_eq!(g.read(0), 1);
}

#[test]
fn no_pmg_preserves_existing_playfield_colors() {
    let mut g = setup();
    for code in 0..7 {
        let expected = g.color(code);
        assert!(
            line(&mut g, [code; 2], true)[hpos(0x22)..=hpos(0xdd)]
                .iter()
                .all(|p| *p == [expected; 2])
        );
    }
}

fn advance_to(g: &mut Gtia, clock: &mut usize, target: usize) {
    while *clock < hpos(target) {
        g.tick_color_clock([0; 2], true);
        *clock += 1;
    }
}

#[test]
fn retrigger_ors_surviving_bits_and_does_not_move_existing_image() {
    let mut g = setup();
    g.write(0, 0x60);
    g.write(13, 0x81);
    g.begin_scanline();
    let mut clock = 0;
    advance_to(&mut g, &mut clock, 0x5e);
    g.write(0, 0x65);
    advance_to(&mut g, &mut clock, 0x60);
    assert_eq!(g.tick_color_clock([0; 2], true), [0x12; 2]);
    clock += 1;
    // Retrigger at $65 combines surviving $20 with $81, producing $A1.
    advance_to(&mut g, &mut clock, 0x65);
    assert_eq!(g.tick_color_clock([0; 2], true), [0x12; 2]);
    assert_eq!(g.tick_color_clock([0; 2], true), [4; 2]);
    assert_eq!(g.tick_color_clock([0; 2], true), [0x12; 2]);
}

#[test]
fn overlapping_quad_images_or_into_shift_register_not_onto_screen() {
    let mut g = setup();
    g.write(0, 0x60);
    g.write(8, 3);
    g.write(13, 0x81);
    g.begin_scanline();
    let mut clock = 0;
    advance_to(&mut g, &mut clock, 0x61);
    g.write(0, 0x68);
    advance_to(&mut g, &mut clock, 0x68);
    let pixels: Vec<_> = (0..32)
        .map(|_| g.tick_color_clock([0; 2], true)[0])
        .collect();
    // At +8 the old low bit has reached bit 2. OR with $81 creates $85:
    // four on, sixteen off, four on, four off, four on.
    for (x, pixel) in pixels.into_iter().enumerate() {
        assert_eq!(
            pixel,
            if x < 4 || (20..24).contains(&x) || x >= 28 {
                0x12
            } else {
                4
            }
        );
    }
}

#[test]
fn size_two_lockup_survives_blank_and_scanline_boundary_until_retrigger() {
    let mut g = setup();
    g.write(0, 0x80);
    g.write(8, 1);
    g.write(13, 0xff);
    g.begin_scanline();
    let mut clock = 0;
    advance_to(&mut g, &mut clock, 0x80);
    // The size write reaches the stretch machine three clocks later, when
    // the double-width state is 1, and enters the size-%10 lockup.
    advance_to(&mut g, &mut clock, 0x82);
    g.write(8, 2);
    g.write(0, 0xe4); // Never retrigger in the real 228-clock counter.
    advance_to(&mut g, &mut clock, 0x90);
    assert_eq!(g.tick_color_clock([0; 2], true), [0x12; 2]);
    line(&mut g, [0; 2], false);
    let pixels = line(&mut g, [0; 2], true);
    assert_eq!(pixels[hpos(0x30)], [0x12; 2]);
    g.write(0, 0x40);
    g.write(13, 0);
    let pixels = line(&mut g, [0; 2], true);
    assert_eq!(pixels[hpos(0x50)], [4; 2]);
}

#[test]
fn collision_edges_include_partial_objects_but_not_horizontal_blank() {
    for (position, data, expected) in [
        (0x21, 0x80, 0),
        (0x21, 0x40, 2),
        (0xdd, 0x80, 2),
        (0xde, 0x80, 0),
        (0xe4, 0xff, 0),
    ] {
        let mut g = setup();
        g.write(0, position);
        g.write(13, data);
        line(&mut g, [2; 2], true);
        assert_eq!(g.read(4), expected, "HPOS={position:02x}");
    }
}

#[test]
fn writes_keep_color_priority_delays_and_graphics_load_deadlines() {
    for (register, after, pf, expected) in [
        (18, 0x42, 0, vec![0x12, 0x42, 0x42]),
        (27, 4, 1, vec![0x12, 0x12, 0x86]),
    ] {
        let mut g = setup();
        g.write(0, 0x80);
        g.write(13, 0xff);
        g.write(27, 1);
        g.begin_scanline();
        let mut clock = 0;
        advance_to(&mut g, &mut clock, 0x80);
        g.write(register, after);
        for color in expected {
            assert_eq!(g.tick_color_clock([pf; 2], true), [color; 2]);
        }
    }
    for (position, expected) in [(0x82, 0x12), (0x83, 4)] {
        let mut g = setup();
        g.write(0, position);
        g.write(13, 0xff);
        g.begin_scanline();
        let mut clock = 0;
        advance_to(&mut g, &mut clock, 0x80);
        g.write(13, 0);
        advance_to(&mut g, &mut clock, position.into());
        assert_eq!(g.tick_color_clock([0; 2], true), [expected; 2]);
    }
    for (write_hpos, expected) in [(0x79, 2), (0x7a, 0)] {
        let mut g = setup();
        g.write(0, 0x90);
        g.write(13, 0x80);
        g.begin_scanline();
        let mut clock = 0;
        advance_to(&mut g, &mut clock, write_hpos);
        g.write(0, 0x80);
        advance_to(&mut g, &mut clock, 0x80);
        g.tick_color_clock([2; 2], true);
        assert_eq!(g.read(4), expected, "write at ${write_hpos:02x}");
    }
    let mut g = setup();
    g.write(0, 0x80);
    g.write(13, 0xff);
    g.begin_scanline();
    let mut clock = 0;
    advance_to(&mut g, &mut clock, 0x80);
    g.write(8, 3);
    for _ in 0..17 {
        assert_eq!(g.tick_color_clock([0; 2], true), [0x12; 2]);
    }
    assert_eq!(g.tick_color_clock([0; 2], true), [4; 2]);
}

#[test]
fn acid800_resize_collision_patterns_at_current_machine_bus_phase() {
    // Acid800 gtia_pmresize.s, runtest and all seven observed testpat tables.
    // The released .lst/.xex has $54 at testpat7[9]; the bundled source has
    // a stale $50 there. Use the actual released test's $54 expectation.
    // A real Host trace measured index 1's STY SIZEP0 at GTIA clock 94,
    // output HPOS $5C (CPU cycle 47). Index 0 instead arrives at clock 92;
    // this fixture does not establish whole-Machine WSYNC phase acceptance.
    let mut mismatches = Vec::new();
    for (old, new, first_position, expected) in [
        (
            3,
            0,
            0x48,
            [
                0x80, 0x80, 0x40, 0x40, 0x40, 0x40, 0xa0, 0xa0, 0xa0, 0xa0, 0x50, 0x50, 0x50, 0x50,
                0xa8, 0xa8,
            ],
        ),
        (
            3,
            1,
            0x48,
            [
                0x80, 0xc0, 0x60, 0x30, 0x60, 0x30, 0x98, 0xcc, 0x98, 0xcc, 0x66, 0x32, 0x66, 0x32,
                0x98, 0xcc,
            ],
        ),
        (
            1,
            3,
            0x54,
            [
                0xe0, 0xf0, 0x1e, 0x0e, 0xe0, 0xf0, 0x1e, 0x0e, 0xe0, 0xf0, 0x1e, 0x0e, 0xe0, 0xf0,
                0x78, 0x3c,
            ],
        ),
        (
            0,
            1,
            0x54,
            [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x30, 0xcc, 0x32, 0xcc, 0x32, 0xcc,
                0x66, 0x32,
            ],
        ),
        (
            0,
            3,
            0x54,
            [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x0e, 0xf0, 0x0e, 0xf0, 0x0e, 0xf0,
                0x78, 0x3c,
            ],
        ),
        (
            1,
            2,
            0x54,
            [
                0xfe, 0x80, 0x00, 0x40, 0xfe, 0xa0, 0x00, 0x50, 0xfe, 0xa8, 0x00, 0x54, 0xfe, 0xaa,
                0x54, 0x2a,
            ],
        ),
        (
            3,
            2,
            0x54,
            [
                0x00, 0x50, 0xa8, 0xfe, 0xfe, 0xa8, 0x54, 0x00, 0x00, 0x54, 0xaa, 0xfe, 0xfe, 0xaa,
                0x54, 0x2a,
            ],
        ),
    ] {
        for (offset, expected) in expected.into_iter().enumerate() {
            let mut g = setup();
            let position = first_position + offset as u8;
            g.write(0, position);
            g.write(8, old);
            g.write(13, 0xaa);
            for player in 1..4 {
                g.write(player, 0x60 + player);
                g.write(13 + player, 0x80);
            }

            for missile in 0..4 {
                g.write(4 + missile, 0x64 + missile);
            }
            g.write(17, 0xaa);
            g.begin_scanline();
            let mut clock = 0;
            advance_to(&mut g, &mut clock, 0x5c);
            g.write(8, new);
            while clock < 228 {
                g.tick_color_clock([0; 2], true);
                clock += 1;
            }
            let mut pattern = 0;
            for collision in [13, 14, 15, 8, 9, 10, 11] {
                pattern = (pattern << 1) | (g.read(collision) & 1);
            }
            if pattern << 1 != expected {
                mismatches.push(format!(
                    "size {old}->{new}, HPOS {position:02x}: {:02x} != {expected:02x}",
                    pattern << 1
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn size_write_reaches_player_and_missile_stretch_before_output_stage() {
    for (position_register, size_register, graphics_register, graphics) in
        [(0, 8, 13, 0xc0), (4, 12, 17, 3)]
    {
        let mut g = setup();
        g.write(position_register, 0x80);
        g.write(size_register, 3);
        g.write(graphics_register, graphics);
        g.begin_scanline();
        let mut clock = 0;
        advance_to(&mut g, &mut clock, 0x80);
        g.write(size_register, 0);
        let pixels: Vec<_> = (0..9)
            .map(|_| g.tick_color_clock([0; 2], true)[0])
            .collect();
        // Three clocks to the stretch machine, two to its output. The
        // shortened second bit ends at $85 rather than the original $87.
        assert_eq!(pixels, [0x12, 0x12, 0x12, 0x12, 0x12, 0x12, 4, 4, 4]);
    }
}

#[test]
fn acid800_quad_retrigger_shifts_surviving_data_when_stretch_resets() {
    // gtia_pmoverlap.s pass 3, wide_data_4 at HPOS $65: observed $78.
    let mut g = setup();
    g.write(0, 0x60);
    g.write(8, 3);
    g.write(13, 0x81);
    g.begin_scanline();
    let mut clock = 0;
    advance_to(&mut g, &mut clock, 0x5a);
    g.write(0, 0x65);
    advance_to(&mut g, &mut clock, 0x78);
    let mut pattern = 0;
    for _ in 0..8 {
        pattern = (pattern << 1) | u8::from(g.tick_color_clock([0; 2], true)[0] == 0x12);
    }
    assert_eq!(pattern, 0x78);
}

#[test]
fn retrigger_deadline_preserves_an_image_already_in_the_output_pipeline() {
    // Acid800 gtia_pmretrigger.s detects the original $40 image with P1,
    // and the later $B8 image with P2. Check either side of each deadline.
    for (write_at, expected) in [(0x39, 4), (0x3a, 6), (0xb1, 6), (0xb2, 2)] {
        let mut g = setup();
        for (player, position) in [(0, 0x40), (1, 0x40), (2, 0xb8)] {
            g.write(player, position);
            g.write(13 + player, 0xff);
        }
        g.begin_scanline();
        let mut clock = 0;
        advance_to(&mut g, &mut clock, write_at);
        g.write(0, 0xb8);
        while clock < 228 {
            g.tick_color_clock([0; 2], true);
            clock += 1;
        }
        assert_eq!(g.read(12) & 6, expected, "write at {write_at:02x}");
    }
}
