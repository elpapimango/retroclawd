//! Wait states in `accurate` mode (waits.rs): what P02 showed in Hatari 2.6.1, pinned here without it.

mod common;
use common::{BRA_SELF, rom};

use atari_st::{Accuracy, AtariSt, Config, Monitor, RamSize, Region};
use machine::Machine;

fn machine(program: &[u16], accuracy: Accuracy) -> AtariSt {
    AtariSt::new(Config {
        ram: RamSize::M1,
        region: Region::Pal,
        accuracy,
        monitor: Monitor::Colour,
        rom: rom(program, &[]),
        trace_limit: 0,
        io_log_limit: 0,
    })
    .unwrap()
}

/// Clocks each of the first `n` instructions took.
fn clocks(program: &[u16], accuracy: Accuracy, n: usize) -> Vec<u32> {
    let mut m = machine(program, accuracy);
    (0..n).map(|_| m.step()).collect()
}

#[test]
fn an_instruction_of_six_clocks_costs_eight_in_a_row() {
    // BTST D2,D1 is 6 clocks by the book: its last 2 are internal, so the next fetch has to wait for its slot.
    let mut program = vec![0x7201, 0x7407];
    program.extend([0x0501; 10]);
    program.push(BRA_SELF);
    let accurate = clocks(&program, Accuracy::Accurate, 12);
    assert!(accurate[3..].iter().all(|&c| c == 8), "{accurate:?}");
    let fast = clocks(&program, Accuracy::Fast, 12);
    assert!(fast[2..].iter().all(|&c| c == 6), "{fast:?}");
}

#[test]
fn a_long_shift_costs_a_multiple_of_four_clocks_in_a_row() {
    // LSL.L #8,D1: 8 + 2 * 8 = 24 clocks by the book; LSL.W #1,D1: 6.
    let mut program = vec![0x7201];
    program.extend([0xE189; 6]);
    program.extend([0xE349; 6]);
    program.push(BRA_SELF);
    let accurate = clocks(&program, Accuracy::Accurate, 13);
    assert!(accurate[2..7].iter().all(|&c| c == 24), "{accurate:?}");
    assert!(accurate[8..].iter().all(|&c| c == 8), "{accurate:?}");
}

#[test]
fn an_access_to_the_mfp_costs_four_clocks_more_than_one_to_ram() {
    // MOVE.B $0601.W,D1 against MOVE.B $FFFA01.W,D1 (GPIP).
    let ram = clocks(&[0x1238, 0x0601, BRA_SELF], Accuracy::Accurate, 1)[0];
    let mfp = clocks(&[0x1238, 0xFA01, BRA_SELF], Accuracy::Accurate, 1)[0];
    assert_eq!(mfp, ram + 4);
}

#[test]
fn an_access_to_the_shifter_costs_what_one_to_ram_does() {
    // MOVE.W $FF8240.W,D1 (palette 0) against MOVE.W $0600.W,D1.
    let ram = clocks(&[0x3238, 0x0600, BRA_SELF], Accuracy::Accurate, 1)[0];
    let shifter = clocks(&[0x3238, 0x8240, BRA_SELF], Accuracy::Accurate, 1)[0];
    assert_eq!(shifter, ram);
}
