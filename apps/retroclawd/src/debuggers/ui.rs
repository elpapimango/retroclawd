//! `debug-ui`: the browser debugger. A small `tiny_http` server (127.0.0.1
//! only) serves one self-contained page (`assets/debugger.html`, no external
//! requests) and a JSON API it polls and posts commands to.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use plugin_api::{DebugValue, NamedValue};
use tiny_http::{Header, Method, Response, Server};

use super::api::{
    DebuggerHistoryEntry, DebuggerHostCommand, DebuggerHostRequest, DebuggerHostResponse,
    DebuggerPluginInfo, DebuggerPluginOptionInfo, DebuggerRunMode, DebuggerStateSnapshot,
    DebuggerWatch,
};
use super::{HostBridge, Running};

pub(super) const SELECTOR: &str = "debug-ui";
const DEBUGGER_HTML: &str = include_str!("../../../../assets/debugger.html");

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

fn parse_u16(token: &str) -> Result<u16, String> {
    let token = token.trim();
    if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        u16::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else {
        token.parse::<u16>().map_err(|e| e.to_string())
    }
}

#[derive(serde::Serialize)]
struct BreakpointDetailResponse {
    addr: u16,
    condition: Option<String>,
}

#[derive(serde::Serialize)]
struct DisasmLineResponse {
    addr: u16,
    bytes: Vec<u8>,
    text: String,
}

#[derive(serde::Serialize)]
struct StateResponse {
    mode: DebuggerRunMode,
    registers: BTreeMap<String, u64>,
    halted: bool,
    pc: u16,
    memory_addr: u16,
    memory: Vec<u8>,
    breakpoints: Vec<u16>,
    breakpoint_details: Vec<BreakpointDetailResponse>,
    last_break: Option<u16>,
    watches: Vec<DebuggerWatch>,
    history: Vec<DebuggerHistoryEntry>,
    reverse_depth: usize,
    last_event: Option<String>,
    disassembly: Vec<DisasmLineResponse>,
    following_pc: bool,
    bitmap_palette: Vec<plugin_api::BitmapPaletteColor>,
    bitmap_defaults: Option<plugin_api::BitmapExplorerDefaults>,
    port_breakpoints: Vec<plugin_api::PortWatchSpec>,
    last_port_hit: Option<plugin_api::PortHitInfo>,
}

impl From<DebuggerStateSnapshot> for StateResponse {
    fn from(state: DebuggerStateSnapshot) -> Self {
        let mut registers = BTreeMap::new();
        for reg in state.registers {
            registers.insert(reg.name, reg.value);
        }
        let breakpoints = state.breakpoints.iter().map(|bp| bp.address).collect();
        let breakpoint_details = state
            .breakpoints
            .into_iter()
            .map(|bp| BreakpointDetailResponse {
                addr: bp.address,
                condition: bp.condition,
            })
            .collect();
        let disassembly = state
            .disassembly
            .into_iter()
            .map(|line| DisasmLineResponse {
                addr: line.address as u16,
                bytes: line.bytes,
                text: line.text,
            })
            .collect();

        Self {
            mode: state.mode,
            registers,
            halted: state.halted,
            pc: state.pc,
            memory_addr: state.memory_addr,
            memory: state.memory,
            breakpoints,
            breakpoint_details,
            last_break: state.last_break,
            watches: state.watches,
            history: state.history,
            reverse_depth: state.reverse_depth,
            last_event: state.last_event,
            disassembly,
            following_pc: state.following_pc,
            bitmap_palette: state.bitmap_palette,
            bitmap_defaults: state.bitmap_defaults,
            port_breakpoints: state.port_breakpoints,
            last_port_hit: state.last_port_hit,
        }
    }
}

#[derive(serde::Deserialize)]
struct PortBreakpointBody {
    port: u16,
    #[serde(default = "full_port_mask")]
    mask: u16,
    #[serde(default)]
    on_read: bool,
    #[serde(default)]
    on_write: bool,
}

fn full_port_mask() -> u16 {
    0xFFFF
}

#[derive(serde::Deserialize)]
struct AddrBody {
    addr: u16,
}

#[derive(serde::Deserialize)]
struct BreakpointBody {
    addr: u16,
    #[serde(default)]
    condition: Option<String>,
}

#[derive(serde::Deserialize)]
struct PokeBody {
    addr: u16,
    bytes: Vec<u8>,
}

#[derive(serde::Deserialize)]
struct MemoryWindowBody {
    addr: u16,
    len: u16,
}

#[derive(serde::Deserialize)]
struct CountBody {
    count: u16,
}

#[derive(serde::Deserialize)]
struct AddWatchBody {
    expr: String,
}

#[derive(serde::Deserialize)]
struct RemoveWatchBody {
    index: usize,
}

#[derive(serde::Deserialize)]
struct AssembleBody {
    addr: u16,
    text: String,
}

#[derive(serde::Serialize)]
struct AssembleResponse {
    bytes: Vec<u8>,
}

#[derive(serde::Serialize)]
struct PokApplyResponse {
    applied: usize,
    trainer: String,
}

struct DebugServerHandle {
    port: u16,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Drop for DebugServerHandle {
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
) -> Result<DebugServerHandle, String> {
    let server = Server::http(("127.0.0.1", port))
        .map_err(|e| format!("failed to bind debug-ui HTTP server: {e}"))?;
    let bound_port = server
        .server_addr()
        .to_ip()
        .map(|addr| addr.port())
        .unwrap_or(port);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();

    let join = thread::Builder::new()
        .name("debug-ui-plugin".into())
        .spawn(move || {
            while !stop_thread.load(Ordering::Relaxed) {
                match server.recv_timeout(Duration::from_millis(200)) {
                    Ok(Some(request)) => handle_request(request, &host, &instance_id),
                    Ok(None) => {}
                    Err(_) => {}
                }
            }
        })
        .map_err(|e| format!("failed to spawn debug-ui thread: {e}"))?;

    Ok(DebugServerHandle {
        port: bound_port,
        stop,
        join: Some(join),
    })
}

fn url_path(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

fn query_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let query = url.split('?').nth(1)?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then_some(v)
    })
}

fn query_param_u16(url: &str, key: &str) -> Option<u16> {
    let raw = query_param(url, key)?;
    parse_u16(raw).ok()
}

fn query_param_usize(url: &str, key: &str) -> Option<usize> {
    query_param(url, key)?.parse().ok()
}

fn read_body_bytes(request: &mut tiny_http::Request) -> Vec<u8> {
    let mut body = Vec::new();
    let _ = std::io::Read::read_to_end(request.as_reader(), &mut body);
    body
}

fn read_body_json<T: serde::de::DeserializeOwned>(
    request: &mut tiny_http::Request,
) -> Result<T, String> {
    let mut body = String::new();
    std::io::Read::read_to_string(request.as_reader(), &mut body)
        .map_err(|e| format!("failed to read request body: {e}"))?;
    serde_json::from_str(&body).map_err(|e| format!("invalid JSON body: {e}"))
}

fn respond_ok(request: tiny_http::Request) {
    let _ = request.respond(Response::from_string("ok"));
}

fn respond_not_found(request: tiny_http::Request) {
    let _ = request.respond(Response::from_string("not found").with_status_code(404));
}

fn respond_error(request: tiny_http::Request, message: String) {
    let _ = request.respond(Response::from_string(message).with_status_code(400));
}

fn respond_json<T: serde::Serialize>(request: tiny_http::Request, value: &T) {
    let body = serde_json::to_string(value).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"));
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    let _ = request.respond(Response::from_string(body).with_header(header));
}

fn respond_bytes(request: tiny_http::Request, bytes: Vec<u8>) {
    let header =
        Header::from_bytes(&b"Content-Type"[..], &b"application/octet-stream"[..]).unwrap();
    let _ = request.respond(Response::from_data(bytes).with_header(header));
}

fn expect_ack(result: Result<DebuggerHostResponse, String>) -> Result<(), String> {
    match result? {
        DebuggerHostResponse::Ack => Ok(()),
        other => Err(format!("host returned non-ack response: {other:?}")),
    }
}

fn handle_request(mut request: tiny_http::Request, host: &HostBridge, instance_id: &str) {
    let method = request.method().clone();
    let path = url_path(request.url()).to_string();

    match (&method, path.as_str()) {
        (Method::Get, "/") => {
            let header =
                Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
            let _ = request.respond(Response::from_string(DEBUGGER_HTML).with_header(header));
        }

        (Method::Get, "/api/state") => match host_state(host, instance_id) {
            Ok(state) => {
                let response: StateResponse = state.into();
                respond_json(request, &response);
            }
            Err(e) => respond_error(request, e),
        },

        (Method::Post, "/api/pause") => {
            let result = host_command(host, instance_id, DebuggerHostCommand::Pause);
            match expect_ack(result) {
                Ok(()) => respond_ok(request),
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/resume") => {
            let result = host_command(host, instance_id, DebuggerHostCommand::Resume);
            match expect_ack(result) {
                Ok(()) => respond_ok(request),
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/step") => {
            let result = host_command(host, instance_id, DebuggerHostCommand::StepInstruction);
            match expect_ack(result) {
                Ok(()) => respond_ok(request),
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/step-over") => {
            let result = host_command(host, instance_id, DebuggerHostCommand::StepOverInstruction);
            match expect_ack(result) {
                Ok(()) => respond_ok(request),
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/backstep") => {
            let result = host_command(host, instance_id, DebuggerHostCommand::BackstepInstruction);
            match expect_ack(result) {
                Ok(()) => respond_ok(request),
                Err(e) => respond_error(request, e),
            }
        }

        (Method::Post, "/api/breakpoint/set") => {
            match read_body_json::<BreakpointBody>(&mut request) {
                Ok(body) => {
                    let result = host_command(
                        host,
                        instance_id,
                        DebuggerHostCommand::SetBreakpoint {
                            address: body.addr,
                            condition: body.condition,
                        },
                    );
                    match expect_ack(result) {
                        Ok(()) => respond_ok(request),
                        Err(e) => respond_error(request, e),
                    }
                }
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/breakpoint/condition") => {
            match read_body_json::<BreakpointBody>(&mut request) {
                Ok(body) => {
                    let result = host_command(
                        host,
                        instance_id,
                        DebuggerHostCommand::SetBreakpointCondition {
                            address: body.addr,
                            condition: body.condition,
                        },
                    );
                    match expect_ack(result) {
                        Ok(()) => respond_ok(request),
                        Err(e) => respond_error(request, e),
                    }
                }
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/breakpoint/clear") => match read_body_json::<AddrBody>(&mut request) {
            Ok(body) => {
                let result = host_command(
                    host,
                    instance_id,
                    DebuggerHostCommand::ClearBreakpoint { address: body.addr },
                );
                match expect_ack(result) {
                    Ok(()) => respond_ok(request),
                    Err(e) => respond_error(request, e),
                }
            }
            Err(e) => respond_error(request, e),
        },
        (Method::Post, "/api/port-breakpoint/set") => {
            match read_body_json::<PortBreakpointBody>(&mut request) {
                Ok(body) => {
                    let result = host_command(
                        host,
                        instance_id,
                        DebuggerHostCommand::SetPortBreakpoint {
                            port: body.port,
                            mask: body.mask,
                            on_read: body.on_read,
                            on_write: body.on_write,
                        },
                    );
                    match expect_ack(result) {
                        Ok(()) => respond_ok(request),
                        Err(e) => respond_error(request, e),
                    }
                }
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/port-breakpoint/clear-all") => {
            let result = host_command(host, instance_id, DebuggerHostCommand::ClearPortBreakpoints);
            match expect_ack(result) {
                Ok(()) => respond_ok(request),
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/breakpoint/clear-all") => {
            let result = host_command(host, instance_id, DebuggerHostCommand::ClearAllBreakpoints);
            match expect_ack(result) {
                Ok(()) => respond_ok(request),
                Err(e) => respond_error(request, e),
            }
        }

        (Method::Post, "/api/watch/add") => match read_body_json::<AddWatchBody>(&mut request) {
            Ok(body) => {
                let result = host_command(
                    host,
                    instance_id,
                    DebuggerHostCommand::AddWatch { expr: body.expr },
                );
                match expect_ack(result) {
                    Ok(()) => respond_ok(request),
                    Err(e) => respond_error(request, e),
                }
            }
            Err(e) => respond_error(request, e),
        },
        (Method::Post, "/api/watch/remove") => {
            match read_body_json::<RemoveWatchBody>(&mut request) {
                Ok(body) => {
                    let result = host_command(
                        host,
                        instance_id,
                        DebuggerHostCommand::RemoveWatch { index: body.index },
                    );
                    match expect_ack(result) {
                        Ok(()) => respond_ok(request),
                        Err(e) => respond_error(request, e),
                    }
                }
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/watch/clear") => {
            let result = host_command(host, instance_id, DebuggerHostCommand::ClearWatches);
            match expect_ack(result) {
                Ok(()) => respond_ok(request),
                Err(e) => respond_error(request, e),
            }
        }

        (Method::Post, "/api/poke") => match read_body_json::<PokeBody>(&mut request) {
            Ok(body) => {
                let result = host_command(
                    host,
                    instance_id,
                    DebuggerHostCommand::Poke {
                        address: body.addr,
                        bytes: body.bytes,
                    },
                );
                match expect_ack(result) {
                    Ok(()) => respond_ok(request),
                    Err(e) => respond_error(request, e),
                }
            }
            Err(e) => respond_error(request, e),
        },

        (Method::Post, "/api/pok/parse") => {
            let bytes = read_body_bytes(&mut request);
            match pok::parse(&bytes) {
                Ok(file) => respond_json(request, &file),
                Err(error) => respond_error(request, error.to_string()),
            }
        }
        (Method::Post, "/api/pok/apply") => {
            let Some(trainer_index) = query_param_usize(request.url(), "trainer") else {
                respond_error(
                    request,
                    "missing or invalid trainer query parameter".to_string(),
                );
                return;
            };
            let bytes = read_body_bytes(&mut request);
            let file = match pok::parse(&bytes) {
                Ok(file) => file,
                Err(error) => {
                    respond_error(request, error.to_string());
                    return;
                }
            };
            let Some(trainer) = file.trainers.get(trainer_index) else {
                respond_error(
                    request,
                    format!("trainer index {trainer_index} out of range"),
                );
                return;
            };
            for entry in &trainer.entries {
                let command = if entry.ignores_bank() {
                    DebuggerHostCommand::Poke {
                        address: entry.address,
                        bytes: vec![entry.value],
                    }
                } else {
                    DebuggerHostCommand::PokeBank {
                        bank: entry.bank,
                        address: entry.address,
                        bytes: vec![entry.value],
                    }
                };
                if let Err(error) = expect_ack(host_command(host, instance_id, command)) {
                    respond_error(request, error);
                    return;
                }
            }
            respond_json(
                request,
                &PokApplyResponse {
                    applied: trainer.entries.len(),
                    trainer: trainer.name.clone(),
                },
            );
        }

        (Method::Post, "/api/memory-window") => {
            match read_body_json::<MemoryWindowBody>(&mut request) {
                Ok(body) => {
                    let result = host_command(
                        host,
                        instance_id,
                        DebuggerHostCommand::SetMemoryWindow {
                            address: body.addr,
                            len: body.len,
                        },
                    );
                    match expect_ack(result) {
                        Ok(()) => respond_ok(request),
                        Err(e) => respond_error(request, e),
                    }
                }
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/disassembly-window") => {
            match read_body_json::<AddrBody>(&mut request) {
                Ok(body) => {
                    let result = host_command(
                        host,
                        instance_id,
                        DebuggerHostCommand::SetDisassemblyWindow { address: body.addr },
                    );
                    match expect_ack(result) {
                        Ok(()) => respond_ok(request),
                        Err(e) => respond_error(request, e),
                    }
                }
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/disassembly-follow-pc") => {
            let result = host_command(host, instance_id, DebuggerHostCommand::FollowPc);
            match expect_ack(result) {
                Ok(()) => respond_ok(request),
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/disassembly-line-count") => {
            match read_body_json::<CountBody>(&mut request) {
                Ok(body) => {
                    let result = host_command(
                        host,
                        instance_id,
                        DebuggerHostCommand::SetDisassemblyLineCount { count: body.count },
                    );
                    match expect_ack(result) {
                        Ok(()) => respond_ok(request),
                        Err(e) => respond_error(request, e),
                    }
                }
                Err(e) => respond_error(request, e),
            }
        }

        (Method::Get, "/api/memory-bytes") => {
            let addr = query_param_u16(request.url(), "addr");
            let len = query_param_u16(request.url(), "len");
            match (addr, len) {
                (Some(addr), Some(len)) => {
                    let response = host_command(
                        host,
                        instance_id,
                        DebuggerHostCommand::ReadMemory { address: addr, len },
                    );
                    match response {
                        Ok(DebuggerHostResponse::Bytes(bytes)) => respond_bytes(request, bytes),
                        Ok(other) => {
                            respond_error(request, format!("unexpected host response: {other:?}"))
                        }
                        Err(e) => respond_error(request, e),
                    }
                }
                _ => respond_error(
                    request,
                    "missing or invalid addr/len query params".to_string(),
                ),
            }
        }

        (Method::Post, "/api/load-state") => {
            let bytes = read_body_bytes(&mut request);
            let result = host_command(host, instance_id, DebuggerHostCommand::LoadState { bytes });
            match expect_ack(result) {
                Ok(()) => respond_ok(request),
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Post, "/api/load-snapshot") => {
            let extension = query_param(request.url(), "ext").unwrap_or("").to_string();
            let bytes = read_body_bytes(&mut request);
            let result = host_command(
                host,
                instance_id,
                DebuggerHostCommand::LoadSnapshot { extension, bytes },
            );
            match expect_ack(result) {
                Ok(()) => respond_ok(request),
                Err(e) => respond_error(request, e),
            }
        }

        (Method::Post, "/api/assemble") => match read_body_json::<AssembleBody>(&mut request) {
            Ok(body) => {
                let response = host_command(
                    host,
                    instance_id,
                    DebuggerHostCommand::Assemble {
                        addr: body.addr,
                        text: body.text,
                    },
                );
                match response {
                    Ok(DebuggerHostResponse::AssembledBytes(bytes)) => {
                        respond_json(request, &AssembleResponse { bytes })
                    }
                    Ok(other) => {
                        respond_error(request, format!("unexpected host response: {other:?}"))
                    }
                    Err(e) => respond_error(request, e),
                }
            }
            Err(e) => respond_error(request, e),
        },

        (Method::Get, "/api/save-state") => {
            let response = host_command(host, instance_id, DebuggerHostCommand::SaveState);
            match response {
                Ok(DebuggerHostResponse::Bytes(bytes)) => respond_bytes(request, bytes),
                Ok(other) => respond_error(request, format!("unexpected host response: {other:?}")),
                Err(e) => respond_error(request, e),
            }
        }
        (Method::Get, "/api/save-snapshot") => {
            let extension = query_param(request.url(), "ext").unwrap_or("").to_string();
            let response = host_command(
                host,
                instance_id,
                DebuggerHostCommand::SaveSnapshot { extension },
            );
            match response {
                Ok(DebuggerHostResponse::Bytes(bytes)) => respond_bytes(request, bytes),
                Ok(other) => respond_error(request, format!("unexpected host response: {other:?}")),
                Err(e) => respond_error(request, e),
            }
        }

        _ => respond_not_found(request),
    }
}

pub(super) fn describe() -> DebuggerPluginInfo {
    DebuggerPluginInfo {
        selector: SELECTOR.to_string(),
        display_name: "Debug UI".to_string(),
        summary: "Serves the browser debugger on localhost over HTTP.".to_string(),
        options: vec![DebuggerPluginOptionInfo {
            key: "port".to_string(),
            value_hint: "0-65535".to_string(),
            description: "HTTP listen port (0 picks an OS-assigned free port).".to_string(),
        }],
    }
}

pub(super) fn start(
    options: &[NamedValue],
    instance_id: &str,
    host: &HostBridge,
) -> Result<Running, String> {
    let port = option_u16(options, "port")?.unwrap_or(0);
    let server = spawn_server(host.clone(), instance_id.to_string(), port)
        .map_err(|e| format!("debug-ui: failed to start the HTTP server: {e}"))?;
    let web_url = format!("http://127.0.0.1:{}/", server.port);
    Ok(Running::new(Some(web_url), None, server))
}
