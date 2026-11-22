//! An image read back from a serialised state is held to the same rules as one parsed from a file.

use st_floppy::Image;

fn encode(image: &Image) -> Vec<u8> {
    bincode::serde::encode_to_vec(image, bincode::config::standard()).unwrap()
}

fn decode(bytes: &[u8]) -> Result<Image, String> {
    bincode::serde::decode_from_slice::<Image, _>(bytes, bincode::config::standard())
        .map(|(i, _)| i)
        .map_err(|e| e.to_string())
}

#[test]
fn an_image_round_trips_through_a_state() {
    let img = Image::parse(&vec![0x5A; 737_280]).unwrap();
    assert_eq!(decode(&encode(&img)).unwrap(), img);
}

#[test]
fn a_geometry_that_does_not_match_the_data_is_refused() {
    let img = Image::parse(&vec![0x5A; 737_280]).unwrap();
    let mut b = encode(&img);
    // The geometry is the first three bytes (cylinders, sides, sectors): claim 10 sectors for 9 sectors of data.
    assert_eq!(&b[..3], &[80, 2, 9]);
    b[2] = 10;
    assert!(decode(&b).is_err());
}

#[test]
fn an_impossible_geometry_is_refused() {
    let img = Image::parse(&vec![0x5A; 737_280]).unwrap();
    for bad in [[0u8, 2, 9], [80, 0, 9], [80, 3, 9], [80, 2, 0], [255, 2, 9]] {
        let mut b = encode(&img);
        b[..3].copy_from_slice(&bad);
        assert!(decode(&b).is_err(), "{bad:?}");
    }
}
