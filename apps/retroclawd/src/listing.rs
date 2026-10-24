//! Word-wrapped text for the listings the application prints itself (`--list-plugins`, the machine options appended to
//! `--help`). clap's `wrap_help` only wraps what clap renders; these are plain strings, so they are wrapped here to the terminal's
//! width, with a hanging indent so a long description stays under its flag.

use std::fmt::Write;

use crate::plugin_bridge::StartupReport;

/// Width used when output is not a terminal (and `COLUMNS` is not set).
const DEFAULT_WIDTH: usize = 100;
/// Narrowest and widest text column used: a very wide window reads badly, a tiny one cannot hold the indents.
const MIN_WIDTH: usize = 50;
const MAX_WIDTH: usize = 120;

/// The text width for listings: the terminal's, else `COLUMNS`, else 100; clamped to 50-120.
pub(crate) fn text_width() -> usize {
    let width = terminal_size::terminal_size()
        .map(|(w, _)| usize::from(w.0))
        .or_else(|| std::env::var("COLUMNS").ok()?.trim().parse().ok())
        .unwrap_or(DEFAULT_WIDTH);
    width.clamp(MIN_WIDTH, MAX_WIDTH)
}

/// Appends `text` wrapped at `width` columns: the first line starts with `first`, the others with `rest`. Breaks only at
/// whitespace; a word longer than the line (a path) stands on a line of its own. Ends with a newline.
pub(crate) fn wrap_into(out: &mut String, text: &str, width: usize, first: &str, rest: &str) {
    let mut line = String::from(first);
    let mut line_has_word = false;
    for word in text.split_whitespace() {
        let needed = line.chars().count() + usize::from(line_has_word) + word.chars().count();
        if line_has_word && needed > width {
            out.push_str(&line);
            out.push('\n');
            line = String::from(rest);
            line_has_word = false;
        }
        if line_has_word {
            line.push(' ');
        }
        line.push_str(word);
        line_has_word = true;
    }
    out.push_str(line.trim_end());
    out.push('\n');
}

/// One option: `--flag <hint>` on a line, its description wrapped below.
pub(crate) fn option_entry(
    out: &mut String,
    flag_line: &str,
    description: &str,
    width: usize,
    indent: &str,
) {
    out.push_str(indent);
    out.push_str(flag_line);
    out.push('\n');
    let body = format!("{indent}    ");
    wrap_into(out, description, width, &body, &body);
}

/// The `--list-plugins` text for a startup report (or the error that stopped the scan) and the built-in debuggers.
pub(crate) fn plugins(
    report: Result<&StartupReport, &str>,
    debuggers: &[crate::debuggers::api::DebuggerPluginInfo],
    width: usize,
) -> String {
    let mut out = String::new();
    out.push_str("Host\n");
    wrap_into(
        &mut out,
        &format!(
            "plugin ABI {}.{} (a plugin needs the same major and at least this minor), protocol {}, machine profile contract {}",
            plugin_api::ABI_MAJOR_V1,
            plugin_api::ABI_MINOR_V1,
            plugin_api::PROTOCOL_VERSION,
            plugin_api::MACHINE_PROFILE_CONTRACT_VERSION
        ),
        width,
        "  ",
        "  ",
    );

    out.push_str("\nMachine plugins\n");
    match report {
        Ok(report) => {
            if report.plugins.is_empty() {
                out.push_str("  (none found)\n");
            }
            for plugin in &report.plugins {
                let _ = writeln!(
                    out,
                    "  {}  (ABI {}.{})",
                    crate::version::plugin_line(plugin),
                    plugin.abi.0,
                    plugin.abi.1
                );
                wrap_into(
                    &mut out,
                    &format!("machines: {}", plugin.machine_ids.join(", ")),
                    width,
                    "    ",
                    "              ",
                );
                wrap_into(
                    &mut out,
                    &format!("library:  {}", plugin.source),
                    width,
                    "    ",
                    "              ",
                );
            }
            if report.profiles.iter().any(|p| !p.options.is_empty()) {
                out.push_str("\nMachine options\n");
                let mut profiles: Vec<_> = report.profiles.iter().collect();
                profiles.sort_by(|a, b| a.machine_id.cmp(&b.machine_id));
                for profile in profiles.iter().filter(|p| !p.options.is_empty()) {
                    let _ = writeln!(out, "  {}", profile.machine_id);
                    for opt in &profile.options {
                        option_entry(
                            &mut out,
                            &format!(
                                "--{} <{}>",
                                opt.flag,
                                crate::machine_option_value_hint(&opt.kind)
                            ),
                            &opt.description,
                            width,
                            "    ",
                        );
                    }
                }
            }
            for warning in &report.warnings {
                wrap_into(
                    &mut out,
                    &format!("warning: machine plugin scan: {warning}"),
                    width,
                    "",
                    "  ",
                );
            }
        }
        Err(e) => wrap_into(
            &mut out,
            &format!("machine plugin discovery failed: {e}"),
            width,
            "  ",
            "  ",
        ),
    }

    out.push_str("\nDebuggers (built in, selected with --plugin)\n");
    if debuggers.is_empty() {
        wrap_into(
            &mut out,
            "(none: this build has no debug-ui, debug-zrcp or debug-log feature)",
            width,
            "  ",
            "  ",
        );
    }
    for info in debuggers {
        let _ = writeln!(out, "  {} ({})", info.selector, info.display_name);
        wrap_into(&mut out, &info.summary, width, "    ", "    ");
        if info.options.is_empty() {
            out.push_str("    options: (none)\n");
        } else {
            out.push_str("    options:\n");
            for opt in &info.options {
                option_entry(
                    &mut out,
                    &format!("{} <{}>", opt.key, opt.value_hint),
                    &opt.description,
                    width,
                    "      ",
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrapped(text: &str, width: usize, first: &str, rest: &str) -> String {
        let mut out = String::new();
        wrap_into(&mut out, text, width, first, rest);
        out
    }

    #[test]
    fn long_text_wraps_under_a_hanging_indent() {
        let out = wrapped("aaa bbb ccc ddd eee", 12, "- ", "  ");
        assert_eq!(out, "- aaa bbb\n  ccc ddd\n  eee\n");
        assert!(out.lines().all(|l| l.chars().count() <= 12));
    }

    #[test]
    fn a_word_longer_than_the_line_stays_whole_on_its_own_line() {
        let out = wrapped(
            "see C:\\very\\long\\path\\that\\does\\not\\fit ok",
            16,
            "  ",
            "  ",
        );
        assert_eq!(
            out,
            "  see\n  C:\\very\\long\\path\\that\\does\\not\\fit\n  ok\n"
        );
    }

    #[test]
    fn short_and_empty_text_stay_on_one_line() {
        assert_eq!(wrapped("hi", 40, "> ", "  "), "> hi\n");
        assert_eq!(wrapped("", 40, "> ", "  "), ">\n");
    }

    #[test]
    fn the_width_is_clamped() {
        let w = text_width();
        assert!((MIN_WIDTH..=MAX_WIDTH).contains(&w));
    }

    #[test]
    fn the_listing_fits_its_width_and_names_everything() {
        let report = StartupReport {
            plugins: vec![crate::plugin_bridge::StartupPluginReport {
                plugin_id: "retroclawd.cpc.machine".into(),
                plugin_version: "0.1.0+98.b7c8d9e0f1.2026-10-17".into(),
                abi: (1, 2),
                source: "plugins/cpc_machine_plugin.dll".into(),
                machine_ids: vec!["cpc464".into(), "cpc664".into(), "cpc6128".into()],
            }],
            profiles: Vec::new(),
            warnings: vec!["a warning ".repeat(12)],
        };
        let out = plugins(Ok(&report), &[], 60);
        assert!(out.contains("  cpc 0.1.0 build 98 (b7c8d9e0f1, 2026-10-17)  (ABI 1.2)\n"));
        assert!(out.contains("    machines: cpc464, cpc664, cpc6128\n"));
        assert!(out.contains("Debuggers"));
        for l in out.lines() {
            assert!(l.chars().count() <= 60, "{l:?}");
        }
    }
}
