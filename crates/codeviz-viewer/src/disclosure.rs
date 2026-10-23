//! How much of the active journey shows. A journey opens with only what its root calls
//! directly; more opens a level at a time, under one call by hand, or call by call as
//! the cursor moves through time. A closure is part of the function that defines it:
//! it takes no level of its own, and the calls made inside it count as its caller's.
//!
//! Every disclosed call has a number, `2` for the root's second call and `2.1` for that
//! call's first, which the flame graph, the outline and the graph all show, so a bar
//! and its node can be matched at a glance.
//!
//! A call that does exactly what an earlier one does (the same function, making the
//! same calls all the way down) shows once: the repeat stays folded and points at the
//! first, "same as 1.1.3". And a level opens only as far as the flame graph has room
//! for: the calls that widen it least open first, and the rest stay folded.

use std::collections::{HashMap, HashSet};

use codeviz_core::SpanId;
use codeviz_layout::CodebaseMap;

use crate::explorer::{Track, is_conditional};

/// How much of the active journey shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Disclose {
    /// Calls this many levels below the root.
    Depth(u32),
    /// Everything entered up to the cursor, so stepping through time reveals the
    /// journey call by call.
    Time,
}

impl Default for Disclose {
    fn default() -> Self {
        Self::Depth(1)
    }
}

/// What a disclosure shows of one journey.
#[derive(Debug, Default)]
pub(crate) struct Disclosed {
    shown: HashSet<SpanId>,
    numbers: HashMap<SpanId, String>,
    /// The calls each shown span makes, closures flattened, shown or not.
    children: HashMap<SpanId, Vec<SpanId>>,
    levels: HashMap<SpanId, u32>,
    /// Shown calls that do what an earlier shown call does, each with that call.
    same_as: HashMap<SpanId, SpanId>,
    /// The other way: each call with its repeats, in order.
    repeats: HashMap<SpanId, Vec<SpanId>>,
    /// Calls a level would have opened but left folded for want of room.
    held: usize,
    /// The nodes shown, each with the numbers of its spans, shallowest first.
    tiles: HashMap<usize, Vec<String>>,
    /// Shown spans where another outcome splits off: where the failure happens, or the
    /// nearest shown call it happens under.
    fails: HashSet<SpanId>,
    /// Nodes that can fail, and nodes every shown run of which is conditional.
    failing_tiles: HashSet<usize>,
    sometimes_tiles: HashSet<usize>,
}

impl Disclosed {
    /// Works out what `disclose` shows of `track`'s journey, with the spans in
    /// `expanded` opened a level further by hand. By level, the flame graph grows no
    /// wider than `width` slots, except where opened by hand.
    #[expect(clippy::too_many_lines, reason = "one breadth-first pass")]
    pub(crate) fn of(
        track: &Track,
        map: &CodebaseMap,
        disclose: Disclose,
        expanded: &HashSet<SpanId>,
        width: usize,
    ) -> Self {
        let mut disclosed = Self::default();
        let Some(journey) = track.journey() else {
            return disclosed;
        };
        let tile = |span: SpanId| track.unit_id(span).and_then(|u| map.tile_of(u));
        let cursor = track.selected();
        let cursor_enter = cursor.and_then(|s| journey.span(s)).map_or(0, |s| s.enter);
        // Stepping through time walks into a repeat too, so what the cursor is in
        // never folds away.
        let walked: HashSet<SpanId> = match disclose {
            Disclose::Time => std::iter::successors(cursor, |&s| journey.span(s)?.parent).collect(),
            Disclose::Depth(_) => HashSet::new(),
        };
        let root = journey.root();
        disclosed.shown.insert(root);
        disclosed.numbers.insert(root, String::new());
        disclosed.levels.insert(root, 0);
        // The first shown call of each shape that makes calls, shallowest first, then
        // earliest.
        let mut first: HashMap<usize, SpanId> = HashMap::new();
        let mut slots = 1;
        let mut row = vec![root];
        for level in 0.. {
            if row.is_empty() {
                break;
            }
            for &span in &row {
                // Calls made from inside a closure count as the caller's own; the
                // closure shows with it.
                let mut children = Vec::new();
                let mut pending: Vec<SpanId> = journey
                    .span(span)
                    .map_or_else(Vec::new, |s| s.children.iter().rev().copied().collect());
                while let Some(child) = pending.pop() {
                    let inside = tile(child).is_some()
                        && journey.span(child).and_then(|s| s.parent).and_then(tile) == tile(child);
                    if inside {
                        disclosed.shown.insert(child);
                        if let Some(s) = journey.span(child) {
                            pending.extend(s.children.iter().rev().copied());
                        }
                    } else {
                        children.push(child);
                    }
                }
                if span != root
                    && !children.is_empty()
                    && !walked.contains(&span)
                    && let Some(shape) = track.shape(span)
                {
                    match first.get(&shape) {
                        Some(&original) => {
                            disclosed.same_as.insert(span, original);
                            disclosed.repeats.entry(original).or_default().push(span);
                        }
                        None => {
                            first.insert(shape, span);
                        }
                    }
                }
                disclosed.children.insert(span, children);
            }
            // Which calls open: by level, the narrowest first while there is room; by
            // time, each call once the cursor reaches it.
            let mut opening: Vec<SpanId> = row
                .iter()
                .copied()
                .filter(|span| !disclosed.same_as.contains_key(span))
                .collect();
            opening.sort_by_key(|&span| disclosed.children(span).len());
            let mut open: HashSet<SpanId> = HashSet::new();
            for span in opening {
                let calls = disclosed.children(span).len();
                if calls == 0 {
                    continue;
                }
                match disclose {
                    Disclose::Time => {
                        open.insert(span);
                    }
                    Disclose::Depth(depth) => {
                        let by_hand = expanded.contains(&span);
                        if !(level < depth || by_hand) {
                            continue;
                        }
                        // A chain widens nothing, so it opens even past the room that
                        // calls opened by hand took.
                        if span == root || by_hand || slots + calls - 1 <= width.max(slots) {
                            slots += calls - 1;
                            open.insert(span);
                        } else {
                            disclosed.held += 1;
                        }
                    }
                }
            }
            let mut next = Vec::new();
            for span in row {
                if !open.contains(&span) {
                    continue;
                }
                let number = disclosed.numbers.get(&span).cloned().unwrap_or_default();
                let children = disclosed.children.get(&span).cloned().unwrap_or_default();
                for (index, child) in children.into_iter().enumerate() {
                    let reached = journey.span(child).is_some_and(|s| s.enter <= cursor_enter);
                    if disclose == Disclose::Time && !reached {
                        continue;
                    }
                    let child_number = if number.is_empty() {
                        format!("{}", index + 1)
                    } else {
                        format!("{number}.{}", index + 1)
                    };
                    disclosed.shown.insert(child);
                    disclosed.numbers.insert(child, child_number);
                    disclosed.levels.insert(child, level + 1);
                    next.push(child);
                }
            }
            row = next;
        }
        disclosed.fails = track
            .divergences()
            .iter()
            .filter_map(|divergence| {
                std::iter::successors(Some(divergence.span), |&s| journey.span(s)?.parent)
                    .find(|s| disclosed.numbers.contains_key(s))
            })
            .collect();
        let mut always = HashSet::new();
        for &span in disclosed.numbers.keys() {
            let Some(tile) = tile(span) else {
                continue;
            };
            if disclosed.fails.contains(&span) {
                disclosed.failing_tiles.insert(tile);
            }
            if journey.span(span).is_some_and(is_conditional) {
                disclosed.sometimes_tiles.insert(tile);
            } else {
                always.insert(tile);
            }
        }
        disclosed
            .sometimes_tiles
            .retain(|tile| !always.contains(tile));
        // Shallowest first, then in order: a repeat points at the first of these.
        let mut numbered: Vec<(&SpanId, &String)> = disclosed.numbers.iter().collect();
        numbered.sort_by_key(|&(span, number)| (number.matches('.').count(), span));
        for (&span, number) in numbered {
            if let Some(tile) = tile(span) {
                let list = disclosed.tiles.entry(tile).or_default();
                if !number.is_empty() {
                    list.push(number.clone());
                }
            }
        }
        disclosed
    }

    /// Whether a span shows.
    pub(crate) fn shows(&self, span: SpanId) -> bool {
        self.shown.contains(&span)
    }

    /// A shown call's number; empty for the root.
    pub(crate) fn number(&self, span: SpanId) -> Option<&str> {
        self.numbers.get(&span).map(String::as_str)
    }

    /// Every shown call with its number, the root's (empty) included, in no order.
    pub(crate) fn numbered(&self) -> impl Iterator<Item = (SpanId, &str)> {
        self.numbers
            .iter()
            .map(|(&span, number)| (span, number.as_str()))
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

    /// The calls a shown span makes, closures flattened, in order.
    pub(crate) fn children(&self, span: SpanId) -> &[SpanId] {
        self.children.get(&span).map_or(&[], Vec::as_slice)
    }

    /// How many calls under a shown span are folded away; none under a repeat, whose
    /// calls show under the call it repeats.
    pub(crate) fn folded(&self, span: SpanId) -> usize {
        if self.same_as.contains_key(&span) {
            return 0;
        }
        self.children(span)
            .iter()
            .filter(|&&c| !self.shows(c))
            .count()
    }

    /// The earlier shown call a shown call repeats, doing exactly the same.
    pub(crate) fn same_as(&self, span: SpanId) -> Option<SpanId> {
        self.same_as.get(&span).copied()
    }

    /// The shown calls that repeat this one, in order.
    pub(crate) fn repeats(&self, span: SpanId) -> &[SpanId] {
        self.repeats.get(&span).map_or(&[], Vec::as_slice)
    }

    /// How many calls a level would have opened, but left folded for want of room.
    pub(crate) fn held(&self) -> usize {
        self.held
    }

    /// The nodes shown, each with the numbers of the calls that run it, shallowest
    /// first.
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

    /// Whether more than the root shows.
    pub(crate) fn opened(&self) -> bool {
        self.numbers.len() > 1
    }
}

#[cfg(test)]
mod tests {
    use codeviz_core::{JourneyOptions, System};
    use codeviz_ir::{Document, UnitId};

    use super::*;
    use crate::explorer::{Explorer, Step};

    /// An explorer on `x:main`, each unit named with the units it calls, in order.
    fn explorer(units: &[(&str, &[&str])]) -> Explorer {
        let units: Vec<String> = units
            .iter()
            .map(|(name, calls)| {
                let calls: Vec<String> = calls
                    .iter()
                    .enumerate()
                    .map(|(n, c)| {
                        format!(
                            r#"{{ "key": "{c}#{n}", "text": "{c}", "targets": [{{ "unit": "x:{c}", "certainty": "exact" }}] }}"#
                        )
                    })
                    .collect();
                format!(
                    r#"{{ "id": "x:{name}", "name": "{name}", "kind": "fn", "role": "invocable", "calls": [{}] }}"#,
                    calls.join(",")
                )
            })
            .collect();
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
            units.join(",")
        );
        let system = System::new([Document::from_json(&json).unwrap()]);
        Explorer::new(
            system,
            Some(UnitId::new("x:main")),
            JourneyOptions::default(),
        )
    }

    /// The shown span numbered `number`.
    fn at(explorer: &Explorer, number: &str) -> SpanId {
        explorer
            .disclosure()
            .numbered()
            .find(|&(_, n)| n == number)
            .map_or_else(|| panic!("{number} shows"), |(span, _)| span)
    }

    /// Every shown call as `number name`, in order, with `= number` on a repeat.
    fn shown(explorer: &Explorer) -> Vec<String> {
        let disclosed = explorer.disclosure();
        let mut calls: Vec<(Vec<u32>, String)> = disclosed
            .numbered()
            .filter(|(_, number)| !number.is_empty())
            .map(|(span, number)| {
                let mut line = format!("{number} {}", explorer.label(span));
                if let Some(original) = disclosed.same_as(span).and_then(|s| disclosed.number(s)) {
                    line = format!("{line} = {original}");
                }
                (
                    number.split('.').map(|n| n.parse().unwrap()).collect(),
                    line,
                )
            })
            .collect();
        calls.sort();
        calls.into_iter().map(|(_, line)| line).collect()
    }

    /// `main → (load → (read, parse), render → (load, paint), load)`.
    const REPEATS: &[(&str, &[&str])] = &[
        ("main", &["load", "render", "load"]),
        ("load", &["read", "parse"]),
        ("render", &["load", "paint"]),
        ("read", &[]),
        ("parse", &[]),
        ("paint", &[]),
    ];

    #[test]
    fn a_repeat_shows_once_and_points_at_the_first() {
        let mut explorer = explorer(REPEATS);
        assert_eq!(
            shown(&explorer),
            ["1 load", "2 render", "3 load = 1"],
            "the second load does what the first does"
        );
        let disclosed = explorer.disclosure();
        let (first, again) = (at(&explorer, "1"), at(&explorer, "3"));
        assert_eq!(
            disclosed.folded(first),
            2,
            "read and parse fold under the first"
        );
        assert_eq!(
            disclosed.folded(again),
            0,
            "and show there, not under the repeat"
        );

        explorer.set_disclose(Disclose::Depth(3));
        assert_eq!(
            shown(&explorer),
            [
                "1 load",
                "1.1 read",
                "1.2 parse",
                "2 render",
                "2.1 load = 1",
                "2.2 paint",
                "3 load = 1",
            ],
            "a repeat deeper down points up at the shallowest"
        );
        let first = at(&explorer, "1");
        let repeats: Vec<SpanId> = [at(&explorer, "3"), at(&explorer, "2.1")].into();
        assert_eq!(
            explorer.disclosure().repeats(first),
            repeats,
            "level by level"
        );
    }

    #[test]
    fn calls_that_make_no_calls_are_not_repeats() {
        let explorer = explorer(&[("main", &["read", "read"]), ("read", &[])]);
        assert_eq!(shown(&explorer), ["1 read", "2 read"]);
    }

    #[test]
    fn a_repeats_cursor_moves_to_the_first_and_back() {
        let mut explorer = explorer(REPEATS);
        let (first, again) = (at(&explorer, "1"), at(&explorer, "3"));
        explorer.select(Some(again));
        assert_eq!(explorer.jump_repeat(), Some(first));
        assert_eq!(
            explorer.jump_repeat(),
            Some(again),
            "back where it came from"
        );
        explorer.select(Some(at(&explorer, "2")));
        assert_eq!(explorer.jump_repeat(), None, "render is no repeat");
    }

    #[test]
    fn stepping_through_time_walks_into_a_repeat() {
        let mut explorer = explorer(REPEATS);
        explorer.set_disclose(Disclose::Time);
        // main, load, read, parse, render, load, read: inside the second load.
        for _ in 0..6 {
            explorer.step(Step::Into);
        }
        let shown = shown(&explorer);
        assert!(
            shown.contains(&"2.1 load".to_owned()) && shown.contains(&"2.1.1 read".to_owned()),
            "the load the cursor is in opens: {shown:?}"
        );
    }

    #[test]
    fn a_level_opens_only_as_wide_as_there_is_room() {
        let mut explorer = explorer(&[
            ("main", &["wide", "narrow"]),
            ("wide", &["a", "b", "c", "d"]),
            ("narrow", &["e"]),
            ("e", &["f"]),
            ("a", &[]),
            ("b", &[]),
            ("c", &[]),
            ("d", &[]),
            ("f", &[]),
        ]);
        explorer.set_room(3);
        explorer.set_disclose(Disclose::Depth(3));
        assert_eq!(
            shown(&explorer),
            ["1 wide", "2 narrow", "2.1 e", "2.1.1 f"],
            "a chain takes no more room; four calls side by side would"
        );
        let wide = at(&explorer, "1");
        assert_eq!(explorer.disclosure().folded(wide), 4);
        assert_eq!(explorer.disclosure().held(), 1);

        explorer.toggle_expanded(wide);
        assert_eq!(
            shown(&explorer).len(),
            8,
            "opened by hand, it opens whatever room it takes"
        );
    }
}
