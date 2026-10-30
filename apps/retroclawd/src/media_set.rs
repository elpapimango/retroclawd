//! Media that is several files at once -- a CD image as a `.cue` sheet beside its `.bin` tracks -- arrives as a
//! `.zip` (what RomM serves for a game kept as a folder, or what a user packs), and the machine wants a path to
//! the sheet with the tracks beside it. A text option whose value is `romm:<id>` or a `.zip` is therefore
//! unpacked, whole, into the cache and replaced by the path of the file that names the set. Any other value is
//! left as it is.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Unpacked sets nobody has used for this long are deleted before the next unpacking.
const STALE_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Marks an unpacked folder that was written completely, so an interrupted unpacking is redone.
const DONE: &str = ".unpacked";

/// The extensions that name a set, best first.
const SET_FILES: [&str; 5] = ["cue", "gdi", "iso", "chd", "toc"];

/// Whether an option value is one this module handles.
fn is_set_source(value: &str) -> bool {
    value.starts_with("romm:") || value.to_ascii_lowercase().ends_with(".zip")
}

/// The value an option should carry: for a RomM item or a `.zip`, the path of the file that names the unpacked
/// set; anything else comes back unchanged.
pub(crate) fn resolve(value: &str) -> Result<String, String> {
    if !is_set_source(value) {
        return Ok(value.to_string());
    }
    let archive = match crate::romm::parse_source(value) {
        Some(item) => crate::romm::fetch(item?)?,
        None => PathBuf::from(value),
    };
    if !is_zip(&archive)? {
        // A single file from RomM (an .iso, a .chd): nothing to unpack.
        return Ok(archive.display().to_string());
    }
    let root = crate::romm::cache_root()?.join("unpacked");
    Ok(unpack_into(&archive, &root)?.display().to_string())
}

fn is_zip(path: &Path) -> Result<bool, String> {
    use io::Read;
    let mut magic = [0u8; 4];
    let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(file.read_exact(&mut magic).is_ok() && magic == *b"PK\x03\x04")
}

/// Unpacks `archive` under `root` (once per archive version) and returns the file that names the set.
fn unpack_into(archive: &Path, root: &Path) -> Result<PathBuf, String> {
    let meta = fs::metadata(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let stamp = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let stem = archive
        .file_stem()
        .map_or_else(|| "set".to_string(), |s| s.to_string_lossy().into_owned());
    let dir = root.join(format!("{stem}-{}-{stamp}", meta.len()));
    if !dir.join(DONE).is_file() {
        prune(root, &dir);
        let _ = fs::remove_dir_all(&dir);
        extract(archive, &dir)?;
        fs::write(dir.join(DONE), b"").map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    touch(&dir);
    pick(&dir, archive)
}

fn extract(archive: &Path, dir: &Path) -> Result<(), String> {
    let name = archive.display();
    let file = fs::File::open(archive).map_err(|e| format!("{name}: {e}"))?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(file))
        .map_err(|e| format!("{name} is not a readable zip: {e}"))?;
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| format!("{name}: {e}"))?;
        // An entry that would land outside the folder (`..`, a drive or a root) is skipped, never followed.
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let target = dir.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut out =
            fs::File::create(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        io::copy(&mut entry, &mut out).map_err(|e| format!("{name}: {}: {e}", target.display()))?;
    }
    Ok(())
}

/// The file that names the set: the first `.cue` (then `.gdi`, `.iso`, `.chd`, `.toc`) in name order.
fn pick(dir: &Path, archive: &Path) -> Result<PathBuf, String> {
    let mut files = Vec::new();
    collect(dir, &mut files);
    files.sort();
    for wanted in SET_FILES {
        if let Some(found) = files.iter().find(|p| {
            p.extension()
                .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(wanted))
        }) {
            return Ok(found.clone());
        }
    }
    Err(format!(
        "{} holds no disc image: expected a {} file",
        archive.display(),
        SET_FILES.map(|e| format!(".{e}")).join(", ")
    ))
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    for path in read.flatten().map(|e| e.path()) {
        if path.is_dir() {
            collect(&path, out);
        } else if path.file_name().is_some_and(|n| n != DONE) {
            out.push(path);
        }
    }
}

fn touch(dir: &Path) {
    set_used(&dir.join(DONE), SystemTime::now());
}

/// Records `when` as the last use of an unpacked set (the marker's modification time).
fn set_used(marker: &Path, when: SystemTime) {
    let _ = fs::OpenOptions::new()
        .write(true)
        .open(marker)
        .and_then(|f| f.set_modified(when));
}

/// Deletes the unpacked sets under `root` (except `keep`) that have not been used for a week.
fn prune(root: &Path, keep: &Path) {
    let Ok(read) = fs::read_dir(root) else { return };
    for path in read.flatten().map(|e| e.path()) {
        let used = fs::metadata(path.join(DONE)).and_then(|m| m.modified());
        let stale = used.is_ok_and(|t| t.elapsed().is_ok_and(|age| age > STALE_AFTER));
        if path != keep && stale {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("retroclawd-set-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zip_of(path: &Path, files: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, data) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn values_that_are_not_archives_pass_through() {
        assert_eq!(resolve("saves/x.brm").unwrap(), "saves/x.brm");
        assert_eq!(resolve("game.cue").unwrap(), "game.cue");
    }

    #[test]
    fn a_zip_of_a_cue_sheet_and_its_tracks_is_unpacked_whole_and_names_the_sheet() {
        let dir = scratch("cue");
        let zip = dir.join("Game (USA).zip");
        zip_of(
            &zip,
            &[
                ("Game (USA) (Track 2).bin", b"audio"),
                ("Game (USA).cue", b"FILE ..."),
                ("Game (USA) (Track 1).bin", b"data"),
            ],
        );
        let cue = unpack_into(&zip, &dir.join("cache")).unwrap();
        assert_eq!(cue.file_name().unwrap(), "Game (USA).cue");
        assert_eq!(
            fs::read(cue.with_file_name("Game (USA) (Track 1).bin")).unwrap(),
            b"data"
        );
        assert_eq!(
            fs::read(cue.with_file_name("Game (USA) (Track 2).bin")).unwrap(),
            b"audio"
        );
    }

    #[test]
    fn a_second_call_reuses_the_unpacked_folder() {
        let dir = scratch("again");
        let zip = dir.join("g.zip");
        zip_of(&zip, &[("g.cue", b"x"), ("g.bin", b"y")]);
        let cache = dir.join("cache");
        let first = unpack_into(&zip, &cache).unwrap();
        fs::write(&first, b"changed on disk").unwrap();
        let second = unpack_into(&zip, &cache).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            fs::read(&second).unwrap(),
            b"changed on disk",
            "not unpacked again"
        );
    }

    #[test]
    fn an_interrupted_unpacking_is_redone() {
        let dir = scratch("redo");
        let zip = dir.join("g.zip");
        zip_of(&zip, &[("g.cue", b"x")]);
        let cache = dir.join("cache");
        let cue = unpack_into(&zip, &cache).unwrap();
        fs::remove_file(cue.parent().unwrap().join(DONE)).unwrap();
        fs::remove_file(&cue).unwrap();
        assert_eq!(fs::read(unpack_into(&zip, &cache).unwrap()).unwrap(), b"x");
    }

    #[test]
    fn an_iso_in_a_zip_is_found_and_a_zip_without_a_disc_is_an_error() {
        let dir = scratch("iso");
        let iso = dir.join("a.zip");
        zip_of(&iso, &[("readme.txt", b"hi"), ("sub/disc.ISO", b"iso")]);
        let found = unpack_into(&iso, &dir.join("cache")).unwrap();
        assert_eq!(found.file_name().unwrap(), "disc.ISO");
        let none = dir.join("b.zip");
        zip_of(&none, &[("readme.txt", b"hi")]);
        let err = unpack_into(&none, &dir.join("cache")).unwrap_err();
        assert!(
            err.contains("no disc image") && err.contains(".cue"),
            "{err}"
        );
    }

    #[test]
    fn an_entry_that_climbs_out_of_the_folder_is_not_written() {
        let dir = scratch("slip");
        let zip = dir.join("s.zip");
        zip_of(&zip, &[("../escaped.txt", b"bad"), ("s.cue", b"x")]);
        unpack_into(&zip, &dir.join("cache")).unwrap();
        assert!(!dir.join("escaped.txt").exists());
        assert!(!dir.join("cache").join("escaped.txt").exists());
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_not_unpacked() {
        let dir = scratch("plain");
        let iso = dir.join("game.iso");
        fs::write(&iso, b"not a zip at all").unwrap();
        assert!(!is_zip(&iso).unwrap());
    }

    #[test]
    fn old_unpacked_sets_are_pruned_and_the_one_in_use_is_kept() {
        let dir = scratch("prune");
        let root = dir.join("cache");
        let (old, keep) = (root.join("old"), root.join("keep"));
        for d in [&old, &keep] {
            fs::create_dir_all(d).unwrap();
            fs::write(d.join(DONE), b"").unwrap();
        }
        let long_ago = SystemTime::now() - STALE_AFTER - Duration::from_secs(60);
        for d in [&old, &keep] {
            set_used(&d.join(DONE), long_ago);
        }
        prune(&root, &keep);
        assert!(!old.exists() && keep.exists());
    }
}
