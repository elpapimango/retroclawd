//! The application's side of the debugger boundary: answers the requests a
//! debugger module makes (`DebuggerHostRequest`) from the emulation thread's
//! shared state and command channel.

use std::sync::Arc;

use plugin_api::{DebugValue, DisassemblyLine, NamedValue, RegisterField};

use super::HostApi;
use super::api::{
    DebuggerHostCommand, DebuggerHostRequest, DebuggerHostResponse, DebuggerInstanceInfo,
    DebuggerRunMode, DebuggerStateSnapshot, DebuggerWatch,
};
use crate::debugger::{
    BreakpointDetail, DebuggerCommand, DebuggerCommandSender, DebuggerState, DebuggerStateHandle,
    DebuggerTimingCounters, InstructionHistoryEntry, RunMode,
};

pub(super) const PRIMARY_INSTANCE_ID: &str = "primary";

/// The failure reasons a host command can report; rendered into the `String`
/// error a debugger module sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostError {
    InvalidRequest,
    NotFound,
    Internal,
}

impl HostError {
    fn label(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid request",
            Self::NotFound => "not found",
            Self::Internal => "internal",
        }
    }
}

pub(super) struct HostApiContext {
    pub(super) commands: DebuggerCommandSender,
    pub(super) state: DebuggerStateHandle,
    pub(super) machine_id: String,
}

impl HostApi for HostApiContext {
    fn request(&self, request: DebuggerHostRequest) -> Result<DebuggerHostResponse, String> {
        let result = match request {
            DebuggerHostRequest::ListInstances => Ok(DebuggerHostResponse::Instances(vec![
                DebuggerInstanceInfo {
                    instance_id: PRIMARY_INSTANCE_ID.to_string(),
                    machine_id: self.machine_id.clone(),
                },
            ])),
            DebuggerHostRequest::GetState { instance_id } => {
                if instance_id != PRIMARY_INSTANCE_ID {
                    Err(HostError::NotFound)
                } else {
                    let state = self.state.lock().unwrap();
                    Ok(DebuggerHostResponse::State(Box::new(state_snapshot(
                        &state,
                    ))))
                }
            }
            DebuggerHostRequest::Command {
                instance_id,
                command,
            } => {
                if instance_id != PRIMARY_INSTANCE_ID {
                    Err(HostError::NotFound)
                } else {
                    execute_host_command(self, command)
                }
            }
        };
        result.map_err(|err| format!("host call failed: {}", err.label()))
    }
}

pub(super) fn host_api(
    commands: DebuggerCommandSender,
    state: DebuggerStateHandle,
    machine_id: String,
) -> Arc<dyn HostApi> {
    Arc::new(HostApiContext {
        commands,
        state,
        machine_id,
    })
}

fn send_command(
    commands: &DebuggerCommandSender,
    command: DebuggerCommand,
) -> Result<(), HostError> {
    commands.send(command).map_err(|_| HostError::Internal)
}

fn execute_host_command(
    ctx: &HostApiContext,
    command: DebuggerHostCommand,
) -> Result<DebuggerHostResponse, HostError> {
    match command {
        DebuggerHostCommand::Pause => {
            send_command(&ctx.commands, DebuggerCommand::Pause)?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::Resume => {
            send_command(&ctx.commands, DebuggerCommand::Resume)?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::ResetMachine => {
            send_command(&ctx.commands, DebuggerCommand::ResetMachine)?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::KeyEvent { key, pressed } => {
            send_command(&ctx.commands, DebuggerCommand::KeyEvent { key, pressed })?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::StepInstruction => {
            send_command(&ctx.commands, DebuggerCommand::StepInstruction)?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::StepOverInstruction => {
            send_command(&ctx.commands, DebuggerCommand::StepOverInstruction)?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::BackstepInstruction => {
            send_command(&ctx.commands, DebuggerCommand::BackstepInstruction)?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::SetPc { address } => {
            send_command(&ctx.commands, DebuggerCommand::SetPc(address))?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::SetRegister { name, value } => {
            send_command(&ctx.commands, DebuggerCommand::SetRegister { name, value })?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::SetBreakpoint { address, condition } => {
            if condition.is_some() {
                send_command(
                    &ctx.commands,
                    DebuggerCommand::SetBreakpointCondition {
                        addr: address,
                        condition,
                    },
                )?;
            } else {
                send_command(
                    &ctx.commands,
                    DebuggerCommand::SetBreakpoint { addr: address },
                )?;
            }
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::SetBreakpointCondition { address, condition } => {
            send_command(
                &ctx.commands,
                DebuggerCommand::SetBreakpointCondition {
                    addr: address,
                    condition,
                },
            )?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::ClearBreakpoint { address } => {
            send_command(&ctx.commands, DebuggerCommand::ClearBreakpoint(address))?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::ClearAllBreakpoints => {
            send_command(&ctx.commands, DebuggerCommand::ClearAllBreakpoints)?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::AddWatch { expr } => {
            send_command(&ctx.commands, DebuggerCommand::AddWatch(expr))?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::RemoveWatch { index } => {
            send_command(&ctx.commands, DebuggerCommand::RemoveWatch(index))?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::ClearWatches => {
            send_command(&ctx.commands, DebuggerCommand::ClearWatches)?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::SetMemoryWindow { address, len } => {
            send_command(
                &ctx.commands,
                DebuggerCommand::SetMemoryWindow { addr: address, len },
            )?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::SetDisassemblyWindow { address } => {
            send_command(
                &ctx.commands,
                DebuggerCommand::SetDisassemblyWindow(address),
            )?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::FollowPc => {
            send_command(&ctx.commands, DebuggerCommand::FollowPc)?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::SetDisassemblyLineCount { count } => {
            send_command(
                &ctx.commands,
                DebuggerCommand::SetDisassemblyLineCount(count),
            )?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::ReadMemory { address, len } => {
            let (tx, rx) = std::sync::mpsc::channel();
            send_command(
                &ctx.commands,
                DebuggerCommand::ReadMemory {
                    addr: address,
                    len,
                    reply: tx,
                },
            )?;
            let bytes = rx
                .recv_timeout(std::time::Duration::from_millis(500))
                .map_err(|_| HostError::Internal)?;
            Ok(DebuggerHostResponse::Bytes(bytes))
        }
        DebuggerHostCommand::Poke { address, bytes } => {
            send_command(
                &ctx.commands,
                DebuggerCommand::Poke {
                    addr: address,
                    bytes,
                },
            )?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::PokeBank {
            bank,
            address,
            bytes,
        } => {
            // No `machine_id` pre-check -- the machine plugin's own
            // `MachineCommand::WriteMemory` handler is the sole authority on
            // whether it supports a given `space` (here, `"ram-bank:{bank}"`,
            // see `write_memory_space`), and reports a clear error if not.
            // Delegating avoids hardcoding which machine ids support banked
            // writes here (previously `zx-spectrum` only, even though e.g.
            // CPC's own 128K/expansion RAM banking could support it too).
            let (tx, rx) = std::sync::mpsc::channel();
            send_command(
                &ctx.commands,
                DebuggerCommand::PokeBank {
                    bank,
                    addr: address,
                    bytes,
                    reply: tx,
                },
            )?;
            let result = rx
                .recv_timeout(std::time::Duration::from_millis(500))
                .map_err(|_| HostError::Internal)?;
            match result {
                Ok(()) => Ok(DebuggerHostResponse::Ack),
                Err(e) => {
                    tracing::debug!(target: "retroclawd.debugger", "banked poke failed: {e}");
                    Err(HostError::InvalidRequest)
                }
            }
        }
        DebuggerHostCommand::SetPortBreakpoint {
            port,
            mask,
            on_read,
            on_write,
        } => {
            let (tx, rx) = std::sync::mpsc::channel();
            send_command(
                &ctx.commands,
                DebuggerCommand::SetPortBreakpoint {
                    port,
                    mask,
                    on_read,
                    on_write,
                    reply: tx,
                },
            )?;
            port_breakpoint_reply(&rx)
        }
        DebuggerHostCommand::ClearPortBreakpoints => {
            let (tx, rx) = std::sync::mpsc::channel();
            send_command(
                &ctx.commands,
                DebuggerCommand::ClearPortBreakpoints { reply: tx },
            )?;
            port_breakpoint_reply(&rx)
        }
        DebuggerHostCommand::WritePort { port, value } => {
            let (tx, rx) = std::sync::mpsc::channel();
            send_command(
                &ctx.commands,
                DebuggerCommand::WritePort {
                    port,
                    value,
                    reply: tx,
                },
            )?;
            let result = rx
                .recv_timeout(std::time::Duration::from_millis(500))
                .map_err(|_| HostError::Internal)?;
            match result {
                Ok(()) => Ok(DebuggerHostResponse::Ack),
                Err(e) => {
                    tracing::debug!(target: "retroclawd.debugger", "port write failed: {e}");
                    Err(HostError::InvalidRequest)
                }
            }
        }
        DebuggerHostCommand::Assemble { addr, text } => {
            let (tx, rx) = std::sync::mpsc::channel();
            send_command(
                &ctx.commands,
                DebuggerCommand::Assemble {
                    addr,
                    text,
                    reply: tx,
                },
            )?;
            let result = rx
                .recv_timeout(std::time::Duration::from_millis(500))
                .map_err(|_| HostError::Internal)?;
            match result {
                Ok(bytes) => Ok(DebuggerHostResponse::AssembledBytes(bytes)),
                Err(e) => {
                    tracing::debug!(target: "retroclawd.debugger", "assemble failed: {e}");
                    Err(HostError::InvalidRequest)
                }
            }
        }
        DebuggerHostCommand::LoadState { bytes } => {
            send_command(&ctx.commands, DebuggerCommand::LoadState(bytes))?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::LoadSnapshot { extension, bytes } => {
            send_command(
                &ctx.commands,
                DebuggerCommand::LoadSnapshot { extension, bytes },
            )?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::SaveState => {
            let (tx, rx) = std::sync::mpsc::channel();
            send_command(&ctx.commands, DebuggerCommand::SaveState(tx))?;
            let bytes = rx
                .recv_timeout(std::time::Duration::from_millis(500))
                .map_err(|_| HostError::Internal)?;
            Ok(DebuggerHostResponse::Bytes(bytes))
        }
        DebuggerHostCommand::SaveSnapshot { extension } => {
            let (tx, rx) = std::sync::mpsc::channel();
            send_command(&ctx.commands, DebuggerCommand::SaveSnapshot(extension, tx))?;
            let bytes = rx
                .recv_timeout(std::time::Duration::from_millis(500))
                .map_err(|_| HostError::Internal)?
                .map_err(|_| HostError::InvalidRequest)?;
            Ok(DebuggerHostResponse::Bytes(bytes))
        }
        DebuggerHostCommand::GetTimingCounters => {
            let (tx, rx) = std::sync::mpsc::channel::<DebuggerTimingCounters>();
            send_command(&ctx.commands, DebuggerCommand::GetTimingCounters(tx))?;
            let counters = rx
                .recv_timeout(std::time::Duration::from_millis(500))
                .map_err(|_| HostError::Internal)?;
            Ok(DebuggerHostResponse::Bytes(
                format!("{} {}", counters.total_cycles, counters.partial_cycles).into_bytes(),
            ))
        }
        DebuggerHostCommand::ResetTimingPartial => {
            send_command(&ctx.commands, DebuggerCommand::ResetTimingPartial)?;
            Ok(DebuggerHostResponse::Ack)
        }
        DebuggerHostCommand::GetOverlaySnapshot => {
            let (tx, rx) = std::sync::mpsc::channel();
            send_command(&ctx.commands, DebuggerCommand::GetOverlaySnapshot(tx))?;
            let snapshot = rx
                .recv_timeout(std::time::Duration::from_millis(500))
                .map_err(|_| HostError::Internal)?;
            let payload = match snapshot {
                Some(snapshot) if !snapshot.fields.is_empty() => {
                    format_overlay_fields(&snapshot.fields)
                }
                _ => "unavailable".to_string(),
            };
            Ok(DebuggerHostResponse::Bytes(payload.into_bytes()))
        }
    }
}

/// Formats a `ChipSnapshot`'s fields as `key=value key=value ...`, in
/// whatever order the plugin reported them -- no hardcoded field-name
/// knowledge, unlike the fixed-field-list format this replaced (which only
/// ever worked for CPC's own specific PPI/keyboard/PSG field names). Any
/// machine plugin's `"overlay"` chip formats readably through this, CPC's
/// richer telemetry included (see `cpc-machine-plugin`'s own merged
/// `"overlay"` chip).
fn format_overlay_fields(fields: &[NamedValue]) -> String {
    fields
        .iter()
        .map(|field| format!("{}={}", field.key, format_debug_value(&field.value)))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn format_debug_value(value: &DebugValue) -> String {
    match value {
        DebugValue::Bool(v) => v.to_string(),
        DebugValue::U64(v) => v.to_string(),
        DebugValue::I64(v) => v.to_string(),
        DebugValue::Text(v) => v.clone(),
        DebugValue::Bytes(v) => format!("{}B", v.len()),
    }
}

fn port_breakpoint_reply(
    rx: &std::sync::mpsc::Receiver<Result<(), String>>,
) -> Result<DebuggerHostResponse, HostError> {
    let result = rx
        .recv_timeout(std::time::Duration::from_millis(500))
        .map_err(|_| HostError::Internal)?;
    match result {
        Ok(()) => Ok(DebuggerHostResponse::Ack),
        Err(e) => {
            tracing::debug!(target: "retroclawd.debugger", "port breakpoint failed: {e}");
            Err(HostError::InvalidRequest)
        }
    }
}

fn state_snapshot(state: &DebuggerState) -> DebuggerStateSnapshot {
    let breakpoints = if state.breakpoint_details.is_empty() {
        state
            .breakpoints
            .iter()
            .copied()
            .map(|addr| super::api::DebuggerBreakpoint {
                address: addr,
                condition: None,
            })
            .collect()
    } else {
        state
            .breakpoint_details
            .iter()
            .map(
                |BreakpointDetail { addr, condition }| super::api::DebuggerBreakpoint {
                    address: *addr,
                    condition: condition.clone(),
                },
            )
            .collect()
    };

    let registers = state
        .registers
        .iter()
        .map(|(name, value)| RegisterField {
            bank: "cpu".to_string(),
            name: (*name).to_string(),
            value: *value,
            width_bits: register_width_bits(name, *value),
        })
        .collect();

    let watches = state
        .watches
        .iter()
        .map(|watch| DebuggerWatch {
            expr: watch.expr.clone(),
            display: watch.display.clone(),
            value: watch.value,
            is_true: watch.is_true,
            error: watch.error.clone(),
        })
        .collect();

    let history = state
        .history
        .iter()
        .map(
            |InstructionHistoryEntry {
                 before_pc,
                 after_pc,
                 text,
             }| super::api::DebuggerHistoryEntry {
                before_pc: *before_pc,
                after_pc: *after_pc,
                text: text.clone(),
            },
        )
        .collect();

    let disassembly = state
        .disassembly
        .iter()
        .map(|(addr, bytes, text)| DisassemblyLine {
            address: *addr,
            bytes: bytes.clone(),
            text: text.clone(),
        })
        .collect();

    DebuggerStateSnapshot {
        mode: match state.mode {
            RunMode::Running => DebuggerRunMode::Running,
            RunMode::Paused => DebuggerRunMode::Paused,
        },
        halted: state.halted,
        pc: state.pc,
        registers,
        memory_addr: state.memory_addr,
        memory: state.memory.clone(),
        breakpoints,
        last_break: state.last_break,
        watches,
        history,
        reverse_depth: state.reverse_depth,
        last_event: state.last_event.clone(),
        disassembly,
        following_pc: state.following_pc,
        bitmap_palette: state.bitmap_palette.clone(),
        bitmap_defaults: state.bitmap_defaults,
        port_breakpoints: state.port_breakpoints.clone(),
        last_port_hit: state.last_port_hit,
    }
}

fn register_width_bits(name: &str, value: u64) -> u16 {
    if matches!(
        name,
        "A" | "F" | "B" | "C" | "D" | "E" | "H" | "L" | "I" | "R"
    ) {
        8
    } else if matches!(name, "PC" | "SP" | "AF" | "BC" | "DE" | "HL" | "IX" | "IY")
        || value <= 0xFFFF
    {
        16
    } else {
        64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn width_helper_prefers_explicit_8_bit_register_names() {
        assert_eq!(register_width_bits("A", 0x1234), 8);
        assert_eq!(register_width_bits("I", 0xFFFF), 8);
    }

    fn width_helper_defaults_to_16_for_common_cpu_widths() {
        assert_eq!(register_width_bits("PC", 0x1234), 16);
        assert_eq!(register_width_bits("CUSTOM", 0xFFFF), 16);
    }

    fn width_helper_uses_64_when_value_is_large_and_name_unknown() {
        assert_eq!(register_width_bits("CUSTOM", 0x1_0000), 64);
    }
    /// Merged (every check kept): `width_helper_prefers_explicit_8_bit_register_names`, `width_helper_defaults_to_16_for_common_cpu_widths`, `width_helper_uses_64_when_value_is_large_and_name_unknown`.
    #[test]
    fn width_helper() {
        width_helper_prefers_explicit_8_bit_register_names();
        width_helper_defaults_to_16_for_common_cpu_widths();
        width_helper_uses_64_when_value_is_large_and_name_unknown();
    }
}
