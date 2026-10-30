//! The steps of a scripted session (`--script`), one per line, as a person would take
//! them: keys, clicks and hovers on functions by name or on their drawn names, clicks
//! on journey bars and
//! buttons by what they say, drags, scrolls, waits, and snapshots of what shows.

use std::time::Duration;

use bevy::math::Vec2;
use codeviz_ir::UnitId;

use crate::capture::Press;

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
}

/// One step of a session.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Step {
    Key(Press),
    /// A gesture on a function, found as `--focus` finds it; the error says why it
    /// could not be, and fails the step when it comes, not the script.
    Point {
        gesture: Gesture,
        unit: Result<UnitId, String>,
    },
    /// A click on the journey bar with this number, as `1.2`.
    Bar(String),
    /// A click on the button or chip that says this.
    Button(String),
    /// Orbits by a drag of this many pixels.
    Drag(Vec2),
    /// Zooms by this many wheel notches; positive is nearer.
    Scroll(i32),
    Wait(Duration),
    /// Saves what shows, under this label.
    Snap(Option<String>),
}

/// Reads a script, a step per line; blank lines and `#` comments are skipped. `find`
/// looks a function up by name. Returns each step with its line.
///
/// # Errors
///
/// Names the first line it cannot read.
pub(crate) fn parse(
    text: &str,
    find: &dyn Fn(&str) -> Result<UnitId, String>,
) -> Result<Vec<(String, Step)>, String> {
    text.lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
        .map(|(number, line)| {
            step(line, find)
                .map(|step| (line.to_owned(), step))
                .map_err(|error| format!("line {number}: {error}: `{line}`"))
        })
        .collect()
}

fn step(line: &str, find: &dyn Fn(&str) -> Result<UnitId, String>) -> Result<Step, String> {
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
    let number = |text: &str| {
        text.parse::<f32>()
            .map_err(|_| format!("`{text}` is not a number"))
    };
    let point = |gesture| {
        Ok(Step::Point {
            gesture,
            unit: find(needs("a function")?),
        })
    };
    match verb.to_lowercase().as_str() {
        "key" => {
            let key = needs("a key")?;
            Press::parse(key)
                .map(Step::Key)
                .ok_or_else(|| format!("unknown key `{key}`"))
        }
        "hover" => point(Gesture::Hover),
        "click" => point(Gesture::Click),
        "shift+click" => point(Gesture::ShiftClick),
        "doubleclick" => point(Gesture::DoubleClick),
        "click-label" => point(Gesture::LabelClick),
        "bar" => Ok(Step::Bar(needs("a number")?.to_owned())),
        "button" => Ok(Step::Button(needs("its text")?.to_owned())),
        "drag" => {
            let parts: Vec<&str> = needs("dx and dy")?.split_whitespace().collect();
            let [dx, dy] = parts.as_slice() else {
                return Err("`drag` needs dx and dy".to_owned());
            };
            Ok(Step::Drag(Vec2::new(number(dx)?, number(dy)?)))
        }
        "scroll" => {
            let notches = needs("a number of notches")?;
            notches
                .parse()
                .map(Step::Scroll)
                .map_err(|_| format!("`{notches}` is not a whole number"))
        }
        "wait" => {
            let ms = needs("milliseconds")?;
            ms.parse()
                .map(|ms| Step::Wait(Duration::from_millis(ms)))
                .map_err(|_| format!("`{ms}` is not a number of milliseconds"))
        }
        "snap" => Ok(Step::Snap((!rest.is_empty()).then(|| rest.to_owned()))),
        _ => Err(format!("unknown step `{verb}`")),
    }
}

/// A snapshot's file stem: its number, then its label in lowercase words joined by
/// dashes.
pub(crate) fn snap_name(number: usize, label: Option<&str>) -> String {
    let words: Vec<String> = label
        .unwrap_or("snap")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let label = if words.is_empty() {
        "snap".to_owned()
    } else {
        words.join("-")
    };
    format!("{number:02}-{label}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(name: &str) -> Result<UnitId, String> {
        if name == "missing" {
            Err(format!("nothing matches `{name}`"))
        } else {
            Ok(UnitId::new(format!("x:{name}")))
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
            bar 1.2
            button 3 callers
            drag 120 -40
            scroll -2
            wait 300
            snap
        ";
        let steps: Vec<Step> = parse(script, &find)
            .unwrap()
            .into_iter()
            .map(|(_, s)| s)
            .collect();
        assert_eq!(steps.len(), 14);
        assert_eq!(steps[0], Step::Snap(Some("start".to_owned())));
        assert_eq!(steps[1], Step::Key(Press::parse("shift+i").unwrap()));
        assert_eq!(
            steps[3],
            Step::Point {
                gesture: Gesture::Click,
                unit: Ok(UnitId::new("x:Explorer::spotlit")),
            }
        );
        assert_eq!(
            steps[7],
            Step::Point {
                gesture: Gesture::LabelClick,
                unit: Ok(UnitId::new("x:main")),
            }
        );
        assert_eq!(steps[8], Step::Bar("1.2".to_owned()));
        assert_eq!(steps[9], Step::Button("3 callers".to_owned()));
        assert_eq!(steps[10], Step::Drag(Vec2::new(120.0, -40.0)));
        assert_eq!(steps[11], Step::Scroll(-2));
        assert_eq!(steps[12], Step::Wait(Duration::from_millis(300)));
        assert_eq!(steps[13], Step::Snap(None));
    }

    #[test]
    fn a_function_not_found_fails_its_step_not_the_script() {
        let steps = parse("click missing", &find).unwrap();
        assert!(matches!(&steps[0].1, Step::Point { unit: Err(e), .. } if e.contains("missing")));
    }

    #[test]
    fn a_line_it_cannot_read_is_named() {
        assert_eq!(
            parse("snap\nkey nope", &find),
            Err("line 2: unknown key `nope`: `key nope`".to_owned())
        );
        assert!(parse("drag 3", &find).unwrap_err().contains("dx and dy"));
        assert!(
            parse("fly away", &find)
                .unwrap_err()
                .contains("unknown step")
        );
    }

    #[test]
    fn names_snapshots_by_number_and_label() {
        assert_eq!(snap_name(3, Some("After X!")), "03-after-x");
        assert_eq!(snap_name(12, None), "12-snap");
    }
}
