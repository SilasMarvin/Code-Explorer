//! Checks run on every snapshot of a scripted session, from the snapshot's own JSON
//! (see [`crate::truth::scene`]), so what they judge is what the snapshot reports.
//! Some hold of any one picture: dots inside their spheres and apart, spheres apart,
//! dot and sphere sizes, names by their dots and clear of the rest and on screen, a
//! few text sizes, graph and journey panel agreeing. Some hold after a kind of step,
//! judged against the snapshot before: the expanded module filling the view after
//! `x`, F and R framing what they frame, a wheel turn zooming about the pointer and
//! within bounds, and nothing moving after a step that must move nothing. Each passes
//! (`pass: true`), fails with the offending functions, labels and pixel numbers, or
//! does not apply (`pass: null`, saying why).

use std::collections::{BTreeMap, HashMap};

use bevy::math::{Vec2, Vec3};
use bevy_egui::egui;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{nucleus::VIEW, script::Kind};

// What the checks allow, each in one place. Points are window points, as in the PNG;
// world sizes are the scene's units.

/// How far a dot may poke out of its sphere, as a share of the sphere's radius,
/// before it counts as outside: rounding in the positions, not leeway.
const SPHERE_SLACK: f32 = 0.01;
/// Least clear space between two dots on screen, edge to edge, in points.
const DOT_GAP: f32 = 1.0;
/// Least radius of a lit dot on screen: three points across.
const LEAST_LIT_DOT: f32 = 1.5;
/// Most radius of any dot on screen, however near the camera.
const MOST_DOT: f32 = 16.0;
/// How far two spheres may overlap, in the world, before they count as intersecting:
/// rounding again.
const SPHERE_OVERLAP: f32 = 0.002;
/// Largest sphere's radius against the smallest's (CORE: "no more than ~4×").
const SPHERE_RATIO: f32 = 4.0;
/// Farthest a function's name may sit from its dot, and a module's from its sphere's
/// rim, edge to edge, in points (CORE: "≤ ~40 px"); past it a name names nothing a
/// reader can see. The placer keeps to its own rule of the same reach,
/// [`crate::labels`]'s `NEAR`.
pub(crate) const LABEL_REACH: f32 = 40.0;
/// Smallest and largest font size of a function's name, in points.
const NAME_SIZES: (f32, f32) = (11.0, 18.0);
/// Most distinct font sizes on screen at once: module name, function name, panel,
/// and small print.
const MOST_TEXT_SIZES: usize = 4;
/// Share of the free area's smaller side the expanded module's diameter fills.
const EXPANDED_FILL: (f32, f32) = (0.6, 0.9);
/// How far a framed sphere or dot may stick past the free area, in points.
const EDGE: f32 = 2.0;
/// Margin F leaves round the dots it frames, in points.
const FRAME_MARGIN: f32 = 4.0;
/// How far from the usual view ([`VIEW`]) R may leave the camera's yaw and pitch, in
/// radians. The usual view is the front the map is laid out for and opens at, not
/// yaw and pitch zero.
const FRONT_SLACK: f32 = 0.05;
/// How far what lay under the pointer may drift on a wheel turn: points, plus a
/// share of its distance from the pointer.
const ZOOM_DRIFT: (f32, f32) = (4.0, 0.05);
/// Zoomed out, the least share of the free area's smaller side the whole map spans.
const ZOOM_OUT_LEAST: f32 = 0.4;
/// Least a function or sphere moves in the world, or a dot or name on screen in
/// points, or the camera turns in radians, to count as moved.
const MOVED_WORLD: f32 = 0.01;
const MOVED_SCREEN: f32 = 0.5;
const MOVED_ANGLE: f32 = 0.002;
/// Offenders listed per check; `count` says how many in all.
const LISTED: usize = 12;

/// Every check, in the order a snapshot lists them.
pub(crate) const NAMES: [&str; 15] = [
    "dots-in-spheres",
    "dots-apart",
    "dot-sizes",
    "spheres-apart",
    "sphere-sizes",
    "labels-near",
    "labels-clear",
    "named-on-screen",
    "text-sizes",
    "panel-agrees",
    "expanded-fill",
    "framed",
    "zoom-to-pointer",
    "zoom-bounds",
    "nothing-moved",
];

/// What the checks read of a snapshot.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Snap {
    graph_area: [f32; 4],
    state: State,
    nodes: Vec<Node>,
    modules: Vec<Module>,
    labels: Vec<Label>,
    bars: Vec<Bar>,
    overlays: Vec<Overlay>,
    text: Vec<Text>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct State {
    mode: String,
    nucleus: String,
    /// The module expanded, by name.
    expanded: Option<String>,
    pointer: Option<[f32; 2]>,
    camera: Option<Camera>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(default)]
struct Camera {
    target: [f32; 3],
    distance: f32,
    yaw: f32,
    pitch: f32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Node {
    name: String,
    unit: Option<String>,
    module: Option<String>,
    roles: Vec<String>,
    numbers: Vec<String>,
    at: [f32; 2],
    radius: f32,
    world: Option<[f32; 3]>,
    world_radius: f32,
    on_screen: bool,
    label: Option<String>,
    label_rect: Option<[f32; 4]>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Module {
    name: String,
    body: String,
    shown: bool,
    functions: usize,
    at: [f32; 2],
    radius: f32,
    world: [f32; 3],
    world_radius: f32,
    on_screen: bool,
    label: Option<String>,
    label_rect: Option<[f32; 4]>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Label {
    text: String,
    rect: [f32; 4],
    names: String,
    of: Option<String>,
    overlaps: Vec<usize>,
    cut_off: bool,
    under: Vec<String>,
    size: f32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Bar {
    number: Option<String>,
    name: String,
    unit: Option<String>,
    cursor: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Overlay {
    name: String,
    rect: [f32; 4],
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Text {
    text: String,
    size: f32,
}

impl Node {
    /// Whether the dot is lit: anything but a faint speck.
    fn lit(&self) -> bool {
        !self.roles.iter().all(|r| r == "speck")
    }
}

impl Snap {
    fn expanded(&self) -> bool {
        self.state.nucleus == "expanded"
    }

    fn spheres(&self) -> impl Iterator<Item = &Module> {
        self.modules.iter().filter(|m| m.shown)
    }

    /// The expanded module's sphere: the one the state names, else the nucleus.
    fn expanded_sphere(&self) -> Option<&Module> {
        match &self.state.expanded {
            Some(name) => self.spheres().find(|m| &m.name == name),
            None => self.spheres().find(|m| m.body == "nucleus"),
        }
    }

    /// The graph's part of the window that nothing lies over: the graph area less any
    /// panel docked along one of its sides.
    fn free(&self) -> egui::Rect {
        let mut free = rect(self.graph_area);
        for overlay in &self.overlays {
            let r = rect(overlay.rect);
            if !r.intersects(free) {
                continue;
            }
            let wide = r.width() >= free.width() * 0.5;
            let tall = r.height() >= free.height() * 0.5;
            if wide && r.max.y >= free.max.y - EDGE {
                free.max.y = free.max.y.min(r.min.y);
            } else if wide && r.min.y <= free.min.y + EDGE {
                free.min.y = free.min.y.max(r.max.y);
            } else if tall && r.max.x >= free.max.x - EDGE {
                free.max.x = free.max.x.min(r.min.x);
            } else if tall && r.min.x <= free.min.x + EDGE {
                free.min.x = free.min.x.max(r.max.x);
            }
        }
        free
    }
}

/// Runs every check on the snapshot `now`, given the snapshot before it, if any, and
/// what each step between them may move.
pub(crate) fn run(now: &Value, before: Option<&Value>, kinds: &[Kind]) -> Value {
    let read = |v: &Value| serde_json::from_value::<Snap>(v.clone());
    let (now, before) = match (read(now), before.map(read).transpose()) {
        (Ok(now), Ok(before)) => (now, before),
        (Err(error), _) | (_, Err(error)) => {
            let why = format!("the snapshot's JSON could not be read: {error}");
            return NAMES
                .iter()
                .map(|&name| (name.to_owned(), fail(&why)))
                .collect();
        }
    };
    let before = before.as_ref();
    let results = [
        dots_in_spheres(&now),
        dots_apart(&now),
        dot_sizes(&now),
        spheres_apart(&now),
        sphere_sizes(&now),
        labels_near(&now),
        labels_clear(&now),
        named_on_screen(&now),
        text_sizes(&now),
        panel_agrees(&now),
        expanded_fill(&now, kinds),
        framed(&now, kinds),
        zoom_to_pointer(&now, before, kinds),
        zoom_bounds(&now, kinds),
        nothing_moved(&now, before, kinds),
    ];
    NAMES
        .iter()
        .zip(results)
        .map(|(&name, result)| (name.to_owned(), result))
        .collect::<Map<String, Value>>()
        .into()
}

/// What moved from one snapshot to the next: the camera; functions drawn in both, in
/// the world and on screen; spheres; names; and how many functions came and went.
pub(crate) fn moved(before: &Value, now: &Value, since: &str) -> Value {
    let read = |v: &Value| serde_json::from_value::<Snap>(v.clone()).unwrap_or_default();
    let mut summary = compare(&read(before), &read(now), false).0;
    summary["since"] = json!(since);
    summary
}

/// How a session's snapshots fared, check by check: how many passed, failed or did
/// not apply, and which failed.
pub(crate) fn summarise(snaps: &[(String, Value)]) -> Value {
    NAMES
        .iter()
        .map(|&name| {
            let verdicts: Vec<(&str, &Value)> = snaps
                .iter()
                .map(|(snap, checks)| (snap.as_str(), &checks[name]["pass"]))
                .collect();
            let failed: Vec<&str> = verdicts
                .iter()
                .filter(|(_, pass)| **pass == json!(false))
                .map(|(snap, _)| *snap)
                .collect();
            let passed = verdicts.iter().filter(|(_, p)| **p == json!(true)).count();
            let summary = json!({
                "pass": failed.is_empty(),
                "passed": passed,
                "failed": failed.len(),
                "not_applied": verdicts.len() - passed - failed.len(),
                "failed_in": failed,
            });
            (name.to_owned(), summary)
        })
        .collect::<Map<String, Value>>()
        .into()
}

fn dots_in_spheres(s: &Snap) -> Value {
    let spheres: HashMap<&str, &Module> = s.spheres().map(|m| (m.name.as_str(), m)).collect();
    if spheres.is_empty() {
        return skip("no module spheres drawn");
    }
    let offenders = s
        .nodes
        .iter()
        .filter_map(|n| {
            let m = spheres.get(n.module.as_deref()?)?;
            let centre = Vec3::from_array(m.world);
            let out = Vec3::from_array(n.world?).distance(centre) + n.world_radius - m.world_radius;
            (out > m.world_radius * SPHERE_SLACK).then(|| {
                let share = out / m.world_radius.max(f32::EPSILON);
                let offender = json!({
                    "function": n.name, "module": m.name, "at": pt(n.at),
                    "out_by": r3(out), "sphere_radius": r3(m.world_radius),
                });
                (share, offender)
            })
        })
        .collect();
    verdict(offenders)
}

fn dots_apart(s: &Snap) -> Value {
    let dots: Vec<&Node> = s
        .nodes
        .iter()
        .filter(|n| n.on_screen && n.radius > 0.0)
        .collect();
    let mut offenders = Vec::new();
    for (i, a) in dots.iter().enumerate() {
        for b in &dots[i + 1..] {
            let apart = v2(a.at).distance(v2(b.at));
            let short = a.radius + b.radius + DOT_GAP - apart;
            if short > 0.0 {
                let lit = a.lit() || b.lit();
                let offender = json!({
                    "functions": [a.name, b.name], "lit": lit, "at": [pt(a.at), pt(b.at)],
                    "apart": r1(apart), "radii": [r1(a.radius), r1(b.radius)], "short_by": r1(short),
                });
                // Lit pairs first: those are the ones a reader is meant to tell apart.
                offenders.push((short + if lit { 1e4 } else { 0.0 }, offender));
            }
        }
    }
    verdict(offenders)
}

fn dot_sizes(s: &Snap) -> Value {
    let offenders = s
        .nodes
        .iter()
        .filter(|n| n.on_screen)
        .filter_map(|n| {
            let (off, bound) = if n.lit() && n.radius < LEAST_LIT_DOT {
                (LEAST_LIT_DOT - n.radius, json!({ "least": LEAST_LIT_DOT }))
            } else if n.radius > MOST_DOT {
                (n.radius - MOST_DOT, json!({ "most": MOST_DOT }))
            } else {
                return None;
            };
            let offender = json!({
                "function": n.name, "roles": n.roles, "radius": r1(n.radius), "allowed": bound,
            });
            Some((off, offender))
        })
        .collect();
    verdict(offenders)
}

fn spheres_apart(s: &Snap) -> Value {
    let spheres: Vec<&Module> = s.spheres().collect();
    if spheres.len() < 2 {
        return skip("fewer than two module spheres drawn");
    }
    let mut offenders = Vec::new();
    for (i, a) in spheres.iter().enumerate() {
        for b in &spheres[i + 1..] {
            let apart = Vec3::from_array(a.world).distance(Vec3::from_array(b.world));
            let overlap = a.world_radius + b.world_radius - apart;
            if overlap > SPHERE_OVERLAP {
                let smaller = a.world_radius.min(b.world_radius).max(f32::EPSILON);
                let offender = json!({
                    "modules": [a.name, b.name], "overlap": r3(overlap),
                    "share_of_smaller": r2(overlap / smaller),
                });
                offenders.push((overlap / smaller, offender));
            }
        }
    }
    verdict(offenders)
}

fn sphere_sizes(s: &Snap) -> Value {
    // Expanded, the module in the middle is meant to be bigger than the rest.
    let sized: Vec<&Module> = s
        .spheres()
        .filter(|m| !(s.expanded() && m.body == "nucleus"))
        .filter(|m| m.world_radius > 0.0)
        .collect();
    let (Some(small), Some(big)) = (
        sized
            .iter()
            .min_by(|a, b| a.world_radius.total_cmp(&b.world_radius)),
        sized
            .iter()
            .max_by(|a, b| a.world_radius.total_cmp(&b.world_radius)),
    ) else {
        return skip("no module spheres drawn");
    };
    let ratio = big.world_radius / small.world_radius;
    let mut offenders = Vec::new();
    if ratio > SPHERE_RATIO {
        let offender = json!({
            "largest": big.name, "smallest": small.name, "ratio": r2(ratio), "most": SPHERE_RATIO,
        });
        offenders.push((ratio, offender));
    }
    // Bigger with more functions, but less than in proportion.
    for a in &sized {
        for b in &sized {
            if a.functions <= b.functions || b.functions == 0 {
                continue;
            }
            let grows = a.world_radius / b.world_radius;
            let count = a.functions as f32 / b.functions as f32;
            let why = if grows < 1.0 - SPHERE_SLACK {
                "holds more functions, but is smaller"
            } else if grows > count * (1.0 + SPHERE_SLACK) {
                "is bigger than in proportion to its functions"
            } else {
                continue;
            };
            let offender = json!({
                "module": a.name, "functions": a.functions, "radius": r3(a.world_radius),
                "against": b.name, "their_functions": b.functions, "their_radius": r3(b.world_radius),
                "why": why,
            });
            offenders.push(((grows - 1.0).abs(), offender));
        }
    }
    with(verdict(offenders), "ratio", json!(r2(ratio)))
}

fn labels_near(s: &Snap) -> Value {
    let mut offenders = Vec::new();
    for n in &s.nodes {
        let (Some(label), Some(r)) = (&n.label, n.label_rect) else {
            continue;
        };
        let gap = (rect(r).distance_to_pos(pos(n.at)) - n.radius).max(0.0);
        if !n.on_screen {
            let offender =
                json!({ "function": n.name, "label": label, "why": "its dot is off screen" });
            offenders.push((f32::MAX, offender));
        } else if gap > LABEL_REACH {
            let offender =
                json!({ "function": n.name, "label": label, "gap": r1(gap), "most": LABEL_REACH });
            offenders.push((gap, offender));
        }
    }
    for m in s.spheres() {
        let (Some(label), Some(r)) = (&m.label, m.label_rect) else {
            continue;
        };
        let gap = rim_gap(rect(r), v2(m.at), m.radius);
        if gap > LABEL_REACH {
            let offender = json!({ "module": m.name, "label": label, "gap_to_rim": r1(gap), "most": LABEL_REACH });
            offenders.push((gap, offender));
        }
    }
    verdict(offenders)
}

fn labels_clear(s: &Snap) -> Value {
    let naming = |l: &Label| matches!(l.names.as_str(), "function" | "module");
    let mut offenders = Vec::new();
    for (i, label) in s.labels.iter().enumerate() {
        if !naming(label) {
            continue;
        }
        for &j in &label.overlaps {
            let Some(other) = s.labels.get(j) else {
                continue;
            };
            // Each pair of names once.
            if naming(other) && j < i {
                continue;
            }
            let both = rect(label.rect).intersect(rect(other.rect));
            let offender = json!({
                "label": label.text, "over_label": other.text,
                "overlap": [r1(both.width()), r1(both.height())],
            });
            offenders.push((both.area(), offender));
        }
    }
    let lit: Vec<&Node> = s.nodes.iter().filter(|n| n.on_screen && n.lit()).collect();
    for n in &s.nodes {
        let (Some(label), Some(r)) = (&n.label, n.label_rect) else {
            continue;
        };
        let pill = rect(r).shrink(1.0);
        for other in &lit {
            if std::ptr::eq(*other, n) {
                continue;
            }
            let into = other.radius - pill.distance_to_pos(pos(other.at));
            if into > 0.0 {
                let offender =
                    json!({ "label": label, "of": n.name, "over_dot": other.name, "by": r1(into) });
                offenders.push((into, offender));
            }
        }
    }
    let spheres: Vec<&Module> = s.spheres().filter(|m| m.on_screen).collect();
    for m in s.spheres() {
        let (Some(label), Some(r)) = (&m.label, m.label_rect) else {
            continue;
        };
        for other in &spheres {
            if other.name == m.name {
                continue;
            }
            let into = other.radius - rect(r).distance_to_pos(pos(other.at));
            if into > 1.0 {
                let offender = json!({ "label": label, "over_sphere": other.name, "by": r1(into) });
                offenders.push((into, offender));
            }
        }
    }
    verdict(offenders)
}

fn named_on_screen(s: &Snap) -> Value {
    let mut offenders = Vec::new();
    for label in &s.labels {
        if !matches!(label.names.as_str(), "function" | "module") {
            continue;
        }
        if label.cut_off {
            let offender = json!({ "label": label.text, "of": label.of, "rect": rect_json(label.rect), "why": "cut off by the graph's edge" });
            offenders.push((2.0, offender));
        }
        if !label.under.is_empty() {
            let offender = json!({ "label": label.text, "of": label.of, "rect": rect_json(label.rect), "under": label.under });
            offenders.push((1.0, offender));
        }
    }
    verdict(offenders)
}

fn text_sizes(s: &Snap) -> Value {
    let mut offenders = Vec::new();
    for label in s.labels.iter().filter(|l| l.names == "function") {
        let (least, most) = NAME_SIZES;
        let off = (least - label.size).max(label.size - most);
        if off > 0.01 {
            let offender =
                json!({ "label": label.text, "size": label.size, "allowed": [least, most] });
            offenders.push((off, offender));
        }
    }
    // Each size seen, to the half point, with a text drawn at it.
    let mut sizes: BTreeMap<i32, &str> = BTreeMap::new();
    let texts = s
        .labels
        .iter()
        .map(|l| (l.size, l.text.as_str()))
        .chain(s.text.iter().map(|t| (t.size, t.text.as_str())))
        .filter(|(size, _)| *size > 0.0);
    for (size, text) in texts {
        #[expect(clippy::cast_possible_truncation, reason = "a font size")]
        sizes.entry((size * 2.0).round() as i32).or_insert(text);
    }
    let seen: Vec<Value> = sizes
        .iter()
        .map(|(&half, text)| json!({ "size": f64::from(half) / 2.0, "e.g.": text }))
        .collect();
    if sizes.len() > MOST_TEXT_SIZES {
        let offender =
            json!({ "distinct_sizes": sizes.len(), "most": MOST_TEXT_SIZES, "sizes": seen });
        offenders.push((f32::MAX, offender));
    }
    with(
        verdict(offenders),
        "sizes",
        json!(
            sizes
                .keys()
                .map(|&h| f64::from(h) / 2.0)
                .collect::<Vec<_>>()
        ),
    )
}

fn panel_agrees(s: &Snap) -> Value {
    if s.bars.is_empty() {
        return skip("no journey bars drawn");
    }
    let node_of = |unit: &str| s.nodes.iter().find(|n| n.unit.as_deref() == Some(unit));
    let mut offenders = Vec::new();
    for bar in &s.bars {
        // The start's bar carries no number.
        let (Some(number), Some(unit)) = (bar.number.as_ref().filter(|n| !n.is_empty()), &bar.unit)
        else {
            continue;
        };
        if let Some(n) = node_of(unit)
            && !n.numbers.contains(number)
        {
            let offender = json!({ "bar": number, "name": bar.name, "node_numbers": n.numbers });
            offenders.push((1.0, offender));
        }
        for n in &s.nodes {
            if n.numbers.contains(number) && n.unit.as_ref() != Some(unit) {
                let offender =
                    json!({ "bar": number, "name": bar.name, "but_on_the_graph": n.name });
                offenders.push((1.0, offender));
            }
        }
    }
    if let Some(bar) = s.bars.iter().find(|b| b.cursor) {
        let marked: Vec<&Node> = s
            .nodes
            .iter()
            .filter(|n| n.roles.iter().any(|r| r == "cursor"))
            .collect();
        let drawn = bar.unit.as_deref().and_then(node_of);
        let agrees = marked.iter().all(|n| n.unit == bar.unit)
            && drawn.is_none_or(|n| n.roles.iter().any(|r| r == "cursor"));
        if !agrees {
            let offender = json!({
                "cursor_bar": bar.number, "name": bar.name,
                "cursor_on_the_graph": marked.iter().map(|n| &n.name).collect::<Vec<_>>(),
            });
            offenders.push((2.0, offender));
        }
    }
    verdict(offenders)
}

fn expanded_fill(s: &Snap, kinds: &[Kind]) -> Value {
    if !kinds.contains(&Kind::Expand) {
        return skip("no expand (x, double click) since the last snapshot");
    }
    if !s.expanded() {
        return skip("collapsed");
    }
    let Some(m) = s.expanded_sphere() else {
        return verdict(vec![(
            1.0,
            json!({ "why": "expanded, but no module drawn expanded" }),
        )]);
    };
    let free = s.free();
    let side = free.width().min(free.height());
    let fill = 2.0 * m.radius / side;
    let mut offenders = Vec::new();
    if !(EXPANDED_FILL.0..=EXPANDED_FILL.1).contains(&fill) {
        let offender = json!({
            "module": m.name, "fill": r2(fill), "allowed": [EXPANDED_FILL.0, EXPANDED_FILL.1],
            "diameter": r1(2.0 * m.radius), "free_area": [r1(free.width()), r1(free.height())],
        });
        offenders.push(((fill - 0.75).abs(), offender));
    }
    let out = circle_out(free, v2(m.at), m.radius);
    if out > EDGE {
        let offender = json!({ "module": m.name, "sticks_out_by": r1(out), "at": pt(m.at), "radius": r1(m.radius) });
        offenders.push((out, offender));
    }
    with(verdict(offenders), "fill", json!(r2(fill)))
}

fn framed(s: &Snap, kinds: &[Kind]) -> Value {
    let last = kinds.iter().rev().find(|k| **k != Kind::Still);
    let free = s.free();
    let mut offenders = Vec::new();
    match last {
        Some(Kind::FrameAll) => {
            for m in s.spheres() {
                let out = circle_out(free, v2(m.at), m.radius);
                if out > EDGE || !m.on_screen {
                    let offender = json!({ "module": m.name, "sticks_out_by": r1(out), "at": pt(m.at), "radius": r1(m.radius) });
                    offenders.push((out, offender));
                }
            }
            if let Some(camera) = s.state.camera
                && ((camera.yaw - VIEW.0).abs() > FRONT_SLACK
                    || (camera.pitch - VIEW.1).abs() > FRONT_SLACK)
            {
                let offender = json!({
                    "why": "R leaves the camera off the usual view", "yaw": r3(camera.yaw),
                    "pitch": r3(camera.pitch), "usual": [r3(VIEW.0), r3(VIEW.1)],
                });
                offenders.push((f32::MAX, offender));
            }
        }
        Some(Kind::FrameFocus) => {
            if s.expanded() {
                if let Some(m) = s.expanded_sphere() {
                    let out = circle_out(free, v2(m.at), m.radius);
                    if out > EDGE {
                        let offender = json!({ "module": m.name, "sticks_out_by": r1(out) });
                        offenders.push((out, offender));
                    }
                }
            } else {
                let lit: Vec<&Node> = s.nodes.iter().filter(|n| n.lit()).collect();
                if lit.is_empty() {
                    return skip("nothing lit for F to frame");
                }
                let inner = free.shrink(FRAME_MARGIN);
                for n in lit {
                    let out = circle_out(inner, v2(n.at), n.radius);
                    if out > 0.0 || !n.on_screen {
                        let offender = json!({ "function": n.name, "roles": n.roles, "at": pt(n.at), "out_by": r1(out) });
                        offenders.push((out, offender));
                    }
                }
            }
        }
        _ => {
            return skip(
                "the last step since the snapshot before that moves the view is not F or R",
            );
        }
    }
    verdict(offenders)
}

fn zoom_to_pointer(s: &Snap, before: Option<&Snap>, kinds: &[Kind]) -> Value {
    let turns = kinds.iter().filter(|k| **k == Kind::Zoom).count();
    if turns == 0 {
        return skip("no wheel turn since the last snapshot");
    }
    if turns > 1 || kinds.iter().any(|k| !matches!(k, Kind::Zoom | Kind::Still)) {
        return skip("more than one wheel turn, or other moves, since the last snapshot");
    }
    let (Some(before), Some(pointer)) = (before, s.state.pointer) else {
        return skip("no snapshot before, or no pointer");
    };
    let pointer = v2(pointer);
    // The sphere whose middle lay nearest the pointer: a sphere's size on screen
    // grows as its distance from the pointer does, when the zoom is about the pointer.
    let was: HashMap<&str, &Module> = before
        .spheres()
        .filter(|m| m.on_screen && m.radius > 0.0)
        .map(|m| (m.name.as_str(), m))
        .collect();
    let probe = s
        .spheres()
        .filter(|m| m.radius > 0.0)
        .filter_map(|m| Some((m, *was.get(m.name.as_str())?)))
        .min_by(|a, b| {
            let d = |m: &Module| v2(m.at).distance(pointer);
            d(a.1).total_cmp(&d(b.1))
        });
    let Some((now, then)) = probe else {
        return skip("no sphere on screen in both snapshots");
    };
    let k = now.radius / then.radius;
    let expected = pointer + (v2(then.at) - pointer) * k;
    let drift = v2(now.at).distance(expected);
    let allowed = ZOOM_DRIFT.0 + ZOOM_DRIFT.1 * expected.distance(pointer);
    let mut offenders = Vec::new();
    if drift > allowed {
        let offender = json!({
            "probe": now.name, "pointer": pt(pointer.to_array()), "was_at": pt(then.at),
            "now_at": pt(now.at), "expected_at": pt(expected.to_array()),
            "drift": r1(drift), "allowed": r1(allowed),
        });
        offenders.push((drift, offender));
    }
    with(verdict(offenders), "zoomed_by", json!(r2(k)))
}

fn zoom_bounds(s: &Snap, kinds: &[Kind]) -> Value {
    if !kinds.contains(&Kind::Zoom) {
        return skip("no wheel turn since the last snapshot");
    }
    let free = s.free();
    let spheres: Vec<&Module> = s.spheres().collect();
    if spheres.is_empty() {
        return skip("no module spheres drawn");
    }
    let circle =
        |m: &Module| egui::Rect::from_center_size(pos(m.at), egui::Vec2::splat(2.0 * m.radius));
    let mut offenders = Vec::new();
    if !spheres.iter().any(|m| circle(m).intersects(free)) {
        let offender = json!({ "why": "nothing of the map is on screen" });
        offenders.push((f32::MAX, offender));
    } else if spheres.iter().all(|m| free.contains_rect(circle(m))) {
        let extent = spheres
            .iter()
            .map(|m| circle(m))
            .reduce(egui::Rect::union)
            .unwrap_or(egui::Rect::NOTHING);
        let span = extent.width().max(extent.height());
        let least = ZOOM_OUT_LEAST * free.width().min(free.height());
        if span < least {
            let offender = json!({ "why": "zoomed out past the whole map", "map_spans": r1(span), "least": r1(least) });
            offenders.push((least - span, offender));
        }
    }
    verdict(offenders)
}

fn nothing_moved(s: &Snap, before: Option<&Snap>, kinds: &[Kind]) -> Value {
    let Some(before) = before else {
        return skip("no snapshot before");
    };
    if kinds.is_empty() {
        return skip("no step since the last snapshot");
    }
    if let Some(kind) = kinds
        .iter()
        .find(|k| !matches!(k, Kind::Still | Kind::Click | Kind::Depth))
    {
        return skip(&format!(
            "a step since the last snapshot may move the view ({kind:?})"
        ));
    }
    if kinds.contains(&Kind::Click) && s.expanded() && s.state.mode == "journey" {
        return skip("a click expanded in a journey may re-centre on what it clicks");
    }
    // A depth change may bring the camera round to show what it brought in.
    let camera_free = kinds.contains(&Kind::Depth);
    let (_, offenders) = compare(before, s, camera_free);
    verdict(offenders)
}

/// What moved between two snapshots, and, as offenders, everything that did: when
/// `camera_free`, only what moved in the world, the camera having had leave to move.
fn compare(before: &Snap, now: &Snap, camera_free: bool) -> (Value, Vec<(f32, Value)>) {
    let mut offenders = Vec::new();
    let camera = before.state.camera.zip(now.state.camera).map(|(a, b)| {
        let by = Vec3::from_array(a.target).distance(Vec3::from_array(b.target));
        let nearer = b.distance - a.distance;
        let turned = (b.yaw - a.yaw).abs().max((b.pitch - a.pitch).abs());
        let moved = by > MOVED_WORLD || nearer.abs() > MOVED_WORLD || turned > MOVED_ANGLE;
        json!({ "moved": moved, "target_by": r3(by), "distance_by": r3(nearer), "turned": r3(turned) })
    });
    let camera_moved = camera.as_ref().is_some_and(|c| c["moved"] == json!(true));
    let screen_counts = !(camera_free && camera_moved);
    if camera_moved && !camera_free {
        offenders.push((f32::MAX, json!({ "camera": camera })));
    }

    let was: HashMap<&str, &Node> = before
        .nodes
        .iter()
        .filter_map(|n| Some((n.unit.as_deref()?, n)))
        .collect();
    let mut shifts: Vec<(&str, f32, f32)> = now
        .nodes
        .iter()
        .filter_map(|n| {
            let then = was.get(n.unit.as_deref()?)?;
            let world = n.world.zip(then.world).map_or(0.0, |(a, b)| {
                Vec3::from_array(a).distance(Vec3::from_array(b))
            });
            Some((n.name.as_str(), world, v2(n.at).distance(v2(then.at))))
        })
        .collect();
    let compared = shifts.len();
    shifts.retain(|&(_, world, screen)| world > MOVED_WORLD || screen > MOVED_SCREEN);
    shifts.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.total_cmp(&a.2)));
    for &(name, world, screen) in &shifts {
        if world > MOVED_WORLD || screen_counts {
            let offender = json!({ "function": name, "world": r3(world), "screen": r1(screen) });
            offenders.push((world * 1e3 + screen, offender));
        }
    }

    let spheres: HashMap<&str, &Module> = before.spheres().map(|m| (m.name.as_str(), m)).collect();
    let mut sphere_shifts = Vec::new();
    for m in now.spheres() {
        let Some(then) = spheres.get(m.name.as_str()) else {
            continue;
        };
        let by = Vec3::from_array(m.world).distance(Vec3::from_array(then.world));
        let grew = m.world_radius - then.world_radius;
        if by > MOVED_WORLD || grew.abs() > MOVED_WORLD {
            let offender = json!({ "module": m.name, "moved": r3(by), "grew": r3(grew) });
            sphere_shifts.push(offender.clone());
            offenders.push(((by + grew.abs()) * 1e3, offender));
        }
    }

    let (labels, moved_labels) = labels_moved(before, now);
    if screen_counts {
        offenders.extend(
            moved_labels
                .iter()
                .map(|(by, offender)| (*by, offender.clone())),
        );
    }

    let units =
        |s: &Snap| -> Vec<String> { s.nodes.iter().filter_map(|n| n.unit.clone()).collect() };
    let (old, new) = (units(before), units(now));
    let summary = json!({
        "camera": camera,
        "nodes": {
            "compared": compared,
            "moved_in_world": shifts.iter().filter(|s| s.1 > MOVED_WORLD).count(),
            "moved_on_screen": shifts.iter().filter(|s| s.2 > MOVED_SCREEN).count(),
            "most_world": r3(shifts.iter().map(|s| s.1).fold(0.0, f32::max)),
            "most_screen": r1(shifts.iter().map(|s| s.2).fold(0.0, f32::max)),
            "examples": shifts.iter().take(8).map(|&(name, world, screen)| json!({
                "name": name, "world": r3(world), "screen": r1(screen),
            })).collect::<Vec<_>>(),
        },
        "modules": { "moved": sphere_shifts.len(), "examples": sphere_shifts.iter().take(6).collect::<Vec<_>>() },
        "labels": { "compared": labels, "moved": moved_labels.len() },
        "appeared": new.iter().filter(|u| !old.contains(u)).count(),
        "gone": old.iter().filter(|u| !new.contains(u)).count(),
    });
    (summary, offenders)
}

/// How many names show in both snapshots, and those that moved, with how far.
fn labels_moved(before: &Snap, now: &Snap) -> (usize, Vec<(f32, Value)>) {
    let key = |l: &Label| {
        matches!(l.names.as_str(), "function" | "module")
            .then(|| (l.names.clone(), l.of.clone()))
            .filter(|(_, of)| of.is_some())
    };
    let named: HashMap<(String, Option<String>), &Label> = before
        .labels
        .iter()
        .filter_map(|l| Some((key(l)?, l)))
        .collect();
    let mut compared = 0;
    let mut moved = Vec::new();
    for label in &now.labels {
        let Some(then) = key(label).and_then(|k| named.get(&k)) else {
            continue;
        };
        compared += 1;
        let by = label_shift(rect(then.rect), rect(label.rect));
        if by > MOVED_SCREEN {
            let offender = json!({
                "label": label.text, "from": rect_json(then.rect), "to": rect_json(label.rect), "by": r1(by),
            });
            moved.push((by, offender));
        }
    }
    (compared, moved)
}

/// How far a name moved as a whole, rather than grew or shrank about one edge or its
/// middle as its text changed: along whichever axis moved more, the least of how far
/// its two edges and its middle moved.
fn label_shift(a: egui::Rect, b: egui::Rect) -> f32 {
    let along = |a0: f32, a1: f32, b0: f32, b1: f32| {
        let middle = ((b0 + b1) - (a0 + a1)).abs() / 2.0;
        (b0 - a0).abs().min((b1 - a1).abs()).min(middle)
    };
    along(a.min.x, a.max.x, b.min.x, b.max.x).max(along(a.min.y, a.max.y, b.min.y, b.max.y))
}

/// How far a rectangle lies from a circle's rim: none when the rim crosses it.
fn rim_gap(r: egui::Rect, centre: Vec2, radius: f32) -> f32 {
    let c = pos(centre.to_array());
    let near = r.distance_to_pos(c);
    let far = [
        r.left_top(),
        r.right_top(),
        r.left_bottom(),
        r.right_bottom(),
    ]
    .iter()
    .map(|corner| corner.distance(c))
    .fold(0.0, f32::max);
    if radius < near {
        near - radius
    } else if radius > far {
        radius - far
    } else {
        0.0
    }
}

/// How far a circle sticks out of a rectangle, past its farthest side.
fn circle_out(r: egui::Rect, centre: Vec2, radius: f32) -> f32 {
    [
        r.min.x - (centre.x - radius),
        (centre.x + radius) - r.max.x,
        r.min.y - (centre.y - radius),
        (centre.y + radius) - r.max.y,
    ]
    .into_iter()
    .fold(0.0, f32::max)
}

/// Passes with no offenders; else fails, listing the worst first.
fn verdict(mut offenders: Vec<(f32, Value)>) -> Value {
    offenders.sort_by(|a, b| b.0.total_cmp(&a.0));
    let count = offenders.len();
    let listed: Vec<Value> = offenders.into_iter().take(LISTED).map(|(_, v)| v).collect();
    if count == 0 {
        json!({ "pass": true })
    } else {
        json!({ "pass": false, "count": count, "offenders": listed })
    }
}

fn fail(why: &str) -> Value {
    json!({ "pass": false, "why": why })
}

fn skip(why: &str) -> Value {
    json!({ "pass": null, "why": why })
}

fn with(mut result: Value, key: &str, value: Value) -> Value {
    result[key] = value;
    result
}

fn rect(r: [f32; 4]) -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(r[0], r[1]), egui::pos2(r[2], r[3]))
}

fn rect_json(r: [f32; 4]) -> Value {
    json!(r.map(r1))
}

fn v2(at: [f32; 2]) -> Vec2 {
    Vec2::from_array(at)
}

fn pos(at: [f32; 2]) -> egui::Pos2 {
    egui::pos2(at[0], at[1])
}

fn pt(at: [f32; 2]) -> Value {
    json!([r1(at[0]), r1(at[1])])
}

fn r1(v: f32) -> f64 {
    (f64::from(v) * 10.0).round() / 10.0
}

fn r2(v: f32) -> f64 {
    (f64::from(v) * 100.0).round() / 100.0
}

fn r3(v: f32) -> f64 {
    (f64::from(v) * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot of two modules side by side, `a` holding `f` and `g`, `b` holding
    /// `h`, with `f` named.
    fn scene() -> Value {
        json!({
            "graph_area": [0.0, 24.0, 1600.0, 1000.0],
            "state": {
                "mode": "browse", "nucleus": "modules", "pointer": [400.0, 500.0],
                "camera": { "target": [0.0, 0.0, 0.0], "distance": 100.0, "yaw": -0.45, "pitch": -0.38 },
            },
            "modules": [
                { "name": "a", "body": "open", "shown": true, "functions": 2, "at": [400.0, 500.0],
                  "radius": 100.0, "world": [-10.0, 0.0, 0.0], "world_radius": 5.0, "on_screen": true,
                  "label": "A", "label_rect": [380.0, 390.0, 420.0, 402.0] },
                { "name": "b", "body": "open", "shown": true, "functions": 1, "at": [1000.0, 500.0],
                  "radius": 80.0, "world": [10.0, 0.0, 0.0], "world_radius": 4.0, "on_screen": true },
            ],
            "nodes": [
                { "name": "f", "unit": "x:a::f", "module": "a", "roles": ["centre"], "at": [380.0, 500.0],
                  "radius": 5.0, "world": [-11.0, 0.0, 0.0], "world_radius": 0.25, "on_screen": true,
                  "label": "f", "label_rect": [390.0, 494.0, 410.0, 506.0] },
                { "name": "g", "unit": "x:a::g", "module": "a", "roles": ["speck"], "at": [440.0, 500.0],
                  "radius": 2.0, "world": [-8.0, 0.0, 0.0], "world_radius": 0.1, "on_screen": true },
                { "name": "h", "unit": "x:b::h", "module": "b", "roles": ["callee"], "at": [1000.0, 500.0],
                  "radius": 4.0, "world": [10.0, 0.0, 0.0], "world_radius": 0.2, "on_screen": true },
            ],
            "labels": [
                { "text": "A", "rect": [380.0, 390.0, 420.0, 402.0], "names": "module", "of": "a",
                  "overlaps": [], "cut_off": false, "under": [], "size": 12.0 },
                { "text": "f", "rect": [390.0, 494.0, 410.0, 506.0], "names": "function", "of": "f",
                  "overlaps": [], "cut_off": false, "under": [], "size": 12.0 },
            ],
            "bars": [],
            "overlays": [],
            "text": [{ "text": "codeviz", "size": 13.0 }],
        })
    }

    fn check(now: &Value, name: &str) -> Value {
        run(now, None, &[])[name].clone()
    }

    fn after(before: &Value, now: &Value, kinds: &[Kind], name: &str) -> Value {
        run(now, Some(before), kinds)[name].clone()
    }

    fn passes(result: &Value) -> bool {
        result["pass"] == json!(true)
    }

    #[test]
    fn a_tidy_scene_passes_every_check_that_applies() {
        let results = run(&scene(), None, &[]);
        for name in NAMES {
            assert_ne!(
                results[name]["pass"],
                json!(false),
                "{name}: {}",
                results[name]
            );
        }
        assert_eq!(results["nothing-moved"]["pass"], Value::Null);
        assert_eq!(results["panel-agrees"]["pass"], Value::Null);
    }

    #[test]
    fn a_dot_outside_its_sphere_is_named() {
        let mut s = scene();
        s["nodes"][1]["world"] = json!([-4.0, 0.0, 0.0]);
        let result = check(&s, "dots-in-spheres");
        assert!(!passes(&result));
        assert_eq!(result["offenders"][0]["function"], "g");
        assert_eq!(result["offenders"][0]["out_by"], 1.1);
    }

    #[test]
    fn dots_too_close_are_named_in_pairs_lit_first() {
        let mut s = scene();
        s["nodes"][1]["at"] = json!([385.0, 500.0]);
        s["nodes"][2]["at"] = json!([443.0, 500.0]);
        s["nodes"][2]["module"] = json!("a");
        let result = check(&s, "dots-apart");
        assert_eq!(result["count"], 1);
        assert_eq!(result["offenders"][0]["functions"], json!(["f", "g"]));
        assert_eq!(result["offenders"][0]["short_by"], 3.0);
    }

    #[test]
    fn a_lit_dot_too_small_or_any_too_big_is_named() {
        let mut s = scene();
        s["nodes"][0]["radius"] = json!(1.0);
        s["nodes"][1]["radius"] = json!(1.0);
        s["nodes"][2]["radius"] = json!(20.0);
        let result = check(&s, "dot-sizes");
        let named: Vec<&Value> = result["offenders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| &o["function"])
            .collect();
        assert_eq!(named, [&json!("h"), &json!("f")], "a speck may be small");
    }

    #[test]
    fn intersecting_spheres_and_sizes_out_of_step_are_named() {
        let mut s = scene();
        s["modules"][1]["world"] = json!([-2.0, 0.0, 0.0]);
        let result = check(&s, "spheres-apart");
        assert_eq!(result["offenders"][0]["modules"], json!(["a", "b"]));
        assert_eq!(result["offenders"][0]["overlap"], 1.0);

        s["modules"][1]["world_radius"] = json!(25.0);
        let result = check(&s, "sphere-sizes");
        assert_eq!(result["ratio"], 5.0);
        assert_eq!(result["offenders"][0]["largest"], "b");
        assert_eq!(
            result["offenders"][1]["why"],
            "holds more functions, but is smaller"
        );
    }

    #[test]
    fn a_name_far_from_its_dot_or_over_another_is_named() {
        let mut s = scene();
        s["nodes"][0]["label_rect"] = json!([500.0, 494.0, 520.0, 506.0]);
        let result = check(&s, "labels-near");
        assert_eq!(result["offenders"][0]["function"], "f");
        assert_eq!(result["offenders"][0]["gap"], 115.0);

        let mut s = scene();
        s["nodes"][1]["roles"] = json!(["caller"]);
        s["nodes"][1]["at"] = json!([405.0, 500.0]);
        s["labels"][0]["overlaps"] = json!([1]);
        s["labels"][1]["overlaps"] = json!([0]);
        let result = check(&s, "labels-clear");
        assert_eq!(result["count"], 2, "{result}");
        let kinds: Vec<bool> = result["offenders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o.get("over_dot").is_some())
            .collect();
        assert!(kinds.contains(&true) && kinds.contains(&false));
    }

    #[test]
    fn a_module_name_off_its_rim_is_named() {
        let mut s = scene();
        s["modules"][0]["label_rect"] = json!([380.0, 494.0, 420.0, 506.0]);
        let result = check(&s, "labels-near");
        assert_eq!(result["offenders"][0]["module"], "a");
        assert_eq!(result["offenders"][0]["gap_to_rim"], 79.1);
    }

    #[test]
    fn names_cut_off_or_under_a_panel_are_named() {
        let mut s = scene();
        s["labels"][1]["under"] = json!(["journey panel"]);
        s["labels"][0]["cut_off"] = json!(true);
        let result = check(&s, "named-on-screen");
        assert_eq!(result["count"], 2);
        assert_eq!(result["offenders"][0]["why"], "cut off by the graph's edge");
    }

    #[test]
    fn text_sizes_keep_to_a_few_and_names_to_their_range() {
        let mut s = scene();
        s["labels"][1]["size"] = json!(10.0);
        s["text"] = json!([
            { "text": "a", "size": 13.0 },
            { "text": "b", "size": 14.0 },
            { "text": "c", "size": 15.5 },
        ]);
        let result = check(&s, "text-sizes");
        assert_eq!(result["sizes"], json!([10.0, 12.0, 13.0, 14.0, 15.5]));
        assert_eq!(result["count"], 2);
        assert_eq!(result["offenders"][0]["distinct_sizes"], 5);
        assert_eq!(result["offenders"][1]["label"], "f");
    }

    #[test]
    fn graph_and_panel_must_agree_on_numbers_and_cursor() {
        let mut s = scene();
        s["nodes"][2]["numbers"] = json!(["1"]);
        s["nodes"][2]["roles"] = json!(["journey level 1", "cursor"]);
        s["bars"] = json!([
            { "number": "1", "name": "h", "unit": "x:b::h", "cursor": false },
            { "number": "2", "name": "g", "unit": "x:a::g", "cursor": true },
        ]);
        let result = check(&s, "panel-agrees");
        assert_eq!(result["count"], 2, "{result}");
        s["bars"][1]["cursor"] = json!(false);
        s["bars"][0]["cursor"] = json!(true);
        s["nodes"][1]["numbers"] = json!(["2"]);
        assert!(passes(&check(&s, "panel-agrees")));
    }

    #[test]
    fn the_expanded_module_fills_the_free_area_after_expanding() {
        let mut s = scene();
        s["state"]["nucleus"] = json!("expanded");
        s["modules"][0]["body"] = json!("nucleus");
        s["modules"][0]["at"] = json!([800.0, 512.0]);
        s["modules"][0]["radius"] = json!(400.0);
        let fill = |s: &Value| after(s, s, &[Kind::Expand], "expanded-fill");
        let result = fill(&s);
        assert!(passes(&result), "{result}");
        assert_eq!(result["fill"], 0.82);
        s["overlays"] = json!([{ "name": "journey panel", "rect": [0.0, 800.0, 1600.0, 1000.0] }]);
        let result = fill(&s);
        assert!(!passes(&result), "the panel takes the bottom: {result}");
        assert_eq!(result["count"], 2, "too big, and under the panel");
        assert_eq!(
            check(&s, "expanded-fill")["pass"],
            Value::Null,
            "only after x"
        );
    }

    #[test]
    fn the_expanded_module_is_the_one_the_state_names() {
        let mut s = scene();
        s["state"]["nucleus"] = json!("expanded");
        s["state"]["expanded"] = json!("b");
        s["modules"][0]["body"] = json!("nucleus");
        s["modules"][1]["at"] = json!([800.0, 512.0]);
        s["modules"][1]["radius"] = json!(400.0);
        let result = after(&s, &s, &[Kind::Expand], "expanded-fill");
        assert!(
            passes(&result),
            "b fills the view, not the nucleus a: {result}"
        );
    }

    #[test]
    fn r_shows_every_sphere_from_the_front() {
        let mut s = scene();
        let framed = |s: &Value| after(s, s, &[Kind::FrameAll, Kind::Still], "framed");
        assert!(passes(&framed(&s)));
        s["modules"][1]["at"] = json!([1580.0, 500.0]);
        s["state"]["camera"]["yaw"] = json!(-0.35);
        assert_eq!(VIEW.0, -0.45, "the scene's camera is at the usual view");
        let result = framed(&s);
        assert_eq!(result["count"], 2);
        assert_eq!(result["offenders"][1]["sticks_out_by"], 60.0);
        assert_eq!(
            after(&s, &s, &[Kind::FrameAll, Kind::Zoom], "framed")["pass"],
            Value::Null
        );
    }

    #[test]
    fn a_wheel_turn_keeps_what_was_under_the_pointer_there() {
        let before = scene();
        let mut now = scene();
        // Twice as near about the pointer, which sits on `a`'s middle.
        now["modules"][0]["radius"] = json!(200.0);
        now["modules"][1]["radius"] = json!(160.0);
        now["modules"][1]["at"] = json!([1600.0, 500.0]);
        let result = after(&before, &now, &[Kind::Zoom], "zoom-to-pointer");
        assert!(passes(&result), "{result}");
        assert_eq!(result["zoomed_by"], 2.0);
        // About the window's middle instead, with the pointer off it.
        now["state"]["pointer"] = json!([300.0, 500.0]);
        let mut was = before.clone();
        was["state"]["pointer"] = json!([300.0, 500.0]);
        now["modules"][0]["at"] = json!([0.0, 488.0]);
        let result = after(&was, &now, &[Kind::Zoom], "zoom-to-pointer");
        assert!(!passes(&result), "{result}");
        assert_eq!(result["offenders"][0]["expected_at"], json!([500.0, 500.0]));
    }

    #[test]
    fn zooming_out_stops_at_the_whole_map() {
        let mut s = scene();
        s["modules"][0]["at"] = json!([790.0, 500.0]);
        s["modules"][0]["radius"] = json!(5.0);
        s["modules"][1]["at"] = json!([810.0, 500.0]);
        s["modules"][1]["radius"] = json!(4.0);
        let result = after(&s, &s, &[Kind::Zoom], "zoom-bounds");
        assert_eq!(
            result["offenders"][0]["why"],
            "zoomed out past the whole map"
        );
    }

    #[test]
    fn nothing_may_move_after_a_click() {
        let before = scene();
        let mut now = scene();
        assert!(passes(&after(
            &before,
            &now,
            &[Kind::Click],
            "nothing-moved"
        )));
        now["nodes"][2]["at"] = json!([1003.0, 500.0]);
        now["labels"][1]["rect"] = json!([380.0, 494.0, 400.0, 506.0]);
        now["state"]["camera"]["yaw"] = json!(0.1);
        let result = after(&before, &now, &[Kind::Click], "nothing-moved");
        assert_eq!(result["count"], 3, "{result}");
        assert!(result["offenders"][0].get("camera").is_some());
        assert_eq!(
            after(&before, &now, &[Kind::Click, Kind::Moves], "nothing-moved")["pass"],
            Value::Null,
            "a step that may move the view leaves the check out"
        );
        // A depth change may turn the camera, but must move nothing in the world.
        let result = after(&before, &now, &[Kind::Depth], "nothing-moved");
        assert!(passes(&result), "{result}");
        now["modules"][1]["world"] = json!([11.0, 0.0, 0.0]);
        let result = after(&before, &now, &[Kind::Depth], "nothing-moved");
        assert_eq!(result["offenders"][0]["module"], "b");
    }

    #[test]
    fn a_name_whose_text_grows_from_its_edge_has_not_moved() {
        let a = rect([100.0, 10.0, 140.0, 22.0]);
        assert!(label_shift(a, rect([100.0, 10.0, 180.0, 22.0])) < MOVED_SCREEN);
        assert!(label_shift(a, rect([80.0, 10.0, 160.0, 22.0])) < MOVED_SCREEN);
        assert_eq!(label_shift(a, rect([110.0, 10.0, 150.0, 22.0])), 10.0);
    }

    #[test]
    fn says_what_moved_and_sums_a_session_up() {
        let before = scene();
        let mut now = scene();
        now["nodes"][0]["world"] = json!([-11.5, 0.0, 0.0]);
        now["nodes"].as_array_mut().unwrap().pop();
        let moved = moved(&before, &now, "01-open");
        assert_eq!(moved["since"], "01-open");
        assert_eq!(moved["nodes"]["moved_in_world"], 1);
        assert_eq!(moved["gone"], 1);

        let ok = run(&scene(), None, &[]);
        let mut bad = ok.clone();
        bad["dots-apart"] = json!({ "pass": false });
        let summary = summarise(&[("01-a".to_owned(), ok), ("02-b".to_owned(), bad)]);
        assert_eq!(summary["dots-apart"]["failed_in"], json!(["02-b"]));
        assert_eq!(summary["dots-apart"]["passed"], 1);
        assert_eq!(summary["nothing-moved"]["not_applied"], 2);
        assert_eq!(summary["dots-apart"]["pass"], false);
    }
}
