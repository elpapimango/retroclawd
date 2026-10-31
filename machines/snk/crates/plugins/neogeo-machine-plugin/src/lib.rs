//! The `neogeo` machine plugin: profile, options and commands over the `neogeo` machine crate. The C entry points follow
//! the other machine plugins: an instance registry behind a mutex, every exported function wrapped by
//! `plugin_api::guard`, and the raw fast path for `step`/`halted`/`pc`. The `unsafe` here is the C ABI boundary only;
//! the machine crate forbids it.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use neogeo::system::{FRAME_CLOCKS, NeoGeo};
use plugin_api::{
    ABI_MAJOR_V1, ABI_MINOR_V1, AddressSpaceInfo, CodecError, CpuFamily, CpuInfo, DebugCommand,
    DebugResponse, DebugValue, MACHINE_PROFILE_CONTRACT_VERSION, MachineCapability, MachineCommand,
    MachineCreateRequest, MachineCreateResponse, MachineInstanceCallRequest,
    MachineInstanceCallResponse, MachineInstanceHandle, MachineKeyInfo, MachineMetadata,
    MachineOptionInfo, MachineOptionKind, MachinePluginVTableV1, MachineProfile, MachineResponse,
    MediaKind, PluginDescriptorV1, PluginKind, REQUIRED_MACHINE_DEBUG_CAPABILITIES, RcBuffer,
    RcCallResult, RcSlice, RcStatus, RegisterField, RegisterSnapshot, StatusCode,
};

const MACHINE_ID: &str = "neogeo";
const PLUGIN_ID: &[u8] = b"retroclawd.neogeo.machine";
const PLUGIN_VERSION: &[u8] = build_info::version_tag!().as_bytes();
/// The master clock in Hz (fact N4).
const MASTER_HZ: u64 = 24_000_000;
/// The sound chip's sample rate (fact Y1).
const AUDIO_RATE: u32 = ym2610::SAMPLE_RATE;

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
    machine: NeoGeo,
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

/// The names and default host keys of the machine's inputs (arrows, Z X C V, Enter, Right Shift, 5 and 9).
const KEYS: [(&str, &[&str]); 22] = [
    ("Joy1Up", &["up"]),
    ("Joy1Down", &["down"]),
    ("Joy1Left", &["left"]),
    ("Joy1Right", &["right"]),
    ("Joy1A", &["z"]),
    ("Joy1B", &["x"]),
    ("Joy1C", &["c"]),
    ("Joy1D", &["v"]),
    ("Start1", &["enter", "return"]),
    ("Select1", &["shiftright"]),
    ("Coin1", &["5"]),
    ("Coin2", &["6"]),
    ("Service", &["9"]),
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
    let mut diag_file = None;
    let mut game = None;
    for option in &req.options {
        match (option.key.as_str(), &option.value) {
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
            if record.machine.set_key(&key, pressed) {
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
            let cycles = record.machine.step() as u32;
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
            .map_or(1, |r| u8::from(r.machine.bus.stop.is_some()))
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
    state
        .instances
        .get(&instance)
        .map_or(0, |r| u64::from(r.machine.cpu.pc))
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
    fn the_profile_names_the_machine_and_two_cpus() {
        let p = machine_profile();
        assert_eq!(p.machine_id, "neogeo");
        assert_eq!(p.cpus.len(), 2);
        assert!(p.options.iter().any(|o| o.key == "rom_bios" && o.required));
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
