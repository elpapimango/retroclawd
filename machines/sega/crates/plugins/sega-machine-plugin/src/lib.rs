//! The `genesis` machine plugin: profile, options and commands over the `genesis` machine crate. The C entry
//! points follow the other machine plugins: an instance registry behind a mutex, every exported function wrapped by
//! `plugin_api::guard`, and the raw fast path for `step`/`halted`/`pc`. The `unsafe` here is the C ABI boundary only;
//! the machine crate forbids it.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use genesis::{Config, Genesis, Region, Tmss};
use machine::Machine;
use plugin_api::{
    ABI_MAJOR_V1, ABI_MINOR_V1, AddressSpaceInfo, CodecError, CpuFamily, CpuInfo, DebugCommand,
    DebugResponse, DebugValue, DisassemblyLine, MACHINE_PROFILE_CONTRACT_VERSION,
    MachineCapability, MachineCommand, MachineCreateRequest, MachineCreateResponse,
    MachineInstanceCallRequest, MachineInstanceCallResponse, MachineInstanceHandle, MachineKeyInfo,
    MachineMetadata, MachineOptionInfo, MachineOptionKind, MachinePluginVTableV1, MachineProfile,
    MachineResponse, MediaKind, PluginDescriptorV1, PluginKind,
    REQUIRED_MACHINE_DEBUG_CAPABILITIES, RcBuffer, RcCallResult, RcSlice, RcStatus, RegisterField,
    RegisterSnapshot, StatusCode,
};

const MACHINE_ID: &str = "genesis";
const PLUGIN_ID: &[u8] = b"retroclawd.genesis.machine";
const PLUGIN_VERSION: &[u8] = build_info::version_tag!().as_bytes();

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
    machine: Genesis,
    breakpoints: Vec<u64>,
    diag_file: Option<std::path::PathBuf>,
    /// Where battery RAM is kept between sessions (only for a cartridge with battery RAM).
    sram_path: Option<std::path::PathBuf>,
}

/// The most a battery RAM file may hold when it is read.
const MAX_SRAM_FILE: u64 = 1 << 20;

/// Where battery RAM goes when the option does not say: `saves/<name>-<hash>.srm` under the working directory (the same
/// place convention as the host's `states/` and `screenshots/`).
fn default_sram_path(header: &md_cart::Header, rom: &[u8]) -> std::path::PathBuf {
    let name: String = header
        .overseas_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(24)
        .collect();
    let mut hash = 0x811C_9DC5u32;
    for b in rom {
        hash = (hash ^ u32::from(*b)).wrapping_mul(0x0100_0193);
    }
    let name = if name.is_empty() {
        "game".to_string()
    } else {
        name
    };
    std::path::Path::new("saves").join(format!("{name}-{hash:08x}.srm"))
}

impl InstanceRecord {
    /// Writes battery RAM to its file (through a temporary file, so a crash never leaves half of one).
    fn save_sram(&mut self) {
        let (Some(path), Some((data, true))) = (&self.sram_path, self.machine.sram()) else {
            return;
        };
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir)?;
            }
            let temp = path.with_extension("srm.tmp");
            std::fs::write(&temp, data)?;
            std::fs::rename(&temp, path)
        };
        match write() {
            Ok(()) => self.machine.mark_sram_saved(),
            Err(e) => eprintln!(
                "genesis: cannot save battery RAM to {}: {e}",
                path.display()
            ),
        }
    }

    /// Writes the stop report once, to the diagnostic file or stderr; saves battery RAM the guest has just locked.
    fn after_step(&mut self) {
        if self.machine.take_sram_flush() {
            self.save_sram();
        }
        if let Some(report) = self.machine.take_stop_report() {
            match &self.diag_file {
                Some(path) => {
                    if let Err(e) = std::fs::write(path, &report) {
                        eprintln!("genesis: cannot write {}: {e}", path.display());
                    }
                }
                None => eprintln!("genesis: {report}"),
            }
        }
    }
}

impl Drop for InstanceRecord {
    fn drop(&mut self) {
        if self.machine.sram_dirty() {
            self.save_sram();
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

fn machine_profile() -> MachineProfile {
    MachineProfile {
        machine_id: MACHINE_ID.to_string(),
        display_name: "Sega Mega Drive / Genesis".to_string(),
        cpu_family: CpuFamily::M68k,
        default_region: Some("ntsc".to_string()),
        contract_version: MACHINE_PROFILE_CONTRACT_VERSION,
        debug_capabilities: REQUIRED_MACHINE_DEBUG_CAPABILITIES.to_vec(),
        capabilities: vec![
            MachineCapability::Video,
            MachineCapability::Audio,
            MachineCapability::SaveState,
            MachineCapability::Debug,
        ],
        media_kinds: vec![MediaKind::Cartridge],
        options: vec![
            MachineOptionInfo {
                required: true,
                ..option(
                    "rom",
                    MachineOptionKind::File,
                    None,
                    "The cartridge: a Mega Drive ROM (.md, .gen, .bin) or an interleaved .smd dump",
                )
            },
            option(
                "lock_on",
                MachineOptionKind::File,
                None,
                "A cartridge to lock onto a Sonic & Knuckles cartridge (Sonic 3 for the combined game): its ROM shows from $200000. Ignored unless the main cartridge is Sonic & Knuckles",
            ),
            option(
                "interlace_output",
                MachineOptionKind::Enum {
                    choices: ["field", "weave"].iter().map(|c| c.to_string()).collect(),
                },
                Some(DebugValue::Text("field".to_string())),
                "What interlace mode 2 shows: one field per frame (field, 224 lines, as Genesis Plus GX does) or both fields woven into a 448-line picture (weave)",
            ),
            option(
                "cheats",
                MachineOptionKind::Text,
                None,
                "Cheat codes separated by commas: Game Genie codes (XXXX-YYYY, patching a word of the ROM), AAAAAA:DDDD (a ROM word patch below $400000) and FFxxxx:DD or FFxxxx:DDDD (work RAM written once a frame, as an Action Replay does). The Game Genie's own menus are not emulated",
            ),
            option(
                "rom_tmss",
                MachineOptionKind::File,
                None,
                "The console's 2 KB boot ROM (TMSS, bios_MD.bin, SHA-1 3f50b76b...): it runs first, shows its licence screen and starts the cartridge. Optional; giving it turns the TMSS lock on",
            ),
            option(
                "tmss",
                MachineOptionKind::Enum {
                    choices: ["auto", "off", "on"]
                        .iter()
                        .map(|c| c.to_string())
                        .collect(),
                },
                Some(DebugValue::Text("auto".to_string())),
                "Trademark security: on shuts the VDP until the game writes SEGA to the TMSS register and makes the version register report a TMSS console; auto turns it on when a boot ROM is given",
            ),
            option(
                "console_region",
                MachineOptionKind::Enum {
                    choices: ["auto", "japan", "usa", "europe"]
                        .iter()
                        .map(|c| c.to_string())
                        .collect(),
                },
                Some(DebugValue::Text("auto".to_string())),
                "Console region: auto picks it from the cartridge header (the Americas first, then Japan, then Europe); europe is PAL (50 Hz, 313 lines), the others NTSC",
            ),
            option(
                "sram_file",
                MachineOptionKind::Text,
                None,
                "Where the cartridge's battery RAM is kept between sessions (default: saves/<game>-<hash>.srm); read at start, written when the game locks it after a change and at exit",
            ),
            option(
                "diag_file",
                MachineOptionKind::Text,
                None,
                "Write the diagnostic report of a stop at an unemulated device here (default: stderr)",
            ),
        ],
        key_names: key_names(),
        cpus: vec![
            CpuInfo {
                id: "main".to_string(),
                name: "MC68000".to_string(),
                family: CpuFamily::M68k,
                address_space: "main".to_string(),
            },
            CpuInfo {
                id: "z80".to_string(),
                name: "Z80".to_string(),
                family: CpuFamily::Z80,
                address_space: "z80".to_string(),
            },
        ],
        axis_names: Vec::new(),
    }
}

/// The host keyboard names a pad 1 button answers to by default (arrows, Z X C, A S D, Enter, Right Shift).
const PAD1_KEYBOARD: [(&str, &[&str]); 12] = [
    ("Up", &["up"]),
    ("Down", &["down"]),
    ("Left", &["left"]),
    ("Right", &["right"]),
    ("A", &["z"]),
    ("B", &["x"]),
    ("C", &["c"]),
    ("X", &["a"]),
    ("Y", &["s"]),
    ("Z", &["d"]),
    ("Start", &["enter", "return"]),
    ("Mode", &["shiftright"]),
];

/// The extra names the host's gamepad defaults bind (`Joy{n}Fire1` is B, `Fire2` A, `Fire3` C).
const FIRE_KEYS: [(&str, &str); 3] = [("Fire1", "B"), ("Fire2", "A"), ("Fire3", "C")];

fn key_names() -> Vec<MachineKeyInfo> {
    let mut keys = Vec::new();
    for port in 1..=4 {
        for (button, aliases) in PAD1_KEYBOARD {
            let aliases = if port == 1 { aliases } else { &[] };
            keys.push(MachineKeyInfo::with_aliases(
                &format!("Joy{port}{button}"),
                aliases,
            ));
        }
        for (name, _) in FIRE_KEYS {
            keys.push(MachineKeyInfo::new(&format!("Joy{port}{name}")));
        }
    }
    keys
}

/// The pad and button a key name stands for.
fn pad_key(name: &str) -> Option<(usize, &'static str)> {
    let rest = name.get(3..)?;
    if !name[..3].eq_ignore_ascii_case("joy") {
        return None;
    }
    let port = match rest.as_bytes().first()? {
        b'1' => 0,
        b'2' => 1,
        b'3' => 2,
        b'4' => 3,
        _ => return None,
    };
    let button = &rest[1..];
    for (n, _) in PAD1_KEYBOARD {
        if n.eq_ignore_ascii_case(button) {
            return Some((port, n));
        }
    }
    FIRE_KEYS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(button))
        .map(|&(_, b)| (port, b))
}

fn crate_cheats(text: &str) -> Result<Vec<genesis::Cheat>, String> {
    genesis::parse_cheats(text).map_err(|e| format!("option \"cheats\": {e}"))
}

fn metadata(machine: &Genesis) -> MachineMetadata {
    let rate = machine.frame_rate();
    MachineMetadata {
        machine_id: MACHINE_ID.to_string(),
        region: Some(machine.region().name().to_string()),
        frame_rate: (rate.num, rate.den),
        audio_channels: machine.audio_format().channels,
        audio_sample_rate: machine.audio_format().sample_rate,
        ..MachineMetadata::default()
    }
}

fn blob<'a>(roms: &'a [plugin_api::BinaryBlob], key: &str) -> Option<&'a [u8]> {
    roms.iter()
        .find(|blob| blob.key.eq_ignore_ascii_case(key))
        .map(|blob| blob.bytes.as_slice())
}

fn create_machine(req: MachineCreateRequest) -> Result<MachineCreateResponse, String> {
    if req.machine_id != MACHINE_ID {
        return Err(format!(
            "unsupported machine id \"{}\" (plugin supports genesis)",
            req.machine_id
        ));
    }
    let rom =
        blob(&req.roms, "rom").ok_or_else(|| "missing required binary blob: rom".to_string())?;
    let cartridge = md_cart::Cartridge::parse(rom).map_err(|e| e.to_string())?;
    let mut console_region = None;
    let mut tmss_choice = "auto".to_string();
    let mut diag_file = None;
    let mut sram_file = None;
    let mut cheats = Vec::new();
    let mut weave = false;
    for option in &req.options {
        match (option.key.as_str(), &option.value) {
            ("tmss", DebugValue::Text(v)) if matches!(v.as_str(), "auto" | "off" | "on") => {
                tmss_choice = v.clone();
            }
            ("console_region", DebugValue::Text(v)) if v == "auto" => {}
            ("console_region", DebugValue::Text(v)) => {
                console_region =
                    Some(Region::parse(v).ok_or_else(|| {
                        format!("option \"console_region\": unknown value \"{v}\"")
                    })?);
            }
            ("diag_file", DebugValue::Text(v)) => diag_file = Some(std::path::PathBuf::from(v)),
            ("interlace_output", DebugValue::Text(v)) => weave = v == "weave",
            ("cheats", DebugValue::Text(v)) => cheats = crate_cheats(v)?,
            ("sram_file", DebugValue::Text(v)) => sram_file = Some(std::path::PathBuf::from(v)),
            (key, _) => {
                return Err(format!(
                    "unsupported machine-create option \"{key}\" for {MACHINE_ID}"
                ));
            }
        }
    }
    let region = console_region.unwrap_or_else(|| Region::from_header(cartridge.header.regions));
    let sram_default = default_sram_path(&cartridge.header, &cartridge.rom);
    let boot_rom = blob(&req.roms, "rom_tmss").map(<[u8]>::to_vec);
    if let Some(rom) = &boot_rom
        && rom.len() != 2048
    {
        return Err(format!(
            "option \"rom_tmss\": the boot ROM is 2048 bytes, this file has {}",
            rom.len()
        ));
    }
    let mut config = Config::new(cartridge.rom, region);
    config.cheats = cheats;
    config.interlace_weave = weave;
    if let Some(bytes) = blob(&req.roms, "lock_on") {
        config.lock_on = Some(
            md_cart::Cartridge::parse(bytes)
                .map_err(|e| format!("option \"lock_on\": {e}"))?
                .rom,
        );
    }
    config.tmss = match tmss_choice.as_str() {
        "on" => Some(Tmss { boot_rom }),
        "auto" if boot_rom.is_some() => Some(Tmss { boot_rom }),
        _ => None,
    };
    let mut machine = Genesis::new(config)?;
    let sram_path = machine
        .sram()
        .is_some_and(|(_, battery)| battery)
        .then(|| sram_file.unwrap_or(sram_default));
    if let Some(path) = &sram_path
        && let Ok(meta) = std::fs::metadata(path)
        && meta.is_file()
    {
        if meta.len() > MAX_SRAM_FILE {
            return Err(format!(
                "battery RAM file {} is {} bytes; at most {MAX_SRAM_FILE} are read",
                path.display(),
                meta.len()
            ));
        }
        machine.load_sram(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?);
    }
    let metadata = metadata(&machine);
    let mut state = state()
        .lock()
        .map_err(|_| "plugin state mutex poisoned".to_string())?;
    let instance = state.allocate_instance();
    state.instances.insert(
        instance,
        InstanceRecord {
            machine,
            breakpoints: Vec::new(),
            diag_file,
            sram_path,
        },
    );
    Ok(MachineCreateResponse { instance, metadata })
}

/// 68000 instructions from `start`.
fn disassembly_68k(machine: &Genesis, start: u64, lines: u32) -> Vec<DisassemblyLine> {
    let mut cursor = start & 0xFF_FFFF;
    let mut out = Vec::with_capacity(lines as usize);
    for _ in 0..lines {
        let raw: Vec<u8> = (0..10u64)
            .map(|i| machine.read_memory(cursor + i))
            .collect();
        let (text, len) = genesis::mc68000::disassemble::disassemble(&raw, cursor as u32);
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

/// Z80 instructions from `start`.
fn disassembly_z80(machine: &Genesis, start: u64, lines: u32) -> Vec<DisassemblyLine> {
    let mut cursor = (start & 0xFFFF) as u16;
    let mut out = Vec::with_capacity(lines as usize);
    for _ in 0..lines {
        let raw: Vec<u8> = (0..4u16)
            .map(|i| machine.z80_peek(cursor.wrapping_add(i)))
            .collect();
        let (text, len) = genesis::z80::disassemble::disassemble_at(&raw, cursor);
        let len = len.clamp(1, 4);
        out.push(DisassemblyLine {
            address: u64::from(cursor),
            bytes: raw[..len].to_vec(),
            text,
        });
        cursor = cursor.wrapping_add(len as u16);
    }
    out
}

fn z80_register_snapshot(machine: &Genesis) -> RegisterSnapshot {
    let registers = machine
        .z80_registers()
        .into_iter()
        .map(|(name, value)| RegisterField {
            bank: "z80".to_string(),
            name: name.to_string(),
            value,
            width_bits: match name {
                "I" | "R" | "IM" | "IFF1" | "IFF2" | "HALT" => 8,
                _ => 16,
            },
        })
        .collect();
    RegisterSnapshot {
        cpu_family: CpuFamily::Z80,
        registers,
        flags: Vec::new(),
    }
}

/// A debug command addressed to the Z80 (`OnCpu { cpu: "z80", .. }`).
fn handle_z80_debug(
    record: &mut InstanceRecord,
    command: DebugCommand,
) -> Result<DebugResponse, String> {
    match command {
        DebugCommand::GetRegisters => Ok(DebugResponse::Registers(z80_register_snapshot(
            &record.machine,
        ))),
        DebugCommand::ReadMemory { address, len, .. } => Ok(DebugResponse::Memory(
            (0..len as u64)
                .map(|i| {
                    record
                        .machine
                        .z80_peek((address.wrapping_add(i) & 0xFFFF) as u16)
                })
                .collect(),
        )),
        DebugCommand::GetDisassembly { start, lines } => {
            Ok(DebugResponse::Disassembly(disassembly_z80(
                &record.machine,
                start.unwrap_or(u64::from(record.machine.z80_pc())),
                lines,
            )))
        }
        DebugCommand::GetAddressSpaces => {
            Ok(DebugResponse::AddressSpaces(vec![AddressSpaceInfo {
                name: "z80".to_string(),
                addr_width_bits: 16,
                bank_context: Vec::new(),
            }]))
        }
        DebugCommand::Pause | DebugCommand::Resume => Ok(DebugResponse::Ack),
        _ => Err("unsupported: this debug command is not implemented for the Z80 yet".to_string()),
    }
}

fn register_snapshot(machine: &Genesis) -> RegisterSnapshot {
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
        DebugCommand::GetAddressSpaces => Ok(DebugResponse::AddressSpaces(vec![
            AddressSpaceInfo {
                name: "main".to_string(),
                addr_width_bits: 24,
                bank_context: Vec::new(),
            },
            AddressSpaceInfo {
                name: "z80".to_string(),
                addr_width_bits: 16,
                bank_context: Vec::new(),
            },
        ])),
        DebugCommand::ReadMemory { address, len, .. } => Ok(DebugResponse::Memory(
            (0..len as u64)
                .map(|i| record.machine.read_memory(address.wrapping_add(i)))
                .collect(),
        )),
        DebugCommand::GetDisassembly { start, lines } => Ok(DebugResponse::Disassembly(
            disassembly_68k(&record.machine, start.unwrap_or(record.machine.pc()), lines),
        )),
        DebugCommand::OnCpu { cpu, command } => {
            if cpu == "main" {
                handle_debug_command(record, *command)
            } else if cpu == "z80" {
                handle_z80_debug(record, *command)
            } else {
                Err(format!(
                    "unknown cpu \"{cpu}\" (this machine has \"main\" and \"z80\")"
                ))
            }
        }
        DebugCommand::GetMachineMetadata => {
            Ok(DebugResponse::MachineMetadata(metadata(&record.machine)))
        }
        _ => Err(
            "unsupported: this debug command is not implemented for the genesis yet".to_string(),
        ),
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
            (0..u64::from(len))
                .map(|i| record.machine.read_memory(address.wrapping_add(i)))
                .collect(),
        )),
        MachineCommand::WriteMemory { space: Some(_), .. } => {
            Err("unsupported: banked memory writes on the genesis".to_string())
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
        MachineCommand::KeyEvent { key, pressed } => {
            let (port, button) =
                pad_key(&key).ok_or_else(|| format!("unknown key \"{key}\" (try --list-keys)"))?;
            record.machine.set_pad_button(port, button, pressed)?;
            Ok(MachineResponse::Ack)
        }
        MachineCommand::SaveState => Ok(MachineResponse::StateBlob(record.machine.save_state())),
        MachineCommand::LoadState { bytes } => {
            record.machine.load_state(&bytes)?;
            Ok(MachineResponse::Ack)
        }
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
        _ => Err("unsupported: this command is not implemented for the genesis yet".to_string()),
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
        profiles: vec![machine_profile()],
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

    #[test]
    fn plugin_lists_the_genesis_profile() {
        let p = machine_profile();
        assert_eq!(p.machine_id, "genesis");
        assert_eq!(p.cpus.len(), 2);
        assert_eq!(p.cpus[0].id, "main");
        assert_eq!(p.cpus[1].id, "z80");
    }

    /// A minimal ROM: vectors, then `MOVE.W #$BEEF,$FF0000 ; BRA.S *` at $200.
    fn rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x400];
        rom[0..4].copy_from_slice(&0x00FF_FE00u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        let program: [u16; 5] = [0x33FC, 0xBEEF, 0x00FF, 0x0000, 0x60FE];
        for (i, w) in program.iter().enumerate() {
            rom[0x200 + 2 * i..0x202 + 2 * i].copy_from_slice(&w.to_be_bytes());
        }
        rom[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
        rom[0x1F0..0x1F3].copy_from_slice(b"E  ");
        rom
    }

    fn request(options: Vec<plugin_api::NamedValue>) -> MachineCreateRequest {
        MachineCreateRequest {
            machine_id: MACHINE_ID.to_string(),
            region: None,
            roms: vec![plugin_api::BinaryBlob {
                key: "rom".to_string(),
                bytes: rom(),
            }],
            options,
        }
    }

    #[test]
    fn a_rom_given_as_a_blob_boots_the_machine_and_the_header_picks_the_region() {
        let created = create_machine(request(Vec::new())).unwrap();
        assert_eq!(created.metadata.region.as_deref(), Some("europe"));
        assert_eq!(created.metadata.frame_rate, (53_203_424, 1_070_460));
        for _ in 0..5 {
            assert!(plugin_step_inner(created.instance) > 0);
        }
        assert_eq!(plugin_halted_v1(created.instance), 0);
        // SAFETY: the handle was just created by this plugin.
        unsafe { plugin_destroy_instance_inner(created.instance) };
        assert_eq!(plugin_step_inner(created.instance), 0);
    }

    #[test]
    fn the_console_region_option_overrides_the_header() {
        let created = create_machine(request(vec![plugin_api::NamedValue {
            key: "console_region".to_string(),
            value: DebugValue::Text("japan".to_string()),
        }]))
        .unwrap();
        assert_eq!(created.metadata.region.as_deref(), Some("japan"));
        assert_eq!(created.metadata.frame_rate, (53_693_175, 896_040));
    }

    #[test]
    fn a_missing_or_empty_rom_and_unknown_options_are_refused() {
        let mut none = request(Vec::new());
        none.roms.clear();
        assert!(create_machine(none).is_err());
        let mut empty = request(Vec::new());
        empty.roms[0].bytes.clear();
        assert!(create_machine(empty).is_err());
        let bogus = request(vec![plugin_api::NamedValue {
            key: "bogus".to_string(),
            value: DebugValue::Bool(true),
        }]);
        assert!(create_machine(bogus).is_err());
        let bad_region = request(vec![plugin_api::NamedValue {
            key: "console_region".to_string(),
            value: DebugValue::Text("mars".to_string()),
        }]);
        assert!(create_machine(bad_region).is_err());
    }

    #[test]
    fn the_key_list_and_the_key_handler_agree() {
        let keys = key_names();
        assert!(keys.len() >= 30);
        for k in &keys {
            assert!(pad_key(&k.name).is_some(), "{}", k.name);
        }
        assert_eq!(pad_key("Joy2Start"), Some((1, "Start")));
        assert_eq!(pad_key("Joy1Fire1"), Some((0, "B")));
        assert_eq!(pad_key("joy1up"), Some((0, "Up")));
        assert!(pad_key("Joy3Up").is_none());
        assert!(pad_key("Space").is_none());
    }

    #[test]
    fn a_pressed_key_reaches_the_pad() {
        let created = create_machine(request(Vec::new())).unwrap();
        let mut state = state().lock().unwrap();
        let record = state.record_mut(created.instance).unwrap();
        let result = handle_machine_command(
            record,
            MachineCommand::KeyEvent {
                key: "Joy1Up".to_string(),
                pressed: true,
            },
        );
        assert!(matches!(result, Ok(MachineResponse::Ack)));
        let bad = handle_machine_command(
            record,
            MachineCommand::KeyEvent {
                key: "Nope".to_string(),
                pressed: true,
            },
        );
        assert!(bad.is_err());
    }

    /// A ROM with battery RAM at $200001 whose program unlocks the RAM, stores $42, and locks it again.
    fn sram_rom() -> Vec<u8> {
        let mut rom = rom();
        let program: [u16; 14] = [
            0x13FC, 0x0001, 0x00A1, 0x30F1, 0x13FC, 0x0042, 0x0020, 0x0001, 0x13FC, 0x0000, 0x00A1,
            0x30F1, 0x60FE, 0x4E71,
        ];
        for (i, w) in program.iter().enumerate() {
            rom[0x200 + 2 * i..0x202 + 2 * i].copy_from_slice(&w.to_be_bytes());
        }
        rom[0x1B0..0x1B4].copy_from_slice(&[b'R', b'A', 0xF8, 0x20]);
        rom[0x1B4..0x1B8].copy_from_slice(&0x20_0001u32.to_be_bytes());
        rom[0x1B8..0x1BC].copy_from_slice(&0x20_0FFFu32.to_be_bytes());
        rom
    }

    #[test]
    fn battery_ram_is_saved_when_the_game_locks_it_and_loaded_at_the_next_start() {
        let dir = std::env::temp_dir().join(format!("sega-sram-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("game.srm");
        let options = || {
            vec![plugin_api::NamedValue {
                key: "sram_file".to_string(),
                value: DebugValue::Text(file.to_string_lossy().into_owned()),
            }]
        };
        let mut req = request(options());
        req.roms[0].bytes = sram_rom();
        let created = create_machine(req).unwrap();
        for _ in 0..30 {
            plugin_step_inner(created.instance);
        }
        let saved = std::fs::read(&file).expect("saved when the game locked the RAM");
        assert_eq!(saved[0], 0x42);
        assert_eq!(saved[1], 0xFF);
        // a second start reads it back
        let mut again = request(options());
        again.roms[0].bytes = sram_rom();
        let second = create_machine(again).unwrap();
        {
            let mut state = state().lock().unwrap();
            let record = state.record_mut(second.instance).unwrap();
            assert_eq!(record.machine.read_memory(0x20_0001), 0x42);
        }
        // an absurdly big file is refused
        std::fs::write(&file, vec![0u8; (MAX_SRAM_FILE + 1) as usize]).unwrap();
        let mut big = request(options());
        big.roms[0].bytes = sram_rom();
        assert!(create_machine(big).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_boot_rom_of_the_wrong_size_is_refused_and_tmss_can_be_chosen() {
        let mut req = request(Vec::new());
        req.roms.push(plugin_api::BinaryBlob {
            key: "rom_tmss".to_string(),
            bytes: vec![0; 100],
        });
        assert!(create_machine(req).unwrap_err().contains("2048"));
        let on = request(vec![plugin_api::NamedValue {
            key: "tmss".to_string(),
            value: DebugValue::Text("on".to_string()),
        }]);
        let created = create_machine(on).unwrap();
        let mut state = state().lock().unwrap();
        let record = state.record_mut(created.instance).unwrap();
        assert!(record.machine.bus().tmss.vdp_locked());
        let bad = request(vec![plugin_api::NamedValue {
            key: "tmss".to_string(),
            value: DebugValue::Text("maybe".to_string()),
        }]);
        assert!(create_machine(bad).is_err());
    }

    #[test]
    fn a_state_saved_through_the_plugin_loads_again() {
        let created = create_machine(request(Vec::new())).unwrap();
        let mut state = state().lock().unwrap();
        let record = state.record_mut(created.instance).unwrap();
        for _ in 0..50 {
            record.machine.step();
        }
        let Ok(MachineResponse::StateBlob(blob)) =
            handle_machine_command(record, MachineCommand::SaveState)
        else {
            panic!("no state");
        };
        assert!(blob.starts_with(b"RCST"));
        assert!(matches!(
            handle_machine_command(record, MachineCommand::LoadState { bytes: blob }),
            Ok(MachineResponse::Ack)
        ));
        assert!(
            handle_machine_command(
                record,
                MachineCommand::LoadState {
                    bytes: vec![1, 2, 3]
                }
            )
            .is_err()
        );
    }

    #[test]
    fn both_cpus_answer_the_debugger() {
        let created = create_machine(request(Vec::new())).unwrap();
        let mut state = state().lock().unwrap();
        let record = state.record_mut(created.instance).unwrap();
        let Ok(DebugResponse::Disassembly(lines)) = handle_debug_command(
            record,
            DebugCommand::GetDisassembly {
                start: Some(0x200),
                lines: 2,
            },
        ) else {
            panic!("no 68000 disassembly");
        };
        assert_eq!(lines[0].address, 0x200);
        assert!(
            lines[0].text.to_ascii_uppercase().contains("MOVE"),
            "{}",
            lines[0].text
        );
        let z80 = |c| DebugCommand::OnCpu {
            cpu: "z80".to_string(),
            command: Box::new(c),
        };
        let Ok(DebugResponse::Registers(regs)) =
            handle_debug_command(record, z80(DebugCommand::GetRegisters))
        else {
            panic!("no Z80 registers");
        };
        assert_eq!(regs.cpu_family, CpuFamily::Z80);
        assert!(regs.registers.iter().any(|r| r.name == "PC"));
        let Ok(DebugResponse::Disassembly(zl)) = handle_debug_command(
            record,
            z80(DebugCommand::GetDisassembly {
                start: Some(0),
                lines: 1,
            }),
        ) else {
            panic!("no Z80 disassembly");
        };
        assert_eq!(zl.len(), 1);
        assert!(
            handle_debug_command(
                record,
                DebugCommand::OnCpu {
                    cpu: "nope".to_string(),
                    command: Box::new(DebugCommand::GetRegisters)
                }
            )
            .is_err()
        );
    }

    #[test]
    fn the_rom_option_is_required_and_a_file() {
        let p = machine_profile();
        let rom = p.options.iter().find(|o| o.key == "rom").unwrap();
        assert!(rom.required);
        assert_eq!(rom.kind, MachineOptionKind::File);
    }
}
