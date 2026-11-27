//! The `pcengine` machine plugin: profile, options and commands over the `pcengine` machine crate. It is linked
//! into the program and answers the host through `plugin_api::MachinePlugin`: an instance registry behind a mutex,
//! every entry point wrapped by `plugin_api::guard`, and the plain fast path for `step`/`halted`/`pc`. The machine
//! crate forbids `unsafe`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use pcengine::{CANVAS_W, Console, FRAME_CLOCKS, MASTER_HZ, PcEngine, SAMPLE_RATE};
use plugin_api::{
    AddressSpaceInfo, CpuFamily, CpuInfo, DebugCommand, DebugResponse, DebugValue, DisassemblyLine,
    FnPlugin, ListProfilesRequest, ListProfilesResponse, MACHINE_PROFILE_CONTRACT_VERSION,
    MachineCapability, MachineCommand, MachineCreateRequest, MachineCreateResponse,
    MachineInstanceCallRequest, MachineInstanceCallResponse, MachineInstanceHandle, MachineKeyInfo,
    MachineMetadata, MachineOptionInfo, MachineOptionKind, MachinePlugin, MachineProfile,
    MachineResponse, MediaKind, NamedValue, REQUIRED_MACHINE_DEBUG_CAPABILITIES, RegisterField,
    RegisterSnapshot,
};

const MACHINE_ID: &str = "pcengine";
const PLUGIN_ID: &str = "retroclawd.pcengine.machine";
const PLUGIN_VERSION: &str = build_info::version_tag!();

static STATE: OnceLock<Mutex<PluginState>> = OnceLock::new();

struct PluginState {
    next_instance: MachineInstanceHandle,
    instances: HashMap<MachineInstanceHandle, InstanceRecord>,
}

struct InstanceRecord {
    machine: PcEngine,
    breakpoints: Vec<u64>,
    /// The backup RAM's file and the contents read from it (written back at exit when they changed).
    backup: Option<(std::path::PathBuf, Vec<u8>)>,
}

/// Where the backup unit's RAM is kept when the `backup_ram` option names no file: one file, as one backup unit
/// serves every card.
const DEFAULT_BACKUP: &str = "saves/pcengine-backup.bin";

impl Drop for InstanceRecord {
    /// Writes the backup RAM to its file when the machine changed it (to a temporary file, then renamed).
    fn drop(&mut self) {
        let (Some((path, loaded)), Some(now)) = (&self.backup, self.machine.backup_ram()) else {
            return;
        };
        if now == &loaded[..] {
            return;
        }
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir)?;
            }
            let temp = path.with_extension("tmp");
            std::fs::write(&temp, now)?;
            std::fs::rename(&temp, path)
        };
        if let Err(e) = write() {
            eprintln!("pcengine: could not write the backup RAM to {}: {e}", path.display());
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

fn machine_profile() -> MachineProfile {
    MachineProfile {
        machine_id: MACHINE_ID.to_string(),
        display_name: "NEC PC Engine / TurboGrafx-16 (HuCard)".to_string(),
        cpu_family: CpuFamily::Other("huc6280".to_string()),
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
                key: "rom".to_string(),
                flag: "rom".to_string(),
                short: None,
                kind: MachineOptionKind::File,
                default: None,
                required: true,
                description: "The HuCard: a PC Engine / TurboGrafx-16 ROM (.pce, .tg16, .bin); a 512-byte copier header and bit-reversed US dumps are put right".to_string(),
            },
            MachineOptionInfo {
                key: "console".to_string(),
                flag: "console".to_string(),
                short: None,
                kind: MachineOptionKind::Enum {
                    choices: vec!["pce".to_string(), "tg16".to_string()],
                },
                default: Some(DebugValue::Text("pce".to_string())),
                required: false,
                description: "Which console to be: the I/O port tells the game (pce = Japanese PC Engine, tg16 = American TurboGrafx-16)".to_string(),
            },
            MachineOptionInfo {
                key: "multitap".to_string(),
                flag: "multitap".to_string(),
                short: None,
                kind: MachineOptionKind::Bool,
                default: Some(DebugValue::Bool(false)),
                required: false,
                description: "Plug in the five-player multitap so the second pad's keys (Joy2*) and pads 3-5 are read".to_string(),
            },
            MachineOptionInfo {
                key: "supergrafx".to_string(),
                flag: "supergrafx".to_string(),
                short: None,
                kind: MachineOptionKind::Bool,
                default: Some(DebugValue::Bool(false)),
                required: false,
                description: "Be a SuperGrafx: a second video controller, the video priority controller and 32 KB of RAM (for .sgx cards)".to_string(),
            },
            MachineOptionInfo {
                key: "backup_ram".to_string(),
                flag: "backup-ram".to_string(),
                short: None,
                kind: MachineOptionKind::Text,
                default: None,
                required: false,
                description: "The backup unit's 2 KB RAM (game saves): its file (default saves/pcengine-backup.bin; a missing file is a newly formatted RAM), read at start and written at exit when it changed; none: the unit with no file; off: no backup unit".to_string(),
            },
        ],
        key_names: key_names(),
        cpus: vec![CpuInfo {
            id: "main".to_string(),
            name: "HuC6280".to_string(),
            family: CpuFamily::Other("huc6280".to_string()),
            address_space: "main".to_string(),
        }],
        axis_names: Vec::new(),
    }
}

/// The inputs and their default host keys: arrows, Z (I), X (II), Right Shift (Select), Enter (Run).
const KEYS: [(&str, &[&str]); 16] = [
    ("Joy1Up", &["up", "arrowup"]),
    ("Joy1Down", &["down", "arrowdown"]),
    ("Joy1Left", &["left", "arrowleft"]),
    ("Joy1Right", &["right", "arrowright"]),
    ("Joy1I", &["x", "keyx"]),
    ("Joy1II", &["z", "keyz"]),
    ("Joy1Select", &["shift_right", "shiftright"]),
    ("Joy1Run", &["enter", "return"]),
    ("Joy2Up", &[]),
    ("Joy2Down", &[]),
    ("Joy2Left", &[]),
    ("Joy2Right", &[]),
    ("Joy2I", &[]),
    ("Joy2II", &[]),
    ("Joy2Select", &[]),
    ("Joy2Run", &[]),
];

fn key_names() -> Vec<MachineKeyInfo> {
    KEYS.iter()
        .map(|(name, aliases)| MachineKeyInfo::with_aliases(name, aliases))
        .collect()
}

fn metadata() -> MachineMetadata {
    MachineMetadata {
        machine_id: MACHINE_ID.to_string(),
        region: Some("ntsc".to_string()),
        frame_rate: (MASTER_HZ as u32, FRAME_CLOCKS),
        // 512 dots across a 4:3 picture of about 240 lines
        pixel_aspect: (5, 8),
        audio_channels: 2,
        audio_sample_rate: SAMPLE_RATE,
        ..MachineMetadata::default()
    }
}

const MAX_CARD: usize = 0x28_0200;

fn create_machine(req: MachineCreateRequest) -> Result<MachineCreateResponse, String> {
    if req.machine_id != MACHINE_ID {
        return Err(format!(
            "unsupported machine id \"{}\" (plugin supports pcengine)",
            req.machine_id
        ));
    }
    let mut console = Console::PcEngine;
    let mut multitap = false;
    let mut backup_option: Option<String> = None;
    let mut supergrafx = false;
    for option in &req.options {
        match (option.key.as_str(), &option.value) {
            ("multitap", DebugValue::Bool(v)) => multitap = *v,
            ("supergrafx", DebugValue::Bool(v)) => supergrafx = *v,
            ("backup_ram", DebugValue::Text(v)) => backup_option = Some(v.clone()),
            ("console", DebugValue::Text(v)) => {
                console = match v.as_str() {
                    "pce" => Console::PcEngine,
                    "tg16" => Console::TurboGrafx16,
                    other => {
                        return Err(format!(
                            "option \"console\": \"{other}\" is not pce or tg16"
                        ));
                    }
                };
            }
            ("execution_mode", _) => {}
            (key, _) => {
                return Err(format!(
                    "unsupported machine-create option \"{key}\" for {MACHINE_ID}"
                ));
            }
        }
    }
    let rom = req
        .roms
        .iter()
        .find(|blob| blob.key.eq_ignore_ascii_case("rom"))
        .map(|blob| blob.bytes.clone())
        .ok_or_else(|| "missing required binary blob: rom".to_string())?;
    if rom.len() > MAX_CARD {
        return Err(format!(
            "option \"rom\": {} bytes is more than a HuCard's {MAX_CARD}",
            rom.len()
        ));
    }
    let mut machine = PcEngine::from_card(rom, console)?;
    machine.set_multitap(multitap);
    machine.set_supergrafx(supergrafx);
    let backup = match backup_option.as_deref() {
        Some("off") => {
            machine.set_backup_ram(None)?;
            None
        }
        Some("none") => None,
        other => {
            let path = std::path::PathBuf::from(other.unwrap_or(DEFAULT_BACKUP));
            match std::fs::read(&path) {
                Ok(bytes) => machine
                    .set_backup_ram(Some(bytes))
                    .map_err(|e| format!("option \"backup_ram\": {}: {e}", path.display()))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("option \"backup_ram\": {}: {e}", path.display())),
            }
            let loaded = machine.backup_ram().map(<[u8]>::to_vec).unwrap_or_default();
            Some((path, loaded))
        }
    };
    let mut state = state()
        .lock()
        .map_err(|_| "plugin state mutex poisoned".to_string())?;
    let instance = state.allocate_instance();
    state.instances.insert(
        instance,
        InstanceRecord {
            machine,
            breakpoints: Vec::new(),
            backup,
        },
    );
    Ok(MachineCreateResponse {
        instance,
        metadata: metadata(),
    })
}

fn register_snapshot(machine: &PcEngine) -> RegisterSnapshot {
    let c = &machine.cpu;
    let field = |name: &str, value: u64, bits: u16| RegisterField {
        bank: "main".to_string(),
        name: name.to_string(),
        value,
        width_bits: bits,
    };
    let mut registers = vec![
        field("A", u64::from(c.a), 8),
        field("X", u64::from(c.x), 8),
        field("Y", u64::from(c.y), 8),
        field("S", u64::from(c.s), 8),
        field("PC", u64::from(c.pc), 16),
        field("P", u64::from(c.p), 8),
    ];
    for (i, m) in c.mpr.iter().enumerate() {
        registers.push(field(&format!("MPR{i}"), u64::from(*m), 8));
    }
    let flag = |name: &str, bit: u8| NamedValue {
        key: name.to_string(),
        value: DebugValue::Bool(c.p & bit != 0),
    };
    RegisterSnapshot {
        cpu_family: CpuFamily::Other("huc6280".to_string()),
        registers,
        flags: vec![
            flag("N", huc6280::FLAG_N),
            flag("V", huc6280::FLAG_V),
            flag("T", huc6280::FLAG_T),
            flag("B", huc6280::FLAG_B),
            flag("D", huc6280::FLAG_D),
            flag("I", huc6280::FLAG_I),
            flag("Z", huc6280::FLAG_Z),
            flag("C", huc6280::FLAG_C),
        ],
    }
}

fn read_memory(machine: &PcEngine, space: Option<&str>, address: u64) -> u8 {
    match space {
        Some("physical") => machine.peek(address as u32 & 0x1F_FFFF),
        _ => machine.peek_logical(address as u16),
    }
}

fn disassemble(machine: &PcEngine, start: u64, lines: u32) -> Vec<DisassemblyLine> {
    let mut out = Vec::new();
    let mut pc = start as u16;
    for _ in 0..lines {
        let bytes: Vec<u8> = (0..7)
            .map(|i| machine.peek_logical(pc.wrapping_add(i)))
            .collect();
        let (text, len) = huc6280::disassemble::disassemble(&bytes, pc);
        out.push(DisassemblyLine {
            address: u64::from(pc),
            bytes: bytes[..len].to_vec(),
            text,
        });
        pc = pc.wrapping_add(len as u16);
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
                addr_width_bits: 16,
                bank_context: Vec::new(),
            },
            AddressSpaceInfo {
                name: "physical".to_string(),
                addr_width_bits: 21,
                bank_context: Vec::new(),
            },
        ])),
        DebugCommand::ReadMemory {
            address,
            len,
            space,
        } => Ok(DebugResponse::Memory(
            (0..u64::from(len))
                .map(|i| read_memory(&record.machine, space.as_deref(), address.wrapping_add(i)))
                .collect(),
        )),
        DebugCommand::GetDisassembly { start, lines } => {
            let start = start.unwrap_or_else(|| u64::from(record.machine.cpu.pc));
            Ok(DebugResponse::Disassembly(disassemble(
                &record.machine,
                start,
                lines,
            )))
        }
        DebugCommand::GetMachineMetadata => Ok(DebugResponse::MachineMetadata(metadata())),
        DebugCommand::OnCpu { command, .. } => handle_debug_command(record, *command),
        _ => Err(
            "unsupported: this debug command is not implemented for the pcengine yet".to_string(),
        ),
    }
}

fn handle_machine_command(
    record: &mut InstanceRecord,
    command: MachineCommand,
) -> Result<MachineResponse, String> {
    match command {
        MachineCommand::GetClockHz => Ok(MachineResponse::ClockHz(MASTER_HZ)),
        MachineCommand::GetFrameHz => Ok(MachineResponse::FrameHz(60)),
        MachineCommand::RequestInterrupt => Ok(MachineResponse::Ack),
        MachineCommand::Reset => {
            record.machine.reset();
            Ok(MachineResponse::Ack)
        }
        MachineCommand::Step => {
            let cycles = record.machine.step();
            Ok(MachineResponse::Step(plugin_api::StepResult {
                cycles,
                pc: u64::from(record.machine.cpu.pc),
                halted: false,
            }))
        }
        MachineCommand::IsHalted => Ok(MachineResponse::Halted(false)),
        MachineCommand::GetPc => Ok(MachineResponse::Pc(u64::from(record.machine.cpu.pc))),
        MachineCommand::ReadMemory {
            address,
            len,
            space,
        } => Ok(MachineResponse::Memory(
            (0..u64::from(len))
                .map(|i| read_memory(&record.machine, space.as_deref(), address.wrapping_add(i)))
                .collect(),
        )),
        MachineCommand::KeyEvent { key, pressed } => {
            let canonical = key_names()
                .into_iter()
                .find(|info| info.matches(&key))
                .map_or(key.clone(), |info| info.name);
            if record.machine.set_key(&canonical, pressed) {
                Ok(MachineResponse::Ack)
            } else {
                Err(format!("unknown key \"{key}\" (try --list-keys)"))
            }
        }
        MachineCommand::SaveState => Ok(MachineResponse::StateBlob(record.machine.save_state())),
        MachineCommand::LoadState { bytes } => {
            record.machine.load_state(&bytes)?;
            Ok(MachineResponse::Ack)
        }
        MachineCommand::RenderFrame => {
            let (width, height, rgb) = record.machine.render_rgb();
            debug_assert_eq!(width as usize, CANVAS_W);
            Ok(MachineResponse::Frame { width, height, rgb })
        }
        MachineCommand::DrainAudio => Ok(MachineResponse::AudioPcm {
            channels: 2,
            sample_rate: SAMPLE_RATE,
            samples: record.machine.drain_audio(),
        }),
        MachineCommand::Debug(command) => Ok(MachineResponse::Debug(handle_debug_command(
            record, command,
        )?)),
        _ => Err("unsupported: this command is not implemented for the pcengine yet".to_string()),
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

fn plugin_list_profiles(_request: ListProfilesRequest) -> Result<ListProfilesResponse, String> {
    plugin_api::guard::guarded_call("list_profiles", || {
        Ok(ListProfilesResponse {
            profiles: vec![machine_profile()],
        })
    })
}

fn plugin_create_instance(request: MachineCreateRequest) -> Result<MachineCreateResponse, String> {
    plugin_api::guard::guarded_call("create_instance", || create_machine(request))
}

fn plugin_destroy_instance(instance: MachineInstanceHandle) -> Result<(), String> {
    plugin_api::guard::guarded_call("destroy_instance", || {
        let mut state = state()
            .lock()
            .map_err(|_| "plugin state mutex poisoned".to_string())?;
        if state.instances.remove(&instance).is_some() {
            Ok(())
        } else {
            Err(format!("unknown machine instance handle {instance}"))
        }
    })
}

fn plugin_instance_call(
    instance: MachineInstanceHandle,
    request: MachineInstanceCallRequest,
) -> Result<MachineInstanceCallResponse, String> {
    plugin_api::guard::guarded_call("instance_call", || handle_instance_call(instance, request))
}

fn plugin_step_v1(instance: MachineInstanceHandle) -> u32 {
    if plugin_api::guard::has_crashed() {
        return 4;
    }
    plugin_api::guard::guarded("step", 4, || {
        let Ok(mut state) = state().lock() else {
            return 0;
        };
        match state.instances.get_mut(&instance) {
            Some(record) => record.machine.step(),
            None => 0,
        }
    })
}

fn plugin_halted(instance: MachineInstanceHandle) -> bool {
    plugin_api::guard::has_crashed()
        || !state()
            .lock()
            .is_ok_and(|s| s.instances.contains_key(&instance))
}

fn plugin_pc_wide_v1(instance: MachineInstanceHandle) -> u64 {
    plugin_api::guard::guarded("pc_wide", 0, || {
        let Ok(state) = state().lock() else {
            return 0;
        };
        state
            .instances
            .get(&instance)
            .map_or(0, |r| u64::from(r.machine.cpu.pc))
    })
}

static PLUGIN: FnPlugin = FnPlugin {
    id: PLUGIN_ID,
    version: PLUGIN_VERSION,
    list_profiles: plugin_list_profiles,
    create_instance: plugin_create_instance,
    destroy_instance: plugin_destroy_instance,
    instance_call: plugin_instance_call,
    step: plugin_step_v1,
    halted: plugin_halted,
    pc: plugin_pc_wide_v1,
};

/// The machines of this family, for the host to link in.
#[must_use]
pub fn plugin() -> &'static dyn MachinePlugin {
    &PLUGIN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_names_the_machine() {
        let p = machine_profile();
        assert_eq!(p.machine_id, "pcengine");
        assert!(p.options.iter().any(|o| o.key == "rom" && o.required));
    }

    #[test]
    fn the_host_key_names_resolve_to_machine_keys() {
        let names = key_names();
        for (host, key) in [
            ("z", "Joy1II"),
            ("KeyX", "Joy1I"),
            ("up", "Joy1Up"),
            ("Enter", "Joy1Run"),
            ("ShiftRight", "Joy1Select"),
            ("ArrowLeft", "Joy1Left"),
        ] {
            let found = names
                .iter()
                .find(|i| i.matches(host))
                .map(|i| i.name.as_str());
            assert_eq!(found, Some(key), "{host}");
        }
    }

    #[test]
    fn a_missing_card_is_refused() {
        let request = MachineCreateRequest {
            machine_id: MACHINE_ID.to_string(),
            region: None,
            roms: Vec::new(),
            options: Vec::new(),
        };
        assert!(create_machine(request).is_err());
    }
}
