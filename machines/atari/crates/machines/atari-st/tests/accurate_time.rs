//! `accurate` mode: time passes with every bus cycle, and a device sees an access at the cycle it is made in (spec
//! sections 8 and 11). `fast` mode sees every access at the start of its instruction.

mod common;
use common::{BRA_SELF, rom};

use atari_st::{Accuracy, AtariSt, Config, Monitor, RamSize, Region, StBus};
use machine::Machine;
use mc68000::{FunctionCode, M68kBus, Size};

const NOP: u16 = 0x4E71;

fn config(rom: Vec<u8>, accuracy: Accuracy) -> Config {
    Config {
        ram: RamSize::M1,
        region: Region::Pal,
        accuracy,
        monitor: Monitor::Colour,
        rom,
        trace_limit: 0,
        io_log_limit: 0,
    }
}

/// Timer A in delay mode, prescaler 4, data 200 (`early` NOPs before the control write); `pad` NOPs; then
/// `MOVE.B $FFFA1F.W,$0600.W` (the timer's counter into RAM) and a loop.
fn timer_read_program(early: usize, pad: usize) -> Vec<u16> {
    let mut p = vec![];
    p.extend(common::move_b(200, 0xFA1F)); // TADR
    p.extend(std::iter::repeat_n(NOP, early));
    p.extend(common::move_b(0x01, 0xFA19)); // TACR: delay mode, /4
    p.extend(std::iter::repeat_n(NOP, pad));
    p.extend(common::move_b_abs(0xFA1F, 0x0600));
    p.push(BRA_SELF);
    p
}

fn timer_value(early: usize, pad: usize, accuracy: Accuracy) -> u8 {
    let mut m = AtariSt::new(config(rom(&timer_read_program(early, pad), &[]), accuracy)).unwrap();
    for _ in 0..(2 + early + pad + 2) {
        m.step();
    }
    m.bus().peek(0x0600)
}

#[test]
fn a_device_is_caught_up_to_the_cycle_of_the_access() {
    // The MFP data register is written 8 clocks into its instruction and the control register after 8 clocks of the next
    // one; the counter is read 4 clocks into its own (each fetches its address words first), an access to the MFP waits 4
    // clocks more (waits.rs), which also lengthens the instructions that make one, and the device sees the write at the
    // 4th clock of the cycle and the read at its end (scheduler.rs). `fast` mode sees every access at the start of its
    // instruction. A `fast` program with 4 NOPs more before the control write puts the two accesses within a clock of the
    // same absolute clocks, so the counts agree to within one for every starting point.
    let mut differs = false;
    for pad in 0..60 {
        let accurate = timer_value(0, pad, Accuracy::Accurate);
        let fast_shifted = timer_value(4, pad, Accuracy::Fast);
        let fast_here = timer_value(0, pad, Accuracy::Fast);
        assert!(
            (accurate as i32 - fast_shifted as i32).abs() <= 1,
            "pad {pad}: {accurate} here, {fast_shifted} in fast mode at the same clocks"
        );
        differs |= accurate != fast_here;
    }
    assert!(
        differs,
        "over 30 starting points a 4-clock shift must cross a timer tick (13 clocks each)"
    );
}

#[test]
fn accurate_and_fast_agree_when_nothing_timing_sensitive_is_touched() {
    // Register arithmetic, RAM and ROM only, interrupts masked (the reset leaves the mask at 7).
    let program = [
        0x303C, 0x0007, // MOVE.W #7,D0
        0x2200, // MOVE.L D0,D1
        0xE349, // LSL.W #1,D1
        0x4EB9, 0x00FC, 0x0030, // JSR $FC0030
        0x33C1, 0x0000, 0x0600, // MOVE.W D1,$600.L
        BRA_SELF,
    ];
    let routine = [0x5240, 0x4E75]; // ADDQ.W #1,D0; RTS
    let blocks: &[(usize, &[u16])] = &[(0x30, &routine)];
    let run = |accuracy| {
        let mut m = AtariSt::new(config(rom(&program, blocks), accuracy)).unwrap();
        let mut clocks = 0u64;
        for _ in 0..40 {
            clocks += m.step() as u64;
        }
        (m.now(), clocks, m.cpu().d, m.cpu().pc, m.bus().peek(0x0600))
    };
    // The wait states of `accurate` mode make the same program take longer; nothing else differs.
    let (fast, accurate) = (run(Accuracy::Fast), run(Accuracy::Accurate));
    assert_eq!(
        (fast.2, fast.3, fast.4),
        (accurate.2, accurate.3, accurate.4)
    );
    assert!(
        accurate.0 >= fast.0 && accurate.0 % 4 == 0,
        "{} then {}",
        fast.0,
        accurate.0
    );
}

/// A tiny deterministic generator for the property test.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn events_run_in_order_and_time_never_decreases() {
    // Random bus cycles and internal clocks on the bare bus: after each, the lines that have ended are exactly
    // those whose end time has passed (a line is 512 clocks at 50 Hz), time only moves forward, and an access to a
    // device does not skip an event.
    let rom = rom(&[BRA_SELF], &[]);
    let mut bus = StBus::new(&config(rom, Accuracy::Accurate)).unwrap();
    let fc = FunctionCode::SUPERVISOR_DATA;
    let mut rng = Lcg(7);
    let mut last = bus.now();
    for _ in 0..200_000 {
        match rng.next() % 6 {
            0 => {
                let _ = bus.read(0x0100 + (rng.next() % 0x1000) as u32 * 2, Size::Word, fc);
            }
            1 => {
                let _ = bus.write(
                    0x0100 + (rng.next() % 0x1000) as u32 * 2,
                    Size::Word,
                    0x1234,
                    fc,
                );
            }
            2 => {
                // The MFP's timer A data register.
                let _ = bus.read(0xFF_FA1F, Size::Byte, fc);
            }
            3 => {
                let _ = bus.read(0xFF_8209, Size::Byte, fc);
            }
            4 => bus.sync((rng.next() % 40) as u32),
            _ => {
                let _ = bus.write(0xFF_8240, Size::Word, (rng.next() & 0x777) as u32, fc);
            }
        }
        let now = bus.now();
        assert!(now >= last, "time went backwards: {last} then {now}");
        last = now;
        assert_eq!(
            bus.hbl_count(),
            now / 512,
            "every line that ended by {now} was processed, none early"
        );
        assert_eq!(bus.vbl_count(), bus.hbl_count() / 313);
    }
}

#[test]
fn emutos_reaches_its_desktop_in_accurate_mode() {
    let Some(rom) = common::emutos() else {
        return;
    };
    let run = |accuracy| {
        let mut m = AtariSt::new(config(rom.clone(), accuracy)).unwrap();
        while m.io_stop().is_none() && m.vbl_count() < 300 {
            m.step();
        }
        assert!(
            m.io_stop().is_none(),
            "{accuracy:?} stopped: {:?}",
            m.io_stop()
        );
        m.render_frame().unwrap().2
    };
    let fast = run(Accuracy::Fast);
    let accurate = run(Accuracy::Accurate);
    // The desktop is the same picture whichever way time passed (no wait state is modelled yet).
    assert_eq!(fast, accurate);
}
