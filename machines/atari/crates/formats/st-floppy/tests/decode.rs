//! Reading an MFM track back into sectors: what the controller wrote (WRITE SECTOR, WRITE TRACK) becomes image data.
//! Format: WD177X data sheet p.1-18, the same layout `Image::track` synthesises.

use st_floppy::decode::{DecodeError, sectors_from_mfm};
use st_floppy::{Image, TRACK_BYTES};

fn image() -> Image {
    let bytes: Vec<u8> = (0..737_280usize).map(|i| (i * 7 + i / 512) as u8).collect();
    Image::parse(&bytes).unwrap()
}

#[test]
fn mfm_of_a_synthesised_track_decodes_to_the_same_sectors() {
    let img = image();
    let sectors = sectors_from_mfm(&img.track(3, 1)).unwrap();
    assert_eq!(sectors.len(), 9);
    for s in &sectors {
        assert_eq!((s.cylinder, s.side), (3, 1));
        assert_eq!(s.data.as_slice(), img.sector(3, 1, s.record).unwrap());
    }
    let records: Vec<u8> = sectors.iter().map(|s| s.record).collect();
    assert_eq!(records, (1..=9).collect::<Vec<u8>>());
}

#[test]
fn a_bad_data_crc_is_an_error_naming_the_sector() {
    let mut t = image().track(0, 0);
    // The first data field: find the first 0xFB mark and flip a data byte after it.
    let at = t.iter().position(|&b| b == 0xFB).unwrap() + 10;
    t[at] ^= 0x01;
    match sectors_from_mfm(&t) {
        Err(DecodeError::DataCrc { record, .. }) => assert_eq!(record, 1),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_bad_id_crc_is_an_error() {
    let mut t = image().track(0, 0);
    let at = t.iter().position(|&b| b == 0xFE).unwrap() + 2; // the R byte
    t[at] ^= 0x01;
    assert!(matches!(
        sectors_from_mfm(&t),
        Err(DecodeError::IdCrc { .. })
    ));
}

#[test]
fn a_track_with_no_address_marks_decodes_to_no_sectors() {
    assert_eq!(sectors_from_mfm(&vec![0x4E; TRACK_BYTES]).unwrap(), vec![]);
    assert_eq!(sectors_from_mfm(&[]).unwrap(), vec![]);
}

#[test]
fn a_data_field_cut_by_the_end_of_the_track_is_dropped_not_a_panic() {
    let t = image().track(0, 0);
    for cut in (0..t.len()).step_by(97) {
        let _ = sectors_from_mfm(&t[..cut]);
    }
}

#[test]
fn applying_a_decoded_track_writes_the_image_and_reports_what_changed() {
    let mut img = image();
    let mut t = img.track(2, 0);
    // Rewrite sector 4's data (and its CRC) the way the controller would.
    let mark = t
        .iter()
        .enumerate()
        .filter(|&(i, &b)| b == 0xFB && t[i - 1] == 0x1A1)
        .nth(3)
        .unwrap()
        .0;
    for b in &mut t[mark + 1..mark + 513] {
        *b = 0x5A;
    }
    let crc = t[mark - 3..mark + 513]
        .iter()
        .map(|&b| b as u8)
        .fold(0xFFFFu16, wd1770::crc_byte);
    t[mark + 513] = crc >> 8;
    t[mark + 514] = crc & 0xFF;
    assert_eq!(img.apply_track(2, 0, &t).unwrap(), 1);
    assert!(img.sector(2, 0, 4).unwrap().iter().all(|&b| b == 0x5A));
    // A track that decodes to what the image already holds changes nothing.
    assert_eq!(img.apply_track(2, 0, &img.track(2, 0)).unwrap(), 0);
}

#[test]
fn sectors_outside_the_geometry_are_ignored() {
    // A 10-sector format written to a 9-sector image: record 10 has nowhere to go, the nine others land.
    let mut img = image();
    let ten = Image::parse(&vec![0xE5u8; 80 * 2 * 10 * 512]).unwrap();
    assert_eq!(img.apply_track(0, 0, &ten.track(0, 0)).unwrap(), 9);
    assert!(img.sector(0, 0, 9).unwrap().iter().all(|&b| b == 0xE5));
}
