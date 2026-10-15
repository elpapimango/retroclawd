//! Everything the user hands the emulator to read -- media, programs, ROMs,
//! states, configs -- may be a plain file, a `.zip` holding one file (or
//! `archive.zip#entry` for one of several), or an `http(s)://` URL (to
//! either). `file_loader::FileLoader` does the fetching and unzipping; this
//! module adds
//!
//! * a process-wide cache so the extension probe (which decides what the file
//!   *is*, e.g. the `.d64` inside `game.zip`) and the read that follows share
//!   one download/extraction;
//! * one loader for the whole run, so every unzipped file lives in a single
//!   temporary folder that [`cleanup`] (or dropping a [`CleanupGuard`]) removes
//!   at exit, and folders a crashed run left behind are swept at the next start;
//! * progress reporting: lines on stderr with `--progress`, and one-line toasts
//!   for downloads that start after the window is open.

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use file_loader::{FileLoader, LoaderConfig, Progress, ProgressFn};

use crate::emu_thread::Messages;

/// Temporary folders older than this are leftovers of a run that never got to
/// clean up (killed, crashed) and are removed at the next start.
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// A loaded input: its bytes and its effective name (the entry's name for a
/// zip, the last URL path segment for a URL), which decides its extension.
#[derive(Debug)]
pub(crate) struct Loaded {
    pub(crate) bytes: Vec<u8>,
    pub(crate) name: String,
}

fn cache() -> &'static Mutex<HashMap<String, Arc<Loaded>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Loaded>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The run's one loader; `None` before the first load and after [`cleanup`].
fn loader() -> &'static Mutex<Option<FileLoader>> {
    static LOADER: Mutex<Option<FileLoader>> = Mutex::new(None);
    &LOADER
}

static STDERR_PROGRESS: AtomicBool = AtomicBool::new(false);

fn toast_target() -> &'static Mutex<Option<Messages>> {
    static TARGET: Mutex<Option<Messages>> = Mutex::new(None);
    &TARGET
}

/// `--progress`: print download / unzip progress on stderr.
pub(crate) fn enable_stderr_progress() {
    STDERR_PROGRESS.store(true, Ordering::Relaxed);
}

/// From now on, show downloads as toasts through `messages` (set once the
/// window exists; downloads before that have no toast to land on).
pub(crate) fn set_toast_target(messages: Messages) {
    if let Ok(mut target) = toast_target().lock() {
        *target = Some(messages);
    }
}

/// Removes every unzipped file. Loaded inputs stay cached in memory, so
/// nothing already read is lost; a later zip simply unpacks into a new folder.
pub(crate) fn cleanup() {
    if let Ok(mut guard) = loader().lock()
        && let Some(mut l) = guard.take()
    {
        l.cleanup();
    }
}

/// Runs [`cleanup`] when dropped -- at the end of `main`, including while
/// unwinding from a panic.
pub(crate) struct CleanupGuard;

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        cleanup();
    }
}

fn display_name(source: &str) -> String {
    let base = file_loader::split_entry_selector(source).0;
    base.trim_end_matches('/')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(base)
        .to_string()
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

/// The progress line for `event` (`None` = nothing worth saying) and whether
/// it ends the line.
fn progress_text(event: &Progress) -> Option<(String, bool)> {
    match event {
        Progress::Downloading {
            source,
            received,
            total,
        } => {
            let name = display_name(source);
            Some((
                match total {
                    Some(total) if *total > 0 => format!(
                        "Downloading {name}: {} of {} ({}%)",
                        megabytes(*received),
                        megabytes(*total),
                        received * 100 / total
                    ),
                    _ if *received > 0 => format!("Downloading {name}: {}", megabytes(*received)),
                    _ => format!("Downloading {name}"),
                },
                false,
            ))
        }
        Progress::Downloaded {
            source,
            bytes,
            seconds,
        } => Some((
            format!(
                "Downloaded {} ({}) in {seconds:.1} s",
                display_name(source),
                megabytes(*bytes)
            ),
            true,
        )),
        Progress::Extracting { entry } => Some((format!("Unpacking {entry}"), true)),
    }
}

/// Progress of a download made outside `FileLoader` (RomM).
pub(crate) fn report_progress(event: &Progress) {
    report(event);
}

fn report(event: &Progress) {
    let Some((text, ends_line)) = progress_text(event) else {
        return;
    };
    if STDERR_PROGRESS.load(Ordering::Relaxed) {
        let mut err = std::io::stderr();
        if err.is_terminal() {
            // Rewrite one line in place; pad over a longer previous one.
            let _ = write!(err, "\r{text:<78}");
            if ends_line {
                let _ = writeln!(err);
            }
        } else if ends_line || matches!(event, Progress::Downloading { received: 0, .. }) {
            // Redirected: only the first and last lines, not every update.
            let _ = writeln!(err, "{text}");
        }
    }
    // A toast has room for the start and the end, not a running percentage.
    let toast = match event {
        Progress::Downloading { received: 0, .. } | Progress::Downloaded { .. } => true,
        Progress::Downloading { .. } | Progress::Extracting { .. } => false,
    };
    if toast
        && let Ok(target) = toast_target().lock()
        && let Some(messages) = target.as_ref()
    {
        crate::emu_thread::post_message(messages, text);
    }
}

#[cfg(test)]
fn temp_root() -> Option<std::path::PathBuf> {
    loader()
        .lock()
        .ok()?
        .as_ref()
        .and_then(|l| l.temp_root().map(Path::to_path_buf))
}

/// Reads `source` (path or URL, optionally `archive.zip#entry`), unpacking a
/// zip into the run's temporary folder.
pub(crate) fn load(source: &str) -> Result<Arc<Loaded>, String> {
    // `romm:<id>` / `romm:fw/<id>` (optionally `#entry` for a multi-file zip):
    // fetched into the RomM cache, then read as the local file it is.
    if let Some(parsed) = crate::romm::parse_source(source.split('#').next().unwrap_or(source)) {
        if let Some(hit) = cache().lock().ok().and_then(|c| c.get(source).cloned()) {
            return Ok(hit);
        }
        let path = crate::romm::fetch(parsed?)?;
        let local = match source.split_once('#') {
            Some((_, entry)) => format!("{}#{entry}", path.display()),
            None => path.display().to_string(),
        };
        let loaded = load(&local)?;
        if let Ok(mut c) = cache().lock() {
            c.insert(source.to_string(), Arc::clone(&loaded));
        }
        return Ok(loaded);
    }
    if let Some(hit) = cache().lock().ok().and_then(|c| c.get(source).cloned()) {
        return Ok(hit);
    }
    let loaded = {
        let mut guard = loader()
            .lock()
            .map_err(|_| "the input loader is unavailable".to_string())?;
        if guard.is_none() {
            file_loader::remove_stale_temp_dirs(STALE_AFTER);
            let mut fresh = FileLoader::new(LoaderConfig::default())?;
            let sink: ProgressFn = Arc::new(report);
            fresh.set_progress(Some(sink));
            *guard = Some(fresh);
        }
        guard.as_mut().expect("created above").load_binary(source)?
    };
    let loaded = Arc::new(Loaded {
        bytes: loaded.bytes,
        name: loaded.name,
    });
    if let Ok(mut c) = cache().lock() {
        c.insert(source.to_string(), Arc::clone(&loaded));
    }
    Ok(loaded)
}

/// The lowercase extension of what `source` really is: the entry inside a
/// zip, the file named by a URL. Falls back to the source's own extension
/// when it cannot be read (the read itself reports that error).
pub(crate) fn extension(source: &str) -> String {
    let name = load(source).map_or_else(|_| source.to_string(), |l| l.name.clone());
    Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Whether `source` is an ordinary local file that can be written back to
/// (not a URL, not a zip entry).
pub(crate) fn is_writable_file(source: &str) -> bool {
    let base = file_loader::split_entry_selector(source).0;
    !(base.starts_with("http://") || base.starts_with("https://") || base.starts_with("romm:"))
        && !base.to_ascii_lowercase().ends_with(".zip")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_bytes(name: &str, data: &[u8]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
            zip.finish().unwrap();
        }
        out.into_inner()
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("retroclawd-inputs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn a_plain_file_is_read_as_is() {
        let path = temp("plain.d64");
        std::fs::write(&path, b"abc").unwrap();
        let source = path.to_string_lossy().into_owned();
        assert_eq!(load(&source).unwrap().bytes, b"abc");
        assert_eq!(extension(&source), "d64");
    }

    #[test]
    fn a_zip_is_unpacked_and_its_entry_names_the_format() {
        let path = temp("game.zip");
        std::fs::write(&path, zip_bytes("Some Game (19xx).D64", b"disk bytes")).unwrap();
        let source = path.to_string_lossy().into_owned();
        assert_eq!(load(&source).unwrap().bytes, b"disk bytes");
        assert_eq!(extension(&source), "d64", "from the entry, not `.zip`");
    }

    #[test]
    fn a_missing_file_is_a_clear_error_and_keeps_its_own_extension() {
        let source = temp("missing.tap").to_string_lossy().into_owned();
        assert!(load(&source).unwrap_err().contains("failed to read"));
        assert_eq!(extension(&source), "tap");
    }

    /// A one-shot local HTTP server answering every request with `body`.
    fn serve(body: Vec<u8>) -> String {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            while let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[test]
    fn a_url_is_fetched_and_a_zip_behind_it_is_unpacked() {
        let url = format!("{}/files/a.zip", serve(zip_bytes("inner.prg", b"prg")));
        assert_eq!(load(&url).unwrap().bytes, b"prg");
        assert_eq!(extension(&url), "prg");
        let plain = format!("{}/x/tape.TAP", serve(b"tap".to_vec()));
        assert_eq!(load(&plain).unwrap().bytes, b"tap");
        assert_eq!(extension(&plain), "tap");
    }

    #[test]
    fn only_local_non_zip_files_can_be_written_back() {
        assert!(is_writable_file("disk.d64"));
        assert!(!is_writable_file("disk.ZIP"));
        assert!(!is_writable_file("https://example.org/disk.d64"));
    }

    #[test]
    fn one_entry_of_a_multi_file_zip_is_picked_with_a_hash() {
        let path = temp("multi.zip");
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            for (name, data) in [("a.d64", &b"A"[..]), ("b.TAP", &b"B"[..])] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(data).unwrap();
            }
            zip.finish().unwrap();
        }
        std::fs::write(&path, out.into_inner()).unwrap();
        let source = path.to_string_lossy().into_owned();
        assert!(load(&source).unwrap_err().contains("#<entry>"));
        assert_eq!(load(&format!("{source}#b.tap")).unwrap().bytes, b"B");
        assert_eq!(extension(&format!("{source}#b.tap")), "tap");
        assert!(!is_writable_file(&format!("{source}#a.d64")));
    }

    #[test]
    fn cleanup_removes_the_unzipped_files() {
        let path = temp("cleanup.zip");
        std::fs::write(&path, zip_bytes("inner.bin", b"x")).unwrap();
        load(&path.to_string_lossy()).unwrap();
        let root = temp_root().expect("a zip was unpacked");
        assert!(root.exists());
        cleanup();
        assert!(!root.exists());
        assert!(temp_root().is_none());
        // Already-read inputs stay cached; a new zip unpacks into a new folder.
        assert_eq!(load(&path.to_string_lossy()).unwrap().bytes, b"x");
        let path2 = temp("cleanup2.zip");
        std::fs::write(&path2, zip_bytes("other.bin", b"y")).unwrap();
        load(&path2.to_string_lossy()).unwrap();
        assert!(temp_root().is_some_and(|r| r.exists()));
    }

    #[test]
    fn progress_lines_name_the_file_and_the_amount() {
        let start = Progress::Downloading {
            source: "https://h/dir/Game%20One.zip#x".into(),
            received: 0,
            total: None,
        };
        assert_eq!(
            progress_text(&start),
            Some(("Downloading Game%20One.zip".to_string(), false))
        );
        let part = Progress::Downloading {
            source: "https://h/g.zip".into(),
            received: 1_048_576,
            total: Some(4_194_304),
        };
        assert_eq!(
            progress_text(&part).unwrap().0,
            "Downloading g.zip: 1.0 MB of 4.0 MB (25%)"
        );
        let done = Progress::Downloaded {
            source: "https://h/g.zip".into(),
            bytes: 2_097_152,
            seconds: 1.25,
        };
        assert_eq!(
            progress_text(&done),
            Some(("Downloaded g.zip (2.0 MB) in 1.2 s".to_string(), true))
        );
    }
}
