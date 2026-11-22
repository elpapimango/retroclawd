//! What `debuggers` offers in a build without the `debug` feature (`rcd`): no debuggers, just the few names the rest
//! of the application mentions.

#[allow(dead_code)]
#[path = "debuggers/api.rs"]
pub(crate) mod api;

use api::DebuggerPluginInfo;
use plugin_api::DebugValue;

/// A debugger the user asked for. There is none without the `debug` feature, so the list stays empty.
#[derive(Debug, Clone)]
pub(crate) struct LaunchSelection;

/// For `--list-plugins`: no debuggers are built in.
pub(crate) fn built_in() -> Vec<DebuggerPluginInfo> {
    Vec::new()
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
