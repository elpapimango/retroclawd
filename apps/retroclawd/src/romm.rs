//! RomM integration: list platforms / games / firmware of the owner's RomM
//! server as text, and read `romm:<id>` / `romm:fw/<id>` sources (a ROM or a
//! firmware file by its RomM id) through a download cache.
//!
//! * Server and token: `ROMM_URL` / `ROMM_TOKEN` in the environment, else
//!   `[romm] url, token` in `retroclawd.toml`. The token is only ever sent as
//!   a bearer header; it is never printed, logged or put into an error.
//! * Endpoints used (RomM 5.x, `/openapi.json`): `GET /api/platforms`,
//!   `GET /api/roms` (`platform_ids`, `search_term`, `limit`, `offset`),
//!   `GET /api/roms/{id}`, `GET /api/roms/{id}/content/{file_name}`,
//!   `GET /api/firmware` (`platform_id`), `GET /api/firmware/{id}`,
//!   `GET /api/firmware/{id}/content/{file_name}`.
//! * Cache: `<cache dir>/romm/{rom,fw}-<id>/<file>`, reused while its size
//!   matches the server's (or when the server cannot be reached). A reuse
//!   refreshes the file's modified time, which `cache clean --days N` reads
//!   as "last used".

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use file_loader::Progress;
use serde_json::Value;

const PAGE: usize = 1000;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Default)]
struct Settings {
    url: Option<String>,
    token: Option<String>,
}

fn configured() -> &'static Mutex<Option<app_config::RommConfig>> {
    static CONFIGURED: Mutex<Option<app_config::RommConfig>> = Mutex::new(None);
    &CONFIGURED
}

/// Hands over `retroclawd.toml`'s `[romm]` section once the config is loaded.
pub(crate) fn configure(config: &app_config::RommConfig) {
    if let Ok(mut slot) = configured().lock() {
        *slot = Some(config.clone());
    }
}

fn settings() -> Settings {
    let from_file = configured()
        .lock()
        .ok()
        .and_then(|c| c.clone())
        .or_else(|| {
            // Not configured yet (a source read before the config was loaded):
            // look at ./retroclawd.toml, ignoring an unreadable one.
            let text = fs::read_to_string("retroclawd.toml").ok()?;
            app_config::parse(&text).ok().map(|c| c.romm)
        })
        .unwrap_or_default();
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    Settings {
        url: env("ROMM_URL").or(from_file.url),
        token: env("ROMM_TOKEN").or(from_file.token),
    }
}

/// Where a `romm:` source points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Item {
    Rom(u64),
    Firmware(u64),
}

/// Parses `romm:<id>` / `romm:fw/<id>` (`None` = not a RomM source, `Some(Err)`
/// = a malformed one).
pub(crate) fn parse_source(source: &str) -> Option<Result<Item, String>> {
    let rest = source.strip_prefix("romm:")?;
    let bad = || format!("\"{source}\" is not a RomM source (use romm:<id> or romm:fw/<id>)");
    Some(match rest.strip_prefix("fw/") {
        Some(id) => id.parse().map(Item::Firmware).map_err(|_| bad()),
        None => rest.parse().map(Item::Rom).map_err(|_| bad()),
    })
}

fn cache_dir_name(item: Item) -> String {
    match item {
        Item::Rom(id) => format!("rom-{id}"),
        Item::Firmware(id) => format!("fw-{id}"),
    }
}

fn item_label(item: Item) -> String {
    match item {
        Item::Rom(id) => format!("romm:{id}"),
        Item::Firmware(id) => format!("romm:fw/{id}"),
    }
}

/// The root of everything this application caches.
pub(crate) fn cache_root() -> Result<PathBuf, String> {
    let env = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    env("RETROCLAWD_CACHE_DIR")
        .or_else(|| env("LOCALAPPDATA").map(|p| p.join("retroclawd").join("cache")))
        .or_else(|| env("XDG_CACHE_HOME").map(|p| p.join("retroclawd")))
        .or_else(|| env("HOME").map(|p| p.join(".cache").join("retroclawd")))
        .ok_or_else(|| "no cache folder: set RETROCLAWD_CACHE_DIR".to_string())
}

fn romm_cache() -> Result<PathBuf, String> {
    Ok(cache_root()?.join("romm"))
}

// ---------------------------------------------------------------- HTTP

struct Client {
    http: reqwest::blocking::Client,
    base: reqwest::Url,
    token: String,
}

impl Client {
    fn new() -> Result<Self, String> {
        let Settings { url, token } = settings();
        let url =
            url.ok_or("RomM is not configured: set ROMM_URL (or [romm] url in retroclawd.toml)")?;
        let token = token
            .ok_or("RomM is not configured: set ROMM_TOKEN (or [romm] token in retroclawd.toml)")?;
        let mut base = reqwest::Url::parse(url.trim())
            .map_err(|e| format!("ROMM_URL is not a valid URL: {e}"))?;
        if base.cannot_be_a_base() || !matches!(base.scheme(), "http" | "https") {
            return Err("ROMM_URL must be an http:// or https:// URL".into());
        }
        // A trailing slash would double up when path segments are appended.
        if let Ok(mut segments) = base.path_segments_mut() {
            segments.pop_if_empty();
        }
        let http = reqwest::blocking::Client::builder()
            .build()
            .map_err(|e| format!("failed to initialize HTTP client: {e}"))?;
        Ok(Self { http, base, token })
    }

    fn url(&self, segments: &[&str], query: &[(&str, String)]) -> reqwest::Url {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.extend(segments);
        }
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        url
    }

    fn send(&self, url: reqwest::Url) -> Result<reqwest::blocking::Response, String> {
        let shown = url.path().to_string();
        let response = self
            .http
            .get(url)
            .bearer_auth(&self.token)
            .send()
            .map_err(|e| format!("RomM request {shown} failed: {}", describe(e)))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        Err(match status.as_u16() {
            401 | 403 => format!(
                "RomM refused {shown} ({status}): check the token (a login page here means an SSO proxy is in the way)"
            ),
            404 => format!("RomM has nothing at {shown} (404)"),
            _ => format!("RomM request {shown} failed: {status}"),
        })
    }

    fn json(&self, segments: &[&str], query: &[(&str, String)]) -> Result<Value, String> {
        let response = self.send(self.url(segments, query))?;
        let shown = response.url().path().to_string();
        let body = response
            .bytes()
            .map_err(|e| format!("RomM request {shown} failed: {}", e.without_url()))?;
        serde_json::from_slice(&body)
            .map_err(|_| format!("RomM answered {shown} with something that is not JSON"))
    }
}

/// An HTTP error with its causes (`connection refused`, `proxy authentication
/// required`, ...) and without the URL.
fn describe(error: reqwest::Error) -> String {
    use std::error::Error;
    let error = error.without_url();
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

// ------------------------------------------------------------- listing

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("-")
        .to_string()
}

fn number(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Column alignment in a [`table`].
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Align {
    Left,
    Right,
}

/// A text table with a header, a rule and columns padded to their widest cell
/// (widths count characters, so accented names stay aligned). Cells are
/// separated by two spaces; the last column is not padded, so lines carry no
/// trailing blanks.
pub(crate) fn table(headers: &[&str], aligns: &[Align], rows: &[Vec<String>]) -> String {
    let widths: Vec<usize> = (0..headers.len())
        .map(|c| {
            rows.iter()
                .map(|r| r[c].chars().count())
                .chain([headers[c].chars().count()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: &[&str]| -> String {
        let mut text = String::new();
        for (c, cell) in cells.iter().enumerate() {
            if c > 0 {
                text.push_str("  ");
            }
            let pad = widths[c] - cell.chars().count();
            match aligns[c] {
                Align::Right => {
                    text.push_str(&" ".repeat(pad));
                    text.push_str(cell);
                }
                Align::Left if c + 1 == cells.len() => text.push_str(cell),
                Align::Left => {
                    text.push_str(cell);
                    text.push_str(&" ".repeat(pad));
                }
            }
        }
        text.push('\n');
        text
    };
    let mut out = line(headers);
    let rule: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    out.push_str(&line(&rule.iter().map(String::as_str).collect::<Vec<_>>()));
    for row in rows {
        out.push_str(&line(&row.iter().map(String::as_str).collect::<Vec<_>>()));
    }
    out
}

const ITEM_HEADERS: [&str; 4] = ["ID", "NAME", "SIZE", "SHA1"];
const ITEM_ALIGNS: [Align; 4] = [Align::Right, Align::Left, Align::Right, Align::Left];

fn item_row(id: u64, name: &str, size: u64, hash: &str) -> Vec<String> {
    vec![
        id.to_string(),
        name.to_string(),
        size.to_string(),
        hash.to_string(),
    ]
}

fn platform_name(p: &Value) -> String {
    p.get("display_name")
        .and_then(Value::as_str)
        .unwrap_or_else(|| p.get("name").and_then(Value::as_str).unwrap_or("-"))
        .to_string()
}

/// Platforms with neither ROMs nor firmware are noise unless `all`.
fn platform_rows(platforms: &[Value], all: bool) -> Vec<Vec<String>> {
    let mut rows: Vec<(u64, Vec<String>)> = platforms
        .iter()
        .filter(|p| all || number(p, "rom_count") > 0 || number(p, "firmware_count") > 0)
        .map(|p| {
            (
                number(p, "id"),
                vec![
                    number(p, "id").to_string(),
                    platform_name(p),
                    text(p, "slug"),
                    number(p, "rom_count").to_string(),
                    number(p, "firmware_count").to_string(),
                ],
            )
        })
        .collect();
    rows.sort_by_key(|(id, _)| *id);
    rows.into_iter().map(|(_, row)| row).collect()
}

fn rom_row(r: &Value) -> Vec<String> {
    let name = r
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .map_or_else(|| text(r, "fs_name"), str::to_string);
    item_row(
        number(r, "id"),
        &name,
        number(r, "fs_size_bytes"),
        &text(r, "sha1_hash"),
    )
}

fn firmware_row(f: &Value) -> Vec<String> {
    item_row(
        number(f, "id"),
        &text(f, "file_name"),
        number(f, "file_size_bytes"),
        &text(f, "sha1_hash"),
    )
}

/// A platform given as an id, a slug or a name.
fn resolve_platform(client: &Client, wanted: &str) -> Result<u64, String> {
    let platforms = client.json(&["api", "platforms"], &[])?;
    let list = platforms
        .as_array()
        .ok_or("unexpected /api/platforms answer")?;
    if let Ok(id) = wanted.parse::<u64>() {
        if list.iter().any(|p| number(p, "id") == id) {
            return Ok(id);
        }
        return Err(format!("RomM has no platform with id {id}"));
    }
    let wanted_lc = wanted.to_ascii_lowercase();
    let matches: Vec<&Value> = list
        .iter()
        .filter(|p| {
            ["slug", "fs_slug", "name", "display_name"]
                .iter()
                .any(|k| text(p, k).to_ascii_lowercase() == wanted_lc)
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok(number(one, "id")),
        [] => Err(format!(
            "no platform \"{wanted}\" on the RomM server (see `retroclawd romm platforms`)"
        )),
        many => Err(format!(
            "\"{wanted}\" matches {} platforms; use an id: {}",
            many.len(),
            many.iter()
                .map(|p| format!("{} ({})", number(p, "id"), text(p, "slug")))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn list_platforms(client: &Client, all: bool, out: &mut impl Write) -> Result<(), String> {
    let platforms = client.json(&["api", "platforms"], &[])?;
    let list = platforms
        .as_array()
        .ok_or("unexpected /api/platforms answer")?;
    let rows = platform_rows(list, all);
    write!(
        out,
        "{}",
        table(
            &["ID", "NAME", "SLUG", "ROMS", "FIRMWARE"],
            &[
                Align::Right,
                Align::Left,
                Align::Left,
                Align::Right,
                Align::Right
            ],
            &rows,
        )
    )
    .map_err(|e| e.to_string())?;
    let hidden = list.len() - rows.len();
    if hidden > 0 {
        writeln!(
            out,
            "\n{hidden} platform(s) with no ROMs and no firmware hidden (--all shows them)"
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn list_games(
    client: &Client,
    platform: &str,
    search: Option<&str>,
    out: &mut impl Write,
) -> Result<(), String> {
    let platform_id = resolve_platform(client, platform)?;
    let mut offset = 0usize;
    let mut rows = Vec::new();
    loop {
        let mut query = vec![
            ("platform_ids", platform_id.to_string()),
            ("limit", PAGE.to_string()),
            ("offset", offset.to_string()),
            ("order_by", "name".to_string()),
        ];
        if let Some(term) = search {
            query.push(("search_term", term.to_string()));
        }
        let page = client.json(&["api", "roms"], &query)?;
        let items = page
            .get("items")
            .and_then(Value::as_array)
            .ok_or("unexpected /api/roms answer")?;
        rows.extend(items.iter().map(rom_row));
        offset += items.len();
        let total = page.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
        if items.is_empty() || offset >= total {
            return write!(out, "{}", table(&ITEM_HEADERS, &ITEM_ALIGNS, &rows))
                .map_err(|e| e.to_string());
        }
    }
}

fn list_firmware(
    client: &Client,
    platform: Option<&str>,
    out: &mut impl Write,
) -> Result<(), String> {
    let query: Vec<(&str, String)> = match platform {
        Some(p) => vec![("platform_id", resolve_platform(client, p)?.to_string())],
        None => vec![],
    };
    let firmware = client.json(&["api", "firmware"], &query)?;
    let rows: Vec<Vec<String>> = firmware
        .as_array()
        .ok_or("unexpected /api/firmware answer")?
        .iter()
        .map(firmware_row)
        .collect();
    write!(out, "{}", table(&ITEM_HEADERS, &ITEM_ALIGNS, &rows)).map_err(|e| e.to_string())
}

/// A firmware file the server holds, as far as matching needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteFirmware {
    pub(crate) id: u64,
    pub(crate) name: String,
    /// Lower-case SHA-1 of the file.
    pub(crate) sha1: String,
}

/// A listed firmware file with a SHA-1 that the server can actually serve (the
/// same dump can be listed twice, one entry's file gone from its library).
fn remote_firmware(f: &Value) -> Option<RemoteFirmware> {
    if f.get("missing_from_fs").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let sha1 = f.get("sha1_hash")?.as_str()?.trim().to_ascii_lowercase();
    (!sha1.is_empty()).then(|| RemoteFirmware {
        id: number(f, "id"),
        name: text(f, "file_name"),
        sha1,
    })
}

/// Every firmware file on the server that has a SHA-1 and is present.
pub(crate) fn firmware_index() -> Result<Vec<RemoteFirmware>, String> {
    let firmware = Client::new()?.json(&["api", "firmware"], &[])?;
    Ok(firmware
        .as_array()
        .ok_or("unexpected /api/firmware answer")?
        .iter()
        .filter_map(remote_firmware)
        .collect())
}

// ------------------------------------------------------------ download

struct Meta {
    file_name: String,
    size: u64,
    label: String,
}

fn meta(client: &Client, item: Item) -> Result<Meta, String> {
    let (value, name_key, size_key) = match item {
        Item::Rom(id) => (
            client.json(&["api", "roms", &id.to_string()], &[])?,
            "fs_name",
            "fs_size_bytes",
        ),
        Item::Firmware(id) => (
            client.json(&["api", "firmware", &id.to_string()], &[])?,
            "file_name",
            "file_size_bytes",
        ),
    };
    if value.get("missing_from_fs").and_then(Value::as_bool) == Some(true) {
        return Err(format!(
            "{} is listed on the RomM server but its file is missing from the server's library",
            item_label(item)
        ));
    }
    let file_name = text(&value, name_key);
    if file_name == "-" || file_name.contains(['/', '\\']) {
        return Err(format!(
            "{} has no usable file name on the server",
            item_label(item)
        ));
    }
    Ok(Meta {
        file_name,
        size: number(&value, size_key),
        label: item_label(item),
    })
}

fn report(source: &str, received: u64, total: Option<u64>) {
    crate::inputs::report_progress(&Progress::Downloading {
        source: source.to_string(),
        received,
        total,
    });
}

fn download(client: &Client, item: Item, meta: &Meta, target: &Path) -> Result<(), String> {
    let url = match item {
        Item::Rom(id) => client.url(
            &["api", "roms", &id.to_string(), "content", &meta.file_name],
            &[],
        ),
        Item::Firmware(id) => client.url(
            &[
                "api",
                "firmware",
                &id.to_string(),
                "content",
                &meta.file_name,
            ],
            &[],
        ),
    };
    let started = Instant::now();
    let mut response = client.send(url)?;
    let total = response.content_length();
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let part = target.with_extension("part");
    let mut file =
        fs::File::create(&part).map_err(|e| format!("cannot write {}: {e}", part.display()))?;
    report(&meta.file_name, 0, total);
    let (mut received, mut last, mut buffer) = (0u64, Instant::now(), vec![0u8; 64 * 1024]);
    let result = (|| -> Result<(), String> {
        loop {
            let n = response
                .read(&mut buffer)
                .map_err(|e| format!("download of {} was interrupted: {e}", meta.label))?;
            if n == 0 {
                return Ok(());
            }
            file.write_all(&buffer[..n])
                .map_err(|e| format!("cannot write {}: {e}", part.display()))?;
            received += n as u64;
            if last.elapsed() >= PROGRESS_INTERVAL {
                last = Instant::now();
                report(&meta.file_name, received, total);
            }
        }
    })();
    drop(file);
    if let Err(e) = result {
        let _ = fs::remove_file(&part);
        return Err(e);
    }
    if meta.size != 0 && received != meta.size {
        let _ = fs::remove_file(&part);
        return Err(format!(
            "{} downloaded {received} bytes, the server lists {}",
            meta.label, meta.size
        ));
    }
    fs::rename(&part, target).map_err(|e| format!("cannot move the download into place: {e}"))?;
    crate::inputs::report_progress(&Progress::Downloaded {
        source: meta.file_name.clone(),
        bytes: received,
        seconds: started.elapsed().as_secs_f64(),
    });
    Ok(())
}

fn touch(path: &Path) {
    if let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// The cached copy of `item`, downloaded first when missing or a different
/// size than the server lists. Falls back to a cached copy when the server
/// cannot be reached.
pub(crate) fn fetch(item: Item) -> Result<PathBuf, String> {
    let dir = romm_cache()?.join(cache_dir_name(item));
    let cached = first_file(&dir);
    let fetched = Client::new().and_then(|client| Ok((meta(&client, item)?, client)));
    match fetched {
        Ok((meta, client)) => {
            let target = dir.join(&meta.file_name);
            let fresh = fs::metadata(&target)
                .is_ok_and(|m| m.is_file() && (meta.size == 0 || m.len() == meta.size));
            if !fresh {
                // Anything else in the folder is an older version of this item.
                let _ = fs::remove_dir_all(&dir);
                download(&client, item, &meta, &target)?;
            }
            touch(&target);
            Ok(target)
        }
        Err(e) => match cached {
            Some(path) => {
                touch(&path);
                Ok(path)
            }
            None => Err(e),
        },
    }
}

fn first_file(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_file() && p.extension().is_none_or(|e| e != "part"))
}

// --------------------------------------------------------------- cache

struct CacheEntry {
    item: Item,
    dir: PathBuf,
    file_name: String,
    size: u64,
    used: SystemTime,
}

fn parse_cache_dir(name: &str) -> Option<Item> {
    if let Some(id) = name.strip_prefix("rom-") {
        id.parse().ok().map(Item::Rom)
    } else {
        name.strip_prefix("fw-")?.parse().ok().map(Item::Firmware)
    }
}

fn cache_entries() -> Result<Vec<CacheEntry>, String> {
    let root = romm_cache()?;
    let Ok(read) = fs::read_dir(&root) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for dir in read.flatten().map(|e| e.path()) {
        let Some(item) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(parse_cache_dir)
        else {
            continue;
        };
        let Some(file) = first_file(&dir) else {
            continue;
        };
        let Ok(m) = fs::metadata(&file) else { continue };
        entries.push(CacheEntry {
            item,
            file_name: file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            dir,
            size: m.len(),
            used: m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    entries.sort_by_key(|e| match e.item {
        Item::Rom(id) => (0, id),
        Item::Firmware(id) => (1, id),
    });
    Ok(entries)
}

fn age_days(used: SystemTime, now: SystemTime) -> u64 {
    now.duration_since(used).map_or(0, |d| d.as_secs() / 86_400)
}

fn cache_list(out: &mut impl Write) -> Result<(), String> {
    let (now, mut total) = (SystemTime::now(), 0u64);
    let mut rows = Vec::new();
    for e in cache_entries()? {
        total += e.size;
        rows.push(vec![
            item_label(e.item),
            e.file_name.clone(),
            e.size.to_string(),
            format!("{}d", age_days(e.used, now)),
        ]);
    }
    write!(
        out,
        "{}",
        table(
            &["SOURCE", "FILE", "SIZE", "UNUSED"],
            &[Align::Left, Align::Left, Align::Right, Align::Right],
            &rows,
        )
    )
    .map_err(|e| e.to_string())?;
    writeln!(out, "\ntotal {total} bytes in {}", romm_cache()?.display()).map_err(|e| e.to_string())
}

fn remove_entry(e: &CacheEntry, out: &mut impl Write) -> Result<(), String> {
    fs::remove_dir_all(&e.dir)
        .map_err(|err| format!("cannot remove {}: {err}", e.dir.display()))?;
    writeln!(out, "removed {} ({})", item_label(e.item), e.file_name).map_err(|e| e.to_string())
}

/// `days = None` removes everything, `Some(n)` what was last used at least
/// `n` days ago.
fn cache_clean(days: Option<u64>, out: &mut impl Write) -> Result<(), String> {
    let now = SystemTime::now();
    let mut freed = 0u64;
    for e in cache_entries()? {
        if days.is_none_or(|d| age_days(e.used, now) >= d) {
            remove_entry(&e, out)?;
            freed += e.size;
        }
    }
    writeln!(out, "freed {freed} bytes").map_err(|e| e.to_string())
}

fn cache_remove(sources: &[String], out: &mut impl Write) -> Result<(), String> {
    if sources.is_empty() {
        return Err("usage: retroclawd cache remove <romm:id | romm:fw/id>...".into());
    }
    let entries = cache_entries()?;
    for source in sources {
        let item = parse_source(source).ok_or_else(|| {
            format!("\"{source}\" is not a cached source (use romm:<id> or romm:fw/<id>)")
        })??;
        let entry = entries
            .iter()
            .find(|e| e.item == item)
            .ok_or_else(|| format!("{source} is not in the cache"))?;
        remove_entry(entry, out)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- verbs

pub(crate) const USAGE: &str = "\
usage:
  retroclawd romm platforms [--all]
  retroclawd romm games <platform> [--search <text>]
  retroclawd romm firmware [<platform>]
  retroclawd romm match [<machine>]
  retroclawd cache list
  retroclawd cache clean (--days <N> | --all)
  retroclawd cache remove <romm:id | romm:fw/id>...

<platform> is a RomM platform id, slug or name. `platforms` hides platforms
with no ROMs and no firmware unless --all is given. Lists are aligned tables
(ID, NAME, SIZE in bytes, SHA1); `romm match` shows, per machine option,
which firmware on the server satisfies it (the `--firmware auto` lookup).
Use `romm:<id>` or `romm:fw/<id>` wherever a file is expected.
ROMM_URL / ROMM_TOKEN (or [romm] in retroclawd.toml) name the server;
RETROCLAWD_CACHE_DIR moves the cache. Behind a proxy: HTTP_PROXY /
HTTPS_PROXY / NO_PROXY are honoured (as is user:password@ in the proxy
URL).";

/// Runs `retroclawd romm ...` / `retroclawd cache ...` (`args` = argv after the
/// program name, first element the verb).
pub(crate) fn run_verb(args: &[String], out: &mut impl Write) -> Result<(), String> {
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    match (args.first().map(String::as_str), rest.as_slice()) {
        (Some("romm"), ["platforms"]) => list_platforms(&Client::new()?, false, out),
        (Some("romm"), ["platforms", "--all"]) => list_platforms(&Client::new()?, true, out),
        (Some("romm"), ["games", platform, tail @ ..]) => {
            let search = match tail {
                [] => None,
                ["--search", term] => Some(*term),
                _ => return Err(USAGE.into()),
            };
            list_games(&Client::new()?, platform, search, out)
        }
        (Some("romm"), ["match"]) => crate::firmware::print_matches(None, out),
        (Some("romm"), ["match", machine]) => crate::firmware::print_matches(Some(machine), out),
        (Some("romm"), ["firmware"]) => list_firmware(&Client::new()?, None, out),
        (Some("romm"), ["firmware", platform]) => {
            list_firmware(&Client::new()?, Some(platform), out)
        }
        (Some("cache"), ["list"]) => cache_list(out),
        (Some("cache"), ["clean", "--all"]) => cache_clean(None, out),
        (Some("cache"), ["clean", "--days", n]) => {
            let days = n
                .parse()
                .map_err(|_| format!("--days needs a number, got \"{n}\""))?;
            cache_clean(Some(days), out)
        }
        (Some("cache"), ["remove", sources @ ..]) => cache_remove(
            &sources.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
            out,
        ),
        _ => Err(USAGE.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_parse() {
        assert_eq!(parse_source("romm:42"), Some(Ok(Item::Rom(42))));
        assert_eq!(parse_source("romm:fw/7"), Some(Ok(Item::Firmware(7))));
        assert!(matches!(parse_source("romm:abc"), Some(Err(_))));
        assert!(matches!(parse_source("romm:fw/"), Some(Err(_))));
        assert_eq!(parse_source("game.zip"), None);
    }

    #[test]
    fn clean_by_age_keeps_recent_files() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: only this test touches the variable.
        unsafe { std::env::set_var("RETROCLAWD_CACHE_DIR", dir.path()) };
        let root = dir.path().join("romm");
        for (name, days) in [("rom-1", 40u64), ("fw-2", 1)] {
            let file = root.join(name).join("x.bin");
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, b"data").unwrap();
            let when = SystemTime::now() - Duration::from_secs(days * 86_400);
            fs::OpenOptions::new()
                .write(true)
                .open(&file)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        let mut out = Vec::new();
        cache_clean(Some(30), &mut out).unwrap();
        assert!(!root.join("rom-1").exists());
        assert!(root.join("fw-2").exists());
        cache_remove(&["romm:fw/2".to_string()], &mut out).unwrap();
        assert!(!root.join("fw-2").exists());
    }
}
