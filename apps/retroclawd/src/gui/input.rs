use std::collections::VecDeque;
use std::sync::mpsc::Sender;
use std::time::Duration;

use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::prelude::*;

use super::audio::{AudioControl, AudioControlRes};
use super::debug_overlay::{DebugSnapshotRes, OverlayStateRes};
use super::hotkeys::{Action, HotkeyState, Hotkeys, Trigger};
use super::keys::{KeyRouter, LogicalKeys};
use super::pads::{PadButton, PadRouter, PadSlots, PadState};
use super::shader;
use super::toast::ToastRes;
use super::video::{self, ShaderSettingsRes};
use crate::dialogs::FileDialogs;
use crate::emu_thread::{EmuCommand, MouseEvent};
const AUTO_RUN_STARTUP_DELAY_MS: u64 = 1_000;
const AUTO_RUN_KEY_EVENT_MS: u64 = 140;
const CPC_TAPE_AUTORUN_READY_PC: u64 = 0x1A4F;
/// Real KERNAL "wait for a keypress" idle loop (`$E5CD: LDA $C6 / STA $CC
/// / STA $0292 / BEQ $E5CD`, confirmed by disassembling this project's
/// own C64 KERNAL (`kernal-901227-03`) this session, not
/// recalled -- `$C6` is the real keyboard-buffer count, NDX, the same
/// variable a real `POKE 198,...`/`PEEK(198)` BASIC one-liner reads/
/// writes) -- landing anywhere in this 4-instruction loop is this
/// project's own real, live-confirmed signal that KERNAL/BASIC have
/// finished cold-booting and are genuinely ready to accept typed input,
/// the same role `CPC_TAPE_AUTORUN_READY_PC` plays for CPC. A *range*,
/// not one exact address like the CPC case, since `DebugSnapshotRes`
/// only publishes whatever PC the emu thread's own last-completed step
/// happened to land on -- requiring one specific instruction out of the
/// loop's own 4 would need up to 4x as many snapshots to land on it by
/// chance. Real bug this fixed, found live rather than guessed: an
/// earlier version had no readiness wait for either C64 profile at all
/// (same "just wait `AUTO_RUN_STARTUP_DELAY_MS` and go" shape `CpcDisk`
/// already uses) -- a live GUI run under this sandbox's own software
/// (llvmpipe) rendering showed the primary `LOAD"*",8,1` sequence's
/// keystrokes never reaching the screen at all (confirmed via the
/// debugger's own `/api/memory-bytes` endpoint dumping screen RAM),
/// while the *later*, `C64_LOAD_AUTORUN_FOLLOWUP_DELAY_MS`-delayed `RUN`
/// follow-up worked correctly -- i.e., real GPU/shader-compile startup
/// cost (this project's own audio setup code independently documents
/// this same real, multi-second-capable window) can genuinely outlast a
/// flat 1s startup delay, silently swallowing early keystrokes sent
/// before BASIC's own input loop is actually polling `$C6` yet.
const C64_READY_PC_RANGE: std::ops::RangeInclusive<u64> = 0xE5CD..=0xE5D4;
/// The C128 editor's "wait for a key" loop (`$C25E-$C262`, where a booted
/// machine sits at its `READY.` prompt -- sampled over thousands of steps at
/// the prompt on the real ROMs, not disassembled) -- the C128-mode counterpart
/// of `C64_READY_PC_RANGE`.
const C128_READY_PC_RANGE: std::ops::RangeInclusive<u64> = 0xC25E..=0xC262;
/// Minimum wait before a C64-mode C128 disk load is checked for completion
/// (the real drive protocol is slow: the load itself takes many seconds).
const C128_C64_LOAD_FOLLOWUP_DELAY_MS: u64 = 3_000;
const CPC_TAPE_AUTORUN_WAIT_TIMEOUT_MS: u64 = 20_000;
const CPC_TAPE_AUTORUN_WAIT_LOG_MS: u64 = 1_000;
const CPC_TAPE_AUTORUN_FOLLOWUP_DELAY_MS: u64 = 15_000;
const CPC_TAPE_KEY_HOLD_REPEATS: usize = 4;
/// Real, unlike `CPC_TAPE_AUTORUN_FOLLOWUP_DELAY_MS`'s own 15s (a genuine
/// tape load can take a long time), `C64::try_fast_kernal_load_trap`
/// completes an entire disk LOAD near-instantly -- this only needs to
/// comfortably outlast however many real frames BASIC's own command-line
/// parsing + KERNAL call + return-to-READY takes, an engineering margin
/// (same epistemic tier as `AUTO_RUN_STARTUP_DELAY_MS` itself), not a
/// measured minimum.
const C64_LOAD_AUTORUN_FOLLOWUP_DELAY_MS: u64 = 2_000;

pub(crate) fn stable_host_key_name(key: KeyCode) -> String {
    let name = match key {
        KeyCode::KeyA => "a",
        KeyCode::KeyB => "b",
        KeyCode::KeyC => "c",
        KeyCode::KeyD => "d",
        KeyCode::KeyE => "e",
        KeyCode::KeyF => "f",
        KeyCode::KeyG => "g",
        KeyCode::KeyH => "h",
        KeyCode::KeyI => "i",
        KeyCode::KeyJ => "j",
        KeyCode::KeyK => "k",
        KeyCode::KeyL => "l",
        KeyCode::KeyM => "m",
        KeyCode::KeyN => "n",
        KeyCode::KeyO => "o",
        KeyCode::KeyP => "p",
        KeyCode::KeyQ => "q",
        KeyCode::KeyR => "r",
        KeyCode::KeyS => "s",
        KeyCode::KeyT => "t",
        KeyCode::KeyU => "u",
        KeyCode::KeyV => "v",
        KeyCode::KeyW => "w",
        KeyCode::KeyX => "x",
        KeyCode::KeyY => "y",
        KeyCode::KeyZ => "z",
        KeyCode::Digit0 => "0",
        KeyCode::Digit1 => "1",
        KeyCode::Digit2 => "2",
        KeyCode::Digit3 => "3",
        KeyCode::Digit4 => "4",
        KeyCode::Digit5 => "5",
        KeyCode::Digit6 => "6",
        KeyCode::Digit7 => "7",
        KeyCode::Digit8 => "8",
        KeyCode::Digit9 => "9",
        KeyCode::ArrowLeft => "left",
        KeyCode::ArrowRight => "right",
        KeyCode::ArrowUp => "up",
        KeyCode::ArrowDown => "down",
        KeyCode::Space => "space",
        KeyCode::Enter => "enter",
        KeyCode::ShiftLeft => "shift_left",
        KeyCode::ShiftRight => "shift_right",
        KeyCode::Backspace => "backspace",
        KeyCode::Escape => "escape",
        KeyCode::Comma => "comma",
        KeyCode::Period => "period",
        KeyCode::Slash => "slash",
        KeyCode::Semicolon => "semicolon",
        KeyCode::CapsLock => "caps_lock",
        KeyCode::Quote => "quote",
        KeyCode::Minus => "minus",
        KeyCode::Equal => "equal",
        KeyCode::BracketLeft => "bracket_left",
        KeyCode::BracketRight => "bracket_right",
        KeyCode::Backslash => "backslash",
        KeyCode::Backquote => "backquote",
        KeyCode::F1 => "f1",
        KeyCode::F2 => "f2",
        KeyCode::F3 => "f3",
        KeyCode::F4 => "f4",
        KeyCode::F5 => "f5",
        KeyCode::F6 => "f6",
        KeyCode::F7 => "f7",
        KeyCode::F8 => "f8",
        KeyCode::F9 => "f9",
        KeyCode::F10 => "f10",
        KeyCode::F11 => "f11",
        KeyCode::F12 => "f12",
        _ => return format!("{key:?}").to_ascii_lowercase(),
    };
    name.to_string()
}

/// Channel the emu thread reads queued key events from -- see
/// `emu_thread::tick`'s doc comment. `keyboard_input` below is the only
/// system that sends into it.
#[derive(Resource)]
pub(crate) struct InputTxRes(pub(crate) Sender<(String, bool)>);

/// Startup auto-run profiles, triggered by `--run` in GUI mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoRunProfile {
    /// Send Ctrl+small Enter, then small Enter
    /// ("Press PLAY then any key").
    CpcTape,
    /// Type `RUN"DISC"` + Enter for a mounted disk.
    CpcDisk,
    /// Type `LOAD"*",8,1` + Enter for an attached `.d64`/`.d71`/`.d81`
    /// (serviced near-instantly by `c64::C64`'s own IEC-level fast loader
    /// trap regardless of which real disk geometry is attached -- see
    /// `TODO.md`'s "drives and baseline media" section), then, after a
    /// short follow-up delay, `RUN` + Enter.
    C64Load,
    /// Type `LOAD` + Enter for an attached `.tap` (serviced near-instantly by
    /// `c64::C64`'s fast tape trap: a bare `LOAD` takes the next program on
    /// the tape), then `RUN` + Enter like `C64Load`.
    C64Tape,
    /// Type bare `RUN` + Enter -- for a `.prg`/`.t64` program already
    /// poked directly into memory (no LOAD statement needed at all, since
    /// there's no real KERNAL LOAD/BASIC command involved for either
    /// format -- see `c64-machine-plugin`'s own `finish_basic_program_
    /// load` doc comment for what already made this safe to `RUN`).
    C64Run,
    /// C128 mode with a `--true-drive` disk: type `RUN"*"` + Enter (BASIC 7
    /// loads the first program and runs it).
    C128Disk,
    /// C128 mode, a `.prg` BASIC program already injected at `$1C01`: type
    /// `RUN` + Enter.
    C128Run,
    /// C64 mode on the C128 with a `--true-drive` disk: `LOAD"*",8,1`, then
    /// (once the slow real-drive load is over and the prompt is back) `RUN`.
    C128C64Disk,
}

/// One key event in a scripted `AutoRunState` sequence -- either a real
/// host key (resolved through the same keymap real keyboard input uses,
/// `key_name_for`) or a stable machine-side key name sent as-is. The
/// latter exists for a key with no natural host `KeyCode`/default keymap
/// binding at all -- the C64's own dedicated `Asterisk` key (no modern
/// keyboard has one) is the reason this variant exists; every CPC action
/// below still goes through `Host`, unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoRunKey {
    Host(KeyCode),
    Named(&'static str),
}

/// One-shot scripted keyboard sequence sent into the same input channel as
/// real key presses.
#[derive(Resource)]
pub(crate) struct AutoRunState {
    profile: AutoRunProfile,
    startup_delay: Timer,
    key_repeat: Timer,
    actions: VecDeque<(AutoRunKey, bool)>,
    followup_delay: Option<Timer>,
    followup_actions: VecDeque<(AutoRunKey, bool)>,
    sent_events: usize,
    started: bool,
    waiting_log: Timer,
    waiting_timeout: Timer,
    done: bool,
}
impl AutoRunState {
    pub(crate) fn new(profile: AutoRunProfile) -> Self {
        let (followup_delay, followup_actions) = match profile {
            AutoRunProfile::CpcTape => (
                Some(Timer::new(
                    Duration::from_millis(CPC_TAPE_AUTORUN_FOLLOWUP_DELAY_MS),
                    TimerMode::Once,
                )),
                cpc_tape_followup_actions(),
            ),
            AutoRunProfile::CpcDisk
            | AutoRunProfile::C64Run
            | AutoRunProfile::C128Disk
            | AutoRunProfile::C128Run => (None, VecDeque::new()),
            AutoRunProfile::C128C64Disk => (
                Some(Timer::new(
                    Duration::from_millis(C128_C64_LOAD_FOLLOWUP_DELAY_MS),
                    TimerMode::Once,
                )),
                c64_run_followup_actions(),
            ),
            AutoRunProfile::C64Load | AutoRunProfile::C64Tape => (
                Some(Timer::new(
                    Duration::from_millis(C64_LOAD_AUTORUN_FOLLOWUP_DELAY_MS),
                    TimerMode::Once,
                )),
                c64_run_followup_actions(),
            ),
        };

        Self {
            profile,
            startup_delay: Timer::new(
                Duration::from_millis(AUTO_RUN_STARTUP_DELAY_MS),
                TimerMode::Once,
            ),
            key_repeat: Timer::new(
                Duration::from_millis(AUTO_RUN_KEY_EVENT_MS),
                TimerMode::Repeating,
            ),
            actions: auto_run_actions(profile),
            followup_delay,
            followup_actions,
            sent_events: 0,
            started: false,
            waiting_log: Timer::new(
                Duration::from_millis(CPC_TAPE_AUTORUN_WAIT_LOG_MS),
                TimerMode::Repeating,
            ),
            waiting_timeout: Timer::new(
                Duration::from_millis(CPC_TAPE_AUTORUN_WAIT_TIMEOUT_MS),
                TimerMode::Once,
            ),
            done: false,
        }
    }
}

fn register_value(registers: &[(&'static str, u64)], name: &str) -> Option<u64> {
    registers
        .iter()
        .find_map(|(register_name, value)| (*register_name == name).then_some(*value))
}

fn current_pc(snapshot: Option<&Res<DebugSnapshotRes>>) -> Option<u64> {
    let snapshot = snapshot?;
    let guard = snapshot.0.lock().ok()?;
    register_value(&guard.registers, "PC")
}

/// Whether this profile needs to wait for a real, confirmed "ready for
/// input" signal at all before typing -- `CpcDisk` doesn't (its own
/// `RUN"DISC"` idiom just fires after the flat startup delay, same as it
/// always has); every other profile here landed on a genuine correctness
/// gap without one (see `C64_READY_PC_RANGE`'s own doc comment for how
/// this was found for the C64 profiles specifically).
fn auto_run_needs_readiness_wait(profile: AutoRunProfile) -> bool {
    !matches!(profile, AutoRunProfile::CpcDisk)
}

fn auto_run_ready(profile: AutoRunProfile, snapshot: Option<&Res<DebugSnapshotRes>>) -> bool {
    match profile {
        AutoRunProfile::CpcTape => current_pc(snapshot) == Some(CPC_TAPE_AUTORUN_READY_PC),
        AutoRunProfile::C64Load
        | AutoRunProfile::C64Tape
        | AutoRunProfile::C64Run
        | AutoRunProfile::C128C64Disk => {
            current_pc(snapshot).is_some_and(|pc| C64_READY_PC_RANGE.contains(&pc))
        }
        AutoRunProfile::C128Disk | AutoRunProfile::C128Run => {
            current_pc(snapshot).is_some_and(|pc| C128_READY_PC_RANGE.contains(&pc))
        }
        AutoRunProfile::CpcDisk => true,
    }
}

fn auto_run_ready_target_desc(profile: AutoRunProfile) -> String {
    match profile {
        AutoRunProfile::CpcTape => format!("0x{CPC_TAPE_AUTORUN_READY_PC:04X}"),
        AutoRunProfile::C64Load
        | AutoRunProfile::C64Tape
        | AutoRunProfile::C64Run
        | AutoRunProfile::C128C64Disk => format!(
            "0x{:04X}..=0x{:04X}",
            C64_READY_PC_RANGE.start(),
            C64_READY_PC_RANGE.end()
        ),
        AutoRunProfile::C128Disk | AutoRunProfile::C128Run => format!(
            "0x{:04X}..=0x{:04X}",
            C128_READY_PC_RANGE.start(),
            C128_READY_PC_RANGE.end()
        ),
        AutoRunProfile::CpcDisk => String::new(),
    }
}

fn push_key(actions: &mut VecDeque<(AutoRunKey, bool)>, key: KeyCode) {
    actions.push_back((AutoRunKey::Host(key), true));
    actions.push_back((AutoRunKey::Host(key), false));
}

fn push_held_key(actions: &mut VecDeque<(AutoRunKey, bool)>, key: KeyCode, hold_repeats: usize) {
    actions.push_back((AutoRunKey::Host(key), true));
    for _ in 0..hold_repeats {
        actions.push_back((AutoRunKey::Host(key), true));
    }
    actions.push_back((AutoRunKey::Host(key), false));
}

fn push_quote(actions: &mut VecDeque<(AutoRunKey, bool)>) {
    // Both CPC's and the C64's own real keyboards produce `"` via Shift+2.
    actions.push_back((AutoRunKey::Host(KeyCode::ShiftLeft), true));
    actions.push_back((AutoRunKey::Host(KeyCode::Digit2), true));
    actions.push_back((AutoRunKey::Host(KeyCode::Digit2), false));
    actions.push_back((AutoRunKey::Host(KeyCode::ShiftLeft), false));
}

/// A stable machine-side key name with no natural host key/default
/// keymap binding at all -- see `AutoRunKey::Named`'s own doc comment.
fn push_named_key(actions: &mut VecDeque<(AutoRunKey, bool)>, name: &'static str) {
    actions.push_back((AutoRunKey::Named(name), true));
    actions.push_back((AutoRunKey::Named(name), false));
}

fn auto_run_actions(profile: AutoRunProfile) -> VecDeque<(AutoRunKey, bool)> {
    use KeyCode::*;

    let mut actions = VecDeque::new();
    match profile {
        AutoRunProfile::CpcTape => {
            // Match ZEsarUX's CPC tape autoload helper: Ctrl+small Enter,
            // then one extra small Enter for "Press PLAY then any key".
            actions.push_back((AutoRunKey::Host(ControlLeft), true));
            push_held_key(&mut actions, NumpadEnter, CPC_TAPE_KEY_HOLD_REPEATS);
            actions.push_back((AutoRunKey::Host(ControlLeft), false));
            push_held_key(&mut actions, NumpadEnter, CPC_TAPE_KEY_HOLD_REPEATS);
        }
        AutoRunProfile::CpcDisk => {
            // RUN"DISC"
            push_key(&mut actions, KeyR);
            push_key(&mut actions, KeyU);
            push_key(&mut actions, KeyN);
            push_quote(&mut actions);
            push_key(&mut actions, KeyD);
            push_key(&mut actions, KeyI);
            push_key(&mut actions, KeyS);
            push_key(&mut actions, KeyC);
            push_quote(&mut actions);
            push_key(&mut actions, Enter);
        }
        AutoRunProfile::C128Disk => {
            // RUN"*"
            push_key(&mut actions, KeyR);
            push_key(&mut actions, KeyU);
            push_key(&mut actions, KeyN);
            push_quote(&mut actions);
            push_named_key(&mut actions, "Asterisk");
            push_quote(&mut actions);
            push_key(&mut actions, Enter);
        }
        AutoRunProfile::C128Run => {
            push_key(&mut actions, KeyR);
            push_key(&mut actions, KeyU);
            push_key(&mut actions, KeyN);
            push_key(&mut actions, Enter);
        }
        AutoRunProfile::C64Load | AutoRunProfile::C128C64Disk => {
            // LOAD"*",8,1 -- the real, universally-relied-upon CBM DOS
            // "load the first program on the disk" idiom, serviced by
            // `C64::try_fast_kernal_load_trap`. `RUN` itself is a
            // follow-up phase (`c64_run_followup_actions`), since C64
            // BASIC's LOAD never auto-runs the way CPC's `RUN"DISC"` does.
            push_key(&mut actions, KeyL);
            push_key(&mut actions, KeyO);
            push_key(&mut actions, KeyA);
            push_key(&mut actions, KeyD);
            push_quote(&mut actions);
            // The C64's own dedicated Asterisk key -- no natural host
            // KeyCode/default keymap binding exists for it.
            push_named_key(&mut actions, "Asterisk");
            push_quote(&mut actions);
            push_key(&mut actions, Comma);
            push_key(&mut actions, Digit8);
            push_key(&mut actions, Comma);
            push_key(&mut actions, Digit1);
            push_key(&mut actions, Enter);
        }
        AutoRunProfile::C64Tape => {
            // A bare LOAD: device 1 (tape), no file name -- the trap loads
            // the next program on the tape.
            push_key(&mut actions, KeyL);
            push_key(&mut actions, KeyO);
            push_key(&mut actions, KeyA);
            push_key(&mut actions, KeyD);
            push_key(&mut actions, Enter);
        }
        AutoRunProfile::C64Run => {
            push_key(&mut actions, KeyR);
            push_key(&mut actions, KeyU);
            push_key(&mut actions, KeyN);
            push_key(&mut actions, Enter);
        }
    }

    actions
}
fn cpc_tape_followup_actions() -> VecDeque<(AutoRunKey, bool)> {
    let mut actions = VecDeque::new();
    // Some CPC tape loaders prompt for another "any key" after an
    // extended black-screen load phase. Keep this as the same small Enter
    // key shape as the initial prompt handling.
    push_held_key(
        &mut actions,
        KeyCode::NumpadEnter,
        CPC_TAPE_KEY_HOLD_REPEATS,
    );
    actions
}

/// `RUN` + Enter, typed once `C64Load`'s own `LOAD"*",8,1` has had time to
/// complete -- see `C64_LOAD_AUTORUN_FOLLOWUP_DELAY_MS`'s own doc comment.
fn c64_run_followup_actions() -> VecDeque<(AutoRunKey, bool)> {
    use KeyCode::*;
    let mut actions = VecDeque::new();
    push_key(&mut actions, KeyR);
    push_key(&mut actions, KeyU);
    push_key(&mut actions, KeyN);
    push_key(&mut actions, Enter);
    actions
}

fn advance_auto_run(state: &mut AutoRunState, input_tx: &InputTxRes, router: &KeyRouter) -> bool {
    if let Some((key, pressed)) = state.actions.pop_front() {
        state.sent_events += 1;
        let names = match key {
            AutoRunKey::Host(code) => router.plain_names(code),
            AutoRunKey::Named(name) => vec![name.to_string()],
        };
        tracing::info!(
            profile = ?state.profile,
            event = state.sent_events,
            key = ?key,
            pressed,
            remaining = state.actions.len(),
            "auto-run: key event"
        );
        for name in names {
            if let Err(e) = input_tx.0.send((name, pressed)) {
                tracing::warn!(key = ?key, pressed, error = %e, "auto-run key delivery failed");
                state.done = true;
                break;
            }
        }
        true
    } else {
        false
    }
}
pub(crate) fn auto_run_startup_sequence(
    time: Res<Time>,
    snapshot: Option<Res<DebugSnapshotRes>>,
    input_tx: Res<InputTxRes>,
    router: Res<KeyRouter>,
    auto_run: Option<ResMut<AutoRunState>>,
) {
    let Some(mut auto_run) = auto_run else {
        return;
    };
    if auto_run.done {
        return;
    }

    if !auto_run.started {
        auto_run.startup_delay.tick(time.delta());
        if !auto_run.startup_delay.is_finished() {
            return;
        }

        if auto_run_needs_readiness_wait(auto_run.profile)
            && !auto_run_ready(auto_run.profile, snapshot.as_ref())
        {
            auto_run.waiting_log.tick(time.delta());
            auto_run.waiting_timeout.tick(time.delta());
            let observed_pc = current_pc(snapshot.as_ref());
            let target = auto_run_ready_target_desc(auto_run.profile);
            if auto_run.waiting_log.just_finished() {
                tracing::info!(
                    profile = ?auto_run.profile,
                    target_pc = %target,
                    observed_pc = observed_pc.map(|pc| format!("0x{pc:04X}")),
                    timeout_ms = CPC_TAPE_AUTORUN_WAIT_TIMEOUT_MS,
                    "auto-run: waiting for machine ready state"
                );
            }
            if !auto_run.waiting_timeout.is_finished() {
                return;
            }
            tracing::warn!(
                profile = ?auto_run.profile,
                target_pc = %target,
                observed_pc = observed_pc.map(|pc| format!("0x{pc:04X}")),
                timeout_ms = CPC_TAPE_AUTORUN_WAIT_TIMEOUT_MS,
                "auto-run: ready-state wait timed out; starting sequence anyway"
            );
        }

        auto_run.started = true;
        tracing::info!(
            profile = ?auto_run.profile,
            startup_delay_ms = AUTO_RUN_STARTUP_DELAY_MS,
            key_event_ms = AUTO_RUN_KEY_EVENT_MS,
            events = auto_run.actions.len(),
            trigger_pc = current_pc(snapshot.as_ref()).map(|pc| format!("0x{pc:04X}")),
            "auto-run: starting"
        );
        advance_auto_run(&mut auto_run, &input_tx, &router);
        auto_run.key_repeat.reset();
        return;
    }

    if !auto_run.actions.is_empty() {
        auto_run.key_repeat.tick(time.delta());
        if auto_run.key_repeat.just_finished() {
            let _ = advance_auto_run(&mut auto_run, &input_tx, &router);
        }
        return;
    }

    let mut followup_ready = false;
    if let Some(delay) = auto_run.followup_delay.as_mut() {
        delay.tick(time.delta());
        followup_ready = delay.is_finished();
    }
    // A real-drive load takes as long as it takes: only type the follow-up once
    // the prompt is back.
    if followup_ready
        && matches!(auto_run.profile, AutoRunProfile::C128C64Disk)
        && !auto_run_ready(auto_run.profile, snapshot.as_ref())
    {
        return;
    }

    if followup_ready {
        auto_run.followup_delay = None;
        auto_run.actions = std::mem::take(&mut auto_run.followup_actions);
        tracing::info!(
            profile = ?auto_run.profile,
            followup_delay_ms = CPC_TAPE_AUTORUN_FOLLOWUP_DELAY_MS,
            events = auto_run.actions.len(),
            "auto-run: starting delayed follow-up key phase"
        );
        if auto_run.actions.is_empty() {
            auto_run.done = true;
            tracing::info!(events = auto_run.sent_events, "auto-run: sequence complete");
            return;
        }
        let _ = advance_auto_run(&mut auto_run, &input_tx, &router);
        auto_run.key_repeat.reset();
        return;
    }

    if auto_run.followup_delay.is_some() {
        return;
    }

    auto_run.done = true;
    tracing::info!(events = auto_run.sent_events, "auto-run: sequence complete");
}

/// Channel the emu thread reads host actions from (reset, pause, screenshot,
/// quick save/load, fast-forward) -- see `emu_thread::EmuCommand`.
/// `hotkey_input` below is the only system that sends into it.
#[derive(Resource)]
pub(crate) struct CommandTxRes(pub(crate) Sender<EmuCommand>);

/// Polls the configured hotkeys (`gui::hotkeys`) once per frame and carries
/// out the actions that live on the GUI side (fullscreen, quit) or forwards
/// the rest to the emu thread as `EmuCommand`s. Runs before
/// `keyboard_input`, which skips every key this system has swallowed, so a
/// key bound to a hotkey (F12 reset by default) never also reaches the
/// emulated machine -- neither its press nor its release.
// Bevy hands a system everything it touches as one parameter each; grouping
// them into a `SystemParam` struct would only move the same list one level down.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hotkey_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    hotkeys: Res<Hotkeys>,
    mut state: ResMut<HotkeyState>,
    command_tx: Res<CommandTxRes>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
    audio: Res<AudioControlRes>,
    mut overlay: ResMut<OverlayStateRes>,
    mut toast: ResMut<ToastRes>,
    mut shaders: ResMut<Assets<Shader>>,
    mut shader_settings: ResMut<ShaderSettingsRes>,
    dialogs: Res<FileDialogs>,
    router: Res<KeyRouter>,
    input_tx: Res<InputTxRes>,
    mut paste: ResMut<super::paste::PasteState>,
) {
    // A file dialog (or its question) is open: nothing typed meanwhile may
    // trigger a hotkey or reach the machine.
    if dialogs.is_busy() {
        state.swallow_all(&keyboard);
        return;
    }
    for trigger in hotkeys.poll(&keyboard, &mut state) {
        let send = |command: EmuCommand| {
            let _ = command_tx.0.send(command);
        };
        if let Trigger::Pressed(action) = trigger
            && let Some(file_action) = action.file_action()
        {
            dialogs.start(file_action);
            continue;
        }
        match trigger {
            Trigger::Pressed(Action::Reset) => send(EmuCommand::Reset),
            Trigger::Pressed(Action::Freeze) => send(EmuCommand::Freeze),
            Trigger::Pressed(Action::Paste) => {
                toast.push(super::paste::toggle(
                    &mut paste,
                    &router,
                    &input_tx,
                    super::paste::clipboard_text,
                ));
            }
            Trigger::Pressed(Action::Pause) => send(EmuCommand::TogglePause),
            Trigger::Pressed(Action::Screenshot) => send(EmuCommand::Screenshot),
            Trigger::Pressed(Action::QuickSave) => send(EmuCommand::QuickSave),
            Trigger::Pressed(Action::QuickLoad) => send(EmuCommand::QuickLoad),
            Trigger::Pressed(Action::StateSlotNext) => send(EmuCommand::StateSlot(1)),
            Trigger::Pressed(Action::StateSlotPrev) => send(EmuCommand::StateSlot(-1)),
            Trigger::Pressed(Action::FastForward) => send(EmuCommand::FastForward(true)),
            Trigger::Released(Action::FastForward) => send(EmuCommand::FastForward(false)),
            Trigger::Pressed(Action::Fullscreen) => {
                if let Ok(mut window) = windows.single_mut() {
                    super::window::toggle_fullscreen(&mut window);
                }
            }
            Trigger::Pressed(Action::Quit) => {
                exit.write(AppExit::Success);
            }
            Trigger::Pressed(Action::Mute) => {
                toast.push(if audio.0.toggle_mute() {
                    "Sound muted".to_string()
                } else {
                    format!("Sound on ({}%)", audio.0.volume_percent())
                });
            }
            Trigger::Pressed(action @ (Action::VolumeUp | Action::VolumeDown)) => {
                let step = if action == Action::VolumeUp {
                    AudioControl::STEP
                } else {
                    -AudioControl::STEP
                };
                toast.push(format!("Volume {}%", audio.0.adjust_volume(step)));
            }
            Trigger::Pressed(Action::Overlay) => {
                overlay.toggle();
            }
            Trigger::Pressed(action @ (Action::ShaderNext | Action::ShaderPrev)) => {
                let delta = if action == Action::ShaderNext { 1 } else { -1 };
                let name = shader::cycle(shader_settings.name.as_deref(), delta);
                let switched = shader::resolve_source(Some(name))
                    .is_ok_and(|source| video::switch_shader(&mut shaders, source));
                if switched {
                    shader_settings.name = Some(name.to_string());
                    toast.push(format!("Shader: {name}"));
                } else {
                    toast.push("Shader switch failed");
                }
            }
            // The file actions were handled above; hold actions other than
            // fast-forward do not exist.
            Trigger::Pressed(_) | Trigger::Released(_) => {}
        }
    }
}

/// Forwards the keyboard to the emulated machine through `KeyRouter` (see
/// `gui::keys`): bindings from `[keys]`, chords, the combined Shift, and
/// skipping every key `hotkey_input` swallowed this frame. Sends events over
/// `InputTxRes` rather than calling the machine directly -- the machine lives
/// on the emu thread.
pub(crate) fn keyboard_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    input_tx: Res<InputTxRes>,
    hotkey_state: Res<HotkeyState>,
    logical: Res<LogicalKeys>,
    mut router: ResMut<KeyRouter>,
) {
    for event in router.route(&keyboard, &hotkey_state, &logical) {
        let _ = input_tx.0.send(event);
    }
}

/// Records which character each host key typed as it went down, for the
/// symbolic keyboard layout (`gui::keys::Layout`). Runs before
/// `keyboard_input`, in the same frame as the events that updated
/// `ButtonInput<KeyCode>`. Only keys that type a single plain character are
/// recorded: Shift, F-keys, arrows, Enter and dead keys fall through to the
/// physical key's own name.
pub(crate) fn record_logical_keys(
    mut events: MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut logical: ResMut<LogicalKeys>,
) {
    use bevy::input::keyboard::Key;
    for event in events.read() {
        if !event.state.is_pressed() {
            logical.released(event.key_code);
            continue;
        }
        if event.repeat {
            continue;
        }
        let typed = match &event.logical_key {
            Key::Character(text) => {
                let mut chars = text.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Some(c),
                    _ => None,
                }
            }
            _ => None,
        };
        tracing::debug!(
            target: "retroclawd::input",
            "key down: physical={:?} typed={:?}",
            event.key_code,
            typed
        );
        logical.pressed(event.key_code, typed);
    }
}

/// Reads every connected gamepad once per frame and sends the machine keys
/// its buttons are bound to (`gui::pads`). Pads are numbered by `PadSlots`: the
/// first pad plugged in is `[pads.1]`.
pub(crate) fn pad_input(
    gamepads: Query<(Entity, &Gamepad)>,
    input_tx: Res<InputTxRes>,
    mut pads: ResMut<PadRouter>,
    mut slots: ResMut<PadSlots>,
    mut router: ResMut<KeyRouter>,
) {
    let deadzone = pads.deadzone();
    let present: Vec<u64> = gamepads.iter().map(|(e, _)| e.to_bits()).collect();
    let states: Vec<(u8, PadState)> = slots
        .assign(&present)
        .into_iter()
        .filter_map(|(slot, id)| {
            gamepads
                .iter()
                .find(|(entity, _)| entity.to_bits() == id)
                .map(|(_, gamepad)| (slot, pad_state(gamepad, deadzone)))
        })
        .collect();
    let (releases, presses) = pads.route(&states);
    for event in router.apply(releases, presses) {
        let _ = input_tx.0.send(event);
    }
}

/// One pad's buttons as the digital `PadButton` set.
fn pad_state(gamepad: &Gamepad, deadzone: f32) -> PadState {
    let mut state = PadState::default();
    for (button, source) in [
        (PadButton::Up, GamepadButton::DPadUp),
        (PadButton::Down, GamepadButton::DPadDown),
        (PadButton::Left, GamepadButton::DPadLeft),
        (PadButton::Right, GamepadButton::DPadRight),
        (PadButton::South, GamepadButton::South),
        (PadButton::East, GamepadButton::East),
        (PadButton::West, GamepadButton::West),
        (PadButton::North, GamepadButton::North),
        (PadButton::L1, GamepadButton::LeftTrigger),
        (PadButton::R1, GamepadButton::RightTrigger),
        (PadButton::L2, GamepadButton::LeftTrigger2),
        (PadButton::R2, GamepadButton::RightTrigger2),
        (PadButton::Select, GamepadButton::Select),
        (PadButton::Start, GamepadButton::Start),
        (PadButton::L3, GamepadButton::LeftThumb),
        (PadButton::R3, GamepadButton::RightThumb),
    ] {
        state.set(button, gamepad.pressed(source));
    }
    let stick = gamepad.left_stick();
    state.set_stick(stick.x, stick.y, deadzone);
    state
}

/// Channel the emu thread reads queued Kempston-mouse-shaped events from --
/// see `emu_thread::MouseEvent`'s own doc comment. `mouse_input` below is
/// the only system that sends into it.
#[derive(Resource)]
pub(crate) struct MouseTxRes(pub(crate) Sender<MouseEvent>);

/// Reads real host mouse motion/buttons/wheel and forwards them as
/// `MouseEvent`s -- a no-op on every machine but `ZxSpectrum` (see
/// `HostedMachine::mouse_event`), same "send regardless, let the receiving
/// side decide relevance" shape `keyboard_input` already uses (harmless on
/// machines with no Kempston mouse modeled).
///
/// Motion is summed across every `MouseMotion` event this frame and sent
/// as one relative delta (real Kempston hardware reports relative motion,
/// not an absolute screen position -- see `zx_spectrum::bus::
/// ZxSpectrumBus::move_mouse`'s own doc comment), clamped to `i8` range
/// per send so a single very-fast mouse swipe can't overflow the real
/// port's 8-bit-per-event granularity; Y is passed through unmodified
/// (not inverted) -- an unverified engineering choice, not a confirmed
/// real Kempston-mouse-driver convention, same epistemic tier as
/// `zx_spectrum::SpeakerType`'s filter cutoffs.
pub(crate) fn mouse_input(
    mut motion_events: MessageReader<MouseMotion>,
    mut wheel_events: MessageReader<MouseWheel>,
    buttons: Res<ButtonInput<bevy::input::mouse::MouseButton>>,
    mouse_tx: Res<MouseTxRes>,
) {
    let mut dx = 0.0f32;
    let mut dy = 0.0f32;
    for ev in motion_events.read() {
        dx += ev.delta.x;
        dy += ev.delta.y;
    }
    if dx != 0.0 || dy != 0.0 {
        let _ = mouse_tx.0.send(MouseEvent::Motion {
            dx: dx.clamp(i8::MIN as f32, i8::MAX as f32) as i8,
            dy: dy.clamp(i8::MIN as f32, i8::MAX as f32) as i8,
        });
    }

    // One tick per wheel event, sign only -- real hardware's own port
    // accumulates a 4-bit delta *count* since the last CPU read, not a
    // pixel/line magnitude, so summing raw `ev.y` values here (which can
    // be fractional "pixel" units depending on `MouseScrollUnit`) would
    // misrepresent what the real port actually counts.
    for ev in wheel_events.read() {
        let delta = ev.y.signum() as i8;
        if delta != 0 {
            let _ = mouse_tx.0.send(MouseEvent::Scroll { delta });
        }
    }

    for button in buttons.get_just_pressed() {
        let _ = mouse_tx.0.send(MouseEvent::Button {
            button: *button,
            pressed: true,
        });
    }
    for button in buttons.get_just_released() {
        let _ = mouse_tx.0.send(MouseEvent::Button {
            button: *button,
            pressed: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AutoRunKey, AutoRunProfile, AutoRunState, CPC_TAPE_KEY_HOLD_REPEATS, auto_run_actions,
        auto_run_needs_readiness_wait, auto_run_ready, auto_run_ready_target_desc,
        c64_run_followup_actions, cpc_tape_followup_actions,
    };
    use bevy::input::keyboard::KeyCode;

    fn host(key: KeyCode) -> AutoRunKey {
        AutoRunKey::Host(key)
    }

    #[test]
    fn c64_profiles_need_a_readiness_wait_but_cpc_disk_does_not() {
        assert!(auto_run_needs_readiness_wait(AutoRunProfile::C64Load));
        assert!(auto_run_needs_readiness_wait(AutoRunProfile::C64Run));
        assert!(auto_run_needs_readiness_wait(AutoRunProfile::CpcTape));
        assert!(
            !auto_run_needs_readiness_wait(AutoRunProfile::CpcDisk),
            "CpcDisk's own RUN\"DISC\" idiom never waited for a ready signal before this fix"
        );
    }

    #[test]
    fn auto_run_ready_target_desc_names_the_real_c64_idle_loop_range() {
        let desc = auto_run_ready_target_desc(AutoRunProfile::C64Load);
        assert!(desc.contains("E5CD"), "got: {desc}");
        assert!(desc.contains("E5D4"), "got: {desc}");
    }

    #[test]
    fn auto_run_ready_with_no_snapshot_is_never_ready() {
        assert!(!auto_run_ready(AutoRunProfile::C64Load, None));
        assert!(!auto_run_ready(AutoRunProfile::C64Run, None));
        assert!(!auto_run_ready(AutoRunProfile::CpcTape, None));
    }

    #[test]
    fn cpc_tape_sequence_sends_ctrl_small_enter_then_small_enter() {
        let actions: Vec<_> = auto_run_actions(AutoRunProfile::CpcTape)
            .into_iter()
            .collect();

        let mut cursor = 0usize;
        assert_eq!(actions[cursor], (host(KeyCode::ControlLeft), true));
        cursor += 1;

        for _ in 0..=CPC_TAPE_KEY_HOLD_REPEATS {
            assert_eq!(actions[cursor], (host(KeyCode::NumpadEnter), true));
            cursor += 1;
        }
        assert_eq!(actions[cursor], (host(KeyCode::NumpadEnter), false));
        cursor += 1;

        assert_eq!(actions[cursor], (host(KeyCode::ControlLeft), false));
        cursor += 1;

        for _ in 0..=CPC_TAPE_KEY_HOLD_REPEATS {
            assert_eq!(actions[cursor], (host(KeyCode::NumpadEnter), true));
            cursor += 1;
        }
        assert_eq!(actions[cursor], (host(KeyCode::NumpadEnter), false));
        cursor += 1;

        assert_eq!(cursor, actions.len());
        assert!(
            !actions.iter().any(|(key, _)| *key == host(KeyCode::Enter)),
            "CPC tape autorun must use the small Enter key (NumpadEnter), not Return"
        );
    }

    #[test]
    fn cpc_disk_sequence_keeps_single_enter_after_run_disc() {
        let actions: Vec<_> = auto_run_actions(AutoRunProfile::CpcDisk)
            .into_iter()
            .collect();
        let enter_presses = actions
            .iter()
            .filter(|(key, pressed)| *pressed && *key == host(KeyCode::Enter))
            .count();

        assert_eq!(enter_presses, 1);
    }

    #[test]
    fn cpc_tape_followup_actions_keep_using_small_enter() {
        let actions: Vec<_> = cpc_tape_followup_actions().into_iter().collect();
        let enter_presses = actions
            .iter()
            .filter(|(key, pressed)| *pressed && *key == host(KeyCode::Enter))
            .count();
        let numpad_enter_presses = actions
            .iter()
            .filter(|(key, pressed)| *pressed && *key == host(KeyCode::NumpadEnter))
            .count();

        assert_eq!(enter_presses, 0, "follow-up must not use Return");
        assert!(
            numpad_enter_presses > 0,
            "follow-up must send small Enter as the any-key action"
        );
    }

    #[test]
    fn c64_load_sequence_types_load_star_8_1_using_the_named_asterisk_key() {
        let actions: Vec<_> = auto_run_actions(AutoRunProfile::C64Load)
            .into_iter()
            .collect();

        assert!(
            actions
                .iter()
                .any(|(key, pressed)| *pressed && *key == AutoRunKey::Named("Asterisk")),
            "must type the C64's own dedicated Asterisk key for the '*' wildcard"
        );
        let enter_presses = actions
            .iter()
            .filter(|(key, pressed)| *pressed && *key == host(KeyCode::Enter))
            .count();
        assert_eq!(
            enter_presses, 1,
            "LOAD\"*\",8,1 is one statement, one Enter"
        );
    }

    #[test]
    fn c128_profiles_type_run_star_or_run_and_wait_for_their_own_idle_loop() {
        let keys = |profile| {
            auto_run_actions(profile)
                .into_iter()
                .filter(|(_, pressed)| *pressed)
                .map(|(key, _)| key)
                .collect::<Vec<_>>()
        };
        let disk = keys(AutoRunProfile::C128Disk);
        assert!(disk.contains(&AutoRunKey::Named("Asterisk")));
        assert_eq!(
            disk.iter().filter(|k| **k == host(KeyCode::Enter)).count(),
            1
        );
        assert_eq!(
            keys(AutoRunProfile::C128Run),
            [KeyCode::KeyR, KeyCode::KeyU, KeyCode::KeyN, KeyCode::Enter].map(host)
        );
        assert!(auto_run_needs_readiness_wait(AutoRunProfile::C128Disk));
        // C128 mode idles in the editor, C64 mode in the KERNAL.
        assert_ne!(
            auto_run_ready_target_desc(AutoRunProfile::C128Run),
            auto_run_ready_target_desc(AutoRunProfile::C128C64Disk)
        );
        // The C64-mode disk types LOAD first and RUN only as the follow-up.
        assert!(
            AutoRunState::new(AutoRunProfile::C128C64Disk)
                .followup_delay
                .is_some()
        );
    }

    #[test]
    fn c64_run_sequence_and_load_followup_both_type_bare_run() {
        for actions in [
            auto_run_actions(AutoRunProfile::C64Run)
                .into_iter()
                .collect::<Vec<_>>(),
            c64_run_followup_actions().into_iter().collect::<Vec<_>>(),
        ] {
            assert_eq!(
                actions
                    .iter()
                    .filter(|(key, pressed)| *pressed && *key == host(KeyCode::Enter))
                    .count(),
                1
            );
            assert!(
                !actions
                    .iter()
                    .any(|(key, _)| *key == AutoRunKey::Named("Asterisk")),
                "a bare RUN must never type the wildcard key"
            );
        }
    }

    #[test]
    fn c64_load_profile_enables_a_short_delayed_followup_run_and_c64_run_does_not() {
        let load_state = AutoRunState::new(AutoRunProfile::C64Load);
        assert!(load_state.followup_delay.is_some());
        assert!(!load_state.followup_actions.is_empty());

        let run_state = AutoRunState::new(AutoRunProfile::C64Run);
        assert!(run_state.followup_delay.is_none());
        assert!(run_state.followup_actions.is_empty());
    }

    #[test]
    fn cpc_tape_profile_enables_delayed_followup_phase() {
        let state = AutoRunState::new(AutoRunProfile::CpcTape);
        assert!(state.followup_delay.is_some());
        assert!(
            !state.followup_actions.is_empty(),
            "CPC tape profile should queue delayed follow-up keys"
        );

        let disk_state = AutoRunState::new(AutoRunProfile::CpcDisk);
        assert!(disk_state.followup_delay.is_none());
        assert!(disk_state.followup_actions.is_empty());
    }

    #[test]
    fn pad_input_sends_the_bound_machine_keys_for_each_connected_pad() {
        use super::super::keys::{KeyBindings, KeyNames, KeyRouter};
        use super::super::pads::{PadBindings, PadRouter};
        use super::{InputTxRes, pad_input};
        use bevy::prelude::*;

        let names = KeyNames::only(&plugin_api::key_infos([
            "Joy1Up",
            "Joy1Left",
            "Joy1Fire1",
            "Joy2Fire1",
        ]));
        let bindings =
            PadBindings::resolve(&app_config::AppConfig::default(), "m", &names).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new();
        app.insert_resource(InputTxRes(tx))
            .insert_resource(KeyRouter::new(KeyBindings::default()))
            .insert_resource(PadRouter::new(bindings))
            .insert_resource(super::super::pads::PadSlots::default())
            .add_systems(Update, pad_input);

        let first = app.world_mut().spawn(Gamepad::default()).id();
        let drain = |rx: &std::sync::mpsc::Receiver<(String, bool)>| {
            let mut got: Vec<_> = rx.try_iter().collect();
            got.sort();
            got
        };
        let ev = |name: &str, down: bool| (name.to_string(), down);

        // D-pad up + South on the first pad.
        {
            let mut e = app.world_mut().entity_mut(first);
            let mut pad = e.get_mut::<Gamepad>().unwrap();
            pad.digital_mut().press(GamepadButton::DPadUp);
            pad.digital_mut().press(GamepadButton::South);
        }
        app.update();
        assert_eq!(drain(&rx), [ev("Joy1Fire1", true), ev("Joy1Up", true)]);

        // The left stick past the deadzone is a direction too.
        {
            let mut e = app.world_mut().entity_mut(first);
            let mut pad = e.get_mut::<Gamepad>().unwrap();
            pad.analog_mut().set(GamepadAxis::LeftStickX, -0.9);
        }
        app.update();
        assert_eq!(drain(&rx), [ev("Joy1Left", true)]);

        // A second pad is slot 2.
        let second = app.world_mut().spawn(Gamepad::default()).id();
        app.world_mut()
            .entity_mut(second)
            .get_mut::<Gamepad>()
            .unwrap()
            .digital_mut()
            .press(GamepadButton::East);
        app.update();
        assert_eq!(drain(&rx), [ev("Joy2Fire1", true)]);

        // Unplugging the first releases what it held; the second stays down.
        app.world_mut().despawn(first);
        app.update();
        assert_eq!(
            drain(&rx),
            [
                ev("Joy1Fire1", false),
                ev("Joy1Left", false),
                ev("Joy1Up", false),
            ]
        );
    }
}
