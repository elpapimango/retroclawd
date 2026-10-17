//! Machine keyboard routing: which emulated key(s) a host key press becomes.
//!
//! Configured in `retroclawd.toml` under `[keys]` (every machine) and
//! `[machines.<id>.keys]` (per machine, layered over `[keys]` binding by
//! binding): `host key or chord = "machine key"`, a list of machine keys
//! (pressed together), or `"none"` (the key does nothing). Host chords are
//! the same syntax as hotkeys (`semicolon`, `shift+2`, `ctrl+f1`); machine key
//! names are checked at startup against the names the machine plugin reports
//! (`MachineProfile::key_names`), so a typo is an error listing the valid
//! names. A host key with no binding sends its own name (`KeyA`,
//! `ArrowLeft`, ...), which is what every plugin understands natively.
//!
//! `KeyRouter` is the Bevy-free part: given the keyboard state once per
//! frame it returns the machine key events to send. It remembers what each key
//! pressed (so a release always undoes that press, whatever modifiers are held
//! by then), keeps a reference count per machine key (two host keys bound to
//! one machine key never release it early), combines both Shift keys and the
//! arrow keys into the one "machine Shift is down" state the machines expect
//! (see `SHIFT_SOURCE_ARROWS`), and implements chord bindings: while a chord
//! like `shift+2` is held, the modifiers in it are *consumed* -- the real
//! Shift is not sent to the machine, so `shift+2 = "At"` gives exactly `At`
//! and not `At` plus a stray Shift.

use std::collections::{HashMap, HashSet};

use app_config::{AppConfig, OneOrMany};
use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;
use bevy::prelude::Resource;
use plugin_api::MachineKeyInfo;

use super::hotkeys::{
    ALT_KEYS, CTRL_KEYS, Chord, HotkeyState, Hotkeys, META_KEYS, SHIFT_KEYS, key_display_name,
    parse_chord,
};

/// The arrow keys also hold the machine's Shift: the ZX81/Spectrum cursor
/// keys are Shift+5..8 and every machine plugin's key handler implements the
/// machine-specific half (unchanged from before bindings existed).
const SHIFT_SOURCE_ARROWS: [KeyCode; 4] = [
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Mods {
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
}

impl Mods {
    fn of_chord(chord: &Chord) -> Mods {
        Mods {
            ctrl: chord.ctrl,
            shift: chord.shift,
            alt: chord.alt,
            meta: chord.meta,
        }
    }

    fn from_keys(held: &HashSet<KeyCode>) -> Mods {
        let any = |keys: &[KeyCode; 2]| keys.iter().any(|k| held.contains(k));
        Mods {
            ctrl: any(&CTRL_KEYS),
            shift: any(&SHIFT_KEYS),
            alt: any(&ALT_KEYS),
            meta: any(&META_KEYS),
        }
    }

    fn union(self, other: Mods) -> Mods {
        Mods {
            ctrl: self.ctrl || other.ctrl,
            shift: self.shift || other.shift,
            alt: self.alt || other.alt,
            meta: self.meta || other.meta,
        }
    }

    fn is_empty(self) -> bool {
        self == Mods::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModClass {
    Ctrl,
    Shift,
    Alt,
    Meta,
}

fn mod_class(key: KeyCode) -> Option<ModClass> {
    if CTRL_KEYS.contains(&key) {
        Some(ModClass::Ctrl)
    } else if SHIFT_KEYS.contains(&key) {
        Some(ModClass::Shift)
    } else if ALT_KEYS.contains(&key) {
        Some(ModClass::Alt)
    } else if META_KEYS.contains(&key) {
        Some(ModClass::Meta)
    } else {
        None
    }
}

fn consumed(mods: Mods, class: ModClass) -> bool {
    match class {
        ModClass::Ctrl => mods.ctrl,
        ModClass::Shift => mods.shift,
        ModClass::Alt => mods.alt,
        ModClass::Meta => mods.meta,
    }
}

/// The name a host key sends when nothing is bound to it.
fn default_name(key: KeyCode) -> String {
    format!("{key:?}")
}

/// The key names `KeyBindings::resolve` validates against.
#[derive(Debug, Clone, Default)]
pub(crate) struct KeyNames {
    /// The selected machine's own names (empty: it reports none, so values
    /// pass through unchecked).
    pub(crate) machine: Vec<MachineKeyInfo>,
    /// Every discovered machine's names. A `[keys]` binding (the layer meant
    /// for all machines) naming a key this machine lacks is skipped quietly
    /// when another machine has it, and is an error only if no machine does.
    pub(crate) all: Vec<MachineKeyInfo>,
}

impl KeyNames {
    /// Machine and "all machines" are the same list (tests, single machine).
    #[cfg(test)]
    pub(crate) fn only(names: &[MachineKeyInfo]) -> Self {
        KeyNames {
            machine: names.to_vec(),
            all: names.to_vec(),
        }
    }
}

/// How a host key becomes a machine key when nothing is bound to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Layout {
    /// The host key at a physical position is the machine key at that
    /// position (`KeyA` is the key where QWERTY has A). Today's behaviour.
    #[default]
    Positional,
    /// What the host key *types* decides: the key labelled `Z` is the
    /// machine's Z on a German or French keyboard too, and `!` is whatever
    /// the machine's `!` is, wherever the host puts it.
    Symbolic,
}

impl Layout {
    pub(crate) fn parse(text: &str) -> Result<Layout, String> {
        match text.trim().to_ascii_lowercase().as_str() {
            "positional" => Ok(Layout::Positional),
            "symbolic" => Ok(Layout::Symbolic),
            other => Err(format!(
                "unknown keyboard layout \"{other}\" (use \"positional\" or \"symbolic\")"
            )),
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Layout::Positional => "positional",
            Layout::Symbolic => "symbolic",
        }
    }
}

/// A keymap file already read and parsed, with where it came from.
#[derive(Debug, Clone)]
pub(crate) struct LoadedKeymap {
    /// Shown in error messages (the file name as given).
    pub(crate) label: String,
    /// Given with `--keymap` (layered above the config's inline `[keys]`)
    /// rather than listed in `[input] keymap` (layered below it).
    pub(crate) upper: bool,
    pub(crate) file: app_config::KeymapFile,
}

/// What a typed character does to the host's modifiers when it is turned into
/// a machine key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharKind {
    /// A letter: a held Shift still reaches the machine (a capital).
    Letter,
    /// A digit: Shift was only how this layout typed it, so it is consumed.
    Digit,
    /// Punctuation: Shift and AltGr were only how this layout typed it.
    Symbol,
}

/// Which machine key each typed ASCII character is, built from the key list
/// the plugin reports: `KeyA`..`KeyZ` for letters, `Digit0`..`Digit9` for
/// digits, and the `chars` each other key declares.
#[derive(Debug, Clone, Default)]
pub(crate) struct SymbolMap {
    map: HashMap<char, (String, CharKind)>,
}

impl SymbolMap {
    pub(crate) fn from_names(names: &[MachineKeyInfo]) -> SymbolMap {
        let mut map: HashMap<char, (String, CharKind)> = HashMap::new();
        for info in names {
            let mut put = |c: char, kind: CharKind| {
                map.entry(c).or_insert_with(|| (info.name.clone(), kind));
            };
            if let Some(letter) = info.name.strip_prefix("Key")
                && let [c @ 'A'..='Z'] = letter.chars().collect::<Vec<_>>()[..]
            {
                put(c, CharKind::Letter);
                put(c.to_ascii_lowercase(), CharKind::Letter);
            } else if let Some(digit) = info.name.strip_prefix("Digit")
                && let [c @ '0'..='9'] = digit.chars().collect::<Vec<_>>()[..]
            {
                put(c, CharKind::Digit);
            }
            for c in info.chars.chars() {
                put(c, CharKind::Symbol);
            }
        }
        SymbolMap { map }
    }

    fn get(&self, c: char) -> Option<&(String, CharKind)> {
        self.map.get(&c)
    }

    /// The key for `c`, or for the plain letter an accented one folds to (`é` types `e`) when the machine has no key for it.
    fn lookup(&self, c: char) -> Option<&(String, CharKind)> {
        self.get(c)
            .or_else(|| super::fold::ascii_fold_char(c).and_then(|plain| self.get(plain)))
    }

    /// The machine key that types `c`, if it has one.
    pub(crate) fn key_for(&self, c: char) -> Option<&str> {
        self.get(c).map(|(name, _)| name.as_str())
    }
}

/// The character each held host key typed when it went down, recorded from
/// the keyboard events (`input::record_logical_keys`). Only keys that type a
/// plain character are here (not Shift, F-keys, arrows, Enter, ...).
#[derive(Resource, Debug, Default)]
pub(crate) struct LogicalKeys {
    typed: HashMap<KeyCode, char>,
    /// Keys that went down as a dead key (an accent waiting for its letter): they type nothing by themselves.
    dead: HashSet<KeyCode>,
}

impl LogicalKeys {
    /// `key` went down as a dead key: under the symbolic layout it sends nothing (the composed letter that follows is typed).
    pub(crate) fn pressed_dead(&mut self, key: KeyCode) {
        self.typed.remove(&key);
        self.dead.insert(key);
    }

    pub(crate) fn pressed(&mut self, key: KeyCode, typed: Option<char>) {
        self.dead.remove(&key);
        match typed {
            Some(c) => {
                self.typed.insert(key, c);
            }
            None => {
                self.typed.remove(&key);
            }
        }
    }

    pub(crate) fn released(&mut self, key: KeyCode) {
        self.typed.remove(&key);
        self.dead.remove(&key);
    }

    fn get(&self, key: KeyCode) -> Option<char> {
        self.typed.get(&key).copied()
    }

    fn is_dead(&self, key: KeyCode) -> bool {
        self.dead.contains(&key)
    }
}

/// One layer of bindings while resolving: the table (if any), its section name
/// for error messages, and whether it is the all-machines kind (`shared`).
type Layer<'a> = (Option<&'a HashMap<String, OneOrMany>>, String, bool);

/// The resolved bindings for one machine: chord -> machine key names
/// (empty = the key does nothing).
#[derive(Debug, Clone, Default)]
pub(crate) struct KeyBindings {
    map: HashMap<Chord, Vec<String>>,
}

impl KeyBindings {
    /// Builds the table from `[keys]` then `[machines.<machine>.keys]`.
    /// `names` is the machine's reported key list; empty means the plugin
    /// does not report names, and values pass through unchecked. A binding on
    /// a chord that is also a (ungated) hotkey is an error -- the hotkey would
    /// swallow the key before the binding ever saw it.
    pub(crate) fn resolve(
        config: &AppConfig,
        machine: &str,
        names: &KeyNames,
        hotkeys: &Hotkeys,
        files: &[LoadedKeymap],
    ) -> Result<KeyBindings, String> {
        let machine_section = format!("[machines.{machine}.keys]");
        // Lowest first: keymap files from `[input] keymap`, then the config's
        // own `[keys]` and machine keys, then files given with `--keymap`.
        let mut layers: Vec<Layer> = Vec::new();
        let file_layers = |upper: bool, layers: &mut Vec<_>| {
            for loaded in files.iter().filter(|f| f.upper == upper) {
                layers.push((
                    Some(&loaded.file.keys),
                    format!("[keys] in {}", loaded.label),
                    true,
                ));
                layers.push((
                    loaded.file.machines.get(machine).map(|m| &m.keys),
                    format!("[machines.{machine}.keys] in {}", loaded.label),
                    false,
                ));
            }
        };
        file_layers(false, &mut layers);
        layers.push((Some(&config.keys), "[keys]".to_string(), true));
        layers.push((
            config.machines.get(machine).map(|m| &m.keys),
            machine_section,
            false,
        ));
        file_layers(true, &mut layers);
        let mut map = HashMap::new();
        for (layer, section, shared) in layers {
            let section = section.as_str();
            let Some(layer) = layer else { continue };
            // Sorted so the first error reported is the same every run.
            let mut entries: Vec<_> = layer.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            for (host, targets) in entries {
                let chord = parse_chord(host).map_err(|e| format!("{section}: {e}"))?;
                let Some(resolved) =
                    resolve_targets(targets.as_slice(), host, section, machine, names, shared)?
                else {
                    continue;
                };
                map.insert(chord, resolved);
            }
        }
        if hotkeys.modifier().is_none() {
            let mut clashes: Vec<String> = hotkeys
                .all_chords()
                .filter(|(_, chord)| map.contains_key(chord))
                .map(|(action, chord)| format!("\"{}\" ({})", chord.to_text(), action.info().name))
                .collect();
            clashes.sort();
            if let Some(first) = clashes.first() {
                return Err(format!(
                    "key binding for {first} can never fire: the hotkey takes that key first -- \
                     rebind or unbind the hotkey ([hotkeys], or \"none\")"
                ));
            }
        }
        Ok(KeyBindings { map })
    }

    /// Machine keys bound to exactly this chord, if any.
    fn get(&self, chord: &Chord) -> Option<&Vec<String>> {
        self.map.get(chord)
    }

    /// Plain-text table of the bindings for `--list-keys`.
    pub(crate) fn describe(&self) -> String {
        if self.map.is_empty() {
            return "(no key bindings: every host key sends its own name)\n".to_string();
        }
        let mut rows: Vec<(String, String)> = self
            .map
            .iter()
            .map(|(chord, targets)| {
                let target = if targets.is_empty() {
                    "(nothing)".to_string()
                } else {
                    targets.join(" + ")
                };
                (chord.to_text(), target)
            })
            .collect();
        rows.sort();
        rows.iter()
            .map(|(host, target)| format!("{host:<20} -> {target}\n"))
            .collect()
    }
}

/// Turns the right side of one binding into canonical machine key names:
/// `"none"` is an empty list (the input does nothing). `Ok(None)` means the
/// binding names a key only some *other* machine has and `shared` (the
/// all-machines layer) says to leave it out quietly; every other problem is
/// an error listing the valid names.
pub(super) fn resolve_targets(
    list: &[String],
    host: &str,
    section: &str,
    machine: &str,
    names: &KeyNames,
    shared: bool,
) -> Result<Option<Vec<String>>, String> {
    if list.is_empty() {
        return Err(format!("{section}: \"{host}\" has an empty list"));
    }
    if list.iter().any(|t| t.trim().eq_ignore_ascii_case("none")) {
        if list.len() > 1 {
            return Err(format!(
                "{section}: \"{host}\" mixes \"none\" with other keys"
            ));
        }
        return Ok(Some(Vec::new()));
    }
    let mut keys = Vec::new();
    let mut skip = false;
    for target in list {
        match canonical_machine_key(target, host, section, machine, &names.machine) {
            Ok(name) => keys.push(name),
            // A key some other machine has: not this machine's business, so
            // the whole binding is left out here.
            Err(_) if shared && names.all.iter().any(|i| i.matches(target)) => skip = true,
            Err(e) if shared && !names.all.is_empty() => {
                return Err(e.replace(&format!("keys for {machine}"), "keys for this machine"));
            }
            Err(e) => return Err(e),
        }
    }
    Ok((!skip).then_some(keys))
}

fn canonical_machine_key(
    text: &str,
    host: &str,
    section: &str,
    machine: &str,
    names: &[MachineKeyInfo],
) -> Result<String, String> {
    if names.is_empty() {
        return Ok(text.trim().to_string());
    }
    names
        .iter()
        .find(|info| info.matches(text))
        .map(|info| info.name.clone())
        .ok_or_else(|| {
            let list: Vec<&str> = names.iter().map(|info| info.name.as_str()).collect();
            format!(
                "unknown machine key \"{text}\" for \"{host}\" in {section} (keys for {machine}: \
                 {}; `retroclawd {machine} --list-keys` shows aliases)",
                list.join(", ")
            )
        })
}

/// Table of a machine's key names (and aliases) for `--list-keys`.
pub(crate) fn describe_names(machine: &str, names: &[MachineKeyInfo]) -> String {
    if names.is_empty() {
        return format!("{machine} does not report its key names\n");
    }
    let mut out = String::new();
    for info in names {
        if info.aliases.is_empty() {
            out.push_str(&format!("{}\n", info.name));
        } else {
            out.push_str(&format!(
                "{:<18} also: {}\n",
                info.name,
                info.aliases.join(", ")
            ));
        }
    }
    out
}

/// What one key currently holds down on the machine.
#[derive(Debug, Clone)]
struct Held {
    names: Vec<String>,
    /// Modifiers this key's chord consumed (the machine must not see them).
    consumed: Mods,
}

#[derive(Resource, Debug)]
pub(crate) struct KeyRouter {
    bindings: KeyBindings,
    layout: Layout,
    symbols: SymbolMap,
    held: HashMap<KeyCode, Held>,
    /// Machine key -> how many host keys currently hold it down.
    counts: HashMap<String, u32>,
    /// The names the combined machine Shift pressed, while it is down.
    shift_names: Option<Vec<String>>,
}

impl KeyRouter {
    pub(crate) fn new(bindings: KeyBindings) -> Self {
        KeyRouter {
            bindings,
            layout: Layout::Positional,
            symbols: SymbolMap::default(),
            held: HashMap::new(),
            counts: HashMap::new(),
            shift_names: None,
        }
    }

    /// Switches to the symbolic layout with the machine's character map.
    pub(crate) fn with_layout(mut self, layout: Layout, symbols: SymbolMap) -> Self {
        self.layout = layout;
        self.symbols = symbols;
        self
    }

    /// The machine's character map (what each typed character is).
    pub(crate) fn symbols(&self) -> &SymbolMap {
        &self.symbols
    }

    /// The names a bare press of `key` sends (no chord): its plain binding,
    /// else its own name. Used by the startup auto-typer, which presses
    /// `KeyCode`s as if typed.
    pub(crate) fn plain_names(&self, key: KeyCode) -> Vec<String> {
        let plain = Chord {
            key,
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
        };
        self.bindings
            .get(&plain)
            .cloned()
            .unwrap_or_else(|| vec![default_name(key)])
    }

    fn resolve_press(&self, key: KeyCode, mods: Mods, typed: Option<char>, dead: bool) -> Held {
        if !mods.is_empty() {
            let exact = Chord {
                key,
                ctrl: mods.ctrl,
                shift: mods.shift,
                alt: mods.alt,
                meta: mods.meta,
            };
            if let Some(names) = self.bindings.get(&exact) {
                return Held {
                    names: names.clone(),
                    consumed: Mods::of_chord(&exact),
                };
            }
        }
        // Symbolic layout: an explicit plain binding for this physical key
        // still wins, otherwise the character the key typed picks the machine
        // key. Characters the machine has no key for fall through to the
        // physical key's own name, as in the positional layout -- except a dead
        // key, which types nothing by itself, and an accented letter, which
        // types its plain letter.
        let plain_binding = Chord {
            key,
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
        };
        if self.layout == Layout::Symbolic && dead && self.bindings.get(&plain_binding).is_none() {
            return Held {
                names: Vec::new(),
                consumed: Mods::default(),
            };
        }
        if self.layout == Layout::Symbolic
            && let Some(c) = typed
            && self
                .bindings
                .get(&Chord {
                    key,
                    ctrl: false,
                    shift: false,
                    alt: false,
                    meta: false,
                })
                .is_none()
            && let Some((name, kind)) = self.symbols.lookup(c)
        {
            let consumed = match kind {
                CharKind::Letter => Mods::default(),
                CharKind::Digit => Mods {
                    shift: mods.shift,
                    ..Mods::default()
                },
                CharKind::Symbol => Mods {
                    shift: mods.shift,
                    alt: mods.alt,
                    ..Mods::default()
                },
            };
            return Held {
                names: vec![name.clone()],
                consumed,
            };
        }
        Held {
            names: self.plain_names(key),
            consumed: Mods::default(),
        }
    }

    /// Compares the keyboard to what has been sent so far and returns the
    /// machine key events (`name`, pressed) to send now: releases first, then
    /// presses. Call once per frame, after `Hotkeys::poll`.
    pub(crate) fn route(
        &mut self,
        keyboard: &ButtonInput<KeyCode>,
        hot: &HotkeyState,
        logical: &LogicalKeys,
    ) -> Vec<(String, bool)> {
        let physical: HashSet<KeyCode> = keyboard
            .get_pressed()
            .copied()
            .filter(|key| !hot.is_swallowed(*key))
            .collect();
        let physical_mods = Mods::from_keys(&physical);

        let mut releases: Vec<Vec<String>> = Vec::new();
        let mut presses: Vec<Vec<String>> = Vec::new();

        // Ordinary keys that were let go.
        let gone: Vec<KeyCode> = self
            .held
            .keys()
            .copied()
            .filter(|key| mod_class(*key).is_none() && !physical.contains(key))
            .collect();
        for key in gone {
            if let Some(held) = self.held.remove(&key) {
                releases.push(held.names);
            }
        }
        // Ordinary keys newly pressed (sorted: a deterministic event order).
        let mut fresh: Vec<KeyCode> = physical
            .iter()
            .copied()
            .filter(|key| mod_class(*key).is_none() && !self.held.contains_key(key))
            .collect();
        fresh.sort_by_key(|key| default_name(*key));
        for key in fresh {
            let held =
                self.resolve_press(key, physical_mods, logical.get(key), logical.is_dead(key));
            presses.push(held.names.clone());
            self.held.insert(key, held);
        }

        // Modifiers a held chord has consumed are hidden from the machine.
        let masked = self
            .held
            .iter()
            .filter(|(key, _)| mod_class(**key).is_none())
            .fold(Mods::default(), |acc, (_, held)| acc.union(held.consumed));

        // Ctrl / Alt / Super keys, one machine key each.
        let mut modifier_keys: Vec<KeyCode> = CTRL_KEYS
            .iter()
            .chain(&ALT_KEYS)
            .chain(&META_KEYS)
            .copied()
            .collect();
        modifier_keys.sort_by_key(|key| default_name(*key));
        for key in modifier_keys {
            let class = mod_class(key).expect("a modifier key has a class");
            let wanted = physical.contains(&key) && !consumed(masked, class);
            let have = self.held.contains_key(&key);
            if have && !wanted {
                if let Some(held) = self.held.remove(&key) {
                    releases.push(held.names);
                }
            } else if !have && wanted {
                let names = self.plain_names(key);
                presses.push(names.clone());
                self.held.insert(
                    key,
                    Held {
                        names,
                        consumed: Mods::default(),
                    },
                );
            }
        }

        // Shift: both Shift keys and the arrows are one machine Shift.
        let shift_wanted = (SHIFT_KEYS.iter().any(|key| physical.contains(key)) && !masked.shift)
            || SHIFT_SOURCE_ARROWS.iter().any(|key| physical.contains(key));
        match (self.shift_names.is_some(), shift_wanted) {
            (true, false) => releases.push(self.shift_names.take().unwrap_or_default()),
            (false, true) => {
                let names = self.plain_names(KeyCode::ShiftLeft);
                presses.push(names.clone());
                self.shift_names = Some(names);
            }
            _ => {}
        }

        self.apply(releases, presses)
    }

    /// Turns lists of machine keys that went up / down into the events to
    /// send, through the shared reference counts: a machine key held by two
    /// sources (two host keys, a key and a pad button) goes up only when the
    /// last one lets go, and down only on the first press. Releases come first.
    pub(crate) fn apply(
        &mut self,
        releases: Vec<Vec<String>>,
        presses: Vec<Vec<String>>,
    ) -> Vec<(String, bool)> {
        let mut events = Vec::new();
        for names in releases {
            for name in names {
                let count = self.counts.entry(name.clone()).or_insert(1);
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.counts.remove(&name);
                    events.push((name, false));
                }
            }
        }
        for names in presses {
            for name in names {
                let count = self.counts.entry(name.clone()).or_insert(0);
                *count += 1;
                if *count == 1 {
                    events.push((name, true));
                }
            }
        }
        events
    }
}

/// A listing of the host key names `[keys]` accepts on its left side.
pub(crate) fn describe_host_keys() -> String {
    let mut names: Vec<String> = super::hotkeys::bindable_keys()
        .iter()
        .map(|key| key_display_name(*key))
        .collect();
    names.sort();
    names.dedup();
    names.join(", ") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(names: &[&str]) -> Vec<MachineKeyInfo> {
        plugin_api::key_infos(names.iter().copied())
    }

    fn cfg(text: &str) -> AppConfig {
        app_config::parse(text).unwrap()
    }

    fn defaults() -> Hotkeys {
        Hotkeys::resolve(&AppConfig::default(), "m").unwrap()
    }

    fn router(text: &str, names: &[&str]) -> KeyRouter {
        let bindings = KeyBindings::resolve(
            &cfg(text),
            "m",
            &KeyNames::only(&info(names)),
            &defaults(),
            &[],
        )
        .unwrap();
        KeyRouter::new(bindings)
    }

    fn frame(
        router: &mut KeyRouter,
        keyboard: &mut ButtonInput<KeyCode>,
        hot: &HotkeyState,
    ) -> Vec<(String, bool)> {
        let events = router.route(keyboard, hot, &LogicalKeys::default());
        keyboard.clear();
        events
    }

    fn ev(name: &str, pressed: bool) -> (String, bool) {
        (name.to_string(), pressed)
    }

    const ALL: &[&str] = &[
        "KeyA",
        "KeyB",
        "Digit1",
        "Digit2",
        "ShiftLeft",
        "ShiftRight",
        "ControlLeft",
        "At",
        "Colon",
        "CapsShift",
        "SymShift",
        "ArrowLeft",
        "Space",
        "Fire",
        "Edit",
    ];

    #[test]
    fn unbound_keys_send_their_own_name_and_release_it() {
        let mut r = router("", ALL);
        let hot = HotkeyState::default();
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::KeyA);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("KeyA", true)]);
        // Held across frames: nothing new.
        assert!(frame(&mut r, &mut kb, &hot).is_empty());
        kb.release(KeyCode::KeyA);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("KeyA", false)]);
    }

    #[test]
    fn a_plain_binding_replaces_the_name_and_none_silences_the_key() {
        let mut r = router(
            "[keys]\nsemicolon = \"colon\"\nbackquote = \"none\"\nf = [\"caps_shift\", \"1\"]",
            ALL,
        );
        let hot = HotkeyState::default();
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::Semicolon);
        kb.press(KeyCode::Backquote);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("Colon", true)]);
        kb.release(KeyCode::Semicolon);
        kb.release(KeyCode::Backquote);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("Colon", false)]);

        // A list presses every key, and releases every key.
        kb.press(KeyCode::KeyF);
        assert_eq!(
            frame(&mut r, &mut kb, &hot),
            [ev("CapsShift", true), ev("Digit1", true)]
        );
        kb.release(KeyCode::KeyF);
        let mut released = frame(&mut r, &mut kb, &hot);
        released.sort();
        assert_eq!(released, [ev("CapsShift", false), ev("Digit1", false)]);
    }

    #[test]
    fn aliases_in_the_config_resolve_to_the_plugins_own_names() {
        let names = vec![
            MachineKeyInfo::with_aliases("SymShift", &["symbol_shift"]),
            MachineKeyInfo::new("KeyA"),
        ];
        let bindings = KeyBindings::resolve(
            &cfg("[keys]\ncaps_lock = \"symbol_shift\"\nbracket_left = \"a\""),
            "m",
            &KeyNames::only(&names),
            &defaults(),
            &[],
        )
        .unwrap();
        let r = KeyRouter::new(bindings);
        assert_eq!(r.plain_names(KeyCode::CapsLock), ["SymShift"]);
        assert_eq!(r.plain_names(KeyCode::BracketLeft), ["KeyA"]);
        // And an unbound key falls back to its own name.
        assert_eq!(r.plain_names(KeyCode::KeyZ), ["KeyZ"]);
    }

    fn a_shift_chord_consumes_the_real_shift() {
        let mut r = router("[keys]\n\"shift+2\" = \"At\"", ALL);
        let hot = HotkeyState::default();
        let mut kb = ButtonInput::<KeyCode>::default();

        // Shift first: it reaches the machine...
        kb.press(KeyCode::ShiftLeft);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("ShiftLeft", true)]);
        // ...then 2: the machine's Shift is released *before* At is pressed.
        kb.press(KeyCode::Digit2);
        assert_eq!(
            frame(&mut r, &mut kb, &hot),
            [ev("ShiftLeft", false), ev("At", true)]
        );
        // Releasing 2 while Shift is still down gives Shift back.
        kb.release(KeyCode::Digit2);
        assert_eq!(
            frame(&mut r, &mut kb, &hot),
            [ev("At", false), ev("ShiftLeft", true)]
        );
        kb.release(KeyCode::ShiftLeft);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("ShiftLeft", false)]);
    }

    #[test]
    fn a_chord_pressed_in_the_same_frame_as_its_modifier_never_sends_the_modifier() {
        let mut r = router("[keys]\n\"shift+2\" = \"At\"", ALL);
        let hot = HotkeyState::default();
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::ShiftRight);
        kb.press(KeyCode::Digit2);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("At", true)]);
        // Shift released first while 2 is held: still nothing stray.
        kb.release(KeyCode::ShiftRight);
        assert!(frame(&mut r, &mut kb, &hot).is_empty());
        kb.release(KeyCode::Digit2);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("At", false)]);
    }

    #[test]
    fn without_a_matching_chord_shift_passes_through_with_the_plain_binding() {
        let mut r = router("[keys]\nbackquote = \"none\"\nf = \"b\"", ALL);
        let hot = HotkeyState::default();
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::ShiftLeft);
        kb.press(KeyCode::KeyF);
        // Shift+F has no chord binding, so the plain F binding applies and
        // Shift is still sent.
        let mut got = frame(&mut r, &mut kb, &hot);
        got.sort();
        assert_eq!(got, [ev("KeyB", true), ev("ShiftLeft", true)]);
    }

    fn both_shift_keys_are_one_machine_shift() {
        let mut r = router("", ALL);
        let hot = HotkeyState::default();
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::ShiftLeft);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("ShiftLeft", true)]);
        kb.press(KeyCode::ShiftRight);
        assert!(frame(&mut r, &mut kb, &hot).is_empty());
        kb.release(KeyCode::ShiftLeft);
        assert!(
            frame(&mut r, &mut kb, &hot).is_empty(),
            "right shift still down"
        );
        kb.release(KeyCode::ShiftRight);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("ShiftLeft", false)]);
    }

    fn arrow_keys_hold_the_machine_shift_as_before() {
        let mut r = router("", ALL);
        let hot = HotkeyState::default();
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::ArrowLeft);
        assert_eq!(
            frame(&mut r, &mut kb, &hot),
            [ev("ArrowLeft", true), ev("ShiftLeft", true)]
        );
        // A real Shift held too keeps the shift down after the arrow goes.
        kb.press(KeyCode::ShiftRight);
        assert!(frame(&mut r, &mut kb, &hot).is_empty());
        kb.release(KeyCode::ArrowLeft);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("ArrowLeft", false)]);
        kb.release(KeyCode::ShiftRight);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("ShiftLeft", false)]);
    }
    /// Merged (every check kept): `both_shift_keys_are_one_machine_shift`, `arrow_keys_hold_the_machine_shift_as_before`.
    #[test]
    fn machine_shift_comes_from_either_shift_and_the_arrows() {
        both_shift_keys_are_one_machine_shift();
        arrow_keys_hold_the_machine_shift_as_before();
    }

    fn a_ctrl_chord_consumes_ctrl_too() {
        let mut r = router("[keys]\n\"ctrl+a\" = \"Edit\"", ALL);
        let hot = HotkeyState::default();
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::ControlLeft);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("ControlLeft", true)]);
        kb.press(KeyCode::KeyA);
        assert_eq!(
            frame(&mut r, &mut kb, &hot),
            [ev("ControlLeft", false), ev("Edit", true)]
        );
    }
    /// Merged (every check kept): `a_shift_chord_consumes_the_real_shift`, `a_ctrl_chord_consumes_ctrl_too`.
    #[test]
    fn chords_consume_their_modifiers() {
        a_shift_chord_consumes_the_real_shift();
        a_ctrl_chord_consumes_ctrl_too();
    }

    #[test]
    fn two_host_keys_on_one_machine_key_release_it_only_when_both_are_up() {
        let mut r = router("[keys]\nz = \"Fire\"\nx = \"Fire\"", ALL);
        let hot = HotkeyState::default();
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::KeyZ);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("Fire", true)]);
        kb.press(KeyCode::KeyX);
        assert!(frame(&mut r, &mut kb, &hot).is_empty());
        kb.release(KeyCode::KeyZ);
        assert!(frame(&mut r, &mut kb, &hot).is_empty());
        kb.release(KeyCode::KeyX);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("Fire", false)]);
    }

    #[test]
    fn a_release_undoes_what_the_press_sent_even_if_modifiers_changed() {
        let mut r = router("[keys]\n\"shift+2\" = \"At\"", ALL);
        let hot = HotkeyState::default();
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::Digit2);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("Digit2", true)]);
        // Shift arrives while 2 is held: the key keeps what it already sent.
        kb.press(KeyCode::ShiftLeft);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("ShiftLeft", true)]);
        kb.release(KeyCode::Digit2);
        assert_eq!(frame(&mut r, &mut kb, &hot), [ev("Digit2", false)]);
    }

    #[test]
    fn hotkey_swallowed_keys_never_reach_the_machine() {
        let mut r = router("", ALL);
        let mut hot = HotkeyState::default();
        hot.swallow(KeyCode::F12);
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::F12);
        assert!(frame(&mut r, &mut kb, &hot).is_empty());
        kb.release(KeyCode::F12);
        assert!(frame(&mut r, &mut kb, &hot).is_empty());
    }

    #[test]
    fn layers_override_per_chord_and_machine_keys_are_validated() {
        let names = info(&["KeyA", "KeyB", "Colon"]);
        let config =
            cfg("[keys]\nsemicolon = \"colon\"\nq = \"a\"\n[machines.m.keys]\nsemicolon = \"b\"");
        let r = KeyRouter::new(
            KeyBindings::resolve(&config, "m", &KeyNames::only(&names), &defaults(), &[]).unwrap(),
        );
        assert_eq!(r.plain_names(KeyCode::Semicolon), ["KeyB"], "machine wins");
        assert_eq!(
            r.plain_names(KeyCode::KeyQ),
            ["KeyA"],
            "global still applies"
        );
        // Another machine only sees the global layer.
        let other = KeyRouter::new(
            KeyBindings::resolve(&config, "other", &KeyNames::only(&names), &defaults(), &[])
                .unwrap(),
        );
        assert_eq!(other.plain_names(KeyCode::Semicolon), ["Colon"]);
    }

    #[test]
    fn config_errors_name_the_section_and_the_valid_choices() {
        let names = info(&["KeyA", "Colon"]);
        let resolve = |text: &str| {
            KeyBindings::resolve(&cfg(text), "m", &KeyNames::only(&names), &defaults(), &[])
                .unwrap_err()
        };
        let unknown = resolve("[machines.m.keys]\nsemicolon = \"colom\"");
        assert!(
            unknown.contains("\"colom\"") && unknown.contains("[machines.m.keys]"),
            "{unknown}"
        );
        assert!(
            unknown.contains("KeyA") && unknown.contains("Colon"),
            "{unknown}"
        );
        assert!(resolve("[keys]\nnope = \"a\"").contains("unknown key"));
        assert!(resolve("[keys]\nq = [\"a\", \"none\"]").contains("mixes"));
        assert!(resolve("[keys]\nq = []").contains("empty"));
        // F12 is the reset hotkey: a binding there could never fire.
        let clash = resolve("[keys]\nf12 = \"a\"");
        assert!(
            clash.contains("KC_F12") && clash.contains("reset"),
            "{clash}"
        );
        // ...unless hotkeys sit behind a hold modifier.
        let gated = Hotkeys::resolve(&cfg("[hotkeys]\nmodifier = \"alt_right\""), "m").unwrap();
        assert!(
            KeyBindings::resolve(
                &cfg("[keys]\nf12 = \"a\""),
                "m",
                &KeyNames::only(&names),
                &gated,
                &[]
            )
            .is_ok()
        );
    }

    #[test]
    fn a_shared_binding_naming_another_machines_key_is_skipped_but_a_typo_is_not() {
        let spectrum = info(&["KeyA", "At", "SymShift"]);
        let zx81 = info(&["KeyA"]);
        let all: Vec<MachineKeyInfo> = spectrum.clone();
        let for_zx81 = KeyNames {
            machine: zx81,
            all: all.clone(),
        };
        let for_spectrum = KeyNames {
            machine: spectrum,
            all,
        };
        let config = cfg("[keys]\n\"shift+2\" = \"at\"\nq = \"a\"");
        let zx = KeyBindings::resolve(&config, "zx81", &for_zx81, &defaults(), &[]).unwrap();
        let zx = KeyRouter::new(zx);
        assert_eq!(
            zx.plain_names(KeyCode::KeyQ),
            ["KeyA"],
            "other bindings still apply"
        );
        let sp = KeyRouter::new(
            KeyBindings::resolve(&config, "zx-spectrum", &for_spectrum, &defaults(), &[]).unwrap(),
        );
        assert_eq!(sp.bindings.map.len(), 2, "the spectrum has both");
        assert_eq!(zx.bindings.map.len(), 1, "zx81 skipped the At binding");

        // Nobody has `atx`: an error even in the shared layer.
        let typo = cfg("[keys]\n\"shift+2\" = \"atx\"");
        let err = KeyBindings::resolve(&typo, "zx81", &for_zx81, &defaults(), &[]).unwrap_err();
        assert!(err.contains("atx") && err.contains("[keys]"), "{err}");
        // A machine-specific binding is strict: zx81 has no `at`.
        let strict = cfg("[machines.zx81.keys]\n\"shift+2\" = \"at\"");
        assert!(KeyBindings::resolve(&strict, "zx81", &for_zx81, &defaults(), &[]).is_err());
    }

    #[test]
    fn a_plugin_without_key_names_passes_values_through() {
        let bindings = KeyBindings::resolve(
            &cfg("[keys]\nq = \"Anything\""),
            "m",
            &KeyNames::default(),
            &defaults(),
            &[],
        )
        .unwrap();
        assert_eq!(
            KeyRouter::new(bindings).plain_names(KeyCode::KeyQ),
            ["Anything"]
        );
    }

    // ---- symbolic layout -------------------------------------------------

    fn spectrum_like() -> Vec<MachineKeyInfo> {
        let mut infos = plugin_api::key_infos([
            "KeyA",
            "KeyZ",
            "KeyY",
            "Digit1",
            "Digit2",
            "ShiftLeft",
            "AltRight",
            "Exclamation",
            "At",
            "DoubleQuote",
            "Semicolon",
            "Space",
        ]);
        plugin_api::set_key_chars(
            &mut infos,
            &[
                ("Exclamation", "!"),
                ("At", "@"),
                ("DoubleQuote", "\""),
                ("Semicolon", ";"),
                ("Space", " "),
            ],
        );
        infos
    }

    fn symbolic_router(text: &str) -> KeyRouter {
        let names = spectrum_like();
        let bindings =
            KeyBindings::resolve(&cfg(text), "m", &KeyNames::only(&names), &defaults(), &[])
                .unwrap();
        KeyRouter::new(bindings).with_layout(Layout::Symbolic, SymbolMap::from_names(&names))
    }

    /// One frame with `key` going down, having typed `c` (None: typed nothing).
    fn press_typing(
        router: &mut KeyRouter,
        kb: &mut ButtonInput<KeyCode>,
        key: KeyCode,
        c: Option<char>,
    ) -> Vec<(String, bool)> {
        let mut logical = LogicalKeys::default();
        logical.pressed(key, c);
        kb.press(key);
        let events = router.route(kb, &HotkeyState::default(), &logical);
        kb.clear();
        events
    }

    #[test]
    fn the_symbol_map_comes_from_letters_digits_and_declared_characters() {
        let map = SymbolMap::from_names(&spectrum_like());
        assert_eq!(map.get('z').unwrap().0, "KeyZ");
        assert_eq!(map.get('Z').unwrap().0, "KeyZ");
        assert_eq!(map.get('1').unwrap().0, "Digit1");
        assert_eq!(map.get('!').unwrap().0, "Exclamation");
        assert_eq!(map.get(' ').unwrap().0, "Space");
        assert!(map.get('~').is_none());
        assert!(map.get('3').is_none(), "this machine list has no Digit3");
    }

    #[test]
    fn an_accented_letter_types_its_plain_letter_when_the_machine_has_no_key_for_it() {
        let mut symbolic = symbolic_router("");
        let mut kb = ButtonInput::<KeyCode>::default();
        // The key that typed `à` (a French keyboard's 2 key, say) sends the A key, not its physical position's name.
        assert_eq!(
            press_typing(&mut symbolic, &mut kb, KeyCode::Digit2, Some('\u{e0}')),
            [ev("KeyA", true)]
        );
        // A character with no plain twin still falls back to the physical key's name.
        let mut symbolic = symbolic_router("");
        let mut kb = ButtonInput::<KeyCode>::default();
        assert_eq!(
            press_typing(&mut symbolic, &mut kb, KeyCode::KeyQ, Some('\u{4e2d}')),
            [ev("KeyQ", true)]
        );
    }

    #[test]
    fn a_dead_key_types_nothing_in_the_symbolic_layout_but_an_explicit_binding_still_wins() {
        let mut symbolic = symbolic_router("");
        let mut logical = LogicalKeys::default();
        logical.pressed_dead(KeyCode::Equal);
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::Equal);
        assert_eq!(
            symbolic.route(&kb, &HotkeyState::default(), &logical),
            [],
            "the accent key sends no machine key"
        );
        // The composed letter that follows is typed as usual (here `à`, on its own physical key).
        kb.clear();
        kb.release(KeyCode::Equal);
        logical.released(KeyCode::Equal);
        symbolic.route(&kb, &HotkeyState::default(), &logical);
        kb.clear();
        assert_eq!(
            press_typing(&mut symbolic, &mut kb, KeyCode::KeyE, Some('\u{e0}')),
            [ev("KeyA", true)]
        );
        // An explicit binding for the physical key still wins over "dead".
        let mut bound = symbolic_router("[keys]\nKC_EQL = \"Semicolon\"");
        let mut logical = LogicalKeys::default();
        logical.pressed_dead(KeyCode::Equal);
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::Equal);
        assert_eq!(
            bound.route(&kb, &HotkeyState::default(), &logical),
            [ev("Semicolon", true)]
        );
    }

    #[test]
    fn a_german_keyboards_y_key_types_z_in_the_symbolic_layout() {
        // The key at the QWERTY Y position is labelled Z on a German keyboard.
        let mut positional = router("", &["KeyY", "KeyZ"]);
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::KeyY);
        assert_eq!(
            frame(&mut positional, &mut kb, &HotkeyState::default()),
            [ev("KeyY", true)]
        );

        let mut symbolic = symbolic_router("");
        let mut kb = ButtonInput::<KeyCode>::default();
        assert_eq!(
            press_typing(&mut symbolic, &mut kb, KeyCode::KeyY, Some('z')),
            [ev("KeyZ", true)]
        );
        // And its release undoes that press, even though nothing is typed on release.
        kb.release(KeyCode::KeyY);
        assert_eq!(
            symbolic.route(&kb, &HotkeyState::default(), &LogicalKeys::default()),
            [ev("KeyZ", false)]
        );
    }

    #[test]
    fn a_typed_symbol_becomes_the_machines_key_and_consumes_shift_and_altgr() {
        let mut r = symbolic_router("");
        let mut kb = ButtonInput::<KeyCode>::default();
        // Shift goes down on its own first (it reaches the machine)...
        kb.press(KeyCode::ShiftLeft);
        assert_eq!(
            r.route(&kb, &HotkeyState::default(), &LogicalKeys::default()),
            [ev("ShiftLeft", true)]
        );
        kb.clear();
        // ...then Shift+2 types `"` on a UK/German layout: the machine's Shift is
        // released *before* DoubleQuote is pressed.
        assert_eq!(
            press_typing(&mut r, &mut kb, KeyCode::Digit2, Some('"')),
            [ev("ShiftLeft", false), ev("DoubleQuote", true)]
        );
        // Letting go gives the machine its Shift back.
        kb.release(KeyCode::Digit2);
        assert_eq!(
            r.route(&kb, &HotkeyState::default(), &LogicalKeys::default()),
            [ev("DoubleQuote", false), ev("ShiftLeft", true)]
        );
    }

    #[test]
    fn digits_consume_shift_but_letters_keep_it() {
        // AZERTY: Shift+& types 1.
        let mut r = symbolic_router("");
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::ShiftLeft);
        r.route(&kb, &HotkeyState::default(), &LogicalKeys::default());
        kb.clear();
        assert_eq!(
            press_typing(&mut r, &mut kb, KeyCode::Digit1, Some('1')),
            [ev("ShiftLeft", false), ev("Digit1", true)]
        );
        // A capital A: Shift stays down for the machine.
        let mut r = symbolic_router("");
        let mut kb = ButtonInput::<KeyCode>::default();
        kb.press(KeyCode::ShiftLeft);
        r.route(&kb, &HotkeyState::default(), &LogicalKeys::default());
        kb.clear();
        assert_eq!(
            press_typing(&mut r, &mut kb, KeyCode::KeyA, Some('A')),
            [ev("KeyA", true)]
        );
    }

    #[test]
    fn characters_the_machine_lacks_and_untyped_keys_stay_positional() {
        let mut r = symbolic_router("");
        let mut kb = ButtonInput::<KeyCode>::default();
        // `~` has no machine key: the physical key's own name is sent.
        assert_eq!(
            press_typing(&mut r, &mut kb, KeyCode::Backquote, Some('~')),
            [ev("Backquote", true)]
        );
        // A key that typed nothing (an arrow, F-key, ...) is its own name too.
        assert_eq!(
            press_typing(&mut r, &mut kb, KeyCode::F5, None),
            [ev("F5", true)]
        );
    }

    #[test]
    fn an_explicit_binding_beats_the_symbolic_translation() {
        let mut r = symbolic_router("[keys]\nsemicolon = \"KeyA\"");
        let mut kb = ButtonInput::<KeyCode>::default();
        assert_eq!(
            press_typing(&mut r, &mut kb, KeyCode::Semicolon, Some(';')),
            [ev("KeyA", true)]
        );
    }

    #[test]
    fn the_positional_layout_ignores_what_was_typed() {
        let mut r = router("", ALL);
        let mut kb = ButtonInput::<KeyCode>::default();
        assert_eq!(
            press_typing(&mut r, &mut kb, KeyCode::KeyY, Some('z')),
            [ev("KeyY", true)]
        );
    }

    #[test]
    fn keymap_files_layer_below_and_above_the_inline_keys() {
        let names = spectrum_like();
        let file = |text: &str, upper: bool, label: &str| LoadedKeymap {
            label: label.to_string(),
            upper,
            file: app_config::parse_keymap(text).unwrap(),
        };
        let files = [
            file(
                "[keys]\nsemicolon = \"KeyA\"\nbackquote = \"KeyZ\"",
                false,
                "low.toml",
            ),
            file("[keys]\nbackquote = \"KeyY\"", true, "high.toml"),
        ];
        let b = KeyBindings::resolve(
            &cfg("[keys]\nsemicolon = \"KeyY\""),
            "m",
            &KeyNames::only(&names),
            &defaults(),
            &files,
        )
        .unwrap();
        let r = KeyRouter::new(b);
        // Inline [keys] beats the lower file; the --keymap file beats inline.
        assert_eq!(r.plain_names(KeyCode::Semicolon), ["KeyY"]);
        assert_eq!(r.plain_names(KeyCode::Backquote), ["KeyY"]);
        // A file's error names the file.
        let bad = [file("[keys]\nq = \"Nope\"", false, "bad.toml")];
        let err = KeyBindings::resolve(&cfg(""), "m", &KeyNames::only(&names), &defaults(), &bad)
            .unwrap_err();
        assert!(
            err.contains("[keys] in bad.toml") && err.contains("Nope"),
            "{err}"
        );
    }
}
