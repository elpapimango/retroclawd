//! Bezels: a picture frame drawn over the window's edges, like the overlays of RetroArch. A bezel is an image with a
//! transparent hole where the emulated screen shows; the picture is put behind the hole (`layout::place`).
//!
//! `bezel = "default"` is a television frame drawn by `default_bezel` (no file is shipped); `bezel = "path/to/file.png"` is any
//! PNG with a transparent screen. The hole is found from the image: the transparent run through its centre, along the middle
//! row and the middle column.

use super::layout::BezelGeometry;

/// Which bezel to show.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) enum BezelSource {
    #[default]
    Off,
    Default,
    File(String),
}

impl BezelSource {
    /// `off` / `none` / `false`, `default` / `on` / `true`, or a PNG path.
    pub(crate) fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "" | "off" | "none" | "false" | "no" => Self::Off,
            "default" | "on" | "true" | "yes" => Self::Default,
            _ => Self::File(text.trim().to_string()),
        }
    }
}

/// A decoded bezel: RGBA bytes and where its screen is.
#[derive(Clone, Debug)]
pub(crate) struct LoadedBezel {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
    pub(crate) geometry: BezelGeometry,
}

/// Pixels with less alpha than this are the screen hole.
const HOLE_ALPHA: u8 = 8;

/// Loads the bezel `source` names (nothing for `Off`).
pub(crate) fn load(source: &BezelSource) -> Result<Option<LoadedBezel>, String> {
    match source {
        BezelSource::Off => Ok(None),
        BezelSource::Default => Ok(Some(default_bezel())),
        BezelSource::File(path) => {
            let (width, height, rgba) = decode_png(
                &std::fs::read(path).map_err(|e| format!("bezel: failed to read {path}: {e}"))?,
            )
            .map_err(|e| format!("bezel {path}: {e}"))?;
            let geometry = find_hole(width, height, &rgba).ok_or_else(|| {
                format!("bezel {path}: no transparent screen hole in the middle of the image")
            })?;
            Ok(Some(LoadedBezel {
                width,
                height,
                rgba,
                geometry,
            }))
        }
    }
}

/// Decodes a PNG into width, height and RGBA bytes (8 bits a channel, whatever the file's colour type).
pub(crate) fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("not a PNG: {e}"))?;
    let size = reader
        .output_buffer_size()
        .ok_or("the image is too large")?;
    let mut buf = vec![0u8; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("bad PNG: {e}"))?;
    buf.truncate(info.buffer_size());
    let pixels = info.width as usize * info.height as usize;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("an indexed PNG was not expanded".to_string()),
    };
    if rgba.len() != pixels * 4 {
        return Err("the PNG's data does not match its size".to_string());
    }
    Ok((info.width, info.height, rgba))
}

/// The hole of a bezel image: the transparent run through the centre, along the middle row and the middle column.
pub(crate) fn find_hole(width: u32, height: u32, rgba: &[u8]) -> Option<BezelGeometry> {
    if width < 2 || height < 2 || rgba.len() != width as usize * height as usize * 4 {
        return None;
    }
    let clear =
        |x: u32, y: u32| rgba[(y as usize * width as usize + x as usize) * 4 + 3] < HOLE_ALPHA;
    let (cx, cy) = (width / 2, height / 2);
    if !clear(cx, cy) {
        return None;
    }
    let mut left = cx;
    while left > 0 && clear(left - 1, cy) {
        left -= 1;
    }
    let mut right = cx;
    while right + 1 < width && clear(right + 1, cy) {
        right += 1;
    }
    let mut top = cy;
    while top > 0 && clear(cx, top - 1) {
        top -= 1;
    }
    let mut bottom = cy;
    while bottom + 1 < height && clear(cx, bottom + 1) {
        bottom += 1;
    }
    let (w, h) = (width as f32, height as f32);
    Some(BezelGeometry {
        aspect: w / h,
        hole: [
            left as f32 / w,
            top as f32 / h,
            (right + 1) as f32 / w,
            (bottom + 1) as f32 / h,
        ],
    })
}

/// A television frame: a dark plastic surround with a bevel, round inside corners and a darkening towards the glass. 16:9, with a
/// 4:3 screen hole in the middle.
pub(crate) fn default_bezel() -> LoadedBezel {
    const W: u32 = 1920;
    const H: u32 = 1080;
    // the hole: 4:3, 88 % of the height
    let hole_h = (H as f32 * 0.88).round();
    let hole_w = (hole_h * 4.0 / 3.0).round();
    let (hx, hy) = ((W as f32 - hole_w) / 2.0, (H as f32 - hole_h) / 2.0);
    let radius = 46.0_f32;
    let (half_w, half_h) = (hole_w / 2.0, hole_h / 2.0);
    let (mid_x, mid_y) = (W as f32 / 2.0, H as f32 / 2.0);
    let mut rgba = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            // signed distance to the rounded hole: negative inside
            let dx = (x as f32 + 0.5 - mid_x).abs() - (half_w - radius);
            let dy = (y as f32 + 0.5 - mid_y).abs() - (half_h - radius);
            let outside = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - radius;
            let at = ((y * W + x) * 4) as usize;
            if outside < 0.0 {
                continue; // the screen: transparent
            }
            // a lit bevel on the inside edge, then the plastic, lighter at the top, with a fine vertical sheen
            let bevel = (1.0 - outside / 14.0).clamp(0.0, 1.0);
            let top_light = 1.0 - (y as f32 / H as f32) * 0.6;
            let sheen = 0.5 + 0.5 * ((x as f32 / W as f32) * 12.0).cos();
            let base = 26.0 + 14.0 * top_light + 4.0 * sheen;
            let shade = base * (1.0 - 0.7 * bevel) + 0.0;
            let edge = if outside < 2.5 { 0.0 } else { 1.0 };
            let v = (shade * edge + 6.0 * (1.0 - edge)).clamp(0.0, 255.0) as u8;
            rgba[at] = v;
            rgba[at + 1] = v.saturating_sub(2);
            rgba[at + 2] = v.saturating_sub(5);
            rgba[at + 3] = 255;
        }
    }
    LoadedBezel {
        width: W,
        height: H,
        rgba,
        geometry: BezelGeometry {
            aspect: W as f32 / H as f32,
            hole: [
                hx / W as f32,
                hy / H as f32,
                (hx + hole_w) / W as f32,
                (hy + hole_h) / H as f32,
            ],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_parse() {
        assert_eq!(BezelSource::parse("off"), BezelSource::Off);
        assert_eq!(BezelSource::parse(" None "), BezelSource::Off);
        assert_eq!(BezelSource::parse("Default"), BezelSource::Default);
        assert_eq!(
            BezelSource::parse("bezels/tv.png"),
            BezelSource::File("bezels/tv.png".to_string())
        );
    }

    fn frame(width: u32, height: u32, hole: (u32, u32, u32, u32)) -> Vec<u8> {
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let inside = x >= hole.0 && x < hole.2 && y >= hole.1 && y < hole.3;
                rgba[((y * width + x) * 4 + 3) as usize] = if inside { 0 } else { 255 };
            }
        }
        rgba
    }

    #[test]
    fn the_hole_is_found_from_the_middle() {
        let rgba = frame(200, 100, (50, 10, 150, 90));
        let geo = find_hole(200, 100, &rgba).unwrap();
        assert_eq!(geo.aspect, 2.0);
        assert_eq!(geo.hole, [0.25, 0.1, 0.75, 0.9]);
    }

    #[test]
    fn a_frame_without_a_hole_in_the_middle_is_refused() {
        let rgba = frame(100, 100, (0, 0, 10, 10));
        assert_eq!(find_hole(100, 100, &rgba), None);
        assert_eq!(find_hole(1, 1, &[0, 0, 0, 0]), None);
        assert_eq!(find_hole(10, 10, &[0; 8]), None);
    }

    #[test]
    fn the_default_bezel_has_a_four_by_three_screen_in_the_middle() {
        let bezel = default_bezel();
        assert_eq!(bezel.rgba.len(), (bezel.width * bezel.height * 4) as usize);
        let found = find_hole(bezel.width, bezel.height, &bezel.rgba).unwrap();
        let [l, t, r, b] = found.hole;
        let shape = (r - l) * bezel.width as f32 / ((b - t) * bezel.height as f32);
        assert!((shape - 4.0 / 3.0).abs() < 0.01, "{shape}");
        assert!((l + r - 1.0).abs() < 0.01 && (t + b - 1.0).abs() < 0.01);
        // the frame is opaque at the corner
        assert_eq!(bezel.rgba[3], 255);
    }

    #[test]
    fn a_png_round_trips() {
        let rgba = frame(8, 6, (2, 2, 6, 4));
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 8, 6);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&rgba)
                .unwrap();
        }
        let (w, h, decoded) = decode_png(&bytes).unwrap();
        assert_eq!((w, h), (8, 6));
        assert_eq!(decoded, rgba);
        assert!(decode_png(b"not a png").is_err());
    }

    #[test]
    fn a_missing_file_is_a_clear_error() {
        let err = load(&BezelSource::File("no/such/bezel.png".to_string())).unwrap_err();
        assert!(err.contains("no/such/bezel.png"));
        assert!(load(&BezelSource::Off).unwrap().is_none());
    }
}
