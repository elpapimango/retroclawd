use bevy::prelude::*;
use bevy::window::{
    Monitor, MonitorSelection, PrimaryMonitor, PrimaryWindow, Window, WindowMode, WindowPosition,
};
use bevy::winit::WinitWindows;

use super::layout;
use super::video::BaseResolution;

/// How the window opens: the size the user asked for (`--window-size`, `window_size`), and whether a bezel is shown (which makes
/// the window 16:9, so the picture is not squeezed to fit it).
#[derive(Resource)]
pub(crate) struct WindowSetupRes {
    pub(crate) requested: Option<(u32, u32)>,
    pub(crate) bezel: bool,
}

/// Sizes and centres the window once a `Monitor` entity exists (`bevy_winit` adds it after `Startup`, so this retries every
/// `Update` until it does, like `set_window_icon`): the size asked for, or the first of `layout::LADDER` (1440x1080, 1280x960,
/// 1024x768, 800x600, 640x480) that fits the monitor, in logical pixels -- on a 4K monitor the first. Not scheduled when the
/// window opens fullscreen. After this the window size is the user's, and `video::layout_screen` fits the picture to it.
pub(crate) fn apply_initial_window_size(
    setup: Res<WindowSetupRes>,
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

    let logical = monitor.physical_size().as_vec2() / monitor.scale_factor as f32;
    let size = layout::startup_window(logical, base.aspect(), setup.bezel, setup.requested);
    window.resolution.set(size.x, size.y);
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

/// Toggles between windowed and fullscreen (the `fullscreen` hotkey, F11 by default; see `hotkeys`). `BorderlessFullscreen`
/// rather than exclusive `Fullscreen`: it uses the current monitor's own resolution and refresh rate, no display-mode
/// negotiation. The picture keeps its 4:3 shape (`video::layout_screen`): the rest of a 16:9 screen is black, or the bezel.
pub(crate) fn toggle_fullscreen(window: &mut Window) {
    window.mode = match window.mode {
        WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        _ => WindowMode::Windowed,
    };
}
