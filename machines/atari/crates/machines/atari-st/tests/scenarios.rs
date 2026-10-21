//! The scenario programs (`scenarios/`, built into `software/scenarios/` by `scenarios/build.sh`) run as intended on the
//! machine. Skipped, with a note, when the media have not been built.

mod common;
use common::{BRA_SELF, config, rom};

use std::path::PathBuf;

use atari_st::{AtariSt, RamSize};
use machine::Machine;

fn media(name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../software/scenarios")
        .join(name);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("no {} (run scenarios/build.sh)", path.display());
            None
        }
    }
}

/// `tone.bin` at `$8000` (the manifest's `--org`): tone A of the YM2149 at period 284, so `fT = (CPU clock / 4) / (16 × 284)` (YM2149 catalog,
/// section (1)); the DC-free output crosses zero twice per period.
#[test]
fn tone_bin_plays_the_formula_pitch() {
    let Some(code) = media("tone.bin") else {
        return;
    };
    let mut m = AtariSt::new(config(rom(&[BRA_SELF], &[]), RamSize::M1)).unwrap();
    m.load(0x8000, &code);
    m.set_pc(0x8000);
    let mut audio = Vec::new();
    while m.vbl_count() < 50 {
        m.step();
    }
    audio.extend(m.drain_audio());
    let mean = audio.iter().sum::<f32>() / audio.len() as f32;
    let crossings = audio
        .windows(2)
        .filter(|w| (w[0] - mean).signum() != (w[1] - mean).signum())
        .count() as i64;
    let expected = (2 * m.now() / (4 * 16 * 284)) as i64;
    assert!(
        (crossings - expected).abs() <= 2,
        "{crossings} crossings, expected {expected}"
    );
}

/// Boots EmuTOS with `keys.st` until the boot program has installed its joystick handler and waits in `Bconin`.
fn keys_machine() -> Option<AtariSt> {
    let rom = common::emutos()?;
    let disk = st_floppy::Image::parse(&media("keys.st")?).unwrap();
    let mut m = AtariSt::new(common::config(rom, RamSize::M1)).unwrap();
    m.insert_disk(0, disk).unwrap();
    while m.vbl_count() < 500 {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
    Some(m)
}

fn run_frames(m: &mut AtariSt, n: u64) {
    let end = m.vbl_count() + n;
    while m.vbl_count() < end {
        m.step();
    }
}

/// `keys.st` echoes typed characters through the BIOS: the console's picture changes with each key.
#[test]
fn keys_st_echoes_a_typed_key() {
    let Some(mut m) = keys_machine() else { return };
    let before = m.render_frame().unwrap().2;
    for key in ["KeyH", "KeyI"] {
        m.key_event(key, true).unwrap();
        run_frames(&mut m, 3);
        m.key_event(key, false).unwrap();
        run_frames(&mut m, 3);
    }
    run_frames(&mut m, 5);
    let after = m.render_frame().unwrap().2;
    assert_ne!(before, after, "the echoed characters are on the screen");
}

/// `keys.st`'s joystick handler: joystick 1 fire (event byte `$80`) sets palette 0 to `$80 >> 1` = `$040`.
#[test]
fn keys_st_joystick_sets_palette_0() {
    let Some(mut m) = keys_machine() else { return };
    m.joystick(1, 0x80);
    run_frames(&mut m, 5);
    assert_eq!(m.bus().shifter().palette()[0], 0x040);
}
