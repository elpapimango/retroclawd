//! `.st` images: geometry from the boot sector's BPB (*Atari ST Internals* pp.178-179) or the file size, sectors, and
//! MFM tracks in the WD177X data sheet's double-density format (page 1-18).

use st_floppy::{Error, Geometry, Image, TRACK_BYTES};

/// A blank image of `cyl × sides × spt` sectors, sector `n` of the disk filled with `n as u8`, and a boot sector
/// holding a BPB for that geometry when `bpb` is true.
fn image(cyl: u16, sides: u16, spt: u16, bpb: bool) -> Vec<u8> {
    let total = cyl * sides * spt;
    let mut img: Vec<u8> = (0..total as usize)
        .flat_map(|n| std::iter::repeat_n(n as u8, 512))
        .collect();
    if bpb {
        set_bpb(&mut img, 512, total, spt, sides);
    }
    img
}

fn set_bpb(img: &mut [u8], eps: u16, total: u16, spt: u16, sides: u16) {
    img[11..13].copy_from_slice(&eps.to_le_bytes());
    img[19..21].copy_from_slice(&total.to_le_bytes());
    img[24..26].copy_from_slice(&spt.to_le_bytes());
    img[26..28].copy_from_slice(&sides.to_le_bytes());
}

#[test]
fn parses_720k_by_bpb() {
    let img = Image::parse(&image(80, 2, 9, true)).unwrap();
    assert_eq!(
        img.geometry,
        Geometry {
            cylinders: 80,
            sides: 2,
            sectors: 9
        }
    );
    // Sector order: cylinder, then side, then sector.
    assert_eq!(img.sector(0, 1, 1).unwrap()[0], 9);
    assert_eq!(img.sector(1, 0, 3).unwrap()[0], 20);
    assert!(img.sector(0, 0, 0).is_none(), "sectors count from 1");
    assert!(img.sector(80, 0, 1).is_none());
}

/// A BPB that is not plausible (here: all zeros) leaves the size to decide.
#[test]
fn falls_back_to_size_table() {
    for (cyl, sides, spt) in [
        (80, 1, 9),
        (80, 1, 10),
        (80, 2, 9),
        (80, 2, 10),
        (82, 2, 10),
    ] {
        let g = Image::parse(&image(cyl, sides, spt, false))
            .unwrap()
            .geometry;
        assert_eq!(
            (g.cylinders as u16, g.sides as u16, g.sectors as u16),
            (cyl, sides, spt)
        );
    }
}

/// A BPB is used when it agrees with the size; a geometry that only fits the BPB (72 × 2 × 10 = 720K) works too.
#[test]
fn a_consistent_bpb_wins_over_the_table() {
    let mut img = image(80, 2, 9, false);
    set_bpb(&mut img, 512, 1440, 10, 2);
    let g = Image::parse(&img).unwrap().geometry;
    assert_eq!((g.cylinders, g.sides, g.sectors), (72, 2, 10));
}

#[test]
fn bpb_disagreeing_with_size_is_an_error_naming_both() {
    let mut img = image(80, 2, 9, false);
    set_bpb(&mut img, 512, 720, 9, 1);
    let err = Image::parse(&img).unwrap_err();
    assert!(matches!(err, Error::Geometry { .. }), "{err:?}");
    let text = err.to_string();
    assert!(text.contains("720") && text.contains("1440"), "{text}");
}

#[test]
fn rejects_empty_oversize_and_odd_sizes() {
    assert!(Image::parse(&[]).is_err());
    assert!(Image::parse(&vec![0; 2 * 1024 * 1024 + 512]).is_err());
    assert!(Image::parse(&vec![0; 737_281]).is_err());
    assert!(
        Image::parse(&vec![0; 1000 * 512]).is_err(),
        "no geometry fits"
    );
}

/// Every truncation of a 720K image in 4 KiB steps, and 1000 pseudo-random boot sectors: errors, never a panic.
#[test]
fn garbled_and_truncated_images_never_panic() {
    let full = image(80, 2, 9, true);
    for len in (0..full.len()).step_by(4096) {
        let _ = Image::parse(&full[..len]);
    }
    let mut seed = 0x1234_5678u64;
    for _ in 0..1000 {
        let mut img = full.clone();
        for b in &mut img[..32] {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *b = (seed >> 33) as u8;
        }
        if let Ok(i) = Image::parse(&img) {
            let g = i.geometry;
            for (c, s) in [(0, 0), (g.cylinders - 1, g.sides - 1), (g.cylinders, 0)] {
                assert_eq!(i.track(c, s).len(), TRACK_BYTES);
            }
        }
    }
}

/// The test's own reader of a synthesized track: the ID fields and data fields with their CRCs (CRC-CCITT preset
/// `$FFFF`, over the three `A1`s, the mark and the field).
fn decode(track: &[u16]) -> Vec<([u8; 4], Vec<u8>)> {
    let crc = |bytes: &[u8]| bytes.iter().fold(0xFFFFu16, |c, &b| wd1770::crc_byte(c, b));
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 < track.len() {
        if track[i..i + 3] == [0x1A1; 3] && track[i + 3] == 0xFE {
            let id: Vec<u8> = track[i..i + 10].iter().map(|&b| b as u8).collect();
            assert_eq!(crc(&id), 0, "ID CRC");
            let mut j = i + 10;
            while !(track[j..j + 3] == [0x1A1; 3] && track[j + 3] == 0xFB) {
                j += 1;
            }
            let size = 128usize << id[7];
            let field: Vec<u8> = track[j..j + 4 + size + 2]
                .iter()
                .map(|&b| b as u8)
                .collect();
            assert_eq!(crc(&field), 0, "data CRC");
            out.push(([id[4], id[5], id[6], id[7]], field[4..4 + size].to_vec()));
            i = j + 4 + size + 2;
        } else {
            i += 1;
        }
    }
    out
}

#[test]
fn track_has_9_ids_and_data_with_good_crcs() {
    let img = Image::parse(&image(80, 2, 9, true)).unwrap();
    let fields = decode(&img.track(3, 1));
    assert_eq!(fields.len(), 9);
    for (n, (id, data)) in fields.iter().enumerate() {
        assert_eq!(*id, [3, 1, n as u8 + 1, 2], "C H R N");
        assert_eq!(data.len(), 512);
        assert_eq!(data[100], ((3 * 2 + 1) * 9 + n) as u8);
    }
}

#[test]
fn ten_sector_tracks_fit_too() {
    let img = Image::parse(&image(80, 2, 10, true)).unwrap();
    assert_eq!(decode(&img.track(79, 0)).len(), 10);
}

#[test]
fn eleven_sectors_do_not_fit_a_track() {
    let err = Image::parse(&image(80, 2, 11, true)).unwrap_err();
    assert!(err.to_string().contains("11"), "{err}");
}

#[test]
fn track_length_is_one_revolution_at_the_cpu_clock() {
    let img = Image::parse(&image(80, 1, 9, true)).unwrap();
    assert_eq!(img.track(0, 0).len(), 6267);
    assert_eq!(TRACK_BYTES, 6267);
}

#[test]
fn round_trip_bytes() {
    let bytes = image(80, 2, 9, true);
    let mut img = Image::parse(&bytes).unwrap();
    assert_eq!(img.to_bytes(), bytes);
    img.write_sector(5, 1, 2, &[0xAB; 512]).unwrap();
    assert_eq!(img.sector(5, 1, 2).unwrap()[511], 0xAB);
    assert!(img.write_sector(5, 1, 2, &[0; 10]).is_err());
    assert!(img.write_sector(99, 1, 2, &[0; 512]).is_err());
}
