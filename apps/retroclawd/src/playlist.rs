//! `.m3u` disk playlists: `--disk games.m3u` attaches the first entry and the
//! `next_disk` / `prev_disk` hotkeys step through the rest (multi-disk games).

use std::path::{Path, PathBuf};

/// Parses an `.m3u` playlist: one entry per line, `#` lines (including
/// `#EXTM3U` / `#EXTINF`) and blank lines ignored. An entry is a path, a URL or
/// an `archive.zip#entry`; a relative one resolves against `playlist_source`'s
/// own folder (or URL directory), an absolute path or a URL stays as written.
pub(crate) fn parse_m3u(text: &str, playlist_source: &str) -> Result<Vec<PathBuf>, String> {
    let entries: Vec<PathBuf> = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| resolve_entry(l, playlist_source))
        .collect();
    if entries.is_empty() {
        return Err("the playlist lists no disks".to_string());
    }
    Ok(entries)
}

fn is_url(text: &str) -> bool {
    text.starts_with("http://") || text.starts_with("https://")
}

fn resolve_entry(entry: &str, playlist_source: &str) -> PathBuf {
    if is_url(entry) {
        return PathBuf::from(entry);
    }
    let entry = entry.replace('\\', "/");
    if is_url(playlist_source) {
        // Relative to the playlist's URL directory (query and fragment dropped).
        let url = playlist_source
            .split(['?', '#'])
            .next()
            .unwrap_or(playlist_source);
        let dir = url.rsplit_once('/').map_or(url, |(dir, _)| dir);
        return PathBuf::from(if let Some(root) = entry.strip_prefix('/') {
            // Site-absolute: keep scheme and host.
            let origin_end = url.find("://").map_or(0, |i| i + 3);
            let host_end = url[origin_end..]
                .find('/')
                .map_or(url.len(), |i| origin_end + i);
            format!("{}/{root}", &url[..host_end])
        } else {
            format!("{dir}/{entry}")
        });
    }
    let path = PathBuf::from(&entry);
    if path.is_absolute() {
        path
    } else {
        Path::new(playlist_source)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    }
}

/// True when `path` names a playlist.
pub(crate) fn is_playlist(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("m3u"))
}

/// The index `step` places away from `current`, wrapping around `len`.
pub(crate) fn step_index(current: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_blank_lines_are_skipped_and_paths_resolve_against_the_playlist_folder() {
        let text =
            "\u{feff}#EXTM3U\n#EXTINF:0,Disk 1\nside1.d64\n\n  sub\\side2.d64  \n/abs/side3.d64\n";
        let list = parse_m3u(text, "/games/list.m3u").unwrap();
        assert_eq!(
            list,
            vec![
                PathBuf::from("/games/side1.d64"),
                PathBuf::from("/games/sub/side2.d64"),
                PathBuf::from("/abs/side3.d64"),
            ]
        );
    }

    #[test]
    fn urls_zips_and_entries_resolve_against_a_url_playlist() {
        let text = "side1.d64\nhttps://other.org/s2.zip\n/abs/s3.d64\npack.zip#disk 4.d64\n";
        let list = parse_m3u(text, "https://host.org/games/c64/list.m3u?x=1").unwrap();
        assert_eq!(
            list,
            vec![
                PathBuf::from("https://host.org/games/c64/side1.d64"),
                PathBuf::from("https://other.org/s2.zip"),
                PathBuf::from("https://host.org/abs/s3.d64"),
                PathBuf::from("https://host.org/games/c64/pack.zip#disk 4.d64"),
            ]
        );
        // A local playlist may name URLs too.
        let local = parse_m3u("https://x.org/a.d64\nb.zip#c.d64\n", "/g/list.m3u").unwrap();
        assert_eq!(local[0], PathBuf::from("https://x.org/a.d64"));
        assert_eq!(local[1], PathBuf::from("/g/b.zip#c.d64"));
    }

    #[test]
    fn an_empty_playlist_is_an_error() {
        assert!(parse_m3u("#EXTM3U\n\n", "list.m3u").is_err());
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        assert_eq!(step_index(0, 3, true), 1);
        assert_eq!(step_index(2, 3, true), 0);
        assert_eq!(step_index(0, 3, false), 2);
        assert_eq!(step_index(1, 3, false), 0);
        assert_eq!(step_index(0, 0, true), 0);
    }

    #[test]
    fn playlists_are_recognised_by_extension() {
        assert!(is_playlist("a/b.M3U"));
        assert!(!is_playlist("a/b.d64"));
    }
}
