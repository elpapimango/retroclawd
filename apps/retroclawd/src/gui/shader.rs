//! Shader presets: what `--shader` (and `shader =` in the settings file) names, resolved to the WGSL source of every pass and the
//! textures a preset loads, before any window opens, so a bad file fails cleanly.
//!
//! A preset is a chain of passes. Each pass draws a full-screen quad with its own WGSL fragment shader; it reads the output of the
//! pass before it (`input`; the first pass reads the emulated picture), the emulated picture itself (`original`) and one optional
//! extra texture (`aux`, an earlier pass's output or a texture file); every pass but the last draws into an image, the last draws
//! the picture on screen. The bindings every pass shader can use are in `assets/shaders/README` (and in `gui::video`).
//!
//! `--shader` takes
//! * a bundled name: `passthrough`, `crt`, `scanlines`, `sharp-bilinear`, `crt-geom`, `zfast-crt`, `crt-potato` (one pass each)
//!   or `crt-bloom` (four passes);
//! * a path to a `.toml` preset (`[[pass]]` and `[[texture]]` tables, see `PresetFile`);
//! * a path to a `.wgsl` file: a one-pass preset.

use std::path::Path;

use serde::Deserialize;

const PASSTHROUGH: &str = include_str!("../../../../assets/shaders/passthrough.wgsl");
const CRT: &str = include_str!("../../../../assets/shaders/crt.wgsl");
const SCANLINES: &str = include_str!("../../../../assets/shaders/scanlines.wgsl");
const SHARP_BILINEAR: &str = include_str!("../../../../assets/shaders/sharp-bilinear.wgsl");
const CRT_GEOM: &str = include_str!("../../../../assets/shaders/crt-geom.wgsl");
const ZFAST_CRT: &str = include_str!("../../../../assets/shaders/zfast-crt.wgsl");
const CRT_POTATO: &str = include_str!("../../../../assets/shaders/crt-potato.wgsl");
const BLOOM_PREFILTER: &str = include_str!("../../../../assets/shaders/bloom-prefilter.wgsl");
const BLUR_H: &str = include_str!("../../../../assets/shaders/blur-h.wgsl");
const BLUR_V: &str = include_str!("../../../../assets/shaders/blur-v.wgsl");
const CRT_BLOOM: &str = include_str!("../../../../assets/shaders/crt-bloom.wgsl");
const CRT_BLOOM_PRESET: &str = include_str!("../../../../assets/shaders/crt-bloom.toml");

/// The bundled presets, in the order `shader_next`/`shader_prev` cycle through them.
pub(crate) const BUNDLED: [&str; 8] = [
    "passthrough",
    "crt",
    "scanlines",
    "sharp-bilinear",
    "crt-geom",
    "zfast-crt",
    "crt-potato",
    "crt-bloom",
];

/// The WGSL of a bundled pass shader, by name (every bundled preset name is also a pass shader; the bloom parts are only passes).
fn bundled_shader(name: &str) -> Option<&'static str> {
    Some(match name {
        "passthrough" => PASSTHROUGH,
        "crt" => CRT,
        "scanlines" => SCANLINES,
        "sharp-bilinear" => SHARP_BILINEAR,
        "crt-geom" => CRT_GEOM,
        "zfast-crt" => ZFAST_CRT,
        "crt-potato" => CRT_POTATO,
        "bloom-prefilter" => BLOOM_PREFILTER,
        "blur-h" => BLUR_H,
        "blur-v" => BLUR_V,
        "crt-bloom" => CRT_BLOOM,
        _ => return None,
    })
}

/// How a pass's output size follows the sizes around it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ScaleType {
    /// `scale` times the size of the pass's input (the emulated picture for the first pass).
    #[default]
    Source,
    /// `scale` times the size of the picture on screen, in physical pixels.
    Viewport,
    /// `width` x `height`.
    Absolute,
}

/// How the next pass samples a pass's output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Filter {
    Nearest,
    #[default]
    Linear,
}

/// The extra texture a pass may read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Aux {
    #[default]
    None,
    /// The output of an earlier pass.
    Pass(usize),
    /// A texture of the preset.
    Texture(usize),
}

/// One pass, resolved.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedPass {
    pub(crate) wgsl: String,
    pub(crate) scale_type: ScaleType,
    pub(crate) scale: f32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) filter: Filter,
    pub(crate) aux: Aux,
}

/// A texture a preset loads (RGBA, 8 bits a channel).
#[derive(Clone, Debug)]
pub(crate) struct ResolvedTexture {
    pub(crate) name: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

/// A preset ready to run.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedPreset {
    pub(crate) name: String,
    pub(crate) passes: Vec<ResolvedPass>,
    pub(crate) textures: Vec<ResolvedTexture>,
}

impl ResolvedPreset {
    fn single(name: &str, wgsl: String) -> Self {
        Self {
            name: name.to_string(),
            passes: vec![ResolvedPass {
                wgsl,
                scale_type: ScaleType::Source,
                scale: 1.0,
                width: 0,
                height: 0,
                filter: Filter::Linear,
                aux: Aux::None,
            }],
            textures: Vec::new(),
        }
    }
}

/// The `.toml` preset format.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetFile {
    name: Option<String>,
    #[serde(default, rename = "pass")]
    passes: Vec<PassFile>,
    #[serde(default, rename = "texture")]
    textures: Vec<TextureFile>,
}

/// `[[pass]]`: `shader` is a bundled shader name or a `.wgsl` path (relative to the preset); `scale_type` is `source` (default),
/// `viewport` or `absolute` (with `width` and `height`); `scale` multiplies the source or viewport size (default 1);
/// `filter` (`linear` default or `nearest`) is how the next pass samples this pass's output; `aux` is `pass<N>` (an earlier pass,
/// counted from 0) or the `name` of a `[[texture]]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PassFile {
    shader: String,
    #[serde(default)]
    scale_type: ScaleType,
    scale: Option<f32>,
    width: Option<u32>,
    height: Option<u32>,
    #[serde(default)]
    filter: Filter,
    aux: Option<String>,
}

/// `[[texture]]`: a PNG (path relative to the preset) a pass can read as `aux`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextureFile {
    name: String,
    path: String,
}

/// The largest side an intermediate image may have.
const MAX_SIDE: u32 = 8192;

fn resolve_preset(text: &str, dir: Option<&Path>, origin: &str) -> Result<ResolvedPreset, String> {
    let file: PresetFile = toml::from_str(text).map_err(|e| format!("{origin}: {e}"))?;
    if file.passes.is_empty() {
        return Err(format!("{origin}: a preset needs at least one [[pass]]"));
    }
    let mut textures = Vec::new();
    for t in &file.textures {
        if textures.iter().any(|x: &ResolvedTexture| x.name == t.name) {
            return Err(format!("{origin}: texture \"{}\" is listed twice", t.name));
        }
        let path = dir.map_or_else(|| Path::new(&t.path).to_path_buf(), |d| d.join(&t.path));
        let bytes = std::fs::read(&path).map_err(|e| {
            format!(
                "{origin}: texture \"{}\": failed to read {}: {e}",
                t.name,
                path.display()
            )
        })?;
        let (width, height, rgba) = super::bezel::decode_png(&bytes)
            .map_err(|e| format!("{origin}: texture \"{}\": {e}", t.name))?;
        textures.push(ResolvedTexture {
            name: t.name.clone(),
            width,
            height,
            rgba,
        });
    }
    let mut passes = Vec::new();
    for (index, p) in file.passes.iter().enumerate() {
        let at = format!("{origin}: pass {index}");
        let wgsl = match bundled_shader(&p.shader) {
            Some(text) => text.to_string(),
            None => {
                let path =
                    dir.map_or_else(|| Path::new(&p.shader).to_path_buf(), |d| d.join(&p.shader));
                std::fs::read_to_string(&path).map_err(|e| {
                    format!(
                        "{at}: shader \"{}\": failed to read {}: {e}",
                        p.shader,
                        path.display()
                    )
                })?
            }
        };
        let scale = p.scale.unwrap_or(1.0);
        if !(scale > 0.0 && scale <= 16.0) {
            return Err(format!("{at}: scale must be above 0 and at most 16"));
        }
        let (width, height) = match p.scale_type {
            ScaleType::Absolute => {
                let (w, h) = (p.width.unwrap_or(0), p.height.unwrap_or(0));
                if !(1..=MAX_SIDE).contains(&w) || !(1..=MAX_SIDE).contains(&h) {
                    return Err(format!(
                        "{at}: scale_type absolute needs width and height between 1 and {MAX_SIDE}"
                    ));
                }
                (w, h)
            }
            _ => (0, 0),
        };
        let aux = match p.aux.as_deref() {
            None | Some("") | Some("none") => Aux::None,
            Some(text) => match text
                .strip_prefix("pass")
                .and_then(|n| n.parse::<usize>().ok())
            {
                Some(n) if n < index => Aux::Pass(n),
                Some(n) => {
                    return Err(format!("{at}: aux \"pass{n}\" is not an earlier pass"));
                }
                None => match textures.iter().position(|t| t.name == text) {
                    Some(i) => Aux::Texture(i),
                    None => {
                        return Err(format!(
                            "{at}: aux \"{text}\" is neither passN nor a [[texture]] name"
                        ));
                    }
                },
            },
        };
        passes.push(ResolvedPass {
            wgsl,
            scale_type: p.scale_type,
            scale,
            width,
            height,
            filter: p.filter,
            aux,
        });
    }
    Ok(ResolvedPreset {
        name: file.name.unwrap_or_else(|| origin.to_string()),
        passes,
        textures,
    })
}

/// The bundled preset or the file `selector` names (`None` is `passthrough`).
pub(crate) fn resolve(selector: Option<&str>) -> Result<ResolvedPreset, String> {
    let selector = selector.unwrap_or("passthrough");
    if selector == "crt-bloom" {
        return resolve_preset(CRT_BLOOM_PRESET, None, "crt-bloom");
    }
    if let Some(text) = bundled_shader(selector) {
        return Ok(ResolvedPreset::single(selector, text.to_string()));
    }
    let path = Path::new(selector);
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("--shader: failed to read {selector}: {e}"))?;
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
    {
        resolve_preset(&text, path.parent(), selector)
    } else {
        Ok(ResolvedPreset::single(selector, text))
    }
}

/// The bundled preset `delta` places from `current` (wrapping). A custom path or `None` counts as `passthrough`, so the first
/// press from either lands on a real, named shader.
pub(crate) fn cycle(current: Option<&str>, delta: i32) -> &'static str {
    let index = BUNDLED
        .iter()
        .position(|name| Some(*name) == current)
        .unwrap_or(0) as i32;
    BUNDLED[(index + delta).rem_euclid(BUNDLED.len() as i32) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycling_walks_the_bundled_list_and_wraps() {
        assert_eq!(cycle(Some("passthrough"), 1), "crt");
        assert_eq!(cycle(Some("crt-bloom"), 1), "passthrough");
        assert_eq!(cycle(Some("passthrough"), -1), "crt-bloom");
        assert_eq!(cycle(Some("crt"), -1), "passthrough");
        // No shader chosen / a custom file: start from the beginning.
        assert_eq!(cycle(None, 1), "crt");
        assert_eq!(cycle(Some("my/custom.wgsl"), 1), "crt");
    }

    #[test]
    fn every_bundled_name_resolves_to_a_preset() {
        for name in BUNDLED {
            let preset = resolve(Some(name)).unwrap();
            assert!(!preset.passes.is_empty(), "{name}");
            assert!(preset.passes.iter().all(|p| !p.wgsl.is_empty()), "{name}");
        }
    }

    #[test]
    fn the_single_pass_shaders_are_one_pass() {
        for name in &BUNDLED[..7] {
            assert_eq!(resolve(Some(name)).unwrap().passes.len(), 1, "{name}");
        }
        assert_eq!(resolve(None).unwrap().name, "passthrough");
    }

    #[test]
    fn crt_bloom_is_a_chain_that_ends_on_the_picture() {
        let preset = resolve(Some("crt-bloom")).unwrap();
        assert_eq!(preset.passes.len(), 4);
        assert_eq!(preset.passes[0].scale, 0.5);
        assert_eq!(preset.passes[3].scale_type, ScaleType::Source);
    }

    #[test]
    fn every_pass_shader_declares_the_shared_bindings() {
        // each shader reads the params and the input texture; the multi-pass ones also the original
        for name in ["bloom-prefilter", "blur-h", "blur-v", "crt-bloom"] {
            let text = bundled_shader(name).unwrap();
            assert!(text.contains("@binding(0) var<uniform> params"), "{name}");
            assert!(text.contains("@binding(1)"), "{name}");
        }
        assert!(
            CRT_BLOOM.contains("@binding(3)"),
            "the composite reads the original"
        );
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("retroclawd-preset-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_preset_file_resolves_relative_to_itself() {
        let dir = temp_dir("rel");
        std::fs::write(dir.join("mine.wgsl"), "// mine").unwrap();
        std::fs::write(
            dir.join("p.toml"),
            "name = \"mine\"\n[[pass]]\nshader = \"mine.wgsl\"\nscale = 2.0\nfilter = \"nearest\"\n\
             [[pass]]\nshader = \"crt\"\naux = \"pass0\"\n",
        )
        .unwrap();
        let preset = resolve(Some(dir.join("p.toml").to_str().unwrap())).unwrap();
        assert_eq!(preset.name, "mine");
        assert_eq!(preset.passes[0].wgsl, "// mine");
        assert_eq!(preset.passes[0].scale, 2.0);
        assert_eq!(preset.passes[0].filter, Filter::Nearest);
        assert_eq!(preset.passes[1].aux, Aux::Pass(0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_wgsl_path_is_a_one_pass_preset() {
        let dir = temp_dir("wgsl");
        let path = dir.join("custom.wgsl");
        std::fs::write(&path, "// custom").unwrap();
        let preset = resolve(Some(path.to_str().unwrap())).unwrap();
        assert_eq!(preset.passes.len(), 1);
        assert_eq!(preset.passes[0].wgsl, "// custom");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_presets_say_what_is_wrong() {
        let bad = |text: &str| resolve_preset(text, None, "t.toml").unwrap_err();
        assert!(bad("").contains("at least one"));
        assert!(bad("[[pass]]\nshader = \"crt\"\nscale = 0.0").contains("scale"));
        assert!(
            bad("[[pass]]\nshader = \"crt\"\nscale_type = \"absolute\"")
                .contains("width and height")
        );
        assert!(bad("[[pass]]\nshader = \"crt\"\naux = \"pass0\"").contains("earlier pass"));
        assert!(bad("[[pass]]\nshader = \"crt\"\naux = \"mask\"").contains("mask"));
        assert!(bad("[[pass]]\nshader = \"no/such.wgsl\"").contains("no/such.wgsl"));
        assert!(bad("[[pass]]\nshader = \"crt\"\ncolour = 1").contains("colour"));
        assert!(
            bad("[[texture]]\nname = \"m\"\npath = \"no/such.png\"\n[[pass]]\nshader = \"crt\"")
                .contains("no/such.png")
        );
    }

    #[test]
    fn an_unknown_name_is_treated_as_a_path_and_fails_to_read() {
        let err = resolve(Some("not-a-real-shader")).unwrap_err();
        assert!(err.contains("not-a-real-shader"));
    }
}
