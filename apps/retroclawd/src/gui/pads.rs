//! Gamepad routing: which emulated key(s) a pad button becomes.
//!
//! Configured in `retroclawd.toml` under `[pads]` (every machine) and
//! `[machines.<id>.pads]` (per machine, layered binding by binding):
//! `[pads.1]` is the first connected pad, `[pads.2]` the second, each holding
//! `button = "machine key"` pairs (a list presses several keys, `"none"`
//! unbinds). The right side uses the machine's own key names, the same ones
//! `[keys]` takes (`retroclawd <machine> --list-keys`), so a pad drives
//! joysticks and keyboard keys alike.
//!
//! Without any configuration pad 1 drives the machine's `Joy1*` keys and pad
//! 2 its `Joy2*` keys (D-pad and left stick for directions, South/East for
//! `Fire1`, West/North for `Fire2`/`Fire3` where the machine has them) -- for
//! every machine whose plugin reports those names (ZX Spectrum, CPC, C64).
//!
//! `PadRouter` is the Bevy-free part: given each pad's button state once per
//! frame it returns which machine keys went up and down; `KeyRouter::apply`
//! turns that into events so a key held from the keyboard and a pad at the same
//! time is only released when both let go.

use std::collections::{HashMap, HashSet};

use app_config::AppConfig;
use bevy::prelude::Resource;

use super::keys::{KeyNames, resolve_targets};

/// Pad slots (connection order) the config accepts.
pub(crate) const MAX_PADS: u8 = 4;

/// Default for `deadzone`: how far a stick must be pushed to count as a
/// direction.
pub(crate) const DEFAULT_DEADZONE: f32 = 0.5;

/// One digital input of a pad. Sticks are digitised: `Stick*` is the left
/// stick pushed past the deadzone in that direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum PadButton {
    Up,
    Down,
    Left,
    Right,
    South,
    East,
    West,
    North,
    L1,
    R1,
    L2,
    R2,
    Select,
    Start,
    L3,
    R3,
    StickUp,
    StickDown,
    StickLeft,
    StickRight,
}

impl PadButton {
    pub(crate) const ALL: [PadButton; 20] = [
        PadButton::Up,
        PadButton::Down,
        PadButton::Left,
        PadButton::Right,
        PadButton::South,
        PadButton::East,
        PadButton::West,
        PadButton::North,
        PadButton::L1,
        PadButton::R1,
        PadButton::L2,
        PadButton::R2,
        PadButton::Select,
        PadButton::Start,
        PadButton::L3,
        PadButton::R3,
        PadButton::StickUp,
        PadButton::StickDown,
        PadButton::StickLeft,
        PadButton::StickRight,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            PadButton::Up => "up",
            PadButton::Down => "down",
            PadButton::Left => "left",
            PadButton::Right => "right",
            PadButton::South => "south",
            PadButton::East => "east",
            PadButton::West => "west",
            PadButton::North => "north",
            PadButton::L1 => "l1",
            PadButton::R1 => "r1",
            PadButton::L2 => "l2",
            PadButton::R2 => "r2",
            PadButton::Select => "select",
            PadButton::Start => "start",
            PadButton::L3 => "l3",
            PadButton::R3 => "r3",
            PadButton::StickUp => "stick_up",
            PadButton::StickDown => "stick_down",
            PadButton::StickLeft => "stick_left",
            PadButton::StickRight => "stick_right",
        }
    }

    /// Parses a config button name: case, `-` and `_` do not matter, and the
    /// Xbox letters (`a`/`b`/`x`/`y` = south/east/west/north) are accepted.
    pub(crate) fn parse(text: &str) -> Option<PadButton> {
        let key = text.trim().to_ascii_lowercase().replace('-', "_");
        let alias = match key.as_str() {
            "a" => "south",
            "b" => "east",
            "x" => "west",
            "y" => "north",
            "dpad_up" => "up",
            "dpad_down" => "down",
            "dpad_left" => "left",
            "dpad_right" => "right",
            other => other,
        };
        PadButton::ALL.into_iter().find(|b| b.name() == alias)
    }

    fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

/// The buttons of one pad held right now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PadState(u32);

impl PadState {
    pub(crate) fn set(&mut self, button: PadButton, down: bool) {
        if down {
            self.0 |= button.bit();
        } else {
            self.0 &= !button.bit();
        }
    }

    pub(crate) fn is_down(self, button: PadButton) -> bool {
        self.0 & button.bit() != 0
    }

    /// Adds the four `Stick*` directions for a left-stick position
    /// (`x` right-positive, `y` up-positive, both -1..1).
    pub(crate) fn set_stick(&mut self, x: f32, y: f32, deadzone: f32) {
        self.set(PadButton::StickLeft, x < -deadzone);
        self.set(PadButton::StickRight, x > deadzone);
        self.set(PadButton::StickUp, y > deadzone);
        self.set(PadButton::StickDown, y < -deadzone);
    }
}

/// The resolved bindings for one machine: (pad slot, button) -> machine key
/// names (empty = the button does nothing).
#[derive(Debug, Clone, Default)]
pub(crate) struct PadBindings {
    pub(crate) deadzone: f32,
    map: HashMap<(u8, PadButton), Vec<String>>,
}

impl PadBindings {
    /// Built-in bindings: pad `n` drives the machine's `Joy{n}*` keys, for
    /// the names the machine reports. A machine that reports no names, or no
    /// joystick keys (the ZX81), gets none.
    fn defaults(names: &KeyNames) -> HashMap<(u8, PadButton), Vec<String>> {
        let mut map = HashMap::new();
        for slot in 1..=2u8 {
            let key = |suffix: &str| {
                let wanted = format!("Joy{slot}{suffix}");
                names
                    .machine
                    .iter()
                    .find(|info| info.name == wanted)
                    .map(|info| info.name.clone())
            };
            let mut bind = |button: PadButton, suffix: &str| {
                if let Some(name) = key(suffix) {
                    map.insert((slot, button), vec![name]);
                }
            };
            bind(PadButton::Up, "Up");
            bind(PadButton::StickUp, "Up");
            bind(PadButton::Down, "Down");
            bind(PadButton::StickDown, "Down");
            bind(PadButton::Left, "Left");
            bind(PadButton::StickLeft, "Left");
            bind(PadButton::Right, "Right");
            bind(PadButton::StickRight, "Right");
            bind(PadButton::South, "Fire1");
            bind(PadButton::East, "Fire1");
            bind(PadButton::West, "Fire2");
            bind(PadButton::North, "Fire3");
        }
        map
    }

    /// Defaults, then `[pads]`, then `[machines.<machine>.pads]`.
    pub(crate) fn resolve(
        config: &AppConfig,
        machine: &str,
        names: &KeyNames,
    ) -> Result<PadBindings, String> {
        let machine_section = format!("[machines.{machine}.pads");
        let machine_pads = config.machines.get(machine).map(|m| &m.pads);

        let deadzone = machine_pads
            .and_then(|p| p.deadzone)
            .or(config.pads.deadzone)
            .unwrap_or(DEFAULT_DEADZONE);
        if !(deadzone > 0.0 && deadzone < 1.0) {
            return Err(format!(
                "--config: pads deadzone must be between 0 and 1 (exclusive), not {deadzone}"
            ));
        }

        let mut map = PadBindings::defaults(names);
        let layers = [
            (Some(&config.pads), "[pads".to_string(), true),
            (machine_pads, machine_section, false),
        ];
        for (layer, section, shared) in layers {
            let Some(layer) = layer else { continue };
            let mut slots: Vec<_> = layer.slots.iter().collect();
            slots.sort_by(|a, b| a.0.cmp(b.0));
            for (slot_text, buttons) in slots {
                let slot: u8 = slot_text
                    .parse()
                    .ok()
                    .filter(|n| (1..=MAX_PADS).contains(n))
                    .ok_or_else(|| {
                        format!(
                            "{section}]: \"{slot_text}\" is not a pad slot -- use [pads.1] to \
                             [pads.{MAX_PADS}] (or `deadzone = 0.5`)"
                        )
                    })?;
                let section_name = format!("{section}.{slot}]");
                let mut buttons: Vec<_> = buttons.iter().collect();
                buttons.sort_by(|a, b| a.0.cmp(b.0));
                for (button_text, targets) in buttons {
                    let button = PadButton::parse(button_text).ok_or_else(|| {
                        format!(
                            "{section_name}: unknown pad button \"{button_text}\" (buttons: {})",
                            PadButton::ALL.map(PadButton::name).join(", ")
                        )
                    })?;
                    let Some(resolved) = resolve_targets(
                        targets.as_slice(),
                        button_text,
                        &section_name,
                        machine,
                        names,
                        shared,
                    )?
                    else {
                        continue;
                    };
                    map.insert((slot, button), resolved);
                }
            }
        }
        Ok(PadBindings { deadzone, map })
    }

    fn get(&self, slot: u8, button: PadButton) -> Option<&Vec<String>> {
        self.map.get(&(slot, button))
    }

    /// Plain-text table for `--list-keys`.
    pub(crate) fn describe(&self) -> String {
        if self.map.is_empty() {
            return "(no pad bindings: this machine has no joystick keys; bind buttons in \
                    [pads.1])\n"
                .to_string();
        }
        let mut rows: Vec<(u8, PadButton, String)> = self
            .map
            .iter()
            .map(|((slot, button), targets)| {
                let target = if targets.is_empty() {
                    "(nothing)".to_string()
                } else {
                    targets.join(" + ")
                };
                (*slot, *button, target)
            })
            .collect();
        rows.sort();
        rows.iter()
            .map(|(slot, button, target)| format!("pad {slot} {:<12} -> {target}\n", button.name()))
            .collect()
    }
}

/// Lists the button names `[pads.N]` accepts.
pub(crate) fn describe_buttons() -> String {
    PadButton::ALL.map(PadButton::name).join(", ") + "\n(also a/b/x/y for south/east/west/north)\n"
}

/// Which connected pad is slot 1, 2, ...: the first pad seen takes slot 1, a
/// pad plugged in later takes the lowest free slot, and unplugging one leaves
/// the others where they are (so pad 2 does not silently become pad 1 in the
/// middle of a game). `id` is any stable per-pad number (the Bevy entity).
#[derive(Resource, Debug, Default)]
pub(crate) struct PadSlots {
    slots: Vec<Option<u64>>,
}

impl PadSlots {
    /// Updates the slots for the pads present now; returns `(slot, id)` for
    /// every pad that has one (pads beyond `MAX_PADS` are ignored).
    pub(crate) fn assign(&mut self, present: &[u64]) -> Vec<(u8, u64)> {
        for slot in &mut self.slots {
            if slot.is_some_and(|id| !present.contains(&id)) {
                *slot = None;
            }
        }
        let mut fresh: Vec<u64> = present
            .iter()
            .copied()
            .filter(|id| !self.slots.contains(&Some(*id)))
            .collect();
        fresh.sort_unstable();
        for id in fresh {
            if let Some(free) = self.slots.iter_mut().find(|slot| slot.is_none()) {
                *free = Some(id);
            } else if self.slots.len() < usize::from(MAX_PADS) {
                self.slots.push(Some(id));
            }
        }
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, id)| id.map(|id| (i as u8 + 1, id)))
            .collect()
    }
}

#[derive(Resource, Debug)]
pub(crate) struct PadRouter {
    bindings: PadBindings,
    /// Buttons currently down and the machine keys each pressed (so a
    /// release undoes its own press even if the binding table were swapped).
    held: HashMap<(u8, PadButton), Vec<String>>,
}

impl PadRouter {
    pub(crate) fn new(bindings: PadBindings) -> Self {
        PadRouter {
            bindings,
            held: HashMap::new(),
        }
    }

    pub(crate) fn deadzone(&self) -> f32 {
        self.bindings.deadzone
    }

    /// Compares the pads (slot, state) to what was sent so far; returns the
    /// machine keys to release and to press, ready for `KeyRouter::apply`.
    /// A pad missing from `pads` (unplugged) releases everything it held.
    pub(crate) fn route(
        &mut self,
        pads: &[(u8, PadState)],
    ) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
        let down: HashSet<(u8, PadButton)> = pads
            .iter()
            .flat_map(|(slot, state)| {
                PadButton::ALL
                    .into_iter()
                    .filter(|b| state.is_down(*b))
                    .map(|b| (*slot, b))
            })
            .collect();

        let mut gone: Vec<(u8, PadButton)> = self
            .held
            .keys()
            .copied()
            .filter(|key| !down.contains(key))
            .collect();
        gone.sort();
        let releases = gone
            .into_iter()
            .filter_map(|key| self.held.remove(&key))
            .collect();

        let mut fresh: Vec<(u8, PadButton)> = down
            .into_iter()
            .filter(|key| !self.held.contains_key(key))
            .collect();
        fresh.sort();
        let mut presses = Vec::new();
        for (slot, button) in fresh {
            if let Some(names) = self.bindings.get(slot, button) {
                presses.push(names.clone());
                self.held.insert((slot, button), names.clone());
            }
        }
        (releases, presses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(names: &[&str]) -> KeyNames {
        KeyNames::only(&plugin_api::key_infos(names.iter().copied()))
    }

    fn cfg(text: &str) -> AppConfig {
        app_config::parse(text).unwrap()
    }

    const JOY: &[&str] = &[
        "Joy1Up",
        "Joy1Down",
        "Joy1Left",
        "Joy1Right",
        "Joy1Fire1",
        "Joy1Fire2",
        "Joy2Up",
        "Joy2Fire1",
        "KeyA",
        "Space",
    ];

    fn state(buttons: &[PadButton]) -> PadState {
        let mut s = PadState::default();
        for b in buttons {
            s.set(*b, true);
        }
        s
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pad_slots_keep_their_numbers_when_a_pad_is_unplugged() {
        let mut slots = PadSlots::default();
        assert_eq!(slots.assign(&[7, 3]), [(1, 3), (2, 7)]);
        // Pad 3 leaves: pad 7 stays slot 2, it does not slide down to slot 1.
        assert_eq!(slots.assign(&[7]), [(2, 7)]);
        // A newcomer takes the free slot 1.
        assert_eq!(slots.assign(&[7, 9]), [(1, 9), (2, 7)]);
        // More pads than slots: the extras are ignored.
        let many: Vec<u64> = (20..30).collect();
        let assigned = slots.assign(&[vec![7, 9], many].concat());
        assert_eq!(assigned.len(), usize::from(MAX_PADS));
        assert!(assigned.contains(&(2, 7)) && assigned.contains(&(1, 9)));
    }

    #[test]
    fn button_names_parse_loosely() {
        assert_eq!(PadButton::parse("South"), Some(PadButton::South));
        assert_eq!(PadButton::parse("a"), Some(PadButton::South));
        assert_eq!(PadButton::parse("stick-up"), Some(PadButton::StickUp));
        assert_eq!(PadButton::parse("dpad_left"), Some(PadButton::Left));
        assert_eq!(PadButton::parse("turbo"), None);
        // Every button's own name parses back to itself.
        for b in PadButton::ALL {
            assert_eq!(PadButton::parse(b.name()), Some(b));
        }
    }

    #[test]
    fn sticks_become_directions_past_the_deadzone() {
        let mut s = PadState::default();
        s.set_stick(0.4, 0.0, 0.5);
        assert_eq!(s, PadState::default());
        s.set_stick(-0.9, 0.7, 0.5);
        assert!(s.is_down(PadButton::StickLeft) && s.is_down(PadButton::StickUp));
        assert!(!s.is_down(PadButton::StickRight) && !s.is_down(PadButton::StickDown));
        // Going back inside the deadzone clears them.
        s.set_stick(0.0, 0.0, 0.5);
        assert_eq!(s, PadState::default());
    }

    #[test]
    fn defaults_drive_the_joystick_keys_the_machine_reports() {
        let b = PadBindings::resolve(&AppConfig::default(), "m", &info(JOY)).unwrap();
        assert_eq!(b.get(1, PadButton::Up).unwrap(), &names(&["Joy1Up"]));
        assert_eq!(
            b.get(1, PadButton::StickLeft).unwrap(),
            &names(&["Joy1Left"])
        );
        assert_eq!(b.get(1, PadButton::South).unwrap(), &names(&["Joy1Fire1"]));
        assert_eq!(b.get(1, PadButton::West).unwrap(), &names(&["Joy1Fire2"]));
        // The machine has no Fire3 or Joy2Down: those stay unbound.
        assert!(b.get(1, PadButton::North).is_none());
        assert!(b.get(2, PadButton::Down).is_none());
        assert_eq!(b.get(2, PadButton::Up).unwrap(), &names(&["Joy2Up"]));
        // No joystick keys at all (ZX81) or no reported names: nothing bound.
        let none = PadBindings::resolve(&AppConfig::default(), "m", &info(&["KeyA"])).unwrap();
        assert!(none.describe().contains("no pad bindings"));
        let unreported = PadBindings::resolve(&AppConfig::default(), "m", &KeyNames::default());
        assert!(unreported.unwrap().get(1, PadButton::Up).is_none());
    }

    #[test]
    fn config_layers_override_defaults_and_none_unbinds() {
        let b = PadBindings::resolve(
            &cfg(
                "[pads.1]\nsouth = \"space\"\neast = \"none\"\nl1 = [\"KeyA\", \"joy1up\"]\n\
                  [machines.m.pads.1]\nsouth = \"KeyA\"\n[machines.m.pads]\ndeadzone = 0.3",
            ),
            "m",
            &info(JOY),
        )
        .unwrap();
        // Machine layer beats the global one, which beat the default.
        assert_eq!(b.get(1, PadButton::South).unwrap(), &names(&["KeyA"]));
        assert_eq!(b.get(1, PadButton::East).unwrap(), &Vec::<String>::new());
        assert_eq!(
            b.get(1, PadButton::L1).unwrap(),
            &names(&["KeyA", "Joy1Up"])
        );
        assert_eq!(b.deadzone, 0.3);
    }

    #[test]
    fn mistakes_are_explained() {
        let err = |text: &str| PadBindings::resolve(&cfg(text), "m", &info(JOY)).unwrap_err();
        assert!(err("[pads.9]\nsouth = \"space\"").contains("not a pad slot"));
        assert!(err("[pads.deadzonee]\nsouth = \"space\"").contains("not a pad slot"));
        assert!(err("[pads.1]\nturbo = \"space\"").contains("unknown pad button \"turbo\""));
        assert!(err("[pads.1]\nsouth = \"nonesuch\"").contains("unknown machine key"));
        assert!(err("[pads]\ndeadzone = 1.5").contains("deadzone"));
        assert!(err("[pads.1]\nsouth = [\"none\", \"space\"]").contains("mixes"));
    }

    #[test]
    fn the_shared_layer_skips_keys_only_other_machines_have() {
        // `[pads.1]` is for every machine: Fire2 exists elsewhere, not here.
        let names = KeyNames {
            machine: plugin_api::key_infos(["Joy1Up", "Joy1Fire1"]),
            all: plugin_api::key_infos(["Joy1Up", "Joy1Fire1", "Joy1Fire2"]),
        };
        let shared =
            PadBindings::resolve(&cfg("[pads.1]\nl1 = \"Joy1Fire2\""), "m", &names).unwrap();
        assert!(shared.get(1, PadButton::L1).is_none());
        // The machine's own section is strict.
        assert!(
            PadBindings::resolve(&cfg("[machines.m.pads.1]\nl1 = \"Joy1Fire2\""), "m", &names)
                .is_err()
        );
    }

    #[test]
    fn the_router_presses_and_releases_with_the_buttons() {
        let b = PadBindings::resolve(&AppConfig::default(), "m", &info(JOY)).unwrap();
        let mut r = PadRouter::new(b);

        let (rel, press) = r.route(&[(1, state(&[PadButton::Up, PadButton::South]))]);
        assert!(rel.is_empty());
        assert_eq!(press, [names(&["Joy1Up"]), names(&["Joy1Fire1"])]);
        // Held: nothing new.
        let (rel, press) = r.route(&[(1, state(&[PadButton::Up, PadButton::South]))]);
        assert!(rel.is_empty() && press.is_empty());
        // Let go of one.
        let (rel, press) = r.route(&[(1, state(&[PadButton::Up]))]);
        assert_eq!(rel, [names(&["Joy1Fire1"])]);
        assert!(press.is_empty());
        // Unplugging the pad releases what it held.
        let (rel, press) = r.route(&[]);
        assert_eq!(rel, [names(&["Joy1Up"])]);
        assert!(press.is_empty());
        // Unbound buttons do nothing, and are not remembered as held.
        let (rel, press) = r.route(&[(1, state(&[PadButton::Start]))]);
        assert!(rel.is_empty() && press.is_empty());
        let (rel, _) = r.route(&[]);
        assert!(rel.is_empty());
    }

    #[test]
    fn two_sources_on_one_machine_key_release_it_only_when_both_let_go() {
        // Stick and D-pad both bound to Joy1Left: the shared reference count in
        // `KeyRouter::apply` keeps the key down until both are released.
        let b = PadBindings::resolve(&AppConfig::default(), "m", &info(JOY)).unwrap();
        let mut pads = PadRouter::new(b);
        let mut keys = super::super::keys::KeyRouter::new(Default::default());

        let (rel, press) = pads.route(&[(1, state(&[PadButton::Left, PadButton::StickLeft]))]);
        assert_eq!(keys.apply(rel, press), [("Joy1Left".to_string(), true)]);
        let (rel, press) = pads.route(&[(1, state(&[PadButton::StickLeft]))]);
        assert!(keys.apply(rel, press).is_empty());
        let (rel, press) = pads.route(&[(1, state(&[]))]);
        assert_eq!(keys.apply(rel, press), [("Joy1Left".to_string(), false)]);
    }
}
