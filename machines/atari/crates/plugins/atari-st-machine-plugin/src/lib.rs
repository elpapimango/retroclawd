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

use atari_st::{Accuracy, AtariSt, Config, Monitor, RamSize, Region};
use machine::{AUDIO_SAMPLE_RATE, Machine};
use plugin_api::{
    ABI_MAJOR_V1, ABI_MINOR_V1, AddressSpaceInfo, BinaryBlob, CodecError, CpuFamily, CpuInfo,
    DebugCommand, DebugResponse, DebugValue, DisassemblyLine, MACHINE_PROFILE_CONTRACT_VERSION,
    MachineCapability, MachineCommand, MachineCreateRequest, MachineCreateResponse,
    MachineInstanceCallRequest, MachineInstanceCallResponse, MachineInstanceHandle,
    MachineMetadata, MachineOptionInfo, MachineOptionKind, MachinePluginVTableV1, MachineProfile,
    MachineResponse, MediaSnapshot, PluginDescriptorV1, PluginKind,
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

fn machine_profile() -> MachineProfile {
    let text = |s: &str| Some(DebugValue::Text(s.to_string()));
    MachineProfile {
        machine_id: MACHINE_ID.to_string(),
        display_name: "Atari ST / STF / STFM".to_string(),
        cpu_family: CpuFamily::M68k,
        default_region: Some("pal".to_string()),
        contract_version: MACHINE_PROFILE_CONTRACT_VERSION,
        debug_capabilities: REQUIRED_MACHINE_DEBUG_CAPABILITIES.to_vec(),
        capabilities: vec![MachineCapability::Video, MachineCapability::Debug],
        media_kinds: Vec::new(),
        options: vec![
            MachineOptionInfo {
                required: true,
                ..option(
                    "rom",
                    MachineOptionKind::File,
                    None,
                    "192 KB TOS image (EmuTOS 1.4 etos192*.img via --firmware auto, or your own TOS 1.0x)",
                )
            },
            option("ram", text_choices(&RAM_CHOICES), text("1m"), "RAM fitted"),
            option(
                "monitor",
                text_choices(&["color", "mono"]),
                text("color"),
                "Monitor: color (mono is refused until its frame timing is sourced)",
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
                "trace_file",
                MachineOptionKind::Text,
                None,
                "Write the PC of every instruction to this file",
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
        ],
        key_names: Vec::new(),
        cpus: vec![CpuInfo {
            id: "main".to_string(),
            name: "MC68000".to_string(),
            family: CpuFamily::M68k,
            address_space: "main".to_string(),
        }],
        axis_names: Vec::new(),
    }
}

fn blob<'a>(roms: &'a [BinaryBlob], key: &str) -> Option<&'a [u8]> {
    roms.iter()
        .find(|blob| blob.key.eq_ignore_ascii_case(key))
        .map(|blob| blob.bytes.as_slice())
}

fn metadata(machine: &AtariSt, region: &str, model: &str) -> MachineMetadata {
    let rate = machine.frame_rate();
    MachineMetadata {
        machine_id: MACHINE_ID.to_string(),
        model: Some(model.to_string()),
        region: Some(region.to_string()),
        notes: vec![
            "slice 1: CPU, memory map and VBL; stops at the first unemulated device".to_string(),
        ],
        frame_rate: (rate.num, rate.den),
        ..MachineMetadata::default()
    }
}

fn create_machine(req: MachineCreateRequest) -> Result<MachineCreateResponse, String> {
    if !req.machine_id.eq_ignore_ascii_case(MACHINE_ID) {
        return Err(format!(
            "unsupported machine id \"{}\" (plugin supports only {MACHINE_ID})",
            req.machine_id
        ));
    }
    let rom =
        blob(&req.roms, "rom").ok_or_else(|| "missing required binary blob: rom".to_string())?;
    let mut ram = RamSize::M1;
    let mut accuracy = Accuracy::Fast;
    let mut monitor = Monitor::Colour;
    let mut model = "stfm".to_string();
    let mut trace_file = None;
    let mut trace_limit = DEFAULT_TRACE_LIMIT;
    let mut diag_file = None;
    let mut io_trace_file = None;
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
            ("diag_file", DebugValue::Text(v)) => diag_file = Some(PathBuf::from(v)),
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
    let machine = AtariSt::new(Config {
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
    })?;
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
        },
    );
    Ok(MachineCreateResponse { instance, metadata })
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
            cartridge: None,
        })),
        DebugCommand::GetChipSnapshots => Ok(DebugResponse::ChipSnapshots(Vec::new())),
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
        MachineCommand::SaveState | MachineCommand::LoadState { .. } => {
            Err("unsupported: atari-st save states arrive in Phase 2".to_string())
        }
        MachineCommand::SaveSnapshot { .. } => {
            Err("unsupported: the atari-st has no snapshot format".to_string())
        }
        MachineCommand::AttachMedia { .. }
        | MachineCommand::DetachMedia { .. }
        | MachineCommand::ExportDisk { .. }
        | MachineCommand::MarkDiskSaved { .. }
        | MachineCommand::ExportTape { .. }
        | MachineCommand::Freeze
        | MachineCommand::ExportCartridge
        | MachineCommand::ExportPrinter => {
            Err("unsupported: atari-st media arrive in later slices".to_string())
        }
        MachineCommand::GetMediaSnapshot => Ok(MachineResponse::MediaSnapshot(MediaSnapshot {
            tape: None,
            disks: Vec::new(),
            cartridge: None,
        })),
        MachineCommand::SetBorder { .. }
        | MachineCommand::KeyEvent { .. }
        | MachineCommand::MouseMotion { .. }
        | MachineCommand::MouseButton { .. }
        | MachineCommand::MouseWheel { .. } => Ok(MachineResponse::Ack),
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
        MachineCommand::DrainAudio => Ok(MachineResponse::AudioPcm {
            channels: 1,
            sample_rate: AUDIO_SAMPLE_RATE,
            samples: Vec::new(),
        }),
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
        // MOVE.B $FF8800,D0
        let id = create(
            &[0x1039, 0x00FF, 0x8800],
            vec![text("diag_file", diag.to_str().unwrap())],
        );
        for _ in 0..1000 {
            plugin_step_inner(id);
        }
        assert_eq!(plugin_halted_v1(id), 1);
        let report = std::fs::read_to_string(&diag).expect("report written");
        assert!(report.contains("stop: read byte $FF8800"), "{report}");
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
}
