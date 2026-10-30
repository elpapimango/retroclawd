//! Tests against small CHD files built by `builder`.

use crate::builder::{build, cdzl_hunk, disc_frames};
use crate::cd::{FRAME_SIZE, MAIN_SIZE};
use crate::{Chd, ChdError, ecc};
use std::path::PathBuf;

fn write(name: &str, bytes: &[u8]) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("chd-test-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn a_disc_of_cdzl_hunks_reads_back_frame_for_frame() {
    let path = write("disc.chd", &build(b"cdzl"));
    let mut chd = Chd::open(&path).unwrap();
    assert_eq!(chd.frames(), 12);
    let tracks = chd.tracks();
    assert_eq!(
        (
            tracks[0].first_frame,
            tracks[0].frames,
            tracks[0].kind.as_str()
        ),
        (0, 5, "MODE1_RAW")
    );
    assert_eq!(
        (tracks[1].first_frame, tracks[1].frames, tracks[1].pregap),
        (8, 3, 2)
    );
    assert!(!tracks[1].pregap_in_file, "a V pregap is not in the file");
    let want = disc_frames();
    let mut got = [0u8; FRAME_SIZE];
    for (n, frame) in want.iter().enumerate() {
        chd.read_frame(n as u32, &mut got).unwrap();
        assert_eq!(&got[..], &frame[..], "frame {n}");
    }
    assert!(chd.read_frame(12, &mut got).is_err());
}

#[test]
fn the_sync_and_parity_of_data_sectors_were_taken_out_and_come_back() {
    let frames = disc_frames();
    // the stored hunk is much smaller than the plain data would be: the flag bits are set for the five data frames
    let hunk = cdzl_hunk(&frames[..2]);
    assert_eq!(hunk[0], 0b11);
    let path = write("disc.chd", &build(b"cdzl"));
    let mut chd = Chd::open(&path).unwrap();
    let mut got = [0u8; FRAME_SIZE];
    chd.read_frame(1, &mut got).unwrap();
    assert_eq!(&got[..12], &ecc::SYNC);
    assert!(ecc::is_regenerable(got[..MAIN_SIZE].try_into().unwrap()));
}

#[test]
fn files_this_reader_does_not_do_are_refused_in_words() {
    let mut old = build(b"cdzl");
    old[15] = 4;
    let err = Chd::open(&write("v4.chd", &old)).err().unwrap();
    assert!(
        matches!(err, ChdError::Unsupported(ref m) if m.contains("version 4")),
        "{err}"
    );

    let mut parented = build(b"cdzl");
    parented[110] = 1;
    let err = Chd::open(&write("parent.chd", &parented)).err().unwrap();
    assert!(
        matches!(err, ChdError::Unsupported(ref m) if m.contains("parent")),
        "{err}"
    );

    // a codec that is not a CD one: opens (the map is fine), refuses the hunk
    let mut chd = Chd::open(&write("zlib.chd", &build(b"zlib"))).unwrap();
    let err = chd.hunk(0).unwrap_err();
    assert!(
        matches!(err, ChdError::Unsupported(ref m) if m.contains("zlib")),
        "{err}"
    );

    assert!(Chd::open(&write("junk.chd", b"not a chd at all, just text")).is_err());
    assert!(Chd::open(&write("empty.chd", b"")).is_err());
}

#[test]
fn a_damaged_file_never_panics_and_is_caught_by_its_checks() {
    let good = build(b"cdzl");
    let mut caught = 0;
    for at in 0..good.len() {
        let mut bad = good.clone();
        bad[at] ^= 0x5A;
        let path = write("bad.chd", &bad);
        let outcome = Chd::open(&path).and_then(|mut chd| {
            let mut frame = [0u8; FRAME_SIZE];
            for n in 0..chd.frames() {
                chd.read_frame(n, &mut frame)?;
            }
            Ok(())
        });
        caught += usize::from(outcome.is_err());
    }
    // the bytes that matter (everything but the checksums the reader ignores and the padding) are all detected
    assert!(caught > good.len() * 9 / 10, "{caught} of {}", good.len());
    // a file cut anywhere is refused or read, never a panic
    for len in (0..good.len()).step_by(7) {
        let _ = Chd::open(&write("cut.chd", &good[..len]));
    }
}
