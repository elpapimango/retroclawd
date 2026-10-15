//! The in-app question a file action can ask ("this disk has unsaved
//! changes: [S] save as  [O] overwrite  [D] discard  [Esc] cancel").
//!
//! The XDG desktop portal has no message boxes, so confirmations are drawn
//! by the GUI itself: the dialog thread sends a `PromptRequest`, this system
//! shows its text in the toast line until a key answers it and sends the
//! chosen option's index back. While a file action runs (`FileDialogs::
//! is_busy`) `hotkey_input` keeps every key away from the machine, so the
//! answer key never also types into the emulated computer.

use std::sync::Mutex;
use std::sync::mpsc::Receiver;

use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;
use bevy::prelude::*;

use super::toast::ToastRes;
use crate::dialogs::PromptRequest;

#[derive(Resource)]
pub(crate) struct PromptRes {
    requests: Mutex<Receiver<PromptRequest>>,
    active: Option<PromptRequest>,
}

impl PromptRes {
    pub(crate) fn new(requests: Receiver<PromptRequest>) -> Self {
        PromptRes {
            requests: Mutex::new(requests),
            active: None,
        }
    }
}

const LETTERS: [KeyCode; 26] = [
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
];

/// The key that answers an option labelled with this upper-case letter.
pub(crate) fn key_for_letter(letter: char) -> Option<KeyCode> {
    let letter = letter.to_ascii_uppercase();
    letter
        .is_ascii_uppercase()
        .then(|| LETTERS[(letter as u8 - b'A') as usize])
}

/// Which option (if any) this frame's key presses choose; `Some(None)` is
/// Escape (cancel), `None` means no answer yet.
pub(crate) fn answer(
    request: &PromptRequest,
    keyboard: &ButtonInput<KeyCode>,
) -> Option<Option<usize>> {
    if keyboard.just_pressed(KeyCode::Escape) {
        return Some(None);
    }
    request
        .options
        .iter()
        .position(|o| key_for_letter(o.key).is_some_and(|k| keyboard.just_pressed(k)))
        .map(Some)
}

pub(crate) fn prompt_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut prompt: ResMut<PromptRes>,
    mut toast: ResMut<ToastRes>,
) {
    if prompt.active.is_none() {
        let next = prompt.requests.lock().unwrap().try_recv().ok();
        if let Some(request) = next {
            tracing::info!(target: "retroclawd::dialogs", "prompt shown: {}", request.text);
            toast.set_sticky(Some(request.text.clone()));
            prompt.active = Some(request);
        }
        return;
    }
    let Some(request) = &prompt.active else {
        return;
    };
    if let Some(choice) = answer(request, &keyboard) {
        tracing::info!(target: "retroclawd::dialogs", "prompt answered: {choice:?}");
        let _ = request.reply.send(choice);
        prompt.active = None;
        toast.set_sticky(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogs::PromptOption;

    fn request() -> PromptRequest {
        let (reply, _rx) = std::sync::mpsc::channel();
        PromptRequest {
            text: "q".to_string(),
            options: vec![
                PromptOption {
                    key: 'S',
                    label: "save".into(),
                },
                PromptOption {
                    key: 'D',
                    label: "discard".into(),
                },
            ],
            reply,
        }
    }

    #[test]
    fn letters_map_to_their_keys() {
        assert_eq!(key_for_letter('S'), Some(KeyCode::KeyS));
        assert_eq!(key_for_letter('a'), Some(KeyCode::KeyA));
        assert_eq!(key_for_letter('Z'), Some(KeyCode::KeyZ));
        assert_eq!(key_for_letter('1'), None);
    }

    #[test]
    fn an_option_key_or_escape_answers_and_other_keys_do_not() {
        let r = request();
        let mut kb = ButtonInput::<KeyCode>::default();
        assert_eq!(answer(&r, &kb), None);
        kb.press(KeyCode::KeyX);
        assert_eq!(
            answer(&r, &kb),
            None,
            "an unrelated key leaves the question open"
        );
        kb.clear();
        kb.press(KeyCode::KeyD);
        assert_eq!(answer(&r, &kb), Some(Some(1)));
        kb.clear();
        // Held but not newly pressed: not an answer (the key that opened it).
        assert_eq!(answer(&r, &kb), None);
        kb.press(KeyCode::Escape);
        assert_eq!(answer(&r, &kb), Some(None));
    }
}
