//! The gamepad layouts that ship with the program (`assets/controllers/layouts.toml`): which machine key each pad button
//! presses, per machine family. They are the bottom layer of `PadBindings::resolve`; `[pads]` and `[machines.<id>.pads]`
//! in `retroclawd.toml` replace them button by button.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use super::keys::KeyNames;
use super::pads::PadButton;

const SHIPPED: &str = include_str!("../../../../assets/controllers/layouts.toml");

/// Pads a layout is applied to.
const SLOTS: u8 = 2;

#[derive(Deserialize)]
struct Layouts {
    layout: Vec<Layout>,
}

#[derive(Deserialize)]
struct Layout {
    /// Machine ids; `"*"` is every machine without a layout of its own.
    machines: Vec<String>,
    buttons: BTreeMap<String, OneOrMany>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn keys(&self) -> Vec<&str> {
        match self {
            OneOrMany::One(key) => vec![key.as_str()],
            OneOrMany::Many(keys) => keys.iter().map(String::as_str).collect(),
        }
    }
}

fn parse(text: &str) -> Result<Layouts, String> {
    toml::from_str(text).map_err(|e| format!("controllers/layouts.toml: {e}"))
}

/// The shipped layout of `machine`: its own entry, else the `"*"` one. Keys the machine does not have are left out of a
/// binding (and a binding left with none is not made), so a layout can name more than a machine offers.
pub(super) fn bindings(machine: &str, names: &KeyNames) -> HashMap<(u8, PadButton), Vec<String>> {
    let layouts = parse(SHIPPED).expect("the shipped pad layouts are valid (tested)");
    let find = |wanted: &str| {
        layouts
            .layout
            .iter()
            .find(|l| l.machines.iter().any(|m| m == wanted))
    };
    let mut map = HashMap::new();
    let Some(layout) = find(machine).or_else(|| find("*")) else {
        return map;
    };
    for slot in 1..=SLOTS {
        for (button_text, targets) in &layout.buttons {
            let Some(button) = PadButton::parse(button_text) else {
                continue;
            };
            let keys: Vec<String> = targets
                .keys()
                .into_iter()
                .filter_map(|template| {
                    let wanted = template.replace("{n}", &slot.to_string());
                    names
                        .machine
                        .iter()
                        .find(|info| info.name == wanted)
                        .map(|info| info.name.clone())
                })
                .collect();
            if !keys.is_empty() {
                map.insert((slot, button), keys);
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> KeyNames {
        KeyNames {
            machine: plugin_api::key_infos(list.iter().copied()),
            ..KeyNames::default()
        }
    }

    #[test]
    fn the_shipped_file_parses_and_names_only_real_pad_buttons() {
        let layouts = parse(SHIPPED).unwrap();
        assert!(layouts.layout.iter().any(|l| l.machines == ["*"]));
        for layout in &layouts.layout {
            for button in layout.buttons.keys() {
                assert!(
                    PadButton::parse(button).is_some(),
                    "{:?}: unknown pad button {button}",
                    layout.machines
                );
            }
        }
        // a machine is in at most one layout
        let mut seen = std::collections::HashSet::new();
        for m in layouts.layout.iter().flat_map(|l| &l.machines) {
            assert!(seen.insert(m.clone()), "{m} has two layouts");
        }
    }

    #[test]
    fn the_genesis_layout_puts_a_b_c_on_the_bottom_row_and_start_and_mode_on_the_centre_buttons() {
        let n = names(&[
            "Joy1Up",
            "Joy1A",
            "Joy1B",
            "Joy1C",
            "Joy1X",
            "Joy1Y",
            "Joy1Z",
            "Joy1Start",
            "Joy1Mode",
            "Joy2A",
            "Joy2Start",
        ]);
        let map = bindings("genesis", &n);
        let one = |s: &str| vec![s.to_string()];
        assert_eq!(map[&(1, PadButton::West)], one("Joy1A"));
        assert_eq!(map[&(1, PadButton::South)], one("Joy1B"));
        assert_eq!(map[&(1, PadButton::East)], one("Joy1C"));
        assert_eq!(map[&(1, PadButton::L1)], one("Joy1X"));
        assert_eq!(map[&(1, PadButton::North)], one("Joy1Y"));
        assert_eq!(map[&(1, PadButton::R1)], one("Joy1Z"));
        assert_eq!(map[&(1, PadButton::Start)], one("Joy1Start"));
        assert_eq!(map[&(1, PadButton::Select)], one("Joy1Mode"));
        assert_eq!(map[&(2, PadButton::West)], one("Joy2A"));
        assert_eq!(map[&(1, PadButton::StickUp)], one("Joy1Up"));
        assert!(
            !map.contains_key(&(2, PadButton::Up)),
            "pad 2 has no Up key here"
        );
        assert_eq!(
            bindings("segacd", &n),
            map,
            "the Sega CD shares the Genesis layout"
        );
    }

    #[test]
    fn a_machine_without_its_own_layout_gets_one_fire_button_on_every_face_and_shoulder_button() {
        let n = names(&["Joy1Up", "Joy1Fire1"]);
        let map = bindings("c64", &n);
        for b in [
            PadButton::South,
            PadButton::East,
            PadButton::West,
            PadButton::North,
            PadButton::L1,
            PadButton::R2,
        ] {
            assert_eq!(map[&(1, b)], vec!["Joy1Fire1".to_string()], "{b:?}");
        }
        assert!(bindings("c64", &KeyNames::default()).is_empty());
    }

    #[test]
    fn the_atari_ste_pad_buttons_and_the_cpc_fire_buttons_are_spread_over_the_face_buttons() {
        let ste = names(&[
            "Joy1Fire1",
            "Joy1PadA",
            "Joy1PadB",
            "Joy1PadC",
            "Joy1PadOption",
            "Joy1PadPause",
        ]);
        let map = bindings("atari-ste", &ste);
        assert_eq!(map[&(1, PadButton::South)], ["Joy1Fire1", "Joy1PadA"]);
        assert_eq!(map[&(1, PadButton::Start)], ["Joy1PadPause"]);
        // the plain ST has the generic layout
        assert_eq!(
            bindings("atari-st", &names(&["Joy1Fire1"]))[&(1, PadButton::West)],
            ["Joy1Fire1"]
        );
        let cpc = names(&["Joy1Fire1", "Joy1Fire2", "Joy1Fire3"]);
        let map = bindings("cpc6128", &cpc);
        assert_eq!(
            (
                map[&(1, PadButton::South)][0].as_str(),
                map[&(1, PadButton::East)][0].as_str(),
                map[&(1, PadButton::West)][0].as_str()
            ),
            ("Joy1Fire1", "Joy1Fire2", "Joy1Fire3")
        );
    }
}
