//! The `genesis` machine plugin: profile, options and commands over the `genesis` machine crate. The C entry
//! points follow the other machine plugins: an instance registry behind a mutex, every exported function wrapped by
//! `plugin_api::guard`, and the raw fast path for `step`/`halted`/`pc`. The `unsafe` here is the C ABI boundary only;
//! the machine crate forbids it.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use genesis::Genesis;
use machine::Machine;
use plugin_api::{
    ABI_MAJOR_V1, ABI_MINOR_V1, AddressSpaceInfo, CodecError, CpuFamily, CpuInfo, DebugCommand,
    DebugResponse, DebugValue, DisassemblyLine, MACHINE_PROFILE_CONTRACT_VERSION,
    MachineCapability, MachineCommand, MachineCreateRequest, MachineCreateResponse,
    MachineInstanceCallRequest, MachineInstanceCallResponse, MachineInstanceHandle,
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
        options: vec![option(
            "diag_file",
            MachineOptionKind::Text,
            None,
            "Write the diagnostic report of a stop at an unemulated device here (default: stderr)",
        )],
        key_names: Vec::new(),
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

fn metadata(machine: &Genesis) -> MachineMetadata {
    let rate = machine.frame_rate();
    MachineMetadata {
        machine_id: MACHINE_ID.to_string(),
        region: Some("ntsc".to_string()),
        frame_rate: (rate.num, rate.den),
        audio_channels: machine.audio_format().channels,
        audio_sample_rate: machine.audio_format().sample_rate,
        ..MachineMetadata::default()
    }
}

fn create_machine(req: MachineCreateRequest) -> Result<MachineCreateResponse, String> {
    if req.machine_id != MACHINE_ID {
        return Err(format!(
            "unsupported machine id \"{}\" (plugin supports genesis)",
            req.machine_id
        ));
    }
    if let Some(option) = req.options.first() {
        return Err(format!(
            "unsupported machine-create option \"{}\" for {MACHINE_ID}",
            option.key
        ));
    }
    let machine = Genesis::new();
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
        },
    );
    Ok(MachineCreateResponse { instance, metadata })
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
        DebugCommand::GetDisassembly { .. } => {
            Ok(DebugResponse::Disassembly(Vec::<DisassemblyLine>::new()))
        }
        DebugCommand::OnCpu { cpu, command } => {
            if cpu == "main" || cpu == "z80" {
                handle_debug_command(record, *command)
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
    state
        .instances
        .get_mut(&instance)
        .map_or(0, |record| record.machine.step())
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

    #[test]
    fn an_instance_can_be_created_stepped_and_destroyed() {
        let created = create_machine(MachineCreateRequest {
            machine_id: MACHINE_ID.to_string(),
            region: None,
            roms: Vec::new(),
            options: Vec::new(),
        })
        .unwrap();
        assert_eq!(created.metadata.frame_rate, (53_693_175, 896_040));
        assert_eq!(plugin_step_inner(created.instance), 1);
        assert_eq!(plugin_halted_v1(created.instance), 0);
        // SAFETY: the handle was just created by this plugin.
        unsafe { plugin_destroy_instance_inner(created.instance) };
        assert_eq!(plugin_step_inner(created.instance), 0);
    }

    #[test]
    fn an_unknown_option_is_refused() {
        let result = create_machine(MachineCreateRequest {
            machine_id: MACHINE_ID.to_string(),
            region: None,
            roms: Vec::new(),
            options: vec![plugin_api::NamedValue {
                key: "bogus".to_string(),
                value: DebugValue::Bool(true),
            }],
        });
        assert!(result.is_err());
    }
}
