//! A disc opened from a CHD file: the track table, the silent pregap, big-endian audio turned little-endian.

use chd::builder::{build, disc_frames};
use disc::{Disc, DiscError, RAW_SECTOR};
use std::path::PathBuf;

fn open(codec: &[u8; 4]) -> (Disc, PathBuf) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("disc-chd-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.chd");
    std::fs::write(&path, build(codec)).unwrap();
    (Disc::open(&path).unwrap(), path)
}

#[test]
fn the_tracks_are_laid_out_from_the_metadata() {
    let (disc, _) = open(b"cdzl");
    let t = disc.tracks();
    assert_eq!(t.len(), 2);
    // track 1: five data frames at the start; track 2: a virtual two-sector pregap, then three audio frames
    assert_eq!(
        (t[0].number, t[0].start_lba, t[0].pregap_lba, t[0].sectors),
        (1, 0, 0, 5)
    );
    assert_eq!(
        (t[1].number, t[1].start_lba, t[1].pregap_lba, t[1].sectors),
        (2, 7, 5, 3)
    );
    assert_eq!(disc.lead_out(), 10);
}

#[test]
fn data_sectors_come_back_raw_the_pregap_is_silent_and_audio_is_little_endian() {
    let (mut disc, _) = open(b"cdzl");
    let frames = disc_frames();
    let mut sector = [0u8; RAW_SECTOR];
    for lba in 0..5 {
        disc.read_raw(lba, &mut sector).unwrap();
        assert_eq!(
            &sector[..],
            &frames[lba as usize][..RAW_SECTOR],
            "data sector {lba}"
        );
    }
    for lba in 5..7 {
        disc.read_raw(lba, &mut sector).unwrap();
        assert!(sector.iter().all(|&b| b == 0), "pregap sector {lba}");
    }
    // the file holds audio big-endian; the disc hands out the bytes of a BIN file
    disc.read_raw(7, &mut sector).unwrap();
    let stored = &frames[8][..RAW_SECTOR];
    for (got, want) in sector.chunks(2).zip(stored.chunks(2)) {
        assert_eq!((got[0], got[1]), (want[1], want[0]));
    }
    let mut user = [0u8; 2048];
    disc.read_user(2, &mut user).unwrap();
    assert_eq!(&user[..], &frames[2][16..2064]);
    assert_eq!(
        disc.read_raw(10, &mut sector),
        Err(DiscError::OutOfRange(10))
    );
}

#[test]
fn a_chd_with_a_codec_that_is_not_a_cd_one_fails_when_read_not_when_opened() {
    let (mut disc, _) = open(b"zlib");
    let mut sector = [0u8; RAW_SECTOR];
    assert!(matches!(
        disc.read_raw(0, &mut sector),
        Err(DiscError::Unsupported(_))
    ));
}
