//! A deliberately dull machine plugin for retroclawd's own unit tests.
//!
//! The host repository holds no machine code (every real machine lives in a
//! family repository and ships as a shared library), but the host's tests still
//! need *a* machine to drive: stepping, pausing, media attach/eject, state
//! export. `stub-machine` is that: 64K of RAM, a program counter that walks
//! through NOPs, a disk slot that accepts `.dsk` images, a tape slot that
//! accepts `.tap`, `.sna` snapshots and a state blob. It is linked only into
//! the host's test binary and is never built as a shared library.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use machine::AUDIO_SAMPLE_RATE;
use plugin_api::{
    ABI_MAJOR_V1, ABI_MINOR_V1, AddressSpaceInfo, BitmapExplorerDefaults, BitmapPaletteColor,
    ChipSnapshot, CodecError, CpuFamily, DebugCommand, DebugResponse, DisassemblyLine,
    MACHINE_PROFILE_CONTRACT_VERSION, MachineCapability, MachineCommand, MachineCreateRequest,
    MachineCreateResponse, MachineInstanceCallRequest, MachineInstanceCallResponse,
    MachineInstanceHandle, MachineMetadata, MachinePluginVTableV1, MachineProfile, MachineResponse,
    MediaKind, MediaSlot, MediaSnapshot, PluginDescriptorV1, PluginKind,
    REQUIRED_MACHINE_DEBUG_CAPABILITIES, RcBuffer, RcCallResult, RcSlice, RcStatus, RegisterField,
    RegisterSnapshot, StatusCode,
};

/// The one machine id this plugin offers.
pub const MACHINE_ID: &str = "stub-machine";
const PLUGIN_ID: &[u8] = b"retroclawd.test.machine";
const PLUGIN_VERSION: &[u8] = env!("CARGO_PKG_VERSION").as_bytes();
const STATE_MAGIC: &[u8; 4] = b"STUB";
const CLOCK_HZ: u64 = 1_000_000;
const FRAME_HZ: u64 = 50;

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

/// The "machine": NOPs forever, with a disk, a tape deck and 64K of memory.
struct StubMachine {
    pc: u16,
    ram: Vec<u8>,
    /// Drive A and drive B.
    disks: [Option<Vec<u8>>; 2],
    disk_modified: [bool; 2],
    tape: bool,
}

impl StubMachine {
    fn new() -> Self {
        StubMachine {
            pc: 0x0100,
            ram: vec![0; 0x10000],
            disks: [None, None],
            disk_modified: [false; 2],
            tape: false,
        }
    }

    fn step(&mut self) -> u32 {
        self.pc = self.pc.wrapping_add(1);
        4
    }

    fn save_state(&self) -> Vec<u8> {
        let mut out = STATE_MAGIC.to_vec();
        out.extend_from_slice(&self.pc.to_le_bytes());
        out
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() != 6 || &bytes[..4] != STATE_MAGIC {
            return Err("not a stub-machine state".to_string());
        }
        self.pc = u16::from_le_bytes([bytes[4], bytes[5]]);
        Ok(())
    }

    fn load(&mut self, address: u16, bytes: &[u8]) -> usize {
        let start = address as usize;
        let n = bytes.len().min(self.ram.len() - start);
        self.ram[start..start + n].copy_from_slice(&bytes[..n]);
        n
    }

    fn read(&self, address: u16, len: u16) -> Vec<u8> {
        (0..len)
            .map(|o| self.ram[address.wrapping_add(o) as usize])
            .collect()
    }
}

struct InstanceRecord {
    machine: StubMachine,
    paused: bool,
    breakpoints: Vec<u64>,
}

struct PluginState {
    next_instance: MachineInstanceHandle,
    instances: HashMap<MachineInstanceHandle, InstanceRecord>,
}

impl PluginState {
    fn new() -> Self {
        Self {
            next_instance: 1,
            instances: HashMap::new(),
        }
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

fn machine_profile() -> MachineProfile {
    MachineProfile {
        machine_id: MACHINE_ID.to_string(),
        display_name: "Stub machine (host unit tests)".to_string(),
        cpu_family: CpuFamily::Z80,
        default_region: Some("pal".to_string()),
        contract_version: MACHINE_PROFILE_CONTRACT_VERSION,
        debug_capabilities: REQUIRED_MACHINE_DEBUG_CAPABILITIES.to_vec(),
        capabilities: vec![
            MachineCapability::Snapshot,
            MachineCapability::SaveState,
            MachineCapability::Debug,
        ],
        media_kinds: vec![MediaKind::Disk, MediaKind::Tape, MediaKind::Snapshot],
        options: Vec::new(),
        key_names: Vec::new(),
        cpus: Vec::new(),
        axis_names: AXIS_NAMES.iter().map(|name| name.to_string()).collect(),
    }
}

/// The analogue axes the stub accepts (`AxisEvent` with any other name is an error).
const AXIS_NAMES: [&str; 2] = ["Joy1X", "Joy1Y"];

fn metadata() -> MachineMetadata {
    MachineMetadata {
        machine_id: MACHINE_ID.to_string(),
        model: Some(MACHINE_ID.to_string()),
        region: Some("pal".to_string()),
        notes: vec!["host unit-test stub".to_string()],
        bitmap_palette: vec![
            BitmapPaletteColor {
                name: "black".to_string(),
                rgb: [0, 0, 0],
            },
            BitmapPaletteColor {
                name: "white".to_string(),
                rgb: [255, 255, 255],
            },
        ],
        bitmap_defaults: Some(BitmapExplorerDefaults {
            address: 0,
            bytes_width: 32,
            bitplanes: 1,
            zoom: 3,
        }),
        ..MachineMetadata::default()
    }
}

fn create_machine(req: MachineCreateRequest) -> Result<MachineCreateResponse, String> {
    if !req.machine_id.eq_ignore_ascii_case(MACHINE_ID) {
        return Err(format!(
            "unsupported machine id \"{}\" (this plugin supports only {MACHINE_ID})",
            req.machine_id
        ));
    }
    let mut state = state()
        .lock()
        .map_err(|_| "plugin state mutex poisoned".to_string())?;
    let instance = state.next_instance;
    state.next_instance = state.next_instance.wrapping_add(1).max(1);
    state.instances.insert(
        instance,
        InstanceRecord {
            machine: StubMachine::new(),
            paused: false,
            breakpoints: Vec::new(),
        },
    );
    Ok(MachineCreateResponse {
        instance,
        metadata: metadata(),
    })
}

fn extension(name: Option<&str>) -> String {
    name.and_then(|n| n.rsplit('.').next())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// A key event with this name marks the disk in drive A / drive B as written to.
pub const DIRTY_DISK_A_KEY: &str = "dirty-disk-a";
pub const DIRTY_DISK_B_KEY: &str = "dirty-disk-b";

/// The stub has the two drives of a CPC or +3 interface.
fn stub_drive(drive: u8) -> Result<usize, String> {
    if drive < 2 {
        Ok(usize::from(drive))
    } else {
        Err(format!("drive {drive} does not exist (drives 0 and 1)"))
    }
}

/// A key event with this name panics inside the plugin.
pub const CRASH_KEY: &str = "crash-now";

fn handle_machine_command(
    record: &mut InstanceRecord,
    command: MachineCommand,
) -> Result<MachineResponse, String> {
    let m = &mut record.machine;
    match command {
        MachineCommand::GetClockHz => Ok(MachineResponse::ClockHz(CLOCK_HZ)),
        MachineCommand::GetFrameHz => Ok(MachineResponse::FrameHz(FRAME_HZ)),
        // Lets a test crash the plugin on demand (the host's crash handling is tested with it, in a process of its own).
        MachineCommand::KeyEvent { key, .. } if key == CRASH_KEY => {
            panic!("test plugin: crash requested")
        }
        // Lets a test play the guest writing to a disk.
        MachineCommand::KeyEvent { key, .. } if key == DIRTY_DISK_A_KEY => {
            m.disk_modified[0] = m.disks[0].is_some();
            Ok(MachineResponse::Ack)
        }
        MachineCommand::KeyEvent { key, .. } if key == DIRTY_DISK_B_KEY => {
            m.disk_modified[1] = m.disks[1].is_some();
            Ok(MachineResponse::Ack)
        }
        MachineCommand::KeyEvent { .. } => Ok(MachineResponse::Ack),
        MachineCommand::RequestInterrupt
        | MachineCommand::SetBorder { .. }
        | MachineCommand::MouseMotion { .. }
        | MachineCommand::MouseButton { .. }
        | MachineCommand::MouseWheel { .. }
        | MachineCommand::WriteIo { .. }
        | MachineCommand::SetPortWatches { .. } => Ok(MachineResponse::Ack),
        MachineCommand::Step => {
            let cycles = m.step();
            Ok(MachineResponse::Step(plugin_api::StepResult {
                cycles,
                pc: u64::from(m.pc),
                halted: false,
            }))
        }
        MachineCommand::IsHalted => Ok(MachineResponse::Halted(false)),
        MachineCommand::GetPc => Ok(MachineResponse::Pc(m.pc.into())),
        MachineCommand::Reset => {
            m.pc = 0x0100;
            Ok(MachineResponse::Ack)
        }
        MachineCommand::ReadMemory { address, len, .. } => Ok(MachineResponse::Memory(
            m.read(address.try_into().unwrap(), len.try_into().unwrap()),
        )),
        MachineCommand::WriteMemory { space: Some(_), .. } => {
            Err("unsupported: the stub machine has no banked memory".to_string())
        }
        MachineCommand::WriteMemory { address, bytes, .. }
        | MachineCommand::LoadBytes { address, bytes } => Ok(MachineResponse::BytesWritten(
            m.load(address.try_into().unwrap(), &bytes) as u32,
        )),
        MachineCommand::SetPc { address } => {
            m.pc = address as u16;
            Ok(MachineResponse::Ack)
        }
        MachineCommand::SaveState => Ok(MachineResponse::StateBlob(m.save_state())),
        MachineCommand::LoadState { bytes } => {
            m.load_state(&bytes)?;
            Ok(MachineResponse::Ack)
        }
        MachineCommand::SaveSnapshot { extension } => {
            if extension == "sna" {
                Ok(MachineResponse::SnapshotBlob(m.save_state()))
            } else {
                Err(format!(
                    "unsupported snapshot save extension \".{extension}\""
                ))
            }
        }
        MachineCommand::AttachMedia {
            slot,
            kind,
            name,
            bytes,
            ..
        } => {
            let ext = extension(name.as_deref());
            match kind {
                MediaKind::Disk => {
                    if ext == "nib" {
                        return Err(".nib images are not supported by the stub machine".to_string());
                    }
                    dsk::parse(&bytes).map_err(|e| format!("not a disk image: {e}"))?;
                    let drive = stub_drive(match slot {
                        MediaSlot::Disk { drive } => drive,
                        _ => 0,
                    })?;
                    m.disks[drive] = Some(bytes);
                    m.disk_modified[drive] = false;
                    Ok(MachineResponse::Ack)
                }
                MediaKind::Tape => {
                    if ext != "tap" {
                        return Err(format!("unsupported tape format \".{ext}\""));
                    }
                    m.tape = true;
                    Ok(MachineResponse::Ack)
                }
                MediaKind::Snapshot if ext == "sna" => {
                    m.load_state(&bytes)?;
                    Ok(MachineResponse::Ack)
                }
                other => Err(format!("unsupported media: {other:?} .{ext}")),
            }
        }
        MachineCommand::DetachMedia { slot } => match slot {
            MediaSlot::Disk { drive } => {
                let drive = stub_drive(drive)?;
                m.disks[drive] = None;
                m.disk_modified[drive] = false;
                Ok(MachineResponse::Ack)
            }
            MediaSlot::Tape => {
                m.tape = false;
                Ok(MachineResponse::Ack)
            }
            other => Err(format!("unsupported: nothing to detach in {other:?}")),
        },
        MachineCommand::ExportDisk {
            drive,
            only_if_modified,
        } => {
            let drive = stub_drive(drive)?;
            Ok(MachineResponse::DiskImage(match &m.disks[drive] {
                Some(image) if !only_if_modified || m.disk_modified[drive] => Some(image.clone()),
                _ => None,
            }))
        }
        MachineCommand::MarkDiskSaved { drive } => {
            m.disk_modified[stub_drive(drive)?] = false;
            Ok(MachineResponse::Ack)
        }
        MachineCommand::ExportTape { .. } => Ok(MachineResponse::TapeImage(None)),
        MachineCommand::Freeze
        | MachineCommand::ExportCartridge
        | MachineCommand::ExportPrinter => {
            Err("unsupported: the stub machine has no freezer, cartridge or printer".to_string())
        }
        MachineCommand::GetMediaSnapshot => Ok(MachineResponse::MediaSnapshot(MediaSnapshot {
            tape: None,
            disks: Vec::new(),
            cartridge: None,
        })),
        MachineCommand::TakePortHit => Ok(MachineResponse::PortHit(None)),
        MachineCommand::RenderFrame => {
            Err("unsupported: the stub machine has no video".to_string())
        }
        MachineCommand::AxisEvent { axis, .. } if AXIS_NAMES.contains(&axis.as_str()) => {
            Ok(MachineResponse::Ack)
        }
        MachineCommand::AxisEvent { axis, .. } => Err(format!("unknown axis \"{axis}\"")),
        MachineCommand::DrainAudio => Ok(MachineResponse::AudioPcm {
            channels: 1,
            sample_rate: AUDIO_SAMPLE_RATE,
            samples: Vec::new(),
        }),
        MachineCommand::Debug(command) => {
            handle_debug_command(record, command).map(MachineResponse::Debug)
        }
    }
}

fn handle_debug_command(
    record: &mut InstanceRecord,
    command: DebugCommand,
) -> Result<DebugResponse, String> {
    let m = &mut record.machine;
    match command {
        DebugCommand::Pause => {
            record.paused = true;
            Ok(DebugResponse::Ack)
        }
        DebugCommand::Resume => {
            record.paused = false;
            Ok(DebugResponse::Ack)
        }
        DebugCommand::StepInstruction | DebugCommand::StepOver => {
            m.step();
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
        DebugCommand::GetRegisters => Ok(DebugResponse::Registers(RegisterSnapshot {
            cpu_family: CpuFamily::Z80,
            registers: vec![RegisterField {
                bank: "main".to_string(),
                name: "PC".to_string(),
                value: m.pc as u64,
                width_bits: 16,
            }],
            flags: Vec::new(),
        })),
        DebugCommand::SetRegister { name, value } => {
            if name.eq_ignore_ascii_case("pc") {
                m.pc = value as u16;
                Ok(DebugResponse::Ack)
            } else {
                Err(format!("unknown register {name}"))
            }
        }
        DebugCommand::GetAddressSpaces => {
            Ok(DebugResponse::AddressSpaces(vec![AddressSpaceInfo {
                name: "main".to_string(),
                addr_width_bits: 16,
                bank_context: Vec::new(),
            }]))
        }
        DebugCommand::ReadMemory { address, len, .. } => Ok(DebugResponse::Memory(
            m.read(address.try_into().unwrap(), len.try_into().unwrap()),
        )),
        DebugCommand::WriteMemory { address, bytes, .. } => {
            m.load(address.try_into().unwrap(), &bytes);
            Ok(DebugResponse::Ack)
        }
        DebugCommand::GetDisassembly { start, lines } => {
            let mut cursor = start.unwrap_or(m.pc.into());
            let mut out = Vec::with_capacity(lines as usize);
            for _ in 0..lines {
                out.push(DisassemblyLine {
                    address: cursor,
                    bytes: vec![m.ram[cursor as usize]],
                    text: "NOP".to_string(),
                });
                cursor = cursor.wrapping_add(1);
            }
            Ok(DebugResponse::Disassembly(out))
        }
        DebugCommand::GetMediaSnapshot => Ok(DebugResponse::MediaSnapshot(MediaSnapshot {
            tape: None,
            disks: Vec::new(),
            cartridge: None,
        })),
        DebugCommand::GetChipSnapshots => {
            Ok(DebugResponse::ChipSnapshots(Vec::<ChipSnapshot>::new()))
        }
        DebugCommand::OnCpu { cpu, command } => {
            if cpu == "main" {
                handle_debug_command(record, *command)
            } else {
                Err(format!(
                    "unknown cpu \"{cpu}\" (this machine has only \"main\")"
                ))
            }
        }
        DebugCommand::GetMachineMetadata => Ok(DebugResponse::MachineMetadata(metadata())),
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
    plugin_api::guard::guarded_call("list_profiles", || {
        let request: plugin_api::ListProfilesRequest = match decode_request(req) {
            Ok(req) => req,
            Err(e) => return e.into_call_result(),
        };
        let _ = request.include_experimental;
        encode_ok(&plugin_api::ListProfilesResponse {
            profiles: vec![machine_profile()],
        })
    })
}

unsafe extern "C" fn plugin_create_instance_v1(req: RcSlice) -> RcCallResult {
    plugin_api::guard::guarded_call("create_instance", || {
        let request: MachineCreateRequest = match decode_request(req) {
            Ok(req) => req,
            Err(e) => return e.into_call_result(),
        };
        match create_machine(request) {
            Ok(response) => encode_ok(&response),
            Err(error) => RcCallResult::error_with_message(status_from_error(&error), &error),
        }
    })
}

unsafe extern "C" fn plugin_destroy_instance_v1(instance: MachineInstanceHandle) -> RcStatus {
    plugin_api::guard::guarded(
        "destroy_instance",
        RcStatus::from_code(StatusCode::InternalError),
        || {
            let Ok(mut state) = state().lock() else {
                return RcStatus::from_code(StatusCode::InternalError);
            };
            if state.instances.remove(&instance).is_some() {
                RcStatus::ok()
            } else {
                RcStatus::from_code(StatusCode::NotFound)
            }
        },
    )
}

unsafe extern "C" fn plugin_instance_call_v1(
    instance: MachineInstanceHandle,
    req: RcSlice,
) -> RcCallResult {
    plugin_api::guard::guarded_call("instance_call", || {
        let request: MachineInstanceCallRequest = match decode_request(req) {
            Ok(req) => req,
            Err(e) => return e.into_call_result(),
        };
        match handle_instance_call(instance, request) {
            Ok(response) => encode_ok(&response),
            Err(error) => RcCallResult::error_with_message(status_from_error(&error), &error),
        }
    })
}

extern "C" fn plugin_step_v1(instance: MachineInstanceHandle) -> u32 {
    if plugin_api::guard::has_crashed() {
        return 4;
    }
    plugin_api::guard::guarded("step", 4, || {
        let Ok(mut state) = state().lock() else {
            return 0;
        };
        state
            .instances
            .get_mut(&instance)
            .map_or(0, |record| record.machine.step())
    })
}

extern "C" fn plugin_halted_v1(_instance: MachineInstanceHandle) -> u8 {
    u8::from(plugin_api::guard::has_crashed())
}

extern "C" fn plugin_pc_v1(instance: MachineInstanceHandle) -> u16 {
    plugin_api::guard::guarded("pc", 0, || {
        let Ok(state) = state().lock() else {
            return 0;
        };
        state
            .instances
            .get(&instance)
            .map_or(0, |record| record.machine.pc)
    })
}

extern "C" fn plugin_pc_wide_v1(instance: MachineInstanceHandle) -> u64 {
    u64::from(plugin_pc_v1(instance))
}

/// # Safety
///
/// Returns immutable descriptor storage owned by this plugin.
pub unsafe extern "C" fn entry_v1() -> *const PluginDescriptorV1 {
    &PLUGIN_DESCRIPTOR_V1.0
}
