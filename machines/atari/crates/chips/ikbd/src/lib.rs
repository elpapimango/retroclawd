//! The Atari ST's intelligent keyboard controller (an HD6301 with Atari's firmware), modelled by its behaviour as
//! Atari's *Intelligent Keyboard (ikbd) Protocol* (revised 26 February 1985) describes it: commands in, key codes,
//! mouse, joystick and clock reports out. It is not a 6301 core. Section numbers below are the document's.
//!
//! Facts not in the document are labelled as in `docs/hardware-facts.md`: [O] observed in Hatari 2.6.1 (black box),
//! [H?] a documented choice.
//!
//! Time is the host's CPU cycle count; the crate holds no clock of its own beyond the time-of-day counter and the
//! sample timers, which the host advances (`tick_clock`, or the `at` of any call). The host tells the crate its CPU
//! clock once with [`Ikbd::set_cycles_per_second`].
#![forbid(unsafe_code)]

mod scancodes;

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

pub use scancodes::{SCANCODES, scancode};

/// [O] CPU cycles from receiving RESET's second byte (or power-up) to the version code being ready: Hatari 2.6.1
/// sends it 502 002 cycles (62.6 ms at 8 MHz) later; §9.1 allows up to 300 ms.
pub const RESET_REPLY_DELAY: u64 = 502_002;
/// [O] CPU cycles from receiving the last byte of an interrogation command (`$0D`, `$16`, `$1C`, `$21`) to the answer being
/// ready, and likewise for a status inquiry (`$87` to `$9A`): means of Hatari 2.6.1's first-answer latencies in probe P20 (456
/// and 430 loop runs of 60 clocks against 347 with no delay; Hatari's own figures vary by about 30 runs from one start to
/// another, the 6301's loop position). The document leaves them open.
pub const QUERY_REPLY_DELAY: u64 = 6_500;
pub const STATUS_REPLY_DELAY: u64 = 5_000;
/// [O] the version code: §8 gives `$F0` for the first release and `$F1` for a second; Hatari's ST answers `$F1`.
pub const VERSION: u8 = 0xF1;
/// [H?] bytes the controller can hold for the host. The 6301 has 128 bytes of RAM (*Internals* p.69); the document
/// leaves the queue size `<tbd>` (§9.14). A packet that does not fit whole is dropped; older bytes are kept.
pub const QUEUE_BYTES: usize = 256;
/// CPU cycles one character takes on the keyboard line (7812.5 baud, 8N1: 1024 cycles a bit at 8 MHz). [H?] the
/// IKBD's own bit clock equals the ACIA's (see `docs/hardware-facts.md`). §5.4 paces fire button monitoring by it.
pub const BYTE_CYCLES: u64 = 10_240;
/// §4.3 and §5.5: the pseudo key codes of the mouse buttons (and joystick fire buttons) in key modes.
const LEFT_BUTTON_KEY: u8 = 0x74;
const RIGHT_BUTTON_KEY: u8 = 0x75;
/// §4.3: cursor key codes (Appendix A).
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
/// Samples a catch-up generates at most: more than the queue could hold anyway (a host that did not run the
/// controller for a long time must not make it loop).
const MAX_CATCH_UP: u64 = 512;
/// [H?] §9.24 allows 0-128 bytes per load; the controller memory model keeps at most this many addresses.
const MEMORY_MODEL_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum MouseMode {
    /// §9.3.
    Relative,
    /// §9.4: maxima in scaled units.
    Absolute { x_max: u16, y_max: u16 },
    /// §9.5: clicks per cursor key.
    Keycode { dx: u8, dy: u8 },
}

/// §9.20: tenths of a second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct KeyTimes {
    rx: u8,
    ry: u8,
    tx: u8,
    ty: u8,
    vx: u8,
    vy: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum JoyMode {
    /// §9.15 (`$14`).
    Event,
    /// §9.16 (`$15`).
    Interrogation,
    /// §9.18 (`$17`): samples every `rate` hundredths of a second.
    Monitoring { rate: u8 },
    /// §9.19 (`$18`).
    FireButton,
    /// §9.20 (`$19`).
    Keycode(KeyTimes),
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct Axis {
    /// -1 (up/left), 0, +1 (down/right).
    dir: i8,
    /// The cycle the direction was taken.
    since: u64,
    /// The next repeat of the key, if it repeats.
    next: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Packet {
    at: u64,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ikbd {
    queue: VecDeque<Packet>,
    /// Bytes of the front packet already sent.
    sent: usize,
    paused: bool,
    /// The command bytes received so far.
    cmd: Vec<u8>,
    mouse: MouseMode,
    mouse_enabled: bool,
    button_action: u8,
    threshold: (u8, u8),
    scale: (u8, u8),
    y_at_bottom: bool,
    /// Accumulated unreported motion (relative and key code modes).
    acc: (i32, i32),
    /// Absolute position in scaled units, and the clicks not yet a whole unit.
    abs: (i32, i32),
    abs_sub: (i32, i32),
    /// Button changes since the last interrogation (§9.8), bits as reported there.
    abs_buttons: u8,
    left: bool,
    right: bool,
    joy_mode: JoyMode,
    joy_enabled: bool,
    /// §8: after a joystick command port 0 is a joystick too.
    port0_joystick: bool,
    joy: [u8; 2],
    /// §5.3, §5.4: the next sample of the monitoring modes, and the fire button samples packed so far.
    sample_at: u64,
    fire_bits: u8,
    fire_count: u8,
    /// §5.5: the cursor key repeat of the two axes (x, y) of joystick 0.
    axes: [Axis; 2],
    /// yy MM DD hh mm ss, packed BCD; the cycle of the last whole second.
    clock: [u8; 6],
    clock_at: u64,
    cycles_per_second: u64,
    /// Cycles an answer to a query or a status inquiry waits after its command (0: the document's, immediate). Set by the
    /// host from the [O] constants; not part of a state.
    #[serde(skip)]
    query_delay: u64,
    #[serde(skip)]
    status_delay: u64,
    /// [H?] §9.24-9.26: a model of the controller memory the host loaded; nothing else is there.
    memory: BTreeMap<u16, u8>,
    /// [H?] the addresses of §9.26 controller-execute commands received (the program is not run), newest last.
    executed: Vec<u16>,
}

impl Default for Ikbd {
    fn default() -> Self {
        Self::new()
    }
}

impl Ikbd {
    /// Power-up: the defaults of §8; the version code is ready after the self-test. [O] the clock reads all zeros.
    pub fn new() -> Self {
        let mut k = Self {
            queue: VecDeque::new(),
            sent: 0,
            paused: false,
            cmd: Vec::new(),
            mouse: MouseMode::Relative,
            mouse_enabled: true,
            button_action: 0,
            threshold: (1, 1),
            scale: (1, 1),
            y_at_bottom: false,
            acc: (0, 0),
            abs: (0, 0),
            abs_sub: (0, 0),
            abs_buttons: 0,
            left: false,
            right: false,
            joy_mode: JoyMode::Event,
            joy_enabled: true,
            port0_joystick: false,
            joy: [0; 2],
            sample_at: 0,
            fire_bits: 0,
            fire_count: 0,
            axes: [Axis::default(); 2],
            clock: [0; 6],
            clock_at: 0,
            cycles_per_second: 0,
            query_delay: 0,
            status_delay: 0,
            memory: BTreeMap::new(),
            executed: Vec::new(),
        };
        k.push(RESET_REPLY_DELAY, &[VERSION]);
        k
    }

    /// The delays of the answers to queries and status inquiries ([`QUERY_REPLY_DELAY`], [`STATUS_REPLY_DELAY`] on the ST).
    pub fn set_reply_delays(&mut self, query: u64, status: u64) {
        self.query_delay = query;
        self.status_delay = status;
    }

    /// The host's CPU clock, which converts the document's hundredths and tenths of a second to cycles.
    pub fn set_cycles_per_second(&mut self, cps: u64) {
        self.cycles_per_second = cps;
    }

    /// §9.1: back to the power-up modes; the clock is kept. [H?] a packet being sent when the reset arrives is cut
    /// off, the output queue is emptied, and the loaded controller memory is gone (the self-test runs again).
    fn reset(&mut self, at: u64) {
        let (clock, clock_at, cps) = (self.clock, self.clock_at, self.cycles_per_second);
        let delays = (self.query_delay, self.status_delay);
        *self = Self::new();
        self.queue.clear();
        self.clock = clock;
        self.clock_at = clock_at;
        self.cycles_per_second = cps;
        (self.query_delay, self.status_delay) = delays;
        self.push(at + RESET_REPLY_DELAY, &[VERSION]);
    }

    fn queued_bytes(&self) -> usize {
        self.queue.iter().map(|p| p.bytes.len()).sum::<usize>() - self.sent
    }

    fn push(&mut self, at: u64, bytes: &[u8]) {
        if self.queued_bytes() + bytes.len() <= QUEUE_BYTES {
            self.queue.push_back(Packet {
                at,
                bytes: bytes.to_vec(),
            });
        }
    }

    /// The next byte for the host, if one is ready at `now` (§9.14: a paused controller finishes the packet it has
    /// begun).
    pub fn pop_output(&mut self, now: u64) -> Option<u8> {
        self.advance(now);
        self.pop_queued(now)
    }

    /// Lets everything that happens by itself happen up to `now` (the host does this before it asks what is queued).
    pub fn advance_to(&mut self, now: u64) {
        self.advance(now);
    }

    /// Like [`Ikbd::pop_output`] but without running the controller to `now`: for a byte stamped in the future (the
    /// reset reply), which must not move the clock ahead.
    pub fn pop_queued(&mut self, now: u64) -> Option<u8> {
        let front = self.queue.front()?;
        if front.at > now || (self.paused && self.sent == 0) {
            return None;
        }
        let byte = front.bytes[self.sent];
        self.sent += 1;
        if self.sent == front.bytes.len() {
            self.queue.pop_front();
            self.sent = 0;
        }
        Some(byte)
    }

    /// When the next queued byte will be ready, if any (`None` while paused between packets).
    pub fn next_output_at(&self) -> Option<u64> {
        let front = self.queue.front()?;
        if self.paused && self.sent == 0 {
            return None;
        }
        Some(front.at)
    }

    /// The cycle at which the controller will produce output by itself (a monitoring sample or a key repeat), if it
    /// is going to; the host calls [`Ikbd::pop_output`] then.
    pub fn next_wake(&self) -> Option<u64> {
        let sampling = match self.joy_mode {
            JoyMode::Monitoring { .. } | JoyMode::FireButton
                if self.joy_enabled && !self.paused && self.period().is_some() =>
            {
                Some(self.sample_at)
            }
            _ => None,
        };
        let repeat = self.axes.iter().filter_map(|a| a.next).min();
        [sampling, repeat].into_iter().flatten().min()
    }

    /// Advances the time-of-day clock to `now` (`cycles_per_second` CPU cycles per second).
    pub fn tick_clock(&mut self, now: u64, cycles_per_second: u64) {
        self.cycles_per_second = cycles_per_second;
        self.advance(now);
    }

    /// Everything that happens by itself up to `now`: the clock, the monitoring samples, the key repeats. Every
    /// input calls it first, so a sample sees the state as it was at its own time.
    fn advance(&mut self, now: u64) {
        self.advance_clock(now);
        let Some(period) = self.period() else {
            self.advance_repeats(now);
            return;
        };
        if self.sample_at <= now {
            let n = (now - self.sample_at) / period + 1;
            let skip = n.saturating_sub(MAX_CATCH_UP);
            self.sample_at += skip * period;
            if self.paused || !self.joy_enabled {
                // §9.14: while paused the monitoring modes do not queue samples.
                self.sample_at += (n - skip) * period;
            } else {
                while self.sample_at <= now {
                    let at = self.sample_at;
                    self.sample(at);
                    self.sample_at += period;
                }
            }
        }
        self.advance_repeats(now);
    }

    /// The cycles between samples in the monitoring modes.
    fn period(&self) -> Option<u64> {
        let cps = self.cycles_per_second;
        match self.joy_mode {
            // §9.18: hundredths of a second. [H?] a rate of 0 samples as fast as 1.
            JoyMode::Monitoring { rate } if cps != 0 => {
                Some((cps / 100 * rate.max(1) as u64).max(1))
            }
            // §9.19: eight samples in the time a character takes.
            JoyMode::FireButton => Some(BYTE_CYCLES / 8),
            _ => None,
        }
    }

    /// One sample of a monitoring mode at `at`.
    fn sample(&mut self, at: u64) {
        match self.joy_mode {
            JoyMode::Monitoring { .. } => {
                // §9.18: `%000000ab` fire buttons of joystick 1 (a) and 0 (b), then `%nnnnmmmm` the sticks.
                let fire = (self.joy[1] >> 7) << 1 | (self.joy[0] >> 7);
                let sticks = (self.joy[1] & 0x0F) << 4 | (self.joy[0] & 0x0F);
                self.push(at, &[fire, sticks]);
            }
            JoyMode::FireButton => {
                // §9.19: joystick 1's fire button, the first sample the most significant bit.
                self.fire_bits = self.fire_bits << 1 | (self.joy[1] >> 7);
                self.fire_count += 1;
                if self.fire_count == 8 {
                    let byte = self.fire_bits;
                    self.fire_bits = 0;
                    self.fire_count = 0;
                    self.push(at, &[byte]);
                }
            }
            _ => {}
        }
    }

    fn advance_clock(&mut self, now: u64) {
        let cps = self.cycles_per_second;
        if cps == 0 || now < self.clock_at + cps {
            return;
        }
        let seconds = (now - self.clock_at) / cps;
        self.clock_at += seconds * cps;
        self.add_seconds(seconds);
    }

    /// `n` seconds on the packed-BCD clock, with the calendar's carries ([H?] two-digit years, leap when divisible
    /// by 4). The calendar repeats every 36 525 days (100 years of this rule), so the work is bounded however long
    /// the host did not look.
    fn add_seconds(&mut self, n: u64) {
        let bcd = |v: u8| (v >> 4) as u64 * 10 + (v & 0x0F) as u64;
        let to = |v: u64| ((v / 10) << 4 | (v % 10)) as u8;
        let of_day = bcd(self.clock[3]) * 3600 + bcd(self.clock[4]) * 60 + bcd(self.clock[5]);
        let total = of_day + n % 86_400;
        let days = (n / 86_400 + total / 86_400) % 36_525;
        let of_day = total % 86_400;
        self.clock[3] = to(of_day / 3600);
        self.clock[4] = to(of_day % 3600 / 60);
        self.clock[5] = to(of_day % 60);
        for _ in 0..days {
            self.add_day();
        }
    }

    fn add_day(&mut self) {
        let bcd = |v: u8| (v >> 4) * 10 + (v & 0x0F);
        let to = |v: u8| ((v / 10) << 4) | (v % 10);
        let (mut year, mut month, mut day) =
            (bcd(self.clock[0]), bcd(self.clock[1]), bcd(self.clock[2]));
        day += 1;
        let month_len = match month {
            2 if year.is_multiple_of(4) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        if day > month_len {
            day = 1;
            month += 1;
        }
        if month > 12 {
            month = 1;
            year = (year + 1) % 100;
        }
        self.clock[0] = to(year);
        self.clock[1] = to(month);
        self.clock[2] = to(day);
    }

    /// A byte from the ST, received at `at`.
    pub fn command_byte(&mut self, byte: u8, at: u64) {
        self.advance(at);
        self.cmd.push(byte);
        let first = self.cmd[0];
        if first == 0x80 {
            if self.cmd.len() == 2 {
                let second = self.cmd[1];
                self.cmd.clear();
                if second == 0x01 {
                    self.reset(at);
                }
            }
            return;
        }
        let needed = match first {
            0x07 | 0x17 => 2,
            0x0A..=0x0C | 0x21 | 0x22 => 3,
            0x09 => 5,
            0x0E => 6,
            0x19 | 0x1B => 7,
            // §9.24: address, count, then that many data bytes.
            0x20 => {
                if self.cmd.len() < 4 {
                    4
                } else {
                    4 + self.cmd[3] as usize
                }
            }
            _ => 1,
        };
        if self.cmd.len() < needed {
            return;
        }
        let cmd = std::mem::take(&mut self.cmd);
        // §9.12: any command received while paused resumes output; §9.14: motion piled up meanwhile goes out as
        // the fewest packets.
        if std::mem::take(&mut self.paused) {
            self.flush_motion(at);
        }
        self.execute(&cmd, at);
    }

    fn execute(&mut self, cmd: &[u8], at: u64) {
        match cmd[0] {
            0x07 => self.button_action = cmd[1] & 0x07,
            0x08 => {
                self.mouse = MouseMode::Relative;
                self.mouse_command();
            }
            0x09 => {
                self.mouse = MouseMode::Absolute {
                    x_max: u16::from_be_bytes([cmd[1], cmd[2]]),
                    y_max: u16::from_be_bytes([cmd[3], cmd[4]]),
                };
                self.abs = (0, 0);
                self.abs_sub = (0, 0);
                self.mouse_command();
            }
            0x0A => {
                // [H?] a distance of 0 clicks is read as 1.
                self.mouse = MouseMode::Keycode {
                    dx: cmd[1].max(1),
                    dy: cmd[2].max(1),
                };
                self.mouse_command();
            }
            // §9.6: "valid only in RELATIVE MOUSE POSITIONING mode"; [H?] a threshold of 0 is read as 1.
            0x0B if self.mouse == MouseMode::Relative => {
                self.threshold = (cmd[1].max(1), cmd[2].max(1));
            }
            0x0B => {}
            // §9.7; [H?] a scale of 0 is read as 1.
            0x0C => self.scale = (cmd[1].max(1), cmd[2].max(1)),
            0x0D => {
                let [xh, xl] = (self.abs.0 as u16).to_be_bytes();
                let [yh, yl] = (self.abs.1 as u16).to_be_bytes();
                let b = std::mem::take(&mut self.abs_buttons);
                self.push(at + self.query_delay, &[0xF7, b, xh, xl, yh, yl]);
            }
            0x0E => {
                self.abs = (
                    u16::from_be_bytes([cmd[2], cmd[3]]) as i32,
                    u16::from_be_bytes([cmd[4], cmd[5]]) as i32,
                );
                self.abs_sub = (0, 0);
            }
            0x0F => self.y_at_bottom = true,
            0x10 => self.y_at_bottom = false,
            0x11 => {}
            0x12 => self.mouse_enabled = false,
            0x13 => self.paused = true,
            0x14 => self.joystick_command(JoyMode::Event, at),
            0x15 => self.joystick_command(JoyMode::Interrogation, at),
            0x16 => {
                let j0 = if self.port0_joystick { self.joy[0] } else { 0 };
                self.push(at + self.query_delay, &[0xFD, j0, self.joy[1]]);
            }
            0x17 => self.joystick_command(JoyMode::Monitoring { rate: cmd[1] }, at),
            0x18 => self.joystick_command(JoyMode::FireButton, at),
            0x19 => self.joystick_command(
                JoyMode::Keycode(KeyTimes {
                    rx: cmd[1],
                    ry: cmd[2],
                    tx: cmd[3],
                    ty: cmd[4],
                    vx: cmd[5],
                    vy: cmd[6],
                }),
                at,
            ),
            0x1A => {
                self.joy_enabled = false;
                self.axes = [Axis::default(); 2];
            }
            0x1B => {
                for (field, &v) in self.clock.iter_mut().zip(&cmd[1..7]) {
                    // §9.22: a field with a digit that is not BCD is left alone.
                    if v >> 4 <= 9 && v & 0x0F <= 9 {
                        *field = v;
                    }
                }
                // [H?] setting the time restarts the second: the next one is a whole second after the command.
                self.clock_at = at;
            }
            0x1C => {
                let c = self.clock;
                self.push(
                    at + self.query_delay,
                    &[0xFC, c[0], c[1], c[2], c[3], c[4], c[5]],
                );
            }
            0x20 => self.memory_load(cmd),
            // §9.25: `$F6 $20` and six bytes from the address. [H?] only what a load put there is known; the rest
            // reads as zero.
            0x21 => {
                let addr = u16::from_be_bytes([cmd[1], cmd[2]]);
                let mut p = [0u8; 8];
                p[0] = 0xF6;
                p[1] = 0x20;
                for (i, b) in p[2..].iter_mut().enumerate() {
                    *b = self
                        .memory
                        .get(&addr.wrapping_add(i as u16))
                        .copied()
                        .unwrap_or(0);
                }
                self.push(at + self.query_delay, &p);
            }
            // [H?] §9.26: the controller's firmware is not emulated, so nothing runs; the address is kept for tests
            // and diagnostics.
            0x22 => {
                if self.executed.len() == 16 {
                    self.executed.remove(0);
                }
                self.executed.push(u16::from_be_bytes([cmd[1], cmd[2]]));
            }
            0x87..=0x9A => self.status(cmd[0], at),
            _ => {}
        }
    }

    /// §9.24: `$20`, address, count (0-128), data. [H?] a count over 128 loads its first 128 bytes.
    fn memory_load(&mut self, cmd: &[u8]) {
        let addr = u16::from_be_bytes([cmd[1], cmd[2]]);
        let n = (cmd[3] as usize).min(128);
        for (i, &b) in cmd[4..4 + n].iter().enumerate() {
            let a = addr.wrapping_add(i as u16);
            if self.memory.len() < MEMORY_MODEL_BYTES || self.memory.contains_key(&a) {
                self.memory.insert(a, b);
            }
        }
    }

    /// The addresses of the controller-execute commands received, oldest first (the code is not run).
    pub fn executed(&self) -> &[u16] {
        &self.executed
    }

    fn mouse_command(&mut self) {
        self.mouse_enabled = true;
        self.port0_joystick = false;
        self.acc = (0, 0);
    }

    fn joystick_command(&mut self, mode: JoyMode, at: u64) {
        self.joy_mode = mode;
        self.joy_enabled = true;
        self.port0_joystick = true;
        self.axes = [Axis::default(); 2];
        self.fire_bits = 0;
        self.fire_count = 0;
        if let Some(period) = self.period() {
            self.sample_at = at + period;
        }
    }

    /// §9.27: `$F6`, the answer (laid out like the command that sets it, so the host can send it back), padded to
    /// 8 bytes. "Not valid if the ikbd is in JOYSTICK MONITORING mode or FIRE BUTTON MONITORING mode": ignored.
    fn status(&mut self, code: u8, at: u64) {
        if matches!(
            self.joy_mode,
            JoyMode::Monitoring { .. } | JoyMode::FireButton
        ) {
            return;
        }
        let mut r = [0u8; 7];
        match code {
            0x87 => r[..2].copy_from_slice(&[0x07, self.button_action]),
            0x88..=0x8A => match self.mouse {
                MouseMode::Relative => r[0] = 0x08,
                MouseMode::Absolute { x_max, y_max } => {
                    r[0] = 0x09;
                    r[1..3].copy_from_slice(&x_max.to_be_bytes());
                    r[3..5].copy_from_slice(&y_max.to_be_bytes());
                }
                MouseMode::Keycode { dx, dy } => r[..3].copy_from_slice(&[0x0A, dx, dy]),
            },
            0x8B => r[..3].copy_from_slice(&[0x0B, self.threshold.0, self.threshold.1]),
            0x8C => r[..3].copy_from_slice(&[0x0C, self.scale.0, self.scale.1]),
            0x8F | 0x90 => r[0] = if self.y_at_bottom { 0x0F } else { 0x10 },
            0x92 => r[0] = if self.mouse_enabled { 0x00 } else { 0x12 },
            0x94 | 0x95 | 0x99 => match self.joy_mode {
                JoyMode::Event => r[0] = 0x14,
                JoyMode::Interrogation => r[0] = 0x15,
                JoyMode::Keycode(t) => {
                    r[..7].copy_from_slice(&[0x19, t.rx, t.ry, t.tx, t.ty, t.vx, t.vy])
                }
                // Unreachable: the monitoring modes returned above.
                JoyMode::Monitoring { .. } | JoyMode::FireButton => return,
            },
            0x9A => r[0] = if self.joy_enabled { 0x00 } else { 0x1A },
            _ => return,
        }
        let mut packet = vec![0xF6];
        packet.extend_from_slice(&r);
        self.push(at + self.status_delay, &packet);
    }

    /// Whether a monitoring mode has the controller to itself (§9.18, §9.19: "nothing but monitor the serial command
    /// line, maintain the clock and monitor the joystick").
    fn monitoring(&self) -> bool {
        matches!(
            self.joy_mode,
            JoyMode::Monitoring { .. } | JoyMode::FireButton
        ) && self.joy_enabled
    }

    /// A key went down or up (`scan` is its make code).
    pub fn key(&mut self, scan: u8, down: bool, at: u64) {
        self.advance(at);
        if self.monitoring() {
            return;
        }
        self.push(at, &[if down { scan } else { scan | 0x80 }]);
    }

    fn buttons(&self) -> u8 {
        (self.left as u8) << 1 | self.right as u8
    }

    /// Mouse motion in mouse clicks (X right, Y toward the user positive), at `at`.
    pub fn mouse_move(&mut self, dx: i32, dy: i32, at: u64) {
        self.advance(at);
        if !self.mouse_enabled || self.port0_joystick {
            return;
        }
        match self.mouse {
            MouseMode::Relative => {
                let dy = if self.y_at_bottom { -dy } else { dy };
                self.acc.0 += dx;
                self.acc.1 += dy;
                // §9.14: while paused the motion piles up and goes out when output resumes.
                if !self.paused
                    && (self.acc.0.unsigned_abs() >= self.threshold.0 as u32
                        || self.acc.1.unsigned_abs() >= self.threshold.1 as u32)
                {
                    self.relative_report(at);
                }
            }
            MouseMode::Absolute { x_max, y_max } => {
                let dy = if self.y_at_bottom { -dy } else { dy };
                let step = |pos: &mut i32, sub: &mut i32, d: i32, scale: u8, max: u16| {
                    *sub += d;
                    let units = *sub / scale as i32;
                    *sub -= units * scale as i32;
                    // §9.4: no wrap; motion below 0 or above the maximum is ignored.
                    *pos = (*pos + units).clamp(0, max as i32);
                };
                step(
                    &mut self.abs.0,
                    &mut self.abs_sub.0,
                    dx,
                    self.scale.0,
                    x_max,
                );
                step(
                    &mut self.abs.1,
                    &mut self.abs_sub.1,
                    dy,
                    self.scale.1,
                    y_max,
                );
            }
            // §9.5: "this command is not affected by the mouse motion origin".
            MouseMode::Keycode { dx: kx, dy: ky } => {
                self.acc.0 += dx;
                self.acc.1 += dy;
                while self.acc.0.unsigned_abs() >= kx as u32 {
                    let (key, step) = if self.acc.0 > 0 {
                        (KEY_RIGHT, kx as i32)
                    } else {
                        (KEY_LEFT, -(kx as i32))
                    };
                    self.push(at, &[key, key | 0x80]);
                    self.acc.0 -= step;
                }
                while self.acc.1.unsigned_abs() >= ky as u32 {
                    let (key, step) = if self.acc.1 > 0 {
                        (KEY_DOWN, ky as i32)
                    } else {
                        (KEY_UP, -(ky as i32))
                    };
                    self.push(at, &[key, key | 0x80]);
                    self.acc.1 -= step;
                }
            }
        }
    }

    /// §4.1: the accumulated motion as relative records, split into the -128..127 range.
    fn relative_report(&mut self, at: u64) {
        loop {
            let x = self.acc.0.clamp(-128, 127);
            let y = self.acc.1.clamp(-128, 127);
            self.push(at, &[0xF8 | self.buttons(), x as u8, y as u8]);
            self.acc.0 -= x;
            self.acc.1 -= y;
            if self.acc == (0, 0) {
                break;
            }
        }
    }

    /// Motion held back while output was paused goes out now.
    fn flush_motion(&mut self, at: u64) {
        if self.mouse == MouseMode::Relative
            && self.mouse_enabled
            && !self.port0_joystick
            && self.acc != (0, 0)
        {
            self.relative_report(at);
        }
    }

    /// A mouse button changed (`left` false = right button).
    pub fn mouse_button(&mut self, left: bool, down: bool, at: u64) {
        self.advance(at);
        if left {
            self.left = down;
        } else {
            self.right = down;
        }
        if self.port0_joystick {
            // §8: both ports are joysticks; [H?] the right button is joystick 1's fire, the left joystick 0's.
            let port = if left { 0 } else { 1 };
            let bits = (self.joy[port] & 0x0F) | if down { 0x80 } else { 0 };
            self.joystick(port as u8, bits, at);
            return;
        }
        if !self.mouse_enabled {
            return;
        }
        // §9.2: "100 = mouse buttons act like keys"; key code mode presumes it.
        if self.button_action & 0x04 != 0 || matches!(self.mouse, MouseMode::Keycode { .. }) {
            let code = if left {
                LEFT_BUTTON_KEY
            } else {
                RIGHT_BUTTON_KEY
            };
            self.key(code, down, at);
            return;
        }
        match self.mouse {
            // §9.14: a button press or release sends the motion piled up so far at once.
            MouseMode::Relative => self.relative_report(at),
            MouseMode::Absolute { .. } => {
                // §9.8: down/up since the last interrogation; §9.2 bits 0/1 ask for a report on press/release.
                let bit = match (left, down) {
                    (false, true) => 0x01,
                    (false, false) => 0x02,
                    (true, true) => 0x04,
                    (true, false) => 0x08,
                };
                self.abs_buttons |= bit;
                if (down && self.button_action & 0x01 != 0)
                    || (!down && self.button_action & 0x02 != 0)
                {
                    self.execute(&[0x0D], at);
                }
            }
            MouseMode::Keycode { .. } => {}
        }
    }

    /// Joystick `port` (0 or 1) now reads `bits`: bit 7 the trigger, bits 3-0 the stick ([H?] bit 0 up, 1 down,
    /// 2 left, 3 right; the document says only "stick position").
    pub fn joystick(&mut self, port: u8, bits: u8, at: u64) {
        self.advance(at);
        let port = (port & 1) as usize;
        let bits = bits & 0x8F;
        let old = self.joy[port];
        if old == bits {
            return;
        }
        self.joy[port] = bits;
        if !self.joy_enabled || (port == 0 && !self.port0_joystick) {
            return;
        }
        match self.joy_mode {
            JoyMode::Event => self.push(at, &[0xFE | port as u8, bits]),
            JoyMode::Keycode(times) => self.joystick_keys(times, port, old, bits, at),
            JoyMode::Interrogation | JoyMode::Monitoring { .. } | JoyMode::FireButton => {}
        }
    }

    /// §5.5: the fire buttons are the pseudo keys `$74` (joystick 0) and `$75` (joystick 1); [H?] only joystick 0's
    /// stick makes cursor keys (§9.20: "joystick 0 is scanned in a way that simulates cursor keystrokes", where §5.5
    /// says "either joystick").
    fn joystick_keys(&mut self, t: KeyTimes, port: usize, old: u8, bits: u8, at: u64) {
        if (old ^ bits) & 0x80 != 0 {
            let code = if port == 0 {
                LEFT_BUTTON_KEY
            } else {
                RIGHT_BUTTON_KEY
            };
            self.push(at, &[if bits & 0x80 != 0 { code } else { code | 0x80 }]);
        }
        if port != 0 {
            return;
        }
        let dir = |minus: u8, plus: u8| -> i8 {
            match (bits & minus != 0, bits & plus != 0) {
                (true, false) => -1,
                (false, true) => 1,
                _ => 0,
            }
        };
        // Up is bit 0, down bit 1, left bit 2, right bit 3.
        let new = [dir(0x04, 0x08), dir(0x01, 0x02)];
        for (axis, dir) in new.into_iter().enumerate() {
            if dir == self.axes[axis].dir {
                continue;
            }
            self.axes[axis] = Axis {
                dir,
                since: at,
                next: None,
            };
            if dir != 0 {
                self.push_cursor(axis, dir, at);
                self.axes[axis].next = self.repeat_after(t, axis, at);
            }
        }
    }

    fn push_cursor(&mut self, axis: usize, dir: i8, at: u64) {
        let key = match (axis, dir < 0) {
            (0, true) => KEY_LEFT,
            (0, false) => KEY_RIGHT,
            (_, true) => KEY_UP,
            (_, false) => KEY_DOWN,
        };
        self.push(at, &[key, key | 0x80]);
    }

    /// §9.20: pairs every T tenths until R tenths have passed since the closure, then every V tenths; with R = 0 the
    /// velocity part is the only one. [H?] a repeat of 0 tenths does not repeat.
    fn repeat_after(&self, t: KeyTimes, axis: usize, from: u64) -> Option<u64> {
        let (r, tn, v) = if axis == 0 {
            (t.rx, t.tx, t.vx)
        } else {
            (t.ry, t.ty, t.vy)
        };
        let tenth = self.cycles_per_second / 10;
        let since = self.axes[axis].since;
        let breakpoint = since + r as u64 * tenth;
        let gap = if r == 0 || from + tn as u64 * tenth >= breakpoint {
            v as u64 * tenth
        } else {
            tn as u64 * tenth
        };
        (gap != 0).then_some(from + gap)
    }

    /// The cursor key repeats due by `now`, in time order.
    fn advance_repeats(&mut self, now: u64) {
        let JoyMode::Keycode(times) = self.joy_mode else {
            return;
        };
        for _ in 0..MAX_CATCH_UP {
            let Some((axis, at)) = (0..2)
                .filter_map(|a| self.axes[a].next.map(|t| (a, t)))
                .filter(|&(_, t)| t <= now)
                .min_by_key(|&(_, t)| t)
            else {
                return;
            };
            let dir = self.axes[axis].dir;
            self.push_cursor(axis, dir, at);
            self.axes[axis].next = self.repeat_after(times, axis, at);
        }
        // A host that did not run the controller for ages: drop the backlog rather than loop.
        for a in &mut self.axes {
            if a.next.is_some_and(|t| t <= now) {
                a.next = Some(now + 1);
            }
        }
    }
}
