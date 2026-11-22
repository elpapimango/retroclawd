//! The video address counter ($FF8205/07/09, read only): the address of the next word the Shifter reads. Fitted to probe
//! P04 (docs/hardware-facts.md, "Timing (Phase 3)").

mod common;
use common::{BRA_SELF, rom};

use atari_st::{Accuracy, AtariSt, Config, Monitor, RamSize, Region, StBus};
use machine::Machine;
use mc68000::{FunctionCode, M68kBus, Size};

const SUPER: FunctionCode = FunctionCode::SUPERVISOR_DATA;

fn bus() -> StBus {
    StBus::new(&Config {
        ram: RamSize::M1,
        region: Region::Pal,
        accuracy: Accuracy::Accurate,
        monitor: Monitor::Colour,
        rom: rom(&[BRA_SELF], &[]),
        trace_limit: 0,
        io_log_limit: 0,
    })
    .unwrap()
}

fn counter(b: &mut StBus) -> u32 {
    let byte = |b: &mut StBus, a: u32| b.read(a, Size::Byte, SUPER).unwrap();
    byte(b, 0xFF8205) << 16 | byte(b, 0xFF8207) << 8 | byte(b, 0xFF8209)
}

/// Runs the bare bus to the clock `t` (internal clocks only).
fn run_to(b: &mut StBus, t: u64) {
    let now = b.now();
    assert!(t >= now);
    b.sync((t - now) as u32);
}

const LINE: u64 = 512;
const FIRST_PICTURE_LINE: u64 = 63;

#[test]
fn the_counter_holds_the_base_above_the_picture() {
    let mut b = bus();
    b.write(0xFF8201, Size::Byte, 0x0F, SUPER).unwrap();
    b.write(0xFF8203, Size::Byte, 0x80, SUPER).unwrap();
    run_to(&mut b, 10 * LINE + 100);
    assert_eq!(counter(&mut b), 0x0F8000);
}

#[test]
fn inside_a_picture_line_the_counter_moves_one_word_every_four_clocks_from_clock_56() {
    let mut b = bus();
    b.write(0xFF8201, Size::Byte, 0x0F, SUPER).unwrap();
    b.write(0xFF8203, Size::Byte, 0x80, SUPER).unwrap();
    let line = FIRST_PICTURE_LINE + 5;
    // The three byte reads take 8 clocks each (a slot of 4 and the cycle), so the low byte is read 16 clocks after the
    // high one: 8 bytes later.
    run_to(&mut b, line * LINE + 56);
    let c = counter(&mut b);
    assert!(
        (0x0F8000 + 5 * 160..=0x0F8000 + 5 * 160 + 8).contains(&c),
        "{c:06X}"
    );
    run_to(&mut b, line * LINE + 56 + 40);
    let c = counter(&mut b);
    assert!(
        (0x0F8000 + 5 * 160 + 20..=0x0F8000 + 5 * 160 + 28).contains(&c),
        "{c:06X}"
    );
}

#[test]
fn after_the_window_the_counter_holds_the_end_of_the_line_and_after_the_picture_its_end() {
    let mut b = bus();
    b.write(0xFF8201, Size::Byte, 0x0F, SUPER).unwrap();
    b.write(0xFF8203, Size::Byte, 0x80, SUPER).unwrap();
    run_to(&mut b, (FIRST_PICTURE_LINE + 3) * LINE + 450);
    assert_eq!(counter(&mut b), 0x0F8000 + 4 * 160);
    run_to(&mut b, 290 * LINE);
    assert_eq!(counter(&mut b), 0x0F8000 + 200 * 160);
}

#[test]
fn a_write_to_the_counter_is_ignored() {
    let mut b = bus();
    run_to(&mut b, 5 * LINE);
    let before = counter(&mut b);
    b.write(0xFF8205, Size::Byte, 0x07, SUPER).unwrap();
    assert_eq!(counter(&mut b), before);
}

#[test]
fn a_program_can_read_the_counter_through_the_machine() {
    // MOVE.B $FF8209.W,$0600.W
    let program = [0x11F8, 0x8209, 0x0600, BRA_SELF];
    let mut m = AtariSt::new(Config {
        ram: RamSize::M1,
        region: Region::Pal,
        accuracy: Accuracy::Accurate,
        monitor: Monitor::Colour,
        rom: rom(&program, &[]),
        trace_limit: 0,
        io_log_limit: 0,
    })
    .unwrap();
    m.step();
    assert!(m.io_stop().is_none());
}
