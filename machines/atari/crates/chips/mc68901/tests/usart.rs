//! The USART against the MC68901 datasheet (Advance Information, January 1984), section 6. Timer D is the baud clock
//! (*Internals* p.61): with the prescaler at 4 and the data register at 1, one timer D time-out is 4 timer clocks.

use mc68901::Mfp;

const IERA: u8 = 3;
const IMRA: u8 = 9;
const IPRA: u8 = 5;
const ISRA: u8 = 7;
const TCDCR: u8 = 14;
const TDDR: u8 = 18;
const UCR: u8 = 20;
const RSR: u8 = 21;
const TSR: u8 = 22;
const UDR: u8 = 23;

const TE: u8 = 0x01;
const LOOPBACK: u8 = 0x06;
const BE: u8 = 0x80;
const UE: u8 = 0x40;
const END: u8 = 0x10;
const BF: u8 = 0x80;
const OE: u8 = 0x40;
const RE: u8 = 0x01;

/// Channels 9-12 are bits 1-4 of the A registers (Table 3-1).
const CH_TX_ERROR: u8 = 1 << 1;
const CH_TX_EMPTY: u8 = 1 << 2;
const CH_RX_ERROR: u8 = 1 << 3;
const CH_RX_FULL: u8 = 1 << 4;

/// Timer D in delay mode, prescaler 4, data 1: a time-out every 4 timer clocks.
fn running() -> Mfp {
    let mut m = Mfp::new();
    m.write(TDDR, 1);
    m.write(TCDCR, 0x01);
    m
}

fn enable(m: &mut Mfp, channels: u8) {
    m.write(IERA, channels);
    m.write(IMRA, channels);
}

/// §6.1.3 Figure 6-2: CLK divide by 16, 8 data bits, one start and one stop bit, no parity.
const FORMAT_8N1_16: u8 = 0x88;

/// §6.1.3: ten bits of sixteen clock periods, each period two time-outs of the timer output: 320 time-outs, 1280
/// timer clocks; the first time-out only moves the word from the buffer to the shift register.
#[test]
fn a_byte_written_to_udr_is_transmitted_after_one_character_time() {
    let mut m = running();
    m.write(UCR, FORMAT_8N1_16);
    m.write(TSR, TE);
    m.write(UDR, 0x41);
    assert_eq!(m.read(TSR) & BE, 0, "§6.3.2: a write to UDR clears BE");
    m.tick(4);
    assert_eq!(
        m.read(TSR) & BE,
        BE,
        "BE sets when the word moves to the shift register"
    );
    m.tick(1279 - 4);
    assert_eq!(m.usart_take_tx(), None, "one clock early");
    m.tick(1);
    assert_eq!(m.usart_take_tx(), Some(0x41));
    assert_eq!(m.usart_take_tx(), None);
}

/// Figure 6-2: the stop bits (`ST1 ST0` = 11 is two) and the parity bit lengthen the character; divide by one makes
/// a bit one clock period.
#[test]
fn the_character_format_sets_the_character_time() {
    // (UCR, time-outs): 8N1/16 = 10 × 16 × 2 = 320; 8N2/16 = 11 × 16 × 2 = 352; 8E1/16 = 352; 7N1/16 = 9 × 32 = 288;
    // 8N1/1 = 10 × 2 = 20.
    for (ucr, timeouts) in [
        (0x88u8, 320u64),
        (0x98, 352),
        (0x8C, 352),
        (0xA8, 288),
        (0x08, 20),
    ] {
        let mut m = running();
        m.write(UCR, ucr);
        m.write(TSR, TE);
        m.write(UDR, 0x55);
        m.tick(4 * (timeouts - 1));
        assert_eq!(
            m.usart_take_tx(),
            None,
            "UCR {ucr:#04x}: one time-out early"
        );
        m.tick(4);
        assert!(m.usart_take_tx().is_some(), "UCR {ucr:#04x}");
    }
}

/// §6.1: the unused data bits of a short word are zero.
#[test]
fn a_seven_bit_word_loses_its_top_bit() {
    let mut m = running();
    m.write(UCR, 0xA8);
    m.write(TSR, TE);
    m.write(UDR, 0xFF);
    m.tick(4 * 288);
    assert_eq!(m.usart_take_tx(), Some(0x7F));
}

/// §6.3: a word in the buffer waits while the transmitter is disabled, and the buffer can be loaded first.
#[test]
fn nothing_is_sent_while_the_transmitter_is_disabled() {
    let mut m = running();
    m.write(UCR, FORMAT_8N1_16);
    m.write(UDR, 0x41);
    m.tick(4 * 1000);
    assert_eq!(m.usart_take_tx(), None);
    assert_eq!(m.read(TSR) & BE, 0, "the word is still in the buffer");
    m.write(TSR, TE);
    m.tick(4 * 320);
    assert_eq!(m.usart_take_tx(), Some(0x41));
}

/// §6.3 / §6.3.2: a second word written while the first is going out follows it back to back; with the buffer empty
/// at the end of a word the underrun flag UE sets, and reading TSR clears it.
#[test]
fn a_word_in_the_buffer_follows_and_an_empty_buffer_is_an_underrun() {
    let mut m = running();
    m.write(UCR, 0x08);
    m.write(TSR, TE);
    m.write(UDR, 1);
    m.tick(4);
    m.write(UDR, 2);
    m.tick(4 * 19);
    assert_eq!(m.usart_take_tx(), Some(1));
    assert_eq!(
        m.read(TSR) & UE,
        0,
        "the second word was waiting: no underrun"
    );
    m.tick(4 * 20);
    assert_eq!(m.usart_take_tx(), Some(2));
    assert_eq!(m.read(TSR) & UE, UE, "nothing followed the second word");
    assert_eq!(m.read(TSR) & UE, 0, "reading TSR clears UE");
}

/// §6.3.2: with TE cleared during a word the word is finished and END sets after it; enabling clears END.
#[test]
fn disabling_the_transmitter_finishes_the_word_and_sets_end() {
    let mut m = running();
    m.write(UCR, 0x08);
    m.write(TSR, TE);
    m.write(UDR, 9);
    m.tick(4 * 5);
    m.write(TSR, 0);
    assert_eq!(m.read(TSR) & END, 0, "the word is still going out");
    m.tick(4 * 15);
    assert_eq!(m.usart_take_tx(), Some(9));
    assert_eq!(m.read(TSR) & END, END);
    m.write(TSR, TE);
    assert_eq!(m.read(TSR) & END, 0, "enabling clears END");
}

/// §6.3.2: with nothing in the shift register END sets at once.
#[test]
fn disabling_an_idle_transmitter_sets_end_at_once() {
    let mut m = running();
    m.write(TSR, TE);
    m.write(TSR, 0);
    assert_eq!(m.read(TSR) & END, END);
}

/// §6.3.2 H, L = 1, 1: the transmitter output feeds the receiver; §6.2.1 the received word raises the buffer full
/// interrupt (channel 12).
#[test]
fn loopback_delivers_the_byte_and_raises_the_receive_interrupt() {
    let mut m = running();
    enable(&mut m, CH_RX_FULL);
    m.write(UCR, FORMAT_8N1_16);
    m.write(RSR, RE);
    m.write(TSR, TE | LOOPBACK);
    m.write(UDR, 0x5A);
    m.tick(4 * 319);
    assert!(!m.irq(), "one time-out early");
    m.tick(4);
    assert!(m.irq());
    assert_eq!(m.read(IPRA) & CH_RX_FULL, CH_RX_FULL);
    assert_eq!(m.read(RSR) & BF, BF);
    assert_eq!(m.read(UDR), 0x5A);
    assert_eq!(m.read(RSR) & BF, 0, "§6.2.2: reading the buffer clears BF");
    assert_eq!(m.iack(), Some(0x0C), "the buffer full channel");
}

/// §6.2.2 OE: a word that arrives while the buffer is full overwrites nothing; OE shows once the buffer has been
/// read and clears when RSR is read; no new word is assembled until then (§6.2.2).
#[test]
fn overrun_sets_the_flag() {
    let mut m = running();
    m.write(UCR, 0x08);
    m.write(RSR, RE);
    m.write(TSR, TE | LOOPBACK);
    m.write(UDR, 0x11);
    m.tick(4);
    m.write(UDR, 0x22);
    m.tick(4 * 19);
    assert_eq!(m.read(RSR) & BF, BF, "the first word is in the buffer");
    m.tick(4 * 20);
    assert_eq!(m.read(RSR) & OE, 0, "OE waits for the buffer to be read");
    assert_eq!(m.read(UDR), 0x11, "the first word was not overwritten");
    assert_eq!(m.read(RSR) & OE, OE);
    assert_eq!(m.read(RSR) & OE, 0, "reading RSR clears OE");
}

/// §6.2.1: an overrun interrupts on the receive error channel (11) when it is enabled, else on channel 12.
#[test]
fn an_overrun_uses_the_error_channel_when_it_is_enabled() {
    for (enabled, expect) in [(CH_RX_ERROR | CH_RX_FULL, 0x0B), (CH_RX_FULL, 0x0C)] {
        let mut m = running();
        m.write(UCR, 0x08);
        m.write(RSR, RE);
        m.write(TSR, TE | LOOPBACK);
        m.write(UDR, 0x11);
        m.tick(4);
        m.write(UDR, 0x22);
        m.tick(4 * 39);
        enable(&mut m, enabled);
        m.read(UDR);
        assert_eq!(m.iack(), Some(expect), "channels {enabled:#04x}");
    }
}

/// §6.2.2 RE: clearing it disables the receiver at once and clears every flag.
#[test]
fn clearing_re_clears_the_flags() {
    let mut m = running();
    m.write(UCR, 0x08);
    m.write(RSR, RE);
    m.write(TSR, TE | LOOPBACK);
    m.write(UDR, 0x11);
    m.tick(4 * 20);
    assert_eq!(m.read(RSR) & BF, BF);
    m.write(RSR, 0);
    assert_eq!(m.read(RSR), 0);
}

/// A character from the line, handed in by the machine, takes the same time as one sent.
#[test]
fn a_character_from_the_line_arrives_after_one_character_time() {
    let mut m = running();
    m.write(UCR, FORMAT_8N1_16);
    assert!(!m.usart_can_receive(), "the receiver is disabled");
    m.write(RSR, RE);
    assert!(m.usart_can_receive());
    m.usart_rx_begin(0xA5);
    assert!(!m.usart_can_receive(), "busy assembling the word");
    assert_eq!(m.read(RSR) & 0x04, 0x04, "CIP: a word is being assembled");
    m.tick(4 * 319);
    assert_eq!(m.read(RSR) & BF, 0);
    m.tick(4);
    assert_eq!(m.read(RSR) & BF, BF);
    assert_eq!(m.read(RSR) & 0x04, 0);
    assert_eq!(m.read(UDR), 0xA5);
}

/// §6.3.2 AT: after the transmitter is disabled and its last word is out, the receiver is enabled.
#[test]
fn auto_turnaround_enables_the_receiver() {
    let mut m = running();
    m.write(UCR, 0x08);
    m.write(TSR, TE);
    m.write(UDR, 3);
    m.tick(4 * 2);
    m.write(TSR, 0x20);
    assert_eq!(m.read(RSR) & RE, 0);
    m.tick(4 * 20);
    assert_eq!(m.read(RSR) & RE, RE);
    assert_eq!(
        m.read(TSR) & 0x20,
        0,
        "AT clears at the end of the transmission"
    );
}

/// §2.2.3: reset clears UCR and RSR and stops a word in flight; TSR is not cleared ([H?] against Figure 6-4's TE).
#[test]
fn reset_disables_the_usart() {
    let mut m = running();
    m.write(UCR, FORMAT_8N1_16);
    m.write(RSR, RE);
    m.write(TSR, TE);
    m.write(UDR, 7);
    m.tick(4 * 5);
    m.reset();
    assert_eq!(m.read(UCR), 0);
    assert_eq!(m.read(RSR), 0);
    assert_eq!(m.read(TSR) & TE, TE, "TSR is not cleared");
    m.write(UCR, FORMAT_8N1_16);
    m.tick(4 * 400);
    assert_eq!(m.usart_take_tx(), None, "the word in flight is gone");
}

/// §6.1.2: the synchronous format (start/stop bits 00) is not modelled: nothing is sent.
#[test]
fn the_synchronous_format_sends_nothing() {
    let mut m = running();
    m.write(UCR, 0x80);
    m.write(TSR, TE);
    m.write(UDR, 1);
    m.tick(4 * 2000);
    assert_eq!(m.usart_take_tx(), None);
}

/// Transmit buffer empty and underrun/end use channels 10 and 9.
#[test]
fn the_transmitter_channels_are_9_and_10() {
    let mut m = running();
    enable(&mut m, CH_TX_EMPTY | CH_TX_ERROR);
    m.write(UCR, 0x08);
    m.write(TSR, TE);
    m.write(UDR, 1);
    m.tick(4);
    assert_eq!(m.iack(), Some(0x0A), "buffer empty");
    m.write(ISRA, !CH_TX_EMPTY); // software end of interrupt (VR S = 1 after reset)
    m.tick(4 * 19);
    assert_eq!(m.iack(), Some(0x09), "underrun");
}
