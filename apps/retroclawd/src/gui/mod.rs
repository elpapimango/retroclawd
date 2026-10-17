use std::time::{Duration, Instant};

use bevy::audio::AddAudioSource;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, Window, WindowMode, WindowPlugin};

use crate::emu_thread;
use crate::plugin_bridge::HostedMachine;

mod audio;
mod debug_overlay;
mod fold;
pub(crate) mod hotkeys;
mod input;
pub(crate) mod keys;
pub(crate) mod pads;
pub(crate) mod paste;
mod prompt;
pub(crate) mod shader;
mod toast;
mod video;
mod window;

use audio::{AudioControlRes, AudioQueueRes, RequestedCountRes, UnderrunCountRes};
use debug_overlay::DebugSnapshotRes;
use hotkeys::HotkeyState;
use input::{AutoRunState, CommandTxRes, InputTxRes, MouseTxRes};
use video::{BaseResolution, FrameBufferRes, ShaderSettingsRes};
use window::WindowScalePercentRes;

use crate::AppSettings;

pub(crate) use debug_overlay::OverlayPanels;
pub(crate) use input::AutoRunProfile;

#[derive(Resource)]
struct GuiFpsLimiter {
    target: Option<Duration>,
    max_fps: Option<u32>,
    last_frame: Instant,
    diag_start: Instant,
    diag_frames: u32,
}

impl GuiFpsLimiter {
    fn new(max_fps: Option<u32>) -> Self {
        let now = Instant::now();
        Self {
            target: max_fps.map(|fps| Duration::from_secs_f64(1.0 / f64::from(fps))),
            max_fps,
            last_frame: now,
            diag_start: now,
            diag_frames: 0,
        }
    }
}

fn limit_gui_fps(mut limiter: ResMut<GuiFpsLimiter>) {
    if let Some(target) = limiter.target {
        let elapsed = limiter.last_frame.elapsed();
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }
    limiter.last_frame = Instant::now();

    if let Some(max_fps) = limiter.max_fps {
        limiter.diag_frames = limiter.diag_frames.saturating_add(1);
        let elapsed = limiter.diag_start.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let observed = f64::from(limiter.diag_frames) / elapsed.as_secs_f64();
            tracing::info!("gui limiter: target={max_fps} observed/s={observed:.1}");
            limiter.diag_start = Instant::now();
            limiter.diag_frames = 0;
        }
    }
}

fn log_window_close_requests(
    mut close_requested: MessageReader<bevy::window::WindowCloseRequested>,
    mut closing: MessageReader<bevy::window::WindowClosing>,
    mut closed: MessageReader<bevy::window::WindowClosed>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    for ev in close_requested.read() {
        tracing::warn!(
            "window close requested: window={:?} esc={} f4={} alt_l={} alt_r={}",
            ev.window,
            keys.pressed(KeyCode::Escape),
            keys.pressed(KeyCode::F4),
            keys.pressed(KeyCode::AltLeft),
            keys.pressed(KeyCode::AltRight)
        );
    }
    for ev in closing.read() {
        tracing::info!("window closing: window={:?}", ev.window);
    }
    for ev in closed.read() {
        tracing::info!("window closed: window={:?}", ev.window);
    }
}

/// Opens a live window showing `machine`'s video output and driving its
/// keyboard from window input. Blocks until the window is closed -- same
/// "owns the whole run" shape as `Machine::run()`, just Bevy's own loop
/// instead. Panics if `machine.render_frame()` is `None` -- callers must
/// check that before calling this (see `main.rs`'s GUI branch, which
/// checks up front with the same error message `--dump-screen` uses for
/// this case).
///
/// CPU stepping, audio generation, and frame production all happen on a
/// dedicated background thread (`emu_thread::spawn`), not on this Bevy
/// app's own thread -- a stall anywhere in Bevy's render/window handling
/// (a vsync wait, a slow present) used to also stall audio generation,
/// since both ran on the same thread; moving the machine off this thread
/// entirely means a render-side stall can no longer starve audio. See
/// `docs/superpowers/specs/2026-09-09-emu-thread-tracy-design.md`.
///
/// `overlay_panels` turns on a semi-transparent overlay in the top-left
/// corner when `Some`, updated every `DEBUG_OVERLAY_INTERVAL`. Which
/// panels it shows (`cpu`/`memory`/`disk`/`tape`) comes from the
/// `--show-overlay` (`-so`) CLI flag's own value -- see
/// `debug_overlay::OverlayPanels`'s own doc comment. A CLI flag rather
/// than a runtime keyboard toggle deliberately: every key on this window
/// drives the emulated machine's own keyboard (see
/// `input::keyboard_input`), so there's no key left free to reserve for
/// a toggle without either stealing it from the emulated keyboard or
/// picking one that collides with a real ZX81/Spectrum key.
///
/// `start_fullscreen` opens the window already in `BorderlessFullscreen`
/// mode (at the current monitor's own resolution) instead of windowed.
/// Either way, F11 toggles fullscreen live (see `window::toggle_fullscreen`),
/// and F12 resets the emulated machine (see `input::reset_hotkey`) -- F1/F3/
/// F5/F7 are real C64 function keys (`c64-machine-plugin`), but F11/F12
/// are free on every machine this app supports, so neither collides with
/// an emulated key the way a toggle on, say, a letter key would.
///
/// `no_vsync` selects `AutoNoVsync` instead of the default `AutoVsync`
/// present mode. This is an explicit opt-in escape hatch for drivers/
/// compositors that quantize VSync to a low divisor despite low emulator
/// load.
///
/// `max_fps` caps the GUI update/render loop rate when present. `main`
/// resolves `--no-vsync` + no explicit `--max-fps` to a 60 FPS preset,
/// so this parameter is usually `Some(60)` in that mode. With VSync on,
/// display pacing still applies on top of this cap.
///
/// `run_ahead_frames` is the `--run-ahead` input-latency-reduction knob
/// (`0` disables it, the default), forwarded straight through to
/// `emu_thread::spawn` -- see its own doc comment for the mechanism.
///
/// `auto_run_profile` is an optional startup key script (`--run`) injected
/// through the same input channel as real keyboard events (today: CPC tape
/// boot helper + delayed follow-up "any key" pulse, or `RUN"DISC"` for
/// disk), after a short firmware-boot delay.
///
/// `debugger_plugins` lists debugger plugin selectors/options requested by
/// `--plugin` CLI arguments. When non-empty, `run_gui` asks the host bridge
/// to load/start every selected debugger plugin concurrently.
/// `app_settings` carries the already-resolved `--shader`/`--config`
/// settings (see `crate::AppSettings`'s own doc comment) -- shader source
/// text is resolved before this function is ever called, so a bad custom
/// `--shader <path>` fails cleanly in `main` before any window opens.
// Every parameter here is independently meaningful and documented above;
// bundling them into a settings struct would just move the same count of
// fields one level down without reducing real wiring complexity.
#[allow(clippy::too_many_arguments)]
pub fn run_gui(
    machine: HostedMachine,
    machine_id: &str,
    overlay_panels: Option<OverlayPanels>,
    start_fullscreen: bool,
    no_vsync: bool,
    max_fps: Option<u32>,
    run_ahead_frames: u32,
    auto_run_profile: Option<AutoRunProfile>,
    debugger_plugins: Vec<crate::debuggers::LaunchSelection>,
    app_settings: AppSettings,
) {
    let (width, height, _) = machine
        .render_frame()
        .expect("caller must check render_frame().is_some() before calling run_gui");
    tracing::info!(
        "run_gui: {width}x{height} @ {}Hz, clock_hz={}",
        machine.frame_hz(),
        machine.clock_hz()
    );

    // `MonitorSelection::Current` only resolves relative to an existing
    // window (used by `window::toggle_fullscreen`, where one already
    // exists) -- at initial window creation there's no window yet to be
    // "current" relative to, so this needs `Primary` instead (confirmed
    // live: using `Current` here logs "Can't select current monitor on
    // window creation").
    let mode = if start_fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Primary)
    } else {
        WindowMode::Windowed
    };
    let present_mode = if no_vsync {
        bevy::window::PresentMode::AutoNoVsync
    } else {
        bevy::window::PresentMode::AutoVsync
    };

    // Any debugger selector in `--plugin` enables emu-thread diagnostics, and
    // every selector -- `debug-log` included -- is launched through the
    // plugin bridge. (An earlier version filtered `debug-log` out on the
    // assumption `main.rs` installed a host-side tracing sink for it; that
    // sink was removed, so the plugin never started and `--plugin
    // debug-log:...` produced neither a file nor stdout output.)
    let debugger_enabled = !debugger_plugins.is_empty();
    let debugger_plugin_launches = debugger_plugins;

    // Moves `machine` onto its own thread -- see `emu_thread`'s own doc
    // comment. `handle` is a plain local, not an ECS resource: `app.run()`
    // below blocks until the window closes, and dropping `handle` at the
    // end of this function (after `app.run()` returns) is what stops and
    // joins the thread -- see `EmuThreadHandle`'s `Drop` impl.
    let mut overlay_state = debug_overlay::OverlayStateRes::new(
        overlay_panels,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    overlay_state.version_text = crate::version::overlay_text(
        machine_id,
        crate::plugin_bridge::startup_report().ok().as_ref(),
    );
    let handle = emu_thread::spawn(
        machine,
        machine_id,
        run_ahead_frames,
        debugger_enabled,
        overlay_state.data_wanted_flag(),
        app_settings.media.clone(),
    );
    // From here on a download (an `.m3u` disk swap to a URL) shows a toast.
    crate::inputs::set_toast_target(handle.messages.clone());
    let audio_queue = handle.audio_queue.clone();
    let frame_buffer = handle.frame_buffer.clone();
    let debug_snapshot = handle.debug_snapshot.clone();
    let underrun_count = handle.underrun_count.clone();
    let requested_count = handle.requested_count.clone();
    let input_tx = handle.input_tx.clone();
    let command_tx = handle.command_tx.clone();
    let mouse_tx = handle.mouse_tx.clone();
    let (prompt_tx, prompt_rx) = std::sync::mpsc::channel();
    let file_dialogs = crate::dialogs::FileDialogs::new(
        app_settings.dialogs.clone(),
        command_tx.clone(),
        handle.messages.clone(),
        prompt_tx,
    );

    // Declared after `handle` so it drops (and stops accepting new debugger
    // commands) BEFORE the emu thread that would service them goes away --
    // Rust drops locals in reverse declaration order. Named with a leading
    // underscore since nothing reads it again -- it's held purely for this
    // drop-order side effect.
    let _debugger_plugin_handles = if debugger_plugin_launches.is_empty() {
        Vec::new()
    } else {
        match crate::debuggers::start_debuggers(
            crate::debuggers::LaunchOptions {
                selections: debugger_plugin_launches,
            },
            handle
                .debugger_commands
                .clone()
                .expect("debugger_commands set when debugger plugins are enabled"),
            handle
                .debugger_state
                .clone()
                .expect("debugger_state set when debugger plugins are enabled"),
            machine_id.to_string(),
        ) {
            Ok(plugins) => {
                for plugin in &plugins {
                    tracing::info!(
                        target: "retroclawd.debugger",
                        "debugger started: selector={}",
                        plugin.selector
                    );
                    if let Some(url) = &plugin.web_url {
                        tracing::info!(target: "retroclawd.debugger", "debugger UI: {url}");
                    }
                    if let Some(addr) = &plugin.dezog_addr {
                        tracing::info!(target: "retroclawd.debugger", "debugger TCP: {addr}");
                    }
                }
                plugins
            }
            Err(e) => {
                eprintln!("failed to start debugger: {e}");
                panic!("a debugger was requested but could not be started");
            }
        }
    };

    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins
            .build()
            .disable::<bevy::log::LogPlugin>()
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "retroclawd".to_string(),
                    resolution: (
                        (width as f32 * video::INITIAL_SCALE) as u32,
                        (height as f32 * video::INITIAL_SCALE) as u32,
                    )
                        .into(),
                    mode,
                    present_mode,
                    ..default()
                }),
                ..default()
            }),
        bevy::sprite_render::Material2dPlugin::<video::ScreenMaterial>::default(),
    ))
    .insert_resource(ClearColor(Color::BLACK))
    .insert_resource(BaseResolution { width, height })
    .insert_resource(AudioQueueRes(audio_queue))
    .insert_resource(UnderrunCountRes(underrun_count))
    .insert_resource(RequestedCountRes(requested_count))
    .insert_resource(FrameBufferRes(frame_buffer))
    .insert_resource(DebugSnapshotRes(debug_snapshot))
    .insert_resource(InputTxRes(input_tx))
    .insert_resource(CommandTxRes(command_tx))
    .insert_resource(AudioControlRes::default())
    .insert_resource(overlay_state)
    .insert_resource(toast::ToastRes::new(handle.messages.clone()))
    .insert_resource(app_settings.hotkeys.clone())
    .insert_resource(HotkeyState::default())
    .insert_resource(hotkeys::PadTriggers::default())
    .insert_resource(
        keys::KeyRouter::new(app_settings.key_bindings.clone())
            .with_layout(app_settings.layout, app_settings.symbols.clone()),
    )
    .insert_resource(keys::LogicalKeys::default())
    .insert_resource(paste::PasteState::new(
        app_settings.paste_case,
        app_settings.paste_delay_ms,
    ))
    .insert_resource(pads::PadRouter::new(app_settings.pad_bindings.clone()))
    .insert_resource(pads::PadSlots::default())
    .insert_resource(file_dialogs)
    .insert_resource(prompt::PromptRes::new(prompt_rx))
    .insert_resource(MouseTxRes(mouse_tx))
    .insert_resource(ShaderSettingsRes {
        source: app_settings.shader_source,
        name: app_settings.shader_name,
        shader_intensity: app_settings.shader_intensity,
        scanline_intensity: app_settings.scanline_intensity,
    })
    .insert_resource(WindowScalePercentRes(app_settings.window_scale_percent))
    .insert_resource(GuiFpsLimiter::new(max_fps));

    if let Some(profile) = auto_run_profile {
        app.insert_resource(AutoRunState::new(profile));
    }

    app.add_audio_source::<audio::EmulatorAudio>()
        .add_systems(
            Startup,
            (
                video::setup,
                audio::setup_audio,
                toast::setup_toast,
                debug_overlay::setup_debug_overlay,
            ),
        )
        .add_systems(PreUpdate, limit_gui_fps)
        .add_systems(
            Update,
            (
                log_window_close_requests,
                video::render_screen,
                input::hotkey_input,
                input::record_logical_keys.before(input::keyboard_input),
                input::keyboard_input.after(input::hotkey_input),
                input::pad_input.after(input::keyboard_input),
                prompt::prompt_input,
                input::mouse_input,
                paste::paste_typing,
                toast::update_toast,
                debug_overlay::update_debug_overlay,
                input::auto_run_startup_sequence,
                video::fit_screen,
                window::set_window_icon,
                window::apply_initial_window_size.run_if(move || !start_fullscreen),
            ),
        );

    app.run();
    // `handle` drops here: stops the emu thread and joins it before this
    // function returns.
}
