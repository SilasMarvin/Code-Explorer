//! Non-interactive mode: render a few frames, press any keys asked for, save a
//! screenshot and what the view means to show beside it (`shot.png`,
//! `shot.truth.json`), exit.

use std::{path::PathBuf, time::Duration};

use bevy::{
    input::{
        ButtonState, InputSystems,
        keyboard::{Key, KeyboardInput},
    },
    prelude::*,
    render::view::screenshot::{Capturing, Screenshot, save_to_disk},
    window::PrimaryWindow,
};

use crate::{diff_view::DiffView, explorer::Explorer};

/// Frames to let the scene settle (assets load, camera eases) before capturing.
const SETTLE_FRAMES: u32 = 90;
/// Time to let it settle as well: on a first run shaders compile in the background,
/// and until they do the frames come quickly and draw nothing.
const SETTLE_TIME: Duration = Duration::from_secs(2);
/// Frames between one key and the next.
const KEY_GAP: u32 = 15;
/// Frames and time to let the view settle after the last key, as the camera glides.
const AFTER_KEYS_FRAMES: u32 = 60;
const AFTER_KEYS_TIME: Duration = Duration::from_millis(1500);
/// Frames to wait after capture so the file write completes.
const DRAIN_FRAMES: u32 = 30;

/// A key to press, with shift or alt held or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Press {
    key: KeyCode,
    shift: bool,
    alt: bool,
}

/// Keys named in words.
const NAMED: [(&str, KeyCode); 17] = [
    ("up", KeyCode::ArrowUp),
    ("down", KeyCode::ArrowDown),
    ("left", KeyCode::ArrowLeft),
    ("right", KeyCode::ArrowRight),
    ("enter", KeyCode::Enter),
    ("esc", KeyCode::Escape),
    ("escape", KeyCode::Escape),
    ("space", KeyCode::Space),
    ("tab", KeyCode::Tab),
    ("home", KeyCode::Home),
    ("end", KeyCode::End),
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
    /// Reads a key as `o`, `.`, `up`, `enter`, `shift+]` or `alt+left`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let mut name = text.to_lowercase();
        let (mut shift, mut alt) = (false, false);
        loop {
            if let Some(rest) = name.strip_prefix("shift+") {
                shift = true;
                name = rest.to_owned();
            } else if let Some(rest) = name.strip_prefix("alt+") {
                alt = true;
                name = rest.to_owned();
            } else {
                break;
            }
        }
        let named = NAMED.iter().find(|(n, _)| *n == name).map(|&(_, key)| key);
        let key = named.or_else(|| {
            let mut chars = name.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            TYPED.iter().find(|(t, _)| *t == c).map(|&(_, key)| key)
        })?;
        Some(Self { key, shift, alt })
    }

    /// The modifiers, each with whether it is held.
    fn modifiers(self) -> [(bool, KeyCode, Key); 2] {
        [
            (self.shift, KeyCode::ShiftLeft, Key::Shift),
            (self.alt, KeyCode::AltLeft, Key::Alt),
        ]
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
            KeyCode::Home => Key::Home,
            KeyCode::End => Key::End,
            _ => Key::Backspace,
        };
        (key, None)
    }
}

pub(crate) struct CapturePlugin {
    pub(crate) path: PathBuf,
    pub(crate) keys: Vec<Press>,
}

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Capture {
            path: self.path.clone(),
            keys: self.keys.clone(),
            frame: 0,
            started: None,
            pressed: 0,
            last_press: None,
            held: None,
            requested_at: None,
        })
        // Keys go in with the window's own input, so the frame's systems see them.
        .add_systems(PreUpdate, press_keys.before(InputSystems))
        .add_systems(Update, capture);
    }
}

#[derive(Resource)]
struct Capture {
    path: PathBuf,
    keys: Vec<Press>,
    frame: u32,
    started: Option<Duration>,
    /// How many of `keys` have been pressed.
    pressed: usize,
    /// The frame and time the last key went down.
    last_press: Option<(u32, Duration)>,
    /// The key down now, to let go of next frame.
    held: Option<Press>,
    requested_at: Option<u32>,
}

impl Capture {
    /// Whether the scene has had time to settle since it started.
    fn settled(&self, now: Duration) -> bool {
        self.started.is_some_and(|started| {
            self.frame >= SETTLE_FRAMES && now.saturating_sub(started) >= SETTLE_TIME
        })
    }

    /// Whether every key has been pressed and the view has settled since the last.
    fn ready(&self, now: Duration) -> bool {
        self.settled(now)
            && self.pressed == self.keys.len()
            && self.held.is_none()
            && self.last_press.is_none_or(|(frame, at)| {
                self.frame >= frame + AFTER_KEYS_FRAMES && now.saturating_sub(at) >= AFTER_KEYS_TIME
            })
    }
}

/// Once the scene settles, presses each key in turn, a few frames apart. Keys go in
/// as the window would send them, so egui sees them as well as the scene does.
fn press_keys(
    mut state: ResMut<Capture>,
    mut input: MessageWriter<KeyboardInput>,
    window: Single<Entity, With<PrimaryWindow>>,
    time: Res<Time<Real>>,
) {
    let mut send = |key: KeyCode, logical: Key, text: Option<String>, button: ButtonState| {
        input.write(KeyboardInput {
            key_code: key,
            logical_key: logical,
            state: button,
            text: text.map(|t| t.as_str().into()),
            repeat: false,
            window: *window,
        });
    };
    if let Some(held) = state.held.take() {
        let (logical, _) = held.logical();
        send(held.key, logical, None, ButtonState::Released);
        for (on, key, logical) in held.modifiers() {
            if on {
                send(key, logical, None, ButtonState::Released);
            }
        }
        return;
    }
    let now = time.elapsed();
    let due = state
        .last_press
        .is_none_or(|(frame, _)| state.frame >= frame + KEY_GAP);
    if !state.settled(now) || !due {
        return;
    }
    let Some(&press) = state.keys.get(state.pressed) else {
        return;
    };
    for (on, key, logical) in press.modifiers() {
        if on {
            send(key, logical, None, ButtonState::Pressed);
        }
    }
    let (logical, text) = press.logical();
    send(press.key, logical, text, ButtonState::Pressed);
    state.held = Some(press);
    state.pressed += 1;
    state.last_press = Some((state.frame, now));
}

fn capture(
    mut commands: Commands,
    mut state: ResMut<Capture>,
    capturing: Query<(), With<Capturing>>,
    mut exit: MessageWriter<AppExit>,
    explorer: Res<Explorer>,
    diff: Option<Res<DiffView>>,
    time: Res<Time<Real>>,
) {
    state.frame += 1;
    let now = time.elapsed();
    state.started.get_or_insert(now);
    match state.requested_at {
        None if state.ready(now) => {
            info!(
                "frame time at capture: {:.1} ms",
                time.delta_secs() * 1000.0
            );
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(state.path.clone()));
            let truth = crate::truth::of(&explorer, diff.as_deref());
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
        let press = |text| Press::parse(text).map(|p| (p.key, p.shift, p.alt));
        assert_eq!(press("o"), Some((KeyCode::KeyO, false, false)));
        assert_eq!(press("I"), Some((KeyCode::KeyI, false, false)));
        assert_eq!(press("."), Some((KeyCode::Period, false, false)));
        assert_eq!(press("period"), Some((KeyCode::Period, false, false)));
        assert_eq!(press("3"), Some((KeyCode::Digit3, false, false)));
        assert_eq!(press("shift+]"), Some((KeyCode::BracketRight, true, false)));
        assert_eq!(press("alt+left"), Some((KeyCode::ArrowLeft, false, true)));
        assert_eq!(press("up"), Some((KeyCode::ArrowUp, false, false)));
        assert_eq!(press("oops"), None);
        assert_eq!(
            Press::parse("i").map(Press::logical),
            Some((Key::Character("i".into()), Some("i".to_owned()))),
            "a letter types itself"
        );
    }
}
