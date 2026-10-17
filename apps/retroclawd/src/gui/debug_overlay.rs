use std::time::Duration;

use bevy::prelude::*;

use crate::emu_thread::DebugSnapshotHandle;

/// Snapshot of the emu thread's machine state (halted flag + registers),
/// written every tick by `emu_thread::tick` -- `update_debug_overlay`
/// reads this instead of the machine directly, since the machine now
/// lives only on the emu thread, not in this ECS world.
#[derive(Resource)]
pub(crate) struct DebugSnapshotRes(pub(crate) DebugSnapshotHandle);

/// Marks the single UI text entity `update_debug_overlay` rewrites.
#[derive(Component)]
pub(crate) struct DebugOverlayText;

/// Which `--show-overlay` panels are enabled -- parsed from the flag's
/// optional colon-separated value (`--show-overlay cpu:mem:tape`), or
/// `cpu`-only when the flag is given with no value at all (clap's
/// `default_missing_value`, see `main.rs`'s own `show_overlay` field).
/// `all` is a shorthand enabling every panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OverlayPanels {
    pub(crate) cpu: bool,
    pub(crate) memory: bool,
    pub(crate) disk: bool,
    pub(crate) tape: bool,
    /// The retroclawd and plugin version, build number and commit (the `version` hotkey flips it).
    pub(crate) version: bool,
}

impl OverlayPanels {
    /// Parses `--show-overlay`'s value. `all` enables every panel; any
    /// other value is a `:`-delimited list of `cpu`/`mem`(or `memory`)/
    /// `disk`/`tape` tokens (case-insensitive; blank tokens from a stray
    /// leading/trailing/doubled `:` are silently skipped, not an error).
    /// An unrecognized token is a clean `Err` -- same "fail before the
    /// window opens" convention `--shader`'s own path validation already
    /// uses (see `main.rs`'s call site).
    pub(crate) fn parse(spec: &str) -> Result<Self, String> {
        if spec.eq_ignore_ascii_case("all") {
            return Ok(Self {
                cpu: true,
                memory: true,
                disk: true,
                tape: true,
                version: true,
            });
        }
        let mut panels = Self::default();
        for token in spec.split(':') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            match token.to_ascii_lowercase().as_str() {
                "cpu" => panels.cpu = true,
                "mem" | "memory" => panels.memory = true,
                "disk" => panels.disk = true,
                "tape" => panels.tape = true,
                "version" => panels.version = true,
                other => {
                    return Err(format!(
                        "unknown --show-overlay panel {other:?} (expected cpu, mem, disk, tape, version, or all)"
                    ));
                }
            }
        }
        Ok(panels)
    }

    /// Whether any panel needing the comparatively expensive memory
    /// hex-window read + `GetChipSnapshots` plugin round trip is enabled
    /// -- `cpu` alone (the default, bare `--show-overlay`) never needs
    /// either, so `emu_thread::tick`'s throttled fetch stays skipped.
    pub(crate) fn needs_overlay_data(self) -> bool {
        self.memory || self.disk || self.tape
    }
}

/// Which panels the overlay shows and whether it is showing at all -- see
/// `OverlayPanels`'s own doc comment. Always present: `--show-overlay` only
/// decides the starting state, and the `overlay` hotkey flips `visible` at
/// runtime (showing the `cpu` panel when the flag was never given).
#[derive(Resource, Clone)]
pub(crate) struct OverlayStateRes {
    pub(crate) panels: OverlayPanels,
    pub(crate) visible: bool,
    /// The text of the `version` panel (the host and the plugin of the running machine), set once at start.
    pub(crate) version_text: String,
    /// Shared with the emu thread: it only fetches the slower memory/disk/
    /// tape panel data while this is set, so a hidden overlay costs nothing.
    data_wanted: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl OverlayStateRes {
    /// `flag_panels` is `--show-overlay`'s value (`None` = flag not given).
    pub(crate) fn new(
        flag_panels: Option<OverlayPanels>,
        data_wanted: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        let state = OverlayStateRes {
            panels: flag_panels.unwrap_or(OverlayPanels {
                cpu: true,
                ..OverlayPanels::default()
            }),
            visible: flag_panels.is_some(),
            version_text: String::new(),
            data_wanted,
        };
        state.publish_data_wanted();
        state
    }

    /// The flag the emu thread watches (see `data_wanted`).
    pub(crate) fn data_wanted_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.data_wanted.clone()
    }

    /// Shows the version panel (and the overlay with it, if it was hidden), or hides the panel again.
    pub(crate) fn toggle_version(&mut self) {
        if self.visible && self.panels.version {
            self.panels.version = false;
        } else {
            self.visible = true;
            self.panels.version = true;
        }
        self.publish_data_wanted();
    }

    pub(crate) fn toggle(&mut self) -> bool {
        self.visible = !self.visible;
        self.publish_data_wanted();
        self.visible
    }

    fn publish_data_wanted(&self) {
        self.data_wanted.store(
            self.visible && self.panels.needs_overlay_data(),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

/// How often the `--show-overlay` (`-so`) text is rewritten. Deliberately
/// not every frame -- register/PC values changing 50-60 times a second
/// would be unreadable, and re-laying-out a text block every frame is
/// wasted work for something meant to be glanced at, not read live.
const DEBUG_OVERLAY_INTERVAL: Duration = Duration::from_millis(500);

/// Spawns the debug overlay's single text entity: a `Node` positioned
/// absolutely in the top-left corner (independent of the sprite/camera, so
/// it doesn't scale or move with the emulated screen) with a translucent
/// black background so the video output stays visible through it.
/// `update_debug_overlay` fills in the actual text -- starts empty so
/// nothing flashes on screen for one frame before the first tick.
pub(crate) fn setup_debug_overlay(mut commands: Commands) {
    commands.spawn((
        DebugOverlayText,
        Visibility::Hidden,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(14.0),
            ..default()
        },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(6.0),
            left: Val::Px(6.0),
            padding: UiRect::all(Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
    ));
}

/// Rewrites the overlay every `DEBUG_OVERLAY_INTERVAL`. The system itself
/// runs once per rendered Bevy frame, so the interval's frame count divided
/// by real elapsed time is the GUI frame rate regardless of emulated machine
/// or CPU family.
// Bevy system parameters, one per resource/query the system reads.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_debug_overlay(
    time: Res<Time>,
    snapshot: Res<DebugSnapshotRes>,
    state: Res<OverlayStateRes>,
    mut timer: Local<Option<Timer>>,
    mut rendered_frames: Local<u32>,
    mut frame_time: Local<f64>,
    mut was_visible: Local<bool>,
    mut text: Query<(&mut Text, &mut Visibility), With<DebugOverlayText>>,
) {
    let Ok((mut text, mut visibility)) = text.single_mut() else {
        return;
    };
    *visibility = if state.visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if !state.visible {
        *was_visible = false;
        return;
    }
    // Refresh at once when the overlay has just been switched on, instead of
    // showing an empty box until the next 500ms tick.
    let just_shown = !*was_visible;
    *was_visible = true;

    *rendered_frames = rendered_frames.saturating_add(1);
    *frame_time += time.delta().as_secs_f64();

    let timer =
        timer.get_or_insert_with(|| Timer::new(DEBUG_OVERLAY_INTERVAL, TimerMode::Repeating));
    timer.tick(time.delta());
    if !timer.just_finished() && !just_shown {
        return;
    }

    let fps = if *frame_time > 0.0 {
        f64::from(*rendered_frames) / *frame_time
    } else {
        0.0
    };
    *rendered_frames = 0;
    *frame_time = 0.0;

    let snapshot = snapshot.0.lock().unwrap();
    let mut out = format_overlay(
        state.panels,
        snapshot.halted,
        &snapshot.registers,
        fps,
        snapshot.memory_hex_addr,
        &snapshot.memory_hex,
        &snapshot.overlay,
    );
    if state.panels.version {
        out.push_str(&state.version_text);
    }
    text.0 = out;
}

#[allow(clippy::too_many_arguments)]
fn format_overlay(
    panels: OverlayPanels,
    halted: bool,
    registers: &[(&'static str, u64)],
    fps: f64,
    memory_hex_addr: u16,
    memory_hex: &[u8],
    overlay: &crate::plugin_bridge::OverlayDebugSnapshot,
) -> String {
    let mut out = format!("FPS {fps:5.1}\n");

    if panels.cpu {
        if halted {
            out.push_str("HALTED\n");
        }
        if is_6502(registers) {
            format_6502(registers, &mut out);
        } else {
            for pair in registers.chunks(2) {
                match pair {
                    [(n1, v1), (n2, v2)] => {
                        out.push_str(&format!("{n1:<4} 0x{v1:04X}  {n2:<4} 0x{v2:04X}\n"));
                    }
                    [(name, value)] => out.push_str(&format!("{name:<4} 0x{value:04X}\n")),
                    _ => unreachable!(),
                }
            }
        }
    }

    if panels.memory {
        format_memory_panel(
            memory_hex_addr,
            memory_hex,
            overlay.memory_summary.as_deref(),
            &mut out,
        );
    }
    if panels.disk {
        format_disk_panel(overlay.fdc_phase.as_deref(), overlay.disk_motor, &mut out);
    }
    if panels.tape {
        format_tape_panel(
            overlay.tape_motor,
            overlay.tape_position,
            overlay.tape_total,
            overlay.tape_edge_cycles,
            &overlay.tape_pulse_widths,
            &overlay.tape_waveform,
            &mut out,
        );
    }

    out
}

/// Hex dump of the small fixed-size memory window at the current PC, plus
/// the machine's own paging/bank summary line if it has one (`None` on
/// machines/models with no banking concept at all, e.g. ZX81 or 48K
/// Spectrum).
fn format_memory_panel(addr: u16, hex: &[u8], summary: Option<&str>, out: &mut String) {
    if hex.is_empty() {
        return;
    }
    let bytes = hex
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    out.push_str(&format!("MEM @{addr:04X}: {bytes}\n"));
    if let Some(summary) = summary {
        out.push_str(&format!("  bank: {summary}\n"));
    }
}

/// Floppy disk controller phase + motor line, for machines/models with a
/// disk drive (`Model::Spectrum2A3`, CPC664/6128) -- `n/a` when neither
/// field is present (no disk support on this machine, or no disk-capable
/// model selected).
fn format_disk_panel(fdc_phase: Option<&str>, disk_motor: Option<bool>, out: &mut String) {
    if fdc_phase.is_none() && disk_motor.is_none() {
        out.push_str("DISK: n/a\n");
        return;
    }
    let phase = fdc_phase.unwrap_or("n/a");
    let motor = match disk_motor {
        Some(true) => "on",
        Some(false) => "off",
        None => "n/a",
    };
    out.push_str(&format!("DISK: phase={phase} motor={motor}\n"));
}

/// Cassette motor state, playback progress, next-edge countdown, a
/// recent numeric pulse-width list, and an EAR-bit-history ASCII bar --
/// for machines/models with tape support (ZX Spectrum, CPC). `n/a` when
/// nothing at all is present (no tape support on this machine, or none
/// installed). Each piece answers a different debugging question:
/// `pos`/`edge_in` are the stall-vs-real-pause signal (frozen values
/// between two overlay refreshes mean playback genuinely isn't
/// advancing, not just between two real pulses); `pulses` is the
/// corrupt-encoding signal -- compare against the standard ROM
/// protocol's known-good widths (pilot ~2168T, sync ~667/735T, bit0
/// ~855T, bit1 ~1710T); the bar (`0`=low/`1`=high, oldest-first,
/// time-uniform samples -- see the producing crates' own
/// `TAPE_WAVEFORM_SAMPLE_INTERVAL_CYCLES`) is for spotting gross visual
/// patterns (dropouts, wildly irregular timing) at a glance.
#[allow(clippy::too_many_arguments)]
fn format_tape_panel(
    tape_motor: Option<bool>,
    tape_position: Option<u64>,
    tape_total: Option<u64>,
    tape_edge_cycles: Option<u64>,
    pulse_widths: &[u16],
    waveform: &[u8],
    out: &mut String,
) {
    if tape_motor.is_none()
        && tape_position.is_none()
        && pulse_widths.is_empty()
        && waveform.is_empty()
    {
        out.push_str("TAPE: n/a\n");
        return;
    }
    let motor = match tape_motor {
        Some(true) => "on",
        Some(false) => "off",
        None => "n/a",
    };
    out.push_str(&format!("TAPE: motor={motor}"));
    if let (Some(position), Some(total)) = (tape_position, tape_total) {
        let pct = if total > 0 {
            (position as f64 / total as f64 * 100.0).round() as u64
        } else {
            0
        };
        out.push_str(&format!(" pos={position}/{total} ({pct}%)"));
    }
    if let Some(edge_cycles) = tape_edge_cycles {
        out.push_str(&format!(" edge_in={edge_cycles}T"));
    }
    out.push('\n');
    if !pulse_widths.is_empty() {
        let widths = pulse_widths
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        out.push_str(&format!("  pulses: {widths}\n"));
    }
    if !waveform.is_empty() {
        let bar: String = waveform
            .iter()
            .map(|&level| if level == 0 { '0' } else { '1' })
            .collect();
        out.push_str(&format!("  {bar}\n"));
    }
}

fn is_6502(registers: &[(&'static str, u64)]) -> bool {
    ["A", "P", "PC", "S", "X", "Y"]
        .iter()
        .all(|name| register(registers, name).is_some())
}

fn format_6502(registers: &[(&'static str, u64)], out: &mut String) {
    let a = register(registers, "A").unwrap() as u8;
    let p = register(registers, "P").unwrap() as u8;
    let pc = register(registers, "PC").unwrap() as u16;
    let s = register(registers, "S").unwrap() as u8;
    let x = register(registers, "X").unwrap() as u8;
    let y = register(registers, "Y").unwrap() as u8;
    let flags = format_6502_flags(p);

    out.push_str(&format!("A   0x{a:02X}  P   0x{p:02X}  [{flags}]\n"));
    out.push_str(&format!("PC  0x{pc:04X}  S   0x{s:02X}\n"));
    out.push_str(&format!("X   0x{x:02X}  Y   0x{y:02X}\n"));
}

fn register(registers: &[(&'static str, u64)], name: &str) -> Option<u64> {
    registers
        .iter()
        .find_map(|(candidate, value)| (*candidate == name).then_some(*value))
}

/// Common debugger notation: uppercase means set, lowercase means clear.
/// Bit 5 is printed as `-` because it has no hardware flag; B is shown even
/// though it normally exists only in a status byte pushed to the stack.
fn format_6502_flags(p: u8) -> String {
    let mut flags = String::with_capacity(8);
    for (mask, set, clear) in [
        (0x80, 'N', 'n'),
        (0x40, 'V', 'v'),
        (0x00, '-', '-'),
        (0x10, 'B', 'b'),
        (0x08, 'D', 'd'),
        (0x04, 'I', 'i'),
        (0x02, 'Z', 'z'),
        (0x01, 'C', 'c'),
    ] {
        flags.push(if p & mask != 0 { set } else { clear });
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_every_6502_register_with_native_width_and_flags() {
        let registers = [
            ("A", 0x12),
            ("X", 0x34),
            ("Y", 0x56),
            ("S", 0xFD),
            ("PC", 0xC000),
            ("P", 0xE5),
        ];
        let overlay = crate::plugin_bridge::OverlayDebugSnapshot::default();
        let panels = OverlayPanels {
            cpu: true,
            ..Default::default()
        };
        let text = format_overlay(panels, false, &registers, 59.94, 0, &[], &overlay);

        assert!(text.contains("FPS  59.9"));
        assert!(text.contains("A   0x12  P   0xE5  [NV-bdIzC]"));
        assert!(text.contains("PC  0xC000  S   0xFD"));
        assert!(text.contains("X   0x34  Y   0x56"));
    }

    #[test]
    fn generic_cpu_overlay_keeps_existing_register_layout_and_fps() {
        let overlay = crate::plugin_bridge::OverlayDebugSnapshot::default();
        let panels = OverlayPanels {
            cpu: true,
            ..Default::default()
        };
        let text = format_overlay(
            panels,
            true,
            &[("AF", 0x1234), ("PC", 0x5678)],
            50.0,
            0,
            &[],
            &overlay,
        );
        assert!(text.starts_with("FPS  50.0\nHALTED\n"));
        assert!(text.contains("AF   0x1234  PC   0x5678"));
    }

    fn cpu_only_panels_omit_memory_disk_and_tape_lines_entirely() {
        let overlay = crate::plugin_bridge::OverlayDebugSnapshot::default();
        let panels = OverlayPanels {
            cpu: true,
            ..Default::default()
        };
        let text = format_overlay(panels, false, &[("PC", 0x0000)], 50.0, 0, &[], &overlay);

        assert!(!text.contains("MEM @"));
        assert!(!text.contains("DISK:"));
        assert!(!text.contains("TAPE:"));
    }

    fn overlay_panels_show_n_a_when_enabled_but_machine_has_no_data() {
        let overlay = crate::plugin_bridge::OverlayDebugSnapshot::default();
        let panels = OverlayPanels {
            memory: true,
            disk: true,
            tape: true,
            ..Default::default()
        };
        let text = format_overlay(panels, false, &[("PC", 0x0000)], 50.0, 0, &[], &overlay);

        assert!(!text.contains("MEM @"));
        assert!(text.contains("DISK: n/a"));
        assert!(text.contains("TAPE: n/a"));
    }
    /// Merged (every check kept): `cpu_only_panels_omit_memory_disk_and_tape_lines_entirely`, `overlay_panels_show_n_a_when_enabled_but_machine_has_no_data`.
    #[test]
    fn overlay_panels_without_data() {
        cpu_only_panels_omit_memory_disk_and_tape_lines_entirely();
        overlay_panels_show_n_a_when_enabled_but_machine_has_no_data();
    }

    #[test]
    fn overlay_panels_render_memory_disk_and_tape_data_when_present() {
        let overlay = crate::plugin_bridge::OverlayDebugSnapshot {
            memory_summary: Some("ROM1 RAM5".to_string()),
            fdc_phase: Some("command".to_string()),
            disk_motor: Some(true),
            tape_motor: Some(false),
            tape_position: Some(1234),
            tape_total: Some(3223),
            tape_edge_cycles: Some(320),
            tape_waveform: vec![1, 0, 0, 1, 1],
            tape_pulse_widths: vec![1710, 855, 1710],
        };
        let panels = OverlayPanels {
            memory: true,
            disk: true,
            tape: true,
            ..Default::default()
        };
        let text = format_overlay(
            panels,
            false,
            &[("PC", 0x1234)],
            50.0,
            0x1234,
            &[0x3E, 0x05, 0xCD],
            &overlay,
        );

        assert!(text.contains("MEM @1234: 3E 05 CD"));
        assert!(text.contains("bank: ROM1 RAM5"));
        assert!(text.contains("DISK: phase=command motor=on"));
        assert!(text.contains("TAPE: motor=off pos=1234/3223 (38%) edge_in=320T"));
        assert!(text.contains("pulses: 1710 855 1710"));
        assert!(text.contains("10011"));
    }

    fn parse_defaults_to_cpu_only_when_flag_has_no_value() {
        // Matches clap's `default_missing_value = "cpu"` for a bare
        // `--show-overlay`.
        let panels = OverlayPanels::parse("cpu").unwrap();
        assert_eq!(
            panels,
            OverlayPanels {
                cpu: true,
                ..Default::default()
            }
        );
    }

    fn parse_accepts_colon_delimited_panel_list() {
        let panels = OverlayPanels::parse("cpu:mem:tape").unwrap();
        assert_eq!(
            panels,
            OverlayPanels {
                cpu: true,
                memory: true,
                disk: false,
                tape: true,
                version: false,
            }
        );
    }

    fn parse_accepts_disk_panel() {
        let panels = OverlayPanels::parse("cpu:disk:tape").unwrap();
        assert_eq!(
            panels,
            OverlayPanels {
                cpu: true,
                memory: false,
                disk: true,
                tape: true,
                version: false,
            }
        );
    }

    fn parse_all_enables_every_panel() {
        let panels = OverlayPanels::parse("all").unwrap();
        assert_eq!(
            panels,
            OverlayPanels {
                cpu: true,
                memory: true,
                disk: true,
                tape: true,
                version: true,
            }
        );
        assert_eq!(
            OverlayPanels::parse("ALL").unwrap(),
            panels,
            "case-insensitive"
        );
    }

    fn parse_rejects_unknown_panel_name() {
        let err = OverlayPanels::parse("cpu:bogus").unwrap_err();
        assert!(
            err.contains("bogus"),
            "error must name the bad token: {err}"
        );
    }

    fn parse_ignores_blank_tokens_from_stray_colons() {
        let panels = OverlayPanels::parse(":cpu::tape:").unwrap();
        assert_eq!(
            panels,
            OverlayPanels {
                cpu: true,
                tape: true,
                ..Default::default()
            }
        );
    }
    /// Merged (every check kept): `parse_defaults_to_cpu_only_when_flag_has_no_value`, `parse_accepts_colon_delimited_panel_list`, `parse_accepts_disk_panel`, `parse_all_enables_every_panel`, `parse_rejects_unknown_panel_name`, `parse_ignores_blank_tokens_from_stray_colons`.
    #[test]
    fn overlay_panels_parse() {
        parse_defaults_to_cpu_only_when_flag_has_no_value();
        parse_accepts_colon_delimited_panel_list();
        parse_accepts_disk_panel();
        parse_all_enables_every_panel();
        parse_rejects_unknown_panel_name();
        parse_ignores_blank_tokens_from_stray_colons();
    }

    #[test]
    fn the_version_panel_parses_and_the_hotkey_flips_it_and_shows_the_overlay() {
        assert!(OverlayPanels::parse("cpu:version").unwrap().version);
        assert!(!OverlayPanels::parse("cpu").unwrap().version);
        // Version info needs no machine data from the emu thread.
        assert!(
            !OverlayPanels::parse("version")
                .unwrap()
                .needs_overlay_data()
        );
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut state = OverlayStateRes::new(None, flag);
        state.version_text = "retroclawd 0.1.0 build 1
"
        .to_string();
        assert!(!state.visible);
        state.toggle_version();
        assert!(
            state.visible && state.panels.version,
            "shows the overlay too"
        );
        state.toggle_version();
        assert!(
            state.visible && !state.panels.version,
            "a second press hides only the panel"
        );
        state.toggle_version();
        assert!(state.panels.version);
    }

    fn needs_overlay_data_is_false_for_cpu_only() {
        let panels = OverlayPanels::parse("cpu").unwrap();
        assert!(!panels.needs_overlay_data());
    }

    fn needs_overlay_data_is_true_when_any_data_panel_enabled() {
        assert!(OverlayPanels::parse("mem").unwrap().needs_overlay_data());
        assert!(OverlayPanels::parse("disk").unwrap().needs_overlay_data());
        assert!(OverlayPanels::parse("tape").unwrap().needs_overlay_data());
    }
    /// Merged (every check kept): `needs_overlay_data_is_false_for_cpu_only`, `needs_overlay_data_is_true_when_any_data_panel_enabled`.
    #[test]
    fn needs_overlay_data() {
        needs_overlay_data_is_false_for_cpu_only();
        needs_overlay_data_is_true_when_any_data_panel_enabled();
    }
}
