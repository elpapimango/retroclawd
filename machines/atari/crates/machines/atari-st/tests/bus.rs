//! The ST memory map as the CPU sees it (tests against `StBus` through the 68000 bus trait).

use atari_st::{Accuracy, Config, RamSize, Region, StBus};
use mc68000::{FunctionCode, M68kBus, Size};

const SUPER: FunctionCode = FunctionCode::SUPERVISOR_DATA;
const USER: FunctionCode = FunctionCode::USER_DATA;

/// A synthetic 192 KB ROM: `60 2E 01 04`, then the reset PC `$00FC0030` at offset 4, then a counting pattern.
fn rom() -> Vec<u8> {
    let mut rom: Vec<u8> = (0..196_608u32).map(|i| i as u8).collect();
    rom[0..4].copy_from_slice(&[0x60, 0x2E, 0x01, 0x04]);
    rom[4..8].copy_from_slice(&0x00FC_0030u32.to_be_bytes());
    rom
}

fn bus() -> StBus {
    StBus::new(&Config {
        ram: RamSize::M1,
        region: Region::Pal,
        accuracy: Accuracy::Fast,
        rom: rom(),
        trace_limit: 0,
    })
    .expect("valid config")
}

#[test]
fn rom_reads_big_endian_at_fc0000() {
    let mut b = bus();
    assert_eq!(b.read(0xFC0000, Size::Word, SUPER), Ok(0x602E));
    assert_eq!(b.read(0xFC0002, Size::Word, SUPER), Ok(0x0104));
    assert_eq!(b.read(0xFC0001, Size::Byte, SUPER), Ok(0x2E));
}

#[test]
fn reset_vectors_come_from_rom_at_address_zero() {
    let mut b = bus();
    let long = |b: &mut StBus, a: u32| {
        let hi = b
            .read(a, Size::Word, FunctionCode::SUPERVISOR_PROGRAM)
            .unwrap();
        let lo = b
            .read(a + 2, Size::Word, FunctionCode::SUPERVISOR_PROGRAM)
            .unwrap();
        hi << 16 | lo
    };
    assert_eq!(long(&mut b, 0), 0x602E_0104);
    assert_eq!(long(&mut b, 4), 0x00FC_0030);
}

#[test]
fn ram_round_trips_byte_word_long_big_endian() {
    let mut b = bus();
    b.write(0x1000, Size::Word, 0x1122, SUPER).unwrap();
    b.write(0x1002, Size::Word, 0x3344, SUPER).unwrap();
    let bytes: Vec<u32> = (0..4)
        .map(|i| b.read(0x1000 + i, Size::Byte, SUPER).unwrap())
        .collect();
    assert_eq!(bytes, [0x11, 0x22, 0x33, 0x44]);
    b.write(0x1001, Size::Byte, 0xAB, SUPER).unwrap();
    assert_eq!(b.read(0x1000, Size::Word, SUPER), Ok(0x11AB));
}

#[test]
fn addresses_wrap_to_24_bits() {
    let mut b = bus();
    b.write(0x1000, Size::Word, 0xBEEF, SUPER).unwrap();
    assert_eq!(b.read(0xFF00_1000, Size::Word, SUPER), Ok(0xBEEF));
}

#[test]
fn user_mode_access_below_0x800_is_a_bus_error() {
    let mut b = bus();
    assert!(b.read(0x7FE, Size::Word, USER).is_err());
    assert!(b.write(0x400, Size::Byte, 1, USER).is_err());
    assert_eq!(b.read(0x800, Size::Word, USER), Ok(0));
    assert_eq!(b.read(0x7FE, Size::Word, SUPER), Ok(0));
}

#[test]
fn user_mode_access_to_io_is_a_bus_error() {
    let mut b = bus();
    assert!(b.read(0xFF8001, Size::Byte, USER).is_err());
    assert!(
        b.stop().is_none(),
        "a refused user access is not an I/O stop"
    );
}

#[test]
fn nonexistent_memory_is_a_bus_error_in_supervisor_mode() {
    let mut b = bus();
    assert!(b.read(0x400000, Size::Word, SUPER).is_err());
    assert!(b.read(0xF00000, Size::Word, SUPER).is_err());
}

#[test]
fn first_io_access_records_a_stop() {
    let mut b = bus();
    assert!(b.read(0xFF8001, Size::Byte, SUPER).is_ok());
    let stop = b.stop().expect("stop recorded");
    assert_eq!(stop.addr, 0xFF8001);
    assert!(!stop.write);
    b.write(0xFF8240, Size::Word, 0x777, SUPER).unwrap();
    assert_eq!(b.stop().unwrap().addr, 0xFF8001, "the first stop is kept");
}

#[test]
fn debug_peek_has_no_side_effects() {
    let b = bus();
    assert_eq!(b.peek(0xFC0000), 0x60);
    assert_eq!(b.peek(0xFF8001), 0xFF);
    assert!(b.stop().is_none());
}

#[test]
fn a_rom_of_the_wrong_size_is_refused_with_the_sizes() {
    let err = StBus::new(&Config {
        ram: RamSize::M1,
        region: Region::Pal,
        accuracy: Accuracy::Fast,
        rom: vec![0; 262_144],
        trace_limit: 0,
    })
    .unwrap_err();
    assert!(err.contains("196608") && err.contains("262144"), "{err}");
    assert!(err.contains("STE"), "{err}");
}

fn config(rom: Vec<u8>) -> Config {
    Config {
        ram: RamSize::M1,
        region: Region::Pal,
        accuracy: Accuracy::Fast,
        rom,
        trace_limit: 0,
    }
}

#[test]
fn a_byte_swapped_rom_is_refused_as_swapped() {
    let swapped: Vec<u8> = rom().chunks(2).flat_map(|w| [w[1], w[0]]).collect();
    let err = StBus::new(&config(swapped)).unwrap_err();
    assert!(err.contains("byte-swapped"), "{err}");
}

#[test]
fn a_reset_pc_outside_the_rom_is_refused() {
    let mut r = rom();
    r[4..8].copy_from_slice(&0x0000_3000u32.to_be_bytes());
    let err = StBus::new(&config(r)).unwrap_err();
    assert!(err.contains("$003000") && err.contains("$FC0000"), "{err}");
    let mut r = rom();
    r[4..8].copy_from_slice(&0x00FC_0031u32.to_be_bytes());
    let err = StBus::new(&config(r)).unwrap_err();
    assert!(err.contains("odd"), "{err}");
}
