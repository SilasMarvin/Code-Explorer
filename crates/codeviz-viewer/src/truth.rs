//! What a view means to show, written beside its screenshot so a reader's account of
//! the picture can be checked against it: the journey's start or the selection, every
//! disclosed call with its number, the module it lives in, and whether it can fail or
//! only sometimes runs. A scripted session's snapshots say more: what is
//! drawn where on screen, by name, and what can be clicked ([`scene`]).

use std::collections::HashSet;

use bevy::{
    color::Color,
    math::{Vec2, Vec3},
};
use bevy_egui::egui;
use serde_json::{Value, json};

use crate::{
    drive::{ArrowShape, Drawn, Painted, Screen, View},
    explorer::{Explorer, is_conditional},
    journey_panel::{bar_fill, caller_fill, guessed},
    labels::{Named, node_name},
    nucleus::{Flow, Link},
};

/// The view's content, as JSON.
pub(crate) fn of(explorer: &Explorer) -> Value {
    let disclosed = explorer.disclosure();
    let track = explorer.track();
    let tile_of = |span| track.tile(span, explorer.map());
    let mut calls: Vec<(Vec<u32>, Value)> = disclosed
        .numbered()
        .filter(|(_, number)| !number.is_empty())
        .filter_map(|(span_id, number)| {
            let span = track.span(span_id)?;
            let name = track.unit_id(span_id).map(|u| explorer.function_name(u))?;
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
    let name = |tile: usize| Some(node_name(explorer, tile));
    // The function examined: the start, or browsing, the selection.
    let examined = explorer.start().or_else(|| explorer.selection());
    // Browsing, the selection's own calls, unnumbered.
    if explorer.browsing()
        && let Some(selection) = explorer.selection()
    {
        calls = explorer
            .graph()
            .callees(selection)
            .map(|callee| {
                (
                    Vec::new(),
                    json!({
                        "number": Value::Null,
                        "name": node_name(explorer, callee),
                        "module": module_of(explorer, callee),
                    }),
                )
            })
            .collect();
    }
    json!({
        "start": explorer.start().and_then(name),
        "selection": explorer.selection().and_then(name),
        "start_fails": root.is_some_and(|r| disclosed.fails(r)),
        "start_hidden_calls": root.map_or(0, |r| disclosed.folded(r)),
        "callers": examined.map_or_else(Vec::new, |tile| {
            explorer
                .graph()
                .callers(tile)
                .map(|caller| node_name(explorer, caller))
                .collect()
        }),
        "calls": calls.into_iter().map(|(_, call)| call).collect::<Vec<_>>(),
    })
}

/// The view's state in words: the selection or the journey's start, cursor and
/// depth, the function set aside, the expanded centre, what is hovered, which
/// overlays show, and where back and forward lead.
pub(crate) fn state(view: &View) -> Value {
    let explorer = &*view.explorer;
    let name = |tile: usize| node_name(explorer, tile);
    let nucleus = explorer.nucleus();
    let disclosed = explorer.disclosure();
    json!({
        "mode": if explorer.browsing() { "browse" } else { "journey" },
        "selection": explorer.selection().map(name),
        "start": explorer.start().map(name),
        // The module in the middle: the start's, or browsing, the selection's.
        "start_module": nucleus.start().and_then(|t| module_of(explorer, t)),
        "cursor": explorer.cursor_span().filter(|_| !explorer.browsing()).map(|span| json!({
            "number": disclosed.number(span),
            "name": explorer.label(span),
        })),
        "aside": explorer.aside().map(name),
        "nucleus": if nucleus.expanded() { "expanded" } else { "modules" },
        "expanded": nucleus.expanded_module().map(|c| explorer.cluster_name(c)),
        "depth": explorer.disclose().0,
        "outside_callers": explorer.outside_shown(),
        "hovered": explorer.hovered().map(name),
        "under_pointer": explorer.under_pointer.map(name),
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
                // The last wheel turn out panned to hold the map in the free area.
                "held": view.orbit_held(),
                // Zoomed out as far as the wheel goes.
                "farthest": view.orbit_farthest(),
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
    let (labels, chrome) = graph_labels(screen);
    let drawn = view.drawn();
    let nucleus = explorer.nucleus();
    let balls: Vec<(usize, Vec2, f32)> = nucleus
        .balls()
        .filter_map(|(cluster, ball)| {
            let at = view.to_screen(ball.center)?;
            Some((cluster, at, view.screen_radius(ball.center, ball.radius)))
        })
        .collect();
    let named = match_labels(explorer, &labels, &screen.labels, &balls);
    let label_of = |what: Labelled| named.iter().position(|&n| n == Some(what));

    let nodes: Vec<Value> = drawn
        .iter()
        .map(|d| {
            let label = label_of(Labelled::Node(d.tile));
            json!({
                "name": node_name(explorer, d.tile),
                "unit": explorer.tile_unit(d.tile).map(ToString::to_string),
                "module": module_of(explorer, d.tile),
                "roles": named_roles(explorer, d.tile, label.is_some()),
                "numbers": numbers(explorer, d.tile),
                "at": point(d.at),
                "radius": round(d.radius),
                "world": fine3(d.world),
                "world_radius": fine(d.world_radius),
                "on_screen": d.on_screen,
                "label": label.map(|i| labels[i].text.clone()),
                "label_rect": label.map(|i| rect(labels[i].rect)),
                "name_size": name_size(screen, Named::Node(d.tile)),
                "kept_off": screen.kept_off.iter().find(|k| k.0 == d.tile).map(|k| round(k.2)),
                "arrivals": arrivals(screen, d.tile),
                "crowded_by": crowding(explorer, &drawn, d),
                "color": d.color.map(hex),
                "rings": rings(view, d),
            })
        })
        .collect();
    let modules: Vec<Value> = balls
        .iter()
        .filter_map(|&(cluster, at, radius)| {
            let ball = nucleus.ball(cluster)?;
            let label = label_of(Labelled::Module(cluster));
            Some(json!({
                "name": explorer.cluster_name(cluster),
                "body": format!("{:?}", ball.body).to_lowercase(),
                "functions": nucleus.count(cluster),
                "named": nucleus.opened(cluster),
                "shown": nucleus.shows_body(ball.body),
                "at": point(at),
                "world": fine3(ball.center),
                "radius": round(radius),
                "world_radius": fine(ball.radius),
                "on_screen": view.on_screen(at),
                "label": label.map(|i| labels[i].text.clone()),
                "label_rect": label.map(|i| rect(labels[i].rect)),
                "name_size": name_size(screen, Named::Group(cluster)),
            }))
        })
        .collect();
    let overlays: Vec<Value> = screen
        .overlays
        .iter()
        .map(|(name, r)| json!({ "name": name, "rect": rect(*r), "clipped": !window.contains_rect(*r) }))
        .collect();
    let label_list = label_entries(explorer, screen, &labels, &named);
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
        "free_area": rect(screen.names),
        "camera": camera(view),
        "state": state(view),
        "nodes": nodes,
        "held": held(explorer, screen, &drawn),
        "modules": modules,
        "arrows": arrows(view),
        "module_arrows": module_arrows(view),
        "exits": exits(view),
        "labels": label_list,
        "bars": bars(view),
        "panel": panel(view),
        "search_rows": search_rows(view),
        "buttons": screen.widgets.iter().map(|w| json!({
            "text": w.text, "rect": rect(w.rect), "enabled": w.enabled, "cut_off": w.cut_off(),
        })).collect::<Vec<_>>(),
        "overlays": overlays,
        "help": help.flatten(),
        "text": chrome.iter().map(|t| json!({ "text": t.text, "rect": rect(t.rect), "size": t.size })).collect::<Vec<_>>(),
        "content": of(explorer),
    })
}

/// The last stretch of each call coming into a function's dot that its name keeps
/// off, from the dot back.
fn arrivals(screen: &Screen, tile: usize) -> Vec<Vec<[f64; 2]>> {
    (screen.arrivals.iter())
        .filter(|(t, _)| *t == tile)
        .map(|(_, tail)| tail.iter().map(|p| [round(p.x), round(p.y)]).collect())
        .collect()
}

/// Where the journey's calls a deeper level shows would be, which names keep off:
/// each with where, and how far names keep from it.
fn held(explorer: &Explorer, screen: &Screen, drawn: &[Drawn]) -> Vec<Value> {
    let shown: HashSet<usize> = drawn.iter().map(|d| d.tile).collect();
    screen
        .kept_off
        .iter()
        .filter(|k| !shown.contains(&k.0))
        .map(|&(tile, at, radius)| {
            json!({
                "name": node_name(explorer, tile),
                "unit": explorer.tile_unit(tile).map(ToString::to_string),
                "at": [round(at.x), round(at.y)],
                "radius": round(radius),
            })
        })
        .collect()
}

/// Where the camera is in the world, and which way it faces.
fn camera(view: &View) -> Value {
    let Some((at, facing)) = view.camera() else {
        return Value::Null;
    };
    let ahead = facing * Vec3::NEG_Z;
    let target = view.orbit_pose().map(|pose| pose.parts().0);
    json!({
        "at": fine3(at),
        "facing": fine3(ahead),
        // Where what the camera looks at shows: a dolly in or out leaves it there.
        "target_at": target.and_then(|t| view.to_screen(t)).map(point),
    })
}

/// How big the name of `what` was laid out, room found for it or not.
fn name_size(screen: &Screen, what: Named) -> Option<[f64; 2]> {
    let size = screen.name_sizes.get(&what)?;
    Some([round(size.x), round(size.y)])
}

/// The text drawn on the graph, its names among it, and the rest: the chrome. Names
/// are clipped to the graph, or with a journey open to the part of it above the
/// panel.
fn graph_labels(screen: &Screen) -> (Vec<&Painted>, Vec<&Painted>) {
    let near = |a: egui::Rect, b: egui::Rect| {
        (a.min - b.min).length() < 1.5 && (a.max - b.max).length() < 1.5
    };
    screen
        .texts
        .iter()
        .filter(|t| t.opacity > 0.05)
        .partition(|t| near(t.clip, screen.graph) || near(t.clip, screen.names))
}

/// Every label on the graph: what it says and names, where, which others it
/// overlaps, whether it is cut off, and what lies over it.
fn label_entries(
    explorer: &Explorer,
    screen: &Screen,
    labels: &[&Painted],
    named: &[Option<Labelled>],
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
            let (kind, of, unit) = match named[index] {
                Some(Labelled::Node(tile)) => (
                    "function",
                    Some(node_name(explorer, tile)),
                    explorer.tile_unit(tile).map(ToString::to_string),
                ),
                Some(Labelled::Module(cluster)) => {
                    ("module", Some(explorer.cluster_name(cluster)), None)
                }
                None => ("other", None, None),
            };
            json!({
                "index": index,
                "text": text.text,
                "rect": rect(text.rect),
                "names": kind,
                "of": of,
                "unit": unit,
                "overlaps": crossed,
                "cut_off": !text.clip.contains_rect(text.rect) || !window.contains_rect(text.rect),
                "under": under,
                "opacity": round(text.opacity),
                "size": text.size,
            })
        })
        .collect()
}

/// Whether a function is drawn only as a faint speck, and not hovered.
fn speck(explorer: &Explorer, tile: usize) -> bool {
    let n = explorer.nucleus();
    n.speck(tile) && n.hovered() != Some(tile)
}

/// What a label on the graph names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Labelled {
    Node(usize),
    Module(usize),
}

/// Which label names which function or module: a function's, the name the label
/// placer drew on that function's pill; a module's, the nearest label left that
/// carries its name, each taken once, nearest pairs first. Any other text on the
/// graph (the key hint, a notice, a footer) names nothing.
fn match_labels(
    explorer: &Explorer,
    labels: &[&Painted],
    pills: &[(egui::Rect, usize)],
    balls: &[(usize, Vec2, f32)],
) -> Vec<Option<Labelled>> {
    // How far a module's name may sit from its sphere's rim.
    const REACH: f32 = 160.0;
    let mut named: Vec<Option<Labelled>> = labels
        .iter()
        .map(|label| {
            pills
                .iter()
                .rev()
                .find(|(pill, _)| pill.contains(label.rect.center()))
                .map(|&(_, tile)| Labelled::Node(tile))
        })
        .collect();
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for &(cluster, at, radius) in balls {
        let name = explorer.cluster_name(cluster).to_uppercase();
        for (i, label) in labels.iter().enumerate() {
            if named[i].is_some() || !carries(&label.text, &name) {
                continue;
            }
            let distance = label.rect.distance_to_pos(egui::pos2(at.x, at.y));
            if distance <= radius + REACH {
                pairs.push((distance, i, cluster));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut taken = Vec::new();
    for (_, label, cluster) in pairs {
        if named[label].is_none() && !taken.contains(&cluster) {
            named[label] = Some(Labelled::Module(cluster));
            taken.push(cluster);
        }
    }
    named
}

/// Whether `text` leads with `name` as a whole word, as a name on the graph does:
/// after a journey number, if any, and the type or path it may be qualified by
/// (`Explorer.root`). A hint that only mentions the word (`R whole map`) names
/// nothing.
fn carries(text: &str, name: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let number = |c: char| c.is_ascii_digit() || matches!(c, '.' | '+' | ' ');
    !name.is_empty()
        && text.match_indices(name).any(|(at, _)| {
            let lead = text[..at].trim_start_matches(number);
            let qualified = lead.is_empty()
                || (!lead.contains(char::is_whitespace)
                    && (lead.ends_with('.') || lead.ends_with("::")));
            let after = text[at + name.len()..].chars().next();
            qualified && !after.is_some_and(word)
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

/// What a drawn function is to the view: the journey's start or the selection, the
/// journey's cursor, the function set aside, one on the cursor's path down from the
/// start (`path`), a caller, a call of the journey so many
/// levels down, a caller from off the journey that I brings in (`outside`), one that
/// calls back besides, one of the functions of a module
/// expanded while browsing, hovered; else a speck.
fn roles(explorer: &Explorer, tile: usize) -> Vec<String> {
    let mut roles = Vec::new();
    let n = explorer.nucleus();
    let is = |of: Option<usize>| of == Some(tile);
    for (role, of) in [
        ("start", explorer.start()),
        ("selection", explorer.selection()),
        ("cursor", n.cursor()),
        ("aside", explorer.aside()),
    ] {
        if is(of) {
            roles.push(role.to_owned());
        }
    }
    if crate::labels::cursor_path(explorer).contains_key(&tile) {
        roles.push("path".to_owned());
    }
    match n.level(tile) {
        Some(-1) if n.folded(tile) => {
            roles.push("caller, folded into its module's count".to_owned());
        }
        Some(-1) => roles.push("caller".to_owned()),
        Some(level) if level > 0 => roles.push(format!("journey level {level}")),
        _ => {}
    }
    if n.outside(tile) {
        roles.push("outside".to_owned());
    }
    if n.calls_back(tile) {
        roles.push("calls back".to_owned());
    }
    if n.opened_up(tile) && n.level(tile).is_none() && !is(n.start()) {
        roles.push("opened".to_owned());
    }
    if crate::graph_view::guessed(explorer, tile) {
        roles.push("guessed".to_owned());
    }
    if is(n.hovered()) {
        roles.push("hovered".to_owned());
    }
    if roles.is_empty() {
        roles.push("speck".to_owned());
    }
    roles
}

/// [`roles`], but a speck the view names is a plain function, not a faint one.
fn named_roles(explorer: &Explorer, tile: usize, named: bool) -> Vec<String> {
    let roles = roles(explorer, tile);
    if named && roles == ["speck"] {
        vec!["plain".to_owned()]
    } else {
        roles
    }
}

/// A function's numbers in the open journey.
fn numbers(explorer: &Explorer, tile: usize) -> Vec<String> {
    explorer
        .disclosure()
        .tiles()
        .get(&tile)
        .cloned()
        .unwrap_or_default()
}

/// What an arrow's color says.
fn flow_role(flow: Flow) -> String {
    match flow {
        Flow::Into => "calls the start or the selection (violet)".to_owned(),
        Flow::Journey(1) => "the start's or the selection's own call (blue)".to_owned(),
        Flow::Journey(level) => format!("journey level {level} (amber, darker the deeper)"),
        Flow::Outside => "calls into the journey from off it (pale)".to_owned(),
        Flow::Inner => "calls among the expanded module's functions (faint)".to_owned(),
    }
}

/// An arrow's flow, as the checks read it: `into`, `journey` with its level,
/// `outside` or `module`.
fn flow_kind(flow: Flow) -> (&'static str, Option<u32>) {
    match flow {
        Flow::Into => ("into", None),
        Flow::Journey(level) => ("journey", Some(level)),
        Flow::Outside => ("outside", None),
        Flow::Inner => ("inner", None),
    }
}

/// How an arrow shows: how wide its line, how long its head from its back to its
/// tip and how wide, in points, and where its tip is.
fn shape(view: &View, shape: &ArrowShape) -> Value {
    json!({
        "width": round(shape.width),
        "head": round(shape.head),
        "head_width": round(shape.head_width),
        "tip": point(shape.tip),
        "path": shape.path.iter().map(|&p| point(p)).collect::<Vec<_>>(),
        "on_screen": crosses_free(view.screen.names, &shape.path),
    })
}

/// Whether any piece of the line through `path` runs over `free`, the graph less a
/// docked panel: a line can cross the view with both its ends off it.
fn crosses_free(free: egui::Rect, path: &[Vec2]) -> bool {
    let at = |p: Vec2| egui::pos2(p.x, p.y);
    path.iter().any(|&p| free.contains(at(p)))
        || path.windows(2).any(|pair| {
            let (a, b) = (at(pair[0]), at(pair[1]));
            // Where it runs inside each pair of the area's sides, as shares of it.
            let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
            for (start, delta, low, high) in [
                (a.x, b.x - a.x, free.min.x, free.max.x),
                (a.y, b.y - a.y, free.min.y, free.max.y),
            ] {
                if delta.abs() < f32::EPSILON {
                    if start < low || start > high {
                        return false;
                    }
                    continue;
                }
                let (u0, u1) = ((low - start) / delta, (high - start) / delta);
                t0 = t0.max(u0.min(u1));
                t1 = t1.min(u0.max(u1));
            }
            t0 < t1
        })
}

/// Every call drawn as an arrow, function to function, as drawn: its color, how wide
/// its line, and its head.
fn arrows(view: &View) -> Vec<Value> {
    let explorer = &*view.explorer;
    let name = |tile: usize| node_name(explorer, tile);
    let unit = |tile: usize| explorer.tile_unit(tile).map(ToString::to_string);
    view.marks
        .calls
        .iter()
        .map(|call| {
            let (flow, level) = flow_kind(call.flow);
            let link = Link {
                from: call.from,
                to: call.to,
                flow: call.flow,
            };
            let mut arrow = json!({
                "from": name(call.from),
                "to": name(call.to),
                "from_unit": unit(call.from),
                "to_unit": unit(call.to),
                "flow": flow,
                "level": level,
                "role": flow_role(call.flow),
                "color": hex(call.color),
                "guessed": explorer.nucleus().guessed(&link),
            });
            merge(&mut arrow, shape(view, &call.shape));
            arrow
        })
        .collect()
}

/// Expanded in a journey, where each call leaves the free area, labelled with the
/// function at its far end: the call, that function, what the label says and where.
fn exits(view: &View) -> Vec<Value> {
    let explorer = &*view.explorer;
    let unit = |tile: usize| explorer.tile_unit(tile).map(ToString::to_string);
    view.marks
        .exits
        .iter()
        .map(|exit| {
            json!({
                "from_unit": unit(exit.from),
                "to_unit": unit(exit.to),
                "far_unit": unit(exit.far),
                "far": node_name(explorer, exit.far),
                "text": exit.text,
                "rect": rect(exit.rect),
                "at": point(exit.at),
            })
        })
        .collect()
}

/// Browsing with nothing selected, the lines between module spheres, each carrying
/// every call one way, as drawn: how wide, and its head.
fn module_arrows(view: &View) -> Vec<Value> {
    let explorer = &*view.explorer;
    explorer
        .nucleus()
        .tubes()
        .iter()
        .enumerate()
        .map(|(index, tube)| {
            let mut arrow = json!({
                "from": explorer.cluster_name(tube.from),
                "to": explorer.cluster_name(tube.to),
                "calls": tube.calls,
                "flow": "module",
                "level": Value::Null,
                "role": "calls from one module to another, nothing selected (faint)",
                "drawn": false,
            });
            if let Some(mark) = view.marks.tubes.iter().find(|m| m.index == index) {
                arrow["drawn"] = json!(true);
                merge(&mut arrow, shape(view, &mark.shape));
            }
            arrow
        })
        .collect()
}

/// The rings drawn round a function: how big on screen, and in what color.
fn rings(view: &View, d: &Drawn) -> Vec<Value> {
    view.marks
        .rings
        .iter()
        .filter(|ring| ring.tile == d.tile)
        .map(|ring| {
            json!({
                "radius": round(view.screen_radius(d.world, ring.radius)),
                "color": hex(ring.color),
            })
        })
        .collect()
}

/// The search box's rows, top first: what each says, the function it finds, where it
/// is, whether it shows or is scrolled away, and whether Enter takes it.
fn search_rows(view: &View) -> Value {
    let Some(search) = &view.screen.search else {
        return Value::Null;
    };
    search
        .rows
        .iter()
        .map(|row| {
            json!({
                "text": row.text,
                "unit": row.unit,
                "rect": rect(row.rect),
                "shown": search.content.intersects(row.rect),
                "highlighted": row.highlighted,
            })
        })
        .collect()
}

fn merge(into: &mut Value, more: Value) {
    if let (Some(into), Value::Object(more)) = (into.as_object_mut(), more) {
        into.extend(more);
    }
}

/// How much of the journey the panel shows: how many of its bars lie below what it
/// shows, cut off or out of view, and whether a track down its side says so.
fn panel(view: &View) -> Value {
    let Some(drawn) = &view.screen.bars else {
        return Value::Null;
    };
    let below = drawn
        .bars
        .iter()
        .filter(|(_, r)| r.max.y > drawn.content.max.y + 0.5)
        .count();
    json!({ "below": below, "rows_track": drawn.rows_track.is_some() })
}

/// The journey's bars drawn in its panel, by number, then the bars of its start's
/// callers in the row above the start's (`caller`, with no number).
fn bars(view: &View) -> Vec<Value> {
    let explorer = &*view.explorer;
    let Some(drawn) = &view.screen.bars else {
        return Vec::new();
    };
    let disclosed = explorer.disclosure();
    let cursor = explorer.cursor_span();
    let track = explorer.track();
    drawn
        .bars
        .iter()
        .filter(|(_, r)| r.intersects(drawn.content))
        .map(|&(span, r)| {
            let tile = track.tile(span, explorer.map());
            let chip = drawn
                .chips
                .iter()
                .find(|(s, _)| *s == span)
                .map(|(_, r)| rect(*r));
            json!({
                "number": disclosed.bar_number(span),
                "name": explorer.label(span),
                "unit": track.unit_id(span).map(ToString::to_string),
                "rect": rect(r),
                "folded": disclosed.folded(span),
                "chip": chip,
                "cursor": cursor == Some(span),
                "color": hex32(bar_fill(explorer, span)),
                "guessed": guessed(explorer, span),
                "lit": tile.is_some() && tile == explorer.hovered(),
                "cut_off": !drawn.content.contains_rect(r),
            })
        })
        .chain(drawn.callers.iter().map(|&(tile, r)| {
            let unit = explorer.tile_unit(tile);
            json!({
                "number": null,
                "name": unit.map(|u| explorer.function_name(u)).unwrap_or_default(),
                "unit": unit.map(ToString::to_string),
                "rect": rect(r),
                "caller": true,
                "aside": explorer.aside() == Some(tile),
                "cursor": false,
                "color": hex32(caller_fill(explorer, tile)),
                "lit": explorer.hovered() == Some(tile),
                "cut_off": false,
            })
        }))
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

/// A panel color as [`hex`] writes a scene's: with its alpha after it where it is
/// see-through, as a faint bar is.
fn hex32(color: egui::Color32) -> String {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    if a == u8::MAX {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    }
}

fn hex(color: Color) -> String {
    color.to_srgba().to_hex()
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn a_label_carries_a_name_only_as_a_whole_word() {
        assert!(carries(" 1.2  spotlit can fail", "spotlit"));
        assert!(carries("EXPLORER 3 of 40", "EXPLORER"));
        assert!(!carries("spotlit_more", "spotlit"));
        assert!(!carries("respotlit", "spotlit"));
        assert!(!carries("anything", ""));
        assert!(carries("Explorer.root", "root"), "qualified by its type");
        assert!(
            !carries("Browsing · R whole map · H keys", "map"),
            "a hint only mentions it"
        );
    }

    #[test]
    fn lists_the_disclosed_calls_in_order_with_what_is_folded() {
        let explorer = crate::explorer::tests::explorer();
        let truth = of(&explorer);
        assert_eq!(truth["start"], "main");
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
        let mut explorer = crate::explorer::tests::opened(system, "t:place");
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
