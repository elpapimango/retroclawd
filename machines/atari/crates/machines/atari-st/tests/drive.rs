//! The floppy drives behind the WD1772 (facts: `docs/hardware-facts.md`, "Floppy drives"): rotation and index,
//! head stepping, side and drive select, no disk.

mod common;

use atari_st::{Drives, StDrive};
use st_floppy::{Image, TRACK_BYTES};
use wd1770::Floppy;

/// A 720K image, every byte numbered by its position.
fn image() -> Image {
    let bytes: Vec<u8> = (0..737_280usize).map(|i| (i * 7 + i / 512) as u8).collect();
    Image::parse(&bytes).unwrap()
}

fn drive_with_disk() -> StDrive {
    let mut d = StDrive::new();
    d.insert(image());
    d
}

#[test]
fn index_once_per_revolution() {
    let mut d = drive_with_disk();
    d.index_count_reset();
    let mut first = None;
    for t in 0..3 * TRACK_BYTES as u64 {
        d.rotate(1);
        if first.is_none() && d.index_count() == 1 {
            first = Some(t);
        }
    }
    assert_eq!(d.index_count(), 3);
    assert!(first.is_some());
}

#[test]
fn no_disk_no_index() {
    let mut d = StDrive::new();
    d.index_count_reset();
    d.rotate(100_000);
    assert_eq!(d.index_count(), 0);
    assert!(!d.index());
    assert!(d.write_protect(), "[O] no disk reads write-protected");
}

#[test]
fn step_and_track0() {
    let mut d = drive_with_disk();
    assert!(d.track0());
    for _ in 0..3 {
        d.seek_pulse(true);
    }
    assert_eq!(d.cylinder(), 3);
    assert!(!d.track0());
    for _ in 0..5 {
        d.seek_pulse(false);
    }
    assert_eq!(d.cylinder(), 0);
    assert!(d.track0());
}

/// The bytes of one revolution from the selected drive and side.
fn revolution(f: &mut impl Floppy) -> Vec<u16> {
    (0..TRACK_BYTES).map(|_| f.read()).collect()
}

/// The first ID field's C H R N in a revolution.
fn first_id(track: &[u16]) -> Option<[u16; 4]> {
    (0..track.len() - 8)
        .find(|&i| track[i..i + 3] == [0x1A1; 3] && track[i + 3] == 0xFE)
        .map(|i| [track[i + 4], track[i + 5], track[i + 6], track[i + 7]])
}

/// Port A (active low): bit 1 selects drive 0, bit 2 drive 1, bit 0 clear = side 1.
#[test]
fn side_select_reads_the_other_side() {
    let mut f = Drives::new(2);
    f.drive_mut(0).insert(image());
    f.set_port_a(0x05); // drive 0, side 0
    assert_eq!(first_id(&revolution(&mut f)).unwrap()[1], 0);
    f.set_port_a(0x04); // drive 0, side 1
    assert_eq!(first_id(&revolution(&mut f)).unwrap()[1], 1);
}

#[test]
fn side_change_mid_command() {
    let mut f = Drives::new(2);
    f.drive_mut(0).insert(image());
    f.set_port_a(0x05);
    let half: Vec<u16> = (0..TRACK_BYTES / 2).map(|_| f.read()).collect();
    f.set_port_a(0x04);
    let rest: Vec<u16> = (0..TRACK_BYTES).map(|_| f.read()).collect();
    assert!(first_id(&half).is_some());
    assert_eq!(
        first_id(&rest).unwrap()[1],
        1,
        "from the switch on, side 1's bytes"
    );
}

#[test]
fn deselected_drive_does_not_rotate_data_into_the_fdc() {
    let mut f = Drives::new(2);
    f.drive_mut(0).insert(image());
    f.set_port_a(0x07); // nothing selected
    f.index_count_reset();
    let track = revolution(&mut f);
    assert!(first_id(&track).is_none());
    assert!(!f.index());
    assert_eq!(f.index_count(), 0);
}

#[test]
fn drive_b_with_drives_1_is_empty() {
    let mut f = Drives::new(1);
    assert!(f.insert(1, image()).is_err());
    f.set_port_a(0x03); // drive 1, side 0
    f.index_count_reset();
    f.rotate(3 * TRACK_BYTES as u64);
    assert_eq!(f.index_count(), 0);
    assert!(first_id(&revolution(&mut f)).is_none());
}

/// [O] Hatari's status with a disk in has no write-protect bit (`$24`, `$A4`); without one it has (`$E4`).
#[test]
fn a_disk_reads_not_write_protected() {
    let d = drive_with_disk();
    assert!(!d.write_protect());
    assert!(StDrive::new().write_protect());
}

/// A disk going in or out while the machine runs blocks the write-protect sensor for a moment, which is how TOS
/// notices a swap ([H?] 0.5 s, `atari_st::MEDIA_CHANGE_BYTES`); a disk there from power-on shows nothing.
#[test]
fn a_swap_shows_write_protect_for_a_while() {
    let hold = atari_st::MEDIA_CHANGE_BYTES;
    let mut f = Drives::new(2);
    f.insert(0, image()).unwrap();
    f.set_port_a(0x05);
    assert!(!f.write_protect(), "in from power-on");
    f.rotate(10);
    f.eject(0);
    f.insert(0, image()).unwrap();
    assert!(f.write_protect(), "swapped in one go");
    f.rotate(hold - 1);
    assert!(f.write_protect());
    f.rotate(1);
    assert!(!f.write_protect());
    f.eject(0);
    assert!(f.write_protect(), "no disk");
}
