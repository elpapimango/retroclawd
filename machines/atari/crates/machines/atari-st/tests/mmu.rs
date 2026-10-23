//! The memory controller: the configuration register at `$FF8001` and how CPU addresses reach the fitted banks
//! (facts: `docs/hardware-facts.md`, "MMU").

use atari_st::{Accuracy, BankSize, Config, Monitor, RamSize, Region, StBus, installed_banks};
use mc68000::{FunctionCode, M68kBus, Size};

const SUPER: FunctionCode = FunctionCode::SUPERVISOR_DATA;

fn rom() -> Vec<u8> {
    let mut rom = vec![0u8; 196_608];
    rom[0..4].copy_from_slice(&[0x60, 0x2E, 0x01, 0x04]);
    rom[4..8].copy_from_slice(&0x00FC_0030u32.to_be_bytes());
    rom
}

fn bus(ram: RamSize) -> StBus {
    StBus::new(&Config {
        ram,
        region: Region::Pal,
        accuracy: Accuracy::Fast,
        monitor: Monitor::Colour,
        rom: rom(),
        trace_limit: 0,
        io_log_limit: 0,
    })
    .unwrap()
}

fn configure(b: &mut StBus, value: u8) {
    b.write(0xFF8001, Size::Byte, value as u32, SUPER).unwrap();
}

#[test]
fn config_register_keeps_the_low_four_bits() {
    let mut b = bus(RamSize::M1);
    assert_eq!(
        b.read(0xFF8001, Size::Byte, SUPER),
        Ok(0x00),
        "power-on value [O]"
    );
    configure(&mut b, 0xFF);
    assert_eq!(b.read(0xFF8001, Size::Byte, SUPER), Ok(0x0F));
    assert!(b.stop().is_none());
}

#[test]
fn installed_banks_per_ram_option() {
    use BankSize::*;
    assert_eq!(installed_banks(RamSize::K256), [K128, K128]);
    assert_eq!(installed_banks(RamSize::K512), [K512, None]);
    assert_eq!(installed_banks(RamSize::M1), [K512, K512]);
    assert_eq!(installed_banks(RamSize::M2), [M2, None]);
    assert_eq!(installed_banks(RamSize::M2_5), [M2, K512]);
    assert_eq!(installed_banks(RamSize::M4), [M2, M2]);
}

#[test]
fn bank_1_starts_after_the_configured_bank_0() {
    let mut b = bus(RamSize::M1);
    configure(&mut b, 0x05); // 512K + 512K
    b.write(0x1000, Size::Word, 0x1111, SUPER).unwrap();
    b.write(0x81000, Size::Word, 0x2222, SUPER).unwrap();
    assert_eq!(b.read(0x1000, Size::Word, SUPER), Ok(0x1111));
    assert_eq!(b.read(0x81000, Size::Word, SUPER), Ok(0x2222));
    // 2M + 512K: bank 1 now starts at $200000; its first cells are the same chips.
    configure(&mut b, 0x09);
    assert_eq!(b.read(0x201000, Size::Word, SUPER), Ok(0x2222));
}

#[test]
fn no_ram_above_the_configured_total() {
    let mut b = bus(RamSize::M1);
    configure(&mut b, 0x05);
    assert_eq!(b.read(0x100000, Size::Word, SUPER), Ok(0xFFFF));
    assert!(b.write(0x100000, Size::Word, 0x1234, SUPER).is_ok());
    assert_eq!(b.read(0x100000, Size::Word, SUPER), Ok(0xFFFF));
    // An empty bank 1 (512K fitted as bank 0 only).
    let mut b = bus(RamSize::K512);
    configure(&mut b, 0x05);
    assert_eq!(b.read(0x81000, Size::Word, SUPER), Ok(0xFFFF));
}

/// [H?] row/column multiplexing (see `mmu.rs`): in a bank configured for 2M, smaller chips ignore the top row and
/// column lines.
#[test]
fn smaller_chips_alias_on_the_top_row_and_column_lines() {
    // 512K fitted (256K-bit chips), configured 2M: A10 (column bit 9) and A20 (row bit 9) are ignored.
    let mut b = bus(RamSize::M1);
    configure(&mut b, 0x0A);
    b.write(0x8, Size::Word, 0xA5A5, SUPER).unwrap();
    assert_eq!(b.read(0x408, Size::Word, SUPER), Ok(0xA5A5));
    assert_eq!(
        b.read(0x80008, Size::Word, SUPER),
        Ok(0),
        "A19 is row bit 8: kept"
    );
    // 128K fitted (64K-bit chips), configured 2M: A9 and A10 ignored.
    let mut b = bus(RamSize::K256);
    configure(&mut b, 0x0A);
    b.write(0x8, Size::Word, 0x5A5A, SUPER).unwrap();
    assert_eq!(b.read(0x208, Size::Word, SUPER), Ok(0x5A5A));
    assert_eq!(b.read(0x40008, Size::Word, SUPER), Ok(0));
    // Configured as fitted: every cell distinct.
    let mut b = bus(RamSize::M1);
    configure(&mut b, 0x05);
    b.write(0x8, Size::Word, 0x1111, SUPER).unwrap();
    assert_eq!(b.read(0x408, Size::Word, SUPER), Ok(0));
}
