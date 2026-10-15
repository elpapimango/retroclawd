//! Parses a per-game `.toml` config file: which machine/ROM/region to boot,
//! and which media file (snapshot/tape/floppy) to load. Deliberately
//! machine-agnostic and clap-agnostic -- same "format crate stays pure" split
//! as `z80-snapshot`/`zx81-snapshot`. `machine`/`region` are kept as raw
//! strings rather than this crate inventing its own enum for them: turning
//! those strings into real types (and applying them to a real machine) is
//! `retroclawd`'s own job, which already owns `MachineKind`/`RegionArg`.

use std::collections::HashMap;

use serde::Deserialize;

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
