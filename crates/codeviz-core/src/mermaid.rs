//! A story as a Mermaid sequence diagram, which GitHub and most doc tools draw from a
//! fenced `mermaid` block. Each participant is what the called functions belong to
//! (their module, class or type), so a flow through a few modules stays a few columns
//! wide however many functions it runs; messages carry the function and its number.
//! Calls that need a branch arm sit in `opt` and `alt` blocks, calls in a loop in a
//! `loop` block, and failures return as crossed arrows. It keeps to the long-standing
//! part of the syntax so older renderers draw it too.

use std::fmt::Write as _;

use codeviz_ir::{ExitKind, Mode};

use crate::{
    Condition, StopReason,
    markdown::{change_word, otherwise, tidy, when},
    story::{Beat, Ending, Story},
};

/// Messages and block labels longer than this are cut, to keep the diagram readable.
const MAX_LABEL: usize = 60;

/// Words Mermaid reads as keywords, which a participant id must not be.
const RESERVED: [&str; 22] = [
    "activate",
    "actor",
    "alt",
    "and",
    "as",
    "autonumber",
    "box",
    "break",
    "critical",
    "deactivate",
    "else",
    "end",
    "left",
    "link",
    "links",
    "loop",
    "note",
    "opt",
    "over",
    "par",
    "participant",
    "rect",
];

impl Story {
    /// The story as a Mermaid `sequenceDiagram`, without the code fence.
    #[must_use]
    pub fn mermaid(&self) -> String {
        let mut diagram = Diagram::default();
        diagram.lane_of(&self.root);
        for beat in self.beats() {
            if beat.unit.is_some() {
                diagram.lane_of(beat);
            }
        }
        let mut out = String::from("sequenceDiagram\n");
        if self.entry.is_some() {
            out.push_str("    actor caller as Caller\n");
        }
        for (id, label) in &diagram.lanes {
            if id == label {
                let _ = writeln!(out, "    participant {id}");
            } else {
                let _ = writeln!(out, "    participant {id} as {}", text(label));
            }
        }
        let root = diagram.lane_of(&self.root);
        if let Some(entry) = &self.entry {
            let _ = writeln!(out, "    caller->>{root}: {}", text(entry));
        }
        let _ = writeln!(out, "    activate {root}");
        let mut body = Body {
            diagram: &mut diagram,
            out: &mut out,
        };
        body.calls(&root, &self.root.calls, 0);
        body.endings(&root, self.entry.as_ref().map(|_| "caller"), &self.root, 0);
        let _ = writeln!(out, "    deactivate {root}");
        out
    }
}

/// The participants, in order of first appearance: an id Mermaid accepts, and what
/// the participant is called.
#[derive(Default)]
struct Diagram {
    lanes: Vec<(String, String)>,
}

impl Diagram {
    /// The participant a call goes to, added if new.
    fn lane_of(&mut self, beat: &Beat) -> String {
        let label = if beat.home.is_empty() {
            &beat.short
        } else {
            &beat.home
        };
        if let Some((id, _)) = self.lanes.iter().find(|(_, l)| l == label) {
            return id.clone();
        }
        let id = self.fresh_id(label);
        self.lanes.push((id.clone(), label.clone()));
        id
    }

    /// `shop_services_orders` for `shop.services.orders`; never a keyword, never taken.
    fn fresh_id(&self, label: &str) -> String {
        let mut base: String = label
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>()
            .trim_matches('_')
            .to_owned();
        if !base.starts_with(|c: char| c.is_ascii_alphabetic()) {
            base.insert(0, 'm');
        }
        let free = |id: &str| {
            id != "caller"
                && !RESERVED.contains(&id.to_ascii_lowercase().as_str())
                && self.lanes.iter().all(|(taken, _)| taken != id)
        };
        if free(&base) {
            return base;
        }
        // One of these is free: there are fewer lanes than numbers to try.
        (2..=self.lanes.len() + 2)
            .map(|n| format!("{base}_{n}"))
            .find(|id| free(id))
            .unwrap_or(base)
    }
}

/// What a call needs to happen: a loop around it, or a branch arm.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    Loop,
    Arm(Condition),
}

impl Block {
    /// Whether `other` is another arm of the same branch.
    fn sibling(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Arm(a), Self::Arm(b)) => a.guard == b.guard && a.arm != b.arm,
            _ => false,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Loop => "repeats".to_owned(),
            Self::Arm(condition) => {
                let phrase = when(std::slice::from_ref(condition)).replace('`', "");
                phrase.split_whitespace().collect::<Vec<_>>().join(" ")
            }
        }
    }

    /// The label of an arm that follows another of its branch in an `alt`.
    fn else_label(&self) -> String {
        match self {
            Self::Arm(condition) if otherwise(condition) => "otherwise".to_owned(),
            _ => self.label(),
        }
    }
}

fn blocks(beat: &Beat) -> Vec<Block> {
    let looped = beat.repeats.then_some(Block::Loop);
    looped
        .into_iter()
        .chain(beat.conditions.iter().cloned().map(Block::Arm))
        .collect()
}

struct Body<'a> {
    diagram: &'a mut Diagram,
    out: &'a mut String,
}

impl Body<'_> {
    fn line(&mut self, depth: usize, line: &str) {
        let _ = writeln!(self.out, "{}{line}", "    ".repeat(depth + 1));
    }

    /// The calls one caller makes, each inside the blocks it needs. Calls that need the
    /// same arm share one block, and the arms of one branch share an `alt`.
    fn calls(&mut self, caller: &str, beats: &[Beat], depth: usize) {
        let mut open: Vec<Block> = Vec::new();
        for (index, beat) in beats.iter().enumerate() {
            let want = blocks(beat);
            let mut common = open.iter().zip(&want).take_while(|(a, b)| a == b).count();
            let switch = matches!(
                (open.get(common), want.get(common)),
                (Some(was), Some(now)) if was.sibling(now)
            );
            let keep = if switch { common + 1 } else { common };
            while open.len() > keep {
                open.pop();
                self.line(depth + open.len(), "end");
            }
            if switch && let (Some(slot), Some(now)) = (open.get_mut(common), want.get(common)) {
                self.line(depth + common, &format!("else {}", text(&now.else_label())));
                *slot = now.clone();
                common += 1;
            }
            for (level, block) in want.iter().enumerate().skip(common) {
                let keyword = match block {
                    Block::Loop => "loop",
                    Block::Arm(_) if has_sibling(&beats[index + 1..], &want, level) => "alt",
                    Block::Arm(_) => "opt",
                };
                self.line(
                    depth + level,
                    &format!("{keyword} {}", text(&block.label())),
                );
                open.push(block.clone());
            }
            self.beat(caller, beat, depth + open.len());
        }
        while open.pop().is_some() {
            self.line(depth + open.len(), "end");
        }
    }

    fn beat(&mut self, caller: &str, beat: &Beat, depth: usize) {
        // A call nothing resolved has nowhere to go: the caller makes it.
        let lane = if beat.unit.is_some() {
            self.diagram.lane_of(beat)
        } else {
            caller.to_owned()
        };
        let arrow = match beat.mode {
            Mode::Spawn | Mode::Register => "-)",
            Mode::Call | Mode::Unknown => "->>",
        };
        let label = format!("{} {}{}", beat.number, beat.short, remarks(beat));
        self.line(depth, &format!("{caller}{arrow}{lane}: {}", text(&label)));
        if !beat.dropped.is_empty() {
            let gone = format!("no longer calls {}", beat.dropped.join(", "));
            self.line(depth, &format!("Note right of {lane}: {}", text(&gone)));
        }
        if !beat.calls.is_empty() {
            self.line(depth, &format!("activate {lane}"));
            self.calls(&lane, &beat.calls, depth);
            self.line(depth, &format!("deactivate {lane}"));
        }
        self.endings(&lane, Some(caller), beat, depth);
    }

    /// Where a call can fail, as crossed arrows back to its caller; on its own lane
    /// when nothing called it.
    fn endings(&mut self, lane: &str, caller: Option<&str>, beat: &Beat, depth: usize) {
        let to = caller.unwrap_or(lane);
        let failures = beat.failures();
        if !failures.is_empty() {
            self.line(depth, "opt can fail");
            for ending in failures {
                let label = match &ending.origin {
                    Some(origin) if *origin != beat.name => {
                        format!("{} in {origin}", failure(ending))
                    }
                    _ => failure(ending),
                };
                self.line(depth + 1, &format!("{lane}--x{to}: {}", text(&label)));
            }
            self.line(depth, "end");
        }
        if let Some(ending) = &beat.ends {
            self.line(depth, &format!("{lane}--x{to}: {}", text(&failure(ending))));
        }
    }
}

/// Whether a later call, before the block at `level` closes, takes another arm of its
/// branch, so the block is an `alt` rather than an `opt`.
fn has_sibling(rest: &[Beat], want: &[Block], level: usize) -> bool {
    let Some(block) = want.get(level) else {
        return false;
    };
    for beat in rest {
        let theirs = blocks(beat);
        if theirs.get(..level) != want.get(..level) {
            return false;
        }
        match theirs.get(level) {
            Some(other) if other == block => {}
            Some(other) => return block.sibling(other),
            None => return false,
        }
    }
    false
}

/// ` (new, +2 calls)`: what the message alone cannot show.
fn remarks(beat: &Beat) -> String {
    let mut remarks: Vec<String> = beat
        .change
        .map(change_word)
        .map(str::to_owned)
        .into_iter()
        .collect();
    match beat.mode {
        Mode::Spawn => remarks.push("in the background".to_owned()),
        Mode::Register => remarks.push("runs later".to_owned()),
        Mode::Call | Mode::Unknown => {}
    }
    if beat.stopped == Some(StopReason::Recursion) {
        remarks.push("recursive".to_owned());
    }
    if beat.guessed {
        remarks.push("guessed".to_owned());
    }
    match beat.folded {
        0 => {}
        1 => remarks.push("+1 call".to_owned()),
        n => remarks.push(format!("+{n} calls")),
    }
    if remarks.is_empty() {
        String::new()
    } else {
        format!(" ({})", remarks.join(", "))
    }
}

/// `✗ unknown sku`, or `✗ panic: overflow`.
fn failure(ending: &Ending) -> String {
    match ending.kind {
        ExitKind::Panic => format!("✗ panic: {}", tidy(&ending.label)),
        ExitKind::Error | ExitKind::Return | ExitKind::Unknown => {
            format!("✗ {}", tidy(&ending.label))
        }
    }
}

/// Text Mermaid shows as written: on one line, cut to [`MAX_LABEL`], and with the characters it
/// would read as syntax (`;` ends a statement, `#` starts an entity, `%%` a comment,
/// `<` and `>` tags) written as entities.
fn text(raw: &str) -> String {
    let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let long = line.chars().count() > MAX_LABEL;
    let mut out = String::with_capacity(line.len());
    for c in line.chars().take(MAX_LABEL - usize::from(long)) {
        match c {
            '#' | ';' | '%' | '<' | '>' | '&' => {
                let _ = write!(out, "#{};", u32::from(c));
            }
            c => out.push(c),
        }
    }
    if long {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_syntax_characters_as_entities() {
        assert_eq!(text("a; b # c"), "a#59; b #35; c");
        assert_eq!(text("x >= 10 && y < 2"), "x #62;= 10 #38;#38; y #60; 2");
        assert_eq!(text("100%%"), "100#37;#37;");
        assert_eq!(text(&"x".repeat(80)).chars().count(), MAX_LABEL);
    }

    #[test]
    fn participant_ids_avoid_keywords_and_each_other() {
        let mut diagram = Diagram::default();
        let beat = |home: &str| Beat {
            number: String::new(),
            unit: None,
            name: String::new(),
            short: String::new(),
            home: home.to_owned(),
            change: None,
            dropped: Vec::new(),
            conditions: Vec::new(),
            repeats: false,
            mode: Mode::Call,
            stopped: None,
            guessed: false,
            fails: Vec::new(),
            ends: None,
            folded: 0,
            calls: Vec::new(),
        };
        assert_eq!(
            diagram.lane_of(&beat("shop.services.orders")),
            "shop_services_orders"
        );
        assert_eq!(diagram.lane_of(&beat("end")), "end_2");
        assert_eq!(
            diagram.lane_of(&beat("shop/services/orders")),
            "shop_services_orders_2"
        );
        assert_eq!(diagram.lane_of(&beat("2fa")), "m2fa");
        assert_eq!(
            diagram.lane_of(&beat("shop.services.orders")),
            "shop_services_orders"
        );
    }
}
