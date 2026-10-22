//! Disk games from the owner's RomM (platform `atari-st`) boot on the machine, deterministically, and reading them
//! never changes them. The images are copyrighted and never committed: `RC_ROMM_GAMES` names a folder of unzipped
//! `.st` files (fetch them with `retroclawd romm games atari-st` and `--disk romm:<id>`). Without it the tests print
//! where they looked and pass, like the EmuTOS tests. Nothing from the images is printed except names and hashes.

mod common;

use std::path::PathBuf;

use atari_st::{AtariSt, RamSize};
use machine::Machine;
use st_floppy::Image;

const FRAMES: u64 = 700;

fn games() -> Vec<(String, Vec<u8>)> {
    let Some(dir) = std::env::var_os("RC_ROMM_GAMES").map(PathBuf::from) else {
        eprintln!("RC_ROMM_GAMES is not set: no RomM game images to test");
        return Vec::new();
    };
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("cannot read {}", dir.display());
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("st"))
            && let Ok(bytes) = std::fs::read(&path)
        {
            out.push((e.file_name().to_string_lossy().into_owned(), bytes));
        }
    }
    out.sort();
    out
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

/// A machine with EmuTOS and `disk` in drive A, run for `FRAMES` frames; its frame hash and whether it stopped.
fn run(rom: &[u8], disk: Option<&[u8]>) -> (AtariSt, u64) {
    let mut m = AtariSt::new(common::config(rom.to_vec(), RamSize::M1)).unwrap();
    if let Some(bytes) = disk {
        m.insert_disk(0, Image::parse(bytes).expect("the image parses"))
            .unwrap();
    }
    while m.vbl_count() < FRAMES && m.io_stop().is_none() {
        m.step();
    }
    let hash = fnv(&m.render_frame().unwrap().2);
    (m, hash)
}

/// The boot sector's word sum is `$1234`: TOS runs it.
fn is_executable(bytes: &[u8]) -> bool {
    bytes[..512].chunks(2).fold(0u16, |s, w| {
        s.wrapping_add(u16::from_be_bytes([w[0], w[1]]))
    }) == 0x1234
}

#[test]
fn games_boot_without_a_stop_and_deterministically() {
    let Some(rom) = common::emutos() else { return };
    for (name, bytes) in games() {
        let (m1, h1) = run(&rom, Some(&bytes));
        assert!(m1.io_stop().is_none(), "{name}: {:?}", m1.io_stop());
        let (_, h2) = run(&rom, Some(&bytes));
        assert_eq!(h1, h2, "{name}: two runs give the same frame");
        eprintln!("{name}: frame hash {h1:016x}");
    }
}

#[test]
fn an_executable_boot_sector_is_run() {
    let Some(rom) = common::emutos() else { return };
    let (_, plain) = run(&rom, None);
    for (name, bytes) in games() {
        if !is_executable(&bytes) {
            continue;
        }
        // The program ran if the picture differs from the plain desktop's at some frame before the desktop returns.
        let mut m = AtariSt::new(common::config(rom.clone(), RamSize::M1)).unwrap();
        m.insert_disk(0, Image::parse(&bytes).unwrap()).unwrap();
        let mut differed = false;
        let mut checked = 0;
        while m.vbl_count() < FRAMES && m.io_stop().is_none() {
            m.step();
            if m.vbl_count() >= checked + 25 {
                checked = m.vbl_count();
                differed |= fnv(&m.render_frame().unwrap().2) != plain;
            }
        }
        assert!(differed, "{name}: its boot sector changed the picture");
    }
}

#[test]
fn games_are_not_modified_by_reading() {
    let Some(rom) = common::emutos() else { return };
    for (name, bytes) in games() {
        let (mut m, _) = run(&rom, Some(&bytes));
        m.commit_disks();
        assert!(
            !m.disk_modified(0),
            "{name}: reading marked the disk modified"
        );
        assert_eq!(
            m.disk(0).unwrap().to_bytes(),
            bytes,
            "{name}: the image is unchanged"
        );
    }
}
