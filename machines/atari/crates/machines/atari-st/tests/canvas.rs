//! `borders = on`: the picture is drawn with its borders (832 x 552); off leaves the 640 x 400 picture as it was.

mod common;
use common::{config, emutos, rom};

use atari_st::{Accuracy, AtariSt, RamSize};
use machine::Machine;

fn machine(rom: &[u8], accuracy: Accuracy, borders: bool) -> AtariSt {
    let mut cfg = config(rom.to_vec(), RamSize::M1);
    cfg.accuracy = accuracy;
    let mut m = AtariSt::new(cfg).unwrap();
    m.set_borders(borders).unwrap();
    m
}

fn run_to_vbl(m: &mut AtariSt, vbls: u64) {
    while m.io_stop().is_none() && m.vbl_count() < vbls {
        m.step();
    }
}

#[test]
fn borders_off_is_the_640_by_400_picture() {
    let m = machine(&rom(&[0x60FE], &[]), Accuracy::Accurate, false);
    let (w, h, rgb) = m.render_frame().unwrap();
    assert_eq!((w, h), (640, 400));
    assert_eq!(rgb.len(), 640 * 400 * 3);
}

#[test]
fn borders_on_is_the_832_by_552_canvas() {
    let m = machine(&rom(&[0x60FE], &[]), Accuracy::Accurate, true);
    let (w, h, rgb) = m.render_frame().unwrap();
    assert_eq!((w, h), (832, 552));
    assert_eq!(rgb.len(), 832 * 552 * 3);
}

#[test]
fn borders_need_the_accurate_mode() {
    let mut m = machine(&rom(&[0x60FE], &[]), Accuracy::Fast, false);
    let err = m.set_borders(true).unwrap_err();
    assert!(err.contains("accurate"), "{err}");
    assert_eq!(m.render_frame().unwrap().0, 640, "the canvas is unchanged");
}

#[test]
fn a_state_belongs_to_the_canvas_it_was_saved_with() {
    let Some(rom) = emutos() else {
        return;
    };
    let mut on = machine(&rom, Accuracy::Accurate, true);
    run_to_vbl(&mut on, 5);
    let state = on.save_state_checked().unwrap();
    let mut off = machine(&rom, Accuracy::Accurate, false);
    let err = off.load_state_checked(&state).unwrap_err().to_string();
    assert!(err.to_lowercase().contains("border"), "{err}");
    let mut same = machine(&rom, Accuracy::Accurate, true);
    same.load_state_checked(&state).unwrap();
    assert_eq!(same.render_frame().unwrap().0, 832);
}

#[test]
fn the_desktop_is_inside_the_borders_on_the_canvas() {
    let Some(rom) = emutos() else {
        return;
    };
    let mut m = machine(&rom, Accuracy::Accurate, true);
    run_to_vbl(&mut m, 700);
    let (w, _, rgb) = m.render_frame().unwrap();
    let px = |x: u32, y: u32| {
        let i = ((y * w + x) * 3) as usize;
        [rgb[i], rgb[i + 1], rgb[i + 2]]
    };
    // The border columns are one colour; the picture area holds more than one (the desktop's menu bar and icons).
    let border: std::collections::HashSet<_> = (0..90)
        .flat_map(|x| (60..450).map(move |y| (x, y)))
        .map(|(x, y)| px(x, y))
        .collect();
    assert_eq!(border.len(), 1, "uniform border");
    let picture: std::collections::HashSet<_> = (96..736)
        .flat_map(|x| (58..458).map(move |y| (x, y)))
        .map(|(x, y)| px(x, y))
        .collect();
    assert!(picture.len() > 1, "the desktop is in the picture area");
}

#[test]
#[ignore = "report"]
fn mono_palette_report() {
    let Some(rom) = emutos() else {
        return;
    };
    let mut cfg = config(rom, RamSize::M1);
    cfg.monitor = st_shifter::Monitor::Mono;
    let mut m = AtariSt::new(cfg).unwrap();
    run_to_vbl(&mut m, 300);
    println!("palette {:X?}", m.bus().shifter().palette());
}
