//! The MFP's USART (datasheet section 6): UCR, RSR, TSR, UDR, the transmitter and receiver, loopback and the four
//! interrupt conditions. Asynchronous formats only; the synchronous format (UCR start/stop bits `00`) is not
//! modelled ([H?]: a transmitter in that format never finishes a character, a receiver never assembles one).
//!
//! Time is counted in time-outs of timer D, which drives both USART clocks on the ST (*Internals* p.61: "Timer D ...
//! sends the timing signal for the MFP's built-in serial interface"). The timer output toggles on every time-out
//! (§5.1.1), so one clock period is two time-outs; a bit is one clock period, or sixteen in divide-by-16 mode (UCR
//! CLK, §6.1). The chip holds no peer: finished characters wait for the machine in a queue, and the machine hands
//! received characters in with [`Usart::rx_begin`]. A peer may mark a character faulty ([`RxFault`]): the parity error, frame
//! error and break flags of the RSR then describe the character in the receive buffer ([H?]: no data sheet text was to
//! hand for their clearing, so each flag is rewritten with every character that reaches the buffer) and the receive
//! error interrupt is raised as for an overrun.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Transmit error channel events (UE, END), channel 9.
pub(crate) const EV_TX_ERROR: u8 = 1;
/// Transmit buffer empty, channel 10.
pub(crate) const EV_TX_EMPTY: u8 = 2;
/// Receive error (overrun), channel 11: raised on channel 12 when channel 11 is disabled (§6.2.1).
pub(crate) const EV_RX_ERROR: u8 = 4;
/// Receive buffer full, channel 12.
pub(crate) const EV_RX_FULL: u8 = 8;

// RSR bits (Figure 6-3).
const RSR_BF: u8 = 0x80;
const RSR_OE: u8 = 0x40;
const RSR_PE: u8 = 0x20;
const RSR_FE: u8 = 0x10;
/// Found/search in a synchronous format; in an asynchronous one it flags a break (all zeros through the stop bit).
const RSR_B: u8 = 0x08;
const RSR_CIP: u8 = 0x04;
const RSR_SS: u8 = 0x02;
const RSR_RE: u8 = 0x01;
// TSR bits (Figure 6-4).
const TSR_BE: u8 = 0x80;
const TSR_UE: u8 = 0x40;
const TSR_AT: u8 = 0x20;
const TSR_END: u8 = 0x10;
const TSR_B: u8 = 0x08;
const TSR_H: u8 = 0x04;
const TSR_L: u8 = 0x02;
const TSR_TE: u8 = 0x01;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Shift {
    byte: u8,
    /// Timer D time-outs until the character is out (or in).
    left: u64,
    /// A received character's fault (`RxFault::code`); 0 for a clean one and for everything transmitted.
    fault: u8,
}

/// What is wrong with a character a peer sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RxFault {
    /// A clean frame.
    None,
    /// The parity bit disagrees (shows only when the UCR enables parity).
    Parity,
    /// The stop bit is not a stop bit.
    Frame,
    /// The line was held low for the whole frame: the data is 0, with a frame error and the break flag.
    Break,
}

impl RxFault {
    fn code(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Usart {
    scr: u8,
    ucr: u8,
    rsr: u8,
    tsr: u8,
    /// The receive buffer, read through UDR.
    rx_buf: u8,
    tx_buf: Option<u8>,
    tx_shift: Option<Shift>,
    rx_shift: Option<Shift>,
    /// A word arrived while the receive buffer was full: OE rises when the buffer is read (§6.2.2).
    overrun_pending: bool,
    /// Characters out of the transmitter, for the peer.
    tx_done: VecDeque<u8>,
}

impl Usart {
    /// Power-on: [O] TSR reads `$80` (the buffer is empty), everything else zero.
    pub(crate) fn new() -> Self {
        Self {
            scr: 0,
            ucr: 0,
            rsr: 0,
            tsr: TSR_BE,
            rx_buf: 0,
            tx_buf: None,
            tx_shift: None,
            rx_shift: None,
            overrun_pending: false,
            tx_done: VecDeque::new(),
        }
    }

    /// §2.2.3: every register is cleared except UDR and TSR, and the receiver and transmitter stop. [H?] Figure 6-4
    /// says reset clears TE while §2.2.3 leaves TSR alone; TSR is left alone, as the register list says.
    pub(crate) fn reset(&mut self) {
        self.scr = 0;
        self.ucr = 0;
        self.rsr = 0;
        self.tx_shift = None;
        self.rx_shift = None;
        self.overrun_pending = false;
    }

    pub(crate) fn read_scr(&self) -> u8 {
        self.scr
    }

    pub(crate) fn write_scr(&mut self, v: u8) {
        self.scr = v;
    }

    pub(crate) fn read_ucr(&self) -> u8 {
        // Figure 6-2: bit 0 is unused and reads as zero.
        self.ucr & 0xFE
    }

    pub(crate) fn write_ucr(&mut self, v: u8) {
        self.ucr = v & 0xFE;
    }

    /// Reading the RSR clears OE (§6.2.2).
    pub(crate) fn read_rsr(&mut self) -> u8 {
        let v = self.rsr;
        self.rsr &= !RSR_OE;
        v
    }

    /// RE and SS are the writable bits; clearing RE disables the receiver at once and clears every flag (§6.2.2).
    pub(crate) fn write_rsr(&mut self, v: u8) {
        if v & RSR_RE == 0 {
            self.rsr = v & RSR_SS;
            self.rx_shift = None;
            self.overrun_pending = false;
        } else {
            self.rsr = (self.rsr & !(RSR_RE | RSR_SS)) | (v & (RSR_RE | RSR_SS));
        }
    }

    /// Reading TSR clears UE (§6.3.2).
    pub(crate) fn read_tsr(&mut self) -> u8 {
        let v = self.tsr;
        self.tsr &= !TSR_UE;
        v
    }

    /// The writable bits are AT, B, H, L and TE; BE, UE and END are status. Disabling the transmitter clears UE and,
    /// with nothing in the shift register, sets END at once (and runs the auto-turnaround); enabling it clears END.
    pub(crate) fn write_tsr(&mut self, v: u8) -> u8 {
        let was = self.tsr & TSR_TE != 0;
        let keep = TSR_BE | TSR_UE | TSR_END;
        self.tsr = (self.tsr & keep) | (v & (TSR_AT | TSR_B | TSR_H | TSR_L | TSR_TE));
        let now = self.tsr & TSR_TE != 0;
        let mut ev = 0;
        if was && !now {
            self.tsr &= !TSR_UE;
            if self.tx_shift.is_none() {
                self.tx_finished(&mut ev);
            }
        } else if !was && now {
            self.tsr &= !TSR_END;
        }
        ev
    }

    /// The receive buffer; reading it clears BF and lets a held-back overrun show (§6.2, §6.2.2).
    pub(crate) fn read_udr(&mut self) -> (u8, u8) {
        let mut ev = 0;
        self.rsr &= !RSR_BF;
        if self.overrun_pending {
            self.overrun_pending = false;
            self.rsr |= RSR_OE;
            ev |= EV_RX_ERROR;
        }
        (self.rx_buf, ev)
    }

    /// A write loads the transmit buffer and clears BE (§6.3.2); it may be loaded before the transmitter is enabled.
    pub(crate) fn write_udr(&mut self, v: u8) {
        self.tx_buf = Some(v);
        self.tsr &= !TSR_BE;
    }

    fn word_bits(&self) -> u64 {
        // Figure 6-2 WL1 WL0 (bits 6-5): 8, 7, 6, 5 bits.
        8 - ((self.ucr >> 5) & 3) as u64
    }

    fn word_mask(&self) -> u8 {
        (0xFFu16 >> (8 - self.word_bits())) as u8
    }

    /// Time-outs of timer D one character takes, or `None` in the synchronous format.
    fn char_timeouts(&self) -> Option<u64> {
        let div = if self.ucr & 0x80 != 0 { 16 } else { 1 };
        let stop = match (self.ucr >> 3) & 3 {
            0 => return None,
            1 => div,
            2 => div + div / 2,
            _ => 2 * div,
        };
        let parity = u64::from(self.ucr & 0x04 != 0);
        // One start bit, the data, the parity bit, then the stop bits; a clock period is two time-outs.
        Some(2 * ((1 + self.word_bits() + parity) * div + stop))
    }

    fn loopback(&self) -> bool {
        self.tsr & (TSR_H | TSR_L) == TSR_H | TSR_L
    }

    /// Whether the receiver would take a character now: enabled, idle, no overrun to acknowledge, an asynchronous
    /// format, and not wired to the transmitter.
    pub(crate) fn can_receive(&self) -> bool {
        self.rsr & RSR_RE != 0
            && self.rsr & RSR_OE == 0
            && self.rx_shift.is_none()
            && self.char_timeouts().is_some()
            && !self.loopback()
    }

    /// A character from the line starts coming in.
    pub(crate) fn rx_begin(&mut self, byte: u8, fault: RxFault) {
        if let Some(left) = self.char_timeouts() {
            self.rx_shift = Some(Shift {
                byte,
                left,
                fault: fault.code(),
            });
            self.rsr |= RSR_CIP;
        }
    }

    /// The oldest finished character for the peer.
    pub(crate) fn take_tx(&mut self) -> Option<u8> {
        self.tx_done.pop_front()
    }

    fn load_shifter(&mut self, ev: &mut u8) {
        if self.tx_shift.is_none()
            && self.tsr & TSR_TE != 0
            && self.tsr & TSR_B == 0
            && let Some(left) = self.char_timeouts()
            && let Some(byte) = self.tx_buf.take()
        {
            self.tx_shift = Some(Shift {
                byte,
                left,
                fault: 0,
            });
            self.tsr |= TSR_BE;
            *ev |= EV_TX_EMPTY;
        }
    }

    /// The shift register is empty and nothing refilled it.
    fn tx_finished(&mut self, ev: &mut u8) {
        if self.tsr & TSR_TE != 0 {
            // Enabled but starved (§6.3 underrun).
            self.tsr |= TSR_UE;
        } else {
            self.tsr |= TSR_END;
            // Auto-turnaround: the receiver comes on after the last character (Figure 6-4 AT).
            if self.tsr & TSR_AT != 0 {
                self.tsr &= !TSR_AT;
                self.rsr |= RSR_RE;
            }
        }
        *ev |= EV_TX_ERROR;
    }

    fn rx_complete(&mut self, byte: u8, fault: u8, ev: &mut u8) {
        self.rsr &= !RSR_CIP;
        if self.rsr & RSR_BF != 0 {
            // §6.2.2: neither the buffer nor the RSR is overwritten; OE shows once the buffer has been read.
            self.overrun_pending = true;
        } else {
            let fault = match fault {
                1 if self.ucr & 0x04 != 0 => RxFault::Parity,
                2 => RxFault::Frame,
                3 => RxFault::Break,
                _ => RxFault::None,
            };
            self.rx_buf = if fault == RxFault::Break { 0 } else { byte } & self.word_mask();
            self.rsr &= !(RSR_PE | RSR_FE | RSR_B);
            match fault {
                RxFault::Parity => self.rsr |= RSR_PE,
                RxFault::Frame => self.rsr |= RSR_FE,
                RxFault::Break => self.rsr |= RSR_FE | RSR_B,
                RxFault::None => {}
            }
            self.rsr |= RSR_BF;
            *ev |= EV_RX_FULL;
            if fault != RxFault::None {
                *ev |= EV_RX_ERROR;
            }
        }
    }

    /// Lets `n` time-outs of timer D pass; returns the events.
    pub(crate) fn clock(&mut self, mut n: u64) -> u8 {
        let mut ev = 0;
        while n > 0 {
            self.load_shifter(&mut ev);
            let step = [self.tx_shift.map(|s| s.left), self.rx_shift.map(|s| s.left)]
                .into_iter()
                .flatten()
                .min();
            let Some(step) = step.map(|s| s.min(n)) else {
                break;
            };
            n -= step;
            if let Some(s) = &mut self.tx_shift {
                s.left -= step;
                if s.left == 0 {
                    let byte = s.byte & self.word_mask();
                    self.tx_shift = None;
                    self.tx_done.push_back(byte);
                    if self.loopback() {
                        self.rx_complete(byte, 0, &mut ev);
                    }
                    self.load_shifter(&mut ev);
                    if self.tx_shift.is_none() {
                        self.tx_finished(&mut ev);
                    }
                }
            }
            if let Some(s) = &mut self.rx_shift {
                s.left -= step;
                if s.left == 0 {
                    let (byte, fault) = (s.byte, s.fault);
                    self.rx_shift = None;
                    self.rx_complete(byte, fault, &mut ev);
                }
            }
        }
        ev
    }
}
