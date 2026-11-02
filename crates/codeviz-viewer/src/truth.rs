//! What a view means to show, written beside its screenshot so a reader's account of
//! the picture can be checked against it: the examined function, every disclosed call
//! with its number, the module it lives in, and whether it can fail or only sometimes
//! runs. A scripted session's snapshots say more: what is
//! drawn where on screen, by name, and what can be clicked ([`scene`]).

use bevy::{
    color::Color,
    math::{Vec2, Vec3},
};
use bevy_egui::egui;
use serde_json::{Value, json};

use crate::{
    drive::{Drawn, Painted, Screen, View},
    explorer::{Explorer, is_conditional},
    nucleus::{Flow, Nucleus},
    ui::node_name,
};

/// The view's content, as JSON.
pub(crate) fn of(explorer: &Explorer) -> Value {
    let disclosed = explorer.disclosure();
    let track = explorer.track();
    let tile_of = |span| track.unit_id(span).and_then(|u| explorer.map().tile_of(u));
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
    // Browsing, what the view examines is the function selected.
    let anchor = explorer.anchor().or_else(|| explorer.selection());
    json!({
        "focus": match explorer.selection() {
            Some(tile) => explorer.tile_unit(tile).map(|u| explorer.unit_name(u)),
            None => explorer.root().map(|u| explorer.unit_name(u)),
        },
        "focus_fails": root.is_some_and(|r| disclosed.fails(r)),
        "focus_hidden_calls": root.map_or(0, |r| disclosed.folded(r)),
        "callers": anchor.map_or_else(Vec::new, |tile| {
            explorer
                .callers_of(tile)
                .into_iter()
                .map(|caller| node_name(explorer, caller))
                .collect()
        }),
        "calls": calls.into_iter().map(|(_, call)| call).collect::<Vec<_>>(),
    })
}

/// The view's state in words: what is focused and centred, the journey's cursor and
/// depth, what is hidden, which panels show, and where back and forward lead.
pub(crate) fn state(view: &View) -> Value {
    let explorer = &*view.explorer;
    let name = |tile: usize| node_name(explorer, tile);
    let nucleus = explorer.nucleus();
    let disclosed = explorer.disclosure();
    // Expanded in a journey, the view centres on the cursor; the module in the middle
    // is the start's still.
    let centre = nucleus.centre();
    let middle = nucleus.focus();
    json!({
        "mode": if explorer.browsing() { "browse" } else { "journey" },
        "selection": explorer.selection().map(name),
        "focus": explorer.graph_focus().map(name),
        "centre": centre.map(name),
        "centre_module": middle.and_then(|t| module_of(explorer, t)),
        "nucleus": if nucleus.expanded() { "expanded" } else { "modules" },
        "journey_start": explorer.root().map(|u| explorer.unit_name(u)),
        "cursor": explorer.selected().map(|span| json!({
            "number": disclosed.number(span),
            "name": explorer.label(span),
        })),
        "aside": explorer.aside().map(name),
        "depth": explorer.disclose().0,
        "outside_callers": explorer.outside_shown(),
        "pointed": explorer.pointed().map(name),
        "under_pointer": explorer.hovered_tile.map(name),
        "search": view.chrome.search,
        "help": view.help.0,
        "back": explorer.can_go_back(),
        "forward": explorer.can_go_forward(),
        "camera": view.orbit_pose().map(|pose| {
            let (target, distance, (yaw, pitch)) = pose.parts();
            json!({
                "target": fine3(target),
                "distance": fine(distance),
                "yaw": fine(yaw),
                "pitch": fine(pitch),
            })
        }),
    })
}

/// What a snapshot shows, by name and place on screen (window points, `[x0, y0, x1,
/// y1]` for a rectangle): every function drawn with its roles and its label, every
/// module sphere, every arrow, every label with what it overlaps, the journey's bars,
/// the buttons, the overlays, any other text, and [`state`] and [`of`] besides.
pub(crate) fn scene(view: &View) -> Value {
    let explorer = &*view.explorer;
    let screen = &*view.screen;
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, screen.window);
    let near = |a: egui::Rect, b: egui::Rect| {
        (a.min - b.min).length() < 1.5 && (a.max - b.max).length() < 1.5
    };
    // Graph labels are clipped to the graph; the rest is chrome.
    let (labels, chrome): (Vec<&Painted>, Vec<&Painted>) = screen
        .texts
        .iter()
        .filter(|t| t.opacity > 0.05)
        .partition(|t| near(t.clip, screen.graph));
    let drawn = view.drawn();
    let nucleus = explorer.nucleus();
    let balls: Vec<(usize, Vec2, f32)> = nucleus
        .balls()
        .filter_map(|(cluster, ball)| {
            let at = view.to_screen(ball.center)?;
            Some((cluster, at, view.screen_radius(ball.center, ball.radius)))
        })
        .collect();
    let named = match_labels(explorer, &labels, &drawn, &balls);
    let label_of = |what: Named| named.iter().position(|&n| n == Some(what));

    let nodes: Vec<Value> = drawn
        .iter()
        .map(|d| {
            let label = label_of(Named::Node(d.tile));
            json!({
                "name": node_name(explorer, d.tile),
                "unit": explorer.tile_unit(d.tile).map(ToString::to_string),
                "module": module_of(explorer, d.tile),
                "roles": roles(explorer, d.tile),
                "numbers": numbers(explorer, d.tile),
                "at": point(d.at),
                "radius": round(d.radius),
                "world": fine3(d.world),
                "world_radius": fine(d.world_radius),
                "on_screen": d.on_screen,
                "label": label.map(|i| labels[i].text.clone()),
                "label_rect": label.map(|i| rect(labels[i].rect)),
                "crowded_by": crowding(explorer, &drawn, d),
            })
        })
        .collect();
    let modules: Vec<Value> = balls
        .iter()
        .filter_map(|&(cluster, at, radius)| {
            let n = nucleus;
            let ball = n.ball(cluster)?;
            let label = label_of(Named::Module(cluster));
            Some(json!({
                "name": explorer.cluster_name(cluster),
                "body": format!("{:?}", ball.body).to_lowercase(),
                "functions": n.count(cluster),
                "named": n.opened(cluster),
                "shown": n.shows_body(ball.body),
                "at": point(at),
                "world": fine3(ball.center),
                "radius": round(radius),
                "world_radius": fine(ball.radius),
                "on_screen": view.on_screen(at),
                "label": label.map(|i| labels[i].text.clone()),
                "label_rect": label.map(|i| rect(labels[i].rect)),
            }))
        })
        .collect();
    let overlays: Vec<Value> = screen
        .overlays
        .iter()
        .map(|(name, r)| json!({ "name": name, "rect": rect(*r), "clipped": !window.contains_rect(*r) }))
        .collect();
    let label_list = label_entries(explorer, screen, &labels, &named);
    let mislabelled = mislabelled(explorer, &labels, &named, &drawn);
    let help = view.help.0.then(|| {
        screen
            .overlays
            .iter()
            .find(|(name, _)| name.starts_with("key help"))
            .map(|(_, r)| json!({ "rect": rect(*r), "clipped": !window.contains_rect(*r) }))
    });
    json!({
        "window": [screen.window.x, screen.window.y],
        "graph_area": rect(screen.graph),
        "camera": camera(view),
        "state": state(view),
        "nodes": nodes,
        "modules": modules,
        "arrows": arrows(view),
        "module_arrows": module_arrows(explorer, nucleus),
        "labels": label_list,
        "mislabelled": mislabelled,
        "bars": bars(view),
        "buttons": screen.widgets.iter().map(|w| json!({
            "text": w.text, "rect": rect(w.rect), "enabled": w.enabled, "cut_off": w.cut_off(),
        })).collect::<Vec<_>>(),
        "overlays": overlays,
        "help": help.flatten(),
        "text": chrome.iter().map(|t| json!({ "text": t.text, "rect": rect(t.rect), "size": t.size })).collect::<Vec<_>>(),
        "content": of(explorer),
    })
}

/// Where the camera is in the world, and which way it faces.
fn camera(view: &View) -> Value {
    let Some((at, facing)) = view.camera() else {
        return Value::Null;
    };
    let ahead = facing * Vec3::NEG_Z;
    json!({
        "at": [round(at.x), round(at.y), round(at.z)],
        "facing": fine3(ahead),
    })
}

/// Every label on the graph: what it says and names, where, which others it
/// overlaps, whether it is cut off, and what lies over it.
fn label_entries(
    explorer: &Explorer,
    screen: &Screen,
    labels: &[&Painted],
    named: &[Option<Named>],
) -> Vec<Value> {
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, screen.window);
    labels
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let crossed: Vec<usize> = labels
                .iter()
                .enumerate()
                .filter(|&(other_index, other)| {
                    other_index != index && other.rect.shrink(1.0).intersects(text.rect.shrink(1.0))
                })
                .map(|(other_index, _)| other_index)
                .collect();
            let under: Vec<&str> = screen
                .overlays
                .iter()
                .filter(|(_, r)| r.intersects(text.rect))
                .map(|(name, _)| name.as_str())
                .collect();
            let (kind, of) = match named[index] {
                Some(Named::Node(tile)) => ("function", Some(node_name(explorer, tile))),
                Some(Named::Module(cluster)) => ("module", Some(explorer.cluster_name(cluster))),
                None => ("other", None),
            };
            json!({
                "index": index,
                "text": text.text,
                "rect": rect(text.rect),
                "names": kind,
                "of": of,
                "overlaps": crossed,
                "cut_off": !text.clip.contains_rect(text.rect) || !window.contains_rect(text.rect),
                "under": under,
                "opacity": round(text.opacity),
                "size": text.size,
            })
        })
        .collect()
}

/// Every function's label that lies nearer another named node than its own, edge to
/// edge, so a reader would tie it to the other: what it says, whose it is, which node
/// it sits nearer, and both gaps in points.
fn mislabelled(
    explorer: &Explorer,
    labels: &[&Painted],
    named: &[Option<Named>],
    drawn: &[Drawn],
) -> Vec<Value> {
    let gap = |rect: egui::Rect, d: &Drawn| {
        (rect.distance_to_pos(egui::pos2(d.at.x, d.at.y)) - d.radius).max(0.0)
    };
    let nodes: Vec<&Drawn> = drawn
        .iter()
        .filter(|d| d.on_screen && !speck(explorer, d.tile))
        .collect();
    labels
        .iter()
        .zip(named)
        .filter_map(|(label, what)| {
            let Some(Named::Node(tile)) = *what else {
                return None;
            };
            let own = drawn.iter().find(|d| d.tile == tile)?;
            let mine = gap(label.rect, own);
            let nearest = nodes
                .iter()
                .filter(|d| d.tile != tile)
                .map(|d| (d, gap(label.rect, d)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            match nearest {
                Some((other, theirs)) if theirs + 1.0 < mine => Some(json!({
                    "text": label.text,
                    "of": node_name(explorer, tile),
                    "nearer": node_name(explorer, other.tile),
                    "own_gap": round(mine),
                    "nearer_gap": round(theirs),
                })),
                // Far from every node, its own included, it names nothing a reader
                // can see.
                _ if mine > FAR_LABEL => Some(json!({
                    "text": label.text,
                    "of": node_name(explorer, tile),
                    "nearer": Value::Null,
                    "own_gap": round(mine),
                })),
                _ => None,
            }
        })
        .collect()
}

/// How far, in points, a function's label may sit from its node before it counts as
/// naming nothing a reader can see.
const FAR_LABEL: f32 = 40.0;

/// Whether a function is drawn only as a faint speck.
fn speck(explorer: &Explorer, tile: usize) -> bool {
    let n = explorer.nucleus();
    !n.lit(tile) && n.focus() != Some(tile) && n.hovered() != Some(tile)
}

/// What a label on the graph names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Named {
    Node(usize),
    Module(usize),
}

/// Which label names which function or module: the nearest label that carries its
/// name, each label taken once, nearest pairs first.
fn match_labels(
    explorer: &Explorer,
    labels: &[&Painted],
    drawn: &[Drawn],
    balls: &[(usize, Vec2, f32)],
) -> Vec<Option<Named>> {
    // How far a label may sit from what it names, past its edge: the placer nudges a
    // crowded one a few rows along.
    const REACH: f32 = 160.0;
    let mut pairs: Vec<(f32, usize, Named)> = Vec::new();
    let mut consider = |what: Named, name: &str, at: Vec2, edge: f32| {
        for (i, label) in labels.iter().enumerate() {
            if !carries(&label.text, name) {
                continue;
            }
            let distance = label.rect.distance_to_pos(egui::pos2(at.x, at.y));
            if distance <= edge + REACH {
                pairs.push((distance, i, what));
            }
        }
    };
    for d in drawn {
        consider(
            Named::Node(d.tile),
            &node_name(explorer, d.tile),
            d.at,
            d.radius,
        );
    }
    for &(cluster, at, radius) in balls {
        let name = explorer.cluster_name(cluster).to_uppercase();
        consider(Named::Module(cluster), &name, at, radius);
    }
    // Module names first: theirs are in capitals and hold counts, so a function named
    // like a word in one (`of` in `EXPLORER 3 of 106`) must not take it.
    pairs.sort_by(|a, b| {
        let node = |what: &Named| matches!(what, Named::Node(_));
        node(&a.2).cmp(&node(&b.2)).then(a.0.total_cmp(&b.0))
    });
    let mut named = vec![None; labels.len()];
    let mut taken = Vec::new();
    for (_, label, what) in pairs {
        if named[label].is_none() && !taken.contains(&what) {
            named[label] = Some(what);
            taken.push(what);
        }
    }
    named
}

/// Whether `text` carries `name` as a whole word.
fn carries(text: &str, name: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    !name.is_empty()
        && text.match_indices(name).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + name.len()..].chars().next();
            !before.is_some_and(word) && !after.is_some_and(word)
        })
}

/// The other functions drawn so near one that they cover half of it or more, but for
/// specks: two calls can land on one spot and read as one node.
fn crowding(explorer: &Explorer, drawn: &[Drawn], node: &Drawn) -> Vec<String> {
    if speck(explorer, node.tile) {
        return Vec::new();
    }
    drawn
        .iter()
        .filter(|other| other.tile != node.tile && !speck(explorer, other.tile))
        .filter(|other| other.at.distance(node.at) < node.radius.max(other.radius) * 0.5)
        .map(|other| node_name(explorer, other.tile))
        .collect()
}

fn module_of(explorer: &Explorer, tile: usize) -> Option<String> {
    explorer
        .graph()
        .cluster(tile)
        .map(|cluster| explorer.cluster_name(cluster))
}

/// What a drawn function is to the view: the centre, the journey's cursor, a caller,
/// a call of the journey so many levels down, under the pointer, an end of the call
/// the pointer's function makes or takes; else a speck.
fn roles(explorer: &Explorer, tile: usize) -> Vec<String> {
    let mut roles = Vec::new();
    let n = explorer.nucleus();
    if n.centre() == Some(tile) {
        roles.push("centre".to_owned());
    }
    if n.focus() == Some(tile) && n.centre() != Some(tile) {
        roles.push("start".to_owned());
    }
    if n.marks(tile) {
        roles.push("cursor".to_owned());
    }
    match n.level(tile) {
        Some(-1) if n.folded(tile) => {
            roles.push("caller, folded into its module's count".to_owned());
        }
        Some(-1) => roles.push("caller".to_owned()),
        Some(_) if n.aside() => roles.push("callee".to_owned()),
        Some(level) if level > 0 => roles.push(format!("journey level {level}")),
        _ => {}
    }
    if n.on_journey(tile) {
        roles.push("on the open journey".to_owned());
    }
    if n.hovered() == Some(tile) {
        roles.push("hovered".to_owned());
    }
    if roles.is_empty() {
        roles.push("speck".to_owned());
    }
    roles
}

/// A function's numbers in the open journey.
fn numbers(explorer: &Explorer, tile: usize) -> Vec<String> {
    explorer.nucleus().numbers(tile).to_vec()
}

/// What an arrow's color says.
fn flow_role(flow: Flow) -> String {
    match flow {
        Flow::Into => "calls the centre (violet)".to_owned(),
        Flow::Journey(1) => "the centre's call (blue)".to_owned(),
        Flow::Journey(level) => format!("journey level {level} (amber, darker the deeper)"),
        Flow::Outside => "calls into the journey from off it (pale)".to_owned(),
        Flow::Module => "calls from one module to another, nothing selected (faint)".to_owned(),
    }
}

/// Every call drawn as an arrow, function to function: those the nucleus names.
fn arrows(view: &View) -> Vec<Value> {
    let explorer = &*view.explorer;
    let name = |tile: usize| node_name(explorer, tile);
    let n = explorer.nucleus();
    n.links()
        .iter()
        .map(|link| {
            json!({
                "from": name(link.from),
                "to": name(link.to),
                "role": flow_role(link.flow),
                "color": hex(link.flow.color()),
            })
        })
        .collect()
}

/// The thick arrows between module spheres, each carrying every call one way.
fn module_arrows(explorer: &Explorer, n: &Nucleus) -> Vec<Value> {
    n.tubes()
        .iter()
        .map(|tube| {
            json!({
                "from": explorer.cluster_name(tube.from),
                "to": explorer.cluster_name(tube.to),
                "calls": tube.calls,
                "role": flow_role(tube.flow),
            })
        })
        .collect()
}

/// The journey's bars drawn in its panel, by number.
fn bars(view: &View) -> Vec<Value> {
    let explorer = &*view.explorer;
    let Some(drawn) = &view.screen.bars else {
        return Vec::new();
    };
    let disclosed = explorer.disclosure();
    let cursor = explorer.selected();
    let track = explorer.track();
    drawn
        .bars
        .iter()
        .filter(|(_, r)| r.intersects(drawn.content))
        .map(|&(span, r)| {
            let tile = track.unit_id(span).and_then(|u| explorer.map().tile_of(u));
            json!({
                "number": disclosed.number(span),
                "name": explorer.label(span),
                "unit": track.unit_id(span).map(ToString::to_string),
                "rect": rect(r),
                "cursor": cursor == Some(span),
                "lit": tile.is_some() && tile == explorer.hovered_tile,
                "cut_off": !drawn.content.contains_rect(r),
            })
        })
        .collect()
}

/// To a tenth, as the JSON shows points: rounded as a decimal, so it reads `132.9`
/// rather than `132.89999389648438`.
fn round(value: f32) -> f64 {
    (f64::from(value) * 10.0).round() / 10.0
}

/// To a thousandth, for the world, where a module is a few units across.
fn fine(value: f32) -> f64 {
    (f64::from(value) * 1000.0).round() / 1000.0
}

fn fine3(at: Vec3) -> Value {
    json!([fine(at.x), fine(at.y), fine(at.z)])
}

fn point(at: Vec2) -> Value {
    json!([round(at.x), round(at.y)])
}

fn rect(r: egui::Rect) -> Value {
    json!([
        round(r.min.x),
        round(r.min.y),
        round(r.max.x),
        round(r.max.y)
    ])
}

fn hex(color: Color) -> String {
    color.to_srgba().to_hex()
}

#[cfg(test)]
mod tests {
    use codeviz_ir::UnitId;

    use super::*;

    #[test]
    fn a_label_carries_a_name_only_as_a_whole_word() {
        assert!(carries(" 1.2  spotlit can fail", "spotlit"));
        assert!(carries("EXPLORER 3 of 40", "EXPLORER"));
        assert!(!carries("spotlit_more", "spotlit"));
        assert!(!carries("respotlit", "spotlit"));
        assert!(!carries("anything", ""));
    }

    #[test]
    fn lists_the_disclosed_calls_in_order_with_what_is_folded() {
        let mut explorer = crate::explorer::tests::explorer();
        let main = explorer.map().tile_of(&UnitId::new("x:main")).unwrap();
        explorer.open_at(main);
        let truth = of(&explorer);
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
            of(explorer)["calls"]
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
        explorer.set_disclose(crate::disclosure::Disclose(2));
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
