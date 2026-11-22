//! The `neogeo` machine plugin: profile, options and commands over the `neogeo` machine crate. The C entry points follow
//! the other machine plugins: an instance registry behind a mutex, every exported function wrapped by
//! `plugin_api::guard`, and the raw fast path for `step`/`halted`/`pc`. The `unsafe` here is the C ABI boundary only;
//! the machine crate forbids it.

#![cfg_attr(not(feature = "debug"), allow(unused_mut))]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use neogeo::system::{FRAME_CLOCKS, NeoGeo};
use plugin_api::{AddressSpaceInfo, CpuFamily, CpuInfo, DebugCommand, DebugResponse, DebugValue, FnPlugin, ListProfilesRequest, ListProfilesResponse, MACHINE_PROFILE_CONTRACT_VERSION, MachineCapability, MachineCommand, MachineCreateRequest, MachineCreateResponse, MachineInstanceCallRequest, MachineInstanceCallResponse, MachineInstanceHandle, MachineKeyInfo, MachineMetadata, MachineOptionInfo, MachineOptionKind, MachinePlugin, MachineProfile, MachineResponse, MediaKind, REQUIRED_MACHINE_DEBUG_CAPABILITIES, RegisterField, RegisterSnapshot};

const MACHINE_ID: &str = "neogeo";
const PLUGIN_ID: &str = "retroclawd.neogeo.machine";
const PLUGIN_VERSION: &str = build_info::version_tag!();
/// The master clock in Hz (fact N4).
const MASTER_HZ: u64 = 24_000_000;
/// The sound chip's sample rate (fact Y1).
const AUDIO_RATE: u32 = ym2610::SAMPLE_RATE;

static STATE: OnceLock<Mutex<PluginState>> = OnceLock::new();

struct PluginState {
    next_instance: MachineInstanceHandle,
    instances: HashMap<MachineInstanceHandle, InstanceRecord>,
}

struct InstanceRecord {
    machine: NeoGeo,
    /// The BIOS set and the cartridge set the machine was built from, to build it again on a reset.
    sets: (Vec<u8>, Vec<u8>),
    breakpoints: Vec<u64>,
    diag_file: Option<std::path::PathBuf>,
    reported: bool,
}

impl InstanceRecord {
    /// Writes the stop report once, to the diagnostic file or stderr.
    fn after_step(&mut self) {
        if self.reported {
            return;
        }
        if let Some(report) = &self.machine.bus.stop {
            self.reported = true;
            match &self.diag_file {
                Some(path) => {
                    if let Err(e) = std::fs::write(path, report) {
                        eprintln!("neogeo: cannot write {}: {e}", path.display());
                    }
                }
                None => eprintln!("neogeo: {report}"),
            }
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
        display_name: "SNK Neo Geo (MVS cartridge)".to_string(),
        cpu_family: CpuFamily::M68k,
        default_region: Some("ntsc".to_string()),
        contract_version: MACHINE_PROFILE_CONTRACT_VERSION,
        debug_capabilities: REQUIRED_MACHINE_DEBUG_CAPABILITIES.to_vec(),
        capabilities: vec![
            MachineCapability::Video,
            MachineCapability::Audio,
            MachineCapability::Debug,
        ],
        media_kinds: vec![MediaKind::Cartridge],
        options: vec![
            MachineOptionInfo {
                required: true,
                ..option(
                    "rom",
                    MachineOptionKind::Text,
                    None,
                    "The cartridge: the game's zip set (P, S, C, M and V files, as FBNeo and MAME sets are), a path or romm:<id>",
                )
            },
            MachineOptionInfo {
                required: true,
                ..option(
                    "rom_bios",
                    MachineOptionKind::File,
                    None,
                    "The system ROM set: neogeo.zip as a whole (write neogeo.zip#* for a file; --firmware auto finds it on RomM): the 68000 BIOS files, sm1.sm1, sfix.sfix and 000-lo.lo",
                )
            },
            #[cfg(feature = "debug")]
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

/// The names and default host keys of the machine's inputs (arrows, Z X C V, Enter, Right Shift, 5 and 9). The host sends a key's
/// own name (`Digit5`, `KeyZ`, `ArrowUp`, `ShiftRight`) when nothing is bound, so those are aliases too.
const KEYS: [(&str, &[&str]); 22] = [
    ("Joy1Up", &["up", "arrowup"]),
    ("Joy1Down", &["down", "arrowdown"]),
    ("Joy1Left", &["left", "arrowleft"]),
    ("Joy1Right", &["right", "arrowright"]),
    ("Joy1A", &["z", "keyz"]),
    ("Joy1B", &["x", "keyx"]),
    ("Joy1C", &["c", "keyc"]),
    ("Joy1D", &["v", "keyv"]),
    ("Start1", &["enter", "return"]),
    ("Select1", &["shift_right", "shiftright"]),
    ("Coin1", &["5", "digit5"]),
    ("Coin2", &["6", "digit6"]),
    ("Service", &["9", "digit9"]),
    ("Joy2Up", &[]),
    ("Joy2Down", &[]),
    ("Joy2Left", &[]),
    ("Joy2Right", &[]),
    ("Joy2A", &[]),
    ("Joy2B", &[]),
    ("Joy2C", &[]),
    ("Joy2D", &[]),
    ("Start2", &[]),
];

fn key_names() -> Vec<MachineKeyInfo> {
    let mut keys: Vec<MachineKeyInfo> = KEYS
        .iter()
        .map(|(name, aliases)| MachineKeyInfo::with_aliases(name, aliases))
        .collect();
    keys.push(MachineKeyInfo::new("Select2"));
    keys
}

fn metadata() -> MachineMetadata {
    MachineMetadata {
        machine_id: MACHINE_ID.to_string(),
        region: Some("ntsc".to_string()),
        frame_rate: (MASTER_HZ as u32, FRAME_CLOCKS as u32),
        audio_channels: 2,
        audio_sample_rate: AUDIO_RATE,
        ..MachineMetadata::default()
    }
}

/// The most a game set or the system set may be when read.
const MAX_GAME: u64 = 256 << 20;
const MAX_BIOS: u64 = 16 << 20;

fn read_set(option: &str, path: &str, limit: u64) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("option \"{option}\": {path}: {e}"))?;
    if meta.len() > limit {
        return Err(format!(
            "option \"{option}\": {path} is {} bytes; at most {limit} are read",
            meta.len()
        ));
    }
    std::fs::read(path).map_err(|e| format!("option \"{option}\": {path}: {e}"))
}

fn create_machine(req: MachineCreateRequest) -> Result<MachineCreateResponse, String> {
    if req.machine_id != MACHINE_ID {
        return Err(format!(
            "unsupported machine id \"{}\" (plugin supports neogeo)",
            req.machine_id
        ));
    }
    #[allow(unused_mut)]
    let mut diag_file: Option<std::path::PathBuf> = None;
    let mut game = None;
    for option in &req.options {
        match (option.key.as_str(), &option.value) {
            #[cfg(feature = "debug")]
            ("diag_file", DebugValue::Text(v)) => diag_file = Some(std::path::PathBuf::from(v)),
            ("rom", DebugValue::Text(v)) => game = Some(read_set("rom", v, MAX_GAME)?),
            ("execution_mode", _) => {}
            (key, _) => {
                return Err(format!(
                    "unsupported machine-create option \"{key}\" for {MACHINE_ID}"
                ));
            }
        }
    }
    let game = game.ok_or_else(|| "missing required option: rom".to_string())?;
    let bios = req
        .roms
        .iter()
        .find(|blob| blob.key.eq_ignore_ascii_case("rom_bios"))
        .map(|blob| blob.bytes.as_slice())
        .ok_or_else(|| "missing required binary blob: rom_bios".to_string())?;
    if bios.len() as u64 > MAX_BIOS {
        return Err(format!(
            "option \"rom_bios\": {} bytes is more than the {MAX_BIOS} read",
            bios.len()
        ));
    }
    let machine = NeoGeo::from_sets(bios, &game)?;
    let sets = (bios.to_vec(), game);
    let mut state = state()
        .lock()
        .map_err(|_| "plugin state mutex poisoned".to_string())?;
    let instance = state.allocate_instance();
    state.instances.insert(
        instance,
        InstanceRecord {
            machine,
            sets,
            breakpoints: Vec::new(),
            diag_file,
            reported: false,
        },
    );
    Ok(MachineCreateResponse {
        instance,
        metadata: metadata(),
    })
}

fn register_snapshot(machine: &NeoGeo) -> RegisterSnapshot {
    const D: [&str; 8] = ["D0", "D1", "D2", "D3", "D4", "D5", "D6", "D7"];
    const A: [&str; 7] = ["A0", "A1", "A2", "A3", "A4", "A5", "A6"];
    let c = &machine.cpu;
    let field = |name: &str, value: u64, bits: u16| RegisterField {
        bank: "main".to_string(),
        name: name.to_string(),
        value,
        width_bits: bits,
    };
    let mut registers: Vec<RegisterField> = D
        .iter()
        .zip(c.d)
        .map(|(n, v)| field(n, u64::from(v), 32))
        .collect();
    registers.extend(A.iter().zip(c.a).map(|(n, v)| field(n, u64::from(v), 32)));
    registers.push(field("A7", u64::from(c.a7()), 32));
    registers.push(field("PC", u64::from(c.pc), 24));
    registers.push(field("SR", u64::from(c.sr()), 16));
    RegisterSnapshot {
        cpu_family: CpuFamily::M68k,
        registers,
        flags: Vec::new(),
    }
}

fn read_memory(machine: &NeoGeo, address: u64) -> u8 {
    machine.bus.peek(address as u32)
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
                .map(|i| read_memory(&record.machine, address.wrapping_add(i)))
                .collect(),
        )),
        DebugCommand::GetMachineMetadata => Ok(DebugResponse::MachineMetadata(metadata())),
        _ => {
            Err("unsupported: this debug command is not implemented for the neogeo yet".to_string())
        }
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
            // the reset button: the machine starts again from its BIOS; the battery-backed RAM keeps what it held
            let mut fresh = NeoGeo::from_sets(&record.sets.0, &record.sets.1)?;
            fresh.bus.backup = std::mem::take(&mut record.machine.bus.backup);
            record.machine = fresh;
            record.reported = false;
            Ok(MachineResponse::Ack)
        }
        MachineCommand::Step => {
            let cycles = record.machine.step() as u32;
            record.after_step();
            Ok(MachineResponse::Step(plugin_api::StepResult {
                cycles,
                pc: u64::from(record.machine.cpu.pc),
                halted: record.machine.bus.stop.is_some(),
            }))
        }
        MachineCommand::IsHalted => Ok(MachineResponse::Halted(record.machine.bus.stop.is_some())),
        MachineCommand::GetPc => Ok(MachineResponse::Pc(u64::from(record.machine.cpu.pc))),
        MachineCommand::ReadMemory { address, len, .. } => Ok(MachineResponse::Memory(
            (0..u64::from(len))
                .map(|i| read_memory(&record.machine, address.wrapping_add(i)))
                .collect(),
        )),
        MachineCommand::KeyEvent { key, pressed } => {
            // the host sends a host key's own name ("5", "z", "up") when nothing is bound: turn an alias into the key's name
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
        MachineCommand::RenderFrame => {
            let (width, height, rgb) = record.machine.render_rgb();
            Ok(MachineResponse::Frame { width, height, rgb })
        }
        MachineCommand::DrainAudio => Ok(MachineResponse::AudioPcm {
            channels: 2,
            sample_rate: AUDIO_RATE,
            samples: record.machine.drain_audio(),
        }),
        MachineCommand::Debug(command) => Ok(MachineResponse::Debug(handle_debug_command(
            record, command,
        )?)),
        _ => Err("unsupported: this command is not implemented for the neogeo yet".to_string()),
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
    plugin_api::guard::guarded("step", 4, || plugin_step_inner(instance))
}

fn plugin_step_inner(instance: MachineInstanceHandle) -> u32 {
    let Ok(mut state) = state().lock() else {
        return 0;
    };
    match state.instances.get_mut(&instance) {
        Some(record) => {
            let cycles = record.machine.step() as u32;
            record.after_step();
            cycles
        }
        None => 0,
    }
}

fn plugin_halted_v1(instance: MachineInstanceHandle) -> u8 {
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
            .map_or(1, |r| u8::from(r.machine.bus.stop.is_some()))
    })
}

fn plugin_pc_wide_v1(instance: MachineInstanceHandle) -> u64 {
    plugin_api::guard::guarded("pc_wide", 0, || plugin_pc_inner(instance))
}

fn plugin_pc_inner(instance: MachineInstanceHandle) -> u64 {
    let Ok(state) = state().lock() else {
        return 0;
    };
    state
        .instances
        .get(&instance)
        .map_or(0, |r| u64::from(r.machine.cpu.pc))
}

fn plugin_halted(instance: MachineInstanceHandle) -> bool {
    plugin_halted_v1(instance) != 0
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
    fn the_profile_names_the_machine_and_two_cpus() {
        let p = machine_profile();
        assert_eq!(p.machine_id, "neogeo");
        assert_eq!(p.cpus.len(), 2);
        assert!(p.options.iter().any(|o| o.key == "rom_bios" && o.required));
    }

    #[test]
    fn the_host_key_names_resolve_to_machine_keys() {
        // with no binding the host sends a key's own name; every default alias must find its key
        let names = key_names();
        for (host, key) in [
            ("5", "Coin1"),
            ("z", "Joy1A"),
            ("up", "Joy1Up"),
            ("enter", "Start1"),
            ("shift_right", "Select1"),
            ("Digit5", "Coin1"),
            ("Digit6", "Coin2"),
            ("Digit9", "Service"),
            ("KeyZ", "Joy1A"),
            ("KeyV", "Joy1D"),
            ("ArrowUp", "Joy1Up"),
            ("ArrowRight", "Joy1Right"),
            ("Enter", "Start1"),
            ("ShiftRight", "Select1"),
        ] {
            let found = names.iter().find(|i| i.matches(host)).map(|i| i.name.as_str());
            assert_eq!(found, Some(key), "{host}");
        }
    }

    #[test]
    fn a_missing_set_or_bios_is_refused() {
        let request = MachineCreateRequest {
            machine_id: MACHINE_ID.to_string(),
            region: None,
            roms: Vec::new(),
            options: Vec::new(),
        };
        assert!(create_machine(request).is_err());
    }
}
