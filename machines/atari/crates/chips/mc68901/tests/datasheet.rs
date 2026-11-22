//! Behaviour from the Motorola MC68901 datasheet (Advance Information, January 1984); each test names its section.

use mc68901::{Mfp, TimerAb};

const GPIP: u8 = 0;
const AER: u8 = 1;
const DDR: u8 = 2;
const IERA: u8 = 3;
const IERB: u8 = 4;
const IPRA: u8 = 5;
const IPRB: u8 = 6;
const ISRA: u8 = 7;
const ISRB: u8 = 8;
const IMRA: u8 = 9;
const IMRB: u8 = 10;
const VR: u8 = 11;
const TACR: u8 = 12;
const TBCR: u8 = 13;
const TCDCR: u8 = 14;
const TADR: u8 = 15;
const TBDR: u8 = 16;
const TCDR: u8 = 17;
const TDDR: u8 = 18;
const TSR: u8 = 22;

/// Channel 5 (Timer C) is bit 5 of the B registers (§3.3, Table 3-1: channels 0-7 in the B registers).
const TIMER_C: u8 = 1 << 5;

fn enable_b(m: &mut Mfp, bits: u8) {
    m.write(IERB, bits);
    m.write(IMRB, bits);
}

/// §2.2.3: reset clears every register except the timer data registers, UDR, TSR and VR, which becomes `$0F`
/// (Figure 3-2: its unused low bits read as zero).
#[test]
fn reset_state() {
    let mut m = Mfp::new();
    for (reg, v) in [
        (AER, 0xFF),
        (DDR, 0xFF),
        (IERA, 0xFF),
        (IMRA, 0xFF),
        (TADR, 0x42),
        (TACR, 0x01),
    ] {
        m.write(reg, v);
    }
    m.write(TACR, 0x00);
    m.write(VR, 0x40);
    m.reset();
    for reg in [
        AER, DDR, IERA, IERB, IPRA, IPRB, ISRA, ISRB, IMRA, IMRB, TACR, TBCR, TCDCR,
    ] {
        assert_eq!(m.read(reg), 0, "register {reg}");
    }
    assert_eq!(m.read(VR), 0x08, "VR = $0F, low three bits read as 0");
    assert_eq!(m.read(TADR), 0x42, "timer data survives reset");
}

#[test]
fn tsr_survives_reset() {
    let mut m = Mfp::new();
    assert_eq!(m.read(TSR), 0x80, "power-on TSR: buffer empty ([O] $80)");
    m.write(TSR, 0x01);
    let written = m.read(TSR);
    m.reset();
    assert_eq!(m.read(TSR), written);
}

/// §4.2.1: outputs read from the data register, inputs from the pins.
#[test]
fn gpip_reads_inputs_and_outputs_by_ddr() {
    let mut m = Mfp::new();
    m.set_input(7, true);
    m.set_input(0, true);
    m.write(GPIP, 0x0F);
    assert_eq!(m.read(GPIP), 0x81, "all inputs");
    m.write(DDR, 0x0F);
    assert_eq!(m.read(GPIP), 0x8F, "low nibble outputs");
}

/// §4.2.2: AER 0 = falling edge, 1 = rising edge; only an enabled channel sets pending (§3.3.1).
#[test]
fn edge_interrupt_follows_aer() {
    let mut m = Mfp::new();
    m.set_input(7, true);
    m.write(IERA, 0x80); // channel 15 = GPIP7
    m.write(IMRA, 0x80);
    m.set_input(7, false);
    assert_eq!(m.read(IPRA), 0x80, "falling edge with AER 0");
    m.write(IPRA, 0x7F);
    // Writing AER moves the detector (§4.2.2 note): input low, edge bit 1 -> the XOR rises; no interrupt.
    m.write(AER, 0x80);
    assert_eq!(m.read(IPRA), 0);
    m.set_input(7, true);
    assert_eq!(m.read(IPRA), 0x80, "rising edge with AER 1");
    // A disabled channel ignores its input (§3.3.1).
    let mut m = Mfp::new();
    m.set_input(0, true);
    m.set_input(0, false);
    assert_eq!(m.read(IPRB), 0);
}

/// §3.3.1: writing 0 to IER clears the channel's pending bit; §3.3.2: writing IPR clears only the zero bits.
#[test]
fn disabling_a_channel_clears_pending() {
    let mut m = Mfp::new();
    m.set_input(0, true);
    m.set_input(1, true);
    m.write(IERB, 0x03);
    m.set_input(0, false);
    m.set_input(1, false);
    assert_eq!(m.read(IPRB), 0x03);
    m.write(IPRB, 0xFF);
    assert_eq!(m.read(IPRB), 0x03, "writing ones has no effect");
    m.write(IERB, 0x02);
    assert_eq!(m.read(IPRB), 0x02);
}

/// §3.3.3: a masked channel still becomes pending but does not request.
#[test]
fn mask_gates_irq_not_pending() {
    let mut m = Mfp::new();
    m.set_input(0, true);
    m.write(IERB, 0x01);
    m.set_input(0, false);
    assert_eq!(m.read(IPRB), 0x01);
    assert!(!m.irq());
    m.write(IMRB, 0x01);
    assert!(m.irq());
}

/// §3.1.1 / §3.1.2: the highest-priority requesting channel's number goes in the low four bits of the vector; the
/// acknowledge clears its pending bit; nothing requesting answers `None`.
#[test]
fn iack_returns_highest_priority_vector() {
    let mut m = Mfp::new();
    m.write(VR, 0x40);
    m.set_input(0, true);
    m.set_input(7, true);
    m.write(IERA, 0x80);
    m.write(IMRA, 0x80);
    m.write(IERB, 0x01);
    m.write(IMRB, 0x01);
    m.set_input(0, false);
    m.set_input(7, false);
    assert_eq!(m.iack(), Some(0x4F), "GPIP7, channel 15");
    assert_eq!(m.iack(), Some(0x40), "GPIP0, channel 0");
    assert_eq!(m.iack(), None);
    assert!(!m.irq());
}

/// §3.4.3: with S set, the acknowledged channel's in-service bit blocks it and every lower channel.
#[test]
fn software_eoi_blocks_lower_channels_until_isr_cleared() {
    let mut m = Mfp::new();
    m.write(VR, 0x48);
    m.set_input(4, true);
    m.set_input(0, true);
    m.write(IERB, 0x41); // channel 6 (GPIP4), channel 0
    m.write(IMRB, 0x41);
    m.set_input(4, false);
    assert_eq!(m.iack(), Some(0x46));
    assert_eq!(m.read(ISRB), 0x40);
    m.set_input(0, false);
    assert!(!m.irq(), "channel 0 is below the one in service");
    m.write(ISRB, 0xBF);
    assert!(m.irq());
    assert_eq!(m.iack(), Some(0x40));
}

/// §3.4.2: with S clear the in-service bits stay low.
#[test]
fn automatic_eoi_never_sets_isr() {
    let mut m = Mfp::new();
    m.write(VR, 0x40);
    m.set_input(4, true);
    m.write(IERB, 0x40);
    m.write(IMRB, 0x40);
    m.set_input(4, false);
    assert_eq!(m.iack(), Some(0x46));
    assert_eq!(m.read(ISRB), 0);
}

/// §5.1.1 and Figure 5-3: the timer times out after prescale × data clocks, for each prescaler.
#[test]
fn delay_mode_times_out_after_prescale_times_data() {
    for (code, prescale) in [
        (1u8, 4u64),
        (2, 10),
        (3, 16),
        (4, 50),
        (5, 64),
        (6, 100),
        (7, 200),
    ] {
        let mut m = Mfp::new();
        enable_b(&mut m, TIMER_C);
        m.write(TCDR, 10);
        m.write(TCDCR, code << 4);
        assert_eq!(
            m.next_timeout(),
            Some(prescale * 10),
            "prescaler {prescale}"
        );
        m.tick(prescale * 10 - 1);
        assert_eq!(m.read(IPRB) & TIMER_C, 0, "one clock early");
        m.tick(1);
        assert_eq!(m.read(IPRB) & TIMER_C, TIMER_C, "prescaler {prescale}");
    }
}

/// §5.1.1: data 0 counts 256 pulses.
#[test]
fn data_register_zero_counts_256() {
    let mut m = Mfp::new();
    m.write(TCDR, 0);
    m.write(TCDCR, 1 << 4);
    assert_eq!(m.next_timeout(), Some(4 * 256));
}

/// §5.2.1: a write while running is loaded at the next timeout.
#[test]
fn writing_tdr_while_running_loads_at_next_timeout() {
    let mut m = Mfp::new();
    m.write(TCDR, 10);
    m.write(TCDCR, 1 << 4);
    m.tick(8); // two count pulses: counter 8
    m.write(TCDR, 3);
    assert_eq!(m.read(TCDR), 8, "the counter, not the new data");
    assert_eq!(m.next_timeout(), Some(32));
    m.tick(32);
    assert_eq!(m.read(TCDR), 3, "reloaded from the new data");
    assert_eq!(m.next_timeout(), Some(12));
}

/// §5.2.1: reading the data register gives the main counter; stopping keeps it (Figure 5-3 note).
#[test]
fn reading_tdr_returns_the_counter() {
    let mut m = Mfp::new();
    m.write(TDDR, 100);
    m.write(TCDCR, 0x01);
    m.tick(4 * 30);
    assert_eq!(m.read(TDDR), 70);
    m.write(TCDCR, 0x00);
    m.tick(1000);
    assert_eq!(m.read(TDDR), 70);
}

/// §5.1.3: in event count mode each active TBI edge counts; the timeout sets Timer B pending.
#[test]
fn event_count_mode_counts_active_edges() {
    let mut m = Mfp::new();
    m.write(IERA, 1); // channel 8, Timer B, is bit 0 of the A registers
    m.write(TBDR, 3);
    m.write(AER, 0x08); // GPIP3 edge bit: TBI counts on rising edges
    m.write(TBCR, 0x08);
    for i in 0..3 {
        assert_eq!(m.read(IPRA) & 1, 0, "edge {i}");
        m.timer_input(TimerAb::B, true);
        m.timer_input(TimerAb::B, false);
    }
    assert_eq!(m.read(IPRA) & 1, 1);
    assert_eq!(m.read(TBDR), 3, "reloaded");
}

/// §5.1.2: in pulse width mode the timer runs only while TAI is active; the interrupt on the GPIP4 channel (6)
/// comes on the opposite edge, when the pulse ends.
#[test]
fn pulse_width_mode_counts_while_input_active() {
    let mut m = Mfp::new();
    m.write(TADR, 200);
    m.write(AER, 0x10); // GPIP4 edge bit: TAI active high
    m.write(TACR, 0x09); // pulse width, prescaler 4
    m.tick(400);
    assert_eq!(m.read(TADR), 200, "input inactive");
    m.timer_input(TimerAb::A, true);
    m.tick(40);
    assert_eq!(m.read(TADR), 190);
    m.write(IERB, 0x40);
    m.timer_input(TimerAb::A, false);
    assert_eq!(m.read(IPRB) & 0x40, 0x40);
    m.tick(400);
    assert_eq!(m.read(TADR), 190);
}

/// §5.1.1: the output toggles on each timeout; TACR bit 4 forces it low (Figure 5-3).
#[test]
fn timer_output_toggles_on_timeout() {
    let mut m = Mfp::new();
    m.write(TADR, 1);
    m.write(TACR, 0x01);
    assert!(!m.timer_output(0));
    m.tick(4);
    assert!(m.timer_output(0));
    m.tick(4);
    assert!(!m.timer_output(0));
    m.tick(4);
    assert!(m.timer_output(0));
    m.write(TACR, 0x11);
    assert!(!m.timer_output(0));
}

#[test]
fn next_timeout_is_exact() {
    let mut m = Mfp::new();
    m.write(TCDR, 192);
    m.write(TDDR, 2);
    m.write(TCDCR, 0x51); // Timer C /64 (the ST's 200 Hz set-up), Timer D /4
    assert_eq!(m.next_timeout(), Some(8));
    m.tick(8);
    assert_eq!(m.next_timeout(), Some(8));
    m.write(TCDCR, 0x50);
    m.tick(64 * 192 - 8 - 3);
    assert_eq!(m.next_timeout(), Some(3));
}

/// `tick(a + b)` and `tick(a); tick(b)` end in the same state (seeded random splits).
#[test]
fn tick_split_is_equivalent() {
    let mut seed = 0x1234_5678u64;
    let mut rand = |n: u64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    for _ in 0..200 {
        let mut a = Mfp::new();
        a.write(TCDR, rand(256) as u8);
        a.write(TDDR, rand(256) as u8);
        a.write(TADR, rand(256) as u8);
        a.write(TCDCR, ((rand(8) as u8) << 4) | rand(8) as u8);
        a.write(TACR, rand(8) as u8);
        a.write(IERA, 0xFF);
        a.write(IERB, 0xFF);
        let mut b = a.clone();
        let (x, y) = (rand(100_000), rand(100_000));
        a.tick(x + y);
        b.tick(x);
        b.tick(y);
        for reg in 0..24 {
            assert_eq!(a.read(reg), b.read(reg), "register {reg} after {x}+{y}");
        }
        assert_eq!(a.next_timeout(), b.next_timeout());
    }
}
