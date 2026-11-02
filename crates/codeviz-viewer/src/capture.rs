//! Non-interactive mode: render a few frames, save a screenshot and what the view
//! means to show beside it (`shot.png`, `shot.truth.json`), exit. [`Press`], a key as
//! the window sends it, is shared with scripted sessions.

use std::{path::PathBuf, time::Duration};

use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    },
    prelude::*,
    render::view::screenshot::{Capturing, Screenshot, save_to_disk},
};

use crate::explorer::Explorer;

/// Frames to let the scene settle (assets load, camera eases) before capturing.
pub(crate) const SETTLE_FRAMES: u32 = 90;
/// Time to let it settle as well: on a first run shaders compile in the background,
/// and until they do the frames come quickly and draw nothing.
pub(crate) const SETTLE_TIME: Duration = Duration::from_secs(2);
/// Frames to wait after capture so the file write completes.
pub(crate) const DRAIN_FRAMES: u32 = 30;

/// A key to press, with shift held or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Press {
    key: KeyCode,
    shift: bool,
}

/// Keys named in words.
const NAMED: [(&str, KeyCode); 15] = [
    ("up", KeyCode::ArrowUp),
    ("down", KeyCode::ArrowDown),
    ("left", KeyCode::ArrowLeft),
    ("right", KeyCode::ArrowRight),
    ("enter", KeyCode::Enter),
    ("esc", KeyCode::Escape),
    ("escape", KeyCode::Escape),
    ("space", KeyCode::Space),
    ("tab", KeyCode::Tab),
    ("backspace", KeyCode::Backspace),
    ("comma", KeyCode::Comma),
    ("period", KeyCode::Period),
    ("slash", KeyCode::Slash),
    ("minus", KeyCode::Minus),
    ("equal", KeyCode::Equal),
];

/// Keys that type a character, with the character.
const TYPED: [(char, KeyCode); 43] = [
    (',', KeyCode::Comma),
    ('.', KeyCode::Period),
    ('/', KeyCode::Slash),
    ('[', KeyCode::BracketLeft),
    (']', KeyCode::BracketRight),
    ('-', KeyCode::Minus),
    ('=', KeyCode::Equal),
    ('a', KeyCode::KeyA),
    ('b', KeyCode::KeyB),
    ('c', KeyCode::KeyC),
    ('d', KeyCode::KeyD),
    ('e', KeyCode::KeyE),
    ('f', KeyCode::KeyF),
    ('g', KeyCode::KeyG),
    ('h', KeyCode::KeyH),
    ('i', KeyCode::KeyI),
    ('j', KeyCode::KeyJ),
    ('k', KeyCode::KeyK),
    ('l', KeyCode::KeyL),
    ('m', KeyCode::KeyM),
    ('n', KeyCode::KeyN),
    ('o', KeyCode::KeyO),
    ('p', KeyCode::KeyP),
    ('q', KeyCode::KeyQ),
    ('r', KeyCode::KeyR),
    ('s', KeyCode::KeyS),
    ('t', KeyCode::KeyT),
    ('u', KeyCode::KeyU),
    ('v', KeyCode::KeyV),
    ('w', KeyCode::KeyW),
    ('x', KeyCode::KeyX),
    ('y', KeyCode::KeyY),
    ('z', KeyCode::KeyZ),
    ('0', KeyCode::Digit0),
    ('1', KeyCode::Digit1),
    ('2', KeyCode::Digit2),
    ('3', KeyCode::Digit3),
    ('4', KeyCode::Digit4),
    ('5', KeyCode::Digit5),
    ('6', KeyCode::Digit6),
    ('7', KeyCode::Digit7),
    ('8', KeyCode::Digit8),
    ('9', KeyCode::Digit9),
];

impl Press {
    /// Reads a key as `o`, `.`, `up`, `enter` or `shift+]`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.to_lowercase();
        let (shift, name) = match text.strip_prefix("shift+") {
            Some(rest) => (true, rest),
            None => (false, text.as_str()),
        };
        let named = NAMED.iter().find(|(n, _)| *n == name).map(|&(_, key)| key);
        let key = named.or_else(|| {
            let mut chars = name.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            TYPED.iter().find(|(t, _)| *t == c).map(|&(_, key)| key)
        })?;
        Some(Self { key, shift })
    }

    /// The modifiers, each with whether it is held.
    fn modifiers(self) -> [(bool, KeyCode, Key); 1] {
        [(self.shift, KeyCode::ShiftLeft, Key::Shift)]
    }

    /// What the window sends for the key going down or up: Shift first going down,
    /// last coming up.
    pub(crate) fn messages(self, state: ButtonState, window: Entity) -> Vec<KeyboardInput> {
        let message = |key_code, logical_key, text: Option<String>| KeyboardInput {
            key_code,
            logical_key,
            state,
            text: text.map(|t| t.as_str().into()),
            repeat: false,
            window,
        };
        let (logical, text) = self.logical();
        let key = message(
            self.key,
            logical,
            text.filter(|_| state == ButtonState::Pressed),
        );
        let modifiers = self
            .modifiers()
            .into_iter()
            .filter(|&(on, ..)| on)
            .map(|(_, code, logical)| message(code, logical, None));
        match state {
            ButtonState::Pressed => modifiers.chain([key]).collect(),
            ButtonState::Released => [key].into_iter().chain(modifiers).collect(),
        }
    }

    /// The key as the keyboard layout reads it, and the text it types.
    fn logical(self) -> (Key, Option<String>) {
        if let Some(&(c, _)) = TYPED.iter().find(|(_, key)| *key == self.key) {
            let text = c.to_string();
            return (Key::Character(text.as_str().into()), Some(text));
        }
        let key = match self.key {
            KeyCode::ArrowUp => Key::ArrowUp,
            KeyCode::ArrowDown => Key::ArrowDown,
            KeyCode::ArrowLeft => Key::ArrowLeft,
            KeyCode::ArrowRight => Key::ArrowRight,
            KeyCode::Enter => Key::Enter,
            KeyCode::Escape => Key::Escape,
            KeyCode::Space => return (Key::Space, Some(" ".to_owned())),
            KeyCode::Tab => Key::Tab,
            _ => Key::Backspace,
        };
        (key, None)
    }
}

pub(crate) struct CapturePlugin {
    pub(crate) path: PathBuf,
}

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Capture {
            path: self.path.clone(),
            frame: 0,
            started: None,
            requested_at: None,
        })
        .add_systems(Update, capture);
    }
}

#[derive(Resource)]
struct Capture {
    path: PathBuf,
    frame: u32,
    started: Option<Duration>,
    requested_at: Option<u32>,
}

impl Capture {
    /// Whether the scene has had time to settle since it started.
    fn ready(&self, now: Duration) -> bool {
        self.started.is_some_and(|started| {
            self.frame >= SETTLE_FRAMES && now.saturating_sub(started) >= SETTLE_TIME
        })
    }
}

fn capture(
    mut commands: Commands,
    mut state: ResMut<Capture>,
    capturing: Query<(), With<Capturing>>,
    mut exit: MessageWriter<AppExit>,
    explorer: Res<Explorer>,
    time: Res<Time<Real>>,
) {
    state.frame += 1;
    let now = time.elapsed();
    state.started.get_or_insert(now);
    match state.requested_at {
        None if state.ready(now) => {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(state.path.clone()));
            let truth = crate::truth::of(&explorer);
            let path = state.path.with_extension("truth.json");
            if let Err(error) = std::fs::write(&path, format!("{truth:#}\n")) {
                error!("could not write {}: {error}", path.display());
            }
            state.requested_at = Some(state.frame);
        }
        Some(at) if capturing.is_empty() && state.frame >= at + DRAIN_FRAMES => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_keys_as_written() {
        let press = |text| Press::parse(text).map(|p| (p.key, p.shift));
        assert_eq!(press("o"), Some((KeyCode::KeyO, false)));
        assert_eq!(press("I"), Some((KeyCode::KeyI, false)));
        assert_eq!(press("."), Some((KeyCode::Period, false)));
        assert_eq!(press("period"), Some((KeyCode::Period, false)));
        assert_eq!(press("3"), Some((KeyCode::Digit3, false)));
        assert_eq!(press("shift+]"), Some((KeyCode::BracketRight, true)));
        assert_eq!(press("shift+backspace"), Some((KeyCode::Backspace, true)));
        assert_eq!(press("up"), Some((KeyCode::ArrowUp, false)));
        assert_eq!(press("alt+left"), None, "no key takes alt");
        assert_eq!(press("oops"), None);
        assert_eq!(
            Press::parse("i").map(Press::logical),
            Some((Key::Character("i".into()), Some("i".to_owned()))),
            "a letter types itself"
        );
    }
}
