use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use machine::{AUDIO_SAMPLE_RATE, Addr, Machine};
use plugin_api::{
    BinaryBlob, ChipSnapshot, CpuFamily, DebugCommand, DebugResponse, DebugValue, HostMouseButton,
    ListProfilesRequest, ListProfilesResponse, MACHINE_PROFILE_CONTRACT_VERSION, MachineCapability,
    MachineCommand, MachineCreateRequest, MachineCreateResponse, MachineDebugCapability,
    MachineInstanceCallRequest, MachineInstanceCallResponse, MachineInstanceHandle,
    MachineMetadata, MachinePlugin, MachineProfile, MachineResponse, MediaKind, MediaSlot,
    NamedValue, PortHitInfo, PortWatchSpec, REQUIRED_MACHINE_DEBUG_CAPABILITIES,
};

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
    plugin: Arc<PluginBackend>,
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
    /// Where each ACSI hard disk (target, file) goes at exit (`--acsi-save`); never cloned either.
    acsi_exit: Vec<(u8, PathBuf)>,
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
            acsi_exit: Vec::new(),
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

/// One machine family, linked into the program.
struct PluginBackend {
    plugin: &'static dyn MachinePlugin,
    templates: Mutex<HashMap<MachineInstanceHandle, MachineCreateRequest>>,
    plugin_id: String,
    plugin_version: String,
    source: String,
}

impl PluginBackend {
    fn new(plugin: &'static dyn MachinePlugin) -> Self {
        Self {
            plugin,
            templates: Mutex::new(HashMap::new()),
            plugin_id: plugin.id().to_string(),
            plugin_version: plugin.version().to_string(),
            source: "built in".to_string(),
        }
    }

    fn list_profiles(&self, req: ListProfilesRequest) -> Result<ListProfilesResponse, String> {
        self.plugin.list_profiles(req)
    }

    fn create_instance(&self, req: MachineCreateRequest) -> Result<MachineCreateResponse, String> {
        let response = self.plugin.create_instance(req.clone())?;
        let mut templates = self.templates.lock().unwrap();
        templates.insert(response.instance, req);
        Ok(response)
    }

    fn destroy_instance(&self, instance: MachineInstanceHandle) -> Result<(), String> {
        let mut templates = self.templates.lock().unwrap();
        templates.remove(&instance);
        self.plugin.destroy_instance(instance)
    }

    fn instance_call(
        &self,
        instance: MachineInstanceHandle,
        req: MachineInstanceCallRequest,
    ) -> Result<MachineInstanceCallResponse, String> {
        self.plugin.instance_call(instance, req)
    }

    fn step_fast(&self, instance: MachineInstanceHandle) -> u32 {
        self.plugin.step(instance)
    }

    fn halted_fast(&self, instance: MachineInstanceHandle) -> bool {
        self.plugin.halted(instance)
    }

    fn pc_fast(&self, instance: MachineInstanceHandle) -> u64 {
        self.plugin.pc(instance)
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
    source: String,
    machine_ids: Vec<String>,
}

struct RuntimeCatalog {
    plugins: Vec<LoadedPluginSummary>,
    profiles: Vec<MachineProfile>,
    machine_backends: HashMap<String, Arc<PluginBackend>>,
    warnings: Vec<String>,
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
    source: &str,
    host_contract_version: u16,
) -> Result<Option<String>, String> {
    if profile.contract_version != host_contract_version {
        return Err(format!(
            "machine profile contract mismatch for {} from {} ({}@{}): host={}, plugin={}",
            profile.machine_id,
            source,
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
        source,
        plugin_id,
        plugin_version,
        missing_labels
    ))
}

fn validate_profile_contract(
    profile: &MachineProfile,
    plugin_id: &str,
    plugin_version: &str,
    source: &str,
) -> Result<Option<String>, String> {
    validate_profile_contract_with_version(
        profile,
        plugin_id,
        plugin_version,
        source,
        MACHINE_PROFILE_CONTRACT_VERSION,
    )
}

/// The machine families linked into this program: the ones the build selected, plus the stub
/// machine in this program's own unit tests.
fn linked_plugins() -> Vec<&'static dyn MachinePlugin> {
    #[allow(unused_mut)]
    let mut plugins: Vec<&'static dyn MachinePlugin> = crate::families::all();
    #[cfg(test)]
    plugins.push(test_machine_plugin_impl::plugin());
    plugins
}

type LoadedBackends = (
    Vec<LoadedPluginSummary>,
    Vec<MachineProfile>,
    HashMap<String, Arc<PluginBackend>>,
    Vec<String>,
);

fn load_backends(plugins: Vec<&'static dyn MachinePlugin>) -> Result<LoadedBackends, String> {
    let mut summaries = Vec::new();
    let mut profiles = Vec::new();
    let mut machine_backends: HashMap<String, Arc<PluginBackend>> = HashMap::new();
    let mut warnings = Vec::new();

    for plugin in plugins {
        let backend = Arc::new(PluginBackend::new(plugin));
        let plugin_id = backend.plugin_id.clone();
        let plugin_version = backend.plugin_version.clone();
        let source = backend.source.clone();

        let response = match backend.list_profiles(ListProfilesRequest {
            include_experimental: false,
        }) {
            Ok(response) => response,
            Err(error) => {
                warnings.push(format!(
                    "skip {plugin_id}@{plugin_version}: list_profiles failed: {error}"
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

        summaries.push(LoadedPluginSummary {
            plugin_id,
            plugin_version,
            source,
            machine_ids,
        });
    }

    Ok((summaries, profiles, machine_backends, warnings))
}

fn discover_runtime_catalog() -> Result<RuntimeCatalog, String> {
    let (mut plugins, mut profiles, machine_backends, warnings) = load_backends(linked_plugins())?;

    if profiles.is_empty() {
        let mut message =
            "no machines are built into this program (build with at least one machine family feature)"
                .to_string();
        if !warnings.is_empty() {
            message.push_str("; probe errors:\n");
            message.push_str(&warnings.join("\n"));
        }
        return Err(message);
    }

    profiles.sort_by(|a, b| a.machine_id.cmp(&b.machine_id));
    plugins.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));

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
                source: p.source.clone(),
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
        acsi_exit: Vec::new(),
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
            crate::emu_thread::MouseEvent::Axis { axis, value } => {
                MachineCommand::AxisEvent { axis, value }
            }
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

    fn apply_cartridge(
        &self,
        instance: MachineInstanceHandle,
        extension: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        self.expect_ack(
            instance,
            MachineCommand::AttachMedia {
                slot: MediaSlot::Cartridge { slot: 0 },
                kind: MediaKind::Cartridge,
                name: Some(format!("cartridge.{}", extension.to_ascii_lowercase())),
                bytes: bytes.to_vec(),
                options: Vec::new(),
            },
            "cartridge attach",
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
        if [
            "tap", "tzx", "cdt", "pzx", "csw", "voc", "wav", "t64", "cas",
        ]
        .contains(&lower.as_str())
        {
            self.apply_tape(instance, &lower, bytes, tape_speed)
        } else if lower == "dsk"
            || lower == "edsk"
            || lower == "d64"
            || lower == "d71"
            || lower == "d81"
            || lower == "g64"
            || lower == "x64"
            || lower == "nib"
            || lower == "atr"
            || lower == "xfd"
            || lower == "st"
            || lower == "msa"
        {
            self.apply_disk(instance, 0, &lower, bytes)
        } else if lower == "xex" {
            self.expect_ack(
                instance,
                MachineCommand::AttachMedia {
                    slot: MediaSlot::Other("program".to_string()),
                    kind: MediaKind::Program,
                    name: Some("program.xex".to_string()),
                    bytes: bytes.to_vec(),
                    options: Vec::new(),
                },
                "formatted program attach",
            )
        } else if [
            "cpr", "crt", "stc", "car", "20", "40", "60", "a0", "b0", "bin", "rom",
        ]
        .contains(&lower.as_str())
        {
            self.apply_cartridge(instance, &lower, bytes)
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
        start: Option<Addr>,
        lines: u16,
    ) -> Result<Vec<plugin_api::DisassemblyLine>, String> {
        match self.call_machine(MachineCommand::Debug(DebugCommand::GetDisassembly {
            start,
            lines: lines.into(),
        }))? {
            MachineResponse::Debug(DebugResponse::Disassembly(lines)) => Ok(lines),
            other => Err(format!(
                "disassembly expected Debug::Disassembly, got {other:?}"
            )),
        }
    }

    /// Width in bits of the primary CPU address space, 16 when the machine does not say.
    pub(crate) fn address_bits(&self) -> u8 {
        match self.call_machine(MachineCommand::Debug(DebugCommand::GetAddressSpaces)) {
            Ok(MachineResponse::Debug(DebugResponse::AddressSpaces(spaces))) => spaces
                .first()
                .map_or(16, |space| space.addr_width_bits.clamp(1, 64)),
            _ => 16,
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
        address: Addr,
        bytes: &[u8],
        space: Option<String>,
    ) -> Result<(), String> {
        match self.call_machine(MachineCommand::WriteMemory {
            address,
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

    pub(crate) fn frame_rate(&self) -> machine::FrameRate {
        <Self as Machine>::frame_rate(self)
    }

    pub(crate) fn step(&mut self) -> u32 {
        <Self as Machine>::step(self)
    }

    pub(crate) fn halted(&self) -> bool {
        <Self as Machine>::halted(self)
    }

    pub(crate) fn pc(&self) -> Addr {
        <Self as Machine>::pc(self)
    }

    pub(crate) fn render_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
        <Self as Machine>::render_frame(self)
    }

    pub(crate) fn read_memory(&self, addr: Addr) -> u8 {
        <Self as Machine>::read_memory(self, addr)
    }

    pub(crate) fn load(&mut self, addr: Addr, bytes: &[u8]) -> usize {
        <Self as Machine>::load(self, addr, bytes)
    }

    pub(crate) fn set_pc(&mut self, addr: Addr) {
        <Self as Machine>::set_pc(self, addr);
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

    /// A mouse event with the plugin's answer (`mouse_event` drops it, for the window's input path).
    pub(crate) fn try_mouse_event(
        &mut self,
        event: crate::emu_thread::MouseEvent,
    ) -> Result<(), String> {
        self.backend.mouse_event(self.instance, event)
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
        self.finish_acsi_on_exit();
    }

    /// The drive number the machine plugin gives the ACSI hard disk behind `target` (`ExportDisk`, `MarkDiskSaved`).
    pub(crate) const ACSI_DRIVE_BASE: u8 = 8;

    /// `--acsi-save N=FILE`: a hard disk the guest wrote to is written to `file` when the session ends.
    pub(crate) fn set_acsi_save(&mut self, target: u8, path: PathBuf) {
        self.acsi_exit.retain(|(t, _)| *t != target);
        self.acsi_exit.push((target, path));
    }

    /// Writes the hard disks that were modified; silent for one that was not.
    pub(crate) fn finish_acsi_on_exit(&mut self) {
        for (target, path) in std::mem::take(&mut self.acsi_exit) {
            let drive = Self::ACSI_DRIVE_BASE + target;
            let image = match self.export_disk(drive, true) {
                Ok(Some(image)) => image,
                Ok(None) => continue,
                Err(e) => {
                    eprintln!("could not read back the ACSI {target} hard disk image: {e}");
                    continue;
                }
            };
            match write_file_atomically(&path, &image) {
                Ok(()) => {
                    let _ = self.mark_disk_saved(drive);
                    eprintln!(
                        "saved modified ACSI {target} hard disk image to {}",
                        path.display()
                    );
                }
                Err(e) => eprintln!(
                    "FAILED to save the modified ACSI {target} hard disk image to {}: {e}",
                    path.display()
                ),
            }
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

    /// Hands a program file to a machine plugin that declares `MediaKind::Program` and runs executables itself (the
    /// Atari ST builds a boot floppy around a GEMDOS `.PRG`); `name` is the file's own, for the name on that floppy.
    pub(crate) fn attach_program_file(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        match self.call_machine(MachineCommand::AttachMedia {
            slot: MediaSlot::Other("program".to_string()),
            kind: MediaKind::Program,
            name: Some(name.to_string()),
            bytes: bytes.to_vec(),
            options: Vec::new(),
        })? {
            MachineResponse::Ack => Ok(()),
            other => Err(format!("program attach: unexpected response {other:?}")),
        }
    }

    /// Hands a program to the machine plugin as `MediaKind::Program` (the
    /// plugin decides how to start it -- e.g. a BASIC program is injected
    /// once the machine has booted) instead of poking it and jumping to it.
    pub(crate) fn attach_program(&mut self, org: Addr, bytes: &[u8]) -> Result<(), String> {
        match self.call_machine(MachineCommand::AttachMedia {
            slot: MediaSlot::Other("program".to_string()),
            kind: MediaKind::Program,
            name: None,
            bytes: bytes.to_vec(),
            options: vec![NamedValue {
                key: "org".to_string(),
                value: DebugValue::U64(org),
            }],
        })? {
            MachineResponse::Ack => Ok(()),
            other => Err(format!("program attach: unexpected response {other:?}")),
        }
    }

    pub(crate) fn apply_cartridge(&mut self, extension: &str, bytes: &[u8]) -> Result<(), String> {
        self.backend
            .apply_cartridge(self.instance, extension, bytes)
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

    fn frame_rate(&self) -> machine::FrameRate {
        match self.metadata.frame_rate {
            (num, den) if num != 0 && den != 0 => machine::FrameRate { num, den },
            _ => machine::FrameRate::whole(u32::try_from(self.frame_hz()).unwrap_or(u32::MAX)),
        }
    }

    fn pixel_aspect(&self) -> (u16, u16) {
        match self.metadata.pixel_aspect {
            (0, _) | (_, 0) => (1, 1),
            aspect => aspect,
        }
    }

    fn audio_format(&self) -> machine::AudioFormat {
        machine::AudioFormat {
            channels: self.metadata.audio_channels.max(1),
            sample_rate: match self.metadata.audio_sample_rate {
                0 => AUDIO_SAMPLE_RATE,
                rate => rate,
            },
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
            // Interleaved in the layout `audio_format` reports; the GUI's
            // decoder and `--dump-audio` read it in that layout.
            Ok(MachineResponse::AudioPcm { samples, .. }) => samples,
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
    use plugin_api::{
        CpuFamily, MACHINE_PROFILE_CONTRACT_VERSION, MachineCapability, MachineDebugCapability,
        MachineProfile, MediaKind,
    };

    use super::{
        chip_blob_u16_pairs, validate_profile_contract, validate_profile_contract_with_version,
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

    #[test]
    fn validate_profile_contract_accepts_full_debug_contract() {
        let profile = profile_with_debug_contract();
        let warning = validate_profile_contract(
            &profile,
            "retroclawd.test.machine",
            "0.1.0",
            "plugin.dll",
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
            "plugin.dll",
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
            "plugin.dll",
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
            "plugin.dll",
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
            "plugin.dll",
        )
        .expect_err("missing required debug capability must fail");
        assert_eq!(
            err,
            "profile zx81 from plugin.dll (retroclawd.test.machine@0.1.0) is missing required debug contract capabilities: disassembly"
        );
    }

    #[test]
    fn an_axis_event_reaches_the_plugin_under_the_name_its_profile_reports() {
        use plugin_api::MachineCreateRequest;
        let machine = super::create_hosted_machine(MachineCreateRequest {
            machine_id: "stub-machine".to_string(),
            region: Some("pal".to_string()),
            roms: Vec::new(),
            options: Vec::new(),
        })
        .expect("the bundled stub machine");
        let axis = |name: &str| crate::emu_thread::MouseEvent::Axis {
            axis: name.to_string(),
            value: -12_000,
        };
        assert_eq!(
            machine.backend.mouse_event(machine.instance, axis("Joy1X")),
            Ok(())
        );
        let error = machine
            .backend
            .mouse_event(machine.instance, axis("Paddle9"))
            .unwrap_err();
        assert!(error.contains("Paddle9"), "{error}");
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

    /// An Atari ST `.st` image is a disk: `--game-config` media and the extension routing hand it to the disk slot.
    #[test]
    fn st_extension_is_a_disk() {
        use plugin_api::MachineCreateRequest;
        let mut machine = super::create_hosted_machine(MachineCreateRequest {
            machine_id: "stub-machine".to_string(),
            region: Some("pal".to_string()),
            roms: Vec::new(),
            options: Vec::new(),
        })
        .expect("the bundled stub machine");
        let error = machine.apply_media("st", b"anything", 1).unwrap_err();
        assert!(
            error.contains("not a disk image"),
            "went to the disk slot: {error}"
        );
    }

    #[test]
    fn msa_extension_is_a_disk() {
        use plugin_api::MachineCreateRequest;
        let mut machine = super::create_hosted_machine(MachineCreateRequest {
            machine_id: "stub-machine".to_string(),
            region: Some("pal".to_string()),
            roms: Vec::new(),
            options: Vec::new(),
        })
        .expect("the bundled stub machine");
        let error = machine.apply_media("msa", b"anything", 1).unwrap_err();
        assert!(
            error.contains("not a disk image"),
            "went to the disk slot: {error}"
        );
    }

    #[test]
    fn stc_extension_is_a_cartridge() {
        use plugin_api::MachineCreateRequest;
        let mut machine = super::create_hosted_machine(MachineCreateRequest {
            machine_id: "stub-machine".to_string(),
            region: Some("pal".to_string()),
            roms: Vec::new(),
            options: Vec::new(),
        })
        .expect("the bundled stub machine");
        let error = machine.apply_media("stc", b"anything", 1).unwrap_err();
        assert!(
            error.contains("Cartridge"),
            "went to the cartridge slot: {error}"
        );
        let error = machine.apply_media("xyz", b"anything", 1).unwrap_err();
        assert!(!error.contains("Cartridge"), "not a cartridge: {error}");
    }

    #[test]
    fn cartridge_routing_preserves_the_explicit_kind_and_effective_extension() {
        let mut machine = super::create_hosted_machine(plugin_api::MachineCreateRequest {
            machine_id: "stub-machine".to_string(),
            region: Some("pal".to_string()),
            roms: Vec::new(),
            options: Vec::new(),
        })
        .unwrap();
        // The stub rejects cartridges, so its error proves the selected command and name.
        for extension in ["prg", "A0", "crt", "cpr"] {
            let error = machine.apply_cartridge(extension, &[0; 16]).unwrap_err();
            assert!(
                error.contains(&format!("Cartridge .{}", extension.to_ascii_lowercase())),
                "{error}"
            );
        }
        for extension in [
            "20", "40", "60", "A0", "b0", "bin", "rom", "crt", "cpr", "CAR",
        ] {
            let error = machine.apply_media(extension, &[0; 16], 1).unwrap_err();
            assert!(
                error.contains(&format!("Cartridge .{}", extension.to_ascii_lowercase())),
                "{error}"
            );
        }
        let error = machine.apply_media("prg", &[0; 16], 1).unwrap_err();
        assert!(
            !error.contains("Cartridge"),
            "implicit PRG must not be reclassified: {error}"
        );
    }

    #[test]
    fn atari_media_routes_to_disk_tape_and_program_not_snapshot() {
        let mut machine = super::create_hosted_machine(plugin_api::MachineCreateRequest {
            machine_id: "stub-machine".to_string(),
            region: Some("pal".to_string()),
            roms: Vec::new(),
            options: Vec::new(),
        })
        .unwrap();
        for extension in ["ATR", "XFD"] {
            let error = machine
                .apply_media(extension, b"not a disk", 1)
                .unwrap_err();
            assert!(error.contains("not a disk image"), "{error}");
        }
        let error = machine.apply_media("CAS", b"not a tape", 1).unwrap_err();
        assert!(
            error.contains("unsupported tape format \".cas\""),
            "{error}"
        );
        let error = machine
            .apply_media("XEX", &[0xff, 0xff, 0, 0x20, 0, 0x20, 0xea], 1)
            .unwrap_err();
        assert!(error.contains("Program .xex"), "{error}");
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
