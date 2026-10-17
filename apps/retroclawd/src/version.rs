//! Version, build number and git commit of retroclawd itself and of the machine plugins it found (`--version`, the help text, the
//! `--list-plugins` listing and the overlay's version panel).
//!
//! The host is stamped by its own `build.rs`; a plugin carries the same stamp as its descriptor's `version` string (see the
//! `build-info` crate in the shared repository), so each reports the commit of the repository it was built in -- the host's, or its
//! machine family's. A plugin built without the stamp shows only its plain version.

use build_info::{BuildInfo, Info};

use crate::plugin_bridge::{StartupPluginReport, StartupReport};

/// What this executable was built from.
pub(crate) const HOST: BuildInfo = build_info::build_info!();

pub(crate) fn host_line() -> String {
    format!("retroclawd {HOST}")
}

/// `retroclawd.cpc.machine` -> `cpc`: the middle of a plugin id, for the short lines.
fn short_id(id: &str) -> &str {
    id.strip_prefix("retroclawd.")
        .and_then(|rest| rest.strip_suffix(".machine"))
        .unwrap_or(id)
}

/// `cpc 0.1.0 build 98 (b7c8d9e0f1, 2026-10-17)`.
pub(crate) fn plugin_line(plugin: &StartupPluginReport) -> String {
    format!(
        "{} {}",
        short_id(&plugin.plugin_id),
        Info::parse(&plugin.plugin_version)
    )
}

/// The host line, then one line per machine plugin found (`--version`, the top of `--help`).
pub(crate) fn summary(report: Option<&StartupReport>) -> String {
    let mut out = host_line();
    out.push('\n');
    match report {
        Some(report) if !report.plugins.is_empty() => {
            out.push_str("machine plugins:\n");
            for plugin in &report.plugins {
                out.push_str(&format!("  {}\n", plugin_line(plugin)));
            }
        }
        Some(_) => out.push_str("machine plugins: (none found)\n"),
        None => {}
    }
    out
}

/// The overlay's version panel: the host and the plugin that serves `machine_id`.
pub(crate) fn overlay_text(machine_id: &str, report: Option<&StartupReport>) -> String {
    let mut out = format!("{}\n", host_line());
    let plugin = report.and_then(|r| {
        r.plugins
            .iter()
            .find(|p| p.machine_ids.iter().any(|id| id == machine_id))
    });
    match plugin {
        Some(plugin) => out.push_str(&format!("plugin {}\n", plugin_line(plugin))),
        None => out.push_str(&format!("plugin for {machine_id}: unknown\n")),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(id: &str, version: &str, machines: &[&str]) -> StartupPluginReport {
        StartupPluginReport {
            plugin_id: id.to_string(),
            plugin_version: version.to_string(),
            abi: (1, 1),
            source: "x".to_string(),
            machine_ids: machines.iter().map(|m| m.to_string()).collect(),
        }
    }

    fn report() -> StartupReport {
        StartupReport {
            plugins: vec![
                plugin(
                    "retroclawd.cpc.machine",
                    "0.1.0+98.b7c8d9e0f1.2026-10-17",
                    &["cpc464", "cpc6128"],
                ),
                plugin("retroclawd.old.machine", "0.1.0", &["old"]),
            ],
            profiles: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn the_host_is_stamped_with_a_version_and_a_build_number() {
        let line = host_line();
        assert!(line.starts_with("retroclawd 0."), "{line}");
        assert!(line.contains(" build "), "{line}");
        assert_eq!(HOST.version, env!("CARGO_PKG_VERSION"));
        // Built inside this git repository: the commit is a real hash, not the fallback.
        assert_ne!(HOST.commit, "unknown", "no git commit was stamped");
    }

    #[test]
    fn plugin_lines_read_the_stamp_and_tolerate_a_plain_version() {
        let report = report();
        assert_eq!(
            plugin_line(&report.plugins[0]),
            "cpc 0.1.0 build 98 (b7c8d9e0f1, 2026-10-17)"
        );
        assert_eq!(plugin_line(&report.plugins[1]), "old 0.1.0");
        let text = summary(Some(&report));
        assert!(
            text.contains("machine plugins:\n  cpc 0.1.0 build 98"),
            "{text}"
        );
        assert!(
            summary(Some(&StartupReport {
                plugins: Vec::new(),
                profiles: Vec::new(),
                warnings: Vec::new()
            }))
            .contains("(none found)")
        );
    }

    #[test]
    fn the_overlay_names_the_plugin_that_serves_the_machine() {
        let report = report();
        let text = overlay_text("cpc6128", Some(&report));
        assert!(text.contains("plugin cpc 0.1.0 build 98"), "{text}");
        assert!(overlay_text("zx81", Some(&report)).contains("plugin for zx81: unknown"));
    }
}
