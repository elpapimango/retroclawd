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
}

impl PadKind {
    pub fn parse(text: &str) -> Option<PadKind> {
        match text {
            "none" => Some(PadKind::None),
            "three" | "3" => Some(PadKind::Three),
            "six" | "6" => Some(PadKind::Six),
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

    /// Notes the pin's new level; a rising edge advances a 6-button pad.
    fn update_th(&mut self, now: u64) {
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
        let pins = self.pad_pins() | if self.th_level() { 0x40 } else { 0 };
        (self.data & self.control) | (pins & !self.control)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Io {
    version: u8,
    /// The two controller ports, the expansion port, and the two pads of a J-Cart cartridge.
    ports: [Port; 5],
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
                Port::new(pad1),
                Port::new(pad2),
                Port::new(PadKind::None),
                Port::new(PadKind::Three),
                Port::new(PadKind::Three),
            ],
        }
    }

    /// The register at `$A10000 + 2 * index` (byte accessed at the odd address `2 * index + 1`).
    pub fn read(&mut self, index: usize, now: u64) -> u8 {
        match index & 0x0F {
            0 => self.version,
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
            }
            i @ 4..=6 => {
                let p = &mut self.ports[i - 4];
                p.expire(now);
                p.control = value;
                p.update_th(now);
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

    /// A key of a pad: `port` 0 and 1 are the controller ports, 2 and 3 the pads of a J-Cart.
    pub fn set_button(&mut self, port: usize, mask: u16, pressed: bool) {
        let port = if port >= 2 { port + 1 } else { port };
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
