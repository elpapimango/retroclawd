//! Word-wrapped text for the listings the application prints itself (`--list-plugins`, the machine options appended to
//! `--help`). clap's `wrap_help` only wraps what clap renders; these are plain strings, so they are wrapped here to the terminal's
//! width, with a hanging indent so a long description stays under its flag.

use std::fmt::Write;

use plugin_api::MachineProfile;

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

/// One declared option as printed: the flag without `--`, its value hint and its description.
#[derive(Clone, Debug, PartialEq, Eq)]
struct OptLine {
    flag: String,
    hint: String,
    desc: String,
}

/// `acsi3` -> (`acsi`, 3).
fn split_index(flag: &str) -> Option<(&str, u32)> {
    let stem = flag.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = &flag[stem.len()..];
    if stem.is_empty() || digits.is_empty() || digits.len() > 3 {
        return None;
    }
    Some((stem, digits.parse().ok()?))
}

/// The description with the word `index` (alone, or followed by punctuation) written as `N`.
fn generic_desc(desc: &str, index: u32) -> String {
    let digits = index.to_string();
    desc.split_whitespace()
        .map(|word| match word.strip_prefix(digits.as_str()) {
            Some(rest) if !rest.starts_with(|c: char| c.is_ascii_alphanumeric()) => {
                format!("N{rest}")
            }
            _ => word.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `(flag line, description)` pairs; a run of three or more numbered options that differ only in their number
/// (`--acsi0` to `--acsi7`) becomes one entry.
fn collapse(opts: &[OptLine]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < opts.len() {
        let o = &opts[i];
        if let Some((stem, first)) = split_index(&o.flag) {
            let generic = generic_desc(&o.desc, first);
            let mut last = first;
            let mut j = i + 1;
            while let Some(next) = opts.get(j) {
                let fits = split_index(&next.flag).is_some_and(|(s, n)| {
                    s == stem
                        && n == last + 1
                        && next.hint == o.hint
                        && generic_desc(&next.desc, n) == generic
                });
                if !fits {
                    break;
                }
                last += 1;
                j += 1;
            }
            if j - i >= 3 {
                out.push((
                    format!("--{stem}{first} .. --{stem}{last} <{}>", o.hint),
                    format!("{generic} (N = {first}-{last})"),
                ));
                i = j;
                continue;
            }
        }
        out.push((format!("--{} <{}>", o.flag, o.hint), o.desc.clone()));
        i += 1;
    }
    out
}

/// Machines grouped by identical option lists, in machine-id order of each group's first member.
fn option_groups(profiles: &[MachineProfile]) -> Vec<(Vec<String>, Vec<OptLine>)> {
    let mut sorted: Vec<&MachineProfile> =
        profiles.iter().filter(|p| !p.options.is_empty()).collect();
    sorted.sort_by(|a, b| a.machine_id.cmp(&b.machine_id));
    let mut groups: Vec<(Vec<String>, Vec<OptLine>)> = Vec::new();
    for profile in sorted {
        let lines: Vec<OptLine> = profile
            .options
            .iter()
            .map(|o| OptLine {
                flag: o.flag.clone(),
                hint: crate::machine_option_value_hint(&o.kind),
                desc: o.description.clone(),
            })
            .collect();
        match groups.iter_mut().find(|(_, l)| *l == lines) {
            Some((names, _)) => names.push(profile.machine_id.clone()),
            None => groups.push((vec![profile.machine_id.clone()], lines)),
        }
    }
    groups
}

/// Every machine's options: one block per distinct option list, headed by the machines that share it.
pub(crate) fn machine_options_into(
    out: &mut String,
    profiles: &[MachineProfile],
    width: usize,
    heading: &str,
    indent: &str,
) {
    for (names, lines) in option_groups(profiles) {
        wrap_into(out, &names.join(", "), width, heading, heading);
        for (flag_line, desc) in collapse(&lines) {
            option_entry(out, &flag_line, &desc, width, indent);
        }
    }
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
                machine_options_into(&mut out, &report.profiles, width, "  ", "    ");
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

    fn opt(flag: &str, desc: &str) -> OptLine {
        OptLine {
            flag: flag.into(),
            hint: "file path".into(),
            desc: desc.into(),
        }
    }

    #[test]
    fn numbered_options_that_differ_only_in_their_number_collapse() {
        let opts: Vec<OptLine> = (0..8)
            .map(|n| {
                opt(
                    &format!("acsi{n}"),
                    &format!("disk behind target {n}: 512 MiB"),
                )
            })
            .chain([opt("rom", "the ROM")])
            .collect();
        let c = collapse(&opts);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].0, "--acsi0 .. --acsi7 <file path>");
        assert_eq!(c[0].1, "disk behind target N: 512 MiB (N = 0-7)");
        assert_eq!(c[1].0, "--rom <file path>");
    }

    #[test]
    fn two_numbered_options_or_different_texts_stay_separate() {
        let two = [opt("a1", "x 1"), opt("a2", "x 2")];
        assert_eq!(collapse(&two).len(), 2);
        let differ = [opt("a1", "x 1"), opt("a2", "x 2"), opt("a3", "other 3")];
        assert_eq!(collapse(&differ).len(), 3);
    }
}
