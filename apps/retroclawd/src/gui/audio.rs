use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use bevy::asset::Asset;
use bevy::audio::{
    AudioPlayer, ChannelCount, Decodable, PlaybackSettings, Sample, SampleRate, Source,
};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use crate::emu_thread::{AudioQueue, RequestedCount, UnderrunCount};

/// Mute and volume, shared between the hotkey system (writer) and the audio
/// backend's real-time callback thread (reader) -- plain relaxed atomics, no
/// lock on the callback path. Volume is a percentage (0-100) applied on top
/// of the emulator's own mix level; mute is separate so un-muting restores
/// the volume the player had set.
#[derive(Debug)]
pub(crate) struct AudioControl {
    muted: AtomicBool,
    volume_percent: AtomicU32,
}

impl Default for AudioControl {
    fn default() -> Self {
        Self {
            muted: AtomicBool::new(false),
            volume_percent: AtomicU32::new(100),
        }
    }
}

impl AudioControl {
    /// Volume step for `volume_up` / `volume_down`.
    pub(crate) const STEP: i32 = 10;

    /// Flips mute; returns whether audio is now muted.
    pub(crate) fn toggle_mute(&self) -> bool {
        !self.muted.fetch_xor(true, Ordering::Relaxed)
    }

    pub(crate) fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub(crate) fn volume_percent(&self) -> u32 {
        self.volume_percent.load(Ordering::Relaxed)
    }

    /// Moves the volume by `delta` percentage points (clamped to 0-100);
    /// returns the new volume. Raising the volume also un-mutes, like
    /// every volume key on every device.
    pub(crate) fn adjust_volume(&self, delta: i32) -> u32 {
        let new = (self.volume_percent() as i32 + delta).clamp(0, 100) as u32;
        self.volume_percent.store(new, Ordering::Relaxed);
        if delta > 0 {
            self.muted.store(false, Ordering::Relaxed);
        }
        new
    }

    /// The gain the decoder should be at right now (0.0-1.0).
    pub(crate) fn target_gain(&self) -> f32 {
        if self.is_muted() {
            0.0
        } else {
            self.volume_percent() as f32 / 100.0
        }
    }
}

/// How far the decoder's gain may move per output sample: a full 0-to-1
/// sweep takes ~512 samples (~10 ms), fast enough to feel instant and slow
/// enough that muting mid-waveform doesn't click.
const GAIN_RAMP_PER_SAMPLE: f32 = 1.0 / 512.0;

/// A `Decodable` asset type whose "decoding" is really just draining a
/// live, ever-growing `AudioQueue` -- see `Decodable` and `Source`'s impls
/// below. There's exactly one of these per run (`setup_audio` creates and
/// plays it once), not a real per-file asset; it exists at all because
/// Bevy's audio playback API is built around `Handle<Source>` +
/// `AudioPlayer`, and `Decodable`/`add_audio_source` is the documented
/// escape hatch for streaming synthesized audio instead of a decoded file
/// (see `bevy_audio::audio_source::Decodable`'s own doc comment).
///
/// `output_rate` is the real audio output device's own configured sample
/// rate (queried via `cpal` in `setup_audio`). This exists because of a
/// real, confirmed bug: Bevy's `Decodable`/`add_audio_source` escape hatch
/// does NOT resample a custom source to the device's real rate the way
/// normal file-decoded playback does (rodio's own `Mixer::add` wraps every
/// appended source in a `UniformSourceIterator` that resamples to the
/// mixer's fixed rate -- confirmed by reading rodio's `mixer.rs` -- but
/// Bevy's `sink.append(decoder...)` path for a custom `Decodable` doesn't
/// go through that same conversion). Diagnosed live: with a 44100Hz-
/// declared source played on a real 48000Hz-configured device, the
/// backend pulled samples from `EmulatorAudioDecoder` at ~48000/sec (not
/// 44100/sec) with zero conversion -- every real second was ~8% short of
/// actual generated audio, heard as continuous skipping, not an
/// occasional glitch (diagnostic log evidence: `requested/s` tracked the
/// real device rate, not `machine::AUDIO_SAMPLE_RATE`). Since Bevy/rodio
/// won't do this conversion for us on this code path, `EmulatorAudioDecoder`
/// does its own linear-interpolation resampling instead.
#[derive(Asset, TypePath, Clone)]
pub(crate) struct EmulatorAudio {
    queue: AudioQueue,
    underrun_count: UnderrunCount,
    requested_count: RequestedCount,
    control: Arc<AudioControl>,
    output_rate: u32,
    /// The machine's sample layout (`Machine::audio_format`).
    format: machine::AudioFormat,
}

/// The actual `rodio::Source`: resamples the shared `AudioQueue` (frames in
/// the machine's own `machine::AudioFormat`: its channel count and sample
/// rate) to `output_rate` via linear interpolation between the two input
/// frames bracketing each output frame's fractional input position -- see
/// `EmulatorAudio`'s doc comment for why this has to happen here rather than
/// upstream. The input-consumption rate this naturally settles at is the
/// machine's own sample rate exactly, regardless of how often `next` itself
/// gets called -- so the queue drains at the rate it's actually filled,
/// whatever `output_rate` turns out to be on a given machine.
///
/// Output is always stereo (left, right, left, ...): a mono machine plays the
/// same sample on both sides, a stereo machine keeps its two channels, and a
/// machine with more channels contributes its first two.
///
/// Emits silence (`0.0`) for an input frame that underruns rather than
/// ending the stream -- returning `None` from `next` would tell the audio
/// backend playback finished, which would stop it from ever polling again
/// even once the queue has more samples later. Counts underruns and total
/// requests (cheap relaxed atomic increments -- this runs on the audio
/// backend's own real-time callback thread, so it must stay allocation/
/// lock-contention-free beyond the queue's own already-necessary lock)
/// for `emu_thread`'s 1Hz diagnostics log -- see `UnderrunCount`/
/// `RequestedCount`'s doc comments. Both count *frames*: `requested_count`
/// once per output frame (so `requested/s` still reads as the device rate),
/// `underrun_count` once per missing input frame the resampler needed.
pub(crate) struct EmulatorAudioDecoder {
    queue: AudioQueue,
    underrun_count: UnderrunCount,
    requested_count: RequestedCount,
    control: Arc<AudioControl>,
    /// Current output gain, ramped toward `control.target_gain()`.
    gain: f32,
    output_rate: u32,
    format: machine::AudioFormat,
    /// Fractional position within the input stream, in input-frame
    /// units, advanced by `format.sample_rate / output_rate` per output
    /// frame produced. `prev`/`next` are the two input frames (left, right)
    /// this position currently falls between; a new input frame is popped
    /// each time the position crosses a whole-number boundary.
    position: f64,
    prev: [f32; 2],
    next: [f32; 2],
    /// The right half of the output frame whose left half `next` returned.
    right_pending: Option<f32>,
    initialized: bool,
}

impl EmulatorAudioDecoder {
    fn new(
        queue: AudioQueue,
        underrun_count: UnderrunCount,
        requested_count: RequestedCount,
        control: Arc<AudioControl>,
        output_rate: u32,
        format: machine::AudioFormat,
    ) -> Self {
        let gain = control.target_gain();
        Self {
            queue,
            underrun_count,
            requested_count,
            control,
            gain,
            output_rate,
            format: machine::AudioFormat {
                channels: format.channels.max(1),
                sample_rate: if format.sample_rate == 0 {
                    machine::AUDIO_SAMPLE_RATE
                } else {
                    format.sample_rate
                },
            },
            position: 0.0,
            prev: [0.0; 2],
            next: [0.0; 2],
            right_pending: None,
            initialized: false,
        }
    }

    /// Pops one input frame as (left, right). A frame the queue cannot fill
    /// completely is an underrun and plays as silence.
    fn pop_input(&mut self) -> [f32; 2] {
        let channels = usize::from(self.format.channels);
        let mut queue = self.queue.lock().unwrap();
        if queue.len() < channels {
            queue.clear();
            drop(queue);
            self.underrun_count.fetch_add(1, Ordering::Relaxed);
            return [0.0; 2];
        }
        let left = queue.pop_front().unwrap_or(0.0);
        let right = if channels >= 2 {
            queue.pop_front().unwrap_or(0.0)
        } else {
            left
        };
        if channels > 2 {
            queue.drain(..channels - 2);
        }
        [left, right]
    }
}

impl Iterator for EmulatorAudioDecoder {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if let Some(right) = self.right_pending.take() {
            return Some(right);
        }
        self.requested_count.fetch_add(1, Ordering::Relaxed);

        if !self.initialized {
            self.prev = self.pop_input();
            self.next = self.pop_input();
            self.initialized = true;
        }

        let frac = self.position.fract() as f32;
        let target = self.control.target_gain();
        self.gain += (target - self.gain).clamp(-GAIN_RAMP_PER_SAMPLE, GAIN_RAMP_PER_SAMPLE);
        let [left, right] =
            [0, 1].map(|c| (self.prev[c] + (self.next[c] - self.prev[c]) * frac) * self.gain);

        self.position += f64::from(self.format.sample_rate) / f64::from(self.output_rate);
        while self.position >= 1.0 {
            self.position -= 1.0;
            self.prev = self.next;
            self.next = self.pop_input();
        }

        self.right_pending = Some(right);
        Some(left)
    }
}

impl Source for EmulatorAudioDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None // infinite stream
    }

    fn channels(&self) -> ChannelCount {
        NonZero::new(2).unwrap()
    }

    fn sample_rate(&self) -> SampleRate {
        // `output_rate` is validated non-zero when `setup_audio` obtains
        // it (falls back to `machine::AUDIO_SAMPLE_RATE`, itself a fixed
        // non-zero constant, if the real device query fails).
        NonZero::new(self.output_rate).unwrap()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Decodable for EmulatorAudio {
    type Decoder = EmulatorAudioDecoder;

    fn decoder(&self) -> Self::Decoder {
        EmulatorAudioDecoder::new(
            self.queue.clone(),
            self.underrun_count.clone(),
            self.requested_count.clone(),
            self.control.clone(),
            self.output_rate,
            self.format,
        )
    }
}

/// Wraps the emu thread's own audio queue as a Bevy resource -- the queue
/// itself is created by `emu_thread::spawn`, not here; `setup_audio` below
/// just clones it into the playable `EmulatorAudio` asset.
#[derive(Resource)]
pub(crate) struct AudioQueueRes(pub(crate) AudioQueue, pub(crate) machine::AudioFormat);

/// Wraps the emu thread's underrun counter -- see `UnderrunCount`'s doc
/// comment. Same "created by `emu_thread::spawn`, just cloned here" shape
/// as `AudioQueueRes`.
#[derive(Resource)]
pub(crate) struct UnderrunCountRes(pub(crate) UnderrunCount);

/// Wraps the emu thread's requested-sample counter -- see
/// `RequestedCount`'s doc comment. Same shape as `AudioQueueRes`/
/// `UnderrunCountRes`.
#[derive(Resource)]
pub(crate) struct RequestedCountRes(pub(crate) RequestedCount);

/// Mute/volume shared with the hotkey system -- see `AudioControl`.
#[derive(Resource, Clone, Default)]
pub(crate) struct AudioControlRes(pub(crate) Arc<AudioControl>);

/// Queries the real default output device's own configured sample rate
/// directly via `cpal` (already a transitive dependency through
/// `bevy_audio`/rodio -- added as a direct one here since we now need to
/// call it ourselves, see `EmulatorAudio`'s doc comment for why). `None`
/// if there's no default output device or the query otherwise fails --
/// `setup_audio` falls back to `machine::AUDIO_SAMPLE_RATE` (no
/// resampling needed) in that case rather than treating it as fatal.
fn default_output_sample_rate() -> Option<u32> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let device = cpal::default_host().default_output_device()?;
    let config = device.default_output_config().ok()?;
    Some(config.sample_rate())
}

/// Registers the emu thread's audio queue as a playable asset and starts
/// it playing on a loop -- the "loop" never actually restarts anything
/// (the decoder is an infinite stream, see above), it just means the sink
/// doesn't consider a never-ending source an error.
pub(crate) fn setup_audio(
    mut commands: Commands,
    mut audio_sources: ResMut<Assets<EmulatorAudio>>,
    queue: Res<AudioQueueRes>,
    underrun_count: Res<UnderrunCountRes>,
    requested_count: Res<RequestedCountRes>,
    control: Res<AudioControlRes>,
) {
    // The emu thread starts producing samples as soon as it's spawned (see
    // `gui::run_gui`), well before this `Startup` system runs -- window
    // creation, GPU adapter/device init, and shader compilation can take
    // several hundred ms to a couple of seconds, with nobody draining the
    // queue in the meantime. Whatever backlog built up during that window
    // is stale, pre-playback audio, not real dropped audio -- clear it right
    // before playback starts so it doesn't become permanent latency for the
    // rest of the session (production/consumption rates match from here on,
    // so nothing would otherwise shrink the queue back down later).
    queue.0.lock().unwrap().clear();

    let output_rate = default_output_sample_rate().unwrap_or(machine::AUDIO_SAMPLE_RATE);
    tracing::info!("audio: resampling to the real output device rate ({output_rate}Hz)");

    let handle = audio_sources.add(EmulatorAudio {
        queue: queue.0.clone(),
        underrun_count: underrun_count.0.clone(),
        requested_count: requested_count.0.clone(),
        control: control.0.clone(),
        output_rate,
        format: queue.1,
    });
    commands.spawn((AudioPlayer(handle), PlaybackSettings::LOOP));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicU64;
    use std::sync::{Arc, Mutex};

    fn decoder(
        samples: Vec<f32>,
        format: machine::AudioFormat,
        output_rate: u32,
    ) -> (EmulatorAudioDecoder, UnderrunCount, RequestedCount) {
        let underrun_count: UnderrunCount = Arc::new(AtomicU64::new(0));
        let requested_count: RequestedCount = Arc::new(AtomicU64::new(0));
        let decoder = EmulatorAudioDecoder::new(
            Arc::new(Mutex::new(VecDeque::from(samples))),
            underrun_count.clone(),
            requested_count.clone(),
            Arc::new(AudioControl::default()),
            output_rate,
            format,
        );
        (decoder, underrun_count, requested_count)
    }

    #[test]
    fn resamples_input_rate_to_a_higher_output_rate_via_linear_interpolation() {
        // 5 mono input samples read out at exactly 2x the input rate --
        // position advances by 0.5 per output frame, so odd frames land
        // exactly on an input sample and even frames exactly halfway between
        // two. 5 (not 4) input samples so the resampler's one-frame-ahead
        // lookahead pop doesn't underrun before the 6th frame. A mono
        // machine plays each value on both sides.
        let (mut decoder, underruns, requested) = decoder(
            vec![1.0, 3.0, 5.0, 7.0, 9.0],
            machine::AudioFormat::MONO,
            machine::AUDIO_SAMPLE_RATE * 2,
        );

        let outputs: Vec<f32> = (0..12).map(|_| decoder.next().unwrap()).collect();

        assert_eq!(
            outputs,
            vec![1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 5.0, 5.0, 6.0, 6.0]
        );
        assert_eq!(underruns.load(Ordering::Relaxed), 0);
        assert_eq!(requested.load(Ordering::Relaxed), 6);
    }

    #[test]
    fn a_stereo_machine_keeps_its_channels_and_its_own_sample_rate_is_used() {
        // Stereo at 22050 Hz played on a 44100 Hz device: two output frames
        // per input frame, left and right interpolated separately.
        let format = machine::AudioFormat {
            channels: 2,
            sample_rate: 22_050,
        };
        let (mut decoder, underruns, _) = decoder(
            vec![0.0, 1.0, 0.5, -1.0, 1.0, 0.0, 0.0, 0.0],
            format,
            44_100,
        );

        let outputs: Vec<f32> = (0..8).map(|_| decoder.next().unwrap()).collect();

        assert_eq!(outputs, vec![0.0, 1.0, 0.25, 0.0, 0.5, -1.0, 0.75, -0.5]);
        assert_eq!(underruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn counts_an_underrun_once_per_missing_input_frame_not_per_output_call() {
        let (mut decoder, underruns, requested) = decoder(
            Vec::new(),
            machine::AudioFormat::MONO,
            machine::AUDIO_SAMPLE_RATE, // 1:1 -- exactly one input pop per output frame
        );

        decoder.next(); // seeds prev+next (2 underruns), then its own loop pops 1 more (3)
        decoder.next(); // the right half of the same frame: no pop
        decoder.next(); // one more pop in its own loop (4)

        assert_eq!(underruns.load(Ordering::Relaxed), 4);
        assert_eq!(requested.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn mute_and_volume_state_behave_like_a_device() {
        let control = AudioControl::default();
        assert_eq!(control.target_gain(), 1.0);
        assert!(control.toggle_mute());
        assert_eq!(control.target_gain(), 0.0);
        assert!(!control.toggle_mute());

        assert_eq!(control.adjust_volume(-30), 70);
        assert_eq!(control.target_gain(), 0.7);
        assert_eq!(control.adjust_volume(-200), 0, "clamped at 0");
        assert_eq!(control.adjust_volume(500), 100, "clamped at 100");

        // Raising the volume while muted un-mutes; lowering does not.
        control.toggle_mute();
        control.adjust_volume(-10);
        assert!(control.is_muted());
        control.adjust_volume(10);
        assert!(!control.is_muted());
        // Un-muting restores the volume that was set, not 100%.
        control.adjust_volume(-50);
        control.toggle_mute();
        control.toggle_mute();
        assert_eq!(control.volume_percent(), 50);
    }

    #[test]
    fn the_decoder_applies_volume_and_ramps_to_mute_without_a_step() {
        let control = Arc::new(AudioControl::default());
        let queue: AudioQueue = Arc::new(Mutex::new(VecDeque::from(vec![1.0; 4000])));
        let mut decoder = EmulatorAudioDecoder::new(
            queue,
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicU64::new(0)),
            control.clone(),
            machine::AUDIO_SAMPLE_RATE,
            machine::AudioFormat::MONO,
        );
        // Full volume passes samples through unchanged.
        assert_eq!(decoder.next().unwrap(), 1.0);
        assert_eq!(decoder.next().unwrap(), 1.0);

        control.toggle_mute();
        let ramp: Vec<f32> = (0..1024).map(|_| decoder.next().unwrap()).collect();
        assert!(ramp[0] > 0.99, "no instant cut: {}", ramp[0]);
        assert!(
            ramp.windows(2).all(|w| w[1] <= w[0]),
            "gain only falls while muting"
        );
        assert_eq!(*ramp.last().unwrap(), 0.0, "fully silent after the ramp");

        control.toggle_mute();
        control.adjust_volume(-50);
        let after: Vec<f32> = (0..1024).map(|_| decoder.next().unwrap()).collect();
        assert!((after.last().unwrap() - 0.5).abs() < 1e-6);
    }
}
