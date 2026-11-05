use codeviz_core::System;
use codeviz_ir::{Document, UnitId};
use codeviz_layout::{CallGraph, CodebaseMap, map};

use super::*;

/// Seven modules: `web::route` calls `core::run`, which calls `db::put`,
/// `log::say` and `core::help`; `core::help` calls `core::idle` and `ext::lib`;
/// `db::put` calls `disk::write`; `log::tail` calls `log::say`; `far` calls nothing.
fn codebase() -> (CallGraph, CodebaseMap) {
    let module = |name: &str| {
        format!(r#"{{ "id": "m:{name}", "name": "{name}", "kind": "module", "role": "group" }}"#)
    };
    let function = |id: &str, calls: &[&str]| {
        let (parent, name) = id.split_once("::").expect("module::name");
        let calls: Vec<String> = calls
            .iter()
            .map(|to| {
                format!(
                    r#"{{ "key": "{to}#0", "text": "{to}", "targets": [{{ "unit": "f:{to}", "certainty": "exact" }}] }}"#
                )
            })
            .collect();
        format!(
            r#"{{ "id": "f:{id}", "name": "{name}", "kind": "fn", "role": "invocable", "parent": "m:{parent}", "calls": [{}] }}"#,
            calls.join(",")
        )
    };
    let units: Vec<String> = ["web", "core", "db", "log", "disk", "ext", "far"]
        .into_iter()
        .map(module)
        .chain([
            function("web::route", &["core::run"]),
            function("core::run", &["db::put", "log::say", "core::help"]),
            function("core::help", &["core::idle", "ext::lib"]),
            function("core::idle", &[]),
            function("db::put", &["disk::write"]),
            function("db::other", &[]),
            function("log::say", &[]),
            function("log::tail", &["log::say"]),
            function("disk::write", &[]),
            function("ext::lib", &[]),
            function("far::alone", &[]),
        ])
        .collect();
    let json = format!(
        r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
        units.join(",")
    );
    let system = System::new([Document::from_json(&json).expect("a valid document")]);
    let map = map(&system);
    (CallGraph::new(&system, &map), map)
}

fn tile(map: &CodebaseMap, id: &str) -> usize {
    map.tile_of(&UnitId::new(format!("f:{id}")))
        .expect("a function in the map")
}

fn module(graph: &CallGraph, map: &CodebaseMap, id: &str) -> usize {
    graph.cluster(tile(map, id)).expect("a module")
}

/// What the graph says `from` calls, `depth` calls down, as a journey would run it.
fn trail(graph: &CallGraph, from: usize, depth: u32) -> Trail {
    let mut trail = Trail::default();
    let mut stack = vec![(from, 0)];
    while let Some((tile, level)) = stack.pop() {
        if level == depth {
            continue;
        }
        let mut callees: Vec<usize> = Vec::new();
        for callee in graph.callees(tile) {
            if callee != tile && !callees.contains(&callee) {
                callees.push(callee);
            }
        }
        for callee in callees {
            trail.add(tile, callee, level + 1, false);
            stack.push((callee, level + 1));
        }
    }
    // Everything below at any depth, as the journey would give it.
    let mut wave = vec![from];
    for level in 1..10 {
        let mut next = Vec::new();
        for tile in wave {
            for callee in graph.callees(tile) {
                if callee != from && !trail.whole.contains_key(&callee) {
                    trail.whole.insert(callee, level);
                    next.push(callee);
                }
            }
        }
        wave = next;
    }
    trail
}

/// The nucleus round `id`, naming the journey from it `depth` calls down.
fn around(graph: &CallGraph, map: &CodebaseMap, id: &str, depth: u32) -> Nucleus {
    let start = tile(map, id);
    Nucleus::gather(graph, Some(start), trail(graph, start, depth), false)
}

/// `core::start` calls `db::put`, which calls `db::deep`; nine functions call
/// `core::start`: `web::c0` to `web::c5`, of which `web::c0` calls `db::put` too,
/// and `core::c6` (twice) to `core::c8`.
fn crowded() -> (CallGraph, CodebaseMap) {
    let module = |name: &str| {
        format!(r#"{{ "id": "m:{name}", "name": "{name}", "kind": "module", "role": "group" }}"#)
    };
    let function = |id: &str, calls: &[&str]| {
        let (parent, name) = id.split_once("::").expect("module::name");
        let calls: Vec<String> = calls
            .iter()
            .enumerate()
            .map(|(index, to)| {
                format!(
                    r#"{{ "key": "{to}#{index}", "text": "{to}", "targets": [{{ "unit": "f:{to}", "certainty": "exact" }}] }}"#
                )
            })
            .collect();
        format!(
            r#"{{ "id": "f:{id}", "name": "{name}", "kind": "fn", "role": "invocable", "parent": "m:{parent}", "calls": [{}] }}"#,
            calls.join(",")
        )
    };
    let mut units: Vec<String> = ["web", "core", "db"].into_iter().map(module).collect();
    units.push(function("core::start", &["db::put"]));
    units.push(function("db::put", &["db::deep"]));
    units.push(function("db::deep", &[]));
    units.push(function("web::c0", &["core::start", "db::put"]));
    for caller in ["web::c1", "web::c2", "web::c3", "web::c4", "web::c5"] {
        units.push(function(caller, &["core::start"]));
    }
    units.push(function("core::c6", &["core::start", "core::start"]));
    units.push(function("core::c7", &["core::start"]));
    units.push(function("core::c8", &["core::start"]));
    let json = format!(
        r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
        units.join(",")
    );
    let system = System::new([Document::from_json(&json).expect("a valid document")]);
    let map = map(&system);
    (CallGraph::new(&system, &map), map)
}

#[test]
fn the_start_names_a_handful_of_its_callers_and_counts_the_rest() {
    let (graph, map) = crowded();
    for depth in [1, 2] {
        let nucleus = around(&graph, &map, "core::start", depth);
        // Calling into the journey, calling it most, and living by it come first.
        for id in ["web::c0", "core::c6", "core::c7", "core::c8"] {
            let caller = tile(&map, id);
            assert!(!nucleus.folded(caller), "{id} is named at depth {depth}");
        }
        let folded: Vec<usize> = (0..graph.len()).filter(|&t| nucleus.folded(t)).collect();
        assert_eq!(folded.len(), 4, "nine callers, five named");
        for &caller in &folded {
            assert!(nucleus.placed(caller) && nucleus.lit(caller), "still drawn");
        }
    }
    let shallow = around(&graph, &map, "core::start", 1);
    let deep = around(&graph, &map, "core::start", 2);
    assert_eq!(shallow.now.folded, deep.now.folded, "a depth changes none");
}

#[test]
fn the_cursor_badge_leads_with_the_call_it_is_on() {
    let numbers: Vec<String> = ["1.1", "3.1.1", "1.1"].map(str::to_owned).to_vec();
    assert_eq!(badge(&numbers, None).as_deref(), Some("1.1 +1"));
    assert_eq!(badge(&numbers, Some("3.1.1")).as_deref(), Some("3.1.1 +1"));
    assert_eq!(badge(&numbers[..1], Some("1.1")).as_deref(), Some("1.1"));
}

#[test]
fn calls_run_into_and_out_of_the_nucleus() {
    let (graph, map) = codebase();
    let nucleus = around(&graph, &map, "core::run", 2);
    let has = |from: &str, to: &str, flow| {
        nucleus
            .links()
            .iter()
            .any(|l| l.from == tile(&map, from) && l.to == tile(&map, to) && l.flow == flow)
    };
    assert!(has("web::route", "core::run", Flow::Into));
    assert!(has("core::run", "db::put", Flow::Journey(1)));
    assert!(has("db::put", "disk::write", Flow::Journey(2)));
    assert!(
        nucleus.tubes().is_empty(),
        "no line between modules in a journey"
    );
}

#[test]
fn expanded_the_start_calls_in_and_out_of_its_module() {
    let (graph, map) = codebase();
    let mut nucleus = around(&graph, &map, "core::run", 1);
    nucleus.expand(&graph, Some(nucleus.middle()));
    let (run, help, put) = (
        tile(&map, "core::run"),
        tile(&map, "core::help"),
        tile(&map, "db::put"),
    );
    let link = |from, to| {
        nucleus
            .links()
            .iter()
            .find(|l| l.from == from && l.to == to)
            .copied()
    };
    let inside = link(run, help).expect("the call inside the module");
    assert_eq!(inside.flow, Flow::Journey(1));
    assert!(link(run, put).is_some(), "the call out to db");
    assert!(nucleus.placed(put), "drawn in db's sphere");
    assert!(nucleus.level(help).is_some(), "called by the start, named");
    assert!(
        nucleus.level(tile(&map, "core::idle")).is_none(),
        "two calls along, not named"
    );
}

/// Where every module and function sits, as drawn.
fn places(graph: &CallGraph, nucleus: &Nucleus) -> (Vec<Option<Vec3>>, Vec<(Vec3, f32)>) {
    let points = (0..graph.len()).map(|t| nucleus.point(t)).collect();
    let balls = nucleus.balls().map(|(_, b)| (b.center, b.radius)).collect();
    (points, balls)
}

#[test]
fn nothing_moves_whatever_the_view_lights() {
    let (graph, map) = codebase();
    let run = tile(&map, "core::run");
    let mut nucleus = Nucleus::gather(&graph, None, Trail::default(), true);
    let before = places(&graph, &nucleus);
    let same = |nucleus: &Nucleus, what: &str| {
        assert_eq!(places(&graph, nucleus), before, "nothing moves: {what}");
    };
    for id in ["db::put", "far::alone", "core::help"] {
        nucleus.recenter(&graph, Some(tile(&map, id)));
        same(&nucleus, id);
    }
    nucleus.set_browse(&graph, false);
    nucleus.recenter(&graph, Some(run));
    nucleus.retrace(&graph, &trail(&graph, run, 1));
    same(&nucleus, "a journey");
    nucleus.retrace(&graph, &trail(&graph, run, 2));
    same(&nucleus, "a level more");
    nucleus.set_cursor(&graph, Some(tile(&map, "log::say")));
    nucleus.set_outside(&graph, true);
    same(&nucleus, "callers from outside");
    nucleus.set_aside(Some(tile(&map, "far::alone")));
    same(&nucleus, "a function set aside");
    for id in ["core::run", "db::put", "far::alone"] {
        nucleus.expand(&graph, Some(module(&graph, &map, id)));
        same(&nucleus, &format!("{id}'s module expanded"));
    }
    nucleus.expand(&graph, None);
    nucleus.set_browse(&graph, true);
    same(&nucleus, "browsing again");
    assert_eq!(
        places(
            &graph,
            &Nucleus::gather(&graph, None, Trail::default(), true)
        ),
        before,
        "laid out alike every time"
    );
}

#[test]
fn every_function_inside_its_module_and_none_touching() {
    let (graph, map) = codebase();
    let nucleus = Nucleus::gather(&graph, None, Trail::default(), true);
    for tile in 0..graph.len() {
        let cluster = nucleus.module_of(tile).expect("a module");
        let ball = nucleus.ball(cluster).expect("a sphere");
        let room = nucleus.room(cluster).expect("room");
        let at = nucleus.point(tile).expect("a place");
        assert!(
            at.distance(ball.center) + room <= ball.radius * 1.001,
            "inside"
        );
        for other in (tile + 1..graph.len()).filter(|&t| nucleus.module_of(t) == Some(cluster)) {
            let apart = at.distance(nucleus.point(other).expect("a place"));
            assert!(apart >= 2.0 * room * 0.998, "apart: {apart}");
        }
    }
    let balls: Vec<Ball> = nucleus.balls().map(|(_, b)| b).collect();
    for (i, a) in balls.iter().enumerate() {
        for b in &balls[i + 1..] {
            assert!(a.center.distance(b.center) >= a.radius + b.radius, "apart");
        }
    }
    // What calls what sits near it.
    let far = module(&graph, &map, "far::alone");
    let core = module(&graph, &map, "core::run");
    let near = |id: &str| nucleus.ball(module(&graph, &map, id)).unwrap().center;
    let alone = nucleus.ball(far).unwrap().center;
    let middle = nucleus.ball(core).unwrap().center;
    assert!(
        near("db::put").distance(middle) < alone.distance(middle),
        "core calls db; nothing calls far"
    );
}

#[test]
fn a_journey_lights_its_modules_where_they_are_and_dims_the_rest() {
    let (graph, map) = codebase();
    let nucleus = around(&graph, &map, "core::run", 2);
    let core = module(&graph, &map, "core::run");
    assert_eq!(nucleus.ball(core).map(|b| b.body), Some(Body::Nucleus));
    for id in ["web::route", "db::put", "log::say", "disk::write"] {
        assert!(nucleus.level(tile(&map, id)).is_some(), "{id} is named");
        let ball = nucleus.ball(module(&graph, &map, id));
        assert_eq!(
            ball.map(|b| b.body),
            Some(Body::Open),
            "{id}'s module opens"
        );
    }
    let far = nucleus.ball(module(&graph, &map, "far::alone"));
    assert_eq!(far.map(|b| b.body), Some(Body::Faint), "the rest dims");
    for id in ["db::other", "far::alone"] {
        let tile = tile(&map, id);
        assert!(
            nucleus.speck(tile) && !nucleus.drawn(tile),
            "{id} left to its sphere"
        );
    }
}

#[test]
fn the_depth_says_what_is_named() {
    let (graph, map) = codebase();
    let run = tile(&map, "core::run");
    let mut nucleus = around(&graph, &map, "core::run", 1);
    let (disk, idle) = (tile(&map, "disk::write"), tile(&map, "core::idle"));
    assert_eq!(
        nucleus.level(tile(&map, "web::route")),
        Some(-1),
        "a caller"
    );
    assert_eq!(nucleus.level(tile(&map, "db::put")), Some(1), "a call");
    assert!(nucleus.level(disk).is_none() && nucleus.level(idle).is_none());
    let disk_module = module(&graph, &map, "disk::write");
    assert_eq!(nucleus.ball(disk_module).map(|b| b.body), Some(Body::Faint));
    assert!(nucleus.retrace(&graph, &trail(&graph, run, 2)));
    assert!(
        !nucleus.retrace(&graph, &trail(&graph, run, 2)),
        "no change"
    );
    assert_eq!(nucleus.level(disk), Some(2));
    assert_eq!(nucleus.ball(disk_module).map(|b| b.body), Some(Body::Open));
    assert!(nucleus.retrace(&graph, &trail(&graph, run, 1)));
    assert!(nucleus.level(disk).is_none(), "a smaller depth lets it go");
}

#[test]
fn an_outside_caller_lights_where_it_is() {
    let (graph, map) = codebase();
    // log::tail calls log::say, on core::run's journey, from off it.
    let mut nucleus = around(&graph, &map, "core::run", 1);
    nucleus.set_cursor(&graph, Some(tile(&map, "log::say")));
    let tail = tile(&map, "log::tail");
    assert!(!nucleus.lit(tail));
    nucleus.set_outside(&graph, true);
    assert!(nucleus.outside(tail) && nucleus.lit(tail), "it lights");
    assert!(
        nucleus
            .links()
            .iter()
            .any(|l| l.flow == Flow::Outside && l.from == tail)
    );
    assert!(
        !nucleus.outside(tile(&map, "core::run")),
        "nor is the start"
    );
}

#[test]
fn a_function_run_twice_shows_once_with_both_numbers() {
    let mut trail = Trail::default();
    trail.add(0, 1, 1, false);
    trail.add(0, 2, 1, false);
    trail.add(2, 1, 2, false);
    trail.add(1, 3, 3, false);
    trail.add(1, 3, 2, false);
    assert_eq!(trail.calls.len(), 4, "each call once");
    assert_eq!(trail.calls[3].level, 2, "at its shallowest");
    let numbers: Vec<String> = ["2.1", "1"].map(str::to_owned).to_vec();
    assert_eq!(badge(&numbers, None).as_deref(), Some("1 +1"));
    let numbers: Vec<String> = ["1.10", "1.9"].map(str::to_owned).to_vec();
    assert_eq!(
        badge(&numbers, None).as_deref(),
        Some("1.9 +1"),
        "in the journey's order"
    );
    assert_eq!(badge(&[], None), None);
    let both = ["1.1".to_owned(), "2".to_owned(), "2".to_owned()];
    assert_eq!(
        badge(&both, None).as_deref(),
        Some("2 +1"),
        "sorted, once each"
    );
}

#[test]
fn a_badge_keeps_the_call_nearest_the_root_wherever_the_cursor_is() {
    let mut numbers: Vec<String> = ["1.1", "2"].map(str::to_owned).to_vec();
    assert_eq!(
        badge(&numbers, None).as_deref(),
        Some("2 +1"),
        "the shallower first"
    );
    // A level deeper adds a run further down, and the number stays.
    numbers.push("1.1.4".to_owned());
    assert_eq!(badge(&numbers, None).as_deref(), Some("2 +2"));
}

#[test]
fn browsing_with_nothing_selected_every_module_points_to_each_it_calls() {
    let (graph, map) = codebase();
    let nucleus = Nucleus::gather(&graph, None, Trail::default(), true);
    let tubes = nucleus.tubes();
    let mut pairs: Vec<(usize, usize)> = graph
        .calls
        .iter()
        .filter_map(|&(a, b)| Some((graph.cluster(a)?, graph.cluster(b)?)))
        .filter(|(a, b)| a != b)
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    let mut drawn: Vec<(usize, usize)> = tubes.iter().map(|t| (t.from, t.to)).collect();
    drawn.sort_unstable();
    assert_eq!(drawn, pairs, "no dependency between modules is left out");
    let core = module(&graph, &map, "core::run");
    assert_eq!(
        tubes.iter().filter(|t| t.from == core).count(),
        3,
        "core calls three modules"
    );
    let selected = Nucleus::gather(
        &graph,
        Some(tile(&map, "core::run")),
        Trail::default(),
        true,
    );
    assert!(
        selected.tubes().is_empty(),
        "a selection's own calls take over"
    );
}
