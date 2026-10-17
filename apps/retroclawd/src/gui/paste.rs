//! Paste: types the host clipboard's text into the emulated machine, one key at
//! a time, as if it had been typed on the real keyboard.
//!
//! A retro machine has no clipboard, so "paste" means *typing*: each character
//! becomes the machine key the plugin says types it (`MachineKeyInfo::chars`,
//! the same table the symbolic keyboard layout uses), pressed and released with
//! a pause between events so the machine's own keyboard scan sees every key.
//! The planning (`plan`) is a pure function of the text and the key tables;
//! `PasteState`/`paste_typing` just drip the planned events into the input
//! channel the real keyboard uses, and `start` reads the clipboard.
//!
//! Policy decisions, all configurable under `[input]`:
//! - `paste_case = "fold"` (default) types every letter as the plain, unshifted
//!   key. That is what an uppercase BASIC listing needs on a C64 (Shift+letter
//!   is a graphics glyph there); `"exact"` holds the machine's Shift for
//!   capitals instead (right for Spectrum/CPC text).
//! - `paste_delay_ms` (default 60) is the time of each press and each release.
//!
//! Typographic quotes/dashes are folded to their ASCII twins and tabs become
//! spaces; a character the machine has no key for is skipped and counted.

use std::collections::VecDeque;
use std::time::Duration;

use bevy::prelude::*;

use super::input::InputTxRes;
use super::keys::{KeyRouter, SymbolMap};
use super::toast::ToastRes;

/// How long each press and each release lasts unless `paste_delay_ms` says
/// otherwise: three 50 Hz frames, enough for the slowest keyboard scan here
/// (the Spectrum ROM wants a key seen on consecutive interrupts).
pub(crate) const DEFAULT_PASTE_DELAY_MS: u64 = 60;

/// What happens to the case of pasted letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PasteCase {
    /// Every letter is the plain key (a capital is *not* shifted).
    #[default]
    Fold,
    /// A capital holds the machine's Shift.
    Exact,
}

impl PasteCase {
    pub(crate) fn parse(text: &str) -> Result<PasteCase, String> {
        match text.trim().to_ascii_lowercase().as_str() {
            "fold" => Ok(PasteCase::Fold),
            "exact" => Ok(PasteCase::Exact),
            other => Err(format!(
                "unknown paste_case {other:?} (expected \"fold\" or \"exact\")"
            )),
        }
    }
}

/// The machine key events that type some text.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PastePlan {
    /// `(machine key name, pressed)` in order.
    pub(crate) events: VecDeque<(String, bool)>,
    /// Characters that became key presses.
    pub(crate) typed: usize,
    /// Characters the machine has no key for.
    pub(crate) skipped: usize,
}

/// The ASCII character a pasted one stands for: typographic quotes and
/// dashes from web pages and word processors, no-break space, tab.
fn normalize(c: char) -> char {
    match c {
        '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' => '\'',
        '\u{201C}' | '\u{201D}' | '\u{201F}' | '\u{2033}' => '"',
        '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
        '\u{00A0}' | '\u{2007}' | '\u{202F}' | '\t' => ' ',
        other => other,
    }
}

/// Turns `text` into the key events that type it. `enter` and `shift` are the
/// machine key names the host's Enter and Shift keys send (`KeyRouter::
/// plain_names`).
pub(crate) fn plan(
    text: &str,
    symbols: &SymbolMap,
    enter: &[String],
    shift: &[String],
    case: PasteCase,
) -> PastePlan {
    let mut plan = PastePlan::default();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let c = normalize(c);
        // CR LF is one Enter; a lone CR or LF is one too.
        if c == '\r' || c == '\n' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            for name in enter {
                plan.events.push_back((name.clone(), true));
            }
            for name in enter.iter().rev() {
                plan.events.push_back((name.clone(), false));
            }
            plan.typed += 1;
            continue;
        }
        // A character the machine has no key for is typed as the plain letters it folds to (`é` as `e`, `ß` as `ss`) if it
        // has keys for those.
        let typed: Vec<char> = if symbols.key_for(c).is_some() {
            vec![c]
        } else {
            match super::fold::ascii_fold(c) {
                Some(plain) if plain.chars().all(|p| symbols.key_for(p).is_some()) => {
                    plain.chars().collect()
                }
                _ => Vec::new(),
            }
        };
        if typed.is_empty() {
            plan.skipped += 1;
            continue;
        }
        for c in typed {
            let key = symbols.key_for(c).expect("checked above");
            let capital = case == PasteCase::Exact && c.is_ascii_uppercase();
            if capital {
                for name in shift {
                    plan.events.push_back((name.clone(), true));
                }
            }
            plan.events.push_back((key.to_string(), true));
            plan.events.push_back((key.to_string(), false));
            if capital {
                for name in shift.iter().rev() {
                    plan.events.push_back((name.clone(), false));
                }
            }
        }
        plan.typed += 1;
    }
    plan
}

/// The typing in progress, drained by `paste_typing`.
#[derive(Resource)]
pub(crate) struct PasteState {
    queue: VecDeque<(String, bool)>,
    /// Keys the paste has pressed and not yet released (released on cancel).
    down: Vec<String>,
    timer: Timer,
    case: PasteCase,
}

impl PasteState {
    pub(crate) fn new(case: PasteCase, delay_ms: u64) -> Self {
        PasteState {
            queue: VecDeque::new(),
            down: Vec::new(),
            timer: Timer::new(Duration::from_millis(delay_ms.max(1)), TimerMode::Repeating),
            case,
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Drops what is left and returns the releases for every key still held.
    fn cancel(&mut self) -> Vec<(String, bool)> {
        self.queue.clear();
        self.down
            .drain(..)
            .rev()
            .map(|name| (name, false))
            .collect()
    }
}

/// What the paste hotkey does: a second press while typing cancels it,
/// otherwise the clipboard's text is planned and queued. Returns the message
/// to show.
pub(crate) fn toggle(
    state: &mut PasteState,
    router: &KeyRouter,
    input_tx: &InputTxRes,
    clipboard: impl FnOnce() -> Result<String, String>,
) -> String {
    if state.is_active() {
        for event in state.cancel() {
            let _ = input_tx.0.send(event);
        }
        return "Paste cancelled".to_string();
    }
    let text = match clipboard() {
        Ok(text) => text,
        Err(e) => return format!("Paste failed: {e}"),
    };
    if text.is_empty() {
        return "Clipboard has no text".to_string();
    }
    let enter = router.plain_names(KeyCode::Enter);
    let shift = router.plain_names(KeyCode::ShiftLeft);
    let plan = plan(&text, router.symbols(), &enter, &shift, state.case);
    if plan.typed == 0 {
        return format!(
            "Nothing to paste: this machine has no key for any of the {} characters",
            plan.skipped
        );
    }
    let message = if plan.skipped == 0 {
        format!("Pasting {} characters (paste again to cancel)", plan.typed)
    } else {
        format!(
            "Pasting {} characters, {} skipped (paste again to cancel)",
            plan.typed, plan.skipped
        )
    };
    state.queue = plan.events;
    state.timer.reset();
    message
}

/// Whether a host modifier is down. The paste hotkey is usually a chord
/// (Shift+Insert), and a Shift still held when the first character is typed
/// would reach the machine and turn `1` into `!`.
fn modifier_held(keyboard: &ButtonInput<KeyCode>) -> bool {
    keyboard.any_pressed([
        KeyCode::ShiftLeft,
        KeyCode::ShiftRight,
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::AltLeft,
        KeyCode::AltRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ])
}

/// Sends one planned key event per timer tick into the keyboard channel,
/// waiting while a host modifier is held.
pub(crate) fn paste_typing(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<PasteState>,
    input_tx: Res<InputTxRes>,
    mut toast: ResMut<ToastRes>,
) {
    if state.queue.is_empty() {
        return;
    }
    if modifier_held(&keyboard) {
        state.timer.reset();
        return;
    }
    state.timer.tick(time.delta());
    if !state.timer.just_finished() {
        return;
    }
    if let Some((name, pressed)) = state.queue.pop_front() {
        if pressed {
            state.down.push(name.clone());
        } else if let Some(i) = state.down.iter().rposition(|n| *n == name) {
            state.down.remove(i);
        }
        if input_tx.0.send((name, pressed)).is_err() {
            state.queue.clear();
            state.down.clear();
            return;
        }
    }
    if state.queue.is_empty() {
        toast.push("Paste finished");
    }
}

/// The clipboard's text.
pub(crate) fn clipboard_text() -> Result<String, String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get_text())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_api::MachineKeyInfo;

    fn key(name: &str, chars: &str) -> MachineKeyInfo {
        MachineKeyInfo {
            name: name.to_string(),
            aliases: Vec::new(),
            chars: chars.to_string(),
        }
    }

    fn symbols() -> SymbolMap {
        SymbolMap::from_names(&[
            key("KeyA", ""),
            key("KeyB", ""),
            key("Digit1", ""),
            key("Space", " "),
            key("Plus", "+"),
            key("DoubleQuote", "\""),
            key("Minus", "-"),
        ])
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn run(text: &str, case: PasteCase) -> Vec<(String, bool)> {
        plan(
            text,
            &symbols(),
            &names(&["Enter"]),
            &names(&["ShiftLeft"]),
            case,
        )
        .events
        .into_iter()
        .collect()
    }

    fn ev(name: &str, pressed: bool) -> (String, bool) {
        (name.to_string(), pressed)
    }

    fn each_character_is_a_press_then_a_release_of_its_key() {
        assert_eq!(
            run("a1+", PasteCase::Fold),
            vec![
                ev("KeyA", true),
                ev("KeyA", false),
                ev("Digit1", true),
                ev("Digit1", false),
                ev("Plus", true),
                ev("Plus", false),
            ]
        );
    }

    fn repeated_characters_release_between_presses() {
        let events = run("aa", PasteCase::Fold);
        assert_eq!(
            events,
            vec![
                ev("KeyA", true),
                ev("KeyA", false),
                ev("KeyA", true),
                ev("KeyA", false)
            ]
        );
    }
    /// Merged (every check kept): `each_character_is_a_press_then_a_release_of_its_key`, `repeated_characters_release_between_presses`.
    #[test]
    fn paste_presses_and_releases_each_character() {
        each_character_is_a_press_then_a_release_of_its_key();
        repeated_characters_release_between_presses();
    }

    fn fold_types_a_capital_as_the_plain_key() {
        assert_eq!(run("A", PasteCase::Fold), run("a", PasteCase::Fold));
    }

    fn exact_holds_the_machine_shift_around_a_capital_only() {
        assert_eq!(
            run("Ab", PasteCase::Exact),
            vec![
                ev("ShiftLeft", true),
                ev("KeyA", true),
                ev("KeyA", false),
                ev("ShiftLeft", false),
                ev("KeyB", true),
                ev("KeyB", false),
            ]
        );
    }

    #[test]
    fn line_endings_all_press_enter_once() {
        for text in ["a\nb", "a\r\nb", "a\rb"] {
            let events = run(text, PasteCase::Fold);
            let enters = events.iter().filter(|e| e.0 == "Enter" && e.1).count();
            assert_eq!(enters, 1, "{text:?}");
            assert_eq!(events.len(), 6, "{text:?}");
        }
    }

    #[test]
    fn typographic_characters_fold_to_ascii() {
        let curly = run("\u{201C}a\u{201D} \u{2013} \u{00A0}\t", PasteCase::Fold);
        let plain = run("\"a\" - \u{20}\u{20}", PasteCase::Fold);
        assert_eq!(curly, plain);
    }

    #[test]
    fn accented_letters_are_pasted_as_their_plain_letters() {
        // `à` and `Ä` type the A key, `ß` needs an s key (this machine has none: skipped).
        let p = plan(
            "\u{e0}\u{c4}b \u{df}",
            &symbols(),
            &names(&["Enter"]),
            &names(&["ShiftLeft"]),
            PasteCase::Fold,
        );
        let typed: Vec<&str> = p
            .events
            .iter()
            .filter(|(_, down)| *down)
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(typed, ["KeyA", "KeyA", "KeyB", "Space"]);
        assert_eq!((p.typed, p.skipped), (4, 1), "ß has no s key here");
        // Two-letter folds type both letters and count as one character.
        let p = plan(
            "\u{e6}",
            &SymbolMap::from_names(&[key("KeyA", ""), key("KeyE", "")]),
            &names(&["Enter"]),
            &names(&["ShiftLeft"]),
            PasteCase::Fold,
        );
        assert_eq!(p.typed, 1);
        assert_eq!(p.events.len(), 4);
    }

    #[test]
    fn a_character_without_a_key_is_skipped_and_counted() {
        let p = plan(
            "a\u{4E2D}b~",
            &symbols(),
            &names(&["Enter"]),
            &names(&["ShiftLeft"]),
            PasteCase::Fold,
        );
        assert_eq!((p.typed, p.skipped), (2, 2));
        assert_eq!(p.events.len(), 4);
    }

    fn paste_case_parses_its_two_words() {
        assert_eq!(PasteCase::parse("fold"), Ok(PasteCase::Fold));
        assert_eq!(PasteCase::parse(" Exact "), Ok(PasteCase::Exact));
        assert!(PasteCase::parse("upper").is_err());
    }
    /// Merged (every check kept): `fold_types_a_capital_as_the_plain_key`, `exact_holds_the_machine_shift_around_a_capital_only`, `paste_case_parses_its_two_words`.
    #[test]
    fn paste_case_fold_and_exact() {
        fold_types_a_capital_as_the_plain_key();
        exact_holds_the_machine_shift_around_a_capital_only();
        paste_case_parses_its_two_words();
    }

    mod system {
        use super::*;
        use crate::gui::keys::{KeyBindings, Layout};
        use std::sync::mpsc::{Receiver, channel};

        const DELAY_MS: u64 = 10;

        fn setup() -> (App, Receiver<(String, bool)>, KeyRouter, InputTxRes) {
            let (tx, rx) = channel();
            let router =
                KeyRouter::new(KeyBindings::default()).with_layout(Layout::Positional, symbols());
            let mut app = App::new();
            app.init_resource::<Time>()
                .init_resource::<ButtonInput<KeyCode>>()
                .insert_resource(InputTxRes(tx.clone()))
                .insert_resource(ToastRes::new(Default::default()))
                .insert_resource(PasteState::new(PasteCase::Exact, DELAY_MS))
                .add_systems(Update, paste_typing);
            (app, rx, router, InputTxRes(tx))
        }

        fn tick(app: &mut App) {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(Duration::from_millis(DELAY_MS));
            app.update();
        }

        fn start(app: &mut App, router: &KeyRouter, tx: &InputTxRes, text: &str) -> String {
            let mut state = app.world_mut().resource_mut::<PasteState>();
            toggle(&mut state, router, tx, || Ok(text.to_string()))
        }

        #[test]
        fn the_system_sends_one_planned_event_per_delay_in_order() {
            let (mut app, rx, router, tx) = setup();
            let message = start(&mut app, &router, &tx, "Ab");
            assert!(message.starts_with("Pasting 2 characters"), "{message}");
            // Nothing is sent before the first delay has passed.
            app.update();
            assert!(rx.try_recv().is_err());
            for _ in 0..12 {
                tick(&mut app);
            }
            let got: Vec<_> = rx.try_iter().collect();
            let expected = vec![
                ev("ShiftLeft", true),
                ev("KeyA", true),
                ev("KeyA", false),
                ev("ShiftLeft", false),
                ev("KeyB", true),
                ev("KeyB", false),
            ];
            assert_eq!(got, expected);
            assert!(!app.world().resource::<PasteState>().is_active());
        }

        #[test]
        fn typing_waits_for_the_host_modifiers_to_come_up() {
            let (mut app, rx, router, tx) = setup();
            start(&mut app, &router, &tx, "1");
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::ShiftLeft);
            for _ in 0..6 {
                tick(&mut app);
            }
            assert!(rx.try_recv().is_err(), "nothing typed while Shift is down");
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::ShiftLeft);
            for _ in 0..4 {
                tick(&mut app);
            }
            assert_eq!(
                rx.try_iter().collect::<Vec<_>>(),
                vec![ev("Digit1", true), ev("Digit1", false)]
            );
        }

        #[test]
        fn pasting_again_cancels_and_releases_what_is_held() {
            let (mut app, rx, router, tx) = setup();
            start(&mut app, &router, &tx, "Ab");
            tick(&mut app); // ShiftLeft down
            tick(&mut app); // KeyA down
            assert_eq!(
                rx.try_iter().collect::<Vec<_>>(),
                vec![ev("ShiftLeft", true), ev("KeyA", true)]
            );
            let message = start(&mut app, &router, &tx, "ignored");
            assert_eq!(message, "Paste cancelled");
            // Held keys come back up, last pressed first, and nothing more follows.
            assert_eq!(
                rx.try_iter().collect::<Vec<_>>(),
                vec![ev("KeyA", false), ev("ShiftLeft", false)]
            );
            for _ in 0..8 {
                tick(&mut app);
            }
            assert!(rx.try_recv().is_err());
        }

        #[test]
        fn a_clipboard_error_or_empty_text_starts_nothing() {
            let (mut app, _rx, router, tx) = setup();
            let mut state = app.world_mut().resource_mut::<PasteState>();
            let failed = toggle(&mut state, &router, &tx, || Err("no display".into()));
            assert_eq!(failed, "Paste failed: no display");
            let empty = toggle(&mut state, &router, &tx, || Ok(String::new()));
            assert_eq!(empty, "Clipboard has no text");
            let unmapped = toggle(&mut state, &router, &tx, || Ok("\u{4E2D}\u{6587}".into()));
            assert!(unmapped.starts_with("Nothing to paste"), "{unmapped}");
            assert!(!state.is_active());
        }
    }
}
