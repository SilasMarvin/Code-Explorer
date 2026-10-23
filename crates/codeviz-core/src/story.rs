//! A journey told the way an engineer would explain it to a teammate: the calls shown,
//! numbered the way the viewer numbers them, each with what is worth saying about it
//! (when it runs, where it can fail, what a diff did to it), and the ways it can end.
//! [`Story::markdown`] and [`Story::mermaid`] write it out.
//!
//! A story tells only the calls it is given numbers for, so it shows as much of the
//! journey as the view it comes from: [`number_calls`] numbers them by depth, as the
//! viewer does, and the viewer passes its own numbers.

use std::collections::HashMap;

use codeviz_ir::{Certainty, ExitKind, Mode, UnitId};

use crate::{
    Change, Condition, Diff, Journey, Scenario, Span, SpanId, SpanTarget, StopReason, System,
    display_name,
};

/// A journey, told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Story {
    /// The root's entry label, when it is a declared entry: `POST /users`.
    pub entry: Option<String>,
    /// The root, with the calls shown under it.
    pub root: Beat,
    /// The outcome told; `None` tells every path at once.
    pub told: Option<Ending>,
    /// Every way the root can end, normal completion first.
    pub endings: Vec<Ending>,
}

/// One shown call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beat {
    /// `2.1`, as the viewer numbers it; empty for the root.
    pub number: String,
    /// The function called (a closure's, the function that defines it), if the call
    /// resolved to exactly one.
    pub unit: Option<UnitId>,
    /// `orders.place`: the function with what it belongs to.
    pub name: String,
    /// The function alone: `place`.
    pub short: String,
    /// What the function belongs to (its module, class or type): `orders`.
    pub home: String,
    pub change: Option<Change>,
    /// Calls the function made before a diff and no longer makes.
    pub dropped: Vec<String>,
    /// Where to read the function's source, on the web.
    pub link: Option<String>,
    /// Branch arms that must be taken in the caller for this call to happen.
    pub conditions: Vec<Condition>,
    /// The call sits in a loop.
    pub repeats: bool,
    pub mode: Mode,
    pub stopped: Option<StopReason>,
    /// The call was matched by name or among several candidates, not resolved.
    pub guessed: bool,
    /// Outcomes that split off here, or in a call folded under this one.
    pub fails: Vec<Ending>,
    /// The told outcome, when it happens here or in a call folded under this one.
    pub ends: Option<Ending>,
    /// Calls this one makes that are not shown.
    pub folded: usize,
    pub calls: Vec<Beat>,
}

impl Beat {
    /// The outcomes that split off at this call, an error a producer saw both raised
    /// and passed on (`NotFound`, `NotFound fails`) named once.
    #[must_use]
    pub fn failures(&self) -> Vec<&Ending> {
        self.fails
            .iter()
            .filter(|e| {
                e.label.strip_suffix(" fails").is_none_or(|raised| {
                    !self
                        .fails
                        .iter()
                        .any(|other| other.label == raised && other.origin == e.origin)
                })
            })
            .collect()
    }
}

/// A way a journey can end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ending {
    pub kind: ExitKind,
    /// `unknown sku`, or `.ok_or fails` when a call the producer could not follow
    /// passes an error on.
    pub label: String,
    /// The function it happens in (a closure's, the function that defines it), for an
    /// escape; `None` for normal completion.
    pub origin: Option<String>,
}

impl Story {
    /// Tells `journey` (unrolled from one of `scenarios`, or from none of them), with
    /// the calls in `numbers` shown. Outcomes other than the one told show where they
    /// split off, at the nearest shown call.
    #[must_use]
    pub fn new(
        system: &System,
        journey: &Journey,
        scenarios: &[Scenario],
        scenario: Option<usize>,
        numbers: &HashMap<SpanId, String>,
    ) -> Self {
        let endings: Vec<Ending> = scenarios.iter().map(|s| ending(system, s)).collect();
        let told = scenario.and_then(|i| endings.get(i)).cloned();
        let mut fails: HashMap<SpanId, Vec<Ending>> = HashMap::new();
        let mut ends: HashMap<SpanId, Ending> = HashMap::new();
        for (index, scenario_) in scenarios.iter().enumerate().skip(1) {
            let at =
                divergence(journey, scenario_).and_then(|span| shown_at(journey, numbers, span));
            let (Some(at), Some(end)) = (at, endings.get(index)) else {
                continue;
            };
            match scenario {
                Some(told) if told == index => {
                    ends.insert(at, end.clone());
                }
                Some(told) if told > 0 => {}
                _ => {
                    let here = fails.entry(at).or_default();
                    if !here.contains(end) {
                        here.push(end.clone());
                    }
                }
            }
        }
        let teller = Teller {
            system,
            journey,
            numbers,
            fails,
            ends,
        };
        let root = teller.beat(journey.root());
        let entry = match &root.unit {
            Some(unit) => system
                .entries()
                .iter()
                .find(|e| &e.unit == unit)
                .map(|e| e.label.clone()),
            None => None,
        };
        Self {
            entry,
            root,
            told,
            endings,
        }
    }

    /// Marks each call with how `diff` changed it, and with the calls it no longer
    /// makes, named as in `before`, the old version.
    #[must_use]
    pub fn with_diff(mut self, diff: &Diff, before: &System) -> Self {
        fn mark(beat: &mut Beat, diff: &Diff, before: &System) {
            if let Some(unit) = &beat.unit {
                beat.change = diff.change(unit);
                let mut dropped: Vec<String> = diff
                    .removed_calls
                    .iter()
                    .filter(|(from, _)| from == unit)
                    .map(|(_, to)| display_name(before, to))
                    .collect();
                dropped.sort();
                beat.dropped = dropped;
            }
            for call in &mut beat.calls {
                mark(call, diff, before);
            }
        }
        mark(&mut self.root, diff, before);
        self
    }

    /// Links each call, and the root, to its function's source, by unit.
    #[must_use]
    pub fn with_links(mut self, links: &HashMap<UnitId, String>) -> Self {
        fn link(beat: &mut Beat, links: &HashMap<UnitId, String>) {
            beat.link = beat.unit.as_ref().and_then(|u| links.get(u)).cloned();
            for call in &mut beat.calls {
                link(call, links);
            }
        }
        link(&mut self.root, links);
        self
    }

    /// The root and every shown call under it, in order, depth first.
    pub fn all_beats(&self) -> impl Iterator<Item = &Beat> {
        std::iter::once(&self.root).chain(self.beats())
    }

    /// Every shown call under the root, in order, depth first.
    pub fn beats(&self) -> impl Iterator<Item = &Beat> {
        let mut stack: Vec<&Beat> = self.root.calls.iter().rev().collect();
        std::iter::from_fn(move || {
            let beat = stack.pop()?;
            stack.extend(beat.calls.iter().rev());
            Some(beat)
        })
    }
}

/// Where `scenario` splits off from `journey`: the first span running the deepest unit
/// on its chain that the journey runs. On the scenario's own journey, that is where it
/// ends.
#[must_use]
pub fn divergence(journey: &Journey, scenario: &Scenario) -> Option<SpanId> {
    scenario.chain.iter().rev().find_map(|site| {
        journey
            .spans()
            .find(|(_, s)| matches!(&s.target, SpanTarget::Unit(u) if *u == site.unit))
            .map(|(id, _)| id)
    })
}

/// Numbers the calls `depth` levels below the root, `1`, `2`, `2.1`, the way the
/// viewer does. A closure is part of the function that defines it: it takes no number,
/// and the calls made inside it count as its caller's.
#[must_use]
pub fn number_calls(system: &System, journey: &Journey, depth: u32) -> HashMap<SpanId, String> {
    let mut numbers = HashMap::new();
    if journey.is_empty() {
        return numbers;
    }
    let mut stack = vec![(journey.root(), String::new(), 0)];
    while let Some((span, number, level)) = stack.pop() {
        numbers.insert(span, number.clone());
        if level >= depth {
            continue;
        }
        for (index, call) in calls_of(system, journey, span).into_iter().enumerate() {
            let number = if number.is_empty() {
                format!("{}", index + 1)
            } else {
                format!("{number}.{}", index + 1)
            };
            stack.push((call, number, level + 1));
        }
    }
    numbers
}

/// The most levels [`number_calls`] can show without numbering more than `calls`
/// calls; at least one, and no more than the journey has.
#[must_use]
pub fn depth_to_fit(system: &System, journey: &Journey, calls: usize) -> u32 {
    let mut depth = 1;
    let mut shown = number_calls(system, journey, depth).len();
    loop {
        let deeper = number_calls(system, journey, depth + 1).len();
        // The root is numbered too, with no number.
        if deeper == shown || deeper - 1 > calls {
            return depth;
        }
        depth += 1;
        shown = deeper;
    }
}

/// The calls a span makes, in order, with those made inside its closures as its own.
#[must_use]
pub fn calls_of(system: &System, journey: &Journey, span: SpanId) -> Vec<SpanId> {
    let function = |span: SpanId| -> Option<&UnitId> {
        match &journey.span(span)?.target {
            SpanTarget::Unit(unit)
                if system
                    .unit(unit)
                    .is_some_and(|u| u.role == codeviz_ir::Role::Invocable) =>
            {
                Some(system.owner(unit))
            }
            _ => None,
        }
    };
    let mut calls = Vec::new();
    let mut pending: Vec<SpanId> = journey
        .span(span)
        .map_or_else(Vec::new, |s| s.children.iter().rev().copied().collect());
    while let Some(child) = pending.pop() {
        let parent = journey.span(child).and_then(|s| s.parent);
        let inside = function(child).is_some() && parent.and_then(function) == function(child);
        if inside {
            if let Some(s) = journey.span(child) {
                pending.extend(s.children.iter().rev().copied());
            }
        } else {
            calls.push(child);
        }
    }
    calls
}

/// The nearest span at or above `span` that is shown.
fn shown_at(journey: &Journey, numbers: &HashMap<SpanId, String>, span: SpanId) -> Option<SpanId> {
    std::iter::successors(Some(span), |&s| journey.span(s)?.parent)
        .find(|s| numbers.contains_key(s))
}

fn ending(system: &System, scenario: &Scenario) -> Ending {
    Ending {
        kind: scenario.outcome.kind,
        label: scenario.outcome.label.clone(),
        origin: scenario
            .chain
            .last()
            .map(|site| display_name(system, system.owner(&site.unit))),
    }
}

struct Teller<'a> {
    system: &'a System,
    journey: &'a Journey,
    numbers: &'a HashMap<SpanId, String>,
    fails: HashMap<SpanId, Vec<Ending>>,
    ends: HashMap<SpanId, Ending>,
}

impl Teller<'_> {
    /// The span and the closures it was called from inside, innermost first, up to
    /// the shown call that made it.
    fn through(&self, id: SpanId) -> impl Iterator<Item = &Span> {
        let parent = |s: SpanId| self.journey.span(s)?.parent;
        let closures = std::iter::successors(parent(id), move |&s| parent(s))
            .take_while(|s| !self.numbers.contains_key(s));
        std::iter::once(id)
            .chain(closures)
            .filter_map(|s| self.journey.span(s))
    }

    /// The arms a call needs: its own, after those of the closures it is made in.
    fn conditions(&self, id: SpanId) -> Vec<Condition> {
        let spans: Vec<&Span> = self.through(id).collect();
        spans
            .into_iter()
            .rev()
            .flat_map(|s| s.conditions.iter().cloned())
            .collect()
    }

    fn beat(&self, id: SpanId) -> Beat {
        let span = self.journey.span(id);
        let unit = span.and_then(|s| match &s.target {
            SpanTarget::Unit(unit) => Some(self.system.owner(unit).clone()),
            _ => None,
        });
        let (name, short, home) = match span.map(|s| &s.target) {
            Some(SpanTarget::Unit(unit)) => {
                let function = self.system.owner(unit);
                let short = self
                    .system
                    .unit(function)
                    .map_or_else(|| function.to_string(), |u| u.name.clone());
                let home = self
                    .system
                    .unit(function)
                    .and_then(|u| u.parent.as_ref())
                    .and_then(|p| self.system.unit(p))
                    .map_or_else(String::new, |p| p.name.clone());
                (display_name(self.system, function), short, home)
            }
            Some(SpanTarget::Ambiguous(ids)) => {
                let text = span.and_then(|s| s.text.clone()).unwrap_or_default();
                let names: Vec<String> =
                    ids.iter().map(|id| display_name(self.system, id)).collect();
                (
                    format!("{text} (one of {})", names.join(", ")),
                    text,
                    String::new(),
                )
            }
            Some(SpanTarget::Unresolved(text)) => (text.clone(), text.clone(), String::new()),
            None => (String::new(), String::new(), String::new()),
        };
        let all = calls_of(self.system, self.journey, id);
        let mut shown: Vec<(Vec<u32>, SpanId)> = all
            .iter()
            .filter_map(|&call| {
                let number = self.numbers.get(&call)?;
                Some((
                    number.split('.').filter_map(|n| n.parse().ok()).collect(),
                    call,
                ))
            })
            .collect();
        shown.sort();
        Beat {
            number: self.numbers.get(&id).cloned().unwrap_or_default(),
            unit,
            name,
            short,
            home,
            change: None,
            dropped: Vec::new(),
            link: None,
            conditions: self.conditions(id),
            repeats: self.through(id).any(|s| s.repeats),
            mode: span.map_or(Mode::Call, |s| s.mode),
            stopped: span.and_then(|s| s.stopped),
            guessed: span.is_some_and(|s| {
                matches!(s.target, SpanTarget::Ambiguous(_))
                    || s.certainty == Some(Certainty::Inferred)
            }),
            fails: self.fails.get(&id).cloned().unwrap_or_default(),
            ends: self.ends.get(&id).cloned(),
            folded: all.len() - shown.len(),
            calls: shown.into_iter().map(|(_, call)| self.beat(call)).collect(),
        }
    }
}
