//! Non-interactive mode: render a few frames, press any keys asked for, save a
//! screenshot and what the view means to show beside it (`shot.png`,
//! `shot.truth.json`), exit. A live view waits for a new version too, and captures a
//! moment after the last, so a script can change the code while it renders and catch
//! the update as it lands.

use std::{path::PathBuf, time::Duration};

use bevy::{
    input::InputSystems,
    prelude::*,
    render::view::screenshot::{Capturing, Screenshot, save_to_disk},
};

use crate::{
    diff_view::DiffView,
    explorer::Explorer,
    live::{Live, Status},
};

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
/// How long a live view waits for a new version before capturing anyway.
const RELOAD_WAIT: Duration = Duration::from_secs(40);
/// Time after a new version lands before capturing: long enough for the graph to
/// start settling round it, soon enough to catch the pulse and the note. The
/// `CODEVIZ_SHOT_AFTER_UPDATE` environment variable sets another, in seconds, to see
/// how the graph has settled since.
const AFTER_RELOAD: Duration = Duration::from_millis(700);

fn after_reload() -> Duration {
    std::env::var("CODEVIZ_SHOT_AFTER_UPDATE")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .and_then(|seconds| Duration::try_from_secs_f32(seconds).ok())
        .unwrap_or(AFTER_RELOAD)
}

/// A key to press, with shift held or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Press {
    key: KeyCode,
    shift: bool,
}

impl Press {
    /// Reads a key as `o`, `.`, `up`, `enter` or `shift+]`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let lower = text.to_lowercase();
        let (shift, name) = lower
            .strip_prefix("shift+")
            .map_or((false, lower.as_str()), |rest| (true, rest));
        let key = match name {
            "," | "comma" => KeyCode::Comma,
            "." | "period" => KeyCode::Period,
            "/" | "slash" => KeyCode::Slash,
            "[" => KeyCode::BracketLeft,
            "]" => KeyCode::BracketRight,
            "-" | "minus" => KeyCode::Minus,
            "=" | "equal" => KeyCode::Equal,
            "up" => KeyCode::ArrowUp,
            "down" => KeyCode::ArrowDown,
            "left" => KeyCode::ArrowLeft,
            "right" => KeyCode::ArrowRight,
            "enter" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Escape,
            "space" => KeyCode::Space,
            "tab" => KeyCode::Tab,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "backspace" => KeyCode::Backspace,
            single => {
                let mut chars = single.chars();
                let (Some(c), None) = (chars.next(), chars.next()) else {
                    return None;
                };
                letter_or_digit(c)?
            }
        };
        Some(Self { key, shift })
    }
}

fn letter_or_digit(c: char) -> Option<KeyCode> {
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
    const DIGITS: [KeyCode; 10] = [
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
    ];
    if c.is_ascii_lowercase() {
        LETTERS.get(usize::from(c as u8 - b'a')).copied()
    } else if c.is_ascii_digit() {
        DIGITS.get(usize::from(c as u8 - b'0')).copied()
    } else {
        None
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
            reloaded: None,
            heard: 0,
        })
        // Keys go in after the window's own input, so the frame's systems see them.
        .add_systems(PreUpdate, press_keys.after(InputSystems))
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
    /// When a live view last heard of a new version, read or not.
    reloaded: Option<Duration>,
    /// How many it had heard of then.
    heard: u64,
}

impl Capture {
    /// Whether the scene has had time to settle since it started.
    fn settled(&self, now: Duration) -> bool {
        self.started.is_some_and(|started| {
            self.frame >= SETTLE_FRAMES && now.saturating_sub(started) >= SETTLE_TIME
        })
    }

    /// Whether a live view has taken in a new version and shown it a moment, or
    /// waited long enough for one.
    fn updated(&self, now: Duration) -> bool {
        match (self.reloaded, self.started) {
            (Some(at), _) => now.saturating_sub(at) >= after_reload(),
            (None, Some(started)) => now.saturating_sub(started) >= RELOAD_WAIT,
            (None, None) => false,
        }
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

/// Once the scene settles, presses each key in turn, a few frames apart.
fn press_keys(
    mut state: ResMut<Capture>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
) {
    if let Some(held) = state.held.take() {
        keys.release(held.key);
        if held.shift {
            keys.release(KeyCode::ShiftLeft);
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
    if press.shift {
        keys.press(KeyCode::ShiftLeft);
    }
    keys.press(press.key);
    state.held = Some(press);
    state.pressed += 1;
    state.last_press = Some((state.frame, now));
}

#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn capture(
    mut commands: Commands,
    mut state: ResMut<Capture>,
    capturing: Query<(), With<Capturing>>,
    mut exit: MessageWriter<AppExit>,
    explorer: Res<Explorer>,
    diff: Option<Res<DiffView>>,
    live: Option<Res<Live>>,
    time: Res<Time<Real>>,
) {
    state.frame += 1;
    let now = time.elapsed();
    state.started.get_or_insert(now);
    if let Some(heard) = live.as_ref().map(|live| live.heard())
        && heard != state.heard
    {
        state.heard = heard;
        state.reloaded = Some(now);
    }
    match state.requested_at {
        None if state.ready(now) && (live.is_none() || state.updated(now)) => {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(state.path.clone()));
            let mut truth = crate::truth::of(&explorer, diff.as_deref());
            if let Some(live) = &live {
                truth["live"] = serde_json::json!({
                    "reloads": explorer.reloads(),
                    "status": match &live.status {
                        Status::Watching => "watching".to_owned(),
                        Status::Reading => "reading".to_owned(),
                        Status::Failed(why) => format!("failed: {why}"),
                    },
                    "updated": live.last.as_ref().map(|(note, _)| note.text()),
                    "since_opened": live.since_opened,
                });
            }
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
        assert_eq!(press("3"), Some((KeyCode::Digit3, false)));
        assert_eq!(press("shift+]"), Some((KeyCode::BracketRight, true)));
        assert_eq!(press("up"), Some((KeyCode::ArrowUp, false)));
        assert_eq!(press("oops"), None);
    }
}
