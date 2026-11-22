//! Arbitrary bytes as an `.msa` file: refused or parsed, never a panic; what parses writes back and parses again to the
//! same image.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(image) = st_floppy::msa::parse(data) {
        let again = st_floppy::msa::write(&image);
        let back = st_floppy::msa::parse(&again).expect("what we wrote parses");
        assert_eq!(back, image);
    }
});
