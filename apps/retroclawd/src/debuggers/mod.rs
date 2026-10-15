//! The debuggers: a browser UI (`debug-ui`), a ZRCP-style TCP command interface
//! (`debug-zrcp`) and the `tracing` log sink (`debug-log`).
//!
//! They are ordinary modules of the application, each behind a cargo feature of
//! the same name (all on by default), not plugins: there is no library to load,
//! no ABI and no serialisation between a debugger and the application. A
//! debugger talks to the emulation thread through [`HostBridge`], an in-process
//! request/response handle onto [`host::HostApiContext`].
//!
//! Machines remain plugins (`plugin-api`); only the debuggers were folded in.

// A build with every debugger feature off keeps the machinery but uses none of it.
#![cfg_attr(
    not(any(feature = "debug-ui", feature = "debug-zrcp")),
    allow(dead_code, unused_imports)
)]

pub(crate) mod api;
mod host;
#[cfg(feature = "debug-log")]
mod log;
#[cfg(feature = "debug-ui")]
mod ui;
#[cfg(feature = "debug-zrcp")]
mod zrcp;

use std::sync::Arc;

use plugin_api::NamedValue;

use crate::debugger::{DebuggerCommandSender, DebuggerStateHandle};
use api::{DebuggerHostRequest, DebuggerHostResponse, DebuggerPluginInfo};

pub(crate) use host::format_debug_value;

/// What a debugger module asks of the application. Implemented by
/// [`host::HostApiContext`]; tests substitute a fake.
pub(crate) trait HostApi: Send + Sync {
    fn request(&self, request: DebuggerHostRequest) -> Result<DebuggerHostResponse, String>;
}

/// A cloneable handle a debugger module (and its server threads) uses to reach
/// the application.
#[derive(Clone)]
pub(crate) struct HostBridge {
    api: Arc<dyn HostApi>,
}

impl HostBridge {
    pub(crate) fn new(api: Arc<dyn HostApi>) -> Self {
        Self { api }
    }

    pub(crate) fn call(
        &self,
        request: DebuggerHostRequest,
    ) -> Result<DebuggerHostResponse, String> {
        self.api.request(request)
    }
}

/// A started debugger. Dropping it stops the debugger (its server thread is
/// joined, its listener closed).
pub(crate) struct Running {
    pub(crate) web_url: Option<String>,
    pub(crate) dezog_addr: Option<String>,
    _guard: Box<dyn Send>,
}

impl Running {
    pub(super) fn new(
        web_url: Option<String>,
        dezog_addr: Option<String>,
        guard: impl Send + 'static,
    ) -> Self {
        Self {
            web_url,
            dezog_addr,
            _guard: Box::new(guard),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LaunchSelection {
    pub(crate) selector: String,
    pub(crate) options: Vec<NamedValue>,
}

#[derive(Debug, Clone)]
pub(crate) struct LaunchOptions {
    pub(crate) selections: Vec<LaunchSelection>,
}

pub(crate) struct StartedDebugger {
    pub(crate) selector: String,
    pub(crate) web_url: Option<String>,
    pub(crate) dezog_addr: Option<String>,
    _running: Running,
}

/// The debuggers this build contains.
#[allow(clippy::vec_init_then_push)]
fn built_in_infos() -> Vec<DebuggerPluginInfo> {
    let mut infos: Vec<DebuggerPluginInfo> = Vec::new();
    #[cfg(feature = "debug-log")]
    infos.push(log::describe());
    #[cfg(feature = "debug-ui")]
    infos.push(ui::describe());
    #[cfg(feature = "debug-zrcp")]
    infos.push(zrcp::describe());
    infos.sort_by(|a, b| a.selector.cmp(&b.selector));
    infos
}

/// For `--list-plugins`.
pub(crate) fn built_in() -> Vec<DebuggerPluginInfo> {
    built_in_infos()
}

/// Maps a user-written selector (including the old aliases) to the debugger's
/// own name.
pub(crate) fn canonical_selector(selector: &str) -> Option<&'static str> {
    if selector.eq_ignore_ascii_case("debugger") || selector.eq_ignore_ascii_case("debug-ui") {
        Some("debug-ui")
    } else if selector.eq_ignore_ascii_case("debug-log") {
        Some("debug-log")
    } else if selector.eq_ignore_ascii_case("debug-zrcp")
        || selector.eq_ignore_ascii_case("debug-tcp")
    {
        Some("debug-zrcp")
    } else {
        None
    }
}

fn start_one(selector: &str, options: &[NamedValue], host: &HostBridge) -> Result<Running, String> {
    let instance = host::PRIMARY_INSTANCE_ID;
    // `options` and `host` go unused when no debugger feature is compiled in.
    let _ = (&options, &host, instance);
    match selector {
        #[cfg(feature = "debug-log")]
        "debug-log" => log::start(options, instance),
        #[cfg(feature = "debug-ui")]
        "debug-ui" => ui::start(options, instance, host),
        #[cfg(feature = "debug-zrcp")]
        "debug-zrcp" => zrcp::start(options, instance, host),
        other => Err(format!(
            "the {other} debugger is not part of this build (built-in debuggers: {})",
            built_in_infos()
                .iter()
                .map(|info| info.selector.clone())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub(crate) fn start_debuggers(
    options: LaunchOptions,
    commands: DebuggerCommandSender,
    state: DebuggerStateHandle,
    machine_id: String,
) -> Result<Vec<StartedDebugger>, String> {
    let mut started = Vec::new();
    for selection in options.selections {
        let Some(selector) = canonical_selector(&selection.selector) else {
            return Err(format!(
                "unsupported debugger selector {:?}; expected one of debug-ui, debug-log, debug-zrcp (or legacy debug-tcp)",
                selection.selector
            ));
        };
        let host = HostBridge::new(host::host_api(
            commands.clone(),
            state.clone(),
            machine_id.clone(),
        ));
        let running = start_one(selector, &selection.options, &host)?;
        started.push(StartedDebugger {
            selector: selector.to_string(),
            web_url: running.web_url.clone(),
            dezog_addr: running.dezog_addr.clone(),
            _running: running,
        });
    }
    Ok(started)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_normalization_maps_the_aliases() {
        assert_eq!(canonical_selector("debug-zrcp"), Some("debug-zrcp"));
        assert_eq!(canonical_selector("debug-tcp"), Some("debug-zrcp"));
        assert_eq!(canonical_selector("debugger"), Some("debug-ui"));
        assert_eq!(canonical_selector("debug-ui"), Some("debug-ui"));
        assert_eq!(canonical_selector("debug-log"), Some("debug-log"));
        assert_eq!(canonical_selector("something-else"), None);
    }

    #[cfg(feature = "debug-log")]
    #[test]
    fn a_debug_log_selection_starts_and_writes_the_requested_file() {
        use plugin_api::DebugValue;
        // Regression: the GUI launcher used to filter `debug-log` out of the
        // selections (assuming a host-side sink existed), so `--plugin
        // debug-log:out.log:debug` never started and wrote nothing.
        let path = std::env::temp_dir().join(format!(
            "retroclawd_debuggers_debug_log_test_{}.log",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let (commands, _commands_rx) = std::sync::mpsc::channel();
        let started = start_debuggers(
            LaunchOptions {
                selections: vec![LaunchSelection {
                    selector: "debug-log".to_string(),
                    options: vec![
                        NamedValue {
                            key: "output".to_string(),
                            value: DebugValue::Text(path.to_string_lossy().into_owned()),
                        },
                        NamedValue {
                            key: "level".to_string(),
                            value: DebugValue::Text("debug".to_string()),
                        },
                    ],
                }],
            },
            commands,
            crate::debugger::DebuggerLink::new_handle(),
            "cpc464".to_string(),
        )
        .expect("debug-log must start");
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].selector, "debug-log");
        let text = std::fs::read_to_string(&path).expect("the requested log file must exist");
        assert!(text.contains("debug-log started"), "{text:?}");
        drop(started);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn every_built_in_debugger_describes_itself_under_its_own_selector() {
        for info in built_in_infos() {
            assert_eq!(
                canonical_selector(&info.selector),
                Some(info.selector.as_str())
            );
        }
    }
}
