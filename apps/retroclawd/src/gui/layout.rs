//! Window and picture geometry: which size the window opens at, the shape of the picture and where it sits inside the window
//! (and inside a bezel, when one is shown). Pure arithmetic on `Vec2`, no ECS, so every rule has a unit test.
//!
//! The picture always has the shape of a television, 4:3, however many pixels the machine draws (a 256x192 or a 320x224 picture is
//! stretched to it), unless the machine reports a non-square pixel aspect, which gives its own shape (`display_aspect`).

use bevy::math::Vec2;

/// The shape of a television picture.
pub(crate) const TV_ASPECT: f32 = 4.0 / 3.0;

/// The shape a window takes when a bezel is shown (the bezel image is made for a 16:9 screen).
pub(crate) const BEZEL_WINDOW_ASPECT: f32 = 16.0 / 9.0;

/// The start-up window sizes, tried in this order; the first that fits the monitor is used. All are 4:3.
pub(crate) const LADDER: [(u32, u32); 5] = [
    (1440, 1080),
    (1280, 960),
    (1024, 768),
    (800, 600),
    (640, 480),
];

/// A window may take this share of the monitor's height and width at start-up (room for the task bar and the title bar).
const MONITOR_FRACTION: f32 = 0.9;

/// The shape of the picture: 4:3, unless the machine reports a pixel aspect other than square, which gives the shape of
/// `width * pixel_aspect : height`.
pub(crate) fn display_aspect(width: u32, height: u32, pixel_aspect: (u16, u16)) -> f32 {
    let (num, den) = pixel_aspect;
    if num == 0 || den == 0 || num == den || width == 0 || height == 0 {
        return TV_ASPECT;
    }
    width as f32 * f32::from(num) / f32::from(den) / height as f32
}

/// Parses `1440x1080` (also `1440X1080` or `1440*1080`) into a window size.
pub(crate) fn parse_window_size(text: &str) -> Result<(u32, u32), String> {
    let bad = || format!("window size \"{text}\": expected WIDTHxHEIGHT, for example 1440x1080");
    let (w, h) = text.split_once(['x', 'X', '*']).ok_or_else(bad)?;
    let w: u32 = w.trim().parse().map_err(|_| bad())?;
    let h: u32 = h.trim().parse().map_err(|_| bad())?;
    if !(160..=16384).contains(&w) || !(120..=16384).contains(&h) {
        return Err(format!(
            "window size \"{text}\": each side must be between 160 and 16384"
        ));
    }
    Ok((w, h))
}

/// The largest box of shape `aspect` (width / height) that fits inside `area`.
pub(crate) fn fit_in(area: Vec2, aspect: f32) -> Vec2 {
    if area.x <= 0.0 || area.y <= 0.0 || aspect <= 0.0 {
        return Vec2::ZERO;
    }
    if area.x / area.y > aspect {
        Vec2::new(area.y * aspect, area.y)
    } else {
        Vec2::new(area.x, area.x / aspect)
    }
}

/// The window size (logical pixels) the window opens at.
///
/// `requested` (the user's own `--window-size`/`window_size`) is taken as it is. Otherwise the first size of `LADDER` whose
/// window fits `monitor` (the monitor's logical size) is used: 1440x1080, then 1280x960, 1024x768, 800x600, and 640x480 when
/// nothing fits. The ladder entry gives the height; the width follows the picture's shape, or the bezel's (16:9) when a bezel
/// is shown, so the picture is never squeezed to make the bezel fit.
pub(crate) fn startup_window(
    monitor: Vec2,
    aspect: f32,
    bezel: bool,
    requested: Option<(u32, u32)>,
) -> Vec2 {
    if let Some((w, h)) = requested {
        return Vec2::new(w as f32, h as f32);
    }
    let window_aspect = if bezel { BEZEL_WINDOW_ASPECT } else { aspect };
    let limit = monitor * MONITOR_FRACTION;
    for (_, h) in LADDER {
        let size = Vec2::new((h as f32 * window_aspect).round(), h as f32);
        if size.x <= limit.x && size.y <= limit.y {
            return size;
        }
    }
    let (_, h) = LADDER[LADDER.len() - 1];
    Vec2::new((h as f32 * window_aspect).round(), h as f32)
}

/// A bezel image's shape and where its transparent screen sits, so the picture can be put behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BezelGeometry {
    /// Image width / height.
    pub(crate) aspect: f32,
    /// The screen hole as fractions of the image, `[left, top, right, bottom]`, from the top left corner.
    pub(crate) hole: [f32; 4],
}

/// Where everything goes, in window coordinates (origin at the window's centre, y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub(crate) picture_center: Vec2,
    pub(crate) picture_size: Vec2,
    /// The bezel's size (it is centred), when one is shown.
    pub(crate) bezel_size: Option<Vec2>,
}

/// Places the picture in a window of `window` logical pixels: the largest picture of shape `aspect` that fits, centred, or the
/// largest that fits the bezel's hole, with the bezel itself as large as the window allows.
pub(crate) fn place(window: Vec2, aspect: f32, bezel: Option<BezelGeometry>) -> Placement {
    let Some(bezel) = bezel else {
        return Placement {
            picture_center: Vec2::ZERO,
            picture_size: fit_in(window, aspect),
            bezel_size: None,
        };
    };
    let size = fit_in(window, bezel.aspect);
    let [left, top, right, bottom] = bezel.hole;
    let hole = Vec2::new((right - left) * size.x, (bottom - top) * size.y);
    let center = Vec2::new(
        ((left + right) / 2.0 - 0.5) * size.x,
        -((top + bottom) / 2.0 - 0.5) * size.y,
    );
    Placement {
        picture_center: center,
        picture_size: fit_in(hole, aspect),
        bezel_size: Some(size),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEZEL: BezelGeometry = BezelGeometry {
        aspect: 16.0 / 9.0,
        hole: [0.25, 0.1, 0.75, 0.9],
    };

    #[test]
    fn the_picture_is_four_by_three_whatever_the_machine_draws() {
        assert_eq!(display_aspect(256, 192, (1, 1)), TV_ASPECT);
        assert_eq!(display_aspect(320, 224, (0, 0)), TV_ASPECT);
        assert_eq!(display_aspect(384, 272, (1, 1)), TV_ASPECT);
    }

    #[test]
    fn a_machine_with_a_pixel_aspect_gives_its_own_shape() {
        // 320 wide pixels 2:1 over 200 lines is 640:200.
        assert_eq!(display_aspect(320, 200, (2, 1)), 3.2);
        assert_eq!(display_aspect(0, 200, (2, 1)), TV_ASPECT);
    }

    #[test]
    fn window_sizes_parse() {
        assert_eq!(parse_window_size("1440x1080"), Ok((1440, 1080)));
        assert_eq!(parse_window_size("800X600"), Ok((800, 600)));
        assert_eq!(parse_window_size(" 640 * 480 "), Ok((640, 480)));
        for bad in ["", "1440", "axb", "1440x", "10x10", "99999x99999", "-1x5"] {
            assert!(parse_window_size(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn fitting_keeps_the_shape() {
        assert_eq!(
            fit_in(Vec2::new(1920.0, 1080.0), TV_ASPECT),
            Vec2::new(1440.0, 1080.0)
        );
        assert_eq!(
            fit_in(Vec2::new(800.0, 1000.0), TV_ASPECT),
            Vec2::new(800.0, 600.0)
        );
        assert_eq!(fit_in(Vec2::ZERO, TV_ASPECT), Vec2::ZERO);
    }

    #[test]
    fn the_ladder_picks_the_first_size_that_fits() {
        let at = |w, h| startup_window(Vec2::new(w, h), TV_ASPECT, false, None);
        // a 4k monitor at 150 % is 2560x1440 logical
        assert_eq!(at(2560.0, 1440.0), Vec2::new(1440.0, 1080.0));
        assert_eq!(at(3840.0, 2160.0), Vec2::new(1440.0, 1080.0));
        // 1080p: 1080 is more than 90 % of it
        assert_eq!(at(1920.0, 1080.0), Vec2::new(1280.0, 960.0));
        assert_eq!(at(1366.0, 768.0), Vec2::new(800.0, 600.0)); // 90 % of 768 is 691
        assert_eq!(at(1280.0, 800.0), Vec2::new(800.0, 600.0)); // 720 high allowed
        assert_eq!(at(1024.0, 768.0), Vec2::new(800.0, 600.0));
        assert_eq!(at(800.0, 600.0), Vec2::new(640.0, 480.0)); // 540 allowed
        // nothing fits: the smallest anyway
        assert_eq!(at(320.0, 240.0), Vec2::new(640.0, 480.0));
    }

    #[test]
    fn every_ladder_size_is_four_by_three() {
        for (w, h) in LADDER {
            assert_eq!(w * 3, h * 4, "{w}x{h}");
        }
    }

    #[test]
    fn a_requested_size_is_taken_as_it_is() {
        let size = startup_window(
            Vec2::new(800.0, 600.0),
            TV_ASPECT,
            false,
            Some((1920, 1080)),
        );
        assert_eq!(size, Vec2::new(1920.0, 1080.0));
    }

    #[test]
    fn a_bezel_makes_the_window_sixteen_by_nine() {
        let size = startup_window(Vec2::new(2560.0, 1440.0), TV_ASPECT, true, None);
        assert_eq!(size, Vec2::new(1920.0, 1080.0));
        // without a bezel the shape follows the picture
        let other = startup_window(Vec2::new(7680.0, 4320.0), 3.2, false, None);
        assert_eq!(other, Vec2::new(3456.0, 1080.0));
    }

    #[test]
    fn without_a_bezel_the_picture_is_centred_with_bars() {
        let p = place(Vec2::new(3840.0, 2160.0), TV_ASPECT, None);
        assert_eq!(p.picture_size, Vec2::new(2880.0, 2160.0));
        assert_eq!(p.picture_center, Vec2::ZERO);
        assert_eq!(p.bezel_size, None);
    }

    #[test]
    fn the_picture_sits_in_the_bezel_hole() {
        let p = place(Vec2::new(1920.0, 1080.0), TV_ASPECT, Some(BEZEL));
        assert_eq!(p.bezel_size, Some(Vec2::new(1920.0, 1080.0)));
        // the hole is 960 x 864; a 4:3 picture fits 960 x 720
        assert_eq!(p.picture_size, Vec2::new(960.0, 720.0));
        assert_eq!(p.picture_center, Vec2::ZERO);
    }

    #[test]
    fn a_bezel_keeps_its_shape_in_a_wider_window() {
        let p = place(Vec2::new(3440.0, 1440.0), TV_ASPECT, Some(BEZEL));
        assert_eq!(p.bezel_size, Some(Vec2::new(2560.0, 1440.0)));
        assert!((p.picture_size.x / p.picture_size.y - TV_ASPECT).abs() < 1e-4);
    }

    #[test]
    fn an_off_centre_hole_moves_the_picture() {
        let geo = BezelGeometry {
            aspect: 2.0,
            hole: [0.0, 0.0, 0.5, 1.0],
        };
        let p = place(Vec2::new(2000.0, 1000.0), 1.0, Some(geo));
        // the hole is the left half, 1000 x 1000, centred at x = -500
        assert_eq!(p.picture_center, Vec2::new(-500.0, 0.0));
        assert_eq!(p.picture_size, Vec2::new(1000.0, 1000.0));
    }
}
