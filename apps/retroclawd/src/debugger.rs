//! Shared vocabulary the emu thread (`emu_thread.rs`) and the debugger
//! plugin runtime threads speak -- owned by neither, same reasoning as
//! `any_machine.rs` being a shared type both `main.rs` and `gui/mod.rs` import.
//! `gui/mod.rs` import. Mirrors two patterns `emu_thread.rs` already
//! established: `AudioQueue`/`FrameBuffer`/`DebugSnapshotHandle`'s
//! `Arc<Mutex<T>>` one-way-published-state handoff, and `input_tx`'s
//! one-way `mpsc` command channel.

use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use bevy::input::keyboard::KeyCode;
use plugin_api::{
    BitmapExplorerDefaults, BitmapPaletteColor, ChipSnapshot, CpuFamily, PortHitInfo, PortWatchSpec,
};

use crate::plugin_bridge::HostedMachine;
/// One-way commands from debugger plugin runtime threads into the emu thread,
/// drained with `try_recv()` at the top of every `emu_thread::tick` call
/// before stepping -- mirrors `EmuThreadHandle::input_tx`'s existing shape
/// exactly. Every variant is small and owned (no borrows) so it can cross
/// threads freely.
#[derive(Debug)]
pub(crate) enum DebuggerCommand {
    Pause,
    Resume,
    ResetMachine,
    KeyEvent {
        key: String,
        pressed: bool,
    },
    /// Execute exactly one instruction, then return to `Paused`. Silently
    /// ignored by `DebuggerLink::drain_commands` if the machine isn't
    /// currently paused -- stepping only makes sense from a stopped state.
    StepInstruction,
    /// Execute one instruction with step-over semantics: if the current
    /// instruction is a call-like control transfer (`CALL`/`RST`), keep
    /// running until the post-instruction fallthrough address is reached (or
    /// another breakpoint fires first).
    StepOverInstruction,
    /// Restore the previous debugger-captured machine state (one instruction
    /// of reverse-stepping history).
    BackstepInstruction,
    SetPc(u16),
    SetRegister {
        name: String,
        value: u64,
    },
    SetBreakpoint {
        addr: u16,
    },
    SetBreakpointCondition {
        addr: u16,
        condition: Option<String>,
    },
    ClearBreakpoint(u16),
    /// Removes every breakpoint at once -- the "Clear breakpoints" toolbar
    /// button, same idea as `ClearBreakpoint` but for the whole set.
    ClearAllBreakpoints,
    /// Adds one watch expression (if not empty and under the configured cap).
    AddWatch(String),
    RemoveWatch(usize),
    ClearWatches,
    /// All bytes land atomically within one `drain_commands` pass -- an
    /// assembled instruction is 1-4 bytes, and poking them one command at a
    /// time would risk the emu thread ticking between two single-byte
    /// pokes, leaving memory briefly holding half an instruction.
    Poke {
        addr: u16,
        bytes: Vec<u8>,
    },
    /// Reply channel carries the real success/failure of the write (unlike
    /// `Poke`, which is fire-and-forget) -- `write_memory_space` delegates
    /// entirely to the machine plugin's own `MachineCommand::WriteMemory`
    /// handler to decide whether the requested bank/space is supported, so
    /// this needs a way to report a genuine rejection back to the caller
    /// rather than silently ack-ing a write that never actually landed.
    PokeBank {
        bank: u8,
        addr: u16,
        bytes: Vec<u8>,
        reply: Sender<Result<(), String>>,
    },
    /// Reuses `Machine::load_state` (the existing save-state mechanism) --
    /// these bytes are exactly what that method already expects, no new
    /// format.
    LoadState(Vec<u8>),
    /// Reuses the hosted machine's snapshot loader -- `extension` is the
    /// lowercase, no-dot extension it already expects (e.g. "z80", "sna", "p", "p81").
    LoadSnapshot {
        extension: String,
        bytes: Vec<u8>,
    },
    /// Changes which memory range `DebuggerState.memory` covers going
    /// forward -- see that field's own doc comment for why this exists.
    SetMemoryWindow {
        addr: u16,
        len: u16,
    },
    /// Scrolls the disassembly panel to start at `addr` instead of
    /// following `pc` -- lets the UI browse code away from where execution
    /// currently is (e.g. to set a breakpoint further down) without losing
    /// track of the real PC, same "independent scroll window" idea as
    /// `SetMemoryWindow`. Cleared by `FollowPc` ("Back to PC" button).
    SetDisassemblyWindow(u16),
    /// Resumes following `pc` for the disassembly panel -- undoes
    /// `SetDisassemblyWindow`.
    FollowPc,
    /// How many lines `publish` disassembles per tick, replacing the fixed
    /// default -- the browser measures how many rows actually fit in the
    /// disassembly panel's current height and sends that, so a taller
    /// window shows more code instead of leaving blank space below a
    /// fixed-size list. Clamped server-side (see `MAX_DISASSEMBLY_LINES`).
    SetDisassemblyLineCount(u16),
    ResetTimingPartial,
    GetTimingCounters(Sender<DebuggerTimingCounters>),
    /// Whatever the machine plugin's generic `chip_id: "overlay"` snapshot
    /// currently carries, verbatim -- no fixed field-name expectations, see
    /// `HostedMachine::overlay_diagnostic_snapshot`'s own doc comment. Not
    /// CPC-specific despite feeding ZRCP's `get-paging-state` command and
    /// `--debug`'s diagnostic log -- any machine's `"overlay"` chip answers
    /// it (mirrors `plugin_api::DebuggerHostCommand::GetOverlaySnapshot`,
    /// renamed from `GetCpcIoSnapshot` alongside this).
    GetOverlaySnapshot(Sender<Option<ChipSnapshot>>),
    /// One-shot reply channel: the plugin's HTTP route handler waits briefly
    /// (bounded by a timeout on its own side) for `Machine::save_state`'s
    /// bytes back. The only genuine request/response pair in this module.
    SaveState(Sender<Vec<u8>>),
    /// Same one-shot-reply shape as `SaveState`: reads `len` raw bytes
    /// starting at `addr` (wrapping past `0xFFFF`) straight off
    /// `Machine::read_memory`, no interpretation. Backs the Bitmap Explorer
    /// tab -- unlike `SetMemoryWindow`'s small, continuously-republished hex
    /// page, this is a one-shot pull of a caller-chosen (and potentially
    /// much larger, e.g. a whole sprite sheet) span, decoded into pixels on
    /// the browser side, not here.
    ReadMemory {
        addr: u16,
        len: u16,
        reply: Sender<Vec<u8>>,
    },
    /// Same shape as `SaveState`, for hosted-machine snapshot saving.
    SaveSnapshot(String, Sender<Result<Vec<u8>, String>>),
    /// Assemble one Z80 instruction and poke bytes into memory. Text is the
    /// instruction mnemonic (e.g. "LD A,5"). Reply channel sends assembled
    /// bytes on success or error message on failure.
    Assemble {
        addr: u16,
        text: String,
        reply: Sender<Result<Vec<u8>, String>>,
    },
    /// `OUT (port),value` on the machine's I/O bus (ZRCP `write-port`).
    /// Replies with an error for a machine that has no I/O-port space.
    WritePort {
        port: u16,
        value: u8,
        reply: Sender<Result<(), String>>,
    },
    /// Adds/replaces (keyed by `port`+`mask`) a port breakpoint, or removes it
    /// when neither direction is set. Replies with an error for a machine
    /// that has no I/O-port space.
    SetPortBreakpoint {
        port: u16,
        mask: u16,
        on_read: bool,
        on_write: bool,
        reply: Sender<Result<(), String>>,
    },
    ClearPortBreakpoints {
        reply: Sender<Result<(), String>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DebuggerTimingCounters {
    pub(crate) total_cycles: u64,
    pub(crate) partial_cycles: u64,
}

pub(crate) type DebuggerCommandSender = Sender<DebuggerCommand>;

/// Run/pause state, mirrored into `DebuggerState.mode` for the HTTP poller.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunMode {
    Running,
    Paused,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct BreakpointDetail {
    pub(crate) addr: u16,
    pub(crate) condition: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct WatchValue {
    pub(crate) expr: String,
    pub(crate) display: String,
    pub(crate) value: Option<u64>,
    pub(crate) is_true: Option<bool>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct InstructionHistoryEntry {
    pub(crate) before_pc: u16,
    pub(crate) after_pc: u16,
    pub(crate) text: String,
}

/// Outbound state the HTTP server thread polls to build its JSON response --
/// exactly the same `Arc<Mutex<T>>`-refreshed-once-per-tick shape as
/// `emu_thread::DebugSnapshotHandle`, just carrying more fields. Refreshed
/// every tick regardless of run state, so a paused/stepped/poked view
/// updates immediately without needing a real step to happen first.
///
/// `memory`/`memory_addr` deliberately never cover the full 64KB address
/// space -- only whatever range `DebuggerCommand::SetMemoryWindow` last
/// requested (a small window, e.g. a hex-grid page) -- so a debugger that
/// isn't scrolled anywhere near where the emulator is stepping doesn't pay
/// for a full-address-space copy every tick.
pub(crate) struct DebuggerState {
    pub(crate) mode: RunMode,
    pub(crate) registers: Vec<(&'static str, u64)>,
    pub(crate) halted: bool,
    pub(crate) pc: u16,
    pub(crate) memory_addr: u16,
    pub(crate) memory: Vec<u8>,
    pub(crate) breakpoints: Vec<u16>,
    pub(crate) breakpoint_details: Vec<BreakpointDetail>,
    pub(crate) last_break: Option<u16>,
    pub(crate) watches: Vec<WatchValue>,
    pub(crate) history: Vec<InstructionHistoryEntry>,
    pub(crate) reverse_depth: usize,
    pub(crate) last_event: Option<String>,
    /// Disassembly of a fixed number of instructions starting at `pc` --
    /// computed here (not in plugin web routes) since only the emu thread has
    /// `Machine::read_memory` access.
    /// (addr, raw bytes, mnemonic text) per line.
    pub(crate) disassembly: Vec<(u16, Vec<u8>, String)>,
    /// Whether the disassembly panel is currently following `pc` (`true`,
    /// the default) or parked at an address the UI scrolled to via
    /// `SetDisassemblyWindow` (`false`) -- lets the UI grey out/show its
    /// own "Back to PC" affordance correctly.
    pub(crate) bitmap_palette: Vec<BitmapPaletteColor>,
    pub(crate) bitmap_defaults: Option<BitmapExplorerDefaults>,
    pub(crate) following_pc: bool,
    pub(crate) port_breakpoints: Vec<PortWatchSpec>,
    pub(crate) last_port_hit: Option<PortHitInfo>,
}

impl DebuggerState {
    fn initial() -> Self {
        Self {
            mode: RunMode::Running,
            registers: Vec::new(),
            halted: false,
            pc: 0,
            memory_addr: 0,
            memory: Vec::new(),
            breakpoints: Vec::new(),
            breakpoint_details: Vec::new(),
            last_break: None,
            watches: Vec::new(),
            history: Vec::new(),
            reverse_depth: 0,
            last_event: None,
            disassembly: Vec::new(),
            following_pc: true,
            bitmap_palette: Vec::new(),
            bitmap_defaults: None,
            port_breakpoints: Vec::new(),
            last_port_hit: None,
        }
    }
}

pub(crate) type DebuggerStateHandle = Arc<Mutex<DebuggerState>>;

/// Default line count before the browser has measured and requested a real
/// value via `SetDisassemblyLineCount` -- enough to fill a typical panel
/// without needless per-tick work before that first measurement lands.
const DEFAULT_DISASSEMBLY_LINES: u16 = 24;

/// Upper bound on `SetDisassemblyLineCount` -- each line costs a few memory
/// reads plus a disassemble call inside the machine plugin, so an
/// unbounded value from a misbehaving/malicious client could turn every
/// tick's publish into unbounded work; no real window needs more than this
/// many disassembly rows at once.
const MAX_DISASSEMBLY_LINES: u16 = 200;

const MAX_WATCHES: usize = 32;
const MAX_WATCH_EXPR_LEN: usize = 128;
const MAX_HISTORY_ENTRIES: usize = 512;
const MAX_PUBLISHED_HISTORY_ENTRIES: usize = 64;
const MAX_REVERSE_STATES: usize = MAX_HISTORY_ENTRIES;
const STEP_OVER_MAX_INSTRUCTIONS: usize = 200_000;

/// Everything `emu_thread::tick` needs to support the debugger, bundled as
/// one `Option` (in `emu_thread.rs`) so the entire feature collapses to a
/// single `None` branch when no debugger plugin was requested -- no separate
/// "is enabled" flag to keep in sync with whether the channel/state exist.
pub(crate) struct DebuggerLink {
    commands: Receiver<DebuggerCommand>,
    state: DebuggerStateHandle,
    // Emu-thread-local, never shared/locked -- only this thread ever reads
    // or writes these, unlike `state` above.
    mode: RunMode,
    breakpoints: BTreeMap<u16, Option<String>>,
    memory_window: (u16, u16),
    pending_step: bool,
    pending_step_over: bool,
    pending_backstep: bool,
    /// `None` = disassembly follows `pc` every tick (the default). `Some`
    /// = parked at that address by `SetDisassemblyWindow`, until `FollowPc`
    /// clears it back to `None`.
    disassembly_addr: Option<u16>,
    /// How many lines `publish` disassembles -- see `SetDisassemblyLineCount`.
    disassembly_lines: u16,
    /// Reverse-debug ring buffer of machine save states captured before each
    /// debugger-driven instruction step.
    reverse_states: VecDeque<Vec<u8>>,
    /// Human-readable executed instruction log.
    history: VecDeque<InstructionHistoryEntry>,
    watches: Vec<String>,
    /// Port (`IN`/`OUT`) breakpoints, mirrored into the machine's own watch
    /// list whenever they change.
    port_breakpoints: Vec<PortWatchSpec>,
    /// A port breakpoint hit taken during a debugger-driven step, waiting to
    /// be reported.
    port_hit: Option<PortHitInfo>,
    /// The hit that is currently the reason for being stopped (published).
    last_port_hit: Option<PortHitInfo>,
    last_break: Option<u16>,
    last_event: Option<String>,
    total_cycles: u64,
    partial_cycles: u64,
}
impl DebuggerLink {
    pub(crate) fn new(commands: Receiver<DebuggerCommand>, state: DebuggerStateHandle) -> Self {
        Self {
            commands,
            state,
            mode: RunMode::Running,
            breakpoints: BTreeMap::new(),
            memory_window: (0, 0),
            pending_step: false,
            pending_step_over: false,
            pending_backstep: false,
            disassembly_addr: None,
            disassembly_lines: DEFAULT_DISASSEMBLY_LINES,
            reverse_states: VecDeque::new(),
            history: VecDeque::new(),
            watches: Vec::new(),
            port_breakpoints: Vec::new(),
            port_hit: None,
            last_port_hit: None,
            last_break: None,
            last_event: None,
            total_cycles: 0,
            partial_cycles: 0,
        }
    }

    pub(crate) fn new_handle() -> DebuggerStateHandle {
        Arc::new(Mutex::new(DebuggerState::initial()))
    }

    pub(crate) fn is_paused(&self) -> bool {
        self.mode == RunMode::Paused
    }

    pub(crate) fn record_cycles(&mut self, cycles: u64) {
        self.total_cycles = self.total_cycles.saturating_add(cycles);
        self.partial_cycles = self.partial_cycles.saturating_add(cycles);
    }

    pub(crate) fn take_pending_step(&mut self) -> bool {
        let was_pending = self.pending_step;
        self.pending_step = false;
        was_pending
    }

    pub(crate) fn take_pending_step_over(&mut self) -> bool {
        let was_pending = self.pending_step_over;
        self.pending_step_over = false;
        was_pending
    }

    pub(crate) fn take_pending_backstep(&mut self) -> bool {
        let was_pending = self.pending_backstep;
        self.pending_backstep = false;
        was_pending
    }

    /// Drains every pending command -- called once at the top of `tick`,
    /// mirroring the existing `input_rx.try_recv()` drain loop exactly.
    pub(crate) fn drain_commands(&mut self, machine: &mut HostedMachine) {
        while let Ok(cmd) = self.commands.try_recv() {
            match cmd {
                DebuggerCommand::Pause => {
                    self.mode = RunMode::Paused;
                    self.last_event = Some("paused".to_string());
                }
                DebuggerCommand::Resume => {
                    self.mode = RunMode::Running;
                    self.last_break = None;
                    self.last_port_hit = None;
                    self.last_event = Some("running".to_string());
                }
                DebuggerCommand::ResetMachine => {
                    machine.reset();
                    self.reverse_states.clear();
                    self.history.clear();
                    self.last_break = None;
                    self.last_event = Some("reset".to_string());
                }
                DebuggerCommand::KeyEvent { key, pressed } => match parse_debugger_keycode(&key) {
                    Some(code) => machine.key_event(code, pressed),
                    None => {
                        self.last_event =
                            Some(format!("unknown key for debugger key event: {key}"));
                    }
                },
                DebuggerCommand::StepInstruction => {
                    if self.mode == RunMode::Paused {
                        self.pending_step = true;
                    }
                }
                DebuggerCommand::StepOverInstruction => {
                    if self.mode == RunMode::Paused {
                        self.pending_step_over = true;
                    }
                }
                DebuggerCommand::BackstepInstruction => {
                    if self.mode == RunMode::Paused {
                        self.pending_backstep = true;
                    }
                }
                DebuggerCommand::SetBreakpoint { addr } => {
                    self.breakpoints.insert(addr, None);
                }
                DebuggerCommand::SetBreakpointCondition { addr, condition } => {
                    self.breakpoints
                        .insert(addr, normalize_optional_text(condition));
                }
                DebuggerCommand::ClearBreakpoint(addr) => {
                    self.breakpoints.remove(&addr);
                }
                DebuggerCommand::ClearAllBreakpoints => {
                    self.breakpoints.clear();
                }
                DebuggerCommand::AddWatch(expr) => {
                    if self.watches.len() >= MAX_WATCHES {
                        self.last_event = Some(format!("watch limit reached ({MAX_WATCHES})"));
                        continue;
                    }
                    if let Some(expr) = normalize_watch_expression(expr) {
                        self.watches.push(expr);
                    }
                }
                DebuggerCommand::RemoveWatch(index) => {
                    if index < self.watches.len() {
                        self.watches.remove(index);
                    }
                }
                DebuggerCommand::ClearWatches => {
                    self.watches.clear();
                }
                DebuggerCommand::SetPc(addr) => {
                    machine.set_pc(addr);
                    self.last_break = None;
                }
                DebuggerCommand::SetRegister { name, value } => {
                    if let Err(err) = machine.set_register(&name, value) {
                        self.last_event = Some(format!("set-register failed for {name}: {err}"));
                    } else {
                        self.last_break = None;
                    }
                }
                DebuggerCommand::Poke { addr, bytes } => {
                    machine.load(addr, &bytes);
                }
                DebuggerCommand::PokeBank {
                    bank,
                    addr,
                    bytes,
                    reply,
                } => {
                    let result =
                        machine.write_memory_space(addr, &bytes, Some(format!("ram-bank:{bank}")));
                    if let Err(ref error) = result {
                        self.last_event = Some(format!("banked poke failed: {error}"));
                    }
                    let _ = reply.send(result);
                }
                DebuggerCommand::LoadState(bytes) => match machine.load_state(&bytes) {
                    Ok(()) => {
                        self.reverse_states.clear();
                        self.history.clear();
                        self.last_break = None;
                    }
                    Err(e) => {
                        self.last_event = Some(format!("load-state failed: {e}"));
                    }
                },
                DebuggerCommand::LoadSnapshot { extension, bytes } => {
                    match machine.apply_snapshot(&extension, &bytes) {
                        Ok(()) => {
                            self.reverse_states.clear();
                            self.history.clear();
                            self.last_break = None;
                        }
                        Err(e) => {
                            self.last_event = Some(format!("load-snapshot failed: {e}"));
                        }
                    }
                }
                DebuggerCommand::SetMemoryWindow { addr, len } => {
                    self.memory_window = (addr, len);
                }
                DebuggerCommand::SetDisassemblyWindow(addr) => {
                    self.disassembly_addr = Some(addr);
                }
                DebuggerCommand::FollowPc => {
                    self.disassembly_addr = None;
                }
                DebuggerCommand::SetDisassemblyLineCount(count) => {
                    self.disassembly_lines = count.clamp(1, MAX_DISASSEMBLY_LINES);
                }
                DebuggerCommand::ResetTimingPartial => {
                    self.partial_cycles = 0;
                }
                DebuggerCommand::GetTimingCounters(reply) => {
                    let _ = reply.send(DebuggerTimingCounters {
                        total_cycles: self.total_cycles,
                        partial_cycles: self.partial_cycles,
                    });
                }
                DebuggerCommand::GetOverlaySnapshot(reply) => {
                    let _ = reply.send(machine.overlay_diagnostic_snapshot());
                }
                DebuggerCommand::SaveState(reply) => {
                    let _ = reply.send(machine.save_state());
                }
                DebuggerCommand::ReadMemory { addr, len, reply } => {
                    let bytes = (0..len)
                        .map(|i| machine.read_memory(addr.wrapping_add(i)))
                        .collect();
                    let _ = reply.send(bytes);
                }
                DebuggerCommand::SaveSnapshot(extension, reply) => {
                    let _ = reply.send(machine.save_snapshot(&extension));
                }
                DebuggerCommand::WritePort { port, value, reply } => {
                    let _ = reply.send(machine.write_port(port, value));
                }
                DebuggerCommand::SetPortBreakpoint {
                    port,
                    mask,
                    on_read,
                    on_write,
                    reply,
                } => {
                    let mut updated = self.port_breakpoints.clone();
                    updated.retain(|w| !(w.port == port && w.mask == mask));
                    if on_read || on_write {
                        updated.push(PortWatchSpec {
                            port,
                            mask,
                            on_read,
                            on_write,
                        });
                    }
                    let _ = reply.send(self.apply_port_breakpoints(machine, updated));
                }
                DebuggerCommand::ClearPortBreakpoints { reply } => {
                    let _ = reply.send(self.apply_port_breakpoints(machine, Vec::new()));
                }
                DebuggerCommand::Assemble { addr, text, reply } => {
                    match assemble_with_symbols(
                        &machine.cpu_family(),
                        machine.machine_id(),
                        &text,
                        addr,
                    ) {
                        Ok(bytes) => {
                            machine.load(addr, &bytes);
                            let _ = reply.send(Ok(bytes));
                        }
                        Err(e) => {
                            let _ = reply.send(Err(e));
                        }
                    }
                }
            }
        }
    }

    /// Handles debugger-controlled single-step execution from paused mode.
    pub(crate) fn execute_step_instruction(&mut self, machine: &mut HostedMachine) -> u64 {
        if self.mode != RunMode::Paused {
            return 0;
        }
        let outcome = self.step_once(machine);
        self.last_break = None;
        self.report_port_hit();
        outcome.cycles
    }

    /// Handles debugger-controlled step-over execution from paused mode.
    pub(crate) fn execute_step_over_instruction(&mut self, machine: &mut HostedMachine) -> u64 {
        if self.mode != RunMode::Paused {
            return 0;
        }
        let mut total_cycles: u64 = 0;
        let outcome = self.step_once(machine);
        total_cycles = total_cycles.saturating_add(outcome.cycles);
        if self.report_port_hit() {
            return total_cycles;
        }
        let fallthrough_pc = outcome.before_pc.wrapping_add(outcome.consumed as u16);
        if !is_step_over_candidate(&outcome.text) || outcome.after_pc == fallthrough_pc {
            self.last_break = None;
            return total_cycles;
        }

        for _ in 0..STEP_OVER_MAX_INSTRUCTIONS {
            if machine.pc() == fallthrough_pc {
                self.last_break = None;
                return total_cycles;
            }
            if let Some(hit) = self.breakpoint_hit_at_pc(machine) {
                self.last_break = Some(hit);
                return total_cycles;
            }
            let outcome = self.step_once(machine);
            total_cycles = total_cycles.saturating_add(outcome.cycles);
            if self.report_port_hit() {
                return total_cycles;
            }
            if machine.pc() == fallthrough_pc {
                self.last_break = None;
                return total_cycles;
            }
        }

        self.last_event = Some(format!(
            "step-over limit ({STEP_OVER_MAX_INSTRUCTIONS}) reached before 0x{fallthrough_pc:04X}"
        ));
        total_cycles
    }

    /// Restores the most recently captured pre-step state.
    pub(crate) fn execute_backstep_instruction(&mut self, machine: &mut HostedMachine) -> u64 {
        if self.mode != RunMode::Paused {
            return 0;
        }
        let Some(state) = self.reverse_states.pop_back() else {
            self.last_event = Some("no reverse history available".to_string());
            return 0;
        };
        match machine.load_state(&state) {
            Ok(()) => {
                let _ = self.history.pop_back();
                self.last_break = None;
            }
            Err(e) => {
                self.last_event = Some(format!("backstep failed: {e}"));
            }
        }
        0
    }

    /// Running-mode post-instruction breakpoint check.
    pub(crate) fn on_post_step_running(&mut self, machine: &HostedMachine) -> bool {
        if self.mode != RunMode::Running {
            return false;
        }
        if !self.port_breakpoints.is_empty() {
            self.port_hit = machine.take_port_hit();
            if self.report_port_hit() {
                self.mode = RunMode::Paused;
                return true;
            }
        }
        if let Some(hit) = self.breakpoint_hit_at_pc(machine) {
            self.mode = RunMode::Paused;
            self.last_break = Some(hit);
            return true;
        }
        false
    }

    /// Refreshes the shared `DebuggerState` from `machine`'s current
    /// values -- called every tick regardless of run mode. The disassembly
    /// comes from the machine plugin (`HostedMachine::disassemble`), so any
    /// CPU a plugin can decode shows up correctly.
    pub(crate) fn publish(&mut self, machine: &HostedMachine) {
        let (addr, len) = self.memory_window;
        let memory = (0..len)
            .map(|i| machine.read_memory(addr.wrapping_add(i)))
            .collect();

        let start = self.disassembly_addr.unwrap_or_else(|| machine.pc());
        let disassembly = disassembly_window(machine, start, self.disassembly_lines);

        let registers = machine.registers();
        let watches = self
            .watches
            .iter()
            .map(|expr| evaluate_watch(machine, &registers, expr))
            .collect();

        let breakpoints: Vec<u16> = self.breakpoints.keys().copied().collect();
        let breakpoint_details = self
            .breakpoints
            .iter()
            .map(|(addr, condition)| BreakpointDetail {
                addr: *addr,
                condition: condition.clone(),
            })
            .collect();
        let history_skip = self
            .history
            .len()
            .saturating_sub(MAX_PUBLISHED_HISTORY_ENTRIES);
        let history = self.history.iter().skip(history_skip).cloned().collect();

        *self.state.lock().unwrap() = DebuggerState {
            mode: self.mode,
            registers,
            halted: machine.halted(),
            pc: machine.pc(),
            memory_addr: addr,
            memory,
            breakpoints,
            breakpoint_details,
            last_break: self.last_break,
            watches,
            history,
            reverse_depth: self.reverse_states.len(),
            last_event: self.last_event.clone(),
            disassembly,
            following_pc: self.disassembly_addr.is_none(),
            bitmap_palette: machine.metadata().bitmap_palette.clone(),
            bitmap_defaults: machine.metadata().bitmap_defaults,
            port_breakpoints: self.port_breakpoints.clone(),
            last_port_hit: self.last_port_hit,
        };
    }

    /// Installs `updated` as the port breakpoint list, in the machine first so
    /// a machine with no port space (the C64) refuses it and the host's own
    /// list stays truthful.
    fn apply_port_breakpoints(
        &mut self,
        machine: &HostedMachine,
        updated: Vec<PortWatchSpec>,
    ) -> Result<(), String> {
        // Nothing set and nothing to set: succeed without involving the
        // machine, so clearing on a machine with no port space isn't an error.
        if updated.is_empty() && self.port_breakpoints.is_empty() {
            return Ok(());
        }
        machine.set_port_watches(&updated)?;
        self.port_breakpoints = updated;
        self.port_hit = None;
        Ok(())
    }

    /// Turns a pending port hit into the visible "why we stopped" event.
    /// Returns whether there was one.
    fn report_port_hit(&mut self) -> bool {
        let Some(hit) = self.port_hit.take() else {
            return false;
        };
        self.last_event = Some(format_port_hit(&hit));
        self.last_break = None;
        self.last_port_hit = Some(hit);
        true
    }

    fn step_once(&mut self, machine: &mut HostedMachine) -> StepOutcome {
        self.last_port_hit = None;
        self.push_reverse_state(machine);
        let before_pc = machine.pc();
        let (text, consumed) = disassemble_one(machine, before_pc);
        let cycles = machine.step() as u64;
        self.record_cycles(cycles);
        if !self.port_breakpoints.is_empty() {
            self.port_hit = machine.take_port_hit();
        }
        let after_pc = machine.pc();
        self.push_history_entry(InstructionHistoryEntry {
            before_pc,
            after_pc,
            text: text.clone(),
        });
        StepOutcome {
            before_pc,
            after_pc,
            consumed,
            cycles,
            text,
        }
    }

    fn push_reverse_state(&mut self, machine: &HostedMachine) {
        if self.reverse_states.len() == MAX_REVERSE_STATES {
            let _ = self.reverse_states.pop_front();
        }
        self.reverse_states.push_back(machine.save_state());
    }

    fn push_history_entry(&mut self, entry: InstructionHistoryEntry) {
        if self.history.len() == MAX_HISTORY_ENTRIES {
            let _ = self.history.pop_front();
        }
        self.history.push_back(entry);
    }

    fn breakpoint_hit_at_pc(&mut self, machine: &HostedMachine) -> Option<u16> {
        let pc = machine.pc();
        let condition = self.breakpoints.get(&pc).cloned()?;
        let Some(condition) = condition else {
            return Some(pc);
        };
        let registers = machine.registers();
        match evaluate_condition(machine, &registers, &condition) {
            Ok(true) => Some(pc),
            Ok(false) => None,
            Err(e) => {
                self.last_event = Some(format!("invalid breakpoint condition at 0x{pc:04X}: {e}"));
                None
            }
        }
    }
}

/// `OUT (0x7F00),0x8D` / `IN (0xF400) -> 0xFF` -- how a port breakpoint hit is
/// reported to the debugger UIs.
pub(crate) fn format_port_hit(hit: &PortHitInfo) -> String {
    if hit.write {
        format!(
            "port breakpoint: OUT (0x{:04X}),0x{:02X}",
            hit.port, hit.value
        )
    } else {
        format!(
            "port breakpoint: IN (0x{:04X}) -> 0x{:02X}",
            hit.port, hit.value
        )
    }
}

struct StepOutcome {
    before_pc: u16,
    after_pc: u16,
    consumed: usize,
    cycles: u64,
    text: String,
}

/// `lines` disassembled instructions from `start`, decoded by the machine plugin (it knows which CPU
/// is running). If the plugin cannot answer, each byte shows as a `DB` line instead of leaving the
/// view empty.
fn disassembly_window(
    machine: &HostedMachine,
    start: u16,
    lines: u16,
) -> Vec<(u16, Vec<u8>, String)> {
    let family = machine.cpu_family();
    let symbols = crate::debug_symbols::table_for(machine.machine_id());
    match machine.disassemble(Some(start), lines) {
        Ok(decoded) if !decoded.is_empty() => decoded
            .into_iter()
            .map(|line| {
                let text = present_text(
                    &family,
                    line.address.try_into().unwrap(),
                    &line.bytes,
                    line.text,
                );
                let text = symbols.annotate(&text);
                (line.address as u16, line.bytes, text)
            })
            .collect(),
        _ => {
            let mut cursor = start;
            (0..lines)
                .map(|_| {
                    let byte = machine.read_memory(cursor);
                    let line = (cursor, vec![byte], format!("DB ${byte:02X}"));
                    cursor = cursor.wrapping_add(1);
                    line
                })
                .collect()
        }
    }
}

/// The plugin's text, with a Z80 `JR`/`DJNZ` shown as its absolute target (the 6502 disassembler
/// already does this for branches) so every CPU reads alike and a target can be named by a symbol.
fn present_text(family: &CpuFamily, address: u16, bytes: &[u8], text: String) -> String {
    match family {
        CpuFamily::Z80 if bytes.len() == 2 => z80::disassemble::disassemble_at(bytes, address).0,
        _ => text,
    }
}

/// The text and length of the instruction at `addr`.
fn disassemble_one(machine: &HostedMachine, addr: u16) -> (String, usize) {
    match disassembly_window(machine, addr, 1).into_iter().next() {
        Some((_, bytes, text)) => (text, bytes.len().max(1)),
        None => (String::from("DB $00"), 1),
    }
}

/// Assembles one instruction for the CPU that is running. The assemblers live with their CPU cores;
/// the host only picks one by the machine's reported CPU family.
/// `assemble_for` after names from the machine's symbol tables (built-ins and `--symbols` files) are replaced by their addresses:
/// `JSR CHROUT` on a C64, `CALL LD_BYTES` on a Spectrum.
fn assemble_with_symbols(
    family: &CpuFamily,
    machine_id: &str,
    text: &str,
    addr: u16,
) -> Result<Vec<u8>, String> {
    let prefix = if *family == CpuFamily::Z80 { "0x" } else { "$" };
    let text = crate::debug_symbols::table_for(machine_id).substitute(text, prefix);
    assemble_for(family, &text, addr)
}

fn assemble_for(family: &CpuFamily, text: &str, addr: u16) -> Result<Vec<u8>, String> {
    match family {
        CpuFamily::Z80 => z80::disassemble::assemble_one_at(text, addr),
        CpuFamily::Mos6502 => mos6502::disassemble::assemble_one(text, addr),
        CpuFamily::M68k => Err("no assembler for the 68000 CPU yet".to_string()),
        CpuFamily::Other(name) => Err(format!("no assembler for the {name} CPU")),
    }
}

fn is_step_over_candidate(text: &str) -> bool {
    let upper = text.trim_start().to_ascii_uppercase();
    // Z80 CALL/RST, 6502 JSR
    upper.starts_with("CALL") || upper.starts_with("RST") || upper.starts_with("JSR")
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_watch_expression(expression: String) -> Option<String> {
    let trimmed = expression.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut normalized = trimmed.to_string();
    if normalized.len() > MAX_WATCH_EXPR_LEN {
        normalized.truncate(MAX_WATCH_EXPR_LEN);
    }
    Some(normalized)
}

fn evaluate_watch(
    machine: &HostedMachine,
    registers: &[(&'static str, u64)],
    expr: &str,
) -> WatchValue {
    match split_condition(expr) {
        Some((lhs, op, rhs)) => match (
            evaluate_numeric(machine, registers, lhs),
            evaluate_numeric(machine, registers, rhs),
        ) {
            (Ok(a), Ok(b)) => {
                let is_true = apply_comparison(a, b, op);
                WatchValue {
                    expr: expr.to_string(),
                    display: format!("{is_true} ({a:#X} {op} {b:#X})"),
                    value: None,
                    is_true: Some(is_true),
                    error: None,
                }
            }
            (Err(e), _) | (_, Err(e)) => WatchValue {
                expr: expr.to_string(),
                display: format!("error: {e}"),
                value: None,
                is_true: None,
                error: Some(e),
            },
        },
        None => match evaluate_numeric(machine, registers, expr) {
            Ok(value) => WatchValue {
                expr: expr.to_string(),
                display: format!("{value:#X} ({value})"),
                value: Some(value),
                is_true: None,
                error: None,
            },
            Err(e) => WatchValue {
                expr: expr.to_string(),
                display: format!("error: {e}"),
                value: None,
                is_true: None,
                error: Some(e),
            },
        },
    }
}

fn evaluate_condition(
    machine: &HostedMachine,
    registers: &[(&'static str, u64)],
    expr: &str,
) -> Result<bool, String> {
    match split_condition(expr) {
        Some((lhs, op, rhs)) => {
            let a = evaluate_numeric(machine, registers, lhs)?;
            let b = evaluate_numeric(machine, registers, rhs)?;
            Ok(apply_comparison(a, b, op))
        }
        None => Ok(evaluate_numeric(machine, registers, expr)? != 0),
    }
}

fn evaluate_numeric(
    machine: &HostedMachine,
    registers: &[(&'static str, u64)],
    expr: &str,
) -> Result<u64, String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err("empty expression".to_string());
    }
    if let Some(inner) = expr.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let addr = evaluate_numeric(machine, registers, inner)? as u16;
        return Ok(machine.read_memory(addr) as u64);
    }
    if let Some(value) = parse_number(expr) {
        return Ok(value);
    }
    if expr.eq_ignore_ascii_case("pc") {
        return Ok(machine.pc() as u64);
    }
    if let Some((_, value)) = registers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(expr))
    {
        return Ok(*value);
    }
    Err(format!("unknown symbol `{expr}`"))
}

fn parse_number(token: &str) -> Option<u64> {
    if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        return u64::from_str_radix(hex, 16).ok();
    }
    if let Some(hex) = token.strip_prefix('$') {
        return u64::from_str_radix(hex, 16).ok();
    }
    token.parse::<u64>().ok()
}

fn split_condition(expr: &str) -> Option<(&str, &'static str, &str)> {
    const OPS: [&str; 6] = ["==", "!=", ">=", "<=", ">", "<"];
    OPS.iter().find_map(|op| {
        expr.find(op).map(|idx| {
            let lhs = &expr[..idx];
            let rhs = &expr[idx + op.len()..];
            (lhs, *op, rhs)
        })
    })
}

fn apply_comparison(lhs: u64, rhs: u64, op: &str) -> bool {
    match op {
        "==" => lhs == rhs,
        "!=" => lhs != rhs,
        ">=" => lhs >= rhs,
        "<=" => lhs <= rhs,
        ">" => lhs > rhs,
        "<" => lhs < rhs,
        _ => false,
    }
}

fn parse_debugger_keycode(name: &str) -> Option<KeyCode> {
    let normalized = name.trim().to_ascii_uppercase();
    let key = normalized.strip_prefix("KEY").unwrap_or(&normalized);
    let digit = normalized.strip_prefix("DIGIT");
    if let Some(d) = digit {
        return match d {
            "0" => Some(KeyCode::Digit0),
            "1" => Some(KeyCode::Digit1),
            "2" => Some(KeyCode::Digit2),
            "3" => Some(KeyCode::Digit3),
            "4" => Some(KeyCode::Digit4),
            "5" => Some(KeyCode::Digit5),
            "6" => Some(KeyCode::Digit6),
            "7" => Some(KeyCode::Digit7),
            "8" => Some(KeyCode::Digit8),
            "9" => Some(KeyCode::Digit9),
            _ => None,
        };
    }
    match key {
        "A" => Some(KeyCode::KeyA),
        "B" => Some(KeyCode::KeyB),
        "C" => Some(KeyCode::KeyC),
        "D" => Some(KeyCode::KeyD),
        "E" => Some(KeyCode::KeyE),
        "F" => Some(KeyCode::KeyF),
        "G" => Some(KeyCode::KeyG),
        "H" => Some(KeyCode::KeyH),
        "I" => Some(KeyCode::KeyI),
        "J" => Some(KeyCode::KeyJ),
        "K" => Some(KeyCode::KeyK),
        "L" => Some(KeyCode::KeyL),
        "M" => Some(KeyCode::KeyM),
        "N" => Some(KeyCode::KeyN),
        "O" => Some(KeyCode::KeyO),
        "P" => Some(KeyCode::KeyP),
        "Q" => Some(KeyCode::KeyQ),
        "R" => Some(KeyCode::KeyR),
        "S" => Some(KeyCode::KeyS),
        "T" => Some(KeyCode::KeyT),
        "U" => Some(KeyCode::KeyU),
        "V" => Some(KeyCode::KeyV),
        "W" => Some(KeyCode::KeyW),
        "X" => Some(KeyCode::KeyX),
        "Y" => Some(KeyCode::KeyY),
        "Z" => Some(KeyCode::KeyZ),
        "SPACE" => Some(KeyCode::Space),
        "ENTER" | "RETURN" => Some(KeyCode::Enter),
        "TAB" => Some(KeyCode::Tab),
        "BACKSPACE" => Some(KeyCode::Backspace),
        "DELETE" | "DEL" => Some(KeyCode::Delete),
        "HOME" => Some(KeyCode::Home),
        "END" => Some(KeyCode::End),
        "ESC" | "ESCAPE" => Some(KeyCode::Escape),
        "PAGEUP" => Some(KeyCode::PageUp),
        "PAGEDOWN" => Some(KeyCode::PageDown),
        "SHIFT" | "SHIFTLEFT" => Some(KeyCode::ShiftLeft),
        "SHIFTRIGHT" => Some(KeyCode::ShiftRight),
        "CONTROL" | "CTRL" | "CONTROLLEFT" | "CTRLLEFT" => Some(KeyCode::ControlLeft),
        "CONTROLRIGHT" | "CTRLRIGHT" => Some(KeyCode::ControlRight),
        "ALT" | "ALTLEFT" => Some(KeyCode::AltLeft),
        "ALTRIGHT" => Some(KeyCode::AltRight),
        "METALEFT" | "WINKEYLEFT" => Some(KeyCode::SuperLeft),
        "METARIGHT" | "WINKEYRIGHT" => Some(KeyCode::SuperRight),
        "CAPSLOCK" => Some(KeyCode::CapsLock),
        "COMMA" => Some(KeyCode::Comma),
        "PERIOD" | "DOT" => Some(KeyCode::Period),
        "SEMICOLON" => Some(KeyCode::Semicolon),
        "QUOTE" | "APOSTROPHE" => Some(KeyCode::Quote),
        "MINUS" => Some(KeyCode::Minus),
        "EQUAL" | "EQUALS" => Some(KeyCode::Equal),
        "SLASH" => Some(KeyCode::Slash),
        "BACKSLASH" => Some(KeyCode::Backslash),
        "BRACKETLEFT" => Some(KeyCode::BracketLeft),
        "BRACKETRIGHT" => Some(KeyCode::BracketRight),
        "BACKQUOTE" | "GRAVE" => Some(KeyCode::Backquote),
        "F1" => Some(KeyCode::F1),
        "F2" => Some(KeyCode::F2),
        "F3" => Some(KeyCode::F3),
        "F4" => Some(KeyCode::F4),
        "F5" => Some(KeyCode::F5),
        "F6" => Some(KeyCode::F6),
        "F7" => Some(KeyCode::F7),
        "F8" => Some(KeyCode::F8),
        "F9" => Some(KeyCode::F9),
        "F10" => Some(KeyCode::F10),
        "F11" => Some(KeyCode::F11),
        "F12" => Some(KeyCode::F12),
        "ARROWUP" | "UP" => Some(KeyCode::ArrowUp),
        "ARROWDOWN" | "DOWN" => Some(KeyCode::ArrowDown),
        "ARROWLEFT" | "LEFT" => Some(KeyCode::ArrowLeft),
        "ARROWRIGHT" | "RIGHT" => Some(KeyCode::ArrowRight),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        assemble_for, assemble_with_symbols, disassemble_one, disassembly_window,
        is_step_over_candidate, parse_debugger_keycode, present_text,
    };
    use bevy::input::keyboard::KeyCode;
    use plugin_api::{CpuFamily, MachineCreateRequest};

    fn stub_machine() -> crate::plugin_bridge::HostedMachine {
        crate::plugin_bridge::create_hosted_machine(MachineCreateRequest {
            machine_id: "stub-machine".to_string(),
            region: Some("pal".to_string()),
            roms: Vec::new(),
            options: Vec::new(),
        })
        .expect("the bundled stub machine")
    }

    /// A Z80 relative jump is shown (and assembled) as its absolute target; other text is untouched.
    #[test]
    fn z80_relative_jumps_show_their_absolute_target() {
        let z80 = CpuFamily::Z80;
        assert_eq!(
            present_text(&z80, 0x8000, &[0x20, 0xFE], "JR NZ,0xFE".into()),
            "JR NZ,0x8000"
        );
        assert_eq!(
            present_text(&z80, 0x8000, &[0x3E, 0x05], "LD A,0x05".into()),
            "LD A,0x05"
        );
        assert_eq!(
            present_text(
                &CpuFamily::Mos6502,
                0xC000,
                &[0xD0, 0xFE],
                "BNE $C000".into()
            ),
            "BNE $C000"
        );
        assert_eq!(
            assemble_for(&z80, "JR NZ,0x8000", 0x8000).unwrap(),
            [0x20, 0xFE]
        );
    }

    /// The view shows what the machine plugin decodes, not a host-side `DB` placeholder.
    #[test]
    fn the_disassembly_window_comes_from_the_machine_plugin() {
        let machine = stub_machine();
        let window = disassembly_window(&machine, 0x4000, 3);
        let shown: Vec<(u16, &str)> = window.iter().map(|(a, _, t)| (*a, t.as_str())).collect();
        // The stub machine decodes every byte as a one-byte NOP.
        assert_eq!(shown, [(0x4000, "NOP"), (0x4001, "NOP"), (0x4002, "NOP")]);
        assert_eq!(disassemble_one(&machine, 0x4000), ("NOP".to_string(), 1));
    }

    #[test]
    fn the_assembler_follows_the_cpu_family() {
        assert_eq!(
            assemble_for(&CpuFamily::Z80, "LD A,5", 0).unwrap(),
            vec![0x3E, 0x05]
        );
        assert_eq!(
            assemble_for(&CpuFamily::Mos6502, "LDA #$05", 0xC000).unwrap(),
            vec![0xA9, 0x05]
        );
        // A 6502 branch needs the address it is assembled at.
        assert_eq!(
            assemble_for(&CpuFamily::Mos6502, "BNE $C000", 0xC000).unwrap(),
            vec![0xD0, 0xFE]
        );
        assert!(assemble_for(&CpuFamily::Other("68000".into()), "NOP", 0).is_err());
    }

    #[test]
    fn symbol_names_assemble_as_their_addresses() {
        // The built-in tables: the C64 KERNAL's CHROUT is $FFD2, the Spectrum ROM's LD_BYTES is $0556.
        assert_eq!(
            assemble_with_symbols(&CpuFamily::Mos6502, "c64", "JSR CHROUT", 0xC000).unwrap(),
            assemble_for(&CpuFamily::Mos6502, "JSR $FFD2", 0xC000).unwrap()
        );
        assert_eq!(
            assemble_with_symbols(&CpuFamily::Z80, "zx-spectrum", "CALL LD_BYTES", 0x8000).unwrap(),
            assemble_for(&CpuFamily::Z80, "CALL 0x0556", 0x8000).unwrap()
        );
        // Constant offsets and the 6502's low/high byte selectors fold into the operand.
        assert_eq!(
            assemble_with_symbols(&CpuFamily::Mos6502, "c64", "JMP CHROUT+1", 0xC000).unwrap(),
            assemble_for(&CpuFamily::Mos6502, "JMP $FFD3", 0xC000).unwrap()
        );
        assert_eq!(
            assemble_with_symbols(&CpuFamily::Mos6502, "c64", "LDA #>CHROUT", 0xC000).unwrap(),
            assemble_for(&CpuFamily::Mos6502, "LDA #$FF", 0xC000).unwrap()
        );
        assert_eq!(
            assemble_with_symbols(&CpuFamily::Z80, "zx-spectrum", "CALL LD_BYTES+3", 0x8000)
                .unwrap(),
            assemble_for(&CpuFamily::Z80, "CALL 0x0559", 0x8000).unwrap()
        );
        // A name the machine does not have is still the assembler's error.
        assert!(assemble_with_symbols(&CpuFamily::Mos6502, "c64", "JSR NOWHERE", 0).is_err());
        assert_eq!(stub_machine().cpu_family(), CpuFamily::Z80);
    }

    #[test]
    fn step_over_applies_to_calls_on_both_cpus() {
        for text in [
            "CALL 0x1234",
            "call nz,0x10",
            "RST 0x38",
            "JSR $FFD2",
            "jsr $c000",
        ] {
            assert!(is_step_over_candidate(text), "{text}");
        }
        for text in ["JP 0x1234", "JMP $C000", "RTS", "LDA #$01", "NOP"] {
            assert!(!is_step_over_candidate(text), "{text}");
        }
    }

    #[test]
    fn parse_debugger_keycode_accepts_supported_spellings() {
        assert_eq!(parse_debugger_keycode("KeyA"), Some(KeyCode::KeyA));
        assert_eq!(parse_debugger_keycode("A"), Some(KeyCode::KeyA));
        assert_eq!(parse_debugger_keycode("Digit3"), Some(KeyCode::Digit3));
        assert_eq!(parse_debugger_keycode("space"), Some(KeyCode::Space));
        assert_eq!(parse_debugger_keycode("Return"), Some(KeyCode::Enter));
        assert_eq!(
            parse_debugger_keycode("ControlLeft"),
            Some(KeyCode::ControlLeft)
        );
        assert_eq!(parse_debugger_keycode("F11"), Some(KeyCode::F11));
        assert_eq!(
            parse_debugger_keycode("Backslash"),
            Some(KeyCode::Backslash)
        );
        assert_eq!(parse_debugger_keycode("left"), Some(KeyCode::ArrowLeft));
        assert_eq!(parse_debugger_keycode("ArrowUp"), Some(KeyCode::ArrowUp));
    }

    #[test]
    fn parse_debugger_keycode_rejects_unknown_keys() {
        assert_eq!(parse_debugger_keycode("MouseLeft"), None);
        assert_eq!(parse_debugger_keycode("GamepadStart"), None);
    }
}
