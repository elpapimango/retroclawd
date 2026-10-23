//! Which accesses raise a bus error, one test per region, each with the source of its row in
//! `docs/hardware-facts.md`; and the rule that an address no source covers still stops the machine with a report
//! instead of answering something invented.

mod common;
use common::{BRA_SELF, config, move_b, rom};

use atari_st::{AtariSt, RamSize, StBus, StopReason};
use machine::Machine;
use mc68000::{BusFault, FunctionCode, M68kBus, Size};

const SUPER: FunctionCode = FunctionCode::SUPERVISOR_DATA;
const USER: FunctionCode = FunctionCode::USER_DATA;

fn bus() -> StBus {
    StBus::new(&config(rom(&[BRA_SELF], &[]), RamSize::M1)).unwrap()
}

/// [H] *Internals* p.55 and the TOS documentation: a user mode access to the hardware registers, or below `$800`, is
/// a bus error.
#[test]
fn access_to_the_io_area_in_user_mode_raises_a_bus_error() {
    let mut b = bus();
    for addr in [0xFF8001, 0xFF8800, 0xFFFA01, 0xFFFC00] {
        assert_eq!(b.read(addr, Size::Byte, USER), Err(BusFault), "{addr:#x}");
        assert_eq!(
            b.write(addr, Size::Byte, 0, USER),
            Err(BusFault),
            "{addr:#x}"
        );
    }
    assert!(b.stop().is_none(), "a bus error is not a stop");
}

#[test]
fn access_to_the_vectors_in_user_mode_raises_a_bus_error() {
    let mut b = bus();
    assert_eq!(b.read(0x7FE, Size::Word, USER), Err(BusFault));
    assert_eq!(b.write(0x400, Size::Byte, 1, USER), Err(BusFault));
    assert!(b.read(0x800, Size::Word, USER).is_ok());
}

/// [H] *Internals* p.96: "a write access to these addresses creates a bus error. The GLUE takes care of this."
#[test]
fn a_write_to_the_cartridge_window_raises_a_bus_error() {
    let mut b = bus();
    assert_eq!(b.write(0xFA0000, Size::Word, 0, SUPER), Err(BusFault));
    assert_eq!(b.write(0xFBFFFF, Size::Byte, 0, SUPER), Err(BusFault));
    assert!(
        b.read(0xFA0000, Size::Word, SUPER).is_ok(),
        "a read is not an error"
    );
}

/// [O] Hatari, `--memsize 4`: "Bus error reading at address $4fffff"; the same decode for every RAM size here.
#[test]
fn access_above_the_ram_decode_raises_a_bus_error() {
    let mut b = bus();
    for addr in [0x400000, 0x4FFFFF, 0x800000, 0xF00000, 0xF9FFFF] {
        assert_eq!(b.read(addr, Size::Byte, SUPER), Err(BusFault), "{addr:#x}");
        assert_eq!(
            b.write(addr, Size::Byte, 0, SUPER),
            Err(BusFault),
            "{addr:#x}"
        );
    }
}

/// [H?] `$FF0000-$FF7FFF` is below the I/O area: nothing is decoded there.
#[test]
fn access_between_the_rom_and_the_io_area_raises_a_bus_error() {
    let mut b = bus();
    for addr in [0xFF0000, 0xFF7FFE] {
        assert_eq!(b.read(addr, Size::Word, SUPER), Err(BusFault), "{addr:#x}");
    }
}

/// [O] Hatari io: "Bus error reading at address $ffff8006" and "$ffff8a3c" (no Blitter on the ST).
#[test]
fn the_documented_unmapped_io_addresses_raise_a_bus_error() {
    let mut b = bus();
    assert_eq!(b.read(0xFF8006, Size::Word, SUPER), Err(BusFault));
    assert_eq!(b.read(0xFF8A3C, Size::Word, SUPER), Err(BusFault));
    assert!(b.stop().is_none());
}

/// No source covers `$FF9000`: the machine stops with the report and names the missing fact, and what the program
/// then sees is `$FF`, never an invented register value.
#[test]
fn an_unsourced_region_still_stops_with_the_report() {
    let mut b = bus();
    assert_eq!(b.read(0xFF9000, Size::Byte, SUPER), Ok(0xFF));
    let stop = b.stop().expect("the machine stopped");
    assert_eq!(stop.reason, StopReason::NoFact);
    assert_eq!(stop.addr, 0xFF9000);
}

/// The exception itself: a supervisor program writes to the cartridge window; the 68000 takes vector 2 with the group
/// 0 stack frame (14 bytes: status word, the access address, the instruction word, SR, PC), and the handler runs.
#[test]
fn the_cpu_takes_the_bus_error_vector_with_the_documented_frame() {
    let handler = 0xFC_0100u32;
    let mut program = Vec::new();
    // MOVE.L #handler,$0008.W (the bus error vector is in RAM: the ROM mirror covers only the first 8 bytes)
    program.extend([0x21FC, (handler >> 16) as u16, handler as u16, 0x0008]);
    // MOVE.W #1,$FA0000.L
    program.extend([0x33FC, 0x0001, 0x00FA, 0x0000]);
    program.push(BRA_SELF);
    let mut at_handler = Vec::new();
    at_handler.extend(move_b(0x5A, 0x0600));
    at_handler.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&program, &[(0x100, &at_handler)]), RamSize::M1)).unwrap();
    for _ in 0..60 {
        m.step();
    }
    assert_eq!(m.bus().peek(0x600), 0x5A, "the handler ran");
    // The stack pointer starts at $8000 and the frame is 14 bytes: the access address is the long at +2.
    let frame: Vec<u8> = (0x7FF2..0x8000).map(|a| m.bus().peek(a)).collect();
    assert_eq!(
        &frame[2..6],
        &[0x00, 0xFA, 0x00, 0x00],
        "the address that faulted"
    );
    assert!(m.io_stop().is_none());
}
