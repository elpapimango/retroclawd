//! `--firmware auto`: fill the file options a machine still lacks (its ROMs)
//! from the firmware the RomM server holds, matched by SHA-1.
//!
//! Which dump satisfies which option is data, not code: every machine family
//! ships a `firmware.toml` (staged next to its libraries as
//! `<family>.firmware.toml`) listing, per machine option, the accepted SHA-1s,
//! preferred first. This module reads those files from the machine plugin
//! folders, asks RomM for its firmware list (`GET /api/firmware`, which carries
//! each file's `sha1_hash`), downloads the winners through the normal RomM
//! cache and checks the downloaded bytes against the hash before they reach a
//! machine. A firmware chosen by id instead is `--rom romm:fw/<id>`, which
//! needs none of this.
//!
//! An entry may also name where the firmware is published (`url`, the
//! `archive_sha1` of that zip and the `entry` inside it): EmuTOS for the Atari
//! ST, free firmware a machine needs without a RomM server. Such a file is
//! downloaded, checked against both hashes and kept in the cache folder
//! (`<cache>/firmware/<sha1>.bin`, verified again on every use), so a later run
//! needs no network. Lookup order per option: the cache, RomM, the URL.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};

use plugin_api::{BinaryBlob, DebugValue, MachineOptionInfo, MachineOptionKind};
use serde::Deserialize;
use sha1::{Digest, Sha1};

use crate::romm::{self, Item, RemoteFirmware};

// ------------------------------------------------------------ the data

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FirmwareFile {
    #[serde(default)]
    rom: Vec<RawEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    machines: Vec<String>,
    option: String,
    label: String,
    #[serde(default)]
    sha1: Vec<String>,
    #[serde(default)]
    parts: Vec<String>,
    #[serde(default)]
    when: BTreeMap<String, String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    archive_sha1: Option<String>,
    #[serde(default)]
    entry: Option<String>,
}

/// Where a firmware is published: a zip at `url` with SHA-1 `archive_sha1`, holding `entry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UrlSource {
    pub(crate) url: String,
    pub(crate) archive_sha1: String,
    pub(crate) entry: String,
}

/// Downloads a URL's body (the HTTP client in production, a fake in tests).
pub(crate) trait Fetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// The largest download or zip entry accepted.
const MAX_DOWNLOAD: u64 = 64 << 20;

struct HttpFetcher;

impl Fetcher for HttpFetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?;
        let response = client.get(url).send().map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        if response.content_length().is_some_and(|n| n > MAX_DOWNLOAD) {
            return Err(format!("larger than {} MiB", MAX_DOWNLOAD >> 20));
        }
        let mut body = Vec::new();
        response
            .take(MAX_DOWNLOAD + 1)
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        if body.len() as u64 > MAX_DOWNLOAD {
            return Err(format!("larger than {} MiB", MAX_DOWNLOAD >> 20));
        }
        Ok(body)
    }
}

/// What an entry accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Wants {
    /// Any one of these single-file dumps, preferred first.
    Any(Vec<String>),
    /// These files, in this order, joined into one image.
    Parts(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) machines: Vec<String>,
    pub(crate) option: String,
    pub(crate) label: String,
    pub(crate) wants: Wants,
    pub(crate) when: BTreeMap<String, String>,
    pub(crate) source: Option<UrlSource>,
}

fn clean_hash(hash: &str, origin: &str, label: &str) -> Result<String, String> {
    let hash = hash.trim().to_ascii_lowercase();
    if hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(hash)
    } else {
        Err(format!(
            "{origin}: \"{label}\": \"{hash}\" is not a SHA-1 (40 hex digits)"
        ))
    }
}

/// Parses one `firmware.toml`; `origin` names it in errors.
pub(crate) fn parse(text: &str, origin: &str) -> Result<Vec<Entry>, String> {
    let file: FirmwareFile = toml::from_str(text).map_err(|e| format!("{origin}: {e}"))?;
    file.rom
        .into_iter()
        .map(|raw| {
            let hashes = |list: Vec<String>| -> Result<Vec<String>, String> {
                list.iter()
                    .map(|h| clean_hash(h, origin, &raw.label))
                    .collect()
            };
            if raw.machines.is_empty() {
                return Err(format!("{origin}: \"{}\" lists no machines", raw.label));
            }
            let wants = match (raw.sha1.is_empty(), raw.parts.is_empty()) {
                (false, true) => Wants::Any(hashes(raw.sha1.clone())?),
                (true, false) => Wants::Parts(hashes(raw.parts.clone())?),
                _ => {
                    return Err(format!(
                        "{origin}: \"{}\" needs exactly one of sha1 = [...] or parts = [...]",
                        raw.label
                    ));
                }
            };
            let source = match (raw.url, raw.archive_sha1, raw.entry) {
                (None, None, None) => None,
                (Some(url), Some(archive_sha1), Some(entry)) => {
                    if !matches!(&wants, Wants::Any(h) if h.len() == 1) {
                        return Err(format!(
                            "{origin}: \"{}\" with a url needs exactly one sha1 (the file inside the zip)",
                            raw.label
                        ));
                    }
                    Some(UrlSource {
                        url,
                        archive_sha1: clean_hash(&archive_sha1, origin, &raw.label)?,
                        entry,
                    })
                }
                _ => {
                    return Err(format!(
                        "{origin}: \"{}\" needs all of url, archive_sha1 and entry, or none",
                        raw.label
                    ));
                }
            };
            Ok(Entry {
                machines: raw.machines,
                option: raw.option,
                label: raw.label,
                wants,
                when: raw.when,
                source,
            })
        })
        .collect()
}

/// The `*.firmware.toml` / `firmware.toml` files in the machine plugin
/// folders, entries in file-name order. Unreadable or invalid files are
/// returned as warnings, not failures.
pub(crate) fn discover() -> (Vec<Entry>, Vec<String>, Vec<String>) {
    let (mut entries, mut warnings, mut searched) = (Vec::new(), Vec::new(), Vec::new());
    let mut files = Vec::new();
    for dir in crate::plugin_bridge::plugin_search_dirs() {
        searched.push(dir.display().to_string());
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for item in read.flatten() {
            let path = item.path();
            let name = item.file_name().to_string_lossy().to_ascii_lowercase();
            if name == "firmware.toml" || name.ends_with(".firmware.toml") {
                files.push(path);
            }
        }
    }
    files.sort_by_key(|p| p.file_name().map(std::ffi::OsStr::to_os_string));
    files.dedup();
    for path in files {
        let origin = path.display().to_string();
        match fs::read_to_string(&path)
            .map_err(|e| format!("{origin}: {e}"))
            .and_then(|text| parse(&text, &origin))
        {
            Ok(found) => entries.extend(found),
            Err(e) => warnings.push(e),
        }
    }
    (entries, warnings, searched)
}

// ------------------------------------------------------------ matching

/// `value` as the text a `when = { key = "..." }` condition is compared with.
fn value_text(value: &DebugValue) -> String {
    match value {
        DebugValue::Bool(v) => v.to_string(),
        DebugValue::U64(v) => v.to_string(),
        DebugValue::I64(v) => v.to_string(),
        DebugValue::Text(v) => v.clone(),
        DebugValue::Bytes(v) => format!("{}B", v.len()),
    }
}

fn applies(
    entry: &Entry,
    machine: &str,
    option: &str,
    values: &HashMap<String, DebugValue>,
) -> bool {
    entry.option == option
        && entry.machines.iter().any(|m| m == machine)
        && entry
            .when
            .iter()
            .all(|(key, wanted)| values.get(key).is_some_and(|v| value_text(v) == *wanted))
}

/// The remote firmware that satisfies `wants`, in the order the files are
/// to be used (one for `Any`, one per part for `Parts`).
fn find<'a>(
    wants: &Wants,
    remote: &'a [RemoteFirmware],
) -> Option<Vec<(&'a RemoteFirmware, String)>> {
    let by_hash = |hash: &str| remote.iter().find(|f| f.sha1 == hash);
    match wants {
        Wants::Any(hashes) => hashes
            .iter()
            .find_map(|h| by_hash(h).map(|f| vec![(f, h.clone())])),
        Wants::Parts(hashes) => hashes
            .iter()
            .map(|h| by_hash(h).map(|f| (f, h.clone())))
            .collect(),
    }
}

/// The entry that matched and the files to fetch, each with the SHA-1 it must have.
#[cfg(test)]
type Match<'a> = (&'a Entry, Vec<(&'a RemoteFirmware, String)>);

/// One option's outcome (the RomM-only matching rules `fill_from` follows; kept for their tests).
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct Planned<'a> {
    pub(crate) required: bool,
    /// The entry that matched and the files to fetch (with the SHA-1 each must have), or the labels tried.
    pub(crate) found: Result<Match<'a>, Vec<String>>,
}

#[cfg(test)]
/// For every `File` option without a value that has an applicable entry, what
/// the server can supply. Options no entry applies to (a drive ROM without
/// `--true-drive`) are left alone.
pub(crate) fn plan<'a>(
    entries: &'a [Entry],
    machine: &str,
    declared: &[MachineOptionInfo],
    values: &HashMap<String, DebugValue>,
    have: &HashSet<String>,
    remote: &'a [RemoteFirmware],
) -> Vec<Planned<'a>> {
    let mut planned = Vec::new();
    for info in declared {
        if info.kind != MachineOptionKind::File || have.contains(&info.key) {
            continue;
        }
        let applicable: Vec<&Entry> = entries
            .iter()
            .filter(|e| applies(e, machine, &info.key, values))
            .collect();
        if applicable.is_empty() {
            continue;
        }
        let found = applicable
            .iter()
            .find_map(|e| find(&e.wants, remote).map(|files| (*e, files)))
            .ok_or_else(|| applicable.iter().map(|e| e.label.clone()).collect());
        planned.push(Planned {
            required: info.required,
            found,
        });
    }
    planned
}

fn sha1_hex(bytes: &[u8]) -> String {
    Sha1::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Downloads (or reuses from the cache) the files of one match, checks each
/// against the hash it was chosen by, and joins them.
fn realize(files: &[(&RemoteFirmware, String)]) -> Result<Vec<u8>, String> {
    let mut image = Vec::new();
    for (firmware, expected) in files {
        let path = romm::fetch(Item::Firmware(firmware.id))?;
        let bytes = fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let actual = sha1_hex(&bytes);
        if actual != *expected {
            return Err(format!(
                "romm:fw/{} ({}) has SHA-1 {actual} but the server listed {expected}; \
                 `retroclawd cache remove romm:fw/{}` drops the cached copy",
                firmware.id, firmware.name, firmware.id
            ));
        }
        image.extend_from_slice(&bytes);
    }
    Ok(image)
}

/// Whether `entry` names a file option the machine does not declare -- the
/// toml and the plugin disagree on a key, so the entry could never apply.
fn names_undeclared_option(entry: &Entry, declared: &[MachineOptionInfo]) -> bool {
    !declared
        .iter()
        .any(|i| i.kind == MachineOptionKind::File && i.key == entry.option)
}

/// The declared file option keys, for messages.
fn declared_file_keys(declared: &[MachineOptionInfo]) -> String {
    let keys: Vec<&str> = declared
        .iter()
        .filter(|i| i.kind == MachineOptionKind::File)
        .map(|i| i.key.as_str())
        .collect();
    keys.join(", ")
}

/// The verified cached copy of the file with SHA-1 `sha1`. A copy that does not
/// hash to its name is removed (and fetched again by the caller).
fn cached(cache: &std::path::Path, sha1: &str) -> Option<Vec<u8>> {
    let path = cache.join(format!("{sha1}.bin"));
    let bytes = fs::read(&path).ok()?;
    if sha1_hex(&bytes) == sha1 {
        return Some(bytes);
    }
    eprintln!(
        "firmware: the cached {} does not hash to its name; removed, fetching again",
        path.display()
    );
    let _ = fs::remove_file(&path);
    None
}

/// Stores `bytes` as `<sha1>.bin` (written aside, then renamed into place).
fn store(cache: &std::path::Path, sha1: &str, bytes: &[u8]) {
    let path = cache.join(format!("{sha1}.bin"));
    let partial = cache.join(format!("{sha1}.part"));
    let result = fs::create_dir_all(cache)
        .and_then(|()| fs::write(&partial, bytes))
        .and_then(|()| fs::rename(&partial, &path));
    if let Err(e) = result {
        eprintln!("firmware: cannot cache {}: {e}", path.display());
    }
}

/// Downloads `source`, checks the zip's SHA-1, takes `source.entry` out of it and
/// checks that against `want`.
fn fetch_url(source: &UrlSource, want: &str, fetcher: &dyn Fetcher) -> Result<Vec<u8>, String> {
    let url = &source.url;
    let archive = fetcher
        .get(url)
        .map_err(|e| format!("{url}: {e} (wanted SHA-1 {want})"))?;
    let actual = sha1_hex(&archive);
    if actual != source.archive_sha1 {
        return Err(format!(
            "{url} has SHA-1 {actual} but firmware.toml pins {}",
            source.archive_sha1
        ));
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .map_err(|e| format!("{url}: not a zip: {e}"))?;
    let file = zip
        .by_name(&source.entry)
        .map_err(|e| format!("{url}: no {}: {e}", source.entry))?;
    if file.size() > MAX_DOWNLOAD {
        return Err(format!(
            "{url}#{}: larger than {} MiB",
            source.entry,
            MAX_DOWNLOAD >> 20
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_DOWNLOAD)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{url}#{}: {e}", source.entry))?;
    let actual = sha1_hex(&bytes);
    if actual != want {
        return Err(format!(
            "{url}#{} has SHA-1 {actual} but firmware.toml pins {want}",
            source.entry
        ));
    }
    Ok(bytes)
}

/// The image `entry` yields from the cache, RomM or its URL, in that order.
fn obtain(
    entry: &Entry,
    remote: &[RemoteFirmware],
    fetcher: &dyn Fetcher,
    cache: &std::path::Path,
) -> Result<Option<Vec<u8>>, String> {
    if let Wants::Any(hashes) = &entry.wants
        && let Some(bytes) = hashes.iter().find_map(|h| cached(cache, h))
    {
        return Ok(Some(bytes));
    }
    if let Some(files) = find(&entry.wants, remote) {
        let bytes = realize(&files)?;
        if let [(_, hash)] = files.as_slice() {
            store(cache, hash, &bytes);
        }
        return Ok(Some(bytes));
    }
    match (&entry.source, &entry.wants) {
        (Some(source), Wants::Any(hashes)) => {
            let bytes = fetch_url(source, &hashes[0], fetcher)?;
            store(cache, &hashes[0], &bytes);
            Ok(Some(bytes))
        }
        _ => Ok(None),
    }
}

/// `--firmware auto` with its inputs given: fills every `have`-less file option
/// some entry applies to. `region` is also a `when` key (`when = { region = "ntsc" }`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_from(
    entries: &[Entry],
    machine: &str,
    region: &str,
    declared: &[MachineOptionInfo],
    values: &HashMap<String, DebugValue>,
    have: &HashSet<String>,
    remote: &[RemoteFirmware],
    fetcher: &dyn Fetcher,
    cache: &std::path::Path,
) -> Result<Vec<BinaryBlob>, String> {
    let mut values = values.clone();
    values
        .entry("region".to_string())
        .or_insert_with(|| DebugValue::Text(region.to_string()));
    let mut blobs = Vec::new();
    let mut missing = Vec::new();
    for info in declared {
        if info.kind != MachineOptionKind::File || have.contains(&info.key) {
            continue;
        }
        let applicable: Vec<&Entry> = entries
            .iter()
            .filter(|e| applies(e, machine, &info.key, &values))
            .collect();
        if applicable.is_empty() {
            continue;
        }
        let mut problems = Vec::new();
        let mut found = None;
        for entry in &applicable {
            match obtain(entry, remote, fetcher, cache) {
                Ok(Some(bytes)) => {
                    found = Some((entry.label.clone(), bytes));
                    break;
                }
                Ok(None) => {}
                Err(e) => problems.push(format!("{}: {e}", entry.label)),
            }
        }
        match found {
            Some((label, bytes)) => {
                eprintln!("firmware: {} <- {label}", info.key);
                blobs.push(BinaryBlob {
                    key: info.key.clone(),
                    bytes,
                });
            }
            None => {
                let wanted: Vec<String> = applicable.iter().map(|e| e.label.clone()).collect();
                let detail = if problems.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", problems.join("; "))
                };
                let text = format!("{} (wanted: {}){detail}", info.key, wanted.join("; "));
                if info.required {
                    missing.push(text);
                } else {
                    eprintln!("firmware: nothing found for {text}");
                }
            }
        }
    }
    if missing.is_empty() {
        Ok(blobs)
    } else {
        Err(format!(
            "--firmware auto: no firmware with a matching SHA-1 for: {}\n\
             see `retroclawd romm match {machine}`; or name a file with --<option> romm:fw/<id>",
            missing.join(", ")
        ))
    }
}

/// `--firmware auto`: fills `have`-less file options from the cache, RomM or the
/// published URL. Returns the blobs to add to the machine's ROMs; required options
/// nothing satisfied are one error, worded for the user.
pub(crate) fn auto_fill(
    machine: &str,
    region: &str,
    declared: &[MachineOptionInfo],
    values: &HashMap<String, DebugValue>,
    have: &HashSet<String>,
) -> Result<Vec<BinaryBlob>, String> {
    let (entries, warnings, searched) = discover();
    for warning in warnings {
        eprintln!("firmware: {warning}");
    }
    let mine: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.machines.iter().any(|m| m == machine))
        .collect();
    if mine.is_empty() {
        return Err(format!(
            "--firmware auto: no firmware.toml next to the machine libraries lists \"{machine}\" \
             (searched {})",
            searched.join(", ")
        ));
    }
    for entry in &mine {
        if names_undeclared_option(entry, declared) {
            eprintln!(
                "firmware: \"{}\" names option \"{}\", which {machine} does not declare (file options: {}); fix the firmware.toml",
                entry.label,
                entry.option,
                declared_file_keys(declared)
            );
        }
    }
    let remote = match romm::firmware_index() {
        Ok(remote) => remote,
        Err(e) if mine.iter().any(|e| e.source.is_some()) => {
            eprintln!("firmware: no RomM server ({e}); using the cache and the published URLs");
            Vec::new()
        }
        Err(e) => return Err(e),
    };
    let cache = romm::cache_root()?.join("firmware");
    fill_from(
        &entries,
        machine,
        region,
        declared,
        values,
        have,
        &remote,
        &HttpFetcher,
        &cache,
    )
}

// ----------------------------------------------------------- the verb

/// `retroclawd romm match [<machine>]`: each firmware.toml entry and what the
/// server has for it.
pub(crate) fn print_matches(machine: Option<&str>, out: &mut impl Write) -> Result<(), String> {
    let (entries, warnings, searched) = discover();
    for warning in warnings {
        eprintln!("firmware: {warning}");
    }
    if entries.is_empty() {
        return Err(format!(
            "no firmware.toml found next to the machine libraries (searched {})",
            searched.join(", ")
        ));
    }
    let remote = romm::firmware_index()?;
    let profiles = crate::plugin_bridge::startup_report()
        .map(|r| r.profiles)
        .unwrap_or_default();
    let mut rows = Vec::new();
    for entry in &entries {
        for id in &entry.machines {
            if machine.is_some_and(|m| m != id) {
                continue;
            }
            let wanted = if entry.when.is_empty() {
                entry.label.clone()
            } else {
                let cond: Vec<String> =
                    entry.when.iter().map(|(k, v)| format!("{k}={v}")).collect();
                format!("{} [{}]", entry.label, cond.join(", "))
            };
            let declared = profiles
                .iter()
                .find(|p| p.machine_id == *id)
                .map(|p| p.options.as_slice());
            let status = match find(&entry.wants, &remote) {
                _ if declared.is_some_and(|d| names_undeclared_option(entry, d)) => format!(
                    "OPTION NOT DECLARED by the plugin (file options: {})",
                    declared_file_keys(declared.unwrap_or_default())
                ),
                Some(files) => files
                    .iter()
                    .map(|(f, _)| format!("{} {}", f.id, f.name))
                    .collect::<Vec<_>>()
                    .join(" + "),
                None => "not on the server".to_string(),
            };
            rows.push(vec![id.clone(), entry.option.clone(), wanted, status]);
        }
    }
    if rows.is_empty() {
        return Err(format!(
            "no firmware.toml entry for machine \"{}\"",
            machine.unwrap_or("")
        ));
    }
    write!(
        out,
        "{}",
        romm::table(
            &["MACHINE", "OPTION", "WANTED", "ROMM FIRMWARE"],
            &[romm::Align::Left; 4],
            &rows
        )
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const H1: &str = "5ea7c2b824672e914525d1d5c419d71b84a426a2";
    const H2: &str = "16375d42ea109b47edded7a16028de7fdb3013a1";

    fn remote(id: u64, name: &str, sha1: &str) -> RemoteFirmware {
        RemoteFirmware {
            id,
            name: name.to_string(),
            sha1: sha1.to_string(),
        }
    }

    fn file_option(key: &str, required: bool) -> MachineOptionInfo {
        MachineOptionInfo {
            key: key.to_string(),
            flag: key.replace('_', "-"),
            short: None,
            kind: MachineOptionKind::File,
            default: None,
            required,
            description: String::new(),
        }
    }

    fn entry_for(option: &str) -> Entry {
        Entry {
            machines: vec!["c64".into()],
            option: option.into(),
            label: "C64 KERNAL".into(),
            wants: Wants::Any(vec![H1.into()]),
            when: BTreeMap::new(),
            source: None,
        }
    }

    #[test]
    fn an_entry_must_name_a_declared_file_option() {
        // The commodore plugins declare `kernal`; a manifest saying `rom_kernal`
        // never matched anything.
        let declared = [file_option("kernal", true), file_option("rom_1541", false)];
        assert!(!names_undeclared_option(&entry_for("kernal"), &declared));
        assert!(names_undeclared_option(&entry_for("rom_kernal"), &declared));
        assert_eq!(declared_file_keys(&declared), "kernal, rom_1541");
        // A non-file option of that name does not count.
        let mut text = file_option("kernal", false);
        text.kind = MachineOptionKind::Text;
        assert!(names_undeclared_option(&entry_for("kernal"), &[text]));
    }

    const TOML: &str = r#"
[[rom]]
machines = ["zx-spectrum"]
option = "rom"
label = "48K ROM"
sha1 = ["5EA7C2B824672E914525D1D5C419D71B84A426A2", "16375d42ea109b47edded7a16028de7fdb3013a1"]

[[rom]]
machines = ["c64", "c128"]
option = "rom_1541"
label = "1581 DOS"
when = { true_drive = "true", drive_type = "1581" }
sha1 = ["01228eae6f066bd9b7b2b6a7fa3f667e41dad393"]
"#;

    #[test]
    fn a_firmware_file_parses_and_normalizes_hashes() {
        let entries = parse(TOML, "test").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].wants, Wants::Any(vec![H1.into(), H2.into()]));
        assert_eq!(entries[1].when["drive_type"], "1581");
    }

    #[test]
    fn bad_files_are_refused_with_the_file_named() {
        let err = parse(
            "[[rom]]\nmachines=[\"x\"]\noption=\"rom\"\nlabel=\"L\"\nsha1=[\"abc\"]",
            "f.toml",
        )
        .unwrap_err();
        assert!(
            err.contains("f.toml") && err.contains("not a SHA-1"),
            "{err}"
        );
        let err = parse(
            "[[rom]]\nmachines=[\"x\"]\noption=\"rom\"\nlabel=\"L\"",
            "f.toml",
        )
        .unwrap_err();
        assert!(err.contains("exactly one of"), "{err}");
        assert!(parse("[[rom]]\nbogus=1", "f.toml").is_err());
    }

    #[test]
    fn the_preferred_hash_wins_over_a_later_one() {
        let entries = parse(TOML, "t").unwrap();
        let both = [remote(9, "other.rom", H2), remote(4, "48.rom", H1)];
        let declared = [file_option("rom", true)];
        let planned = plan(
            &entries,
            "zx-spectrum",
            &declared,
            &HashMap::new(),
            &HashSet::new(),
            &both,
        );
        let (entry, files) = planned[0].found.as_ref().unwrap();
        assert_eq!(entry.label, "48K ROM");
        assert_eq!(files[0].0.id, 4);
    }

    #[test]
    fn options_with_a_value_or_without_an_applicable_entry_are_skipped() {
        let entries = parse(TOML, "t").unwrap();
        let declared = [file_option("rom", true), file_option("rom_1541", false)];
        let have: HashSet<String> = ["rom".to_string()].into();
        // `rom` is given; `rom_1541` has no applicable entry without --true-drive.
        assert!(
            plan(
                &entries,
                "zx-spectrum",
                &declared,
                &HashMap::new(),
                &have,
                &[]
            )
            .is_empty()
        );
        // Another machine's entries never apply.
        assert!(
            plan(
                &entries,
                "cpc464",
                &declared,
                &HashMap::new(),
                &HashSet::new(),
                &[]
            )
            .is_empty()
        );
    }

    #[test]
    fn when_conditions_select_the_drive_rom() {
        let entries = parse(TOML, "t").unwrap();
        let declared = [file_option("rom_1541", false)];
        let dos = remote(3, "VC1581", "01228eae6f066bd9b7b2b6a7fa3f667e41dad393");
        let mut values = HashMap::new();
        values.insert("true_drive".to_string(), DebugValue::Bool(true));
        values.insert("drive_type".to_string(), DebugValue::Text("1571".into()));
        let none = plan(
            &entries,
            "c64",
            &declared,
            &values,
            &HashSet::new(),
            std::slice::from_ref(&dos),
        );
        assert!(none.is_empty(), "1571 selected, only a 1581 entry exists");
        values.insert("drive_type".to_string(), DebugValue::Text("1581".into()));
        let some = plan(
            &entries,
            "c64",
            &declared,
            &values,
            &HashSet::new(),
            std::slice::from_ref(&dos),
        );
        assert_eq!(some[0].found.as_ref().unwrap().1[0].0.id, 3);
    }

    #[test]
    fn parts_need_every_file() {
        let wants = Wants::Parts(vec![H1.into(), H2.into()]);
        let only_first = [remote(1, "lo", H1)];
        assert!(find(&wants, &only_first).is_none());
        let both = [remote(2, "hi", H2), remote(1, "lo", H1)];
        let ids: Vec<u64> = find(&wants, &both)
            .unwrap()
            .iter()
            .map(|(f, _)| f.id)
            .collect();
        assert_eq!(ids, [1, 2], "in the order the entry lists them");
    }

    #[test]
    fn an_unmatched_required_option_names_what_was_wanted() {
        let entries = parse(TOML, "t").unwrap();
        let declared = [file_option("rom", true)];
        let planned = plan(
            &entries,
            "zx-spectrum",
            &declared,
            &HashMap::new(),
            &HashSet::new(),
            &[],
        );
        assert_eq!(
            planned[0].found.as_ref().unwrap_err(),
            &["48K ROM".to_string()]
        );
        assert!(planned[0].required);
    }

    #[test]
    fn sha1_matches_a_known_digest() {
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    // ------------------------------------------------- URL sources

    /// A zip holding `name` with `bytes`, stored.
    fn zip_with(name: &str, bytes: &[u8]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file(name, opts).unwrap();
        zip.write_all(bytes).unwrap();
        zip.finish().unwrap();
        out.into_inner()
    }

    struct FakeFetcher {
        body: Result<Vec<u8>, String>,
        calls: std::cell::Cell<u32>,
    }

    impl Fetcher for FakeFetcher {
        fn get(&self, _url: &str) -> Result<Vec<u8>, String> {
            self.calls.set(self.calls.get() + 1);
            self.body.clone()
        }
    }

    const ROM: &[u8] = b"pretend this is a TOS image";

    fn url_entry(region: &str, archive_sha1: &str, rom_sha1: &str) -> Entry {
        Entry {
            machines: vec!["atari-st".into()],
            option: "rom".into(),
            label: format!("EmuTOS ({region})"),
            wants: Wants::Any(vec![rom_sha1.into()]),
            when: BTreeMap::from([("region".to_string(), region.to_string())]),
            source: Some(UrlSource {
                url: format!("https://example.invalid/emutos-{region}.zip"),
                archive_sha1: archive_sha1.into(),
                entry: format!("emutos/{region}.img"),
            }),
        }
    }

    fn fill(
        entries: &[Entry],
        region: &str,
        fetcher: &dyn Fetcher,
        cache: &std::path::Path,
    ) -> Result<Vec<BinaryBlob>, String> {
        fill_from(
            entries,
            "atari-st",
            region,
            &[file_option("rom", true)],
            &HashMap::new(),
            &HashSet::new(),
            &[],
            fetcher,
            cache,
        )
    }

    #[test]
    fn a_url_entry_parses_and_needs_all_three_fields() {
        let good = "[[rom]]
machines=[\"atari-st\"]
option=\"rom\"
label=\"E\"
sha1=[\"904ea9dc09ff8286f6e0272c12255dfe85beb17a\"]
            url=\"https://x/e.zip\"
archive_sha1=\"6f146a126065f0671a1df3d9a273c01f962e9d06\"
entry=\"e/uk.img\"
";
        let parsed = parse(good, "t").unwrap();
        assert_eq!(parsed[0].source.as_ref().unwrap().entry, "e/uk.img");
        let partial = "[[rom]]
machines=[\"atari-st\"]
option=\"rom\"
label=\"E\"
sha1=[\"904ea9dc09ff8286f6e0272c12255dfe85beb17a\"]
url=\"https://x/e.zip\"
";
        let err = parse(partial, "t").unwrap_err();
        assert!(err.contains("url, archive_sha1 and entry"), "{err}");
    }

    #[test]
    fn a_url_source_downloads_verifies_and_caches() {
        let cache = tempfile::tempdir().unwrap();
        let zip = zip_with("emutos/pal.img", ROM);
        let entries = [url_entry("pal", &sha1_hex(&zip), &sha1_hex(ROM))];
        let fetcher = FakeFetcher {
            body: Ok(zip),
            calls: Default::default(),
        };
        let blobs = fill(&entries, "pal", &fetcher, cache.path()).unwrap();
        assert_eq!(blobs[0].bytes, ROM);
        assert_eq!(fetcher.calls.get(), 1);
        let again = fill(&entries, "pal", &fetcher, cache.path()).unwrap();
        assert_eq!(again[0].bytes, ROM);
        assert_eq!(fetcher.calls.get(), 1, "the second run uses the cache");
    }

    #[test]
    fn a_wrong_archive_hash_is_refused() {
        let cache = tempfile::tempdir().unwrap();
        let zip = zip_with("emutos/pal.img", ROM);
        let entries = [url_entry("pal", H1, &sha1_hex(ROM))];
        let fetcher = FakeFetcher {
            body: Ok(zip),
            calls: Default::default(),
        };
        let err = fill(&entries, "pal", &fetcher, cache.path()).unwrap_err();
        assert!(err.contains("SHA-1") && err.contains(H1), "{err}");
    }

    #[test]
    fn a_wrong_entry_hash_is_refused() {
        let cache = tempfile::tempdir().unwrap();
        let zip = zip_with("emutos/pal.img", ROM);
        let entries = [url_entry("pal", &sha1_hex(&zip), H2)];
        let fetcher = FakeFetcher {
            body: Ok(zip),
            calls: Default::default(),
        };
        let err = fill(&entries, "pal", &fetcher, cache.path()).unwrap_err();
        assert!(err.contains(H2), "{err}");
    }

    #[test]
    fn a_tampered_cache_file_is_refused_and_refetched() {
        let cache = tempfile::tempdir().unwrap();
        let zip = zip_with("emutos/pal.img", ROM);
        let entries = [url_entry("pal", &sha1_hex(&zip), &sha1_hex(ROM))];
        let fetcher = FakeFetcher {
            body: Ok(zip),
            calls: Default::default(),
        };
        fill(&entries, "pal", &fetcher, cache.path()).unwrap();
        let cached = cache.path().join(format!("{}.bin", sha1_hex(ROM)));
        std::fs::write(&cached, b"tampered").unwrap();
        let blobs = fill(&entries, "pal", &fetcher, cache.path()).unwrap();
        assert_eq!(blobs[0].bytes, ROM);
        assert_eq!(fetcher.calls.get(), 2);
        assert_eq!(std::fs::read(&cached).unwrap(), ROM);
    }

    #[test]
    fn offline_with_no_cache_names_the_url_and_hash() {
        let cache = tempfile::tempdir().unwrap();
        let entries = [url_entry("pal", H1, H2)];
        let fetcher = FakeFetcher {
            body: Err("connection refused".into()),
            calls: Default::default(),
        };
        let err = fill(&entries, "pal", &fetcher, cache.path()).unwrap_err();
        assert!(
            err.contains("https://example.invalid/emutos-pal.zip"),
            "{err}"
        );
        assert!(
            err.contains(H2) && err.contains("connection refused"),
            "{err}"
        );
    }

    #[test]
    fn region_selects_the_ntsc_image() {
        let cache = tempfile::tempdir().unwrap();
        let pal_zip = zip_with("emutos/pal.img", b"pal");
        let ntsc_zip = zip_with("emutos/ntsc.img", b"ntsc");
        let entries = [
            url_entry("pal", &sha1_hex(&pal_zip), &sha1_hex(b"pal")),
            url_entry("ntsc", &sha1_hex(&ntsc_zip), &sha1_hex(b"ntsc")),
        ];
        let fetcher = FakeFetcher {
            body: Ok(ntsc_zip),
            calls: Default::default(),
        };
        let blobs = fill(&entries, "ntsc", &fetcher, cache.path()).unwrap();
        assert_eq!(blobs[0].bytes, b"ntsc");
    }
}
