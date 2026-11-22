//! What a machine family offers the host: a [`MachinePlugin`], linked into the program.
//!
//! There is no foreign-function boundary and no wire format: the host holds a
//! `&'static dyn MachinePlugin` per family and calls it with the typed
//! requests of `protocol.rs`. A family exports its plugin as one `static`
//! [`FnPlugin`].

use crate::protocol::{
    ListProfilesRequest, ListProfilesResponse, MachineCreateRequest, MachineCreateResponse,
    MachineInstanceCallRequest, MachineInstanceCallResponse,
};

/// Handle of one machine instance inside a plugin.
pub type MachineInstanceHandle = u64;

/// A family of machines.
pub trait MachinePlugin: Sync + Send {
    /// Stable identifier, e.g. `retroclawd.neogeo.machine`.
    fn id(&self) -> &'static str;
    /// Version tag of the build.
    fn version(&self) -> &'static str;
    fn list_profiles(&self, req: ListProfilesRequest) -> Result<ListProfilesResponse, String>;
    fn create_instance(&self, req: MachineCreateRequest) -> Result<MachineCreateResponse, String>;
    fn destroy_instance(&self, instance: MachineInstanceHandle) -> Result<(), String>;
    fn instance_call(
        &self,
        instance: MachineInstanceHandle,
        req: MachineInstanceCallRequest,
    ) -> Result<MachineInstanceCallResponse, String>;
    /// Hot path: one CPU instruction, returns the cycles it took.
    fn step(&self, instance: MachineInstanceHandle) -> u32;
    /// Hot path: whether the machine stopped.
    fn halted(&self, instance: MachineInstanceHandle) -> bool;
    /// Hot path: the program counter at full width.
    fn pc(&self, instance: MachineInstanceHandle) -> u64;
}

type InstanceCallFn = fn(
    MachineInstanceHandle,
    MachineInstanceCallRequest,
) -> Result<MachineInstanceCallResponse, String>;

/// A [`MachinePlugin`] made of plain functions, so a family writes no trait impl.
pub struct FnPlugin {
    pub id: &'static str,
    pub version: &'static str,
    pub list_profiles: fn(ListProfilesRequest) -> Result<ListProfilesResponse, String>,
    pub create_instance: fn(MachineCreateRequest) -> Result<MachineCreateResponse, String>,
    pub destroy_instance: fn(MachineInstanceHandle) -> Result<(), String>,
    pub instance_call: InstanceCallFn,
    pub step: fn(MachineInstanceHandle) -> u32,
    pub halted: fn(MachineInstanceHandle) -> bool,
    pub pc: fn(MachineInstanceHandle) -> u64,
}

impl MachinePlugin for FnPlugin {
    fn id(&self) -> &'static str {
        self.id
    }
    fn version(&self) -> &'static str {
        self.version
    }
    fn list_profiles(&self, req: ListProfilesRequest) -> Result<ListProfilesResponse, String> {
        (self.list_profiles)(req)
    }
    fn create_instance(&self, req: MachineCreateRequest) -> Result<MachineCreateResponse, String> {
        (self.create_instance)(req)
    }
    fn destroy_instance(&self, instance: MachineInstanceHandle) -> Result<(), String> {
        (self.destroy_instance)(instance)
    }
    fn instance_call(
        &self,
        instance: MachineInstanceHandle,
        req: MachineInstanceCallRequest,
    ) -> Result<MachineInstanceCallResponse, String> {
        (self.instance_call)(instance, req)
    }
    fn step(&self, instance: MachineInstanceHandle) -> u32 {
        (self.step)(instance)
    }
    fn halted(&self, instance: MachineInstanceHandle) -> bool {
        (self.halted)(instance)
    }
    fn pc(&self, instance: MachineInstanceHandle) -> u64 {
        (self.pc)(instance)
    }
}
