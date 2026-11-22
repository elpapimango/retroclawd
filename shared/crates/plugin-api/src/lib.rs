pub mod execution_mode;
pub mod guard;
pub mod plugin;
pub mod protocol;
#[cfg(feature = "testkit")]
pub mod testkit;
#[cfg(feature = "testkit")]
pub use testkit::{decode_message, encode_message};

pub use plugin::{FnPlugin, MachineInstanceHandle, MachinePlugin};
pub use execution_mode::ExecutionMode;
pub use protocol::{
    AddressSpaceInfo, BinaryBlob, BitmapExplorerDefaults, BitmapPaletteColor, CartridgeStatus,
    ChipSnapshot, CpuFamily, CpuInfo, DebugCommand, DebugResponse, DebugValue, DisassemblyLine,
    DiskStatus, HostMouseButton, ListProfilesRequest, ListProfilesResponse,
    MACHINE_PROFILE_CONTRACT_VERSION, MachineCapability, MachineCommand, MachineCreateRequest,
    MachineCreateResponse, MachineDebugCapability, MachineInstanceCallRequest,
    MachineInstanceCallResponse, MachineKeyInfo, MachineMetadata, MachineOptionInfo,
    MachineOptionKind, MachineProfile, MachineResponse, MediaKind, MediaSlot, MediaSnapshot,
    NamedValue, PROTOCOL_VERSION, PortHitInfo, PortWatchSpec, ProtocolEnvelope,
    REQUIRED_MACHINE_DEBUG_CAPABILITIES, RegisterField, RegisterSnapshot, StepResult, TapeStatus,
    key_infos, normalize_key_name, set_key_chars,
};
