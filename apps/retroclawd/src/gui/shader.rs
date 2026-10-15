//! Resolves `--shader`'s value into WGSL source text. `None` (the
//! default) or one of six bundled preset names (`crt`/`scanlines`/
//! `sharp-bilinear`/`crt-geom`/`zfast-crt`/`crt-potato`) returns this
//! binary's own embedded `.wgsl` text (via `include_str!`, so no runtime
//! dependency on `assets/shaders/` exists); anything else is treated as a
//! filesystem path and read directly with `std::fs` -- deliberately not
//! through Bevy's asset server, which resolves relative paths against its
//! own `assets/` root. See each shader file and `CREDITS.md` for upstream
//! provenance and port limitations.

const PASSTHROUGH: &str = include_str!("../../../../assets/shaders/passthrough.wgsl");
const CRT: &str = include_str!("../../../../assets/shaders/crt.wgsl");
const SCANLINES: &str = include_str!("../../../../assets/shaders/scanlines.wgsl");
const SHARP_BILINEAR: &str = include_str!("../../../../assets/shaders/sharp-bilinear.wgsl");
const CRT_GEOM: &str = include_str!("../../../../assets/shaders/crt-geom.wgsl");
const ZFAST_CRT: &str = include_str!("../../../../assets/shaders/zfast-crt.wgsl");
const CRT_POTATO: &str = include_str!("../../../../assets/shaders/crt-potato.wgsl");

/// The bundled shader names, in the order `shader_next`/`shader_prev` cycle
/// through them.
pub(crate) const BUNDLED: [&str; 7] = [
    "passthrough",
    "crt",
    "scanlines",
    "sharp-bilinear",
    "crt-geom",
    "zfast-crt",
    "crt-potato",
];

/// The bundled shader `delta` places from `current` (wrapping). A custom
/// path or `None` counts as `passthrough`, so the first press from either
/// lands on a real, named shader.
pub(crate) fn cycle(current: Option<&str>, delta: i32) -> &'static str {
    let index = BUNDLED
        .iter()
        .position(|name| Some(*name) == current)
        .unwrap_or(0) as i32;
    BUNDLED[(index + delta).rem_euclid(BUNDLED.len() as i32) as usize]
}

/// Returns the WGSL source text `selector` names, or an error string if a
/// custom path couldn't be read. Never fails for a bundled name or `None`
/// -- those are compiled into the binary.
pub(crate) fn resolve_source(selector: Option<&str>) -> Result<String, String> {
    Ok(match selector {
        None | Some("passthrough") => PASSTHROUGH.to_string(),
        Some("crt") => CRT.to_string(),
        Some("scanlines") => SCANLINES.to_string(),
        Some("sharp-bilinear") => SHARP_BILINEAR.to_string(),
        Some("crt-geom") => CRT_GEOM.to_string(),
        Some("zfast-crt") => ZFAST_CRT.to_string(),
        Some("crt-potato") => CRT_POTATO.to_string(),
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| format!("--shader: failed to read {path}: {e}"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycling_walks_the_bundled_list_and_wraps() {
        assert_eq!(cycle(Some("passthrough"), 1), "crt");
        assert_eq!(cycle(Some("crt-potato"), 1), "passthrough");
        assert_eq!(cycle(Some("passthrough"), -1), "crt-potato");
        assert_eq!(cycle(Some("crt"), -1), "passthrough");
        // No shader chosen / a custom file: start from the beginning.
        assert_eq!(cycle(None, 1), "crt");
        assert_eq!(cycle(Some("my/custom.wgsl"), 1), "crt");
    }

    #[test]
    fn every_bundled_name_resolves() {
        for name in BUNDLED {
            assert!(!resolve_source(Some(name)).unwrap().is_empty(), "{name}");
        }
    }

    #[test]
    fn no_selector_is_the_passthrough_shader() {
        assert_eq!(resolve_source(None).unwrap(), PASSTHROUGH);
    }

    #[test]
    fn bundled_names_resolve_to_their_own_source() {
        assert_eq!(resolve_source(Some("crt")).unwrap(), CRT);
        assert_eq!(resolve_source(Some("scanlines")).unwrap(), SCANLINES);
        assert_eq!(
            resolve_source(Some("sharp-bilinear")).unwrap(),
            SHARP_BILINEAR
        );
        assert_eq!(resolve_source(Some("crt-geom")).unwrap(), CRT_GEOM);
        assert_eq!(resolve_source(Some("zfast-crt")).unwrap(), ZFAST_CRT);
        assert_eq!(resolve_source(Some("crt-potato")).unwrap(), CRT_POTATO);
    }

    #[test]
    fn unknown_name_is_treated_as_a_path_and_fails_to_read() {
        let err = resolve_source(Some("not-a-real-shader")).unwrap_err();
        assert!(err.contains("not-a-real-shader"));
    }

    #[test]
    fn a_real_path_is_read_from_disk() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "retroclawd-shader-test-{}.wgsl",
            std::process::id()
        ));
        std::fs::write(&path, "// custom").unwrap();
        let source = resolve_source(Some(path.to_str().unwrap())).unwrap();
        assert_eq!(source, "// custom");
        std::fs::remove_file(&path).unwrap();
    }
}
