# Dedicated Emulation Thread + Tracy Profiling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move CPU stepping, audio generation, and frame production off the Bevy render thread onto a dedicated background thread (fixing audio skipping caused by render-thread stalls starving the audio queue), split the now-larger `gui.rs` into focused files by concern, and wire up Tracy profiling to confirm the fix.

**Architecture:** A new `emu_thread.rs` owns a `std::thread` that runs `AnyMachine` at its own pace (mirroring today's `step_machine`'s cycle-budget math), publishing audio samples, the latest video frame, and a register/halted snapshot into shared `Arc<Mutex<...>>` handoff points. `gui.rs` becomes `gui/mod.rs` (assembly only) plus `gui/{audio,video,window,debug_overlay,input}.rs`, each reading from those handoff points instead of calling the machine directly. `EmuThreadHandle`'s `Drop` impl stops and joins the thread automatically when `run_gui` returns.

**Tech Stack:** Rust, Bevy 0.19, `std::thread`/`std::sync::mpsc`/`Arc<Mutex<_>>`, `tracing` (for the profiling span), Tracy (`bevy/trace_tracy` feature).

**Spec:** `docs/superpowers/specs/2026-09-09-emu-thread-tracy-design.md`

## Global Constraints

- Fix must be generic across every `Machine` impl (zx81, zx-spectrum, and any future machine) with zero per-machine code -- it lives against `AnyMachine`/`Machine`, never a concrete machine type.
- Headless mode (`--headless`, `machine::Clock`/`Machine::run()`) is untouched.
- ZX81 stays parked -- this work is not ZX81-specific and does not change its priority (see `TODO.md`).
- `profiling` Cargo feature (forwarding to `bevy/trace_tracy` + `tracing`) must be off by default -- zero cost for normal builds.
- Calling a trait method via dot-syntax on an `&dyn Machine`/`&mut dyn Machine`-typed value does **not** require `use machine::Machine;` in this codebase (confirmed: `apps/retroclawd/src/gui.rs`'s existing `step_machine` already does this without that import, and compiles) -- do not add that import out of caution, it's simply unnecessary.

---

## Task 1: `emu_thread.rs` -- shared types + testable per-tick logic

**Files:**
- Create: `apps/retroclawd/src/emu_thread.rs`
- Modify: `apps/retroclawd/src/main.rs` (add `mod emu_thread;`)

**Interfaces:**
- Consumes: `crate::any_machine::AnyMachine` (`as_machine()`/`as_machine_mut()` -> `&dyn Machine`/`&mut dyn Machine`, `key_event(KeyCode, bool)`), `machine::AUDIO_SAMPLE_RATE: u32`.
- Produces (for Task 2 and Task 3 to consume):
  - `pub(crate) type AudioQueue = Arc<Mutex<VecDeque<f32>>>;`
  - `pub(crate) type FrameBuffer = Arc<Mutex<Option<(u32, u32, Vec<u8>)>>>;`
  - `pub(crate) struct DebugSnapshot { pub(crate) halted: bool, pub(crate) registers: Vec<(&'static str, u64)> }`
  - `pub(crate) type DebugSnapshotHandle = Arc<Mutex<DebugSnapshot>>;`
  - `fn tick(machine: &mut AnyMachine, input_rx: &Receiver<(KeyCode, bool)>, audio_queue: &AudioQueue, frame_buffer: &FrameBuffer, debug_snapshot: &DebugSnapshotHandle)` (private to this file; Task 2's `spawn` calls it in a loop).

- [ ] **Step 1: Write `emu_thread.rs` with the shared types and the `tick` function**

```rust
use std::collections::VecDeque;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use bevy::input::keyboard::KeyCode;

use crate::any_machine::AnyMachine;

/// Samples queued between the emu thread generating them and the audio
/// backend actually consuming them (see `gui::audio`'s `EmulatorAudioDecoder`,
/// which drains this on the audio backend's own thread). A plain
/// `Mutex<VecDeque>` rather than a lock-free ring buffer: contention here
/// is negligible (one push per emu-thread tick, one pop per output sample,
/// never held across any blocking work).
pub(crate) type AudioQueue = Arc<Mutex<VecDeque<f32>>>;

/// Upper bound on queued-but-unplayed samples (~2 seconds at
/// `machine::AUDIO_SAMPLE_RATE`) -- if the audio backend falls behind for
/// any reason, `tick` drops the oldest samples rather than growing this
/// unboundedly; a couple of seconds of latency is already far past the
/// point of being useful to keep.
const MAX_QUEUED_AUDIO_SAMPLES: usize = machine::AUDIO_SAMPLE_RATE as usize * 2;

/// The emu thread's latest rendered frame -- `gui::video::render_screen`
/// reads whatever's here at Bevy's own render pace, independent of how
/// often the emu thread actually produces a new one.
pub(crate) type FrameBuffer = Arc<Mutex<Option<(u32, u32, Vec<u8>)>>>;

/// Snapshot of the machine's halted flag + registers, refreshed every emu
/// thread tick -- `gui::debug_overlay::update_debug_overlay` reads this
/// instead of the machine directly, since the machine only lives on the
/// emu thread now.
pub(crate) struct DebugSnapshot {
    pub(crate) halted: bool,
    pub(crate) registers: Vec<(&'static str, u64)>,
}

pub(crate) type DebugSnapshotHandle = Arc<Mutex<DebugSnapshot>>;

/// One tick's worth of work: apply any queued input, step the machine for
/// one `frame_hz()` tick's cycle budget (or until halted), then publish
/// audio/frame/debug state for the Bevy side to pick up. Kept as a plain
/// function (not inlined into `spawn`'s loop, see Task 2) so it's testable
/// directly, without a real thread or real sleeping.
fn tick(
    machine: &mut AnyMachine,
    input_rx: &Receiver<(KeyCode, bool)>,
    audio_queue: &AudioQueue,
    frame_buffer: &FrameBuffer,
    debug_snapshot: &DebugSnapshotHandle,
) {
    while let Ok((key, pressed)) = input_rx.try_recv() {
        machine.key_event(key, pressed);
    }

    let m = machine.as_machine_mut();

    // Once per tick, matching real hardware's frame interrupt (see
    // `Machine::request_interrupt`'s doc comment). Only sets a pending
    // flag -- doesn't clear `halted()` by itself, so the loop below must
    // call `step()` at least once even if `m.halted()` is currently true,
    // or a HALT-with-interrupts-enabled ROM loop would never get the
    // chance to wake back up.
    m.request_interrupt();

    let budget = (m.clock_hz() / m.frame_hz()).max(1);
    let mut spent = 0u64;
    #[cfg(feature = "profiling")]
    let _span = tracing::info_span!("emu_thread_step_budget").entered();
    loop {
        spent += m.step() as u64;
        if spent >= budget || m.halted() {
            break;
        }
    }

    let samples = m.drain_audio();
    if !samples.is_empty() {
        let mut queue = audio_queue.lock().unwrap();
        queue.extend(samples);
        // Drop the oldest samples rather than let a stalled audio backend
        // grow this without bound -- see MAX_QUEUED_AUDIO_SAMPLES's doc
        // comment.
        if queue.len() > MAX_QUEUED_AUDIO_SAMPLES {
            let excess = queue.len() - MAX_QUEUED_AUDIO_SAMPLES;
            queue.drain(..excess);
        }
    }

    if let Some(frame) = m.render_frame() {
        *frame_buffer.lock().unwrap() = Some(frame);
    }

    *debug_snapshot.lock().unwrap() = DebugSnapshot {
        halted: m.halted(),
        registers: m.registers(),
    };
}
```

Note: the `#[cfg(feature = "profiling")]` span is added here now (rather than
deferred to Task 4) because it belongs textually inside `tick`, right around
the step loop it measures -- Task 4 only adds the Cargo.toml plumbing
(`profiling` feature + `tracing` dependency) that makes this line compile
when the feature is on. With the feature off (the default), this line is
compiled out entirely -- zero cost, and no "unused `tracing`" build error
either, since `tracing` is only a dependency at all when `profiling` is on.

- [ ] **Step 2: Add `mod emu_thread;` to `main.rs`**

Add near the existing `mod any_machine;` / `mod gui;` lines (`apps/retroclawd/src/main.rs:1-3`):

```rust
mod any_machine;
mod emu_thread;
mod gui;
```

- [ ] **Step 3: Write the failing tests**

Append to `emu_thread.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use machine::Region;
    use std::sync::mpsc::channel;

    #[test]
    fn tick_applies_queued_input_before_stepping_and_updates_debug_snapshot() {
        let mut machine = AnyMachine::ZxSpectrum(zx_spectrum::ZxSpectrum::new(
            vec![0; zx_spectrum::ROM_SIZE],
            Region::Pal,
            zx_spectrum::Model::Spectrum48,
        ));
        {
            let m = machine.as_machine_mut();
            // LD A,0b1111_1011 / IN A,(0xFE) / HALT -- probes row 2 (Q's row).
            m.load(0x4000, &[0x3E, 0b1111_1011, 0xDB, 0xFE, 0x76]);
            m.set_pc(0x4000);
        }

        let (input_tx, input_rx) = channel();
        input_tx.send((KeyCode::KeyQ, true)).unwrap();

        let audio_queue: AudioQueue = Arc::new(Mutex::new(VecDeque::new()));
        let frame_buffer: FrameBuffer = Arc::new(Mutex::new(None));
        let debug_snapshot: DebugSnapshotHandle = Arc::new(Mutex::new(DebugSnapshot {
            halted: false,
            registers: Vec::new(),
        }));

        tick(
            &mut machine,
            &input_rx,
            &audio_queue,
            &frame_buffer,
            &debug_snapshot,
        );

        let a_register = (machine.as_machine().registers()[0].1 >> 8) as u8;
        assert_eq!(a_register, 0b1111_1110); // Q pressed -> bit 0 of row 2 cleared

        assert!(frame_buffer.lock().unwrap().is_some());
        assert!(debug_snapshot.lock().unwrap().halted);
    }

    #[test]
    fn tick_generates_audio_samples_over_a_full_cycle_budget() {
        // Zero-filled ROM = an infinite stream of NOPs (opcode 0x00) --
        // never halts, so this exercises a tick's full cycle budget rather
        // than stopping after a handful of instructions like the test
        // above. `tick`'s own logic is machine-agnostic (only calls
        // `Machine` trait methods), so one machine is enough to cover it --
        // `any_machine`'s own tests already prove `key_event`'s per-machine
        // dispatch works for both ZX81 and ZX Spectrum.
        let mut machine = AnyMachine::ZxSpectrum(zx_spectrum::ZxSpectrum::new(
            vec![0; zx_spectrum::ROM_SIZE],
            Region::Pal,
            zx_spectrum::Model::Spectrum48,
        ));
        let (_input_tx, input_rx) = channel();
        let audio_queue: AudioQueue = Arc::new(Mutex::new(VecDeque::new()));
        let frame_buffer: FrameBuffer = Arc::new(Mutex::new(None));
        let debug_snapshot: DebugSnapshotHandle = Arc::new(Mutex::new(DebugSnapshot {
            halted: false,
            registers: Vec::new(),
        }));

        tick(
            &mut machine,
            &input_rx,
            &audio_queue,
            &frame_buffer,
            &debug_snapshot,
        );

        assert!(!audio_queue.lock().unwrap().is_empty());
        assert!(!debug_snapshot.lock().unwrap().halted);
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail (or don't compile yet)**

Run: `cargo test -p retroclawd emu_thread`
Expected: compile error or FAIL, since `tick` didn't exist before Step 1 was applied in this same task -- if you're doing Steps 1 and 3 together, instead just run this after Step 1+3 both exist and confirm PASS directly (there's no meaningful "red" state here since `tick`'s implementation is written in the same step as its type signatures, not discovered via failing-test iteration -- the tests exist to lock in the two most important behaviors: input applied before stepping, and audio/frame/debug state all get published).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p retroclawd emu_thread`
Expected: both tests PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/retroclawd/src/emu_thread.rs apps/retroclawd/src/main.rs
git commit -m "feat(retroclawd): add emu_thread's shared types and per-tick logic

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 2: `EmuThreadHandle` -- spawn the background thread

**Files:**
- Modify: `apps/retroclawd/src/emu_thread.rs` (append `EmuThreadHandle`, `spawn`, `Drop` impl, and a test)

**Interfaces:**
- Consumes: `tick` and the shared types from Task 1; `AnyMachine` (moved by value).
- Produces (for Task 3 to consume):
  - `pub(crate) struct EmuThreadHandle { pub(crate) audio_queue: AudioQueue, pub(crate) frame_buffer: FrameBuffer, pub(crate) debug_snapshot: DebugSnapshotHandle, pub(crate) input_tx: Sender<(KeyCode, bool)>, ... }`
  - `pub(crate) fn spawn(machine: AnyMachine) -> EmuThreadHandle`

- [ ] **Step 1: Write the failing test**

Add to `emu_thread.rs`'s `tests` module (below the two from Task 1):

```rust
    #[test]
    fn spawn_and_drop_actually_stops_the_background_thread() {
        let machine = AnyMachine::ZxSpectrum(zx_spectrum::ZxSpectrum::new(
            vec![0; zx_spectrum::ROM_SIZE],
            Region::Pal,
            zx_spectrum::Model::Spectrum48,
        ));
        let handle = spawn(machine);
        std::thread::sleep(Duration::from_millis(50)); // let it start ticking

        // Dropping on another thread + a bounded `recv_timeout`, rather
        // than dropping directly here, turns "the join hangs forever" into
        // an explicit test failure instead of an indefinitely hung test
        // run.
        let (done_tx, done_rx) = channel();
        std::thread::spawn(move || {
            drop(handle);
            let _ = done_tx.send(());
        });

        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("dropping EmuThreadHandle must stop and join the background thread within 2s");
    }
```

This needs `use std::time::Duration;` in the `tests` module's imports (`use super::*;` already brings in the outer module's `Duration` import once Step 2 below adds it).

- [ ] **Step 2: Run test to verify it fails to compile**

Run: `cargo test -p retroclawd emu_thread`
Expected: FAIL to compile -- `spawn` and `EmuThreadHandle` don't exist yet.

- [ ] **Step 3: Implement `EmuThreadHandle` and `spawn`**

Add near the top of `emu_thread.rs` (with the other `use` statements):

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
```

Append after `tick`:

```rust
/// Owns the background thread `spawn` starts, and every shared handoff
/// point it writes to. Dropping this (see `Drop` impl below) is the only
/// way to stop the thread -- there's no separate `stop()` method, so a
/// caller can't forget to shut it down.
pub(crate) struct EmuThreadHandle {
    pub(crate) audio_queue: AudioQueue,
    pub(crate) frame_buffer: FrameBuffer,
    pub(crate) debug_snapshot: DebugSnapshotHandle,
    pub(crate) input_tx: Sender<(KeyCode, bool)>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Drop for EmuThreadHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Moves `machine` onto its own thread, running it at its own `frame_hz()`
/// pace via real-time sleep (the same accumulator idea `machine::Clock`
/// already uses for headless/`Machine::run()`, just with audio/frame/debug
/// side effects added at each tick -- see `tick` above). Bevy-agnostic:
/// depends only on `std` and `AnyMachine`, so it works the same for every
/// machine with no per-machine code, same as `AnyMachine` itself.
pub(crate) fn spawn(mut machine: AnyMachine) -> EmuThreadHandle {
    let audio_queue: AudioQueue = Arc::new(Mutex::new(VecDeque::new()));
    let frame_buffer: FrameBuffer = Arc::new(Mutex::new(None));
    let debug_snapshot: DebugSnapshotHandle = Arc::new(Mutex::new(DebugSnapshot {
        halted: false,
        registers: Vec::new(),
    }));
    let (input_tx, input_rx) = mpsc::channel::<(KeyCode, bool)>();
    let stop = Arc::new(AtomicBool::new(false));

    let thread_audio_queue = audio_queue.clone();
    let thread_frame_buffer = frame_buffer.clone();
    let thread_debug_snapshot = debug_snapshot.clone();
    let thread_stop = stop.clone();

    let join = thread::spawn(move || {
        let tick_duration = Duration::from_secs_f64(1.0 / machine.as_machine().frame_hz() as f64);
        let mut logged_halt = false;

        while !thread_stop.load(Ordering::Relaxed) {
            let tick_start = Instant::now();

            tick(
                &mut machine,
                &input_rx,
                &thread_audio_queue,
                &thread_frame_buffer,
                &thread_debug_snapshot,
            );

            if machine.as_machine().halted() {
                if !logged_halt {
                    let pc = machine
                        .as_machine()
                        .registers()
                        .iter()
                        .find(|(name, _)| *name == "PC")
                        .map(|(_, v)| *v)
                        .unwrap_or(0);
                    crate::debug_log!("machine halted at pc=0x{pc:04X}");
                    logged_halt = true;
                }
            } else {
                logged_halt = false;
            }

            let elapsed = tick_start.elapsed();
            if elapsed < tick_duration {
                thread::sleep(tick_duration - elapsed);
            }
        }
    });

    EmuThreadHandle {
        audio_queue,
        frame_buffer,
        debug_snapshot,
        input_tx,
        stop,
        join: Some(join),
    }
}
```

If `cargo build` reports `AnyMachine` (or `Zx81`/`ZxSpectrum`) does not implement
`Send`, inspect whichever field is the actual cause -- this was checked during
design (both structs hold only owned `Vec<u8>`/primitives/enums, no `Rc` or raw
pointers) and is not expected, but confirm against the real compiler error
rather than assume the design-time read was complete.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p retroclawd emu_thread`
Expected: all three tests in `emu_thread::tests` PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/retroclawd/src/emu_thread.rs
git commit -m "feat(retroclawd): spawn the emu thread, stop+join it on Drop

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 3: Split `gui.rs` into `gui/{mod,audio,video,window,debug_overlay,input}.rs`

**Files:**
- Delete: `apps/retroclawd/src/gui.rs`
- Create: `apps/retroclawd/src/gui/mod.rs`
- Create: `apps/retroclawd/src/gui/audio.rs`
- Create: `apps/retroclawd/src/gui/video.rs`
- Create: `apps/retroclawd/src/gui/window.rs`
- Create: `apps/retroclawd/src/gui/debug_overlay.rs`
- Create: `apps/retroclawd/src/gui/input.rs`

**Interfaces:**
- Consumes: `emu_thread::{spawn, AudioQueue, FrameBuffer, DebugSnapshotHandle, EmuThreadHandle}` (Tasks 1-2), `any_machine::{AnyMachine, SHIFT_SOURCE_KEYS}`.
- Produces: `gui::run_gui(machine: AnyMachine, show_debug_info: bool, start_fullscreen: bool)` -- same public signature as before, so `main.rs`'s call site (`apps/retroclawd/src/main.rs:283`) does not change.

This codebase doesn't unit-test Bevy systems (verified by `gui.rs` having none today) -- this task is verified by `cargo build`, `cargo clippy`, and a manual run, not new automated tests.

- [ ] **Step 1: Delete `gui.rs` and create the directory's files**

```bash
git rm apps/retroclawd/src/gui.rs
```

- [ ] **Step 2: Write `apps/retroclawd/src/gui/audio.rs`**

```rust
use std::num::NonZero;
use std::time::Duration;

use bevy::asset::Asset;
use bevy::audio::{AudioPlayer, ChannelCount, Decodable, PlaybackSettings, Sample, SampleRate, Source};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use crate::emu_thread::AudioQueue;

/// A `Decodable` asset type whose "decoding" is really just draining a
/// live, ever-growing `AudioQueue` -- see `Decodable` and `Source`'s impls
/// below. There's exactly one of these per run (`setup_audio` creates and
/// plays it once), not a real per-file asset; it exists at all because
/// Bevy's audio playback API is built around `Handle<Source>` +
/// `AudioPlayer`, and `Decodable`/`add_audio_source` is the documented
/// escape hatch for streaming synthesized audio instead of a decoded file
/// (see `bevy_audio::audio_source::Decodable`'s own doc comment).
#[derive(Asset, TypePath, Clone)]
pub(crate) struct EmulatorAudio(AudioQueue);

/// The actual `rodio::Source`: pulls samples out of the shared queue,
/// emitting silence (`0.0`) on underrun rather than ending the stream --
/// returning `None` from `next` would tell the audio backend playback
/// finished, which would stop it from ever polling again even once the
/// queue has more samples later.
pub(crate) struct EmulatorAudioDecoder(AudioQueue);

impl Iterator for EmulatorAudioDecoder {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        Some(self.0.lock().unwrap().pop_front().unwrap_or(0.0))
    }
}

impl Source for EmulatorAudioDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None // infinite stream
    }

    fn channels(&self) -> ChannelCount {
        NonZero::new(1).unwrap()
    }

    fn sample_rate(&self) -> SampleRate {
        NonZero::new(machine::AUDIO_SAMPLE_RATE).unwrap()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Decodable for EmulatorAudio {
    type Decoder = EmulatorAudioDecoder;

    fn decoder(&self) -> Self::Decoder {
        EmulatorAudioDecoder(self.0.clone())
    }
}

/// Wraps the emu thread's own audio queue as a Bevy resource -- the queue
/// itself is created by `emu_thread::spawn`, not here; `setup_audio` below
/// just clones it into the playable `EmulatorAudio` asset.
#[derive(Resource)]
pub(crate) struct AudioQueueRes(pub(crate) AudioQueue);

/// Registers the emu thread's audio queue as a playable asset and starts
/// it playing on a loop -- the "loop" never actually restarts anything
/// (the decoder is an infinite stream, see above), it just means the sink
/// doesn't consider a never-ending source an error.
pub(crate) fn setup_audio(
    mut commands: Commands,
    mut audio_sources: ResMut<Assets<EmulatorAudio>>,
    queue: Res<AudioQueueRes>,
) {
    let handle = audio_sources.add(EmulatorAudio(queue.0.clone()));
    commands.spawn((AudioPlayer(handle), PlaybackSettings::LOOP));
}
```

- [ ] **Step 3: Write `apps/retroclawd/src/gui/video.rs`**

```rust
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::{PrimaryWindow, Window};

use crate::emu_thread::FrameBuffer;

/// The real, tiny picture size (`Machine::render_frame`'s own output size,
/// border included) -- captured once at startup so `fit_screen` doesn't
/// need to re-render a frame just to read its dimensions every tick.
#[derive(Resource)]
pub(crate) struct BaseResolution {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// Marks the single sprite entity that displays the emulated picture, so
/// `fit_screen` can find it without also matching UI/debug-overlay nodes.
#[derive(Component)]
pub(crate) struct ScreenSprite;

#[derive(Resource)]
pub(crate) struct ScreenTexture(Handle<Image>);

/// Holds the emu thread's latest rendered frame -- `render_screen` copies
/// whatever's here into the live texture every `Update` tick, at Bevy's
/// own pace, independent of how often the emu thread actually produces a
/// new one.
#[derive(Resource)]
pub(crate) struct FrameBufferRes(pub(crate) FrameBuffer);

/// Initial window scale over the real, tiny 256x192 (or similar) picture --
/// only used to size the window at startup. After that, `fit_screen`
/// recomputes the sprite's actual scale every frame from the window's live
/// size, so this is a starting point, not a cap.
pub(crate) const INITIAL_SCALE: f32 = 3.0;

/// Creates the live texture at `base`'s size, filled with black until the
/// emu thread produces its first real frame (`render_screen` overwrites it
/// on the very next `Update` tick in practice) -- unlike before the
/// emu-thread split, there's no machine resource to read an initial frame
/// from directly here, since the machine now lives only on the emu thread.
pub(crate) fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    base: Res<BaseResolution>,
) {
    let pixel_count = base.width as usize * base.height as usize;
    let mut image = Image::new(
        Extent3d {
            width: base.width,
            height: base.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; pixel_count * 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    // Real hardware has hard pixel edges -- linear filtering (bevy's
    // per-image default) blurs them under any non-integer or magnified
    // scale. `fit_screen` only ever applies integer scale factors, but
    // nearest-neighbor is what makes each of those factors look crisp
    // rather than softened.
    image.sampler = ImageSampler::nearest();
    let handle = images.add(image);

    commands.spawn(Camera2d);
    commands.spawn((
        ScreenSprite,
        Sprite::from_image(handle.clone()),
        Transform::from_scale(Vec3::splat(INITIAL_SCALE)),
    ));
    commands.insert_resource(ScreenTexture(handle));
}

/// `render_frame`'s output is RGB (3 bytes/pixel); Bevy's texture format
/// here is RGBA (4 bytes/pixel) -- insert a fully-opaque alpha byte after
/// each pixel.
fn rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(rgb.len() / 3 * 4);
    for chunk in rgb.as_chunks::<3>().0 {
        rgba.extend_from_slice(chunk);
        rgba.push(255);
    }
    rgba
}

/// Copies the emu thread's latest frame (`FrameBufferRes`) into the live
/// texture. Runs every `Update` tick at Bevy's own render pace -- the emu
/// thread may have produced several frames or none since the last time
/// this ran, and either is fine, since only the latest one is ever kept.
/// Clones the locked `Option<(u32, u32, Vec<u8>)>` out before converting
/// to RGBA, rather than converting while holding the lock, so the emu
/// thread is never blocked waiting on this system's (comparatively slow)
/// pixel-format conversion.
pub(crate) fn render_screen(
    frame_buffer: Res<FrameBufferRes>,
    screen: Res<ScreenTexture>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some((_, _, rgb)) = frame_buffer.0.lock().unwrap().clone() else {
        return;
    };
    if let Some(mut image) = images.get_mut(&screen.0) {
        image.data = Some(rgb_to_rgba(&rgb));
    }
}

/// Recomputes the on-screen sprite's scale every frame from the window's
/// current (live) size -- what makes resizing the window actually rescale
/// the picture instead of leaving it stuck at `INITIAL_SCALE`. Always an
/// integer factor (floored), so every pixel of the real, tiny picture maps
/// to a whole number of screen pixels -- pixel-perfect, letterboxed with
/// `ClearColor` (black) filling any leftover space, rather than stretching
/// to fill the window exactly (which would soften the image at whatever
/// factor doesn't divide evenly). Runs every frame rather than only on a
/// resize event since it also needs to react to fullscreen toggling, which
/// doesn't necessarily fire the same event -- the math is cheap enough that
/// this doesn't matter.
pub(crate) fn fit_screen(
    base: Res<BaseResolution>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut sprite: Query<&mut Transform, With<ScreenSprite>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(mut transform) = sprite.single_mut() else {
        return;
    };

    let scale = (window.width() / base.width as f32)
        .min(window.height() / base.height as f32)
        .floor()
        .max(1.0);
    transform.scale = Vec3::splat(scale);
}
```

- [ ] **Step 4: Write `apps/retroclawd/src/gui/window.rs`**

```rust
use bevy::input::keyboard::KeyCode;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, Window, WindowMode};
use bevy::winit::WinitWindows;

/// Sets the real window-manager icon (title bar / taskbar / alt-tab), which
/// `winresource`'s build-time `.ico` embedding (`build.rs`) doesn't cover --
/// that only sets the icon Explorer shows for the `.exe` file itself before
/// it's running. Needs the raw `winit` crate directly (not just
/// `bevy::winit`, which re-exports `WinitWindows` but not `Icon` --
/// verified against `winit-0.30`'s own source, not guessed) at the same
/// version bevy_winit itself depends on, so the `Icon` type it constructs
/// is the same type `set_window_icon` expects.
///
/// Runs in `Update`, not `Startup`: `bevy_winit`'s actual native window
/// creation happens via its own event-loop-driven runner, not the App's
/// `Startup` schedule, so `WinitWindows` isn't guaranteed to exist yet when
/// `Startup` systems run (confirmed live -- an earlier `Startup`-scheduled
/// version of this panicked with "Non-send data not found"). `done` makes
/// this retry every frame until the window actually exists, then never run
/// its body again.
///
/// NOTE the `include_bytes!` path below is one `../` deeper than it was in
/// the old single-file `gui.rs`, since this file now lives at
/// `apps/retroclawd/src/gui/window.rs` (one directory deeper than
/// `apps/retroclawd/src/gui.rs` was).
pub(crate) fn set_window_icon(
    windows: Option<NonSend<WinitWindows>>,
    primary: Query<Entity, With<PrimaryWindow>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(windows) = windows else {
        return;
    };
    let Ok(entity) = primary.single() else {
        return;
    };
    let Some(winit_window) = windows.get_window(entity) else {
        return;
    };
    *done = true;

    let bytes = include_bytes!("../../../../assets/icon-64.png");
    let Ok(mut reader) = png::Decoder::new(std::io::Cursor::new(bytes.as_slice())).read_info()
    else {
        return;
    };
    let Some(buffer_size) = reader.output_buffer_size() else {
        return;
    };
    let mut buf = vec![0u8; buffer_size];
    let Ok(info) = reader.next_frame(&mut buf) else {
        return;
    };
    let rgba = buf[..info.buffer_size()].to_vec();

    if let Ok(icon) = winit::window::Icon::from_rgba(rgba, info.width, info.height) {
        winit_window.set_window_icon(Some(icon));
    }
}

/// F11 toggles fullscreen live (see `run_gui`'s doc comment for why this key
/// is safe to reserve). `BorderlessFullscreen` rather than exclusive
/// `Fullscreen`: it just uses the current monitor's own resolution/refresh
/// rate directly, no separate display-mode negotiation, and `video::fit_screen`
/// is what actually gets the "scale to 4K" vs "scale to 1080p" effect the
/// user wants -- a bigger monitor doesn't force a bigger scale factor, it
/// just leaves more (or less) letterboxed space around whatever integer
/// factor fits.
pub(crate) fn toggle_fullscreen(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !keyboard.just_pressed(KeyCode::F11) {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    window.mode = match window.mode {
        WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        _ => WindowMode::Windowed,
    };
}
```

- [ ] **Step 5: Write `apps/retroclawd/src/gui/debug_overlay.rs`**

```rust
use std::time::Duration;

use bevy::prelude::*;

use crate::emu_thread::DebugSnapshotHandle;

/// Snapshot of the emu thread's machine state (halted flag + registers),
/// written every tick by `emu_thread::tick` -- `update_debug_overlay`
/// reads this instead of the machine directly, since the machine now
/// lives only on the emu thread, not in this ECS world.
#[derive(Resource)]
pub(crate) struct DebugSnapshotRes(pub(crate) DebugSnapshotHandle);

/// Marks the single UI text entity `update_debug_overlay` rewrites.
#[derive(Component)]
pub(crate) struct DebugOverlayText;

/// How often the `--debug-info` overlay's text is rewritten. Deliberately
/// not every frame -- register/PC values changing 50-60 times a second
/// would be unreadable, and re-laying-out a text block every frame is
/// wasted work for something meant to be glanced at, not read live.
const DEBUG_OVERLAY_INTERVAL: Duration = Duration::from_millis(500);

/// Spawns the debug overlay's single text entity: a `Node` positioned
/// absolutely in the top-left corner (independent of the sprite/camera, so
/// it doesn't scale or move with the emulated screen) with a translucent
/// black background so the video output stays visible through it.
/// `update_debug_overlay` fills in the actual text -- starts empty so
/// nothing flashes on screen for one frame before the first tick.
pub(crate) fn setup_debug_overlay(mut commands: Commands) {
    commands.spawn((
        DebugOverlayText,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(14.0),
            ..default()
        },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(6.0),
            left: Val::Px(6.0),
            padding: UiRect::all(Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
    ));
}

/// Rewrites the overlay's text on a `DEBUG_OVERLAY_INTERVAL` timer (real
/// wall-clock time via `Time`, not `Time<Fixed>` -- this is a UI refresh
/// rate, unrelated to the emulation's own cycle pacing) rather than every
/// `Update` tick, since `PC`/registers changing 50-60 times a second would
/// be unreadable anyway.
pub(crate) fn update_debug_overlay(
    time: Res<Time>,
    snapshot: Res<DebugSnapshotRes>,
    mut timer: Local<Option<Timer>>,
    mut text: Query<&mut Text, With<DebugOverlayText>>,
) {
    let timer =
        timer.get_or_insert_with(|| Timer::new(DEBUG_OVERLAY_INTERVAL, TimerMode::Repeating));
    timer.tick(time.delta());
    if !timer.just_finished() {
        return;
    }

    let snapshot = snapshot.0.lock().unwrap();
    let mut out = String::from(if snapshot.halted { "HALTED\n" } else { "" });
    for pair in snapshot.registers.chunks(2) {
        match pair {
            [(n1, v1), (n2, v2)] => {
                out.push_str(&format!("{n1:<4} 0x{v1:04X}  {n2:<4} 0x{v2:04X}\n"))
            }
            [(n1, v1)] => out.push_str(&format!("{n1:<4} 0x{v1:04X}\n")),
            _ => unreachable!(),
        }
    }

    if let Ok(mut text) = text.single_mut() {
        text.0 = out;
    }
}
```

- [ ] **Step 6: Write `apps/retroclawd/src/gui/input.rs`**

```rust
use std::sync::mpsc::Sender;

use bevy::input::keyboard::KeyCode;
use bevy::prelude::*;

use crate::any_machine;

/// Channel the emu thread reads queued key events from -- see
/// `emu_thread::tick`'s doc comment. `keyboard_input` below is the only
/// system that sends into it.
#[derive(Resource)]
pub(crate) struct InputTxRes(pub(crate) Sender<(KeyCode, bool)>);

/// Every key drives its own emulated key 1:1, EXCEPT the two real Shift
/// keys: those, together with the arrow keys (real hardware's Shift+digit
/// cursor combo -- see `any_machine::zx81_key`/`spectrum_key`), all
/// contribute to one combined "is shift down" state instead, computed
/// fresh every frame from live key state rather than edge-triggered
/// per-key. Edge-triggering each source independently would double-drive
/// the emulated Shift key and, worse, mis-release it: e.g. holding a real
/// Shift key, then pressing and releasing an arrow key, must leave Shift
/// still down -- an arrow key's own release-edge calling `key_event(false)`
/// on the shared Shift key would incorrectly clear it out from under the
/// still-held real Shift key if each source triggered it independently.
///
/// Sends events over `InputTxRes` rather than calling the machine
/// directly -- the machine lives on the emu thread now (see
/// `emu_thread::tick`), not in this ECS world. A send failure (the emu
/// thread's receiver already gone, e.g. mid-shutdown) is silently
/// dropped -- there's nothing useful to do about it from here, and the
/// app is already on its way down in that case.
pub(crate) fn keyboard_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    input_tx: Res<InputTxRes>,
    mut shift_was_down: Local<bool>,
) {
    // Arrow keys DO still go through here (their digit half -- see
    // `zx81_key`/`spectrum_key`); only the two literal Shift keys are
    // excluded, since they'd otherwise drive the same emulated Shift key
    // the combined check below also drives.
    let is_real_shift_key = |key: &KeyCode| matches!(key, KeyCode::ShiftLeft | KeyCode::ShiftRight);
    for key in keyboard.get_just_pressed() {
        if !is_real_shift_key(key) {
            let _ = input_tx.0.send((*key, true));
        }
    }
    for key in keyboard.get_just_released() {
        if !is_real_shift_key(key) {
            let _ = input_tx.0.send((*key, false));
        }
    }

    let shift_is_down = any_machine::SHIFT_SOURCE_KEYS
        .iter()
        .any(|k| keyboard.pressed(*k));
    if shift_is_down != *shift_was_down {
        let _ = input_tx.0.send((KeyCode::ShiftLeft, shift_is_down));
        *shift_was_down = shift_is_down;
    }
}
```

- [ ] **Step 7: Write `apps/retroclawd/src/gui/mod.rs`**

```rust
use bevy::audio::AddAudioSource;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, Window, WindowMode, WindowPlugin};

use crate::any_machine::AnyMachine;
use crate::emu_thread;

mod audio;
mod debug_overlay;
mod input;
mod video;
mod window;

use audio::AudioQueueRes;
use debug_overlay::DebugSnapshotRes;
use input::InputTxRes;
use video::{BaseResolution, FrameBufferRes};

/// Opens a live window showing `machine`'s video output and driving its
/// keyboard from window input. Blocks until the window is closed -- same
/// "owns the whole run" shape as `Machine::run()`, just Bevy's own loop
/// instead. Panics if `machine.render_frame()` is `None` -- callers must
/// check that before calling this (see `main.rs`'s GUI branch, which
/// checks up front with the same error message `--dump-screen` uses for
/// this case).
///
/// CPU stepping, audio generation, and frame production all happen on a
/// dedicated background thread (`emu_thread::spawn`), not on this Bevy
/// app's own thread -- a stall anywhere in Bevy's render/window handling
/// (a vsync wait, a slow present) used to also stall audio generation,
/// since both ran on the same thread; moving the machine off this thread
/// entirely means a render-side stall can no longer starve audio. See
/// `docs/superpowers/specs/2026-09-09-emu-thread-tracy-design.md`.
///
/// `show_debug_info` turns on a semi-transparent register/PC/HALT overlay
/// in the top-left corner, updated every `DEBUG_OVERLAY_INTERVAL`. This is
/// a `--debug-info` CLI flag rather than a runtime keyboard toggle
/// deliberately: every key on this window drives the emulated machine's
/// own keyboard (see `input::keyboard_input`), so there's no key left free
/// to reserve for a toggle without either stealing it from the emulated
/// keyboard or picking one that collides with a real ZX81/Spectrum key.
///
/// `start_fullscreen` opens the window already in `BorderlessFullscreen`
/// mode (at the current monitor's own resolution) instead of windowed.
/// Either way, F11 toggles fullscreen live (see `window::toggle_fullscreen`)
/// -- unlike the debug overlay, F-keys aren't mapped to any emulated key on
/// either machine, so there's no keyboard conflict to avoid here.
pub fn run_gui(machine: AnyMachine, show_debug_info: bool, start_fullscreen: bool) {
    let (width, height, _) = machine
        .as_machine()
        .render_frame()
        .expect("caller must check render_frame().is_some() before calling run_gui");
    crate::debug_log!(
        "run_gui: {width}x{height} @ {}Hz, clock_hz={}",
        machine.as_machine().frame_hz(),
        machine.as_machine().clock_hz()
    );

    // `MonitorSelection::Current` only resolves relative to an existing
    // window (used by `window::toggle_fullscreen`, where one already
    // exists) -- at initial window creation there's no window yet to be
    // "current" relative to, so this needs `Primary` instead (confirmed
    // live: using `Current` here logs "Can't select current monitor on
    // window creation").
    let mode = if start_fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Primary)
    } else {
        WindowMode::Windowed
    };

    // Moves `machine` onto its own thread -- see `emu_thread`'s own doc
    // comment. `handle` is a plain local, not an ECS resource: `app.run()`
    // below blocks until the window closes, and dropping `handle` at the
    // end of this function (after `app.run()` returns) is what stops and
    // joins the thread -- see `EmuThreadHandle`'s `Drop` impl.
    let handle = emu_thread::spawn(machine);
    let audio_queue = handle.audio_queue.clone();
    let frame_buffer = handle.frame_buffer.clone();
    let debug_snapshot = handle.debug_snapshot.clone();
    let input_tx = handle.input_tx.clone();

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "retroclawd".to_string(),
                resolution: (
                    (width as f32 * video::INITIAL_SCALE) as u32,
                    (height as f32 * video::INITIAL_SCALE) as u32,
                )
                    .into(),
                mode,
                ..default()
            }),
            ..default()
        }),
    )
    .insert_resource(ClearColor(Color::BLACK))
    .insert_resource(BaseResolution { width, height })
    .insert_resource(AudioQueueRes(audio_queue))
    .insert_resource(FrameBufferRes(frame_buffer))
    .insert_resource(DebugSnapshotRes(debug_snapshot))
    .insert_resource(InputTxRes(input_tx))
    .add_audio_source::<audio::EmulatorAudio>()
    .add_systems(Startup, (video::setup, audio::setup_audio))
    .add_systems(
        Update,
        (
            video::render_screen,
            input::keyboard_input,
            video::fit_screen,
            window::toggle_fullscreen,
            window::set_window_icon,
        ),
    );

    if show_debug_info {
        app.add_systems(Startup, debug_overlay::setup_debug_overlay)
            .add_systems(Update, debug_overlay::update_debug_overlay);
    }

    app.run();
    // `handle` drops here: stops the emu thread and joins it before this
    // function returns.
}
```

- [ ] **Step 8: Build**

Run: `cargo build -p retroclawd`
Expected: succeeds. Fix any import/path mistakes surfaced here before continuing (this is a large mechanical split -- compiler errors here are expected on the first pass, not a sign the design is wrong).

- [ ] **Step 9: Lint**

Run: `cargo clippy -p retroclawd -- -D warnings`
Expected: clean.

- [ ] **Step 10: Manual smoke test**

Run each of these and confirm: window opens at the right scale, resizing the
window rescales the picture, F11 toggles fullscreen, the taskbar/title-bar
icon is the real app icon, keyboard input (including arrow keys) works, and
-- the actual point of this task -- audio no longer skips during
`USPIRITS.Z80`:

```
cargo run --release -p retroclawd -- --machine zx-spectrum-128 --rom .\roms\zx-spectrum\128.rom --snapshot ".\software\zxspectrum\USPIRITS.Z80"
```

Also smoke-test `--debug-info` (confirm the overlay still shows real
register/PC/HALT values, refreshing twice a second):

```
cargo run --release -p retroclawd -- --machine zx-spectrum-128 --rom .\roms\zx-spectrum\128.rom --snapshot ".\software\zxspectrum\USPIRITS.Z80" --debug-info
```

- [ ] **Step 11: Run the full test suite**

Run: `cargo test --workspace`
Expected: all tests pass (this task doesn't add workspace tests beyond Tasks 1-2's, but confirms the split didn't break anything elsewhere).

- [ ] **Step 12: Commit**

```bash
git add apps/retroclawd/src
git commit -m "refactor(retroclawd): split gui.rs into gui/{audio,video,window,debug_overlay,input}.rs

Reads/writes go through emu_thread's shared queue/buffer/channel instead
of a direct MachineRes resource, now that the machine runs on its own
thread. gui/mod.rs is assembly only.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 4: Tracy profiling wiring

**Files:**
- Modify: `apps/retroclawd/Cargo.toml`

**Interfaces:**
- Consumes: the `#[cfg(feature = "profiling")] let _span = tracing::info_span!(...)` line already written inside `emu_thread.rs`'s `tick` function in Task 1 (it doesn't compile until this task adds `tracing` as a dependency).
- Produces: a `profiling` Cargo feature, off by default.

- [ ] **Step 1: Add the feature and dependency**

Edit `apps/retroclawd/Cargo.toml`:

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
png = "0.18"
winit = "0.30"
bevy = { version = "0.19", default-features = false, features = ["2d", "keyboard", "bevy_ui", "bevy_text", "bevy_ui_render", "default_font", "bevy_audio"] }
tracing = { version = "0.1", optional = true }
machine = { path = "../../crates/machine" }
zx81 = { path = "../../crates/zx81" }
zx-spectrum = { path = "../../crates/zx-spectrum" }
z80-snapshot = { path = "../../crates/z80-snapshot" }
zx81-snapshot = { path = "../../crates/zx81-snapshot" }

[features]
profiling = ["bevy/trace_tracy", "dep:tracing"]

[build-dependencies]
winresource = "0.1"
```

(`bevy/trace_tracy` -- confirmed against bevy 0.19's own `Cargo.toml`: it
enables `["trace", "bevy_internal/trace_tracy", "debug"]`, which in turn
adds `bevy_internal/trace` + `dep:tracing` -- i.e. it wires up Tracy's
transport for Bevy's own internal schedule/system spans. It does **not**
pull in `bevy_log`/`bevy::log`'s span macros, which is why this app adds a
direct `tracing` dependency of its own for the one manual span in
`emu_thread.rs`, rather than relying on a bevy re-export.)

- [ ] **Step 2: Build with the feature on**

Run: `cargo build --release -p retroclawd --features profiling`
Expected: succeeds -- this is the build that actually has a Tracy transport
active.

- [ ] **Step 3: Build with the feature off (the default) and confirm nothing regressed**

Run: `cargo build --release -p retroclawd`
Expected: succeeds, identical to before this task (the `profiling` feature
is additive-only).

- [ ] **Step 4: Record how to actually use it (for later, not part of this task's automated verification)**

Attaching Tracy's own viewer application (a separate download from
<https://github.com/wolfpld/tracy/releases>, not something this repo
installs) to a run built with `--features profiling` is how you'll compare
before/after this change -- the "before" comparison isn't captured
automatically anywhere, since Task 3 already replaced the old
single-thread code path; if a true before/after capture matters, it would
need to be done against a checkout of the commit right before Task 3's
commit, with the `profiling` feature added there too. This step is a note
for the user's own manual verification, not an automated check.

- [ ] **Step 5: Commit**

```bash
git add apps/retroclawd/Cargo.toml
git commit -m "feat(retroclawd): wire up Tracy profiling behind a profiling feature

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Post-plan cleanup

- [ ] Update `CLAUDE.md`'s `retroclawd` section to describe the emu thread
      architecture (replacing the now-inaccurate description of
      `step_machine`/`drain_audio_into_queue` running in `FixedUpdate` on
      the Bevy thread) and the new `gui/` file layout, once all four tasks
      are done and verified. Not its own task above since it's pure
      documentation with no test cycle of its own -- fold it into whichever
      of the above tasks' commits is most convenient, or do it as one final
      commit after Task 4.
