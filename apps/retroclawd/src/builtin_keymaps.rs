//! The keymap files that ship with the program (`assets/keymaps/`), found by name: `--keymap de` or `[input] keymap = "de"`.
//! A file of that name next to the config (or in the working directory for `--keymap`) wins over the shipped one.

const SHIPPED: &[(&str, &str)] = &[
    ("da", include_str!("../../../assets/keymaps/da.toml")),
    ("de", include_str!("../../../assets/keymaps/de.toml")),
    ("es", include_str!("../../../assets/keymaps/es.toml")),
    ("fr", include_str!("../../../assets/keymaps/fr.toml")),
    ("no", include_str!("../../../assets/keymaps/no.toml")),
    ("sv", include_str!("../../../assets/keymaps/sv.toml")),
    ("uk", include_str!("../../../assets/keymaps/uk.toml")),
    ("us", include_str!("../../../assets/keymaps/us.toml")),
    ("us-c64", include_str!("../../../assets/keymaps/us-c64.toml")),
];

/// The text of the shipped keymap called `name` (with or without `.toml`).
pub(crate) fn find(name: &str) -> Option<&'static str> {
    let name = name.strip_suffix(".toml").unwrap_or(name);
    SHIPPED.iter().find(|(n, _)| *n == name).map(|(_, text)| *text)
}

/// The names of the shipped keymaps.
pub(crate) fn names() -> impl Iterator<Item = &'static str> {
    SHIPPED.iter().map(|(n, _)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_keymap_parses_and_the_requested_languages_are_there() {
        for (name, text) in SHIPPED {
            app_config::parse_keymap(text).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for wanted in ["uk", "us", "es", "de", "fr", "sv", "no", "da"] {
            assert!(find(wanted).is_some(), "{wanted}");
        }
        assert!(find("de.toml").is_some() && find("klingon").is_none());
        assert_eq!(names().count(), SHIPPED.len());
    }

    #[test]
    fn the_language_keymaps_use_the_symbolic_layout() {
        for name in ["uk", "us", "es", "de", "fr", "sv", "no", "da"] {
            let file = app_config::parse_keymap(find(name).unwrap()).unwrap();
            assert_eq!(file.layout.as_deref(), Some("symbolic"), "{name}");
        }
    }
}
