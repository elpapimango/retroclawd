//! Parses a per-game `.toml` config file: which machine/ROM/region to boot,
//! and which media file (snapshot/tape/floppy) to load. Deliberately
//! machine-agnostic and clap-agnostic -- same "format crate stays pure" split
//! as `z80-snapshot`/`zx81-snapshot`. `machine`/`region` are kept as raw
//! strings rather than this crate inventing its own enum for them: turning
//! those strings into real types (and applying them to a real machine) is
//! `retroclawd`'s own job, which already owns `MachineKind`/`RegionArg`.

use std::collections::HashMap;

use serde::Deserialize;

/// A string or a list of strings (`symbols = "game.sym"` or `symbols = ["a.sym", "b.sym"]`).
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            OneOrMany::One(text) => vec![text.clone()],
            OneOrMany::Many(list) => list.clone(),
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct GameConfig {
    pub machine: Option<String>,
    pub region: Option<String>,
    /// Path to a snapshot/tape/floppy file. Deliberately one field, not a
    /// tagged union -- kind is inferred from the extension, exactly like
    /// the CLI's own `--snapshot`/`--tape`/`--disk` flags already do.
    pub media: Option<String>,
    pub tape_speed: Option<u32>,
    /// Freeform machine-plugin-declared options (e.g. `rom`, `ram_banks`,
    /// `speaker_type`), under `[options]`. Overrides the same key in
    /// `retroclawd.toml`'s `[machines.<id>]` section when both are
    /// present. Validated against the selected machine's declared
    /// `MachineOptionInfo` schema at runtime, not by serde -- includes
    /// `rom` itself now (superseding this format's former dedicated `rom`
    /// field), matching app-config's identical shape so the host's
    /// resolver treats both sources uniformly.
    #[serde(default)]
    pub options: HashMap<String, toml::Value>,
    /// `[keys]`: host key (or chord) -> machine key name(s), layered over `retroclawd.toml`'s `[keys]` and the machine's own
    /// `[machines.<id>.keys]` for this game only. The host validates the names against the machine's key list, like the
    /// config file's.
    #[serde(default)]
    pub keys: HashMap<String, OneOrMany>,
    /// `layout = "symbolic"`: the keyboard layout for this game (`--layout` still wins).
    pub layout: Option<String>,
    /// Symbol file(s) for the debugger's disassembly (`--symbols`), loaded before the command line's own.
    pub symbols: Option<OneOrMany>,
}

/// Parses toml text into a `GameConfig`. Unknown keys are a hard error
/// (`deny_unknown_fields`) rather than silently ignored -- a typo'd key
/// should not look like a no-op.
pub fn parse(text: &str) -> Result<GameConfig, toml::de::Error> {
    toml::from_str(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_fields() {
        let config = parse(
            r#"
            machine = "zx-spectrum-2a3"
            region = "pal"
            media = "game.z80"
            tape_speed = 4

            [options]
            rom = "roms/zx-spectrum/2a3.rom"
            "#,
        )
        .unwrap();

        assert_eq!(config.machine, Some("zx-spectrum-2a3".to_string()));
        assert_eq!(config.region, Some("pal".to_string()));
        assert_eq!(config.media, Some("game.z80".to_string()));
        assert_eq!(config.tape_speed, Some(4));
        assert_eq!(
            config.options.get("rom"),
            Some(&toml::Value::String("roms/zx-spectrum/2a3.rom".to_string()))
        );
    }

    #[test]
    fn keys_layout_and_symbols_are_per_game_settings() {
        let config = parse(
            r#"
            layout = "symbolic"
            symbols = "elite.sym"

            [keys]
            "KC_SPC" = "fire"
            "KC_Z" = ["left", "fire"]
            "#,
        )
        .unwrap();
        assert_eq!(config.layout.as_deref(), Some("symbolic"));
        assert_eq!(config.symbols.unwrap().to_vec(), ["elite.sym"]);
        assert_eq!(config.keys["KC_SPC"].to_vec(), ["fire"]);
        assert_eq!(config.keys["KC_Z"].to_vec(), ["left", "fire"]);
        assert!(
            parse("symbols = [\"a.sym\", \"b.sym\"]")
                .unwrap()
                .symbols
                .unwrap()
                .to_vec()
                .len()
                == 2
        );
    }

    #[test]
    fn empty_toml_is_all_none() {
        let config = parse("").unwrap();
        assert_eq!(config, GameConfig::default());
    }

    #[test]
    fn unknown_field_is_rejected() {
        assert!(parse(r#"machin = "zx81""#).is_err());
    }

    #[test]
    fn malformed_toml_is_rejected() {
        assert!(parse("machine = ").is_err());
    }
}
