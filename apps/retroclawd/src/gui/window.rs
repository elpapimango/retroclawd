use bevy::prelude::*;
use bevy::window::{
    Monitor, MonitorSelection, PrimaryMonitor, PrimaryWindow, Window, WindowMode, WindowPosition,
};
use bevy::winit::WinitWindows;

use super::video::{self, BaseResolution};

/// `--config`'s `window_scale_percent` (default 60, see
/// `crate::AppSettings`'s own doc comment) -- read once by
/// `apply_initial_window_size`.
#[derive(Resource)]
pub(crate) struct WindowScalePercentRes(pub(crate) u32);

/// Sizes the window to the largest whole-number multiple of the emulated
/// picture's own resolution (`BaseResolution`) that still fits within
/// `WindowScalePercentRes` percent of the primary monitor's physical
/// resolution -- same size for every machine relative to *its own*
/// picture, but each machine's own native aspect ratio (a 48K Spectrum's
/// isn't a CPC's) decides the window's actual shape, once, the first
/// frame a `Monitor` entity actually exists. Deliberately reuses
/// `video::integer_scale` -- the exact same "largest integer scale that
/// fits a bounding box" math `fit_screen` already runs every frame for
/// live resizing -- rather than just clamping width/height to 60%
/// independently: that earlier approach left an aspect-mismatched window
/// with real letterboxing inside it (a monitor's own aspect ratio rarely
/// matches an emulated machine's 4:3-ish picture). Picking an *exact*
/// integer multiple instead means the window's aspect ratio always
/// matches the picture's, and the picture fills it edge to edge with zero
/// letterboxing at startup -- confirmed live, see `TODO.md`'s own entry
/// for this fix. Monitor entities are populated by `bevy_winit` after
/// `Startup`, not during it, same underlying reason (and same
/// retry-via-`Local<bool>`-sentinel fix) as `set_window_icon` above
/// needing `WinitWindows` to exist first.
///
/// Not scheduled at all when `--fullscreen` was given (see `run_gui`'s
/// `.run_if`) -- there's no "60% of the monitor" size to apply when the
/// window opens straight into `BorderlessFullscreen`. This system only
/// ever runs once, at startup; after that, window size is entirely
/// user/window-manager driven, and `video::fit_screen` scales the bitmap
/// to whatever the current window size is.
pub(crate) fn apply_initial_window_size(
    scale_percent: Res<WindowScalePercentRes>,
    base: Res<BaseResolution>,
    monitors: Query<&Monitor, With<PrimaryMonitor>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Ok(monitor) = monitors.single() else {
        return;
    };
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    *done = true;

    let percent = scale_percent.0 as f32 / 100.0;
    // `fit_screen` compares `base_size` (the picture's native *texel*
    // count) against the window's own *logical* size (`Window::width`/
    // `height`, matching Bevy's 2D camera default of
    // 1 world unit == 1 logical pixel) -- so the
    // monitor's physical bounds need converting to logical here too
    // (divide by its own scale factor) before the same comparison is
    // valid, rather than mixing units the way an earlier version of this
    // function did (computed the target size against physical monitor
    // bounds, then wrote it back via `set_physical_resolution` -- correct
    // by coincidence only at `scale_factor == 1.0`, wrong on any real
    // HiDPI display).
    let bounds = monitor.physical_size().as_vec2() / monitor.scale_factor as f32 * percent;
    let base_size = base.display_size();
    let scale = video::integer_scale(bounds, base_size);
    let target = base_size * scale;
    window.resolution.set(target.x, target.y);
    window.position = WindowPosition::Centered(MonitorSelection::Primary);
}

/// Sets the real window-manager icon (title bar / taskbar / alt-tab), which
/// `winresource`'s build-time `.ico` embedding (`build.rs`) doesn't cover --
/// that only sets the icon Explorer shows for the `.exe` file itself before
/// it's running. Needs the raw `winit` crate directly (not just
/// `bevy::winit`, which re-exports `WinitWindows` but not `Icon` --
/// verified against `winit-0.30`'s own source, not guessed) at the same
/// version bevy_winit itself depends on, so the `Icon` type it constructs
/// is the same type `set_window_icon` expects.
///
/// Runs in `Update`, not `Startup`: `bevy_winit`'s actual native window
/// creation happens via its own event-loop-driven runner, not the App's
/// `Startup` schedule, so `WinitWindows` isn't guaranteed to exist yet when
/// `Startup` systems run (confirmed live -- an earlier `Startup`-scheduled
/// version of this panicked with "Non-send data not found"). `done` makes
/// this retry every frame until the window actually exists, then never run
/// its body again.
///
/// NOTE the `include_bytes!` path below is one `../` deeper than it was in
/// the old single-file `gui.rs`, since this file now lives at
/// `apps/retroclawd/src/gui/window.rs` (one directory deeper than
/// `apps/retroclawd/src/gui.rs` was).
pub(crate) fn set_window_icon(
    windows: Option<NonSend<WinitWindows>>,
    primary: Query<Entity, With<PrimaryWindow>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(windows) = windows else {
        return;
    };
    let Ok(entity) = primary.single() else {
        return;
    };
    let Some(winit_window) = windows.get_window(entity) else {
        return;
    };
    *done = true;

    let bytes = include_bytes!("../../../../assets/icon-64.png");
    let Ok(mut reader) = png::Decoder::new(std::io::Cursor::new(bytes.as_slice())).read_info()
    else {
        return;
    };
    let Some(buffer_size) = reader.output_buffer_size() else {
        return;
    };
    let mut buf = vec![0u8; buffer_size];
    let Ok(info) = reader.next_frame(&mut buf) else {
        return;
    };
    let rgba = buf[..info.buffer_size()].to_vec();

    if let Ok(icon) = winit::window::Icon::from_rgba(rgba, info.width, info.height) {
        winit_window.set_window_icon(Some(icon));
    }
}

/// Toggles between windowed and fullscreen (the `fullscreen` hotkey, F11 by
/// default; see `hotkeys`). `BorderlessFullscreen` rather than exclusive
/// `Fullscreen`: it just uses the current monitor's own resolution/refresh
/// rate directly, no separate display-mode negotiation, and `video::fit_screen`
/// is what actually gets the "scale to 4K" vs "scale to 1080p" effect the
/// user wants -- a bigger monitor doesn't force a bigger scale factor, it
/// just leaves more (or less) letterboxed space around whatever integer
/// factor fits.
pub(crate) fn toggle_fullscreen(window: &mut Window) {
    window.mode = match window.mode {
        WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        _ => WindowMode::Windowed,
    };
}
