//! A one-line message at the bottom of the window ("Paused", "State saved:
//! slot 2", "Volume 70%", ...). Messages come from two places: the emu
//! thread (`emu_thread::Messages`, for actions that run there) and the GUI's
//! own hotkey system (`ToastRes::push`). `ToastQueue` is the pure part --
//! which message is showing and for how long -- so it is unit-tested without
//! Bevy.

use std::collections::VecDeque;
use std::time::Duration;

use bevy::prelude::*;

use crate::emu_thread::Messages;

/// How long a message stays up when nothing is waiting behind it.
const SHOW_FOR: Duration = Duration::from_millis(2200);
/// How long it stays when more messages are queued (so a burst of volume
/// presses keeps up instead of lagging seconds behind).
const SHOW_FOR_WHEN_BUSY: Duration = Duration::from_millis(600);
const MAX_PENDING: usize = 6;

#[derive(Debug, Default)]
pub(crate) struct ToastQueue {
    current: Option<(String, Duration)>,
    pending: VecDeque<String>,
}

impl ToastQueue {
    pub(crate) fn push(&mut self, message: impl Into<String>) {
        let message = message.into();
        // A repeat of what is on screen or already waiting adds nothing.
        if self.current.as_ref().is_some_and(|(m, _)| *m == message)
            || self.pending.back() == Some(&message)
        {
            if let Some((_, left)) = &mut self.current
                && self.pending.is_empty()
            {
                *left = SHOW_FOR; // keep the repeated message up
            }
            return;
        }
        self.pending.push_back(message);
        while self.pending.len() > MAX_PENDING {
            self.pending.pop_front();
        }
    }

    /// Advances time by `dt`; returns the text to show now (`None` = nothing).
    pub(crate) fn tick(&mut self, dt: Duration) -> Option<&str> {
        if let Some((_, left)) = &mut self.current {
            *left = left.saturating_sub(dt);
            if left.is_zero() {
                self.current = None;
            }
        }
        if self.current.is_none()
            && let Some(next) = self.pending.pop_front()
        {
            let time = if self.pending.is_empty() {
                SHOW_FOR
            } else {
                SHOW_FOR_WHEN_BUSY
            };
            self.current = Some((next, time));
        }
        // A waiting message cuts the current one short once it has been
        // readable for a moment.
        if let Some((_, left)) = &mut self.current
            && !self.pending.is_empty()
            && *left > SHOW_FOR_WHEN_BUSY
        {
            *left = SHOW_FOR_WHEN_BUSY;
        }
        self.current.as_ref().map(|(m, _)| m.as_str())
    }
}

#[derive(Resource)]
pub(crate) struct ToastRes {
    queue: ToastQueue,
    emu_messages: Messages,
    /// A message that stays until cleared (a question waiting for its key).
    sticky: Option<String>,
}

impl ToastRes {
    pub(crate) fn new(emu_messages: Messages) -> Self {
        ToastRes {
            queue: ToastQueue::default(),
            emu_messages,
            sticky: None,
        }
    }

    /// Shows `message` until cleared with `None`, over the regular toasts.
    pub(crate) fn set_sticky(&mut self, message: Option<String>) {
        self.sticky = message;
    }

    pub(crate) fn push(&mut self, message: impl Into<String>) {
        self.queue.push(message);
    }
}

#[derive(Component)]
pub(crate) struct ToastText;

pub(crate) fn setup_toast(mut commands: Commands) {
    commands.spawn((
        ToastText,
        Visibility::Hidden,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.95)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(10.0),
            left: Val::Px(10.0),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
    ));
}

pub(crate) fn update_toast(
    time: Res<Time>,
    mut toast: ResMut<ToastRes>,
    mut text: Query<(&mut Text, &mut Visibility), With<ToastText>>,
) {
    let incoming: Vec<String> = toast.emu_messages.lock().unwrap().drain(..).collect();
    for message in incoming {
        toast.queue.push(message);
    }
    let Ok((mut text, mut visibility)) = text.single_mut() else {
        return;
    };
    let sticky = toast.sticky.clone();
    let shown = toast.queue.tick(time.delta()).map(str::to_string);
    match sticky.or(shown) {
        Some(message) => {
            if text.0 != message {
                text.0 = message;
            }
            *visibility = Visibility::Inherited;
        }
        None => *visibility = Visibility::Hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn a_message_shows_then_expires() {
        let mut q = ToastQueue::default();
        assert_eq!(q.tick(MS(16)), None);
        q.push("Paused");
        assert_eq!(q.tick(MS(16)), Some("Paused"));
        assert_eq!(q.tick(MS(2000)), Some("Paused"));
        assert_eq!(q.tick(MS(400)), None);
    }

    #[test]
    fn queued_messages_follow_in_order_and_move_along_quickly() {
        let mut q = ToastQueue::default();
        q.push("Volume 90%");
        q.push("Volume 80%");
        q.push("Volume 70%");
        assert_eq!(q.tick(MS(16)), Some("Volume 90%"));
        // Others are waiting: the first is cut to the short time.
        assert_eq!(q.tick(MS(700)), Some("Volume 80%"));
        assert_eq!(q.tick(MS(700)), Some("Volume 70%"));
        // The last one gets the full time.
        assert_eq!(q.tick(MS(1500)), Some("Volume 70%"));
        assert_eq!(q.tick(MS(1000)), None);
    }

    #[test]
    fn repeating_the_message_on_screen_extends_it_without_queueing() {
        let mut q = ToastQueue::default();
        q.push("Muted");
        q.tick(MS(16));
        q.tick(MS(2000));
        q.push("Muted");
        assert_eq!(q.tick(MS(1000)), Some("Muted"));
        assert_eq!(q.pending.len(), 0);
    }

    #[test]
    fn a_flood_keeps_only_the_latest_few() {
        let mut q = ToastQueue::default();
        for n in 0..50 {
            q.push(format!("m{n}"));
        }
        assert_eq!(q.pending.len(), MAX_PENDING);
        assert_eq!(q.pending.back().unwrap(), "m49");
    }
}
