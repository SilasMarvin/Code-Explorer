//! Where to start in a codebase you have just opened: its entry points, the ones that
//! reach the most of it first, and those in demo, example or test code after the
//! rest. Without declared entries, the functions nothing calls stand in for them.

use std::collections::HashSet;

use codeviz_core::{System, display_name};
use codeviz_layout::{CodebaseMap, ForceGraph};

/// Path segments that mark code beside the codebase proper rather than part of it.
const ASIDE: [&str; 12] = [
    "demo", "demos", "example", "examples", "test", "tests", "bench", "benches", "fixture",
    "fixtures", "spike", "testdata",
];

/// A place to start exploring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Start {
    pub(crate) tile: usize,
    /// The entry's label, or the function's name.
    pub(crate) label: String,
    /// How many functions it reaches through its calls.
    pub(crate) reach: usize,
    /// Whether it lives in demo, example, test or fixture code.
    pub(crate) aside: bool,
}

/// The starts worth offering, best first: declared entries, or without any the
/// functions nothing calls that call something. What reaches the most comes first,
/// with what is set aside after the rest.
pub(crate) fn starts(system: &System, map: &CodebaseMap, graph: &ForceGraph) -> Vec<Start> {
    let aside = |tile: usize| {
        map.tile(tile)
            .and_then(|t| system.unit(&t.unit))
            .and_then(|u| u.location.as_ref())
            .is_some_and(|l| set_aside(&l.uri))
    };
    let mut seen = HashSet::new();
    let entries: Vec<(usize, String)> = system
        .entries()
        .iter()
        .filter_map(|e| Some((map.tile_of(&e.unit)?, e.label.clone())))
        .filter(|(tile, _)| seen.insert(*tile))
        .collect();
    let candidates = if entries.is_empty() {
        (0..map.tiles.len())
            .filter(|&t| graph.callers(t).next().is_none() && graph.callees(t).next().is_some())
            .filter_map(|t| Some((t, display_name(system, &map.tile(t)?.unit))))
            .collect()
    } else {
        entries
    };
    let mut starts: Vec<Start> = candidates
        .into_iter()
        .map(|(tile, label)| Start {
            tile,
            label,
            reach: graph.reach(tile).downstream.len(),
            aside: aside(tile),
        })
        .collect();
    // When everything is demo or test code, as in a demo, none of it is set aside.
    if starts.iter().all(|s| s.aside) {
        for start in &mut starts {
            start.aside = false;
        }
    }
    starts.sort_by(|a, b| {
        (a.aside, std::cmp::Reverse(a.reach), &a.label).cmp(&(
            b.aside,
            std::cmp::Reverse(b.reach),
            &b.label,
        ))
    });
    starts
}

/// Whether a source path lies in demo, example, test or fixture code.
fn set_aside(uri: &str) -> bool {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    path.split(['/', '\\'])
        .any(|segment| ASIDE.contains(&segment.to_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use codeviz_ir::Document;
    use codeviz_layout::{ForceConfig, MapConfig, map};

    use super::*;

    fn system(entries: bool) -> System {
        let unit = |name: &str, file: &str, calls: &[&str]| {
            let calls: Vec<String> = calls
                .iter()
                .map(|c| {
                    format!(
                        r#"{{ "key": "{c}", "text": "{c}", "targets": [{{ "unit": "x:{c}", "certainty": "exact" }}] }}"#
                    )
                })
                .collect();
            format!(
                r#"{{ "id": "x:{name}", "name": "{name}", "kind": "fn", "role": "invocable",
                   "location": {{ "uri": "{file}" }}, "calls": [{}] }}"#,
                calls.join(",")
            )
        };
        let units = [
            unit("main", "src/main.rs", &["a"]),
            unit("a", "src/a.rs", &["b"]),
            unit("b", "src/a.rs", &[]),
            unit("demo", "demo/app/main.rs", &["a", "b", "c"]),
            unit("c", "demo/app/main.rs", &[]),
            unit("unused", "src/a.rs", &[]),
        ];
        let entries = if entries {
            r#", "entries": [
                { "unit": "x:demo", "label": "demo main", "kind": "main" },
                { "unit": "x:a", "label": "a", "kind": "main" },
                { "unit": "x:main", "label": "main", "kind": "main" }
            ]"#
        } else {
            ""
        };
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}]{entries} }}"#,
            units.join(",")
        );
        System::new([Document::from_json(&json).unwrap()])
    }

    fn labels(entries: bool) -> Vec<(String, usize, bool)> {
        let system = system(entries);
        let map = map(&system, &MapConfig::default());
        let graph = ForceGraph::new(&system, &map, ForceConfig::default());
        starts(&system, &map, &graph)
            .into_iter()
            .map(|s| (s.label, s.reach, s.aside))
            .collect()
    }

    #[test]
    fn entries_that_reach_the_most_come_first_and_demos_last() {
        assert_eq!(
            labels(true),
            [
                ("main".to_owned(), 2, false),
                ("a".to_owned(), 1, false),
                ("demo main".to_owned(), 3, true),
            ]
        );
    }

    #[test]
    fn without_entries_what_nothing_calls_stands_in() {
        assert_eq!(
            labels(false),
            [("main".to_owned(), 2, false), ("demo".to_owned(), 3, true)],
            "what calls nothing, like `unused`, is no place to start"
        );
    }

    #[test]
    fn a_demo_alone_is_not_set_aside() {
        let json = r#"{ "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" },
            "units": [
              { "id": "x:a", "name": "a", "kind": "fn", "role": "invocable",
                "location": { "uri": "demo/a.rs" },
                "calls": [{ "key": "b", "text": "b", "targets": [{ "unit": "x:b", "certainty": "exact" }] }] },
              { "id": "x:b", "name": "b", "kind": "fn", "role": "invocable", "location": { "uri": "demo/a.rs" } }
            ] }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let map = map(&system, &MapConfig::default());
        let graph = ForceGraph::new(&system, &map, ForceConfig::default());
        let starts = starts(&system, &map, &graph);
        assert_eq!(starts.len(), 1);
        assert!(!starts[0].aside);
    }

    #[test]
    fn test_and_demo_code_is_set_aside() {
        assert!(set_aside("demo/axum-server/src/handlers.rs"));
        assert!(set_aside("file:///repo/crates/x/tests/fixtures/a.rs"));
        assert!(!set_aside("crates/codeviz/src/main.rs"));
        assert!(!set_aside("src/latest.rs"), "a segment, not a substring");
    }
}
