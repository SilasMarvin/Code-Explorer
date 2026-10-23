//! What a view means to show, written beside its screenshot so a reader's account of
//! the picture can be checked against it: the examined function, every disclosed call
//! with its number, the module it lives in, whether it can fail or only sometimes
//! runs, and what a diff changed.

use codeviz_core::Change;
use serde_json::{Value, json};

use crate::{
    diff_view::DiffView,
    explorer::{Explorer, is_conditional},
    ui::node_name,
};

/// The view's content, as JSON.
pub(crate) fn of(explorer: &Explorer, diff: Option<&DiffView>) -> Value {
    let disclosed = explorer.disclosure();
    let track = explorer.track();
    let tile_of = |span| track.unit_id(span).and_then(|u| explorer.map().tile_of(u));
    let change = |tile: Option<usize>| {
        diff.zip(tile)
            .and_then(|(diff, tile)| diff.change(tile))
            .map(name_of)
    };
    let mut calls: Vec<(Vec<u32>, Value)> = disclosed
        .numbered()
        .filter(|(_, number)| !number.is_empty())
        .filter_map(|(span_id, number)| {
            let span = track.span(span_id)?;
            let name = track.unit_id(span_id).map(|u| explorer.unit_name(u))?;
            let parent = number.rsplit_once('.').map_or("", |(parent, _)| parent);
            let module = tile_of(span_id)
                .and_then(|tile| explorer.graph().cluster(tile))
                .map(|cluster| explorer.cluster_name(cluster));
            let call = json!({
                "number": number,
                "name": name,
                "parent": parent,
                "module": module,
                "change": change(tile_of(span_id)),
                "conditional": is_conditional(span),
                "fails": disclosed.fails(span_id),
                "hidden_calls": disclosed.folded(span_id),
            });
            Some((
                number.split('.').filter_map(|n| n.parse().ok()).collect(),
                call,
            ))
        })
        .collect();
    calls.sort_by(|a, b| a.0.cmp(&b.0));

    let root = explorer.journey().map(codeviz_core::Journey::root);
    let anchor = explorer.anchor();
    let changes = diff.map(|diff| {
        let mut by_kind = serde_json::Map::new();
        for kind in [
            Change::Added,
            Change::Removed,
            Change::Modified,
            Change::Moved,
        ] {
            let mut names: Vec<String> = diff
                .changed()
                .filter(|&(_, c)| c == kind)
                .filter_map(|(tile, _)| explorer.tile_unit(tile))
                .map(|unit| explorer.unit_name(unit))
                .collect();
            names.sort();
            by_kind.insert(name_of(kind).to_owned(), json!(names));
        }
        Value::Object(by_kind)
    });
    json!({
        "focus": explorer.root().map(|u| explorer.unit_name(u)),
        "focus_change": change(anchor),
        "focus_fails": root.is_some_and(|r| disclosed.fails(r)),
        "focus_hidden_calls": root.map_or(0, |r| disclosed.folded(r)),
        "callers": anchor.map_or_else(Vec::new, |tile| {
            explorer
                .callers_of(tile)
                .into_iter()
                // Only those the lens shows, as the graph names them.
                .filter(|&caller| explorer.visibility(caller) >= 0.5)
                .map(|caller| node_name(explorer, caller))
                .collect()
        }),
        "calls": calls.into_iter().map(|(_, call)| call).collect::<Vec<_>>(),
        "changes": changes,
        "review": diff.map(|diff| review(explorer, diff)),
    })
}

/// Where stepping through a diff stands: the changes in reading order, which of them
/// is focused, the entry points the way up from it reaches, and what the graph does
/// not draw.
fn review(explorer: &Explorer, diff: &DiffView) -> Value {
    let name = |tile: usize| explorer.tile_unit(tile).map(|u| explorer.unit_name(u));
    let focus = explorer.graph_focus();
    json!({
        "order": diff.order().iter().map(|&t| name(t)).collect::<Vec<_>>(),
        "at": focus.and_then(|f| diff.place(f)).map(|at| at + 1),
        "route_ends": diff.route().map_or_else(Vec::new, |route| {
            route.ends.iter().map(|&t| node_name(explorer, t)).collect()
        }),
        "not_drawn": diff.offstage.as_ref().map(|files| {
            files
                .iter()
                .map(|f| json!({ "path": f.path, "places": f.places }))
                .collect::<Vec<_>>()
        }),
    })
}

fn name_of(change: Change) -> &'static str {
    match change {
        Change::Added => "added",
        Change::Removed => "removed",
        Change::Modified => "modified",
        Change::Moved => "moved",
    }
}

#[cfg(test)]
mod tests {
    use codeviz_ir::UnitId;

    use super::*;

    #[test]
    fn lists_the_disclosed_calls_in_order_with_what_is_folded() {
        let mut explorer = crate::explorer::tests::explorer();
        let main = explorer.map().tile_of(&UnitId::new("x:main")).unwrap();
        explorer.open_at(main);
        let truth = of(&explorer, None);
        assert_eq!(truth["focus"], "main");
        let calls: Vec<(&str, &str, u64)> = truth["calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["number"].as_str().unwrap(),
                    c["name"].as_str().unwrap(),
                    c["hidden_calls"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(calls, [("1", "a", 1), ("2", "b", 0)], "c is folded under a");
    }

    /// `place → (check, quote → (price, discount if bulk))`, where `check` and `price`
    /// can fail.
    const ORDERS: &str = r#"{
      "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" },
      "units": [
        { "id": "t:place", "name": "place", "kind": "fn", "role": "invocable",
          "calls": [
            { "key": "check#0", "text": "check", "propagates": true, "targets": [{ "unit": "t:check", "certainty": "exact" }] },
            { "key": "quote#0", "text": "quote", "propagates": true, "targets": [{ "unit": "t:quote", "certainty": "exact" }] }
          ] },
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
        { "id": "t:discount", "name": "discount", "kind": "fn", "role": "invocable" }
      ]
    }"#;

    #[test]
    fn a_failure_marks_the_nearest_shown_call_and_conditions_mark_theirs() {
        let system = codeviz_core::System::new([codeviz_ir::Document::from_json(ORDERS).unwrap()]);
        let mut explorer = Explorer::new(
            system,
            Some(UnitId::new("t:place")),
            codeviz_core::JourneyOptions::default(),
        );
        let place = explorer.map().tile_of(&UnitId::new("t:place")).unwrap();
        explorer.open_at(place);
        let marks = |explorer: &Explorer| -> Vec<(String, bool, bool)> {
            of(explorer, None)["calls"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| {
                    (
                        format!(
                            "{} {}",
                            c["number"].as_str().unwrap(),
                            c["name"].as_str().unwrap()
                        ),
                        c["fails"].as_bool().unwrap(),
                        c["conditional"].as_bool().unwrap(),
                    )
                })
                .collect()
        };
        assert_eq!(
            marks(&explorer),
            [
                ("1 check".to_owned(), true, false),
                ("2 quote".to_owned(), true, false),
            ],
            "price fails under quote, folded away"
        );
        explorer.set_disclose(crate::disclosure::Disclose::Depth(2));
        assert_eq!(
            marks(&explorer),
            [
                ("1 check".to_owned(), true, false),
                ("2 quote".to_owned(), false, false),
                ("2.1 price".to_owned(), true, false),
                ("2.2 discount".to_owned(), false, true),
            ],
            "once price shows, it carries its own failure"
        );
    }
}
