//! How much of the journey shows. A journey opens with only what its root calls
//! directly; more opens a level at a time, or under one call by hand. A closure is
//! part of the function that defines it: it takes no level of its own, and the calls
//! made inside it count as its caller's.
//!
//! Every call has a number, `2` for the root's second call and `2.1` for that call's
//! first, which the panel and the graph both show, so a bar and its node can be
//! matched at a glance.

use std::collections::{HashMap, HashSet};

use codeviz_core::{Journey, Span, SpanId};
use codeviz_layout::CodebaseMap;

use crate::{
    explorer::{Confidence, Track, is_conditional, is_handoff},
    nucleus::Trail,
};

/// How much of the journey shows: calls this many levels below the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Disclose(pub(crate) u32);

impl Default for Disclose {
    fn default() -> Self {
        Self(1)
    }
}

/// What a disclosure shows of one journey.
#[derive(Debug, Default)]
pub(crate) struct Disclosed {
    /// Every call's number from the root, as the panel numbers it, shown or not; the
    /// root's is empty.
    numbers: HashMap<SpanId, String>,
    /// The calls each span makes, closures flattened, shown or not.
    calls: HashMap<SpanId, Vec<SpanId>>,
    shown: HashSet<SpanId>,
    /// How many levels below the root each shown call is: the calls numbered.
    levels: HashMap<SpanId, u32>,
    /// The nodes shown, each with the numbers of its spans.
    tiles: HashMap<usize, Vec<String>>,
    /// The nodes the journey runs at any depth, each with the numbers of its spans,
    /// shown or not.
    every: HashMap<usize, Vec<String>>,
    /// Shown spans where another outcome splits off: where the failure happens, or the
    /// nearest shown call it happens under.
    fails: HashSet<SpanId>,
    /// Nodes that can fail, nodes every shown run of which is conditional, and nodes
    /// every shown call to which is only guessed or handed off.
    failing_tiles: HashSet<usize>,
    sometimes_tiles: HashSet<usize>,
    guessed_tiles: HashSet<usize>,
    /// For a node every shown run of which is conditional, when it runs, where every
    /// run says the same.
    whens: HashMap<usize, Option<String>>,
    /// The journey as the nucleus draws it.
    trail: Trail,
    /// Shown calls the graph draws nowhere.
    unseen: HashSet<SpanId>,
}

impl Disclosed {
    /// Works out what `disclose` shows of `track`'s journey, with the spans in
    /// `expanded` opened a level further by hand.
    pub(crate) fn of(
        track: &Track,
        map: &CodebaseMap,
        disclose: Disclose,
        expanded: &HashSet<SpanId>,
    ) -> Self {
        let Disclose(depth) = disclose;
        let mut disclosed = Self::default();
        let Some(journey) = track.journey() else {
            return disclosed;
        };
        let tile = |span: SpanId| track.tile(span, map);
        // A call only guessed among several functions is drawn to those of them the
        // journey runs anyway.
        let ends = |span: SpanId| track.ends(span, map);
        let root = journey.root();
        // Every call, numbered from the root, and the closures each runs.
        let mut inside: HashMap<SpanId, Vec<SpanId>> = HashMap::new();
        disclosed.numbers.insert(root, String::new());
        let mut stack = vec![root];
        while let Some(span) = stack.pop() {
            let number = disclosed.numbers.get(&span).cloned().unwrap_or_default();
            let (children, closures) = calls_of(journey, &tile, span);
            for (index, &child) in children.iter().enumerate() {
                let child_number = if number.is_empty() {
                    format!("{}", index + 1)
                } else {
                    format!("{number}.{}", index + 1)
                };
                disclosed.numbers.insert(child, child_number);
                stack.push(child);
            }
            disclosed.calls.insert(span, children);
            inside.insert(span, closures);
        }
        // What shows: down to the depth from the root, and a level more under each
        // span opened by hand.
        disclosed.shown.insert(root);
        disclosed.levels.insert(root, 0);
        let mut stack = vec![root];
        while let Some(span) = stack.pop() {
            let level = disclosed.levels.get(&span).copied().unwrap_or_default();
            // The closure shows with its caller.
            disclosed
                .shown
                .extend(inside.get(&span).into_iter().flatten().copied());
            if level >= depth && !expanded.contains(&span) {
                continue;
            }
            let children = disclosed.children(span).to_vec();
            for child in children {
                disclosed.shown.insert(child);
                disclosed.levels.insert(child, level + 1);
                stack.push(child);
            }
        }
        disclosed.fails = track
            .divergences()
            .iter()
            .filter_map(|&divergence| {
                std::iter::successors(Some(divergence), |&s| journey.span(s)?.parent)
                    .find(|s| disclosed.levels.contains_key(s))
            })
            .collect();
        disclosed.mark_tiles(journey, &ends, root);
        let by_tile = |spans: &mut dyn Iterator<Item = (SpanId, &str)>| {
            let mut tiles: HashMap<usize, Vec<String>> = HashMap::new();
            for (span, number) in spans {
                for tile in ends(span) {
                    let list = tiles.entry(tile).or_default();
                    if !number.is_empty() {
                        list.push(number.to_owned());
                    }
                }
            }
            for numbers in tiles.values_mut() {
                numbers.sort_by_key(|n| number_order(n));
            }
            tiles
        };
        let tiles = by_tile(&mut disclosed.numbered());
        let every = by_tile(&mut disclosed.numbers.iter().map(|(&s, n)| (s, n.as_str())));
        let unseen = disclosed
            .numbered()
            .filter(|&(span, _)| ends(span).is_empty())
            .map(|(span, _)| span)
            .collect();
        (disclosed.tiles, disclosed.every, disclosed.unseen) = (tiles, every, unseen);
        disclosed.trail = disclosed.trace(
            (&tile, &ends),
            &|span| journey.span(span).is_some_and(guessed),
            root,
        );
        disclosed
    }

    /// Marks each node shown that can fail, that runs only sometimes every time it
    /// shows (and when), and that every shown call to is only guessed.
    fn mark_tiles(
        &mut self,
        journey: &Journey,
        ends: &impl Fn(SpanId) -> Vec<usize>,
        root: SpanId,
    ) {
        let mut always = HashSet::new();
        let mut sure = HashSet::new();
        for (&span, tile) in self
            .levels
            .keys()
            .flat_map(|span| ends(*span).into_iter().map(move |tile| (span, tile)))
        {
            if span != root {
                match journey.span(span).filter(|s| guessed(s)) {
                    Some(_) => self.guessed_tiles.insert(tile),
                    None => sure.insert(tile),
                };
            }
            if self.fails.contains(&span) {
                self.failing_tiles.insert(tile);
            }
            if let Some(span) = journey.span(span).filter(|s| is_conditional(s)) {
                self.sometimes_tiles.insert(tile);
                let said = when(span);
                self.whens
                    .entry(tile)
                    .and_modify(|before| {
                        if *before != said {
                            *before = None;
                        }
                    })
                    .or_insert(said);
            } else {
                always.insert(tile);
            }
        }
        self.sometimes_tiles.retain(|tile| !always.contains(tile));
        self.guessed_tiles.retain(|tile| !sure.contains(tile));
    }

    /// The journey from `start` as the nucleus draws it: each call shown, from the
    /// node it runs in (`tile`) to each it is drawn to (`ends`); and every function
    /// below the start at any depth and every call it makes, by node, what the
    /// layout is for, which a guess never changes.
    fn trace(
        &self,
        (tile, ends): (
            &impl Fn(SpanId) -> Option<usize>,
            &impl Fn(SpanId) -> Vec<usize>,
        ),
        guessed: &impl Fn(SpanId) -> bool,
        start: SpanId,
    ) -> Trail {
        let mut trail = Trail::default();
        // Everything below the start at any depth, each at the fewest calls from it, so
        // the modules round the start are laid out once for every depth.
        let mut wave = vec![start];
        let mut seen = HashSet::from([start]);
        for level in 1.. {
            let mut next = Vec::new();
            for span in wave {
                for &child in self.children(span) {
                    if seen.insert(child) {
                        if let Some(to) = tile(child) {
                            trail.whole.entry(to).or_insert(level);
                        }
                        next.push(child);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            wave = next;
        }
        // Every function below the start in the order the journey first runs it, and
        // every call it makes, at any depth.
        let mut stack = vec![start];
        while let Some(span) = stack.pop() {
            if span != start
                && let Some(to) = tile(span)
                && !trail.order.contains(&to)
            {
                trail.order.push(to);
            }
            if let Some(from) = tile(span) {
                for &child in self.children(span) {
                    if let Some(to) = tile(child)
                        && from != to
                        && !trail.reach.contains(&(from, to))
                    {
                        trail.reach.push((from, to));
                    }
                }
            }
            stack.extend(self.children(span).iter().rev());
        }
        // Down from the start, in the journey's order, each call shown.
        let mut stack = vec![(start, 0)];
        while let Some((span, level)) = stack.pop() {
            let Some(from) = tile(span) else {
                continue;
            };
            let level = level + 1;
            let open: Vec<SpanId> = self
                .children(span)
                .iter()
                .copied()
                .filter(|&child| self.shows(child))
                .collect();
            for &child in &open {
                for to in ends(child) {
                    trail.add(from, to, level, guessed(child));
                }
            }
            stack.extend(open.into_iter().rev().map(|child| (child, level)));
        }
        trail
    }

    /// The journey as the nucleus draws it.
    pub(crate) fn trail(&self) -> &Trail {
        &self.trail
    }

    /// How many levels below the root its deepest call is, shown or not: as deep as
    /// `.` goes.
    pub(crate) fn deepest(&self) -> u32 {
        let levels = self
            .numbers
            .values()
            .filter(|n| !n.is_empty())
            .map(|n| n.matches('.').count() + 1)
            .max()
            .unwrap_or(0);
        u32::try_from(levels).unwrap_or(u32::MAX)
    }

    /// Whether a span shows.
    pub(crate) fn shows(&self, span: SpanId) -> bool {
        self.shown.contains(&span)
    }

    /// A shown call's number as its bar says it: none for one the graph draws
    /// nowhere, a call only guessed among functions the journey runs nowhere else, or
    /// one to nothing on the map, so the panel numbers only what the graph does.
    pub(crate) fn bar_number(&self, span: SpanId) -> Option<&str> {
        self.number(span).filter(|_| !self.unseen.contains(&span))
    }

    /// A shown call's number; empty for the root.
    pub(crate) fn number(&self, span: SpanId) -> Option<&str> {
        self.levels
            .contains_key(&span)
            .then(|| self.numbers.get(&span).map(String::as_str))
            .flatten()
    }

    /// Every shown call with its number, the root's (empty) included, in no order.
    pub(crate) fn numbered(&self) -> impl Iterator<Item = (SpanId, &str)> {
        self.levels
            .keys()
            .filter_map(|&span| Some((span, self.numbers.get(&span)?.as_str())))
    }

    /// Whether the function a call runs calls itself straight back.
    /// Whether a shown span can fail: where another outcome splits off from it, or from
    /// something folded under it.
    pub(crate) fn fails(&self, span: SpanId) -> bool {
        self.fails.contains(&span)
    }

    /// Whether a shown node can fail.
    pub(crate) fn tile_fails(&self, tile: usize) -> bool {
        self.failing_tiles.contains(&tile)
    }

    /// Whether a shown node runs only under a condition or in a loop, every time the
    /// journey reaches it.
    pub(crate) fn tile_sometimes(&self, tile: usize) -> bool {
        self.sometimes_tiles.contains(&tile)
    }

    /// Whether every shown call to a node is only guessed, or handed off to run later:
    /// drawn faint, as its bars are.
    pub(crate) fn tile_guessed(&self, tile: usize) -> bool {
        self.guessed_tiles.contains(&tile)
    }

    /// When a node every shown run of which is conditional runs, where its runs agree.
    pub(crate) fn tile_when(&self, tile: usize) -> Option<&str> {
        self.whens
            .get(&tile)
            .filter(|_| self.tile_sometimes(tile))?
            .as_deref()
    }

    /// The calls a span makes, closures flattened, in order, shown or not.
    pub(crate) fn children(&self, span: SpanId) -> &[SpanId] {
        self.calls.get(&span).map_or(&[], Vec::as_slice)
    }

    /// How many calls under a shown span are folded away.
    pub(crate) fn folded(&self, span: SpanId) -> usize {
        self.children(span)
            .iter()
            .filter(|&&c| !self.shows(c))
            .count()
    }

    /// The nodes shown, each with the numbers of the calls that run it.
    pub(crate) fn tiles(&self) -> &HashMap<usize, Vec<String>> {
        &self.tiles
    }

    /// The nodes the journey runs at any depth, each with the numbers of the calls
    /// that run it, shown or not, in number order.
    pub(crate) fn every_tile(&self) -> &HashMap<usize, Vec<String>> {
        &self.every
    }

    /// Rows the shown spans take, one per level.
    pub(crate) fn rows(&self) -> usize {
        self.levels
            .values()
            .max()
            .map_or(0, |&level| level as usize + 1)
    }

    /// Where each shown span sits in the flame graph: its left edge and width, in
    /// points, and its row. Each takes what `need` says it needs across, at the least
    /// and to say all it says, and a caller spans its calls; the whole is made as
    /// near `width` as that allows: what each says cut short alike down to the least,
    /// spread out past all of it. Returns how wide the whole is.
    pub(crate) fn layout(
        &self,
        root: SpanId,
        width: f32,
        need: &dyn Fn(SpanId) -> (f32, f32),
    ) -> (HashMap<SpanId, (f32, f32, usize)>, f32) {
        fn calls(disclosed: &Disclosed, span: SpanId) -> impl Iterator<Item = SpanId> + '_ {
            disclosed
                .children(span)
                .iter()
                .copied()
                .filter(|&c| disclosed.shows(c))
        }
        // How wide each span is, `cut` of the way from saying all to the least.
        fn needed(
            disclosed: &Disclosed,
            span: SpanId,
            cut: f32,
            own: &HashMap<SpanId, (f32, f32)>,
            out: &mut HashMap<SpanId, f32>,
        ) -> f32 {
            let (least, most) = own.get(&span).copied().unwrap_or_default();
            let mine = most - cut * (most - least);
            let below: f32 = calls(disclosed, span)
                .map(|c| needed(disclosed, c, cut, own, out))
                .sum();
            let wide = mine.max(below);
            out.insert(span, wide);
            wide
        }
        fn place(
            disclosed: &Disclosed,
            (span, left, width, row): (SpanId, f32, f32, usize),
            wide: &HashMap<SpanId, f32>,
            out: &mut HashMap<SpanId, (f32, f32, usize)>,
        ) {
            out.insert(span, (left, width, row));
            let below: f32 = calls(disclosed, span).map(|c| wide[&c]).sum();
            let mut at = left;
            for call in calls(disclosed, span) {
                let share = width * wide[&call] / below;
                place(disclosed, (call, at, share, row + 1), wide, out);
                at += share;
            }
        }
        let mut out = HashMap::new();
        if !self.shows(root) {
            return (out, 0.0);
        }
        let mut own = HashMap::new();
        let mut todo = vec![root];
        while let Some(span) = todo.pop() {
            own.insert(span, need(span));
            todo.extend(calls(self, span));
        }
        let whole = |cut: f32| needed(self, root, cut, &own, &mut HashMap::new());
        // Cut no more than it takes to fit, found by halving.
        let cut = if whole(0.0) <= width {
            0.0
        } else if whole(1.0) >= width {
            1.0
        } else {
            let (mut fits, mut over) = (1.0, 0.0);
            for _ in 0..16 {
                let mid = f32::midpoint(fits, over);
                if whole(mid) <= width {
                    fits = mid;
                } else {
                    over = mid;
                }
            }
            fits
        };
        let mut wide = HashMap::new();
        let across = needed(self, root, cut, &own, &mut wide).max(width);
        place(self, (root, 0.0, across, 0), &wide, &mut out);
        (out, across)
    }
}

/// Whether a call is only guessed, or handed off to run later: drawn faint, its bar,
/// its line and its dot.
pub(crate) fn guessed(span: &Span) -> bool {
    Confidence::of(span) == Confidence::Guessed || is_handoff(span)
}

/// When a call runs, if not every time its caller does, as the panel and the graph
/// both say it: under the branch nearest it (`if user.is_admin()`, `unless cached`,
/// `if Some(x)`), or in a loop.
pub(crate) fn when(span: &Span) -> Option<String> {
    match span.conditions.last() {
        Some(condition) => Some(match (condition.arm.as_str(), &condition.guard) {
            ("then", Some(guard)) => format!("if {guard}"),
            ("else", Some(guard)) => format!("unless {guard}"),
            (arm, _) => format!("if {arm}"),
        }),
        None if span.repeats => Some("in a loop".to_owned()),
        None => None,
    }
}

/// The calls a span makes, in order, and the closures it runs: calls made from inside
/// a closure count as the caller's own.
pub(crate) fn calls_of(
    journey: &Journey,
    tile: &impl Fn(SpanId) -> Option<usize>,
    span: SpanId,
) -> (Vec<SpanId>, Vec<SpanId>) {
    let (mut children, mut inside) = (Vec::new(), Vec::new());
    let mut pending: Vec<SpanId> = journey
        .span(span)
        .map_or_else(Vec::new, |s| s.children.iter().rev().copied().collect());
    while let Some(child) = pending.pop() {
        if is_closure(journey, tile, child) {
            inside.push(child);
            if let Some(s) = journey.span(child) {
                pending.extend(s.children.iter().rev().copied());
            }
        } else {
            children.push(child);
        }
    }
    (children, inside)
}

/// Whether a span runs a closure: it runs the same function as the span that calls
/// it, as a closure is drawn as part of the function around it.
pub(crate) fn is_closure(
    journey: &Journey,
    tile: &impl Fn(SpanId) -> Option<usize>,
    span: SpanId,
) -> bool {
    tile(span).is_some() && journey.span(span).and_then(|s| s.parent).and_then(tile) == tile(span)
}

/// A journey number as its parts, nearest the root first, so `2` sorts before `1.1`
/// and `1.10` after `1.9`.
pub(crate) fn number_order(number: &str) -> (usize, Vec<u32>) {
    let parts: Vec<u32> = number.split('.').filter_map(|p| p.parse().ok()).collect();
    (parts.len(), parts)
}
