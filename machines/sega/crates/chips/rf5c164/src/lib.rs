//! The Ricoh RF5C164 PCM sound source as the Sega CD's sub-CPU sees it (facts P1-P4 in `docs/hardware-facts.md`): eight channels
//! with envelope, pan, address step, loop and start registers, a control register that selects a channel or a 4 KB bank of the
//! 64 KB wave RAM, and the on/off register.
//!
//! Playback (facts P4-P8): every 384 source clocks each channel that is not suspended reads one byte of the wave RAM at its
//! address, the address moves on by the step (5.11 fixed point), a `$FF` byte sends it back to the loop address, and the
//! channels are summed into a left and a right sample through the envelope and the pan.
#![forbid(unsafe_code)]

pub const WAVE_RAM_SIZE: usize = 0x1_0000;
/// The window of the wave RAM in the chip's address space is 4 KB.
pub const BANK_SIZE: usize = 0x1000;
pub const CHANNELS: usize = 8;
/// Source clocks per output sample (fact P4).
pub const CLOCKS_PER_SAMPLE: u64 = 384;
/// The fraction bits of the address and the step.
const FRACTION_BITS: u32 = 11;
/// The wave address has 16 integer bits.
const ADDRESS_MASK: u32 = (1 << (16 + FRACTION_BITS)) - 1;
/// The byte that ends a loop (fact P5).
const LOOP_STOP: u8 = 0xFF;
/// A channel at full envelope, pan and wave amplitude reaches this many output units; eight of them are limited to 16 bits (fact P6).
const OUTPUT_SHIFT: u32 = 5;

#[derive(Clone, Debug, Default)]
struct Channel {
    env: u8,
    pan: u8,
    fd: u16,
    loop_addr: u16,
    start: u8,
    /// The wave address: 16 integer bits and 11 fraction bits.
    addr: u32,
}

impl Channel {
    /// The integer part of the address, which the read-only registers show.
    fn counter(&self) -> u16 {
        (self.addr >> FRACTION_BITS) as u16
    }
}

#[derive(Clone, Debug)]
pub struct Pcm {
    channels: [Channel; CHANNELS],
    wave: Box<[u8]>,
    /// Control register: bit 7 sounding, bit 6 MOD, bits 3-0 wave bank (MOD = 0) or bits 2-0 channel select (MOD = 1).
    on: bool,
    mod_channel: bool,
    bank: u8,
    selected: u8,
    /// The on/off register: a set bit suspends that channel.
    off_mask: u8,
    /// Source clocks not yet a whole sample.
    acc: u64,
}

impl Default for Pcm {
    fn default() -> Self {
        Self::new()
    }
}

impl Pcm {
    pub fn new() -> Self {
        Self {
            channels: Default::default(),
            wave: vec![0; WAVE_RAM_SIZE].into_boxed_slice(),
            on: false,
            mod_channel: false,
            bank: 0,
            selected: 0,
            off_mask: 0xFF,
            acc: 0,
        }
    }

    pub fn sounding(&self) -> bool {
        self.on
    }

    pub fn bank(&self) -> u8 {
        self.bank
    }

    pub fn selected_channel(&self) -> u8 {
        self.selected
    }

    pub fn suspended_mask(&self) -> u8 {
        self.off_mask
    }

    /// The wave RAM in the bank the control register chose, for DMA from the CD controller.
    pub fn bank_mut(&mut self) -> &mut [u8] {
        let at = usize::from(self.bank) * BANK_SIZE;
        &mut self.wave[at..at + BANK_SIZE]
    }

    pub fn wave_ram(&self) -> &[u8] {
        &self.wave
    }

    /// A write to the chip's 13-bit address space (`$0000-$1FFF`). `false`: an address the manual calls not usable.
    pub fn write(&mut self, chip_addr: u16, v: u8) -> bool {
        match chip_addr {
            0x0..=0x6 => {
                let ch = &mut self.channels[usize::from(self.selected)];
                match chip_addr {
                    0 => ch.env = v,
                    1 => ch.pan = v,
                    2 => ch.fd = (ch.fd & 0xFF00) | u16::from(v),
                    3 => ch.fd = (ch.fd & 0x00FF) | (u16::from(v) << 8),
                    4 => ch.loop_addr = (ch.loop_addr & 0xFF00) | u16::from(v),
                    5 => ch.loop_addr = (ch.loop_addr & 0x00FF) | (u16::from(v) << 8),
                    _ => {
                        ch.start = v;
                        // the address restarts at the start address (facts P3, P5)
                        ch.addr = (u32::from(v) << 8) << FRACTION_BITS;
                    }
                }
                true
            }
            0x7 => {
                self.on = v & 0x80 != 0;
                self.mod_channel = v & 0x40 != 0;
                if self.mod_channel {
                    self.selected = v & 7;
                } else {
                    self.bank = v & 0xF;
                }
                true
            }
            0x8 => {
                self.off_mask = v;
                true
            }
            0x1000..=0x1FFF => {
                // wave RAM, in the bank the control register chose; while the chip is sounding the manual restricts writes
                // (facts P3); the bytes are stored either way
                let at = usize::from(self.bank) * BANK_SIZE + usize::from(chip_addr - 0x1000);
                self.wave[at] = v;
                true
            }
            _ => false,
        }
    }

    /// One output sample: the channels that sound add their byte, scaled, to the left and right sums (facts P5, P6).
    fn sample(&mut self) -> (i16, i16) {
        let (mut l, mut r) = (0i32, 0i32);
        if self.on {
            for (i, ch) in self.channels.iter_mut().enumerate() {
                if self.off_mask & (1 << i) != 0 {
                    continue;
                }
                let mut v = self.wave[usize::from(ch.counter())];
                if v == LOOP_STOP {
                    ch.addr = u32::from(ch.loop_addr) << FRACTION_BITS;
                    v = self.wave[usize::from(ch.loop_addr)];
                    if v == LOOP_STOP {
                        continue; // a loop that points at its own stop byte makes no sound
                    }
                }
                ch.addr = (ch.addr + u32::from(ch.fd)) & ADDRESS_MASK;
                let magnitude = i32::from(v & 0x7F);
                let wave = if v & 0x80 != 0 { magnitude } else { -magnitude };
                let level = wave * i32::from(ch.env);
                l += (level * i32::from(ch.pan & 0xF)) >> OUTPUT_SHIFT;
                r += (level * i32::from(ch.pan >> 4)) >> OUTPUT_SHIFT;
            }
        }
        (
            l.clamp(-0x8000, 0x7FFF) as i16,
            r.clamp(-0x8000, 0x7FFF) as i16,
        )
    }

    /// Runs the chip for `clocks` source clocks; every finished sample is pushed to `out`.
    pub fn advance(&mut self, clocks: u64, out: &mut Vec<(i16, i16)>) {
        self.acc += clocks;
        while self.acc >= CLOCKS_PER_SAMPLE {
            self.acc -= CLOCKS_PER_SAMPLE;
            let s = self.sample();
            out.push(s);
        }
    }

    /// A read of the wave RAM window or of the address counters (`$10-$1F`, low then high byte of each channel).
    pub fn read(&self, chip_addr: u16) -> Option<u8> {
        match chip_addr {
            0x1000..=0x1FFF => {
                let at = usize::from(self.bank) * BANK_SIZE + usize::from(chip_addr - 0x1000);
                Some(self.wave[at])
            }
            0x10..=0x1F => {
                let ch = &self.channels[usize::from((chip_addr - 0x10) >> 1)];
                Some(if chip_addr & 1 == 0 {
                    ch.counter() as u8
                } else {
                    (ch.counter() >> 8) as u8
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_control_register_selects_a_channel_or_a_bank_by_its_mod_bit() {
        let mut p = Pcm::new();
        assert!(p.write(7, 0x80 | 0x05)); // MOD = 0: bank 5, sounding
        assert_eq!((p.sounding(), p.bank(), p.selected_channel()), (true, 5, 0));
        assert!(p.write(7, 0x40 | 0x03)); // MOD = 1: channel 4, sound off
        assert_eq!(
            (p.sounding(), p.bank(), p.selected_channel()),
            (false, 5, 3)
        );
    }

    #[test]
    fn channel_registers_go_to_the_selected_channel_and_the_on_off_register_to_all() {
        let mut p = Pcm::new();
        p.write(7, 0x40 | 2);
        for (a, v) in [(0, 0xFF), (1, 0x77), (2, 0x00), (3, 0x04), (6, 0x10)] {
            assert!(p.write(a, v));
        }
        assert_eq!(
            (p.read(0x15), p.read(0x14)),
            (Some(0x10), Some(0)),
            "channel 3 counter from its start address"
        );
        p.write(7, 0x40);
        assert!(p.write(8, 0xFE));
        assert_eq!(p.suspended_mask(), 0xFE);
        assert!(!p.write(0x0F, 0), "not usable addresses are refused");
    }

    #[test]
    fn wave_ram_is_a_4k_window_onto_64k_by_bank() {
        let mut p = Pcm::new();
        p.write(7, 0x03);
        assert!(p.write(0x1000, 0xAA));
        p.write(7, 0x04);
        assert!(p.write(0x1000, 0xBB));
        assert_eq!(p.wave_ram()[3 * BANK_SIZE], 0xAA);
        assert_eq!(p.wave_ram()[4 * BANK_SIZE], 0xBB);
        assert_eq!(p.read(0x1000), Some(0xBB));
        p.write(7, 0x03);
        assert_eq!(p.read(0x1000), Some(0xAA));
        assert_eq!(p.read(0x10), Some(0), "the counters start at zero");
    }

    /// Plays channel 1 over `bytes` of wave RAM from address 0 with the given register values.
    fn play(
        bytes: &[u8],
        env: u8,
        pan: u8,
        fd: u16,
        loop_addr: u16,
        samples: usize,
    ) -> Vec<(i16, i16)> {
        let mut p = Pcm::new();
        p.write(7, 0); // bank 0, sound off
        for (i, b) in bytes.iter().enumerate() {
            p.write(0x1000 + i as u16, *b);
        }
        p.write(7, 0x40); // channel 1
        for (a, v) in [
            (0, env),
            (1, pan),
            (2, fd as u8),
            (3, (fd >> 8) as u8),
            (4, loop_addr as u8),
            (5, (loop_addr >> 8) as u8),
            (6, 0),
        ] {
            p.write(a, v);
        }
        p.write(8, 0xFE); // only channel 1 sounds
        p.write(7, 0x80);
        let mut out = Vec::new();
        p.advance(samples as u64 * CLOCKS_PER_SAMPLE, &mut out);
        out
    }

    fn level(v: i32) -> i16 {
        ((v * 255 * 15) >> OUTPUT_SHIFT) as i16
    }

    #[test]
    fn a_sample_comes_out_every_384_source_clocks() {
        let mut p = Pcm::new();
        let mut out = Vec::new();
        p.advance(383, &mut out);
        assert!(out.is_empty());
        p.advance(1, &mut out);
        assert_eq!(out.len(), 1);
        p.advance(384 * 10, &mut out);
        assert_eq!(out.len(), 11);
    }

    #[test]
    fn the_sign_bit_envelope_and_pan_scale_the_byte() {
        // 0x80 | 100 is +100 and 100 is -100; full envelope, left level 15 and right level 7
        let out = play(&[0x80 | 100, 100, 0xFF], 0xFF, 0x7F, 0x0800, 0, 2);
        let l = (100 * 255 * 15) >> OUTPUT_SHIFT;
        let r = (100 * 255 * 7) >> OUTPUT_SHIFT;
        assert_eq!(out[0], (l as i16, r as i16));
        let (nl, nr) = (
            (-100 * 255 * 15) >> OUTPUT_SHIFT,
            (-100 * 255 * 7) >> OUTPUT_SHIFT,
        );
        assert_eq!(out[1], (nl as i16, nr as i16));
    }

    #[test]
    fn the_address_step_is_a_5_11_fixed_point_number() {
        // a step of 0x0400 is one half: every byte plays twice; a step of 0x1000 is two: every second byte plays
        let wave = [0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0xFF];
        let half: Vec<i16> = play(&wave, 0xFF, 0xFF, 0x0400, 0, 6)
            .iter()
            .map(|s| s.0)
            .collect();
        assert_eq!(half, [1, 1, 2, 2, 3, 3].map(level));
        let double: Vec<i16> = play(&wave, 0xFF, 0xFF, 0x1000, 0, 3)
            .iter()
            .map(|s| s.0)
            .collect();
        assert_eq!(double, [1, 3, 5].map(level));
    }

    #[test]
    fn a_stop_byte_sends_the_address_to_the_loop_address() {
        let wave = [0x80 | 10, 0x80 | 20, 0xFF];
        let out: Vec<i16> = play(&wave, 0xFF, 0xFF, 0x0800, 1, 5)
            .iter()
            .map(|s| s.0)
            .collect();
        assert_eq!(
            out,
            [10, 20, 20, 20, 20].map(level),
            "after the stop byte the loop restarts at byte 1"
        );
    }

    #[test]
    fn a_suspended_channel_and_a_chip_that_is_off_are_silent() {
        let mut p = Pcm::new();
        p.write(7, 0);
        p.write(0x1000, 0x80 | 50);
        p.write(0x1001, 0x80 | 50);
        p.write(7, 0x40);
        p.write(0, 0xFF);
        p.write(1, 0xFF);
        p.write(3, 0x08);
        p.write(8, 0xFF); // all suspended
        p.write(7, 0x80);
        let mut out = Vec::new();
        p.advance(384 * 3, &mut out);
        assert!(out.iter().all(|s| *s == (0, 0)), "suspended");
        p.write(8, 0xFE);
        p.write(7, 0x00); // chip off
        out.clear();
        p.advance(384 * 3, &mut out);
        assert!(out.iter().all(|s| *s == (0, 0)), "off");
        p.write(7, 0x80);
        out.clear();
        p.advance(384, &mut out);
        assert_ne!(out[0], (0, 0));
    }

    #[test]
    fn the_sum_of_the_channels_is_limited_to_sixteen_bits() {
        let mut p = Pcm::new();
        p.write(7, 0);
        p.write(0x1000, 0x80 | 126);
        for ch in 0..8u8 {
            p.write(7, 0x40 | ch);
            for (a, v) in [(0, 0xFF), (1, 0xFF), (3, 0x00), (6, 0)] {
                p.write(a, v);
            }
        }
        p.write(8, 0);
        p.write(7, 0x80);
        let mut out = Vec::new();
        p.advance(384, &mut out);
        assert_eq!(out[0], (0x7FFF, 0x7FFF));
    }
}
