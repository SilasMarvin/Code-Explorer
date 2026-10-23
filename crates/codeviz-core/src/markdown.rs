//! A story as Markdown: a numbered outline of the calls, each with a few words on when
//! it runs and how it can fail, then the ways the journey ends. It reads the same
//! pasted into a pull request, a design doc or a chat thread.

use std::fmt::Write as _;

use codeviz_ir::{ExitKind, Mode};

use crate::{
    Change, Condition, StopReason,
    story::{Beat, Ending, Story},
};

impl Story {
    /// The story as a Markdown outline.
    #[must_use]
    pub fn markdown(&self) -> String {
        let mut out = format!("{}:\n\n", self.title());
        // How the root changed is in the title.
        let notes = notes(&Beat {
            change: None,
            calls: Vec::new(),
            ..self.root.clone()
        });
        if !notes.is_empty() {
            let _ = writeln!(out, "- {} itself: {notes}", code(&self.root.name));
        }
        for beat in &self.root.calls {
            line(beat, 0, &mut out);
        }
        if self.root.calls.is_empty() && notes.is_empty() {
            out.push_str("- (no calls)\n");
        }
        if !self.endings.is_empty() {
            out.push_str("\nOutcomes:\n");
            // Outcomes reached along different paths read the same: one line each.
            let mut lines: Vec<String> = Vec::new();
            for ending in &self.endings {
                let this = if self.told.as_ref() == Some(ending) && self.endings.len() > 1 {
                    " (this path)"
                } else {
                    ""
                };
                let line = format!("- {}{this}\n", outcome(ending));
                if !lines.contains(&line) {
                    lines.push(line);
                }
            }
            out.extend(lines);
        }
        out
    }

    /// `` `POST /users` → `handlers.create_user`, when it completes ``, with how the
    /// root changed under a diff: `` `orders.place` (rewritten), every path ``.
    #[must_use]
    pub fn title(&self) -> String {
        let root = match &self.entry {
            Some(entry) => format!("{} → {}", code(entry), code(&self.root.name)),
            None => code(&self.root.name),
        };
        let root = match self.root.change {
            Some(change) => format!("{root} ({})", change_word(change)),
            None => root,
        };
        let path = match &self.told {
            None => "every path".to_owned(),
            Some(ending) if ending.kind == ExitKind::Return => "when it completes".to_owned(),
            Some(ending) => format!("when it ends in {}", outcome(ending)),
        };
        format!("{root}, {path}")
    }
}

fn line(beat: &Beat, depth: usize, out: &mut String) {
    let notes = notes(beat);
    let _ = writeln!(
        out,
        "{}- {} {}{}",
        "  ".repeat(depth),
        beat.number,
        code(&beat.name),
        if notes.is_empty() {
            String::new()
        } else {
            format!(" — {notes}")
        }
    );
    for call in &beat.calls {
        line(call, depth + 1, out);
    }
}

/// What is worth saying about a call, most telling first, as in ``only if `bulk`, can
/// fail: `unknown sku` ``.
fn notes(beat: &Beat) -> String {
    let mut notes: Vec<String> = beat
        .change
        .map(change_word)
        .map(str::to_owned)
        .into_iter()
        .collect();
    if !beat.dropped.is_empty() {
        let names: Vec<String> = beat.dropped.iter().map(|n| code(n)).collect();
        notes.push(format!("no longer calls {}", names.join(", ")));
    }
    match beat.mode {
        Mode::Spawn => notes.push("in the background".to_owned()),
        Mode::Register => notes.push("handed off, runs later".to_owned()),
        Mode::Call | Mode::Unknown => {}
    }
    if !beat.conditions.is_empty() {
        notes.push(format!("only {}", when(&beat.conditions)));
    }
    if beat.repeats {
        notes.push("repeats".to_owned());
    }
    match beat.stopped {
        Some(StopReason::Recursion) => notes.push("recursive".to_owned()),
        Some(StopReason::DepthLimit) => notes.push("too deep to follow".to_owned()),
        Some(StopReason::NotLoaded) => notes.push("not loaded".to_owned()),
        Some(StopReason::Registered) | None => {}
    }
    if beat.guessed {
        notes.push("guessed".to_owned());
    }
    if !beat.fails.is_empty() {
        notes.push(can_fail(beat));
    }
    if let Some(end) = &beat.ends {
        if end.origin.as_ref() == Some(&beat.name) {
            notes.push(format!(
                "ends in {}",
                outcome(&Ending {
                    origin: None,
                    ..end.clone()
                })
            ));
        } else {
            notes.push(format!("ends in {}", outcome(end)));
        }
    }
    match beat.folded {
        0 => {}
        1 => notes.push("+1 call".to_owned()),
        n => notes.push(format!("+{n} calls")),
    }
    notes.join(", ")
}

/// Code longer than this is cut: a guard or an error can be a whole expression.
const MAX_CODE: usize = 80;

/// `text` as a Markdown code span, on one line and cut to [`MAX_CODE`]. Backticks
/// inside it take a longer fence, as Markdown requires.
pub(crate) fn code(text: &str) -> String {
    let tidy = tidy(text);
    let cut: String = if tidy.chars().count() > MAX_CODE {
        tidy.chars()
            .take(MAX_CODE - 1)
            .chain(std::iter::once('…'))
            .collect()
    } else {
        tidy
    };
    let longest = cut
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    if longest == 0 {
        return format!("`{cut}`");
    }
    let fence = "`".repeat(longest + 1);
    format!("{fence} {cut} {fence}")
}

/// Source text on one line, without the spaces that formatting leaves inside
/// brackets and method chains: `Invalid(format!("bad"))`, `args.iter().map(read)`.
pub(crate) fn tidy(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::with_capacity(joined.len());
    let mut chars = joined.chars().peekable();
    while let Some(c) = chars.next() {
        let next = chars.peek().copied();
        let drop = c == ' '
            && (matches!(next, Some(')' | ']'))
                || matches!(out.chars().last(), Some('(' | '['))
                || next == Some('.')
                    && chars
                        .clone()
                        .nth(1)
                        .is_some_and(|n| n.is_alphabetic() || n == '_'));
        if !drop {
            out.push(c);
        }
    }
    out
}

/// How a diff changed a call, in a word.
pub(crate) fn change_word(change: Change) -> &'static str {
    match change {
        Change::Added => "new",
        Change::Modified => "rewritten",
        Change::Moved => "moved",
        Change::Removed => "removed",
    }
}

/// ``can fail: `unknown sku` ``, or ``can fail in `pricing.price` `` for a failure in
/// a call not shown.
fn can_fail(beat: &Beat) -> String {
    let (here, under): (Vec<&Ending>, Vec<&Ending>) = beat
        .failures()
        .into_iter()
        .partition(|e| e.origin.as_ref() == Some(&beat.name));
    let labels: Vec<String> = here.into_iter().map(failure).collect();
    let mut places: Vec<String> = Vec::new();
    for origin in under.iter().filter_map(|e| e.origin.as_ref()) {
        let place = code(origin);
        if !places.contains(&place) {
            places.push(place);
        }
    }
    match (labels.is_empty(), places.is_empty()) {
        (false, true) => format!("can fail: {}", labels.join(", ")),
        (true, _) => format!("can fail in {}", places.join(", ")),
        (false, false) => format!(
            "can fail: {}, and in {}",
            labels.join(", "),
            places.join(", ")
        ),
    }
}

/// The arms a call needs, as a phrase: ``if `bulk` ``, ``if `sku` is `"widget"` and not
/// `cached` ``, or ``on `except KeyError` `` for the arms of a `try`, which have no
/// guard.
#[must_use]
pub(crate) fn when(conditions: &[Condition]) -> String {
    let (guarded, unguarded): (Vec<&Condition>, Vec<&Condition>) =
        conditions.iter().partition(|c| c.guard.is_some());
    let mut parts = Vec::new();
    if !guarded.is_empty() {
        let tests: Vec<String> = guarded.into_iter().map(test).collect();
        parts.push(format!("if {}", tests.join(" and ")));
    }
    if !unguarded.is_empty() {
        let arms: Vec<String> = unguarded.into_iter().map(|c| code(c.arm.trim())).collect();
        parts.push(format!("on {}", arms.join(", ")));
    }
    parts.join(" and ")
}

/// Whether an arm is the one taken when its guard is false.
pub(crate) fn otherwise(condition: &Condition) -> bool {
    matches!(condition.arm.trim(), "else" | "no" | "false")
}

/// What one guarded arm tests: `` `bulk` ``, `` not `bulk` ``, `` `sku` is `"widget"` ``.
fn test(condition: &Condition) -> String {
    let arm = condition.arm.trim();
    let guard = condition.guard.as_deref().unwrap_or_default().trim();
    match arm {
        "then" | "yes" | "true" => code(guard),
        "else" | "no" | "false" => match negated(guard) {
            Some(positive) => code(positive),
            None => format!("not {}", code(guard)),
        },
        _ => {
            let subject = guard.strip_prefix("match ").unwrap_or(guard);
            let pattern = arm.strip_prefix("case ").unwrap_or(arm);
            format!("{} is {}", code(subject), code(pattern))
        }
    }
}

/// `looks_like(x)` for `!looks_like(x)` or `not looks_like(x)`, when the negation
/// covers the whole guard.
fn negated(guard: &str) -> Option<&str> {
    let rest = guard
        .strip_prefix('!')
        .or_else(|| guard.strip_prefix("not "))?
        .trim_start();
    let mut depth = 0i32;
    for c in rest.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => return None,
            _ => {}
        }
    }
    (!rest.is_empty()).then_some(rest)
}

/// `` `unknown sku` ``, or `` `.ok_or` fails `` for an error passed on by a call the
/// producer could not follow.
fn failure(ending: &Ending) -> String {
    match ending.label.strip_suffix(" fails") {
        Some(call) => format!("{} fails", code(call)),
        None => code(&ending.label),
    }
}

/// ``error `unknown sku` in `pricing.price` ``, or `completes`.
fn outcome(ending: &Ending) -> String {
    let kind = match ending.kind {
        ExitKind::Return => return ending.label.clone(),
        ExitKind::Error => "error",
        ExitKind::Panic => "panic",
        ExitKind::Unknown => "exit",
    };
    match &ending.origin {
        Some(origin) => format!("{kind} {} in {}", failure(ending), code(origin)),
        None => format!("{kind} {}", failure(ending)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_code_as_markdown_reads_it() {
        assert_eq!(code("place"), "`place`");
        assert_eq!(code("bail!(\"no `{q}`\")"), "`` bail!(\"no `{q}`\") ``");
        assert_eq!(
            code("Invalid(format!( \"x\" ))"),
            "`Invalid(format!(\"x\"))`"
        );
        assert_eq!(
            code("args\n    .iter()\n    .map(read)"),
            "`args.iter().map(read)`"
        );
        assert_eq!(code("0 .. 10"), "`0 .. 10`");
        assert_eq!(code(&"x".repeat(100)).chars().count(), MAX_CODE + 2);
    }

    fn arm(guard: Option<&str>, arm: &str) -> String {
        when(&[Condition {
            guard: guard.map(str::to_owned),
            arm: arm.to_owned(),
        }])
    }

    #[test]
    fn reads_arms_as_phrases() {
        assert_eq!(arm(Some("qty >= 10"), "then"), "if `qty >= 10`");
        assert_eq!(arm(Some("user is None"), "yes"), "if `user is None`");
        assert_eq!(
            arm(Some("name.is_empty()"), "else"),
            "if not `name.is_empty()`"
        );
        assert_eq!(arm(Some("!valid(&x)"), "else"), "if `valid(&x)`");
        assert_eq!(arm(Some("not name"), "no"), "if `name`");
        assert_eq!(arm(Some("!a || b"), "else"), "if not `!a || b`");
        assert_eq!(
            arm(Some("match sku"), "\"widget\""),
            "if `sku` is `\"widget\"`"
        );
        assert_eq!(arm(Some("match sku"), "case _"), "if `sku` is `_`");
        assert_eq!(arm(None, "except KeyError"), "on `except KeyError`");
        let both = [
            Condition {
                guard: Some("big".to_owned()),
                arm: "no".to_owned(),
            },
            Condition {
                guard: Some("small".to_owned()),
                arm: "yes".to_owned(),
            },
        ];
        assert_eq!(when(&both), "if not `big` and `small`");
    }
}
