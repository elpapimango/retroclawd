//! The vocabulary the debugger modules share with the application: the
//! commands they may issue, the state snapshot they read and the descriptions
//! `--list-plugins` prints. Plain Rust types, passed in-process; nothing here
//! crosses a library boundary.

use plugin_api::{
    BitmapExplorerDefaults, BitmapPaletteColor, DisassemblyLine, PortHitInfo, PortWatchSpec,
    RegisterField,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebuggerRunMode {
    Running,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebuggerBreakpoint {
    pub address: u16,
    pub condition: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebuggerWatch {
    pub expr: String,
    pub display: String,
    pub value: Option<u64>,
    pub is_true: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebuggerHistoryEntry {
    pub before_pc: u16,
    pub after_pc: u16,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebuggerStateSnapshot {
    pub mode: DebuggerRunMode,
    pub halted: bool,
    pub pc: u16,
    pub registers: Vec<RegisterField>,
    pub memory_addr: u16,
    pub memory: Vec<u8>,
    pub breakpoints: Vec<DebuggerBreakpoint>,
    pub last_break: Option<u16>,
    pub watches: Vec<DebuggerWatch>,
    pub history: Vec<DebuggerHistoryEntry>,
    pub reverse_depth: usize,
    pub last_event: Option<String>,
    pub disassembly: Vec<DisassemblyLine>,
    pub following_pc: bool,
    #[serde(default)]
    pub bitmap_palette: Vec<BitmapPaletteColor>,
    #[serde(default)]
    pub bitmap_defaults: Option<BitmapExplorerDefaults>,
    /// Port (`IN`/`OUT`) breakpoints currently set.
    #[serde(default)]
    pub port_breakpoints: Vec<PortWatchSpec>,
    /// The `IN`/`OUT` that stopped the machine, while it stays stopped by it
    /// (cleared on resume and on the next step), like `last_break`.
    #[serde(default)]
    pub last_port_hit: Option<PortHitInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebuggerInstanceInfo {
    pub instance_id: String,
    pub machine_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DebuggerHostCommand {
    Pause,
    Resume,
    StepInstruction,
    StepOverInstruction,
    BackstepInstruction,
    ResetMachine,
    KeyEvent {
        key: String,
        pressed: bool,
    },
    SetPc {
        address: u16,
    },
    SetRegister {
        name: String,
        value: u64,
    },
    SetBreakpoint {
        address: u16,
        condition: Option<String>,
    },
    SetBreakpointCondition {
        address: u16,
        condition: Option<String>,
    },
    ClearBreakpoint {
        address: u16,
    },
    ClearAllBreakpoints,
    AddWatch {
        expr: String,
    },
    RemoveWatch {
        index: usize,
    },
    ClearWatches,
    SetMemoryWindow {
        address: u16,
        len: u16,
    },
    SetDisassemblyWindow {
        address: u16,
    },
    FollowPc,
    SetDisassemblyLineCount {
        count: u16,
    },
    ReadMemory {
        address: u16,
        len: u16,
    },
    Poke {
        address: u16,
        bytes: Vec<u8>,
    },
    PokeBank {
        bank: u8,
        address: u16,
        bytes: Vec<u8>,
    },
    Assemble {
        addr: u16,
        text: String,
    },
    LoadState {
        bytes: Vec<u8>,
    },
    LoadSnapshot {
        extension: String,
        bytes: Vec<u8>,
    },
    SaveState,
    SaveSnapshot {
        extension: String,
    },
    GetTimingCounters,
    ResetTimingPartial,
    /// Whatever the machine's generic `chip_id: "overlay"` `ChipSnapshot`
    /// currently carries, formatted as host-chosen text (`key=value` pairs)
    /// -- not CPC-specific despite the historical name this replaced
    /// (`GetCpcIoSnapshot`); any machine plugin that populates its
    /// `"overlay"` chip answers it, ZX81 included (empty text back).
    GetOverlaySnapshot,
    /// Debugger-initiated `OUT (port),value` (ZRCP `write-port`); see
    /// `MachineCommand::WriteIo`.
    WritePort {
        port: u16,
        value: u8,
    },
    /// Adds (or replaces, keyed by `port`+`mask`) a port breakpoint that
    /// pauses the machine after an `IN`/`OUT` matching it. With neither
    /// direction set it removes the breakpoint instead.
    SetPortBreakpoint {
        port: u16,
        mask: u16,
        on_read: bool,
        on_write: bool,
    },
    ClearPortBreakpoints,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DebuggerHostRequest {
    ListInstances,
    GetState {
        instance_id: String,
    },
    Command {
        instance_id: String,
        command: DebuggerHostCommand,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DebuggerHostResponse {
    Ack,
    Instances(Vec<DebuggerInstanceInfo>),
    State(Box<DebuggerStateSnapshot>),
    Bytes(Vec<u8>),
    AssembledBytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebuggerPluginOptionInfo {
    pub key: String,
    pub value_hint: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebuggerPluginInfo {
    pub selector: String,
    pub display_name: String,
    pub summary: String,
    pub options: Vec<DebuggerPluginOptionInfo>,
}
