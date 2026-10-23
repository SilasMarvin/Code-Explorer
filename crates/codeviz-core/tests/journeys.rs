//! Scenario enumeration and journey unrolling over small hand-written documents.

#![allow(clippy::unwrap_used, reason = "test helpers fail loudly")]

use codeviz_core::{Journey, JourneyOptions, SpanTarget, System, Zoom, scenarios};
use codeviz_ir::{Document, ExitKind, UnitId};

/// `handler → place → (find ?, check ?, quote ?) → save`, where `find` fails through an
/// unresolved call, `check` and `price` exit with errors, and `quote` only optionally
/// calls `discount`.
const ORDERS: &str = r#"{
  "schema": "codeviz/0.1", "producer": { "name": "test", "version": "0" },
  "units": [
    { "id": "t:handler", "name": "handler", "kind": "fn", "role": "invocable",
      "calls": [{ "key": "place#0", "text": "place", "propagates": true,
                  "targets": [{ "unit": "t:place", "certainty": "exact" }] }] },
    { "id": "t:place", "name": "place", "kind": "fn", "role": "invocable",
      "calls": [
        { "key": "find#0", "text": "find", "propagates": true, "targets": [{ "unit": "t:find", "certainty": "exact" }] },
        { "key": "check#0", "text": "check", "propagates": true, "targets": [{ "unit": "t:check", "certainty": "exact" }] },
        { "key": "quote#0", "text": "quote", "propagates": true, "targets": [{ "unit": "t:quote", "certainty": "exact" }] },
        { "key": "save#0", "text": "save", "targets": [{ "unit": "t:save", "certainty": "exact" }] }
      ] },
    { "id": "t:find", "name": "find", "kind": "fn", "role": "invocable",
      "calls": [{ "key": ".ok_or#0", "text": ".ok_or", "propagates": true, "targets": [] }] },
    { "id": "t:check", "name": "check", "kind": "fn", "role": "invocable",
      "body": [{ "step": "branch", "guard": "in range", "arms": [
        { "label": "no", "body": [{ "step": "exit", "kind": "error", "label": "bad quantity" }] },
        { "label": "yes", "body": [] } ] }] },
    { "id": "t:quote", "name": "quote", "kind": "fn", "role": "invocable",
      "calls": [
        { "key": "price#0", "text": "price", "propagates": true, "targets": [{ "unit": "t:price", "certainty": "exact" }] },
        { "key": "discount#0", "text": "discount", "targets": [{ "unit": "t:discount", "certainty": "exact" }] }
      ],
      "body": [
        { "step": "call", "call": "price#0" },
        { "step": "branch", "guard": "bulk", "arms": [
          { "label": "yes", "body": [{ "step": "call", "call": "discount#0" }] },
          { "label": "no", "body": [] } ] }
      ] },
    { "id": "t:price", "name": "price", "kind": "fn", "role": "invocable",
      "body": [{ "step": "branch", "guard": "match sku", "arms": [
        { "label": "known", "body": [] },
        { "label": "other", "body": [{ "step": "exit", "kind": "error", "label": "unknown sku" }] } ] }] },
    { "id": "t:discount", "name": "discount", "kind": "fn", "role": "invocable" },
    { "id": "t:save", "name": "save", "kind": "fn", "role": "invocable" }
  ]
}"#;

fn system(json: &str) -> System {
    System::new([Document::from_json(json).unwrap()])
}

fn names(system: &System, journey: &Journey) -> Vec<String> {
    journey
        .spans()
        .map(|(_, span)| match &span.target {
            SpanTarget::Unit(id) => system.unit(id).map(|u| u.name.clone()).unwrap_or_default(),
            SpanTarget::Unresolved(text) => text.clone(),
            SpanTarget::Ambiguous(_) => "?".to_owned(),
        })
        .collect()
}

#[test]
fn scenarios_are_distinct_exits_with_their_chains() {
    let system = system(ORDERS);
    let found = scenarios(&system, &UnitId::new("t:handler"), &Zoom::default());
    let summary: Vec<(ExitKind, &str, usize)> = found
        .iter()
        .map(|s| (s.outcome.kind, s.outcome.label.as_str(), s.chain.len()))
        .collect();
    assert_eq!(
        summary,
        vec![
            (ExitKind::Return, "completes", 0),
            (ExitKind::Error, ".ok_or fails", 3),
            (ExitKind::Error, "bad quantity", 3),
            (ExitKind::Error, "unknown sku", 4),
        ]
    );
}

#[test]
fn scenario_journeys_stop_at_the_exit() {
    let system = system(ORDERS);
    let entry = UnitId::new("t:handler");
    let found = scenarios(&system, &entry, &Zoom::default());
    let unroll = |index: usize| {
        Journey::unroll(&system, &entry, found.get(index), JourneyOptions::default()).unwrap()
    };

    // Completing never takes an erroring arm, but keeps optional calls.
    assert_eq!(
        names(&system, &unroll(0)),
        [
            "handler", "place", "find", "check", "quote", "price", "discount", "save"
        ]
    );
    // The unresolved call that fails is shown even though unresolved calls are hidden.
    assert_eq!(
        names(&system, &unroll(1)),
        ["handler", "place", "find", ".ok_or"]
    );
    let unknown_sku = unroll(3);
    assert_eq!(
        names(&system, &unknown_sku),
        ["handler", "place", "find", "check", "quote", "price"]
    );
    let price = unknown_sku.spans().last().unwrap().1;
    assert_eq!(price.exits.len(), 1);
    assert_eq!(price.exits[0].label.as_deref(), Some("unknown sku"));
}

#[test]
fn branch_arms_overlap_in_time() {
    let system = system(ORDERS);
    let journey = Journey::unroll(
        &system,
        &UnitId::new("t:quote"),
        None,
        JourneyOptions::default(),
    )
    .unwrap();
    let ticks: Vec<(u32, u32)> = journey.spans().map(|(_, s)| (s.enter, s.exit)).collect();
    // `price` may exit (one tick), then `discount` overlaps the arm that skips it.
    assert_eq!(ticks, vec![(0, 4), (1, 3), (3, 4)]);
    let discount = journey.spans().last().unwrap().1;
    assert_eq!(discount.conditions.len(), 1);
    assert_eq!(discount.conditions[0].arm, "yes");
}

#[test]
fn dag_nodes_start_after_their_dependencies() {
    let json = r#"{
      "schema": "codeviz/0.1", "producer": { "name": "test", "version": "0" },
      "units": [
        { "id": "ci:p", "name": "p", "kind": "pipeline", "role": "invocable",
          "calls": [
            { "key": "lint#0", "text": "lint", "targets": [{ "unit": "ci:lint", "certainty": "declared" }] },
            { "key": "build#0", "text": "build", "targets": [{ "unit": "ci:build", "certainty": "declared" }] },
            { "key": "deploy#0", "text": "deploy", "targets": [{ "unit": "ci:deploy", "certainty": "declared" }] },
            { "key": "notify#0", "text": "notify", "mode": "spawn", "targets": [{ "unit": "ci:notify", "certainty": "declared" }] }
          ],
          "body": [
            { "step": "dag", "nodes": [
              { "id": "lint", "body": [{ "step": "call", "call": "lint#0" }] },
              { "id": "build", "body": [{ "step": "call", "call": "build#0" }] },
              { "id": "deploy", "after": ["lint", "build"], "body": [{ "step": "call", "call": "deploy#0" }] }
            ] },
            { "step": "call", "call": "notify#0" }
          ] },
        { "id": "ci:lint", "name": "lint", "kind": "step", "role": "invocable" },
        { "id": "ci:build", "name": "build", "kind": "step", "role": "invocable",
          "calls": [{ "key": "compile#0", "text": "compile", "targets": [{ "unit": "ci:compile", "certainty": "declared" }] }] },
        { "id": "ci:compile", "name": "compile", "kind": "step", "role": "invocable" },
        { "id": "ci:deploy", "name": "deploy", "kind": "step", "role": "invocable" },
        { "id": "ci:notify", "name": "notify", "kind": "step", "role": "invocable" }
      ]
    }"#;
    let system = system(json);
    let journey = Journey::unroll(
        &system,
        &UnitId::new("ci:p"),
        None,
        JourneyOptions::default(),
    )
    .unwrap();
    let ticks: Vec<(String, u32)> = names(&system, &journey)
        .into_iter()
        .zip(journey.spans().map(|(_, s)| s.enter))
        .collect();
    assert_eq!(
        ticks,
        vec![
            ("p".to_owned(), 0),
            ("lint".to_owned(), 1),
            ("build".to_owned(), 1),
            ("compile".to_owned(), 2),
            ("deploy".to_owned(), 3),
            ("notify".to_owned(), 4),
        ]
    );
}

#[test]
fn spans_that_do_the_same_share_a_shape() {
    // `main → (load, load, render → load if dirty, walk → walk)`, where `load` reads
    // and `walk` recurses.
    let json = r#"{
      "schema": "codeviz/0.1", "producer": { "name": "test", "version": "0" },
      "units": [
        { "id": "t:main", "name": "main", "kind": "fn", "role": "invocable",
          "calls": [
            { "key": "load#0", "text": "load", "targets": [{ "unit": "t:load", "certainty": "exact" }] },
            { "key": "load#1", "text": "load", "targets": [{ "unit": "t:load", "certainty": "exact" }] },
            { "key": "render#0", "text": "render", "targets": [{ "unit": "t:render", "certainty": "exact" }] },
            { "key": "walk#0", "text": "walk", "targets": [{ "unit": "t:walk", "certainty": "exact" }] }
          ] },
        { "id": "t:load", "name": "load", "kind": "fn", "role": "invocable",
          "calls": [{ "key": "read#0", "text": "read", "targets": [{ "unit": "t:read", "certainty": "exact" }] }] },
        { "id": "t:read", "name": "read", "kind": "fn", "role": "invocable" },
        { "id": "t:render", "name": "render", "kind": "fn", "role": "invocable",
          "calls": [{ "key": "load#0", "text": "load", "targets": [{ "unit": "t:load", "certainty": "exact" }] }],
          "body": [{ "step": "branch", "guard": "dirty", "arms": [
            { "label": "yes", "body": [{ "step": "call", "call": "load#0" }] },
            { "label": "no", "body": [] } ] }] },
        { "id": "t:walk", "name": "walk", "kind": "fn", "role": "invocable",
          "calls": [{ "key": "walk#0", "text": "walk", "targets": [{ "unit": "t:walk", "certainty": "exact" }] }] }
      ]
    }"#;
    let system = system(json);
    let journey = Journey::unroll(
        &system,
        &UnitId::new("t:main"),
        None,
        JourneyOptions::default(),
    )
    .unwrap();
    assert_eq!(
        names(&system, &journey),
        [
            "main", "load", "read", "load", "read", "render", "load", "read", "walk", "walk"
        ]
    );
    let shapes = journey.shapes();
    assert_eq!(shapes.len(), journey.len());
    let [
        _,
        load,
        read,
        load_again,
        read_again,
        render,
        load_if,
        _,
        walk,
        walk_inner,
    ] = shapes.as_slice()
    else {
        panic!("ten spans");
    };
    assert_eq!(load, load_again, "the same call twice");
    assert_eq!(
        load, load_if,
        "only sometimes, but doing the same when it runs"
    );
    assert_eq!(read, read_again);
    assert_ne!(load, read);
    assert_ne!(render, load);
    assert_ne!(
        walk, walk_inner,
        "the inner walk stops at the recursion; the outer one goes on to it"
    );
}
