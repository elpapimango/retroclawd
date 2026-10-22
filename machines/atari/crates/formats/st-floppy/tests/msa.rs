//! `.msa` images (DrCoolZic's description of Damien Burke's format, see `st_floppy::msa`).

use st_floppy::Image;
use st_floppy::msa::{self, MsaError};

fn image(cyl: u16, sides: u16, spt: u16, fill: impl Fn(usize) -> u8) -> Image {
    let total = (cyl * sides * spt) as usize * 512;
    let mut b: Vec<u8> = (0..total).map(fill).collect();
    // A plausible BPB so Image::parse accepts it.
    b[11..13].copy_from_slice(&512u16.to_le_bytes());
    b[19..21].copy_from_slice(&(cyl * sides * spt).to_le_bytes());
    b[24..26].copy_from_slice(&spt.to_le_bytes());
    b[26..28].copy_from_slice(&sides.to_le_bytes());
    Image::parse(&b).unwrap()
}

#[test]
fn round_trips_every_geometry() {
    for (cyl, sides, spt) in [(80, 2, 9), (80, 1, 9), (80, 2, 10), (82, 2, 10), (40, 1, 9)] {
        // Mixed content: runs, literal $E5 bytes and noise.
        let img = image(cyl, sides, spt, |i| match i % 997 {
            0..=300 => 0x00,
            301..=400 => 0xE5,
            n => (n * 31 + i / 7) as u8,
        });
        let bytes = msa::write(&img);
        assert_eq!(msa::parse(&bytes).unwrap(), img, "{cyl}x{sides}x{spt}");
    }
}

#[test]
fn a_real_e5_byte_is_encoded_as_a_run_of_one() {
    // Spec: a real $E5 is $E5 $E5 $0001; a track of mostly zeros compresses, with a lone $E5 in it.
    let img = image(80, 2, 9, |i| if i == 5000 { 0xE5 } else { 0 });
    let bytes = msa::write(&img);
    assert!(bytes.windows(4).any(|w| w == [0xE5, 0xE5, 0x00, 0x01]));
    assert_eq!(msa::parse(&bytes).unwrap(), img);
}

#[test]
fn incompressible_track_is_stored_raw() {
    let img = image(80, 2, 9, |i| (i.wrapping_mul(2654435761) >> 7) as u8 | 1);
    let bytes = msa::write(&img);
    // Header 10 bytes, then the first track's length word is the raw size.
    assert_eq!(u16::from_be_bytes([bytes[10], bytes[11]]), 9 * 512);
}

#[test]
fn the_header_is_big_endian() {
    let img = image(80, 2, 9, |_| 0);
    let b = msa::write(&img);
    assert_eq!(&b[..10], &[0x0E, 0x0F, 0, 9, 0, 1, 0, 0, 0, 79]);
}

#[test]
fn rle_run_longer_than_the_track_is_an_error() {
    let mut b = vec![0x0E, 0x0F, 0, 9, 0, 0, 0, 0, 0, 0];
    b.extend([0, 4, 0xE5, 0xAA, 0xFF, 0xFF]); // one track, a run of 65535
    assert!(matches!(msa::parse(&b), Err(MsaError::Track { .. })));
}

#[test]
fn a_track_that_expands_short_is_an_error() {
    let mut b = vec![0x0E, 0x0F, 0, 9, 0, 0, 0, 0, 0, 0];
    b.extend([0, 4, 0xE5, 0xAA, 0x00, 0x10]);
    assert!(matches!(msa::parse(&b), Err(MsaError::Track { .. })));
}

#[test]
fn truncated_header_is_an_error() {
    assert_eq!(msa::parse(&[0x0E, 0x0F, 0, 9]), Err(MsaError::TooShort));
}

#[test]
fn a_wrong_id_is_an_error() {
    let mut b = vec![0x60, 0x1C, 0, 9, 0, 0, 0, 0, 0, 0];
    assert_eq!(msa::parse(&b), Err(MsaError::BadId(0x601C)));
    b[0] = 0x0E;
    b[1] = 0x0F;
    assert!(msa::parse(&b).is_err());
}

#[test]
fn a_geometry_beyond_the_caps_is_an_error() {
    for h in [
        [0x0E, 0x0F, 0, 0, 0, 0, 0, 0, 0, 0],       // no sectors
        [0x0E, 0x0F, 0, 9, 0, 2, 0, 0, 0, 0],       // 3 sides
        [0x0E, 0x0F, 0, 9, 0, 1, 0, 5, 0, 4],       // first after last
        [0x0E, 0x0F, 0, 9, 0, 1, 0, 0, 0xFF, 0xFF], // 65536 tracks
        [0x0E, 0x0F, 0, 200, 0, 1, 0, 0, 0, 79],    // sectors that do not fit a track
    ] {
        assert!(msa::parse(&h).is_err(), "{h:02X?}");
    }
}

#[test]
fn the_file_ending_inside_a_track_is_an_error() {
    let img = image(80, 2, 9, |_| 0);
    let b = msa::write(&img);
    for cut in [11, 20, b.len() / 2, b.len() - 1] {
        assert!(msa::parse(&b[..cut]).is_err(), "cut at {cut}");
    }
}

#[test]
fn parse_of_random_bytes_never_panics() {
    let mut x = 0x1234_5678u32;
    for n in 0..1000
        * std::env::var("RC_SWEEP")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1)
    {
        let len = (n * 13) % 300;
        let mut b: Vec<u8> = (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        if n % 2 == 0 && b.len() >= 2 {
            b[0] = 0x0E;
            b[1] = 0x0F;
        }
        let _ = msa::parse(&b);
    }
}
