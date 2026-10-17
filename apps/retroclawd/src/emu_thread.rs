use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bevy::input::mouse::MouseButton;
use machine::Machine;

use crate::debugger::{DebuggerCommandSender, DebuggerLink, DebuggerStateHandle};
use crate::debuggers::format_debug_value;
use crate::plugin_bridge::HostedMachine;

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

/// How often `--show-overlay`'s memory/disk/tape panel data (a
/// `GetChipSnapshots` plugin round trip plus a small `read_memory` loop)
/// is refreshed from the outer tick loop -- deliberately not every tick
/// (50-60Hz): matches `gui::debug_overlay::DEBUG_OVERLAY_INTERVAL`'s own
/// "glanced at, not read live" refresh rate, so this never fetches data
/// the GUI side wouldn't even redraw yet.
const OVERLAY_DATA_INTERVAL: Duration = Duration::from_millis(500);

/// Byte width of the `--show-overlay` memory panel's hex-dump window,
/// read starting at the current PC.
const MEMORY_HEX_WINDOW: u16 = 16;

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
    /// Small fixed-size memory window at the current PC, for the
    /// `--show-overlay` memory panel's hex dump -- refreshed at
    /// `OVERLAY_DATA_INTERVAL`, independent of `halted`/`registers`'
    /// every-tick refresh rate (see `tick`'s own publish site, which
    /// mutates only those two fields so this one survives between the
    /// slower refreshes).
    pub(crate) memory_hex: Vec<u8>,
    pub(crate) memory_hex_addr: u16,
    /// Machine-agnostic memory/disk/tape overlay telemetry -- see
    /// `plugin_bridge::OverlayDebugSnapshot`'s own doc comment. Same
    /// slower-refresh-rate reasoning as `memory_hex`.
    pub(crate) overlay: crate::plugin_bridge::OverlayDebugSnapshot,
}

pub(crate) type DebugSnapshotHandle = Arc<Mutex<DebugSnapshot>>;

fn register_value(registers: &[(&'static str, u64)], name: &str) -> Option<u64> {
    registers
        .iter()
        .find(|(reg_name, _)| *reg_name == name)
        .map(|(_, value)| *value)
}

fn is_cpc_firmware_boot_pc(pc: u64) -> bool {
    let pc = pc as u16;
    // Heuristic "warm reboot" window for CPC firmware flow: low reset
    // vectors plus boot-window PCs repeatedly seen in captured traces.
    // Includes both `0x0C14`-style and `0xC14D`-style addresses.
    pc <= 0x01FF || (0x0C00..=0x0DFF).contains(&pc) || (0xC000..=0xC3FF).contains(&pc)
}

/// Cumulative count of audio-queue underruns (the audio backend asking for
/// a sample when the queue was empty) -- incremented from the audio
/// backend's own real-time thread in `gui::audio::EmulatorAudioDecoder`,
/// read back here once/second for debugger-plugin diagnostics (see `spawn`'s
/// thread loop). Diagnostic only: whether this is climbing or flat is
/// evidence for root-causing reported audio skipping, not something the
/// app itself reacts to.
pub(crate) type UnderrunCount = Arc<AtomicU64>;

/// Cumulative count of every sample the audio backend has requested
/// (successful pops and underruns both), incremented alongside
/// `UnderrunCount` from the same real-time callback thread. Diagnostic
/// only, same reasoning as `UnderrunCount` -- lets the 1Hz diagnostics log
/// show the backend's *actual* observed pull rate (`requested` delta per
/// second) directly, rather than assuming it matches the declared
/// `machine::AUDIO_SAMPLE_RATE`. A backend resampling our declared rate to
/// a different real output device rate would show up here as a rate other
/// than 44100/sec, which the underrun count alone can't distinguish from
/// a genuine production stall.
pub(crate) type RequestedCount = Arc<AtomicU64>;

/// The key events to apply this tick, taken off the front of `pending`. A
/// *press* that follows a *release* waits for the next tick (and everything
/// behind it with it, so order holds).
///
/// The host sends "release Shift, press the key" for a chord such as
/// Shift+8 = `*`. Applied together, the switch can land in the middle of the
/// machine's own keyboard scan (the C64 KERNAL reads the matrix column by
/// column in its IRQ), which then sees the old Shift and the new key at once
/// and decodes the key shifted -- 39 of 870 switch offsets in
/// `c64/tests/key_switch_race.rs`. One tick between the two leaves no scan
/// that can see both.
fn take_key_batch(pending: &mut VecDeque<(String, bool)>) -> Vec<(String, bool)> {
    let mut batch = Vec::new();
    let mut released = false;
    while pending
        .front()
        .is_some_and(|(_, pressed)| !(*pressed && released))
    {
        let (key, pressed) = pending.pop_front().expect("front was just checked");
        released |= !pressed;
        batch.push((key, pressed));
    }
    batch
}

/// One tick's worth of work: apply any queued input, step the machine for
/// one `frame_hz()` tick's full cycle budget, then publish audio/frame/debug
/// state for the Bevy side to pick up. Kept as a plain function (not inlined
/// into `spawn`'s loop, see Task 2) so it's testable directly, without a
/// real thread or real sleeping.
/// A single tick taking longer than this in wall-clock time eats visibly
/// into the ~20ms (at 50Hz) real-time budget the pacing loop expects --
/// worth an explicit diagnostics log line with a phase breakdown, since
/// reported audio skipping traced back to a queue that's genuinely running
/// dry (see `UnderrunCount`'s doc comment) rather than a steady drift.
const SLOW_TICK_THRESHOLD: Duration = Duration::from_millis(10);

/// Advances `m` through exactly one `frame_hz()` tick: raises the frame
/// interrupt, then burns a full `clock_hz()/frame_hz()` cycle budget --
/// deliberately never breaking early on `m.halted()` (see the comment
/// below for the audio-underrun bug this fixes; the same reasoning applies
/// to a speculative run-ahead tick's timing, not just real audio). Shared
/// by the real tick and every speculative run-ahead tick in `tick` below,
/// so both advance identically -- a correctness/timing fix to one can't
/// silently diverge from the other.
///
/// `should_break` is called after every `m.step()` (with the machine in its
/// post-instruction state) and returning `true` stops the loop before the
/// tick's full cycle budget is spent -- generic hook used by callers that
/// need an early-stop condition. `|_| false` means "never stop early",
/// bit-identical to this function's own behavior before breakpoints existed
/// -- a monomorphized generic closure, not `&mut dyn FnMut`, so the compiler
/// inlines a trivial `|_| false` away entirely, same "zero cost when off"
/// precedent as `run_ahead_frames == 0`. The speculative run-ahead call
/// site always passes `|_| false`, unconditionally, so a speculative clone
/// can never trigger or be stopped by a real-machine breakpoint.
pub(crate) fn advance_one_frame(
    m: &mut dyn Machine,
    mut should_break: impl FnMut(&dyn Machine) -> bool,
) -> u64 {
    // Once per tick, matching real hardware's frame interrupt (see
    // `Machine::request_interrupt`'s doc comment). Only sets a pending
    // flag -- doesn't clear `halted()` by itself, so the loop below must
    // call `step()` at least once even if `m.halted()` is currently true,
    // or a HALT-with-interrupts-enabled ROM loop would never get the
    // chance to wake back up.
    m.request_interrupt();

    let budget = (m.clock_hz() / m.frame_hz()).max(1);
    let mut spent = 0u64;
    // Deliberately does NOT break early on `m.halted()` -- root cause of a
    // real, reported audio-skipping bug, found by reading `ZxSpectrum::
    // step`: its audio sample generation is driven purely by the cycle
    // count each `step()` call returns (see `audio_cycle_accumulator`'s
    // doc comment there), so a step loop that stops calling `step()` the
    // moment the CPU re-halts mid-tick silently stops producing audio for
    // the rest of that tick's real 20ms too, even though real hardware
    // keeps burning T-states (and therefore keeps needing new sample
    // output) the whole time it idles in HALT. A game whose main loop is
    // EI/HALT-per-frame (the common case -- most work happens in the
    // interrupt handler, which re-HALTs almost immediately after
    // returning) hit this on nearly every single tick, which is why the
    // underrun rate stayed essentially constant no matter what else got
    // fixed. `m.step()` while halted is cheap (a constant no-op instruction
    // step on current machine plugins), so always spending the full budget costs
    // nothing worth avoiding.
    loop {
        spent += m.step() as u64;
        if should_break(m) {
            break;
        }
        if spent >= budget {
            break;
        }
    }
    spent
}
/// Debugger-enabled sibling to `advance_one_frame`: same frame interrupt +
/// full cycle-budget stepping, but checks debugger breakpoints after each
/// real instruction and pauses as soon as one matches.
fn advance_one_frame_with_debugger(m: &mut HostedMachine, dbg: &mut DebuggerLink) -> u64 {
    m.request_interrupt();

    let budget = (m.clock_hz() / m.frame_hz()).max(1);
    let mut spent = 0u64;
    loop {
        let cycles = m.step() as u64;
        spent += cycles;
        dbg.record_cycles(cycles);
        if dbg.on_post_step_running(m) {
            break;
        }
        if spent >= budget {
            break;
        }
    }
    spent
}

/// One Kempston-mouse-shaped input event, queued from the GUI's own
/// mouse-handling system (`gui::input::mouse_input`) into the emu thread --
/// mirrors `(KeyCode, bool)`'s "GUI reads real input, emu thread applies
/// it" split, just carrying three different real mouse event shapes
/// (relative motion, a button edge, a scroll tick) instead of one.
pub(crate) enum MouseEvent {
    /// Relative motion since the last event, already clamped to `i8`
    /// range -- see `mouse_input`'s own doc comment for why this is a
    /// relative delta, not an absolute position, and why Y isn't
    /// inverted (an unverified engineering choice, not a confirmed real
    /// Kempston-mouse-driver convention).
    Motion {
        dx: i8,
        dy: i8,
    },
    Button {
        button: MouseButton,
        pressed: bool,
    },
    /// One scroll-wheel tick, sign gives direction -- see `zx_spectrum::
    /// ZxSpectrum::scroll_mouse`'s own doc comment for why this
    /// accumulates rather than sets an absolute value.
    Scroll {
        delta: i8,
    },
}

/// A host-level action the GUI's hotkey system asks the emu thread to carry
/// out (see `gui::hotkeys`). Everything here touches the machine or the
/// files written for it, so it runs on the emu thread, between ticks.
#[derive(Debug, Clone)]
pub(crate) enum EmuCommand {
    /// `Machine::reset` (CPU only, RAM kept -- the RESET button).
    Reset,
    /// The cartridge's freeze button.
    Freeze,
    TogglePause,
    /// Save the current frame as a PNG under `screenshots/`.
    Screenshot,
    /// Save/load the machine state of the current slot under `states/`.
    QuickSave,
    QuickLoad,
    /// Move the quick-save slot by this many places (wraps 0-9).
    StateSlot(i8),
    /// Hold-to-run-faster on/off.
    FastForward(bool),
    /// A file dialog (or its in-app prompt) is open (`true`) or done
    /// (`false`): the machine holds still and is silent meanwhile, like a
    /// pause the player did not ask for, and a real pause is not undone by it.
    Dialog(bool),
    /// Put media in, take it out, load or replace saved machine state.
    Media(MediaOp),
    /// Asks whether the disk has unsaved writes (answered on the channel).
    DiskStatus(Sender<DiskStatus>),
    /// Serialises something for a "save" dialog (answered on the channel;
    /// `Ok(None)` means there is nothing to save).
    Export(Export, Sender<Result<Option<Vec<u8>>, String>>),
}

/// Media changes requested from a file dialog. The bytes were read on the
/// dialog thread; the emu thread (which owns the machine) applies them and
/// reports the outcome as a toast.
#[derive(Debug, Clone)]
pub(crate) enum MediaOp {
    AttachTape {
        extension: String,
        bytes: Vec<u8>,
    },
    AttachDisk {
        extension: String,
        bytes: Vec<u8>,
        /// The file the image came from (where "overwrite original" goes).
        path: std::path::PathBuf,
    },
    EjectTape,
    EjectDisk,
    LoadSnapshot {
        extension: String,
        bytes: Vec<u8>,
    },
    LoadState {
        bytes: Vec<u8>,
    },
    /// The disk image was written out: its changes are no longer unsaved.
    DiskSaved,
    /// The same for the second drive (drive B of a CPC664/6128 or +3). Replacing or ejecting a drive B disk with unsaved changes is
    /// refused unless the changes can be written to the `--disk-b` file or `--disk-b-save` first.
    AttachDiskB {
        extension: String,
        bytes: Vec<u8>,
        path: std::path::PathBuf,
    },
    EjectDiskB,
    DiskBSaved,
}

/// What a "save" dialog wants serialised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Export {
    /// The drive's current image, guest writes applied (modified or not).
    Disk,
    /// The same for drive B.
    DiskB,
    /// The cassette recording as `cdt`/`tzx`/`voc`.
    Tape {
        format: String,
    },
    /// The internal `.state` format.
    State,
    Snapshot {
        extension: String,
    },
}

/// Answer to `EmuCommand::DiskStatus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiskStatus {
    /// The guest wrote to the disk since it was attached or last saved.
    pub(crate) modified: bool,
    /// The file the disk was loaded from, when known.
    pub(crate) original: Option<std::path::PathBuf>,
}

/// What the media actions need to know about how the session was started.
#[derive(Debug, Clone, Default)]
pub(crate) struct MediaSettings {
    /// Turbo multiplier for tapes loaded from a dialog (`--tape-speed`).
    pub(crate) tape_speed: u32,
    /// `--disk-writeback`: a disk swapped in is written back to its own file
    /// on exit, like the one given on the command line would have been.
    pub(crate) disk_writeback: bool,
    /// `--disk-save`: where modified disks go on exit.
    pub(crate) disk_save: Option<std::path::PathBuf>,
    /// The `--disk` file the session started with.
    pub(crate) disk_path: Option<std::path::PathBuf>,
    /// `--disk-b-save`: where a modified drive B image goes.
    pub(crate) disk_b_save: Option<std::path::PathBuf>,
    /// The `--disk-b` file (or the last disk put in drive B).
    pub(crate) disk_b_path: Option<std::path::PathBuf>,
    /// What was loaded, as a file-name friendly stem (`host_files::game_stem`): screenshots and quick-save states are named
    /// `<machine>-<game>-...` so two games on one machine keep separate slots. `None` when nothing was loaded.
    pub(crate) game: Option<String>,
}

/// Short messages for the player ("State saved: slot 2", "Paused", errors),
/// pushed by the emu thread and shown by the GUI's toast system.
pub(crate) type Messages = Arc<Mutex<VecDeque<String>>>;

/// How many ticks fast-forward runs per real tick period (the pacing sleep
/// is divided by this; the machine itself is not skipped or approximated).
const FAST_FORWARD_FACTOR: u32 = 8;

/// State the emu thread keeps for the hotkey actions, owned by the thread
/// and handed to `tick` -- not shared with the GUI, which only sends
/// `EmuCommand`s and reads `Messages`.
pub(crate) struct HostControl {
    run_ahead_frames: u32,
    machine_id: String,
    paused: bool,
    fast_forward: bool,
    /// Ticks run during the current fast-forward, so only every
    /// `FAST_FORWARD_FACTOR`-th one pays for rendering a frame.
    fast_forward_ticks: u32,
    slot: u8,
    /// Where `screenshots/` and `states/` live (the working directory).
    base_dir: std::path::PathBuf,
    messages: Messages,
    /// A file dialog or prompt is open (`EmuCommand::Dialog`).
    dialog_open: bool,
    media: MediaSettings,
    /// The plugin has crashed (reported once): the machine is held like a pause, since every further call would only fail.
    crashed: bool,
}

/// Queues a one-line message for the player (the GUI's toast shows it).
/// Shared with the file-dialog thread, which reports its own outcomes.
pub(crate) fn post_message(messages: &Messages, message: impl Into<String>) {
    let message = message.into();
    tracing::info!(target: "retroclawd::hotkeys", "{message}");
    let mut queue = messages.lock().unwrap();
    queue.push_back(message);
    while queue.len() > 8 {
        queue.pop_front();
    }
}

impl HostControl {
    pub(crate) fn new(
        run_ahead_frames: u32,
        machine_id: &str,
        messages: Messages,
        media: MediaSettings,
    ) -> Self {
        let file_tag = crate::host_files::game_tag(machine_id, media.game.as_deref());
        HostControl {
            dialog_open: false,
            crashed: false,
            media,
            run_ahead_frames,
            machine_id: file_tag,
            paused: false,
            fast_forward: false,
            fast_forward_ticks: 0,
            slot: 0,
            base_dir: std::path::PathBuf::from("."),
            messages,
        }
    }

    fn say(&self, message: impl Into<String>) {
        post_message(&self.messages, message);
    }

    /// Carries out one command. Returns `true` if the machine's picture or
    /// state may have changed outside a normal step (so the caller should
    /// republish the frame even while paused).
    pub(crate) fn apply(&mut self, command: EmuCommand, machine: &mut HostedMachine) -> bool {
        match command {
            EmuCommand::Reset => {
                tracing::warn!("machine reset requested: source=reset_hotkey");
                machine.reset();
                self.say("Reset");
                true
            }
            EmuCommand::Freeze => {
                match machine.freeze() {
                    Ok(()) => self.say("Freeze"),
                    Err(e) => self.say(format!("Freeze: {e}")),
                }
                true
            }
            EmuCommand::Dialog(open) => {
                self.dialog_open = open;
                false
            }
            EmuCommand::DiskStatus(reply) => {
                let modified = machine
                    .export_disk(0, true)
                    .is_ok_and(|image| image.is_some());
                let _ = reply.send(DiskStatus {
                    modified,
                    original: self.media.disk_path.clone(),
                });
                false
            }
            EmuCommand::Export(what, reply) => {
                let result = match what {
                    Export::State => Ok(Some(machine.save_state())),
                    Export::Snapshot { extension } => machine.save_snapshot(&extension).map(Some),
                    Export::Disk => machine.export_disk(0, false),
                    Export::DiskB => machine.export_disk(1, false),
                    Export::Tape { format } => machine.export_tape(&format),
                };
                let _ = reply.send(result);
                false
            }
            EmuCommand::Media(op) => self.apply_media(op, machine),
            EmuCommand::TogglePause => {
                self.paused = !self.paused;
                self.say(if self.paused { "Paused" } else { "Resumed" });
                false
            }
            EmuCommand::FastForward(on) => {
                self.fast_forward = on;
                self.fast_forward_ticks = 0;
                false
            }
            EmuCommand::StateSlot(delta) => {
                self.slot = crate::host_files::step_slot(self.slot, delta);
                let path = crate::host_files::state_path(
                    &self.base_dir.join("states"),
                    &self.machine_id,
                    self.slot,
                );
                let used = if path.exists() { "in use" } else { "empty" };
                self.say(format!("State slot {} ({used})", self.slot));
                false
            }
            EmuCommand::Screenshot => {
                match machine.render_frame() {
                    Some((width, height, rgb)) => self.write_screenshot(width, height, &rgb),
                    None => self.say("Screenshot failed: no picture"),
                }
                false
            }
            EmuCommand::QuickSave => {
                let path = crate::host_files::state_path(
                    &self.base_dir.join("states"),
                    &self.machine_id,
                    self.slot,
                );
                let result = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&path, machine.save_state()));
                match result {
                    Ok(()) => self.say(format!("State saved: slot {}", self.slot)),
                    Err(e) => self.say(format!("State save failed: {e}")),
                }
                false
            }
            EmuCommand::QuickLoad => {
                let path = crate::host_files::state_path(
                    &self.base_dir.join("states"),
                    &self.machine_id,
                    self.slot,
                );
                match std::fs::read(&path) {
                    Err(_) => {
                        self.say(format!("No saved state in slot {}", self.slot));
                        false
                    }
                    Ok(bytes) => match machine.load_state(&bytes) {
                        Ok(()) => {
                            self.say(format!("State loaded: slot {}", self.slot));
                            true
                        }
                        Err(e) => {
                            self.say(format!("State load failed: {e}"));
                            false
                        }
                    },
                }
            }
        }
    }

    /// Carries out one media change from a file dialog; reports the outcome
    /// as a toast. Returns whether the picture/state may have changed.
    fn apply_media(&mut self, op: MediaOp, machine: &mut HostedMachine) -> bool {
        match op {
            MediaOp::AttachTape { extension, bytes } => {
                match machine.apply_tape(&extension, &bytes, self.media.tape_speed.max(1)) {
                    Ok(()) => self.say("Tape loaded"),
                    Err(e) => self.say(format!("Tape load failed: {e}")),
                }
                true
            }
            MediaOp::AttachDisk {
                extension,
                bytes,
                path,
            } => {
                match machine.apply_disk(&extension, &bytes) {
                    Ok(()) => {
                        // A swapped-in disk is covered by the same exit policy
                        // as the one on the command line: under
                        // `--disk-writeback` its own file, else `--disk-save`'s
                        // target, else a warning if it was modified. Never a
                        // silent discard.
                        let save_to = if self.media.disk_writeback
                            && crate::inputs::is_writable_file(&path.to_string_lossy())
                        {
                            Some(path.clone())
                        } else {
                            self.media.disk_save.clone()
                        };
                        machine
                            .set_disk_exit_policy(crate::plugin_bridge::DiskExitPolicy { save_to });
                        self.media.disk_path = Some(path.clone());
                        self.say(format!("Disk loaded: {}", file_label(&path)));
                    }
                    Err(e) => self.say(format!("Disk load failed: {e}")),
                }
                true
            }
            MediaOp::EjectTape => {
                match machine.detach_media(plugin_api::MediaSlot::Tape) {
                    Ok(()) => self.say("Tape ejected"),
                    Err(e) => self.say(format!("Tape eject failed: {e}")),
                }
                true
            }
            MediaOp::EjectDisk => {
                match machine.detach_media(plugin_api::MediaSlot::Disk { drive: 0 }) {
                    Ok(()) => {
                        self.media.disk_path = None;
                        self.say("Disk ejected");
                    }
                    Err(e) => self.say(format!("Disk eject failed: {e}")),
                }
                true
            }
            MediaOp::LoadSnapshot { extension, bytes } => {
                match machine.apply_snapshot(&extension, &bytes) {
                    Ok(()) => self.say("Snapshot loaded"),
                    Err(e) => self.say(format!("Snapshot load failed: {e}")),
                }
                true
            }
            MediaOp::LoadState { bytes } => {
                match machine.load_state(&bytes) {
                    Ok(()) => self.say("State loaded"),
                    Err(e) => self.say(format!("State load failed: {e}")),
                }
                true
            }
            MediaOp::DiskSaved => {
                if let Err(e) = machine.mark_disk_saved(0) {
                    self.say(format!("Could not mark the disk saved: {e}"));
                }
                false
            }
            MediaOp::AttachDiskB {
                extension,
                bytes,
                path,
            } => {
                if !self.drive_b_can_be_replaced(machine, "replacing it") {
                    return false;
                }
                match machine.apply_disk_in(1, &extension, &bytes) {
                    Ok(()) => {
                        let save_to = if self.media.disk_writeback
                            && crate::inputs::is_writable_file(&path.to_string_lossy())
                        {
                            Some(path.clone())
                        } else {
                            self.media.disk_b_save.clone()
                        };
                        machine.set_disk_exit_policy_for(
                            1,
                            crate::plugin_bridge::DiskExitPolicy { save_to },
                        );
                        self.say(format!("Drive B loaded: {}", file_label(&path)));
                        self.media.disk_b_path = Some(path);
                    }
                    Err(e) => self.say(format!("Drive B load failed: {e}")),
                }
                true
            }
            MediaOp::EjectDiskB => {
                if !self.drive_b_can_be_replaced(machine, "ejecting it") {
                    return false;
                }
                match machine.detach_media(plugin_api::MediaSlot::Disk { drive: 1 }) {
                    Ok(()) => {
                        self.media.disk_b_path = None;
                        self.say("Drive B ejected");
                    }
                    Err(e) => self.say(format!("Drive B eject failed: {e}")),
                }
                true
            }
            MediaOp::DiskBSaved => {
                if let Err(e) = machine.mark_disk_saved(1) {
                    self.say(format!("Could not mark drive B saved: {e}"));
                }
                false
            }
        }
    }

    /// Before the disk in drive B leaves it: unsaved writes are never dropped silently. They are written to the `--disk-b` file
    /// (`--disk-writeback`) or `--disk-b-save` when one of those was given; otherwise the change of disk is refused.
    fn drive_b_can_be_replaced(&mut self, machine: &HostedMachine, doing: &str) -> bool {
        let image = match machine.export_disk(1, true) {
            Ok(Some(image)) => image,
            _ => return true,
        };
        let target = match (&self.media.disk_b_path, self.media.disk_writeback) {
            (Some(path), true) if crate::inputs::is_writable_file(&path.to_string_lossy()) => {
                Some(path.clone())
            }
            _ => self.media.disk_b_save.clone(),
        };
        let Some(target) = target else {
            self.say(format!(
                "Drive B has unsaved changes: save them first (save_disk_b), or start with --disk-b-save, before {doing}"
            ));
            return false;
        };
        match crate::plugin_bridge::write_file_atomically(&target, &image) {
            Ok(()) => {
                let _ = machine.mark_disk_saved(1);
                self.say(format!("Drive B saved to {}", file_label(&target)));
                true
            }
            Err(e) => {
                self.say(format!("Drive B save failed: {e} -- not {doing}"));
                false
            }
        }
    }

    fn write_screenshot(&self, width: u32, height: u32, rgb: &[u8]) {
        let dir = self.base_dir.join("screenshots");
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let path = crate::host_files::unique_path(crate::host_files::screenshot_path(
            &dir,
            &self.machine_id,
            secs,
        ));
        let result = std::fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|()| crate::write_png(&path.to_string_lossy(), width, height, rgb));
        match result {
            Ok(()) => self.say(format!("Screenshot saved: {}", path.display())),
            Err(e) => self.say(format!("Screenshot failed: {e}")),
        }
    }
}

/// A path's file name for messages ("Disk loaded: game.dsk").
fn file_label(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Bundles the three receiver channels `tick` drains at its very top --
/// exists to keep `tick`'s own parameter list under clippy's
/// `too_many_arguments` threshold; all three are one-way GUI-thread-to-
/// emu-thread signal channels with the exact same drain-then-apply shape
/// (see `tick`'s own body), so grouping them is a real simplification,
/// not an arbitrary parameter-count dodge.
struct TickInputs<'a> {
    input_rx: &'a Receiver<(String, bool)>,
    /// Key events `tick` has taken off `input_rx` but not applied yet -- see
    /// the "release, then press" rule in `tick`. Empty between ticks unless
    /// one was deferred.
    held_over: RefCell<VecDeque<(String, bool)>>,
    command_rx: &'a Receiver<EmuCommand>,
    mouse_rx: &'a Receiver<MouseEvent>,
}

/// One tick's worth of work: apply any queued input, step the machine for
/// one `frame_hz()` tick's full cycle budget, then publish audio/frame/debug
/// state for the Bevy side to pick up. Kept as a plain function (not
/// inlined into `spawn`'s loop, see Task 2) so it's testable directly,
/// without a real thread or real sleeping.
///
/// `run_ahead_frames` (`0` = disabled, the default/original behavior) is
/// the run-ahead input-latency-reduction knob (see `--run-ahead` in
/// `main.rs`): after the real machine's own step above, a disposable
/// `HostedMachine::clone()` is stepped `run_ahead_frames` further frames
/// ahead using whatever input is currently *held* (no new real input has
/// happened yet for those future frames -- and since input arrives as
/// persistent press/release bits in the keyboard matrix/Kempston state
/// rather than a per-tick impulse, cloning the real machine right after
/// applying this tick's real input already captures "hold the last-known
/// input steady" for free, no separate tracking needed), and *that*
/// clone's rendered frame is what's published to `FrameBuffer` instead of
/// the real machine's own. The clone is dropped at the end of this call --
/// nothing speculative ever persists across ticks, and the real machine
/// itself is never mutated by this (see the `run_ahead_never_mutates_the_
/// real_machine` test). Every speculative tick's audio is drained and
/// discarded, never reaching `audio_queue` -- speculative frames are a
/// "maybe future" that may be thrown away, so their sound must never reach
/// the sound card (RetroArch mutes audio during run-ahead for the same
/// reason).
/// `debugger` is `None` whenever no debugger plugin was requested -- every
/// branch below that touches it is gated behind that single check, so the
/// whole feature collapses to a cheap `Option::is_none()` when unused,
/// matching this session's established "zero cost when a feature is off"
/// precedent from `run_ahead_frames == 0`. When `Some`, a paused debugger skips
/// stepping/audio/rendering entirely for the tick (except exactly one
/// instruction when a step was requested), and a breakpoint hit on the real
/// machine auto-pauses it -- see `debugger::DebuggerLink`'s own doc
/// comments for the full state machine. Never consulted by the speculative
/// run-ahead branch below: that call always passes `|_| false` to
/// `advance_one_frame`, unconditionally, so a speculative clone can never
/// trigger or be affected by a real-machine breakpoint.
fn tick(
    machine: &mut HostedMachine,
    inputs: &TickInputs,
    audio_queue: &AudioQueue,
    frame_buffer: &FrameBuffer,
    debug_snapshot: &DebugSnapshotHandle,
    control: &mut HostControl,
    debugger: &mut Option<DebuggerLink>,
) {
    let tick_start = Instant::now();
    let run_ahead_frames = control.run_ahead_frames;

    // Drained before keyboard input, same "apply commands before stepping"
    // ordering `debugger.drain_commands` below already uses -- a reset
    // queued this tick should land before any keypress also queued this
    // tick gets applied against the (about to be stale) pre-reset machine.
    // `Machine::reset` only touches CPU state (see its own doc comment),
    // so this is safe to call unconditionally with no debugger involved --
    // unlike `DebuggerCommand`, this hotkey works with or without
    // a debugger plugin.
    let mut republish = false;
    while let Ok(command) = inputs.command_rx.try_recv() {
        republish |= control.apply(command, machine);
    }

    let mut drained_keys = 0_u32;
    let batch = {
        let mut held_over = inputs.held_over.borrow_mut();
        while let Ok(event) = inputs.input_rx.try_recv() {
            held_over.push_back(event);
        }
        take_key_batch(&mut held_over)
    };
    for (key, pressed) in batch {
        drained_keys += 1;
        tracing::info!(target: "retroclawd::input", event = drained_keys, key, pressed, "key event consumed");
        machine.key_event_name(&key, pressed);
        if let Some(tape) = machine.tape_debug_snapshot() {
            tracing::info!(
                target: "retroclawd::input",
                event = drained_keys,
                key = ?key,
                pressed,
                machine = %tape.machine,
                motor = tape.motor_on,
                pos = tape.position,
                events = tape.events,
                playing = tape.playing,
                finished = tape.finished,
                speed = tape.speed_multiplier,
                ear = tape.ear_bit,
                edge_cycles = tape.cycles_until_next_edge,
                spin_down = tape.spin_down_cycles,
                pc = format!("0x{:04X}", machine.pc()),
                "post-key tape snapshot"
            );
        }
    }
    if drained_keys > 1 {
        tracing::info!(target: "retroclawd::input", count = drained_keys, "input burst consumed in one tick");
    }

    while let Ok(event) = inputs.mouse_rx.try_recv() {
        machine.mouse_event(event);
    }

    if let Some(dbg) = debugger.as_mut() {
        dbg.drain_commands(machine);
    }
    let debugger_paused = debugger.as_ref().is_some_and(|d| d.is_paused());
    // A hotkey pause freezes everything like a debugger pause does, but
    // without the debugger's single-step requests.
    // A crashed plugin answers every call with an error, and the step loop's clock query would panic this thread: say so once and
    // hold the machine (its last picture stays up) instead.
    if !control.crashed
        && let Some(message) = machine.crash_message()
    {
        control.crashed = true;
        tracing::error!(target: "retroclawd::plugin", "{message}");
        eprintln!("the machine plugin crashed: {message}");
        control.say("The machine crashed (see the console); reset the app to continue");
    }
    let is_paused = debugger_paused || control.paused || control.dialog_open || control.crashed;

    let m = &mut *machine;

    let step_start = Instant::now();
    if is_paused {
        if !control.paused
            && !control.dialog_open
            && let Some(dbg) = debugger.as_mut()
        {
            if dbg.take_pending_backstep() {
                dbg.execute_backstep_instruction(m);
            } else if dbg.take_pending_step_over() {
                dbg.execute_step_over_instruction(m);
            } else if dbg.take_pending_step() {
                dbg.execute_step_instruction(m);
            }
        }
    } else {
        // Scoped tightly around just the step loop -- this is the part
        // meant to be measured in Tracy, not the audio/frame/debug work
        // below it.
        #[cfg(feature = "profiling")]
        let _span = tracing::info_span!("emu_thread_step_budget").entered();
        if let Some(dbg) = debugger.as_mut() {
            advance_one_frame_with_debugger(m, dbg);
        } else {
            advance_one_frame(m, |_| false);
        }
    }
    let step_elapsed = step_start.elapsed();

    let audio_start = Instant::now();
    if !is_paused {
        let samples = m.drain_audio();
        // Fast-forward drops its audio (sped-up sound is just noise, and
        // queueing it would add seconds of latency after the key is released).
        if !samples.is_empty() && !control.fast_forward {
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
    }
    let audio_elapsed = audio_start.elapsed();

    // Written from the real machine, right after its own step/audio and
    // before any speculative cloning below -- makes "the debug overlay
    // always reflects real state, never a speculative run-ahead frame"
    // true by construction rather than by convention.
    {
        let mut snapshot = debug_snapshot.lock().unwrap();
        snapshot.halted = m.halted();
        snapshot.registers = m.registers();
    }

    let render_start = Instant::now();
    let mut speculative_elapsed = Duration::ZERO;
    // Fast-forward renders only every FAST_FORWARD_FACTOR-th tick: the window
    // shows ~60 frames a second whatever the emulation speed, and rendering
    // is often the most expensive part of a tick.
    let skip_fast_forward_render = control.fast_forward && !is_paused && {
        control.fast_forward_ticks = control.fast_forward_ticks.wrapping_add(1);
        !control
            .fast_forward_ticks
            .is_multiple_of(FAST_FORWARD_FACTOR)
    };
    if (is_paused && !republish) || skip_fast_forward_render {
        // Nothing changed (aside from at most one stepped instruction,
        // already reflected in `debug_snapshot`/the debugger's own state
        // below) -- skip rendering entirely rather than republishing a
        // frame identical to what's already in `FrameBuffer`. The GUI
        // simply keeps showing the last frame it has, "frozen" from the
        // viewer's perspective at zero extra cost.
    } else if run_ahead_frames == 0 || is_paused {
        if let Some(frame) = m.render_frame() {
            *frame_buffer.lock().unwrap() = Some(frame);
        }
    } else {
        // tick clones fresh from the now-current real state and drops the
        // clone at the end of this call (matching RetroArch's own
        // per-frame re-clone design) -- no speculative state ever persists
        // across ticks.
        let speculative_start = Instant::now();
        let mut speculative = machine.clone();
        let spec_m = &mut speculative;
        let mut last_frame = None;
        for i in 0..run_ahead_frames {
            advance_one_frame(spec_m, |_| false);
            let _ = spec_m.drain_audio(); // discarded -- never touches audio_queue
            if i + 1 == run_ahead_frames {
                last_frame = spec_m.render_frame();
            }
        }
        if let Some(frame) = last_frame {
            *frame_buffer.lock().unwrap() = Some(frame);
        }
        speculative_elapsed = speculative_start.elapsed();
    }
    let render_elapsed = render_start.elapsed();

    if let Some(dbg) = debugger.as_mut() {
        dbg.publish(machine);
    }

    let total_elapsed = tick_start.elapsed();
    if total_elapsed > SLOW_TICK_THRESHOLD {
        tracing::info!(
            "slow tick: total={total_elapsed:?} step={step_elapsed:?} audio={audio_elapsed:?} render={render_elapsed:?} speculative={speculative_elapsed:?}"
        );
    }
}

/// Owns the background thread `spawn` starts, and every shared handoff
/// point it writes to. Dropping this (see `Drop` impl below) is the only
/// way to stop the thread -- there's no separate `stop()` method, so a
/// caller can't forget to shut it down.
pub(crate) struct EmuThreadHandle {
    pub(crate) audio_queue: AudioQueue,
    pub(crate) frame_buffer: FrameBuffer,
    pub(crate) debug_snapshot: DebugSnapshotHandle,
    pub(crate) underrun_count: UnderrunCount,
    pub(crate) requested_count: RequestedCount,
    pub(crate) input_tx: Sender<(String, bool)>,
    /// One-way host actions (reset, pause, screenshot, quick save/load,
    /// fast-forward) -- mirrors `input_tx`'s shape. Works with or without
    /// debugger support (unlike `debugger_commands` below), since hotkeys
    /// are basic features, not debug-only ones.
    pub(crate) command_tx: Sender<EmuCommand>,
    /// Player-facing messages from those actions; drained by the GUI's toast.
    pub(crate) messages: Messages,
    pub(crate) mouse_tx: Sender<MouseEvent>,
    /// `Some` only when `spawn` was called with `debugger_enabled: true` --
    /// `gui::run_gui` only starts debugger plugin hosting when both are
    /// `Some`, so debugger-off runs never construct these at all.
    pub(crate) debugger_commands: Option<DebuggerCommandSender>,
    pub(crate) debugger_state: Option<DebuggerStateHandle>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Drop for EmuThreadHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            // `Drop` itself must stay panic-free -- log a panicking emu
            // thread rather than silently discarding it, but don't
            // propagate the panic from here.
            if let Err(err) = join.join() {
                tracing::info!("emu thread panicked: {err:?}");
            }
        }
    }
}

/// Moves `machine` onto its own thread, running it at its own `frame_hz()`
/// pace via a fixed-deadline real-time sleep loop (see the `next_tick`
/// comment below for why it's a deadline schedule rather than a plain
/// "sleep for the remainder of this tick"), with audio/frame/debug side
/// effects added at each tick -- see `tick` above. Bevy-agnostic: depends
/// only on `std` and `HostedMachine`, so it works the same for every machine
/// with no per-machine code, same as the host-machine abstraction itself.
pub(crate) fn spawn(
    mut machine: HostedMachine,
    machine_id: &str,
    run_ahead_frames: u32,
    debugger_enabled: bool,
    overlay_data_wanted: Arc<AtomicBool>,
    media: MediaSettings,
) -> EmuThreadHandle {
    let audio_queue: AudioQueue = Arc::new(Mutex::new(VecDeque::new()));
    let frame_buffer: FrameBuffer = Arc::new(Mutex::new(None));
    let debug_snapshot: DebugSnapshotHandle = Arc::new(Mutex::new(DebugSnapshot {
        halted: false,
        registers: Vec::new(),
        memory_hex: Vec::new(),
        memory_hex_addr: 0,
        overlay: crate::plugin_bridge::OverlayDebugSnapshot::default(),
    }));
    let (input_tx, input_rx) = mpsc::channel::<(String, bool)>();
    let (command_tx, command_rx) = mpsc::channel::<EmuCommand>();
    let messages: Messages = Arc::new(Mutex::new(VecDeque::new()));
    let mut control = HostControl::new(run_ahead_frames, machine_id, messages.clone(), media);
    let (mouse_tx, mouse_rx) = mpsc::channel::<MouseEvent>();
    let stop = Arc::new(AtomicBool::new(false));
    let underrun_count: UnderrunCount = Arc::new(AtomicU64::new(0));
    let requested_count: RequestedCount = Arc::new(AtomicU64::new(0));

    // `debugger_enabled` gates whether any of this is even constructed --
    // debugger-off means no channel, no shared state, and `tick`'s own
    // `debugger: &mut Option<DebuggerLink>` stays `None` for the thread's
    // entire lifetime.
    let debugger_setup = debugger_enabled.then(|| {
        let (tx, rx) = mpsc::channel();
        let state = DebuggerLink::new_handle();
        (tx, rx, state)
    });
    let debugger_commands = debugger_setup.as_ref().map(|(tx, _, _)| tx.clone());
    let debugger_state = debugger_setup.as_ref().map(|(_, _, state)| state.clone());
    let mut debugger_link = debugger_setup.map(|(_, rx, state)| DebuggerLink::new(rx, state));

    let thread_audio_queue = audio_queue.clone();
    let thread_frame_buffer = frame_buffer.clone();
    let thread_debug_snapshot = debug_snapshot.clone();
    let thread_underrun_count = underrun_count.clone();
    let thread_requested_count = requested_count.clone();
    let thread_stop = stop.clone();

    let join = thread::Builder::new()
        .name("emu".into())
        .spawn(move || {
            let tick_duration =
                Duration::from_secs_f64(1.0 / machine.frame_hz() as f64);
            let mut logged_halt = false;
            let diagnostics_enabled = debugger_enabled;

            // Fixed deadline schedule rather than "sleep for tick_duration
            // minus this tick's own elapsed time": `thread::sleep` only
            // guarantees *at least* the requested duration (often more, due
            // to OS scheduling), so measuring from each tick's own start
            // never recovers lost time -- every tick period ends up >=
            // target, and the shortfall compounds. Tracking one absolute
            // `next_tick` deadline instead means tick N's deadline is always
            // `start + N * tick_duration`, so a single overrun doesn't push
            // every later tick back too.
            let mut next_tick = Instant::now() + tick_duration;
            // Diagnostic only (see `UnderrunCount`'s doc comment) -- logged
            // at 1Hz rather than every tick so it stays cheap, and so a
            // human reading the log can see a trend (climbing vs. flat)
            // rather than 50 lines/sec of noise.
            let mut last_diag_log = Instant::now();
            let mut last_requested = 0u64;
            let mut last_underruns = 0u64;
            // Real ticks actually completed since the last diagnostic log --
            // distinct from `slow tick:`'s per-tick internal-cost check:
            // this catches the OS oversleeping `thread::sleep` requests
            // (Windows' default timer resolution is commonly ~15.6ms, so a
            // requested ~20ms sleep can routinely take 30ms+) even when
            // `tick()`'s own CPU-bound work never exceeds SLOW_TICK_THRESHOLD
            // -- a real tick rate well under `frame_hz()` would starve audio
            // production exactly like this, with no single tick ever
            // looking slow in isolation.
            let mut tick_count = 0u64;
            let mut last_tape_position = None::<usize>;
            let mut last_tape_edge_cycles = None::<u64>;
            let mut stagnant_tape_diag_seconds = 0u32;
            let mut last_tape_motor_on = None::<bool>;
            let mut last_tape_observed_position = None::<usize>;
            let mut last_cpu_pc = None::<u64>;
            let mut last_cpu_sp = None::<u64>;
            let mut last_overlay_fetch = Instant::now();

            let log_tape = |prefix: &str, snap: &crate::plugin_bridge::TapeDebugSnapshot| {
                let motor = match snap.motor_on {
                    Some(true) => "on",
                    Some(false) => "off",
                    None => "n/a",
                };
                let ear = match snap.ear_bit {
                    Some(true) => "high",
                    Some(false) => "low",
                    None => "n/a",
                };
                let attached = if snap.attached { "yes" } else { "no" };
                let edge_cycles = snap
                    .cycles_until_next_edge
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "n/a".to_string());
                let spin_down = snap
                    .spin_down_cycles
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "n/a".to_string());
                tracing::info!(
                    "{prefix}: machine={} attached={} motor={} pos={}/{} playing={} finished={} speed={}x ear={} edge_cycles={} spin_down={}",
                    snap.machine,
                    attached,
                    motor,
                    snap.position,
                    snap.events,
                    snap.playing,
                    snap.finished,
                    snap.speed_multiplier,
                    ear,
                    edge_cycles,
                    spin_down
                );
            };
            // Formats whatever fields the machine's generic `chip_id:
            // "overlay"` snapshot happens to carry -- no hardcoded CPC-only
            // field-name knowledge, unlike the fixed-shape version this
            // replaced (which only ever worked for CPC and silently dropped
            // the whole log line if even one expected field was missing).
            let log_overlay_diagnostics = |prefix: &str, snap: &plugin_api::ChipSnapshot| {
                let fields_text = snap
                    .fields
                    .iter()
                    .map(|field| format!("{}={}", field.key, format_debug_value(&field.value)))
                    .collect::<Vec<_>>()
                    .join(" ");
                let waveform = snap
                    .blobs
                    .iter()
                    .find(|blob| blob.key == "tape_waveform")
                    .map(|blob| blob.bytes.as_slice())
                    .unwrap_or(&[]);
                let mut waveform_tail = String::new();
                for &level in waveform.iter().rev().take(48).rev() {
                    waveform_tail.push(if level == 0 { '0' } else { '1' });
                }
                if waveform_tail.is_empty() {
                    waveform_tail.push('-');
                }
                tracing::info!("{prefix}: {fields_text} waveform_tail={waveform_tail}");
            };
            let log_cpu = |prefix: &str, snap: &DebugSnapshot| {
                use std::fmt::Write as _;

                let mut regs = String::new();
                for (idx, (name, value)) in snap.registers.iter().enumerate() {
                    if idx > 0 {
                        regs.push(' ');
                    }
                    let _ = write!(&mut regs, "{name}=0x{value:X}");
                }
                tracing::info!("{prefix}: halted={} {regs}", snap.halted);
            };
            let mut last_tape_signature = if diagnostics_enabled {
                machine.tape_debug_snapshot().map(|s| {
                    (
                        s.machine,
                        s.motor_on,
                        s.playing,
                        s.finished,
                        s.speed_multiplier,
                    )
                })
            } else {
                None
            };
            if diagnostics_enabled
                && let Some(ref snap) = machine.tape_debug_snapshot()
            {
                log_tape("tape state", snap);
            }


            while !thread_stop.load(Ordering::Relaxed) {
                tick(
                    &mut machine,
                    &TickInputs {
                        input_rx: &input_rx,
                        held_over: RefCell::default(),
                        command_rx: &command_rx,
                        mouse_rx: &mouse_rx,
                    },
                    &thread_audio_queue,
                    &thread_frame_buffer,
                    &thread_debug_snapshot,
                    &mut control,
                    &mut debugger_link,
                );
                if overlay_data_wanted.load(Ordering::Relaxed)
                    && last_overlay_fetch.elapsed() >= OVERLAY_DATA_INTERVAL
                {
                    last_overlay_fetch = Instant::now();
                    let pc = {
                        let snapshot = thread_debug_snapshot.lock().unwrap();
                        register_value(&snapshot.registers, "PC").unwrap_or(0) as u16
                    };
                    let memory_hex: Vec<u8> = (0..MEMORY_HEX_WINDOW)
                        .map(|offset| machine.read_memory(pc.wrapping_add(offset)))
                        .collect();
                    let overlay = machine.overlay_debug_snapshot();
                    let mut snapshot = thread_debug_snapshot.lock().unwrap();
                    snapshot.memory_hex = memory_hex;
                    snapshot.memory_hex_addr = pc;
                    snapshot.overlay = overlay;
                }
                if diagnostics_enabled {
                    tick_count += 1;

                    let tape_snapshot = machine.tape_debug_snapshot();
                    let tape_signature = tape_snapshot.clone().map(|s| {
                        (
                            s.machine,
                            s.motor_on,
                            s.playing,
                            s.finished,
                            s.speed_multiplier,
                        )
                    });
                    if tape_signature != last_tape_signature {
                        if let Some(ref snap) = tape_snapshot {
                            log_tape("tape state", snap);
                        } else if last_tape_signature.is_some() {
                            tracing::info!("tape state: detached");
                        }
                        last_tape_signature = tape_signature;
                    }

                    let (pc_now, sp_now) = {
                        let snapshot = thread_debug_snapshot.lock().unwrap();
                        (
                            register_value(&snapshot.registers, "PC"),
                            register_value(&snapshot.registers, "SP"),
                        )
                    };

                    if let Some(ref snap) = tape_snapshot {
                        let cpc_machine = snap.machine.starts_with("cpc");
                        let motor_fell =
                            matches!(last_tape_motor_on, Some(true)) && snap.motor_on == Some(false);
                        let reboot_pc = pc_now.is_some_and(is_cpc_firmware_boot_pc)
                            || last_cpu_pc.is_some_and(is_cpc_firmware_boot_pc);
                        if cpc_machine && snap.playing && !snap.finished && motor_fell {
                            let prev_pc = last_cpu_pc.unwrap_or(0);
                            let prev_sp = last_cpu_sp.unwrap_or(0);
                            let pc = pc_now.unwrap_or(0);
                            let sp = sp_now.unwrap_or(0);
                            let pos_delta = last_tape_observed_position
                                .map(|prev| snap.position.saturating_sub(prev))
                                .unwrap_or(0);
                            tracing::warn!(
                                "cpc tape motor dropped during active playback: machine={} prev_pc=0x{prev_pc:04X} pc=0x{pc:04X} prev_sp=0x{prev_sp:04X} sp=0x{sp:04X} tape_pos={}/{} tape_delta={} reboot_pc={} playing={} finished={}",
                                snap.machine,
                                snap.position,
                                snap.events,
                                pos_delta,
                                reboot_pc,
                                snap.playing,
                                snap.finished,
                            );
                            if reboot_pc {
                                tracing::warn!(
                                    "firmware restart signature: machine={} prev_pc=0x{prev_pc:04X} pc=0x{pc:04X} prev_sp=0x{prev_sp:04X} sp=0x{sp:04X} tape_pos={}/{} tape_delta={} motor=on->off playing={} finished={}",
                                    snap.machine,
                                    snap.position,
                                    snap.events,
                                    pos_delta,
                                    snap.playing,
                                    snap.finished,
                                );
                            }
                        }
                        last_tape_motor_on = snap.motor_on;
                        last_tape_observed_position = Some(snap.position);
                    } else {
                        last_tape_motor_on = None;
                        last_tape_observed_position = None;
                    }

                    if let Some(pc) = pc_now {
                        last_cpu_pc = Some(pc);
                    }
                    if let Some(sp) = sp_now {
                        last_cpu_sp = Some(sp);
                    }

                    let diag_now = Instant::now();
                    let elapsed = diag_now.duration_since(last_diag_log);
                    if elapsed >= Duration::from_secs(1) {
                        let queue_len = thread_audio_queue.lock().unwrap().len();
                        let underruns = thread_underrun_count.load(Ordering::Relaxed);
                        let requested = thread_requested_count.load(Ordering::Relaxed);
                        // Delta-per-second, not just cumulative totals -- this
                        // is what directly answers "is the backend pulling
                        // samples faster than machine::AUDIO_SAMPLE_RATE?"
                        // without the human needing to do the subtraction
                        // across log lines themselves.
                        let requested_rate =
                            (requested - last_requested) as f64 / elapsed.as_secs_f64();
                        let underrun_rate =
                            (underruns - last_underruns) as f64 / elapsed.as_secs_f64();
                        let tick_rate = tick_count as f64 / elapsed.as_secs_f64();
                        tracing::info!("audio diag: queue_len={queue_len} requested/s={requested_rate:.0} underruns/s={underrun_rate:.0} total_underruns={underruns} ticks/s={tick_rate:.1}");
                        {
                            let snapshot = thread_debug_snapshot.lock().unwrap();
                            log_cpu("cpu diag", &snapshot);
                        }
                        let overlay_snapshot = machine.overlay_diagnostic_snapshot();
                        if let Some(ref overlay) = overlay_snapshot {
                            log_overlay_diagnostics("overlay diag", overlay);
                        }
                        if let Some(ref snap) = tape_snapshot {
                            log_tape("tape diag", snap);
                            if snap.playing && !snap.finished {
                                let progressed_within_event =
                                    match (last_tape_edge_cycles, snap.cycles_until_next_edge) {
                                        (Some(prev), Some(now)) => now < prev,
                                        _ => false,
                                    };
                                if Some(snap.position) == last_tape_position
                                    && !progressed_within_event
                                {
                                    stagnant_tape_diag_seconds =
                                        stagnant_tape_diag_seconds.saturating_add(1);
                                } else {
                                    stagnant_tape_diag_seconds = 0;
                                }
                                if stagnant_tape_diag_seconds >= 5 {
                                    let motor = match snap.motor_on {
                                        Some(true) => "on",
                                        Some(false) => "off",
                                        None => "n/a",
                                    };
                                    let ear = match snap.ear_bit {
                                        Some(true) => "high",
                                        Some(false) => "low",
                                        None => "n/a",
                                    };
                                    let edge_cycles = snap
                                        .cycles_until_next_edge
                                        .map(|c| c.to_string())
                                        .unwrap_or_else(|| "n/a".to_string());
                                    let edge_prev = last_tape_edge_cycles
                                        .map(|c| c.to_string())
                                        .unwrap_or_else(|| "n/a".to_string());
                                    let spin_down = snap
                                        .spin_down_cycles
                                        .map(|c| c.to_string())
                                        .unwrap_or_else(|| "n/a".to_string());
                                    let pc = pc_now.unwrap_or(0);
                                    let sp = sp_now.unwrap_or(0);
                                    let pos_delta = last_tape_observed_position
                                        .map(|prev| snap.position.saturating_sub(prev))
                                        .unwrap_or(0);
                                    tracing::warn!(
                                        "tape progress stalled: machine={} attached={} motor={} pos={}/{} tape_delta={} playing={} finished={} speed={}x stagnant_s={} edge_prev={} edge_now={} edge_progress={} spin_down={} ear={} pc=0x{pc:04X} sp=0x{sp:04X}",
                                        snap.machine,
                                        if snap.attached { "yes" } else { "no" },
                                        motor,
                                        snap.position,
                                        snap.events,
                                        pos_delta,
                                        snap.playing,
                                        snap.finished,
                                        snap.speed_multiplier,
                                        stagnant_tape_diag_seconds,
                                        edge_prev,
                                        edge_cycles,
                                        progressed_within_event,
                                        spin_down,
                                        ear,
                                    );
                                    if let Some(ref overlay) = overlay_snapshot {
                                        log_overlay_diagnostics("overlay stall diag", overlay);
                                    }
                                }
                                last_tape_position = Some(snap.position);
                                last_tape_edge_cycles = snap.cycles_until_next_edge;
                            } else {
                                last_tape_position = None;
                                last_tape_edge_cycles = None;
                                stagnant_tape_diag_seconds = 0;
                            }

                        }
                        last_diag_log = diag_now;
                        last_requested = requested;
                        last_underruns = underruns;
                        tick_count = 0;
                    }
                }

                if diagnostics_enabled {
                    // Read halted/PC from the snapshot `tick` just wrote above,
                    // rather than re-querying the machine -- `tick` already did
                    // this same `registers()` lookup once this tick.
                    let halted = {
                        let snapshot = thread_debug_snapshot.lock().unwrap();
                        if snapshot.halted && !logged_halt {
                            let pc = snapshot
                                .registers
                                .iter()
                                .find(|(name, _)| *name == "PC")
                                .map(|(_, v)| *v)
                                .unwrap_or(0);
                            tracing::info!("machine halted at pc=0x{pc:04X}");
                        }
                        snapshot.halted
                    };
                    logged_halt = halted;
                }

                // Fast-forward shortens the tick period instead of skipping
                // work: every tick still steps the full cycle budget.
                let period = if control.fast_forward {
                    tick_duration / FAST_FORWARD_FACTOR
                } else {
                    tick_duration
                };
                let now = Instant::now();
                if now < next_tick {
                    thread::sleep(next_tick - now);
                } else if now.saturating_duration_since(next_tick) > period * 4 {
                    // A long stall (debugger pause, OS suspend, ...) left
                    // `next_tick` far in the past -- resync instead of
                    // bursting through a pile of overdue ticks back-to-back
                    // to "catch up". (Also what ends a fast-forward cleanly:
                    // the deadline it ran ahead of is simply dropped.)
                    next_tick = now;
                }
                next_tick += period;
            }
        })
        .expect("failed to spawn emu thread");

    EmuThreadHandle {
        audio_queue,
        frame_buffer,
        debug_snapshot,
        underrun_count,
        requested_count,
        input_tx,
        command_tx,
        messages,
        mouse_tx,
        debugger_commands,
        debugger_state,
        stop,
        join: Some(join),
    }
}

#[cfg(test)]
mod tests {
    fn events(list: &[(&str, bool)]) -> VecDeque<(String, bool)> {
        list.iter().map(|(k, p)| (k.to_string(), *p)).collect()
    }

    fn batch(list: &[(&str, bool)]) -> Vec<(String, bool)> {
        events(list).into()
    }

    #[test]
    fn a_press_after_a_release_waits_for_the_next_tick() {
        // Shift+8: the host releases Shift and presses the key in one frame.
        let mut pending = events(&[("ShiftLeft", false), ("Asterisk", true), ("KeyA", true)]);
        assert_eq!(take_key_batch(&mut pending), batch(&[("ShiftLeft", false)]));
        // Next tick: the deferred press and everything behind it, in order.
        assert_eq!(
            take_key_batch(&mut pending),
            batch(&[("Asterisk", true), ("KeyA", true)])
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn presses_first_and_plain_releases_are_not_delayed() {
        let mut pending = events(&[("A", true), ("B", true), ("A", false), ("B", false)]);
        assert_eq!(take_key_batch(&mut pending).len(), 4);
        // Release, release, press: only the press waits.
        let mut pending = events(&[("A", false), ("B", false), ("C", true)]);
        assert_eq!(take_key_batch(&mut pending).len(), 2);
        assert_eq!(take_key_batch(&mut pending), batch(&[("C", true)]));
        assert!(take_key_batch(&mut pending).is_empty());
    }

    use super::*;
    use plugin_api::MachineCreateRequest;

    /// The stub machine: it runs NOPs, has a disk slot and a tape slot.
    fn machine() -> HostedMachine {
        crate::plugin_bridge::create_hosted_machine(MachineCreateRequest {
            machine_id: "stub-machine".to_string(),
            region: Some("pal".to_string()),
            roms: Vec::new(),
            options: Vec::new(),
        })
        .expect("the bundled stub machine")
    }

    struct Rig {
        machine: HostedMachine,
        control: HostControl,
        messages: Messages,
    }

    impl Rig {
        fn new(media: MediaSettings) -> Rig {
            let messages: Messages = Arc::new(Mutex::new(VecDeque::new()));
            Rig {
                machine: machine(),
                control: HostControl::new(0, "stub-machine", messages.clone(), media),
                messages,
            }
        }

        fn apply(&mut self, command: EmuCommand) {
            self.control.apply(command, &mut self.machine);
        }

        fn said(&self) -> Vec<String> {
            self.messages.lock().unwrap().drain(..).collect()
        }

        fn disk_status(&mut self) -> DiskStatus {
            let (tx, rx) = mpsc::channel();
            self.apply(EmuCommand::DiskStatus(tx));
            rx.recv().unwrap()
        }

        fn export(&mut self, what: Export) -> Result<Option<Vec<u8>>, String> {
            let (tx, rx) = mpsc::channel();
            self.apply(EmuCommand::Export(what, tx));
            rx.recv().unwrap()
        }
    }

    fn blank_disk() -> Vec<u8> {
        dsk::Disk::new(1).to_bytes()
    }

    #[test]
    fn a_disk_can_be_swapped_and_ejected_while_running() {
        let mut rig = Rig::new(MediaSettings::default());
        assert_eq!(
            rig.export(Export::Disk),
            Ok(None),
            "an empty drive exports nothing"
        );

        rig.apply(EmuCommand::Media(MediaOp::AttachDisk {
            extension: "dsk".to_string(),
            bytes: blank_disk(),
            path: "games/first.dsk".into(),
        }));
        assert_eq!(rig.said(), ["Disk loaded: first.dsk"]);
        let status = rig.disk_status();
        assert!(!status.modified);
        assert_eq!(
            status.original.as_deref(),
            Some(std::path::Path::new("games/first.dsk"))
        );
        assert!(matches!(rig.export(Export::Disk), Ok(Some(_))));

        // A second disk replaces the first, and its path becomes "the original".
        rig.apply(EmuCommand::Media(MediaOp::AttachDisk {
            extension: "dsk".to_string(),
            bytes: blank_disk(),
            path: "games/second.dsk".into(),
        }));
        assert_eq!(
            rig.disk_status().original.as_deref(),
            Some(std::path::Path::new("games/second.dsk"))
        );

        rig.apply(EmuCommand::Media(MediaOp::EjectDisk));
        assert_eq!(rig.said(), ["Disk loaded: second.dsk", "Disk ejected"]);
        assert_eq!(rig.export(Export::Disk), Ok(None));
        assert_eq!(rig.disk_status().original, None);
    }

    #[test]
    fn drive_b_takes_its_own_disk_and_guards_unsaved_changes() {
        let dir = std::env::temp_dir().join(format!("rc-emu-driveb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let save = dir.join("saved-b.dsk");
        let mut rig = Rig::new(MediaSettings {
            disk_b_save: Some(save.clone()),
            ..MediaSettings::default()
        });
        let attach_b = |name: &str| {
            EmuCommand::Media(MediaOp::AttachDiskB {
                extension: "dsk".to_string(),
                bytes: blank_disk(),
                path: dir.join(name),
            })
        };
        // Drive B and drive A are separate: loading B leaves A empty.
        rig.apply(attach_b("one.dsk"));
        assert_eq!(rig.said(), ["Drive B loaded: one.dsk"]);
        assert!(matches!(rig.export(Export::DiskB), Ok(Some(_))));
        assert_eq!(rig.export(Export::Disk), Ok(None), "drive A is still empty");

        // The guest writes to B: replacing it first writes the changes to --disk-b-save, then loads the new disk.
        rig.machine
            .try_key_event(test_machine_plugin_impl::DIRTY_DISK_B_KEY, true)
            .unwrap();
        rig.apply(attach_b("two.dsk"));
        assert_eq!(
            rig.said(),
            ["Drive B saved to saved-b.dsk", "Drive B loaded: two.dsk"]
        );
        assert!(save.exists(), "the changes were written before the swap");
        assert_eq!(
            rig.control.media.disk_b_path.as_deref(),
            Some(dir.join("two.dsk").as_path())
        );

        // Without a place to put them, the swap is refused and the disk stays.
        let mut rig = Rig::new(MediaSettings::default());
        rig.apply(attach_b("one.dsk"));
        rig.said();
        rig.machine
            .try_key_event(test_machine_plugin_impl::DIRTY_DISK_B_KEY, true)
            .unwrap();
        rig.apply(attach_b("two.dsk"));
        let said = rig.said();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].starts_with("Drive B has unsaved changes"),
            "{said:?}"
        );
        assert!(
            matches!(rig.export(Export::DiskB), Ok(Some(_))),
            "B kept its disk"
        );
        rig.apply(EmuCommand::Media(MediaOp::EjectDiskB));
        assert!(rig.said()[0].starts_with("Drive B has unsaved changes"));
        // Saving it by hand (the dialog's Save As) counts as saved, and then the eject goes through.
        rig.apply(EmuCommand::Media(MediaOp::DiskBSaved));
        rig.apply(EmuCommand::Media(MediaOp::EjectDiskB));
        assert_eq!(rig.said(), ["Drive B ejected"]);
        assert_eq!(rig.export(Export::DiskB), Ok(None));
        // A disk that was never written is replaced without a word.
        rig.apply(attach_b("three.dsk"));
        rig.apply(EmuCommand::Media(MediaOp::EjectDiskB));
        assert_eq!(rig.said(), ["Drive B loaded: three.dsk", "Drive B ejected"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quick_save_states_are_named_after_the_game() {
        let dir = std::env::temp_dir().join(format!("rc-emu-states-{}", std::process::id()));
        for (game, file) in [
            (Some("Elite"), "stub-machine-Elite-slot0.state"),
            (None, "stub-machine-slot0.state"),
        ] {
            let mut rig = Rig::new(MediaSettings {
                game: game.map(str::to_string),
                ..MediaSettings::default()
            });
            rig.control.base_dir = dir.clone();
            rig.apply(EmuCommand::QuickSave);
            assert!(
                dir.join("states").join(file).exists(),
                "{file} was not written"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_bad_disk_file_is_reported_not_swallowed() {
        let mut rig = Rig::new(MediaSettings::default());
        rig.apply(EmuCommand::Media(MediaOp::AttachDisk {
            extension: "dsk".to_string(),
            bytes: b"not a disk".to_vec(),
            path: "junk.dsk".into(),
        }));
        let said = rig.said();
        assert_eq!(said.len(), 1);
        assert!(said[0].starts_with("Disk load failed:"), "{said:?}");
        assert!(matches!(rig.export(Export::Disk), Ok(None)));
    }

    #[test]
    fn tapes_load_and_eject_and_unsupported_exports_are_errors() {
        let mut rig = Rig::new(MediaSettings {
            tape_speed: 4,
            ..MediaSettings::default()
        });
        rig.apply(EmuCommand::Media(MediaOp::AttachTape {
            extension: "xyz".to_string(),
            bytes: vec![1, 2, 3],
        }));
        assert!(rig.said()[0].starts_with("Tape load failed:"));
        rig.apply(EmuCommand::Media(MediaOp::EjectTape));
        assert_eq!(rig.said(), ["Tape ejected"]);
        // Nothing recorded yet: no tape file to save.
        assert_eq!(
            rig.export(Export::Tape {
                format: "cdt".to_string()
            }),
            Ok(None)
        );
        // The stub machine saves `.sna`, not `.z80`.
        assert!(
            rig.export(Export::Snapshot {
                extension: "z80".to_string()
            })
            .is_err()
        );
    }

    #[test]
    fn state_exported_by_a_dialog_loads_back_through_media() {
        let mut rig = Rig::new(MediaSettings::default());
        let state = rig.export(Export::State).unwrap().expect("a state");
        rig.apply(EmuCommand::Media(MediaOp::LoadState { bytes: state }));
        assert_eq!(rig.said(), ["State loaded"]);
        rig.apply(EmuCommand::Media(MediaOp::LoadState {
            bytes: vec![0, 1, 2],
        }));
        assert!(rig.said()[0].starts_with("State load failed:"));
    }

    #[test]
    fn an_open_dialog_holds_the_machine_still_until_it_closes() {
        let mut rig = Rig::new(MediaSettings::default());
        let audio: AudioQueue = Arc::new(Mutex::new(VecDeque::new()));
        let frames: FrameBuffer = Arc::new(Mutex::new(None));
        let debug: DebugSnapshotHandle = Arc::new(Mutex::new(DebugSnapshot {
            halted: false,
            registers: Vec::new(),
            memory_hex: Vec::new(),
            memory_hex_addr: 0,
            overlay: crate::plugin_bridge::OverlayDebugSnapshot::default(),
        }));
        let (_input_tx, input_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let (_mouse_tx, mouse_rx) = mpsc::channel();
        let inputs = TickInputs {
            input_rx: &input_rx,
            held_over: RefCell::default(),
            command_rx: &command_rx,
            mouse_rx: &mouse_rx,
        };
        let step_tick = |rig: &mut Rig| {
            tick(
                &mut rig.machine,
                &inputs,
                &audio,
                &frames,
                &debug,
                &mut rig.control,
                &mut None,
            );
            machine::Machine::pc(&rig.machine)
        };

        let start = step_tick(&mut rig);
        let running = step_tick(&mut rig);
        assert_ne!(start, running, "NOPs advance the program counter");

        command_tx.send(EmuCommand::Dialog(true)).unwrap();
        let held = step_tick(&mut rig);
        let still = step_tick(&mut rig);
        assert_eq!(held, still, "no instruction runs while a dialog is open");

        command_tx.send(EmuCommand::Dialog(false)).unwrap();
        assert_ne!(step_tick(&mut rig), still, "closing the dialog resumes");
    }

    /// A plugin panic is remembered process-wide (`plugin_api::guard`), so this test crashes the stub in a copy of the test
    /// binary of its own and only the parent checks that the child passed; no other test sees the crash.
    #[test]
    fn a_crashed_plugin_is_reported_once_and_the_machine_is_held() {
        const CHILD: &str = "RC_CRASH_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "a_crashed_plugin_is_reported_once",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                output.status.success(),
                "the child run failed:
{text}"
            );
            assert!(
                text.contains("1 passed"),
                "the child did not run the test:
{text}"
            );
            assert!(
                text.contains("the machine plugin crashed:"),
                "the console line is missing:
{text}"
            );
            return;
        }

        let mut rig = Rig::new(MediaSettings::default());
        let audio: AudioQueue = Arc::new(Mutex::new(VecDeque::new()));
        let frames: FrameBuffer = Arc::new(Mutex::new(None));
        let debug: DebugSnapshotHandle = Arc::new(Mutex::new(DebugSnapshot {
            halted: false,
            registers: Vec::new(),
            memory_hex: Vec::new(),
            memory_hex_addr: 0,
            overlay: crate::plugin_bridge::OverlayDebugSnapshot::default(),
        }));
        let (_input_tx, input_rx) = mpsc::channel();
        let (_command_tx, command_rx) = mpsc::channel();
        let (_mouse_tx, mouse_rx) = mpsc::channel();
        let inputs = TickInputs {
            input_rx: &input_rx,
            held_over: RefCell::default(),
            command_rx: &command_rx,
            mouse_rx: &mouse_rx,
        };
        let step_tick = |rig: &mut Rig| {
            tick(
                &mut rig.machine,
                &inputs,
                &audio,
                &frames,
                &debug,
                &mut rig.control,
                &mut None,
            );
        };

        step_tick(&mut rig);
        assert!(rig.said().is_empty(), "a healthy machine says nothing");
        assert_eq!(rig.machine.crash_message(), None);

        // The plugin panics while handling this key event.
        rig.machine
            .key_event_name(test_machine_plugin_impl::CRASH_KEY, true);
        let error = rig.machine.crash_message().expect("the plugin has crashed");
        assert!(
            error.contains("panic in") || error.contains("crashed earlier"),
            "{error}"
        );

        // The next tick reports it (it must not panic on the clock query) ...
        step_tick(&mut rig);
        let said = rig.said();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].starts_with("The machine crashed"), "{said:?}");
        // ... only once, and the machine stays where it was.
        let held = machine::Machine::pc(&rig.machine);
        step_tick(&mut rig);
        step_tick(&mut rig);
        assert!(rig.said().is_empty(), "reported once");
        assert_eq!(machine::Machine::pc(&rig.machine), held);
    }
}
