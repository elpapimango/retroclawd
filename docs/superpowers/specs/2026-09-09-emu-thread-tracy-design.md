# Dedicated emulation thread + Tracy profiling -- design

## Problem

`apps/retroclawd`'s GUI runs CPU stepping, audio-buffer generation, and
frame rendering on the same OS thread, inside one `bevy::App::run()` loop
(`FixedUpdate` steps the machine and drains audio; `Update` renders and
handles the window). Real hardware/OS stalls on that thread -- a vsync
wait, a slow present, a GPU driver hiccup -- block `FixedUpdate` too, so
`Machine::drain_audio` doesn't refill `EmulatorAudioDecoder`'s queue for
however long the stall lasts. The decoder returns silence (`0.0`) on
underrun rather than blocking, so the gap is audible as a skip. Observed
running `.\software\zxspectrum\USPIRITS.Z80` under `--machine
zx-spectrum-128`.

Confirmed by reading `apps/retroclawd/src/gui.rs`'s current schedule setup
(`step_machine`/`drain_audio_into_queue` in `FixedUpdate`, `render_screen`
in `Update`, both on the one thread `app.run()` drives) rather than
guessed -- no separate profiling run was needed to find the mechanism, but
Tracy wiring (below) lets us confirm/quantify it and verify the fix.

## Goals

- Eliminate audio underrun caused by render-thread stalls, for every
  machine (`zx81`, `zx-spectrum`, and any future one) with no per-machine
  code -- the fix must live against the `Machine`/`AnyMachine` interface.
- Wire up Tracy profiling (`bevy/trace_tracy`, feature-gated, zero cost by
  default) so before/after captures can confirm the fix actually helped.
- Split `gui.rs` (559 lines today, growing) into focused files by concern.

## Non-goals

- Headless mode (`--headless`) is untouched -- it has no render thread to
  contend with, so it never had this bug. `machine::Clock`/`Machine::run()`
  (what headless uses) are not modified.
- ZX81 stays parked (per `TODO.md`) -- this work isn't ZX81-specific and
  doesn't change its priority.
- Not a general async-ECS rewrite of the GUI -- only CPU stepping, audio
  generation, and frame production move off the render thread.

## Architecture

### `apps/retroclawd/src/emu_thread.rs` (new)

Bevy-agnostic -- depends only on `std` + the `machine`/`AnyMachine`
interface, so it's exactly as generic across machines as `AnyMachine`
itself already is.

```rust
pub struct EmuThreadHandle {
    pub audio_queue: AudioQueue,               // Arc<Mutex<VecDeque<f32>>>, same type gui/audio.rs already defines
    pub frame_buffer: FrameBuffer,             // Arc<Mutex<Option<(u32, u32, Vec<u8>)>>>
    pub debug_snapshot: DebugSnapshotHandle,   // Arc<Mutex<DebugSnapshot>>
    pub input_tx: mpsc::Sender<(KeyCode, bool)>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

pub struct DebugSnapshot {
    pub halted: bool,
    pub registers: Vec<(&'static str, u64)>,
}

pub fn spawn(machine: AnyMachine) -> EmuThreadHandle { ... }
```

`spawn` moves `machine` into a `std::thread::spawn` closure running a loop
that, each tick:

1. Drains `input_rx` (non-blocking `try_recv` loop) and applies each event
   via `AnyMachine::key_event` -- moves keyboard handling off the Bevy
   thread too, since it's the same `&mut AnyMachine` the stepping loop
   owns.
2. Calls `request_interrupt()`, then steps in a cycle-budget loop --
   identical math to today's `step_machine` (`clock_hz() / frame_hz()`
   per tick, `max(1)`), just running here instead.
3. Drains audio (`drain_audio()`) into `audio_queue`, same
   drop-oldest-past-`MAX_QUEUED_AUDIO_SAMPLES` behavior as today.
4. Writes `render_frame()`'s output into `frame_buffer`.
5. Writes `halted()` + `registers()` into `debug_snapshot` (cheap enough
   to do unconditionally every tick rather than gate on `--debug-info`;
   `update_debug_overlay` already only *reads* it every 500ms).
6. Sleeps off whatever real time the tick finished early by, same
   accumulator approach `machine::Clock::run` already uses -- this thread
   is effectively a `Clock::run` loop with audio/frame/debug side effects
   added at each tick, not a new pacing model.
7. Checks `stop` (`Ordering::Relaxed` is fine -- timing-sensitive, not
   correctness-sensitive) and exits the loop if set.

The per-tick body (steps 1-5) is a free function taking `&mut AnyMachine`
and the shared handles, callable directly from unit tests without
spawning a thread or sleeping -- `spawn`'s closure is just that function
in a `loop { ...; sleep(...) }` wrapper. This keeps the actual logic
testable the same way `machine::Clock`'s tests avoid real timing by
calling `run` with tiny, fast-terminating fake machines.

`EmuThreadHandle` sets `stop` and joins in its `Drop` impl, so the thread
always winds down when the handle (owned by a Bevy resource) is dropped
at app exit -- no explicit shutdown call needed at the call site.

### `gui/` module split

`gui.rs` becomes `gui/mod.rs`, assembly only:

```
apps/retroclawd/src/
  emu_thread.rs
  gui/
    mod.rs             # run_gui(): builds App, wires plugins/resources/systems
    audio.rs           # EmulatorAudio/EmulatorAudioDecoder/Decodable/Source, setup_audio, AudioQueueRes
    video.rs           # texture setup, render_screen, rgb_to_rgba, ScreenTexture, BaseResolution, fit_screen, ScreenSprite
    window.rs          # set_window_icon, toggle_fullscreen
    debug_overlay.rs   # setup_debug_overlay, update_debug_overlay, DebugOverlayText, DEBUG_OVERLAY_INTERVAL
    input.rs           # keyboard_input (sends over input_tx instead of calling the machine directly)
```

Each resource type (`AudioQueueRes`, `FrameBufferRes`, `DebugSnapshotRes`,
`InputTxRes`) is defined next to the one module that reads it;
`gui/mod.rs`'s `run_gui` takes `EmuThreadHandle` apart and inserts each
piece as its own resource, so no module other than `mod.rs` needs to know
`EmuThreadHandle` exists. `MachineRes` (today's `AnyMachine` resource) goes
away entirely from the Bevy side -- the machine now lives only on the emu
thread.

`step_machine`/`drain_audio_into_queue` are deleted (their job moved to
`emu_thread.rs`); `FixedUpdate` in `run_gui` is removed unless something
else still needs it (nothing does after this change).

### Removed vs. kept behavior

- `render_screen` reads `FrameBufferRes` instead of calling
  `machine.render_frame()`.
- `keyboard_input` sends `(KeyCode, bool)` pairs over `InputTxRes` instead
  of calling `machine.key_event` directly; the Shift-combining logic
  (arrow keys + real Shift keys -> one synthesized Shift edge) is
  unchanged, it just sends its synthesized events instead of calling
  `key_event` in place.
- `update_debug_overlay` reads `DebugSnapshotRes` instead of `MachineRes`;
  same 500ms-timer gating as today.
- `--snapshot` loading is unchanged -- still applied to `AnyMachine` in
  `main.rs` before the machine is handed to `emu_thread::spawn`.

## Tracy profiling

`apps/retroclawd/Cargo.toml` gets a `profiling` feature forwarding to
`bevy/trace_tracy` (verify the exact bevy-0.19 feature name/any extra
`tracing-tracy` version pinning against docs.rs during implementation,
not assumed). Off by default -- normal builds pay nothing. Usage:

```
cargo run --release --features profiling -- --machine zx-spectrum-128 --rom .\roms\zx-spectrum\128.rom --snapshot '.\software\zxspectrum\USPIRITS.Z80'
```

then attach the Tracy viewer (a separate GUI application from the Tracy
project -- not something this change installs or launches; the user runs
it themselves). Bevy already spans its own schedules/systems once
`trace_tracy` is enabled; `emu_thread`'s per-tick loop gets one manual
`tracy_client::span!` (or the `tracing` macro Bevy itself uses, whichever
`trace_tracy` actually wires up -- confirm during implementation) around
step 2 (the step-until-budget loop) so its cost shows up alongside Bevy's
own spans in the same capture, making a before/after comparison direct
rather than inferred.

## Testing approach

- `emu_thread.rs`'s per-tick function: unit-tested directly (no thread, no
  sleep) against a real `AnyMachine` built the same way other tests in
  this codebase construct one, asserting audio gets pushed, the frame
  buffer gets written, and queued input events get applied before the
  step budget runs -- mirrors how `machine::Clock`'s own tests avoid real
  timing.
- `EmuThreadHandle::spawn`/shutdown: one integration-style test that
  spawns a real thread against a tiny/fast machine, drops the handle, and
  asserts the thread actually joins within a bounded timeout -- proves
  `Drop` really stops the thread rather than leaking it.
- Existing `gui.rs` had no tests (Bevy systems aren't unit-tested in this
  codebase today) -- the new `gui/*.rs` modules follow the same pattern;
  correctness there is verified by running the app, not automated tests.
