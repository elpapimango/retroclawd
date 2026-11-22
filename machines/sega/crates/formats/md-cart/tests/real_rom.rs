//! Checks the parser against a real dump when `RC_MD_ROMS` names a folder holding `altered-beast/ab.smd` (the RomM game
//! "Altered Beast (USA, Europe)", de-interleaved SHA-1 316eb916...). Without it the test prints where it looked and passes.

use md_cart::Cartridge;

#[test]
fn altered_beast_smd_parses_and_its_checksum_matches() {
    let Ok(dir) = std::env::var("RC_MD_ROMS") else {
        eprintln!("RC_MD_ROMS not set: skipping");
        return;
    };
    let path = std::path::Path::new(&dir).join("altered-beast/ab.smd");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("{} not found: skipping", path.display());
        return;
    };
    let cart = Cartridge::parse(&bytes).unwrap();
    assert!(cart.was_smd);
    assert!(cart.header_valid);
    assert_eq!(cart.rom.len(), 524_288);
    assert!(cart.checksum_ok(), "{:04X}", cart.header.checksum);
    assert!(cart.header.regions.usa || cart.header.regions.europe);
}
