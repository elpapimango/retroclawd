//! Feeds arbitrary bytes to `load_state` of a machine built on a tiny test ROM. Any input must be refused or loaded,
//! never panic; a loaded state is run for a few thousand instructions as well.

#![no_main]

use atari_st::state;
use atari_st::{Accuracy, AtariSt, Config, Monitor, RamSize, Region};
use libfuzzer_sys::fuzz_target;
use machine::Machine;

fn rom() -> Vec<u8> {
    let mut rom = vec![0u8; atari_st::ROM_SIZE];
    rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00FC_0030u32.to_be_bytes());
    rom[0x30..0x32].copy_from_slice(&0x60FEu16.to_be_bytes());
    rom
}

fn machine() -> AtariSt {
    let rom = rom();
    AtariSt::new(Config {
        ram: RamSize::K256,
        region: Region::Pal,
        accuracy: Accuracy::Fast,
        monitor: Monitor::Colour,
        rom,
        trace_limit: 0,
        io_log_limit: 0,
    })
    .unwrap()
}

fuzz_target!(|data: &[u8]| {
    let mut m = machine();
    // Input that is not an envelope is wrapped in a valid one (right CRC, right firmware), so the payload decoder and
    // the checks behind it get exercised too, not only the magic number.
    let bytes = if data.starts_with(b"RCST") {
        data.to_vec()
    } else {
        let header = state::Header {
            version_tag: "fuzz".to_string(),
            accuracy: 0,
            firmware_sha1: state::sha1_of(&rom()),
            drives: Default::default(),
        };
        state::encode(&header, data)
    };
    if m.load_state(&bytes).is_ok() {
        for _ in 0..2_000 {
            m.step();
        }
    }
});
