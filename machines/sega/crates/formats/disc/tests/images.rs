use disc::builder::{self, BuildTrack, Fill};
use disc::cue::TrackKind;
use disc::{Disc, DiscError, RAW_SECTOR, USER_SECTOR};
use std::path::PathBuf;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("disc-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn tracks() -> Vec<BuildTrack> {
    vec![
        BuildTrack {
            kind: TrackKind::Mode1Raw,
            sectors: 20,
            fill: Fill::Pattern,
        },
        BuildTrack {
            kind: TrackKind::Audio,
            sectors: 10,
            fill: Fill::Byte(0x55),
        },
        BuildTrack {
            kind: TrackKind::Audio,
            sectors: 5,
            fill: Fill::Pattern,
        },
    ]
}

#[test]
fn single_bin_and_split_bins_give_the_same_toc() {
    let dir = temp("toc");
    let one = builder::write_disc(&dir, "one", &tracks()).unwrap();
    let many = builder::write_split_disc(&dir, "many", &tracks()).unwrap();
    let (a, b) = (Disc::open(&one).unwrap(), Disc::open(&many).unwrap());
    assert_eq!(a.tracks(), b.tracks());
    assert_eq!(a.lead_out(), 35);
    assert_eq!(a.tracks()[1].start_lba, 20);
    assert_eq!(a.tracks()[2].sectors, 5);
}

#[test]
fn sectors_read_back_verbatim() {
    let dir = temp("read");
    let split = builder::write_split_disc(&dir, "d", &tracks()).unwrap();
    let mut d = Disc::open(&split).unwrap();
    let mut raw = [0u8; RAW_SECTOR];
    d.read_raw(3, &mut raw).unwrap();
    assert_eq!(raw[..], builder::sector_bytes(&tracks()[0], 3)[..]);
    d.read_raw(25, &mut raw).unwrap();
    assert!(raw.iter().all(|&b| b == 0x55));
    d.read_raw(33, &mut raw).unwrap();
    assert_eq!(raw[..], builder::sector_bytes(&tracks()[2], 3)[..]);
    let mut user = [0u8; USER_SECTOR];
    d.read_user(3, &mut user).unwrap();
    assert_eq!(user[..], builder::sector_bytes(&tracks()[0], 3)[16..2064]);
}

#[test]
fn cooked_iso_expands_to_a_valid_raw_sector() {
    let dir = temp("iso");
    let t = [BuildTrack {
        kind: TrackKind::Mode1Cooked,
        sectors: 4,
        fill: Fill::Pattern,
    }];
    builder::write_disc(&dir, "c", &t).unwrap();
    std::fs::rename(dir.join("c.bin"), dir.join("c.iso")).unwrap();
    let mut d = Disc::open(&dir.join("c.iso")).unwrap();
    assert_eq!(d.tracks().len(), 1);
    let mut raw = [0u8; RAW_SECTOR];
    d.read_raw(2, &mut raw).unwrap();
    assert_eq!(raw[0], 0);
    assert!(raw[1..11].iter().all(|&b| b == 0xFF));
    assert_eq!(raw[11], 0);
    assert_eq!(raw[12..16], [0x00, 0x02, 0x02, 0x01]); // 00:02:02, mode 1
    assert_eq!(raw[16..2064], builder::sector_bytes(&t[0], 2)[..]);
    assert_eq!(Disc::msf(0), (0, 2, 0));
    assert_eq!(Disc::msf(75 * 60), (1, 2, 0));
}

#[test]
fn reads_outside_the_disc_are_errors() {
    let dir = temp("range");
    let cue = builder::write_disc(&dir, "r", &tracks()).unwrap();
    let mut d = Disc::open(&cue).unwrap();
    let mut raw = [0u8; RAW_SECTOR];
    assert_eq!(d.read_raw(35, &mut raw), Err(DiscError::OutOfRange(35)));
    assert_eq!(
        d.read_raw(u32::MAX, &mut raw),
        Err(DiscError::OutOfRange(u32::MAX))
    );
}

#[test]
fn a_pregap_command_reads_as_silence_and_shifts_the_track() {
    let dir = temp("pregap");
    std::fs::write(dir.join("a.bin"), vec![1u8; RAW_SECTOR * 4]).unwrap();
    std::fs::write(dir.join("b.bin"), vec![2u8; RAW_SECTOR * 3]).unwrap();
    let cue = "FILE a.bin BINARY\nTRACK 01 AUDIO\nINDEX 01 00:00:00\nFILE b.bin BINARY\nTRACK 02 AUDIO\nPREGAP 00:00:10\nINDEX 01 00:00:00\n";
    std::fs::write(dir.join("p.cue"), cue).unwrap();
    let mut d = Disc::open(&dir.join("p.cue")).unwrap();
    let t = d.tracks().to_vec();
    assert_eq!((t[1].pregap_lba, t[1].start_lba, t[1].sectors), (4, 14, 3));
    assert_eq!(d.lead_out(), 17);
    let mut raw = [9u8; RAW_SECTOR];
    d.read_raw(8, &mut raw).unwrap();
    assert!(raw.iter().all(|&b| b == 0));
    d.read_raw(15, &mut raw).unwrap();
    assert!(raw.iter().all(|&b| b == 2));
}

#[test]
fn index_00_data_belongs_to_the_pregap_of_the_track() {
    let dir = temp("idx0");
    std::fs::write(dir.join("a.bin"), vec![7u8; RAW_SECTOR * 12]).unwrap();
    let cue = "FILE a.bin BINARY\nTRACK 01 AUDIO\nINDEX 01 00:00:00\nTRACK 02 AUDIO\nINDEX 00 00:00:08\nINDEX 01 00:00:10\n";
    std::fs::write(dir.join("i.cue"), cue).unwrap();
    let d = Disc::open(&dir.join("i.cue")).unwrap();
    let t = d.tracks();
    assert_eq!((t[0].sectors, t[1].pregap_lba, t[1].start_lba), (8, 8, 10));
    assert_eq!(d.lead_out(), 12);
}

#[test]
fn a_bin_that_is_not_a_whole_number_of_sectors_is_an_error() {
    let dir = temp("odd");
    std::fs::write(dir.join("a.bin"), vec![0u8; RAW_SECTOR * 2 + 5]).unwrap();
    std::fs::write(
        dir.join("o.cue"),
        "FILE a.bin BINARY\nTRACK 01 AUDIO\nINDEX 01 00:00:00\n",
    )
    .unwrap();
    let err = Disc::open(&dir.join("o.cue")).err().unwrap();
    assert!(err.to_string().contains("a.bin"), "{err}");
}

#[test]
fn a_missing_bin_names_the_file() {
    let dir = temp("missing");
    std::fs::write(
        dir.join("m.cue"),
        "FILE gone.bin BINARY\nTRACK 01 AUDIO\nINDEX 01 00:00:00\n",
    )
    .unwrap();
    let err = Disc::open(&dir.join("m.cue")).err().unwrap();
    assert!(err.to_string().contains("gone.bin"), "{err}");
}

#[test]
fn a_bin_shorter_than_the_cue_implies_is_clamped() {
    let dir = temp("short");
    std::fs::write(dir.join("a.bin"), vec![3u8; RAW_SECTOR * 5]).unwrap();
    let cue =
        "FILE a.bin BINARY\nTRACK 01 AUDIO\nINDEX 01 00:00:00\nTRACK 02 AUDIO\nINDEX 01 01:00:00\n";
    std::fs::write(dir.join("s.cue"), cue).unwrap();
    let mut d = Disc::open(&dir.join("s.cue")).unwrap();
    assert!(d.lead_out() <= 5);
    let mut raw = [0u8; RAW_SECTOR];
    assert!(d.read_raw(d.lead_out(), &mut raw).is_err());
}

#[test]
fn cue_paths_stay_inside_the_cue_folder() {
    let dir = temp("escape");
    std::fs::write(
        dir.join("e.cue"),
        "FILE \"../x.bin\" BINARY\nTRACK 01 AUDIO\nINDEX 01 00:00:00\n",
    )
    .unwrap();
    assert!(matches!(
        Disc::open(&dir.join("e.cue")),
        Err(DiscError::PathEscape(_))
    ));
}

#[test]
fn chd_and_unknown_extensions_are_refused_clearly() {
    let dir = temp("ext");
    assert!(matches!(
        Disc::open(&dir.join("x.chd")),
        Err(DiscError::Unsupported(_))
    ));
    assert!(matches!(
        Disc::open(&dir.join("x.zip")),
        Err(DiscError::Unsupported(_))
    ));
}
