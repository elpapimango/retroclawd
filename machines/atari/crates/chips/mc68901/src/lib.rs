//! Motorola MC68901 multi-function peripheral (MFP): the general-purpose I/O port, the 16-channel interrupt
//! controller and the four timers, from the datasheet (Motorola, Advance Information, January 1984). The USART is
//! register storage only so far.
//!
//! Time is counted in timer clocks (the XTAL input), passed in by the machine with `tick`; the chip does no I/O and
//! reads no clock of its own.
#![forbid(unsafe_code)]

mod timer;

use serde::{Deserialize, Serialize};

use timer::{Mode, Timer};

/// The timer with an auxiliary input (TAI / TBI, §5.1.2-5.1.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerAb {
    A,
    B,
}

/// Interrupt channel of each timer and of each GPIP line (Table 3-1).
const CH_TIMER: [u8; 4] = [13, 8, 5, 4];
const CH_GPIP: [u8; 8] = [0, 1, 2, 3, 6, 7, 14, 15];
/// The GPIP line whose interrupt channel and edge bit TAI (timer A) and TBI (timer B) use (§5.1.2).
const LINE_TAI: u8 = 4;
const LINE_TBI: u8 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mfp {
    gpip: u8,
    aer: u8,
    ddr: u8,
    /// Channel n is bit n: the A registers hold channels 15-8, the B registers 7-0 (§3.3).
    ier: u16,
    ipr: u16,
    isr: u16,
    imr: u16,
    vr: u8,
    timers: [Timer; 4],
    scr: u8,
    ucr: u8,
    rsr: u8,
    tsr: u8,
    udr: u8,
    /// Levels driven onto the GPIP pins from outside.
    inputs: u8,
    tai: bool,
    tbi: bool,
    /// Each line's transition detector: line level XOR edge bit (§4.2.2 note).
    detector: u8,
}

impl Default for Mfp {
    fn default() -> Self {
        Self::new()
    }
}

impl Mfp {
    /// Power-on: the reset state (§2.2.3); the registers reset leaves alone start at 0, except TSR, whose buffer
    /// empty bit is set ([O] Hatari reads `$80` before the USART is programmed).
    pub fn new() -> Self {
        let mut m = Self {
            gpip: 0,
            aer: 0,
            ddr: 0,
            ier: 0,
            ipr: 0,
            isr: 0,
            imr: 0,
            vr: 0,
            timers: Default::default(),
            scr: 0,
            ucr: 0,
            rsr: 0,
            tsr: 0x80,
            udr: 0,
            inputs: 0,
            tai: false,
            tbi: false,
            detector: 0,
        };
        m.reset();
        m
    }

    /// §2.2.3: every register cleared except the timer data registers, UDR and TSR; VR = `$0F`; timers stopped,
    /// outputs low; the GPIP lines become inputs; pending interrupts cleared.
    pub fn reset(&mut self) {
        self.gpip = 0;
        self.aer = 0;
        self.ddr = 0;
        self.ier = 0;
        self.ipr = 0;
        self.isr = 0;
        self.imr = 0;
        self.vr = 0x0F & 0xF8;
        for t in &mut self.timers {
            t.reset();
        }
        self.scr = 0;
        self.ucr = 0;
        self.rsr = 0;
        self.detector = self.detector_levels();
    }

    /// Register `reg` (0-23, in the order of the datasheet's register map; on the ST at `$FFFA01 + 2 × reg`).
    pub fn read(&self, reg: u8) -> u8 {
        let a = |r: u16| (r >> 8) as u8;
        let b = |r: u16| r as u8;
        match reg {
            0 => (self.gpip & self.ddr) | (self.inputs & !self.ddr),
            1 => self.aer,
            2 => self.ddr,
            3 => a(self.ier),
            4 => b(self.ier),
            5 => a(self.ipr),
            6 => b(self.ipr),
            7 => a(self.isr),
            8 => b(self.isr),
            9 => a(self.imr),
            10 => b(self.imr),
            11 => self.vr,
            // Figure 5-3: the unused bits and the output reset bit read as zero.
            12 => self.timers[0].control(),
            13 => self.timers[1].control(),
            14 => (self.timers[2].control() << 4) | self.timers[3].control(),
            15..=18 => self.timers[(reg - 15) as usize].read_data(),
            19 => self.scr,
            20 => self.ucr,
            21 => self.rsr,
            22 => self.tsr,
            23 => self.udr,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, reg: u8, value: u8) {
        let v = value as u16;
        match reg {
            0 => self.gpip = value,
            1 => self.aer = value,
            2 => self.ddr = value,
            3 => self.ier = (self.ier & 0x00FF) | v << 8,
            4 => self.ier = (self.ier & 0xFF00) | v,
            // §3.3.2 / §3.4.3: writing a zero clears the bit, a one leaves it.
            5 => self.ipr &= (v << 8) | 0x00FF,
            6 => self.ipr &= v | 0xFF00,
            7 => self.isr &= (v << 8) | 0x00FF,
            8 => self.isr &= v | 0xFF00,
            9 => self.imr = (self.imr & 0x00FF) | v << 8,
            10 => self.imr = (self.imr & 0xFF00) | v,
            11 => {
                // Figure 3-2: bits 2-0 unused; S = 0 forces the in-service bits low (§3.4.2).
                self.vr = value & 0xF8;
                if self.vr & 0x08 == 0 {
                    self.isr = 0;
                }
            }
            12 | 13 => {
                let t = &mut self.timers[(reg - 12) as usize];
                if value & 0x10 != 0 {
                    t.force_output_low();
                }
                t.set_control(value & 0x0F);
            }
            14 => {
                self.timers[2].set_control((value >> 4) & 7);
                self.timers[3].set_control(value & 7);
            }
            15..=18 => self.timers[(reg - 15) as usize].write_data(value),
            19 => self.scr = value,
            20 => self.ucr = value,
            // [H?] the low four bits of RSR and TSR are the programmable ones; the upper (status) bits keep their
            // value until the USART is emulated.
            21 => self.rsr = (self.rsr & 0xF0) | (value & 0x0F),
            22 => self.tsr = (self.tsr & 0xF0) | (value & 0x0F),
            23 => self.udr = value,
            _ => {}
        }
        // §3.3.1: disabling a channel clears its pending bit.
        self.ipr &= self.ier;
        // Changing the data, the direction, an edge bit or a timer mode can move a detector (§4.2.2, §5.1.2).
        self.update_detectors();
    }

    /// Drives GPIP line `line` (0-7) from outside.
    pub fn set_input(&mut self, line: u8, level: bool) {
        self.inputs = set_bit(self.inputs, line, level);
        self.update_detectors();
    }

    /// Drives TAI or TBI.
    pub fn timer_input(&mut self, t: TimerAb, level: bool) {
        match t {
            TimerAb::A => self.tai = level,
            TimerAb::B => self.tbi = level,
        }
        self.update_detectors();
    }

    /// Lets `clocks` timer clocks pass.
    pub fn tick(&mut self, clocks: u64) {
        for (i, &ch) in CH_TIMER.iter().enumerate() {
            let active = self.aux_active(i);
            if self.timers[i].advance(clocks, active) > 0 {
                self.request(ch);
            }
        }
    }

    /// Timer clocks until the earliest timeout of a running timer.
    pub fn next_timeout(&self) -> Option<u64> {
        (0..4)
            .filter_map(|i| self.timers[i].next_timeout(self.aux_active(i)))
            .min()
    }

    /// The IRQ output (active when a channel requests service, §3.3.3, §3.4.3).
    pub fn irq(&self) -> bool {
        self.requesting().is_some()
    }

    /// The interrupt-acknowledge cycle: the vector of the highest requesting channel (§3.1.2), its pending bit
    /// cleared and, in software end-of-interrupt mode, its in-service bit set (§3.4.3). `None`: nothing requests.
    pub fn iack(&mut self) -> Option<u8> {
        let ch = self.requesting()?;
        self.ipr &= !(1 << ch);
        if self.vr & 0x08 != 0 {
            self.isr |= 1 << ch;
        }
        Some((self.vr & 0xF0) | ch)
    }

    /// TAO-TDO (index 0-3).
    pub fn timer_output(&self, timer: usize) -> bool {
        self.timers[timer].output()
    }

    fn requesting(&self) -> Option<u8> {
        let ch = 15 - (self.ipr & self.imr).leading_zeros() as i32;
        if ch < 0 {
            return None;
        }
        // An in-service channel blocks itself and every lower one.
        let in_service = 15 - self.isr.leading_zeros() as i32;
        (ch > in_service).then_some(ch as u8)
    }

    fn request(&mut self, ch: u8) {
        if self.ier & (1 << ch) != 0 {
            self.ipr |= 1 << ch;
        }
    }

    /// Whether timer `i` (A or B) uses its auxiliary input, which then replaces the GPIP line on its channel.
    fn aux_mode(&self, i: usize) -> bool {
        i < 2
            && matches!(
                self.timers[i].mode(),
                Mode::EventCount | Mode::PulseWidth(_)
            )
    }

    /// Pulse width mode runs while the auxiliary input is at the level the edge bit names (§5.1.2), i.e. while
    /// its detector is low.
    fn aux_active(&self, i: usize) -> bool {
        let line = if i == 0 { LINE_TAI } else { LINE_TBI };
        i < 2 && self.detector & (1 << line) == 0
    }

    fn detector_levels(&self) -> u8 {
        let mut level = (self.gpip & self.ddr) | (self.inputs & !self.ddr);
        if self.aux_mode(0) {
            level = set_bit(level, LINE_TAI, self.tai);
        }
        if self.aux_mode(1) {
            level = set_bit(level, LINE_TBI, self.tbi);
        }
        level ^ self.aer
    }

    /// §4.2.2: a line interrupts when its detector (level XOR edge bit) goes from 1 to 0, so AER 0 means a falling
    /// input and AER 1 a rising one. In event count mode that transition also counts (§5.1.3); in pulse width mode
    /// the interrupt comes on the opposite transition, when the pulse ends (§5.1.2).
    fn update_detectors(&mut self) {
        let new = self.detector_levels();
        let fell = self.detector & !new;
        let rose = !self.detector & new;
        self.detector = new;
        for line in 0..8u8 {
            let bit = 1 << line;
            let timer = match line {
                LINE_TAI => Some(0),
                LINE_TBI => Some(1),
                _ => None,
            };
            let mode = timer.map(|i| self.timers[i].mode());
            let interrupt = match mode {
                Some(Mode::PulseWidth(_)) => rose & bit != 0,
                _ => fell & bit != 0,
            };
            if interrupt {
                self.request(CH_GPIP[line as usize]);
            }
            if let (Some(i), Some(Mode::EventCount)) = (timer, mode)
                && fell & bit != 0
                && self.timers[i].count(1) > 0
            {
                self.request(CH_TIMER[i]);
            }
        }
    }
}

fn set_bit(byte: u8, bit: u8, level: bool) -> u8 {
    if level {
        byte | 1 << bit
    } else {
        byte & !(1 << bit)
    }
}
