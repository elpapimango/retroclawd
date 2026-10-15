//! Reads what the user hands the emulator: a local file, an `http(s)://` URL,
//! or a `.zip` (local or remote) holding the file. A zip is unpacked into one
//! per-loader temporary folder under the system temp directory (removed when
//! the [`FileLoader`] is dropped or [`FileLoader::cleanup`] is called; a crash
//! can leave one behind, which [`remove_stale_temp_dirs`] sweeps later).
//!
//! `archive.zip#entry` picks one entry of a zip that holds several files (the
//! entry name is matched exactly, then ignoring case, then by file name alone;
//! `%20`-style escapes are decoded). Without `#entry` the zip must hold exactly
//! one file.

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// Prefix of the folder created under the system temp directory.
pub const TEMP_PREFIX: &str = "retroclawd-unzip-";

/// Minimum time between two [`Progress::Downloading`] reports.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, Default)]
pub struct LoaderConfig {
    pub insecure_skip_tls_verify: bool,
}

#[derive(Debug, Clone)]
pub struct LoadedBinary {
    pub bytes: Vec<u8>,
    /// Effective file name used for extension sniffing after archive extraction.
    pub name: String,
    /// Where a zip entry was unpacked (inside the loader's temporary folder);
    /// `None` for a file that was not inside an archive.
    pub extracted_path: Option<PathBuf>,
}

impl LoadedBinary {
    #[must_use]
    pub fn extension_lowercase(&self) -> Option<String> {
        Path::new(&self.name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
    }
}

/// What a load is doing, for a progress line or toast.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// Bytes received so far of a download (`total` when the server says).
    Downloading {
        source: String,
        received: u64,
        total: Option<u64>,
    },
    /// A download finished.
    Downloaded {
        source: String,
        bytes: u64,
        seconds: f64,
    },
    /// A zip entry is being unpacked.
    Extracting { entry: String },
}

pub type ProgressFn = Arc<dyn Fn(&Progress) + Send + Sync>;

pub struct FileLoader {
    client: reqwest::blocking::Client,
    root: Option<tempfile::TempDir>,
    extracted: u32,
    progress: Option<ProgressFn>,
}

impl FileLoader {
    pub fn new(config: LoaderConfig) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .danger_accept_invalid_certs(config.insecure_skip_tls_verify)
            .build()
            .map_err(|e| format!("failed to initialize HTTP loader: {e}"))?;
        Ok(Self {
            client,
            root: None,
            extracted: 0,
            progress: None,
        })
    }

    /// Reports downloads and unpacking to `progress`.
    pub fn set_progress(&mut self, progress: Option<ProgressFn>) {
        self.progress = progress;
    }

    /// The temporary folder zips are unpacked into (`None` until one was).
    #[must_use]
    pub fn temp_root(&self) -> Option<&Path> {
        self.root.as_ref().map(tempfile::TempDir::path)
    }

    /// Deletes everything unpacked so far.
    pub fn cleanup(&mut self) {
        self.root = None;
    }

    pub fn load_text(&mut self, source: &str) -> Result<String, String> {
        let loaded = self.load_binary(source)?;
        String::from_utf8(loaded.bytes).map_err(|e| {
            format!("{source} is not valid UTF-8 text (needed here for config parsing): {e}")
        })
    }

    pub fn load_binary(&mut self, source: &str) -> Result<LoadedBinary, String> {
        let (base, entry) = split_entry_selector(source);
        let mut bytes = if is_remote(base) {
            self.load_remote(base)?
        } else {
            self.load_local(base)?
        };
        let mut name = effective_name(base);
        let mut extracted_path = None;

        if entry.is_some() || should_unpack_zip(base, &bytes) {
            let unpacked = self.unpack_zip_entry(base, &bytes, entry.as_deref())?;
            name = unpacked.name;
            bytes = unpacked.bytes;
            extracted_path = unpacked.extracted_path;
        }

        Ok(LoadedBinary {
            bytes,
            name,
            extracted_path,
        })
    }

    fn report(&self, progress: &Progress) {
        if let Some(sink) = &self.progress {
            sink(progress);
        }
    }

    fn load_local(&self, source: &str) -> Result<Vec<u8>, String> {
        fs::read(source).map_err(|e| format!("failed to read {source}: {e}"))
    }

    fn load_remote(&self, source: &str) -> Result<Vec<u8>, String> {
        let started = Instant::now();
        self.report(&Progress::Downloading {
            source: source.to_string(),
            received: 0,
            total: None,
        });
        let mut response = self
            .client
            .get(source)
            .send()
            .map_err(|e| format!("failed HTTP GET {source}: {e}"))?
            .error_for_status()
            .map_err(|e| format!("HTTP error for {source}: {e}"))?;
        let total = response.content_length();
        let mut bytes = Vec::with_capacity(total.map_or(0, |t| usize::try_from(t).unwrap_or(0)));
        let mut chunk = [0u8; 64 * 1024];
        let mut last_report = Instant::now();
        loop {
            let n = response
                .read(&mut chunk)
                .map_err(|e| format!("failed reading HTTP body for {source}: {e}"))?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
            if last_report.elapsed() >= PROGRESS_INTERVAL {
                last_report = Instant::now();
                self.report(&Progress::Downloading {
                    source: source.to_string(),
                    received: bytes.len() as u64,
                    total,
                });
            }
        }
        self.report(&Progress::Downloaded {
            source: source.to_string(),
            bytes: bytes.len() as u64,
            seconds: started.elapsed().as_secs_f64(),
        });
        Ok(bytes)
    }

    fn unpack_zip_entry(
        &mut self,
        source: &str,
        bytes: &[u8],
        wanted: Option<&str>,
    ) -> Result<LoadedBinary, String> {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| format!("{source} looks like zip but could not be parsed: {e}"))?;

        let mut files: Vec<(usize, String)> = Vec::new();
        for idx in 0..archive.len() {
            let file = archive
                .by_index(idx)
                .map_err(|e| format!("failed to inspect zip entry #{idx}: {e}"))?;
            if !file.is_dir() {
                files.push((idx, file.name().to_string()));
            }
        }
        let names = || {
            files
                .iter()
                .map(|(_, n)| n.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };

        let index = match (wanted, files.len()) {
            (_, 0) => {
                return Err(format!(
                    "{source} is a zip archive with no regular files to load"
                ));
            }
            (Some(wanted), _) => pick_entry(&files, wanted).ok_or_else(|| {
                format!(
                    "{source} has no entry named {wanted:?}. Entries: {}",
                    names()
                )
            })?,
            (None, 1) => files[0].0,
            (None, _) => {
                return Err(format!(
                    "{source} is a zip archive with multiple files; expected exactly one \
                     (pick one with {source}#<entry>). Entries: {}",
                    names()
                ));
            }
        };

        let mut zip_file = archive
            .by_index(index)
            .map_err(|e| format!("failed to read zip entry #{index}: {e}"))?;
        let enclosed = zip_file
            .enclosed_name()
            .ok_or_else(|| format!("zip entry {:?} has an unsafe path", zip_file.name()))?;
        self.report(&Progress::Extracting {
            entry: enclosed.to_string_lossy().into_owned(),
        });

        if self.root.is_none() {
            self.root = Some(
                tempfile::Builder::new()
                    .prefix(TEMP_PREFIX)
                    .tempdir()
                    .map_err(|e| {
                        format!("failed to allocate temporary directory for zip extraction: {e}")
                    })?,
            );
        }
        // One numbered folder per extraction so two zips holding the same
        // entry name cannot collide.
        self.extracted += 1;
        let extracted_path = self
            .root
            .as_ref()
            .expect("created above")
            .path()
            .join(self.extracted.to_string())
            .join(&enclosed);
        if let Some(parent) = extracted_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "failed creating zip extraction folder {}: {e}",
                    parent.display()
                )
            })?;
        }
        let mut out = fs::File::create(&extracted_path).map_err(|e| {
            format!(
                "failed creating extracted file {}: {e}",
                extracted_path.display()
            )
        })?;
        std::io::copy(&mut zip_file, &mut out)
            .map_err(|e| format!("failed extracting zip entry {:?}: {e}", zip_file.name()))?;
        drop(out);
        let extracted_bytes = fs::read(&extracted_path).map_err(|e| {
            format!(
                "failed reading extracted file {}: {e}",
                extracted_path.display()
            )
        })?;

        Ok(LoadedBinary {
            bytes: extracted_bytes,
            name: enclosed.to_string_lossy().to_string(),
            extracted_path: Some(extracted_path),
        })
    }
}

/// Removes `retroclawd-unzip-*` folders in the system temp directory that were
/// last modified more than `max_age` ago -- what a crashed or killed run left
/// behind. Returns how many were removed. Best effort: errors are ignored.
pub fn remove_stale_temp_dirs(max_age: Duration) -> usize {
    remove_stale_in(&std::env::temp_dir(), max_age)
}

fn remove_stale_in(dir: &Path, max_age: Duration) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let is_ours = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(TEMP_PREFIX));
        if !is_ours {
            continue;
        }
        let old_enough = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= max_age);
        if old_enough && fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Splits `archive.zip#entry` into (`archive.zip`, `entry`). A `#` only
/// selects an entry when what precedes it names a `.zip`, so file names that
/// merely contain `#` are untouched.
#[must_use]
pub fn split_entry_selector(source: &str) -> (&str, Option<String>) {
    if let Some((base, fragment)) = source.rsplit_once('#')
        && base.to_ascii_lowercase().ends_with(".zip")
        && !fragment.is_empty()
    {
        return (base, Some(percent_decode(fragment)));
    }
    (source, None)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(value) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The entry `wanted` names: exact, then ignoring case, then by file name.
fn pick_entry(files: &[(usize, String)], wanted: &str) -> Option<usize> {
    let file_name = |n: &str| n.rsplit(['/', '\\']).next().unwrap_or(n).to_string();
    files
        .iter()
        .find(|(_, n)| n == wanted)
        .or_else(|| files.iter().find(|(_, n)| n.eq_ignore_ascii_case(wanted)))
        .or_else(|| {
            let wanted = file_name(wanted);
            files
                .iter()
                .find(|(_, n)| file_name(n).eq_ignore_ascii_case(&wanted))
        })
        .map(|(idx, _)| *idx)
}

fn is_remote(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

fn should_unpack_zip(source: &str, bytes: &[u8]) -> bool {
    source.to_ascii_lowercase().ends_with(".zip")
        || bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
}

fn effective_name(source: &str) -> String {
    if let Ok(url) = reqwest::Url::parse(source)
        && let Some(last) = url
            .path_segments()
            .and_then(|mut segments| segments.rfind(|s| !s.is_empty()))
    {
        return percent_decode(last);
    }

    Path::new(source)
        .file_name()
        .and_then(|n| n.to_str())
        .map_or_else(|| source.to_string(), ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            for (name, data) in entries {
                zip.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(data).unwrap();
            }
            zip.finish().unwrap();
        }
        out.into_inner()
    }

    fn write_temp(label: &str, name: &str, bytes: &[u8]) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("file-loader-test-{label}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_selector_only_applies_to_zip_names() {
        assert_eq!(
            split_entry_selector("a/b.zip#Game%20One.d64"),
            ("a/b.zip", Some("Game One.d64".to_string()))
        );
        assert_eq!(
            split_entry_selector("https://x/y.ZIP#z.tap"),
            ("https://x/y.ZIP", Some("z.tap".to_string()))
        );
        assert_eq!(split_entry_selector("a/b#1.d64"), ("a/b#1.d64", None));
        assert_eq!(split_entry_selector("a/b.zip#"), ("a/b.zip#", None));
        assert_eq!(split_entry_selector("a/b.zip"), ("a/b.zip", None));
    }

    #[test]
    fn a_zip_unpacks_into_the_loader_temp_folder_and_cleanup_removes_it() {
        let path = write_temp("single", "one.zip", &zip_with(&[("game.d64", b"DISK")]));
        let mut loader = FileLoader::new(LoaderConfig::default()).unwrap();
        let loaded = loader.load_binary(path.to_str().unwrap()).unwrap();
        assert_eq!(loaded.bytes, b"DISK");
        assert_eq!(loaded.name, "game.d64");
        let extracted = loaded.extracted_path.unwrap();
        let root = loader.temp_root().unwrap().to_path_buf();
        assert!(extracted.starts_with(&root));
        assert_eq!(fs::read(&extracted).unwrap(), b"DISK");
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(TEMP_PREFIX)
        );
        loader.cleanup();
        assert!(!root.exists());
        assert!(loader.temp_root().is_none());
    }

    #[test]
    fn dropping_the_loader_removes_the_temp_folder() {
        let path = write_temp("drop", "one.zip", &zip_with(&[("a.bin", b"A")]));
        let mut loader = FileLoader::new(LoaderConfig::default()).unwrap();
        loader.load_binary(path.to_str().unwrap()).unwrap();
        let root = loader.temp_root().unwrap().to_path_buf();
        assert!(root.exists());
        drop(loader);
        assert!(!root.exists());
    }

    #[test]
    fn a_multi_file_zip_needs_an_entry_and_lists_the_choices() {
        let path = write_temp(
            "multi",
            "many.zip",
            &zip_with(&[("disk1.d64", b"ONE"), ("sub/Disk2.d64", b"TWO")]),
        );
        let source = path.to_str().unwrap();
        let mut loader = FileLoader::new(LoaderConfig::default()).unwrap();

        let err = loader.load_binary(source).unwrap_err();
        assert!(
            err.contains("#<entry>") && err.contains("disk1.d64, sub/Disk2.d64"),
            "{err}"
        );

        let exact = loader.load_binary(&format!("{source}#disk1.d64")).unwrap();
        assert_eq!(
            (exact.bytes.as_slice(), exact.name.as_str()),
            (&b"ONE"[..], "disk1.d64")
        );

        // Case-insensitive, and by file name alone.
        let by_name = loader.load_binary(&format!("{source}#disk2.d64")).unwrap();
        assert_eq!(by_name.bytes, b"TWO");

        let missing = loader
            .load_binary(&format!("{source}#nope.d64"))
            .unwrap_err();
        assert!(
            missing.contains("no entry named") && missing.contains("disk1.d64"),
            "{missing}"
        );
    }

    #[test]
    fn two_zips_with_the_same_entry_name_do_not_collide() {
        let a = write_temp("same-a", "a.zip", &zip_with(&[("x.bin", b"A")]));
        let b = write_temp("same-b", "b.zip", &zip_with(&[("x.bin", b"B")]));
        let mut loader = FileLoader::new(LoaderConfig::default()).unwrap();
        let first = loader.load_binary(a.to_str().unwrap()).unwrap();
        let second = loader.load_binary(b.to_str().unwrap()).unwrap();
        assert_ne!(first.extracted_path, second.extracted_path);
        assert_eq!(fs::read(first.extracted_path.unwrap()).unwrap(), b"A");
    }

    #[test]
    fn old_temp_folders_are_swept_and_fresh_ones_kept() {
        // A private directory, so parallel tests' real temp folders are safe.
        let base = std::env::temp_dir().join(format!("file-loader-sweep-{}", std::process::id()));
        let ours = base.join(format!("{TEMP_PREFIX}stale"));
        let other = base.join("keep-me");
        fs::create_dir_all(&ours).unwrap();
        fs::create_dir_all(&other).unwrap();
        // Nothing is older than a day yet.
        assert_eq!(remove_stale_in(&base, Duration::from_secs(86_400)), 0);
        assert!(ours.exists());
        // Everything is at least zero seconds old; only our prefix goes.
        assert_eq!(remove_stale_in(&base, Duration::ZERO), 1);
        assert!(!ours.exists());
        assert!(other.exists());
        fs::remove_dir_all(base).unwrap();
    }
}
