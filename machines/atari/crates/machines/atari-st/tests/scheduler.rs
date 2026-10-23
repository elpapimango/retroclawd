//! The event scheduler and the frame timing built on it.

use atari_st::{
    Accuracy, AtariSt, CLASS_VIDEO, Config, Event, Monitor, RamSize, Region, Scheduler,
};
use machine::Machine;

#[test]
fn simultaneous_events_pop_in_class_then_insertion_order() {
    let mut s = Scheduler::new();
    s.schedule(100, 3, Event::LineEnd);
    s.schedule(100, CLASS_VIDEO, Event::LineEnd);
    s.schedule(50, 5, Event::LineEnd);
    assert_eq!(s.next_at(), Some(50));
    assert_eq!(s.pop_due(49), None);
    assert_eq!(s.pop_due(100), Some((50, 5, Event::LineEnd)));
    assert_eq!(s.pop_due(100), Some((100, CLASS_VIDEO, Event::LineEnd)));
    assert_eq!(s.pop_due(100), Some((100, 3, Event::LineEnd)));
    assert_eq!(s.pop_due(1000), None);
}

/// A 192 KB ROM: SSP `$8000`, reset PC `$FC0010`, `program` from `$FC0010`, `handler` from `$FC0040`.
fn rom(program: &[u16], handler: &[u16]) -> Vec<u8> {
    let mut rom = vec![0u8; 196_608];
    rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00FC_0010u32.to_be_bytes());
    for (i, w) in program.iter().enumerate() {
        rom[0x10 + 2 * i..0x12 + 2 * i].copy_from_slice(&w.to_be_bytes());
    }
    for (i, w) in handler.iter().enumerate() {
        rom[0x40 + 2 * i..0x42 + 2 * i].copy_from_slice(&w.to_be_bytes());
    }
    rom
}

fn machine(rom: Vec<u8>, region: Region) -> AtariSt {
    AtariSt::new(Config {
        ram: RamSize::M1,
        region,
        accuracy: Accuracy::Fast,
        monitor: Monitor::Colour,
        rom,
        trace_limit: 0,
        io_log_limit: 0,
    })
    .unwrap()
}

#[test]
fn a_thousand_frames_take_exactly_a_thousand_frame_periods() {
    // BRA.S * with the interrupt mask at 7 (after reset).
    let mut m = machine(rom(&[0x60FE], &[]), Region::Pal);
    assert_eq!(m.cycles_per_frame(), 160_256);
    while m.vbl_count() < 1000 {
        m.step();
    }
    assert_eq!(m.last_vbl_at(), 1000 * 160_256);
}

#[test]
fn ntsc_frames_are_shorter() {
    let m = machine(rom(&[0x60FE], &[]), Region::Ntsc);
    assert_eq!(m.cycles_per_frame(), 133_604);
}

#[test]
fn vbl_raises_ipl4_and_is_autovectored() {
    // MOVE.L #$FC0040,$70.W; MOVE #$2300,SR; BRA.S *   handler: ADDQ.L #1,$1000.W; RTE
    let program = [0x21FC, 0x00FC, 0x0040, 0x0070, 0x46FC, 0x2300, 0x60FE];
    let mut m = machine(rom(&program, &[0x52B8, 0x1000, 0x4E73]), Region::Pal);
    while m.vbl_count() < 3 {
        m.step();
    }
    // Let the third handler finish.
    for _ in 0..20 {
        m.step();
    }
    let long = (0..4).fold(0u32, |v, i| v << 8 | m.read_memory(0x1000 + i) as u32);
    assert_eq!(long, 3);
}

#[test]
fn step_is_deterministic() {
    let program = [0x21FC, 0x00FC, 0x0040, 0x0070, 0x46FC, 0x2300, 0x60FE];
    let handler = [0x52B8, 0x1000, 0x4E73];
    let mut a = machine(rom(&program, &handler), Region::Pal);
    let mut b = machine(rom(&program, &handler), Region::Pal);
    for _ in 0..200_000 {
        a.step();
        b.step();
    }
    assert_eq!(a.registers(), b.registers());
    let mem = |m: &AtariSt| (0..0x2000u64).map(|i| m.read_memory(i)).collect::<Vec<_>>();
    assert_eq!(mem(&a), mem(&b));
}

#[test]
fn frame_rate_is_the_exact_crystal_ratio() {
    let m = machine(rom(&[0x60FE], &[]), Region::Pal);
    let rate = m.frame_rate();
    // 32.084988 MHz / 4 per CPU clock, 160256 clocks per frame.
    assert_eq!((rate.num, rate.den), (32_084_988, 4 * 160_256));
    assert_eq!(m.clock_hz(), 8_021_247);
}
