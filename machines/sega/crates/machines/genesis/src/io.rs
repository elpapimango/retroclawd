//! The I/O chip at `$A10000`: the version register, the three ports (two controller ports and the expansion port) with their
//! data, control and serial registers, and the pads plugged into the controller ports.
//!
//! Facts: I1-I6 in `docs/hardware-facts.md`. The sequence of a 6-button pad is from the Plutiedev "Controllers" page; the
//! power-on values and the way a word or an even byte reads are from probes P01 and P04 (Genesis Plus GX, PicoDrive and
//! BlastEm agree); the 6-button pad's timeout is the 26 lines Genesis Plus GX uses (fact I5, to be probed).

use crate::config::Region;
use serde::{Deserialize, Serialize};

/// Master clocks after a TH rising edge at which a 6-button pad forgets its position in the sequence.
const PAD_TIMEOUT: u64 = 26 * 3420;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PadKind {
    None,
    Three,
    Six,
    /// The Sega Mouse: ten nibbles per packet, fetched with a TR handshake (fact I7).
    Mouse,
    /// Electronic Arts' 4 Way Play: port 1 reads four pads, port 2 only selects (fact I8). Only valid for port 1.
    FourWay,
    /// Sega's Menacer light gun in port 2: three buttons and a start button, and a position that makes the VDP latch its
    /// counters (fact I9).
    Menacer,
}

impl PadKind {
    pub fn parse(text: &str) -> Option<PadKind> {
        match text {
            "none" => Some(PadKind::None),
            "three" | "3" => Some(PadKind::Three),
            "six" | "6" => Some(PadKind::Six),
            "mouse" => Some(PadKind::Mouse),
            "menacer" => Some(PadKind::Menacer),
            "4way" | "fourway" => Some(PadKind::FourWay),
            _ => None,
        }
    }
}

/// The buttons of a pad, by the bit each has in `Pad::buttons`.
pub const BUTTONS: [(&str, u16); 12] = [
    ("Up", 0x0001),
    ("Down", 0x0002),
    ("Left", 0x0004),
    ("Right", 0x0008),
    ("B", 0x0010),
    ("C", 0x0020),
    ("A", 0x0040),
    ("Start", 0x0080),
    ("Z", 0x0100),
    ("Y", 0x0200),
    ("X", 0x0400),
    ("Mode", 0x0800),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Port {
    /// The output latch (data register).
    data: u8,
    control: u8,
    tx: u8,
    rx: u8,
    serial_control: u8,
    pad: PadKind,
    buttons: u16,
    /// 6-button pad: TH rising edges seen, times two.
    counter: u8,
    th: bool,
    last_rise: u64,
    /// Mouse: the TH/TR levels the mouse last saw, the position in the packet, reads left of the handshake delay, the
    /// motion since the last packet and the motion of the packet being read.
    m_state: u8,
    m_counter: u8,
    m_wait: u8,
    m_acc: (i32, i32),
    m_packet: (i32, i32),
    /// Menacer: where it points, in screen pixels.
    gun: (i32, i32),
}

impl Port {
    fn new(pad: PadKind) -> Self {
        Self {
            data: 0,
            control: 0,
            tx: 0,
            rx: 0,
            serial_control: 0,
            pad,
            buttons: 0,
            counter: 0,
            th: true,
            last_rise: 0,
            m_state: 0x60,
            m_counter: 0,
            m_wait: 0,
            m_acc: (0, 0),
            m_packet: (0, 0),
            gun: (160, 112),
        }
    }

    /// The level on the TH pin: the data latch where the pin is an output, else pulled up.
    fn th_level(&self) -> bool {
        if self.control & 0x40 != 0 {
            self.data & 0x40 != 0
        } else {
            true
        }
    }

    /// A write to the data or control register of a port with a mouse: the mouse sees TH and TR where they are outputs.
    fn mouse_write(&mut self) {
        let new = (self.m_state & !self.control) | (self.data & self.control);
        let changed = self.m_state ^ new;
        if changed & 0x20 != 0 {
            // TR toggled: the next nibble, after a short delay before the mouse answers
            if self.m_counter > 0 && self.m_counter < 9 {
                self.m_counter += 1;
            }
            self.m_wait = 2;
        }
        if changed & 0x40 != 0 {
            // TH falling starts a packet (the motion so far goes into it), TH rising ends it; the motion is spent once a
            // whole packet has been read
            let finished = self.m_counter >= 9;
            self.m_counter = (self.m_state >> 6) & 1;
            if self.m_counter == 1 {
                self.m_packet = (self.m_acc.0.clamp(-255, 255), self.m_acc.1.clamp(-255, 255));
            } else if finished {
                self.m_acc = (0, 0);
            }
        }
        self.m_state = new;
    }

    /// The pins a mouse drives: the nibble of the packet on D3-D0 and TL (D4), which follows TR once the mouse has answered.
    fn mouse_pins(&mut self) -> u8 {
        let (x, y) = self.m_packet;
        let nibble = match self.m_counter {
            1 => 0x0B,
            2 | 3 => 0x0F,
            4 => u8::from(x < 0) | u8::from(y < 0) << 1,
            5 => ((self.buttons >> 4) & 0x0F) as u8,
            6 => ((x >> 4) & 0x0F) as u8,
            7 => (x & 0x0F) as u8,
            8 => ((y >> 4) & 0x0F) as u8,
            9 => (y & 0x0F) as u8,
            _ => 0,
        };
        let tl = if self.m_wait > 0 {
            self.m_wait -= 1;
            !self.m_state & 0x20
        } else {
            self.m_state & 0x20
        } >> 1;
        nibble | tl
    }

    /// Notes the pin's new level; a rising edge advances a 6-button pad.
    fn update_th(&mut self, now: u64) {
        if self.pad == PadKind::Mouse {
            self.mouse_write();
            return;
        }
        let th = self.th_level();
        if th && !self.th && self.control & 0x40 != 0 {
            if self.pad == PadKind::Six && self.counter < 8 {
                self.counter += 2;
            }
            self.last_rise = now;
        }
        self.th = th;
    }

    fn expire(&mut self, now: u64) {
        if self.counter != 0 && now.saturating_sub(self.last_rise) > PAD_TIMEOUT {
            self.counter = 0;
        }
    }

    /// The levels the pad drives on pins 5-0 for the current TH level (a pressed button pulls its pin low).
    fn pad_pins(&self) -> u8 {
        let b = self.buttons;
        let low = |mask: u16| -> u8 { u8::from(b & mask != 0) };
        let dpad = low(0x0001) | low(0x0002) << 1 | low(0x0004) << 2 | low(0x0008) << 3;
        let cb = low(0x0020) << 1 | low(0x0010);
        let sa = low(0x0080) << 1 | low(0x0040);
        let th = self.th_level();
        if self.pad == PadKind::None {
            return 0x3F;
        }
        let step = if self.pad == PadKind::Six {
            self.counter | u8::from(th)
        } else {
            u8::from(th)
        };
        let pulled_low: u8 = match step {
            // third low: D3-D0 forced low
            4 => sa << 4 | 0x0F,
            // fourth high: the extra buttons
            7 => cb << 4 | (low(0x0800) << 3 | low(0x0400) << 2 | low(0x0200) << 1 | low(0x0100)),
            // fourth low: D3-D0 forced high
            6 => sa << 4,
            // 3-button behaviour
            s if s & 1 != 0 => cb << 4 | dpad,
            _ => sa << 4 | 0x0C | (dpad & 0x03),
        };
        0x3F & !pulled_low
    }

    fn read_data(&mut self, now: u64) -> u8 {
        self.expire(now);
        if self.pad == PadKind::Menacer {
            // D0 B, D1 trigger (A), D2 C, D3 start, active high; TL and TR read low, TH high
            let b = (self.buttons >> 4) as u8;
            let pins = (b & 0x09) | ((b >> 1) & 0x02) | ((b << 1) & 0x04) | 0x40;
            return (self.data & self.control) | (pins & !self.control);
        }
        if self.pad == PadKind::Mouse {
            let pins = self.mouse_pins() | 0x60;
            return (self.data & self.control) | (pins & !self.control);
        }
        let pins = self.pad_pins() | if self.th_level() { 0x40 } else { 0 };
        (self.data & self.control) | (pins & !self.control)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Io {
    version: u8,
    /// The two controller ports, the expansion port, and the two pads of a J-Cart cartridge.
    ports: [Port; 6],
    /// 4 Way Play: the latched TH, TR and TL levels of port 2 (TR and TL pick the pad, TH high is the adapter's id).
    way_latch: u8,
    four_way: bool,
}

impl Io {
    pub fn new(region: Region, pad1: PadKind, pad2: PadKind, tmss: bool) -> Self {
        Self {
            version: if tmss {
                region.version_register_tmss()
            } else {
                region.version_register()
            },
            ports: [
                Port::new(if pad1 == PadKind::FourWay {
                    PadKind::Six
                } else {
                    pad1
                }),
                Port::new(pad2),
                Port::new(PadKind::None),
                Port::new(PadKind::Three),
                Port::new(PadKind::Three),
                Port::new(PadKind::Three),
            ],
            way_latch: 0,
            four_way: pad1 == PadKind::FourWay,
        }
    }

    /// The register at `$A10000 + 2 * index` (byte accessed at the odd address `2 * index + 1`).
    pub fn read(&mut self, index: usize, now: u64) -> u8 {
        match index & 0x0F {
            0 => self.version,
            1 if self.four_way => {
                if self.way_latch & 4 != 0 {
                    // the adapter's identification: D1 and D0 low
                    let p = &self.ports[0];
                    (p.data & p.control) | (0x7C & !p.control)
                } else {
                    let sel = self.way_pad(self.way_latch & 3);
                    self.ports[sel].read_data(now)
                }
            }
            2 if self.four_way => {
                let p = &self.ports[1];
                (p.data & p.control) | (0x7F & !p.control)
            }
            i @ 1..=3 => self.ports[i - 1].read_data(now),
            i @ 4..=6 => self.ports[i - 4].control,
            7 => self.ports[0].tx,
            8 => self.ports[0].rx,
            9 => self.ports[0].serial_control,
            10 => self.ports[1].tx,
            11 => self.ports[1].rx,
            12 => self.ports[1].serial_control,
            13 => self.ports[2].tx,
            14 => self.ports[2].rx,
            _ => self.ports[2].serial_control,
        }
    }

    pub fn write(&mut self, index: usize, value: u8, now: u64) {
        match index & 0x0F {
            0 => {}
            i @ 1..=3 => {
                let p = &mut self.ports[i - 1];
                p.expire(now);
                p.data = value;
                p.update_th(now);
                self.four_way_follow(i - 1, now);
            }
            i @ 4..=6 => {
                let p = &mut self.ports[i - 4];
                p.expire(now);
                p.control = value;
                p.update_th(now);
                self.four_way_follow(i - 4, now);
            }
            7 => self.ports[0].tx = value,
            8 => {}
            9 => self.ports[0].serial_control = value & 0xF8,
            10 => self.ports[1].tx = value,
            11 => {}
            12 => self.ports[1].serial_control = value & 0xF8,
            13 => self.ports[2].tx = value,
            14 => {}
            _ => self.ports[2].serial_control = value & 0xF8,
        }
    }

    /// The index in `ports` of pad `n` (0-3) of a 4 Way Play adapter: the first controller port's own pad, then the three
    /// spare ones.
    fn way_pad(&self, n: u8) -> usize {
        [0, 3, 4, 5][usize::from(n & 3)]
    }

    /// 4 Way Play: what port 1 is written goes to the selected pad; what port 2 is written latches the selection.
    fn four_way_follow(&mut self, port: usize, now: u64) {
        if !self.four_way {
            return;
        }
        match port {
            0 if self.way_latch & 4 == 0 => {
                let sel = self.way_pad(self.way_latch & 3);
                if sel != 0 {
                    let (data, control) = (self.ports[0].data, self.ports[0].control);
                    let p = &mut self.ports[sel];
                    p.data = data;
                    p.control = control;
                    p.update_th(now);
                }
            }
            1 => {
                let p = &self.ports[1];
                let d = p.data | !p.control;
                if d & 0x03 == 0 {
                    self.way_latch = (d >> 4) & 7;
                }
            }
            _ => {}
        }
    }

    /// What a J-Cart shows at `$380000-$3FFFFF`: pad 3's pins in the low byte, pad 4's (six pins) in the high byte, and
    /// D14 low (fact H12).
    pub fn jcart_read(&mut self, now: u64) -> u16 {
        let low = self.ports[3].read_data(now);
        let high = self.ports[4].read_data(now) & 0x3F;
        u16::from(high) << 8 | u16::from(low)
    }

    /// A write to a J-Cart's area: bit 0 drives TH of both its pads (fact H12).
    pub fn jcart_write(&mut self, value: u16, now: u64) {
        let th = u8::from(value & 1 != 0) << 6;
        for p in &mut self.ports[3..] {
            p.expire(now);
            p.control = 0x40;
            p.data = th;
            p.update_th(now);
        }
    }

    /// The mouse moves by (`dx`, `dy`) host pixels (right and down positive); the first port with a mouse takes it.
    pub fn mouse_motion(&mut self, dx: i32, dy: i32) {
        if let Some(p) = self.ports[..2]
            .iter_mut()
            .find(|p| p.pad == PadKind::Menacer)
        {
            p.gun = ((p.gun.0 + dx).clamp(0, 319), (p.gun.1 + dy).clamp(0, 239));
            return;
        }
        if let Some(p) = self.ports[..2].iter_mut().find(|p| p.pad == PadKind::Mouse) {
            p.m_acc.0 += dx;
            // the mouse counts up as positive
            p.m_acc.1 -= dy;
        }
    }

    /// A mouse button (0 left, 1 right, 2 middle).
    pub fn mouse_button(&mut self, button: usize, pressed: bool) {
        if let Some(port) = self.ports[..2].iter().position(|p| p.pad == PadKind::Mouse) {
            self.set_button(port, 0x10u16 << button.min(2), pressed);
        } else if let Some(port) = self.ports[..2]
            .iter()
            .position(|p| p.pad == PadKind::Menacer)
        {
            // the trigger is A, the others B and C
            let mask = [0x0040, 0x0010, 0x0020][button.min(2)];
            self.set_button(port, mask, pressed);
        }
    }

    /// Where a Menacer points and whether the port's HL pin (control register bit 7) lets it latch the VDP's counters. The
    /// raw position is stretched to the gun's own aim (games scale it up by 289/320).
    pub fn gun(&self, width: u32, height: u32) -> Option<((u32, u32), bool)> {
        let p = self.ports[..2].iter().find(|p| p.pad == PadKind::Menacer)?;
        let x = ((p.gun.0 as u32) * 289 / 320).min(width.saturating_sub(1));
        let y = (p.gun.1 as u32).min(height.saturating_sub(1));
        Some(((x, y), p.control & 0x80 != 0))
    }

    /// Whether a mouse is plugged in.
    pub fn has_mouse(&self) -> bool {
        self.ports[..2].iter().any(|p| p.pad == PadKind::Mouse)
    }

    /// A key of a pad: `port` 0 and 1 are the controller ports, 2 and 3 the pads of a J-Cart.
    pub fn set_button(&mut self, port: usize, mask: u16, pressed: bool) {
        let port = if self.four_way {
            self.way_pad(port as u8)
        } else if port >= 2 {
            port + 1
        } else {
            port
        };
        if let Some(p) = self.ports.get_mut(port) {
            if pressed {
                p.buttons |= mask;
            } else {
                p.buttons &= !mask;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io(pad: PadKind) -> Io {
        Io::new(Region::Usa, pad, pad, false)
    }

    /// Sets TH to `th` on port 1 (control $40 already set) and reads the data register.
    fn read_at(io: &mut Io, th: bool, now: u64) -> u8 {
        io.write(1, if th { 0x40 } else { 0x00 }, now);
        io.read(1, now)
    }

    /// Reads one packet from a mouse in port 1 the way a game does: TH low starts it, every TR toggle asks for the next nibble
    /// and TL follows TR when the nibble is ready.
    fn mouse_packet(io: &mut Io) -> Vec<u8> {
        io.write(4, 0x60, 0);
        io.write(1, 0x60, 0);
        io.write(1, 0x20, 0);
        let mut tr = 0x20u8;
        let mut out = Vec::new();
        for _ in 0..9 {
            let mut v = io.read(1, 0);
            for _ in 0..8 {
                if (v >> 4) & 1 == (tr >> 5) & 1 {
                    break;
                }
                v = io.read(1, 0);
            }
            out.push(v & 0x0F);
            tr ^= 0x20;
            io.write(1, tr, 0);
        }
        io.write(1, 0x60, 0);
        out
    }

    #[test]
    fn a_mouse_sends_its_motion_and_buttons_in_nine_nibbles() {
        let mut io = Io::new(Region::Usa, PadKind::Mouse, PadKind::Six, false);
        assert!(io.has_mouse());
        // what Genesis Plus GX returns for a move of 5 right and 3 up with the left button held
        io.mouse_motion(5, -3);
        io.mouse_button(0, true);
        assert_eq!(
            mouse_packet(&mut io),
            [0x0B, 0x0F, 0x0F, 0x00, 0x01, 0x00, 0x05, 0x00, 0x03]
        );
        // and for 20 left, 40 down with the right button: signs in the fourth nibble, two's complement bytes
        io.mouse_button(0, false);
        io.mouse_button(1, true);
        io.mouse_motion(-20, 40);
        assert_eq!(
            mouse_packet(&mut io),
            [0x0B, 0x0F, 0x0F, 0x03, 0x02, 0x0E, 0x0C, 0x0D, 0x08]
        );
        // the motion is spent: an idle mouse sends zeros
        let idle = mouse_packet(&mut io);
        assert_eq!(&idle[3..], [0, 2, 0, 0, 0, 0]);
    }

    #[test]
    fn four_way_play_selects_one_of_four_pads_through_port_2() {
        let mut io = Io::new(Region::Usa, PadKind::FourWay, PadKind::Six, false);
        io.write(4, 0x40, 0); // port 1: TH is an output
        io.write(5, 0x7F, 0); // port 2: every pin an output, the selector
        io.write(1, 0x40, 0);
        // TH high on port 2's data: the adapter answers with its identification on D1-D0
        io.write(2, 0x7C, 0);
        assert_eq!(io.read(1, 0) & 0x03, 0x00);
        // select pad 3 (TR high, TL low, UP and DOWN low), then give the selected pad its TH level
        io.set_button(2, 0x0004, true); // Joy3 Left
        io.write(2, 0x20, 0);
        io.write(1, 0x40, 0);
        assert_eq!(io.read(1, 0) & 0x0F, 0x0B, "pad 3 has Left pressed");
        io.write(2, 0x00, 0); // pad 1
        io.write(1, 0x40, 0);
        assert_eq!(io.read(1, 0) & 0x0F, 0x0F, "pad 1 is idle");
    }

    #[test]
    fn a_jcart_shows_two_more_pads_and_th_comes_from_bit_zero_of_a_write() {
        let mut io = io(PadKind::Three);
        io.jcart_write(1, 0);
        assert_eq!(io.jcart_read(0), 0x3F7F, "idle pads, TH high");
        io.jcart_write(0, 0);
        assert_eq!(
            io.jcart_read(0),
            0x3333,
            "TH low: left and right read low, start and A high"
        );
        io.set_button(2, 0x0001, true); // pad 3, Up
        io.set_button(3, 0x0020, true); // pad 4, C
        io.jcart_write(1, 0);
        assert_eq!(io.jcart_read(0), 0x1F7E);
    }

    #[test]
    fn power_on_values_follow_the_probe() {
        let mut io = io(PadKind::Three);
        assert_eq!(io.read(0, 0), 0xA0);
        assert_eq!(io.read(1, 0), 0x7F);
        assert_eq!(io.read(2, 0), 0x7F);
        assert_eq!(
            io.read(3, 0),
            0x7F,
            "the expansion port: nothing plugged in, pins pulled up"
        );
        for i in 4..7 {
            assert_eq!(io.read(i, 0), 0);
        }
        assert_eq!(io.read(8, 0), 0);
        assert_eq!(io.read(9, 0), 0);
    }

    #[test]
    fn output_pins_read_back_what_was_written() {
        let mut io = io(PadKind::Three);
        io.write(4, 0x7F, 0);
        io.write(1, 0x15, 0);
        assert_eq!(io.read(1, 0), 0x15);
        io.write(4, 0x40, 0);
        io.write(1, 0x00, 0);
        assert_eq!(
            io.read(1, 0),
            0x33,
            "TH low, no button pressed: A and Start high, D3-D2 low"
        );
        assert_eq!(io.read(4, 0), 0x40);
    }

    /// A pad as a game leaves it after its setup: control $40, data $40, and a pause that lets a 6-button pad forget.
    fn ready(pad: PadKind) -> (Io, u64) {
        let mut io = io(pad);
        io.write(4, 0x40, 0);
        io.write(1, 0x40, 0);
        (io, PAD_TIMEOUT + 1000)
    }

    #[test]
    fn a_three_button_pad_reports_its_buttons_for_each_th_level() {
        let (mut io, t) = ready(PadKind::Three);
        io.set_button(0, 0x0001 | 0x0010 | 0x0040 | 0x0080, true); // Up, B, A, Start
        // TH high: C B R L D U with Up and B low; TH pin high
        assert_eq!(read_at(&mut io, true, t), 0x6E);
        // TH low: Start A 0 0 D U with Start, A and Up low
        assert_eq!(read_at(&mut io, false, t), 0x02);
    }

    #[test]
    fn the_second_step_has_d3_and_d2_low_and_the_d_pad_in_d1_d0() {
        let (mut io, t) = ready(PadKind::Three);
        io.set_button(0, 0x0002, true); // Down
        let v = read_at(&mut io, false, t);
        assert_eq!(v & 0x0F, 0b0001);
        assert_eq!(v & 0x30, 0x30);
    }

    #[test]
    fn a_six_button_pad_runs_through_its_documented_sequence() {
        let (mut io, mut t) = ready(PadKind::Six);
        io.set_button(0, 0x0800 | 0x0400 | 0x0100 | 0x0020, true); // Mode, X, Z, C
        let mut seen = Vec::new();
        for th in [true, false, true, false, true, false, true, false] {
            seen.push(read_at(&mut io, th, t));
            t += 100;
        }
        // 1-5: the 3-button values (C low in the TH high ones; D-pad released)
        assert_eq!(seen[0], 0x5F);
        assert_eq!(seen[1], 0x33);
        assert_eq!(seen[2], 0x5F);
        assert_eq!(seen[4], 0x5F);
        // 6: Start A 0 0 0 0
        assert_eq!(seen[5], 0x30);
        // 7: C B Mode X Y Z: C, Mode, X and Z low, B and Y high
        assert_eq!(seen[6], 0x52);
        // 8: Start A 1 1 1 1
        assert_eq!(seen[7], 0x3F);
    }

    #[test]
    fn the_six_button_sequence_resets_after_the_timeout() {
        let (mut io, mut t) = ready(PadKind::Six);
        io.set_button(0, 0x0800, true); // Mode
        for th in [true, false, true, false, true, false] {
            read_at(&mut io, th, t);
            t += 100;
        }
        // after a pause the sequence starts again: the next TH high read is a plain 3-button read
        t += PAD_TIMEOUT + 10;
        let v = read_at(&mut io, true, t);
        assert_eq!(v & 0x0F, 0x0F, "Mode is not shown any more");
        // and the two steps after it are the ordinary ones too
        assert_eq!(
            read_at(&mut io, false, t + 100) & 0x0C,
            0,
            "D3-D2 low: a pad is plugged in"
        );
    }

    #[test]
    fn a_three_button_pad_never_shows_the_extra_buttons() {
        let (mut io, mut t) = ready(PadKind::Three);
        io.set_button(0, 0x0800, true);
        for th in [true, false, true, false, true, false, true] {
            let v = read_at(&mut io, th, t);
            if th {
                assert_eq!(v & 0x0F, 0x0F);
            }
            t += 100;
        }
    }

    #[test]
    fn an_empty_port_looks_like_no_button_pressed() {
        let (mut io, t) = ready(PadKind::None);
        assert_eq!(read_at(&mut io, true, t), 0x7F);
        assert_eq!(read_at(&mut io, false, t), 0x3F);
    }

    #[test]
    fn serial_registers_hold_what_is_written_and_status_bits_do_not() {
        let mut io = io(PadKind::Three);
        io.write(7, 0xA5, 0);
        assert_eq!(io.read(7, 0), 0xA5);
        io.write(9, 0xFF, 0);
        assert_eq!(io.read(9, 0), 0xF8);
        io.write(8, 0x12, 0);
        assert_eq!(io.read(8, 0), 0, "receive data is read only");
    }

    #[test]
    fn the_version_register_is_read_only() {
        let mut io = io(PadKind::Three);
        io.write(0, 0x00, 0);
        assert_eq!(io.read(0, 0), 0xA0);
    }
}
