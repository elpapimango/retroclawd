//! Line and frame events, the three interrupt sources (HBL level 2, VBL level 4, MFP level 6) and the MFP's own
//! clock (facts: `docs/hardware-facts.md`, "Timing and interrupts").

use atari_st::{Accuracy, AtariSt, Config, MfpClock, Monitor, RamSize, Region, StBus};
use machine::Machine;
use mc68000::{FunctionCode, IackResponse, M68kBus, Size};

const SUPER: FunctionCode = FunctionCode::SUPERVISOR_DATA;

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

fn config(rom: Vec<u8>) -> Config {
    Config {
        ram: RamSize::M1,
        region: Region::Pal,
        accuracy: Accuracy::Fast,
        monitor: Monitor::Colour,
        rom,
        trace_limit: 0,
        io_log_limit: 0,
    }
}

fn machine(program: &[u16], handler: &[u16]) -> AtariSt {
    AtariSt::new(config(rom(program, handler))).unwrap()
}

const BRA_SELF: u16 = 0x60FE;

#[test]
fn hbl_every_line_and_vbl_every_frame() {
    let mut m = machine(&[BRA_SELF], &[]);
    while m.vbl_count() < 1000 {
        m.step();
    }
    assert_eq!(m.last_vbl_at(), 1000 * 512 * 313);
    assert_eq!(m.hbl_count(), 1000 * 313);
}

#[test]
fn switching_to_60hz_changes_the_next_frame() {
    // MOVE.B #0,$FF820A.W; BRA.S *
    let mut m = machine(&[0x11FC, 0x0000, 0x820A, BRA_SELF], &[]);
    while m.vbl_count() < 2 {
        m.step();
    }
    assert_eq!(m.last_vbl_at(), 512 * 313 + 508 * 263);
}

#[test]
fn mfp_clock_round_trip_is_exact() {
    let clock = MfpClock::new(32_084_988);
    let mut seed = 7u64;
    for _ in 0..10_000 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let tick = seed >> 20;
        let c = clock.cycle_for(tick);
        assert!(clock.ticks_at(c) >= tick, "tick {tick}");
        assert!(
            c == 0 || clock.ticks_at(c - 1) < tick,
            "tick {tick}: cycle {c} is not the first"
        );
    }
}

/// 10 000 PAL frames: the tick count is the exact rational, with nothing accumulated.
#[test]
fn mfp_ticks_never_drift() {
    let clock = MfpClock::new(32_084_988);
    let cycles = 10_000u64 * 160_256;
    let exact = (cycles as u128 * 4 * 2_457_600 / 32_084_988) as u64;
    assert_eq!(clock.ticks_at(cycles), exact);
    let mut m = machine(&[BRA_SELF], &[]);
    while m.vbl_count() < 50 {
        m.step();
    }
    // The MFP is brought up to date on every access, timeout and VBL.
    assert_eq!(m.mfp_ticks(), clock.ticks_at(m.last_vbl_at()));
}

fn bus() -> StBus {
    StBus::new(&config(rom(&[BRA_SELF], &[]))).unwrap()
}

#[test]
fn ipl_is_highest_pending_level() {
    let mut b = bus();
    assert_eq!(b.ipl(), 0);
    b.raise_hbl();
    assert_eq!(b.ipl(), 2);
    b.raise_vbl();
    assert_eq!(b.ipl(), 4);
    // An MFP interrupt: GPIP7 (mono detect) channel 15, enabled and unmasked, line falling.
    b.write(0xFFFA07, Size::Byte, 0x80, SUPER).unwrap();
    b.write(0xFFFA13, Size::Byte, 0x80, SUPER).unwrap();
    b.mfp_mut().set_input(7, false);
    assert_eq!(b.ipl(), 6);
    assert_eq!(b.iack(4), IackResponse::Autovector);
    assert_eq!(b.ipl(), 6);
    assert_eq!(b.iack(2), IackResponse::Autovector);
}

#[test]
fn mfp_iack_returns_vector_and_spurious_when_empty() {
    let mut b = bus();
    b.write(0xFFFA17, Size::Byte, 0x40, SUPER).unwrap();
    b.write(0xFFFA07, Size::Byte, 0x80, SUPER).unwrap();
    b.write(0xFFFA13, Size::Byte, 0x80, SUPER).unwrap();
    b.mfp_mut().set_input(7, false);
    assert_eq!(b.iack(6), IackResponse::Vector(0x4F));
    assert_eq!(b.iack(6), IackResponse::Spurious);
}

#[test]
fn gpip_reads_a1_with_a_colour_monitor() {
    let mut b = bus();
    assert_eq!(b.read(0xFFFA01, Size::Byte, SUPER), Ok(0xA1));
}

#[test]
fn reset_instruction_resets_the_mfp() {
    let mut b = bus();
    b.write(0xFFFA07, Size::Byte, 0x80, SUPER).unwrap();
    b.reset_devices();
    assert_eq!(b.read(0xFFFA07, Size::Byte, SUPER), Ok(0));
}

/// Timer C at 200 Hz, the way TOS sets it up, counting its interrupts in RAM.
#[test]
fn timer_c_200hz_bare_metal() {
    let program = [
        0x21FC, 0x00FC, 0x0040,
        0x0114, // MOVE.L #$FC0040,$114.W   (vector $45: VR $40 + channel 5)
        0x11FC, 0x0048,
        0xFA17, // MOVE.B #$48,$FFFA17.W  VR: vectors $40-$4F, software end of interrupt
        0x11FC, 0x00C0, 0xFA23, // MOVE.B #192,$FFFA23.W  TCDR
        0x11FC, 0x0050, 0xFA1D, // MOVE.B #$50,$FFFA1D.W  TCDCR: Timer C delay mode / 64
        0x11FC, 0x0020, 0xFA09, // MOVE.B #$20,$FFFA09.W  IERB: channel 5
        0x11FC, 0x0020, 0xFA15, // MOVE.B #$20,$FFFA15.W  IMRB: channel 5
        0x46FC,
        0x2500, // MOVE #$2500,SR: only level 6 (the MFP) gets through; no VBL handler here
        0x4E72, 0x2500, // loop: STOP #$2500
        0x60FA, // BRA.S loop
    ];
    let handler = [
        0x52B8, 0x1000, // ADDQ.L #1,$1000.W
        0x11FC, 0x00DF, 0xFA11, // MOVE.B #$DF,$FFFA11.W  ISRB: end of interrupt for channel 5
        0x4E73, // RTE
    ];
    let mut m = machine(&program, &handler);
    while m.vbl_count() < 50 {
        m.step();
    }
    let count = (0..4).fold(0u32, |v, i| v << 8 | m.read_memory(0x1000 + i) as u32);
    // 50 frames = 8 012 800 clocks = 2 454 825 MFP ticks; one interrupt per 64 × 192 ticks, the timer started a few
    // dozen clocks after reset.
    assert_eq!(count, 199);
}

#[test]
fn a_mono_monitor_is_refused_until_its_line_count_is_sourced() {
    let mut cfg = config(rom(&[BRA_SELF], &[]));
    cfg.monitor = Monitor::Mono;
    let err = AtariSt::new(cfg).err().expect("refused");
    assert!(err.contains("mono"), "{err}");
}
