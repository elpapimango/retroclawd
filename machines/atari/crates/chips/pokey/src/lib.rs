//! POKEY timers, audio, polynomial counters, keyboard scan and idle SIO output.
//! Source: Altirra Hardware Reference (2026-01-02), chapter 5 and section 14.4:
//! <https://www.virtualdub.org/downloads/Altirra%20Hardware%20Reference%20Manual.pdf>.
//! Timer counters retain their shared prescaler phase and underflow pipelines.
//! Sub-cycle high-pass delay, analogue saturation and external serial
//! clocks remain gaps.
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
    #[serde(skip)]
    serial: SerialState,
    #[serde(skip)]
    timing: PokeyTimingState,
}

/// Cycle history appended separately by the Machine's versioned state envelope.
/// The original `Pokey` serde field sequence remains unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PokeyTimingState {
    counters: [u8; 4],
    borrow: [u8; 4],
    stimer: u8,
    irq_pending: u8,
    irq_enable: [u8; 3],
    irq_line: [bool; 2],
    noise: [u8; 3],
    // The original serialized `timers` retain exact master-cycle residuals
    // until a migrated channel (or linked pair) completes its first period.
    legacy: [bool; 4],
    serial_clock: bool,
    resync: u8,
    two_tone_output: bool,
    slow_clock: u8,
    serial_delay: [bool; 2],
    clock_release: bool,
}

/// Additional serial State lives outside the original Chip encoding so old
/// Machine states retain their exact field layout.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerialState {
    output: bool,
    input: bool,
    transmit: u16,
    receive: u8,
    receive_shift: u8,
    receive_bits: u8,
    receive_delay: u32,
}

impl Default for SerialState {
    fn default() -> Self {
        Self {
            output: true,
            input: true,
            transmit: 0,
            receive: 0xff,
            receive_shift: 0,
            receive_bits: 0,
            receive_delay: 0,
        }
    }
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
            serial: SerialState::default(),
            timing: PokeyTimingState::default(),
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
            10 => !(self.poly17 >> 8) as u8,
            13 => self.serial.receive,
            14 => self.irq_status,
            15 => self.skstat,
            _ => 0xff,
        }
    }
    pub fn write(&mut self, offset: u8, value: u8) {
        let offset = offset & 15;
        match offset {
            9 => {
                self.timing.stimer = 3;
            }
            10 => self.skstat |= 0xe0,
            11 => {
                self.pot_counter = 0;
            }
            13 => self.serial_pending = Some(value),
            14 => {
                self.irq_status |= !value & !8;
                if !self.irq_status & value == 0 {
                    // IRQEN acknowledges the chip immediately. A CPU may
                    // already have sampled the old IRQ (AHR §5.7); retaining
                    // it here would incorrectly assert a second interrupt
                    // after a disable/re-enable acknowledgment.
                    self.timing.irq_line = [false; 2];
                }
            }
            15 => {
                // AHR §5.2 and Acid800 init/cancellation timing: the shared
                // prescalers leave reset after the SKCTL write clock. Noise
                // and the 1.79MHz timers do not wait for this release.
                if self.registers[15] & 3 == 0 && value & 3 != 0 {
                    self.timing.clock_release = true;
                } else if value & 3 == 0 {
                    self.timing.clock_release = false;
                }
                if value & 3 == 0 {
                    self.clock28 = 19;
                    self.clock114 = 78;
                    self.serial_pending = None;
                    self.serial_bits = 0;
                    self.serial.receive_bits = 0;
                    self.serial.receive_delay = 0;
                    self.skstat |= 2;
                    self.irq_status &= !8;
                }
                if value & 0x70 == 0 {
                    self.serial_phase = false;
                    self.timing.serial_clock = false;
                    self.timing.serial_delay = [false; 2];
                }
                if value & 8 == 0 {
                    self.timing.two_tone_output = false;
                    self.timing.resync = 0;
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
    /// Synchronized IRQ pin level; IRQST can change before this level changes.
    pub fn irq(&self) -> bool {
        self.timing.irq_line[1]
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
    pub fn save_timing(&self) -> PokeyTimingState {
        self.timing.clone()
    }
    pub fn restore_timing(&mut self, state: PokeyTimingState) -> Result<(), String> {
        Self::validate_timing(&state)?;
        self.timing = state;
        Ok(())
    }
    fn validate_timing(state: &PokeyTimingState) -> Result<(), String> {
        if state.stimer > 3
            || state.borrow.iter().any(|&delay| delay > 3)
            || state.irq_pending & !7 != 0
            || state.noise.iter().any(|&history| history > 15)
            || state.resync > 3
            || state.slow_clock > 3
        {
            return Err("invalid POKEY timing state".to_string());
        }
        Ok(())
    }
    /// Explicit migration for v1-v3 states, which did not record byte counters
    /// or pending underflows. Finish the stored period-level countdowns before
    /// switching to byte counters. Missing pipeline history is not invented.
    pub fn reconstruct_legacy_timing(&mut self) -> Result<(), String> {
        self.validate()?;
        let timing = PokeyTimingState {
            irq_enable: [self.registers[14]; 3],
            irq_line: [!self.irq_status & self.registers[14] != 0; 2],
            legacy: [true; 4],
            ..PokeyTimingState::default()
        };
        // Legacy short and long generators were independent. Select the one
        // that was visible, then resume the single physical shift register.
        if self.registers[8] & 0x80 != 0 {
            self.poly17 = (self.poly17 & 255) | (self.poly9 << 8);
        } else {
            // Undo eight shifts so the newly selected RANDOM taps expose the
            // same byte as the old low-byte read, without discarding LFSR phase.
            for _ in 0..8 {
                let bit = !(self.poly17 >> 16 ^ self.poly17 >> 4) & 1;
                self.poly17 = (self.poly17 << 1 & 0x1ffff) | bit;
            }
        }
        self.poly9 = self.poly17 >> 8;
        self.timing = timing;
        Ok(())
    }
    fn linked(&self, low: usize) -> bool {
        self.registers[8] & if low == 0 { 0x10 } else { 8 } != 0
    }
    fn reload(&mut self, channel: usize) {
        self.timing.counters[channel] = self.registers[channel * 2];
    }
    fn decrement(&mut self, channel: usize) {
        let old = self.timing.counters[channel];
        self.timing.counters[channel] = old.wrapping_sub(1);
        if old == 0 {
            self.timing.borrow[channel] = 3;
        }
    }
    fn legacy_clock(&mut self, channel: usize, running: bool) -> bool {
        let fast = channel.is_multiple_of(2)
            && self.registers[8] & if channel == 0 { 0x40 } else { 0x20 } != 0;
        if !running && !fast {
            return false;
        }
        self.timers[channel] = self.timers[channel].saturating_sub(1);
        if self.timers[channel] != 0 {
            return false;
        }
        self.signal([1, 2, 0, 4][channel]);
        self.timers[channel] = self.period(channel);
        let low = channel & !1;
        if self.linked(low) && self.timing.legacy[low + 1] {
            if channel != low {
                self.timing.legacy[low..=low + 1].fill(false);
                self.reload(low);
                self.reload(low + 1);
            }
        } else {
            self.timing.legacy[channel] = false;
            self.reload(channel);
        }
        true
    }
    fn noise_clock(&mut self, running: bool) {
        // AHR §5.5, §14.4 RANDOM and Appendix A: x^9+x^4+1 and
        // x^17+x^12+1, shifting right. RANDOM taps the eight stages below
        // the feedback stage; short mode bypasses the extra eight stages.
        // Each channel samples one cycle later than its predecessor. The
        // RANDOM bit is visible to the CPU before channel 1 receives it.
        let bits = [
            self.poly4 & 1,
            if running { self.poly5 & 1 } else { 1 },
            (self.poly17 >> 8) & 1,
        ];
        let feedback = if running {
            let shift = if self.registers[8] & 0x80 != 0 { 8 } else { 0 };
            (!(self.poly17 >> shift ^ self.poly17 >> (shift + 5))) & 1
        } else {
            0
        };
        self.poly17 = (self.poly17 >> 1) | (feedback << 16);
        self.poly9 = self.poly17 >> 8;
        if running {
            Self::lfsr(&mut self.poly4, 4, 1);
            Self::lfsr(&mut self.poly5, 5, 2);
        } else {
            self.poly4 >>= 1;
            self.poly5 >>= 1;
        }
        for (index, bit) in bits.into_iter().enumerate() {
            self.timing.noise[index] = (self.timing.noise[index] << 1 | bit as u8) & 15;
        }
    }
    fn timer_clock(&mut self, running: bool) -> [bool; 4] {
        // AHR §5.3 "Reload timing"/"STIMER preemption": reload at T+3;
        // fast 8-bit IRQST at T+N+8. Borrow propagates for three clocks,
        // IRQST follows by one. Linked high events lag low events by three.
        // Acid800 pokey_timertiming.s, two-tone refrequency: enabling at
        // T+42 must latch the unchanged T+44 expiration. AHR §5.7's four
        // cycles must not all be imposed before IRQST; CPU propagation is
        // separate. One previous enable sample gives the two-clock deadline.
        let enabled = self.timing.irq_enable[0] & self.registers[14];
        // AHR §5.2: prescalers pulse at 19/78, while IRQST asserts at
        // 24/83. Their pulse reaches the byte counter one clock later.
        let slow = running && self.timing.slow_clock & (1 << (self.registers[8] & 1)) != 0;
        self.timing.slow_clock = if running {
            u8::from(self.clock28 == 28) | (u8::from(self.clock114 == 114) << 1)
        } else {
            0
        };
        self.irq_status &= !(self.timing.irq_pending & enabled);
        self.timing.irq_pending = 0;
        self.timing.irq_enable.rotate_right(1);
        self.timing.irq_enable[0] = self.registers[14];
        let reset = self.timing.stimer == 1;
        self.timing.stimer = self.timing.stimer.saturating_sub(1);
        let resync = self.timing.resync & 1 != 0;
        self.timing.resync >>= 1;
        if reset {
            for channel in 0..4 {
                self.reload(channel);
            }
            self.timing.borrow = [0; 4];
            self.timing.legacy = [false; 4];
            self.timing.resync = 0;
            self.outputs = [true; 4];
            self.highpass = [true; 2];
            return [false; 4];
        }
        let mut fired = [false; 4];
        let mut legacy_fired = [false; 4];
        for pair in [0, 2] {
            if pair == 2 && self.registers[15] & 0x10 != 0 && self.serial.receive_bits == 0 {
                self.reload(pair);
                self.reload(pair + 1);
                self.timing.borrow[pair..=pair + 1].fill(0);
                for channel in pair..=pair + 1 {
                    if self.timing.legacy[channel] {
                        self.timers[channel] = self.period(channel);
                    }
                }
                continue;
            }
            let joined = self.linked(pair);
            let mut reloaded = [false; 2];
            if pair == 0 && resync {
                // AHR §5.6 "Two-tone resync timing": an extra reload two
                // clocks after the regular reload. Do not emit timer pulses.
                self.reload(0);
                self.reload(1);
                self.timing.borrow[..2].fill(0);
                self.timing.legacy[..2].fill(false);
                reloaded = [true; 2];
            }
            let legacy = [self.timing.legacy[pair], self.timing.legacy[pair + 1]];
            for channel in pair..=pair + 1 {
                if legacy[channel - pair] {
                    fired[channel] = self.legacy_clock(channel, running);
                    legacy_fired[channel] = fired[channel];
                    reloaded[channel - pair] = true;
                }
            }
            if joined && legacy[1] && fired[pair + 1] {
                reloaded = [true; 2];
            }
            for channel in pair..=pair + 1 {
                if reloaded[channel - pair] {
                    continue;
                }
                if self.timing.borrow[channel] != 0 {
                    self.timing.borrow[channel] -= 1;
                    if self.timing.borrow[channel] == 0 {
                        fired[channel] = true;
                        if joined && channel == pair + 1 {
                            // Both counters reload together, without another
                            // low-channel pulse (AHR §5.3 linked timers).
                            // AHR's N=0 reload-at-T+9 example conflicts with
                            // Acid800 1.2 pokey_timertiming.s's measured AUDF
                            // deadline T+N+9 (reload T+N+10). Use the latter,
                            // which also agrees with the high IRQ minus one.
                            self.reload(pair);
                            self.reload(pair + 1);
                            reloaded = [true; 2];
                        } else if !joined {
                            self.reload(channel);
                            reloaded[channel - pair] = true;
                        }
                    }
                }
            }
            for channel in pair..=pair + 1 {
                if reloaded[channel - pair] {
                    continue;
                }
                let clock = if channel == pair {
                    self.registers[8] & (0x40 >> (pair / 2)) != 0 || slow
                } else if joined {
                    fired[pair]
                } else {
                    slow
                };
                if clock && (self.timing.borrow[channel] == 0 || (joined && channel == pair)) {
                    self.decrement(channel);
                }
            }
        }
        for (channel, &fire) in fired.iter().enumerate() {
            if fire && !legacy_fired[channel] {
                self.timing.irq_pending |= [1, 2, 0, 4][channel];
            }
        }
        if self.registers[15] & 8 != 0
            && (fired[1] || (fired[0] && self.serial.output && self.registers[15] & 0x80 == 0))
        {
            // Timer 2 always resyncs; timer 1 only for a mark bit. Keep both
            // pending reloads when adjacent firings produce a double resync.
            self.timing.resync |= 2;
            self.timing.two_tone_output = !self.timing.two_tone_output;
        }
        fired
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
        if self.serial_bits > 0 {
            self.serial.transmit >>= 1;
            self.serial_bits -= 1;
        }
        if self.serial_bits == 0 {
            if let Some(byte) = self.serial_pending.take() {
                self.serial.transmit = (u16::from(byte) << 1) | 0x200;
                self.serial_bits = 10;
                self.irq_status |= 8;
                self.signal(0x10);
            } else {
                self.irq_status &= !8;
                return;
            }
        }
        self.serial.output = self.serial.transmit & 1 != 0;
    }
    pub fn serial_output(&self) -> bool {
        if self.registers[15] & 8 != 0 {
            self.timing.two_tone_output
        } else {
            self.serial.output && self.registers[15] & 0x80 == 0
        }
    }
    pub fn set_serial_input(&mut self, level: bool) {
        self.serial.input = level;
    }
    pub fn serial_state(&self) -> SerialState {
        self.serial.clone()
    }
    pub fn restore_serial(&mut self, state: SerialState) -> Result<(), String> {
        if state.transmit > 0x3ff || state.receive_bits > 10 || state.receive_delay > 14_942_208 {
            return Err("invalid POKEY serial state".to_string());
        }
        self.serial = state;
        Ok(())
    }
    fn receive_clock(&mut self) {
        self.skstat = (self.skstat & !0x10) | (u8::from(self.serial.input) << 4);
        if self.registers[15] & 3 == 0 || self.registers[15] & 0x10 == 0 {
            return;
        }
        if self.serial.receive_bits == 0 {
            if self.serial.input {
                return;
            }
            self.serial.receive_bits = 1;
            self.serial.receive_delay = self.period(3);
        }
        self.serial.receive_delay = self.serial.receive_delay.saturating_sub(1);
        if self.serial.receive_delay != 0 {
            return;
        }
        match self.serial.receive_bits {
            1 => {
                if self.serial.input {
                    self.serial.receive_bits = 0;
                    return;
                }
                self.serial.receive_shift = 0;
                self.skstat &= !2;
            }
            2..=9 => {
                self.serial.receive_shift |=
                    u8::from(self.serial.input) << (self.serial.receive_bits - 2);
            }
            10 => {
                self.serial.receive = self.serial.receive_shift;
                if !self.serial.input {
                    self.skstat &= !0x80;
                }
                if self.irq_status & 0x20 == 0 {
                    self.skstat &= !0x20;
                }
                self.signal(0x20);
                self.skstat |= 2;
                self.serial.receive_bits = 0;
                return;
            }
            _ => unreachable!(),
        }
        self.serial.receive_bits += 1;
        self.serial.receive_delay = self.period(3) * 2;
    }
    pub fn tick(&mut self) {
        // The measured 195/196-clock SEROUT load boundary in Acid800's
        // pokey_sertiming.s puts shifter clocking three master clocks after
        // the linked high timer reload, not at the audio/IRQST clock.
        let serial_clock = self.timing.serial_delay[1];
        self.timing.serial_delay[1] = self.timing.serial_delay[0];
        self.timing.serial_delay[0] = std::mem::take(&mut self.timing.serial_clock);
        if serial_clock && self.registers[15] & 3 != 0 {
            self.serial_clock();
        }
        self.receive_clock();
        let running = self.registers[15] & 3 != 0;
        self.noise_clock(running);
        if running && !std::mem::take(&mut self.timing.clock_release) {
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
        let fired = self.timer_clock(running);
        for (channel, &fire) in fired.iter().enumerate() {
            if !fire {
                continue;
            }
            let control = self.registers[channel * 2 + 1];
            let noise = |index: usize| self.timing.noise[index] & (1 << channel) != 0;
            if control & 0x80 != 0 || noise(1) {
                self.outputs[channel] = if control & 0x20 != 0 {
                    !self.outputs[channel]
                } else if control & 0x40 != 0 {
                    noise(0)
                } else {
                    noise(2)
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
        let mode = (self.registers[15] >> 4) & 7;
        if self.registers[15] & 3 != 0
            && ((fired[3] && (2..=5).contains(&mode)) || (fired[1] && mode >= 6))
        {
            self.timing.serial_clock = true;
        }
        // AHR §5.7: IRQST and the CPU-visible IRQ are not simultaneous.
        // Include the synchronizer history for all sources, including SEROC
        // enabled while idle. The CPU's own polling adds its separate latency.
        self.timing.irq_line[1] = self.timing.irq_line[0];
        self.timing.irq_line[0] = !self.irq_status & self.registers[14] != 0;
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
        Self::validate_timing(&self.timing)?;
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
    fn ticks(p: &mut Pokey, count: usize) {
        for _ in 0..count {
            p.tick();
        }
    }
    fn timer(control: u8, low: u8, high: u8) -> Pokey {
        let mut p = Pokey::new();
        p.write(0, low);
        p.write(2, high);
        p.write(8, control);
        p.write(14, 3);
        p.write(15, 3);
        // Finish the SKCTL write clock before measuring from STIMER.
        p.tick();
        p.write(9, 0);
        p
    }
    #[test]
    fn short_random_uses_the_documented_polynomial() {
        let mut p = Pokey::new();
        p.write(8, 0x80);
        p.write(15, 3);
        ticks(&mut p, 4);
        assert_eq!(p.read(10), 0x1f);
        ticks(&mut p, 110);
        assert_eq!(p.read(10), 0x95);
    }
    #[test]
    fn random_shifts_to_reset_instead_of_changing_at_the_skctl_write() {
        let mut p = Pokey::new();
        p.write(8, 0x80);
        p.write(15, 3);
        ticks(&mut p, 114);
        let random = p.read(10);
        p.write(15, 0);
        assert_eq!(p.read(10), random);
        ticks(&mut p, 2);
        assert_eq!(p.read(10), (random >> 2) | 0x80);
        ticks(&mut p, 16);
        assert_eq!(p.read(10), 0xff);
    }
    #[test]
    fn fast_timer_stimer_has_reload_and_irq_pipelines() {
        for frequency in [0, 5, 16, 255] {
            let mut p = timer(0x40, frequency, 0);
            ticks(&mut p, usize::from(frequency) + 7);
            assert_eq!(p.read(14) & 1, 1);
            p.tick();
            assert_eq!(p.read(14) & 1, 0);
            p.write(14, 0);
            p.write(14, 1);
            ticks(&mut p, usize::from(frequency) + 3);
            assert_eq!(p.read(14) & 1, 1);
            p.tick();
            assert_eq!(p.read(14) & 1, 0);
        }
    }
    #[test]
    fn linked_low_wraps_without_reloading_and_high_fires_three_cycles_later() {
        let mut p = timer(0x50, 0, 1);
        let mut events = Vec::new();
        for cycle in 1..=530 {
            p.tick();
            let mask = !p.read(14) & 3;
            if mask != 0 {
                events.push((cycle, mask));
                p.write(14, 0);
                p.write(14, 3);
            }
        }
        assert_eq!(
            events,
            [(8, 1), (264, 1), (267, 2), (271, 1), (527, 1), (530, 2)]
        );
    }
    #[test]
    fn stimer_preempts_only_before_the_underflow_pipeline_commits() {
        for (write_cycle, expected) in [(4, 1), (5, 0)] {
            let mut p = timer(0x40, 0, 0);
            ticks(&mut p, write_cycle);
            p.write(9, 0);
            ticks(&mut p, 8 - write_cycle);
            assert_eq!(p.read(14) & 1, expected);
        }
    }
    #[test]
    fn slow_timers_keep_shared_clock_phase_across_stimer() {
        for (control, edge) in [(0, 19), (1, 78)] {
            let mut p = timer(control, 0, 0);
            ticks(&mut p, 10);
            p.write(9, 0);
            ticks(&mut p, edge - 6);
            assert_eq!(p.read(14) & 3, 3);
            p.tick();
            assert_eq!(p.read(14) & 3, 0);
        }
    }
    #[test]
    fn initialization_release_preserves_fast_clocks_and_is_saved() {
        let mut p = Pokey::new();
        p.write(8, 0x40);
        p.write(15, 3);
        let mut restored = p.clone();
        restored.timing = PokeyTimingState::default();
        restored.restore_timing(p.save_timing()).unwrap();
        p.tick();
        restored.tick();
        assert_eq!(p.save_timing(), restored.save_timing());
        assert_eq!((p.clock28, p.clock114), (19, 78));
        assert_ne!(p.poly17, 0);
        assert_eq!(p.timing.borrow[0], 3);
        // Rewriting an already running SKCTL does not rephase either clock.
        p.write(15, 3);
        p.tick();
        assert_eq!((p.clock28, p.clock114), (18, 77));
    }
    #[test]
    fn two_tone_slow_timer_resync_preserves_only_the_next_fast_expiration() {
        for (frequency, expected) in [(98, 0), (99, 1), (100, 1)] {
            let mut p = Pokey::new();
            p.write(8, 0x40);
            p.write(0, frequency);
            p.write(2, 3);
            p.write(15, 0);
            ticks(&mut p, 114);
            // Actual cancellation test: SKCTL, then STIMER four clocks later.
            p.write(15, 0x8b);
            ticks(&mut p, 4);
            p.write(9, 0);
            ticks(&mut p, 6);
            p.write(14, 0);
            ticks(&mut p, 6);
            p.write(14, 3);
            ticks(&mut p, 104);
            assert_eq!(p.read(14) & 3, expected, "AUDF1={frequency}");
        }
    }
    #[test]
    fn frequency_writes_observe_the_reload_deadline_not_the_irq_deadline() {
        // AHR §5.3: N=16 reloads at T+23; linked N=13 also reloads
        // at T+23. A write there is too late for the next period.
        for (control, frequency, irq_cycle) in [(0x40, 16, 44), (0x50, 13, 41)] {
            for (write_cycle, extra) in [(22, 2), (23, 0)] {
                let mut p = timer(control, frequency, 0);
                ticks(&mut p, write_cycle);
                p.write(0, frequency + 2);
                // Acknowledge the first low-channel IRQ.
                ticks(&mut p, 30 - write_cycle);
                p.write(14, 0);
                p.write(14, 1);
                ticks(&mut p, irq_cycle + extra - 31);
                assert_eq!(p.read(14) & 1, 1);
                p.tick();
                assert_eq!(p.read(14) & 1, 0);
            }
        }
    }
    #[test]
    fn two_tone_reprogramming_deadlines_include_late_irqen_enable() {
        for frequency in [8, 10] {
            let first = usize::from(frequency) + 8;
            let second = first + usize::from(frequency) + 6;
            let third = second + usize::from(frequency) + 6;
            for (write_cycle, enable_cycle, read_cycle, changed) in [
                (first, second - 4, second, true),
                (first + 1, second - 4, second, false),
                (second, third - 2, third + 2, true),
                (second + 1, third - 2, third + 2, false),
            ] {
                let mut p = timer(0x40, frequency, 255);
                p.write(15, 0x0b);
                p.write(14, 0);
                ticks(&mut p, write_cycle);
                p.write(0, 255);
                ticks(&mut p, enable_cycle - write_cycle);
                p.write(14, 1);
                ticks(&mut p, read_cycle - enable_cycle);
                assert_eq!(
                    p.read(14) & 1 != 0,
                    changed,
                    "frequency={frequency}, write={write_cycle}, enable={enable_cycle}"
                );
            }
        }
    }
    #[test]
    fn irq_enable_must_precede_the_timer_irq_by_two_cycles() {
        for (write_cycle, expected) in [(6, 0), (7, 1)] {
            let mut p = timer(0x40, 0, 0);
            p.write(14, 0);
            ticks(&mut p, write_cycle);
            p.write(14, 1);
            ticks(&mut p, 8 - write_cycle);
            assert_eq!(p.read(14) & 1, expected);
        }
    }
    #[test]
    fn two_tone_resync_adds_two_cycles_only_after_the_first_period() {
        for frequency in [0, 8, 10] {
            let mut p = timer(0x40, frequency, 255);
            p.write(15, 0x0b);
            let mut events = Vec::new();
            for cycle in 1..=60 {
                p.tick();
                if p.read(14) & 1 == 0 {
                    events.push(cycle);
                    p.write(14, 0);
                    p.write(14, 1);
                }
            }
            let first = usize::from(frequency) + 8;
            let period = usize::from(frequency) + 6;
            assert_eq!(&events[..3], &[first, first + period, first + period * 2]);
        }
    }
    #[test]
    fn linked_two_tone_force_break_resyncs_only_on_the_high_timer() {
        let mut p = timer(0x50, 0, 0);
        p.write(15, 0x8b);
        let mut events = Vec::new();
        for cycle in 1..=20 {
            p.tick();
            let bits = !p.read(14) & 3;
            if bits != 0 {
                events.push((cycle, bits));
                p.write(14, 0);
                p.write(14, 3);
            }
        }
        assert_eq!(events, [(8, 1), (11, 2), (17, 1), (20, 2)]);
    }
    #[test]
    fn two_tone_allows_the_next_clock_but_preempts_the_second_clock() {
        for delay in [2, 3] {
            let mut p = timer(0x40, 8, 9);
            p.write(15, 0x0b);
            ticks(&mut p, 4);
            p.timing.stimer = 0;
            p.timing.borrow = [1, delay, 0, 0];
            p.tick();
            assert!(p.serial_output());
            p.tick();
            assert_eq!(p.timing.resync, if delay == 2 { 3 } else { 1 });
            p.tick();
            assert_eq!(!p.read(14) & 2 != 0, delay == 2);
            let saved = p.save_timing();
            let mut restored = p.clone();
            restored.restore_timing(saved).unwrap();
            for _ in 0..20 {
                p.tick();
                restored.tick();
                assert_eq!(p.save_timing(), restored.save_timing());
                assert_eq!(p.serial_output(), restored.serial_output());
                assert_eq!(p.read(14), restored.read(14));
            }
        }
    }
    #[test]
    fn irq_pin_is_delayed_from_status_and_from_enabling_serial_complete() {
        let mut p = timer(0x40, 0, 0);
        ticks(&mut p, 8);
        assert_eq!(p.read(14) & 1, 0);
        assert!(!p.irq());
        p.tick();
        assert!(p.irq());
        p.write(14, 0);
        // Acknowledgment clears the pin, not a previously sampled CPU IRQ.
        assert!(!p.irq());
        p.write(14, 8);
        assert!(!p.irq());
        p.tick();
        assert!(!p.irq());
        p.tick();
        assert!(p.irq());
    }
    #[test]
    fn long_random_and_audio_noise_share_the_same_shift_register() {
        let mut p = Pokey::new();
        p.write(15, 3);
        ticks(&mut p, 114);
        assert_eq!(p.read(10), 0x08);
        let last = p.read(10);
        p.tick();
        assert_eq!(p.read(10) & 0x7f, last >> 1);
        assert_eq!(p.timing.noise[2] & 1, (!last & 1));
        // Switching lengths does not reset RANDOM or select another counter.
        p.write(8, 0x80);
        let last = p.read(10);
        assert_eq!(p.read(10), last);
        p.tick();
        assert_eq!(p.read(10) & 0x7f, last >> 1);
    }
    #[test]
    fn timing_extension_restores_pending_reload_borrow_and_noise_history() {
        for cycle in 0..32 {
            let mut p = timer(0x50, 3, 0);
            ticks(&mut p, cycle);
            let state = p.save_timing();
            let mut restored = p.clone();
            restored.timing = PokeyTimingState::default();
            restored.restore_timing(state).unwrap();
            for _ in 0..300 {
                p.tick();
                restored.tick();
                assert_eq!(restored.save_timing(), p.save_timing());
                assert_eq!(restored.read(10), p.read(10));
                assert_eq!(restored.read(14), p.read(14));
                assert_eq!(restored.irq(), p.irq());
                assert_eq!(restored.output(), p.output());
            }
        }
        let mut p = timer(0x40, 0, 0);
        let before = p.save_timing();
        let mut invalid = before.clone();
        invalid.borrow[0] = 4;
        assert!(p.restore_timing(invalid).is_err());
        assert_eq!(p.save_timing(), before);
        let mut invalid = before.clone();
        invalid.irq_pending = 8;
        assert!(p.restore_timing(invalid).is_err());
        assert_eq!(p.save_timing(), before);
        for resync in [false, true] {
            let mut invalid = before.clone();
            if resync {
                invalid.resync = 4;
            } else {
                invalid.slow_clock = 4;
            }
            assert!(p.restore_timing(invalid).is_err());
            assert_eq!(p.save_timing(), before);
        }
    }
    #[test]
    fn legacy_migration_preserves_visible_random_and_long_residual_countdowns() {
        let mut p = Pokey::new();
        p.registers[15] = 3;
        p.poly17 = 0x12573;
        let expected = !(p.poly17 as u8);
        p.reconstruct_legacy_timing().unwrap();
        assert_eq!(p.read(10), expected);
        p.registers[8] = 0x40;
        p.timers[0] = 65536;
        p.write(14, 1);
        p.reconstruct_legacy_timing().unwrap();
        ticks(&mut p, 65535);
        assert_eq!(p.read(14) & 1, 1);
        p.tick();
        assert_eq!(p.read(14) & 1, 0);
        let mut restored = p.clone();
        restored.restore_timing(p.save_timing()).unwrap();
        ticks(&mut p, 100);
        ticks(&mut restored, 100);
        assert_eq!(restored.save_timing(), p.save_timing());
    }
    #[test]
    fn legacy_continuation_is_bounded_persistent_and_superseded_by_stimer() {
        let mut p = Pokey::new();
        p.registers[15] = 3;
        p.registers[8] = 0x50;
        p.timers = [7_471_104; 4];
        p.reconstruct_legacy_timing().unwrap();
        ticks(&mut p, 17);
        assert_eq!(p.timers, [7_471_087; 4]);
        let mut restored = p.clone();
        restored.timing = PokeyTimingState::default();
        restored.restore_timing(p.save_timing()).unwrap();
        for _ in 0..20 {
            p.tick();
            restored.tick();
            assert_eq!(restored.save_timing(), p.save_timing());
            assert_eq!(restored.timers, p.timers);
        }
        p.write(9, 0);
        ticks(&mut p, 3);
        assert_eq!(p.timing.legacy, [false; 4]);
        assert_eq!(p.timing.counters, [0; 4]);
        let mut invalid = p.clone();
        invalid.timers[0] = 7_471_105;
        let before = invalid.save_timing();
        assert!(invalid.reconstruct_legacy_timing().is_err());
        assert_eq!(invalid.save_timing(), before);
    }
    fn serial() -> Pokey {
        let mut p = Pokey::new();
        p.write(4, 40);
        p.write(6, 0);
        p.write(8, 0x28);
        p.write(15, 0x13);
        p.write(9, 0);
        p.write(14, 0x20);
        p
    }
    fn receive(p: &mut Pokey, byte: u8, stop: bool) {
        for bit in std::iter::once(false)
            .chain((0..8).map(|bit| byte & (1 << bit) != 0))
            .chain(std::iter::once(stop))
        {
            p.set_serial_input(bit);
            for _ in 0..94 {
                p.tick();
            }
        }
        p.set_serial_input(true);
    }
    #[test]
    fn serial_receive_latches_only_complete_bytes_and_requires_irq_acknowledgment() {
        let mut p = serial();
        receive(&mut p, 0xa5, true);
        assert_eq!(p.read(13), 0xa5);
        assert!(p.irq());
        assert_eq!(p.read(15) & 0xa0, 0xa0);
        receive(&mut p, 0x42, true);
        assert_eq!(p.read(13), 0x42);
        assert_eq!(p.read(15) & 0x20, 0);
        p.write(14, 0);
        p.write(14, 0x20);
        p.write(10, 0);
        receive(&mut p, 0x55, false);
        assert_eq!(p.read(13), 0x55);
        assert_eq!(p.read(15) & 0x80, 0);
        assert_eq!(p.read(15) & 0x20, 0x20);
    }
    #[test]
    fn serial_output_sends_lsb_first_with_start_stop_and_ready_complete_irqs() {
        let mut p = serial();
        p.write(15, 0x23);
        p.write(14, 0x18);
        p.write(13, 0xa5);
        for _ in 0..52 {
            p.tick();
        }
        assert!(p.serial_output());
        assert_eq!(p.read(14) & 8, 0);
        let pending = p.save_timing();
        let mut restored = p.clone();
        restored.timing = PokeyTimingState::default();
        restored.restore_timing(pending).unwrap();
        p.tick();
        restored.tick();
        assert_eq!(p.save_timing(), restored.save_timing());
        assert_eq!(p.serial_output(), restored.serial_output());
        assert!(!p.serial_output());
        assert_eq!(p.read(14) & 0x18, 8);
        for bit in (0..8).map(|bit| 0xa5 & (1 << bit) != 0).chain([true]) {
            for _ in 0..94 {
                p.tick();
            }
            assert_eq!(p.serial_output(), bit);
        }
        for _ in 0..94 {
            p.tick();
        }
        assert_eq!(p.read(14) & 8, 0);
    }
    #[test]
    fn serial_clock_load_matches_the_measured_stimer_boundary() {
        let mut p = Pokey::new();
        p.write(8, 0x78);
        p.write(0, 221);
        p.write(15, 0x63);
        p.write(9, 0);
        ticks(&mut p, 6);
        p.write(15, 0);
        ticks(&mut p, 6);
        p.write(15, 0x63);
        ticks(&mut p, 4);
        p.write(13, 0x63);
        // Acid800 pokey_sertiming.s: 195/196 CPU clocks followed by the
        // four-clock read, with 18 intervening ANTIC refresh clocks.
        ticks(&mut p, 195 + 4 + 18);
        assert_eq!(p.read(14) & 8, 0);
        p.tick();
        assert_eq!(p.read(14) & 8, 8);
    }
    #[test]
    fn asynchronous_receive_holds_timers_until_a_start_bit() {
        let mut p = serial();
        p.write(14, 4);
        for _ in 0..2000 {
            p.tick();
        }
        assert!(!p.irq());
        p.set_serial_input(false);
        for _ in 0..49 {
            p.tick();
        }
        assert!(p.irq());
        p.write(15, 0);
        p.set_serial_input(true);
        p.write(14, 0);
        p.write(14, 4);
        p.write(15, 0x13);
        for _ in 0..2000 {
            p.tick();
        }
        assert!(!p.irq());
        p.write(15, 3);
        for _ in 0..49 {
            p.tick();
        }
        assert!(p.irq());
    }
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
        p.tick();
        p.write(9, 0);
        p.write(14, 1);
        for _ in 0..2038 {
            p.tick();
        }
        assert!(!p.irq());
        let before = p.output();
        p.tick();
        assert_ne!(p.output(), before);
        assert!(!p.irq());
        p.tick();
        assert_eq!(p.read(14) & 1, 0);
        assert!(!p.irq());
        p.tick();
        assert!(p.irq());
        assert!(p.validate().is_ok());
    }
}
