//! The steps of a scripted session (`--script`), one per line, as a person would take
//! them: keys and typing, clicks and hovers on functions by name, on their drawn names
//! or on a point, double clicks on a module's sphere, clicks on journey bars, their
//! fold badges, search rows and buttons by what they say, drags and pans, wheel turns,
//! a window size, waits, snapshots of what shows (or of each frame of a transition),
//! and `expect` lines that fail the session when the view is not as they say. Names of functions and modules are looked up before the window opens
//! (`drive::resolve`), so a misspelt one stops the script at once.

use std::time::Duration;

use bevy::{input::keyboard::KeyCode, math::Vec2};

use crate::keys::Press;

/// What the pointer does to a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gesture {
    Hover,
    Click,
    /// A click with Shift held.
    ShiftClick,
    DoubleClick,
    /// A click on the function's name, where it is drawn, rather than on its node.
    LabelClick,
    /// The pointer resting on the function's name.
    LabelHover,
}

/// Where the pointer goes: onto a function's node, or a point in window points.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Aim {
    Unit(String),
    At(Vec2),
}

/// One step of a session.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Step {
    Key(Press),
    /// Types this text, a character at a time, as into the search box.
    Type(String),
    /// A gesture on a function, found as `--focus` finds it.
    Point {
        gesture: Gesture,
        unit: String,
    },
    /// A click at a point in window points.
    ClickAt(Vec2),
    /// A click on the graph clear of every node, name and overlay: deselects.
    ClickEmpty,
    /// A double click on a module's sphere, off its functions.
    DoubleClickModule(String),
    /// A click on the journey bar with this number, as `1.2`, or for this function.
    Bar(String),
    /// A click on the badge on that bar that unfolds what is folded under it.
    Chip(String),
    /// A click on the search box's row for this function, by what the row says.
    SearchPick(String),
    /// A click on the button or chip that says this.
    Button(String),
    /// Orbits by a drag of this many points.
    Drag(Vec2),
    /// Pans by a right-button drag of this many points.
    Pan(Vec2),
    /// Turns the wheel this many notches across and down (positive `y` is nearer),
    /// with the pointer here first, else where it is.
    Scroll {
        by: Vec2,
        at: Option<Aim>,
    },
    /// Resizes the window to this many points.
    Window(Vec2),
    Wait(Duration),
    /// Fails the session, saying why, unless the view is as this says.
    Expect(Expect),
    /// Saves what shows, under this label.
    Snap(Option<String>),
    /// Saves what shows on each of the next this many frames, as the step before
    /// plays out, rather than once it rests.
    Frames(u32),
}

/// What an `expect` line says the view shows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expect {
    /// A field of the snapshot's `state` reads this: `mode journey`, `depth 2`.
    State {
        field: String,
        value: String,
    },
    /// The function selected, the journey's cursor, its start, or the function set
    /// aside in a journey; `None` for none.
    Selection(Option<String>),
    Cursor(Option<String>),
    Start(Option<String>),
    Aside(Option<String>),
    /// The function's name is drawn on the graph.
    Named(String),
    /// The function's node is drawn on screen.
    Drawn(String),
    /// A notice, said for a moment, shows these words (any case).
    Notice(String),
    /// The camera stands where it stood at the snapshot with this label.
    CameraSameAs(String),
    /// Nothing moved since the last snapshot: nodes, spheres, names, camera.
    NothingMoved,
    /// A check of [`crate::checks`] passes, or fails, on the view now.
    Check {
        name: String,
        pass: bool,
    },
}

/// What a step may move, which says what the checks hold it to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Moves nothing: a hover, a cursor key, `i`, a wait, an `expect`.
    Still,
    /// A click on a function or a bar: moves nothing, but expanded in a journey it may
    /// re-centre on what it clicks.
    Click,
    /// `,`, `.`, ↓ or a badge: shows or hides calls; the camera moves only to bring new
    /// ones on.
    Depth,
    /// `x` or a double click on a module: expands or collapses.
    Expand,
    /// A wheel turn, nearer or (`out`) further.
    Zoom { out: bool },
    /// An orbit by a drag.
    Orbit,
    /// A pan by a right-button drag.
    Pan,
    /// `f`: frames what the view is on; `-` and `=`, the map spread anew, too.
    FrameFocus,
    /// `r`: frames everything from the front.
    FrameAll,
    /// `window`: the window sized anew, which frames the view again as it was last
    /// framed, unless the hands moved the camera since.
    Resize,
    /// Back or Forward: a place put back, the camera as it stood there, perhaps where
    /// the hands put it.
    Back,
    /// Anything else, which may move the view.
    Moves,
}

/// A script's line: its number, its text, and the step it says.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Line {
    pub(crate) number: usize,
    pub(crate) text: String,
    pub(crate) step: Step,
}

/// Fields of the snapshot's `state` an `expect` may name, with the values each takes
/// (none listed: any).
const STATE_FIELDS: [(&str, &[&str]); 10] = [
    ("mode", &["browse", "journey"]),
    ("nucleus", &["modules", "expanded"]),
    ("expanded", &[]),
    ("depth", &[]),
    ("search", &["true", "false"]),
    ("help", &["true", "false"]),
    ("back", &["true", "false"]),
    ("forward", &["true", "false"]),
    ("outside_callers", &["true", "false"]),
    ("spread", &["0.5", "0.75", "1.0", "1.5"]),
];

/// Every step's verb, to suggest one for a misspelt verb.
const VERBS: [&str; 22] = [
    "key",
    "type",
    "click",
    "click-label",
    "click-empty",
    "hover",
    "hover-label",
    "shift+click",
    "doubleclick",
    "doubleclick-module",
    "bar",
    "chip",
    "search-pick",
    "button",
    "drag",
    "pan",
    "scroll",
    "window",
    "wait",
    "expect",
    "snap",
    "frames",
];

/// What may follow `expect`.
const EXPECTS: [&str; 11] = [
    "selection",
    "cursor",
    "start",
    "aside",
    "named",
    "drawn",
    "notice",
    "camera-same-as",
    "nothing-moved",
    "check",
    "state",
];

/// Reads a script, a step per line; blank lines and `#` comments are skipped.
///
/// # Errors
///
/// Names every line it cannot read.
pub(crate) fn parse(text: &str) -> Result<Vec<Line>, String> {
    let mut lines = Vec::new();
    let mut errors = Vec::new();
    let mut snaps: Vec<String> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let (number, text) = (index + 1, raw.trim());
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        match step(text, &snaps) {
            Ok(step) => {
                if let Step::Snap(label) = &step {
                    snaps.push(snap_label(label.as_deref()));
                }
                lines.push(Line {
                    number,
                    text: text.to_owned(),
                    step,
                });
            }
            Err(error) => errors.push(format!("line {number}: {error}: `{text}`")),
        }
    }
    if errors.is_empty() {
        Ok(lines)
    } else {
        Err(errors.join("\n"))
    }
}

fn step(line: &str, snaps: &[String]) -> Result<Step, String> {
    let (verb, rest) = line
        .split_once(char::is_whitespace)
        .map_or((line, ""), |(verb, rest)| (verb, rest.trim()));
    let needs = |what: &str| {
        if rest.is_empty() {
            Err(format!("`{verb}` needs {what}"))
        } else {
            Ok(rest)
        }
    };
    let point = |gesture| {
        Ok(Step::Point {
            gesture,
            unit: needs("a function")?.to_owned(),
        })
    };
    match verb.to_lowercase().as_str() {
        "key" => {
            let key = needs("a key")?;
            Press::parse(key).map(Step::Key).ok_or_else(|| {
                format!("unknown key `{key}` (keys: a letter, a digit, , . / [ ] - =, up, down, left, right, enter, esc, space, tab, backspace, each with an optional shift+)")
            })
        }
        "type" => Ok(Step::Type(needs("text")?.to_owned())),
        "hover" => point(Gesture::Hover),
        "hover-label" => point(Gesture::LabelHover),
        "click" => match pair(rest) {
            Some(at) => Ok(Step::ClickAt(at)),
            None => point(Gesture::Click),
        },
        "click-empty" => Ok(Step::ClickEmpty),
        "shift+click" => point(Gesture::ShiftClick),
        "doubleclick" => point(Gesture::DoubleClick),
        "click-label" => point(Gesture::LabelClick),
        "doubleclick-module" => Ok(Step::DoubleClickModule(needs("a module")?.to_owned())),
        "bar" => Ok(Step::Bar(needs("a number or a function")?.to_owned())),
        "chip" => Ok(Step::Chip(needs("a bar's number or function")?.to_owned())),
        "search-pick" => Ok(Step::SearchPick(needs("a row's text")?.to_owned())),
        "button" => Ok(Step::Button(needs("its text")?.to_owned())),
        "drag" => pair(needs("dx and dy")?)
            .map(Step::Drag)
            .ok_or_else(|| "`drag` needs dx and dy".to_owned()),
        "pan" => pair(needs("dx and dy")?)
            .map(Step::Pan)
            .ok_or_else(|| "`pan` needs dx and dy".to_owned()),
        "frames" => {
            let n = needs("a number of frames")?;
            n.parse()
                .ok()
                .filter(|n| (1..=MOST_FRAMES).contains(n))
                .map(Step::Frames)
                .ok_or_else(|| format!("`frames` needs a number of frames, 1 to {MOST_FRAMES}"))
        }
        "scroll" => scroll(needs("notches")?),
        "window" => pair(needs("a width and a height")?)
            .filter(|size| size.x >= 200.0 && size.y >= 200.0)
            .map(Step::Window)
            .ok_or_else(|| "`window` needs a width and a height of 200 or more".to_owned()),
        "wait" => {
            let ms = needs("milliseconds")?;
            ms.parse()
                .map(|ms| Step::Wait(Duration::from_millis(ms)))
                .map_err(|_| format!("`{ms}` is not a number of milliseconds"))
        }
        "expect" => expect(needs("what to expect")?, snaps).map(Step::Expect),
        "snap" => Ok(Step::Snap((!rest.is_empty()).then(|| rest.to_owned()))),
        other => Err(unknown("step", other, VERBS)),
    }
}

/// Most frames `frames` saves.
pub(crate) const MOST_FRAMES: u32 = 30;

/// Whether a bar is named by its number (`2`, `1.2`) rather than its function.
pub(crate) fn bar_number(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Two numbers, as `120 -40`.
fn pair(text: &str) -> Option<Vec2> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [x, y] = parts.as_slice() else {
        return None;
    };
    Some(Vec2::new(x.parse().ok()?, y.parse().ok()?))
}

/// `scroll N`, `scroll DX DY`, either followed by `at UNIT` or `at X Y`.
fn scroll(text: &str) -> Result<Step, String> {
    let (notches, at) = match text.split_once(" at ") {
        Some((notches, at)) => (notches.trim(), Some(at.trim())),
        None => (text, None),
    };
    let whole = |n: &str| {
        n.parse::<i32>()
            .map(|n| n as f32)
            .map_err(|_| format!("`{n}` is not a whole number of notches"))
    };
    let parts: Vec<&str> = notches.split_whitespace().collect();
    let by = match parts.as_slice() {
        [dy] => Vec2::new(0.0, whole(dy)?),
        [dx, dy] => Vec2::new(whole(dx)?, whole(dy)?),
        _ => return Err("`scroll` needs notches: `scroll N` or `scroll DX DY`".to_owned()),
    };
    let at = at.map(|at| match pair(at) {
        Some(point) => Aim::At(point),
        None => Aim::Unit(at.to_owned()),
    });
    Ok(Step::Scroll { by, at })
}

fn expect(text: &str, snaps: &[String]) -> Result<Expect, String> {
    let (what, rest) = text
        .split_once(char::is_whitespace)
        .map_or((text, ""), |(what, rest)| (what, rest.trim()));
    let unit = || {
        if rest.is_empty() {
            Err(format!("`expect {what}` needs a function"))
        } else {
            Ok(rest.to_owned())
        }
    };
    let maybe = || unit().map(|unit| (unit != "none").then_some(unit));
    match what {
        "selection" => maybe().map(Expect::Selection),
        "cursor" => maybe().map(Expect::Cursor),
        "start" => maybe().map(Expect::Start),
        "aside" => maybe().map(Expect::Aside),
        "named" => unit().map(Expect::Named),
        "drawn" => unit().map(Expect::Drawn),
        "notice" if rest.is_empty() => Err("`expect notice` needs the words it shows".to_owned()),
        "notice" => Ok(Expect::Notice(rest.to_owned())),
        "nothing-moved" => Ok(Expect::NothingMoved),
        "camera-same-as" => {
            let label = snap_label(Some(rest));
            // `03-before-x` names the same snapshot as `before x`.
            let bare = label
                .split_once('-')
                .filter(|(n, _)| n.chars().all(|c| c.is_ascii_digit()))
                .map_or(label.as_str(), |(_, bare)| bare);
            if rest.is_empty() {
                Err("`expect camera-same-as` needs a snapshot's label".to_owned())
            } else if snaps.iter().any(|s| *s == bare) {
                Ok(Expect::CameraSameAs(bare.to_owned()))
            } else {
                Err(unknown(
                    "snapshot before this line",
                    rest,
                    snaps.iter().map(String::as_str),
                ))
            }
        }
        "check" => {
            let (name, verdict) = rest
                .split_once(char::is_whitespace)
                .map_or((rest, "pass"), |(name, verdict)| (name, verdict.trim()));
            if !crate::checks::NAMES.contains(&name) {
                return Err(unknown("check", name, crate::checks::NAMES));
            }
            let pass = match verdict {
                "pass" => true,
                "fail" => false,
                other => return Err(format!("`{other}` is neither `pass` nor `fail`")),
            };
            Ok(Expect::Check {
                name: name.to_owned(),
                pass,
            })
        }
        "state" => {
            let (field, value) = rest
                .split_once(char::is_whitespace)
                .ok_or("`expect state` needs a field and a value")?;
            state(field, value.trim())
        }
        field if STATE_FIELDS.iter().any(|(f, _)| *f == field) => state(field, rest),
        other => {
            let known = EXPECTS
                .iter()
                .copied()
                .chain(STATE_FIELDS.iter().map(|(f, _)| *f));
            Err(unknown("expectation", other, known))
        }
    }
}

fn state(field: &str, value: &str) -> Result<Expect, String> {
    let Some((_, values)) = STATE_FIELDS.iter().find(|(f, _)| *f == field) else {
        return Err(unknown(
            "state field",
            field,
            STATE_FIELDS.iter().map(|(f, _)| *f),
        ));
    };
    if value.is_empty() {
        return Err(format!("`expect {field}` needs a value"));
    }
    if !values.is_empty() && !values.contains(&value) {
        return Err(format!(
            "`{field}` is one of {}, not `{value}`",
            values.join(", ")
        ));
    }
    if field == "depth" && value.parse::<u32>().is_err() {
        return Err(format!("depth `{value}` is not a number"));
    }
    Ok(Expect::State {
        field: field.to_owned(),
        value: value.to_owned(),
    })
}

/// Says `what` is unknown, with the names nearest it.
pub(crate) fn unknown<'a>(
    what: &str,
    name: &str,
    known: impl IntoIterator<Item = &'a str>,
) -> String {
    let near = close_matches(name, known);
    if near.is_empty() {
        format!("unknown {what} `{name}`")
    } else {
        format!("unknown {what} `{name}`; did you mean {}?", near.join(", "))
    }
}

/// Most names [`close_matches`] offers.
const SUGGESTED: usize = 5;

/// The names among `known` nearest `name`, best first: the same but for case, then
/// those holding it, then those whose last parts (as many as `name` has) are the same
/// or hold each other, then those a few letters off.
pub(crate) fn close_matches<'a>(
    name: &str,
    known: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let parts = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split([':', '.', '/', ' '])
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let wanted = name.to_lowercase();
    let wanted_parts = parts(name);
    let tail = |s: &str| {
        let own = parts(s);
        own[own.len().saturating_sub(wanted_parts.len())..].join("::")
    };
    let wanted_tail = wanted_parts.join("::");
    let mut scored: Vec<(usize, usize, &str)> = known
        .into_iter()
        .filter_map(|candidate| {
            let lower = candidate.to_lowercase();
            let own = tail(candidate);
            let score = if lower == wanted || lower.ends_with(&wanted) {
                0
            } else if lower.contains(&wanted) {
                1
            } else if own == wanted_tail {
                2
            } else if own.len() > 1 && (own.contains(&wanted_tail) || wanted_tail.contains(&own)) {
                3
            } else {
                let off = edits(&own, &wanted_tail);
                if off > (wanted_tail.chars().count() / 3).max(2) {
                    return None;
                }
                4 + off
            };
            Some((score, candidate.len(), candidate))
        })
        .collect();
    scored.sort_unstable();
    let mut near: Vec<String> = Vec::new();
    for (_, _, candidate) in scored {
        if !near.iter().any(|n| n == candidate) {
            near.push(candidate.to_owned());
        }
        if near.len() == SUGGESTED {
            break;
        }
    }
    near
}

/// Letters to add, drop or change to turn `a` into `b`.
fn edits(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ca != cb))
                .min(row[j] + 1)
                .min(above + 1);
            diagonal = above;
        }
    }
    row[b.len()]
}

impl Step {
    /// What the step may move.
    pub(crate) fn kind(&self) -> Kind {
        match self {
            Step::Key(press) => match press.code() {
                (
                    KeyCode::ArrowUp
                    | KeyCode::ArrowLeft
                    | KeyCode::ArrowRight
                    | KeyCode::BracketLeft
                    | KeyCode::BracketRight
                    | KeyCode::KeyI,
                    false,
                ) => Kind::Still,
                // ↓ on a call whose calls are folded opens them, as a badge does.
                (KeyCode::Comma | KeyCode::Period | KeyCode::ArrowDown, false) => Kind::Depth,
                (KeyCode::KeyX, false) => Kind::Expand,
                // - and = spread the map, then frame what the view is on as F does.
                (KeyCode::KeyF | KeyCode::Minus | KeyCode::Equal, false) => Kind::FrameFocus,
                (KeyCode::KeyR, false) => Kind::FrameAll,
                (KeyCode::Backspace, _) => Kind::Back,
                _ => Kind::Moves,
            },
            Step::Point { gesture, .. } => match gesture {
                Gesture::Hover | Gesture::LabelHover => Kind::Still,
                Gesture::Click | Gesture::LabelClick => Kind::Click,
                Gesture::ShiftClick => Kind::Moves,
                Gesture::DoubleClick => Kind::Expand,
            },
            Step::Bar(_) => Kind::Click,
            // Unfolding shows calls, as a depth change does.
            Step::Chip(_) => Kind::Depth,
            Step::DoubleClickModule(_) => Kind::Expand,
            Step::Scroll { by, .. } => Kind::Zoom { out: by.y < 0.0 },
            Step::Drag(_) => Kind::Orbit,
            Step::Pan(_) => Kind::Pan,
            // The panel's Fit frames as F does.
            Step::Button(text) if text.trim().eq_ignore_ascii_case("fit") => Kind::FrameFocus,
            Step::Wait(_) | Step::Expect(_) | Step::Snap(_) | Step::Frames(_) => Kind::Still,
            Step::Type(_)
            | Step::ClickAt(_)
            | Step::ClickEmpty
            | Step::SearchPick(_)
            | Step::Button(_) => Kind::Moves,
            Step::Window(_) => Kind::Resize,
        }
    }

    /// The function or module the step names, to look up before the window opens.
    pub(crate) fn names(&self) -> Option<Name<'_>> {
        match self {
            Step::Point { unit, .. }
            | Step::Scroll {
                at: Some(Aim::Unit(unit)),
                ..
            }
            | Step::Expect(
                Expect::Selection(Some(unit))
                | Expect::Cursor(Some(unit))
                | Expect::Start(Some(unit))
                | Expect::Aside(Some(unit))
                | Expect::Named(unit)
                | Expect::Drawn(unit),
            ) => Some(Name::Unit(unit)),
            Step::Bar(bar) | Step::Chip(bar) if !bar_number(bar) => Some(Name::Bar(bar)),
            Step::DoubleClickModule(module) => Some(Name::Module(module)),
            _ => None,
        }
    }
}

/// A function or module a script names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Name<'a> {
    Unit(&'a str),
    Module(&'a str),
    /// A journey bar by its function: looked up as a function when it names one, else
    /// found by the name on the bar when the step comes.
    Bar(&'a str),
}

/// A snapshot's label as its file name has it: lowercase words joined by dashes.
pub(crate) fn snap_label(label: Option<&str>) -> String {
    let words: Vec<String> = label
        .unwrap_or("snap")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        "snap".to_owned()
    } else {
        words.join("-")
    }
}

/// A snapshot's file stem: its number, then its label.
pub(crate) fn snap_name(number: usize, label: Option<&str>) -> String {
    format!("{number:02}-{}", snap_label(label))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(script: &str) -> Vec<Step> {
        parse(script)
            .unwrap()
            .into_iter()
            .map(|line| line.step)
            .collect()
    }

    fn unit(gesture: Gesture, name: &str) -> Step {
        Step::Point {
            gesture,
            unit: name.to_owned(),
        }
    }

    #[test]
    fn reads_every_kind_of_step() {
        let script = "
            # open it up
            snap start
            key shift+i
            key backspace
            click Explorer::spotlit
            doubleclick main
            shift+click main
            hover main
            click-label main
            hover-label main
            bar 1.2
            button 3 callers
            drag 120 -40
            scroll -2
            wait 300
            snap
            click 400 300
            click POST /users
            click-empty
            doubleclick-module orders
            type spotlit
            scroll 0 3 at orders::place
            scroll 1 -2 at 800 500
            window 1200 800
        ";
        let steps = steps(script);
        assert_eq!(steps.len(), 23);
        assert_eq!(steps[0], Step::Snap(Some("start".to_owned())));
        assert_eq!(steps[1], Step::Key(Press::parse("shift+i").unwrap()));
        assert_eq!(steps[3], unit(Gesture::Click, "Explorer::spotlit"));
        assert_eq!(steps[7], unit(Gesture::LabelClick, "main"));
        assert_eq!(steps[8], unit(Gesture::LabelHover, "main"));
        assert_eq!(steps[9], Step::Bar("1.2".to_owned()));
        assert_eq!(steps[10], Step::Button("3 callers".to_owned()));
        assert_eq!(steps[11], Step::Drag(Vec2::new(120.0, -40.0)));
        assert_eq!(
            steps[12],
            Step::Scroll {
                by: Vec2::new(0.0, -2.0),
                at: None
            }
        );
        assert_eq!(steps[13], Step::Wait(Duration::from_millis(300)));
        assert_eq!(steps[14], Step::Snap(None));
        assert_eq!(steps[15], Step::ClickAt(Vec2::new(400.0, 300.0)));
        assert_eq!(steps[16], unit(Gesture::Click, "POST /users"));
        assert_eq!(steps[17], Step::ClickEmpty);
        assert_eq!(steps[18], Step::DoubleClickModule("orders".to_owned()));
        assert_eq!(steps[19], Step::Type("spotlit".to_owned()));
        assert_eq!(
            steps[20],
            Step::Scroll {
                by: Vec2::new(0.0, 3.0),
                at: Some(Aim::Unit("orders::place".to_owned()))
            }
        );
        assert_eq!(
            steps[21],
            Step::Scroll {
                by: Vec2::new(1.0, -2.0),
                at: Some(Aim::At(Vec2::new(800.0, 500.0)))
            }
        );
        assert_eq!(steps[22], Step::Window(Vec2::new(1200.0, 800.0)));
    }

    #[test]
    fn reads_every_kind_of_expectation() {
        let script = "
            snap before x
            expect mode journey
            expect depth 2
            expect state search true
            expect selection orders::place
            expect cursor none
            expect start main
            expect named main
            expect drawn main
            expect notice Nothing to go back
            expect camera-same-as before x
            expect camera-same-as 01-before-x
            expect nothing-moved
            expect check dots-apart
            expect check labels-near fail
        ";
        let expects: Vec<Expect> = steps(script)
            .into_iter()
            .filter_map(|s| match s {
                Step::Expect(e) => Some(e),
                _ => None,
            })
            .collect();
        let state = |f: &str, v: &str| Expect::State {
            field: f.to_owned(),
            value: v.to_owned(),
        };
        assert_eq!(
            expects,
            [
                state("mode", "journey"),
                state("depth", "2"),
                state("search", "true"),
                Expect::Selection(Some("orders::place".to_owned())),
                Expect::Cursor(None),
                Expect::Start(Some("main".to_owned())),
                Expect::Named("main".to_owned()),
                Expect::Drawn("main".to_owned()),
                Expect::Notice("Nothing to go back".to_owned()),
                Expect::CameraSameAs("before-x".to_owned()),
                Expect::CameraSameAs("before-x".to_owned()),
                Expect::NothingMoved,
                Expect::Check {
                    name: "dots-apart".to_owned(),
                    pass: true
                },
                Expect::Check {
                    name: "labels-near".to_owned(),
                    pass: false
                },
            ]
        );
    }

    #[test]
    fn every_bad_line_is_named_with_what_was_meant() {
        let error = parse(
            "snap\nkey nope\nclik main\nexpect mode joruney\nexpect check dots-apert\n\
             expect camera-same-as later\nsnap later\nexpect selction main\ndrag 3",
        )
        .unwrap_err();
        let lines: Vec<&str> = error.lines().collect();
        assert_eq!(lines.len(), 7, "{error}");
        assert!(lines[0].starts_with("line 2: unknown key `nope`"));
        assert!(lines[1].contains("unknown step `clik`; did you mean click"));
        assert!(lines[2].contains("`mode` is one of browse, journey"));
        assert!(lines[3].contains("did you mean dots-apart?"));
        assert!(lines[4].contains("unknown snapshot before this line `later`"));
        assert!(lines[5].contains("did you mean selection"));
        assert!(lines[6].contains("dx and dy"));
    }

    #[test]
    fn suggests_names_nearest_first() {
        let known = [
            "rust:app::Explorer::spotlit",
            "rust:app::Explorer::spotlight",
            "rust:app::Finder::walk",
            "rust:app::orders::place",
        ];
        assert_eq!(
            close_matches("explorer::spotlit", known)[0],
            "rust:app::Explorer::spotlit"
        );
        assert_eq!(
            close_matches("Explorer::spotlt", known),
            [
                "rust:app::Explorer::spotlit",
                "rust:app::Explorer::spotlight"
            ]
        );
        assert_eq!(
            close_matches("Finder::wakl", known),
            ["rust:app::Finder::walk"]
        );
        assert_eq!(
            close_matches("orders::plcae", known),
            ["rust:app::orders::place"],
            "the module part counts too"
        );
        assert_eq!(close_matches("zzzz", known), Vec::<String>::new());
        assert_eq!(edits("walk", "wakl"), 2);
    }

    #[test]
    fn says_what_each_step_may_move() {
        let kind = |line: &str| steps(line)[0].kind();
        assert_eq!(kind("key down"), Kind::Depth);
        assert_eq!(kind("key up"), Kind::Still);
        assert_eq!(kind("key ]"), Kind::Still);
        assert_eq!(kind("key i"), Kind::Still);
        assert_eq!(kind("key ."), Kind::Depth);
        assert_eq!(kind("key x"), Kind::Expand);
        assert_eq!(kind("key f"), Kind::FrameFocus);
        assert_eq!(kind("key ="), Kind::FrameFocus);
        assert_eq!(kind("key r"), Kind::FrameAll);
        assert_eq!(kind("key esc"), Kind::Moves);
        assert_eq!(kind("key backspace"), Kind::Back);
        assert_eq!(kind("key shift+backspace"), Kind::Back);
        assert_eq!(kind("hover main"), Kind::Still);
        assert_eq!(kind("click main"), Kind::Click);
        assert_eq!(kind("bar 2"), Kind::Click);
        assert_eq!(kind("shift+click main"), Kind::Moves);
        assert_eq!(kind("doubleclick-module m"), Kind::Expand);
        assert_eq!(kind("doubleclick main"), Kind::Expand);
        assert_eq!(kind("scroll 2"), Kind::Zoom { out: false });
        assert_eq!(kind("scroll -1 at main"), Kind::Zoom { out: true });
        assert_eq!(kind("drag 1 1"), Kind::Orbit);
        assert_eq!(kind("pan 1 1"), Kind::Pan);
        assert_eq!(kind("button Fit"), Kind::FrameFocus);
        assert_eq!(kind("chip 2"), Kind::Depth);
        assert_eq!(kind("search-pick place"), Kind::Moves);
        assert_eq!(kind("window 900 700"), Kind::Resize);
        assert_eq!(kind("frames 4"), Kind::Still);
    }

    #[test]
    fn reads_pans_frames_badges_and_search_rows() {
        let steps = steps(
            "key x\nframes 6\npan 30 -10\nchip 2.1\nchip Explorer::spotlit\nsearch-pick orders::place\nbar Explorer.spotlit",
        );
        assert_eq!(steps[1], Step::Frames(6));
        assert_eq!(steps[2], Step::Pan(Vec2::new(30.0, -10.0)));
        assert_eq!(steps[3], Step::Chip("2.1".to_owned()));
        assert_eq!(steps[5], Step::SearchPick("orders::place".to_owned()));
        let names: Vec<Option<Name>> = steps.iter().map(Step::names).collect();
        assert_eq!(names[3], None, "a bar by number names no function");
        assert_eq!(names[4], Some(Name::Bar("Explorer::spotlit")));
        assert_eq!(names[6], Some(Name::Bar("Explorer.spotlit")));
        let error = parse("frames 0\nframes 99\npan 3").unwrap_err();
        assert_eq!(error.lines().count(), 3, "{error}");
    }

    #[test]
    fn lists_the_names_to_look_up() {
        let lines = parse("click a\nscroll 1 at b\nscroll 1 at 3 4\ndoubleclick-module m\nexpect cursor none\nexpect named c").unwrap();
        let names: Vec<Name> = lines.iter().filter_map(|l| l.step.names()).collect();
        assert_eq!(
            names,
            [
                Name::Unit("a"),
                Name::Unit("b"),
                Name::Module("m"),
                Name::Unit("c")
            ]
        );
    }

    #[test]
    fn names_snapshots_by_number_and_label() {
        assert_eq!(snap_name(3, Some("After X!")), "03-after-x");
        assert_eq!(snap_name(12, None), "12-snap");
    }
}
