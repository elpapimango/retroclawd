//! POKEY timers, audio, polynomial counters, keyboard scan and idle SIO output.
//! Source: Altirra Hardware Reference (2026-01-02), chapter 5 and section 14.4:
//! <https://www.virtualdub.org/downloads/Altirra%20Hardware%20Reference%20Manual.pdf>.
//! Timers use period-level reloads; linked low-channel events, pipeline delays,
//! two-tone, analogue saturation and serial input/external clocks remain gaps.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Pokey {
    registers: [u8; 16],
    timers: [u32; 4],
    outputs: [bool; 4],
    highpass: [bool; 2],
    poly4: u32,
    poly5: u32,
    poly9: u32,
    poly17: u32,
    clock28: u8,
    clock114: u8,
    irq_status: u8,
    keycode: u8,
    skstat: u8,
    keys: u64,
    shift: bool,
    control: bool,
    break_down: bool,
    break_latched: bool,
    modifiers: u8,
    scan: u8,
    compare: u8,
    key_state: u8,
    pot_counter: u8,
    serial_pending: Option<u8>,
    serial_bits: u8,
    serial_phase: bool,
}

impl Default for Pokey {
    fn default() -> Self {
        Self {
            registers: [0; 16],
            timers: [28; 4],
            outputs: [true; 4],
            highpass: [true; 2],
            poly4: 0,
            poly5: 0,
            poly9: 0,
            poly17: 0,
            clock28: 19,
            clock114: 78,
            irq_status: 0xf7,
            keycode: 0xff,
            skstat: 0xff,
            keys: 0,
            shift: false,
            control: false,
            break_down: false,
            break_latched: false,
            modifiers: 0,
            scan: 0,
            compare: 0,
            key_state: 0,
            pot_counter: 228,
            serial_pending: None,
            serial_bits: 0,
            serial_phase: false,
        }
    }
}

impl Pokey {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(&self, index: usize) -> u8 {
        self.registers[index & 15]
    }
    pub fn set_keys(&mut self, keys: u64, shift: bool, control: bool, break_down: bool) {
        self.keys = keys;
        self.shift = shift;
        self.control = control;
        self.break_down = break_down;
    }
    pub fn read(&self, offset: u8) -> u8 {
        match offset & 15 {
            0..=7 => self.pot_counter,
            8 => {
                if self.pot_counter < 228 {
                    0xff
                } else {
                    0
                }
            }
            9 => self.keycode,
            10 => {
                if self.registers[15] & 3 == 0 {
                    0xff
                } else if self.registers[8] & 0x80 != 0 {
                    !(self.poly9 as u8)
                } else {
                    !(self.poly17 as u8)
                }
            }
            13 => 0xff,
            14 => self.irq_status,
            15 => self.skstat,
            _ => 0xff,
        }
    }
    pub fn write(&mut self, offset: u8, value: u8) {
        let offset = offset & 15;
        match offset {
            9 => {
                for channel in 0..4 {
                    self.timers[channel] = self.period(channel);
                }
                self.outputs = [true; 4];
                self.highpass = [true; 2];
            }
            10 => self.skstat |= 0xe0,
            11 => {
                self.pot_counter = 0;
            }
            13 => self.serial_pending = Some(value),
            14 => self.irq_status |= !value & !8,
            15 => {
                if value & 3 == 0 {
                    self.poly4 = 0;
                    self.poly5 = 0;
                    self.poly9 = 0;
                    self.poly17 = 0;
                    self.clock28 = 19;
                    self.clock114 = 78;
                    self.serial_pending = None;
                    self.serial_bits = 0;
                    self.irq_status &= !8;
                }
                if value & 2 == 0 {
                    self.key_state = 0;
                    self.scan = 0;
                    self.skstat |= 4;
                }
            }
            _ => {}
        }
        self.registers[usize::from(offset)] = value;
    }
    pub fn irq(&self) -> bool {
        !self.irq_status & self.registers[14] != 0
    }
    fn signal(&mut self, mask: u8) {
        self.irq_status &= !(mask & self.registers[14]);
    }
    fn period(&self, channel: usize) -> u32 {
        let pair = channel & !1;
        let fast = self.registers[8] & if pair == 0 { 0x40 } else { 0x20 } != 0;
        let joined = channel & 1 != 0 && self.registers[8] & if pair == 0 { 0x10 } else { 8 } != 0;
        let frequency = if joined {
            u32::from(u16::from_le_bytes([
                self.registers[pair * 2],
                self.registers[channel * 2],
            ]))
        } else {
            u32::from(self.registers[channel * 2])
        };
        if fast && (channel & 1 == 0 || joined) {
            frequency + if joined { 7 } else { 4 }
        } else {
            (frequency + 1) * if self.registers[8] & 1 == 0 { 28 } else { 114 }
        }
    }
    fn lfsr(value: &mut u32, width: u32, tap: u32) {
        let feedback = (!(*value ^ (*value >> tap))) & 1;
        *value = (*value >> 1) | (feedback << (width - 1));
    }
    fn scan_key(&mut self) {
        if self.registers[15] & 2 == 0 {
            return;
        }
        if self.scan == 0 {
            self.modifiers = (self.modifiers & 0x40) | (u8::from(self.control) << 7);
        }
        if self.scan == 0x10 {
            self.modifiers = (self.modifiers & 0x80) | (u8::from(self.shift) << 6);
            self.skstat = (self.skstat & !8) | (u8::from(!self.shift) << 3);
        }
        if self.scan == 0x30 {
            if self.break_down && !self.break_latched {
                self.signal(0x80);
            }
            self.break_latched = self.break_down;
        }
        let down = self.keys & (1u64 << self.scan) != 0;
        let matches = self.scan == self.compare || self.registers[15] & 1 == 0;
        match self.key_state {
            0 if down => {
                self.compare = self.scan;
                self.key_state = 1;
            }
            1 if matches && down => {
                if self.irq_status & 0x40 == 0 {
                    self.skstat &= !0x40;
                }
                self.keycode = self.scan | self.modifiers;
                self.signal(0x40);
                self.skstat &= !4;
                self.key_state = 3;
            }
            1 if (matches && !down) || (!matches && down) => self.key_state = 0,
            3 if matches && !down => self.key_state = 2,
            2 if matches && !down => {
                self.skstat |= 4;
                self.key_state = 0;
            }
            _ => {}
        }
        self.scan = self.scan.wrapping_sub(1) & 63;
    }
    fn serial_clock(&mut self) {
        self.serial_phase = !self.serial_phase;
        if !self.serial_phase {
            return;
        }
        if self.serial_bits == 0 && self.serial_pending.take().is_some() {
            self.serial_bits = 10;
            self.irq_status |= 8;
            self.signal(0x10);
        } else if self.serial_bits > 0 {
            self.serial_bits -= 1;
            if self.serial_bits == 0 {
                self.irq_status &= !8;
            }
        }
    }
    pub fn tick(&mut self) {
        let running = self.registers[15] & 3 != 0;
        if running {
            Self::lfsr(&mut self.poly4, 4, 1);
            Self::lfsr(&mut self.poly5, 5, 2);
            Self::lfsr(&mut self.poly9, 9, 4);
            Self::lfsr(&mut self.poly17, 17, 5);
            self.clock28 -= 1;
            if self.clock28 == 0 {
                self.clock28 = 28;
            }
            self.clock114 -= 1;
            if self.clock114 == 0 {
                self.clock114 = 114;
                self.scan_key();
                if self.registers[15] & 4 == 0 {
                    self.pot_counter = self.pot_counter.saturating_add(1).min(228);
                }
            }
        }
        if self.registers[15] & 4 != 0 {
            self.pot_counter = self.pot_counter.saturating_add(1).min(228);
        }
        let mut fired = [false; 4];
        for (channel, fire) in fired.iter_mut().enumerate() {
            let fast = channel.is_multiple_of(2)
                && self.registers[8] & if channel == 0 { 0x40 } else { 0x20 } != 0;
            if !running && !fast {
                continue;
            }
            self.timers[channel] = self.timers[channel].saturating_sub(1);
            if self.timers[channel] != 0 {
                continue;
            }
            self.timers[channel] = self.period(channel);
            *fire = true;
            self.signal([1, 2, 0, 4][channel]);
            let control = self.registers[channel * 2 + 1];
            if control & 0x80 != 0 || self.poly5 & 1 != 0 {
                self.outputs[channel] = if control & 0x20 != 0 {
                    !self.outputs[channel]
                } else if control & 0x40 != 0 {
                    self.poly4 & 1 != 0
                } else if self.registers[8] & 0x80 != 0 {
                    self.poly9 & 1 != 0
                } else {
                    self.poly17 & 1 != 0
                };
            }
        }
        for channel in 0..2 {
            if self.registers[8] & (4 >> channel) == 0 {
                self.highpass[channel] = true;
            } else if fired[channel + 2] {
                self.highpass[channel] = self.outputs[channel];
            }
        }
        if fired[3] && self.registers[15] & 0x60 == 0x20 {
            self.serial_clock();
        }
    }
    pub fn output(&self) -> f32 {
        (0..4)
            .map(|channel| {
                let control = self.registers[channel * 2 + 1];
                let enabled = control & 0x10 != 0
                    || (self.outputs[channel] ^ (channel < 2 && self.highpass[channel]));
                if enabled {
                    f32::from(control & 15) / 60.0
                } else {
                    0.0
                }
            })
            .sum()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.scan > 63
            || self.compare > 63
            || self.key_state > 3
            || self.clock28 == 0
            || self.clock28 > 28
            || self.clock114 == 0
            || self.clock114 > 114
            || self.poly4 > 15
            || self.poly5 > 31
            || self.poly9 > 511
            || self.poly17 > 131071
            || self.timers.iter().any(|&n| n > 7_471_104)
            || self.pot_counter > 228
            || self.serial_bits > 10
            || self.modifiers & 0x3f != 0
        {
            return Err("invalid POKEY state".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_scans_debounces_and_acknowledges_through_irqen() {
        let mut p = Pokey::new();
        p.write(15, 3);
        p.write(14, 0x40);
        p.set_keys(1 << 0x3f, false, false, false);
        for _ in 0..15000 {
            p.tick();
        }
        assert_eq!(p.read(9), 0x3f);
        assert!(p.irq());
        assert_eq!(p.read(15) & 4, 0);
        p.write(14, 0);
        assert!(!p.irq());
        p.set_keys(0, false, false, false);
        for _ in 0..15000 {
            p.tick();
        }
        assert_ne!(p.read(15) & 4, 0);
    }
    #[test]
    fn pure_tone_divisor_irq_and_init_random() {
        let mut p = Pokey::new();
        assert_eq!(p.read(10), 0xff);
        p.write(0, 72);
        p.write(1, 0xaf);
        p.write(15, 3);
        p.write(9, 0);
        p.write(14, 1);
        for _ in 0..2043 {
            p.tick();
        }
        assert!(!p.irq());
        let before = p.output();
        p.tick();
        assert!(p.irq());
        assert_ne!(p.output(), before);
        assert!(p.validate().is_ok());
    }
}
