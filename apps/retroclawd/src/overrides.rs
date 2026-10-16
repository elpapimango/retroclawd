//! Settings that belong to one run rather than to `retroclawd.toml`: the `--hotkey ACTION=CHORD` flags and a game config's
//! `[keys]`/`layout`. Both are folded into the loaded `AppConfig` as the highest layer of the machine's own sections, so the
//! normal resolvers (`Hotkeys::resolve`, `KeyBindings::resolve`, `effective_layout`) see them without knowing where they came
//! from and validate them with the same messages.

use app_config::{AppConfig, OneOrMany};
use game_config::GameConfig;

/// Applies `--hotkey ACTION=CHORD` (repeatable): the first flag for an action replaces what the config bound to it, further ones
/// for the same action add chords; `none` unbinds; `modifier=KEY` sets the hold-to-enable key. With a machine the flags go to its
/// own `[machines.<id>.hotkeys]` layer (which beats `[hotkeys]`), without one to `[hotkeys]`.
pub(crate) fn apply_hotkey_flags(
    config: &mut AppConfig,
    machine: &str,
    flags: &[String],
) -> Result<(), String> {
    let layer = if machine.is_empty() {
        &mut config.hotkeys
    } else {
        &mut config
            .machines
            .entry(machine.to_string())
            .or_default()
            .hotkeys
    };
    let mut seen: Vec<String> = Vec::new();
    for flag in flags {
        let (action, chord) = flag
            .split_once('=')
            .map(|(a, c)| (a.trim(), c.trim()))
            .filter(|(a, c)| !a.is_empty() && !c.is_empty())
            .ok_or_else(|| format!("--hotkey \"{flag}\": expected ACTION=CHORD (for example pause=ctrl+p, or reset=none)"))?;
        if action.eq_ignore_ascii_case("modifier") {
            layer.modifier = Some(chord.to_string());
            continue;
        }
        let action = action.to_ascii_lowercase();
        let mut chords = if seen.contains(&action) {
            layer
                .bindings
                .get(&action)
                .map(|existing| existing.as_slice().to_vec())
                .unwrap_or_default()
        } else {
            seen.push(action.clone());
            Vec::new()
        };
        chords.push(chord.to_string());
        let value = if chords.len() == 1 {
            OneOrMany::One(chords.remove(0))
        } else {
            OneOrMany::Many(chords)
        };
        layer.bindings.insert(action, value);
    }
    Ok(())
}

/// Applies a game config's `[keys]` and `layout` to `machine`'s section (bindings of the same host key replace the machine's).
pub(crate) fn apply_game_config(config: &mut AppConfig, machine: &str, game: &GameConfig) {
    if game.keys.is_empty()
        && game.layout.is_none()
        && game.hotkeys == game_config::GameHotkeys::default()
        && game.pads == game_config::GamePads::default()
    {
        return;
    }
    let section = config.machines.entry(machine.to_string()).or_default();
    for (host_key, target) in &game.keys {
        section.keys.insert(host_key.clone(), convert(target));
    }
    // Per action / per button, like every machine layer: what the game names replaces, the rest stays.
    if let Some(modifier) = &game.hotkeys.modifier {
        section.hotkeys.modifier = Some(modifier.clone());
    }
    for (action, chords) in &game.hotkeys.bindings {
        section
            .hotkeys
            .bindings
            .insert(action.clone(), convert(chords));
    }
    if let Some(deadzone) = game.pads.deadzone {
        section.pads.deadzone = Some(deadzone);
    }
    for (slot, buttons) in &game.pads.slots {
        let target = section.pads.slots.entry(slot.clone()).or_default();
        for (button, keys) in buttons {
            target.insert(button.clone(), convert(keys));
        }
    }
    if let Some(layout) = &game.layout {
        section.input.layout = Some(layout.clone());
    }
}

fn convert(value: &game_config::OneOrMany) -> OneOrMany {
    match value {
        game_config::OneOrMany::One(name) => OneOrMany::One(name.clone()),
        game_config::OneOrMany::Many(names) => OneOrMany::Many(names.clone()),
    }
}

/// The symbol files for a run: the game config's, then the command line's (later files win).
pub(crate) fn symbol_files(game: &GameConfig, flags: &[String]) -> Vec<String> {
    let mut files = game
        .symbols
        .as_ref()
        .map(game_config::OneOrMany::to_vec)
        .unwrap_or_default();
    files.extend(flags.iter().cloned());
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_hotkey_flag_replaces_the_config_binding_and_repeats_add_chords() {
        let mut config = app_config::parse(
            r#"
            [hotkeys]
            pause = "pause"
            "#,
        )
        .unwrap();
        apply_hotkey_flags(
            &mut config,
            "c64",
            &flags(&["pause=ctrl+p", "pause=f8", "reset=none"]),
        )
        .unwrap();
        let hotkeys = crate::gui::hotkeys::Hotkeys::resolve(&config, "c64").unwrap();
        let pause: Vec<String> = hotkeys
            .chords_for(crate::gui::hotkeys::Action::from_name("pause").unwrap())
            .iter()
            .map(|c| c.to_text())
            .collect();
        assert_eq!(pause.len(), 2, "{pause:?}");
        assert!(
            pause.iter().any(|c| c.to_lowercase().contains("ctrl")),
            "{pause:?}"
        );
        assert!(
            hotkeys
                .chords_for(crate::gui::hotkeys::Action::from_name("reset").unwrap())
                .is_empty(),
            "reset=none unbinds the default"
        );
    }

    #[test]
    fn hotkey_flags_are_validated_like_the_config() {
        let mut config = AppConfig::default();
        assert!(apply_hotkey_flags(&mut config, "c64", &flags(&["nonsense"])).is_err());
        apply_hotkey_flags(&mut config, "c64", &flags(&["warp=f1"])).unwrap();
        let error = crate::gui::hotkeys::Hotkeys::resolve(&config, "c64").unwrap_err();
        assert!(error.contains("unknown hotkey action \"warp\""), "{error}");
        // The modifier flag sets the hold key.
        let mut config = AppConfig::default();
        apply_hotkey_flags(&mut config, "", &flags(&["modifier=alt_right"])).unwrap();
        assert_eq!(config.hotkeys.modifier.as_deref(), Some("alt_right"));
    }

    #[test]
    fn a_game_config_layers_its_keys_and_layout_over_the_machine_section() {
        let mut config = app_config::parse(
            r#"
            [machines.c64.keys]
            "KC_SPC" = "space"
            "KC_A" = "a"
            "#,
        )
        .unwrap();
        let game = game_config::parse(
            r#"
            layout = "symbolic"
            [keys]
            "KC_SPC" = "joy1_fire1"
            "#,
        )
        .unwrap();
        apply_game_config(&mut config, "c64", &game);
        let section = &config.machines["c64"];
        assert_eq!(section.keys["KC_SPC"].as_slice(), ["joy1_fire1"]);
        assert_eq!(
            section.keys["KC_A"].as_slice(),
            ["a"],
            "other bindings stay"
        );
        assert_eq!(section.input.layout.as_deref(), Some("symbolic"));
    }

    #[test]
    fn a_game_config_layers_its_hotkeys_over_the_machine_section_and_the_flags_beat_it() {
        let mut config = app_config::parse(
            r#"
            [hotkeys]
            pause = "pause"

            [machines.c64.hotkeys]
            quick_save = "f9"
            "#,
        )
        .unwrap();
        let game = game_config::parse(
            r#"
            [hotkeys]
            quick_save = "ctrl+q"
            reset = "none"
            "#,
        )
        .unwrap();
        apply_game_config(&mut config, "c64", &game);
        apply_hotkey_flags(&mut config, "c64", &flags(&["quick_save=ctrl+w"])).unwrap();
        let hotkeys = crate::gui::hotkeys::Hotkeys::resolve(&config, "c64").unwrap();
        let chords = |name: &str| -> Vec<String> {
            hotkeys
                .chords_for(crate::gui::hotkeys::Action::from_name(name).unwrap())
                .iter()
                .map(|c| c.to_text().to_lowercase())
                .collect()
        };
        assert_eq!(chords("quick_save").len(), 1);
        assert!(
            chords("quick_save")[0].contains('w'),
            "--hotkey wins over the game"
        );
        assert!(chords("reset").is_empty(), "the game unbinds the default");
        assert_eq!(chords("pause").len(), 1, "other bindings stay");
    }

    #[test]
    fn a_game_config_layers_its_pad_buttons() {
        let mut config = app_config::parse(
            r#"
            [pads.1]
            south = "space"
            east = "none"
            "#,
        )
        .unwrap();
        let game = game_config::parse(
            r#"
            [pads]
            deadzone = 0.3
            [pads.1]
            south = "KeyA"
            "#,
        )
        .unwrap();
        apply_game_config(&mut config, "m", &game);
        let section = &config.machines["m"];
        assert_eq!(section.pads.deadzone, Some(0.3));
        assert_eq!(section.pads.slots["1"]["south"].as_slice(), ["KeyA"]);
        assert!(
            !section.pads.slots["1"].contains_key("east"),
            "the shared [pads] layer is untouched; the machine layer only holds the game's buttons"
        );
        assert_eq!(config.pads.slots["1"]["south"].as_slice(), ["space"]);
    }

    #[test]
    fn game_symbol_files_come_before_the_command_lines() {
        let game = game_config::parse(r#"symbols = ["a.sym", "b.sym"]"#).unwrap();
        assert_eq!(
            symbol_files(&game, &flags(&["c.sym"])),
            ["a.sym", "b.sym", "c.sym"]
        );
        assert_eq!(
            symbol_files(&GameConfig::default(), &flags(&["c.sym"])),
            ["c.sym"]
        );
    }
}
