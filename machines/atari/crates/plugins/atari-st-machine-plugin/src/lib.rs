//! The `atari-st` machine plugin: profile, options and commands over the `atari-st` machine crate. The C entry
//! points follow the other machine plugins (`zx81-machine-plugin` of retroclawd-sinclair): an instance registry
//! behind a mutex, every exported function wrapped by `plugin_api::guard`, and the raw fast path for
//! `step`/`halted`/`pc`. The `unsafe` here is the C ABI boundary only; the machine crate forbids it.
//!
//! Files: the PC trace (`trace_file`) and the stop report (`diag_file`, else stderr) are written here, so the
//! machine crate does no I/O.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use atari_st::serial::{FileSink, FileSource, LinePeer, MIDI_FRAME_CYCLES};
use atari_st::{Accuracy, AtariSt, Chipset, Config, Monitor, RamSize, Region};
use machine::Machine;
use plugin_api::{
    ABI_MAJOR_V1, ABI_MINOR_V1, AddressSpaceInfo, BinaryBlob, CartridgeStatus, ChipSnapshot,
    CodecError, CpuFamily, CpuInfo, DebugCommand, DebugResponse, DebugValue, DisassemblyLine,
    DiskStatus, HostMouseButton, MACHINE_PROFILE_CONTRACT_VERSION, MachineCapability,
    MachineCommand, MachineCreateRequest, MachineCreateResponse, MachineInstanceCallRequest,
    MachineInstanceCallResponse, MachineInstanceHandle, MachineKeyInfo, MachineMetadata,
    MachineOptionInfo, MachineOptionKind, MachinePluginVTableV1, MachineProfile, MachineResponse,
    MediaKind, MediaSlot, MediaSnapshot, NamedValue, PluginDescriptorV1, PluginKind,
    REQUIRED_MACHINE_DEBUG_CAPABILITIES, RcBuffer, RcCallResult, RcSlice, RcStatus, RegisterField,
    RegisterSnapshot, StatusCode,
};

const MACHINE_ID: &str = "atari-st";
const PLUGIN_ID: &[u8] = b"retroclawd.atari-st.machine";
const PLUGIN_VERSION: &[u8] = build_info::version_tag!().as_bytes();
const RAM_CHOICES: [&str; 6] = ["256k", "512k", "1m", "2m", "2.5m", "4m"];
const DEFAULT_TRACE_LIMIT: u64 = 100_000;
/// I/O accesses the `io_trace_file` records.
const IO_LOG_LIMIT: u32 = 100_000;

const fn static_slice(bytes: &'static [u8]) -> RcSlice {
    RcSlice {
        ptr: bytes.as_ptr(),
        len: bytes.len(),
    }
}

struct StaticPluginDescriptor(PluginDescriptorV1);

// SAFETY: descriptor and vtable refer only to immutable static data/function pointers.
unsafe impl Sync for StaticPluginDescriptor {}

static STATE: OnceLock<Mutex<PluginState>> = OnceLock::new();

static MACHINE_VTABLE_V1: MachinePluginVTableV1 = MachinePluginVTableV1 {
    size: MachinePluginVTableV1::expected_size(),
    list_profiles: plugin_list_profiles_v1,
    create_instance: plugin_create_instance_v1,
    destroy_instance: plugin_destroy_instance_v1,
    instance_call: plugin_instance_call_v1,
    free_buffer: plugin_free_buffer_v1,
    step: plugin_step_v1,
    halted: plugin_halted_v1,
    pc: plugin_pc_v1,
    pc_wide: plugin_pc_wide_v1,
};

static PLUGIN_DESCRIPTOR_V1: StaticPluginDescriptor = StaticPluginDescriptor(PluginDescriptorV1 {
    abi_major: ABI_MAJOR_V1,
    abi_minor: ABI_MINOR_V1,
    kind: PluginKind::Machine,
    id: static_slice(PLUGIN_ID),
    version: static_slice(PLUGIN_VERSION),
    vtable: (&MACHINE_VTABLE_V1 as *const MachinePluginVTableV1).cast(),
});

struct PluginState {
    next_instance: MachineInstanceHandle,
    instances: HashMap<MachineInstanceHandle, InstanceRecord>,
}

struct InstanceRecord {
    machine: AtariSt,
    region: String,
    model: String,
    breakpoints: Vec<u64>,
    trace_file: Option<PathBuf>,
    trace_written: bool,
    io_trace_file: Option<PathBuf>,
    io_trace_written: bool,
    diag_file: Option<PathBuf>,
    /// The host's joystick switches per ST port (bit 7 fire, bits 0-3 the stick).
    joy: [u8; 2],
    /// What each drive's image was loaded as, so it is exported in the same format.
    disk_formats: [DiskFormat; 2],
}

/// The file format an image came in (`.st` raw sectors, `.msa` run-length encoded tracks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DiskFormat {
    #[default]
    St,
    Msa,
}

impl DiskFormat {
    /// By the effective file name's extension; with no name, by the `.msa` ID word.
    fn detect(name: Option<&str>, bytes: &[u8]) -> DiskFormat {
        let by_name = name
            .and_then(|n| n.rsplit_once('.'))
            .map(|(_, e)| e.to_ascii_lowercase());
        match by_name.as_deref() {
            Some("msa") => DiskFormat::Msa,
            Some(_) => DiskFormat::St,
            None if bytes.starts_with(&[0x0E, 0x0F]) => DiskFormat::Msa,
            None => DiskFormat::St,
        }
    }

    fn parse(self, bytes: &[u8]) -> Result<atari_st::st_floppy::Image, String> {
        match self {
            DiskFormat::St => atari_st::st_floppy::Image::parse(bytes).map_err(|e| e.to_string()),
            DiskFormat::Msa => atari_st::st_floppy::msa::parse(bytes).map_err(|e| e.to_string()),
        }
    }

    fn encode(self, image: &atari_st::st_floppy::Image) -> Vec<u8> {
        match self {
            DiskFormat::St => image.to_bytes(),
            DiskFormat::Msa => atari_st::st_floppy::msa::write(image),
        }
    }
}

impl InstanceRecord {
    /// Writes what is due after a step: the stop report once, the trace once it is complete or the machine stopped.
    fn after_step(&mut self) {
        if let Some(report) = self.machine.take_stop_report() {
            match &self.diag_file {
                Some(path) => {
                    if let Err(e) = std::fs::write(path, &report) {
                        eprintln!("atari-st: cannot write {}: {e}", path.display());
                    }
                }
                None => eprint!("atari-st: {report}"),
            }
        }
        if !self.trace_written && (self.machine.trace_full() || self.machine.halted()) {
            self.write_trace();
        }
        if !self.io_trace_written && self.machine.halted() {
            self.write_io_trace();
        }
    }

    fn write_io_trace(&mut self) {
        let Some(path) = &self.io_trace_file else {
            return;
        };
        self.io_trace_written = true;
        let text: String = self
            .machine
            .io_log()
            .iter()
            .map(|a| {
                a.hatari_line()
                    + "
"
            })
            .collect();
        if let Err(e) = std::fs::write(path, text) {
            eprintln!("atari-st: cannot write {}: {e}", path.display());
        }
    }

    fn write_trace(&mut self) {
        let Some(path) = &self.trace_file else { return };
        self.trace_written = true;
        let text: String = self
            .machine
            .trace()
            .iter()
            .map(|pc| format!("{pc:06X}\n"))
            .collect();
        if let Err(e) = std::fs::write(path, text) {
            eprintln!("atari-st: cannot write {}: {e}", path.display());
        }
    }
}

impl Drop for InstanceRecord {
    fn drop(&mut self) {
        if !self.trace_written {
            self.write_trace();
        }
        if !self.io_trace_written {
            self.write_io_trace();
        }
    }
}

impl PluginState {
    fn new() -> Self {
        Self {
            next_instance: 1,
            instances: HashMap::new(),
        }
    }

    fn allocate_instance(&mut self) -> MachineInstanceHandle {
        let instance = self.next_instance;
        self.next_instance = self.next_instance.wrapping_add(1).max(1);
        instance
    }

    fn record_mut(
        &mut self,
        instance: MachineInstanceHandle,
    ) -> Result<&mut InstanceRecord, String> {
        self.instances
            .get_mut(&instance)
            .ok_or_else(|| format!("unknown machine instance handle {instance}"))
    }
}

fn state() -> &'static Mutex<PluginState> {
    STATE.get_or_init(|| Mutex::new(PluginState::new()))
}

fn status_from_error(error: &str) -> StatusCode {
    if error.contains("unknown machine instance handle") {
        StatusCode::NotFound
    } else if error.contains("unsupported") {
        StatusCode::Unsupported
    } else {
        StatusCode::InvalidRequest
    }
}

fn decode_request<T: serde::de::DeserializeOwned>(req: RcSlice) -> Result<T, CodecError> {
    // SAFETY: caller owns request bytes for call duration.
    plugin_api::decode_message(unsafe { req.as_bytes() })
}

fn encode_ok<T: serde::Serialize>(value: &T) -> RcCallResult {
    match plugin_api::encode_message(value) {
        Ok(bytes) => RcCallResult::ok_bytes(bytes),
        Err(e) => e.into_call_result(),
    }
}

fn text_choices(choices: &[&str]) -> MachineOptionKind {
    MachineOptionKind::Enum {
        choices: choices.iter().map(|c| c.to_string()).collect(),
    }
}

fn option(
    key: &str,
    kind: MachineOptionKind,
    default: Option<DebugValue>,
    description: &str,
) -> MachineOptionInfo {
    MachineOptionInfo {
        key: key.to_string(),
        flag: key.replace('_', "-"),
        short: None,
        kind,
        default,
        required: false,
        description: description.to_string(),
    }
}

/// The profile of the ST, STF and STFM (the machine id every test and the first plugin version knew).
#[cfg(test)]
fn machine_profile() -> MachineProfile {
    machine_profile_for(Chipset::St)
}

fn machine_profile_for(chipset: Chipset) -> MachineProfile {
    let text = |s: &str| Some(DebugValue::Text(s.to_string()));
    let (display_name, rom_text) = match chipset {
        Chipset::St => (
            "Atari ST / STF / STFM",
            "192 KB TOS image (EmuTOS 1.4 etos192*.img via --firmware auto, or your own TOS 1.0x)",
        ),
        Chipset::MegaSt => (
            "Atari Mega ST",
            "192 KB TOS image (EmuTOS 1.4 etos192*.img via --firmware auto, or your own TOS 1.02 or 1.04)",
        ),
        Chipset::Ste => (
            "Atari STE",
            "256 KB TOS image at $E00000 (EmuTOS 1.4 etos256*.img via --firmware auto, or your own TOS 1.62 or 2.0x)",
        ),
    };
    MachineProfile {
        machine_id: chipset.id().to_string(),
        display_name: display_name.to_string(),
        cpu_family: CpuFamily::M68k,
        default_region: Some("pal".to_string()),
        contract_version: MACHINE_PROFILE_CONTRACT_VERSION,
        debug_capabilities: REQUIRED_MACHINE_DEBUG_CAPABILITIES.to_vec(),
        capabilities: vec![
            MachineCapability::Video,
            MachineCapability::Audio,
            MachineCapability::Disk,
            MachineCapability::SaveState,
            MachineCapability::Debug,
        ],
        media_kinds: vec![MediaKind::Disk, MediaKind::Cartridge, MediaKind::Program],
        options: vec![
            MachineOptionInfo {
                required: true,
                ..option(
                    "rom",
                    MachineOptionKind::File,
                    None,
                    rom_text,
                )
            },
            option("ram", text_choices(&RAM_CHOICES), text("1m"), "RAM fitted"),
            option(
                "monitor",
                text_choices(&["color", "mono"]),
                text("color"),
                "Monitor: color, or mono (501 lines of 224 clocks, 640x400 one plane)",
            ),
            option(
                "model",
                text_choices(&["st", "stf", "stfm"]),
                text("stfm"),
                "Model (metadata; the models differ only in drives and the TV modulator)",
            ),
            option(
                "accuracy",
                text_choices(&["fast", "accurate"]),
                text("fast"),
                "Emulation granularity: fast batches whole instructions, accurate follows every bus cycle",
            ),
            option(
                "borders",
                text_choices(&["off", "on"]),
                text("off"),
                "Draw the picture with its borders (832x552; needs accuracy = accurate)",
            ),
            option(
                "trace_file",
                MachineOptionKind::Text,
                None,
                "Write the PC of every instruction to this file",
            ),
            option(
                "drives",
                MachineOptionKind::Int {
                    min: Some(1),
                    max: Some(2),
                },
                Some(DebugValue::U64(2)),
                "Floppy drives fitted (A, and B with 2)",
            ),
            option(
                "write_protect",
                MachineOptionKind::Bool,
                Some(DebugValue::Bool(false)),
                "Write-protect the floppy disks: the guest cannot change them (the controller sees the write-protect line)",
            ),
            option(
                "trace_limit",
                MachineOptionKind::Int {
                    min: Some(1),
                    max: Some(10_000_000),
                },
                Some(DebugValue::U64(DEFAULT_TRACE_LIMIT)),
                "Instructions the trace file records",
            ),
            option(
                "io_trace_file",
                MachineOptionKind::Text,
                None,
                "Write every I/O register access to this file, in the format of Hatari's --trace io_all",
            ),
            option(
                "diag_file",
                MachineOptionKind::Text,
                None,
                "Write the diagnostic report of a stop at an unemulated device here (default: stderr)",
            ),
            option(
                "serial_out",
                MachineOptionKind::Text,
                None,
                "Write the characters the guest sends on the RS-232 port to this new file",
            ),
            option(
                "serial_in",
                MachineOptionKind::Text,
                None,
                "Send the bytes of this file (up to 16 MiB) to the guest on the RS-232 port",
            ),
            option(
                "serial_loopback",
                MachineOptionKind::Bool,
                Some(DebugValue::Bool(false)),
                "Connect the RS-232 transmit line to its receive line",
            ),
            option(
                "midi_out",
                MachineOptionKind::Text,
                None,
                "Write the bytes the guest sends to the MIDI OUT port to this new file",
            ),
            option(
                "midi_in",
                MachineOptionKind::Text,
                None,
                "Send the bytes of this file (up to 16 MiB) to the guest on the MIDI IN port",
            ),
            option(
                "midi_loopback",
                MachineOptionKind::Bool,
                Some(DebugValue::Bool(false)),
                "Connect MIDI OUT to MIDI IN",
            ),
            option(
                "centronics_out",
                MachineOptionKind::Text,
                None,
                "Connect a Centronics printer that writes to this new file (without it the printer is off line: BUSY stays set)",
            ),
            option(
                "overwrite_outputs",
                MachineOptionKind::Bool,
                Some(DebugValue::Bool(false)),
                "Let serial_out, midi_out and centronics_out replace a file that already exists (otherwise that is an error)",
            ),
        ]
        .into_iter()
        .chain((0..8).map(|n| {
            option(
                &format!("acsi{n}"),
                MachineOptionKind::File,
                None,
                &format!(
                    "A raw hard disk image (a whole number of 512-byte blocks, at most 512 MiB) behind ACSI target {n}: with an AHDI partition, EmuTOS sees it as a drive. The guest's writes change the copy in memory only."
                ),
            )
        }))
        .collect(),
        key_names: key_names(chipset),
        cpus: vec![CpuInfo {
            id: "main".to_string(),
            name: "MC68000".to_string(),
            family: CpuFamily::M68k,
            address_space: "main".to_string(),
        }],
        axis_names: Vec::new(),
    }
}

/// Host joystick names: `Joy1*` is the ST's joystick port (port 1), `Joy2*` the mouse port (port 0); the bit is the
/// IKBD's (`ikbd::Ikbd::joystick`).
const JOYSTICK_KEYS: [(&str, u8, u8); 10] = [
    ("Joy1Up", 1, 0x01),
    ("Joy1Down", 1, 0x02),
    ("Joy1Left", 1, 0x04),
    ("Joy1Right", 1, 0x08),
    ("Joy1Fire1", 1, 0x80),
    ("Joy2Up", 0, 0x01),
    ("Joy2Down", 0, 0x02),
    ("Joy2Left", 0, 0x04),
    ("Joy2Right", 0, 0x08),
    ("Joy2Fire1", 0, 0x80),
];

/// The characters the unshifted keys type, from the keytops in the IKBD document's Appendix A (letters and digits
/// need none).
const KEY_CHARS: &[(&str, &str)] = &[
    ("Minus", "-"),
    ("Equal", "="),
    ("BracketLeft", "["),
    ("BracketRight", "]"),
    ("Semicolon", ";"),
    ("Quote", "'"),
    ("Backquote", "`"),
    ("Backslash", "\\"),
    ("Comma", ","),
    ("Period", "."),
    ("Slash", "/"),
    ("Space", " "),
];

/// The STE's enhanced-port pad key names: `Joy1PadUp` ... `Joy2PadPause` (`Joy1Pad*` is pad 1, `Joy2Pad*` pad 2).
fn pad_key(name: &str) -> Option<(usize, atari_st::ste::PadButton)> {
    let rest = name
        .get(..4)
        .filter(|p| p.eq_ignore_ascii_case("joy1") || p.eq_ignore_ascii_case("joy2"))?;
    let pad = usize::from(rest.as_bytes()[3] == b'2');
    let suffix = name
        .get(4..)?
        .strip_prefix("Pad")
        .or_else(|| name.get(4..)?.strip_prefix("pad"))?;
    atari_st::ste::PadButton::ALL
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(suffix))
        .map(|&(_, b)| (pad, b))
}

fn key_names(chipset: Chipset) -> Vec<MachineKeyInfo> {
    let mut keys: Vec<MachineKeyInfo> = atari_st::ikbd::SCANCODES
        .iter()
        .map(|(name, _)| MachineKeyInfo::new(name))
        .chain(
            JOYSTICK_KEYS
                .iter()
                .map(|(name, ..)| MachineKeyInfo::new(name)),
        )
        .collect();
    if chipset == Chipset::Ste {
        for pad in [1, 2] {
            for (suffix, _) in atari_st::ste::PadButton::ALL {
                keys.push(MachineKeyInfo::new(&format!("Joy{pad}Pad{suffix}")));
            }
        }
    }
    let missing = plugin_api::set_key_chars(&mut keys, KEY_CHARS);
    debug_assert!(missing.is_empty(), "{missing:?}");
    keys
}

fn blob<'a>(roms: &'a [BinaryBlob], key: &str) -> Option<&'a [u8]> {
    roms.iter()
        .find(|blob| blob.key.eq_ignore_ascii_case(key))
        .map(|blob| blob.bytes.as_slice())
}

fn metadata(machine: &AtariSt, region: &str, model: &str) -> MachineMetadata {
    let rate = machine.frame_rate();
    MachineMetadata {
        machine_id: machine.chipset().id().to_string(),
        model: Some(model.to_string()),
        region: Some(region.to_string()),
        notes: vec![
            "slice 1: CPU, memory map and VBL; stops at the first unemulated device".to_string(),
        ],
        frame_rate: (rate.num, rate.den),
        ..MachineMetadata::default()
    }
}

/// The serial line, MIDI and printer options: files and echoes on the far ends of the ST's lines.
#[derive(Default)]
struct LineOptions {
    serial_out: Option<PathBuf>,
    serial_in: Option<PathBuf>,
    serial_loopback: bool,
    midi_out: Option<PathBuf>,
    midi_in: Option<PathBuf>,
    midi_loopback: bool,
    centronics_out: Option<PathBuf>,
    overwrite: bool,
}

impl LineOptions {
    /// Takes the option when it is one of these; false for any other key.
    fn accept(&mut self, key: &str, value: &DebugValue) -> bool {
        let path = |v: &DebugValue| match v {
            DebugValue::Text(t) if !t.is_empty() => Some(PathBuf::from(t)),
            _ => None,
        };
        let flag = |v: &DebugValue| matches!(v, DebugValue::Bool(true));
        match key {
            "serial_out" => self.serial_out = path(value),
            "serial_in" => self.serial_in = path(value),
            "serial_loopback" => self.serial_loopback = flag(value),
            "midi_out" => self.midi_out = path(value),
            "midi_in" => self.midi_in = path(value),
            "midi_loopback" => self.midi_loopback = flag(value),
            "centronics_out" => self.centronics_out = path(value),
            "overwrite_outputs" => self.overwrite = flag(value),
            _ => return false,
        }
        true
    }

    fn peer(
        &self,
        out: &Option<PathBuf>,
        input: &Option<PathBuf>,
        loopback: bool,
        frame_cycles: u64,
    ) -> Result<Option<LinePeer>, String> {
        if out.is_none() && input.is_none() && !loopback {
            return Ok(None);
        }
        let sink = match out {
            Some(path) => Some(
                FileSink::create(path, self.overwrite)
                    .map_err(|e| format!("{}: {e}", path.display()))?,
            ),
            None => None,
        };
        let source = match input {
            Some(path) => {
                Some(FileSource::open(path).map_err(|e| format!("{}: {e}", path.display()))?)
            }
            None => None,
        };
        Ok(Some(LinePeer::new(sink, source, loopback, frame_cycles)))
    }

    fn connect(&self, machine: &mut AtariSt) -> Result<(), String> {
        if let Some(peer) = self.peer(&self.serial_out, &self.serial_in, self.serial_loopback, 0)? {
            machine.attach_rs232_peer(Box::new(peer));
        }
        if let Some(peer) = self.peer(
            &self.midi_out,
            &self.midi_in,
            self.midi_loopback,
            MIDI_FRAME_CYCLES,
        )? {
            machine.attach_serial_peer(1, Box::new(peer));
        }
        if let Some(path) = &self.centronics_out {
            machine.attach_printer(
                FileSink::create(path, self.overwrite)
                    .map_err(|e| format!("{}: {e}", path.display()))?,
            );
        }
        Ok(())
    }
}

/// The 8.3 name a program gets on its floppy: the file's stem upper-cased with the characters a name may not hold left
/// out and at most eight of them (`PROG` when nothing is left), and `.PRG` (or the file's own `.TOS` / `.TTP`).
fn program_name(name: Option<&str>) -> String {
    let file = name
        .map(|n| n.rsplit(['/', '\\']).next().unwrap_or(n))
        .unwrap_or("PROG.PRG");
    let (stem, ext) = file.rsplit_once('.').unwrap_or((file, "PRG"));
    let clean = |s: &str, max: usize| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .take(max)
            .collect::<String>()
            .to_ascii_uppercase()
    };
    let stem = clean(stem, 8);
    let ext = match clean(ext, 3).as_str() {
        "TOS" => "TOS",
        "TTP" => "TTP",
        _ => "PRG",
    };
    format!("{}.{ext}", if stem.is_empty() { "PROG" } else { &stem })
}

/// `ExportDisk` and `MarkDiskSaved` name the ACSI hard disk behind target `n` as drive `ACSI_DRIVE_BASE + n`.
const ACSI_DRIVE_BASE: u8 = 8;

/// The target number of a media slot called `acsi0` to `acsi7`.
fn acsi_target(slot: &str) -> Result<u8, String> {
    slot.strip_prefix("acsi")
        .and_then(|n| n.parse::<u8>().ok())
        .filter(|&n| n < 8)
        .ok_or_else(|| format!("\"{slot}\" is not an ACSI slot (acsi0 to acsi7)"))
}

fn create_machine(req: MachineCreateRequest) -> Result<MachineCreateResponse, String> {
    let Some(chipset) = Chipset::from_id(&req.machine_id) else {
        return Err(format!(
            "unsupported machine id \"{}\" (plugin supports atari-st, atari-mega-st and atari-ste)",
            req.machine_id
        ));
    };
    let rom =
        blob(&req.roms, "rom").ok_or_else(|| "missing required binary blob: rom".to_string())?;
    let mut ram = RamSize::M1;
    let mut accuracy = Accuracy::Fast;
    let mut monitor = Monitor::Colour;
    let mut borders = false;
    let mut model = "stfm".to_string();
    let mut trace_file = None;
    let mut trace_limit = DEFAULT_TRACE_LIMIT;
    let mut diag_file = None;
    let mut io_trace_file = None;
    let mut drives = 2u8;
    let mut write_protect = false;
    let mut lines = LineOptions::default();
    for option in &req.options {
        match (option.key.as_str(), &option.value) {
            ("ram", DebugValue::Text(v)) => {
                ram = RamSize::parse(v)
                    .ok_or_else(|| format!("option \"ram\": unknown size \"{v}\""))?;
            }
            ("accuracy", DebugValue::Text(v)) => {
                accuracy = match v.as_str() {
                    "fast" => Accuracy::Fast,
                    "accurate" => Accuracy::Accurate,
                    other => return Err(format!("option \"accuracy\": unknown value \"{other}\"")),
                };
            }
            ("borders", DebugValue::Text(v)) => {
                borders = match v.as_str() {
                    "off" => false,
                    "on" => true,
                    other => return Err(format!("option \"borders\": unknown value \"{other}\"")),
                };
            }
            ("model", DebugValue::Text(v)) => model = v.clone(),
            ("monitor", DebugValue::Text(v)) => {
                monitor = match v.as_str() {
                    "color" => Monitor::Colour,
                    "mono" => Monitor::Mono,
                    other => return Err(format!("option \"monitor\": unknown value \"{other}\"")),
                };
            }
            ("trace_file", DebugValue::Text(v)) => trace_file = Some(PathBuf::from(v)),
            ("trace_limit", DebugValue::U64(v)) => trace_limit = *v,
            ("drives", DebugValue::U64(v @ 1..=2)) => drives = *v as u8,
            ("write_protect", DebugValue::Bool(v)) => write_protect = *v,
            ("drives", other) => return Err(format!("option \"drives\": 1 or 2, not {other:?}")),
            ("diag_file", DebugValue::Text(v)) => diag_file = Some(PathBuf::from(v)),
            (key, value) if lines.accept(key, value) => {}
            ("io_trace_file", DebugValue::Text(v)) => io_trace_file = Some(PathBuf::from(v)),
            (key, _) => {
                return Err(format!(
                    "unsupported machine-create option \"{key}\" for {MACHINE_ID}"
                ));
            }
        }
    }
    let region = match req
        .region
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("pal") => Region::Pal,
        Some("ntsc") => Region::Ntsc,
        Some(other) => return Err(format!("unsupported region \"{other}\" (pal or ntsc)")),
    };
    let region_name = if region == Region::Pal { "pal" } else { "ntsc" };
    let machine = AtariSt::with_chipset(
        Config {
            ram,
            region,
            accuracy,
            monitor,
            rom: rom.to_vec(),
            trace_limit: if trace_file.is_some() {
                trace_limit.min(u32::MAX as u64) as u32
            } else {
                0
            },
            io_log_limit: if io_trace_file.is_some() {
                IO_LOG_LIMIT
            } else {
                0
            },
        },
        chipset,
    )?;
    let mut machine = machine;
    machine
        .set_borders(borders)
        .map_err(|e| format!("option \"borders\": {e}"))?;
    lines.connect(&mut machine)?;
    for id in 0..8u8 {
        if let Some(image) = blob(&req.roms, &format!("acsi{id}")) {
            machine
                .attach_hard_disk(id, image.to_vec())
                .map_err(|e| format!("option \"acsi{id}\": {e}"))?;
        }
    }
    machine.set_drives(drives);
    machine.set_version_tag(std::str::from_utf8(PLUGIN_VERSION).unwrap_or("dev"));
    for drive in 0..drives {
        machine.set_write_protect(drive, write_protect);
    }
    let metadata = metadata(&machine, region_name, &model);
    let mut state = state()
        .lock()
        .map_err(|_| "plugin state mutex poisoned".to_string())?;
    let instance = state.allocate_instance();
    state.instances.insert(
        instance,
        InstanceRecord {
            machine,
            region: region_name.to_string(),
            model,
            breakpoints: Vec::new(),
            trace_file,
            trace_written: false,
            io_trace_file,
            io_trace_written: false,
            diag_file,
            joy: [0; 2],
            disk_formats: [DiskFormat::St; 2],
        },
    );
    Ok(MachineCreateResponse { instance, metadata })
}

/// MFP register names in register order (MC68901 register map).
const MFP_REGISTERS: [&str; 24] = [
    "gpip", "aer", "ddr", "iera", "ierb", "ipra", "iprb", "isra", "isrb", "imra", "imrb", "vr",
    "tacr", "tbcr", "tcdcr", "tadr", "tbdr", "tcdr", "tddr", "scr", "ucr", "rsr", "tsr", "udr",
];

/// The devices' state for the debugger: MMU, Shifter, MFP and the scheduler.
fn chip_snapshots(machine: &AtariSt) -> Vec<ChipSnapshot> {
    let chip = |id: &str, fields: Vec<(String, u64)>| ChipSnapshot {
        chip_id: id.to_string(),
        fields: fields
            .into_iter()
            .map(|(key, v)| NamedValue {
                key,
                value: DebugValue::U64(v),
            })
            .collect(),
        blobs: Vec::new(),
    };
    let bus = machine.bus();
    let shifter = bus.shifter();
    let mut video = vec![
        ("video_base".to_string(), shifter.video_base() as u64),
        ("resolution".to_string(), shifter.resolution() as u64),
        ("sync_50hz".to_string(), shifter.sync_50hz() as u64),
        (
            "sync_log_len".to_string(),
            shifter.sync_log().count() as u64,
        ),
    ];
    video.extend(
        shifter
            .palette()
            .iter()
            .enumerate()
            .map(|(i, &c)| (format!("palette{i}"), c as u64)),
    );
    let mut mfp: Vec<(String, u64)> = MFP_REGISTERS
        .iter()
        .enumerate()
        .map(|(i, name)| (name.to_string(), bus.mfp().peek(i as u8) as u64))
        .collect();
    mfp.push(("irq".to_string(), bus.mfp().irq() as u64));
    vec![
        chip("mmu", vec![("config".to_string(), bus.mmu_config() as u64)]),
        chip("shifter", video),
        chip("mfp", mfp),
        chip(
            "scheduler",
            vec![
                ("now".to_string(), machine.now()),
                ("frame".to_string(), machine.vbl_count()),
                ("line".to_string(), machine.line() as u64),
                (
                    "next_event_at".to_string(),
                    machine.next_event_at().unwrap_or(u64::MAX),
                ),
            ],
        ),
        chip(
            "dma",
            vec![
                ("mode".to_string(), bus.dma().mode() as u64),
                ("status".to_string(), bus.dma().status() as u64),
                ("address".to_string(), bus.dma_address() as u64),
            ],
        ),
        chip(
            "fdc",
            ["status", "track", "sector", "data"]
                .iter()
                .enumerate()
                .map(|(i, n)| (n.to_string(), bus.dma().fdc_register_value(i as u8) as u64))
                .collect(),
        ),
    ]
}

fn register_snapshot(machine: &AtariSt) -> RegisterSnapshot {
    let registers = machine
        .registers()
        .into_iter()
        .map(|(name, value)| RegisterField {
            bank: "main".to_string(),
            name: name.to_string(),
            value,
            width_bits: match name {
                "PC" => 24,
                "SR" => 16,
                _ => 32,
            },
        })
        .collect();
    RegisterSnapshot {
        cpu_family: CpuFamily::M68k,
        registers,
        flags: Vec::new(),
    }
}

fn disassembly(machine: &AtariSt, start: u64, lines: u32) -> Vec<DisassemblyLine> {
    let mut cursor = start & 0xFF_FFFF;
    let mut out = Vec::with_capacity(lines as usize);
    for _ in 0..lines {
        let raw: Vec<u8> = (0..10u64)
            .map(|i| machine.read_memory(cursor + i))
            .collect();
        let (text, len) = mc68000::disassemble::disassemble(&raw, cursor as u32);
        let len = len.clamp(2, 10);
        out.push(DisassemblyLine {
            address: cursor,
            bytes: raw[..len].to_vec(),
            text,
        });
        cursor = (cursor + len as u64) & 0xFF_FFFF;
    }
    out
}

fn handle_debug_command(
    record: &mut InstanceRecord,
    command: DebugCommand,
) -> Result<DebugResponse, String> {
    match command {
        DebugCommand::Pause | DebugCommand::Resume => Ok(DebugResponse::Ack),
        DebugCommand::StepInstruction | DebugCommand::StepOver => {
            record.machine.step();
            record.after_step();
            Ok(DebugResponse::Ack)
        }
        DebugCommand::SetBreakpoint { address } => {
            if !record.breakpoints.contains(&address) {
                record.breakpoints.push(address);
                record.breakpoints.sort_unstable();
            }
            Ok(DebugResponse::Ack)
        }
        DebugCommand::ClearBreakpoint { address } => {
            record.breakpoints.retain(|bp| *bp != address);
            Ok(DebugResponse::Ack)
        }
        DebugCommand::ClearAllBreakpoints => {
            record.breakpoints.clear();
            Ok(DebugResponse::Ack)
        }
        DebugCommand::ListBreakpoints => Ok(DebugResponse::Breakpoints(record.breakpoints.clone())),
        DebugCommand::GetRegisters => {
            Ok(DebugResponse::Registers(register_snapshot(&record.machine)))
        }
        DebugCommand::SetRegister { .. } => {
            Err("unsupported: setting registers on the atari-st".to_string())
        }
        DebugCommand::GetAddressSpaces => {
            Ok(DebugResponse::AddressSpaces(vec![AddressSpaceInfo {
                name: "main".to_string(),
                addr_width_bits: 24,
                bank_context: Vec::new(),
            }]))
        }
        DebugCommand::ReadMemory { address, len, .. } => Ok(DebugResponse::Memory(
            (0..len as u64)
                .map(|i| record.machine.read_memory(address.wrapping_add(i)))
                .collect(),
        )),
        DebugCommand::WriteMemory { address, bytes, .. } => {
            record.machine.load(address, &bytes);
            Ok(DebugResponse::Ack)
        }
        DebugCommand::GetDisassembly { start, lines } => Ok(DebugResponse::Disassembly(
            disassembly(&record.machine, start.unwrap_or(record.machine.pc()), lines),
        )),
        DebugCommand::GetMediaSnapshot => Ok(DebugResponse::MediaSnapshot(MediaSnapshot {
            tape: None,
            disks: Vec::new(),
            cartridge: Some(CartridgeStatus {
                attached: record.machine.cartridge().is_some(),
                mapper: None,
                active_banks: Vec::new(),
            }),
        })),
        DebugCommand::GetChipSnapshots => Ok(DebugResponse::ChipSnapshots(chip_snapshots(
            &record.machine,
        ))),
        DebugCommand::OnCpu { cpu, command } => {
            if cpu == "main" {
                handle_debug_command(record, *command)
            } else {
                Err(format!(
                    "unknown cpu \"{cpu}\" (this machine has only \"main\")"
                ))
            }
        }
        DebugCommand::GetMachineMetadata => Ok(DebugResponse::MachineMetadata(metadata(
            &record.machine,
            &record.region,
            &record.model,
        ))),
    }
}

fn handle_machine_command(
    record: &mut InstanceRecord,
    command: MachineCommand,
) -> Result<MachineResponse, String> {
    match command {
        MachineCommand::GetClockHz => Ok(MachineResponse::ClockHz(record.machine.clock_hz())),
        MachineCommand::GetFrameHz => Ok(MachineResponse::FrameHz(record.machine.frame_hz())),
        MachineCommand::RequestInterrupt => Ok(MachineResponse::Ack),
        MachineCommand::Step => {
            let cycles = record.machine.step();
            record.after_step();
            Ok(MachineResponse::Step(plugin_api::StepResult {
                cycles,
                pc: record.machine.pc(),
                halted: record.machine.halted(),
            }))
        }
        MachineCommand::IsHalted => Ok(MachineResponse::Halted(record.machine.halted())),
        MachineCommand::GetPc => Ok(MachineResponse::Pc(record.machine.pc())),
        MachineCommand::Reset => {
            record.machine.reset();
            Ok(MachineResponse::Ack)
        }
        MachineCommand::ReadMemory { address, len, .. } => Ok(MachineResponse::Memory(
            (0..len as u64)
                .map(|i| record.machine.read_memory(address.wrapping_add(i)))
                .collect(),
        )),
        MachineCommand::WriteMemory { space: Some(_), .. } => {
            Err("unsupported: banked memory writes on the atari-st (one address space)".to_string())
        }
        MachineCommand::WriteMemory {
            address,
            bytes,
            space: None,
        }
        | MachineCommand::LoadBytes { address, bytes } => Ok(MachineResponse::BytesWritten(
            record.machine.load(address, &bytes) as u32,
        )),
        MachineCommand::SetPc { address } => {
            record.machine.set_pc(address);
            Ok(MachineResponse::Ack)
        }
        MachineCommand::SaveState => record
            .machine
            .save_state_checked()
            .map(MachineResponse::StateBlob)
            .map_err(|e| e.to_string()),
        MachineCommand::LoadState { bytes } => {
            record
                .machine
                .load_state_checked(&bytes)
                .map_err(|e| e.to_string())?;
            for note in record.machine.take_state_warnings() {
                eprintln!("atari-st: {note}");
            }
            Ok(MachineResponse::Ack)
        }
        MachineCommand::SaveSnapshot { .. } => {
            Err("unsupported: the atari-st has no snapshot format".to_string())
        }
        MachineCommand::AttachMedia {
            slot: MediaSlot::Disk { drive },
            bytes,
            name,
            ..
        } => {
            let format = DiskFormat::detect(name.as_deref(), &bytes);
            let image = format.parse(&bytes)?;
            record.machine.insert_disk(drive, image)?;
            if let Some(f) = record.disk_formats.get_mut(drive as usize) {
                *f = format;
            }
            Ok(MachineResponse::Ack)
        }
        MachineCommand::DetachMedia {
            slot: MediaSlot::Disk { drive },
        } => {
            record.machine.eject_disk(drive);
            Ok(MachineResponse::Ack)
        }
        MachineCommand::ExportDisk {
            drive,
            only_if_modified,
        } => {
            if let Some(id) = drive.checked_sub(ACSI_DRIVE_BASE).filter(|&id| id < 8) {
                // Hard disks are exported by drive number 8 + target.
                return Ok(MachineResponse::DiskImage(
                    if only_if_modified && !record.machine.hard_disk_modified(id) {
                        None
                    } else {
                        record.machine.hard_disk_image(id).map(<[u8]>::to_vec)
                    },
                ));
            }
            if drive >= record.machine.drives() {
                return Err(format!("drive {drive} does not exist"));
            }
            record.machine.commit_disks();
            let format = record.disk_formats[drive as usize];
            Ok(MachineResponse::DiskImage(
                if only_if_modified && !record.machine.disk_modified(drive) {
                    None
                } else {
                    record.machine.disk(drive).map(|image| format.encode(image))
                },
            ))
        }
        MachineCommand::MarkDiskSaved { drive } => {
            if let Some(id) = drive.checked_sub(ACSI_DRIVE_BASE).filter(|&id| id < 8) {
                record.machine.mark_hard_disk_saved(id);
                return Ok(MachineResponse::Ack);
            }
            if drive >= record.machine.drives() {
                return Err(format!("drive {drive} does not exist"));
            }
            record.machine.mark_disk_saved(drive);
            Ok(MachineResponse::Ack)
        }
        MachineCommand::AttachMedia {
            slot: MediaSlot::Other(slot),
            kind: MediaKind::Other(kind),
            bytes,
            ..
        } if kind == "acsi" => {
            let id = acsi_target(&slot)?;
            record.machine.attach_hard_disk(id, bytes)?;
            Ok(MachineResponse::Ack)
        }
        MachineCommand::DetachMedia {
            slot: MediaSlot::Other(slot),
        } if slot.starts_with("acsi") => {
            record.machine.detach_hard_disk(acsi_target(&slot)?);
            Ok(MachineResponse::Ack)
        }
        // A GEMDOS program: the machine gets a floppy built around it (AUTO\NAME.PRG) in drive A, which TOS runs at boot.
        MachineCommand::AttachMedia {
            kind: MediaKind::Program,
            bytes,
            name,
            ..
        } => {
            let file = program_name(name.as_deref());
            let image =
                atari_st::st_floppy::boot::with_auto_program(&file, &bytes).map_err(|e| e.to_string())?;
            record.machine.insert_disk(0, image)?;
            if let Some(f) = record.disk_formats.get_mut(0) {
                *f = DiskFormat::St;
            }
            Ok(MachineResponse::Ack)
        }
        MachineCommand::AttachMedia {
            slot: MediaSlot::Cartridge { slot: 0 },
            kind: MediaKind::Cartridge,
            bytes,
            ..
        } => {
            record.machine.set_cartridge(Some(bytes))?;
            Ok(MachineResponse::Ack)
        }
        MachineCommand::DetachMedia {
            slot: MediaSlot::Cartridge { slot: 0 },
        } => {
            record.machine.set_cartridge(None)?;
            Ok(MachineResponse::Ack)
        }
        MachineCommand::AttachMedia { .. }
        | MachineCommand::DetachMedia { .. }
        | MachineCommand::ExportTape { .. }
        | MachineCommand::Freeze
        | MachineCommand::ExportCartridge
        | MachineCommand::ExportPrinter => {
            Err("unsupported: the atari-st takes floppy disks (.st, .msa) and one ROM cartridge (.stc, up to 128 KB)".to_string())
        }
        MachineCommand::GetMediaSnapshot => Ok(MachineResponse::MediaSnapshot(MediaSnapshot {
            tape: None,
            disks: (0..record.machine.drives())
                .map(|d| DiskStatus {
                    attached: record.machine.disk(d).is_some(),
                    image_id: None,
                    drive: Some(d),
                    head: None,
                    track: None,
                    sector: None,
                    controller_phase: None,
                })
                .collect(),
            cartridge: Some(CartridgeStatus {
                attached: record.machine.cartridge().is_some(),
                mapper: None,
                active_banks: Vec::new(),
            }),
        })),
        MachineCommand::KeyEvent { key, pressed } => {
            if let Some(&(_, port, bit)) = JOYSTICK_KEYS
                .iter()
                .find(|(n, ..)| n.eq_ignore_ascii_case(&key))
            {
                let p = port as usize;
                record.joy[p] = if pressed {
                    record.joy[p] | bit
                } else {
                    record.joy[p] & !bit
                };
                record.machine.joystick(port, record.joy[p]);
            } else if let Some((pad, button)) = pad_key(&key) {
                record.machine.pad_button(pad, button, pressed)?;
            } else {
                record.machine.key_event(&key, pressed)?;
            }
            Ok(MachineResponse::Ack)
        }
        MachineCommand::MouseMotion { dx, dy } => {
            record.machine.mouse_motion(dx as i32, dy as i32);
            Ok(MachineResponse::Ack)
        }
        MachineCommand::MouseButton { button, pressed } => {
            match button {
                HostMouseButton::Left => record.machine.mouse_button(true, pressed),
                HostMouseButton::Right => record.machine.mouse_button(false, pressed),
                // The ST mouse has two buttons.
                HostMouseButton::Middle | HostMouseButton::Other(_) => {}
            }
            Ok(MachineResponse::Ack)
        }
        MachineCommand::SetBorder { .. } | MachineCommand::MouseWheel { .. } => {
            Ok(MachineResponse::Ack)
        }
        MachineCommand::AxisEvent { .. } => Err("this machine has no analogue axes".to_string()),
        MachineCommand::WriteIo { .. } | MachineCommand::SetPortWatches { .. } => Err(
            "unsupported: the 68000 has no I/O port space (devices are memory-mapped)".to_string(),
        ),
        MachineCommand::TakePortHit => Ok(MachineResponse::PortHit(None)),
        MachineCommand::RenderFrame => {
            let (width, height, rgb) = record
                .machine
                .render_frame()
                .ok_or_else(|| "render_frame unavailable".to_string())?;
            Ok(MachineResponse::Frame { width, height, rgb })
        }
        MachineCommand::DrainAudio => {
            let format = record.machine.audio_format();
            Ok(MachineResponse::AudioPcm {
                channels: format.channels,
                sample_rate: format.sample_rate,
                samples: record.machine.drain_audio(),
            })
        }
        MachineCommand::Debug(command) => Ok(MachineResponse::Debug(handle_debug_command(
            record, command,
        )?)),
    }
}

fn handle_instance_call(
    instance: MachineInstanceHandle,
    request: MachineInstanceCallRequest,
) -> Result<MachineInstanceCallResponse, String> {
    let mut state = state()
        .lock()
        .map_err(|_| "plugin state mutex poisoned".to_string())?;
    let record = state.record_mut(instance)?;
    let response = handle_machine_command(record, request.command)?;
    Ok(MachineInstanceCallResponse { response })
}

unsafe extern "C" fn plugin_free_buffer_v1(buffer: RcBuffer) {
    // SAFETY: caller guarantees this buffer originated from RcBuffer::from_vec.
    let _ = unsafe { buffer.into_vec() };
}

unsafe extern "C" fn plugin_list_profiles_v1(req: RcSlice) -> RcCallResult {
    plugin_api::guard::guarded_call("list_profiles", || unsafe {
        plugin_list_profiles_inner(req)
    })
}

unsafe fn plugin_list_profiles_inner(req: RcSlice) -> RcCallResult {
    let _request: plugin_api::ListProfilesRequest = match decode_request(req) {
        Ok(req) => req,
        Err(e) => return e.into_call_result(),
    };
    encode_ok(&plugin_api::ListProfilesResponse {
        profiles: [Chipset::St, Chipset::MegaSt, Chipset::Ste]
            .into_iter()
            .map(machine_profile_for)
            .collect(),
    })
}

unsafe extern "C" fn plugin_create_instance_v1(req: RcSlice) -> RcCallResult {
    plugin_api::guard::guarded_call("create_instance", || unsafe {
        plugin_create_instance_inner(req)
    })
}

unsafe fn plugin_create_instance_inner(req: RcSlice) -> RcCallResult {
    let request: MachineCreateRequest = match decode_request(req) {
        Ok(req) => req,
        Err(e) => return e.into_call_result(),
    };
    match create_machine(request) {
        Ok(response) => encode_ok(&response),
        Err(error) => RcCallResult::error_with_message(status_from_error(&error), &error),
    }
}

unsafe extern "C" fn plugin_destroy_instance_v1(instance: MachineInstanceHandle) -> RcStatus {
    plugin_api::guard::guarded(
        "destroy_instance",
        RcStatus::from_code(StatusCode::InternalError),
        || unsafe { plugin_destroy_instance_inner(instance) },
    )
}

unsafe fn plugin_destroy_instance_inner(instance: MachineInstanceHandle) -> RcStatus {
    let Ok(mut state) = state().lock() else {
        return RcStatus::from_code(StatusCode::InternalError);
    };
    if state.instances.remove(&instance).is_some() {
        RcStatus::ok()
    } else {
        RcStatus::from_code(StatusCode::NotFound)
    }
}

unsafe extern "C" fn plugin_instance_call_v1(
    instance: MachineInstanceHandle,
    req: RcSlice,
) -> RcCallResult {
    plugin_api::guard::guarded_call("instance_call", || unsafe {
        plugin_instance_call_inner(instance, req)
    })
}

unsafe fn plugin_instance_call_inner(
    instance: MachineInstanceHandle,
    req: RcSlice,
) -> RcCallResult {
    let request: MachineInstanceCallRequest = match decode_request(req) {
        Ok(req) => req,
        Err(e) => return e.into_call_result(),
    };
    match handle_instance_call(instance, request) {
        Ok(response) => encode_ok(&response),
        Err(error) => RcCallResult::error_with_message(status_from_error(&error), &error),
    }
}

/// Fast path for `MachinePluginVTableV1::step`: no message round trip.
extern "C" fn plugin_step_v1(instance: MachineInstanceHandle) -> u32 {
    if plugin_api::guard::has_crashed() {
        return 4;
    }
    plugin_api::guard::guarded("step", 4, || plugin_step_inner(instance))
}

fn plugin_step_inner(instance: MachineInstanceHandle) -> u32 {
    let Ok(mut state) = state().lock() else {
        return 0;
    };
    match state.instances.get_mut(&instance) {
        Some(record) => {
            let cycles = record.machine.step();
            record.after_step();
            cycles
        }
        None => 0,
    }
}

extern "C" fn plugin_halted_v1(instance: MachineInstanceHandle) -> u8 {
    if plugin_api::guard::has_crashed() {
        return 1;
    }
    plugin_api::guard::guarded("halted", 1, || {
        let Ok(state) = state().lock() else {
            return 1;
        };
        state
            .instances
            .get(&instance)
            .map_or(1, |r| u8::from(r.machine.halted()))
    })
}

extern "C" fn plugin_pc_v1(instance: MachineInstanceHandle) -> u16 {
    plugin_api::guard::guarded("pc", 0, || plugin_pc_inner(instance) as u16)
}

extern "C" fn plugin_pc_wide_v1(instance: MachineInstanceHandle) -> u64 {
    plugin_api::guard::guarded("pc_wide", 0, || plugin_pc_inner(instance))
}

fn plugin_pc_inner(instance: MachineInstanceHandle) -> u64 {
    let Ok(state) = state().lock() else {
        return 0;
    };
    state.instances.get(&instance).map_or(0, |r| r.machine.pc())
}

/// # Safety
///
/// Returns immutable descriptor storage owned by this plugin.
pub unsafe extern "C" fn entry_v1() -> *const PluginDescriptorV1 {
    &PLUGIN_DESCRIPTOR_V1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine::AUDIO_SAMPLE_RATE;
    use plugin_api::NamedValue;

    fn rom(program: &[u16]) -> Vec<u8> {
        let mut rom = vec![0u8; 196_608];
        rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x00FC_0030u32.to_be_bytes());
        for (i, w) in program.iter().enumerate() {
            rom[0x30 + 2 * i..0x32 + 2 * i].copy_from_slice(&w.to_be_bytes());
        }
        rom
    }

    fn create(program: &[u16], options: Vec<NamedValue>) -> MachineInstanceHandle {
        create_machine(MachineCreateRequest {
            machine_id: MACHINE_ID.to_string(),
            region: Some("pal".to_string()),
            roms: vec![BinaryBlob {
                key: "rom".to_string(),
                bytes: rom(program),
            }],
            options,
        })
        .unwrap()
        .instance
    }

    fn text(key: &str, value: &str) -> NamedValue {
        NamedValue {
            key: key.to_string(),
            value: DebugValue::Text(value.to_string()),
        }
    }

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "atari-st-plugin-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn the_profile_declares_the_options_and_one_m68k_cpu() {
        let p = machine_profile();
        assert_eq!(p.machine_id, "atari-st");
        assert_eq!(p.cpu_family, CpuFamily::M68k);
        assert_eq!(p.cpus.len(), 1);
        let keys: Vec<&str> = p.options.iter().map(|o| o.key.as_str()).collect();
        for key in [
            "rom",
            "ram",
            "monitor",
            "model",
            "accuracy",
            "trace_file",
            "trace_limit",
            "io_trace_file",
            "diag_file",
        ] {
            assert!(keys.contains(&key), "{key} missing from {keys:?}");
        }
        assert!(p.options.iter().any(|o| o.key == "rom" && o.required));
    }

    #[test]
    fn the_io_stop_report_goes_to_the_diag_file_once() {
        let diag = temp("diag.txt");
        let _ = std::fs::remove_file(&diag);
        // MOVE.B $FF9000,D0
        let id = create(
            &[0x1039, 0x00FF, 0x9000],
            vec![text("diag_file", diag.to_str().unwrap())],
        );
        for _ in 0..1000 {
            plugin_step_inner(id);
        }
        assert_eq!(plugin_halted_v1(id), 1);
        let report = std::fs::read_to_string(&diag).expect("report written");
        assert!(report.contains("stop: read byte $FF9000"), "{report}");
        std::fs::write(&diag, "").unwrap();
        for _ in 0..1000 {
            plugin_step_inner(id);
        }
        assert_eq!(std::fs::read_to_string(&diag).unwrap(), "", "reported once");
        state().lock().unwrap().instances.remove(&id);
        let _ = std::fs::remove_file(&diag);
    }

    #[test]
    fn trace_file_lists_pcs_up_to_the_limit() {
        let trace = temp("trace.txt");
        let _ = std::fs::remove_file(&trace);
        let id = create(
            &[0x4E71, 0x4E71, 0x60FE],
            vec![
                text("trace_file", trace.to_str().unwrap()),
                NamedValue {
                    key: "trace_limit".to_string(),
                    value: DebugValue::U64(5),
                },
            ],
        );
        for _ in 0..100 {
            plugin_step_inner(id);
        }
        let lines: Vec<String> = std::fs::read_to_string(&trace)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        assert_eq!(lines, ["FC0030", "FC0032", "FC0034", "FC0034", "FC0034"]);
        state().lock().unwrap().instances.remove(&id);
        let _ = std::fs::remove_file(&trace);
    }

    #[test]
    fn io_trace_file_is_written_on_drop() {
        let io = temp("io.txt");
        let _ = std::fs::remove_file(&io);
        // MOVE.B #$12,$FFFC3B (an open address); BRA.S *
        let id = create(
            &[0x13FC, 0x0012, 0x00FF, 0xFC3B, 0x60FE],
            vec![text("io_trace_file", io.to_str().unwrap())],
        );
        for _ in 0..10 {
            plugin_step_inner(id);
        }
        state().lock().unwrap().instances.remove(&id);
        let text = std::fs::read_to_string(&io).expect("I/O trace written");
        assert_eq!(
            text,
            "IO write.b $fffffc3b = $12 pc=fc0030
"
        );
        let _ = std::fs::remove_file(&io);
    }

    fn command(
        id: MachineInstanceHandle,
        command: MachineCommand,
    ) -> Result<MachineResponse, String> {
        let mut state = state().lock().unwrap();
        let record = state.instances.get_mut(&id).unwrap();
        handle_machine_command(record, command)
    }

    fn mb(value: u8, addr: u16) -> [u16; 3] {
        [0x11FC, value as u16, addr]
    }

    fn mw(value: u16, addr: u16) -> [u16; 3] {
        [0x31FC, value, addr]
    }

    /// Writes zeros from `$10000` to sector 3 of cylinder 0, side 0 by DMA, then loops.
    fn write_sector_3_program() -> Vec<u16> {
        let mut p = Vec::new();
        for (v, a) in [(7, 0x8800), (0xC0, 0x8802), (14, 0x8800), (0x05, 0x8802)] {
            p.extend(mb(v, a));
        }
        p.extend(mb(0, 0x860D));
        p.extend(mb(0, 0x860B));
        p.extend(mb(1, 0x8609));
        p.extend(mw(0x090, 0x8606));
        p.extend(mw(0x190, 0x8606));
        p.extend(mw(1, 0x8604));
        p.extend(mw(0x184, 0x8606));
        p.extend(mw(3, 0x8604));
        p.extend(mw(0x180, 0x8606));
        p.extend(mw(0xA8, 0x8604));
        p.push(0x60FE);
        p
    }

    /// 720K, every byte `$AA`, with the BPB of a 720K disk.
    fn aa_disk() -> Vec<u8> {
        let mut b = vec![0xAAu8; 737_280];
        b[11..13].copy_from_slice(&512u16.to_le_bytes());
        b[19..21].copy_from_slice(&1440u16.to_le_bytes());
        b[24..26].copy_from_slice(&9u16.to_le_bytes());
        b[26..28].copy_from_slice(&2u16.to_le_bytes());
        b
    }

    fn attach_named(id: MachineInstanceHandle, drive: u8, name: &str, bytes: Vec<u8>) {
        command(
            id,
            MachineCommand::AttachMedia {
                slot: MediaSlot::Disk { drive },
                kind: MediaKind::Disk,
                name: Some(name.to_string()),
                bytes,
                options: Vec::new(),
            },
        )
        .unwrap();
    }

    fn run_steps(id: MachineInstanceHandle, steps: usize) {
        for _ in 0..steps {
            command(id, MachineCommand::Step).unwrap();
        }
    }

    fn export(id: MachineInstanceHandle, drive: u8, only_if_modified: bool) -> Option<Vec<u8>> {
        match command(
            id,
            MachineCommand::ExportDisk {
                drive,
                only_if_modified,
            },
        ) {
            Ok(MachineResponse::DiskImage(i)) => i,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn export_returns_the_written_sector_and_dirty() {
        let id = create(&write_sector_3_program(), Vec::new());
        attach_named(id, 0, "disk.st", aa_disk());
        run_steps(id, 400_000);
        let bytes = export(id, 0, true).expect("modified");
        assert_eq!(bytes.len(), 737_280);
        let sector = 2 * 512;
        assert!(
            bytes[sector..sector + 512].iter().all(|&b| b == 0),
            "sector 3 was written"
        );
        assert_eq!(bytes[sector - 1], 0xAA, "sector 2 untouched");
        assert_eq!(bytes[sector + 512], 0xAA, "sector 4 untouched");
        state().lock().unwrap().instances.remove(&id);
    }

    #[test]
    fn mark_saved_clears_dirty() {
        let id = create(&write_sector_3_program(), Vec::new());
        attach_named(id, 0, "disk.st", aa_disk());
        run_steps(id, 400_000);
        assert!(export(id, 0, true).is_some());
        assert!(matches!(
            command(id, MachineCommand::MarkDiskSaved { drive: 0 }),
            Ok(MachineResponse::Ack)
        ));
        assert!(export(id, 0, true).is_none());
        assert!(export(id, 0, false).is_some(), "the image is still there");
        state().lock().unwrap().instances.remove(&id);
    }

    #[test]
    fn an_msa_disk_exports_as_msa() {
        let id = create(&[0x60FE], Vec::new());
        let st = atari_st::st_floppy::Image::parse(&aa_disk()).unwrap();
        attach_named(id, 0, "game.MSA", atari_st::st_floppy::msa::write(&st));
        let out = export(id, 0, false).unwrap();
        assert_eq!(&out[..2], &[0x0E, 0x0F]);
        assert_eq!(atari_st::st_floppy::msa::parse(&out).unwrap(), st);
        state().lock().unwrap().instances.remove(&id);
    }

    #[test]
    fn write_protect_option_refuses_writes() {
        let id = create(
            &write_sector_3_program(),
            vec![NamedValue {
                key: "write_protect".to_string(),
                value: DebugValue::Bool(true),
            }],
        );
        attach_named(id, 0, "disk.st", aa_disk());
        run_steps(id, 400_000);
        assert!(export(id, 0, true).is_none(), "nothing was written");
        assert_eq!(export(id, 0, false).unwrap(), aa_disk());
        state().lock().unwrap().instances.remove(&id);
    }

    fn create_with(options: Vec<NamedValue>) -> Result<MachineInstanceHandle, String> {
        create_machine(MachineCreateRequest {
            machine_id: MACHINE_ID.to_string(),
            region: Some("pal".to_string()),
            roms: vec![BinaryBlob {
                key: "rom".to_string(),
                bytes: rom(&[0x60FE]),
            }],
            options,
        })
        .map(|r| r.instance)
    }

    #[test]
    fn an_existing_output_file_is_an_error_unless_overwrite_is_asked_for() {
        let path = temp("line-out");
        std::fs::write(&path, b"keep").unwrap();
        for key in ["serial_out", "midi_out", "centronics_out"] {
            let Err(err) = create_with(vec![text(key, path.to_str().unwrap())]) else {
                panic!("{key} replaced a file");
            };
            assert!(err.contains("exists"), "{key}: {err}");
            assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        }
        let id = create_with(vec![
            text("serial_out", path.to_str().unwrap()),
            NamedValue {
                key: "overwrite_outputs".to_string(),
                value: DebugValue::Bool(true),
            },
        ])
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"", "replaced on request");
        state().lock().unwrap().instances.remove(&id);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_unreadable_input_file_is_an_error() {
        let Err(err) = create_with(vec![text(
            "serial_in",
            temp("no-such-input").to_str().unwrap(),
        )]) else {
            panic!("an unreadable input was accepted");
        };
        assert!(err.contains("no-such-input"), "{err}");
    }

    #[test]
    fn a_program_attached_as_program_media_becomes_the_floppy_in_drive_a() {
        let id = create(&[0x60FE], Vec::new());
        let mut prg = vec![0u8; 28];
        prg[0] = 0x60;
        prg[1] = 0x1A;
        prg.extend([0x4E, 0x75, 0, 0, 0, 0]);
        let attach = |name: Option<&str>, bytes: Vec<u8>| {
            command(
                id,
                MachineCommand::AttachMedia {
                    slot: MediaSlot::Other("program".to_string()),
                    kind: MediaKind::Program,
                    name: name.map(str::to_string),
                    bytes,
                    options: Vec::new(),
                },
            )
        };
        assert!(matches!(
            attach(Some(r"C:\games\My Tool!.prg"), prg.clone()),
            Ok(MachineResponse::Ack)
        ));
        let Ok(MachineResponse::DiskImage(Some(image))) = command(
            id,
            MachineCommand::ExportDisk {
                drive: 0,
                only_if_modified: false,
            },
        ) else {
            panic!("no disk in drive A");
        };
        // The name is made an 8.3 name: letters and digits, upper case, at most 8: `MYTOOL.PRG`.
        assert!(
            image.windows(11).any(|w| w == b"MYTOOL  PRG"),
            "the program's directory entry"
        );
        // Not a GEMDOS program, and nothing at all, are refused with the reason.
        assert!(attach(None, vec![1, 2, 3]).unwrap_err().contains("$601A"));
        assert!(attach(None, Vec::new()).is_err());
        state().lock().unwrap().instances.remove(&id);
    }

    #[test]
    fn hard_disks_attach_by_option_and_by_media_and_two_targets_are_independent() {
        let blob = |key: &str, bytes: Vec<u8>| BinaryBlob {
            key: key.to_string(),
            bytes,
        };
        // An option with a bad image is an error that names the option.
        let bad = create_machine(MachineCreateRequest {
            machine_id: MACHINE_ID.to_string(),
            region: Some("pal".to_string()),
            roms: vec![blob("rom", rom(&[0x60FE])), blob("acsi2", vec![0; 1000])],
            options: Vec::new(),
        });
        let Err(e) = bad else {
            panic!("an image that is not whole blocks was accepted");
        };
        assert!(e.contains("acsi2") && e.contains("512"), "{e}");
        let id = create_machine(MachineCreateRequest {
            machine_id: MACHINE_ID.to_string(),
            region: Some("pal".to_string()),
            roms: vec![blob("rom", rom(&[0x60FE])), blob("acsi0", vec![1; 1024])],
            options: Vec::new(),
        })
        .unwrap()
        .instance;
        // Target 1 by media: a different image.
        assert!(matches!(
            command(
                id,
                MachineCommand::AttachMedia {
                    slot: MediaSlot::Other("acsi1".to_string()),
                    kind: MediaKind::Other("acsi".to_string()),
                    name: None,
                    bytes: vec![2; 2048],
                    options: Vec::new(),
                }
            ),
            Ok(MachineResponse::Ack)
        ));
        let export = |drive| match command(
            id,
            MachineCommand::ExportDisk {
                drive,
                only_if_modified: false,
            },
        ) {
            Ok(MachineResponse::DiskImage(i)) => i,
            other => panic!("{other:?}"),
        };
        assert_eq!(export(8), Some(vec![1; 1024]));
        assert_eq!(export(9), Some(vec![2; 2048]));
        assert_eq!(export(10), None, "no disk behind target 2");
        // Not modified: nothing to save.
        let Ok(MachineResponse::DiskImage(none)) = command(
            id,
            MachineCommand::ExportDisk {
                drive: 8,
                only_if_modified: true,
            },
        ) else {
            panic!();
        };
        assert!(none.is_none());
        // A state saved and loaded with disks attached still works (the disks are the user's, not the state's).
        let Ok(MachineResponse::StateBlob(saved)) = command(id, MachineCommand::SaveState) else {
            panic!("no state");
        };
        assert!(matches!(
            command(id, MachineCommand::LoadState { bytes: saved }),
            Ok(MachineResponse::Ack)
        ));
        assert_eq!(export(9), Some(vec![2; 2048]));
        // Detaching removes it; a slot that is not an ACSI slot is an error.
        assert!(matches!(
            command(
                id,
                MachineCommand::DetachMedia {
                    slot: MediaSlot::Other("acsi1".to_string())
                }
            ),
            Ok(MachineResponse::Ack)
        ));
        assert_eq!(export(9), None);
        assert!(
            command(
                id,
                MachineCommand::AttachMedia {
                    slot: MediaSlot::Other("acsi9".to_string()),
                    kind: MediaKind::Other("acsi".to_string()),
                    name: None,
                    bytes: vec![0; 512],
                    options: Vec::new(),
                }
            )
            .is_err()
        );
        state().lock().unwrap().instances.remove(&id);
    }

    #[test]
    fn a_cartridge_attaches_shows_in_memory_and_detaches() {
        let id = create(&[0x60FE], Vec::new());
        let image = vec![0xAB, 0xCD, 0xEF, 0x42, 1, 2, 3, 4];
        assert!(matches!(
            command(
                id,
                MachineCommand::AttachMedia {
                    slot: MediaSlot::Cartridge { slot: 0 },
                    kind: MediaKind::Cartridge,
                    name: Some("cartridge.stc".to_string()),
                    bytes: image,
                    options: Vec::new(),
                }
            ),
            Ok(MachineResponse::Ack)
        ));
        let read = |id| match command(
            id,
            MachineCommand::Debug(DebugCommand::ReadMemory {
                address: 0xFA_0000,
                len: 4,
                space: None,
            }),
        ) {
            Ok(MachineResponse::Debug(DebugResponse::Memory(bytes))) => bytes,
            other => panic!("{other:?}"),
        };
        assert_eq!(read(id), vec![0xAB, 0xCD, 0xEF, 0x42]);
        let attached = |id| match command(id, MachineCommand::GetMediaSnapshot) {
            Ok(MachineResponse::MediaSnapshot(m)) => m.cartridge.unwrap().attached,
            other => panic!("{other:?}"),
        };
        assert!(attached(id));
        assert!(matches!(
            command(
                id,
                MachineCommand::DetachMedia {
                    slot: MediaSlot::Cartridge { slot: 0 }
                }
            ),
            Ok(MachineResponse::Ack)
        ));
        assert_eq!(read(id), vec![0xFF; 4]);
        assert!(!attached(id));
        // Too big, and a slot that does not exist, are errors.
        let big = |slot, len| MachineCommand::AttachMedia {
            slot: MediaSlot::Cartridge { slot },
            kind: MediaKind::Cartridge,
            name: None,
            bytes: vec![0; len],
            options: Vec::new(),
        };
        assert!(
            command(id, big(0, 128 * 1024 + 1))
                .unwrap_err()
                .contains("128 KB")
        );
        assert!(command(id, big(1, 16)).is_err());
        state().lock().unwrap().instances.remove(&id);
    }

    #[test]
    fn drive_b_export_is_independent_of_drive_a() {
        let id = create(&write_sector_3_program(), Vec::new());
        attach_named(id, 0, "disk.st", aa_disk());
        attach_named(id, 1, "other.st", aa_disk());
        run_steps(id, 400_000);
        assert!(export(id, 0, true).is_some());
        assert!(
            export(id, 1, true).is_none(),
            "the program writes drive A only"
        );
        assert!(
            command(
                id,
                MachineCommand::ExportDisk {
                    drive: 2,
                    only_if_modified: false
                }
            )
            .is_err()
        );
        state().lock().unwrap().instances.remove(&id);
    }

    #[test]
    fn a_state_saved_through_the_plugin_loads_back() {
        let id = create(&write_sector_3_program(), Vec::new());
        attach_named(id, 0, "disk.st", aa_disk());
        run_steps(id, 50_000);
        let Ok(MachineResponse::StateBlob(saved)) = command(id, MachineCommand::SaveState) else {
            panic!("no state");
        };
        assert_eq!(&saved[..4], b"RCST");
        run_steps(id, 350_000);
        assert!(
            export(id, 0, true).is_some(),
            "the program wrote its sector"
        );
        assert!(matches!(
            command(
                id,
                MachineCommand::LoadState {
                    bytes: saved.clone()
                }
            ),
            Ok(MachineResponse::Ack)
        ));
        // The disk is the user's file: it keeps what the guest wrote since the state, and still counts as modified
        // (so --disk-save does not lose it).
        assert!(export(id, 0, true).is_some(), "the disk is still modified");
        run_steps(id, 350_000);
        assert!(export(id, 0, true).is_some());
        // A state of another machine is refused with the reason.
        let mut bad = saved;
        bad[0] = b'X';
        let err = command(id, MachineCommand::LoadState { bytes: bad }).unwrap_err();
        assert!(err.contains("RCST"), "{err}");
        state().lock().unwrap().instances.remove(&id);
    }

    #[test]
    fn only_the_ste_lists_the_pad_keys() {
        let has = |chipset: Chipset, name: &str| {
            machine_profile_for(chipset)
                .key_names
                .iter()
                .any(|k| k.name == name)
        };
        for name in ["Joy1PadUp", "Joy1PadA", "Joy2PadPause", "Joy2PadOption"] {
            assert!(has(Chipset::Ste, name), "{name}");
            assert!(
                !has(Chipset::St, name) && !has(Chipset::MegaSt, name),
                "{name}"
            );
        }
        assert!(has(Chipset::St, "Joy1Up") && has(Chipset::Ste, "Joy1Up"));
        assert_eq!(pad_key("Joy2PadLeft").map(|(p, _)| p), Some(1));
        assert!(pad_key("Joy1Up").is_none() && pad_key("KeyA").is_none());
    }

    #[test]
    fn profile_lists_atari_key_names() {
        let p = machine_profile();
        let names: Vec<&str> = p.key_names.iter().map(|k| k.name.as_str()).collect();
        for n in [
            "KeyA",
            "Space",
            "Help",
            "Undo",
            "NumpadEnter",
            "IntlBackslash",
            "Joy1Up",
            "Joy1Fire1",
            "Joy2Left",
        ] {
            assert!(names.contains(&n), "{n}");
        }
        assert_eq!(names.len(), atari_st::ikbd::SCANCODES.len() + 10);
        let chars = |n: &str| {
            p.key_names
                .iter()
                .find(|k| k.name == n)
                .unwrap()
                .chars
                .clone()
        };
        assert_eq!(chars("Semicolon"), ";");
        assert_eq!(chars("Space"), " ");
    }

    #[test]
    fn unknown_key_name_is_an_error() {
        let id = create(&[0x60FE], Vec::new());
        let r = command(
            id,
            MachineCommand::KeyEvent {
                key: "NoSuchKey".to_string(),
                pressed: true,
            },
        );
        assert!(r.unwrap_err().contains("NoSuchKey"));
        for key in ["KeyA", "Joy1Fire1"] {
            let r = command(
                id,
                MachineCommand::KeyEvent {
                    key: key.to_string(),
                    pressed: true,
                },
            );
            assert!(matches!(r, Ok(MachineResponse::Ack)), "{key}");
        }
        assert!(matches!(
            command(id, MachineCommand::MouseMotion { dx: 3, dy: -1 }),
            Ok(MachineResponse::Ack)
        ));
        state().lock().unwrap().instances.remove(&id);
    }

    fn attach(
        id: MachineInstanceHandle,
        drive: u8,
        bytes: Vec<u8>,
    ) -> Result<MachineResponse, String> {
        command(
            id,
            MachineCommand::AttachMedia {
                slot: MediaSlot::Disk { drive },
                kind: MediaKind::Disk,
                name: Some("disk.st".to_string()),
                bytes,
                options: Vec::new(),
            },
        )
    }

    #[test]
    fn attach_st_inserts_into_drive_a() {
        let id = create(&[0x60FE], Vec::new());
        assert!(matches!(
            attach(id, 0, vec![0; 737_280]),
            Ok(MachineResponse::Ack)
        ));
        let Ok(MachineResponse::MediaSnapshot(media)) =
            command(id, MachineCommand::GetMediaSnapshot)
        else {
            panic!("no media snapshot");
        };
        assert!(media.disks.iter().any(|d| d.attached && d.drive == Some(0)));
        let Ok(MachineResponse::DiskImage(Some(bytes))) = command(
            id,
            MachineCommand::ExportDisk {
                drive: 0,
                only_if_modified: false,
            },
        ) else {
            panic!("no image");
        };
        assert_eq!(bytes.len(), 737_280);
        assert!(matches!(
            command(
                id,
                MachineCommand::ExportDisk {
                    drive: 0,
                    only_if_modified: true
                }
            ),
            Ok(MachineResponse::DiskImage(None))
        ));
        assert!(matches!(
            command(
                id,
                MachineCommand::DetachMedia {
                    slot: MediaSlot::Disk { drive: 0 }
                }
            ),
            Ok(MachineResponse::Ack)
        ));
        state().lock().unwrap().instances.remove(&id);
    }

    #[test]
    fn attach_garbage_is_an_error_with_the_geometry_message() {
        let id = create(&[0x60FE], Vec::new());
        let err = attach(id, 0, vec![0; 1000]).unwrap_err();
        assert!(err.contains("512-byte sectors"), "{err}");
        state().lock().unwrap().instances.remove(&id);
    }

    #[test]
    fn drive_b_refused_with_drives_1() {
        let one = NamedValue {
            key: "drives".to_string(),
            value: DebugValue::U64(1),
        };
        let id = create(&[0x60FE], vec![one]);
        let err = attach(id, 1, vec![0; 737_280]).unwrap_err();
        assert!(err.contains("no drive B"), "{err}");
        state().lock().unwrap().instances.remove(&id);
    }

    /// The YM2149's samples reach the host: 44 100 per second of machine time, mono.
    #[test]
    fn drain_audio_returns_the_psg_samples() {
        let id = create(&[0x60FE], Vec::new());
        for _ in 0..20_000 {
            command(id, MachineCommand::Step).unwrap();
        }
        let Ok(MachineResponse::AudioPcm {
            channels,
            sample_rate,
            samples,
        }) = command(id, MachineCommand::DrainAudio)
        else {
            panic!("no audio response");
        };
        assert_eq!((channels, sample_rate), (1, AUDIO_SAMPLE_RATE));
        assert!(!samples.is_empty(), "samples for the time run");
        state().lock().unwrap().instances.remove(&id);
    }

    #[test]
    fn chip_snapshots_name_every_device() {
        let id = create(&[0x60FE], Vec::new());
        let chips = {
            let state = state().lock().unwrap();
            chip_snapshots(&state.instances[&id].machine)
        };
        let ids: Vec<&str> = chips.iter().map(|c| c.chip_id.as_str()).collect();
        assert_eq!(ids, ["mmu", "shifter", "mfp", "scheduler", "dma", "fdc"]);
        let field = |chip: &str, key: &str| {
            chips
                .iter()
                .find(|c| c.chip_id == chip)
                .and_then(|c| c.fields.iter().find(|f| f.key == key))
                .map(|f| f.value.clone())
        };
        assert_eq!(field("mfp", "gpip"), Some(DebugValue::U64(0xA1)));
        assert_eq!(field("mfp", "vr"), Some(DebugValue::U64(0x08)));
        assert_eq!(field("shifter", "palette15"), Some(DebugValue::U64(0)));
        assert_eq!(field("mmu", "config"), Some(DebugValue::U64(0)));
        assert!(field("scheduler", "next_event_at").is_some());
        state().lock().unwrap().instances.remove(&id);
    }

    #[test]
    fn mono_monitor_is_accepted_and_has_the_mono_frame() {
        let created = create_machine(MachineCreateRequest {
            machine_id: MACHINE_ID.to_string(),
            region: Some("pal".to_string()),
            roms: vec![BinaryBlob {
                key: "rom".to_string(),
                bytes: rom(&[0x60FE]),
            }],
            options: vec![text("monitor", "mono")],
        })
        .expect("a mono monitor is created");
        let id = created.instance;
        let mut guard = state().lock().unwrap();
        let machine = guard.instances.get(&id).expect("the instance exists");
        assert_eq!(machine.machine.cycles_per_frame(), 224 * 501);
        guard.instances.remove(&id);
    }

    #[test]
    fn the_plugin_declares_three_machine_ids_with_their_options() {
        let profiles: Vec<_> = [Chipset::St, Chipset::MegaSt, Chipset::Ste]
            .into_iter()
            .map(machine_profile_for)
            .collect();
        let ids: Vec<&str> = profiles.iter().map(|p| p.machine_id.as_str()).collect();
        assert_eq!(ids, ["atari-st", "atari-mega-st", "atari-ste"]);
        let rom_text = |p: &MachineProfile| {
            p.options
                .iter()
                .find(|o| o.key == "rom")
                .map(|o| o.description.clone())
                .unwrap()
        };
        assert!(rom_text(&profiles[0]).contains("192 KB"));
        assert!(rom_text(&profiles[2]).contains("256 KB"));
    }

    #[test]
    fn an_ste_is_created_from_a_256_kb_image_and_reports_its_id() {
        let mut rom = vec![0u8; 262_144];
        rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x00E0_0030u32.to_be_bytes());
        rom[0x30..0x32].copy_from_slice(&0x60FEu16.to_be_bytes());
        let created = create_machine(MachineCreateRequest {
            machine_id: "atari-ste".to_string(),
            region: Some("pal".to_string()),
            roms: vec![BinaryBlob {
                key: "rom".to_string(),
                bytes: rom,
            }],
            options: Vec::new(),
        })
        .expect("an STE is created");
        assert_eq!(created.metadata.machine_id, "atari-ste");
        state().lock().unwrap().instances.remove(&created.instance);
        // An ST image is refused by the STE.
        let err = create_machine(MachineCreateRequest {
            machine_id: "atari-ste".to_string(),
            region: Some("pal".to_string()),
            roms: vec![BinaryBlob {
                key: "rom".to_string(),
                bytes: rom_for_st(),
            }],
            options: Vec::new(),
        })
        .expect_err("refused");
        assert!(err.contains("262144"), "{err}");
    }

    fn rom_for_st() -> Vec<u8> {
        rom(&[0x60FE])
    }
}
