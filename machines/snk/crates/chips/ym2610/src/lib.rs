//! The Yamaha YM2610 (OPNB) as the Neo Geo uses it, built from `docs/hardware-facts.md` rows Y1-Y8. A sample is made every 144
//! chip clocks (8 MHz / 144 = 55 555 Hz). So far: the register interface of both address pairs, timers A and B with their
//! flags and interrupt line, the SSG (the shared YM2149 core) and the six ADPCM-A channels.
#![forbid(unsafe_code)]

mod fm;

use ay_3_8910::{Ay3_8910, Variant};

/// Chip clocks between two samples (fact Y1).
pub const CLOCKS_PER_SAMPLE: u32 = 144;
/// The sample rate at the 8 MHz Neo Geo clock.
pub const SAMPLE_RATE: u32 = 8_000_000 / CLOCKS_PER_SAMPLE;

/// How loud the FM channels are in the mix (one carrier at full volume reaches this many of the 32768 levels): set so the BIOS
/// jingle, which is all FM, has the reference's level, and ADPCM-A is scaled 1.25 times its first estimate for the game music (fact Y10).
const FM_GAIN: f32 = 4080.0;

/// The step sizes of the ADPCM-A decoder: 16 times 1.1 to the power n, rounded down (fact Y5).
fn step_table() -> [i32; 49] {
    let mut t = [0i32; 49];
    let mut v = 16.0f64;
    for e in t.iter_mut() {
        *e = v as i32;
        v *= 1.1;
    }
    t
}

/// How the step index moves for the three magnitude bits of a code (fact Y5).
const INDEX_STEP: [i32; 8] = [-1, -1, -1, -1, 2, 5, 7, 9];

#[derive(Clone, Copy, Default)]
struct AdpcmA {
    /// Sample bytes: where it starts and ends (registers `$10-$15`/`$18-$1D` and `$20-$25`/`$28-$2D`, in units of 256 bytes).
    start: u32,
    end: u32,
    /// The channel level (0-31) and its output sides.
    level: u8,
    left: bool,
    right: bool,
    playing: bool,
    /// The read position in nibbles from `start`, and the decoder's state.
    pos: u32,
    acc: i32,
    step_index: i32,
}

#[derive(Clone, Copy, Default)]
struct AdpcmB {
    start: u32,
    end: u32,
    /// The playback rate: the position moves by this much of 65536 a sample.
    delta_n: u32,
    level: u8,
    left: bool,
    right: bool,
    repeat: bool,
    playing: bool,
    phase: u32,
    /// The read position in nibbles from `start`.
    pos: u32,
    acc: i32,
    step: i32,
}

pub struct Ym2610 {
    regs: [[u8; 256]; 2],
    addr: [u8; 2],
    timer_a: u16,
    timer_b: u8,
    counter_a: u32,
    counter_b: u32,
    /// Status bits: 0 timer A flag, 1 timer B flag (fact Y3).
    flags: u8,
    control: u8,
    ssg: Ay3_8910,
    fm: fm::Fm,
    adpcm_phase: u32,
    adpcm_a: [AdpcmA; 6],
    adpcm_b: AdpcmB,
    /// The end flags read at port 2: bits 0-5 ADPCM-A channels, bit 7 ADPCM-B (fact Y7).
    adpcm_flags: u8,
    master_level: u8,
    /// Chip clocks since the last sample, and the samples made (left, right).
    clocks: u32,
    timer_b_prescale: u32,
    out: Vec<(i16, i16)>,
    steps: [i32; 49],
    /// A debugging aid: the sources named in the environment variable RC_MUTE (f, s, a, b) are left out of the mix.
    mute: String,
}

impl Default for Ym2610 {
    fn default() -> Self {
        Self::new()
    }
}

impl Ym2610 {
    pub fn new() -> Ym2610 {
        Ym2610 {
            regs: [[0; 256]; 2],
            addr: [0; 2],
            timer_a: 0,
            timer_b: 0,
            counter_a: 0,
            counter_b: 0,
            flags: 0,
            control: 0,
            ssg: Ay3_8910::with_variant(Variant::Ym2149),
            fm: fm::Fm::new(),
            adpcm_phase: 0,
            adpcm_a: [AdpcmA::default(); 6],
            adpcm_b: AdpcmB::default(),
            adpcm_flags: 0,
            master_level: 0,
            clocks: 0,
            timer_b_prescale: 0,
            out: Vec::new(),
            steps: step_table(),
            mute: std::env::var("RC_MUTE").unwrap_or_default(),
        }
    }

    /// A write to one of the four ports: 0 the address of pair A, 1 its data, 2 and 3 the same for pair B.
    pub fn write(&mut self, port: u8, value: u8) {
        let pair = usize::from((port >> 1) & 1);
        if port & 1 == 0 {
            self.addr[pair] = value;
            return;
        }
        let reg = self.addr[pair];
        self.regs[pair][usize::from(reg)] = value;
        if pair == 0 {
            self.write_a(reg, value);
        } else {
            self.write_b(reg, value);
        }
    }

    fn write_a(&mut self, reg: u8, value: u8) {
        match reg {
            0x00..=0x0F => {
                self.ssg.select_register(reg);
                self.ssg.write(value);
            }
            0x10 => {
                // bit 7 starts, bit 4 repeats, bit 0 resets (fact Y7)
                let b = &mut self.adpcm_b;
                b.repeat = value & 0x10 != 0;
                if value & 1 != 0 {
                    b.playing = false;
                } else if value & 0x80 != 0 {
                    b.playing = true;
                    b.pos = 0;
                    b.phase = 0;
                    b.acc = 0;
                    b.step = 127;
                }
            }
            0x11 => {
                self.adpcm_b.left = value & 0x80 != 0;
                self.adpcm_b.right = value & 0x40 != 0;
            }
            0x12 | 0x13 => {
                let (lo, hi) = (self.regs[0][0x12], self.regs[0][0x13]);
                self.adpcm_b.start = (u32::from(hi) << 8 | u32::from(lo)) << 8;
            }
            0x14 | 0x15 => {
                let (lo, hi) = (self.regs[0][0x14], self.regs[0][0x15]);
                self.adpcm_b.end = ((u32::from(hi) << 8 | u32::from(lo)) << 8) | 0xFF;
            }
            0x19 | 0x1A => {
                self.adpcm_b.delta_n =
                    u32::from(self.regs[0][0x1A]) << 8 | u32::from(self.regs[0][0x19]);
            }
            0x1B => self.adpcm_b.level = value,
            // the end flags are reset by writing their bits here (fact Y7)
            0x1C => self.adpcm_flags &= !value,
            0x28 | 0x30..=0xB7 => self.fm.write(0, reg, value),
            0x24 => self.timer_a = (self.timer_a & 3) | u16::from(value) << 2,
            0x25 => self.timer_a = (self.timer_a & !3) | u16::from(value & 3),
            0x26 => self.timer_b = value,
            0x27 => {
                let was = self.control;
                self.control = value;
                // a load bit going from 0 to 1 loads the counter (fact Y3)
                if value & 1 != 0 && was & 1 == 0 {
                    self.counter_a = u32::from(self.timer_a);
                }
                if value & 2 != 0 && was & 2 == 0 {
                    self.counter_b = u32::from(self.timer_b);
                    self.timer_b_prescale = 0;
                }
                if value & 0x10 != 0 {
                    self.flags &= !1;
                }
                if value & 0x20 != 0 {
                    self.flags &= !2;
                }
            }
            _ => {}
        }
    }

    fn write_b(&mut self, reg: u8, value: u8) {
        match reg {
            0x00 => {
                // bit 7 set stops the channels named in bits 5-0, clear starts them (fact Y4)
                for c in 0..6 {
                    if value & (1 << c) != 0 {
                        let ch = &mut self.adpcm_a[c];
                        if value & 0x80 != 0 {
                            ch.playing = false;
                        } else {
                            ch.playing = true;
                            ch.pos = 0;
                            ch.acc = 0;
                            ch.step_index = 0;
                        }
                    }
                }
            }
            0x01 => self.master_level = value & 0x3F,
            0x30..=0xB7 => self.fm.write(1, reg, value),
            0x08..=0x0D => {
                let ch = &mut self.adpcm_a[usize::from(reg - 0x08)];
                ch.level = value & 0x1F;
                ch.left = value & 0x80 != 0;
                ch.right = value & 0x40 != 0;
            }
            0x10..=0x15 | 0x18..=0x1D => {
                let c = usize::from(reg & 7);
                let (lo, hi) = (self.regs[1][0x10 + c], self.regs[1][0x18 + c]);
                self.adpcm_a[c].start = (u32::from(hi) << 8 | u32::from(lo)) << 8;
            }
            0x20..=0x25 | 0x28..=0x2D => {
                let c = usize::from(reg & 7);
                let (lo, hi) = (self.regs[1][0x20 + c], self.regs[1][0x28 + c]);
                self.adpcm_a[c].end = ((u32::from(hi) << 8 | u32::from(lo)) << 8) | 0xFF;
            }
            _ => {}
        }
    }

    /// A read of a port: 0 the status of pair A (bits 1-0 the timer flags; the busy bit is never set here), 1 the SSG's
    /// selected register when the address is below `$10`; the rest read 0 (fact Y3).
    pub fn read(&self, port: u8) -> u8 {
        match port & 3 {
            0 => self.flags,
            1 if self.addr[0] < 0x10 => {
                let mut ssg = self.ssg.clone();
                ssg.select_register(self.addr[0]);
                ssg.read()
            }
            2 => self.adpcm_flags,
            _ => 0,
        }
    }

    /// The interrupt line to the Z80: a timer flag that is also enabled (fact Y3).
    pub fn irq(&self) -> bool {
        (self.flags & 1 != 0 && self.control & 4 != 0)
            || (self.flags & 2 != 0 && self.control & 8 != 0)
    }

    /// Advances the chip by `clocks` of its 8 MHz clock; `rom_a` is the ADPCM-A sample ROM.
    pub fn advance(&mut self, clocks: u32, rom_a: &[u8], rom_b: &[u8]) {
        self.clocks += clocks;
        while self.clocks >= CLOCKS_PER_SAMPLE {
            self.clocks -= CLOCKS_PER_SAMPLE;
            self.sample(rom_a, rom_b);
        }
    }

    /// The samples made since the last call, as (left, right).
    pub fn take_samples(&mut self) -> Vec<(i16, i16)> {
        std::mem::take(&mut self.out)
    }

    fn sample(&mut self, rom_a: &[u8], rom_b: &[u8]) {
        // timer A counts at the sample rate up to 1024, timer B every 16 samples up to 256 (fact Y3)
        if self.control & 1 != 0 {
            self.counter_a += 1;
            if self.counter_a >= 1024 {
                self.counter_a = u32::from(self.timer_a);
                self.flags |= 1;
            }
        }
        if self.control & 2 != 0 {
            self.timer_b_prescale += 1;
            if self.timer_b_prescale >= 16 {
                self.timer_b_prescale = 0;
                self.counter_b += 1;
                if self.counter_b >= 256 {
                    self.counter_b = u32::from(self.timer_b);
                    self.flags |= 2;
                }
            }
        }
        // the SSG runs at a quarter of the chip clock: 36 SSG clocks a sample (fact Y2)
        self.ssg.tick(36);
        let ssg = if self.mute.contains('s') {
            0.0
        } else {
            self.ssg.output()
        };
        // the FM channels at a gain that puts one full carrier at about a quarter of full scale (fact Y6)
        let (mut fl, mut fr) = self.fm.sample();
        if self.mute.contains('f') {
            (fl, fr) = (0.0, 0.0);
        }
        let mut left = (ssg * 4096.0 + fl * FM_GAIN) as i32;
        let mut right = (ssg * 4096.0 + fr * FM_GAIN) as i32;
        // ADPCM-A plays at a third of the sample rate (8 MHz / 432 = 18.5 kHz): a new nibble every three samples (fact Y5)
        self.adpcm_phase = (self.adpcm_phase + 1) % 3;
        let advance = self.adpcm_phase == 0;
        let master = i32::from(self.master_level);
        for (idx, ch) in self.adpcm_a.iter_mut().enumerate() {
            if !ch.playing {
                continue;
            }
            if advance {
                if ch.start + ch.pos / 2 > ch.end {
                    ch.playing = false;
                    self.adpcm_flags |= 1 << idx;
                    continue;
                }
                let byte = rom_a
                    .get((ch.start + ch.pos / 2) as usize)
                    .copied()
                    .unwrap_or(0);
                let code = if ch.pos & 1 == 0 {
                    byte >> 4
                } else {
                    byte & 0xF
                };
                ch.pos += 1;
                let step = self.steps[ch.step_index as usize];
                let mag = i32::from(code & 7);
                let mut diff = ((2 * mag + 1) * step) >> 3;
                if code & 8 != 0 {
                    diff = -diff;
                }
                // a 12-bit accumulator
                ch.acc = ((ch.acc + diff) << 20) >> 20;
                ch.step_index = (ch.step_index + INDEX_STEP[usize::from(code & 7)]).clamp(0, 48);
            }
            // the level of the channel and the master level scale the 12-bit sample (fact Y5)
            let v = (ch.acc * (i32::from(ch.level) + 1) * (master + 1) * 5) >> 11;
            if ch.left && !self.mute.contains('a') {
                left += v;
            }
            if ch.right && !self.mute.contains('a') {
                right += v;
            }
        }
        // ADPCM-B: the delta decoder at the rate delta-N / 65536 of a nibble a sample (fact Y7)
        let b = &mut self.adpcm_b;
        if b.playing {
            b.phase += b.delta_n;
            while b.phase >= 0x1_0000 && b.playing {
                b.phase -= 0x1_0000;
                if b.start + b.pos / 2 > b.end {
                    if b.repeat {
                        b.pos = 0;
                        b.acc = 0;
                        b.step = 127;
                    } else {
                        b.playing = false;
                        self.adpcm_flags |= 0x80;
                    }
                    continue;
                }
                let byte = rom_b
                    .get((b.start + b.pos / 2) as usize)
                    .copied()
                    .unwrap_or(0);
                let code = if b.pos & 1 == 0 {
                    byte >> 4
                } else {
                    byte & 0xF
                };
                b.pos += 1;
                let mag = i32::from(code & 7);
                let mut diff = ((2 * mag + 1) * b.step) >> 3;
                if code & 8 != 0 {
                    diff = -diff;
                }
                b.acc = (b.acc + diff).clamp(-32768, 32767);
                const ADJUST: [i32; 8] = [57, 57, 57, 57, 77, 102, 128, 153];
                b.step = ((b.step * ADJUST[mag as usize]) >> 6).clamp(127, 24576);
            }
            let v = (b.acc * i32::from(b.level)) >> 10;
            if b.left {
                left += v;
            }
            if b.right {
                right += v;
            }
        }
        self.out.push((
            left.clamp(-32768, 32767) as i16,
            right.clamp(-32768, 32767) as i16,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(chip: &mut Ym2610, pair: u8, reg: u8, value: u8) {
        chip.write(pair * 2, reg);
        chip.write(pair * 2 + 1, value);
    }

    #[test]
    fn the_step_table_runs_from_16_to_1552() {
        let t = step_table();
        assert_eq!(t[0], 16);
        assert_eq!(t[1], 17);
        assert_eq!(t[2], 19);
        assert_eq!(t[48], 1552);
    }

    #[test]
    fn timer_a_sets_its_flag_after_the_counts_left_and_the_flag_clears() {
        let mut chip = Ym2610::new();
        // load value 1020: four counts to 1024
        write(&mut chip, 0, 0x24, (1020 >> 2) as u8);
        write(&mut chip, 0, 0x25, (1020 & 3) as u8);
        write(&mut chip, 0, 0x27, 0b0001_0101); // load A, enable A flag and interrupt, reset flag A
        assert!(!chip.irq());
        chip.advance(CLOCKS_PER_SAMPLE * 3, &[], &[]);
        assert_eq!(chip.read(0) & 1, 0);
        chip.advance(CLOCKS_PER_SAMPLE, &[], &[]);
        assert_eq!(chip.read(0) & 1, 1);
        assert!(chip.irq());
        write(&mut chip, 0, 0x27, 0b0001_0101);
        assert!(!chip.irq());
        // it repeats
        chip.advance(CLOCKS_PER_SAMPLE * 4, &[], &[]);
        assert!(chip.irq());
    }

    #[test]
    fn timer_b_counts_every_sixteen_samples() {
        let mut chip = Ym2610::new();
        write(&mut chip, 0, 0x26, 254);
        write(&mut chip, 0, 0x27, 0b0010_1010); // load B, enable B, reset flag B
        chip.advance(CLOCKS_PER_SAMPLE * 16 * 2 - 1, &[], &[]);
        assert_eq!(chip.read(0) & 2, 0);
        chip.advance(1, &[], &[]);
        assert_eq!(chip.read(0) & 2, 2);
    }

    #[test]
    fn an_adpcm_b_sample_plays_at_its_rate_and_sets_the_end_flag() {
        let mut chip = Ym2610::new();
        let rom = vec![0x77u8; 0x200];
        write(&mut chip, 0, 0x11, 0xC0);
        write(&mut chip, 0, 0x12, 0);
        write(&mut chip, 0, 0x13, 0);
        write(&mut chip, 0, 0x14, 0);
        write(&mut chip, 0, 0x15, 0);
        write(&mut chip, 0, 0x19, 0x00);
        write(&mut chip, 0, 0x1A, 0x80); // delta-N 0x8000: a nibble every two samples
        write(&mut chip, 0, 0x1B, 0xFF);
        write(&mut chip, 0, 0x10, 0x80);
        chip.advance(CLOCKS_PER_SAMPLE * 40, &[], &rom);
        let samples = chip.take_samples();
        assert!(samples.iter().any(|s| s.0 > 0), "it made sound");
        // 256 bytes from 0 to the end address 0xFF: it is still playing after 40 samples
        assert_eq!(chip.read(2) & 0x80, 0);
        chip.advance(CLOCKS_PER_SAMPLE * 2000, &[], &rom);
        assert_eq!(chip.read(2) & 0x80, 0x80);
        write(&mut chip, 0, 0x1C, 0x80);
        assert_eq!(chip.read(2) & 0x80, 0);
    }

    #[test]
    fn the_ssg_registers_reach_the_shared_core() {
        let mut chip = Ym2610::new();
        write(&mut chip, 0, 0x00, 0x34);
        chip.write(0, 0x00);
        assert_eq!(chip.read(1), 0x34);
    }

    #[test]
    fn an_adpcm_a_channel_decodes_its_nibbles_and_stops_at_the_end() {
        let mut chip = Ym2610::new();
        let mut rom = vec![0u8; 0x200];
        rom[0] = 0x77; // two big positive steps
        write(&mut chip, 1, 0x01, 0x3F);
        write(&mut chip, 1, 0x08, 0xDF); // both sides, full level
        write(&mut chip, 1, 0x10, 0);
        write(&mut chip, 1, 0x18, 0);
        write(&mut chip, 1, 0x20, 0);
        write(&mut chip, 1, 0x28, 0);
        write(&mut chip, 1, 0x00, 0x01); // start channel 0
        chip.advance(CLOCKS_PER_SAMPLE * 12, &rom, &[]);
        let samples = chip.take_samples();
        assert_eq!(samples.len(), 12);
        assert!(samples.iter().any(|s| s.0 > 0), "the channel made sound");
        assert_eq!(samples[0].0, samples[0].1);
    }
}
