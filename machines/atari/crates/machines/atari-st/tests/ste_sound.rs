//! The STE's DMA sound on the machine: stereo output from the registers, the volume control, bounds and states. (The
//! counter's progress and the end of a frame are compared with Hatari's dumps in `timing_probes.rs`, P29 and P41.)

mod common;
use common::config;

use atari_st::{AtariSt, Chipset, RamSize};
use machine::Machine;

fn ste_with(program: &[u16]) -> Vec<u8> {
    let mut rom = vec![0u8; 262_144];
    rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00E0_0010u32.to_be_bytes());
    for (i, w) in program.iter().enumerate() {
        rom[0x10 + 2 * i..0x12 + 2 * i].copy_from_slice(&w.to_be_bytes());
    }
    rom
}

fn mb(imm: u8, addr: u32) -> [u16; 4] {
    [0x13FC, u16::from(imm), (addr >> 16) as u16, addr as u16]
}

fn mw(imm: u16, addr: u32) -> [u16; 4] {
    [0x33FC, imm, (addr >> 16) as u16, addr as u16]
}

const DMA: u32 = 0xFF_8900;

/// A Microwire command (mask `$07FF`): the LMC1992 powers up with minimum volume and faders, so a program that wants to be
/// heard sets them.
fn microwire(function: u16, value: u16) -> [u16; 8] {
    let mut c = [0; 8];
    c[..4].copy_from_slice(&mw(0x07FF, DMA + 0x24));
    c[4..].copy_from_slice(&mw((0b10 << 9) | (function << 6) | value, DMA + 0x22));
    c
}

/// Master volume `master` (0 to 40) and both faders at 0 dB.
fn volume(master: u16) -> Vec<u16> {
    [microwire(3, master), microwire(4, 20), microwire(5, 20)].concat()
}

/// Fills `data` at `$20000`, sets the frame `$20000..$20000+len`, the mode and starts playing in loop mode.
fn play_program(data: &[u8], mode: u8, extra: &[u16]) -> Vec<u16> {
    let mut p = Vec::new();
    for (i, b) in data.iter().enumerate() {
        p.extend(mb(*b, 0x20000 + i as u32));
    }
    p.extend(extra);
    let end = 0x20000 + data.len() as u32;
    p.extend(mb(0x02, DMA + 0x03));
    p.extend(mb(0x00, DMA + 0x05));
    p.extend(mb(0x00, DMA + 0x07));
    p.extend(mb((end >> 16) as u8, DMA + 0x0F));
    p.extend(mb((end >> 8) as u8, DMA + 0x11));
    p.extend(mb(end as u8, DMA + 0x13));
    p.extend(mb(mode, DMA + 0x21));
    p.extend(mb(0x03, DMA + 0x01));
    p.push(0x60FE);
    p
}

fn machine(program: &[u16]) -> AtariSt {
    AtariSt::with_chipset(config(ste_with(program), RamSize::M1), Chipset::Ste).unwrap()
}

fn run_frames(m: &mut AtariSt, frames: u64) {
    while m.vbl_count() < frames && m.io_stop().is_none() {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
}

fn peak(samples: &[f32], channel: usize) -> f32 {
    samples
        .chunks(2)
        .skip(2000)
        .map(|c| c[channel])
        .fold(0.0, |a, b| if b.abs() > a.abs() { b } else { a })
}

#[test]
fn an_ste_makes_stereo_audio() {
    let m = machine(&[0x60FE]);
    let f = m.audio_format();
    assert_eq!((f.channels, f.sample_rate), (2, machine::AUDIO_SAMPLE_RATE));
}

#[test]
fn a_stereo_frame_plays_left_and_right_apart() {
    // stereo (bit 7 clear), 50 kHz: left +64, right -64 for ever
    let data: Vec<u8> = (0..64).flat_map(|_| [0x40u8, 0xC0]).collect();
    let mut m = machine(&play_program(&data, 0x03, &volume(40)));
    run_frames(&mut m, 4);
    let audio = m.drain_audio();
    assert!(audio.len() > 4000, "{}", audio.len());
    let (l, r) = (peak(&audio, 0), peak(&audio, 1));
    assert!(
        (l - 0.5).abs() < 0.01 && (r + 0.5).abs() < 0.01,
        "left {l}, right {r}"
    );
}

#[test]
fn a_mono_frame_plays_on_both_channels() {
    let data = vec![0x20u8; 64];
    let mut m = machine(&play_program(&data, 0x83, &volume(40)));
    run_frames(&mut m, 4);
    let audio = m.drain_audio();
    let (l, r) = (peak(&audio, 0), peak(&audio, 1));
    assert!(
        (l - 0.25).abs() < 0.01 && (r - 0.25).abs() < 0.01,
        "left {l}, right {r}"
    );
}

#[test]
fn the_microwire_master_volume_scales_the_output_by_two_decibels_a_step() {
    // mask $07FF, data device %10, command %011 (master volume), 20: 40 dB down
    let extra = volume(20);
    let data = vec![0x40u8; 64];
    let mut m = machine(&play_program(&data, 0x83, &extra));
    run_frames(&mut m, 4);
    let audio = m.drain_audio();
    let l = peak(&audio, 0);
    assert!((l - 0.5 * 0.01).abs() < 0.0005, "{l}");
}

#[test]
fn a_frame_in_rom_io_or_past_ram_or_ending_before_its_base_never_stops_or_loops_forever() {
    for (base, end) in [
        (0xE0_0000u32, 0xE0_0100u32),
        (0xFF_8200, 0xFF_8300),
        (0x3F_FF00, 0x40_0100),
        (0x2_0100, 0x2_0000),
        (0x2_0001, 0x2_0003),
    ] {
        let mut p = Vec::new();
        p.extend(mb((base >> 16) as u8, DMA + 0x03));
        p.extend(mb((base >> 8) as u8, DMA + 0x05));
        p.extend(mb(base as u8, DMA + 0x07));
        p.extend(mb((end >> 16) as u8, DMA + 0x0F));
        p.extend(mb((end >> 8) as u8, DMA + 0x11));
        p.extend(mb(end as u8, DMA + 0x13));
        p.extend(mb(0x03, DMA + 0x21));
        p.extend(mb(0x03, DMA + 0x01));
        p.push(0x60FE);
        let mut m = machine(&p);
        run_frames(&mut m, 3);
        let audio = m.drain_audio();
        assert!(!audio.is_empty(), "{base:x}..{end:x}");
        assert!(audio.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }
}

#[test]
fn the_audio_queue_is_capped_when_nobody_drains_it() {
    let mut m = machine(&play_program(&[0x40; 64], 0x83, &[]));
    run_frames(&mut m, 200);
    let audio = m.drain_audio();
    assert!(audio.len() <= 2 * 2 * 44_100, "{}", audio.len());
}

#[test]
fn the_state_round_trips_while_playing_and_playing_goes_on() {
    let data: Vec<u8> = (0..64).flat_map(|_| [0x40u8, 0xC0]).collect();
    let prog = play_program(&data, 0x03, &[]);
    let mut a = machine(&prog);
    run_frames(&mut a, 3);
    let state = a.save_state_checked().unwrap();
    let mut b = machine(&prog);
    b.load_state_checked(&state).unwrap();
    assert_eq!(b.save_state_checked().unwrap(), state);
    run_frames(&mut a, 6);
    run_frames(&mut b, 6);
    assert_eq!(
        a.drain_audio(),
        b.drain_audio(),
        "a restored machine plays the same samples"
    );
}

#[test]
fn the_volume_control_powers_up_with_minimum_volume_and_a_fader_ignores_its_sixth_bit() {
    // [D] LMC1992/3 data sheet: "powers up with flat tone and min volume/fader"; a fader's D5 is a don't-care bit
    let data = vec![0x40u8; 64];
    let mut m = machine(&play_program(&data, 0x83, &[]));
    run_frames(&mut m, 4);
    assert!(
        peak(&m.drain_audio(), 0).abs() < 0.001,
        "silent until the program sets the volume"
    );
    // master 40, left fader sent as 0b100000 (D5 set, data 0): -40 dB, silent; sent as 0b100000 | 20: 0 dB
    for (fader, audible) in [(0x20u16, false), (0x20 | 20, true)] {
        let mut extra = Vec::new();
        extra.extend(microwire(3, 40));
        extra.extend(microwire(5, fader));
        extra.extend(microwire(4, 20));
        let mut m = machine(&play_program(&data, 0x83, &extra));
        run_frames(&mut m, 4);
        let l = peak(&m.drain_audio(), 0);
        assert_eq!(l.abs() > 0.1, audible, "fader {fader:#x}: {l}");
    }
}

#[test]
fn the_bass_control_boosts_a_steady_level_by_the_shelf_gain_through_the_microwire() {
    // a constant sample is all low frequency: bass 12 (command %001, data 12) gives the shelf's final gain, about 14.6 dB;
    // bass 6 (flat) leaves it alone
    let data = vec![0x08u8; 64];
    let peak_with = |bass: Option<u16>| {
        let mut extra = volume(40);
        if let Some(b) = bass {
            extra.extend(microwire(1, b));
        }
        let mut m = machine(&play_program(&data, 0x83, &extra));
        run_frames(&mut m, 12);
        peak(&m.drain_audio(), 0)
    };
    let flat = peak_with(None);
    assert!((flat - 0.0625).abs() < 0.001, "{flat}");
    let boosted = peak_with(Some(12));
    assert!((boosted / flat - 5.36).abs() < 0.15, "{}", boosted / flat);
    let cut = peak_with(Some(0));
    assert!((cut / flat - 1.0 / 5.36).abs() < 0.02, "{}", cut / flat);
}
