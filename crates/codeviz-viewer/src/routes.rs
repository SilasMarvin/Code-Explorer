//! How execution gets to a function, and where it goes from there. Walking callers
//! from a function finds every route that reaches it, out to the entry points where
//! requests come in; walking callees finds everything it can run. Both count calls,
//! fewest first.

use std::collections::{HashMap, HashSet, VecDeque, hash_map::Entry};

use codeviz_layout::ForceGraph;

use crate::explorer::Side;

/// Fewest calls between each tile and the nearest of `starts`, which count as zero,
/// walking `side`: along callers for what reaches them, along callees for what they
/// reach. Goes no further than `limit` calls when given, and not past a tile `stop`
/// holds, as a route starts at an entry point and does not climb above it.
/// Registrations are wiring, not calls: a router that registers a handler neither
/// reaches what the handler runs nor is reached by it.
pub(crate) fn walk(
    graph: &ForceGraph,
    starts: &[usize],
    side: Side,
    limit: Option<u32>,
    stop: impl Fn(usize) -> bool,
) -> HashMap<usize, u32> {
    let mut next: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(caller, callee) in &graph.calls {
        if graph.registers(caller, callee) {
            continue;
        }
        let (from, to) = match side {
            Side::Callers => (callee, caller),
            Side::Callees => (caller, callee),
        };
        next.entry(from).or_default().push(to);
    }
    let mut found: HashMap<usize, u32> = starts.iter().map(|&tile| (tile, 0)).collect();
    let mut queue: VecDeque<usize> = starts.iter().copied().collect();
    while let Some(at) = queue.pop_front() {
        let hops = found[&at];
        if limit.is_some_and(|limit| hops >= limit) || (hops > 0 && stop(at)) {
            continue;
        }
        for &to in next.get(&at).into_iter().flatten() {
            if let Entry::Vacant(slot) = found.entry(to) {
                slot.insert(hops + 1);
                queue.push_back(to);
            }
        }
    }
    found
}

/// Where a route into a function starts: a declared entry point, or a function
/// nothing calls (`main`, a framework's callback, a test, a library's API).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Start {
    pub(crate) tile: usize,
    pub(crate) label: String,
    /// Calls between it and the function it reaches.
    pub(crate) hops: u32,
    /// Whether it is a declared entry point rather than a function nothing calls.
    pub(crate) declared: bool,
}

/// Where the routes in `upstream`, a walk along callers, start: the declared entries
/// it reached, then whatever it reached that nothing calls (but may register, to be
/// called by a framework), each nearest first.
pub(crate) fn starts(
    graph: &ForceGraph,
    upstream: &HashMap<usize, u32>,
    entries: &HashMap<usize, String>,
    name: impl Fn(usize) -> String,
) -> Vec<Start> {
    let called: HashSet<usize> = graph
        .calls
        .iter()
        .filter(|&&(caller, callee)| !graph.registers(caller, callee))
        .map(|&(_, callee)| callee)
        .collect();
    let mut starts: Vec<Start> = upstream
        .iter()
        .filter(|&(_, &hops)| hops > 0)
        .filter_map(|(&tile, &hops)| {
            let (label, declared) = match entries.get(&tile) {
                Some(label) => (label.clone(), true),
                None if !called.contains(&tile) => (name(tile), false),
                None => return None,
            };
            Some(Start {
                tile,
                label,
                hops,
                declared,
            })
        })
        .collect();
    starts.sort_by(|a, b| (!a.declared, a.hops, &a.label).cmp(&(!b.declared, b.hops, &b.label)));
    starts
}

#[cfg(test)]
mod tests {
    use codeviz_core::System;
    use codeviz_ir::{Document, UnitId};
    use codeviz_layout::{ForceConfig, MapConfig, map};

    use super::*;

    /// `main -> router`, which registers `get` and `post` as handlers; `get -> find`,
    /// `post -> place -> (find, quote)`, with `get` and `post` declared entries; and
    /// `orphan -> quote`, which `main` registers as a callback.
    fn graph() -> (ForceGraph, impl Fn(&str) -> usize) {
        let unit = |name: &str, calls: &[&str]| {
            // `&name` registers it rather than calling it.
            let calls: Vec<String> = calls
                .iter()
                .map(|c| {
                    let (c, mode) = c
                        .strip_prefix('&')
                        .map_or((*c, "call"), |c| (c, "register"));
                    format!(
                        r#"{{ "key": "{c}#0", "text": "{c}", "mode": "{mode}", "targets": [{{ "unit": "x:{c}", "certainty": "exact" }}] }}"#
                    )
                })
                .collect();
            format!(
                r#"{{ "id": "x:{name}", "name": "{name}", "kind": "fn", "role": "invocable", "calls": [{}] }}"#,
                calls.join(",")
            )
        };
        let units = [
            unit("main", &["router", "&orphan"]),
            unit("router", &["&get", "&post"]),
            unit("get", &["find"]),
            unit("post", &["place"]),
            unit("place", &["find", "quote"]),
            unit("find", &[]),
            unit("quote", &[]),
            unit("orphan", &["quote"]),
        ];
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
            units.join(",")
        );
        let system = System::new([Document::from_json(&json).unwrap()]);
        let map = map(&system, &MapConfig::default());
        let graph = ForceGraph::new(&system, &map, ForceConfig::default());
        let tile = move |name: &str| map.tile_of(&UnitId::new(format!("x:{name}"))).unwrap();
        (graph, tile)
    }

    #[test]
    fn climbs_every_route_up_to_the_entries() {
        let (graph, tile) = graph();
        let entries = |t: usize| t == tile("get") || t == tile("post");
        let up = walk(&graph, &[tile("find")], Side::Callers, None, entries);
        let hops = |name: &str| up.get(&tile(name)).copied();
        assert_eq!(hops("find"), Some(0), "the start itself");
        assert_eq!(hops("get"), Some(1));
        assert_eq!(hops("place"), Some(1));
        assert_eq!(hops("post"), Some(2), "through place");
        assert_eq!(hops("router"), None, "not past the entries");
        assert_eq!(hops("main"), None);
        assert_eq!(hops("quote"), None, "a callee is not a caller");

        let near = walk(&graph, &[tile("find")], Side::Callers, Some(1), entries);
        assert_eq!(near.len(), 3, "find and its direct callers");
        let open = walk(&graph, &[tile("find")], Side::Callers, None, |_| false);
        assert_eq!(
            open.get(&tile("router")),
            None,
            "registering a handler is not calling it"
        );
        let down = walk(&graph, &[tile("main")], Side::Callees, None, |_| false);
        assert_eq!(down.len(), 2, "main runs the router, which runs nothing");
    }

    #[test]
    fn walks_callees_down_and_from_several_starts() {
        let (graph, tile) = graph();
        let down = walk(&graph, &[tile("post")], Side::Callees, None, |_| false);
        assert_eq!(down.get(&tile("find")), Some(&2));
        assert_eq!(down.get(&tile("quote")), Some(&2));
        assert_eq!(down.get(&tile("get")), None);

        let both = walk(
            &graph,
            &[tile("find"), tile("quote")],
            Side::Callers,
            None,
            |_| false,
        );
        assert_eq!(both.get(&tile("orphan")), Some(&1), "nearest of either");
        assert_eq!(both.get(&tile("place")), Some(&1));
    }

    #[test]
    fn routes_start_at_entries_then_at_what_nothing_calls() {
        let (graph, tile) = graph();
        let entries: HashMap<usize, String> = [
            (tile("get"), "GET /users".to_owned()),
            (tile("post"), "POST /orders".to_owned()),
        ]
        .into();
        let up = walk(&graph, &[tile("quote")], Side::Callers, None, |t| {
            entries.contains_key(&t)
        });
        let found = starts(&graph, &up, &entries, |t| format!("#{t}"));
        let labels: Vec<(&str, u32, bool)> = found
            .iter()
            .map(|s| (s.label.as_str(), s.hops, s.declared))
            .collect();
        let orphan = format!("#{}", tile("orphan"));
        assert_eq!(
            labels,
            [("POST /orders", 2, true), (orphan.as_str(), 1, false)],
            "declared entries first; get never reaches quote"
        );
    }
}
