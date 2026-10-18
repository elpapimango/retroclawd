use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use machine::{AUDIO_SAMPLE_RATE, Machine};
use plugin_api::{
    ABI_MAJOR_V1, ABI_MINOR_V1, BinaryBlob, ChipSnapshot, CpuFamily, DebugCommand, DebugResponse,
    DebugValue, HostMouseButton, ListProfilesRequest, ListProfilesResponse,
    MACHINE_PROFILE_CONTRACT_VERSION, MachineCapability, MachineCommand, MachineCreateRequest,
    MachineCreateResponse, MachineDebugCapability, MachineInstanceCallRequest,
    MachineInstanceCallResponse, MachineInstanceHandle, MachineMetadata, MachinePluginVTableV1,
    MachineProfile, MachineResponse, MediaKind, MediaSlot, NamedValue, PluginDescriptorV1,
    PluginEntryV1, PluginKind, PortHitInfo, PortWatchSpec, REQUIRED_MACHINE_DEBUG_CAPABILITIES,
    RcBuffer, RcCallResult, RcSlice, decode_message, encode_message,
};

const MACHINE_PLUGIN_ENV_VAR: &str = "RETROCLAWD_MACHINE_PLUGIN";
const MACHINE_PLUGINS_ENV_VAR: &str = "RETROCLAWD_MACHINE_PLUGINS";
const MACHINE_PLUGIN_DIRS_ENV_VAR: &str = "RETROCLAWD_MACHINE_PLUGIN_DIRS";
const MACHINE_PLUGIN_ENTRY_SYMBOLS: [&[u8]; 2] =
    [b"retroclawd_plugin_entry_v1\0", b"plugin_entry_v1\0"];

static REGISTER_LABELS: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
static RUNTIME_CATALOG: OnceLock<Result<RuntimeCatalog, String>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TapeDebugSnapshot {
    pub(crate) machine: String,
    pub(crate) attached: bool,
    pub(crate) motor_on: Option<bool>,
    pub(crate) position: usize,
    pub(crate) events: usize,
    pub(crate) playing: bool,
    pub(crate) finished: bool,
    pub(crate) speed_multiplier: u32,
    pub(crate) ear_bit: Option<bool>,
    pub(crate) cycles_until_next_edge: Option<u64>,
    pub(crate) spin_down_cycles: Option<u32>,
}

/// Machine-agnostic overlay telemetry for the on-screen `--show-overlay`
/// GUI panel -- populated from the generic `chip_id: "overlay"`
/// `ChipSnapshot` every conformant machine plugin may expose (see each
/// `*-machine-plugin`'s own `generic_overlay_chip_snapshot`). Every field is
/// `None`/empty rather than a placeholder when a machine/model has
/// nothing to report -- e.g. `zx81` never emits this chip at all, `c64`
/// only ever sets `memory_summary`. The `--debug` diagnostic log and
/// `get-paging-state` (ZRCP) instead read the same `"overlay"` chip's raw
/// `fields`/`blobs` directly via `overlay_diagnostic_snapshot` below --
/// no fixed struct shape, so a machine that adds a new overlay field (CPC's
/// richer PPI/keyboard/PSG/tape/disk counters included) needs no host
/// change to have it show up there too.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct OverlayDebugSnapshot {
    pub(crate) memory_summary: Option<String>,
    pub(crate) fdc_phase: Option<String>,
    pub(crate) disk_motor: Option<bool>,
    pub(crate) tape_motor: Option<bool>,
    /// Tape playback position, in real `zx_tape::Tape` events (not
    /// bytes), and the tape's total event count -- lets the tape overlay
    /// panel show real progress (`pos/total`) and, more importantly for
    /// stall diagnosis, whether it's advancing at all between refreshes.
    pub(crate) tape_position: Option<u64>,
    pub(crate) tape_total: Option<u64>,
    /// T-states remaining until the EAR line's next level change -- a
    /// second, finer-grained stall signal than `tape_position` (position
    /// only advances once per whole tape *event*, this counts down every
    /// tick within one).
    pub(crate) tape_edge_cycles: Option<u64>,
    pub(crate) tape_waveform: Vec<u8>,
    /// Recent completed EAR-line pulse widths (T-states), oldest first --
    /// the actual "is this tape decoding real bits" signal: compare
    /// against the standard ROM protocol's known-good widths (pilot
    /// ~2168T, sync ~667/735T, bit0 ~855T, bit1 ~1710T, all cited in the
    /// producing crates' own doc comments).
    pub(crate) tape_pulse_widths: Vec<u16>,
}

#[derive(Debug, Clone)]
pub(crate) struct StartupPluginReport {
    pub(crate) plugin_id: String,
    pub(crate) plugin_version: String,
    /// The `(major, minor)` plugin ABI the library was built against.
    pub(crate) abi: (u16, u16),
    pub(crate) source: String,
    pub(crate) machine_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct StartupReport {
    pub(crate) plugins: Vec<StartupPluginReport>,
    pub(crate) profiles: Vec<MachineProfile>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Clone)]
struct HostedBackend {
    plugin: Arc<PluginV1MachineBackend>,
}

/// What to do with a modified disk image when the machine goes away.
///
/// Guest writes always land in the emulated medium; they only reach the
/// host filesystem through this policy, so a session can never overwrite a
/// user's disk file unless they asked for exactly that
/// (`--disk-save`/`--disk-writeback`).
#[derive(Debug, Clone, Default)]
pub(crate) struct DiskExitPolicy {
    /// Where to write the modified image; `None` means don't write, warn.
    pub(crate) save_to: Option<PathBuf>,
}

/// Where to write the cassette recording when the machine goes away (set by
/// `--tape-save`). The format is the file's extension: `cdt`/`tzx`/`voc` (CPC) or `tap` (C64).
#[derive(Debug, Clone)]
pub(crate) struct TapeExitPolicy {
    pub(crate) save_to: PathBuf,
    pub(crate) format: String,
}

/// Where to write a programmed EasyFlash cartridge (`--cartridge-save`) when
/// the machine goes away; `None` target means warn instead.
#[derive(Debug, Clone, Default)]
pub(crate) struct CartridgeExitPolicy {
    pub(crate) save_to: Option<PathBuf>,
}

pub(crate) struct HostedMachine {
    backend: HostedBackend,
    instance: MachineInstanceHandle,
    machine_id: String,
    metadata: MachineMetadata,
    /// Per drive (A, B): set once a disk is attached; acted on when the machine is dropped
    /// (end of a headless run, or when the GUI's emulation thread ends).
    /// Never cloned: a run-ahead clone must not write the user's file.
    disk_exit: [Option<DiskExitPolicy>; 2],
    /// Like `disk_exit`, for a cassette recording; never cloned either.
    tape_exit: Option<TapeExitPolicy>,
    /// Like `disk_exit`, for a flashed cartridge; never cloned either.
    cartridge_exit: Option<CartridgeExitPolicy>,
    /// Where the serial-bus printer's text is written at exit.
    printer_out: Option<PathBuf>,
}

impl Clone for HostedMachine {
    fn clone(&self) -> Self {
        let instance = self
            .backend
            .clone_instance(self.instance)
            .expect("cloning hosted machine instance should succeed");
        Self {
            backend: self.backend.clone(),
            instance,
            machine_id: self.machine_id.clone(),
            metadata: self.metadata.clone(),
            disk_exit: [None, None],
            tape_exit: None,
            cartridge_exit: None,
            printer_out: None,
        }
    }
}

impl Drop for HostedMachine {
    fn drop(&mut self) {
        self.finish_disk_on_exit();
        self.finish_tape_on_exit();
        self.finish_cartridge_on_exit();
        self.finish_printer_on_exit();
        let _ = self.backend.destroy_instance(self.instance);
    }
}

/// Writes `bytes` to `path` via a sibling temp file and a rename, so an
/// interrupted write can never leave a truncated disk image behind.
pub(crate) fn write_file_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// The audio queue and the output stage are mono until a stereo Machine
/// exists; interleaved frames of a wider response are averaged into one sample.
fn downmix_to_mono(channels: u8, samples: Vec<f32>) -> Vec<f32> {
    if channels <= 1 {
        return samples;
    }
    let width = usize::from(channels);
    samples
        .chunks_exact(width)
        .map(|frame| frame.iter().sum::<f32>() / f32::from(channels))
        .collect()
}

fn intern_register_label(label: &str) -> &'static str {
    let labels = REGISTER_LABELS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut labels = labels.lock().unwrap();
    if let Some(existing) = labels.get(label) {
        return existing;
    }
    let leaked: &'static str = Box::leak(label.to_string().into_boxed_str());
    labels.insert(leaked.to_string(), leaked);
    leaked
}

fn status_label(code: u32) -> &'static str {
    match code {
        0x0000_0000 => "ok",
        0x0000_0001 => "unknown",
        0x0000_0002 => "invalid request",
        0x0000_0003 => "unsupported",
        0x0000_0004 => "not found",
        0x0000_0005 => "io",
        0x0000_0006 => "decode",
        0x0000_0007 => "encode",
        0x0000_0008 => "abi mismatch",
        0x0000_0009 => "internal",
        _ => "unknown",
    }
}

fn copy_and_free_payload(vtable: &MachinePluginVTableV1, payload: RcBuffer) -> Vec<u8> {
    let bytes = if payload.ptr.is_null() || payload.len == 0 {
        Vec::new()
    } else {
        // SAFETY: plugin returned a buffer valid for len bytes until free_buffer.
        unsafe { std::slice::from_raw_parts(payload.ptr, payload.len) }.to_vec()
    };
    // SAFETY: plugin contract says free_buffer owns/frees this payload.
    unsafe { (vtable.free_buffer)(payload) };
    bytes
}

fn decode_call_payload(
    vtable: &MachinePluginVTableV1,
    result: RcCallResult,
) -> Result<Vec<u8>, String> {
    let status = result.status;
    if !status.is_ok() {
        // A plugin may explain the failure as UTF-8 text in the payload.
        let message = String::from_utf8_lossy(&copy_and_free_payload(vtable, result.payload))
            .trim()
            .to_string();
        if !message.is_empty() {
            return Err(message);
        }
        return Err(format!(
            "machine plugin call failed: {} ({})",
            status.code,
            status_label(status.code)
        ));
    }
    Ok(copy_and_free_payload(vtable, result.payload))
}

fn rc_slice_utf8(slice: RcSlice) -> String {
    // SAFETY: descriptor bytes are plugin-owned static data by ABI contract.
    let bytes = unsafe { slice.as_bytes() };
    String::from_utf8_lossy(bytes).to_string()
}

struct PluginHost {
    vtable: MachinePluginVTableV1,
}

impl PluginHost {
    fn validate_descriptor_compatibility(
        descriptor: &PluginDescriptorV1,
        host_abi_major: u16,
        host_abi_minor: u16,
    ) -> Result<(), String> {
        plugin_api::compat::check_descriptor(
            descriptor,
            "machine",
            PluginKind::Machine,
            host_abi_major,
            host_abi_minor,
        )
    }

    fn from_descriptor(descriptor: &PluginDescriptorV1) -> Result<Self, String> {
        Self::validate_descriptor_compatibility(descriptor, ABI_MAJOR_V1, ABI_MINOR_V1)?;
        // SAFETY: descriptor.vtable validated non-null above and points at a vtable layout.
        let vtable = unsafe { *descriptor.vtable.cast::<MachinePluginVTableV1>() };
        plugin_api::compat::check_vtable_size(
            "machine",
            MachinePluginVTableV1::expected_size(),
            vtable.size,
        )?;
        Ok(Self { vtable })
    }

    fn list_profiles(&self, req: ListProfilesRequest) -> Result<ListProfilesResponse, String> {
        let req_bytes = encode_message(&req).map_err(|e| e.to_string())?;
        // SAFETY: request slice points to Rust-owned bytes for call duration.
        let result = unsafe { (self.vtable.list_profiles)(RcSlice::from_bytes(&req_bytes)) };
        let payload = decode_call_payload(&self.vtable, result)?;
        decode_message(&payload).map_err(|e| e.to_string())
    }

    fn create_instance(&self, req: MachineCreateRequest) -> Result<MachineCreateResponse, String> {
        let req_bytes = encode_message(&req).map_err(|e| e.to_string())?;
        // SAFETY: request slice points to Rust-owned bytes for call duration.
        let result = unsafe { (self.vtable.create_instance)(RcSlice::from_bytes(&req_bytes)) };
        let payload = decode_call_payload(&self.vtable, result)?;
        decode_message(&payload).map_err(|e| e.to_string())
    }

    fn destroy_instance(&self, instance: MachineInstanceHandle) -> Result<(), String> {
        // SAFETY: instance handle came from this plugin's create_instance response.
        let status = unsafe { (self.vtable.destroy_instance)(instance) };
        if status.is_ok() {
            Ok(())
        } else {
            Err(format!(
                "machine plugin destroy failed: {} ({})",
                status.code,
                status_label(status.code)
            ))
        }
    }

    fn instance_call(
        &self,
        instance: MachineInstanceHandle,
        req: MachineInstanceCallRequest,
    ) -> Result<MachineInstanceCallResponse, String> {
        let req_bytes = encode_message(&req).map_err(|e| e.to_string())?;
        // SAFETY: request slice points to Rust-owned bytes for call duration.
        let result =
            unsafe { (self.vtable.instance_call)(instance, RcSlice::from_bytes(&req_bytes)) };
        let payload = decode_call_payload(&self.vtable, result)?;
        decode_message(&payload).map_err(|e| e.to_string())
    }

    /// Fast path for `Machine::step` -- bypasses `instance_call`'s
    /// bincode-encode/FFI/bincode-decode round trip entirely, since this is
    /// called once per emulated CPU instruction (see `emu_thread.rs`'s
    /// `advance_one_frame`). See `MachinePluginVTableV1::step`'s own doc
    /// comment in `plugin-api` for the full rationale.
    fn step_fast(&self, instance: MachineInstanceHandle) -> u32 {
        // SAFETY: instance handle came from this plugin's create_instance
        // response; the vtable's ABI-minor-gated `step` field is guaranteed
        // present by `from_descriptor`'s own compatibility check.
        unsafe { (self.vtable.step)(instance) }
    }

    /// Fast path for `Machine::halted` -- see `step_fast`'s own doc comment.
    fn halted_fast(&self, instance: MachineInstanceHandle) -> bool {
        // SAFETY: see `step_fast`.
        unsafe { (self.vtable.halted)(instance) != 0 }
    }

    /// Fast path for `Machine::pc` -- see `step_fast`'s own doc comment.
    fn pc_fast(&self, instance: MachineInstanceHandle) -> u64 {
        // SAFETY: see `step_fast`.
        unsafe { (self.vtable.pc_wide)(instance) }
    }
}

struct PluginV1MachineBackend {
    host: PluginHost,
    abi: (u16, u16),
    templates: Mutex<HashMap<MachineInstanceHandle, MachineCreateRequest>>,
    plugin_id: String,
    plugin_version: String,
    source: PathBuf,
}

impl PluginV1MachineBackend {
    fn from_descriptor(descriptor: &PluginDescriptorV1, source: PathBuf) -> Result<Self, String> {
        let plugin_id = rc_slice_utf8(descriptor.id);
        let plugin_version = rc_slice_utf8(descriptor.version);
        let host = PluginHost::from_descriptor(descriptor)?;
        Ok(Self {
            host,
            abi: (descriptor.abi_major, descriptor.abi_minor),
            templates: Mutex::new(HashMap::new()),
            plugin_id,
            plugin_version,
            source,
        })
    }

    fn load_from_static(entry: PluginEntryV1, source: PathBuf) -> Result<Self, String> {
        // SAFETY: a plugin entry returns a pointer to immutable static data.
        let descriptor =
            unsafe { plugin_api::compat::descriptor_from_entry(entry(), "machine", ABI_MAJOR_V1)? };
        Self::from_descriptor(descriptor, source)
    }

    fn load_from_library(path: &Path) -> Result<Self, String> {
        // SAFETY: loading explicit host-selected shared library path.
        let library = unsafe { libloading::Library::new(path) }.map_err(|e| {
            format!(
                "failed to load machine plugin library {}: {e}",
                path.display()
            )
        })?;
        let leaked_library: &'static libloading::Library = Box::leak(Box::new(library));

        let mut descriptor: Option<PluginDescriptorV1> = None;
        let mut last_symbol_error: Option<String> = None;
        for symbol in MACHINE_PLUGIN_ENTRY_SYMBOLS {
            // SAFETY: symbol bytes are static and null-terminated.
            match unsafe { leaked_library.get::<PluginEntryV1>(symbol) } {
                Ok(entry) => {
                    // SAFETY: plugin-owned function pointer contract; the
                    // descriptor is immutable static data of a library that
                    // is never unloaded, and is only interpreted after its
                    // ABI major version matched.
                    descriptor = Some(*unsafe {
                        plugin_api::compat::descriptor_from_entry(entry(), "machine", ABI_MAJOR_V1)
                    }?);
                    break;
                }
                Err(e) => {
                    last_symbol_error = Some(e.to_string());
                }
            }
        }

        let descriptor = descriptor.ok_or_else(|| {
            format!(
                "machine plugin entry symbol not found (tried {:?}): {}",
                MACHINE_PLUGIN_ENTRY_SYMBOLS,
                last_symbol_error.unwrap_or_else(|| "no symbol probe attempted".to_string())
            )
        })?;

        Self::from_descriptor(&descriptor, path.to_path_buf())
    }

    fn list_profiles(&self, req: ListProfilesRequest) -> Result<ListProfilesResponse, String> {
        self.host.list_profiles(req)
    }

    fn create_instance(&self, req: MachineCreateRequest) -> Result<MachineCreateResponse, String> {
        let response = self.host.create_instance(req.clone())?;
        let mut templates = self.templates.lock().unwrap();
        templates.insert(response.instance, req);
        Ok(response)
    }

    fn destroy_instance(&self, instance: MachineInstanceHandle) -> Result<(), String> {
        let mut templates = self.templates.lock().unwrap();
        templates.remove(&instance);
        self.host.destroy_instance(instance)
    }

    fn instance_call(
        &self,
        instance: MachineInstanceHandle,
        req: MachineInstanceCallRequest,
    ) -> Result<MachineInstanceCallResponse, String> {
        self.host.instance_call(instance, req)
    }

    fn step_fast(&self, instance: MachineInstanceHandle) -> u32 {
        self.host.step_fast(instance)
    }

    fn halted_fast(&self, instance: MachineInstanceHandle) -> bool {
        self.host.halted_fast(instance)
    }

    fn pc_fast(&self, instance: MachineInstanceHandle) -> u64 {
        self.host.pc_fast(instance)
    }

    fn clone_instance(
        &self,
        instance: MachineInstanceHandle,
    ) -> Result<MachineInstanceHandle, String> {
        let request = {
            let templates = self.templates.lock().unwrap();
            templates
                .get(&instance)
                .cloned()
                .ok_or_else(|| format!("cannot clone unknown plugin instance handle {instance}"))?
        };

        let saved = self.instance_call(
            instance,
            MachineInstanceCallRequest {
                command: MachineCommand::SaveState,
            },
        )?;
        let bytes = match saved.response {
            MachineResponse::StateBlob(bytes) => bytes,
            other => {
                return Err(format!("clone requires state blob response, got {other:?}"));
            }
        };

        let created = self.create_instance(request)?;
        let new_instance = created.instance;
        match self.instance_call(
            new_instance,
            MachineInstanceCallRequest {
                command: MachineCommand::LoadState { bytes },
            },
        ) {
            Ok(MachineInstanceCallResponse {
                response: MachineResponse::Ack,
            }) => Ok(new_instance),
            Ok(other) => {
                let _ = self.destroy_instance(new_instance);
                Err(format!(
                    "clone restore expected Ack from LoadState, got {:?}",
                    other.response
                ))
            }
            Err(e) => {
                let _ = self.destroy_instance(new_instance);
                Err(format!("clone restore failed: {e}"))
            }
        }
    }
}

#[derive(Debug, Clone)]
struct LoadedPluginSummary {
    plugin_id: String,
    plugin_version: String,
    abi: (u16, u16),
    source: PathBuf,
    machine_ids: Vec<String>,
}

struct RuntimeCatalog {
    plugins: Vec<LoadedPluginSummary>,
    profiles: Vec<MachineProfile>,
    machine_backends: HashMap<String, Arc<PluginV1MachineBackend>>,
    warnings: Vec<String>,
}

fn parse_path_list(var: &str) -> Vec<PathBuf> {
    env::var_os(var)
        .map(|value| env::split_paths(&value).collect())
        .unwrap_or_default()
}

fn plugin_file_extension() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "dll"
    }
    #[cfg(target_os = "linux")]
    {
        "so"
    }
    #[cfg(target_os = "macos")]
    {
        "dylib"
    }
}

fn is_dynamic_library(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(plugin_file_extension()))
}

fn looks_like_machine_plugin_filename(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower = file_name.to_ascii_lowercase();
    lower.contains("machine") && lower.contains("plugin")
}

/// Where machine plugin libraries are looked for when nothing is configured:
/// a `plugins` folder next to the program, the program's own folder (where
/// `cargo build --workspace` leaves them) and a `plugins` folder in the
/// working directory. Unit tests look nowhere by default.
fn default_plugin_dirs() -> Vec<PathBuf> {
    if cfg!(test) {
        return Vec::new();
    }
    let mut dirs = Vec::new();
    if let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.join("plugins"));
        dirs.push(dir.to_path_buf());
    }
    dirs.push(PathBuf::from("plugins"));
    dirs
}

/// `RETROCLAWD_MACHINE_PLUGIN_DIRS` if set, otherwise `default_plugin_dirs`.
pub(crate) fn plugin_search_dirs() -> Vec<PathBuf> {
    let dirs = parse_path_list(MACHINE_PLUGIN_DIRS_ENV_VAR);
    if dirs.is_empty() {
        default_plugin_dirs()
    } else {
        dirs
    }
}

fn plugin_candidate_paths() -> Vec<PathBuf> {
    let mut explicit = Vec::new();
    if let Some(path) = env::var_os(MACHINE_PLUGIN_ENV_VAR) {
        explicit.push(PathBuf::from(path));
    }
    explicit.extend(parse_path_list(MACHINE_PLUGINS_ENV_VAR));

    // Directories searched when no explicit plugin is named:
    // `RETROCLAWD_MACHINE_PLUGIN_DIRS` if set, otherwise the default
    // locations next to the program (see `default_plugin_dirs`).
    let mut candidates = if explicit.is_empty() {
        let dirs = plugin_search_dirs();

        let mut from_dirs = Vec::new();
        for dir in dirs {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && is_dynamic_library(&path)
                    && looks_like_machine_plugin_filename(&path)
                {
                    from_dirs.push(path);
                }
            }
        }
        from_dirs
    } else {
        explicit
    };

    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for path in candidates.drain(..) {
        let normalized = path.canonicalize().unwrap_or(path.clone());
        let key = normalized.to_string_lossy().to_lowercase();
        if seen.insert(key) {
            deduped.push(path);
        }
    }
    deduped.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    deduped
}

fn debug_capability_label(capability: MachineDebugCapability) -> &'static str {
    match capability {
        MachineDebugCapability::PauseResume => "pause/resume",
        MachineDebugCapability::StepInstruction => "step instruction",
        MachineDebugCapability::Breakpoints => "breakpoints",
        MachineDebugCapability::Registers => "register snapshots",
        MachineDebugCapability::AddressSpaces => "address-space metadata",
        MachineDebugCapability::MemoryRead => "memory read",
        MachineDebugCapability::MemoryWrite => "memory write",
        MachineDebugCapability::Disassembly => "disassembly",
        MachineDebugCapability::MachineMetadata => "machine metadata",
        MachineDebugCapability::MediaSnapshot => "media snapshot",
        MachineDebugCapability::ChipSnapshots => "chip snapshots",
    }
}

fn chip_snapshot<'a>(snapshots: &'a [ChipSnapshot], chip_id: &str) -> Option<&'a ChipSnapshot> {
    snapshots
        .iter()
        .find(|snap| snap.chip_id.eq_ignore_ascii_case(chip_id))
}

fn chip_field<'a>(fields: &'a [NamedValue], key: &str) -> Option<&'a DebugValue> {
    fields
        .iter()
        .find(|field| field.key.eq_ignore_ascii_case(key))
        .map(|field| &field.value)
}

fn chip_field_u64(fields: &[NamedValue], key: &str) -> Option<u64> {
    match chip_field(fields, key)? {
        DebugValue::U64(v) => Some(*v),
        DebugValue::I64(v) => u64::try_from(*v).ok(),
        _ => None,
    }
}

fn chip_field_text(fields: &[NamedValue], key: &str) -> Option<String> {
    match chip_field(fields, key)? {
        DebugValue::Text(v) => Some(v.clone()),
        _ => None,
    }
}

fn chip_field_bool(fields: &[NamedValue], key: &str) -> Option<bool> {
    match chip_field(fields, key)? {
        DebugValue::Bool(v) => Some(*v),
        _ => None,
    }
}

fn chip_blob_bytes(blobs: &[BinaryBlob], key: &str) -> Option<Vec<u8>> {
    blobs
        .iter()
        .find(|blob| blob.key.eq_ignore_ascii_case(key))
        .map(|blob| blob.bytes.clone())
}

/// Decodes a blob encoding a sequence of little-endian `u16` values (used
/// by `tape_pulse_widths` -- see each `*-machine-plugin`'s own
/// `generic_overlay_chip_snapshot` for the producing side). A trailing
/// odd byte (malformed/truncated blob) is dropped rather than erroring --
/// this is diagnostic-only overlay data, not a protocol correctness
/// concern worth failing a whole snapshot fetch over.
fn chip_blob_u16_pairs(blobs: &[BinaryBlob], key: &str) -> Vec<u16> {
    let Some(bytes) = chip_blob_bytes(blobs, key) else {
        return Vec::new();
    };
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect()
}

fn validate_profile_contract_with_version(
    profile: &MachineProfile,
    plugin_id: &str,
    plugin_version: &str,
    source: &Path,
    host_contract_version: u16,
) -> Result<Option<String>, String> {
    if profile.contract_version != host_contract_version {
        return Err(format!(
            "machine profile contract mismatch for {} from {} ({}@{}): host={}, plugin={}",
            profile.machine_id,
            source.display(),
            plugin_id,
            plugin_version,
            host_contract_version,
            profile.contract_version
        ));
    }

    let has_debug = profile.capabilities.contains(&MachineCapability::Debug);
    if !has_debug {
        if profile.debug_capabilities.is_empty() {
            return Ok(None);
        }
        return Ok(Some(format!(
            "profile {} declares debug capability table but is missing MachineCapability::Debug",
            profile.machine_id
        )));
    }

    let supported: HashSet<MachineDebugCapability> =
        profile.debug_capabilities.iter().copied().collect();
    let missing: Vec<MachineDebugCapability> = REQUIRED_MACHINE_DEBUG_CAPABILITIES
        .iter()
        .copied()
        .filter(|required| !supported.contains(required))
        .collect();
    if missing.is_empty() {
        return Ok(None);
    }

    let missing_labels = missing
        .iter()
        .map(|capability| debug_capability_label(*capability))
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "profile {} from {} ({}@{}) is missing required debug contract capabilities: {}",
        profile.machine_id,
        source.display(),
        plugin_id,
        plugin_version,
        missing_labels
    ))
}

fn validate_profile_contract(
    profile: &MachineProfile,
    plugin_id: &str,
    plugin_version: &str,
    source: &Path,
) -> Result<Option<String>, String> {
    validate_profile_contract_with_version(
        profile,
        plugin_id,
        plugin_version,
        source,
        MACHINE_PROFILE_CONTRACT_VERSION,
    )
}

/// The stub machine plugin, linked straight into a unit-test binary so the
/// host tests have a machine to drive (the real machines live in their family
/// repositories and are never linked into this one). The shipped
/// binary links no machine at all: every machine is a plugin library loaded
/// from disk (see `default_plugin_dirs`), so removing a machine means
/// deleting its library.
#[cfg(test)]
fn bundled_machine_plugin_entries() -> Vec<(&'static str, PluginEntryV1)> {
    vec![(
        "test-machine-plugin",
        test_machine_plugin_impl::entry_v1 as PluginEntryV1,
    )]
}

#[cfg(not(test))]
fn bundled_machine_plugin_entries() -> Vec<(&'static str, PluginEntryV1)> {
    Vec::new()
}

fn bundled_backend_candidates() -> Vec<(String, Result<PluginV1MachineBackend, String>)> {
    bundled_machine_plugin_entries()
        .into_iter()
        .map(|(name, entry)| {
            let source = PathBuf::from(format!("bundled:{name}"));
            (
                source.display().to_string(),
                PluginV1MachineBackend::load_from_static(entry, source),
            )
        })
        .collect()
}

type LoadedBackends = (
    Vec<LoadedPluginSummary>,
    Vec<MachineProfile>,
    HashMap<String, Arc<PluginV1MachineBackend>>,
    Vec<String>,
);

fn load_backend_candidates(
    backend_candidates: Vec<(String, Result<PluginV1MachineBackend, String>)>,
) -> Result<LoadedBackends, String> {
    let mut plugins = Vec::new();
    let mut profiles = Vec::new();
    let mut machine_backends: HashMap<String, Arc<PluginV1MachineBackend>> = HashMap::new();
    let mut warnings = Vec::new();

    for (candidate_label, backend_result) in backend_candidates {
        let backend = match backend_result {
            Ok(backend) => backend,
            Err(error) => {
                warnings.push(format!("skip {candidate_label}: {error}"));
                continue;
            }
        };

        let plugin_id = backend.plugin_id.clone();
        let plugin_version = backend.plugin_version.clone();
        let abi = backend.abi;
        let source = backend.source.clone();
        let backend = Arc::new(backend);

        let response = match backend.list_profiles(ListProfilesRequest {
            include_experimental: false,
        }) {
            Ok(response) => response,
            Err(error) => {
                warnings.push(format!(
                    "skip {} ({}@{}): list_profiles failed: {error}",
                    source.display(),
                    plugin_id,
                    plugin_version
                ));
                continue;
            }
        };

        let mut machine_ids = Vec::new();
        for profile in response.profiles {
            if let Some(warning) =
                validate_profile_contract(&profile, &plugin_id, &plugin_version, &source)?
            {
                warnings.push(warning);
            }
            let machine_id = profile.machine_id.clone();
            if let Some(existing) = machine_backends.get(&machine_id) {
                return Err(format!(
                    "duplicate machine id \"{machine_id}\": plugin {} also provides it",
                    existing.plugin_id
                ));
            }
            machine_ids.push(machine_id.clone());
            machine_backends.insert(machine_id, backend.clone());
            profiles.push(profile);
        }

        plugins.push(LoadedPluginSummary {
            plugin_id,
            plugin_version,
            abi,
            source,
            machine_ids,
        });
    }

    Ok((plugins, profiles, machine_backends, warnings))
}

fn discover_runtime_catalog() -> Result<RuntimeCatalog, String> {
    let explicit_dynamic = env::var_os(MACHINE_PLUGIN_ENV_VAR).is_some()
        || env::var_os(MACHINE_PLUGINS_ENV_VAR).is_some();
    discover_runtime_catalog_from(
        plugin_candidate_paths(),
        explicit_dynamic,
        plugin_search_dirs(),
    )
}

/// The discovery proper, with its inputs given: the library files to try, whether the user named libraries
/// explicitly, and the folders to mention when nothing is found. (Separate from the environment so a test can run it
/// without touching process-wide variables.)
fn discover_runtime_catalog_from(
    candidates: Vec<PathBuf>,
    explicit_dynamic: bool,
    searched_dirs: Vec<PathBuf>,
) -> Result<RuntimeCatalog, String> {
    if candidates.is_empty() && explicit_dynamic {
        return Err(
            "no machine plugin candidates found from RETROCLAWD_MACHINE_PLUGIN or RETROCLAWD_MACHINE_PLUGINS"
                .to_string(),
        );
    }

    let scanned_disk = !candidates.is_empty();
    let backend_candidates: Vec<(String, Result<PluginV1MachineBackend, String>)> = if scanned_disk
    {
        candidates
            .into_iter()
            .map(|path| {
                let label = path.display().to_string();
                let result = PluginV1MachineBackend::load_from_library(&path);
                (label, result)
            })
            .collect()
    } else {
        bundled_backend_candidates()
    };

    let (mut plugins, mut profiles, machine_backends, warnings) =
        load_backend_candidates(backend_candidates)?;

    if profiles.is_empty() {
        let mut message = format!(
            "no machine plugins found: put the machine plugin libraries (*_machine_plugin.{}) in one of {} (searched; set RETROCLAWD_MACHINE_PLUGIN_DIRS to look elsewhere)",
            plugin_file_extension(),
            searched_dirs
                .iter()
                .map(|d| d.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        if !warnings.is_empty() {
            message.push_str("; probe errors:\n");
            message.push_str(&warnings.join("\n"));
        }
        return Err(message);
    }

    profiles.sort_by(|a, b| a.machine_id.cmp(&b.machine_id));
    plugins.sort_by(|a, b| {
        a.plugin_id
            .cmp(&b.plugin_id)
            .then_with(|| a.source.cmp(&b.source))
    });

    Ok(RuntimeCatalog {
        plugins,
        profiles,
        machine_backends,
        warnings,
    })
}

fn runtime_catalog() -> Result<&'static RuntimeCatalog, String> {
    match RUNTIME_CATALOG.get_or_init(discover_runtime_catalog) {
        Ok(catalog) => Ok(catalog),
        Err(error) => Err(error.clone()),
    }
}

pub(crate) fn startup_report() -> Result<StartupReport, String> {
    let catalog = runtime_catalog()?;
    Ok(StartupReport {
        plugins: catalog
            .plugins
            .iter()
            .map(|p| StartupPluginReport {
                plugin_id: p.plugin_id.clone(),
                plugin_version: p.plugin_version.clone(),
                abi: p.abi,
                source: p.source.display().to_string(),
                machine_ids: p.machine_ids.clone(),
            })
            .collect(),
        profiles: catalog.profiles.clone(),
        warnings: catalog.warnings.clone(),
    })
}

pub(crate) fn create_hosted_machine(
    request: MachineCreateRequest,
) -> Result<HostedMachine, String> {
    let catalog = runtime_catalog()?;
    let backend = catalog
        .machine_backends
        .get(&request.machine_id)
        .cloned()
        .ok_or_else(|| {
            let mut ids: Vec<&str> = catalog
                .profiles
                .iter()
                .map(|p| p.machine_id.as_str())
                .collect();
            ids.sort_unstable();
            format!(
                "unknown machine id \"{}\". available: {}",
                request.machine_id,
                ids.join(", ")
            )
        })?;

    let machine_id = request.machine_id.clone();
    let create = backend.create_instance(request)?;
    Ok(HostedMachine {
        backend: HostedBackend { plugin: backend },
        instance: create.instance,
        machine_id,
        metadata: create.metadata,
        disk_exit: [None, None],
        tape_exit: None,
        cartridge_exit: None,
        printer_out: None,
    })
}

impl HostedBackend {
    fn destroy_instance(&self, instance: MachineInstanceHandle) -> Result<(), String> {
        self.plugin.destroy_instance(instance)
    }

    fn clone_instance(
        &self,
        instance: MachineInstanceHandle,
    ) -> Result<MachineInstanceHandle, String> {
        self.plugin.clone_instance(instance)
    }

    fn instance_call(
        &self,
        instance: MachineInstanceHandle,
        req: MachineInstanceCallRequest,
    ) -> Result<MachineInstanceCallResponse, String> {
        self.plugin.instance_call(instance, req)
    }

    fn step_fast(&self, instance: MachineInstanceHandle) -> u32 {
        self.plugin.step_fast(instance)
    }

    fn halted_fast(&self, instance: MachineInstanceHandle) -> bool {
        self.plugin.halted_fast(instance)
    }

    fn pc_fast(&self, instance: MachineInstanceHandle) -> u64 {
        self.plugin.pc_fast(instance)
    }

    fn expect_ack(
        &self,
        instance: MachineInstanceHandle,
        command: MachineCommand,
        context: &str,
    ) -> Result<(), String> {
        let response = self.instance_call(instance, MachineInstanceCallRequest { command })?;
        match response.response {
            MachineResponse::Ack => Ok(()),
            other => Err(format!("{context} expected Ack, got {other:?}")),
        }
    }

    fn registers(
        &self,
        instance: MachineInstanceHandle,
    ) -> Result<Vec<(&'static str, u64)>, String> {
        let response = self.instance_call(
            instance,
            MachineInstanceCallRequest {
                command: MachineCommand::Debug(DebugCommand::GetRegisters),
            },
        )?;
        match response.response {
            MachineResponse::Debug(DebugResponse::Registers(snapshot)) => Ok(snapshot
                .registers
                .into_iter()
                .map(|reg| (intern_register_label(&reg.name), reg.value))
                .collect()),
            other => Err(format!(
                "register snapshot expected Debug::Registers, got {other:?}"
            )),
        }
    }
    fn set_register(
        &self,
        instance: MachineInstanceHandle,
        name: &str,
        value: u64,
    ) -> Result<(), String> {
        let response = self.instance_call(
            instance,
            MachineInstanceCallRequest {
                command: MachineCommand::Debug(DebugCommand::SetRegister {
                    name: name.to_string(),
                    value,
                }),
            },
        )?;
        match response.response {
            MachineResponse::Debug(DebugResponse::Ack) => Ok(()),
            other => Err(format!("set register expected Debug::Ack, got {other:?}")),
        }
    }

    fn key_event(
        &self,
        instance: MachineInstanceHandle,
        key: &str,
        pressed: bool,
    ) -> Result<(), String> {
        self.expect_ack(
            instance,
            MachineCommand::KeyEvent {
                key: key.to_string(),
                pressed,
            },
            "key event",
        )
    }

    fn mouse_event(
        &self,
        instance: MachineInstanceHandle,
        event: crate::emu_thread::MouseEvent,
    ) -> Result<(), String> {
        let command = match event {
            crate::emu_thread::MouseEvent::Motion { dx, dy } => {
                MachineCommand::MouseMotion { dx, dy }
            }
            crate::emu_thread::MouseEvent::Button { button, pressed } => {
                let button = match button {
                    bevy::input::mouse::MouseButton::Left => HostMouseButton::Left,
                    bevy::input::mouse::MouseButton::Right => HostMouseButton::Right,
                    bevy::input::mouse::MouseButton::Middle => HostMouseButton::Middle,
                    bevy::input::mouse::MouseButton::Other(code) => HostMouseButton::Other(code),
                    _ => HostMouseButton::Other(u16::MAX),
                };
                MachineCommand::MouseButton { button, pressed }
            }
            crate::emu_thread::MouseEvent::Scroll { delta } => MachineCommand::MouseWheel { delta },
        };
        self.expect_ack(instance, command, "mouse event")
    }

    fn tape_debug_snapshot(
        &self,
        instance: MachineInstanceHandle,
        machine_id: &str,
    ) -> Result<Option<TapeDebugSnapshot>, String> {
        let response = self.instance_call(
            instance,
            MachineInstanceCallRequest {
                command: MachineCommand::GetMediaSnapshot,
            },
        )?;
        match response.response {
            MachineResponse::MediaSnapshot(snapshot) => {
                Ok(snapshot.tape.map(|t| TapeDebugSnapshot {
                    machine: machine_id.to_string(),
                    attached: t.attached,
                    motor_on: t.motor_on,
                    position: t.position.unwrap_or(0) as usize,
                    events: t.total.unwrap_or(0) as usize,
                    playing: t.playing,
                    finished: t.finished,
                    speed_multiplier: t.speed_multiplier,
                    ear_bit: t.ear_bit,
                    cycles_until_next_edge: t.cycles_until_next_edge,
                    spin_down_cycles: t.spin_down_cycles,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Returns the machine's generic `chip_id: "overlay"` snapshot verbatim
    /// (whatever `fields`/`blobs` it happens to carry), with no knowledge of
    /// which specific field names to expect and no `machine_id` gating --
    /// replaces the old `cpc_overlay_debug_snapshot`, which required 5
    /// specific CPC-only field names to all be present (returning `None`,
    /// silently dropping the *entire* snapshot, if even one was missing --
    /// a real "typo'd/renamed field silently degrades" trap) and was only
    /// ever called for a machine id starting with `"cpc"`. Every consumer
    /// (`--debug`'s diagnostic log, ZRCP's `get-paging-state`) now formats
    /// whatever fields are actually present generically instead.
    fn overlay_chip_snapshot(
        &self,
        instance: MachineInstanceHandle,
    ) -> Result<Option<ChipSnapshot>, String> {
        let response = self.instance_call(
            instance,
            MachineInstanceCallRequest {
                command: MachineCommand::Debug(DebugCommand::GetChipSnapshots),
            },
        )?;
        let MachineResponse::Debug(DebugResponse::ChipSnapshots(snapshots)) = response.response
        else {
            return Ok(None);
        };
        Ok(chip_snapshot(&snapshots, "overlay").cloned())
    }

    /// Decodes the same `"overlay"` chip snapshot `overlay_chip_snapshot`
    /// returns verbatim into the small, fixed `OverlayDebugSnapshot` shape
    /// the GUI's `--show-overlay` panel wants -- every field here is
    /// independently optional (no single missing field drops the whole
    /// result) since most machines only populate a subset. This one still
    /// reads specific field names by design (the GUI panel has a fixed
    /// layout to fill), unlike `overlay_chip_snapshot`'s callers, which want
    /// the raw, unopinionated field list instead.
    fn overlay_debug_snapshot(
        &self,
        instance: MachineInstanceHandle,
    ) -> Result<OverlayDebugSnapshot, String> {
        let response = self.instance_call(
            instance,
            MachineInstanceCallRequest {
                command: MachineCommand::Debug(DebugCommand::GetChipSnapshots),
            },
        )?;
        let MachineResponse::Debug(DebugResponse::ChipSnapshots(snapshots)) = response.response
        else {
            return Ok(OverlayDebugSnapshot::default());
        };
        let Some(snapshot) = chip_snapshot(&snapshots, "overlay") else {
            return Ok(OverlayDebugSnapshot::default());
        };
        let fields = &snapshot.fields;
        Ok(OverlayDebugSnapshot {
            memory_summary: chip_field_text(fields, "memory_summary"),
            fdc_phase: chip_field_text(fields, "fdc_phase"),
            disk_motor: chip_field_bool(fields, "disk_motor"),
            tape_motor: chip_field_bool(fields, "tape_motor"),
            tape_position: chip_field_u64(fields, "tape_position"),
            tape_total: chip_field_u64(fields, "tape_total"),
            tape_edge_cycles: chip_field_u64(fields, "tape_edge_cycles"),
            tape_waveform: chip_blob_bytes(&snapshot.blobs, "tape_waveform").unwrap_or_default(),
            tape_pulse_widths: chip_blob_u16_pairs(&snapshot.blobs, "tape_pulse_widths"),
        })
    }

    fn set_border(&self, instance: MachineInstanceHandle, enabled: bool) -> Result<(), String> {
        self.expect_ack(
            instance,
            MachineCommand::SetBorder { enabled },
            "set border",
        )
    }

    fn apply_snapshot(
        &self,
        instance: MachineInstanceHandle,
        extension: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        self.expect_ack(
            instance,
            MachineCommand::AttachMedia {
                slot: MediaSlot::Snapshot,
                kind: MediaKind::Snapshot,
                name: Some(format!("snapshot.{extension}")),
                bytes: bytes.to_vec(),
                options: Vec::new(),
            },
            "snapshot attach",
        )
    }

    fn save_snapshot(
        &self,
        instance: MachineInstanceHandle,
        extension: &str,
    ) -> Result<Vec<u8>, String> {
        let response = self.instance_call(
            instance,
            MachineInstanceCallRequest {
                command: MachineCommand::SaveSnapshot {
                    extension: extension.to_string(),
                },
            },
        )?;
        match response.response {
            MachineResponse::SnapshotBlob(bytes) => Ok(bytes),
            other => Err(format!(
                "save snapshot expected SnapshotBlob, got {other:?}"
            )),
        }
    }

    fn apply_tape(
        &self,
        instance: MachineInstanceHandle,
        extension: &str,
        bytes: &[u8],
        speed: u32,
    ) -> Result<(), String> {
        self.expect_ack(
            instance,
            MachineCommand::AttachMedia {
                slot: MediaSlot::Tape,
                kind: MediaKind::Tape,
                name: Some(format!("tape.{extension}")),
                bytes: bytes.to_vec(),
                options: vec![NamedValue {
                    key: "tape_speed".to_string(),
                    value: DebugValue::U64(speed as u64),
                }],
            },
            "tape attach",
        )
    }

    fn apply_disk(
        &self,
        instance: MachineInstanceHandle,
        drive: u8,
        extension: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        self.expect_ack(
            instance,
            MachineCommand::AttachMedia {
                slot: MediaSlot::Disk { drive },
                kind: MediaKind::Disk,
                name: Some(format!("disk.{extension}")),
                bytes: bytes.to_vec(),
                options: Vec::new(),
            },
            "disk attach",
        )
    }

    fn apply_media(
        &self,
        instance: MachineInstanceHandle,
        extension: &str,
        bytes: &[u8],
        tape_speed: u32,
    ) -> Result<(), String> {
        let lower = extension.to_ascii_lowercase();
        if ["tap", "tzx", "cdt", "pzx", "csw", "voc", "wav", "t64"].contains(&lower.as_str()) {
            self.apply_tape(instance, &lower, bytes, tape_speed)
        } else if lower == "dsk"
            || lower == "edsk"
            || lower == "d64"
            || lower == "d71"
            || lower == "d81"
            || lower == "g64"
            || lower == "x64"
            || lower == "nib"
        {
            self.apply_disk(instance, 0, &lower, bytes)
        } else if lower == "cpr" || lower == "crt" {
            self.expect_ack(
                instance,
                MachineCommand::AttachMedia {
                    slot: MediaSlot::Cartridge { slot: 0 },
                    kind: MediaKind::Cartridge,
                    name: Some(format!("cartridge.{lower}")),
                    bytes: bytes.to_vec(),
                    options: Vec::new(),
                },
                "cartridge attach",
            )
        } else {
            self.apply_snapshot(instance, &lower, bytes)
        }
    }
}

/// Whether a call's error is a plugin's own crash report (`plugin_api::guard`: "panic in <site>: <text>" for the call that
/// panicked, "the plugin crashed earlier (panic in ...)" for every call after it).
fn is_crash_error(error: &str) -> bool {
    error.contains("panic in ") || error.contains("crashed earlier")
}

impl HostedMachine {
    pub(crate) fn machine_id(&self) -> &str {
        &self.machine_id
    }

    fn call_machine(&self, command: MachineCommand) -> Result<MachineResponse, String> {
        self.backend
            .instance_call(self.instance, MachineInstanceCallRequest { command })
            .map(|resp| resp.response)
    }

    /// The text of the panic that killed the machine's plugin, if it has crashed: a crashed plugin answers every call with an error
    /// naming the panic (`plugin_api::guard`), so one cheap call is the probe. `None` while it is healthy (or when the failure is
    /// some other error, which the caller of that call reports itself).
    pub(crate) fn crash_message(&self) -> Option<String> {
        match self.call_machine(MachineCommand::GetFrameHz) {
            Err(error) if is_crash_error(&error) => Some(error),
            _ => None,
        }
    }

    pub(crate) fn metadata(&self) -> &MachineMetadata {
        &self.metadata
    }

    /// `lines` instructions of the machine's own disassembly starting at `start` (the program
    /// counter when `None`). The machine plugin decodes them, because only it knows which CPU is
    /// running (a C128 switches between an 8502 and a Z80), so the host stays CPU-agnostic.
    pub(crate) fn disassemble(
        &self,
        start: Option<u16>,
        lines: u16,
    ) -> Result<Vec<plugin_api::DisassemblyLine>, String> {
        match self.call_machine(MachineCommand::Debug(DebugCommand::GetDisassembly {
            start: start.map(u64::from),
            lines: lines.into(),
        }))? {
            MachineResponse::Debug(DebugResponse::Disassembly(lines)) => Ok(lines),
            other => Err(format!(
                "disassembly expected Debug::Disassembly, got {other:?}"
            )),
        }
    }

    /// The CPU family that is running right now, from the machine's register snapshot.
    pub(crate) fn cpu_family(&self) -> CpuFamily {
        match self.call_machine(MachineCommand::Debug(DebugCommand::GetRegisters)) {
            Ok(MachineResponse::Debug(DebugResponse::Registers(snapshot))) => snapshot.cpu_family,
            _ => CpuFamily::Z80,
        }
    }

    pub(crate) fn write_memory_space(
        &mut self,
        address: u16,
        bytes: &[u8],
        space: Option<String>,
    ) -> Result<(), String> {
        match self.call_machine(MachineCommand::WriteMemory {
            address: address.into(),
            bytes: bytes.to_vec(),
            space,
        })? {
            MachineResponse::BytesWritten(written) if written == bytes.len() as u32 => Ok(()),
            MachineResponse::BytesWritten(written) => Err(format!(
                "banked poke wrote {written} of {} byte(s)",
                bytes.len()
            )),
            other => Err(format!(
                "bridge returned wrong response for banked poke: {other:?}"
            )),
        }
    }

    /// Replaces the machine's watched I/O ports (the debugger's port
    /// breakpoints); an error for a machine with no I/O-port space.
    pub(crate) fn set_port_watches(&self, watches: &[PortWatchSpec]) -> Result<(), String> {
        match self.call_machine(MachineCommand::SetPortWatches {
            watches: watches.to_vec(),
        })? {
            MachineResponse::Ack => Ok(()),
            other => Err(format!(
                "bridge returned wrong response for port watches: {other:?}"
            )),
        }
    }

    /// The first watched port access since the last call, if any.
    pub(crate) fn take_port_hit(&self) -> Option<PortHitInfo> {
        match self.call_machine(MachineCommand::TakePortHit) {
            Ok(MachineResponse::PortHit(hit)) => hit,
            _ => None,
        }
    }

    /// `OUT (port),value` on the machine's I/O bus; an error for a machine
    /// with no I/O-port space.
    pub(crate) fn write_port(&mut self, port: u16, value: u8) -> Result<(), String> {
        match self.call_machine(MachineCommand::WriteIo { port, value })? {
            MachineResponse::Ack => Ok(()),
            other => Err(format!(
                "bridge returned wrong response for port write: {other:?}"
            )),
        }
    }

    pub(crate) fn clock_hz(&self) -> u64 {
        <Self as Machine>::clock_hz(self)
    }

    pub(crate) fn frame_hz(&self) -> u64 {
        <Self as Machine>::frame_hz(self)
    }

    pub(crate) fn step(&mut self) -> u32 {
        <Self as Machine>::step(self)
    }

    pub(crate) fn halted(&self) -> bool {
        <Self as Machine>::halted(self)
    }

    pub(crate) fn pc(&self) -> u16 {
        <Self as Machine>::pc(self).try_into().unwrap()
    }

    pub(crate) fn render_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
        <Self as Machine>::render_frame(self)
    }

    pub(crate) fn read_memory(&self, addr: u16) -> u8 {
        <Self as Machine>::read_memory(self, addr.into())
    }

    pub(crate) fn load(&mut self, addr: u16, bytes: &[u8]) -> usize {
        <Self as Machine>::load(self, addr.into(), bytes)
    }

    pub(crate) fn set_pc(&mut self, addr: u16) {
        <Self as Machine>::set_pc(self, addr.into());
    }
    pub(crate) fn set_register(&mut self, name: &str, value: u64) -> Result<(), String> {
        self.backend.set_register(self.instance, name, value)
    }

    pub(crate) fn drain_audio(&mut self) -> Vec<f32> {
        <Self as Machine>::drain_audio(self)
    }

    pub(crate) fn save_state(&self) -> Vec<u8> {
        <Self as Machine>::save_state(self)
    }

    pub(crate) fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        <Self as Machine>::load_state(self, bytes)
    }

    pub(crate) fn reset(&mut self) {
        <Self as Machine>::reset(self);
    }

    pub(crate) fn run(
        &mut self,
        running: &mut dyn FnMut(&dyn Machine) -> bool,
    ) -> machine::RunStats {
        machine::Clock::new(self.frame_hz()).run(self, running)
    }

    pub(crate) fn registers(&self) -> Vec<(&'static str, u64)> {
        self.backend.registers(self.instance).unwrap_or_default()
    }

    pub(crate) fn key_event(&mut self, key: bevy::input::keyboard::KeyCode, pressed: bool) {
        self.key_event_name(&format!("{key:?}"), pressed);
    }

    /// `key_event_name`, but the plugin's refusal (an unknown key name) is returned instead of logged.
    pub(crate) fn try_key_event(&mut self, key: &str, pressed: bool) -> Result<(), String> {
        self.backend.key_event(self.instance, key, pressed)
    }

    pub(crate) fn key_event_name(&mut self, key: &str, pressed: bool) {
        match self.backend.key_event(self.instance, key, pressed) {
            Ok(()) => {
                tracing::info!(key, pressed, "hosted key event delivered");
            }
            Err(e) => {
                tracing::warn!(key, pressed, error = %e, "hosted key event rejected");
            }
        }
    }

    pub(crate) fn mouse_event(&mut self, event: crate::emu_thread::MouseEvent) {
        let _ = self.backend.mouse_event(self.instance, event);
    }

    pub(crate) fn tape_debug_snapshot(&self) -> Option<TapeDebugSnapshot> {
        self.backend
            .tape_debug_snapshot(self.instance, &self.machine_id)
            .ok()
            .flatten()
    }

    /// Returns whatever the plugin's own generic `"overlay"` chip snapshot
    /// contains, verbatim -- no `machine_id` gate (any plugin, `zx81`
    /// included, may safely be asked; one with nothing to report just
    /// returns `None`, same as `chip_snapshot` finding no matching chip at
    /// all) and no fixed field-name expectations, unlike the old CPC-only
    /// `cpc_overlay_debug_snapshot` this replaces. Feeds `--debug`'s
    /// diagnostic log (`emu_thread.rs`) and ZRCP's `get-paging-state`
    /// (`DebuggerCommand::GetOverlaySnapshot`) -- both format whichever
    /// fields are actually present rather than requiring specific ones.
    pub(crate) fn overlay_diagnostic_snapshot(&self) -> Option<ChipSnapshot> {
        self.backend
            .overlay_chip_snapshot(self.instance)
            .ok()
            .flatten()
    }

    /// Machine-agnostic counterpart to `overlay_diagnostic_snapshot` above,
    /// for the on-screen `--show-overlay` GUI panel -- decodes the same
    /// chip into the small, fixed `OverlayDebugSnapshot` shape that panel's
    /// layout wants, rather than returning the raw field list.
    pub(crate) fn overlay_debug_snapshot(&self) -> OverlayDebugSnapshot {
        self.backend
            .overlay_debug_snapshot(self.instance)
            .unwrap_or_default()
    }

    pub(crate) fn set_border(&mut self, enabled: bool) {
        let _ = self.backend.set_border(self.instance, enabled);
    }

    pub(crate) fn apply_snapshot(&mut self, extension: &str, bytes: &[u8]) -> Result<(), String> {
        self.backend.apply_snapshot(self.instance, extension, bytes)
    }

    pub(crate) fn save_snapshot(&self, extension: &str) -> Result<Vec<u8>, String> {
        self.backend.save_snapshot(self.instance, extension)
    }

    pub(crate) fn apply_tape(
        &mut self,
        extension: &str,
        bytes: &[u8],
        speed: u32,
    ) -> Result<(), String> {
        self.backend
            .apply_tape(self.instance, extension, bytes, speed)
    }

    pub(crate) fn apply_disk(&mut self, extension: &str, bytes: &[u8]) -> Result<(), String> {
        self.apply_disk_in(0, extension, bytes)
    }

    /// Puts a disk image in `drive` (0 = A, 1 = B); a machine without that drive refuses with its own message.
    pub(crate) fn apply_disk_in(
        &mut self,
        drive: u8,
        extension: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        self.backend
            .apply_disk(self.instance, drive, extension, bytes)
    }

    /// Takes the tape or disk out of its slot (`DetachMedia`). Unsaved disk
    /// writes are lost with it: export them first.
    pub(crate) fn detach_media(&self, slot: MediaSlot) -> Result<(), String> {
        match self.call_machine(MachineCommand::DetachMedia { slot })? {
            MachineResponse::Ack => Ok(()),
            other => Err(format!(
                "bridge returned wrong response for DetachMedia: {other:?}"
            )),
        }
    }

    /// Chooses what happens to a modified disk image when this machine is
    /// dropped. Call after attaching the disk.
    pub(crate) fn set_disk_exit_policy(&mut self, policy: DiskExitPolicy) {
        self.set_disk_exit_policy_for(0, policy);
    }

    /// `set_disk_exit_policy` for `drive` (0 = A, 1 = B).
    pub(crate) fn set_disk_exit_policy_for(&mut self, drive: u8, policy: DiskExitPolicy) {
        if let Some(slot) = self.disk_exit.get_mut(usize::from(drive)) {
            *slot = Some(policy);
        }
    }

    /// The drive's current disk image (guest writes applied) in the format
    /// it was attached as. With `only_if_modified`, `None` when nothing was
    /// written since it was attached or last marked saved.
    pub(crate) fn export_disk(
        &self,
        drive: u8,
        only_if_modified: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        match self.call_machine(MachineCommand::ExportDisk {
            drive,
            only_if_modified,
        })? {
            MachineResponse::DiskImage(image) => Ok(image),
            other => Err(format!(
                "bridge returned wrong response for ExportDisk: {other:?}"
            )),
        }
    }

    pub(crate) fn mark_disk_saved(&self, drive: u8) -> Result<(), String> {
        match self.call_machine(MachineCommand::MarkDiskSaved { drive })? {
            MachineResponse::Ack => Ok(()),
            other => Err(format!(
                "bridge returned wrong response for MarkDiskSaved: {other:?}"
            )),
        }
    }

    /// Applies the exit policy: writes a modified image to its target, or
    /// warns that changes are being discarded. Silent when nothing was
    /// written to the disk. Reports on stderr -- it runs from `Drop`, where
    /// there is nobody to return an error to.
    fn finish_disk_on_exit(&mut self) {
        for drive in 0..2u8 {
            self.finish_drive_on_exit(drive);
        }
    }

    fn finish_drive_on_exit(&mut self, drive: u8) {
        let Some(policy) = self.disk_exit[usize::from(drive)].take() else {
            return;
        };
        let name = if drive == 0 { "disk" } else { "drive B disk" };
        let image = match self.export_disk(drive, true) {
            Ok(Some(image)) => image,
            Ok(None) => return,
            Err(e) => {
                eprintln!("could not read back the {name} image: {e}");
                return;
            }
        };
        match policy.save_to {
            Some(path) => match write_file_atomically(&path, &image) {
                Ok(()) => {
                    let _ = self.mark_disk_saved(drive);
                    eprintln!("saved modified {name} image to {}", path.display());
                }
                Err(e) => eprintln!(
                    "FAILED to save the modified {name} image to {}: {e}",
                    path.display()
                ),
            },
            None if drive == 0 => eprintln!(
                "the disk image was modified but not saved -- run with --disk-save <path> \
                 (or --disk-writeback to overwrite the --disk file) to keep changes"
            ),
            None => eprintln!(
                "the drive B disk image was modified but not saved -- run with --disk-b-save <path> \
                 (or --disk-writeback to overwrite the --disk-b file) to keep changes"
            ),
        }
    }

    /// Presses the cartridge's freeze button.
    pub(crate) fn freeze(&self) -> Result<(), String> {
        match self.call_machine(MachineCommand::Freeze)? {
            MachineResponse::Ack => Ok(()),
            other => Err(format!(
                "bridge returned wrong response for Freeze: {other:?}"
            )),
        }
    }

    /// The cartridge as a `.crt` file when its flash was programmed.
    pub(crate) fn export_cartridge(&self) -> Result<Option<Vec<u8>>, String> {
        match self.call_machine(MachineCommand::ExportCartridge)? {
            MachineResponse::CartridgeImage(image) => Ok(image),
            other => Err(format!(
                "bridge returned wrong response for ExportCartridge: {other:?}"
            )),
        }
    }

    /// What a serial-bus printer printed, as text; `None` if nothing was.
    pub(crate) fn export_printer(&self) -> Result<Option<Vec<u8>>, String> {
        match self.call_machine(MachineCommand::ExportPrinter)? {
            MachineResponse::PrinterText(text) => Ok(text),
            other => Err(format!(
                "bridge returned wrong response for ExportPrinter: {other:?}"
            )),
        }
    }

    /// Where the printer's text goes when this machine is dropped
    /// (`--printer-out`).
    pub(crate) fn set_printer_out(&mut self, path: PathBuf) {
        self.printer_out = Some(path);
    }

    fn finish_printer_on_exit(&mut self) {
        let Some(path) = self.printer_out.take() else {
            return;
        };
        match self.export_printer() {
            Ok(Some(text)) => match write_file_atomically(&path, &text) {
                Ok(()) => eprintln!("saved the printer output to {}", path.display()),
                Err(e) => eprintln!(
                    "FAILED to save the printer output to {}: {e}",
                    path.display()
                ),
            },
            Ok(None) => eprintln!("nothing was printed, so {} was not written", path.display()),
            Err(e) => eprintln!("could not read back the printer output: {e}"),
        }
    }

    /// Chooses where a programmed cartridge goes when this machine is
    /// dropped (`--cartridge-save`).
    pub(crate) fn set_cartridge_exit_policy(&mut self, policy: CartridgeExitPolicy) {
        self.cartridge_exit = Some(policy);
    }

    /// Writes a programmed EasyFlash to its `--cartridge-save` target, or
    /// warns that the changes are being discarded. Silent for every machine
    /// and cartridge whose flash was never written.
    fn finish_cartridge_on_exit(&mut self) {
        let Some(policy) = self.cartridge_exit.take() else {
            return;
        };
        let Ok(Some(image)) = self.export_cartridge() else {
            return;
        };
        match policy.save_to {
            Some(path) => match write_file_atomically(&path, &image) {
                Ok(()) => eprintln!("saved the programmed cartridge to {}", path.display()),
                Err(e) => eprintln!(
                    "FAILED to save the programmed cartridge to {}: {e}",
                    path.display()
                ),
            },
            None => eprintln!(
                "the cartridge's flash was programmed but not saved -- run with \
                 --cartridge-save <file.crt> to keep the changes"
            ),
        }
    }

    /// Chooses where the cassette recording goes when this machine is
    /// dropped (`--tape-save`).
    pub(crate) fn set_tape_exit_policy(&mut self, policy: TapeExitPolicy) {
        self.tape_exit = Some(policy);
    }

    /// Everything the program wrote to the cassette output line, as a tape
    /// file of the given format (`cdt`, `tzx` or `voc`); `None` when it wrote
    /// nothing.
    pub(crate) fn export_tape(&self, format: &str) -> Result<Option<Vec<u8>>, String> {
        match self.call_machine(MachineCommand::ExportTape {
            format: format.to_string(),
        })? {
            MachineResponse::TapeImage(image) => Ok(image),
            other => Err(format!(
                "bridge returned wrong response for ExportTape: {other:?}"
            )),
        }
    }

    /// Writes the cassette recording to its `--tape-save` target. Unlike a
    /// modified disk, a missing recording is reported too: asking for a tape
    /// file and silently getting none would look like a lost save. Reports
    /// on stderr -- it runs from `Drop`.
    fn finish_tape_on_exit(&mut self) {
        let Some(policy) = self.tape_exit.take() else {
            return;
        };
        match self.export_tape(&policy.format) {
            Ok(Some(image)) => match write_file_atomically(&policy.save_to, &image) {
                Ok(()) => eprintln!(
                    "saved the cassette recording to {} ({} bytes)",
                    policy.save_to.display(),
                    image.len()
                ),
                Err(e) => eprintln!(
                    "FAILED to save the cassette recording to {}: {e}",
                    policy.save_to.display()
                ),
            },
            Ok(None) => eprintln!(
                "nothing was recorded to the cassette output, so {} was not written",
                policy.save_to.display()
            ),
            Err(e) => eprintln!("could not read back the cassette recording: {e}"),
        }
    }

    /// Hands a program to the machine plugin as `MediaKind::Program` (the
    /// plugin decides how to start it -- e.g. a BASIC program is injected
    /// once the machine has booted) instead of poking it and jumping to it.
    pub(crate) fn attach_program(&mut self, org: u16, bytes: &[u8]) -> Result<(), String> {
        match self.call_machine(MachineCommand::AttachMedia {
            slot: MediaSlot::Other("program".to_string()),
            kind: MediaKind::Program,
            name: None,
            bytes: bytes.to_vec(),
            options: vec![NamedValue {
                key: "org".to_string(),
                value: DebugValue::U64(u64::from(org)),
            }],
        })? {
            MachineResponse::Ack => Ok(()),
            other => Err(format!("program attach: unexpected response {other:?}")),
        }
    }

    pub(crate) fn apply_media(
        &mut self,
        extension: &str,
        bytes: &[u8],
        tape_speed: u32,
    ) -> Result<(), String> {
        self.backend
            .apply_media(self.instance, extension, bytes, tape_speed)
    }
}

impl Machine for HostedMachine {
    fn clock_hz(&self) -> u64 {
        match self.call_machine(MachineCommand::GetClockHz) {
            Ok(MachineResponse::ClockHz(v)) => v,
            Ok(_) => panic!("bridge returned wrong response for GetClockHz"),
            Err(e) => panic!("bridge GetClockHz failed: {e}"),
        }
    }

    fn frame_hz(&self) -> u64 {
        match self.call_machine(MachineCommand::GetFrameHz) {
            Ok(MachineResponse::FrameHz(v)) => v,
            Ok(_) => panic!("bridge returned wrong response for GetFrameHz"),
            Err(e) => panic!("bridge GetFrameHz failed: {e}"),
        }
    }

    fn run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> machine::RunStats {
        machine::Clock::new(self.frame_hz()).run(self, running)
    }

    fn step(&mut self) -> u32 {
        // Fast path -- see `PluginHost::step_fast`'s own doc comment. No
        // `MachineCommand`/bincode round trip: this is the dominant cost
        // this project's own architecture investigation found (every
        // emulated CPU instruction was paying a full heap-allocating
        // serialize+deserialize round trip even in bundled mode, where no
        // real process/address-space boundary exists to justify one).
        self.backend.step_fast(self.instance)
    }

    fn halted(&self) -> bool {
        self.backend.halted_fast(self.instance)
    }

    fn pc(&self) -> u64 {
        self.backend.pc_fast(self.instance)
    }

    fn registers(&self) -> Vec<(&'static str, u64)> {
        HostedMachine::registers(self)
    }

    fn render_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
        match self.call_machine(MachineCommand::RenderFrame) {
            Ok(MachineResponse::Frame { width, height, rgb }) => Some((width, height, rgb)),
            Ok(_) => None,
            Err(_) => None,
        }
    }

    fn read_memory(&self, addr: u64) -> u8 {
        match self.call_machine(MachineCommand::ReadMemory {
            address: addr,
            len: 1,
            space: None,
        }) {
            Ok(MachineResponse::Memory(bytes)) => bytes.first().copied().unwrap_or(0xFF),
            Ok(_) => 0xFF,
            Err(_) => 0xFF,
        }
    }

    fn load(&mut self, addr: u64, bytes: &[u8]) -> usize {
        match self.call_machine(MachineCommand::LoadBytes {
            address: addr,
            bytes: bytes.to_vec(),
        }) {
            Ok(MachineResponse::BytesWritten(w)) => w as usize,
            Ok(_) => 0,
            Err(_) => 0,
        }
    }

    fn set_pc(&mut self, addr: u64) {
        let _ = self.call_machine(MachineCommand::SetPc { address: addr });
    }

    fn request_interrupt(&mut self) {
        let _ = self.call_machine(MachineCommand::RequestInterrupt);
    }

    fn drain_audio(&mut self) -> Vec<f32> {
        match self.call_machine(MachineCommand::DrainAudio) {
            Ok(MachineResponse::AudioPcm {
                channels,
                sample_rate: AUDIO_SAMPLE_RATE,
                samples,
            }) => downmix_to_mono(channels, samples),
            Ok(_) => Vec::new(),
            Err(_) => Vec::new(),
        }
    }

    fn save_state(&self) -> Vec<u8> {
        match self.call_machine(MachineCommand::SaveState) {
            Ok(MachineResponse::StateBlob(bytes)) => bytes,
            _ => Vec::new(),
        }
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        match self.call_machine(MachineCommand::LoadState {
            bytes: bytes.to_vec(),
        })? {
            MachineResponse::Ack => Ok(()),
            _ => Err("bridge returned wrong response for LoadState".to_string()),
        }
    }

    fn reset(&mut self) {
        let _ = self.call_machine(MachineCommand::Reset);
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::c_void;
    use std::path::Path;

    use plugin_api::{
        ABI_MAJOR_V1, ABI_MINOR_V1, CpuFamily, MACHINE_PROFILE_CONTRACT_VERSION, MachineCapability,
        MachineDebugCapability, MachineInstanceHandle, MachinePluginVTableV1, MachineProfile,
        MediaKind, PluginDescriptorV1, PluginKind, RcCallResult, RcSlice, RcStatus, StatusCode,
    };

    use super::{
        ListProfilesRequest, PluginHost, PluginV1MachineBackend, chip_blob_u16_pairs,
        validate_profile_contract, validate_profile_contract_with_version,
    };

    #[test]
    fn write_file_atomically_replaces_the_target_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("retroclawd-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("disk.dsk");
        std::fs::write(&target, b"old contents").unwrap();

        super::write_file_atomically(&target, b"new contents").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new contents");
        assert!(
            !dir.join("disk.dsk.tmp").exists(),
            "temp file must be renamed away"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_file_atomically_fails_cleanly_into_a_missing_directory() {
        let target = std::env::temp_dir()
            .join(format!("retroclawd-missing-{}", std::process::id()))
            .join("nope")
            .join("disk.dsk");
        assert!(super::write_file_atomically(&target, b"x").is_err());
    }

    fn chip_blob_u16_pairs_decodes_little_endian_pairs() {
        let widths: [u16; 4] = [2168, 855, 1710, 65535];
        let bytes: Vec<u8> = widths.iter().flat_map(|w| w.to_le_bytes()).collect();
        let blobs = vec![plugin_api::BinaryBlob {
            key: "tape_pulse_widths".to_string(),
            bytes,
        }];

        assert_eq!(chip_blob_u16_pairs(&blobs, "tape_pulse_widths"), widths);
    }

    fn chip_blob_u16_pairs_returns_empty_for_missing_key() {
        assert_eq!(
            chip_blob_u16_pairs(&[], "tape_pulse_widths"),
            Vec::<u16>::new()
        );
    }

    fn chip_blob_u16_pairs_drops_a_trailing_odd_byte() {
        let blobs = vec![plugin_api::BinaryBlob {
            key: "tape_pulse_widths".to_string(),
            bytes: vec![0x78, 0x08, 0xFF], // one full pair (0x0878 = 2168) + a stray byte
        }];

        assert_eq!(chip_blob_u16_pairs(&blobs, "tape_pulse_widths"), vec![2168]);
    }
    /// Merged (every check kept): `chip_blob_u16_pairs_decodes_little_endian_pairs`, `chip_blob_u16_pairs_returns_empty_for_missing_key`, `chip_blob_u16_pairs_drops_a_trailing_odd_byte`.
    #[test]
    fn chip_blob_u16_pairs_decode_cases() {
        chip_blob_u16_pairs_decodes_little_endian_pairs();
        chip_blob_u16_pairs_returns_empty_for_missing_key();
        chip_blob_u16_pairs_drops_a_trailing_odd_byte();
    }

    unsafe extern "C" fn test_list_profiles(_req: RcSlice) -> RcCallResult {
        RcCallResult::error(StatusCode::Unsupported)
    }

    unsafe extern "C" fn test_create_instance(_req: RcSlice) -> RcCallResult {
        RcCallResult::error(StatusCode::Unsupported)
    }

    unsafe extern "C" fn test_destroy_instance(_instance: MachineInstanceHandle) -> RcStatus {
        RcStatus::ok()
    }

    unsafe extern "C" fn test_instance_call(
        _instance: MachineInstanceHandle,
        _req: RcSlice,
    ) -> RcCallResult {
        RcCallResult::error(StatusCode::Unsupported)
    }

    unsafe extern "C" fn test_free_buffer(_buffer: plugin_api::RcBuffer) {}

    extern "C" fn test_step(_instance: MachineInstanceHandle) -> u32 {
        0
    }

    extern "C" fn test_halted(_instance: MachineInstanceHandle) -> u8 {
        1
    }

    extern "C" fn test_pc(_instance: MachineInstanceHandle) -> u16 {
        0
    }

    extern "C" fn test_pc_wide(_instance: MachineInstanceHandle) -> u64 {
        0
    }

    static TEST_VTABLE: MachinePluginVTableV1 = MachinePluginVTableV1 {
        size: MachinePluginVTableV1::expected_size(),
        list_profiles: test_list_profiles,
        create_instance: test_create_instance,
        destroy_instance: test_destroy_instance,
        instance_call: test_instance_call,
        free_buffer: test_free_buffer,
        step: test_step,
        halted: test_halted,
        pc: test_pc,
        pc_wide: test_pc_wide,
    };

    fn test_descriptor(abi_major: u16, abi_minor: u16) -> PluginDescriptorV1 {
        PluginDescriptorV1 {
            abi_major,
            abi_minor,
            kind: PluginKind::Machine,
            id: RcSlice::empty(),
            version: RcSlice::empty(),
            vtable: (&TEST_VTABLE as *const MachinePluginVTableV1).cast::<c_void>(),
        }
    }

    fn profile_with_debug_contract() -> MachineProfile {
        MachineProfile {
            machine_id: "zx81".to_string(),
            display_name: "zx81".to_string(),
            cpu_family: CpuFamily::Z80,
            default_region: Some("pal".to_string()),
            contract_version: MACHINE_PROFILE_CONTRACT_VERSION,
            debug_capabilities: vec![
                MachineDebugCapability::PauseResume,
                MachineDebugCapability::StepInstruction,
                MachineDebugCapability::Breakpoints,
                MachineDebugCapability::Registers,
                MachineDebugCapability::AddressSpaces,
                MachineDebugCapability::MemoryRead,
                MachineDebugCapability::MemoryWrite,
                MachineDebugCapability::Disassembly,
                MachineDebugCapability::MachineMetadata,
                MachineDebugCapability::MediaSnapshot,
                MachineDebugCapability::ChipSnapshots,
            ],
            capabilities: vec![MachineCapability::Video, MachineCapability::Debug],
            media_kinds: vec![MediaKind::Snapshot],
            options: Vec::new(),
            key_names: Vec::new(),
            cpus: Vec::new(),
            axis_names: Vec::new(),
        }
    }

    fn descriptor_compat_accepts_newer_minor_on_same_major() {
        let descriptor = test_descriptor(ABI_MAJOR_V1, ABI_MINOR_V1 + 1);
        PluginHost::validate_descriptor_compatibility(&descriptor, ABI_MAJOR_V1, ABI_MINOR_V1)
            .expect("newer plugin minor should be accepted");
    }

    fn descriptor_compat_rejects_older_minor_against_newer_host() {
        let descriptor = test_descriptor(ABI_MAJOR_V1, 1);
        let err = PluginHost::validate_descriptor_compatibility(&descriptor, ABI_MAJOR_V1, 2)
            .expect_err("older plugin minor must be rejected by newer host");
        assert_eq!(
            err,
            "machine plugin ABI minor too old: host requires >= 2, plugin=1"
        );
    }

    fn descriptor_compat_rejects_major_mismatch() {
        let descriptor = test_descriptor(ABI_MAJOR_V1 + 1, ABI_MINOR_V1);
        let err =
            PluginHost::validate_descriptor_compatibility(&descriptor, ABI_MAJOR_V1, ABI_MINOR_V1)
                .expect_err("major mismatch must fail");
        assert_eq!(err, "machine plugin ABI major mismatch: host=1, plugin=2");
    }
    /// Merged (every check kept): `descriptor_compat_accepts_newer_minor_on_same_major`, `descriptor_compat_rejects_older_minor_against_newer_host`, `descriptor_compat_rejects_major_mismatch`.
    #[test]
    fn descriptor_compat() {
        descriptor_compat_accepts_newer_minor_on_same_major();
        descriptor_compat_rejects_older_minor_against_newer_host();
        descriptor_compat_rejects_major_mismatch();
    }

    // ---- Version skew between host and plugin (policy: docs/plugin-abi.md) ----

    /// What a future plugin (newer ABI minor) looks like to this host: the
    /// vtable it knows about, then fields appended after it.
    #[repr(C)]
    struct FutureVTable {
        base: MachinePluginVTableV1,
        appended_in_a_later_minor: usize,
    }

    static FUTURE_VTABLE: FutureVTable = FutureVTable {
        base: MachinePluginVTableV1 {
            size: std::mem::size_of::<FutureVTable>() as u32,
            list_profiles: test_list_profiles,
            create_instance: test_create_instance,
            destroy_instance: test_destroy_instance,
            instance_call: test_instance_call,
            free_buffer: test_free_buffer,
            step: test_step_future,
            halted: test_halted,
            pc: test_pc,
            pc_wide: test_pc_wide,
        },
        appended_in_a_later_minor: 0xDEAD_BEEF,
    };

    extern "C" fn test_step_future(_instance: MachineInstanceHandle) -> u32 {
        4242
    }

    /// An ABI-minor-0 plugin: the vtable ended after `free_buffer`.
    #[repr(C)]
    struct MinorZeroVTable {
        size: u32,
        list_profiles: plugin_api::abi::MachineListProfilesFnV1,
        create_instance: plugin_api::abi::MachineCreateInstanceFnV1,
        destroy_instance: plugin_api::abi::MachineDestroyInstanceFnV1,
        instance_call: plugin_api::abi::MachineInstanceCallFnV1,
        free_buffer: plugin_api::abi::FreeBufferFnV1,
    }

    static MINOR_ZERO_VTABLE: MinorZeroVTable = MinorZeroVTable {
        size: std::mem::size_of::<MinorZeroVTable>() as u32,
        list_profiles: test_list_profiles,
        create_instance: test_create_instance,
        destroy_instance: test_destroy_instance,
        instance_call: test_instance_call,
        free_buffer: test_free_buffer,
    };

    fn descriptor_with(vtable: *const c_void, abi_minor: u16) -> PluginDescriptorV1 {
        PluginDescriptorV1 {
            abi_major: ABI_MAJOR_V1,
            abi_minor,
            kind: PluginKind::Machine,
            id: RcSlice::empty(),
            version: RcSlice::empty(),
            vtable,
        }
    }

    #[test]
    fn a_newer_plugin_with_a_larger_vtable_loads_and_its_known_fields_work() {
        let descriptor = descriptor_with(
            (&FUTURE_VTABLE as *const FutureVTable).cast::<c_void>(),
            ABI_MINOR_V1 + 1,
        );
        let host = PluginHost::from_descriptor(&descriptor)
            .expect("a newer minor with a larger vtable must load");
        assert_eq!(host.step_fast(1), 4242);
        assert!(FUTURE_VTABLE.appended_in_a_later_minor != 0);
    }

    #[test]
    fn an_older_minor_plugin_is_refused_without_reading_its_short_vtable() {
        // The host's vtable has three more fields than this plugin's; the
        // refusal must come from the version, never from reading past it.
        let descriptor = descriptor_with(
            (&MINOR_ZERO_VTABLE as *const MinorZeroVTable).cast::<c_void>(),
            0,
        );
        let Err(err) = PluginHost::from_descriptor(&descriptor) else {
            panic!("minor 0 must be refused by an ABI-minor-1 host");
        };
        assert_eq!(
            err,
            "machine plugin ABI minor too old: host requires >= 2, plugin=0"
        );
    }

    #[test]
    fn a_minor_one_plugin_without_pc_wide_is_refused_by_a_minor_two_host() {
        // Minor 1 ended after `pc`; this host calls `pc_wide`, so it must
        // never be allowed to read the missing field.
        let descriptor = descriptor_with(
            (&MINOR_ZERO_VTABLE as *const MinorZeroVTable).cast::<c_void>(),
            1,
        );
        let Err(err) = PluginHost::from_descriptor(&descriptor) else {
            panic!("minor 1 must be refused by an ABI-minor-2 host");
        };
        assert_eq!(
            err,
            "machine plugin ABI minor too old: host requires >= 2, plugin=1"
        );
    }

    #[test]
    fn a_plugin_that_claims_the_right_minor_but_ships_a_short_vtable_is_refused() {
        // Reading a host-sized struct from the short one would run past it, so
        // the loader trusts the `size` field before using any later field.
        let err = plugin_api::compat::check_vtable_size(
            "machine",
            MachinePluginVTableV1::expected_size(),
            MINOR_ZERO_VTABLE.size,
        )
        .unwrap_err();
        assert!(err.contains("vtable too small"), "{err}");
    }

    unsafe extern "C" fn skewed_list_profiles(_req: RcSlice) -> RcCallResult {
        // A plugin built against a different protocol: right envelope, the
        // version a newer or older schema would write.
        let bytes = plugin_api::encode(&plugin_api::ProtocolEnvelope {
            protocol_version: plugin_api::PROTOCOL_VERSION + 1,
            payload: ListProfilesRequest {
                include_experimental: true,
            },
        })
        .unwrap();
        RcCallResult::ok_bytes(bytes)
    }

    unsafe extern "C" fn freeing_free_buffer(buffer: plugin_api::RcBuffer) {
        // SAFETY: produced by `RcBuffer::from_vec` in this process.
        let _ = unsafe { buffer.into_vec() };
    }

    #[test]
    fn a_plugin_speaking_another_protocol_version_is_reported_as_such() {
        let vtable = MachinePluginVTableV1 {
            list_profiles: skewed_list_profiles,
            free_buffer: freeing_free_buffer,
            ..TEST_VTABLE
        };
        let host = PluginHost { vtable };
        let err = host
            .list_profiles(ListProfilesRequest {
                include_experimental: false,
            })
            .expect_err("a protocol skew must be an error");
        assert!(err.contains("protocol version mismatch"), "{err}");
        assert!(
            err.contains(&format!("expected {}", plugin_api::PROTOCOL_VERSION)),
            "{err}"
        );
    }

    #[test]
    fn validate_profile_contract_accepts_full_debug_contract() {
        let profile = profile_with_debug_contract();
        let warning = validate_profile_contract(
            &profile,
            "retroclawd.test.machine",
            "0.1.0",
            Path::new("plugin.dll"),
        )
        .expect("validation should pass");
        assert!(warning.is_none());
    }

    fn validate_profile_contract_rejects_newer_plugin_contract_for_old_host() {
        let mut profile = profile_with_debug_contract();
        profile.contract_version = MACHINE_PROFILE_CONTRACT_VERSION + 1;

        let err = validate_profile_contract(
            &profile,
            "retroclawd.test.machine",
            "0.1.0",
            Path::new("plugin.dll"),
        )
        .expect_err("newer plugin contract must fail against old host");
        assert_eq!(
            err,
            "machine profile contract mismatch for zx81 from plugin.dll (retroclawd.test.machine@0.1.0): host=2, plugin=3"
        );
    }

    fn validate_profile_contract_rejects_older_plugin_contract_for_new_host() {
        let profile = profile_with_debug_contract();
        let err = validate_profile_contract_with_version(
            &profile,
            "retroclawd.test.machine",
            "0.1.0",
            Path::new("plugin.dll"),
            MACHINE_PROFILE_CONTRACT_VERSION + 1,
        )
        .expect_err("older plugin contract must fail against newer host");
        assert_eq!(
            err,
            "machine profile contract mismatch for zx81 from plugin.dll (retroclawd.test.machine@0.1.0): host=3, plugin=2"
        );
    }
    /// Merged (every check kept): `validate_profile_contract_rejects_newer_plugin_contract_for_old_host`, `validate_profile_contract_rejects_older_plugin_contract_for_new_host`.
    #[test]
    fn validate_profile_contract_rejects_other_contract_versions() {
        validate_profile_contract_rejects_newer_plugin_contract_for_old_host();
        validate_profile_contract_rejects_older_plugin_contract_for_new_host();
    }

    #[test]
    fn validate_profile_contract_warns_when_debug_table_without_debug_capability() {
        let mut profile = profile_with_debug_contract();
        profile
            .capabilities
            .retain(|cap| *cap != MachineCapability::Debug);
        profile.debug_capabilities = vec![MachineDebugCapability::PauseResume];

        let warning = validate_profile_contract(
            &profile,
            "retroclawd.test.machine",
            "0.1.0",
            Path::new("plugin.dll"),
        )
        .expect("validation should emit warning, not error");
        assert_eq!(
            warning,
            Some(
                "profile zx81 declares debug capability table but is missing MachineCapability::Debug"
                    .to_string()
            )
        );
    }

    #[test]
    fn validate_profile_contract_rejects_missing_required_debug_capability() {
        let mut profile = profile_with_debug_contract();
        profile
            .debug_capabilities
            .retain(|cap| *cap != MachineDebugCapability::Disassembly);

        let err = validate_profile_contract(
            &profile,
            "retroclawd.test.machine",
            "0.1.0",
            Path::new("plugin.dll"),
        )
        .expect_err("missing required debug capability must fail");
        assert_eq!(
            err,
            "profile zx81 from plugin.dll (retroclawd.test.machine@0.1.0) is missing required debug contract capabilities: disassembly"
        );
    }

    #[test]
    fn a_library_that_is_not_a_plugin_is_skipped_with_a_warning_and_nothing_else_is_loaded() {
        // Machines are only ever loaded from libraries: a file that merely
        // has the right name must not bring the (test-only) linked machines
        // back, it is reported and the catalog stays empty.
        let dir = std::env::temp_dir().join(format!(
            "retroclawd_plugin_bogus_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp plugin dir");
        let bogus = dir.join(format!(
            "cpc_machine_plugin.{}",
            super::plugin_file_extension()
        ));
        std::fs::write(&bogus, b"not a real dynamic library").expect("write bogus plugin file");

        // The discovery is given the file directly: no process-wide environment variable is touched, so no other
        // test (several initialise the shared machine catalog) can see this bogus library.
        let result = super::discover_runtime_catalog_from(vec![bogus], false, vec![dir.clone()]);
        std::fs::remove_dir_all(&dir).ok();

        let Err(error) = result else {
            panic!("a bogus library provides no machines");
        };
        assert!(error.contains("no machine plugins found"), "{error}");
        assert!(
            error.contains("failed to load machine plugin library"),
            "{error}"
        );
    }

    /// The real thing: a machine shared library built by a family repository
    /// is a complete plugin that exports its entry point and brings its
    /// machines' profiles with it. This workspace builds no machine library,
    /// so the test runs only when `RETROCLAWD_TEST_MACHINE_LIBRARY` names one
    /// (the family repositories' CI points it at their own build).
    #[test]
    fn a_built_machine_library_loads_on_its_own_and_lists_its_machines() {
        let Some(library) =
            std::env::var_os("RETROCLAWD_TEST_MACHINE_LIBRARY").map(std::path::PathBuf::from)
        else {
            eprintln!("skipped: RETROCLAWD_TEST_MACHINE_LIBRARY is not set");
            return;
        };
        assert!(library.is_file(), "{} is not a file", library.display());
        let backend = PluginV1MachineBackend::load_from_library(&library)
            .expect("the built library is a loadable plugin");
        let profiles = backend
            .list_profiles(ListProfilesRequest {
                include_experimental: false,
            })
            .expect("profiles")
            .profiles;
        assert!(!profiles.is_empty(), "{library:?} offers no machine");
    }

    #[test]
    fn a_plugin_error_message_reaches_the_host_instead_of_a_bare_status_code() {
        use plugin_api::MachineCreateRequest;
        let mut machine = super::create_hosted_machine(MachineCreateRequest {
            machine_id: "stub-machine".to_string(),
            region: Some("pal".to_string()),
            roms: Vec::new(),
            options: Vec::new(),
        })
        .expect("the bundled stub machine");
        let error = machine.apply_disk("nib", b"anything").unwrap_err();
        assert!(
            error.contains(".nib"),
            "the plugin's own explanation: {error}"
        );
        assert!(!error.contains("machine plugin call failed"), "{error}");
    }

    #[test]
    fn a_plugin_crash_report_is_told_apart_from_an_ordinary_error() {
        assert!(super::is_crash_error(
            "the plugin crashed earlier (panic in step: index out of bounds)"
        ));
        assert!(super::is_crash_error("panic in instance_call: boom"));
        assert!(!super::is_crash_error(
            "unsupported media kind for zx81 plugin"
        ));
        // A healthy plugin has no crash message.
        let machine =
            crate::plugin_bridge::create_hosted_machine(plugin_api::MachineCreateRequest {
                machine_id: "stub-machine".to_string(),
                region: Some("pal".to_string()),
                roms: Vec::new(),
                options: Vec::new(),
            })
            .unwrap();
        assert_eq!(machine.crash_message(), None);
    }
}
