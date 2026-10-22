//! The two MC6850 ACIAs (`docs/hardware-facts.md`, "ACIAs"): keyboard at `$FFFC00/02`, MIDI at `$FFFC04/06`
//! ([H] *Internals* p.62).
//!
//! - [H] *Internals* p.47: the ACIAs run from 500 kHz; [H?] that is the CPU clock / 16 (Hatari 2.6.1 uses 501 327 Hz,
//!   the same ratio). A bit is `divide × 16` CPU cycles.
//! - [H] *Internals* p.47: CTS, DCD and RTS are not connected; [O] status `$02` after the reset shows CTS and DCD
//!   low.
//! - [H] both IRQ outputs share GPIP I4 (*Internals* p.61), MFP channel 6 (*TOS hyp* `$118`); the line is active low.
//!   [O] it reads low until the first master reset (Hatari's GPIP `$A1` before EmuTOS resets the ACIAs, `$B1`
//!   after).
//! - [O] the transmitter takes a written character at its next bit-clock edge (Hatari's TDRE comes back within one
//!   bit); [H?] the bit clock's edges are counted from the last control-register write.
//!
//! Each port has one scheduled service event; a generation number tells a stale one apart.

use ikbd::Ikbd;
use mc6850::Acia;
use serde::{Deserialize, Serialize};

/// The other end of a serial line.
pub trait SerialPeer {
    /// A complete character arrived from the ST at CPU cycle `at`.
    fn receive(&mut self, byte: u8, at: u64);
    /// The next character the peer wants to send and the earliest cycle it can start, if any (called when the line
    /// is free).
    fn next_byte(&mut self, now: u64) -> Option<(u64, u8)>;
    /// CPU cycles one character takes on the line, from the peer's side.
    fn frame_cycles(&self) -> u64;
}

/// Nothing connected (the MIDI port in Phase 1).
pub struct NullPeer;

impl SerialPeer for NullPeer {
    fn receive(&mut self, _: u8, _: u64) {}
    fn next_byte(&mut self, _: u64) -> Option<(u64, u8)> {
        None
    }
    fn frame_cycles(&self) -> u64 {
        1
    }
}

/// [H] *Internals* p.47: the keyboard line runs at 7812.5 baud, 8N1; [H?] the IKBD's own clock gives the same bit
/// time as the ACIA's ÷64 (1024 CPU cycles), 10 bits a character.
pub const IKBD_FRAME_CYCLES: u64 = 10 * 64 * ACIA_DIVIDER;

impl SerialPeer for Ikbd {
    fn receive(&mut self, byte: u8, at: u64) {
        self.command_byte(byte, at);
    }

    fn next_byte(&mut self, now: u64) -> Option<(u64, u8)> {
        let at = self.next_output_at()?;
        let byte = self.pop_output(at)?;
        Some((at.max(now), byte))
    }

    fn frame_cycles(&self) -> u64 {
        IKBD_FRAME_CYCLES
    }
}

/// What a port's line is connected to: the IKBD (the keyboard port), or something else (tests, the MIDI port).
pub enum Peer {
    Ikbd(Box<Ikbd>),
    Other(Box<dyn SerialPeer + Send>),
}

impl Peer {
    fn get(&mut self) -> &mut dyn SerialPeer {
        match self {
            Peer::Ikbd(k) => k.as_mut(),
            Peer::Other(p) => p.as_mut(),
        }
    }
}

/// CPU clocks per ACIA clock.
pub const ACIA_DIVIDER: u64 = 16;

/// [H] *Internals* p.47: the MIDI ACIA's clock is divided by 16 (31 250 baud), 8N1: ten bits a character.
pub const MIDI_FRAME_CYCLES: u64 = 10 * 16 * ACIA_DIVIDER;

/// One port's state as data. The keyboard port's peer is the IKBD and is saved with it; any other peer (the MIDI
/// port's, a test's) belongs to the host and is left as it is.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PortState {
    chip: Acia,
    ikbd: Option<Ikbd>,
    phase: u64,
    tx_start: Option<u64>,
    tx_end: Option<u64>,
    tx_started_at: Option<u64>,
    rx: Option<(u64, u8)>,
    never_reset: bool,
    generation: u64,
}

pub struct Port {
    pub(crate) chip: Acia,
    pub(crate) peer: Peer,
    /// The cycle of the last control-register write: the bit clock's phase.
    phase: u64,
    /// The transmitter takes the TDR at this cycle.
    tx_start: Option<u64>,
    /// The character in the shift register is out at this cycle.
    tx_end: Option<u64>,
    tx_started_at: Option<u64>,
    /// A character from the peer completes at this cycle.
    rx: Option<(u64, u8)>,
    /// No master reset yet: [O] the IRQ line reads low.
    never_reset: bool,
    /// The generation of the one current service event.
    pub(crate) generation: u64,
}

impl Port {
    fn new(peer: Peer) -> Self {
        Self {
            chip: Acia::new(),
            peer,
            phase: 0,
            tx_start: None,
            tx_end: None,
            tx_started_at: None,
            rx: None,
            never_reset: true,
            generation: 0,
        }
    }

    pub(crate) fn snapshot(&self) -> PortState {
        PortState {
            chip: self.chip.clone(),
            ikbd: match &self.peer {
                Peer::Ikbd(k) => Some((**k).clone()),
                Peer::Other(_) => None,
            },
            phase: self.phase,
            tx_start: self.tx_start,
            tx_end: self.tx_end,
            tx_started_at: self.tx_started_at,
            rx: self.rx,
            never_reset: self.never_reset,
            generation: self.generation,
        }
    }

    pub(crate) fn restore(&mut self, s: PortState) {
        self.chip = s.chip;
        if let Some(k) = s.ikbd {
            self.peer = Peer::Ikbd(Box::new(k));
        }
        self.phase = s.phase;
        self.tx_start = s.tx_start;
        self.tx_end = s.tx_end;
        self.tx_started_at = s.tx_started_at;
        self.rx = s.rx;
        self.never_reset = s.never_reset;
        self.generation = s.generation;
    }

    fn bit(&self) -> u64 {
        self.chip.divide() as u64 * ACIA_DIVIDER
    }

    /// The IRQ line, true = asserted (pulled low).
    pub(crate) fn irq_line(&self) -> bool {
        self.never_reset || self.chip.irq()
    }

    /// The register at `rs` read by the CPU at `now`.
    pub(crate) fn read(&mut self, rs: bool, now: u64) -> u8 {
        let v = self.chip.read(rs);
        if rs {
            self.pull_rx(now);
        }
        v
    }

    pub(crate) fn write(&mut self, rs: bool, value: u8, now: u64) {
        self.chip.write(rs, value);
        if !rs {
            self.phase = now;
            if value & 0x03 == 0x03 {
                self.never_reset = false;
                self.tx_start = None;
                self.tx_end = None;
            }
        } else if self.tx_end.is_none() && self.tx_start.is_none() {
            self.schedule_tx_start(now);
        }
        self.pull_rx(now);
    }

    fn schedule_tx_start(&mut self, now: u64) {
        let bit = self.bit();
        if bit == 0 {
            return;
        }
        let since = now.saturating_sub(self.phase);
        self.tx_start = Some(self.phase + since.div_ceil(bit) * bit);
    }

    /// Forgets a character on its way from the old peer (a new cable).
    pub(crate) fn drop_rx(&mut self) {
        self.rx = None;
    }

    /// Asks the peer for its next character when the receive line is free.
    pub(crate) fn pull_rx(&mut self, now: u64) {
        if self.rx.is_none()
            && let Some((start, byte)) = self.peer.get().next_byte(now)
        {
            self.rx = Some((start.max(now) + self.peer.get().frame_cycles(), byte));
        }
    }

    /// The earliest pending action.
    pub(crate) fn next_due(&self) -> Option<u64> {
        [self.tx_start, self.tx_end, self.rx.map(|(t, _)| t)]
            .into_iter()
            .flatten()
            .min()
    }

    /// Everything due by `at`, in time order.
    pub(crate) fn service(&mut self, at: u64) {
        while let Some(t) = self.next_due().filter(|&t| t <= at) {
            if self.tx_end == Some(t) {
                self.tx_end = None;
                if let Some(byte) = self.chip.take_tx() {
                    self.peer.get().receive(byte, t);
                    // A command may have a reply.
                    self.pull_rx(t);
                }
                // A character already waiting in the TDR follows back to back.
                if self.chip.start_tx() {
                    self.tx_started_at = Some(t);
                    self.tx_end = Some(t + self.chip.frame_bits() as u64 * self.bit());
                }
            } else if self.tx_start == Some(t) {
                self.tx_start = None;
                if self.chip.start_tx() {
                    self.tx_started_at = Some(t);
                    self.tx_end = Some(t + self.chip.frame_bits() as u64 * self.bit());
                }
            } else if let Some((rt, byte)) = self.rx.filter(|(rt, _)| *rt == t) {
                self.rx = None;
                self.chip.rx_byte(byte);
                self.pull_rx(rt);
            }
        }
    }
}

pub struct Acias {
    pub(crate) ports: [Port; 2],
}

impl Acias {
    pub fn new() -> Self {
        Self {
            ports: [
                Port::new(Peer::Ikbd(Box::new(Ikbd::new()))),
                Port::new(Peer::Other(Box::new(NullPeer))),
            ],
        }
    }

    /// GPIP I4: high unless either ACIA asserts its IRQ.
    pub(crate) fn gpip_level(&self) -> bool {
        !self.ports.iter().any(Port::irq_line)
    }

    /// The IKBD, when it is connected to the keyboard port.
    pub(crate) fn ikbd(&mut self) -> Option<&mut Ikbd> {
        match &mut self.ports[0].peer {
            Peer::Ikbd(k) => Some(k),
            Peer::Other(_) => None,
        }
    }

    pub(crate) fn tx_started_at(&self, port: usize) -> Option<u64> {
        self.ports[port].tx_started_at
    }
}

impl Default for Acias {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Acias {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Acias")
            .field("keyboard", &self.ports[0].chip)
            .field("midi", &self.ports[1].chip)
            .finish()
    }
}
