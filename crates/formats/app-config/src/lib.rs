//! Parses retroclawd's persistent settings `.toml` (auto-loaded from
//! `retroclawd.toml` or selected with `--config`). Settings include shader,
//! window size, fullscreen/vsync/max-fps, each machine's own plugin options
//! (flat in its `[machines.<id>]` section), host-key-to-machine-key bindings
//! (`[keys]`), emulator hotkeys (`[hotkeys]`) and gamepad bindings (`[pads]`). The crate stays
//! machine-agnostic: machine ids and key names remain strings, while the app
//! resolves them against the real plugin key names.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    #[serde(default)]
    pub default: MachineSettings,
    #[serde(default)]
    pub machines: HashMap<String, MachineSettings>,
    /// `[keys]`: host key (or chord) -> machine key(s), for every machine;
    /// `[machines.<id>.keys]` overrides per machine. Values are checked
    /// against the selected machine's own key names by the app.
    #[serde(default)]
    pub keys: HashMap<String, OneOrMany>,
    /// The old `[keymap]` section, kept only so its presence is a clear error
    /// (`[keys]` replaced it) instead of "unknown field".
    #[serde(default)]
    pub keymap: Option<toml::Value>,
    /// Emulator hotkeys (reset, mute, pause, ...): `action = "chord"`.
    /// Per-machine overrides live under `[machines.<id>.hotkeys]`.
    #[serde(default)]
    pub hotkeys: HotkeysConfig,
    /// Gamepad bindings: `[pads]` (`deadzone` plus one `[pads.<slot>]`
    /// sub-table per pad, slot 1 = first connected pad); per-machine
    /// overrides live under `[machines.<id>.pads]`.
    #[serde(default)]
    pub pads: PadsConfig,
    /// Keyboard layout handling and keymap files: `[input]`; per-machine
    /// overrides under `[machines.<id>.input]`.
    #[serde(default)]
    pub input: InputConfig,
    /// `[romm]`: the RomM server `romm:<id>` sources and the `romm` verbs use.
    #[serde(default)]
    pub romm: RommConfig,
}

/// `[romm]`. The `ROMM_URL` / `ROMM_TOKEN` environment variables beat both
/// keys; keep the token out of any file that is committed.
#[derive(Debug, Deserialize, PartialEq, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct RommConfig {
    pub url: Option<String>,
    pub token: Option<String>,
    /// `"auto"`: fill ROM options nothing else names from the server's
    /// firmware, matched by SHA-1 (`--firmware`); `"off"` (the default).
    pub firmware: Option<String>,
}

/// `[input]` or `[machines.<id>.input]`.
#[derive(Debug, Deserialize, PartialEq, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct InputConfig {
    /// `"positional"` (the default: a host key is the machine key at the same
    /// physical position) or `"symbolic"` (what the host key *types* picks the
    /// machine key, whatever the host layout is). Checked by the app.
    pub layout: Option<String>,
    /// Keymap files (see `KeymapFile`) layered under the inline `[keys]`.
    pub keymap: Option<OneOrMany>,
    /// How the paste hotkey types letters: `"fold"` (the default: every letter
    /// is the plain key, right for an uppercase C64 listing) or `"exact"`
    /// (a capital holds the machine's Shift). Checked by the app.
    pub paste_case: Option<String>,
    /// Milliseconds each pasted key stays pressed and stays released
    /// (default 60, i.e. three 50 Hz frames).
    pub paste_delay_ms: Option<u64>,
}

/// A standalone keymap file (`--keymap <path>` or `[input] keymap`): the same
/// `[keys]` / `[machines.<id>.keys]` bindings the config holds, plus an
/// optional layout, so a mapping can be shared or kept per keyboard.
#[derive(Debug, Deserialize, PartialEq, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct KeymapFile {
    /// Layout this map is written for (the config's own `layout` wins).
    pub layout: Option<String>,
    #[serde(default)]
    pub keys: HashMap<String, OneOrMany>,
    #[serde(default)]
    pub machines: HashMap<String, KeymapMachine>,
}

/// `[machines.<id>]` inside a keymap file.
#[derive(Debug, Deserialize, PartialEq, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct KeymapMachine {
    #[serde(default)]
    pub keys: HashMap<String, OneOrMany>,
}

/// Parses a keymap file's text. Unknown keys are an error, like everywhere
/// else in this crate.
pub fn parse_keymap(text: &str) -> Result<KeymapFile, toml::de::Error> {
    toml::from_str(text)
}

/// One string or a list of strings: `reset = "f12"` or `reset = ["f12",
/// "ctrl+r"]` for hotkeys (several chords for one action), `f1 = ["caps_shift",
/// "1"]` for keys (several machine keys pressed together). The syntax inside
/// (`shift+f5`, `alt_right`, `none`) is parsed by the app, which owns the host
/// and machine key names -- this crate only keeps the strings.
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn as_slice(&self) -> &[String] {
        match self {
            OneOrMany::One(chord) => std::slice::from_ref(chord),
            OneOrMany::Many(chords) => chords,
        }
    }
}

/// `[hotkeys]` or `[machines.<id>.hotkeys]`. `modifier` is the optional
/// hold-to-enable key (e.g. `"alt_right"`): when set, every hotkey fires
/// only while it is held, so the plain keys stay free for the machine. Every
/// other key is an action name (validated by the app against the real action
/// list, not by serde, so a typo reports the valid names).
#[derive(Debug, Deserialize, PartialEq, Default, Clone)]
pub struct HotkeysConfig {
    pub modifier: Option<String>,
    #[serde(flatten)]
    pub bindings: HashMap<String, OneOrMany>,
}

/// `[pads]` or `[machines.<id>.pads]`. `deadzone` (0..1, default 0.5) is how
/// far a stick must be pushed before it counts as a direction. Every other
/// key is a pad slot number (`[pads.1]`) holding `button = "machine key"`
/// pairs; slot numbers, button names and machine keys are validated by the
/// app, which knows the real names.
#[derive(Debug, Deserialize, PartialEq, Default, Clone)]
pub struct PadsConfig {
    pub deadzone: Option<f32>,
    #[serde(flatten)]
    pub slots: HashMap<String, HashMap<String, OneOrMany>>,
}

#[derive(Debug, Deserialize, PartialEq, Default, Clone)]
pub struct MachineSettings {
    pub shader: Option<String>,
    pub shader_intensity: Option<f32>,
    pub scanline_intensity: Option<f32>,
    pub window_scale_percent: Option<u32>,
    /// Open the GUI window borderless-fullscreen (same as `--fullscreen`).
    pub fullscreen: Option<bool>,
    /// `false` disables display VSync (same as `--no-vsync`).
    pub vsync: Option<bool>,
    /// Cap the GUI render/update rate (same as `--max-fps`).
    pub max_fps: Option<u32>,
    /// `[machines.<id>.hotkeys]`: per-machine hotkey overrides.
    #[serde(default)]
    pub hotkeys: HotkeysConfig,
    /// `[machines.<id>.keys]`: per-machine key bindings, layered over `[keys]`.
    #[serde(default)]
    pub keys: HashMap<String, OneOrMany>,
    /// `[machines.<id>.pads]`: per-machine gamepad bindings, layered over `[pads]`.
    #[serde(default)]
    pub pads: PadsConfig,
    /// `[machines.<id>.input]`: per-machine layout / keymap files.
    #[serde(default)]
    pub input: InputConfig,
    /// Every other key in the section: machine-plugin-declared options.
    #[serde(flatten)]
    pub options: HashMap<String, toml::Value>,
}

/// Parses toml text into an `AppConfig`. Unknown keys are a hard error
/// (`deny_unknown_fields`) rather than silently ignored -- a typo'd key
/// should not look like a no-op, same stance `game_config::parse` takes.
pub fn parse(text: &str) -> Result<AppConfig, toml::de::Error> {
    toml::from_str(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parses_default_section() {
        let config = parse(
            r#"
            [default]
            shader = "crt"
            shader_intensity = 0.6
            scanline_intensity = 0.3
            window_scale_percent = 60
            "#,
        )
        .unwrap();

        assert_eq!(
            config.default,
            MachineSettings {
                shader: Some("crt".to_string()),
                shader_intensity: Some(0.6),
                scanline_intensity: Some(0.3),
                window_scale_percent: Some(60),
                fullscreen: None,
                vsync: None,
                max_fps: None,
                hotkeys: HotkeysConfig::default(),
                keys: HashMap::new(),
                pads: PadsConfig::default(),
                input: InputConfig::default(),
                options: HashMap::new(),
            }
        );
        assert!(config.machines.is_empty());
    }

    fn parses_per_machine_sections() {
        let config = parse(
            r#"
            [default]
            shader = "crt"

            [machines.zx-spectrum]
            shader = "scanlines"

            [machines.c64]
            shader_intensity = 0.8
            "#,
        )
        .unwrap();

        assert_eq!(config.default.shader, Some("crt".to_string()));
        assert_eq!(
            config.machines.get("zx-spectrum").unwrap().shader,
            Some("scanlines".to_string())
        );
        assert_eq!(
            config.machines.get("c64").unwrap().shader_intensity,
            Some(0.8)
        );
    }

    fn parses_window_and_frame_pacing_settings() {
        let config = parse(
            r#"
            [default]
            fullscreen = true
            vsync = false
            max_fps = 60
            "#,
        )
        .unwrap();
        assert_eq!(config.default.fullscreen, Some(true));
        assert_eq!(config.default.vsync, Some(false));
        assert_eq!(config.default.max_fps, Some(60));
        assert!(config.default.options.is_empty());
    }
    /// Merged (every check kept): `parses_default_section`, `parses_per_machine_sections`, `parses_window_and_frame_pacing_settings`.
    #[test]
    fn parses_host_settings() {
        parses_default_section();
        parses_per_machine_sections();
        parses_window_and_frame_pacing_settings();
    }

    #[test]
    fn machine_options_live_flat_beside_host_settings() {
        let config = parse(
            r#"
            [machines.cpc6128]
            shader = "crt"
            ram_banks = 3
            tape_mode = "caprice"
            rom = "roms/cpc/cpc6128.rom"
            "#,
        )
        .unwrap();

        let machine = config.machines.get("cpc6128").unwrap();
        assert_eq!(machine.shader, Some("crt".to_string()));
        let options = &machine.options;
        assert!(!options.contains_key("shader"));
        assert_eq!(options.get("ram_banks"), Some(&toml::Value::Integer(3)));
        assert_eq!(
            options.get("tape_mode"),
            Some(&toml::Value::String("caprice".to_string()))
        );
        assert_eq!(
            options.get("rom"),
            Some(&toml::Value::String("roms/cpc/cpc6128.rom".to_string()))
        );
    }

    fn parses_global_and_machine_hotkeys() {
        let config = parse(
            r#"
            [hotkeys]
            modifier = "alt_right"
            reset = "f12"
            mute = ["f9", "ctrl+m"]
            quit = "none"

            [machines.c64.hotkeys]
            reset = "ctrl+f12"
            "#,
        )
        .unwrap();
        assert_eq!(config.hotkeys.modifier.as_deref(), Some("alt_right"));
        assert_eq!(config.hotkeys.bindings["reset"].as_slice(), ["f12"]);
        assert_eq!(config.hotkeys.bindings["mute"].as_slice(), ["f9", "ctrl+m"]);
        assert!(!config.hotkeys.bindings.contains_key("modifier"));
        let c64 = &config.machines["c64"];
        assert_eq!(c64.hotkeys.bindings["reset"].as_slice(), ["ctrl+f12"]);
        // `hotkeys` is a typed field, not swallowed into the options map.
        assert!(!c64.options.contains_key("hotkeys"));
    }

    fn a_non_string_hotkey_value_is_rejected() {
        assert!(parse("[hotkeys]\nreset = 12").is_err());
    }

    fn parses_global_and_machine_key_bindings() {
        let config = parse(
            r#"
            [keys]
            semicolon = "colon"
            "shift+2" = ["caps_shift", "2"]
            backquote = "none"

            [machines.c64.keys]
            f2 = "f2"
            "#,
        )
        .unwrap();
        assert_eq!(config.keys["semicolon"].as_slice(), ["colon"]);
        assert_eq!(config.keys["shift+2"].as_slice(), ["caps_shift", "2"]);
        assert_eq!(config.keys["backquote"].as_slice(), ["none"]);
        let c64 = &config.machines["c64"];
        assert_eq!(c64.keys["f2"].as_slice(), ["f2"]);
        // `keys` is a typed field, not swallowed into the options map.
        assert!(!c64.options.contains_key("keys"));
    }

    #[test]
    fn the_old_keymap_section_still_parses_so_the_app_can_explain_the_change() {
        let config = parse("[keymap.default]\nleft = \"left\"").unwrap();
        assert!(config.keymap.is_some());
    }

    #[test]
    fn unknown_top_level_field_is_rejected() {
        assert!(parse(r#"defalut = {}"#).is_err());
    }

    #[test]
    fn a_nested_file_set_table_is_kept_as_an_option() {
        let config = parse(
            r#"
            [machines.cpc6128.rom_slot]
            7 = "amsdos.rom"
            "#,
        )
        .unwrap();
        assert!(matches!(
            config.machines["cpc6128"].options.get("rom_slot"),
            Some(toml::Value::Table(_))
        ));
    }

    fn wrongly_typed_host_setting_is_rejected() {
        assert!(parse("[default]\nmax_fps = \"fast\"").is_err());
        assert!(parse("[default]\nfullscreen = 1").is_err());
    }

    fn malformed_toml_is_rejected() {
        assert!(parse("default = ").is_err());
    }

    fn parses_global_and_machine_pads() {
        let config = parse(
            r#"
            [pads]
            deadzone = 0.4
            [pads.1]
            south = "Joy1Fire1"
            west = ["Joy1Fire1", "Joy1Up"]
            [machines.c64.pads.2]
            start = "none"
            "#,
        )
        .unwrap();
        assert_eq!(config.pads.deadzone, Some(0.4));
        assert_eq!(config.pads.slots["1"]["south"].as_slice(), ["Joy1Fire1"]);
        assert_eq!(
            config.pads.slots["1"]["west"].as_slice(),
            ["Joy1Fire1", "Joy1Up"]
        );
        assert!(!config.pads.slots.contains_key("deadzone"));
        let c64 = &config.machines["c64"];
        assert_eq!(c64.pads.slots["2"]["start"].as_slice(), ["none"]);
        // `pads` is a typed field, not swallowed into the options map.
        assert!(!c64.options.contains_key("pads"));
    }

    fn a_pad_button_must_be_a_string_or_list() {
        assert!(parse("[pads.1]\nsouth = 3").is_err());
    }
    /// Merged (every check kept): `a_non_string_hotkey_value_is_rejected`, `a_pad_button_must_be_a_string_or_list`, `wrongly_typed_host_setting_is_rejected`, `malformed_toml_is_rejected`.
    #[test]
    fn rejects_bad_values() {
        a_non_string_hotkey_value_is_rejected();
        a_pad_button_must_be_a_string_or_list();
        wrongly_typed_host_setting_is_rejected();
        malformed_toml_is_rejected();
    }

    fn parses_input_sections_and_keymap_files() {
        let config = parse(
            r#"
            [input]
            layout = "symbolic"
            keymap = ["a.toml", "b.toml"]
            [machines.c64.input]
            keymap = "c64.toml"
            "#,
        )
        .unwrap();
        assert_eq!(config.input.layout.as_deref(), Some("symbolic"));
        assert_eq!(
            config.input.keymap.unwrap().as_slice(),
            ["a.toml", "b.toml"]
        );
        let c64 = &config.machines["c64"];
        assert_eq!(c64.input.keymap.as_ref().unwrap().as_slice(), ["c64.toml"]);
        assert!(!c64.options.contains_key("input"));
        assert!(parse("[input]\nlayoutt = \"x\"").is_err());

        let map = parse_keymap(
            r#"
            layout = "symbolic"
            [keys]
            semicolon = "colon"
            [machines.zx-spectrum.keys]
            "shift+2" = "at"
            "#,
        )
        .unwrap();
        assert_eq!(map.layout.as_deref(), Some("symbolic"));
        assert_eq!(map.keys["semicolon"].as_slice(), ["colon"]);
        assert_eq!(
            map.machines["zx-spectrum"].keys["shift+2"].as_slice(),
            ["at"]
        );
        assert!(parse_keymap("[hotkeys]\nreset = \"f1\"").is_err());
        assert!(parse_keymap("[machines.c64]\nrom = \"x\"").is_err());
    }
    /// Merged (every check kept): `parses_global_and_machine_hotkeys`, `parses_global_and_machine_key_bindings`, `parses_global_and_machine_pads`, `parses_input_sections_and_keymap_files`.
    #[test]
    fn parses_input_sections() {
        parses_global_and_machine_hotkeys();
        parses_global_and_machine_key_bindings();
        parses_global_and_machine_pads();
        parses_input_sections_and_keymap_files();
    }
}
