//! `debug-log`: routes the application's `tracing` diagnostics to stdout or a
//! file. Selected with `--plugin debug-log:<output>:<level>`; the only thing in
//! the application that installs a `tracing` subscriber (Bevy's `LogPlugin` is
//! disabled).

use std::fs::File;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use plugin_api::{DebugValue, NamedValue};

use super::Running;
use super::api::{DebuggerPluginInfo, DebuggerPluginOptionInfo};

pub(super) const SELECTOR: &str = "debug-log";

/// Logs the stop line when the debugger is dropped.
struct StopLine {
    output: String,
    level: tracing::Level,
    engine_level: tracing::Level,
}

impl Drop for StopLine {
    fn drop(&mut self) {
        tracing::info!(
            target: "retroclawd.debugger",
            "debug-log stopped: output={} level={} engine_level={}",
            self.output,
            self.level,
            self.engine_level
        );
    }
}

enum Sink {
    Stdout,
    File(File),
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Sink::Stdout => std::io::stdout().write(buf),
            Sink::File(file) => file.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Sink::Stdout => std::io::stdout().flush(),
            Sink::File(file) => file.flush(),
        }
    }
}

static LOG_SINK: OnceLock<Mutex<Sink>> = OnceLock::new();

fn sink() -> &'static Mutex<Sink> {
    LOG_SINK.get_or_init(|| Mutex::new(Sink::Stdout))
}

struct SinkMakeWriter;
struct SinkWriter;

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SinkMakeWriter {
    type Writer = SinkWriter;

    fn make_writer(&'a self) -> Self::Writer {
        SinkWriter
    }
}

impl Write for SinkWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut sink) = sink().lock() {
            sink.write(buf)
        } else {
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Ok(mut sink) = sink().lock() {
            sink.flush()
        } else {
            Ok(())
        }
    }
}

fn option_text_any(options: &[NamedValue], keys: &[&str]) -> Option<String> {
    options.iter().rev().find_map(|opt| {
        if keys.iter().any(|key| opt.key.eq_ignore_ascii_case(key)) {
            match &opt.value {
                DebugValue::Text(text) => Some(text.clone()),
                DebugValue::Bool(value) => Some(value.to_string()),
                DebugValue::U64(value) => Some(value.to_string()),
                DebugValue::I64(value) => Some(value.to_string()),
                DebugValue::Bytes(_) => None,
            }
        } else {
            None
        }
    })
}

fn parse_level(level_text: Option<String>) -> Result<tracing::Level, String> {
    let Some(level) = level_text else {
        return Ok(tracing::Level::INFO);
    };
    match level.trim().to_ascii_lowercase().as_str() {
        "trace" => Ok(tracing::Level::TRACE),
        "debug" => Ok(tracing::Level::DEBUG),
        "info" => Ok(tracing::Level::INFO),
        "warn" | "warning" => Ok(tracing::Level::WARN),
        "error" => Ok(tracing::Level::ERROR),
        other => Err(format!("invalid debug-log level {other:?}")),
    }
}

fn configure_log_sink(spec: Option<String>) -> Result<String, String> {
    let requested = spec
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("stdout");

    let (sink_value, label) = if requested.eq_ignore_ascii_case("stdout") || requested == "-" {
        (Sink::Stdout, "stdout".to_string())
    } else {
        let file =
            File::create(requested).map_err(|e| format!("debug-log output {requested}: {e}"))?;
        (Sink::File(file), requested.to_string())
    };

    if let Ok(mut sink_guard) = sink().lock() {
        *sink_guard = sink_value;
    }

    Ok(label)
}

/// Crate-name prefixes of the third-party engine/graphics/audio stack. At
/// `level=debug` these alone produce tens of thousands of lines (every WGSL
/// shader compile logs each overload `naga` considers), burying the
/// emulator's own diagnostics, so they are held to `engine_level` (default
/// `warn`) regardless of `level`.
const ENGINE_CRATE_PREFIXES: &[&str] = &[
    "bevy",
    "wgpu",
    "naga",
    "winit",
    "cosmic_text",
    "gilrs",
    "cpal",
    "rodio",
    "symphonia",
    "calloop",
    "ash",
    "gpu_alloc",
    "gpu_descriptor",
    "mio",
    "offset_allocator",
    "polling",
    "sctk",
    "smithay",
    "async_io",
    "async_executor",
    "blocking",
    "zbus",
    "accesskit",
    "tracing",
];

fn is_engine_target(target: &str) -> bool {
    let krate = target.split("::").next().unwrap_or(target);
    ENGINE_CRATE_PREFIXES
        .iter()
        .any(|prefix| krate.starts_with(prefix))
}

/// Whether an event at `event_level` from `target` passes: everything must
/// satisfy `level`, and the engine stack additionally `engine_level`.
/// (`tracing` orders levels by verbosity: ERROR < WARN < INFO < DEBUG < TRACE.)
fn event_enabled(
    target: &str,
    event_level: &tracing::Level,
    level: tracing::Level,
    engine_level: tracing::Level,
) -> bool {
    *event_level <= level && (!is_engine_target(target) || *event_level <= engine_level)
}

fn init_tracing_subscriber_once(
    level: tracing::Level,
    engine_level: tracing::Level,
) -> Result<(), String> {
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    let init = INIT.get_or_init(|| {
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(SinkMakeWriter)
            .with_ansi(false)
            .with_target(true)
            .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339())
            .with_filter(tracing_subscriber::filter::filter_fn(move |meta| {
                event_enabled(meta.target(), meta.level(), level, engine_level)
            }));
        tracing_subscriber::registry()
            .with(layer)
            .try_init()
            .map_err(|e| format!("failed to initialize tracing subscriber: {e}"))
    });
    match init {
        Ok(()) => Ok(()),
        Err(err) => Err(err.clone()),
    }
}

pub(super) fn describe() -> DebuggerPluginInfo {
    DebuggerPluginInfo {
        selector: SELECTOR.to_string(),
        display_name: "Debug log".to_string(),
        summary: "Routes retroclawd diagnostics to stdout or a log file.".to_string(),
        options: vec![
            DebuggerPluginOptionInfo {
                key: "output".to_string(),
                value_hint: "stdout|<path>".to_string(),
                description:
                    "Log destination. In CLI shorthand this is the first bare :token after debug-log."
                        .to_string(),
            },
            DebuggerPluginOptionInfo {
                key: "level".to_string(),
                value_hint: "trace|debug|info|warn|error".to_string(),
                description: "Maximum tracing level accepted by this sink (default info). In CLI shorthand this is the second bare :token after debug-log."
                    .to_string(),
            },
            DebuggerPluginOptionInfo {
                key: "engine_level".to_string(),
                value_hint: "trace|debug|info|warn|error".to_string(),
                description: "Maximum level for the third-party engine stack (bevy, wgpu, naga, winit, audio); default warn, so level=debug shows the emulator's own diagnostics instead of shader-compiler chatter. Never more verbose than level."
                    .to_string(),
            },
        ],
    }
}

pub(super) fn start(options: &[NamedValue], instance_id: &str) -> Result<Running, String> {
    let output = configure_log_sink(option_text_any(options, &["output", "file", "path"]))?;
    let level = parse_level(option_text_any(options, &["level"]))?;
    let engine_level = match option_text_any(options, &["engine_level"]) {
        Some(text) => parse_level(Some(text))?,
        None => tracing::Level::WARN,
    };

    init_tracing_subscriber_once(level, engine_level)?;

    tracing::info!(
        target: "retroclawd.debugger",
        "debug-log started: instance={instance_id} output={output} level={level} engine_level={engine_level}"
    );

    Ok(Running::new(
        None,
        None,
        StopLine {
            output,
            level,
            engine_level,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_level_supports_common_levels() {
        assert_eq!(
            parse_level(Some("trace".to_string())).unwrap(),
            tracing::Level::TRACE
        );
        assert_eq!(
            parse_level(Some("DEBUG".to_string())).unwrap(),
            tracing::Level::DEBUG
        );
        assert_eq!(parse_level(None).unwrap(), tracing::Level::INFO);
    }

    #[test]
    fn engine_crates_are_held_to_engine_level_but_emulator_targets_follow_level() {
        use tracing::Level;
        let enabled = |target, level, max, engine| event_enabled(target, &level, max, engine);
        // level=debug, engine_level=warn: emulator diagnostics pass, shader
        // compiler chatter doesn't, engine warnings still do.
        assert!(enabled("cpc::bus", Level::DEBUG, Level::DEBUG, Level::WARN));
        assert!(enabled(
            "retroclawd.debugger",
            Level::INFO,
            Level::DEBUG,
            Level::WARN
        ));
        assert!(enabled(
            "retroclawd::input",
            Level::DEBUG,
            Level::DEBUG,
            Level::WARN
        ));
        assert!(!enabled(
            "naga::front::wgsl::lower",
            Level::DEBUG,
            Level::DEBUG,
            Level::WARN
        ));
        assert!(!enabled(
            "bevy_app::app",
            Level::INFO,
            Level::DEBUG,
            Level::WARN
        ));
        assert!(enabled(
            "wgpu_hal::vulkan",
            Level::WARN,
            Level::DEBUG,
            Level::WARN
        ));
        // engine_level=debug restores the chatter; level still caps it.
        assert!(enabled(
            "naga::front",
            Level::DEBUG,
            Level::DEBUG,
            Level::DEBUG
        ));
        assert!(!enabled(
            "naga::front",
            Level::DEBUG,
            Level::INFO,
            Level::DEBUG
        ));
        // Nothing passes above `level`, engine or not.
        assert!(!enabled("cpc::bus", Level::DEBUG, Level::INFO, Level::WARN));
    }
}
