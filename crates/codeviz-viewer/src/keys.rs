//! A key as a script names it (`x`, `.`, `up`, `shift+]`), and as the window sends it.

use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    },
    prelude::*,
};

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

    /// The key, and whether Shift is held.
    pub(crate) fn code(self) -> (KeyCode, bool) {
        (self.key, self.shift)
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
        let shift = self
            .shift
            .then(|| message(KeyCode::ShiftLeft, Key::Shift, None));
        match state {
            ButtonState::Pressed => shift.into_iter().chain([key]).collect(),
            ButtonState::Released => [key].into_iter().chain(shift).collect(),
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
