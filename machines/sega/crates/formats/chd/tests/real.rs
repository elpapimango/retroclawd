//! Reads a real CHD (`RC_CHD=path`) completely and compares the SHA-1 of its decoded hunks with the one in its header, which
//! checks the map, every codec used, the subcode and the regenerated error correction at once. Ignored by default: the image is
//! a game and never committed.

use sha1::{Digest, Sha1};

#[test]
#[ignore]
fn every_hunk_decodes_to_the_checksum_in_the_header() {
    let Ok(path) = std::env::var("RC_CHD") else {
        return;
    };
    let mut chd = chd::Chd::open(std::path::Path::new(&path)).unwrap();
    println!("tracks: {:#?}", chd.tracks());
    let mut hash = Sha1::new();
    for hunk in 0..chd.hunk_count() {
        match chd.hunk(hunk) {
            Ok(bytes) => hash.update(bytes),
            Err(e) => panic!("hunk {hunk}: {e}"),
        }
    }
    let got: [u8; 20] = hash.finalize().into();
    assert_eq!(
        got,
        chd.raw_sha1(),
        "decoded data differs from the header's checksum"
    );
}
