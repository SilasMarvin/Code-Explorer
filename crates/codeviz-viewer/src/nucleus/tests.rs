use codeviz_core::System;
use codeviz_ir::{Document, UnitId};
use codeviz_layout::{
    CodebaseMap, ForceConfig,
    atlas::{self, fibonacci},
    map,
};

use super::{
    inside::{ROW_WIDE, lattice, spread},
    places::{Room, place},
    *,
};
use crate::sizes::Dot;

/// Seven modules: `web::route` calls `core::run`, which calls `db::put`,
/// `log::say` and `core::help`; `core::help` calls `core::idle` and `ext::lib`;
/// `db::put` calls `disk::write`; `log::tail` calls `log::say`; `far` calls nothing.
fn codebase() -> (ForceGraph, CodebaseMap) {
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
    let mut graph = ForceGraph::new(&system, &map, ForceConfig::default());
    for _ in 0..50 {
        graph.step();
    }
    (graph, map)
}

fn tile(map: &CodebaseMap, id: &str) -> usize {
    map.tile_of(&UnitId::new(format!("f:{id}")))
        .expect("a function in the map")
}

fn module(graph: &ForceGraph, map: &CodebaseMap, id: &str) -> usize {
    graph.cluster(tile(map, id)).expect("a module")
}

/// Every function `tile` calls, `left` calls down, in the order written, first
/// calls first, each once.
fn runs(graph: &ForceGraph, tile: usize, left: u32, order: &mut Vec<usize>) {
    for &callee in graph.sequence(tile) {
        if left > 0 && !order.contains(&callee) {
            order.push(callee);
            runs(graph, callee, left - 1, order);
        }
    }
}

/// What the graph says `from` calls, `depth` calls down, as a journey would run it.
fn trail(graph: &ForceGraph, from: usize, depth: u32) -> Trail {
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
    // Everything below in the order a journey runs it, first calls first.
    runs(graph, from, 10, &mut trail.order);
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
fn around(graph: &ForceGraph, map: &CodebaseMap, id: &str, depth: u32) -> Nucleus {
    let start = tile(map, id);
    Nucleus::gather(graph, Some(start), trail(graph, start, depth), false)
}

/// `core::start` calls `db::put`, which calls `db::deep`; nine functions call
/// `core::start`: `web::c0` to `web::c5`, of which `web::c0` calls `db::put` too,
/// and `core::c6` (twice) to `core::c8`.
fn crowded() -> (ForceGraph, CodebaseMap) {
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
    let mut graph = ForceGraph::new(&system, &map, ForceConfig::default());
    for _ in 0..50 {
        graph.step();
    }
    (graph, map)
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
        let folded: Vec<usize> = (0..graph.positions.len())
            .filter(|&t| nucleus.folded(t))
            .collect();
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
fn the_start_module_sits_in_the_middle_holding_all_its_functions() {
    let (graph, map) = codebase();
    let nucleus = around(&graph, &map, "core::run", 2);
    let core = module(&graph, &map, "core::run");
    let ball = nucleus.ball(core).expect("a sphere");
    assert_eq!(ball.body, Body::Nucleus);
    assert!(ball.center.length() < 1e-3, "the nucleus is the middle");
    for id in ["core::run", "core::help", "core::idle"] {
        assert!(
            nucleus.placed(tile(&map, id)),
            "every function inside shows"
        );
        let at = nucleus.point(tile(&map, id)).expect("a point");
        assert!(at.length() <= ball.radius, "inside its sphere");
    }
}

#[test]
fn modules_near_the_start_open_and_the_rest_stay_closed_further_out() {
    let (graph, map) = codebase();
    let nucleus = around(&graph, &map, "core::run", 2);
    for id in ["web::route", "db::put", "log::say", "disk::write"] {
        assert!(nucleus.level(tile(&map, id)).is_some(), "{id} is named");
        let ball = nucleus.ball(module(&graph, &map, id));
        assert_eq!(
            ball.map(|b| b.body),
            Some(Body::Open),
            "{id}'s module opens"
        );
    }
    assert!(
        !nucleus.placed(tile(&map, "db::other")),
        "what an open module does not name stays folded"
    );
    // Clockwise from the top in the order the journey first enters them, level by
    // level, then the callers'.
    let rotation = view_rotation();
    let turned = |id: &str| {
        let at = nucleus
            .ball(module(&graph, &map, id))
            .expect("a sphere")
            .center;
        let angle = at.dot(rotation * Vec3::Y).atan2(at.dot(rotation * Vec3::X));
        (std::f32::consts::FRAC_PI_2 - angle).rem_euclid(std::f32::consts::TAU)
    };
    let turns: Vec<f32> = [
        "db::put",
        "log::say",
        "disk::write",
        "ext::lib",
        "web::route",
    ]
    .map(turned)
    .into();
    assert!(turns[0] < 0.1, "the first at the top: {turns:?}");
    assert!(
        turns.is_sorted(),
        "clockwise in the journey's order: {turns:?}"
    );
    let far = nucleus
        .ball(module(&graph, &map, "far::alone"))
        .expect("a sphere");
    assert_eq!(far.body, Body::Closed);
    assert!(!nucleus.shows_body(far.body), "and a journey leaves it out");
    assert_eq!(
        far.center,
        nucleus.atlas.balls[module(&graph, &map, "far::alone")].center,
        "where the map has it, to show there again browsing"
    );
    let balls: Vec<Ball> = nucleus
        .balls()
        .map(|(_, b)| b)
        .filter(|b| nucleus.shows_body(b.body))
        .collect();
    for (i, a) in balls.iter().enumerate() {
        for b in &balls[i + 1..] {
            assert!(
                a.center.distance(b.center) >= a.radius + b.radius,
                "spheres keep apart"
            );
        }
    }
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
fn a_new_start_recenters_easing_there() {
    let (graph, map) = codebase();
    let mut nucleus = around(&graph, &map, "core::run", 1);
    let db = module(&graph, &map, "db::put");
    let before = nucleus.ball(db).expect("a sphere").center;
    nucleus.recenter(&graph, Some(tile(&map, "db::put")));
    assert_eq!(nucleus.now.balls[db].body, Body::Nucleus);
    assert!(
        nucleus.ball(db).expect("a sphere").center.distance(before) < 1e-3,
        "starts where it was"
    );
    while nucleus.advance(0.1) {}
    assert!(
        nucleus.ball(db).expect("a sphere").center.length() < 1e-3,
        "ends in the middle"
    );
}

#[test]
fn expanding_the_start_s_module_opens_it_where_it_stands() {
    let (graph, map) = codebase();
    let mut nucleus = around(&graph, &map, "core::run", 1);
    let core = module(&graph, &map, "core::run");
    let balls = |nucleus: &Nucleus| nucleus.balls().collect::<Vec<_>>();
    let before = balls(&nucleus);
    nucleus.expand(&graph, Some(core));
    while nucleus.advance(0.1) {}
    assert_eq!(balls(&nucleus), before, "no sphere moves or grows");
    let ball = nucleus.ball(core).expect("a sphere");
    for id in ["core::run", "core::help", "core::idle"] {
        let t = tile(&map, id);
        assert!(nucleus.placed(t), "{id} drawn");
        let at = nucleus.point(t).expect("a point");
        assert!(at.distance(ball.center) < ball.radius, "{id} inside");
    }
    assert!(
        nucleus.placed(tile(&map, "db::put")),
        "what the start calls outside stays named in its module"
    );
    nucleus.expand(&graph, None);
    while nucleus.advance(0.1) {}
    assert_eq!(balls(&nucleus), before);
}

#[test]
fn expanding_in_a_journey_keeps_its_functions_where_they_were() {
    let (graph, map) = codebase();
    let mut nucleus = around(&graph, &map, "core::run", 2);
    for id in ["core::run", "db::put"] {
        let cluster = module(&graph, &map, id);
        let laid: Vec<(usize, Vec3)> = (0..map.tiles.len())
            .filter(|&t| graph.cluster(t) == Some(cluster) && nucleus.lit(t))
            .filter_map(|t| Some((t, nucleus.point(t)?)))
            .collect();
        assert_ne!(laid.len(), 0);
        nucleus.expand(&graph, Some(cluster));
        while nucleus.advance(0.1) {}
        for &(t, was) in &laid {
            assert!(
                nucleus.point(t).unwrap().distance(was) < 1e-4,
                "{id}: a journey function keeps its place"
            );
        }
        let members: Vec<Vec3> = (0..map.tiles.len())
            .filter(|&t| graph.cluster(t) == Some(cluster))
            .map(|t| {
                assert!(nucleus.drawn(t), "{id}: every function drawn");
                nucleus.point(t).unwrap()
            })
            .collect();
        for (i, a) in members.iter().enumerate() {
            for b in &members[i + 1..] {
                assert!(a.distance(*b) > Dot::Focus.radius(), "{id}: apart");
            }
        }
        nucleus.expand(&graph, None);
        while nucleus.advance(0.1) {}
    }
}

#[test]
fn expanded_the_start_calls_in_and_out_of_its_module() {
    let (graph, map) = codebase();
    let mut nucleus = around(&graph, &map, "core::run", 1);
    nucleus.expand(&graph, Some(nucleus.middle()));
    while nucleus.advance(0.1) {}
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

#[test]
fn an_opened_module_spreads_in_rows_wider_along_than_between() {
    let places = lattice(40, 10.0, ROW_WIDE);
    assert_eq!(places.len(), 40);
    assert_eq!(places[0], Vec2::ZERO, "the middle first");
    for (i, a) in places.iter().enumerate() {
        assert!(a.length() <= 10.0 + 1e-4, "inside the disc");
        for b in &places[i + 1..] {
            let apart = (*a - *b).abs();
            assert!(apart.y > 0.5 || apart.x > 2.0, "apart, wider along a row");
        }
    }
    let members: Vec<(usize, Vec3)> = (0..12).map(|t| (t, fibonacci(t, 12) * 3.0)).collect();
    let kept = [(12, Vec3::ZERO)];
    let spread = spread(&members, &kept, 4.0);
    assert_eq!(spread.len(), 12, "every one placed");
    assert!(
        spread.iter().all(|(_, at)| at.length() > 0.5),
        "clear of the dot kept in the middle"
    );
}

#[test]
fn specks_hide_in_a_journey_and_show_browsing() {
    let (graph, map) = codebase();
    // Two calls down from the start, past the depth shown: a speck in its module.
    let idle = tile(&map, "core::idle");
    let mut nucleus = around(&graph, &map, "core::run", 1);
    assert!(nucleus.speck(idle));
    assert!(!nucleus.drawn(idle), "a journey shows only itself");
    nucleus.expand(&graph, Some(nucleus.middle()));
    assert!(
        nucleus.drawn(idle),
        "expanded, every function of the module"
    );
    assert!(!nucleus.lit(tile(&map, "db::other")), "and only those");
    nucleus.set_browse(&graph, true);
    assert!(nucleus.drawn(idle), "browsing, every function shows");
}

#[test]
fn browsing_every_module_keeps_its_place_whatever_is_clicked() {
    let (graph, map) = codebase();
    let run = tile(&map, "core::run");
    let mut nucleus = Nucleus::gather(&graph, Some(run), Trail::default(), true);
    // Every module, every function, lit or not.
    let places = |nucleus: &Nucleus| {
        let points: Vec<Option<Vec3>> = (0..graph.positions.len())
            .map(|t| nucleus.point(t))
            .collect();
        let balls: Vec<(Vec3, f32)> = nucleus.balls().map(|(_, b)| (b.center, b.radius)).collect();
        (points, balls)
    };
    for id in ["db::put", "far::alone", "web::route", "core::help"] {
        let before = places(&nucleus);
        nucleus.recenter(&graph, Some(tile(&map, id)));
        while nucleus.advance(0.1) {}
        assert_eq!(places(&nucleus), before, "nothing moves for {id}");
    }
    nucleus.recenter(&graph, Some(run));
    assert!(
        nucleus.level(tile(&map, "web::route")).is_some(),
        "its caller"
    );
    assert_eq!(nucleus.level(tile(&map, "web::route")), Some(-1));
    assert_eq!(nucleus.level(tile(&map, "db::put")), Some(1), "its call");
    assert!(
        nucleus.level(tile(&map, "disk::write")).is_none(),
        "two calls down, not"
    );
    for id in ["db::put", "web::route", "core::run"] {
        let ball = nucleus.ball(module(&graph, &map, id)).expect("a sphere");
        assert_eq!(ball.body, Body::Open, "{id}'s module opens in place");
    }
    let far = nucleus
        .ball(module(&graph, &map, "far::alone"))
        .expect("a sphere");
    assert_eq!(far.body, Body::Faint, "the rest recedes");
    for id in ["far::alone", "db::other", "core::idle"] {
        let tile = tile(&map, id);
        assert!(nucleus.placed(tile), "{id} is drawn, to be clicked");
        let ball = nucleus.ball(module(&graph, &map, id)).expect("a sphere");
        let at = nucleus.point(tile).expect("a point");
        assert!(
            at.distance(ball.center) <= ball.radius,
            "{id} inside its module"
        );
    }
    let balls: Vec<Ball> = nucleus.balls().map(|(_, b)| b).collect();
    for (i, a) in balls.iter().enumerate() {
        for b in &balls[i + 1..] {
            assert!(a.center.distance(b.center) >= a.radius + b.radius, "apart");
        }
    }
}

#[test]
fn an_outside_caller_hangs_beyond_what_was_there() {
    let (graph, map) = codebase();
    // log::tail calls log::say, on core::run's journey, from off it.
    let run = tile(&map, "core::run");
    let mut nucleus = around(&graph, &map, "core::run", 1);
    nucleus.set_cursor(&graph, Some(tile(&map, "log::say")));
    while nucleus.advance(0.1) {}
    let before: Vec<(usize, Vec3)> = nucleus
        .balls()
        .filter(|(_, b)| nucleus.shows_body(b.body))
        .map(|(c, b)| (c, b.center))
        .collect();
    nucleus.set_outside(&graph, true);
    while nucleus.advance(0.1) {}
    let tail = tile(&map, "log::tail");
    assert!(nucleus.outside(tail), "it comes in from outside");
    assert!(
        nucleus
            .links()
            .iter()
            .any(|l| l.flow == Flow::Outside && l.from == tail)
    );
    assert!(!nucleus.outside(run), "nor is the start");
    for (cluster, middle) in before {
        assert_eq!(
            nucleus.ball(cluster).map(|b| b.center),
            Some(middle),
            "the rest stays where it was"
        );
    }
}

/// `core::run` calls `db::put` and `log::say`; from off its journey, `db::other`
/// calls `db::put`, `log::tail` calls `log::say`, and in a module the journey never
/// enters, `ext::poke` calls `db::put` and `ext::nudge` calls `log::say`.
fn called_from_outside() -> (ForceGraph, CodebaseMap) {
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
    let units: Vec<String> = ["core", "db", "log", "ext"]
        .into_iter()
        .map(module)
        .chain([
            function("core::run", &["db::put", "log::say"]),
            function("db::put", &[]),
            function("db::other", &["db::put"]),
            function("log::say", &[]),
            function("log::tail", &["log::say"]),
            function("ext::poke", &["db::put"]),
            function("ext::nudge", &["log::say"]),
        ])
        .collect();
    let json = format!(
        r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
        units.join(",")
    );
    let system = System::new([Document::from_json(&json).expect("a valid document")]);
    let map = map(&system);
    let mut graph = ForceGraph::new(&system, &map, ForceConfig::default());
    for _ in 0..50 {
        graph.step();
    }
    (graph, map)
}

#[test]
fn with_i_on_a_step_of_the_cursor_moves_nothing() {
    let (graph, map) = called_from_outside();
    let mut nucleus = around(&graph, &map, "core::run", 1);
    nucleus.set_outside(&graph, true);
    // Where every sphere and dot shown rests, with the cursor on `at`.
    let mut seen = |at: &str| {
        nucleus.set_cursor(&graph, Some(tile(&map, at)));
        while nucleus.advance(0.1) {}
        let balls: HashMap<usize, Vec3> = nucleus
            .balls()
            .filter(|(_, b)| nucleus.shows_body(b.body))
            .map(|(c, b)| (c, b.center))
            .collect();
        let dots: HashMap<usize, Vec3> = (0..map.tiles.len())
            .filter(|&t| nucleus.drawn(t))
            .filter_map(|t| Some((t, nucleus.point(t)?)))
            .collect();
        (balls, dots)
    };
    let put = seen("db::put");
    let say = seen("log::say");
    let again = seen("db::put");
    let (poke, nudge) = (tile(&map, "ext::poke"), tile(&map, "ext::nudge"));
    assert!(put.1.contains_key(&poke) && !put.1.contains_key(&nudge));
    assert!(say.1.contains_key(&nudge) && !say.1.contains_key(&poke));
    assert_eq!(put, again, "back where it was, all of it");
    for (a, b) in [(&put, &say), (&say, &again)] {
        for (cluster, center) in &a.0 {
            if let Some(other) = b.0.get(cluster) {
                assert!(center.distance(*other) < 1e-4, "a sphere stays put");
            }
        }
        for (tile, at) in &a.1 {
            if let Some(other) = b.1.get(tile) {
                assert!(at.distance(*other) < 1e-4, "a dot stays put");
            }
        }
    }
}

/// `core::run` calls `db::put` and `db::get`; from off its journey, `db::c0` to
/// `db::c4` call `db::put`, `db::c0` and `db::c1` call `db::get` too, and `core::c5`
/// and `core::c6`, in the start's own module, call `db::put`.
fn fanned_in() -> (ForceGraph, CodebaseMap) {
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
    let mut units: Vec<String> = ["core", "db"]
        .map(|m| {
            format!(r#"{{ "id": "m:{m}", "name": "{m}", "kind": "module", "role": "group" }}"#)
        })
        .to_vec();
    units.push(function("core::run", &["db::put", "db::get"]));
    units.push(function("db::put", &[]));
    units.push(function("db::get", &[]));
    for caller in ["db::c0", "db::c1"] {
        units.push(function(caller, &["db::put", "db::get"]));
    }
    for caller in ["db::c2", "db::c3", "db::c4", "core::c5", "core::c6"] {
        units.push(function(caller, &["db::put"]));
    }
    let json = format!(
        r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
        units.join(",")
    );
    let system = System::new([Document::from_json(&json).expect("a valid document")]);
    let map = map(&system);
    let mut graph = ForceGraph::new(&system, &map, ForceConfig::default());
    for _ in 0..50 {
        graph.step();
    }
    (graph, map)
}

#[test]
fn every_outside_caller_of_the_cursor_has_a_place_of_its_own() {
    let (graph, map) = fanned_in();
    let mut nucleus = around(&graph, &map, "core::run", 1);
    nucleus.set_outside(&graph, true);
    let drawn = |nucleus: &Nucleus| -> HashMap<usize, Vec3> {
        (0..map.tiles.len())
            .filter(|&t| nucleus.drawn(t))
            .filter_map(|t| Some((t, nucleus.point(t)?)))
            .collect()
    };
    let mut seen = Vec::new();
    for (at, callers) in [
        (
            "db::put",
            &[
                "db::c0", "db::c1", "db::c2", "db::c3", "db::c4", "core::c5", "core::c6",
            ][..],
        ),
        ("db::get", &["db::c0", "db::c1"][..]),
    ] {
        nucleus.set_cursor(&graph, Some(tile(&map, at)));
        while nucleus.advance(0.1) {}
        let dots = drawn(&nucleus);
        // Each is drawn in its sphere, or waits for its module to be expanded, where
        // no room is clear of the names round its place.
        let shown: Vec<&&str> = callers
            .iter()
            .filter(|id| nucleus.outside(tile(&map, id)))
            .collect();
        assert_eq!(
            shown.len() + nucleus.outside_waiting(),
            callers.len(),
            "{at}"
        );
        assert!(!shown.is_empty(), "{at}: some brought in");
        for id in shown {
            let caller = tile(&map, id);
            let ball = nucleus.ball(graph.cluster(caller).unwrap()).unwrap();
            assert!(
                dots[&caller].distance(ball.center) < ball.radius,
                "{at}: {id} in its sphere"
            );
        }
        for (a, at_a) in &dots {
            for (b, at_b) in &dots {
                assert!(
                    a == b || at_a.distance(*at_b) > Dot::Focus.radius(),
                    "{at}: {a} and {b} apart"
                );
            }
        }
        seen.push(dots);
    }
    for (tile, at) in &seen[1] {
        if let Some(was) = seen[0].get(tile) {
            assert!(at.distance(*was) < 1e-4, "{tile} keeps its place");
        }
    }
    // Expanded, every caller in the module shows.
    nucleus.set_cursor(&graph, Some(tile(&map, "db::put")));
    nucleus.expand(&graph, Some(module(&graph, &map, "db::put")));
    while nucleus.advance(0.1) {}
    for id in ["db::c0", "db::c1", "db::c2", "db::c3", "db::c4"] {
        assert!(nucleus.outside(tile(&map, id)), "expanded: {id} brought in");
    }
}

#[test]
fn a_function_set_aside_keeps_its_place_as_the_cursor_brings_callers_in() {
    let (graph, map) = fanned_in();
    let mut nucleus = around(&graph, &map, "core::run", 1);
    nucleus.set_outside(&graph, true);
    // Set aside while it calls neither function the cursor goes to: any seat it
    // might share is someone else's.
    for id in ["db::c2", "core::c6"] {
        let aside = tile(&map, id);
        nucleus.set_cursor(&graph, Some(tile(&map, "db::get")));
        nucleus.set_aside(&graph, Some(aside));
        while nucleus.advance(0.1) {}
        let was = nucleus.point(aside).unwrap();
        nucleus.set_cursor(&graph, Some(tile(&map, "db::put")));
        while nucleus.advance(0.1) {}
        assert!(
            nucleus.point(aside).unwrap().distance(was) < 1e-4,
            "{id} stays"
        );
        for other in (0..map.tiles.len()).filter(|&t| t != aside && nucleus.drawn(t)) {
            assert!(
                nucleus.point(other).unwrap().distance(was) > Dot::Focus.radius(),
                "{id}: nothing on it"
            );
        }
        nucleus.set_aside(&graph, None);
    }
}

#[test]
fn a_function_set_aside_off_the_journey_is_drawn_and_moves_nothing() {
    let (graph, map) = called_from_outside();
    let mut nucleus = around(&graph, &map, "core::run", 1);
    let resting = |nucleus: &Nucleus| -> (Vec<Option<Ball>>, Vec<Option<Vec3>>) {
        let balls = (0..graph.cluster_count())
            .map(|c| nucleus.ball(c).filter(|b| nucleus.shows_body(b.body)))
            .collect();
        let dots = (0..map.tiles.len())
            .map(|t| nucleus.drawn(t).then(|| nucleus.point(t)).flatten())
            .collect();
        (balls, dots)
    };
    let before = resting(&nucleus);
    // In a module round the nucleus, and in one the journey never enters.
    for id in ["db::other", "ext::poke"] {
        let aside = tile(&map, id);
        assert!(
            !nucleus.drawn(aside),
            "{id}: the journey draws nothing of it"
        );
        nucleus.set_aside(&graph, Some(aside));
        while nucleus.advance(0.1) {}
        assert!(nucleus.drawn(aside), "{id}: set aside, it is drawn");
        let (balls, dots) = resting(&nucleus);
        let ball = nucleus.ball(graph.cluster(aside).unwrap()).unwrap();
        let at = nucleus.point(aside).unwrap();
        assert!(
            at.distance(ball.center) < ball.radius,
            "{id}: in its sphere"
        );
        for (was, now) in before.0.iter().zip(&balls) {
            if let (Some(was), Some(now)) = (was, now) {
                assert!(
                    was.center.distance(now.center) < 1e-4,
                    "{id}: a sphere stays"
                );
            }
        }
        for (tile, (was, now)) in before.1.iter().zip(&dots).enumerate() {
            if tile != aside {
                assert_eq!(was, now, "{id}: nothing else shows, hides or moves");
            }
        }
    }
    nucleus.set_aside(&graph, None);
    while nucleus.advance(0.1) {}
    assert_eq!(resting(&nucleus), before, "let go, as it was");
}

#[test]
fn the_depth_says_what_is_named_in_both_views() {
    let (graph, map) = codebase();
    let run = tile(&map, "core::run");
    let mut nucleus = around(&graph, &map, "core::run", 1);
    let (disk, idle, other) = (
        tile(&map, "disk::write"),
        tile(&map, "core::idle"),
        tile(&map, "db::other"),
    );
    assert!(
        nucleus.level(tile(&map, "web::route")).is_some(),
        "a caller"
    );
    assert!(
        nucleus.level(tile(&map, "db::put")).is_some(),
        "a call the middle makes"
    );
    assert!(
        nucleus.level(disk).is_none() && nucleus.level(idle).is_none(),
        "two calls down"
    );
    let disk_module = module(&graph, &map, "disk::write");
    assert_eq!(
        nucleus.ball(disk_module).map(|b| b.body),
        Some(Body::Closed)
    );

    assert!(nucleus.retrace(&graph, &trail(&graph, run, 2)));
    assert!(
        !nucleus.retrace(&graph, &trail(&graph, run, 2)),
        "no change"
    );
    while nucleus.advance(0.1) {}
    assert!(
        nucleus.level(disk).is_some() && nucleus.level(idle).is_some(),
        "two calls down"
    );
    assert_eq!(nucleus.level(disk), Some(2));
    assert_eq!(nucleus.level(tile(&map, "web::route")), Some(-1));
    assert!(nucleus.level(other).is_none() && nucleus.level(run).is_none());
    assert_eq!(
        nucleus.ball(disk_module).map(|b| b.body),
        Some(Body::Open),
        "a module the journey reaches comes onto the ring"
    );
    let named = |nucleus: &Nucleus| {
        let mut named: Vec<usize> = nucleus.now.named.keys().copied().collect();
        named.sort_unstable();
        named
    };
    let modules = named(&nucleus);
    nucleus.expand(&graph, Some(nucleus.middle()));
    assert_eq!(named(&nucleus), modules, "the expanded view names the same");
    for &tile in &modules {
        assert!(nucleus.placed(tile), "and draws it");
    }
    assert_eq!(
        nucleus.ball(disk_module).map(|b| b.body),
        Some(Body::Open),
        "on the ring round the wall"
    );

    assert!(nucleus.retrace(&graph, &trail(&graph, run, 1)));
    assert!(nucleus.level(disk).is_none(), "a smaller depth lets it go");
    assert_eq!(
        nucleus.ball(disk_module).map(|b| b.body),
        Some(Body::Closed)
    );
}

#[test]
fn expanded_nothing_moves_as_the_depth_or_the_pointer_changes() {
    let (graph, map) = codebase();
    let run = tile(&map, "core::run");
    let mut nucleus = around(&graph, &map, "core::run", 1);
    nucleus.expand(&graph, Some(nucleus.middle()));
    while nucleus.advance(0.1) {}
    let at = |nucleus: &Nucleus| {
        let inside: Vec<Option<Vec3>> = ["core::run", "core::help", "core::idle"]
            .iter()
            .map(|id| nucleus.point(tile(&map, id)))
            .collect();
        let db = nucleus
            .ball(module(&graph, &map, "db::put"))
            .map(|b| b.center);
        (inside, db)
    };
    let before = at(&nucleus);
    let settle = |nucleus: &mut Nucleus| while nucleus.advance(0.1) {};

    assert!(nucleus.retrace(&graph, &trail(&graph, run, 2)));
    settle(&mut nucleus);
    assert_eq!(at(&nucleus), before, "a new depth");
    nucleus.hover(Some(tile(&map, "core::idle")));
    settle(&mut nucleus);
    assert_eq!(at(&nucleus), before, "the pointer on a function");
}

#[test]
fn a_level_more_or_less_shows_and_hides_and_moves_nothing() {
    let (graph, map) = codebase();
    let run = tile(&map, "core::run");
    let mut nucleus = around(&graph, &map, "core::run", 1);
    let places = |nucleus: &Nucleus| {
        let balls: Vec<Vec3> = nucleus.balls().map(|(_, b)| b.center).collect();
        let points: Vec<Option<Vec3>> = (0..graph.positions.len())
            .map(|t| nucleus.point(t))
            .collect();
        (balls, points)
    };
    let disk = module(&graph, &map, "disk::write");
    let drawn = |nucleus: &Nucleus, cluster: usize| {
        nucleus
            .ball(cluster)
            .is_some_and(|b| nucleus.shows_body(b.body))
    };
    assert!(!drawn(&nucleus, disk), "disk is two calls down: not yet");
    let (before, shown) = (places(&nucleus), nucleus.placed(tile(&map, "db::put")));
    assert!(shown);

    assert!(nucleus.retrace(&graph, &trail(&graph, run, 2)));
    while nucleus.advance(0.1) {}
    assert!(drawn(&nucleus, disk), "a level more brings disk in");
    assert!(nucleus.placed(tile(&map, "disk::write")));
    assert_eq!(places(&nucleus), before, "where it was kept for it");

    assert!(nucleus.retrace(&graph, &trail(&graph, run, 1)));
    while nucleus.advance(0.1) {}
    assert!(!drawn(&nucleus, disk), "and a level less takes it out");
    assert_eq!(places(&nucleus), before, "moving nothing");
}

#[test]
fn expanding_keeps_the_ring_in_its_order() {
    let (graph, map) = codebase();
    let mut nucleus = around(&graph, &map, "core::run", 2);
    // The modules round the middle, clockwise from the top as the usual view sees
    // them.
    let clockwise = |nucleus: &Nucleus| {
        let rotation = view_rotation();
        let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
        let core = nucleus.ball(nucleus.middle()).unwrap().center;
        let mut ring: Vec<(f32, usize)> = nucleus
            .balls()
            .filter(|&(c, b)| c != nucleus.middle() && b.body == Body::Open)
            .map(|(c, b)| {
                let at = b.center - core;
                let angle = at.dot(up).atan2(at.dot(right));
                (
                    (std::f32::consts::FRAC_PI_2 - angle).rem_euclid(std::f32::consts::TAU),
                    c,
                )
            })
            .collect();
        ring.sort_by(|a, b| a.0.total_cmp(&b.0));
        ring.into_iter().map(|(_, c)| c).collect::<Vec<usize>>()
    };
    let before = clockwise(&nucleus);
    assert_eq!(before.len(), 5, "db, disk, log, ext and web: {before:?}");
    assert_eq!(
        before[0],
        module(&graph, &map, "db::put"),
        "the first the journey enters, at the top"
    );
    nucleus.expand(&graph, Some(nucleus.middle()));
    while nucleus.advance(0.1) {}
    assert_eq!(clockwise(&nucleus), before, "expanded");
    nucleus.expand(&graph, None);
    while nucleus.advance(0.1) {}
    assert_eq!(clockwise(&nucleus), before, "and back");
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
fn a_journey_spreads_its_functions_over_their_spheres_clear_of_each_others_calls() {
    // The start in the middle of its sphere calls four functions in a sphere off to
    // its right, which call each other in a chain.
    let rotation = view_rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    let rooms = [
        Room {
            cluster: 0,
            center: Vec3::ZERO,
            radius: 6.0,
            tiles: Vec::new(),
            middle: Some(0),
            spare: 0,
        },
        Room {
            cluster: 1,
            center: right * 30.0,
            radius: 6.0,
            tiles: vec![1, 2, 3, 4],
            middle: None,
            spare: 3,
        },
    ];
    let calls = [(0, 1), (0, 2), (0, 3), (0, 4), (1, 2), (2, 3)];
    let placed = place(&rooms, &calls, &|_| 8);
    let flat = |at: Vec3| Vec2::new(at.dot(right), at.dot(up));
    let at = |tile: usize| flat(placed.points[&tile]);
    assert!(at(0).length() < 1e-4, "the start in the middle");
    for tile in 1..=4 {
        assert!(
            at(tile).distance(flat(right * 30.0)) < atlas::disc(6.0) + 1e-3,
            "inside its sphere"
        );
    }
    let across: Vec<f32> = (1..=4).map(|t| at(t).x).collect();
    let down: Vec<f32> = (1..=4).map(|t| at(t).y).collect();
    let spread = |v: &[f32]| {
        v.iter().fold(f32::MIN, |m, &x| m.max(x)) - v.iter().fold(f32::MAX, |m, &x| m.min(x))
    };
    assert!(
        spread(&across) > 1.0 && spread(&down) > 1.0,
        "over the sphere, not a column"
    );
    // No call's line runs through a third dot.
    for &(from, to) in &calls {
        for third in (0..=4).filter(|&t| t != from && t != to) {
            let (a, b, c) = (at(from), at(to), at(third));
            let along = b - a;
            let t = ((c - a).dot(along) / along.length_squared()).clamp(0.0, 1.0);
            assert!(
                (a + along * t).distance(c) > Dot::Focus.radius(),
                "{from}→{to} through {third}"
            );
        }
    }
    assert_eq!(placed.free[&1].len(), 3, "room kept for outside callers");
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
