//! The STE's DMA sound: eight-bit signed samples played from memory at four rates, and the LMC1992 volume control behind the
//! Microwire interface (`docs/hardware-facts.md`, "STE DMA sound").
//!
//! - [H] *Atari ST Internals*: `$FF8901` bit 0 play, bit 1 loop; the frame base (`$FF8903/05/07`), counter (`$FF8909/0B/0D`) and
//!   end (`$FF890F/11/13`) registers are 24-bit even addresses; `$FF8921` bits 1-0 the rate (6258, 12517, 25033, 50066 Hz) and
//!   bit 7 mono (1) or stereo (0); Microwire commands: device `%10`, then mixer, bass, treble, master volume (0-40), right and
//!   left volume (0-20).
//! - [O] Hatari 2.6.1 (probes P29, P41): the register masks and read-backs of `registers`; the byte rate is `8 010 613 / N`
//!   Hz for N = 1280, 640, 320, 160 (stereo moves two bytes a frame) whatever the CPU clock; the DMA reads ahead: after `c`
//!   bytes were played the counter has fetched `8 + 2 * ceil(c / 2)` bytes; a frame ends when the fetch reaches its end (play
//!   once: the play bit clears and the counter returns to the base at once; loop: the fetch goes on from the base); each frame
//!   end is an event for the MFP's timer A input, and GPIP line 7 follows the playing state.
//!
//! Audio is a zero-order hold of the last frame played, taken at the machine's 44 100 Hz sample cycles.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// The DMA sound's clock: the NTSC machine's 8 010 613 Hz, whatever the CPU runs at ([O] P41).
const DMA_HZ: u128 = 8_010_613;
/// Samples kept undrained (two seconds).
const MAX_SAMPLES: usize = 2 * machine::AUDIO_SAMPLE_RATE as usize;
/// CPU clocks between the start and the first frame played ([O] P29: the counter's first steps; see `docs/hardware-facts.md`).
const START_LATENCY: u64 = 640;
/// Bytes the DMA fetches ahead of the sample being played.
const PREFETCH: u64 = 8;
/// CPU clocks in a scan line (a frame is a whole number of them in every sync mode).
const LINE_CLOCKS: u64 = 512;

/// What a frame end did, at CPU cycle `.0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundEvent {
    /// The fetch reached the end of the frame (a pulse for timer A).
    FrameEnd,
    /// Playing stopped (play once reached the end): GPIP line 7 rises.
    Stopped,
}

/// The LMC1992's settings as Microwire commands set them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lmc {
    pub mixer: u8,
    pub bass: u8,
    pub treble: u8,
    pub master: u8,
    pub right: u8,
    pub left: u8,
}

impl Default for Lmc {
    /// [D] the LMC1992/3 "powers up with flat tone and min volume/fader" (data sheet, features): tone 6 is flat, volume 0 is
    /// -80 dB and the faders 0 are -40 dB each. The input select at power-up is not in the sheet; [H?] the YM mixed in (1),
    /// what TOS sets.
    fn default() -> Self {
        Self {
            mixer: 1,
            bass: 6,
            treble: 6,
            master: 0,
            right: 0,
            left: 0,
        }
    }
}

impl Lmc {
    /// Applies a Microwire word: `data` shifted out through the bits `mask` has set (most significant first) must start with
    /// the address `%10` and end with a 3-bit function and 6 data bits; any number of don't-care bits may come between, so a
    /// stream of fewer than 11 bits or with another address changes nothing ([D] LMC1992/3 data sheet, "Serial Data Entry"
    /// and Table I, `docs/LMC1993.pdf`; [H] *Internals* Microwire table for the registers it is wired to).
    pub fn microwire(&mut self, data: u16, mask: u16) {
        let mut bits = 0u32;
        let mut count = 0;
        for i in (0..16).rev() {
            if mask >> i & 1 != 0 {
                bits = bits << 1 | u32::from(data >> i & 1);
                count += 1;
            }
        }
        if count < 11 || bits >> (count - 2) != 0b10 {
            return;
        }
        // The function select and the six data bits are the last nine; D5 of a fader and D5-D4 of a tone control are
        // don't-care bits ([D] Table I).
        let operand = (bits & 0x3F) as u8;
        match bits >> 6 & 7 {
            0 => self.mixer = operand & 3,
            1 => self.bass = (operand & 0xF).min(12),
            2 => self.treble = (operand & 0xF).min(12),
            3 => self.master = operand.min(40),
            4 => self.right = (operand & 0x1F).min(20),
            5 => self.left = (operand & 0x1F).min(20),
            _ => {}
        }
    }

    fn gain(db: i32) -> f32 {
        10f32.powf(db as f32 / 20.0)
    }

    /// The gains (left, right) of the DMA sound and of the YM: 2 dB a step of master and channel volume; [H?] mixer 0 plays
    /// the YM 12 dB down, 1 at full level and 2-3 not at all.
    pub fn gains(&self) -> [f32; 4] {
        let master = -2 * (40 - i32::from(self.master.min(40)));
        let left = master - 2 * (20 - i32::from(self.left.min(20)));
        let right = master - 2 * (20 - i32::from(self.right.min(20)));
        let ym = match self.mixer {
            0 => -12,
            1 => 0,
            _ => -200,
        };
        [
            Self::gain(left),
            Self::gain(right),
            Self::gain(left + ym),
            Self::gain(right + ym),
        ]
    }
}

/// The DMA sound as data (also its state).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DmaSound {
    base: u32,
    end: u32,
    mode: u8,
    looping: bool,
    playing: bool,
    /// The cycle playing started at, the sample frames played and the bytes played since.
    start: u64,
    frames: u64,
    stream: u64,
    /// The bytes fetched since the start, and the number the counter register shows (as of the last line start).
    fetched: u64,
    visible: u64,
    /// The frame last played.
    out: (i8, i8),
    pub lmc: Lmc,
    /// The machine's audio sample index and the cycle the sound has been advanced to.
    audio_index: u64,
    at: u64,
    /// Output samples: DMA left and right and the YM's gains, already scaled by the volume control.
    samples: VecDeque<[f32; 4]>,
    /// The tone controls' filters (the last samples only: not part of a state).
    #[serde(skip)]
    tone: crate::tone::Tone,
    /// The DAC's output filters (also the last few samples only).
    #[serde(skip)]
    dac: crate::dac::Dac,
}

impl DmaSound {
    /// Whether a state (untrusted) can have come from this device.
    pub fn is_consistent(&self) -> bool {
        self.base < 1 << 24
            && self.end < 1 << 24
            && self.base & 1 == 0
            && self.end & 1 == 0
            && self.mode & 0x70 == 0
            && self.samples.len() <= MAX_SAMPLES
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn stereo(&self) -> bool {
        self.mode & 0x80 == 0
    }

    /// [O] P29: the play bit stays set with an empty frame; [H?] then nothing is played.
    fn active(&self) -> bool {
        self.playing && self.len() > 0
    }

    /// The length of the frame in bytes (0 when the end is not past the base).
    fn len(&self) -> u64 {
        u64::from(self.end.saturating_sub(self.base))
    }

    /// CPU clocks times DMA clocks per frame: the cycle of the `n`th sample frame after the start.
    fn frame_cycle(&self, n: u64, clock_hz: u64) -> u64 {
        let div = 1280u128 >> (self.mode & 3);
        let after = (n as u128 * div * u128::from(clock_hz)).div_ceil(DMA_HZ);
        // A state is untrusted: the sum saturates instead of overflowing.
        self.start
            .saturating_add(START_LATENCY)
            .saturating_add(u64::try_from(after).unwrap_or(u64::MAX))
    }

    fn audio_cycle(&self, n: u64, clock_hz: u64) -> u64 {
        ((n + 1) as u128 * u128::from(clock_hz)).div_ceil(machine::AUDIO_SAMPLE_RATE as u128) as u64
    }

    /// The register byte at offset `off` from `$FF8900` (0 to 0x21), `None` for the bytes the Microwire code serves.
    pub fn read_byte(&self, off: usize) -> Option<u8> {
        let counter = if self.active() {
            self.base + (self.visible % self.len()) as u32
        } else {
            self.base
        };
        Some(match off {
            0x01 => u8::from(self.playing) | u8::from(self.looping) << 1,
            0x03 => (self.base >> 16) as u8,
            0x05 => (self.base >> 8) as u8,
            0x07 => self.base as u8,
            0x09 => (counter >> 16) as u8,
            0x0B => (counter >> 8) as u8,
            0x0D => counter as u8,
            0x0F => (self.end >> 16) as u8,
            0x11 => (self.end >> 8) as u8,
            0x13 => self.end as u8,
            0x21 => self.mode,
            // [O] P29: the even addresses read open ($FF) but for `$FF8900` itself, which reads 0.
            0x00 => 0,
            0x22.. => return None,
            _ => 0xFF,
        })
    }

    /// Whether writing the control register at `now` starts or stops playing.
    pub fn write_byte(&mut self, off: usize, v: u8, now: u64) {
        match off {
            0x01 => {
                self.looping = v & 2 != 0;
                match (self.playing, v & 1 != 0) {
                    (false, true) => self.start_playing(now),
                    (true, false) => self.playing = false,
                    _ => {}
                }
            }
            // [O] P29: the high byte keeps six bits, the low byte's bit 0 reads 0.
            0x03 => self.base = self.base & 0x00_FFFF | u32::from(v & 0x3F) << 16,
            0x05 => self.base = self.base & 0x3F_00FF | u32::from(v) << 8,
            0x07 => self.base = self.base & 0x3F_FF00 | u32::from(v & 0xFE),
            0x0F => self.end = self.end & 0x00_FFFF | u32::from(v & 0x3F) << 16,
            0x11 => self.end = self.end & 0x3F_00FF | u32::from(v) << 8,
            0x13 => self.end = self.end & 0x3F_FF00 | u32::from(v & 0xFE),
            // [O] P29: bits 7 and 3-0 are kept.
            0x21 => {
                self.mode = v & 0x8F;
                if self.playing {
                    // [H?] a change of rate while playing continues from the next frame.
                    self.start = now;
                    self.frames = 0;
                }
            }
            _ => {}
        }
    }

    fn start_playing(&mut self, now: u64) {
        self.playing = true;
        self.start = now;
        self.frames = 0;
        self.stream = 0;
        self.fetched = 0;
        self.visible = 0;
        if self.len() == 0 && !self.looping {
            // [O] P29: play once over an empty frame ends at once; in loop mode the play bit stays set.
            self.playing = false;
        }
    }

    /// The prefetch a start does at once; returns the events it makes.
    pub fn prefetch(&mut self, now: u64, events: &mut Vec<(u64, SoundEvent)>) {
        if self.active() && self.fetched == 0 {
            self.fetch_to(PREFETCH, now, events);
        }
    }

    fn fetch_to(&mut self, target: u64, at: u64, events: &mut Vec<(u64, SoundEvent)>) {
        while self.active() && self.fetched < target {
            self.fetched += 2;
            if self.fetched.is_multiple_of(self.len()) {
                events.push((at, SoundEvent::FrameEnd));
                if !self.looping {
                    self.playing = false;
                    self.out = (0, 0);
                    events.push((at, SoundEvent::Stopped));
                }
            }
        }
    }

    /// Advances to CPU cycle `cycle`, taking audio samples and playing frames in time order; `mem` reads a byte of memory.
    pub fn advance(
        &mut self,
        cycle: u64,
        clock_hz: u64,
        mem: &dyn Fn(u32) -> u8,
        events: &mut Vec<(u64, SoundEvent)>,
    ) {
        if cycle <= self.at {
            return;
        }
        loop {
            let audio = self.audio_cycle(self.audio_index, clock_hz);
            let frame = self
                .active()
                .then(|| self.frame_cycle(self.frames, clock_hz));
            // [O] P29: the counter register changes at the start of a scan line only.
            let line = self
                .active()
                .then(|| (self.at / LINE_CLOCKS + 1) * LINE_CLOCKS);
            let t = [Some(audio), frame, line]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(audio);
            if t > cycle {
                break;
            }
            if frame == Some(t) {
                self.play_frame(t, mem, events);
            } else if line == Some(t) {
                self.visible = self.fetched;
                self.at = t;
            } else {
                let [l, r, yl, yr] = self.lmc.gains();
                if self.samples.len() == MAX_SAMPLES {
                    self.samples.pop_front();
                }
                self.samples.push_back([
                    f32::from(self.out.0) / 128.0 * l,
                    f32::from(self.out.1) / 128.0 * r,
                    yl,
                    yr,
                ]);
                self.audio_index += 1;
            }
        }
        self.at = cycle;
    }

    fn play_frame(
        &mut self,
        at: u64,
        mem: &dyn Fn(u32) -> u8,
        events: &mut Vec<(u64, SoundEvent)>,
    ) {
        let len = self.len();
        let byte = |p: u64| mem(self.base.wrapping_add((p % len) as u32) & 0xFF_FFFF) as i8;
        self.out = if self.stereo() {
            (byte(self.stream), byte(self.stream + 1))
        } else {
            let b = byte(self.stream);
            (b, b)
        };
        self.stream += if self.stereo() { 2 } else { 1 };
        self.frames += 1;
        let target = PREFETCH + 2 * self.stream.div_ceil(2);
        self.fetch_to(target, at, events);
    }

    /// Passes interleaved stereo `samples` through the bass and treble controls ([D] LMC1992/3, see `tone.rs`).
    pub fn apply_tone(&mut self, samples: &mut [f32]) {
        self.tone.apply(self.lmc.bass, self.lmc.treble, samples);
    }

    /// The samples taken so far (DMA left, right, YM gains left, right), removed. The DMA samples have been through the DAC's
    /// output filters ([D] *STe manual* 2.10.1, see `dac.rs`), at the sample rate now selected.
    pub fn drain(&mut self) -> Vec<[f32; 4]> {
        let mut samples: Vec<[f32; 4]> = std::mem::take(&mut self.samples).into();
        for channel in 0..2 {
            let mut column: Vec<f32> = samples.iter().map(|s| s[channel]).collect();
            self.dac.apply(self.mode & 3, channel, &mut column);
            for (s, c) in samples.iter_mut().zip(column) {
                s[channel] = c;
            }
        }
        samples
    }
}
