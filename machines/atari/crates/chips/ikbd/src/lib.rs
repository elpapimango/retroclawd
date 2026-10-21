//! The Atari ST's intelligent keyboard controller (an HD6301 with Atari's firmware), modelled by its behaviour as
//! Atari's *Intelligent Keyboard (ikbd) Protocol* (revised 26 February 1985) describes it: commands in, key codes,
//! mouse, joystick and clock reports out. It is not a 6301 core. Section numbers below are the document's.
//!
//! Facts not in the document are labelled as in `docs/hardware-facts.md`: [O] observed in Hatari 2.6.1 (black box),
//! [H?] a documented choice.
//!
//! Time is the host's CPU cycle count; the crate holds no clock of its own beyond the time-of-day counter, which the
//! host advances with [`Ikbd::tick_clock`].
#![forbid(unsafe_code)]

mod scancodes;

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

pub use scancodes::{SCANCODES, scancode};

/// [O] CPU cycles from receiving RESET's second byte (or power-up) to the version code being ready: Hatari 2.6.1
/// sends it 502 002 cycles (62.6 ms at 8 MHz) later; §9.1 allows up to 300 ms.
pub const RESET_REPLY_DELAY: u64 = 502_002;
/// [O] the version code: §8 gives `$F0` for the first release and `$F1` for a second; Hatari's ST answers `$F1`.
pub const VERSION: u8 = 0xF1;
/// [H?] bytes the controller can hold for the host. The 6301 has 128 bytes of RAM (*Internals* p.69); the document
/// leaves the queue size `<tbd>` (§9.14). A packet that does not fit whole is dropped; older bytes are kept.
pub const QUEUE_BYTES: usize = 256;
/// §4.3 and §5.5: the pseudo key codes of the mouse buttons (and joystick fire buttons) in key modes.
const LEFT_BUTTON_KEY: u8 = 0x74;
const RIGHT_BUTTON_KEY: u8 = 0x75;
/// §4.3: cursor key codes (Appendix A).
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum MouseMode {
    /// §9.3.
    Relative,
    /// §9.4: maxima in scaled units.
    Absolute { x_max: u16, y_max: u16 },
    /// §9.5: clicks per cursor key.
    Keycode { dx: u8, dy: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum JoyMode {
    /// §9.15 (`$14`).
    Event,
    /// §9.16 (`$15`).
    Interrogation,
    /// [H?] `$17`, `$18`, `$19`: monitoring and key code modes are Phase 2; reports stop.
    Unsupported(u8),
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
    /// yy MM DD hh mm ss, packed BCD; the cycle of the last whole second.
    clock: [u8; 6],
    clock_at: u64,
    cycles_per_second: u64,
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
            clock: [0; 6],
            clock_at: 0,
            cycles_per_second: 0,
        };
        k.push(RESET_REPLY_DELAY, &[VERSION]);
        k
    }

    /// §9.1: back to the power-up modes; the clock is kept.
    fn reset(&mut self, at: u64) {
        let (clock, clock_at, cps) = (self.clock, self.clock_at, self.cycles_per_second);
        *self = Self::new();
        self.queue.clear();
        self.clock = clock;
        self.clock_at = clock_at;
        self.cycles_per_second = cps;
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

    /// When the next byte will be ready, if any (`None` while paused between packets).
    pub fn next_output_at(&self) -> Option<u64> {
        let front = self.queue.front()?;
        if self.paused && self.sent == 0 {
            return None;
        }
        Some(front.at)
    }

    /// Advances the time-of-day clock to `now` (`cycles_per_second` CPU cycles per second).
    pub fn tick_clock(&mut self, now: u64, cycles_per_second: u64) {
        self.cycles_per_second = cycles_per_second;
        self.advance_clock(now);
    }

    fn advance_clock(&mut self, now: u64) {
        let cps = self.cycles_per_second;
        if cps == 0 {
            return;
        }
        while now >= self.clock_at + cps {
            self.clock_at += cps;
            self.add_second();
        }
    }

    /// One second on the packed-BCD clock, with the calendar's carries ([H?] two-digit years, leap when divisible
    /// by 4).
    fn add_second(&mut self) {
        let bcd = |v: u8| (v >> 4) * 10 + (v & 0x0F);
        let to = |v: u8| ((v / 10) << 4) | (v % 10);
        let mut f: Vec<u8> = self.clock.iter().map(|&v| bcd(v)).collect();
        f[5] += 1;
        if f[5] == 60 {
            f[5] = 0;
            f[4] += 1;
        }
        if f[4] == 60 {
            f[4] = 0;
            f[3] += 1;
        }
        if f[3] == 24 {
            f[3] = 0;
            f[2] += 1;
        }
        let month_len = match f[1] {
            2 if f[0].is_multiple_of(4) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        if f[2] > month_len {
            f[2] = 1;
            f[1] += 1;
        }
        if f[1] > 12 {
            f[1] = 1;
            f[0] = (f[0] + 1) % 100;
        }
        for (dst, v) in self.clock.iter_mut().zip(f) {
            *dst = to(v);
        }
    }

    /// A byte from the ST, received at `at`.
    pub fn command_byte(&mut self, byte: u8, at: u64) {
        self.advance_clock(at);
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
        // §9.12: any command received while paused resumes output.
        self.paused = false;
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
                self.mouse = MouseMode::Keycode {
                    dx: cmd[1].max(1),
                    dy: cmd[2].max(1),
                };
                self.mouse_command();
            }
            0x0B => self.threshold = (cmd[1].max(1), cmd[2].max(1)),
            0x0C => self.scale = (cmd[1].max(1), cmd[2].max(1)),
            0x0D => {
                let [xh, xl] = (self.abs.0 as u16).to_be_bytes();
                let [yh, yl] = (self.abs.1 as u16).to_be_bytes();
                let b = std::mem::take(&mut self.abs_buttons);
                self.push(at, &[0xF7, b, xh, xl, yh, yl]);
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
            0x14 => self.joystick_command(JoyMode::Event),
            0x15 => self.joystick_command(JoyMode::Interrogation),
            0x16 => {
                let j0 = if self.port0_joystick { self.joy[0] } else { 0 };
                self.push(at, &[0xFD, j0, self.joy[1]]);
            }
            0x17..=0x19 => self.joystick_command(JoyMode::Unsupported(cmd[0])),
            0x1A => self.joy_enabled = false,
            0x1B => {
                for (field, &v) in self.clock.iter_mut().zip(&cmd[1..7]) {
                    // §9.22: a field with a digit that is not BCD is left alone.
                    if v >> 4 <= 9 && v & 0x0F <= 9 {
                        *field = v;
                    }
                }
            }
            0x1C => {
                let c = self.clock;
                self.push(at, &[0xFC, c[0], c[1], c[2], c[3], c[4], c[5]]);
            }
            // [H?] §9.24-9.26 address the 6301's own memory, which is not modelled: loads are consumed and dropped,
            // reads answer zeros, execute does nothing.
            0x20 | 0x22 => {}
            0x21 => self.push(at, &[0xF6, 0x20, 0, 0, 0, 0, 0, 0]),
            0x87..=0x9A => self.status(cmd[0], at),
            _ => {}
        }
    }

    fn mouse_command(&mut self) {
        self.mouse_enabled = true;
        self.port0_joystick = false;
        self.acc = (0, 0);
    }

    fn joystick_command(&mut self, mode: JoyMode) {
        self.joy_mode = mode;
        self.joy_enabled = true;
        self.port0_joystick = true;
    }

    /// §9.27: `$F6`, the answer, padded to 8 bytes.
    fn status(&mut self, code: u8, at: u64) {
        let mut r = [0u8; 7];
        match code {
            0x87 => r[0] = self.button_action,
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
            0x94 | 0x95 | 0x99 => {
                r[0] = match self.joy_mode {
                    JoyMode::Event => 0x14,
                    JoyMode::Interrogation => 0x15,
                    JoyMode::Unsupported(c) => c,
                }
            }
            0x9A => r[0] = if self.joy_enabled { 0x00 } else { 0x1A },
            _ => return,
        }
        let mut packet = vec![0xF6];
        packet.extend_from_slice(&r);
        self.push(at, &packet);
    }

    /// A key went down or up (`scan` is its make code).
    pub fn key(&mut self, scan: u8, down: bool, at: u64) {
        self.push(at, &[if down { scan } else { scan | 0x80 }]);
    }

    fn buttons(&self) -> u8 {
        (self.left as u8) << 1 | self.right as u8
    }

    /// Mouse motion in mouse clicks (X right, Y toward the user positive), at `at`.
    pub fn mouse_move(&mut self, dx: i32, dy: i32, at: u64) {
        if !self.mouse_enabled || self.port0_joystick {
            return;
        }
        let dy = if self.y_at_bottom { -dy } else { dy };
        match self.mouse {
            MouseMode::Relative => {
                self.acc.0 += dx;
                self.acc.1 += dy;
                if self.acc.0.unsigned_abs() >= self.threshold.0 as u32
                    || self.acc.1.unsigned_abs() >= self.threshold.1 as u32
                {
                    self.relative_report(at);
                }
            }
            MouseMode::Absolute { x_max, y_max } => {
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

    /// A mouse button changed (`left` false = right button).
    pub fn mouse_button(&mut self, left: bool, down: bool, at: u64) {
        if left {
            self.left = down;
        } else {
            self.right = down;
        }
        if !self.mouse_enabled && !self.port0_joystick {
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
        let port = (port & 1) as usize;
        let bits = bits & 0x8F;
        if self.joy[port] == bits {
            return;
        }
        self.joy[port] = bits;
        if !self.joy_enabled
            || self.joy_mode != JoyMode::Event
            || (port == 0 && !self.port0_joystick)
        {
            return;
        }
        self.push(at, &[0xFE | port as u8, bits]);
    }
}
