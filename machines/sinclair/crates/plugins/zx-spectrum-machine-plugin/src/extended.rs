//! Keys typed in the Spectrum's extended mode (`~ | \ { } [ ]` and the copyright sign).
//!
//! The 48K keyboard types these with a sequence, not a chord: Caps Shift and Symbol Shift together switch the ROM to
//! extended mode, and the next Symbol Shift + key types the character. The ROM looks at the keyboard once a frame and
//! the shifts must be let go before the key, so each press here runs a small state machine in machine time (T-states
//! from `step`), never wall-clock time.

use std::collections::VecDeque;

use zx_spectrum::ZxSpectrumKey;

/// How long each step of the sequence lasts: two frames of the ROM's keyboard scan.
pub(crate) const STEP_TSTATES: u32 = 2 * 69_888;

/// What the sequence presses and releases; the machine implements it.
pub(crate) trait KeySink {
    fn press(&mut self, key: ZxSpectrumKey);
    fn release(&mut self, key: ZxSpectrumKey);
}

impl KeySink for zx_spectrum::ZxSpectrum {
    fn press(&mut self, key: ZxSpectrumKey) {
        self.press_key(key);
    }
    fn release(&mut self, key: ZxSpectrumKey) {
        self.release_key(key);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Nothing is being typed.
    Idle,
    /// Caps Shift + Symbol Shift are down (extended mode is switched on when they are let go).
    Mode,
    /// Both shifts are up; the ROM has seen extended mode.
    Gap,
    /// Symbol Shift and the key are down (while the host holds the key, at least one step).
    Key,
    /// Everything is up again; the next queued key may start.
    Rest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    key: ZxSpectrumKey,
    /// The host still holds the key.
    down: bool,
}

#[derive(Debug)]
pub(crate) struct ExtendedKeys {
    phase: Phase,
    elapsed: u32,
    current: Option<Entry>,
    waiting: VecDeque<Entry>,
}

impl Default for ExtendedKeys {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            elapsed: 0,
            current: None,
            waiting: VecDeque::new(),
        }
    }
}

impl ExtendedKeys {
    /// The host pressed an extended-mode key. A key already typing or waiting (auto-repeat) is not queued twice.
    pub(crate) fn press(&mut self, sink: &mut impl KeySink, key: ZxSpectrumKey) {
        if self.current.is_some_and(|c| c.key == key) || self.waiting.iter().any(|e| e.key == key) {
            self.mark(key, true);
            return;
        }
        let entry = Entry { key, down: true };
        if self.phase == Phase::Idle {
            self.start(sink, entry);
        } else {
            self.waiting.push_back(entry);
        }
    }

    /// The host let go of an extended-mode key; a tap still types the whole character.
    pub(crate) fn release(&mut self, sink: &mut impl KeySink, key: ZxSpectrumKey) {
        self.mark(key, false);
        if self.phase == Phase::Key && self.elapsed >= STEP_TSTATES {
            self.finish_key(sink);
        }
    }

    /// Whether a sequence is under way (the machine's own shifts are being driven).
    pub(crate) fn busy(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// Whether `key` is the one being typed now (its release must reach the sequence at once, not wait in line).
    pub(crate) fn is_current(&self, key: ZxSpectrumKey) -> bool {
        self.current.is_some_and(|e| e.key == key)
    }

    /// Lets go of everything and forgets the queue (a reset).
    pub(crate) fn clear(&mut self, sink: &mut impl KeySink) {
        if matches!(self.phase, Phase::Mode | Phase::Key) {
            for key in [ZxSpectrumKey::CapsShift, ZxSpectrumKey::SymShift] {
                sink.release(key);
            }
        }
        if let (Phase::Key, Some(entry)) = (self.phase, self.current) {
            sink.release(entry.key);
        }
        *self = Self::default();
    }

    /// Time has passed: `tstates` of machine time.
    pub(crate) fn advance(&mut self, sink: &mut impl KeySink, tstates: u32) {
        if self.phase == Phase::Idle {
            return;
        }
        self.elapsed = self.elapsed.saturating_add(tstates);
        if self.elapsed < STEP_TSTATES {
            return;
        }
        match self.phase {
            Phase::Mode => {
                sink.release(ZxSpectrumKey::CapsShift);
                sink.release(ZxSpectrumKey::SymShift);
                self.next(Phase::Gap);
            }
            Phase::Gap => {
                if let Some(entry) = self.current {
                    sink.press(ZxSpectrumKey::SymShift);
                    sink.press(entry.key);
                }
                self.next(Phase::Key);
            }
            Phase::Key => {
                if self.current.is_some_and(|e| !e.down) {
                    self.finish_key(sink);
                }
            }
            Phase::Rest => match self.waiting.pop_front() {
                Some(entry) => self.start(sink, entry),
                None => {
                    self.phase = Phase::Idle;
                    self.current = None;
                    self.elapsed = 0;
                }
            },
            Phase::Idle => {}
        }
    }

    fn mark(&mut self, key: ZxSpectrumKey, down: bool) {
        if let Some(entry) = self.current.as_mut().filter(|e| e.key == key) {
            entry.down = down;
        }
        for entry in self.waiting.iter_mut().filter(|e| e.key == key) {
            entry.down = down;
        }
    }

    fn start(&mut self, sink: &mut impl KeySink, entry: Entry) {
        self.current = Some(entry);
        sink.press(ZxSpectrumKey::CapsShift);
        sink.press(ZxSpectrumKey::SymShift);
        self.next(Phase::Mode);
    }

    fn finish_key(&mut self, sink: &mut impl KeySink) {
        if let Some(entry) = self.current {
            sink.release(entry.key);
        }
        sink.release(ZxSpectrumKey::SymShift);
        self.next(Phase::Rest);
    }

    fn next(&mut self, phase: Phase) {
        self.phase = phase;
        self.elapsed = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ZxSpectrumKey::{CapsShift, D, SymShift, U};

    /// Records the key state as the sequence drives it.
    #[derive(Default)]
    struct Pad {
        down: Vec<ZxSpectrumKey>,
        log: Vec<(bool, ZxSpectrumKey)>,
    }

    impl KeySink for Pad {
        fn press(&mut self, key: ZxSpectrumKey) {
            self.down.push(key);
            self.log.push((true, key));
        }
        fn release(&mut self, key: ZxSpectrumKey) {
            self.down.retain(|k| *k != key);
            self.log.push((false, key));
        }
    }

    fn run(seq: &mut ExtendedKeys, pad: &mut Pad, steps: u32) {
        for _ in 0..steps {
            seq.advance(pad, STEP_TSTATES);
        }
    }

    #[test]
    fn a_tap_switches_to_extended_mode_lets_the_shifts_go_and_then_types_the_key() {
        let (mut seq, mut pad) = (ExtendedKeys::default(), Pad::default());
        seq.press(&mut pad, D);
        seq.release(&mut pad, D); // a tap: up before anything has happened
        assert_eq!(
            pad.down,
            [CapsShift, SymShift],
            "extended mode is switched on first"
        );
        run(&mut seq, &mut pad, 1);
        assert!(pad.down.is_empty(), "the shifts are let go before the key");
        run(&mut seq, &mut pad, 1);
        assert_eq!(pad.down, [SymShift, D]);
        run(&mut seq, &mut pad, 1);
        assert!(
            pad.down.is_empty(),
            "the tap is released after its minimum hold"
        );
        run(&mut seq, &mut pad, 1);
        assert!(!seq.busy());
    }

    #[test]
    fn a_held_key_stays_down_until_the_host_lets_go() {
        let (mut seq, mut pad) = (ExtendedKeys::default(), Pad::default());
        seq.press(&mut pad, D);
        run(&mut seq, &mut pad, 2);
        run(&mut seq, &mut pad, 5);
        assert_eq!(pad.down, [SymShift, D]);
        seq.release(&mut pad, D);
        assert!(
            pad.down.is_empty(),
            "released at once once the minimum hold is over"
        );
    }

    #[test]
    fn keys_pressed_meanwhile_are_typed_in_order_with_a_rest_between() {
        let (mut seq, mut pad) = (ExtendedKeys::default(), Pad::default());
        seq.press(&mut pad, D);
        seq.release(&mut pad, D);
        seq.press(&mut pad, U);
        seq.press(&mut pad, U); // auto-repeat of a queued key is not queued twice
        seq.release(&mut pad, U);
        run(&mut seq, &mut pad, 4); // mode, gap, key, rest: D is done and U has started
        assert_eq!(pad.down, [CapsShift, SymShift]);
        run(&mut seq, &mut pad, 6);
        let typed: Vec<_> = pad
            .log
            .iter()
            .filter(|(down, key)| *down && matches!(key, D | U))
            .map(|(_, key)| *key)
            .collect();
        assert_eq!(typed, [D, U]);
        assert!(!seq.busy() && pad.down.is_empty());
    }

    #[test]
    fn a_reset_lets_go_of_everything() {
        let (mut seq, mut pad) = (ExtendedKeys::default(), Pad::default());
        seq.press(&mut pad, D);
        run(&mut seq, &mut pad, 2);
        seq.clear(&mut pad);
        assert!(pad.down.is_empty() && !seq.busy());
    }
}
