//! ANTIC display-list, playfield DMA and scan counters.
//! Source: Altirra Hardware Reference (2026-01-02), chapter 4 and section 14.6.
//! <https://www.virtualdub.org/downloads/Altirra%20Hardware%20Reference%20Manual.pdf>
//! This initial engine omits scrolling, P/M DMA, abnormal DMA and exact
//! refresh displacement. It does not claim Acid800 conformance.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const WIDTH: usize = 384;
pub const HEIGHT: usize = 240;
const ROWS: [u8; 16] = [1, 1, 8, 10, 8, 16, 8, 16, 8, 4, 4, 2, 1, 2, 1, 1];
const BYTES: [u8; 16] = [0, 0, 40, 40, 40, 40, 20, 20, 10, 10, 20, 20, 20, 40, 40, 40];

#[derive(Clone, Serialize, Deserialize)]
pub struct Antic {
    registers: [u8; 16],
    lines: u16,
    pub x: u8,
    pub y: u16,
    pub cycles: u64,
    list: u16,
    scan: u16,
    address_low: u8,
    instruction: u8,
    row: u8,
    remaining: u8,
    wait_vblank: bool,
    names: Vec<u8>,
    data: Vec<u8>,
    line: Vec<u8>,
    status: u8,
    nmi: bool,
    wsync_until: u64,
    wsync_delay: u8,
}

impl Antic {
    pub fn new(pal: bool) -> Self {
        Self {
            registers: [0; 16],
            lines: if pal { 312 } else { 262 },
            x: 0,
            y: 0,
            cycles: 0,
            list: 0,
            scan: 0,
            address_low: 0,
            instruction: 0,
            row: 0,
            remaining: 0,
            wait_vblank: false,
            names: vec![0; 48],
            data: vec![0; 48],
            line: vec![0; WIDTH],
            status: 0x1f,
            nmi: false,
            wsync_until: 0,
            wsync_delay: 0,
        }
    }

    pub fn pal(&self) -> bool {
        self.lines == 312
    }
    pub fn register(&self, index: usize) -> u8 {
        self.registers[index & 15]
    }
    pub fn read(&self, offset: u8) -> u8 {
        match offset & 15 {
            11 => {
                let line = if self.x >= 111 { self.y + 1 } else { self.y };
                if line == self.lines && self.x > 111 {
                    0
                } else {
                    (line / 2) as u8
                }
            }
            12 | 13 => 0xff,
            15 => self.status,
            _ => 0xff,
        }
    }

    pub fn write(&mut self, offset: u8, value: u8) {
        let index = usize::from(offset & 15);
        match index {
            2 => self.list = (self.list & 0xff00) | u16::from(value),
            3 => self.list = (self.list & 0xff) | (u16::from(value) << 8),
            10 => {
                let base = self.cycles - u64::from(self.x);
                self.wsync_until = base + 105 + if self.x >= 104 { 114 } else { 0 };
                self.wsync_delay = 1;
            }
            15 => self.status = 0x1f,
            _ => {}
        }
        self.registers[index] = value;
    }

    pub fn reset_nmi(&mut self) {
        self.registers[14] = 0;
        self.status = 0x1f;
        self.nmi = false;
    }

    /// The 400/800 Reset button reaches ANTIC's RNMI, not the CPU RESET pin.
    pub fn reset_button(&mut self) {
        self.status = 0x3f;
        self.nmi = true;
    }

    pub fn take_nmi(&mut self) -> bool {
        std::mem::take(&mut self.nmi)
    }
    pub fn line(&self) -> &[u8] {
        &self.line
    }

    fn active(&self) -> bool {
        (8..248).contains(&self.y)
    }
    fn mode(&self) -> usize {
        usize::from(self.instruction & 15)
    }
    fn count(&self) -> u8 {
        match self.registers[0] & 3 {
            1 => BYTES[self.mode()] * 4 / 5,
            2 => BYTES[self.mode()],
            3 => BYTES[self.mode()] * 6 / 5,
            _ => 0,
        }
    }
    fn fetch_slot(&self) -> Option<(usize, bool)> {
        if !self.active() || self.wait_vblank || self.mode() < 2 {
            return None;
        }
        let count = self.count();
        if count == 0 {
            return None;
        }
        let step = 80 / BYTES[self.mode()];
        let start = 18i16 + (40 - i16::from(count) * i16::from(step) / 2);
        let position = i16::from(self.x) - start;
        if position >= 0 && position % i16::from(step) == 0 {
            let i = position / i16::from(step);
            if i < i16::from(count) && self.row == 0 {
                return Some((i as usize, false));
            }
        }
        if self.mode() <= 7 {
            let position = position - 3;
            if position >= 0 && position % i16::from(step) == 0 {
                let i = position / i16::from(step);
                if i < i16::from(count) {
                    return Some((i as usize, true));
                }
            }
        }
        None
    }

    pub fn cpu_read_blocked(&self) -> bool {
        (self.wsync_delay == 0 && self.cycles < self.wsync_until)
            || (self.x >= 25 && self.x <= 57 && (self.x - 25).is_multiple_of(4))
            || (self.active()
                && !self.wait_vblank
                && self.registers[0] & 0x20 != 0
                && ((self.x == 1 && self.remaining == 0)
                    || ((self.x == 3 || self.x == 4)
                        && self.row == 0
                        && (self.mode() == 1
                            || (self.mode() >= 2 && self.instruction & 0x40 != 0)))))
            || self.fetch_slot().is_some()
    }

    fn list_byte(&mut self, read: &mut impl FnMut(u16) -> u8) -> u8 {
        let value = read(self.list);
        self.list = (self.list & 0xfc00) | (self.list.wrapping_add(1) & 0x3ff);
        value
    }

    fn char_address(&self, name: u8) -> u16 {
        let mode = self.mode();
        let mut row = self.row;
        if mode == 5 || mode == 7 {
            row /= 2;
        }
        row &= 7;
        if self.registers[1] & 4 != 0 {
            row ^= 7;
        }
        let mask = if mode <= 5 { 0x7f } else { 0x3f };
        let base = u16::from(self.registers[9] & if mode <= 5 { 0xfc } else { 0xfe }) << 8;
        base | (u16::from(name & mask) << 3) | u16::from(row)
    }

    /// Advance one system clock; return true after producing one scan line.
    pub fn tick(&mut self, mut read: impl FnMut(u16) -> u8) -> bool {
        if self.x == 0 && self.y == 8 {
            self.wait_vblank = false;
            self.remaining = 0;
            self.instruction &= !0x40;
        }
        if self.x == 1 && self.active() && !self.wait_vblank && self.remaining == 0 {
            self.row = 0;
            if self.registers[0] & 0x20 != 0 {
                self.instruction = self.list_byte(&mut read);
            }
            self.remaining = if self.mode() == 0 {
                ((self.instruction >> 4) & 7) + 1
            } else {
                ROWS[self.mode()]
            };
        }
        if self.active()
            && !self.wait_vblank
            && self.registers[0] & 0x20 != 0
            && self.row == 0
            && (self.mode() == 1 || (self.mode() >= 2 && self.instruction & 0x40 != 0))
        {
            if self.x == 3 {
                self.address_low = self.list_byte(&mut read);
            }
            if self.x == 4 {
                let high = self.list_byte(&mut read);
                let address = u16::from_le_bytes([self.address_low, high]);
                if self.mode() == 1 {
                    self.list = address;
                    self.wait_vblank = self.instruction & 0x40 != 0;
                } else {
                    self.scan = address;
                }
            }
        }
        if let Some((i, font)) = self.fetch_slot() {
            if font {
                let name = self.names[i];
                let mut data = read(self.char_address(name));
                if self.mode() == 3
                    && ((name & 0x60 == 0x60 && self.row < 2)
                        || (name & 0x60 != 0x60 && self.row >= 8))
                {
                    data = 0;
                }
                if self.mode() <= 3 && name & 0x80 != 0 {
                    if self.registers[1] & 1 != 0 {
                        data = 0;
                    }
                    if self.registers[1] & 2 != 0 {
                        data ^= 0xff;
                    }
                }
                self.data[i] = data;
            } else {
                self.names[i] = read(self.scan);
                self.data[i] = self.names[i];
                self.scan = (self.scan & 0xf000) | (self.scan.wrapping_add(1) & 0x0fff);
            }
        }
        let dli = self.active() && self.remaining == 1 && self.instruction & 0x80 != 0;
        if self.x == 6 {
            if self.y == 248 {
                self.status = 0x5f;
            } else if dli {
                self.status |= 0x80;
            }
        }
        if self.x == 12
            && ((self.y == 248 && self.registers[14] & 0x40 != 0)
                || (dli && self.registers[14] & 0x80 != 0))
        {
            self.nmi = true;
        }
        let line_done = self.x == 113;
        if line_done {
            self.draw();
            if self.active() && self.remaining > 0 {
                self.remaining -= 1;
                if self.remaining > 0 {
                    self.row += 1;
                }
            }
        }
        self.wsync_delay = self.wsync_delay.saturating_sub(1);
        self.cycles += 1;
        self.x += 1;
        if self.x == 114 {
            self.x = 0;
            self.y += 1;
            if self.y == self.lines {
                self.y = 0;
            }
        }
        line_done
    }

    fn draw(&mut self) {
        self.line.fill(0);
        let mode = self.mode();
        let count = usize::from(self.count());
        if !self.active() || self.wait_vblank || mode < 2 || count == 0 {
            return;
        }
        let pixels_per_byte = match mode {
            6..=9 => 16 + usize::from(mode >= 8) * 16,
            10..=12 => 16,
            _ => 8,
        };
        let width = count * pixels_per_byte;
        let start = (WIDTH - width) / 2;
        for i in 0..count {
            let byte = self.data[i];
            for x in 0..pixels_per_byte {
                let code = match mode {
                    2 | 3 | 15 => 5 + ((byte >> (7 - x)) & 1),
                    4 | 5 | 8 | 10 | 13 | 14 => {
                        let pair = (byte >> (6 - (x * 4 / pixels_per_byte) * 2)) & 3;
                        if pair == 3 && mode <= 5 && self.names[i] & 0x80 != 0 {
                            4
                        } else {
                            pair
                        }
                    }
                    6 | 7 => {
                        if byte & (0x80 >> (x / 2)) != 0 {
                            (self.names[i] >> 6) + 1
                        } else {
                            0
                        }
                    }
                    _ => u8::from(byte & (0x80 >> (x * 8 / pixels_per_byte)) != 0),
                };
                self.line[start + i * pixels_per_byte + x] = code;
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if ![262, 312].contains(&self.lines)
            || self.x >= 114
            || self.y >= self.lines
            || self.row > 15
            || self.remaining > 16
            || self.names.len() != 48
            || self.data.len() != 48
            || self.line.len() != WIDTH
            || self.line.iter().any(|&x| x > 6)
            || self.wsync_delay > 1
            || self.cycles % (114 * u64::from(self.lines))
                != u64::from(self.y) * 114 + u64::from(self.x)
        {
            return Err("invalid ANTIC state".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_list_wrap_lms_text_and_jvb_repeat_without_cpu_patching() {
        let mut a = Antic::new(true);
        let mut ram = vec![0; 65536];
        ram[0x23fe..0x2400].copy_from_slice(&[0x42, 0]);
        ram[0x2000..0x2005].copy_from_slice(&[0x30, 0x41, 0xfe, 0x23, 0]);
        ram[0x3000] = 1;
        ram[0x4008..0x4010].fill(0x80);
        a.write(2, 0xfe);
        a.write(3, 0x23);
        a.write(9, 0x40);
        a.write(0, 0x22);
        a.write(1, 2);
        for _ in 0..(114 * 9) {
            a.tick(|x| ram[usize::from(x)]);
        }
        assert_eq!(a.line()[32], 6);
        assert_eq!(a.line()[33], 5);
        assert_eq!(a.scan, 0x3028);
        for _ in 0..(114 * 312) {
            a.tick(|x| ram[usize::from(x)]);
        }
        assert_eq!(a.line()[32], 6);
        assert!(a.validate().is_ok());
    }
    #[test]
    fn frame_period_and_vblank_nmi_are_generated_by_chip_clocks() {
        for pal in [false, true] {
            let mut a = Antic::new(pal);
            a.write(14, 0x40);
            let mut interrupts = 0;
            for _ in 0..114 * if pal { 312 } else { 262 } {
                a.tick(|_| 0);
                interrupts += u32::from(a.take_nmi());
            }
            assert_eq!((a.x, a.y, interrupts), (0, 0, 1));
            assert_eq!(a.read(15), 0x5f);
        }
    }
    #[test]
    fn wsync_allows_one_cycle_then_waits_until_horizontal_blank() {
        let mut a = Antic::new(false);
        a.write(10, 0);
        assert!(!a.cpu_read_blocked());
        a.tick(|_| 0);
        assert!(a.cpu_read_blocked());
        for _ in 1..105 {
            a.tick(|_| 0);
        }
        assert!(!a.cpu_read_blocked());
    }
}
