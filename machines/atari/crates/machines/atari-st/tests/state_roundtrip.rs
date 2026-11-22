//! Save states of the machine: a snapshot taken mid-run and restored into a fresh machine continues exactly as the
//! original does (spec section 19). The comparison is the states' own bytes after the same further run: every device,
//! RAM and the CPU are in it, so equal bytes mean an equal machine.

mod common;
use common::{BRA_SELF, config, move_b, rom};

use atari_st::{AtariSt, RamSize};
use machine::Machine;
use st_floppy::Image;

fn move_w(value: u16, addr: u16) -> [u16; 3] {
    [0x31FC, value, addr]
}

fn select_a() -> Vec<u16> {
    [
        move_b(7, 0x8800),
        move_b(0xC0, 0x8802),
        move_b(14, 0x8800),
        move_b(0x05, 0x8802),
    ]
    .concat()
}

fn image() -> Image {
    let bytes: Vec<u8> = (0..737_280usize).map(|i| (i * 7 + i / 512) as u8).collect();
    Image::parse(&bytes).unwrap()
}

/// Fills `$10000..` with `$DEADBEEF`, writes it to sector 3 by DMA and loops: floppy, DMA and CPU all busy for a while.
fn write_sector_program() -> Vec<u16> {
    let mut p = vec![
        0x41F9, 0x0001, 0x0000, 0x303C, 0x007F, 0x20FC, 0xDEAD, 0xBEEF, 0x51C8, 0xFFF8,
    ];
    p.extend(select_a());
    p.extend(move_b(0x00, 0x860D));
    p.extend(move_b(0x00, 0x860B));
    p.extend(move_b(0x01, 0x8609));
    p.extend(move_w(0x090, 0x8606));
    p.extend(move_w(0x190, 0x8606));
    p.extend(move_w(1, 0x8604));
    p.extend(move_w(0x184, 0x8606));
    p.extend(move_w(3, 0x8604));
    p.extend(move_w(0x180, 0x8606));
    p.extend(move_w(0xA0, 0x8604));
    p.push(BRA_SELF);
    p
}

fn machine(program: &[u16], ram: RamSize) -> AtariSt {
    let mut m = AtariSt::new(config(rom(program, &[]), ram)).unwrap();
    m.insert_disk(0, image()).unwrap();
    m
}

fn run_to(m: &mut AtariSt, cycles: u64) {
    while m.now() < cycles && m.io_stop().is_none() {
        m.step();
    }
}

#[test]
fn restore_of_a_snapshot_continues_identically() {
    let mut a = machine(&write_sector_program(), RamSize::M1);
    // 1.5M cycles in: the FDC command is under way, the DMA is feeding it, the IKBD and the MFP are running.
    run_to(&mut a, 1_500_000);
    let saved = a.save_state();
    assert!(!saved.is_empty());
    run_to(&mut a, 12_000_000);
    let reference = a.save_state();

    let mut b = machine(&write_sector_program(), RamSize::M1);
    b.load_state(&saved).unwrap();
    assert_eq!(
        b.save_state(),
        saved,
        "restoring and saving again changes nothing"
    );
    run_to(&mut b, 12_000_000);
    assert_eq!(b.now(), a.now());
    assert_eq!(
        b.save_state(),
        reference,
        "the restored machine ran exactly as the original"
    );
    assert!(b.disk_modified(0), "the written sector is there");
}

#[test]
fn restoring_the_same_state_twice_gives_the_same_run() {
    let mut a = machine(&write_sector_program(), RamSize::M1);
    run_to(&mut a, 1_000_000);
    let saved = a.save_state();
    let mut results = Vec::new();
    for _ in 0..2 {
        let mut b = machine(&write_sector_program(), RamSize::M1);
        b.load_state(&saved).unwrap();
        run_to(&mut b, 3_000_000);
        results.push(b.save_state());
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn emutos_resumes_from_a_state() {
    let Some(tos) = common::emutos() else { return };
    let mut a = AtariSt::new(config(tos.clone(), RamSize::M1)).unwrap();
    while a.vbl_count() < 200 && a.io_stop().is_none() {
        a.step();
    }
    let saved = a.save_state();
    while a.vbl_count() < 260 && a.io_stop().is_none() {
        a.step();
    }
    let reference = a.save_state();
    assert!(a.io_stop().is_none());

    let mut b = AtariSt::new(config(tos, RamSize::M1)).unwrap();
    b.load_state(&saved).unwrap();
    while b.vbl_count() < 260 && b.io_stop().is_none() {
        b.step();
    }
    assert_eq!(b.save_state(), reference);
    assert_eq!(
        b.render_frame().unwrap().2,
        a.render_frame().unwrap().2,
        "the same picture"
    );
}

#[test]
fn a_stopped_machine_has_no_state() {
    // MOVE.B $FF9000,D0: an I/O address with no fact stops the machine.
    let mut m = AtariSt::new(config(
        rom(&[0x1039, 0x00FF, 0x9000, BRA_SELF], &[]),
        RamSize::M1,
    ))
    .unwrap();
    run_to(&mut m, 100_000);
    assert!(m.io_stop().is_some());
    assert!(m.save_state().is_empty());
    assert!(matches!(
        m.save_state_checked(),
        Err(atari_st::StateError::Stopped)
    ));
}

#[test]
fn a_state_for_other_ram_is_refused_and_changes_nothing() {
    let mut a = machine(&[BRA_SELF], RamSize::M1);
    run_to(&mut a, 200_000);
    let saved = a.save_state();
    let mut b = machine(&[BRA_SELF], RamSize::M2);
    run_to(&mut b, 100_000);
    let before = b.save_state();
    let err = b.load_state(&saved).unwrap_err();
    assert!(err.contains("RAM"), "{err}");
    assert_eq!(
        b.save_state(),
        before,
        "a refused state leaves the machine as it was"
    );
}

#[test]
fn a_modified_disk_is_in_the_state_and_a_clean_one_is_not() {
    let mut a = machine(&write_sector_program(), RamSize::M1);
    run_to(&mut a, 12_000_000);
    assert!(a.disk_modified(0));
    let dirty = a.save_state();
    // A fresh machine with the original disk takes the written sector from the state.
    let mut b = machine(&write_sector_program(), RamSize::M1);
    b.load_state(&dirty).unwrap();
    let sector = b.disk(0).unwrap().sector(0, 0, 3).unwrap();
    assert!(sector.chunks(4).all(|c| c == [0xDE, 0xAD, 0xBE, 0xEF]));
    assert!(b.disk_modified(0));

    // Without a write the disk stays the user's file: the state is about the size of the RAM, not RAM + 720K.
    let mut c = machine(&write_sector_program(), RamSize::M1);
    run_to(&mut c, 200_000);
    let clean = c.save_state();
    let mut d = AtariSt::new(config(rom(&write_sector_program(), &[]), RamSize::M1)).unwrap();
    run_to(&mut d, 200_000);
    let no_disk = d.save_state();
    assert!(
        clean.len() < no_disk.len() + 4096,
        "{} bytes with a clean disk, {} without one",
        clean.len(),
        no_disk.len()
    );
}
