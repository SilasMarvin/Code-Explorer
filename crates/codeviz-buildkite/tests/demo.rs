//! The demo pipeline imports into a valid document with the expected shape.

// Helpers outside `#[test]` fns are not exempt; a panic is the failure report here.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use codeviz_buildkite::{Error, import, import_file};
use codeviz_ir::{Document, Mode, Role, Step, Unit, UnitId, validate};

fn demo() -> Document {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo/pipelines/deploy.yml");
    import_file(&path, "orders-deploy").unwrap()
}

fn unit<'d>(document: &'d Document, id: &str) -> &'d Unit {
    document.unit(&UnitId::new(id)).unwrap()
}

fn dag(unit: &Unit) -> &[codeviz_ir::DagNode] {
    match unit.body.as_deref() {
        Some([Step::Dag { nodes }]) => nodes,
        other => panic!("expected a single dag, got {other:?}"),
    }
}

#[test]
fn demo_is_valid_and_deterministic() {
    let document = demo();
    let diagnostics = validate(&document);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert_eq!(document.to_json().unwrap(), demo().to_json().unwrap());
}

#[test]
fn steps_are_children_of_the_pipeline_or_their_group() {
    let document = demo();
    let pipeline = unit(&document, "buildkite:orders-deploy");
    assert_eq!(pipeline.role, Role::Invocable);
    assert_eq!(document.entries[0].kind, "ci_trigger");

    let group = unit(&document, "buildkite:orders-deploy/deploy");
    assert_eq!(group.role, Role::Group);
    let smoke = unit(&document, "buildkite:orders-deploy/smoke");
    assert_eq!(smoke.parent.as_ref(), Some(&group.id));
    assert!(
        document
            .unit(&UnitId::new("buildkite:orders-deploy/cargo-audit"))
            .is_some()
    );
}

#[test]
fn correlated_conditions_share_one_guard() {
    let document = demo();
    let guards: Vec<_> = dag(unit(&document, "buildkite:orders-deploy"))
        .iter()
        .filter_map(|node| match node.body.first() {
            Some(Step::Branch { guard, .. }) => Some(guard.clone().unwrap_or_default()),
            _ => None,
        })
        .collect();
    assert_eq!(guards.len(), 7, "{guards:?}");
    assert!(
        guards
            .iter()
            .all(|guard| guard == r#"build.branch == "main""#)
    );
}

#[test]
fn soft_failures_and_async_triggers_do_not_propagate() {
    let document = demo();
    let pipeline = unit(&document, "buildkite:orders-deploy");
    let propagates = |step: &str| {
        pipeline
            .calls
            .iter()
            .find(|call| call.text == step)
            .unwrap()
            .propagates
    };
    assert!(propagates("build"));
    assert!(!propagates("release"), "a gate cannot fail");

    let checks = &unit(&document, "buildkite:orders-deploy/post-deploy").calls[0];
    assert_eq!(checks.mode, Mode::Spawn);
    assert!(!checks.propagates);
    let migrate = &unit(&document, "buildkite:orders-deploy/migrate").calls[0];
    assert_eq!(migrate.mode, Mode::Call);
    assert!(migrate.propagates);
}

#[test]
fn soft_fail_true_clears_propagation() {
    let document = import("steps:\n  - {command: make lint, soft_fail: true}\n", "ci").unwrap();
    let pipeline = unit(&document, "buildkite:ci");
    assert_eq!(pipeline.calls[0].text, "make-lint");
    assert!(!pipeline.calls[0].propagates);
}

#[test]
fn rejects_invalid_slugs() {
    assert!(matches!(
        import("steps: []", "Orders Deploy"),
        Err(Error::InvalidSlug(_))
    ));
}
