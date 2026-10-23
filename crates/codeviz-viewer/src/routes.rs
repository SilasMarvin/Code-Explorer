//! Where the routes into a function start. Walking callers from a function
//! ([`ForceGraph::walk`]) finds every route that reaches it, out to the entry points
//! where requests come in; the routes start at those, or at functions nothing calls.

use std::collections::{HashMap, HashSet};

use codeviz_layout::ForceGraph;

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

/// The nodes some call runs. One that is only ever registered, as a handler with a
/// router, is not among them: a framework calls it.
pub(crate) fn called(graph: &ForceGraph) -> HashSet<usize> {
    graph
        .calls
        .iter()
        .filter(|&&(caller, callee)| !graph.registers(caller, callee))
        .map(|&(_, callee)| callee)
        .collect()
}

/// Where the routes in `upstream`, a walk along callers, start: the declared entries
/// it reached, each with every route it is declared under, then whatever it reached
/// that nothing calls (but may register, to be called by a framework), each nearest
/// first. What only registers a function on the way starts no route: it never runs
/// it.
pub(crate) fn starts(
    graph: &ForceGraph,
    upstream: &HashMap<usize, u32>,
    entries: &HashMap<usize, Vec<String>>,
    name: impl Fn(usize) -> String,
) -> Vec<Start> {
    let called = called(graph);
    let running: HashSet<usize> = graph
        .calls
        .iter()
        .filter(|&&(caller, callee)| {
            upstream.contains_key(&callee) && !graph.registers(caller, callee)
        })
        .map(|&(caller, _)| caller)
        .collect();
    let mut starts: Vec<Start> = upstream
        .iter()
        .filter(|&(tile, &hops)| hops > 0 && running.contains(tile))
        .filter_map(|(&tile, &hops)| {
            let (label, declared) = match entries.get(&tile) {
                Some(labels) => (labels.join(" · "), true),
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
    use codeviz_layout::{ForceConfig, MapConfig, Toward, map};

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
    fn routes_start_at_entries_then_at_what_nothing_calls() {
        let (graph, tile) = graph();
        let entries: HashMap<usize, Vec<String>> = [
            (tile("get"), vec!["GET /users".to_owned()]),
            (
                tile("post"),
                vec!["POST /orders".to_owned(), "PUT /orders".to_owned()],
            ),
        ]
        .into();
        let up = graph.walk(&[tile("quote")], Toward::Callers, None, |t| {
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
            [
                ("POST /orders · PUT /orders", 2, true),
                (orphan.as_str(), 1, false)
            ],
            "declared entries first, by every route; get never reaches quote"
        );
    }
}
