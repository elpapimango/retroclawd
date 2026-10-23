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

/// The mono monitor's frame is 501 lines of 224 clocks ([O], probes P14 and P15), whatever the sync mode says.
#[test]
fn a_mono_monitor_has_a_501_line_frame_of_224_clocks() {
    let mut cfg = config(rom(&[BRA_SELF], &[]));
    cfg.monitor = Monitor::Mono;
    let mut m = AtariSt::new(cfg).unwrap();
    assert_eq!(m.cycles_per_frame(), 224 * 501);
    while m.vbl_count() < 3 {
        m.step();
    }
    assert_eq!(m.cycles_per_frame(), 224 * 501);
    assert_eq!(m.hbl_count(), 3 * 501);
}

/// A long run with Timer C interrupting and the sync mode flipped every 64 frames by a VBL handler: every frame is
/// exactly one of the two colour shapes, the HBLs add up to their lines, and the MFP has counted exactly the ticks
/// of the time at the last VBL.
#[test]
fn long_run_keeps_exact_time_across_50_60hz_switches() {
    let program = [
        0x21FC, 0x00FC, 0x0080, 0x0070, // MOVE.L #$FC0080,$70.W   VBL handler
        0x21FC, 0x00FC, 0x00C0, 0x0114, // MOVE.L #$FC00C0,$114.W  Timer C handler
        0x11FC, 0x0048, 0xFA17, // VR $48
        0x11FC, 0x00C0, 0xFA23, // TCDR 192
        0x11FC, 0x0050, 0xFA1D, // TCDCR: Timer C / 64
        0x11FC, 0x0020, 0xFA09, // IERB channel 5
        0x11FC, 0x0020, 0xFA15, // IMRB channel 5
        0x46FC, 0x2300, // MOVE #$2300,SR
        BRA_SELF,
    ];
    let vbl: [u16; 11] = [
        0x5278, 0x1000, // ADDQ.W #1,$1000.W
        0x3038, 0x1000, // MOVE.W $1000.W,D0
        0x0240, 0x003F, // ANDI.W #$3F,D0
        0x6606, // BNE.S +6
        0x0A38, 0x0002, 0x820A, // EORI.B #2,$FF820A.W: flip 50/60 Hz
        0x4E73, // RTE
    ];
    let timer: [u16; 6] = [
        0x52B8, 0x1004, // ADDQ.L #1,$1004.W
        0x11FC, 0x00DF, 0xFA11, // ISRB: end of interrupt
        0x4E73, // RTE
    ];
    // The program is longer than $30 bytes, so both handlers go above it.
    let mut image = rom(&program, &[]);
    for (at, code) in [(0x80, &vbl[..]), (0xC0, &timer[..])] {
        for (i, w) in code.iter().enumerate() {
            image[at + 2 * i..at + 2 + 2 * i].copy_from_slice(&w.to_be_bytes());
        }
    }
    let mut m = AtariSt::new(config(image)).unwrap();
    let clock = MfpClock::new(32_084_988);
    let (mut lines, mut shapes, mut last) = (0u64, [0u32; 2], 0u64);
    while m.vbl_count() < 1000 {
        let before = m.vbl_count();
        m.step();
        if m.vbl_count() != before {
            let length = m.last_vbl_at() - last;
            last = m.last_vbl_at();
            lines += match length {
                160_256 => {
                    shapes[0] += 1;
                    313
                }
                133_604 => {
                    shapes[1] += 1;
                    263
                }
                other => panic!("frame {} is {other} clocks", m.vbl_count()),
            };
            assert_eq!(
                m.mfp_ticks(),
                clock.ticks_at(last),
                "frame {}",
                m.vbl_count()
            );
        }
    }
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
    assert!(shapes[0] > 0 && shapes[1] > 0, "{shapes:?}");
    assert_eq!(m.hbl_count(), lines);
    let timer_irqs = (0..4).fold(0u32, |v, i| v << 8 | m.read_memory(0x1004 + i) as u32);
    assert_eq!(
        timer_irqs as u64,
        (clock.ticks_at(last) - 2) / (64 * 192),
        "Timer C kept time"
    );
}
