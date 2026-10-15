//! Pure helpers for the files the hotkey actions write (screenshots and quick
//! save states): paths, slot stepping and a UTC timestamp. No IO here, so
//! the naming rules are unit-tested directly; `emu_thread` does the writing.

use std::path::{Path, PathBuf};

/// Number of quick-save slots (`0..SLOT_COUNT`).
pub(crate) const SLOT_COUNT: u8 = 10;

/// `slot` moved by `delta`, wrapping around `0..SLOT_COUNT`.
pub(crate) fn step_slot(slot: u8, delta: i8) -> u8 {
    let count = i16::from(SLOT_COUNT);
    (i16::from(slot) + i16::from(delta)).rem_euclid(count) as u8
}

/// `<dir>/<machine>-slot<N>.state`
pub(crate) fn state_path(dir: &Path, machine_id: &str, slot: u8) -> PathBuf {
    dir.join(format!("{}-slot{slot}.state", file_safe(machine_id)))
}

/// `<dir>/<machine>-<YYYYMMDD>-<HHMMSS>.png` (UTC), with `-2`, `-3`... added
/// by `unique_path` if two screenshots land in the same second.
pub(crate) fn screenshot_path(dir: &Path, machine_id: &str, unix_secs: u64) -> PathBuf {
    let (y, mo, d, h, mi, s) = utc_from_unix(unix_secs);
    dir.join(format!(
        "{}-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}.png",
        file_safe(machine_id)
    ))
}

/// `<machine>-<YYYYMMDD>-<HHMMSS>.<extension>` (UTC): the name a "save"
/// dialog proposes.
pub(crate) fn timestamped_name(machine_id: &str, unix_secs: u64, extension: &str) -> String {
    let (y, mo, d, h, mi, s) = utc_from_unix(unix_secs);
    format!(
        "{}-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}.{extension}",
        file_safe(machine_id)
    )
}

/// `path` itself if nothing is there, else `name-2.ext`, `name-3.ext`, ...
pub(crate) fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2u32..)
        .map(|n| path.with_file_name(format!("{stem}-{n}{ext}")))
        .find(|candidate| !candidate.exists())
        .expect("an unused file name exists")
}

/// Machine ids are already file-name friendly (`zx-spectrum-128`); keep
/// anything unexpected from becoming a path separator.
fn file_safe(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Seconds since the Unix epoch to a UTC calendar time (Howard Hinnant's
/// civil-from-days algorithm; no time-zone data needed or available here).
pub(crate) fn utc_from_unix(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, mi, s) = (
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    );
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_wrap_in_both_directions() {
        assert_eq!(step_slot(0, 1), 1);
        assert_eq!(step_slot(9, 1), 0);
        assert_eq!(step_slot(0, -1), 9);
        assert_eq!(step_slot(5, -1), 4);
    }

    #[test]
    fn state_paths_are_per_machine_and_slot() {
        let dir = Path::new("states");
        assert_eq!(
            state_path(dir, "zx-spectrum-128", 3),
            Path::new("states/zx-spectrum-128-slot3.state")
        );
        assert_ne!(state_path(dir, "c64", 0), state_path(dir, "c64", 1));
        assert_eq!(
            state_path(dir, "../evil", 0),
            Path::new("states/___evil-slot0.state")
        );
    }

    #[test]
    fn unix_time_converts_to_utc_calendar_time() {
        assert_eq!(utc_from_unix(0), (1970, 1, 1, 0, 0, 0));
        // 2000-03-01 00:00:00 UTC, right after a leap day.
        assert_eq!(utc_from_unix(951_868_800), (2000, 3, 1, 0, 0, 0));
        // 2024-02-29 12:34:56 UTC
        assert_eq!(utc_from_unix(1_709_210_096), (2024, 2, 29, 12, 34, 56));
        // 2026-10-10 00:00:00 UTC
        assert_eq!(utc_from_unix(1_791_590_400), (2026, 10, 10, 0, 0, 0));
    }

    #[test]
    fn screenshot_names_sort_by_time() {
        let dir = Path::new("shots");
        let a = screenshot_path(dir, "c64", 1_709_210_096);
        let b = screenshot_path(dir, "c64", 1_709_210_097);
        assert_eq!(a, Path::new("shots/c64-20240229-123456.png"));
        assert!(a < b);
    }

    #[test]
    fn unique_path_skips_existing_files() {
        let dir = std::env::temp_dir().join(format!("rc-host-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("shot.png");
        assert_eq!(unique_path(first.clone()), first);
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(unique_path(first.clone()), dir.join("shot-2.png"));
        std::fs::write(dir.join("shot-2.png"), b"x").unwrap();
        assert_eq!(unique_path(first), dir.join("shot-3.png"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
