//! The bounded bring-up diagnostics: the I/O stop report and the PC trace.

use atari_st::{Accuracy, AtariSt, Config, RamSize, Region};
use machine::Machine;

/// A 192 KB ROM: SSP `$8000`, reset PC `$FC0030`, then `program`.
fn rom(program: &[u16]) -> Vec<u8> {
    let mut rom = vec![0u8; 196_608];
    rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00FC_0030u32.to_be_bytes());
    for (i, w) in program.iter().enumerate() {
        rom[0x30 + 2 * i..0x32 + 2 * i].copy_from_slice(&w.to_be_bytes());
    }
    rom
}

fn machine(program: &[u16], trace_limit: u32) -> AtariSt {
    AtariSt::new(Config {
        ram: RamSize::M1,
        region: Region::Pal,
        accuracy: Accuracy::Fast,
        rom: rom(program),
        trace_limit,
    })
    .unwrap()
}

#[test]
fn the_io_stop_holds_the_machine_and_reports_once() {
    // MOVEQ #1,D1; MOVE.B $FF8001,D0
    let mut m = machine(&[0x7201, 0x1039, 0x00FF, 0x8001], 0);
    let mut reports = Vec::new();
    for _ in 0..(50 * 160_256 / 4) {
        m.step();
        if let Some(r) = m.take_stop_report() {
            reports.push(r);
        }
    }
    assert!(m.halted());
    assert_eq!(reports.len(), 1);
    let r = &reports[0];
    assert!(r.contains("stop: read byte $FF8001"), "{r}");
    assert!(r.contains("D1=00000001"), "{r}");
    assert!(r.contains("move.b $ff8001,d0"), "{r}");
    assert!(
        r.contains("$FC0030"),
        "the history lists the instructions: {r}"
    );
    assert!(r.lines().count() < 400, "bounded");
}

#[test]
fn trace_lists_pcs_up_to_the_limit() {
    // NOP x 3; BRA.S *
    let mut m = machine(&[0x4E71, 0x4E71, 0x4E71, 0x60FE], 5);
    for _ in 0..100 {
        m.step();
    }
    assert_eq!(
        m.trace(),
        [0xFC0030, 0xFC0032, 0xFC0034, 0xFC0036, 0xFC0036]
    );
}

#[test]
fn exceptions_appear_in_the_report() {
    // ILLEGAL; vector 4 (in RAM at $10) points at $FC0040, whose MOVE.B $FF8001,D0 stops the machine.
    let mut rom = rom(&[0x4AFC]);
    rom[0x40..0x46].copy_from_slice(&[0x10, 0x39, 0x00, 0xFF, 0x80, 0x01]);
    let mut m = AtariSt::new(Config {
        ram: RamSize::M1,
        region: Region::Pal,
        accuracy: Accuracy::Fast,
        rom,
        trace_limit: 0,
    })
    .unwrap();
    m.load(0x10, &0x00FC_0040u32.to_be_bytes());
    let mut report = None;
    for _ in 0..1000 {
        m.step();
        report = report.or_else(|| m.take_stop_report());
    }
    let r = report.expect("stopped");
    assert!(r.contains("vector 4"), "{r}");
}

#[test]
fn a_halted_cpu_is_reported_once() {
    // Reset SSP $8001 (odd): the ILLEGAL's exception frame faults, and so does the address error's frame.
    let mut rom = rom(&[0x4AFC]);
    rom[0..4].copy_from_slice(&0x8001u32.to_be_bytes());
    let mut m = AtariSt::new(Config {
        ram: RamSize::M1,
        region: Region::Pal,
        accuracy: Accuracy::Fast,
        rom,
        trace_limit: 0,
    })
    .unwrap();
    let mut reports = Vec::new();
    for _ in 0..100 {
        m.step();
        reports.extend(m.take_stop_report());
    }
    assert!(m.halted());
    assert_eq!(reports.len(), 1);
    assert!(reports[0].contains("cpu halted"), "{}", reports[0]);
    assert!(
        reports[0].contains("$00007FF"),
        "the faulting address: {}",
        reports[0]
    );
}
