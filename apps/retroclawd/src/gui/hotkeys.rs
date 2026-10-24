//! Emulator hotkeys: host key chords bound to host-level actions (reset,
//! mute, pause, ...), as opposed to the machine keyboard map in
//! `input::KeymapRes`. Configured in `retroclawd.toml` under `[hotkeys]` and
//! `[machines.<id>.hotkeys]` (layered: built-in defaults, then global, then
//! machine, per binding; `"none"` unbinds). See `TODO.md`'s "Input, hotkeys
//! and file dialogs roadmap" for the design.
//!
//! This file is Bevy-input-aware but system-free: `Hotkeys::poll` is a pure
//! function of the keyboard state, so chord matching, modifier handling and
//! "swallowed key" bookkeeping are unit-tested directly. The Bevy system that
//! calls it and applies the actions lives in `input.rs`.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use app_config::{AppConfig, HotkeysConfig};
use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;
use bevy::prelude::Resource;

use super::input::stable_host_key_name;
use super::pads::{MAX_PADS, PadButton, PadState};

/// A host-level action a hotkey can trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Action {
    Reset,
    Fullscreen,
    Mute,
    VolumeUp,
    VolumeDown,
    Overlay,
    Version,
    Pause,
    Screenshot,
    QuickSave,
    QuickLoad,
    StateSlotNext,
    StateSlotPrev,
    FastForward,
    ShaderNext,
    ShaderPrev,
    Quit,
    LoadTape,
    LoadDisk,
    EjectTape,
    EjectDisk,
    LoadSnapshot,
    SaveSnapshot,
    LoadState,
    SaveState,
    SaveTape,
    SaveDisk,
    LoadDiskB,
    EjectDiskB,
    SaveDiskB,
    SaveHardDisk,
    NextDisk,
    PrevDisk,
    Freeze,
    Paste,
}

impl Action {
    /// The file-dialog action this hotkey starts, if it is one.
    pub(crate) fn file_action(self) -> Option<crate::dialogs::FileAction> {
        use crate::dialogs::FileAction;
        Some(match self {
            Action::LoadTape => FileAction::LoadTape,
            Action::LoadDisk => FileAction::LoadDisk,
            Action::EjectTape => FileAction::EjectTape,
            Action::EjectDisk => FileAction::EjectDisk,
            Action::LoadSnapshot => FileAction::LoadSnapshot,
            Action::SaveSnapshot => FileAction::SaveSnapshot,
            Action::LoadState => FileAction::LoadState,
            Action::SaveState => FileAction::SaveState,
            Action::SaveTape => FileAction::SaveTape,
            Action::SaveDisk => FileAction::SaveDisk,
            Action::LoadDiskB => FileAction::LoadDiskB,
            Action::EjectDiskB => FileAction::EjectDiskB,
            Action::SaveDiskB => FileAction::SaveDiskB,
            Action::SaveHardDisk => FileAction::SaveHardDisk,
            Action::NextDisk => FileAction::NextDisk,
            Action::PrevDisk => FileAction::PrevDisk,
            _ => return None,
        })
    }
}

pub(crate) struct ActionInfo {
    pub(crate) action: Action,
    /// The name used in `retroclawd.toml`.
    pub(crate) name: &'static str,
    /// Built-in chord, `None` = unbound by default.
    pub(crate) default: Option<&'static str>,
    pub(crate) help: &'static str,
    /// Active while the key is held (`FastForward`), not a one-shot.
    pub(crate) hold: bool,
}

/// Every action. Defaults deliberately stay on F9-F12, Pause, Print Screen
/// and Scroll Lock (plus chords on those): none of them is a key any
/// emulated machine uses (the C64's F1/F3/F5/F7 and the Spectrum/CPC/C64
/// letter and modifier keys all stay with the machine).
pub(crate) const ACTIONS: [ActionInfo; 35] = [
    ActionInfo {
        action: Action::Reset,
        name: "reset",
        default: Some("f12"),
        help: "reset the machine (CPU only, RAM kept, like the RESET button)",
        hold: false,
    },
    ActionInfo {
        action: Action::Fullscreen,
        name: "fullscreen",
        default: Some("f11"),
        help: "toggle borderless fullscreen",
        hold: false,
    },
    ActionInfo {
        action: Action::Mute,
        name: "mute",
        default: Some("f9"),
        help: "mute / unmute audio",
        hold: false,
    },
    ActionInfo {
        action: Action::VolumeUp,
        name: "volume_up",
        default: Some("ctrl+f10"),
        help: "raise the volume by 10%",
        hold: false,
    },
    ActionInfo {
        action: Action::VolumeDown,
        name: "volume_down",
        default: Some("ctrl+f9"),
        help: "lower the volume by 10%",
        hold: false,
    },
    ActionInfo {
        action: Action::Overlay,
        name: "overlay",
        default: Some("f10"),
        help: "show / hide the debug overlay (cpu panel unless --show-overlay chose panels)",
        hold: false,
    },
    ActionInfo {
        action: Action::Version,
        name: "version",
        default: Some("shift+f9"),
        help: "show / hide the version panel in the overlay (retroclawd and plugin version, build number, git commit)",
        hold: false,
    },
    ActionInfo {
        action: Action::Pause,
        name: "pause",
        default: Some("pause"),
        help: "pause / resume emulation (audio is silent while paused)",
        hold: false,
    },
    ActionInfo {
        action: Action::Paste,
        name: "paste",
        default: Some("shift+insert"),
        help: "type the clipboard's text into the machine (again to cancel)",
        hold: false,
    },
    ActionInfo {
        action: Action::Screenshot,
        name: "screenshot",
        default: Some("print_screen"),
        help: "save the current frame as screenshots/<machine>-<time>.png",
        hold: false,
    },
    ActionInfo {
        action: Action::QuickSave,
        name: "quick_save",
        default: Some("shift+f11"),
        help: "save the machine state to the current slot (states/<machine>-slotN.state)",
        hold: false,
    },
    ActionInfo {
        action: Action::QuickLoad,
        name: "quick_load",
        default: Some("shift+f12"),
        help: "load the machine state from the current slot",
        hold: false,
    },
    ActionInfo {
        action: Action::StateSlotNext,
        name: "state_slot_next",
        default: Some("shift+f10"),
        help: "select the next state slot (0-9)",
        hold: false,
    },
    ActionInfo {
        action: Action::StateSlotPrev,
        name: "state_slot_prev",
        default: None,
        help: "select the previous state slot (0-9)",
        hold: false,
    },
    ActionInfo {
        action: Action::FastForward,
        name: "fast_forward",
        default: Some("scroll_lock"),
        help: "run the machine faster while held (also speeds up tape loading); audio muted",
        hold: true,
    },
    ActionInfo {
        action: Action::ShaderNext,
        name: "shader_next",
        default: Some("ctrl+f11"),
        help: "switch to the next bundled shader",
        hold: false,
    },
    ActionInfo {
        action: Action::ShaderPrev,
        name: "shader_prev",
        default: None,
        help: "switch to the previous bundled shader",
        hold: false,
    },
    ActionInfo {
        action: Action::Quit,
        name: "quit",
        default: Some("ctrl+f12"),
        help: "quit retroclawd",
        hold: false,
    },
    // File dialogs: the machine pauses while one is open. Defaults sit on
    // Ctrl+Shift+F9-F12, the only F9-F12 chords still free.
    ActionInfo {
        action: Action::LoadTape,
        name: "load_tape",
        default: Some("ctrl+shift+f9"),
        help: "choose a tape file and put it in the deck (replaces the current tape)",
        hold: false,
    },
    ActionInfo {
        action: Action::LoadDisk,
        name: "load_disk",
        default: Some("ctrl+shift+f10"),
        help: "choose a disk image and put it in the drive (asks first if the current disk has unsaved changes)",
        hold: false,
    },
    ActionInfo {
        action: Action::LoadDiskB,
        name: "load_disk_b",
        default: None,
        help: "choose a disk image for the second drive (drive B of a CPC664/6128); refused while its disk has unsaved changes",
        hold: false,
    },
    ActionInfo {
        action: Action::EjectDiskB,
        name: "eject_disk_b",
        default: None,
        help: "take the disk out of drive B (refused while it has unsaved changes)",
        hold: false,
    },
    ActionInfo {
        action: Action::SaveDiskB,
        name: "save_disk_b",
        default: None,
        help: "save a copy of the disk in drive B as a .dsk file (and count its changes as saved)",
        hold: false,
    },
    ActionInfo {
        action: Action::SaveHardDisk,
        name: "save_hd",
        default: None,
        help: "save a copy of ACSI hard disk 0 as an image file (and count its changes as saved)",
        hold: false,
    },
    ActionInfo {
        action: Action::EjectTape,
        name: "eject_tape",
        default: None,
        help: "take the tape out of the deck",
        hold: false,
    },
    ActionInfo {
        action: Action::EjectDisk,
        name: "eject_disk",
        default: None,
        help: "take the disk out of the drive (asks first if it has unsaved changes)",
        hold: false,
    },
    ActionInfo {
        action: Action::LoadSnapshot,
        name: "load_snapshot",
        default: Some("ctrl+shift+f11"),
        help: "choose a snapshot file (.z80/.sna/.szx/.p ...) and load it",
        hold: false,
    },
    ActionInfo {
        action: Action::SaveSnapshot,
        name: "save_snapshot",
        default: Some("ctrl+shift+f12"),
        help: "save the machine as a snapshot file (format from the extension you type)",
        hold: false,
    },
    ActionInfo {
        action: Action::LoadState,
        name: "load_state",
        default: None,
        help: "choose a saved .state file and load it (any file, not just a quick-save slot)",
        hold: false,
    },
    ActionInfo {
        action: Action::SaveState,
        name: "save_state",
        default: None,
        help: "save the machine state to a .state file you name",
        hold: false,
    },
    ActionInfo {
        action: Action::SaveTape,
        name: "save_tape",
        default: None,
        help: "save what the program recorded to the cassette as .cdt/.tzx/.voc (CPC)",
        hold: false,
    },
    ActionInfo {
        action: Action::SaveDisk,
        name: "save_disk",
        default: None,
        help: "save a copy of the disk image, guest writes included",
        hold: false,
    },
    ActionInfo {
        action: Action::Freeze,
        name: "freeze",
        default: Some("ctrl+pause"),
        help: "press the freeze button of the cartridge (Action Replay, Final Cartridge III, KCS Power, Retro Replay, Expert)",
        hold: false,
    },
    ActionInfo {
        action: Action::NextDisk,
        name: "next_disk",
        default: None,
        help: "swap to the next disk of the --disk list.m3u playlist (asks first if the current disk has unsaved changes)",
        hold: false,
    },
    ActionInfo {
        action: Action::PrevDisk,
        name: "prev_disk",
        default: None,
        help: "swap to the previous disk of the --disk list.m3u playlist",
        hold: false,
    },
];

impl Action {
    pub(crate) fn info(self) -> &'static ActionInfo {
        ACTIONS
            .iter()
            .find(|info| info.action == self)
            .expect("every Action has an ActionInfo")
    }

    pub(crate) fn from_name(name: &str) -> Option<Action> {
        ACTIONS
            .iter()
            .find(|info| info.name == name)
            .map(|info| info.action)
    }
}

/// One key plus the exact set of modifiers that must be held with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Chord {
    pub(crate) key: KeyCode,
    pub(crate) ctrl: bool,
    pub(crate) shift: bool,
    pub(crate) alt: bool,
    pub(crate) meta: bool,
}

pub(crate) const CTRL_KEYS: [KeyCode; 2] = [KeyCode::ControlLeft, KeyCode::ControlRight];
pub(crate) const SHIFT_KEYS: [KeyCode; 2] = [KeyCode::ShiftLeft, KeyCode::ShiftRight];
pub(crate) const ALT_KEYS: [KeyCode; 2] = [KeyCode::AltLeft, KeyCode::AltRight];
pub(crate) const META_KEYS: [KeyCode; 2] = [KeyCode::SuperLeft, KeyCode::SuperRight];

impl Chord {
    /// True when exactly this chord's modifiers are held. Ignores the chord's
    /// own key (so a chord whose key *is* a modifier, like `alt_right`, still
    /// matches) and `gate`, the `hotkeys.modifier` hold key (it enables the
    /// hotkey, it is not part of the chord: with `modifier = "alt_right"`
    /// the chord `f12` must still match while Right Alt is down).
    fn modifiers_match(&self, keyboard: &ButtonInput<KeyCode>, gate: Option<KeyCode>) -> bool {
        let held = |keys: &[KeyCode; 2]| {
            keys.iter()
                .any(|key| *key != self.key && Some(*key) != gate && keyboard.pressed(*key))
        };
        held(&CTRL_KEYS) == self.ctrl
            && held(&SHIFT_KEYS) == self.shift
            && held(&ALT_KEYS) == self.alt
            && held(&META_KEYS) == self.meta
    }

    pub(crate) fn to_text(self) -> String {
        let mut text = String::new();
        for (on, name) in [
            (self.ctrl, "ctrl+"),
            (self.shift, "shift+"),
            (self.alt, "alt+"),
            (self.meta, "super+"),
        ] {
            if on {
                text.push_str(name);
            }
        }
        text.push_str(&key_display_name(self.key));
        text
    }
}

/// Every `KeyCode` a hotkey or key binding may name. Not every Bevy key -- the named,
/// physical keys people actually bind.
const BINDABLE_KEYS: &[KeyCode] = &[
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
    KeyCode::KeyH,
    KeyCode::KeyI,
    KeyCode::KeyJ,
    KeyCode::KeyK,
    KeyCode::KeyL,
    KeyCode::KeyM,
    KeyCode::KeyN,
    KeyCode::KeyO,
    KeyCode::KeyP,
    KeyCode::KeyQ,
    KeyCode::KeyR,
    KeyCode::KeyS,
    KeyCode::KeyT,
    KeyCode::KeyU,
    KeyCode::KeyV,
    KeyCode::KeyW,
    KeyCode::KeyX,
    KeyCode::KeyY,
    KeyCode::KeyZ,
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::F1,
    KeyCode::F2,
    KeyCode::F3,
    KeyCode::F4,
    KeyCode::F5,
    KeyCode::F6,
    KeyCode::F7,
    KeyCode::F8,
    KeyCode::F9,
    KeyCode::F10,
    KeyCode::F11,
    KeyCode::F12,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::Space,
    KeyCode::Enter,
    KeyCode::Tab,
    KeyCode::Backspace,
    KeyCode::Escape,
    KeyCode::Delete,
    KeyCode::Insert,
    KeyCode::Home,
    KeyCode::End,
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Pause,
    KeyCode::PrintScreen,
    KeyCode::ScrollLock,
    KeyCode::CapsLock,
    KeyCode::NumLock,
    KeyCode::Comma,
    KeyCode::Period,
    KeyCode::Slash,
    KeyCode::Semicolon,
    KeyCode::Quote,
    KeyCode::Minus,
    KeyCode::Equal,
    KeyCode::BracketLeft,
    KeyCode::BracketRight,
    KeyCode::Backslash,
    KeyCode::Backquote,
    KeyCode::Numpad0,
    KeyCode::Numpad1,
    KeyCode::Numpad2,
    KeyCode::Numpad3,
    KeyCode::Numpad4,
    KeyCode::Numpad5,
    KeyCode::Numpad6,
    KeyCode::Numpad7,
    KeyCode::Numpad8,
    KeyCode::Numpad9,
    KeyCode::NumpadAdd,
    KeyCode::NumpadSubtract,
    KeyCode::NumpadMultiply,
    KeyCode::NumpadDivide,
    KeyCode::NumpadEnter,
    KeyCode::NumpadDecimal,
    KeyCode::ShiftLeft,
    KeyCode::ShiftRight,
    KeyCode::ControlLeft,
    KeyCode::ControlRight,
    KeyCode::AltLeft,
    KeyCode::AltRight,
    KeyCode::SuperLeft,
    KeyCode::SuperRight,
];

/// `AltRight` -> `alt_right`, `PrintScreen` -> `print_screen`, `F12` -> `f12`.
fn snake_case(debug_name: &str) -> String {
    let mut out = String::new();
    let mut previous_lower = false;
    for ch in debug_name.chars() {
        if ch.is_ascii_uppercase() && previous_lower {
            out.push('_');
        }
        previous_lower = ch.is_ascii_lowercase();
        out.push(ch.to_ascii_lowercase());
    }
    out
}

/// The QMK keycode (<https://docs.qmk.fm/keycodes_basic>) for a host key --
/// the name `--list-keys`/`--list-hotkeys` print and the config documents.
fn qmk_name(key: KeyCode) -> Option<&'static str> {
    use KeyCode::*;
    Some(match key {
        KeyA => "KC_A",
        KeyB => "KC_B",
        KeyC => "KC_C",
        KeyD => "KC_D",
        KeyE => "KC_E",
        KeyF => "KC_F",
        KeyG => "KC_G",
        KeyH => "KC_H",
        KeyI => "KC_I",
        KeyJ => "KC_J",
        KeyK => "KC_K",
        KeyL => "KC_L",
        KeyM => "KC_M",
        KeyN => "KC_N",
        KeyO => "KC_O",
        KeyP => "KC_P",
        KeyQ => "KC_Q",
        KeyR => "KC_R",
        KeyS => "KC_S",
        KeyT => "KC_T",
        KeyU => "KC_U",
        KeyV => "KC_V",
        KeyW => "KC_W",
        KeyX => "KC_X",
        KeyY => "KC_Y",
        KeyZ => "KC_Z",
        Digit1 => "KC_1",
        Digit2 => "KC_2",
        Digit3 => "KC_3",
        Digit4 => "KC_4",
        Digit5 => "KC_5",
        Digit6 => "KC_6",
        Digit7 => "KC_7",
        Digit8 => "KC_8",
        Digit9 => "KC_9",
        Digit0 => "KC_0",
        F1 => "KC_F1",
        F2 => "KC_F2",
        F3 => "KC_F3",
        F4 => "KC_F4",
        F5 => "KC_F5",
        F6 => "KC_F6",
        F7 => "KC_F7",
        F8 => "KC_F8",
        F9 => "KC_F9",
        F10 => "KC_F10",
        F11 => "KC_F11",
        F12 => "KC_F12",
        ArrowLeft => "KC_LEFT",
        ArrowRight => "KC_RGHT",
        ArrowUp => "KC_UP",
        ArrowDown => "KC_DOWN",
        Space => "KC_SPC",
        Enter => "KC_ENT",
        Tab => "KC_TAB",
        Backspace => "KC_BSPC",
        Escape => "KC_ESC",
        Delete => "KC_DEL",
        Insert => "KC_INS",
        Home => "KC_HOME",
        End => "KC_END",
        PageUp => "KC_PGUP",
        PageDown => "KC_PGDN",
        Pause => "KC_PAUS",
        PrintScreen => "KC_PSCR",
        ScrollLock => "KC_SCRL",
        CapsLock => "KC_CAPS",
        NumLock => "KC_NUM",
        Comma => "KC_COMM",
        Period => "KC_DOT",
        Slash => "KC_SLSH",
        Semicolon => "KC_SCLN",
        Quote => "KC_QUOT",
        Minus => "KC_MINS",
        Equal => "KC_EQL",
        BracketLeft => "KC_LBRC",
        BracketRight => "KC_RBRC",
        Backslash => "KC_BSLS",
        Backquote => "KC_GRV",
        Numpad0 => "KC_P0",
        Numpad1 => "KC_P1",
        Numpad2 => "KC_P2",
        Numpad3 => "KC_P3",
        Numpad4 => "KC_P4",
        Numpad5 => "KC_P5",
        Numpad6 => "KC_P6",
        Numpad7 => "KC_P7",
        Numpad8 => "KC_P8",
        Numpad9 => "KC_P9",
        NumpadAdd => "KC_PPLS",
        NumpadSubtract => "KC_PMNS",
        NumpadMultiply => "KC_PAST",
        NumpadDivide => "KC_PSLS",
        NumpadEnter => "KC_PENT",
        NumpadDecimal => "KC_PDOT",
        ShiftLeft => "KC_LSFT",
        ShiftRight => "KC_RSFT",
        ControlLeft => "KC_LCTL",
        ControlRight => "KC_RCTL",
        AltLeft => "KC_LALT",
        AltRight => "KC_RALT",
        SuperLeft => "KC_LGUI",
        SuperRight => "KC_RGUI",
        _ => return None,
    })
}

/// QMK's long-form spellings of the same keycodes (`KC_ENTER` for `KC_ENT`).
const QMK_ALIASES: &[(&str, KeyCode)] = &[
    ("KC_BACKSPACE", KeyCode::Backspace),
    ("KC_ENTER", KeyCode::Enter),
    ("KC_ESCAPE", KeyCode::Escape),
    ("KC_SPACE", KeyCode::Space),
    ("KC_DELETE", KeyCode::Delete),
    ("KC_INSERT", KeyCode::Insert),
    ("KC_RIGHT", KeyCode::ArrowRight),
    ("KC_PGDOWN", KeyCode::PageDown),
    ("KC_PAUSE", KeyCode::Pause),
    ("KC_PRINT_SCREEN", KeyCode::PrintScreen),
    ("KC_SCROLL_LOCK", KeyCode::ScrollLock),
    ("KC_CAPS_LOCK", KeyCode::CapsLock),
    ("KC_NUM_LOCK", KeyCode::NumLock),
    ("KC_COMMA", KeyCode::Comma),
    ("KC_SLASH", KeyCode::Slash),
    ("KC_SEMICOLON", KeyCode::Semicolon),
    ("KC_QUOTE", KeyCode::Quote),
    ("KC_MINUS", KeyCode::Minus),
    ("KC_EQUAL", KeyCode::Equal),
    ("KC_LEFT_BRACKET", KeyCode::BracketLeft),
    ("KC_RIGHT_BRACKET", KeyCode::BracketRight),
    ("KC_BACKSLASH", KeyCode::Backslash),
    ("KC_GRAVE", KeyCode::Backquote),
    ("KC_KP_0", KeyCode::Numpad0),
    ("KC_KP_1", KeyCode::Numpad1),
    ("KC_KP_2", KeyCode::Numpad2),
    ("KC_KP_3", KeyCode::Numpad3),
    ("KC_KP_4", KeyCode::Numpad4),
    ("KC_KP_5", KeyCode::Numpad5),
    ("KC_KP_6", KeyCode::Numpad6),
    ("KC_KP_7", KeyCode::Numpad7),
    ("KC_KP_8", KeyCode::Numpad8),
    ("KC_KP_9", KeyCode::Numpad9),
    ("KC_KP_PLUS", KeyCode::NumpadAdd),
    ("KC_KP_MINUS", KeyCode::NumpadSubtract),
    ("KC_KP_ASTERISK", KeyCode::NumpadMultiply),
    ("KC_KP_SLASH", KeyCode::NumpadDivide),
    ("KC_KP_ENTER", KeyCode::NumpadEnter),
    ("KC_KP_DOT", KeyCode::NumpadDecimal),
    ("KC_LSHIFT", KeyCode::ShiftLeft),
    ("KC_RSHIFT", KeyCode::ShiftRight),
    ("KC_LEFT_SHIFT", KeyCode::ShiftLeft),
    ("KC_RIGHT_SHIFT", KeyCode::ShiftRight),
    ("KC_LCTRL", KeyCode::ControlLeft),
    ("KC_RCTRL", KeyCode::ControlRight),
    ("KC_LEFT_CTRL", KeyCode::ControlLeft),
    ("KC_RIGHT_CTRL", KeyCode::ControlRight),
    ("KC_LEFT_ALT", KeyCode::AltLeft),
    ("KC_RIGHT_ALT", KeyCode::AltRight),
    ("KC_LOPT", KeyCode::AltLeft),
    ("KC_ROPT", KeyCode::AltRight),
    ("KC_LEFT_GUI", KeyCode::SuperLeft),
    ("KC_RIGHT_GUI", KeyCode::SuperRight),
    ("KC_LWIN", KeyCode::SuperLeft),
    ("KC_RWIN", KeyCode::SuperRight),
    ("KC_LCMD", KeyCode::SuperLeft),
    ("KC_RCMD", KeyCode::SuperRight),
];

/// QMK's shifted keycodes (`KC_PLUS` is `S(KC_EQL)`): the base key plus
/// Shift. They are chords, so they parse only where a chord does -- the left
/// side of `[keys]`, a hotkey -- and never as a bare key (`modifier = ...`).
const QMK_SHIFTED: &[(&str, KeyCode)] = &[
    ("KC_TILD", KeyCode::Backquote),
    ("KC_TILDE", KeyCode::Backquote),
    ("KC_EXLM", KeyCode::Digit1),
    ("KC_EXCLAIM", KeyCode::Digit1),
    ("KC_AT", KeyCode::Digit2),
    ("KC_HASH", KeyCode::Digit3),
    ("KC_DLR", KeyCode::Digit4),
    ("KC_DOLLAR", KeyCode::Digit4),
    ("KC_PERC", KeyCode::Digit5),
    ("KC_PERCENT", KeyCode::Digit5),
    ("KC_CIRC", KeyCode::Digit6),
    ("KC_CIRCUMFLEX", KeyCode::Digit6),
    ("KC_AMPR", KeyCode::Digit7),
    ("KC_AMPERSAND", KeyCode::Digit7),
    ("KC_ASTR", KeyCode::Digit8),
    ("KC_ASTERISK", KeyCode::Digit8),
    ("KC_LPRN", KeyCode::Digit9),
    ("KC_LEFT_PAREN", KeyCode::Digit9),
    ("KC_RPRN", KeyCode::Digit0),
    ("KC_RIGHT_PAREN", KeyCode::Digit0),
    ("KC_UNDS", KeyCode::Minus),
    ("KC_UNDERSCORE", KeyCode::Minus),
    ("KC_PLUS", KeyCode::Equal),
    ("KC_LCBR", KeyCode::BracketLeft),
    ("KC_LEFT_CURLY_BRACE", KeyCode::BracketLeft),
    ("KC_RCBR", KeyCode::BracketRight),
    ("KC_RIGHT_CURLY_BRACE", KeyCode::BracketRight),
    ("KC_PIPE", KeyCode::Backslash),
    ("KC_COLN", KeyCode::Semicolon),
    ("KC_COLON", KeyCode::Semicolon),
    ("KC_DQUO", KeyCode::Quote),
    ("KC_DOUBLE_QUOTE", KeyCode::Quote),
    ("KC_LABK", KeyCode::Comma),
    ("KC_LEFT_ANGLE_BRACKET", KeyCode::Comma),
    ("KC_RABK", KeyCode::Period),
    ("KC_RIGHT_ANGLE_BRACKET", KeyCode::Period),
    ("KC_QUES", KeyCode::Slash),
    ("KC_QUESTION", KeyCode::Slash),
];

/// QMK modifier keycodes accepted in front of a chord's key. A chord's
/// modifier matches either physical side (`KC_LCTL` also matches right Ctrl);
/// `hotkeys.modifier` is the one place a single side can be named.
fn qmk_modifier(word: &str) -> Option<ChordModifier> {
    Some(match word {
        "kc_lctl" | "kc_rctl" | "kc_lctrl" | "kc_rctrl" | "kc_left_ctrl" | "kc_right_ctrl" => {
            ChordModifier::Ctrl
        }
        "kc_lsft" | "kc_rsft" | "kc_lshift" | "kc_rshift" | "kc_left_shift" | "kc_right_shift" => {
            ChordModifier::Shift
        }
        "kc_lalt" | "kc_ralt" | "kc_lopt" | "kc_ropt" | "kc_left_alt" | "kc_right_alt" => {
            ChordModifier::Alt
        }
        "kc_lgui" | "kc_rgui" | "kc_lwin" | "kc_rwin" | "kc_lcmd" | "kc_rcmd" | "kc_left_gui"
        | "kc_right_gui" => ChordModifier::Meta,
        _ => return None,
    })
}

#[derive(Clone, Copy)]
enum ChordModifier {
    Ctrl,
    Shift,
    Alt,
    Meta,
}

/// The name shown for a host key: its QMK keycode where there is one.
pub(crate) fn key_display_name(key: KeyCode) -> String {
    match qmk_name(key) {
        Some(name) => name.to_string(),
        None => snake_case(&format!("{key:?}")),
    }
}

fn key_table() -> &'static HashMap<String, KeyCode> {
    static TABLE: OnceLock<HashMap<String, KeyCode>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = HashMap::new();
        for &key in BINDABLE_KEYS {
            let debug = format!("{key:?}");
            table.insert(stable_host_key_name(key), key);
            table.insert(debug.to_ascii_lowercase(), key);
            table.insert(snake_case(&debug), key);
        }
        for &key in BINDABLE_KEYS {
            if let Some(name) = qmk_name(key) {
                table.insert(name.to_ascii_lowercase(), key);
            }
        }
        for &(name, key) in QMK_ALIASES {
            table.insert(name.to_ascii_lowercase(), key);
        }
        table.insert("ctrl_left".to_string(), KeyCode::ControlLeft);
        table.insert("ctrl_right".to_string(), KeyCode::ControlRight);
        table.insert("esc".to_string(), KeyCode::Escape);
        table.insert("return".to_string(), KeyCode::Enter);
        table.insert("printscreen".to_string(), KeyCode::PrintScreen);
        table
    })
}

pub(crate) fn bindable_keys() -> &'static [KeyCode] {
    BINDABLE_KEYS
}

pub(crate) fn parse_key_name(name: &str) -> Option<KeyCode> {
    key_table().get(&name.trim().to_ascii_lowercase()).copied()
}

/// Parses `shift+KC_F5`, `ctrl+shift+KC_Q`, `KC_F12`, `KC_RALT` (QMK keycodes,
/// <https://docs.qmk.fm/keycodes_basic>; the older `f12`/`alt_right` spellings
/// still work). Modifier words: `ctrl`/`control`, `shift`, `alt`,
/// `super`/`meta`/`win`/`cmd`, or QMK's `KC_LCTL`/`KC_LSFT`/`KC_LALT`/`KC_LGUI`
/// (either side). A QMK shifted keycode such as `KC_PLUS` is its base key plus
/// Shift.
pub(crate) fn parse_chord(text: &str) -> Result<Chord, String> {
    let lower = text.trim().to_ascii_lowercase();
    let mut parts: Vec<&str> = lower.split('+').map(str::trim).collect();
    let key_part = parts.pop().filter(|p| !p.is_empty());
    let Some(key_part) = key_part else {
        return Err(format!("empty key in chord {text:?}"));
    };
    let shifted = QMK_SHIFTED
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key_part))
        .map(|(_, key)| *key);
    let Some(key) = shifted.or_else(|| parse_key_name(key_part)) else {
        return Err(format!("unknown key {key_part:?} in chord {text:?}"));
    };
    let mut chord = Chord {
        key,
        ctrl: false,
        shift: shifted.is_some(),
        alt: false,
        meta: false,
    };
    for modifier in parts {
        let class = match modifier {
            "ctrl" | "control" => ChordModifier::Ctrl,
            "shift" => ChordModifier::Shift,
            "alt" => ChordModifier::Alt,
            "super" | "meta" | "win" | "cmd" => ChordModifier::Meta,
            other => qmk_modifier(other).ok_or_else(|| {
                format!(
                    "unknown modifier {other:?} in chord {text:?} (use ctrl, shift, alt, super, \
                     or KC_LCTL, KC_LSFT, KC_LALT, KC_LGUI)"
                )
            })?,
        };
        let slot = match class {
            ChordModifier::Ctrl => &mut chord.ctrl,
            ChordModifier::Shift => &mut chord.shift,
            ChordModifier::Alt => &mut chord.alt,
            ChordModifier::Meta => &mut chord.meta,
        };
        if *slot {
            return Err(format!("modifier repeated in chord {text:?}"));
        }
        *slot = true;
    }
    Ok(chord)
}

/// A gamepad hotkey: every button of the chord held at once, on one pad (`pad1:select+start`) or on any pad (`pad:select+start`).
/// Written in the same `[hotkeys]` lists as keyboard chords.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PadChord {
    /// Pad slot 1-4, or any pad.
    pub(crate) slot: Option<u8>,
    pub(crate) buttons: Vec<PadButton>,
}

impl PadChord {
    pub(crate) fn to_text(&self) -> String {
        let buttons: Vec<&str> = self.buttons.iter().map(|b| b.name()).collect();
        match self.slot {
            Some(slot) => format!("pad{slot}:{}", buttons.join("+")),
            None => format!("pad:{}", buttons.join("+")),
        }
    }
}

/// `Some` when `text` is a pad chord (`pad:`/`padN:` then buttons joined by `+`): `Ok` with the chord or `Err` naming what is
/// wrong with it; `None` for a keyboard chord.
pub(crate) fn parse_pad_chord(text: &str) -> Option<Result<PadChord, String>> {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower.strip_prefix("pad")?;
    let (digits, buttons) = rest.split_once(':')?;
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let slot = if digits.is_empty() {
        None
    } else {
        match digits.parse::<u8>() {
            Ok(n) if (1..=MAX_PADS).contains(&n) => Some(n),
            _ => {
                return Some(Err(format!(
                    "\"{trimmed}\": pad slot {digits} does not exist (pad1 to pad{MAX_PADS}, or just pad)"
                )));
            }
        }
    };
    let mut list = Vec::new();
    for name in buttons.split('+') {
        let Some(button) = PadButton::parse(name) else {
            return Some(Err(format!(
                "\"{trimmed}\": unknown pad button \"{}\" ({})",
                name.trim(),
                PadButton::ALL.map(PadButton::name).join(", ")
            )));
        };
        if list.contains(&button) {
            return Some(Err(format!(
                "\"{trimmed}\": button \"{}\" repeated",
                button.name()
            )));
        }
        list.push(button);
    }
    Some(Ok(PadChord {
        slot,
        buttons: list,
    }))
}

/// The resolved hotkey table for one machine.
#[derive(Resource, Clone, Debug, Default)]
pub(crate) struct Hotkeys {
    bindings: Vec<(Action, Chord)>,
    /// Gamepad chords (`pad1:select+start`); they need no `modifier` key.
    pad_bindings: Vec<(Action, PadChord)>,
    /// Hold-to-enable key from `hotkeys.modifier`; every chord needs it held.
    modifier: Option<KeyCode>,
}

/// Mutable bookkeeping `Hotkeys::poll` needs between frames.
#[derive(Resource, Default, Debug)]
pub(crate) struct HotkeyState {
    /// Keys whose press fired a hotkey; their press AND release are hidden
    /// from the emulated machine so it never sees half a key stroke.
    swallowed: HashSet<KeyCode>,
    /// Hold actions currently active.
    held: HashSet<Action>,
}

impl HotkeyState {
    /// True when `key`'s current press/release belongs to a hotkey.
    pub(crate) fn is_swallowed(&self, key: KeyCode) -> bool {
        self.swallowed.contains(&key)
    }

    #[cfg(test)]
    pub(crate) fn swallow(&mut self, key: KeyCode) {
        self.swallowed.insert(key);
    }

    /// Hides every key currently down from the machine (and keeps hiding
    /// each until it is released). Used while a file dialog or its question
    /// is open, so nothing typed there reaches the emulated computer.
    pub(crate) fn swallow_all(&mut self, keyboard: &ButtonInput<KeyCode>) {
        self.swallowed.extend(keyboard.get_pressed().copied());
    }
}

/// Bookkeeping `Hotkeys::poll_pads` needs between frames.
#[derive(Resource, Default, Debug)]
pub(crate) struct PadHotkeyState {
    /// Pad chords that are held down now (index into the table) and the pad they are held on.
    active: HashMap<usize, u8>,
    /// Buttons of a chord that fired: hidden from the machine until released, like a swallowed key.
    swallowed: HashSet<(u8, PadButton)>,
}

impl PadHotkeyState {
    /// Clears the swallowed buttons from the pad states before they are routed to the machine.
    pub(crate) fn mask(&self, pads: &mut [(u8, PadState)]) {
        for (slot, state) in pads {
            for (swallowed_slot, button) in &self.swallowed {
                if swallowed_slot == slot {
                    state.set(*button, false);
                }
            }
        }
    }
}

/// What the pad hotkeys need between frames: the chord bookkeeping, and the triggers `pad_input` found, which it hands to
/// `hotkey_input` (the system that owns everything an action needs) for its next run.
#[derive(Resource, Default, Debug)]
pub(crate) struct PadTriggers {
    pub(crate) state: PadHotkeyState,
    pub(crate) triggers: Vec<Trigger>,
}

/// What a hotkey did this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// A one-shot action fired, or a hold action began.
    Pressed(Action),
    /// A hold action ended.
    Released(Action),
}

impl Hotkeys {
    /// Builds the table: built-in defaults, then `[hotkeys]`, then
    /// `[machines.<machine>.hotkeys]`, per binding. Errors name the config
    /// section and the valid choices.
    pub(crate) fn resolve(config: &AppConfig, machine: &str) -> Result<Hotkeys, String> {
        let mut table: HashMap<Action, Vec<String>> = ACTIONS
            .iter()
            .map(|info| {
                (
                    info.action,
                    info.default
                        .map(|chord| vec![chord.to_string()])
                        .unwrap_or_default(),
                )
            })
            .collect();
        let machine_layer = config.machines.get(machine).map(|m| &m.hotkeys);
        let mut modifier: Option<String> = None;
        let layers: [(Option<&HotkeysConfig>, String); 2] = [
            (Some(&config.hotkeys), "[hotkeys]".to_string()),
            (machine_layer, format!("[machines.{machine}.hotkeys]")),
        ];
        for (layer, section) in layers {
            let Some(layer) = layer else { continue };
            if let Some(value) = &layer.modifier {
                modifier = Some(value.clone());
            }
            for (name, chords) in &layer.bindings {
                let Some(action) = Action::from_name(name) else {
                    let names: Vec<&str> = ACTIONS.iter().map(|info| info.name).collect();
                    return Err(format!(
                        "unknown hotkey action \"{name}\" in {section} (actions: modifier, {})",
                        names.join(", ")
                    ));
                };
                let list = chords.as_slice();
                let entry = if list.iter().any(|c| c.trim().eq_ignore_ascii_case("none")) {
                    Vec::new()
                } else {
                    list.to_vec()
                };
                table.insert(action, entry);
            }
        }

        let modifier = match modifier.as_deref().map(str::trim) {
            None | Some("") | Some("none") => None,
            Some(name) => Some(parse_key_name(name).ok_or_else(|| {
                format!("unknown key \"{name}\" for hotkeys.modifier (try alt_right, ctrl_right)")
            })?),
        };

        let mut bindings: Vec<(Action, Chord)> = Vec::new();
        let mut pad_bindings: Vec<(Action, PadChord)> = Vec::new();
        for info in &ACTIONS {
            for text in &table[&info.action] {
                if let Some(parsed) = parse_pad_chord(text) {
                    let chord = parsed.map_err(|e| format!("hotkey \"{}\": {e}", info.name))?;
                    if let Some((other, _)) = pad_bindings.iter().find(|(_, c)| *c == chord) {
                        return Err(format!(
                            "hotkey chord \"{}\" is bound to both \"{}\" and \"{}\" -- rebind one \
                             (or set it to \"none\")",
                            chord.to_text(),
                            other.info().name,
                            info.name
                        ));
                    }
                    pad_bindings.push((info.action, chord));
                    continue;
                }
                let chord =
                    parse_chord(text).map_err(|e| format!("hotkey \"{}\": {e}", info.name))?;
                if let Some((other, _)) = bindings.iter().find(|(_, c)| *c == chord) {
                    return Err(format!(
                        "hotkey chord \"{}\" is bound to both \"{}\" and \"{}\" -- rebind one \
                         (or set it to \"none\")",
                        chord.to_text(),
                        other.info().name,
                        info.name
                    ));
                }
                bindings.push((info.action, chord));
            }
        }
        Ok(Hotkeys {
            bindings,
            pad_bindings,
            modifier,
        })
    }

    /// Chords bound to `action` (for listings and tests).
    pub(crate) fn chords_for(&self, action: Action) -> Vec<Chord> {
        self.bindings
            .iter()
            .filter(|(a, _)| *a == action)
            .map(|(_, chord)| *chord)
            .collect()
    }

    /// Gamepad chords bound to `action`.
    pub(crate) fn pad_chords_for(&self, action: Action) -> Vec<PadChord> {
        self.pad_bindings
            .iter()
            .filter(|(a, _)| *a == action)
            .map(|(_, chord)| chord.clone())
            .collect()
    }

    pub(crate) fn modifier(&self) -> Option<KeyCode> {
        self.modifier
    }

    /// Every bound chord with the action it triggers.
    pub(crate) fn all_chords(&self) -> impl Iterator<Item = (Action, Chord)> + '_ {
        self.bindings.iter().copied()
    }

    /// Compares the keyboard state to the table for one frame. Call once per
    /// frame, before the system that forwards keys to the machine, which
    /// must skip keys for which `state.is_swallowed` is true.
    pub(crate) fn poll(
        &self,
        keyboard: &ButtonInput<KeyCode>,
        state: &mut HotkeyState,
    ) -> Vec<Trigger> {
        // Forget swallowed keys once their release has been seen: a key that
        // is neither held nor released this frame is done.
        state
            .swallowed
            .retain(|key| keyboard.pressed(*key) || keyboard.just_released(*key));

        let modifier_ok = self.modifier.is_none_or(|key| keyboard.pressed(key));
        let mut triggers = Vec::new();
        for (action, chord) in &self.bindings {
            if modifier_ok
                && keyboard.just_pressed(chord.key)
                && chord.modifiers_match(keyboard, self.modifier)
            {
                state.swallowed.insert(chord.key);
                if action.info().hold {
                    state.held.insert(*action);
                }
                triggers.push(Trigger::Pressed(*action));
            }
            if action.info().hold
                && state.held.contains(action)
                && !keyboard.pressed(chord.key)
                && self
                    .bindings
                    .iter()
                    .filter(|(a, _)| a == action)
                    .all(|(_, c)| !keyboard.pressed(c.key))
            {
                state.held.remove(action);
                triggers.push(Trigger::Released(*action));
            }
        }
        triggers
    }

    /// Compares the connected pads (slot, buttons held) to the pad chords for one frame: a chord fires when its last button goes
    /// down (a hold action also reports its end) and its buttons are swallowed until released (`PadHotkeyState::mask`). The
    /// first button of a two-button chord reaches the machine a moment before the second one arrives, as a key would.
    pub(crate) fn poll_pads(
        &self,
        pads: &[(u8, PadState)],
        state: &mut PadHotkeyState,
    ) -> Vec<Trigger> {
        state.swallowed.retain(|(slot, button)| {
            pads.iter()
                .any(|(s, pad)| s == slot && pad.is_down(*button))
        });
        let mut triggers = Vec::new();
        for (index, (action, chord)) in self.pad_bindings.iter().enumerate() {
            let held_on = pads
                .iter()
                .find(|(slot, pad)| {
                    chord.slot.is_none_or(|want| want == *slot)
                        && chord.buttons.iter().all(|b| pad.is_down(*b))
                })
                .map(|(slot, _)| *slot);
            match (held_on, state.active.contains_key(&index)) {
                (Some(slot), false) => {
                    state.active.insert(index, slot);
                    for button in &chord.buttons {
                        state.swallowed.insert((slot, *button));
                    }
                    triggers.push(Trigger::Pressed(*action));
                }
                (None, true) => {
                    state.active.remove(&index);
                    if action.info().hold {
                        triggers.push(Trigger::Released(*action));
                    }
                }
                _ => {}
            }
        }
        triggers
    }

    /// Plain-text table for `--list-hotkeys`.
    pub(crate) fn describe(&self) -> String {
        let width = crate::listing::text_width();
        let mut out = String::new();
        if let Some(key) = self.modifier {
            out.push_str(&format!(
                "hold {} with a hotkey to trigger it (hotkeys.modifier)\n\n",
                key_display_name(key)
            ));
        }
        out.push_str(&format!(
            "{:<16} {:<24} {}\n",
            "action", "bound to", "what it does"
        ));
        for info in &ACTIONS {
            let chords: Vec<String> = self
                .chords_for(info.action)
                .into_iter()
                .map(Chord::to_text)
                .chain(
                    self.pad_chords_for(info.action)
                        .iter()
                        .map(PadChord::to_text),
                )
                .collect();
            let bound = if chords.is_empty() {
                "(unbound)".to_string()
            } else {
                chords.join(", ")
            };
            crate::listing::wrap_into(
                &mut out,
                info.help,
                width,
                &format!("{:<16} {:<24} ", info.name, bound),
                &" ".repeat(42),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> AppConfig {
        app_config::parse(text).unwrap()
    }

    fn defaults() -> Hotkeys {
        Hotkeys::resolve(&AppConfig::default(), "zx-spectrum").unwrap()
    }

    fn keyboard_with(pressed: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut keyboard = ButtonInput::<KeyCode>::default();
        for key in pressed {
            keyboard.press(*key);
        }
        keyboard
    }

    fn pad(slot: u8, buttons: &[PadButton]) -> (u8, PadState) {
        let mut state = PadState::default();
        for b in buttons {
            state.set(*b, true);
        }
        (slot, state)
    }

    #[test]
    fn pad_chords_parse_with_a_slot_or_for_any_pad() {
        let chord = parse_pad_chord("pad1:Select+Start").unwrap().unwrap();
        assert_eq!(chord.slot, Some(1));
        assert_eq!(chord.buttons, [PadButton::Select, PadButton::Start]);
        assert_eq!(chord.to_text(), "pad1:select+start");
        let any = parse_pad_chord("PAD:l1+r1+a").unwrap().unwrap();
        assert_eq!(any.slot, None);
        assert_eq!(
            any.buttons,
            [PadButton::L1, PadButton::R1, PadButton::South]
        );
        // Keyboard chords are not pad chords, errors name the problem.
        assert!(parse_pad_chord("ctrl+f5").is_none());
        assert!(parse_pad_chord("pause").is_none());
        let err = |text: &str| parse_pad_chord(text).unwrap().unwrap_err();
        assert!(err("pad9:start").contains("does not exist"));
        assert!(err("pad:turbo").contains("unknown pad button"));
        assert!(err("pad:start+start").contains("repeated"));
    }

    #[test]
    fn a_pad_chord_resolves_beside_keyboard_chords_and_is_listed() {
        let hotkeys = Hotkeys::resolve(
            &config("[hotkeys]\nreset = [\"f12\", \"pad1:select+start\"]\npause = \"pad:start\""),
            "c64",
        )
        .unwrap();
        let reset = Action::from_name("reset").unwrap();
        assert_eq!(
            hotkeys.chords_for(reset).len(),
            1,
            "the keyboard chord stays"
        );
        assert_eq!(
            hotkeys.pad_chords_for(reset)[0].to_text(),
            "pad1:select+start"
        );
        let listing = hotkeys.describe();
        assert!(listing.contains("KC_F12, pad1:select+start"), "{listing}");
        // The same pad chord on two actions is the same error as for keys; a typo is reported with the action.
        let both = Hotkeys::resolve(
            &config("[hotkeys]\nreset = \"pad:start\"\npause = \"pad:start\""),
            "c64",
        )
        .unwrap_err();
        assert!(both.contains("bound to both"), "{both}");
        let bad = Hotkeys::resolve(&config("[hotkeys]\nreset = \"pad:warp\""), "c64").unwrap_err();
        assert!(
            bad.contains("hotkey \"reset\"") && bad.contains("unknown pad button"),
            "{bad}"
        );
    }

    fn a_pad_chord_fires_once_on_its_last_button_and_hides_its_buttons() {
        let hotkeys = Hotkeys::resolve(
            &config("[hotkeys]\nreset = \"pad1:select+start\"\nfast_forward = \"pad:r2\""),
            "c64",
        )
        .unwrap();
        let reset = Action::from_name("reset").unwrap();
        let fast = Action::from_name("fast_forward").unwrap();
        let mut state = PadHotkeyState::default();
        let mut poll = |pads: &[(u8, PadState)]| hotkeys.poll_pads(pads, &mut state);

        assert!(
            poll(&[pad(1, &[PadButton::Select])]).is_empty(),
            "half a chord does nothing"
        );
        assert_eq!(
            poll(&[pad(1, &[PadButton::Select, PadButton::Start])]),
            [Trigger::Pressed(reset)]
        );
        assert!(
            poll(&[pad(1, &[PadButton::Select, PadButton::Start])]).is_empty(),
            "held: no repeat"
        );
        // Another pad's buttons do not count for `pad1:`; a hold action on any pad reports both edges.
        assert!(poll(&[pad(1, &[]), pad(2, &[PadButton::Select, PadButton::Start])]).is_empty());
        assert_eq!(poll(&[pad(2, &[PadButton::R2])]), [Trigger::Pressed(fast)]);
        assert_eq!(poll(&[pad(2, &[])]), [Trigger::Released(fast)]);
    }

    fn swallowed_pad_buttons_are_masked_until_they_are_let_go() {
        let hotkeys =
            Hotkeys::resolve(&config("[hotkeys]\nreset = \"pad1:select+start\""), "c64").unwrap();
        let mut state = PadHotkeyState::default();
        let mut pads = [pad(
            1,
            &[PadButton::Select, PadButton::Start, PadButton::South],
        )];
        assert_eq!(hotkeys.poll_pads(&pads, &mut state).len(), 1);
        state.mask(&mut pads);
        assert!(!pads[0].1.is_down(PadButton::Select) && !pads[0].1.is_down(PadButton::Start));
        assert!(
            pads[0].1.is_down(PadButton::South),
            "other buttons still reach the machine"
        );
        // Start is let go and pressed again alone: the chord is over, so Start is the machine's again once Select is released
        // and Start comes back.
        let mut pads = [pad(1, &[PadButton::Select])];
        hotkeys.poll_pads(&pads, &mut state);
        state.mask(&mut pads);
        assert!(
            !pads[0].1.is_down(PadButton::Select),
            "Select was not let go yet"
        );
        let mut pads = [pad(1, &[])];
        hotkeys.poll_pads(&pads, &mut state);
        state.mask(&mut pads);
        let mut pads = [pad(1, &[PadButton::Start])];
        hotkeys.poll_pads(&pads, &mut state);
        state.mask(&mut pads);
        assert!(pads[0].1.is_down(PadButton::Start));
    }
    /// Merged (every check kept): `a_pad_chord_fires_once_on_its_last_button_and_hides_its_buttons`, `swallowed_pad_buttons_are_masked_until_they_are_let_go`.
    #[test]
    fn pad_chords_fire_once_and_mask_their_buttons() {
        a_pad_chord_fires_once_on_its_last_button_and_hides_its_buttons();
        swallowed_pad_buttons_are_masked_until_they_are_let_go();
    }

    #[test]
    fn chords_parse_with_modifiers_and_aliases() {
        let chord = parse_chord("Ctrl+Shift+F5").unwrap();
        assert_eq!(chord.key, KeyCode::F5);
        assert!(chord.ctrl && chord.shift && !chord.alt && !chord.meta);
        assert_eq!(
            parse_chord("print_screen").unwrap().key,
            KeyCode::PrintScreen
        );
        assert_eq!(
            parse_chord("printscreen").unwrap().key,
            KeyCode::PrintScreen
        );
        assert_eq!(parse_chord("alt_right").unwrap().key, KeyCode::AltRight);
        assert_eq!(parse_chord("scroll_lock").unwrap().key, KeyCode::ScrollLock);
        assert_eq!(parse_chord("q").unwrap().key, KeyCode::KeyQ);
        assert_eq!(parse_chord("0").unwrap().key, KeyCode::Digit0);
    }

    #[test]
    fn bad_chords_say_what_is_wrong() {
        assert!(parse_chord("").unwrap_err().contains("empty"));
        assert!(parse_chord("shift+").unwrap_err().contains("empty"));
        assert!(parse_chord("hyper+f1").unwrap_err().contains("hyper"));
        assert!(
            parse_chord("ctrl+ctrl+f1")
                .unwrap_err()
                .contains("repeated")
        );
        assert!(parse_chord("f13x").unwrap_err().contains("unknown key"));
    }

    fn qmk_keycodes_name_the_same_keys_as_the_older_spellings() {
        for (qmk, old) in [
            ("KC_F12", "f12"),
            ("kc_ralt", "alt_right"),
            ("KC_ENT", "enter"),
            ("KC_ENTER", "enter"),
            ("KC_SPC", "space"),
            ("KC_RGHT", "right"),
            ("KC_GRV", "backquote"),
            ("KC_MINS", "minus"),
            ("KC_EQL", "equal"),
            ("KC_SCLN", "semicolon"),
            ("KC_BSLS", "backslash"),
            ("KC_PSCR", "print_screen"),
            ("KC_P7", "numpad7"),
            ("KC_PPLS", "numpad_add"),
            ("KC_1", "1"),
            ("KC_Q", "q"),
        ] {
            assert_eq!(parse_key_name(qmk), parse_key_name(old), "{qmk} vs {old}");
            assert!(parse_key_name(qmk).is_some(), "{qmk}");
        }
    }

    fn qmk_modifiers_and_shifted_keycodes_parse_in_chords() {
        let chord = parse_chord("KC_LCTL+KC_LSFT+KC_F9").unwrap();
        assert_eq!(chord.key, KeyCode::F9);
        assert!(chord.ctrl && chord.shift && !chord.alt && !chord.meta);
        assert!(parse_chord("KC_LGUI+KC_Q").unwrap().meta);
        // KC_PLUS is Shift + the "=" key; KC_AT is Shift + 2.
        let plus = parse_chord("KC_PLUS").unwrap();
        assert_eq!((plus.key, plus.shift), (KeyCode::Equal, true));
        let at = parse_chord("KC_AT").unwrap();
        assert_eq!((at.key, at.shift), (KeyCode::Digit2, true));
        // ...but never a bare key, and it cannot be shifted twice.
        assert_eq!(parse_key_name("KC_PLUS"), None);
        assert!(
            parse_chord("shift+KC_PLUS")
                .unwrap_err()
                .contains("repeated")
        );
        assert!(parse_chord("KC_NOPE").unwrap_err().contains("unknown key"));
    }
    /// Merged (every check kept): `qmk_keycodes_name_the_same_keys_as_the_older_spellings`, `qmk_modifiers_and_shifted_keycodes_parse_in_chords`.
    #[test]
    fn qmk_keycodes() {
        qmk_keycodes_name_the_same_keys_as_the_older_spellings();
        qmk_modifiers_and_shifted_keycodes_parse_in_chords();
    }

    fn chord_text_round_trips() {
        for text in [
            "KC_F12",
            "shift+KC_F11",
            "ctrl+shift+KC_Q",
            "KC_RALT",
            "KC_PSCR",
        ] {
            let chord = parse_chord(text).unwrap();
            assert_eq!(parse_chord(&chord.to_text()).unwrap(), chord, "{text}");
            assert_eq!(chord.to_text(), text);
        }
    }

    fn every_bindable_key_has_a_name_that_parses_back() {
        for &key in BINDABLE_KEYS {
            let name = key_display_name(key);
            assert_eq!(parse_key_name(&name), Some(key), "{key:?} shown as {name}");
        }
    }
    /// Merged (every check kept): `chord_text_round_trips`, `every_bindable_key_has_a_name_that_parses_back`.
    #[test]
    fn chord_text_round_trips_for_every_key() {
        chord_text_round_trips();
        every_bindable_key_has_a_name_that_parses_back();
    }

    #[test]
    fn built_in_defaults_are_conflict_free_and_complete() {
        let hotkeys = defaults();
        for info in &ACTIONS {
            assert_eq!(
                hotkeys.chords_for(info.action).is_empty(),
                info.default.is_none(),
                "{}",
                info.name
            );
        }
        // Today's keys are unchanged.
        assert_eq!(hotkeys.chords_for(Action::Reset)[0].key, KeyCode::F12);
        assert_eq!(hotkeys.chords_for(Action::Fullscreen)[0].key, KeyCode::F11);
    }

    #[test]
    fn defaults_avoid_every_key_a_machine_maps_on_its_own() {
        // F1-F8 are C64 function keys (and the letters/digits/symbols are
        // everyone's): a plain default chord on those would steal them.
        let taken = [
            KeyCode::F1,
            KeyCode::F2,
            KeyCode::F3,
            KeyCode::F4,
            KeyCode::F5,
            KeyCode::F6,
            KeyCode::F7,
            KeyCode::F8,
            KeyCode::Tab,
            KeyCode::Escape,
        ];
        for chord in defaults().bindings.iter().map(|(_, c)| c) {
            assert!(!taken.contains(&chord.key), "{}", chord.to_text());
        }
    }

    #[test]
    fn global_then_machine_layers_override_per_binding() {
        let cfg = config(
            r#"
            [hotkeys]
            reset = "f8"
            mute = "none"

            [machines.c64.hotkeys]
            reset = "ctrl+f8"
            "#,
        );
        let spectrum = Hotkeys::resolve(&cfg, "zx-spectrum").unwrap();
        assert_eq!(spectrum.chords_for(Action::Reset)[0].to_text(), "KC_F8");
        assert!(spectrum.chords_for(Action::Mute).is_empty());
        let c64 = Hotkeys::resolve(&cfg, "c64").unwrap();
        assert_eq!(c64.chords_for(Action::Reset)[0].to_text(), "ctrl+KC_F8");
        // Untouched actions keep their defaults.
        assert_eq!(c64.chords_for(Action::Fullscreen)[0].key, KeyCode::F11);
    }

    #[test]
    fn several_chords_can_share_one_action() {
        let cfg = config("[hotkeys]\nreset = [\"f12\", \"ctrl+r\"]");
        let hotkeys = Hotkeys::resolve(&cfg, "c64").unwrap();
        assert_eq!(hotkeys.chords_for(Action::Reset).len(), 2);
    }

    #[test]
    fn resolve_rejects_unknown_actions_keys_and_conflicts() {
        let err = |text: &str| Hotkeys::resolve(&config(text), "c64").unwrap_err();
        assert!(err("[hotkeys]\nrset = \"f1\"").contains("unknown hotkey action \"rset\""));
        assert!(err("[hotkeys]\nreset = \"f99\"").contains("unknown key"));
        assert!(err("[hotkeys]\nmodifier = \"hyper\"").contains("hotkeys.modifier"));
        let conflict = err("[hotkeys]\nmute = \"f12\"");
        assert!(conflict.contains("bound to both"), "{conflict}");
        assert!(
            conflict.contains("reset") && conflict.contains("mute"),
            "{conflict}"
        );
        // Unbinding the old owner resolves the conflict.
        assert!(
            Hotkeys::resolve(
                &config("[hotkeys]\nmute = \"f12\"\nreset = \"none\""),
                "c64"
            )
            .is_ok()
        );
    }

    #[test]
    fn a_plain_chord_fires_only_with_exactly_its_modifiers() {
        let hotkeys = defaults();
        let mut state = HotkeyState::default();

        let mut keyboard = keyboard_with(&[KeyCode::F12]);
        assert_eq!(
            hotkeys.poll(&keyboard, &mut state),
            vec![Trigger::Pressed(Action::Reset)]
        );

        // Shift+F12 is quick_load, not reset.
        let mut state = HotkeyState::default();
        keyboard = keyboard_with(&[KeyCode::ShiftLeft, KeyCode::F12]);
        assert_eq!(
            hotkeys.poll(&keyboard, &mut state),
            vec![Trigger::Pressed(Action::QuickLoad)]
        );

        // Ctrl+Alt+F12 matches nothing (Ctrl+Shift+F12 is save_snapshot).
        let mut state = HotkeyState::default();
        keyboard = keyboard_with(&[KeyCode::ControlLeft, KeyCode::AltRight, KeyCode::F12]);
        assert!(hotkeys.poll(&keyboard, &mut state).is_empty());
        assert!(!state.is_swallowed(KeyCode::F12));
    }

    #[test]
    fn a_held_key_fires_once_and_its_release_is_hidden_from_the_machine() {
        let hotkeys = defaults();
        let mut state = HotkeyState::default();
        let mut keyboard = keyboard_with(&[KeyCode::F12]);
        assert_eq!(hotkeys.poll(&keyboard, &mut state).len(), 1);
        assert!(state.is_swallowed(KeyCode::F12));

        // Next frame: still held, no new press -> no new trigger, still swallowed.
        keyboard.clear();
        assert!(hotkeys.poll(&keyboard, &mut state).is_empty());
        assert!(state.is_swallowed(KeyCode::F12));

        // Release frame: the machine-side system must still see it swallowed.
        keyboard.release(KeyCode::F12);
        assert!(hotkeys.poll(&keyboard, &mut state).is_empty());
        assert!(
            state.is_swallowed(KeyCode::F12),
            "release must be hidden too"
        );

        // The frame after, it is forgotten.
        keyboard.clear();
        hotkeys.poll(&keyboard, &mut state);
        assert!(!state.is_swallowed(KeyCode::F12));
    }

    #[test]
    fn keys_that_are_not_hotkeys_are_never_swallowed() {
        let hotkeys = defaults();
        let mut state = HotkeyState::default();
        let keyboard = keyboard_with(&[KeyCode::F5, KeyCode::KeyA]);
        assert!(hotkeys.poll(&keyboard, &mut state).is_empty());
        assert!(!state.is_swallowed(KeyCode::F5) && !state.is_swallowed(KeyCode::KeyA));
    }

    #[test]
    fn the_hold_modifier_gates_every_hotkey() {
        let cfg = config("[hotkeys]\nmodifier = \"alt_right\"");
        let hotkeys = Hotkeys::resolve(&cfg, "c64").unwrap();
        assert_eq!(hotkeys.modifier(), Some(KeyCode::AltRight));

        // Without the modifier the key is the machine's.
        let mut state = HotkeyState::default();
        let keyboard = keyboard_with(&[KeyCode::F12]);
        assert!(hotkeys.poll(&keyboard, &mut state).is_empty());
        assert!(!state.is_swallowed(KeyCode::F12));

        // With it held the hotkey fires. (The chord itself has no alt, and the
        // modifier key is not counted as the chord's own `alt`.)
        let mut state = HotkeyState::default();
        let mut keyboard = keyboard_with(&[KeyCode::AltRight]);
        keyboard.clear();
        keyboard.press(KeyCode::F12);
        let triggers = hotkeys.poll(&keyboard, &mut state);
        assert_eq!(triggers, vec![Trigger::Pressed(Action::Reset)]);
    }

    #[test]
    fn a_chord_on_a_modifier_key_itself_matches() {
        let cfg = config("[hotkeys]\nmute = \"alt_right\"");
        let hotkeys = Hotkeys::resolve(&cfg, "c64").unwrap();
        let mut state = HotkeyState::default();
        let keyboard = keyboard_with(&[KeyCode::AltRight]);
        assert_eq!(
            hotkeys.poll(&keyboard, &mut state),
            vec![Trigger::Pressed(Action::Mute)]
        );
    }
}
