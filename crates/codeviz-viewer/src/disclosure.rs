//! How much of the journey shows. A journey opens with only what its root calls
//! directly; more opens a level at a time, or under one call by hand. A closure is
//! part of the function that defines it: it takes no level of its own, and the calls
//! made inside it count as its caller's.
//!
//! Every call has a number, `2` for the root's second call and `2.1` for that call's
//! first, which the panel and the graph both show, so a bar and its node can be
//! matched at a glance.

use std::collections::{HashMap, HashSet};

use codeviz_core::{Journey, Span, SpanId, StopReason};
use codeviz_layout::CodebaseMap;

use crate::{
    explorer::{Track, is_conditional},
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
    /// Shown spans where another outcome splits off: where the failure happens, or the
    /// nearest shown call it happens under.
    fails: HashSet<SpanId>,
    /// Nodes that can fail, and nodes every shown run of which is conditional.
    failing_tiles: HashSet<usize>,
    sometimes_tiles: HashSet<usize>,
    /// For a node every shown run of which is conditional, when it runs, where every
    /// run says the same.
    whens: HashMap<usize, Option<String>>,
    /// Calls whose function calls itself straight back, which draws as no call of
    /// its own: it runs on the same node.
    calls_itself: HashSet<SpanId>,
    /// The journey as the nucleus draws it.
    trail: Trail,
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
            // A call back to itself is drawn on its own node, as a closure is.
            if closures.iter().any(|&c| {
                journey
                    .span(c)
                    .is_some_and(|s| s.stopped == Some(StopReason::Recursion))
            }) {
                disclosed.calls_itself.insert(span);
            }
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
        let mut always = HashSet::new();
        for &span in disclosed.levels.keys() {
            let Some(tile) = tile(span) else {
                continue;
            };
            if disclosed.fails.contains(&span) {
                disclosed.failing_tiles.insert(tile);
            }
            if let Some(span) = journey.span(span).filter(|s| is_conditional(s)) {
                disclosed.sometimes_tiles.insert(tile);
                let said = when(span);
                disclosed
                    .whens
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
        disclosed
            .sometimes_tiles
            .retain(|tile| !always.contains(tile));
        let mut tiles: HashMap<usize, Vec<String>> = HashMap::new();
        for (span, number) in disclosed.numbered() {
            if let Some(tile) = tile(span) {
                let list = tiles.entry(tile).or_default();
                if !number.is_empty() {
                    list.push(number.to_owned());
                }
            }
        }
        for numbers in tiles.values_mut() {
            numbers.sort_by_key(|n| number_order(n));
        }
        disclosed.tiles = tiles;
        disclosed.trail = disclosed.trace(&tile, root);
        disclosed
    }

    /// The journey from `start` as the nucleus draws it: each call shown, and every
    /// function below the start at any depth, by node.
    fn trace(&self, tile: &impl Fn(SpanId) -> Option<usize>, start: SpanId) -> Trail {
        let mut trail = Trail::default();
        // Everything below the start at any depth, each at the fewest calls from it, so
        // the expanded nucleus is laid out once for every depth.
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
        // Every function below the start in the order the journey first runs it.
        let mut stack = vec![start];
        while let Some(span) = stack.pop() {
            if span != start
                && let Some(to) = tile(span)
                && !trail.order.contains(&to)
            {
                trail.order.push(to);
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
                if let Some(to) = tile(child) {
                    trail.add(from, to, level);
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
    pub(crate) fn calls_itself(&self, span: SpanId) -> bool {
        self.calls_itself.contains(&span)
    }

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

    /// Rows the shown spans take, one per level.
    pub(crate) fn rows(&self) -> usize {
        self.levels
            .values()
            .max()
            .map_or(0, |&level| level as usize + 1)
    }

    /// Where each shown span sits in the flame graph: its left edge and width, in
    /// slots, and its row. Each call without shown calls of its own takes one slot,
    /// and a caller spans its calls, so every name has room whatever it costs in time.
    pub(crate) fn layout(&self, root: SpanId) -> (HashMap<SpanId, (f32, f32, usize)>, f32) {
        fn place(
            disclosed: &Disclosed,
            span: SpanId,
            left: f32,
            row: usize,
            out: &mut HashMap<SpanId, (f32, f32, usize)>,
        ) -> f32 {
            let mut width = 0.0;
            for &child in disclosed.children(span) {
                if disclosed.shows(child) {
                    width += place(disclosed, child, left + width, row + 1, out);
                }
            }
            let width = width.max(1.0);
            out.insert(span, (left, width, row));
            width
        }
        let mut out = HashMap::new();
        let width = if self.shows(root) {
            place(self, root, 0.0, 0, &mut out)
        } else {
            0.0
        };
        (out, width)
    }
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

#[cfg(test)]
mod tests {
    use codeviz_core::System;
    use codeviz_ir::Document;

    use crate::explorer::tests::opened;

    #[test]
    fn a_function_that_calls_itself_says_so_on_its_own_bar() {
        // walk calls itself, and visit, which calls walk.
        let json = r#"{ "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" }, "units": [
            { "id": "x:walk", "name": "walk", "kind": "fn", "role": "invocable", "calls": [
                { "key": "w#0", "text": "walk", "targets": [ { "unit": "x:walk", "certainty": "exact" } ] },
                { "key": "v#0", "text": "visit", "targets": [ { "unit": "x:visit", "certainty": "exact" } ] } ] },
            { "id": "x:visit", "name": "visit", "kind": "fn", "role": "invocable", "calls": [
                { "key": "w#0", "text": "walk", "targets": [ { "unit": "x:walk", "certainty": "exact" } ] } ] } ] }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let explorer = opened(system, "x:walk");
        let journey = explorer.journey().unwrap();
        let disclosed = explorer.disclosure();
        let root = journey.root();
        assert!(
            disclosed.calls_itself(root),
            "walk calls walk straight back"
        );
        assert_eq!(
            disclosed.children(root).len(),
            1,
            "with no bar for it: visit only"
        );
        let visit = disclosed.children(root)[0];
        assert!(
            !disclosed.calls_itself(visit),
            "visit calls walk, not itself"
        );
    }
}
