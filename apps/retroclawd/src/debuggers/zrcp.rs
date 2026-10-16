//! `debug-zrcp`: a localhost TCP command interface in the style of ZEsarUX's
//! ZRCP, for Z80 machines (Spectrum and CPC), usable from DeZog and similar.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use plugin_api::{DebugValue, NamedValue};

use super::api::{
    DebuggerHostCommand, DebuggerHostRequest, DebuggerHostResponse, DebuggerInstanceInfo,
    DebuggerPluginInfo, DebuggerPluginOptionInfo, DebuggerRunMode, DebuggerStateSnapshot,
};
use super::{HostBridge, Running};

pub(super) const SELECTOR: &str = "debug-zrcp";
const DEBUG_SETTING_REPEAT_LAST_COMMAND_ON_ENTER: u32 = 1 << 4;
const MAPPED_MEMORY_ZONE_LEN: usize = 0x1_0000;
const MAPPED_MEMORY_ZONE_ID: i32 = -1;
const MAPPED_MEMORY_ZONE_ALIAS_ID: i32 = 0;
fn host_call(
    host: &HostBridge,
    request: DebuggerHostRequest,
) -> Result<DebuggerHostResponse, String> {
    host.call(request)
}

fn host_state(host: &HostBridge, instance_id: &str) -> Result<DebuggerStateSnapshot, String> {
    let response = host_call(
        host,
        DebuggerHostRequest::GetState {
            instance_id: instance_id.to_string(),
        },
    )?;
    match response {
        DebuggerHostResponse::State(state) => Ok(*state),
        _ => Err("host returned non-state response".to_string()),
    }
}

fn host_instances(host: &HostBridge) -> Result<Vec<DebuggerInstanceInfo>, String> {
    let response = host_call(host, DebuggerHostRequest::ListInstances)?;
    match response {
        DebuggerHostResponse::Instances(instances) => Ok(instances),
        _ => Err("host returned non-instances response".to_string()),
    }
}

fn is_supported_machine_id(machine_id: &str) -> bool {
    machine_id.starts_with("zx-spectrum") || machine_id.starts_with("cpc")
}

fn instance_machine_id(host: &HostBridge, instance_id: &str) -> Result<String, String> {
    let instances = host_instances(host)?;
    let instance = instances
        .into_iter()
        .find(|item| item.instance_id == instance_id)
        .ok_or_else(|| format!("instance {instance_id:?} not found"))?;
    Ok(instance.machine_id)
}
fn host_command(
    host: &HostBridge,
    instance_id: &str,
    command: DebuggerHostCommand,
) -> Result<DebuggerHostResponse, String> {
    host_call(
        host,
        DebuggerHostRequest::Command {
            instance_id: instance_id.to_string(),
            command,
        },
    )
}

fn expect_ack(result: Result<DebuggerHostResponse, String>) -> Result<(), String> {
    match result? {
        DebuggerHostResponse::Ack => Ok(()),
        other => Err(format!("host returned non-ack response: {other:?}")),
    }
}

fn host_command_bytes(
    host: &HostBridge,
    instance_id: &str,
    command: DebuggerHostCommand,
) -> Result<Vec<u8>, String> {
    match host_command(host, instance_id, command)? {
        DebuggerHostResponse::Bytes(bytes) => Ok(bytes),
        other => Err(format!("unexpected host response: {other:?}")),
    }
}

fn host_timing_counters(host: &HostBridge, instance_id: &str) -> Result<(u64, u64), String> {
    let bytes = host_command_bytes(host, instance_id, DebuggerHostCommand::GetTimingCounters)?;
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    let mut parts = text.split_whitespace();
    let total = parts
        .next()
        .ok_or_else(|| "missing total timing counter".to_string())?
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    let partial = parts
        .next()
        .ok_or_else(|| "missing partial timing counter".to_string())?
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    Ok((total, partial))
}

fn host_overlay_snapshot_text(host: &HostBridge, instance_id: &str) -> Result<String, String> {
    let bytes = host_command_bytes(host, instance_id, DebuggerHostCommand::GetOverlaySnapshot)?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

fn option<'a>(options: &'a [NamedValue], key: &str) -> Option<&'a DebugValue> {
    options
        .iter()
        .rev()
        .find(|opt| opt.key.eq_ignore_ascii_case(key))
        .map(|opt| &opt.value)
}

fn option_u16(options: &[NamedValue], key: &str) -> Result<Option<u16>, String> {
    let Some(value) = option(options, key) else {
        return Ok(None);
    };
    let parsed = match value {
        DebugValue::U64(v) => u16::try_from(*v).map_err(|_| format!("{key} out of range"))?,
        DebugValue::I64(v) => {
            if *v < 0 {
                return Err(format!("{key} out of range"));
            }
            u16::try_from(*v as u64).map_err(|_| format!("{key} out of range"))?
        }
        DebugValue::Text(text) => parse_u16(text).map_err(|_| format!("invalid {key} value"))?,
        DebugValue::Bool(_) | DebugValue::Bytes(_) => {
            return Err(format!("invalid {key} value"));
        }
    };
    Ok(Some(parsed))
}

fn parse_zrcp_number(token: &str) -> Result<u64, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("empty number".to_string());
    }
    if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        return u64::from_str_radix(hex, 16).map_err(|e| e.to_string());
    }
    if let Some(hex) = token.strip_prefix('$') {
        return u64::from_str_radix(hex, 16).map_err(|e| e.to_string());
    }
    if let Some(hex) = token.strip_suffix('H').or_else(|| token.strip_suffix('h')) {
        return u64::from_str_radix(hex, 16).map_err(|e| e.to_string());
    }
    token.parse::<u64>().map_err(|e| e.to_string())
}

fn parse_u16(token: &str) -> Result<u16, String> {
    let value = parse_zrcp_number(token)?;
    u16::try_from(value).map_err(|_| format!("u16 out of range: {token}"))
}

fn parse_usize(token: &str) -> Result<usize, String> {
    let value = parse_zrcp_number(token)?;
    usize::try_from(value).map_err(|_| format!("usize out of range: {token}"))
}

fn parse_hex_bytes(token: &str) -> Result<Vec<u8>, String> {
    let filtered: String = token
        .chars()
        .filter(|c| !matches!(c, ' ' | '\t' | ',' | '_'))
        .collect();
    if filtered.is_empty() {
        return Ok(Vec::new());
    }
    if !filtered.len().is_multiple_of(2) {
        return Err("hex byte string must have even length".to_string());
    }
    let mut out = Vec::with_capacity(filtered.len() / 2);
    let bytes = filtered.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let pair = std::str::from_utf8(&bytes[i..i + 2]).map_err(|e| e.to_string())?;
        out.push(u8::from_str_radix(pair, 16).map_err(|e| e.to_string())?);
        i += 2;
    }
    Ok(out)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (idx, byte) in bytes.iter().enumerate() {
        if idx != 0 {
            out.push(' ');
        }
        out.push_str(&format!("{byte:02X}"));
    }
    out
}

fn bytes_to_compact_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push_str(&format!("{byte:02X}"));
    }
    out
}

fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        let mut x = (crc ^ u32::from(byte)) & 0xFF;
        for _ in 0..8 {
            x = if (x & 1) != 0 {
                0xEDB8_8320 ^ (x >> 1)
            } else {
                x >> 1
            };
        }
        crc = (crc >> 8) ^ x;
    }
    !crc
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BreakpointSlot {
    address: u16,
    condition: Option<String>,
    break_action: bool,
    pass_count_limit: Option<usize>,
    pass_count_current: usize,
}

struct ZrcpSession {
    breakpoints_enabled: bool,
    slots: HashMap<usize, BreakpointSlot>,
    installed_slots: HashMap<usize, BreakpointSlot>,
    debug_settings: u32,
    verbose_level: u8,
    ui_io_ports: [u8; 9],
    mem_breakpoints: Vec<u8>,
    current_memory_zone: i32,
}

impl Default for ZrcpSession {
    fn default() -> Self {
        Self {
            breakpoints_enabled: false,
            slots: HashMap::new(),
            installed_slots: HashMap::new(),
            debug_settings: 0,
            verbose_level: 0,
            ui_io_ports: [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00],
            mem_breakpoints: vec![0; MAPPED_MEMORY_ZONE_LEN],
            current_memory_zone: MAPPED_MEMORY_ZONE_ID,
        }
    }
}

impl ZrcpSession {
    fn repeat_last_command_on_enter(&self) -> bool {
        self.debug_settings & DEBUG_SETTING_REPEAT_LAST_COMMAND_ON_ENTER != 0
    }
}

#[derive(Default)]
struct RunArgs {
    verbose: bool,
    limit: Option<usize>,
    no_stop_on_data: bool,
    update_immediately: bool,
}

fn parse_run_args(parts: impl Iterator<Item = String>) -> Result<RunArgs, String> {
    let mut args = RunArgs::default();
    for token in parts {
        if token.eq_ignore_ascii_case("verbose") {
            args.verbose = true;
            continue;
        }
        if token.eq_ignore_ascii_case("no-stop-on-data") {
            args.no_stop_on_data = true;
            continue;
        }
        if token.eq_ignore_ascii_case("update-immediately") {
            args.update_immediately = true;
            continue;
        }
        if args.limit.is_some() {
            return Err(format!("unexpected run argument {token:?}"));
        }
        let limit = parse_usize(&token)
            .map_err(|_| format!("invalid run limit {token:?} (expected integer)"))?;
        args.limit = Some(limit);
    }
    Ok(args)
}

fn parse_zrcp_breakpoint_condition(condition: &str) -> Result<(u16, Option<String>), String> {
    let compact: String = condition.chars().filter(|c| !c.is_whitespace()).collect();
    let upper = compact.to_ascii_uppercase();
    let value = upper
        .strip_prefix("PC==")
        .or_else(|| upper.strip_prefix("PC="))
        .ok_or_else(|| {
            "only PC=<addr> breakpoint conditions are supported by debug-zrcp right now".to_string()
        })?;
    let address = parse_u16(value)?;
    Ok((address, None))
}

fn sync_slot_breakpoint(
    host: &HostBridge,
    instance_id: &str,
    session: &mut ZrcpSession,
    index: usize,
) -> Result<(), String> {
    let desired = session
        .slots
        .get(&index)
        .filter(|slot| {
            session.breakpoints_enabled && slot.break_action && slot.pass_count_limit.is_none()
        })
        .cloned();
    let installed = session.installed_slots.get(&index).cloned();

    if let Some(installed_slot) = installed {
        if desired.as_ref() == Some(&installed_slot) {
            return Ok(());
        }
        expect_ack(host_command(
            host,
            instance_id,
            DebuggerHostCommand::ClearBreakpoint {
                address: installed_slot.address,
            },
        ))?;
        session.installed_slots.remove(&index);
    }

    if let Some(slot) = desired {
        expect_ack(host_command(
            host,
            instance_id,
            DebuggerHostCommand::SetBreakpoint {
                address: slot.address,
                condition: slot.condition.clone(),
            },
        ))?;
        session.installed_slots.insert(index, slot);
    }

    Ok(())
}

fn sync_all_breakpoints(
    host: &HostBridge,
    instance_id: &str,
    session: &mut ZrcpSession,
) -> Result<(), String> {
    let mut indices: Vec<usize> = session
        .slots
        .keys()
        .chain(session.installed_slots.keys())
        .copied()
        .collect();
    indices.sort_unstable();
    indices.dedup();
    for index in indices {
        sync_slot_breakpoint(host, instance_id, session, index)?;
    }
    Ok(())
}
fn has_non_host_breakpoint_slots(session: &ZrcpSession) -> bool {
    session.slots.values().any(|slot| {
        session.breakpoints_enabled && slot.break_action && slot.pass_count_limit.is_some()
    })
}

fn plain_breakpoint_hit_pc(
    session: &ZrcpSession,
    state: &DebuggerStateSnapshot,
) -> Result<Option<u16>, String> {
    if !session.breakpoints_enabled {
        return Ok(None);
    }
    for (slot_index, slot) in ordered_slot_entries(session) {
        if !slot.break_action || slot.pass_count_limit.is_some() || slot.address != state.pc {
            continue;
        }
        if let Some(condition) = slot.condition.as_deref() {
            let matched = evaluate_condition_expression(state, condition)
                .map_err(|e| format!("invalid breakpoint condition in slot {slot_index}: {e}"))?;
            if !matched {
                continue;
            }
        }
        return Ok(Some(state.pc));
    }
    Ok(None)
}

fn software_breakpoint_slot_indices_for_pc(session: &ZrcpSession, pc: u16) -> Vec<usize> {
    let mut indices: Vec<usize> = session
        .slots
        .iter()
        .filter_map(|(index, slot)| {
            (session.breakpoints_enabled
                && slot.break_action
                && slot.address == pc
                && slot.pass_count_limit.is_some())
            .then_some(*index)
        })
        .collect();
    indices.sort_unstable();
    indices
}

fn software_breakpoint_fired_for_slot(session: &mut ZrcpSession, index: usize) -> bool {
    let Some(slot) = session.slots.get_mut(&index) else {
        return false;
    };
    slot.pass_count_current = slot.pass_count_current.saturating_add(1);
    match slot.pass_count_limit {
        Some(limit) => slot.pass_count_current >= limit,
        None => true,
    }
}

fn software_breakpoint_hit_pc(session: &mut ZrcpSession, pc: u16) -> Option<u16> {
    for index in software_breakpoint_slot_indices_for_pc(session, pc) {
        if software_breakpoint_fired_for_slot(session, index) {
            return Some(pc);
        }
    }
    None
}

fn parse_memory_zone_selector(token: &str) -> Result<i32, String> {
    let trimmed = token.trim();
    if trimmed.eq_ignore_ascii_case("mapped") || trimmed.eq_ignore_ascii_case("mapped-memory") {
        return Ok(MAPPED_MEMORY_ZONE_ID);
    }
    match trimmed {
        "-1" => Ok(MAPPED_MEMORY_ZONE_ID),
        "0" => Ok(MAPPED_MEMORY_ZONE_ALIAS_ID),
        _ => Err("only zones -1/0 (mapped memory) are supported".to_string()),
    }
}

fn prompt_for_state(state: &DebuggerStateSnapshot) -> &'static str {
    match state.mode {
        DebuggerRunMode::Paused => "command@cpu-step> ",
        DebuggerRunMode::Running => "command> ",
    }
}

fn query_prompt(host: &HostBridge, instance_id: &str) -> String {
    match host_state(host, instance_id) {
        Ok(state) => prompt_for_state(&state).to_string(),
        Err(_) => "command> ".to_string(),
    }
}

fn format_register_value(value: u64, width_bits: u16) -> String {
    let digits = usize::from(width_bits.max(4)).div_ceil(4);
    format!("{value:0digits$X}H")
}

fn register_lookup(state: &DebuggerStateSnapshot, name: &str) -> Option<(String, u64, u16)> {
    if name.eq_ignore_ascii_case("PC") {
        return Some(("PC".to_string(), u64::from(state.pc), 16));
    }
    state
        .registers
        .iter()
        .find(|reg| reg.name.eq_ignore_ascii_case(name))
        .map(|reg| (reg.name.clone(), reg.value, reg.width_bits))
}

fn evaluate_atom(state: &DebuggerStateSnapshot, token: &str) -> Result<u64, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("empty expression token".to_string());
    }
    if let Some((_, value, _)) = register_lookup(state, token) {
        return Ok(value);
    }
    parse_zrcp_number(token)
}

fn evaluate_condition_expression(
    state: &DebuggerStateSnapshot,
    expr: &str,
) -> Result<bool, String> {
    for op in ["==", "!=", ">=", "<=", ">", "<"] {
        if let Some((lhs, rhs)) = expr.split_once(op) {
            let lhs = evaluate_atom(state, lhs)?;
            let rhs = evaluate_atom(state, rhs)?;
            return Ok(match op {
                "==" => lhs == rhs,
                "!=" => lhs != rhs,
                ">=" => lhs >= rhs,
                "<=" => lhs <= rhs,
                ">" => lhs > rhs,
                "<" => lhs < rhs,
                _ => unreachable!(),
            });
        }
    }
    Ok(evaluate_atom(state, expr)? != 0)
}

fn machine_cpu_name(machine_id: &str) -> &'static str {
    if machine_id.starts_with("zx-spectrum") || machine_id.starts_with("cpc") {
        "z80"
    } else if machine_id.starts_with("c64") {
        "mos6502"
    } else {
        "unknown"
    }
}

fn machine_clock_hz(machine_id: &str) -> Option<u64> {
    if machine_id == "zx-spectrum" {
        // The 48K's real crystal-derived CPU clock (the machine runs an effective 3.4944 MHz so that a frame is exactly 69888 T).
        Some(3_500_000)
    } else if machine_id.starts_with("zx-spectrum") {
        Some(3_546_900)
    } else if machine_id.starts_with("cpc") {
        Some(4_000_000)
    } else if machine_id.starts_with("c64") {
        Some(985_248)
    } else {
        None
    }
}

fn format_registers_line(state: &DebuggerStateSnapshot) -> String {
    if state.registers.is_empty() {
        return "(no registers)".to_string();
    }
    state
        .registers
        .iter()
        .map(|reg| {
            format!(
                "{}={}",
                reg.name,
                format_register_value(reg.value, reg.width_bits)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_disassembly_lines(state: &DebuggerStateSnapshot, max_lines: usize) -> String {
    if state.disassembly.is_empty() {
        return "(no disassembly)".to_string();
    }
    state
        .disassembly
        .iter()
        .take(max_lines)
        .map(|line| {
            format!(
                "{:04X}: {:<12} {}",
                line.address,
                bytes_to_hex(&line.bytes),
                line.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn format_state_brief(state: &DebuggerStateSnapshot) -> String {
    let mut out = format!(
        "PC={:04X}H mode={} halted={}",
        state.pc,
        match state.mode {
            DebuggerRunMode::Running => "running",
            DebuggerRunMode::Paused => "paused",
        },
        if state.halted { "yes" } else { "no" }
    );
    if let Some(line) = state.disassembly.first() {
        out.push_str(&format!("\n{:04X} {}", line.address, line.text));
    }
    if !state.registers.is_empty() {
        out.push_str("\nregs:");
        for reg in &state.registers {
            out.push_str(&format!(" {}={:X}", reg.name, reg.value));
        }
    }
    out
}

fn parse_optional_index_and_items(
    args: &[String],
) -> Result<(Option<usize>, Option<usize>), String> {
    let index = args.first().map(|token| parse_usize(token)).transpose()?;
    let items = args.get(1).map(|token| parse_usize(token)).transpose()?;
    if args.len() > 2 {
        return Err("too many arguments".to_string());
    }
    Ok((index, items))
}

fn ordered_slot_entries(session: &ZrcpSession) -> Vec<(usize, &BreakpointSlot)> {
    let mut entries: Vec<(usize, &BreakpointSlot)> = session
        .slots
        .iter()
        .map(|(index, slot)| (*index, slot))
        .collect();
    entries.sort_by_key(|(index, _)| *index);
    entries
}

fn slice_slot_entries<'a>(
    entries: &'a [(usize, &'a BreakpointSlot)],
    index: Option<usize>,
    items: Option<usize>,
) -> &'a [(usize, &'a BreakpointSlot)] {
    if entries.is_empty() {
        return entries;
    }
    let start = index.unwrap_or(entries[0].0);
    let start_pos = entries
        .iter()
        .position(|(slot, _)| *slot >= start)
        .unwrap_or(entries.len());
    let capped_items = items.unwrap_or(entries.len().saturating_sub(start_pos));
    let end = start_pos.saturating_add(capped_items).min(entries.len());
    &entries[start_pos..end]
}

fn host_read_memory_range(
    host: &HostBridge,
    instance_id: &str,
    start: u16,
    len: usize,
) -> Result<Vec<u8>, String> {
    if len == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(len);
    let mut addr = start;
    let mut remaining = len;
    while remaining > 0 {
        let chunk_len = remaining.min(usize::from(u16::MAX));
        let chunk_len_u16 = u16::try_from(chunk_len).map_err(|_| "chunk length overflow")?;
        match host_command(
            host,
            instance_id,
            DebuggerHostCommand::ReadMemory {
                address: addr,
                len: chunk_len_u16,
            },
        )? {
            DebuggerHostResponse::Bytes(bytes) => {
                if bytes.len() != chunk_len {
                    return Err(format!(
                        "host returned {} bytes, expected {chunk_len}",
                        bytes.len()
                    ));
                }
                out.extend_from_slice(&bytes);
            }
            other => return Err(format!("unexpected host response: {other:?}")),
        }
        addr = addr.wrapping_add(chunk_len_u16);
        remaining -= chunk_len;
    }
    Ok(out)
}

fn host_poke_bytes(
    host: &HostBridge,
    instance_id: &str,
    start: u16,
    bytes: &[u8],
) -> Result<(), String> {
    if bytes.is_empty() {
        return Ok(());
    }
    let mut addr = start;
    for chunk in bytes.chunks(0x2000) {
        expect_ack(host_command(
            host,
            instance_id,
            DebuggerHostCommand::Poke {
                address: addr,
                bytes: chunk.to_vec(),
            },
        ))?;
        let advance = u16::try_from(chunk.len()).map_err(|_| "poke chunk too large")?;
        addr = addr.wrapping_add(advance);
    }
    Ok(())
}

fn file_extension(path: &str) -> Result<String, String> {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
        .ok_or_else(|| format!("path {path:?} has no file extension"))
}

fn send_host_key_event(
    host: &HostBridge,
    instance_id: &str,
    key: &str,
    pressed: bool,
) -> Result<(), String> {
    expect_ack(host_command(
        host,
        instance_id,
        DebuggerHostCommand::KeyEvent {
            key: key.to_string(),
            pressed,
        },
    ))
}

fn util_key_code_to_key_name(key_code: u64) -> Option<&'static str> {
    match key_code {
        128 => Some("Space"),
        129 => Some("Enter"),
        132 => Some("Delete"),
        133 | 135 => Some("ShiftLeft"),
        134 => Some("ShiftRight"),
        136 => Some("AltLeft"),
        137 => Some("AltRight"),
        138 => Some("ControlLeft"),
        139 => Some("ControlRight"),
        140 => Some("Backspace"),
        141 | 187 | 196 | 197 | 198 => Some("Space"),
        142 | 190 => Some("ArrowLeft"),
        143 | 191 => Some("ArrowRight"),
        144 | 189 => Some("ArrowDown"),
        145 | 188 => Some("ArrowUp"),
        146 => Some("Tab"),
        147 => Some("CapsLock"),
        148 => Some("Comma"),
        149 => Some("Period"),
        150 => Some("F1"),
        151 => Some("F2"),
        152 => Some("F3"),
        153 => Some("F4"),
        154 => Some("F5"),
        155 => Some("F6"),
        156 => Some("F7"),
        157 => Some("F8"),
        158 => Some("F9"),
        159 => Some("F10"),
        160 => Some("F11"),
        161 => Some("F12"),
        165 => Some("Escape"),
        166 => Some("PageUp"),
        167 => Some("PageDown"),
        185 => Some("MetaLeft"),
        186 => Some("MetaRight"),
        _ => None,
    }
}

fn ascii_key_binding(ch: char, machine_id: Option<&str>) -> Option<(&'static str, bool)> {
    if ch == '"' && machine_id.is_some_and(|id| id.starts_with("cpc")) {
        // CPC host-key mapping currently doesn't expose a dedicated Quote key;
        // on real CPC layouts `"` is typed as Shift+2.
        return Some(("Digit2", true));
    }

    match ch {
        'a' | 'A' => Some(("KeyA", ch.is_ascii_uppercase())),
        'b' | 'B' => Some(("KeyB", ch.is_ascii_uppercase())),
        'c' | 'C' => Some(("KeyC", ch.is_ascii_uppercase())),
        'd' | 'D' => Some(("KeyD", ch.is_ascii_uppercase())),
        'e' | 'E' => Some(("KeyE", ch.is_ascii_uppercase())),
        'f' | 'F' => Some(("KeyF", ch.is_ascii_uppercase())),
        'g' | 'G' => Some(("KeyG", ch.is_ascii_uppercase())),
        'h' | 'H' => Some(("KeyH", ch.is_ascii_uppercase())),
        'i' | 'I' => Some(("KeyI", ch.is_ascii_uppercase())),
        'j' | 'J' => Some(("KeyJ", ch.is_ascii_uppercase())),
        'k' | 'K' => Some(("KeyK", ch.is_ascii_uppercase())),
        'l' | 'L' => Some(("KeyL", ch.is_ascii_uppercase())),
        'm' | 'M' => Some(("KeyM", ch.is_ascii_uppercase())),
        'n' | 'N' => Some(("KeyN", ch.is_ascii_uppercase())),
        'o' | 'O' => Some(("KeyO", ch.is_ascii_uppercase())),
        'p' | 'P' => Some(("KeyP", ch.is_ascii_uppercase())),
        'q' | 'Q' => Some(("KeyQ", ch.is_ascii_uppercase())),
        'r' | 'R' => Some(("KeyR", ch.is_ascii_uppercase())),
        's' | 'S' => Some(("KeyS", ch.is_ascii_uppercase())),
        't' | 'T' => Some(("KeyT", ch.is_ascii_uppercase())),
        'u' | 'U' => Some(("KeyU", ch.is_ascii_uppercase())),
        'v' | 'V' => Some(("KeyV", ch.is_ascii_uppercase())),
        'w' | 'W' => Some(("KeyW", ch.is_ascii_uppercase())),
        'x' | 'X' => Some(("KeyX", ch.is_ascii_uppercase())),
        'y' | 'Y' => Some(("KeyY", ch.is_ascii_uppercase())),
        'z' | 'Z' => Some(("KeyZ", ch.is_ascii_uppercase())),
        '0' => Some(("Digit0", false)),
        ')' => Some(("Digit0", true)),
        '1' => Some(("Digit1", false)),
        '!' => Some(("Digit1", true)),
        '2' => Some(("Digit2", false)),
        '@' => Some(("Digit2", true)),
        '3' => Some(("Digit3", false)),
        '#' => Some(("Digit3", true)),
        '4' => Some(("Digit4", false)),
        '$' => Some(("Digit4", true)),
        '5' => Some(("Digit5", false)),
        '%' => Some(("Digit5", true)),
        '6' => Some(("Digit6", false)),
        '^' => Some(("Digit6", true)),
        '7' => Some(("Digit7", false)),
        '&' => Some(("Digit7", true)),
        '8' => Some(("Digit8", false)),
        '*' => Some(("Digit8", true)),
        '9' => Some(("Digit9", false)),
        '(' => Some(("Digit9", true)),
        ' ' => Some(("Space", false)),
        '\n' | '\r' => Some(("Enter", false)),
        '\t' => Some(("Tab", false)),
        ',' => Some(("Comma", false)),
        '<' => Some(("Comma", true)),
        '.' => Some(("Period", false)),
        '>' => Some(("Period", true)),
        '/' => Some(("Slash", false)),
        '?' => Some(("Slash", true)),
        ';' => Some(("Semicolon", false)),
        ':' => Some(("Semicolon", true)),
        '\'' => Some(("Quote", false)),
        '"' => Some(("Quote", true)),
        '-' => Some(("Minus", false)),
        '_' => Some(("Minus", true)),
        '=' => Some(("Equal", false)),
        '+' => Some(("Equal", true)),
        '[' => Some(("BracketLeft", false)),
        '{' => Some(("BracketLeft", true)),
        ']' => Some(("BracketRight", false)),
        '}' => Some(("BracketRight", true)),
        '\\' => Some(("Backslash", false)),
        '|' => Some(("Backslash", true)),
        '`' => Some(("Backquote", false)),
        '~' => Some(("Backquote", true)),
        _ => None,
    }
}

fn send_key_stroke(
    host: &HostBridge,
    instance_id: &str,
    key: &str,
    shift: bool,
    pause_ms: u64,
) -> Result<(), String> {
    if shift {
        send_host_key_event(host, instance_id, "ShiftLeft", true)?;
    }
    send_host_key_event(host, instance_id, key, true)?;
    if pause_ms > 0 {
        thread::sleep(Duration::from_millis(pause_ms));
    }
    send_host_key_event(host, instance_id, key, false)?;
    if shift {
        send_host_key_event(host, instance_id, "ShiftLeft", false)?;
    }
    if pause_ms > 0 {
        thread::sleep(Duration::from_millis(pause_ms));
    }
    Ok(())
}

fn send_ascii_char(
    host: &HostBridge,
    instance_id: &str,
    machine_id: Option<&str>,
    ch: char,
    pause_ms: u64,
) -> Result<(), String> {
    let (key, shift) = ascii_key_binding(ch, machine_id)
        .ok_or_else(|| format!("unsupported ascii character for send-keys: {ch:?}"))?;
    send_key_stroke(host, instance_id, key, shift, pause_ms)
}

fn spectrum_row_bit_key(row: usize, bit: usize) -> Option<&'static str> {
    const MAP: [[Option<&str>; 5]; 8] = [
        [
            Some("ShiftLeft"),
            Some("KeyZ"),
            Some("KeyX"),
            Some("KeyC"),
            Some("KeyV"),
        ],
        [
            Some("KeyA"),
            Some("KeyS"),
            Some("KeyD"),
            Some("KeyF"),
            Some("KeyG"),
        ],
        [
            Some("KeyQ"),
            Some("KeyW"),
            Some("KeyE"),
            Some("KeyR"),
            Some("KeyT"),
        ],
        [
            Some("Digit1"),
            Some("Digit2"),
            Some("Digit3"),
            Some("Digit4"),
            Some("Digit5"),
        ],
        [
            Some("Digit0"),
            Some("Digit9"),
            Some("Digit8"),
            Some("Digit7"),
            Some("Digit6"),
        ],
        [
            Some("KeyP"),
            Some("KeyO"),
            Some("KeyI"),
            Some("KeyU"),
            Some("KeyY"),
        ],
        [
            Some("Enter"),
            Some("KeyL"),
            Some("KeyK"),
            Some("KeyJ"),
            Some("KeyH"),
        ],
        [
            Some("Space"),
            None,
            Some("KeyM"),
            Some("KeyN"),
            Some("KeyB"),
        ],
    ];
    MAP.get(row)
        .and_then(|bits| bits.get(bit).copied())
        .flatten()
}

fn apply_spectrum_ui_io_ports(
    host: &HostBridge,
    instance_id: &str,
    session: &mut ZrcpSession,
    raw: &[u8; 9],
) -> Result<(), String> {
    for (row, &raw_byte) in raw.iter().enumerate().take(8) {
        let prev = session.ui_io_ports[row] & 0x1F;
        let next = raw_byte & 0x1F;
        for bit in 0..5 {
            let mask = 1u8 << bit;
            let prev_pressed = (prev & mask) == 0;
            let next_pressed = (next & mask) == 0;
            if prev_pressed == next_pressed {
                continue;
            }
            let Some(key) = spectrum_row_bit_key(row, bit) else {
                continue;
            };
            send_host_key_event(host, instance_id, key, next_pressed)?;
        }
    }

    let prev_joy = session.ui_io_ports[8] & 0x1F;
    let next_joy = raw[8] & 0x1F;
    let joystick_map = [
        (0x01u8, "ArrowRight"),
        (0x02u8, "ArrowLeft"),
        (0x04u8, "ArrowDown"),
        (0x08u8, "ArrowUp"),
        (0x10u8, "Space"),
    ];
    for (mask, key) in joystick_map {
        let prev_pressed = (prev_joy & mask) != 0;
        let next_pressed = (next_joy & mask) != 0;
        if prev_pressed == next_pressed {
            continue;
        }
        send_host_key_event(host, instance_id, key, next_pressed)?;
    }

    session.ui_io_ports = *raw;
    Ok(())
}

fn run_limited_steps(
    host: &HostBridge,
    instance_id: &str,
    session: &mut ZrcpSession,
    limit: usize,
    verbose: bool,
    stop: &AtomicBool,
) -> Result<String, String> {
    let mut state = host_state(host, instance_id)?;
    if !matches!(state.mode, DebuggerRunMode::Paused) {
        return Err("run requires cpu-step mode (use enter-cpu-step first)".to_string());
    }

    let mut hit_breakpoint = None;
    let mut steps = 0usize;
    while steps < limit {
        if stop.load(Ordering::Relaxed) {
            return Err("server shutting down".to_string());
        }
        expect_ack(host_command(
            host,
            instance_id,
            DebuggerHostCommand::StepInstruction,
        ))?;
        steps += 1;
        state = host_state(host, instance_id)?;
        if state.halted || state.last_port_hit.is_some() {
            break;
        }

        if let Some(pc) = software_breakpoint_hit_pc(session, state.pc) {
            hit_breakpoint = Some(pc);
            break;
        }

        if let Some(pc) = plain_breakpoint_hit_pc(session, &state)? {
            hit_breakpoint = Some(pc);
            break;
        }
    }

    let mut lines = Vec::new();
    if let Some(pc) = hit_breakpoint {
        lines.push(format!("Breakpoint fired: PC={pc:04X}H"));
    } else if let Some(hit) = state.last_port_hit {
        lines.push(format!(
            "Port breakpoint fired: {}",
            host_format_port_hit(&hit)
        ));
    } else if state.halted {
        lines.push("CPU halted".to_string());
    } else {
        lines.push(format!("Run limit reached ({steps} instructions)"));
    }
    if verbose {
        lines.push(format_state_brief(&state));
    }
    Ok(format!("OK {}", lines.join("\n")))
}

fn run_until_event_by_stepping(
    host: &HostBridge,
    instance_id: &str,
    session: &mut ZrcpSession,
    verbose: bool,
    stop: &AtomicBool,
) -> Result<String, String> {
    let mut state = host_state(host, instance_id)?;
    if !matches!(state.mode, DebuggerRunMode::Paused) {
        return Err("run requires cpu-step mode (use enter-cpu-step first)".to_string());
    }

    let mut hit_breakpoint = None;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err("server shutting down".to_string());
        }
        expect_ack(host_command(
            host,
            instance_id,
            DebuggerHostCommand::StepInstruction,
        ))?;
        state = host_state(host, instance_id)?;
        if state.halted || state.last_port_hit.is_some() {
            break;
        }
        if let Some(pc) = software_breakpoint_hit_pc(session, state.pc) {
            hit_breakpoint = Some(pc);
            break;
        }
        if let Some(pc) = plain_breakpoint_hit_pc(session, &state)? {
            hit_breakpoint = Some(pc);
            break;
        }
    }

    let mut lines = Vec::new();
    if let Some(pc) = hit_breakpoint {
        lines.push(format!("Breakpoint fired: PC={pc:04X}H"));
    } else if let Some(hit) = state.last_port_hit {
        lines.push(format!(
            "Port breakpoint fired: {}",
            host_format_port_hit(&hit)
        ));
    } else if state.halted {
        lines.push("CPU halted".to_string());
    } else {
        lines.push("Run stopped".to_string());
    }
    if verbose {
        lines.push(format_state_brief(&state));
    }
    Ok(format!("OK {}", lines.join("\n")))
}

/// How long `run` waits for the host to apply `Resume` before it accepts a
/// paused state identical to the pre-resume one as a genuine stop.
const RESUME_APPLY_GRACE: Duration = Duration::from_millis(300);

/// Whether `now` -- read after sending `Resume` -- is the outcome of the run.
///
/// The host applies `Resume` on its next tick and publishes state after it, so
/// right after sending it the published state is still the old paused
/// snapshot (including its stale `last_break`/`last_port_hit`). A paused state
/// that looks exactly like the one we resumed from, seen within the grace
/// period, is that stale snapshot and not a new stop.
fn run_outcome_ready(
    before: &DebuggerStateSnapshot,
    now: &DebuggerStateSnapshot,
    waited: Duration,
) -> bool {
    if now.halted {
        return true;
    }
    if !matches!(now.mode, DebuggerRunMode::Paused) {
        return false;
    }
    let unchanged = now.pc == before.pc
        && now.last_break == before.last_break
        && now.last_port_hit == before.last_port_hit;
    !unchanged || waited >= RESUME_APPLY_GRACE
}

fn run_until_event(
    host: &HostBridge,
    instance_id: &str,
    verbose: bool,
    stop: &AtomicBool,
) -> Result<String, String> {
    let before = host_state(host, instance_id)?;
    if !matches!(before.mode, DebuggerRunMode::Paused) {
        return Err("run requires cpu-step mode (use enter-cpu-step first)".to_string());
    }

    expect_ack(host_command(host, instance_id, DebuggerHostCommand::Resume))?;

    let resumed_at = std::time::Instant::now();
    let state = loop {
        if stop.load(Ordering::Relaxed) {
            return Err("server shutting down".to_string());
        }
        thread::sleep(Duration::from_millis(20));
        let state = host_state(host, instance_id)?;
        if run_outcome_ready(&before, &state, resumed_at.elapsed()) {
            break state;
        }
    };

    let mut lines = Vec::new();
    if let Some(pc) = state.last_break {
        lines.push(format!("Breakpoint fired: PC={pc:04X}H"));
    } else if let Some(hit) = state.last_port_hit {
        lines.push(format!(
            "Port breakpoint fired: {}",
            host_format_port_hit(&hit)
        ));
    } else if state.halted {
        lines.push("CPU halted".to_string());
    } else {
        lines.push("Run stopped".to_string());
    }
    if verbose {
        lines.push(format_state_brief(&state));
    }

    Ok(format!("OK {}", lines.join("\n")))
}

/// `OUT (0x7F00),0x8D` / `IN (0xF400) -> 0xFF`.
fn host_format_port_hit(hit: &plugin_api::PortHitInfo) -> String {
    if hit.write {
        format!("OUT ({:04X}H),{:02X}H", hit.port, hit.value)
    } else {
        format!("IN ({:04X}H) -> {:02X}H", hit.port, hit.value)
    }
}

fn help_text(command: Option<&str>) -> String {
    match command.map(|cmd| cmd.to_ascii_lowercase()) {
        None => {
            "OK commands: help [command], ls, about, get-version, get-buildnumber, state, get-os, get-video-driver, pwd, cd <path>, get-current-machine, get-machines, enter-cpu-step, exit-cpu-step, cpu-step, cpu-step-over, run [verbose] [limit] [no-stop-on-data] [update-immediately], get-registers, get-register <name>, set-register <name=value>, disassemble [addr] [lines], list-breakpoints, get-breakpoints, get-breakpointsactions, get-breakpointspasscount, enable-breakpoint, disable-breakpoint, enable-breakpoints, disable-breakpoints, set-breakpoint, set-breakpointaction, set-breakpointpasscount, read-memory [addr] [len], set-memory-zone <zone>, get-memory-zones, get-current-memory-zone, get-io-ports, get-ui-io-ports, set-ui-io-ports <18-hex>, get-tstates, get-tstates-partial, reset-tstates-partial, poke <addr> <hex>, load-binary <file> <addr> <len>, save-binary <file> <addr> <len>, snapshot-load <file>, snapshot-save <file>, get-snapshot, load-state <file>, save-state <file>, get-stack-backtrace [items], cpu-history <action>, send-keys-event <keycode> <0|1>, send-keys-string <pause-ms> <text>, send-keys-ascii <pause-ms> <ascii...>, set-debug-settings <mask>, get-debug-settings, reset-cpu, quit"
                .to_string()
        }
        Some(cmd) if cmd == "run" || cmd == "r" => {
            "OK run [verbose] [limit] [no-stop-on-data] [update-immediately]\n- unlimited run: resumes until a breakpoint/halt/pause event\n- limit run: executes exactly N paused-mode single steps\n- no-stop-on-data and update-immediately are accepted for ZRCP compatibility".to_string()
        }
        Some(cmd) if cmd == "set-breakpoint" || cmd == "sb" => {
            "OK set-breakpoint <index> [PC=<addr>]\nIf condition is omitted, the slot is cleared/disabled.\nOnly PC=<addr> conditions are currently supported.".to_string()
        }
        Some(cmd) if cmd == "set-breakpointaction" || cmd == "sba" => {
            "OK set-breakpointaction <index> [break|none]\nDefault action for a newly set breakpoint is none.".to_string()
        }
        Some(cmd) if cmd == "set-register" || cmd == "sr" => {
            "OK set-register <name=value>\nSets a CPU register to a numeric value (e.g. set-register PC=4000H, set-register A=2AH)."
                .to_string()
        }
        Some(cmd) if cmd == "read-memory" || cmd == "mem" || cmd == "hexdump" || cmd == "h" => {
            "OK read-memory [addr] [len]\nNo args = full mapped memory zone (64K), addr only = one byte, len=0 also means full mapped memory."
                .to_string()
        }
        Some(cmd)
            if cmd == "get-tstates"
                || cmd == "get-tstates-partial"
                || cmd == "reset-tstates-partial" =>
        {
            "OK get-tstates | get-tstates-partial | reset-tstates-partial\nCycle counters are sourced from retroclawd's emu-thread debugger bridge.".to_string()
        }
        Some(cmd) if cmd == "get-io-ports" => {
            "OK get-io-ports\nReturns hosted-machine I/O diagnostics (CPC returns PPI/tape/FDC counters; other machines return unavailable).".to_string()
        }
        Some(cmd) if cmd == "get-memory-pages" || cmd == "gmp" => {
            "OK get-memory-pages [verbose]\nReturns mapped-memory view of the active machine.\nUse 'verbose' to include machine id in the response.".to_string()
        }
        Some(cmd) if cmd == "get-breakpoints-optimized" => {
            "OK get-breakpoints-optimized [index] [items]\nReports whether each software breakpoint slot can be handled as a direct host breakpoint.".to_string()
        }
        Some(cmd) if cmd == "set-verbose-level" => {
            "OK set-verbose-level <0..255>\nStores compatibility verbosity level for subsequent run output defaults.".to_string()
        }
        Some(cmd) if cmd == "get-crc32" => {
            "OK get-crc32 <start_address> <length>\nCalculates IEEE CRC32 over mapped memory bytes.".to_string()
        }
        Some(cmd) if cmd == "get-membreakpoints" => {
            "OK get-membreakpoints [address] [items]\nShows memory breakpoint map entries; if only address is given, reports that exact cell.".to_string()
        }
        Some(cmd) if cmd == "set-membreakpoint" => {
            "OK set-membreakpoint <address> <type> [items]\nType 0 disables, 1 read, 2 write, 3 read/write across item count.".to_string()
        }
        Some(cmd) if cmd == "clear-membreakpoints" => {
            "OK clear-membreakpoints\nClears all compatibility memory breakpoints.".to_string()
        }
        Some(cmd) if cmd == "assemble" || cmd == "a" => {
            "OK assemble <address> <instruction...>\nAssembles exactly one instruction (for the CPU the machine runs) and pokes bytes at the given address.".to_string()
        }
        Some(cmd) if cmd == "set-portbreakpoint" => {
            "OK set-portbreakpoint <port> <type> [mask]\nPauses the machine after an IN/OUT whose port matches (port & mask == given port & mask; mask defaults to FFFFH, an exact port). Type 0 removes, 1 IN, 2 OUT, 3 both. retroclawd extension, not a ZEsarUX command; errors on a machine with no I/O-port space.".to_string()
        }
        Some(cmd) if cmd == "get-portbreakpoints" => {
            "OK get-portbreakpoints\nLists the port breakpoints as `port mask : type` (type 1 IN, 2 OUT, 3 both).".to_string()
        }
        Some(cmd) if cmd == "clear-portbreakpoints" => {
            "OK clear-portbreakpoints\nRemoves every port breakpoint.".to_string()
        }
        Some(cmd) if cmd == "write-port" => {
            "OK write-port <port> <value>\nPerforms OUT (port),value on the machine's I/O bus, decoded like a CPU OUT (a machine with no I/O-port space, such as the C64, answers an error).".to_string()
        }
        Some(cmd) if cmd == "extended-stack" => {
            "OK extended-stack [depth]\nDumps N words above SP (default 10), showing the stack contents.".to_string()
        }
        Some(cmd) if cmd == "get-paging-state" => {
            "OK get-paging-state\nReturns paging state for CPC or 128K Spectrum models; returns error for 48K or non-supported machines.".to_string()
        }
        Some(cmd) => format!("ERR no help for command {cmd}"),
    }
}

fn is_known_unimplemented_remote_command(command: &str) -> bool {
    matches!(
        command,
        "ayp"
            | "ayplayer"
            | "close-all-menus"
            | "cpu-code-coverage"
            | "cpu-panic"
            | "cpu-transaction-log"
            | "dump-nested-functions"
            | "dump-scanline-buffer"
            | "esxdoshandler-get-open-files"
            | "find-label"
            | "generate-nmi"
            | "get-audio-buffer-info"
            | "get-ocr"
            | "get-text-overlay"
            | "get-visualmem-written-dump"
            | "gvmwd"
            | "get-visualmem-read-dump"
            | "gvmrd"
            | "get-visualmem-opcode-dump"
            | "gvmod"
            | "hexdump-internal"
            | "ifrom-press-button"
            | "kartusho-press-button"
            | "load-source-code"
            | "lsc"
            | "mmc-reload"
            | "open-menu"
            | "print-error"
            | "print-footer"
            | "qdos-get-open-files"
            | "qlgof"
            | "realtape-open"
            | "save-binary-internal"
            | "save-screen"
            | "set-machine"
            | "sm"
            | "set-text-brightness"
            | "set-window-zoom"
            | "smartload"
            | "sl"
            | "snapshot-inram-get-index"
            | "snapshot-inram-load"
            | "speech-empty-fifo"
            | "speech-send"
            | "tbblue-get-clipwindow"
            | "tbblue-set-clipwindow"
            | "tbblue-get-palette"
            | "tbblue-get-pattern"
            | "tbblue-get-register"
            | "tbblue-get-sprite"
            | "tbblue-set-palette"
            | "tbblue-set-pattern"
            | "tbblue-set-register"
            | "tbblue-set-sprite"
            | "tsconf-get-af-port"
            | "tsconf-set-af-port"
            | "view-basic"
            | "zeng-online"
            | "zo"
            | "zxevo-get-nvram"
            | "zeng-is-master"
    )
}

struct TcpServerHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Drop for TcpServerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn spawn_server(
    host: HostBridge,
    instance_id: String,
    port: u16,
) -> Result<(TcpServerHandle, u16), String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("failed to bind debug-zrcp listener: {e}"))?;
    let bound_port = listener
        .local_addr()
        .map_err(|e| format!("failed to query debug-zrcp listener address: {e}"))?
        .port();
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("failed to configure debug-zrcp listener: {e}"))?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();

    let join = thread::Builder::new()
        .name("debug-zrcp-plugin".into())
        .spawn(move || {
            while !stop_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _addr)) => {
                        let _ = handle_client(stream, &host, &instance_id, &stop_thread);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => {
                        thread::sleep(Duration::from_millis(50));
                    }
                }
            }
        })
        .map_err(|e| format!("failed to spawn debug-zrcp thread: {e}"))?;

    Ok((
        TcpServerHandle {
            stop,
            join: Some(join),
        },
        bound_port,
    ))
}

#[derive(Debug)]
enum CommandOutcome {
    Reply(String),
    Quit,
}

fn handle_command(
    host: &HostBridge,
    instance_id: &str,
    session: &mut ZrcpSession,
    stop: &AtomicBool,
    line: &str,
) -> Result<CommandOutcome, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(CommandOutcome::Reply(help_text(None)));
    }

    let mut parts = line.split_whitespace();
    let command = parts
        .next()
        .map(|cmd| cmd.to_ascii_lowercase())
        .unwrap_or_default();
    let args: Vec<String> = parts.map(str::to_string).collect();

    match command.as_str() {
        "help" | "?" | "ls" => Ok(CommandOutcome::Reply(help_text(
            args.first().map(String::as_str),
        ))),
        "about" => Ok(CommandOutcome::Reply(format!(
            "OK retroclawd debug-zrcp v{}",
            env!("CARGO_PKG_VERSION")
        ))),
        "get-version" => Ok(CommandOutcome::Reply(format!(
            "OK {}",
            env!("CARGO_PKG_VERSION")
        ))),
        "noop" => Ok(CommandOutcome::Reply("OK".to_string())),
        "pwd" => Ok(CommandOutcome::Reply(format!(
            "OK {}",
            std::env::current_dir()
                .map_err(|e| format!("failed to read working directory: {e}"))?
                .display()
        ))),
        "cd" => {
            let Some(path) = args.first() else {
                return Err("cd requires a path".to_string());
            };
            std::env::set_current_dir(path)
                .map_err(|e| format!("failed to change working directory: {e}"))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "get-os" => Ok(CommandOutcome::Reply(format!(
            "OK {}",
            std::env::consts::OS
        ))),
        "get-current-machine" | "gcm" => {
            let machine_id = instance_machine_id(host, instance_id)?;
            Ok(CommandOutcome::Reply(format!("OK {machine_id}")))
        }
        "get-machines" => {
            let mut machines: Vec<String> = host_instances(host)?
                .into_iter()
                .map(|entry| entry.machine_id)
                .collect();
            machines.sort();
            machines.dedup();
            if machines.is_empty() {
                Ok(CommandOutcome::Reply("OK (none)".to_string()))
            } else {
                Ok(CommandOutcome::Reply(format!("OK {}", machines.join("; "))))
            }
        }
        "get-buildnumber" => Ok(CommandOutcome::Reply(format!(
            "OK {}",
            option_env!("SOURCE_DATE_EPOCH").unwrap_or("0")
        ))),
        "get-video-driver" => Ok(CommandOutcome::Reply("OK retroclawd".to_string())),
        "get-cpu-core-name" => {
            let machine_id = instance_machine_id(host, instance_id)?;
            Ok(CommandOutcome::Reply(format!(
                "OK {}",
                machine_cpu_name(&machine_id)
            )))
        }
        "get-cpu-frequency" => {
            let machine_id = instance_machine_id(host, instance_id)?;
            let Some(clock_hz) = machine_clock_hz(&machine_id) else {
                return Err(format!("no cpu frequency mapping for machine {machine_id}"));
            };
            Ok(CommandOutcome::Reply(format!("OK {clock_hz}")))
        }
        "get-cpu-turbo-speed" => Ok(CommandOutcome::Reply("OK 1".to_string())),
        "get-crc32" => {
            if args.len() < 2 {
                return Err("Needs two parameters".to_string());
            }
            let start = parse_u16(&args[0])?;
            let length = parse_usize(&args[1])?;
            if length < 1 {
                return Err("Length must be >0".to_string());
            }
            let bytes = host_read_memory_range(host, instance_id, start, length)?;
            let crc = crc32_ieee(&bytes);
            Ok(CommandOutcome::Reply(format!("OK {crc:08x}")))
        }
        "get-tstates" => {
            let (total, _) = host_timing_counters(host, instance_id)?;
            Ok(CommandOutcome::Reply(format!("OK {total}")))
        }
        "get-tstates-partial" => {
            let (_, partial) = host_timing_counters(host, instance_id)?;
            if partial == u64::MAX {
                Ok(CommandOutcome::Reply("OK OVERFLOW".to_string()))
            } else {
                Ok(CommandOutcome::Reply(format!("OK {partial}")))
            }
        }
        "reset-tstates-partial" => {
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::ResetTimingPartial,
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "evaluate" | "e" => {
            let expr = line
                .split_once(char::is_whitespace)
                .map(|(_, rest)| rest.trim())
                .filter(|expr| !expr.is_empty())
                .ok_or_else(|| "evaluate requires an expression".to_string())?;
            let state = host_state(host, instance_id)?;
            let value = evaluate_atom(&state, expr)?;
            Ok(CommandOutcome::Reply(format!("OK {value} ({value:X}H)")))
        }
        "evaluate-condition" | "ec" => {
            let expr = line
                .split_once(char::is_whitespace)
                .map(|(_, rest)| rest.trim())
                .filter(|expr| !expr.is_empty())
                .ok_or_else(|| "evaluate-condition requires an expression".to_string())?;
            let state = host_state(host, instance_id)?;
            let value = evaluate_condition_expression(&state, expr)?;
            Ok(CommandOutcome::Reply(format!(
                "OK {}",
                if value { 1 } else { 0 }
            )))
        }
        "state" | "get-state" => {
            let state = host_state(host, instance_id)?;
            let json = serde_json::to_string(&state).map_err(|e| e.to_string())?;
            Ok(CommandOutcome::Reply(format!("OK {json}")))
        }
        "get-registers" | "gr" => {
            let state = host_state(host, instance_id)?;
            Ok(CommandOutcome::Reply(format!(
                "OK {}",
                format_registers_line(&state)
            )))
        }
        "get-register" | "greg" => {
            let Some(name) = args.first() else {
                return Err("get-register requires a register name".to_string());
            };
            let state = host_state(host, instance_id)?;
            let Some((reg_name, value, width_bits)) = register_lookup(&state, name) else {
                return Err(format!("unknown register {name:?}"));
            };
            Ok(CommandOutcome::Reply(format!(
                "OK {}={}",
                reg_name,
                format_register_value(value, width_bits)
            )))
        }
        "set-register" | "sr" => {
            let assignment = line
                .split_once(char::is_whitespace)
                .map(|(_, rest)| rest.trim())
                .ok_or_else(|| "set-register requires name=value".to_string())?;
            let (name, value_text) = assignment
                .split_once('=')
                .ok_or_else(|| "set-register requires name=value".to_string())?;
            let register = name.trim();
            if register.is_empty() {
                return Err("set-register requires a register name".to_string());
            }
            let value = parse_zrcp_number(value_text.trim())?;
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::SetRegister {
                    name: register.to_string(),
                    value,
                },
            ))?;
            Ok(CommandOutcome::Reply(format!(
                "OK {}={}",
                register.to_ascii_uppercase(),
                format_register_value(value, 16)
            )))
        }
        "disassemble" | "disasm" | "d" => {
            let start = args.first().map(|token| parse_u16(token)).transpose()?;
            let lines = args
                .get(1)
                .map(|token| parse_u16(token))
                .transpose()?
                .unwrap_or(8)
                .clamp(1, 200);
            if let Some(address) = start {
                expect_ack(host_command(
                    host,
                    instance_id,
                    DebuggerHostCommand::SetDisassemblyWindow { address },
                ))?;
            }
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::SetDisassemblyLineCount { count: lines },
            ))?;
            let state = host_state(host, instance_id)?;
            Ok(CommandOutcome::Reply(format!(
                "OK {}",
                format_disassembly_lines(&state, usize::from(lines))
            )))
        }
        "list-breakpoints" | "lb" => {
            let entries: Vec<String> = ordered_slot_entries(session)
                .iter()
                .map(|(index, slot)| {
                    let pass_count = slot
                        .pass_count_limit
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "none".to_string());
                    format!(
                        "{}: PC={:04X}H action={} passcount={}",
                        index,
                        slot.address,
                        if slot.break_action { "break" } else { "none" },
                        pass_count
                    )
                })
                .collect();
            if entries.is_empty() {
                Ok(CommandOutcome::Reply("OK (none)".to_string()))
            } else {
                Ok(CommandOutcome::Reply(format!("OK {}", entries.join("; "))))
            }
        }
        "get-breakpoints" | "gb" => {
            let (index, items) = parse_optional_index_and_items(&args)?;
            let entries = ordered_slot_entries(session);
            let selected = slice_slot_entries(&entries, index, items);
            if selected.is_empty() {
                return Ok(CommandOutcome::Reply("OK (none)".to_string()));
            }
            let payload = selected
                .iter()
                .map(|(slot_index, slot)| format!("{}: PC={:04X}H", slot_index, slot.address))
                .collect::<Vec<_>>()
                .join("; ");
            Ok(CommandOutcome::Reply(format!("OK {payload}")))
        }
        "get-breakpointsactions" | "gba" => {
            let (index, items) = parse_optional_index_and_items(&args)?;
            let entries = ordered_slot_entries(session);
            let selected = slice_slot_entries(&entries, index, items);
            if selected.is_empty() {
                return Ok(CommandOutcome::Reply("OK (none)".to_string()));
            }
            let payload = selected
                .iter()
                .map(|(slot_index, slot)| {
                    format!(
                        "{}: {}",
                        slot_index,
                        if slot.break_action { "break" } else { "none" }
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            Ok(CommandOutcome::Reply(format!("OK {payload}")))
        }
        "get-breakpointspasscount" | "gbpc" => {
            let (index, items) = parse_optional_index_and_items(&args)?;
            let entries = ordered_slot_entries(session);
            let selected = slice_slot_entries(&entries, index, items);
            if selected.is_empty() {
                return Ok(CommandOutcome::Reply("OK (none)".to_string()));
            }
            let payload = selected
                .iter()
                .map(|(slot_index, slot)| {
                    let configured = slot
                        .pass_count_limit
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "none".to_string());
                    format!(
                        "{}: current={} configured={configured}",
                        slot_index, slot.pass_count_current
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            Ok(CommandOutcome::Reply(format!("OK {payload}")))
        }
        "get-breakpoints-optimized" => {
            let (index, items) = parse_optional_index_and_items(&args)?;
            let entries = ordered_slot_entries(session);
            let selected = slice_slot_entries(&entries, index, items);
            if selected.is_empty() {
                return Ok(CommandOutcome::Reply("OK (none)".to_string()));
            }
            let payload = selected
                .iter()
                .map(|(slot_index, slot)| {
                    let optimized = session.breakpoints_enabled
                        && slot.break_action
                        && slot.condition.is_none()
                        && slot.pass_count_limit.is_none();
                    format!("{}: {}", slot_index, if optimized { "yes" } else { "no" })
                })
                .collect::<Vec<_>>()
                .join("; ");
            Ok(CommandOutcome::Reply(format!("OK {payload}")))
        }
        "get-memory-pages" | "gmp" => {
            let verbose = if args.is_empty() {
                false
            } else if args.len() == 1 && args[0].eq_ignore_ascii_case("verbose") {
                true
            } else {
                return Err("get-memory-pages accepts at most one argument: verbose".to_string());
            };
            let machine_id = instance_machine_id(host, instance_id)?;
            if verbose {
                Ok(CommandOutcome::Reply(format!(
                    "OK machine={machine_id} pages=mapped-memory(0000H-FFFFH)"
                )))
            } else {
                Ok(CommandOutcome::Reply(
                    "OK mapped-memory(0000H-FFFFH)".to_string(),
                ))
            }
        }
        "pause" | "enter-cpu-step" | "encs" => {
            expect_ack(host_command(host, instance_id, DebuggerHostCommand::Pause))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "resume" | "exit-cpu-step" | "ecs" => {
            expect_ack(host_command(host, instance_id, DebuggerHostCommand::Resume))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "step" | "cpu-step" | "cs" => {
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::StepInstruction,
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "step_over" | "cpu-step-over" | "cso" => {
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::StepOverInstruction,
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "backstep" | "step-back" | "cpu-step-back" => {
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::BackstepInstruction,
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "run" | "r" => {
            let run_args = parse_run_args(args.into_iter())?;
            let verbose = run_args.verbose || session.verbose_level > 0;
            let mut reply = if run_args.limit.is_some_and(|limit| limit > 0) {
                run_limited_steps(
                    host,
                    instance_id,
                    session,
                    run_args.limit.unwrap(),
                    verbose,
                    stop,
                )?
            } else if has_non_host_breakpoint_slots(session) {
                run_until_event_by_stepping(host, instance_id, session, verbose, stop)?
            } else {
                run_until_event(host, instance_id, verbose, stop)?
            };
            if run_args.no_stop_on_data || run_args.update_immediately {
                reply.push_str(
                    "\nnote: no-stop-on-data and update-immediately are accepted for compatibility only",
                );
            }
            Ok(CommandOutcome::Reply(reply))
        }
        "cpu-history" => {
            let Some(action) = args.first() else {
                return Err("cpu-history requires an action".to_string());
            };
            let action = action.to_ascii_lowercase();
            let state = host_state(host, instance_id)?;
            match action.as_str() {
                "get-size" => Ok(CommandOutcome::Reply(format!("OK {}", state.history.len()))),
                "get-max-size" => Ok(CommandOutcome::Reply(format!(
                    "OK {}",
                    state.reverse_depth.max(state.history.len())
                ))),
                "find-pc" => {
                    let Some(addr_token) = args.get(1) else {
                        return Err("cpu-history find-pc requires address".to_string());
                    };
                    let address = parse_u16(addr_token)?;
                    let pos = state
                        .history
                        .iter()
                        .position(|entry| entry.before_pc == address || entry.after_pc == address)
                        .map(|index| index.to_string())
                        .unwrap_or_else(|| "-1".to_string());
                    Ok(CommandOutcome::Reply(format!("OK {pos}")))
                }
                "get" | "get-extended" => {
                    let Some(index_token) = args.get(1) else {
                        return Err(format!("cpu-history {action} requires index"));
                    };
                    let index = parse_usize(index_token)?;
                    let Some(entry) = state.history.get(index) else {
                        return Err(format!("cpu-history index {index} out of range"));
                    };
                    if action == "get" {
                        Ok(CommandOutcome::Reply(format!(
                            "OK PC={:04X}H {}",
                            entry.before_pc, entry.text
                        )))
                    } else {
                        Ok(CommandOutcome::Reply(format!(
                            "OK before={:04X}H after={:04X}H {}",
                            entry.before_pc, entry.after_pc, entry.text
                        )))
                    }
                }
                "get-pc" => {
                    let start = args
                        .get(1)
                        .map(|token| parse_usize(token))
                        .transpose()?
                        .unwrap_or(0);
                    let items = args
                        .get(2)
                        .map(|token| parse_usize(token))
                        .transpose()?
                        .unwrap_or(1);
                    let payload = state
                        .history
                        .iter()
                        .skip(start)
                        .take(items)
                        .map(|entry| format!("{:04X}H", entry.before_pc))
                        .collect::<Vec<_>>()
                        .join(" ");
                    Ok(CommandOutcome::Reply(if payload.is_empty() {
                        "OK".to_string()
                    } else {
                        format!("OK {payload}")
                    }))
                }
                _ => Err(format!(
                    "cpu-history action {action} is not implemented in retroclawd yet"
                )),
            }
        }
        "enable-breakpoints" => {
            session.breakpoints_enabled = true;
            sync_all_breakpoints(host, instance_id, session)?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "disable-breakpoints" => {
            session.breakpoints_enabled = false;
            sync_all_breakpoints(host, instance_id, session)?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "enable-breakpoint" | "eb" => {
            let Some(index_token) = args.first() else {
                return Err("enable-breakpoint requires index".to_string());
            };
            let index = parse_usize(index_token)?;
            let Some(slot) = session.slots.get_mut(&index) else {
                return Err(format!("breakpoint slot {index} is not defined"));
            };
            slot.break_action = true;
            sync_slot_breakpoint(host, instance_id, session, index)?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "disable-breakpoint" | "db" => {
            let Some(index_token) = args.first() else {
                return Err("disable-breakpoint requires index".to_string());
            };
            let index = parse_usize(index_token)?;
            let Some(slot) = session.slots.get_mut(&index) else {
                return Err(format!("breakpoint slot {index} is not defined"));
            };
            slot.break_action = false;
            sync_slot_breakpoint(host, instance_id, session, index)?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "set-breakpoint" | "sb" => {
            let Some(index_token) = args.first() else {
                return Err("set-breakpoint requires index [condition]".to_string());
            };
            let index = parse_usize(index_token)?;
            if args.len() < 2 {
                session.slots.remove(&index);
                sync_slot_breakpoint(host, instance_id, session, index)?;
                return Ok(CommandOutcome::Reply("OK".to_string()));
            }
            let condition_text = args[1..].join(" ");
            let (address, condition) = parse_zrcp_breakpoint_condition(&condition_text)?;
            let break_action = session
                .slots
                .get(&index)
                .map(|slot| slot.break_action)
                .unwrap_or(false);
            let pass_count_limit = session
                .slots
                .get(&index)
                .and_then(|slot| slot.pass_count_limit);
            session.slots.insert(
                index,
                BreakpointSlot {
                    address,
                    condition,
                    break_action,
                    pass_count_limit,
                    pass_count_current: 0,
                },
            );
            sync_slot_breakpoint(host, instance_id, session, index)?;
            Ok(CommandOutcome::Reply(format!(
                "OK breakpoint {index} -> PC={address:04X}H action={}",
                if break_action { "break" } else { "none" }
            )))
        }
        "set-breakpointaction" | "sba" => {
            let Some(index_token) = args.first() else {
                return Err("set-breakpointaction requires index [break|none]".to_string());
            };
            let index = parse_usize(index_token)?;
            let action = args
                .get(1)
                .map(|token| token.to_ascii_lowercase())
                .unwrap_or_else(|| "none".to_string());
            let break_action = match action.as_str() {
                "break" => true,
                "none" | "disabled" | "ignore" => false,
                _ => return Err(format!("unsupported breakpoint action {action:?}")),
            };
            let Some(slot) = session.slots.get_mut(&index) else {
                return Err(format!("breakpoint slot {index} is not defined"));
            };
            slot.break_action = break_action;
            sync_slot_breakpoint(host, instance_id, session, index)?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "set-breakpointpasscount" | "sbpc" => {
            let Some(index_token) = args.first() else {
                return Err("set-breakpointpasscount requires index [pass-count]".to_string());
            };
            let index = parse_usize(index_token)?;
            let pass_count_limit = args
                .get(1)
                .map(|pass_count| parse_usize(pass_count))
                .transpose()?
                .filter(|&value| value != 0);
            let Some(slot) = session.slots.get_mut(&index) else {
                return Err(format!("breakpoint slot {index} is not defined"));
            };
            slot.pass_count_limit = pass_count_limit;
            slot.pass_count_current = 0;
            sync_slot_breakpoint(host, instance_id, session, index)?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "break" => {
            let Some(addr_token) = args.first() else {
                return Err("break requires address [condition]".to_string());
            };
            let address = parse_u16(addr_token)?;
            let condition = if args.len() > 1 {
                Some(args[1..].join(" "))
            } else {
                None
            };
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::SetBreakpoint { address, condition },
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "clear-breakpoint" | "clear-break" | "clear_break" => {
            let Some(addr_token) = args.first() else {
                return Err("clear-breakpoint requires address".to_string());
            };
            let address = parse_u16(addr_token)?;
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::ClearBreakpoint { address },
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "clear-breaks" | "clear_breaks" => {
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::ClearAllBreakpoints,
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "clear-membreakpoints" => {
            session.mem_breakpoints.fill(0);
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "get-stack-backtrace" => {
            let items = if let Some(token) = args.first() {
                parse_usize(token)?
            } else {
                5
            }
            .clamp(1, 256);
            let state = host_state(host, instance_id)?;
            let Some((_, sp_value, _)) = register_lookup(&state, "SP") else {
                return Err("stack pointer register SP is not available".to_string());
            };
            let sp = u16::try_from(sp_value & 0xFFFF).map_err(|_| "invalid SP register value")?;
            let bytes = host_read_memory_range(host, instance_id, sp, items.saturating_mul(2))?;
            let words = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .take(items)
                .map(|pair| {
                    let value = u16::from(pair[0]) | (u16::from(pair[1]) << 8);
                    format!("{value:04X}H")
                })
                .collect::<Vec<_>>()
                .join(" ");
            Ok(CommandOutcome::Reply(format!("OK SP={sp:04X}H {words}")))
        }
        "read-memory" | "mem" | "hexdump" | "h" => {
            if args.len() > 2 {
                return Err("read-memory accepts at most two arguments".to_string());
            }
            let (address, len) = if args.is_empty() {
                (0, MAPPED_MEMORY_ZONE_LEN)
            } else {
                let address = parse_u16(&args[0])?;
                let len_u16 = if let Some(len_token) = args.get(1) {
                    parse_u16(len_token)?
                } else {
                    1
                };
                let len = if len_u16 == 0 {
                    MAPPED_MEMORY_ZONE_LEN
                } else {
                    usize::from(len_u16)
                };
                (address, len)
            };
            let bytes = host_read_memory_range(host, instance_id, address, len)?;
            Ok(CommandOutcome::Reply(format!(
                "OK {}",
                bytes_to_compact_hex(&bytes)
            )))
        }
        "set-memory-zone" | "smz" => {
            let Some(zone_token) = args.first() else {
                return Err("set-memory-zone requires a zone index".to_string());
            };
            let zone = parse_memory_zone_selector(zone_token)?;
            session.current_memory_zone = zone;
            Ok(CommandOutcome::Reply(format!("OK {zone}")))
        }
        "get-memory-zones" | "gmz" => Ok(CommandOutcome::Reply(
            "OK Zone: -1 Name: Mapped memory; Zone: 0 Name: Mapped memory".to_string(),
        )),
        "get-current-memory-zone" | "gcmz" => Ok(CommandOutcome::Reply(format!(
            "OK {}",
            session.current_memory_zone
        ))),
        "get-io-ports" | "get-cpc-io" => {
            let payload = host_overlay_snapshot_text(host, instance_id)?;
            Ok(CommandOutcome::Reply(format!("OK {payload}")))
        }
        "get-membreakpoints" => {
            let mut start = 0u16;
            let mut items = MAPPED_MEMORY_ZONE_LEN;
            if let Some(token) = args.first() {
                start = parse_u16(token)?;
                items = 0;
            }
            if let Some(token) = args.get(1) {
                items = parse_usize(token)?;
                if items > MAPPED_MEMORY_ZONE_LEN {
                    return Err("Items out of range".to_string());
                }
            }

            let mut lines = Vec::new();
            lines.push(format!(
                "Breakpoints: {}",
                if session.breakpoints_enabled {
                    "On"
                } else {
                    "Off"
                }
            ));

            if items == 0 {
                let ty = session.mem_breakpoints[usize::from(start)];
                lines.push(format!("{start:04X}H : {ty}"));
            } else {
                let mut listed = 0usize;
                for (addr, ty) in session
                    .mem_breakpoints
                    .iter()
                    .enumerate()
                    .skip(usize::from(start))
                {
                    if *ty == 0 {
                        continue;
                    }
                    lines.push(format!("{addr:04X}H : {ty}"));
                    listed += 1;
                    if listed >= items {
                        break;
                    }
                }
            }

            Ok(CommandOutcome::Reply(format!("OK {}", lines.join("\n"))))
        }
        "get-ui-io-ports" => Ok(CommandOutcome::Reply(format!(
            "OK {}",
            bytes_to_compact_hex(&session.ui_io_ports)
        ))),
        "set-ui-io-ports" => {
            let Some(values) = args.first() else {
                return Err("set-ui-io-ports requires 9 hex bytes".to_string());
            };
            let raw = parse_hex_bytes(values)?;
            if raw.len() != 9 {
                return Err(format!(
                    "set-ui-io-ports expects 9 bytes (18 hex chars), got {}",
                    raw.len()
                ));
            }
            let mut next = [0u8; 9];
            next.copy_from_slice(&raw);
            apply_spectrum_ui_io_ports(host, instance_id, session, &next)?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "set-membreakpoint" => {
            if !session.breakpoints_enabled {
                return Err("Error. You must enable breakpoints first".to_string());
            }
            if args.len() < 2 {
                return Err("Needs two parameters minimum".to_string());
            }
            let start = parse_u16(&args[0])?;
            let ty_value = parse_usize(&args[1])?;
            if ty_value > usize::from(u8::MAX) {
                return Err("Type out of range".to_string());
            }
            let ty = ty_value as u8;
            let items = if let Some(token) = args.get(2) {
                parse_usize(token)?
            } else {
                1
            };

            let start_idx = usize::from(start);
            let end_idx = start_idx.saturating_add(items).min(MAPPED_MEMORY_ZONE_LEN);
            for idx in start_idx..end_idx {
                session.mem_breakpoints[idx] = ty;
            }
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "poke" | "write-memory" | "wm" | "write-memory-raw" => {
            let Some(addr_token) = args.first() else {
                return Err("poke requires address and hex bytes".to_string());
            };
            let address = parse_u16(addr_token)?;
            if args.len() < 2 {
                return Err("poke requires hex bytes".to_string());
            }
            let bytes = parse_hex_bytes(&args[1..].join(""))?;
            host_poke_bytes(host, instance_id, address, &bytes)?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "assemble" | "a" => {
            if args.len() < 2 {
                return Err("assemble requires address and instruction text".to_string());
            }
            let address = parse_u16(&args[0])?;
            let text = args[1..].join(" ");
            let response = host_command(
                host,
                instance_id,
                DebuggerHostCommand::Assemble {
                    addr: address,
                    text,
                },
            )?;
            let bytes = match response {
                DebuggerHostResponse::AssembledBytes(bytes) => bytes,
                other => return Err(format!("unexpected host response: {other:?}")),
            };
            Ok(CommandOutcome::Reply(format!(
                "OK {} bytes: {}",
                bytes.len(),
                bytes_to_hex(&bytes)
            )))
        }
        "set-portbreakpoint" => {
            if args.len() < 2 || args.len() > 3 {
                return Err("set-portbreakpoint requires <port> <type> [mask]".to_string());
            }
            let port = parse_u16(&args[0])?;
            let ty = parse_usize(&args[1])?;
            if ty > 3 {
                return Err("type must be 0 (remove), 1 (IN), 2 (OUT) or 3 (both)".to_string());
            }
            let mask = match args.get(2) {
                Some(token) => parse_u16(token)?,
                None => 0xFFFF,
            };
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::SetPortBreakpoint {
                    port,
                    mask,
                    on_read: ty & 1 != 0,
                    on_write: ty & 2 != 0,
                },
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "clear-portbreakpoints" => {
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::ClearPortBreakpoints,
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "get-portbreakpoints" => {
            let state = host_state(host, instance_id)?;
            let mut lines = vec!["Port breakpoints:".to_string()];
            for w in &state.port_breakpoints {
                let ty = u8::from(w.on_read) | (u8::from(w.on_write) << 1);
                lines.push(format!("{:04X}H {:04X}H : {ty}", w.port, w.mask));
            }
            Ok(CommandOutcome::Reply(format!("OK {}", lines.join("\n"))))
        }
        "write-port" => {
            let (Some(port), Some(value), None) = (args.first(), args.get(1), args.get(2)) else {
                return Err("write-port requires a port and a value".to_string());
            };
            let port = parse_u16(port)?;
            let value = u8::try_from(parse_zrcp_number(value)?)
                .map_err(|_| format!("value out of range 0-255: {value}"))?;
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::WritePort { port, value },
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "extended-stack" => {
            let depth = args
                .first()
                .map(|token| parse_usize(token))
                .transpose()?
                .unwrap_or(10)
                .clamp(1, 256);
            let state = host_state(host, instance_id)?;
            if let Some((_, sp_value, _)) = register_lookup(&state, "SP") {
                let sp =
                    u16::try_from(sp_value & 0xFFFF).map_err(|_| "invalid SP register value")?;
                let bytes = host_read_memory_range(host, instance_id, sp, depth.saturating_mul(2))?;
                let words: Vec<String> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .take(depth)
                    .map(|pair| {
                        let value = u16::from(pair[0]) | (u16::from(pair[1]) << 8);
                        format!("{value:04X}H")
                    })
                    .collect();
                Ok(CommandOutcome::Reply(format!(
                    "OK SP={sp:04X}H {}",
                    words.join(" ")
                )))
            } else {
                Err("stack pointer register SP is not available".to_string())
            }
        }
        "get-paging-state" => {
            // No `machine_id` gate -- any machine plugin whose `"overlay"`
            // chip has fields answers with real paging text (ZX Spectrum's
            // `memory_summary`/CPC's richer telemetry among them, see each
            // plugin's own `generic_overlay_chip_snapshot`/
            // `overlay_chip_snapshot`); one with nothing to report (ZX81,
            // C64) gets the same "not available" reply as before, decided
            // by whether the host actually found any fields rather than by
            // a hardcoded machine-id prefix list here.
            match host_overlay_snapshot_text(host, instance_id) {
                Ok(payload) if payload != "unavailable" => {
                    Ok(CommandOutcome::Reply(format!("OK {payload}")))
                }
                _ => Ok(CommandOutcome::Reply(
                    "ERR paging state not available for this machine".to_string(),
                )),
            }
        }
        "load-binary" => {
            if args.len() < 3 {
                return Err("load-binary requires file addr len".to_string());
            }
            let address = parse_u16(&args[1])?;
            let requested_len = parse_usize(&args[2])?;
            let file_bytes = fs::read(&args[0])
                .map_err(|e| format!("failed to read binary file {:?}: {e}", args[0]))?;
            let actual_len = if requested_len == 0 {
                file_bytes.len()
            } else {
                requested_len.min(file_bytes.len())
            };
            host_poke_bytes(host, instance_id, address, &file_bytes[..actual_len])?;
            Ok(CommandOutcome::Reply(format!(
                "OK loaded {actual_len} byte(s)"
            )))
        }
        "save-binary" => {
            if args.len() < 3 {
                return Err("save-binary requires file addr len".to_string());
            }
            let address = parse_u16(&args[1])?;
            let requested_len = parse_usize(&args[2])?;
            let read_len = if requested_len == 0 {
                MAPPED_MEMORY_ZONE_LEN
            } else {
                requested_len
            };
            let bytes = host_read_memory_range(host, instance_id, address, read_len)?;
            fs::write(&args[0], &bytes)
                .map_err(|e| format!("failed to write binary file {:?}: {e}", args[0]))?;
            Ok(CommandOutcome::Reply(format!(
                "OK saved {} byte(s)",
                bytes.len()
            )))
        }
        "snapshot-load" => {
            let Some(path) = args.first() else {
                return Err("snapshot-load requires file path".to_string());
            };
            let extension = file_extension(path)?;
            let bytes = fs::read(path)
                .map_err(|e| format!("failed to read snapshot file {path:?}: {e}"))?;
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::LoadSnapshot { extension, bytes },
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "snapshot-save" => {
            let Some(path) = args.first() else {
                return Err("snapshot-save requires file path".to_string());
            };
            let extension = file_extension(path)?;
            let bytes = match host_command(
                host,
                instance_id,
                DebuggerHostCommand::SaveSnapshot { extension },
            )? {
                DebuggerHostResponse::Bytes(bytes) => bytes,
                other => return Err(format!("unexpected host response: {other:?}")),
            };
            fs::write(path, &bytes)
                .map_err(|e| format!("failed to write snapshot file {path:?}: {e}"))?;
            Ok(CommandOutcome::Reply(format!(
                "OK saved {} byte(s)",
                bytes.len()
            )))
        }
        "get-snapshot" => {
            let bytes = match host_command(host, instance_id, DebuggerHostCommand::SaveState)? {
                DebuggerHostResponse::Bytes(bytes) => bytes,
                other => return Err(format!("unexpected host response: {other:?}")),
            };
            Ok(CommandOutcome::Reply(format!(
                "OK {}",
                bytes_to_compact_hex(&bytes)
            )))
        }
        "put-snapshot" => {
            let payload = line
                .split_once(char::is_whitespace)
                .map(|(_, rest)| rest.trim())
                .filter(|payload| !payload.is_empty())
                .ok_or_else(|| "put-snapshot requires hexadecimal payload".to_string())?;
            let bytes = parse_hex_bytes(payload)?;
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::LoadState { bytes },
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "watch_add" => {
            let expr = line
                .split_once(char::is_whitespace)
                .map(|(_, rest)| rest.trim().to_string())
                .filter(|expr| !expr.is_empty())
                .ok_or_else(|| "watch_add requires expression".to_string())?;
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::AddWatch { expr },
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "watch_remove" => {
            let Some(idx_token) = args.first() else {
                return Err("watch_remove requires index".to_string());
            };
            let index = parse_usize(idx_token)?;
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::RemoveWatch { index },
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "watch_clear" => {
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::ClearWatches,
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "follow_pc" | "follow-pc" => {
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::FollowPc,
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "debug-analyze-command" => {
            if args.is_empty() {
                return Ok(CommandOutcome::Reply("OK".to_string()));
            }
            let payload = args
                .iter()
                .enumerate()
                .map(|(index, value)| format!("Parameter {index}: [{value}]"))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(CommandOutcome::Reply(format!("OK {payload}")))
        }
        "send-keys-event" => {
            if args.len() < 2 {
                return Err("send-keys-event requires key event [nomenu]".to_string());
            }
            let key_code = parse_zrcp_number(&args[0])?;
            let pressed = parse_zrcp_number(&args[1])? != 0;
            if key_code == 192 {
                let released = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
                apply_spectrum_ui_io_ports(host, instance_id, session, &released)?;
                return Ok(CommandOutcome::Reply("OK".to_string()));
            }
            let key = util_key_code_to_key_name(key_code)
                .ok_or_else(|| format!("unsupported send-keys-event key code {key_code}"))?;
            send_host_key_event(host, instance_id, key, pressed)?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "send-keys-string" => {
            let payload = line
                .split_once(char::is_whitespace)
                .map(|(_, rest)| rest)
                .ok_or_else(|| "send-keys-string requires time and text".to_string())?;
            let payload = payload.trim_start();
            let (pause_token, text) = payload
                .split_once(char::is_whitespace)
                .ok_or_else(|| "send-keys-string requires time and text".to_string())?;
            if text.is_empty() {
                return Err("send-keys-string requires non-empty text".to_string());
            }
            let pause_ms = parse_zrcp_number(pause_token)?;
            let machine_id = instance_machine_id(host, instance_id)?;
            for ch in text.chars() {
                send_ascii_char(host, instance_id, Some(machine_id.as_str()), ch, pause_ms)?;
            }
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "send-keys-ascii" => {
            if args.len() < 2 {
                return Err("send-keys-ascii requires time and at least one ASCII code".to_string());
            }
            let pause_ms = parse_zrcp_number(&args[0])?;
            let machine_id = instance_machine_id(host, instance_id)?;
            for value in args.iter().skip(1) {
                let code = parse_zrcp_number(value)?;
                let code = u8::try_from(code)
                    .map_err(|_| format!("send-keys-ascii code out of range: {value}"))?;
                let ch = match code {
                    10 | 13 => '\n',
                    byte => byte as char,
                };
                send_ascii_char(host, instance_id, Some(machine_id.as_str()), ch, pause_ms)?;
            }
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "set-cr" => {
            if let Some(value) = args.first() {
                match value.to_ascii_lowercase().as_str() {
                    "0" | "off" | "false" | "no" | "1" | "on" | "true" | "yes" => {}
                    _ => return Err(format!("invalid set-cr value {value:?}")),
                }
            }
            Ok(CommandOutcome::Reply(
                "OK note: CRLF line endings are already enabled by default".to_string(),
            ))
        }
        "set-debug-settings" | "sds" => {
            let Some(value) = args.first() else {
                return Err("set-debug-settings requires numeric bitmask".to_string());
            };
            let mask = parse_zrcp_number(value)?;
            let mask = u32::try_from(mask).map_err(|_| "debug-settings value out of range")?;
            session.debug_settings = mask;
            Ok(CommandOutcome::Reply(format!("OK {mask}")))
        }
        "get-debug-settings" | "gds" => Ok(CommandOutcome::Reply(format!(
            "OK {}",
            session.debug_settings
        ))),
        "set-verbose-level" => {
            let Some(level_token) = args.first() else {
                return Err("set-verbose-level requires 0..255".to_string());
            };
            let level = parse_u16(level_token)?;
            let level = u8::try_from(level).map_err(|_| "set-verbose-level expects 0..255")?;
            session.verbose_level = level;
            Ok(CommandOutcome::Reply(format!("OK {level}")))
        }
        "reset-cpu" | "hard-reset-cpu" => {
            expect_ack(host_command(
                host,
                instance_id,
                DebuggerHostCommand::ResetMachine,
            ))?;
            Ok(CommandOutcome::Reply("OK".to_string()))
        }
        "exit-emulator" => Err("exit-emulator is unsupported; close with quit/exit".to_string()),
        "quit" | "exit" | "logout" => Ok(CommandOutcome::Quit),
        _ if is_known_unimplemented_remote_command(command.as_str()) => Err(format!(
            "command {command} is recognized in ZRCP but unsupported in retroclawd"
        )),
        _ => Err(format!("unknown command {command}")),
    }
}

fn write_protocol_line(stream: &mut TcpStream, text: &str) -> std::io::Result<()> {
    let normalized = text.replace('\n', "\r\n");
    stream.write_all(normalized.as_bytes())?;
    stream.write_all(b"\r\n")
}

fn write_protocol_prompt(
    stream: &mut TcpStream,
    host: &HostBridge,
    instance_id: &str,
) -> std::io::Result<()> {
    stream.write_all(query_prompt(host, instance_id).as_bytes())
}

fn take_next_command_frame(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let mut split = None;
    for (idx, byte) in buffer.iter().enumerate() {
        if *byte == b'\n' || *byte == b'\r' {
            split = Some((idx, *byte));
            break;
        }
    }
    let (idx, sep) = split?;

    let mut consume = idx + 1;
    if sep == b'\r' {
        if buffer.get(consume) == Some(&b'\0') {
            consume += 1;
        }
        if buffer.get(consume) == Some(&b'\n') {
            consume += 1;
        }
    } else if buffer.get(consume) == Some(&b'\r') {
        consume += 1;
    }

    let frame = buffer[..idx].to_vec();
    buffer.drain(..consume);
    Some(frame)
}
fn handle_client(
    mut stream: TcpStream,
    host: &HostBridge,
    instance_id: &str,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_millis(250)))?;
    stream.set_write_timeout(Some(Duration::from_millis(250)))?;

    write_protocol_line(
        &mut stream,
        "Welcome to retroclawd remote command protocol (ZRCP-like)",
    )?;
    write_protocol_line(&mut stream, "Write help for available commands")?;
    write_protocol_prompt(&mut stream, host, instance_id)?;
    stream.flush()?;

    let mut session = ZrcpSession::default();
    let mut pending = Vec::<u8>::new();
    let mut chunk = [0_u8; 1024];
    let mut last_command: Option<String> = None;
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                pending.extend_from_slice(&chunk[..read]);
                while let Some(frame) = take_next_command_frame(&mut pending) {
                    let raw = String::from_utf8_lossy(&frame);
                    let trimmed = raw.trim_matches(['\0', '\r', '\n']);
                    let mut effective = trimmed.to_string();
                    if effective.is_empty()
                        && session.repeat_last_command_on_enter()
                        && let Some(previous) = last_command.as_ref()
                    {
                        effective = previous.clone();
                    }
                    if !effective.is_empty() {
                        last_command = Some(effective.clone());
                    }
                    match handle_command(host, instance_id, &mut session, stop, &effective) {
                        Ok(CommandOutcome::Reply(reply)) => {
                            write_protocol_line(&mut stream, &reply)?;
                            write_protocol_prompt(&mut stream, host, instance_id)?;
                            stream.flush()?;
                        }
                        Ok(CommandOutcome::Quit) => {
                            write_protocol_line(&mut stream, "OK bye")?;
                            stream.flush()?;
                            return Ok(());
                        }
                        Err(err) => {
                            write_protocol_line(&mut stream, &format!("ERR {err}"))?;
                            write_protocol_prompt(&mut stream, host, instance_id)?;
                            stream.flush()?;
                        }
                    }
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

pub(super) fn describe() -> DebuggerPluginInfo {
    DebuggerPluginInfo {
        selector: SELECTOR.to_string(),
        display_name: "Debug ZRCP".to_string(),
        summary: "Opens a localhost ZRCP-like TCP debugger command interface for Z80 machines."
            .to_string(),
        options: vec![DebuggerPluginOptionInfo {
            key: "port".to_string(),
            value_hint: "0-65535".to_string(),
            description: "TCP listen port (default 11000).".to_string(),
        }],
    }
}

pub(super) fn start(
    options: &[NamedValue],
    instance_id: &str,
    host: &HostBridge,
) -> Result<Running, String> {
    let port = option_u16(options, "port")?.unwrap_or(11000);
    let machine_id = instance_machine_id(host, instance_id)?;
    if !is_supported_machine_id(&machine_id) {
        return Err(format!(
            "debug-zrcp supports the Spectrum and CPC machines, not {machine_id}"
        ));
    }
    let (server, bound_port) = spawn_server(host.clone(), instance_id.to_string(), port)
        .map_err(|e| format!("debug-zrcp: failed to listen on port {port}: {e}"))?;
    Ok(Running::new(
        None,
        Some(format!("127.0.0.1:{bound_port}")),
        server,
    ))
}

#[cfg(test)]
mod tests {
    use super::super::api::DebuggerHistoryEntry;
    use super::*;
    use plugin_api::RegisterField;

    #[test]
    fn parse_hex_bytes_accepts_compact_and_spaced_forms() {
        assert_eq!(parse_hex_bytes("A0FF").unwrap(), vec![0xA0, 0xFF]);
        assert_eq!(parse_hex_bytes("A0 FF").unwrap(), vec![0xA0, 0xFF]);
    }

    #[test]
    fn parse_u16_supports_zrcp_hex_suffix() {
        assert_eq!(parse_u16("140").unwrap(), 140);
        assert_eq!(parse_u16("8CH").unwrap(), 0x8C);
        assert_eq!(parse_u16("0x8c").unwrap(), 0x8C);
        assert_eq!(parse_u16("$8c").unwrap(), 0x8C);
    }

    #[test]
    fn parse_run_args_accepts_verbose_limit_and_flags() {
        let parsed = parse_run_args(
            ["verbose", "100", "no-stop-on-data", "update-immediately"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();
        assert!(parsed.verbose);
        assert_eq!(parsed.limit, Some(100));
        assert!(parsed.no_stop_on_data);
        assert!(parsed.update_immediately);
    }

    #[test]
    fn parse_zrcp_breakpoint_condition_accepts_pc_equals_hex() {
        let (address, condition) = parse_zrcp_breakpoint_condition("PC=0038H").unwrap();
        assert_eq!(address, 0x0038);
        assert!(condition.is_none());
    }

    #[test]
    fn parse_zrcp_breakpoint_condition_rejects_non_pc_expressions() {
        let error = parse_zrcp_breakpoint_condition("A=1").unwrap_err();
        assert!(error.contains("only PC=<addr>"));
    }

    struct FakeHost {
        state: DebuggerStateSnapshot,
        breakpoints: HashMap<u16, Option<String>>,
        /// Real writable memory for tests that need a genuine
        /// write-then-read round trip (`assemble`, `extended-stack`);
        /// an address never explicitly poked falls back to the
        /// original deterministic `address as u8` value every existing
        /// `ReadMemory` test already relies on.
        memory: HashMap<u16, u8>,
        /// Every `write-port` the host was asked to perform, in order.
        port_writes: Vec<(u16, u8)>,
    }

    fn fake_state(paused: bool, pc: u16) -> DebuggerStateSnapshot {
        DebuggerStateSnapshot {
            mode: if paused {
                DebuggerRunMode::Paused
            } else {
                DebuggerRunMode::Running
            },
            halted: false,
            pc,
            registers: Vec::new(),
            memory_addr: 0,
            memory: vec![0xAA, 0x55],
            breakpoints: Vec::new(),
            last_break: None,
            watches: Vec::new(),
            history: Vec::new(),
            reverse_depth: 0,
            last_event: None,
            disassembly: Vec::new(),
            following_pc: true,
            bitmap_palette: Vec::new(),
            bitmap_defaults: None,
            port_breakpoints: Vec::new(),
            last_port_hit: None,
        }
    }

    /// A stand-in application: answers a debugger module's requests from `FakeHost`.
    /// The tests keep a `&mut FakeHost` to inspect afterwards, so the handle
    /// carries a raw pointer; each test drives the module from one thread.
    struct FakeApi(*mut FakeHost);

    // SAFETY: only used from the test's own thread while the `FakeHost` is alive.
    unsafe impl Send for FakeApi {}
    // SAFETY: as above.
    unsafe impl Sync for FakeApi {}

    impl super::super::HostApi for FakeApi {
        fn request(&self, request: DebuggerHostRequest) -> Result<DebuggerHostResponse, String> {
            // SAFETY: see `FakeApi`.
            let host = unsafe { &mut *self.0 };
            match request {
                DebuggerHostRequest::GetState { .. } => {
                    Ok(DebuggerHostResponse::State(Box::new(host.state.clone())))
                }
                DebuggerHostRequest::ListInstances => Ok(DebuggerHostResponse::Instances(vec![
                    DebuggerInstanceInfo {
                        instance_id: "primary".to_string(),
                        machine_id: "zx-spectrum".to_string(),
                    },
                ])),
                DebuggerHostRequest::Command { command, .. } => {
                    let response = match command {
                        DebuggerHostCommand::Pause => {
                            host.state.mode = DebuggerRunMode::Paused;
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::Resume => {
                            host.state.mode = DebuggerRunMode::Running;
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::ResetMachine => {
                            host.state.mode = DebuggerRunMode::Paused;
                            host.state.halted = false;
                            host.state.pc = 0;
                            host.state.last_event = Some("reset".to_string());
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::KeyEvent { key, pressed } => {
                            host.state.last_event = Some(format!("key:{key}:{pressed}"));
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::StepInstruction => {
                            host.state.pc = host.state.pc.wrapping_add(1);
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::SetPc { address } => {
                            host.state.pc = address;
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::SetRegister { name, value } => {
                            if name.eq_ignore_ascii_case("PC") {
                                host.state.pc = value as u16;
                            }
                            host.state.last_event = Some(format!("set-register:{name}={value}"));
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::SetBreakpoint { address, condition } => {
                            host.breakpoints.insert(address, condition);
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::ClearBreakpoint { address } => {
                            host.breakpoints.remove(&address);
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::ReadMemory { address, len } => {
                            let bytes = (0..len)
                                .map(|offset| {
                                    let addr = address.wrapping_add(offset);
                                    host.memory.get(&addr).copied().unwrap_or(addr as u8)
                                })
                                .collect();
                            DebuggerHostResponse::Bytes(bytes)
                        }
                        DebuggerHostCommand::SaveState => {
                            DebuggerHostResponse::Bytes(vec![0xDE, 0xAD])
                        }
                        DebuggerHostCommand::SaveSnapshot { .. } => {
                            DebuggerHostResponse::Bytes(vec![0xFA, 0xCE])
                        }
                        DebuggerHostCommand::GetTimingCounters => {
                            DebuggerHostResponse::Bytes(b"123456 789".to_vec())
                        }
                        DebuggerHostCommand::GetOverlaySnapshot => {
                            DebuggerHostResponse::Bytes(b"memory_summary=ROM0 RAM0".to_vec())
                        }
                        DebuggerHostCommand::Poke { address, bytes } => {
                            for (i, b) in bytes.iter().enumerate() {
                                host.memory.insert(address.wrapping_add(i as u16), *b);
                            }
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::WritePort { port, value } => {
                            host.port_writes.push((port, value));
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::SetPortBreakpoint {
                            port,
                            mask,
                            on_read,
                            on_write,
                        } => {
                            host.state
                                .port_breakpoints
                                .retain(|w| !(w.port == port && w.mask == mask));
                            if on_read || on_write {
                                host.state.port_breakpoints.push(plugin_api::PortWatchSpec {
                                    port,
                                    mask,
                                    on_read,
                                    on_write,
                                });
                            }
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::ClearPortBreakpoints => {
                            host.state.port_breakpoints.clear();
                            DebuggerHostResponse::Ack
                        }
                        DebuggerHostCommand::Assemble { addr, text } => {
                            // The real host picks the assembler by the machine's CPU family; this stand-in has no
                            // machine, so it accepts whichever of the two assemblers understands the text.
                            let bytes = z80::disassemble::assemble_one_at(&text, addr)
                                .or_else(|_| mos6502::disassemble::assemble_one(&text, addr))
                                .unwrap_or_default();
                            for (i, b) in bytes.iter().enumerate() {
                                host.memory.insert(addr.wrapping_add(i as u16), *b);
                            }
                            DebuggerHostResponse::AssembledBytes(bytes)
                        }
                        DebuggerHostCommand::AddWatch { .. }
                        | DebuggerHostCommand::RemoveWatch { .. }
                        | DebuggerHostCommand::ClearWatches
                        | DebuggerHostCommand::FollowPc
                        | DebuggerHostCommand::StepOverInstruction
                        | DebuggerHostCommand::BackstepInstruction
                        | DebuggerHostCommand::ClearAllBreakpoints
                        | DebuggerHostCommand::SetBreakpointCondition { .. }
                        | DebuggerHostCommand::SetMemoryWindow { .. }
                        | DebuggerHostCommand::SetDisassemblyWindow { .. }
                        | DebuggerHostCommand::SetDisassemblyLineCount { .. }
                        | DebuggerHostCommand::PokeBank { .. }
                        | DebuggerHostCommand::LoadState { .. }
                        | DebuggerHostCommand::LoadSnapshot { .. }
                        | DebuggerHostCommand::ResetTimingPartial => DebuggerHostResponse::Ack,
                    };
                    Ok(response)
                }
            }
        }
    }

    fn fake_host_bridge(host: &mut FakeHost) -> HostBridge {
        HostBridge::new(Arc::new(FakeApi(host as *mut FakeHost)))
    }

    #[test]
    fn zrcp_like_breakpoint_workflow_runs_with_limit() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x0036),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let reply =
            match handle_command(&host, "primary", &mut session, &stop, "enable-breakpoints") {
                Ok(CommandOutcome::Reply(reply)) => reply,
                other => panic!("unexpected response: {other:?}"),
            };
        assert_eq!(reply, "OK");

        let reply = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-breakpoint 1 PC=0038H",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(reply.contains("PC=0038H"));

        let reply = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-breakpointaction 1 break",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(reply, "OK");

        let reply = match handle_command(&host, "primary", &mut session, &stop, "run verbose 16") {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(reply.contains("Breakpoint fired: PC=0038H"));
    }

    #[test]
    fn help_and_unknown_command_behave_as_expected() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let help = match handle_command(&host, "primary", &mut session, &stop, "help") {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(help.contains("commands:"));
        assert!(help.contains("set-breakpoint"));

        let blank = match handle_command(&host, "primary", &mut session, &stop, "") {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(blank.contains("commands:"));

        let err = handle_command(&host, "primary", &mut session, &stop, "helpp").unwrap_err();
        assert!(err.contains("unknown command"));
    }

    #[test]
    fn recognized_unimplemented_command_returns_specific_error() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        // Test that an unimplemented command returns the expected error
        let err = handle_command(&host, "primary", &mut session, &stop, "smartload").unwrap_err();
        assert!(err.contains("recognized in ZRCP"));
    }

    #[test]
    fn metadata_commands_return_expected_shapes() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let buildnumber =
            match handle_command(&host, "primary", &mut session, &stop, "get-buildnumber") {
                Ok(CommandOutcome::Reply(reply)) => reply,
                other => panic!("unexpected response: {other:?}"),
            };
        assert!(buildnumber.starts_with("OK "));

        let driver = match handle_command(&host, "primary", &mut session, &stop, "get-video-driver")
        {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(driver, "OK retroclawd");
    }

    #[test]
    fn timing_and_io_diagnostics_commands_return_expected_payloads() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x2000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let total = match handle_command(&host, "primary", &mut session, &stop, "get-tstates") {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(total, "OK 123456");

        let partial =
            match handle_command(&host, "primary", &mut session, &stop, "get-tstates-partial") {
                Ok(CommandOutcome::Reply(reply)) => reply,
                other => panic!("unexpected response: {other:?}"),
            };
        assert_eq!(partial, "OK 789");

        let reset = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "reset-tstates-partial",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(reset, "OK");

        let io_ports = match handle_command(&host, "primary", &mut session, &stop, "get-io-ports") {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(io_ports, "OK memory_summary=ROM0 RAM0");

        let io_ports_alias =
            match handle_command(&host, "primary", &mut session, &stop, "get-cpc-io") {
                Ok(CommandOutcome::Reply(reply)) => reply,
                other => panic!("unexpected response: {other:?}"),
            };
        assert_eq!(io_ports_alias, "OK memory_summary=ROM0 RAM0");
    }

    #[test]
    fn assemble_goes_to_the_assembler_of_the_running_cpu() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);
        let mut run = |line: &str| match handle_command(&host, "primary", &mut session, &stop, line)
        {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        // 6502: an immediate load and a branch whose operand is the absolute target.
        assert_eq!(run("assemble C000H LDA #$05"), "OK 2 bytes: A9 05");
        assert_eq!(run("assemble C000H BNE $C000"), "OK 2 bytes: D0 FE");
        // Z80: a relative jump is also written as its absolute target.
        assert_eq!(run("assemble 8000H JR 0x8000"), "OK 2 bytes: 18 FE");
    }

    #[test]
    fn assemble_pokes_one_instruction_and_reports_bytes() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let reply = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "assemble 8000H LD A,5",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        // `LD A,n` assembles to 2 bytes: 0x3E 0x05.
        assert_eq!(reply, "OK 2 bytes: 3E 05");

        let readback =
            match handle_command(&host, "primary", &mut session, &stop, "read-memory 8000H 2") {
                Ok(CommandOutcome::Reply(reply)) => reply,
                other => panic!("unexpected response: {other:?}"),
            };
        assert!(
            readback.contains("3E") && readback.contains("05"),
            "assembled bytes should read back from where they were poked: {readback}"
        );
    }

    #[test]
    fn extended_stack_reports_words_above_sp() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        fake.state.registers.push(RegisterField {
            bank: "main".to_string(),
            name: "SP".to_string(),
            value: 0xC000,
            width_bits: 16,
        });
        // Poke two known words directly above SP.
        fake.memory.insert(0xC000, 0x34);
        fake.memory.insert(0xC001, 0x12);
        fake.memory.insert(0xC002, 0x78);
        fake.memory.insert(0xC003, 0x56);
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let reply = match handle_command(&host, "primary", &mut session, &stop, "extended-stack 2")
        {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(reply, "OK SP=C000H 1234H 5678H");
    }

    #[test]
    fn run_ignores_the_stale_paused_snapshot_until_the_resume_lands() {
        let paused_at_hit = {
            let mut state = fake_state(true, 0x400B);
            state.last_port_hit = Some(plugin_api::PortHitInfo {
                port: 0xBC00,
                value: 0,
                write: true,
            });
            state
        };
        let soon = Duration::from_millis(20);
        let late = RESUME_APPLY_GRACE + Duration::from_millis(1);

        // Still the old snapshot, with its old hit: not the outcome yet.
        assert!(!run_outcome_ready(&paused_at_hit, &paused_at_hit, soon));
        // The host really is running again.
        assert!(!run_outcome_ready(
            &paused_at_hit,
            &fake_state(false, 0x4001),
            late
        ));
        // Stopped somewhere new, or on a fresh hit, is a real stop right away.
        assert!(run_outcome_ready(
            &paused_at_hit,
            &fake_state(true, 0x4009),
            soon
        ));
        let mut other_hit = paused_at_hit.clone();
        other_hit.last_port_hit = Some(plugin_api::PortHitInfo {
            port: 0x7F10,
            value: 0x10,
            write: true,
        });
        assert!(run_outcome_ready(&paused_at_hit, &other_hit, soon));
        // The same snapshot after the grace period is a genuine immediate re-stop.
        assert!(run_outcome_ready(&paused_at_hit, &paused_at_hit, late));
        // A halted CPU ends the run whatever else it looks like.
        let mut halted = paused_at_hit.clone();
        halted.halted = true;
        assert!(run_outcome_ready(&paused_at_hit, &halted, soon));
    }

    #[test]
    fn port_breakpoints_can_be_set_listed_replaced_and_cleared() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);
        let mut run = |line: &str| match handle_command(&host, "primary", &mut session, &stop, line)
        {
            Ok(CommandOutcome::Reply(reply)) => Ok(reply),
            Ok(other) => panic!("{line}: unexpected outcome {other:?}"),
            Err(e) => Err(e),
        };

        assert_eq!(run("get-portbreakpoints").unwrap(), "OK Port breakpoints:");
        assert_eq!(run("set-portbreakpoint 0x7ffd 2").unwrap(), "OK");
        assert_eq!(run("set-portbreakpoint $FE 3 0x00FF").unwrap(), "OK");
        assert_eq!(
            run("get-portbreakpoints").unwrap(),
            "OK Port breakpoints:\n7FFDH FFFFH : 2\n00FEH 00FFH : 3"
        );
        // Same port+mask replaces; type 0 removes.
        assert_eq!(run("set-portbreakpoint 0x7ffd 1").unwrap(), "OK");
        assert_eq!(run("set-portbreakpoint $FE 0 0xFF").unwrap(), "OK");
        assert_eq!(
            run("get-portbreakpoints").unwrap(),
            "OK Port breakpoints:\n7FFDH FFFFH : 1"
        );
        assert_eq!(run("clear-portbreakpoints").unwrap(), "OK");
        assert_eq!(run("get-portbreakpoints").unwrap(), "OK Port breakpoints:");

        for line in [
            "set-portbreakpoint",
            "set-portbreakpoint 0x7ffd",
            "set-portbreakpoint 0x7ffd 4",
            "set-portbreakpoint 0x7ffd 1 2 3",
            "set-portbreakpoint 70000 1",
        ] {
            assert!(run(line).is_err(), "{line} should be rejected");
        }
    }

    #[test]
    fn write_port_reaches_the_host_with_zrcp_number_syntax() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        for (line, expected) in [
            ("write-port 254 7", "OK"),
            ("write-port 0x7ffd 0x10", "OK"),
            ("write-port $FE 1Fh", "OK"),
        ] {
            match handle_command(&host, "primary", &mut session, &stop, line) {
                Ok(CommandOutcome::Reply(reply)) => assert_eq!(reply, expected, "{line}"),
                other => panic!("{line}: unexpected response: {other:?}"),
            }
        }
        // Malformed commands are errors and never reach the host.
        for line in [
            "write-port",
            "write-port 254",
            "write-port 254 256",
            "write-port 65536 1",
            "write-port 1 2 3",
        ] {
            assert!(
                handle_command(&host, "primary", &mut session, &stop, line).is_err(),
                "{line} should be rejected"
            );
        }
        assert_eq!(fake.port_writes, [(254, 7), (0x7FFD, 0x10), (0xFE, 0x1F)]);
    }

    #[test]
    fn get_paging_state_reports_cpc_ram_and_rom_selection() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        // No `machine_id` gate any more -- this now takes the "OK" branch
        // purely because the fake host's overlay snapshot text is
        // non-empty, not because "zx-spectrum" is on some hardcoded prefix
        // list (see `get-paging-state`'s own doc comment on the match arm).
        let reply = match handle_command(&host, "primary", &mut session, &stop, "get-paging-state")
        {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(reply, "OK memory_summary=ROM0 RAM0");
    }

    #[test]
    fn crc32_and_memory_breakpoint_commands_work() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x2000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let crc = match handle_command(&host, "primary", &mut session, &stop, "get-crc32 0100H 4") {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(crc, "OK 8bb98613");

        let err = handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-membreakpoint 4000H 2",
        )
        .unwrap_err();
        assert!(err.contains("enable breakpoints"));

        let _ =
            handle_command(&host, "primary", &mut session, &stop, "enable-breakpoints").unwrap();
        let set_reply = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-membreakpoint 4000H 2 2",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(set_reply, "OK");

        let single = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-membreakpoints 4000H",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(single.contains("Breakpoints: On"));
        assert!(single.contains("4000H : 2"));

        let list = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-membreakpoints 3FFFH 2",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(list.contains("4000H : 2"));
        assert!(list.contains("4001H : 2"));

        let cleared = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "clear-membreakpoints",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(cleared, "OK");

        let after_clear = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-membreakpoints 4000H",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(after_clear.contains("4000H : 0"));
    }

    #[test]
    fn lightweight_compatibility_commands_behave_consistently() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x0036),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let set_verbose =
            match handle_command(&host, "primary", &mut session, &stop, "set-verbose-level 1") {
                Ok(CommandOutcome::Reply(reply)) => reply,
                other => panic!("unexpected response: {other:?}"),
            };
        assert_eq!(set_verbose, "OK 1");

        let run_reply = match handle_command(&host, "primary", &mut session, &stop, "run 1") {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(run_reply.contains("PC="));

        let pages = match handle_command(&host, "primary", &mut session, &stop, "get-memory-pages")
        {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(pages, "OK mapped-memory(0000H-FFFFH)");

        let pages_verbose = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-memory-pages verbose",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(pages_verbose.contains("machine=zx-spectrum"));

        let _ =
            handle_command(&host, "primary", &mut session, &stop, "enable-breakpoints").unwrap();
        let _ = handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-breakpoint 1 PC=0038H",
        )
        .unwrap();
        let _ = handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-breakpointaction 1 break",
        )
        .unwrap();

        let optimized = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-breakpoints-optimized",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(optimized.contains("1: yes"));

        let _ = handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-breakpointpasscount 1 2",
        )
        .unwrap();

        let no_opt = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-breakpoints-optimized",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(no_opt.contains("1: no"));
    }
    #[test]
    fn ui_io_ports_and_reset_cpu_commands_work() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x4321),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let initial_ports =
            match handle_command(&host, "primary", &mut session, &stop, "get-ui-io-ports") {
                Ok(CommandOutcome::Reply(reply)) => reply,
                other => panic!("unexpected response: {other:?}"),
            };
        assert_eq!(initial_ports, "OK FFFFFFFFFFFFFFFF00");

        let press_reply = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-ui-io-ports FEFFFFFFFFFFFFFF00",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(press_reply, "OK");
        assert_eq!(fake.state.last_event.as_deref(), Some("key:ShiftLeft:true"));

        let updated_ports =
            match handle_command(&host, "primary", &mut session, &stop, "get-ui-io-ports") {
                Ok(CommandOutcome::Reply(reply)) => reply,
                other => panic!("unexpected response: {other:?}"),
            };
        assert_eq!(updated_ports, "OK FEFFFFFFFFFFFFFF00");

        let release_reply = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-ui-io-ports FFFFFFFFFFFFFFFF00",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(release_reply, "OK");
        assert_eq!(
            fake.state.last_event.as_deref(),
            Some("key:ShiftLeft:false")
        );

        fake.state.mode = DebuggerRunMode::Running;
        fake.state.halted = true;
        fake.state.pc = 0x4321;

        let reset_reply = match handle_command(&host, "primary", &mut session, &stop, "reset-cpu") {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(reset_reply, "OK");
        assert_eq!(fake.state.mode, DebuggerRunMode::Paused);
        assert!(!fake.state.halted);
        assert_eq!(fake.state.pc, 0);
        assert_eq!(fake.state.last_event.as_deref(), Some("reset"));
    }

    #[test]
    fn send_keys_commands_dispatch_key_events() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x4000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let press = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "send-keys-event 128 1",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(press, "OK");
        assert_eq!(fake.state.last_event.as_deref(), Some("key:Space:true"));

        let release = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "send-keys-event 128 0",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(release, "OK");
        assert_eq!(fake.state.last_event.as_deref(), Some("key:Space:false"));

        let by_string = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "send-keys-string 0 Az",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(by_string, "OK");
        assert_eq!(fake.state.last_event.as_deref(), Some("key:KeyZ:false"));

        let by_ascii = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "send-keys-ascii 0 49 50",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(by_ascii, "OK");
        assert_eq!(fake.state.last_event.as_deref(), Some("key:Digit2:false"));

        let _ = handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-ui-io-ports FEFFFFFFFFFFFFFF00",
        )
        .unwrap();
        assert_eq!(fake.state.last_event.as_deref(), Some("key:ShiftLeft:true"));

        let reset_all = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "send-keys-event 192 1",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(reset_all, "OK");
        assert_eq!(
            fake.state.last_event.as_deref(),
            Some("key:ShiftLeft:false")
        );

        let ports = match handle_command(&host, "primary", &mut session, &stop, "get-ui-io-ports") {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(ports, "OK FFFFFFFFFFFFFFFF00");
    }

    #[test]
    fn ascii_quote_maps_to_shift_digit2_for_cpc() {
        assert_eq!(
            ascii_key_binding('"', Some("cpc464")),
            Some(("Digit2", true))
        );
        assert_eq!(
            ascii_key_binding('"', Some("cpc6128")),
            Some(("Digit2", true))
        );
        assert_eq!(
            ascii_key_binding('"', Some("zx-spectrum")),
            Some(("Quote", true))
        );
    }

    #[test]
    fn breakpoint_query_commands_report_actions_and_passcounts() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-breakpoint 3 PC=4000H",
        ) {
            Ok(CommandOutcome::Reply(_)) => {}
            other => panic!("unexpected response: {other:?}"),
        }
        match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-breakpointaction 3 break",
        ) {
            Ok(CommandOutcome::Reply(_)) => {}
            other => panic!("unexpected response: {other:?}"),
        }
        match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-breakpointpasscount 3 5",
        ) {
            Ok(CommandOutcome::Reply(_)) => {}
            other => panic!("unexpected response: {other:?}"),
        }

        let breakpoints =
            match handle_command(&host, "primary", &mut session, &stop, "get-breakpoints") {
                Ok(CommandOutcome::Reply(reply)) => reply,
                other => panic!("unexpected response: {other:?}"),
            };
        assert!(breakpoints.contains("3: PC=4000H"));

        let actions = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-breakpointsactions",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(actions.contains("3: break"));

        let passcounts = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-breakpointspasscount",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(passcounts.contains("3: current=0 configured=5"));
    }

    #[test]
    fn passcount_breakpoint_requires_configured_hit_count() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x0036),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let _ =
            handle_command(&host, "primary", &mut session, &stop, "enable-breakpoints").unwrap();
        let _ = handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-breakpoint 1 PC=0038H",
        )
        .unwrap();
        let _ = handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-breakpointaction 1 break",
        )
        .unwrap();

        let passcount_set = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-breakpointpasscount 1 2",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(passcount_set, "OK");

        let first_run = match handle_command(&host, "primary", &mut session, &stop, "run verbose 4")
        {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(first_run.contains("Run limit reached (4 instructions)"));

        let passcounts_after_first = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-breakpointspasscount",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(passcounts_after_first.contains("1: current=1 configured=2"));

        let set_pc_reply = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-register PC=0037H",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(set_pc_reply, "OK PC=0037H");

        let second_run = match handle_command(&host, "primary", &mut session, &stop, "run 2") {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(second_run.contains("Breakpoint fired: PC=0038H"));

        let passcounts_after_second = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-breakpointspasscount",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(passcounts_after_second.contains("1: current=2 configured=2"));
    }

    #[test]
    fn set_register_forwards_non_pc_names() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let reply = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-register AF=1234H",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(reply, "OK AF=1234H");
        assert_eq!(
            fake.state.last_event.as_deref(),
            Some("set-register:AF=4660")
        );
    }

    #[test]
    fn memory_zone_aliases_round_trip_current_zone() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let default_zone = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-current-memory-zone",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(default_zone, "OK -1");

        let zone0 = match handle_command(&host, "primary", &mut session, &stop, "set-memory-zone 0")
        {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(zone0, "OK 0");

        let current0 = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-current-memory-zone",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(current0, "OK 0");

        let mapped = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "set-memory-zone mapped",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(mapped, "OK -1");

        let err =
            handle_command(&host, "primary", &mut session, &stop, "set-memory-zone 2").unwrap_err();
        assert!(err.contains("only zones -1/0"));
    }

    #[test]
    fn command_aliases_gmz_wm_and_esxgof_behave_compatibly() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x2000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let zones = match handle_command(&host, "primary", &mut session, &stop, "get-memory-zones")
        {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        let zones_alias = match handle_command(&host, "primary", &mut session, &stop, "gmz") {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(zones_alias, zones);

        let write_alias =
            match handle_command(&host, "primary", &mut session, &stop, "wm 4000H AA 55") {
                Ok(CommandOutcome::Reply(reply)) => reply,
                other => panic!("unexpected response: {other:?}"),
            };
        assert_eq!(write_alias, "OK");

        let err = handle_command(&host, "primary", &mut session, &stop, "smartload").unwrap_err();
        assert!(err.contains("recognized in ZRCP"));
    }

    #[test]
    fn cpu_history_and_stack_backtrace_commands_work() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x1000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        fake.state.history = vec![
            DebuggerHistoryEntry {
                before_pc: 0x4000,
                after_pc: 0x4001,
                text: "NOP".to_string(),
            },
            DebuggerHistoryEntry {
                before_pc: 0x4100,
                after_pc: 0x4102,
                text: "JR 4104H".to_string(),
            },
        ];
        fake.state.reverse_depth = 16;
        fake.state.registers = vec![plugin_api::RegisterField {
            bank: "main".to_string(),
            name: "SP".to_string(),
            value: 0x0100,
            width_bits: 16,
        }];

        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        let size = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "cpu-history get-size",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(size, "OK 2");

        let entry = match handle_command(&host, "primary", &mut session, &stop, "cpu-history get 1")
        {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(entry.contains("PC=4100H"));

        let pcs = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "cpu-history get-pc 0 2",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(pcs, "OK 4000H 4100H");

        let found = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "cpu-history find-pc 4100H",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(found, "OK 1");

        let stack = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            "get-stack-backtrace 2",
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(stack.contains("SP=0100H"));
        assert!(stack.contains("0100H 0302H"));
    }
    #[test]
    fn binary_and_snapshot_commands_round_trip_files() {
        let mut fake = FakeHost {
            state: fake_state(true, 0x2000),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let mut session = ZrcpSession::default();
        let stop = AtomicBool::new(false);

        // Use a tempdir (auto-removed on drop, including on panic) rather
        // than nonce-named files in the crate root -- a prior version left
        // stray scratch files behind whenever an assertion below failed
        // before manual cleanup ran.
        let dir = tempfile::tempdir().unwrap();
        let input_path = dir.path().join("in.bin").display().to_string();
        let output_path = dir.path().join("out.bin").display().to_string();
        let snapshot_path = dir.path().join("snap.z80").display().to_string();

        std::fs::write(&input_path, [0x01_u8, 0x02, 0x03, 0x04]).unwrap();

        let load_reply = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            &format!("load-binary {input_path} 4000 0"),
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(load_reply.contains("loaded 4 byte(s)"));

        let save_reply = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            &format!("save-binary {output_path} 0100H 4"),
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(save_reply.contains("saved 4 byte(s)"));
        assert_eq!(
            std::fs::read(&output_path).unwrap(),
            vec![0x00, 0x01, 0x02, 0x03]
        );

        let snapshot_reply = match handle_command(
            &host,
            "primary",
            &mut session,
            &stop,
            &format!("snapshot-save {snapshot_path}"),
        ) {
            Ok(CommandOutcome::Reply(reply)) => reply,
            other => panic!("unexpected response: {other:?}"),
        };
        assert!(snapshot_reply.contains("saved 2 byte(s)"));
        assert_eq!(std::fs::read(&snapshot_path).unwrap(), vec![0xFA, 0xCE]);

        let get_snapshot =
            match handle_command(&host, "primary", &mut session, &stop, "get-snapshot") {
                Ok(CommandOutcome::Reply(reply)) => reply,
                other => panic!("unexpected response: {other:?}"),
            };
        assert_eq!(get_snapshot, "OK DEAD");
    }

    #[test]
    fn tcp_client_flow_accepts_zrcp_style_commands() {
        use std::io::{Read, Write};
        use std::net::{Shutdown, TcpListener, TcpStream};

        let mut fake = FakeHost {
            state: fake_state(true, 0x0036),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let stop = AtomicBool::new(false);

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();

        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .write_all(
                    b"help\nhelpp\nenter-cpu-step\nenable-breakpoints\nset-breakpoint 1 PC=0038H\nset-breakpointaction 1 break\nrun 8\nquit\n",
                )
                .unwrap();
            stream.shutdown(Shutdown::Write).unwrap();

            let mut output = String::new();
            stream.read_to_string(&mut output).unwrap();
            output
        });

        let (stream, _) = listener.accept().unwrap();
        handle_client(stream, &host, "primary", &stop).unwrap();

        let output = client.join().unwrap();
        assert!(output.contains("Welcome to retroclawd remote command protocol"));
        assert!(output.contains("OK commands:"));
        assert!(output.contains("ERR unknown command helpp"));
        assert!(output.contains("command@cpu-step> "));
        assert!(output.contains("Breakpoint fired: PC=0038H"));
        assert!(output.contains("OK bye"));
    }

    #[test]
    fn tcp_client_flow_accepts_cr_nul_framing() {
        use std::io::{Read, Write};
        use std::net::{Shutdown, TcpListener, TcpStream};

        let mut fake = FakeHost {
            state: fake_state(true, 0x0036),
            breakpoints: HashMap::new(),
            memory: HashMap::new(),
            port_writes: Vec::new(),
        };
        let host = fake_host_bridge(&mut fake);
        let stop = AtomicBool::new(false);

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();

        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream.write_all(b"get-version\r\0quit\r\0").unwrap();
            stream.shutdown(Shutdown::Write).unwrap();

            let mut output = String::new();
            stream.read_to_string(&mut output).unwrap();
            output
        });

        let (stream, _) = listener.accept().unwrap();
        handle_client(stream, &host, "primary", &stop).unwrap();

        let output = client.join().unwrap();
        assert!(output.contains("OK 0.1.0"));
        assert!(output.contains("OK bye"));
        assert!(!output.contains("OK commands:"));
    }
}
