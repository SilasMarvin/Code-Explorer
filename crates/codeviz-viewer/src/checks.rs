//! Checks run on every snapshot of a scripted session, from the snapshot's own JSON
//! (see [`crate::truth::scene`]), so what they judge is what the snapshot reports.
//! Some hold of any one picture: dots inside their spheres and apart, spheres apart,
//! dot and sphere sizes, names by their dots and clear of the rest and on screen, a
//! few text sizes, graph and journey panel agreeing, the camera outside every sphere,
//! no sphere cut off in plain view, arrows of a size, module names placed one way,
//! everything named that has room, colors that mean their role. Some hold after a
//! kind of step, judged against the snapshot before: the expanded module filling the
//! view after `x`, F and R framing what they frame, a wheel turn zooming about the
//! pointer and within bounds, the map on screen after zooming out, a journey filling
//! the view once framed, nothing moving after a step that must move nothing, and,
//! while the camera is not where the user's hands put it, what is in focus on screen.
//! Each passes (`pass: true`), fails with the offending functions, labels and pixel
//! numbers, or does not apply (`pass: null`, saying why).

use std::collections::{BTreeMap, HashMap};

use bevy::math::{Vec2, Vec3};
use bevy_egui::egui;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    colors::{CALLEE, CALLER, CURSOR, DEEPER, FURTHER},
    nucleus::VIEW,
    script::Kind,
};

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
/// rim, edge to edge, in points.
const LABEL_REACH: f32 = 40.0;
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
/// How much nearer another lit dot may be to a function's name than its own, edge
/// to edge, in points, before the name reads as the other's.
const NEARER: f32 = 1.0;
/// Share of the map, by area on screen, that must lie inside the free area after
/// zooming out or R, when the whole map would fit there.
const MAP_SHOWN: f32 = 0.9;
/// How far beyond an edge of the graph's part of the window a panel may start, or
/// the window end, for the edge to hide what it cuts, in points.
const HIDDEN_EDGE: f32 = 4.0;
/// Widest an arrow's line or shaft shows, in points; an arrow between modules
/// carrying [`MANY_CALLS`] or more may be [`MANY_WIDER`] wider.
const ARROW_WIDTH: f32 = 2.5;
const MANY_CALLS: usize = 10;
const MANY_WIDER: f32 = 1.0;
/// Least and most an arrow's head shows, in points: its length along the line, or
/// half its width when that is more, as for a head seen end on.
const HEAD_SIZE: (f32, f32) = (5.0, 12.0);
/// How far a module's name may sit off its sphere's middle, across, and how far
/// above its top edge, in points.
const NAME_CENTRED: f32 = 3.0;
const NAME_ABOVE: f32 = 8.0;
/// How far from its dot a name would be put to see whether it fits, and the pad
/// round its text, in points.
const NAME_GAP: f32 = 4.0;
const NAME_PAD: f32 = 2.0;
/// A character's width as a share of its font size, when no name drawn says.
const CHAR_WIDTH: f32 = 0.55;
/// Least share of the free area a framed journey's lit dots and their names span,
/// along whichever of its width and height they come nearer filling.
const JOURNEY_FILL: f32 = 0.7;
/// How far a color's hue may be from its role's, in degrees, and how grey a color
/// is that carries no role's hue (saturation, 0 to 1).
const HUE_SLACK: f32 = 25.0;
const GREY: f32 = 0.3;
/// A color this near white, in every channel (0 to 1), reads as the selection's.
const WHITE: f32 = 0.85;
/// Offenders listed per check; `count` says how many in all.
const LISTED: usize = 12;

/// Every check, in the order a snapshot lists them.
pub(crate) const NAMES: [&str; 24] = [
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
    "focus-on-screen",
    "camera-outside",
    "map-on-screen",
    "spheres-unclipped",
    "arrow-sizes",
    "module-names",
    "named-when-room",
    "journey-fills",
    "colours",
];

/// What happened since the snapshot before: what each step may move, and whether the
/// camera stands where the user's hands put it (a wheel turn, an orbit or a pan since
/// the view last framed itself).
#[derive(Debug, Default, Clone)]
pub(crate) struct Since {
    pub(crate) kinds: Vec<Kind>,
    pub(crate) by_hand: bool,
}

impl Since {
    /// The last step since that moves the view, if any.
    fn last_move(&self) -> Option<Kind> {
        self.kinds.iter().rev().copied().find(|k| *k != Kind::Still)
    }

    fn has(&self, kind: Kind) -> bool {
        self.kinds.contains(&kind)
    }

    fn zooms(&self) -> usize {
        self.kinds
            .iter()
            .filter(|k| matches!(k, Kind::Zoom { .. }))
            .count()
    }
}

/// What the checks read of a snapshot.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Snap {
    window: [f32; 2],
    graph_area: [f32; 4],
    camera: Option<Eye>,
    state: State,
    nodes: Vec<Node>,
    modules: Vec<Module>,
    arrows: Vec<Arrow>,
    module_arrows: Vec<Arrow>,
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
    start: Option<String>,
    /// The module expanded, by name.
    expanded: Option<String>,
    help: bool,
    pointer: Option<[f32; 2]>,
    camera: Option<Camera>,
}

/// Where the camera is in the world, and where what it looks at shows on screen.
#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(default)]
struct Eye {
    at: [f32; 3],
    target_at: Option<[f32; 2]>,
}

/// An arrow as drawn: a call's from function to function, or a module arrow's from
/// module to module.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Arrow {
    from: String,
    to: String,
    from_unit: Option<String>,
    to_unit: Option<String>,
    flow: String,
    level: Option<u32>,
    color: Option<String>,
    calls: usize,
    drawn: Option<bool>,
    width: Option<f32>,
    head: Option<f32>,
    head_width: Option<f32>,
    on_screen: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Ring {
    radius: f32,
    color: String,
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
    color: Option<String>,
    rings: Vec<Ring>,
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
    unit: Option<String>,
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

    fn is(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// Whether it is the selection, the journey's start or cursor, or set aside.
    fn focus(&self) -> bool {
        ["start", "selection", "cursor", "aside"]
            .iter()
            .any(|r| self.is(r))
    }

    /// A faint dot with no name: nothing a reader tells apart.
    fn faint(&self) -> bool {
        !self.lit() && self.label.is_none()
    }

    /// Lit in a journey: the start, a call or a caller of it (not one folded into
    /// its module's count).
    fn in_journey(&self) -> bool {
        self.roles.iter().any(|r| {
            matches!(r.as_str(), "start" | "cursor" | "caller") || r.starts_with("journey level")
        })
    }
}

impl Snap {
    fn expanded(&self) -> bool {
        self.state.nucleus == "expanded"
    }

    fn spheres(&self) -> impl Iterator<Item = &Module> {
        self.modules.iter().filter(|m| m.shown)
    }

    /// The function drawn for a unit.
    fn node(&self, unit: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.unit.as_deref() == Some(unit))
    }

    /// Whether a point lies under an overlay.
    fn under_overlay(&self, r: egui::Rect) -> bool {
        self.overlays.iter().any(|o| rect(o.rect).intersects(r))
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
/// what happened between them.
pub(crate) fn run(now: &Value, before: Option<&Value>, since: &Since) -> Value {
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
        expanded_fill(&now, since),
        framed(&now, since),
        zoom_to_pointer(&now, before, since),
        zoom_bounds(&now, since),
        nothing_moved(&now, before, since),
        focus_on_screen(&now, since),
        camera_outside(&now),
        map_on_screen(&now, since),
        spheres_unclipped(&now),
        arrow_sizes(&now),
        module_names(&now),
        named_when_room(&now),
        journey_fills(&now, before, since),
        colours(&now),
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
            // A faint dot with no name is nothing a reader tells apart.
            if a.faint() || b.faint() {
                continue;
            }
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
    let gap = |r: egui::Rect, n: &Node| (r.distance_to_pos(pos(n.at)) - n.radius).max(0.0);
    let lit: Vec<&Node> = s.nodes.iter().filter(|n| n.on_screen && n.lit()).collect();
    for n in &s.nodes {
        let (Some(label), Some(r)) = (&n.label, n.label_rect) else {
            continue;
        };
        let mine = gap(rect(r), n);
        // The lit dot nearest the name, but its own.
        let nearest = lit
            .iter()
            .filter(|other| other.unit != n.unit)
            .map(|other| (other, gap(rect(r), other)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if !n.on_screen {
            let offender =
                json!({ "function": n.name, "label": label, "why": "its dot is off screen" });
            offenders.push((f32::MAX, offender));
        } else if mine > LABEL_REACH {
            let offender =
                json!({ "function": n.name, "label": label, "gap": r1(mine), "most": LABEL_REACH });
            offenders.push((mine, offender));
        } else if let Some((other, theirs)) = nearest
            && theirs + NEARER < mine
        {
            let offender = json!({
                "function": n.name, "label": label, "nearer_dot": other.name,
                "own_gap": r1(mine), "their_gap": r1(theirs),
            });
            offenders.push((mine - theirs, offender));
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
    verdict(offenders)
}

fn named_on_screen(s: &Snap) -> Value {
    if s.state.help {
        return skip("the key help is open over the graph");
    }
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
    let mut offenders = Vec::new();
    for bar in &s.bars {
        let Some(unit) = &bar.unit else {
            continue;
        };
        let drawn = s.node(unit);
        // The start's bar carries no number.
        if let Some(number) = bar.number.as_ref().filter(|n| !n.is_empty()) {
            if let Some(n) = drawn
                && !n.numbers.contains(number)
            {
                let offender =
                    json!({ "bar": number, "name": bar.name, "node_numbers": n.numbers });
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
        // The same name on the bar as on the graph.
        if let Some(n) = drawn
            && !same_name(&bar.name, n)
        {
            let offender = json!({
                "bar": bar.number, "bar_says": bar.name,
                "graph_says": n.label.as_deref().unwrap_or(&n.name),
            });
            offenders.push((0.5, offender));
        }
    }
    let marked: Vec<&Node> = s.nodes.iter().filter(|n| n.is("cursor")).collect();
    let cursor_bars: Vec<&Bar> = s.bars.iter().filter(|b| b.cursor).collect();
    if !marked.is_empty() && cursor_bars.len() != 1 {
        let offender = json!({
            "why": "the graph has a cursor, but the panel shows not one cursor bar",
            "cursor_on_the_graph": marked.iter().map(|n| &n.name).collect::<Vec<_>>(),
            "cursor_bars": cursor_bars.iter().map(|b| &b.name).collect::<Vec<_>>(),
        });
        offenders.push((3.0, offender));
    }
    if let [bar] = cursor_bars.as_slice() {
        let drawn = bar.unit.as_deref().and_then(|u| s.node(u));
        let agrees =
            marked.iter().all(|n| n.unit == bar.unit) && drawn.is_none_or(|n| n.is("cursor"));
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

/// Whether a bar's name is the one the graph gives its function: what the function's
/// label leads with after its numbers, or with no label, its name.
fn same_name(bar: &str, n: &Node) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    match &n.label {
        Some(label) => {
            let lead = label
                .trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | '+' | ' '));
            lead.strip_prefix(bar)
                .is_some_and(|rest| !rest.starts_with(word))
        }
        None => bar == n.name,
    }
}

fn expanded_fill(s: &Snap, since: &Since) -> Value {
    if !since.has(Kind::Expand) {
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

fn framed(s: &Snap, since: &Since) -> Value {
    let free = s.free();
    let mut offenders = Vec::new();
    match since.last_move() {
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

fn zoom_to_pointer(s: &Snap, before: Option<&Snap>, since: &Since) -> Value {
    let turns = since.zooms();
    if turns == 0 {
        return skip("no wheel turn since the last snapshot");
    }
    let others = since
        .kinds
        .iter()
        .any(|k| !matches!(k, Kind::Zoom { .. } | Kind::Still));
    if turns > 1 || others {
        return skip("more than one wheel turn, or other moves, since the last snapshot");
    }
    let (Some(before), Some(pointer)) = (before, s.state.pointer) else {
        return skip("no snapshot before, or no pointer");
    };
    let pointer = v2(pointer);
    // A point of the world in both snapshots, nearest the pointer: a sphere's middle
    // on screen, else any sphere's, else a function's. How big it shows grows as its
    // distance from the pointer does, when the zoom is about the pointer.
    let spheres = |snap: &Snap, on_screen: bool| -> Vec<(String, [f32; 2], f32)> {
        snap.spheres()
            .filter(|m| m.radius > 0.0 && (m.on_screen || !on_screen))
            .map(|m| (m.name.clone(), m.at, m.radius))
            .collect()
    };
    let dots = |snap: &Snap| -> Vec<(String, [f32; 2], f32)> {
        snap.nodes
            .iter()
            .filter(|n| n.radius > 0.0)
            .filter_map(|n| Some((n.unit.clone()?, n.at, n.radius)))
            .collect()
    };
    let probe = |was: Vec<(String, [f32; 2], f32)>, now: Vec<(String, [f32; 2], f32)>| {
        now.into_iter()
            .filter_map(|(name, at, radius)| {
                let then = was.iter().find(|w| w.0 == name)?;
                Some((name, at, radius, then.1, then.2))
            })
            .min_by(|a, b| {
                v2(a.3)
                    .distance(pointer)
                    .total_cmp(&v2(b.3).distance(pointer))
            })
    };
    let found = probe(spheres(before, true), spheres(s, false))
        .or_else(|| probe(spheres(before, false), spheres(s, false)))
        .or_else(|| probe(dots(before), dots(s)));
    let Some((name, now_at, now_radius, then_at, then_radius)) = found else {
        return skip("nothing drawn in both snapshots");
    };
    let k = now_radius / then_radius;
    let expected = pointer + (v2(then_at) - pointer) * k;
    let drift = v2(now_at).distance(expected);
    let allowed = ZOOM_DRIFT.0 + ZOOM_DRIFT.1 * expected.distance(pointer);
    let mut offenders = Vec::new();
    if drift > allowed {
        let offender = json!({
            "probe": name, "pointer": pt(pointer.to_array()), "was_at": pt(then_at),
            "now_at": pt(now_at), "expected_at": pt(expected.to_array()),
            "drift": r1(drift), "allowed": r1(allowed),
        });
        offenders.push((drift, offender));
    }
    with(verdict(offenders), "zoomed_by", json!(r2(k)))
}

fn zoom_bounds(s: &Snap, since: &Since) -> Value {
    if since.zooms() == 0 {
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

fn nothing_moved(s: &Snap, before: Option<&Snap>, since: &Since) -> Value {
    let Some(before) = before else {
        return skip("no snapshot before");
    };
    if since.kinds.is_empty() {
        return skip("no step since the last snapshot");
    }
    if let Some(kind) = since
        .kinds
        .iter()
        .find(|k| !matches!(k, Kind::Still | Kind::Click | Kind::Depth))
    {
        return skip(&format!(
            "a step since the last snapshot may move the view ({kind:?})"
        ));
    }
    if since.has(Kind::Click) && s.expanded() && s.state.mode == "journey" {
        return skip("a click expanded in a journey may re-centre on what it clicks");
    }
    // A depth change may draw the camera back to show what it brought in, when that
    // would be off the free area, and no more.
    let depth = since.has(Kind::Depth);
    let (_, mut offenders) = compare(before, s, depth);
    if depth {
        offenders.extend(depth_camera(before, s));
    }
    offenders.extend(gone_labels(before, s));
    verdict(offenders)
}

/// How the camera moved on a depth change, when it should not have: anything but
/// drawing back, or drawing back when all that came in would have shown anyway.
fn depth_camera(before: &Snap, now: &Snap) -> Vec<(f32, Value)> {
    let (Some(a), Some(b)) = (before.state.camera, now.state.camera) else {
        return Vec::new();
    };
    let by = Vec3::from_array(a.target).distance(Vec3::from_array(b.target));
    let back = b.distance - a.distance;
    let turned = (b.yaw - a.yaw).abs().max((b.pitch - a.pitch).abs());
    if by <= MOVED_WORLD && back.abs() <= MOVED_WORLD && turned <= MOVED_ANGLE {
        return Vec::new();
    }
    let camera = json!({ "target_by": r3(by), "distance_by": r3(back), "turned": r3(turned) });
    if by > MOVED_WORLD || turned > MOVED_ANGLE || back < 0.0 {
        let why = "a depth change may only draw the camera back";
        return vec![(f32::MAX, json!({ "camera": camera, "why": why }))];
    }
    // Where what came in would have shown from where the camera was: drawn back
    // about what it looks at, a point there shows as much further out from it as
    // the camera is further.
    let Some(middle) = now.camera.and_then(|c| c.target_at) else {
        return Vec::new();
    };
    let old: Vec<&str> = before
        .nodes
        .iter()
        .filter_map(|n| n.unit.as_deref())
        .collect();
    let free = before.free();
    let k = b.distance / a.distance.max(f32::EPSILON);
    let came: Vec<&Node> = now
        .nodes
        .iter()
        .filter(|n| n.unit.as_deref().is_some_and(|u| !old.contains(&u)))
        .collect();
    let off = came.iter().any(|n| {
        let was = v2(middle) + (v2(n.at) - v2(middle)) * k;
        circle_out(free, was, n.radius) > EDGE
    });
    if off {
        return Vec::new();
    }
    let why = if came.is_empty() {
        "the camera drew back, but nothing came in"
    } else {
        "the camera drew back, but what came in would have shown from where it was"
    };
    vec![(
        f32::MAX,
        json!({ "camera": camera, "why": why, "came_in": came.len() }),
    )]
}

/// Names that went while what they name is still drawn on screen: a step that only
/// reads takes no name away.
fn gone_labels(before: &Snap, now: &Snap) -> Vec<(f32, Value)> {
    let named: Vec<(String, Option<String>)> = now.labels.iter().filter_map(label_key).collect();
    before
        .labels
        .iter()
        .filter_map(|l| Some((label_key(l)?, l)))
        .filter(|(key, _)| !named.contains(key))
        .filter(|((kind, of), _)| {
            let Some(of) = of else {
                return false;
            };
            if kind == "function" {
                now.node(of).is_some_and(|n| n.on_screen)
            } else {
                now.spheres().any(|m| &m.name == of && m.on_screen)
            }
        })
        .map(|(_, l)| {
            let offender = json!({ "label": l.text, "gone": true, "was_at": rect_json(l.rect) });
            (MOVED_SCREEN, offender)
        })
        .collect()
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
        "labels": {
            "compared": labels,
            "moved": moved_labels.len(),
            "gone": gone_labels(before, now).len(),
        },
        "appeared": new.iter().filter(|u| !old.contains(u)).count(),
        "gone": old.iter().filter(|u| !new.contains(u)).count(),
    });
    (summary, offenders)
}

/// What a name on the graph names, to find it again: a function's by its unit, a
/// module's by its name.
fn label_key(l: &Label) -> Option<(String, Option<String>)> {
    match l.names.as_str() {
        "function" => Some(("function".to_owned(), Some(l.unit.clone()?))),
        "module" => Some(("module".to_owned(), Some(l.of.clone()?))),
        _ => None,
    }
}

/// How many names show in both snapshots, and those that moved, with how far.
fn labels_moved(before: &Snap, now: &Snap) -> (usize, Vec<(f32, Value)>) {
    let named: HashMap<(String, Option<String>), &Label> = before
        .labels
        .iter()
        .filter_map(|l| Some((label_key(l)?, l)))
        .collect();
    let mut compared = 0;
    let mut moved = Vec::new();
    for label in &now.labels {
        let Some(then) = label_key(label).and_then(|k| named.get(&k)) else {
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

fn focus_on_screen(s: &Snap, since: &Since) -> Value {
    if since.by_hand {
        return skip("the camera is where the user's hands put it");
    }
    let free = s.free();
    let focus: Vec<&Node> = s.nodes.iter().filter(|n| n.focus()).collect();
    if focus.is_empty() {
        return skip("no selection, start or cursor drawn");
    }
    let offenders = focus
        .into_iter()
        .filter_map(|n| {
            let out = circle_out(free, v2(n.at), n.radius);
            (out > EDGE).then(|| {
                let offender = json!({ "function": n.name, "roles": n.roles, "at": pt(n.at), "out_by": r1(out) });
                (out, offender)
            })
        })
        .collect();
    verdict(offenders)
}

fn camera_outside(s: &Snap) -> Value {
    let Some(eye) = s.camera else {
        return skip("no camera");
    };
    let at = Vec3::from_array(eye.at);
    let offenders = s
        .spheres()
        .filter_map(|m| {
            let into = m.world_radius - at.distance(Vec3::from_array(m.world));
            (into > 0.0).then(|| {
                let offender = json!({ "module": m.name, "camera_inside_by": r3(into), "sphere_radius": r3(m.world_radius) });
                (into / m.world_radius.max(f32::EPSILON), offender)
            })
        })
        .collect();
    verdict(offenders)
}

fn map_on_screen(s: &Snap, since: &Since) -> Value {
    let out = since
        .kinds
        .iter()
        .any(|k| matches!(k, Kind::Zoom { out: true } | Kind::FrameAll));
    if !out {
        return skip("no wheel turn out, nor R, since the last snapshot");
    }
    let free = s.free();
    let spheres: Vec<&Module> = s.spheres().filter(|m| m.radius > 0.0).collect();
    let Some(extent) = spheres
        .iter()
        .map(|m| egui::Rect::from_center_size(pos(m.at), egui::Vec2::splat(2.0 * m.radius)))
        .reduce(egui::Rect::union)
    else {
        return skip("no module spheres drawn");
    };
    if extent.width() > free.width() || extent.height() > free.height() {
        return skip("the map is bigger than the free area at this zoom");
    }
    let area = |m: &&Module| m.radius * m.radius;
    let all: f32 = spheres.iter().map(area).sum();
    let inside: f32 = spheres
        .iter()
        .filter(|m| circle_out(free, v2(m.at), m.radius) <= EDGE)
        .map(area)
        .sum();
    let share = inside / all.max(f32::EPSILON);
    let mut offenders = Vec::new();
    if share < MAP_SHOWN {
        let outside: Vec<&str> = spheres
            .iter()
            .filter(|m| circle_out(free, v2(m.at), m.radius) > EDGE)
            .map(|m| m.name.as_str())
            .collect();
        let offender = json!({
            "why": "the whole map would fit in the free area, but not this much of it is there",
            "inside": r2(share), "least": MAP_SHOWN, "map": rect_json([extent.min.x, extent.min.y, extent.max.x, extent.max.y]),
            "outside": outside,
        });
        offenders.push((MAP_SHOWN - share, offender));
    }
    with(verdict(offenders), "inside", json!(r2(share)))
}

fn spheres_unclipped(s: &Snap) -> Value {
    let graph = rect(s.graph_area);
    let free = s.free();
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(s.window[0], s.window[1]));
    if !graph.is_positive() {
        return skip("no graph area");
    }
    // An edge of the graph past which the window shows nothing of the graph, but no
    // panel or window edge either: what it cuts shows cut off.
    let panel_below = |y: f32| {
        s.overlays.iter().any(|o| {
            let r = rect(o.rect);
            r.width() >= graph.width() * 0.5 && (r.min.y - y).abs() <= HIDDEN_EDGE
        })
    };
    let bottom = graph.max.y < window.max.y - HIDDEN_EDGE && !panel_below(graph.max.y);
    let left = graph.min.x > window.min.x + HIDDEN_EDGE;
    let right = graph.max.x < window.max.x - HIDDEN_EDGE;
    let mut offenders = Vec::new();
    for m in s.spheres().filter(|m| m.radius > 0.0) {
        // Zoomed past it: bigger than the free area, it cannot but be cut.
        if 2.0 * m.radius > free.width().min(free.height()) {
            continue;
        }
        let c = v2(m.at);
        let across = c.x + m.radius > graph.min.x && c.x - m.radius < graph.max.x;
        let down = c.y + m.radius > graph.min.y && c.y - m.radius < graph.max.y;
        let cut = [
            (
                bottom && across && (c.y - graph.max.y).abs() < m.radius,
                "bottom",
            ),
            (left && down && (c.x - graph.min.x).abs() < m.radius, "left"),
            (
                right && down && (c.x - graph.max.x).abs() < m.radius,
                "right",
            ),
        ];
        for (_, edge) in cut.iter().filter(|(cut, _)| *cut) {
            let offender = json!({
                "module": m.name, "cut_by": format!("the graph's {edge} edge, in open window"),
                "at": pt(m.at), "radius": r1(m.radius), "graph_area": rect_json(s.graph_area),
            });
            offenders.push((m.radius, offender));
        }
    }
    verdict(offenders)
}

fn arrow_sizes(s: &Snap) -> Value {
    let mut offenders = Vec::new();
    let mut judged = 0;
    let shown = |a: &&Arrow| a.on_screen && a.drawn != Some(false);
    for (kind, arrows) in [("call", &s.arrows), ("module", &s.module_arrows)] {
        for a in arrows.iter().filter(shown) {
            judged += 1;
            let most = if kind == "module" && a.calls >= MANY_CALLS {
                ARROW_WIDTH + MANY_WIDER
            } else {
                ARROW_WIDTH
            };
            let width = a.width.unwrap_or(0.0);
            if width > most {
                let offender = json!({ "arrow": kind, "from": a.from, "to": a.to, "width": r1(width), "most": most });
                offenders.push((width - most, offender));
            }
            let head = a.head.unwrap_or(0.0).max(a.head_width.unwrap_or(0.0) / 2.0);
            let (least, most) = HEAD_SIZE;
            if !(least..=most).contains(&head) {
                let offender = json!({ "arrow": kind, "from": a.from, "to": a.to, "head": r1(head), "allowed": [least, most] });
                offenders.push(((least - head).max(head - most), offender));
            }
        }
    }
    // One head per arrow: a module arrow ends in a head at the sphere's edge only
    // when no call drawn along it goes on to a dot with a head of its own.
    let module = |unit: &Option<String>| {
        unit.as_deref()
            .and_then(|u| s.node(u))
            .and_then(|n| n.module.clone())
    };
    for m in s.module_arrows.iter().filter(shown) {
        let through = s.arrows.iter().filter(shown).find(|a| {
            module(&a.from_unit).as_deref() == Some(m.from.as_str())
                && module(&a.to_unit).as_deref() == Some(m.to.as_str())
        });
        if let Some(call) = through {
            let offender = json!({
                "arrow": "module", "from": m.from, "to": m.to,
                "why": "a head at the sphere's edge, and another where the call goes on to its dot",
                "call": [call.from, call.to],
            });
            offenders.push((1.0, offender));
        }
    }
    if judged == 0 {
        return skip("no arrow drawn on screen");
    }
    verdict(offenders)
}

fn module_names(s: &Snap) -> Value {
    let spheres: Vec<&Module> = s.spheres().filter(|m| m.on_screen).collect();
    let mut offenders = Vec::new();
    let mut named = 0;
    for m in s.spheres() {
        let (Some(label), Some(r)) = (&m.label, m.label_rect) else {
            continue;
        };
        named += 1;
        let r = rect(r);
        let off_middle = (r.center().x - m.at[0]).abs();
        let above = (m.at[1] - m.radius) - r.max.y;
        if off_middle > NAME_CENTRED || !(-EDGE..=NAME_ABOVE).contains(&above) {
            let offender = json!({
                "module": m.name, "label": label,
                "why": "not centred just above its sphere's top edge",
                "off_middle": r1(off_middle), "above_top_by": r1(above),
            });
            offenders.push((off_middle.max(above.abs()), offender));
        }
        for other in &spheres {
            if other.name == m.name {
                continue;
            }
            let into = other.radius - r.distance_to_pos(pos(other.at));
            if into > 1.0 {
                let offender = json!({ "module": m.name, "label": label, "over_sphere": other.name, "by": r1(into) });
                offenders.push((into + 100.0, offender));
            }
        }
        let own = s
            .nodes
            .iter()
            .filter(|n| n.on_screen && n.module.as_ref() == Some(&m.name) && n.radius > 0.0);
        for n in own {
            let into = n.radius - r.shrink(1.0).distance_to_pos(pos(n.at));
            if into > 0.0 {
                let offender = json!({ "module": m.name, "label": label, "over_its_dot": n.name, "by": r1(into) });
                offenders.push((into + 100.0, offender));
            }
        }
    }
    if named == 0 {
        return skip("no module named");
    }
    verdict(offenders)
}

fn named_when_room(s: &Snap) -> Value {
    let free = s.free();
    // How wide a character shows, by the names drawn.
    let mut widths: Vec<f32> = s
        .labels
        .iter()
        .filter(|l| l.names == "function" && l.size > 0.0)
        .map(|l| rect(l.rect).width() / (l.text.trim().chars().count().max(1) as f32 * l.size))
        .collect();
    widths.sort_by(f32::total_cmp);
    let per_size = widths.get(widths.len() / 2).copied().unwrap_or(CHAR_WIDTH);
    let size_of = |kind: &str, fallback: f32| {
        s.labels
            .iter()
            .find(|l| l.names == kind && l.size > 0.0)
            .map_or(fallback, |l| l.size)
    };
    let (function_size, module_size) = (size_of("function", 12.5), size_of("module", 13.5));
    let measure = |text: &str, size: f32| {
        egui::vec2(text.chars().count() as f32 * size * per_size, size * 1.2)
    };
    let taken: Vec<egui::Rect> = s
        .labels
        .iter()
        .map(|l| rect(l.rect).expand(NAME_PAD))
        .collect();
    let lit: Vec<&Node> = s.nodes.iter().filter(|n| n.on_screen && n.lit()).collect();
    let spheres: Vec<&Module> = s.spheres().filter(|m| m.radius > 0.0).collect();
    let clear = |r: egui::Rect, own_dot: Option<&str>, module: Option<&str>| {
        let padded = r.expand(NAME_PAD);
        free.contains_rect(padded)
            && !s.under_overlay(padded)
            && !taken.iter().any(|t| t.intersects(padded))
            && !lit.iter().any(|n| {
                n.unit.as_deref() != own_dot && padded.distance_to_pos(pos(n.at)) < n.radius
            })
            && module.is_none_or(|module| {
                !spheres
                    .iter()
                    .any(|m| m.name != module && padded.distance_to_pos(pos(m.at)) < m.radius)
                    && !s.nodes.iter().any(|n| {
                        n.on_screen
                            && n.module.as_deref() == Some(module)
                            && padded.distance_to_pos(pos(n.at)) < n.radius
                    })
            })
    };
    let mut offenders = Vec::new();
    for n in &lit {
        if n.label.is_some() || n.unit.is_none() {
            continue;
        }
        let text = match n.numbers.first() {
            Some(number) => format!("{number}  {}", n.name),
            None => n.name.clone(),
        };
        let size = measure(&text, function_size);
        let at = pos(n.at);
        let reach = n.radius + NAME_GAP;
        let spots = [
            egui::Rect::from_min_size(egui::pos2(at.x + reach, at.y - size.y / 2.0), size),
            egui::Rect::from_min_size(egui::pos2(at.x - reach - size.x, at.y - size.y / 2.0), size),
            egui::Rect::from_min_size(egui::pos2(at.x - size.x / 2.0, at.y - reach - size.y), size),
            egui::Rect::from_min_size(egui::pos2(at.x - size.x / 2.0, at.y + reach), size),
        ];
        if let Some(spot) = spots.iter().find(|r| clear(**r, n.unit.as_deref(), None)) {
            let offender = json!({ "function": n.name, "roles": n.roles, "would_fit_at": rect_json([spot.min.x, spot.min.y, spot.max.x, spot.max.y]) });
            offenders.push((1.0 + f32::from(u8::from(n.focus())), offender));
        }
    }
    for m in s
        .spheres()
        .filter(|m| m.on_screen && m.radius > 0.0 && m.label.is_none())
    {
        let size = measure(&m.name.to_uppercase(), module_size);
        let top = m.at[1] - m.radius - 2.0;
        let spot =
            egui::Rect::from_min_size(egui::pos2(m.at[0] - size.x / 2.0, top - size.y), size);
        if clear(spot, None, Some(&m.name)) {
            let offender = json!({ "module": m.name, "would_fit_at": rect_json([spot.min.x, spot.min.y, spot.max.x, spot.max.y]) });
            offenders.push((0.5, offender));
        }
    }
    verdict(offenders)
}

fn journey_fills(s: &Snap, before: Option<&Snap>, since: &Since) -> Value {
    if s.state.mode != "journey" {
        return skip("no journey open");
    }
    if s.expanded() {
        return skip("expanded");
    }
    if since.by_hand {
        return skip("the camera is where the user's hands put it");
    }
    let opened = before.is_none_or(|b| b.state.mode != "journey" || b.state.start != s.state.start);
    let framed = matches!(
        since.last_move(),
        Some(Kind::FrameAll | Kind::FrameFocus | Kind::Depth)
    );
    if !opened && !framed {
        return skip("no journey opened, depth change, R, F or Fit since the last snapshot");
    }
    let lit: Vec<&Node> = s.nodes.iter().filter(|n| n.in_journey()).collect();
    if lit.len() < 2 {
        return skip("fewer than two of the journey's functions drawn");
    }
    let free = s.free();
    let span = lit
        .iter()
        .flat_map(|n| {
            let dot = egui::Rect::from_center_size(pos(n.at), egui::Vec2::splat(2.0 * n.radius));
            std::iter::once(dot).chain(n.label_rect.map(rect))
        })
        .reduce(egui::Rect::union)
        .unwrap_or(egui::Rect::NOTHING);
    let share = (span.width() / free.width()).max(span.height() / free.height());
    let mut offenders = Vec::new();
    if share < JOURNEY_FILL {
        let offender = json!({
            "why": "the journey's functions and their names take too little of the free area",
            "share": r2(share), "least": JOURNEY_FILL, "span": [r1(span.width()), r1(span.height())],
            "free_area": [r1(free.width()), r1(free.height())],
        });
        offenders.push((JOURNEY_FILL - share, offender));
    }
    for n in lit {
        let out = circle_out(free, v2(n.at), n.radius);
        if out > EDGE {
            let offender =
                json!({ "function": n.name, "roles": n.roles, "out_of_the_free_area_by": r1(out) });
            offenders.push((out, offender));
        }
    }
    with(verdict(offenders), "share", json!(r2(share)))
}

fn colours(s: &Snap) -> Value {
    let mut offenders = Vec::new();
    // Hovering never looks like the selection: no white fill, no ring the selection's
    // color.
    let marked: Vec<[f32; 3]> = s
        .nodes
        .iter()
        .filter(|n| n.is("start") || n.is("selection"))
        .flat_map(|n| n.rings.iter().filter_map(|r| rgb(&r.color)))
        .collect();
    for n in s.nodes.iter().filter(|n| n.is("hovered") && !n.focus()) {
        let white = n.color.as_deref().and_then(rgb).is_some_and(whitish);
        let ring = n
            .rings
            .iter()
            .filter_map(|r| rgb(&r.color))
            .find(|c| whitish(*c) || marked.iter().any(|m| distance(*m, *c) < 0.1));
        if white || ring.is_some() {
            let offender = json!({
                "function": n.name, "why": "hovered, it looks like the selection",
                "fill": n.color, "rings": n.rings.iter().map(|r| &r.color).collect::<Vec<_>>(),
            });
            offenders.push((2.0, offender));
        }
    }
    // Each function's fill and each arrow's color is its role's.
    let cursor = s
        .nodes
        .iter()
        .find(|n| n.is("cursor"))
        .and_then(|n| n.unit.clone());
    for n in &s.nodes {
        let Some(want) = node_role_color(n) else {
            continue;
        };
        if let Some(got) = n.color.as_deref().and_then(rgb)
            && !like(got, want)
        {
            let offender = json!({ "function": n.name, "roles": n.roles, "fill": n.color, "role_color": hex3(want) });
            offenders.push((1.0, offender));
        }
    }
    for a in &s.arrows {
        let Some(got) = a.color.as_deref().and_then(rgb) else {
            continue;
        };
        let want = flow_color(&a.flow, a.level);
        let into_cursor = cursor.is_some() && a.to_unit == cursor && like(got, srgb(CURSOR));
        if !like(got, want) && !into_cursor {
            let offender = json!({ "arrow": [a.from, a.to], "flow": a.flow, "level": a.level, "color": a.color, "role_color": hex3(want) });
            offenders.push((1.0, offender));
        }
    }
    verdict(offenders)
}

/// The color a function's role gives its fill, if its roles name one.
fn node_role_color(n: &Node) -> Option<[f32; 3]> {
    if n.is("cursor") {
        return Some(srgb(CURSOR));
    }
    if n.is("start") || n.is("selection") || n.is("aside") {
        return Some([1.0, 1.0, 1.0]);
    }
    if n.is("caller") {
        return Some(srgb(CALLER));
    }
    let level = n
        .roles
        .iter()
        .find_map(|r| r.strip_prefix("journey level ")?.parse::<u32>().ok())?;
    Some(flow_color("journey", Some(level)))
}

/// The color an arrow's role gives it: violet into, blue the first level, amber
/// deeper, grey the rest.
fn flow_color(flow: &str, level: Option<u32>) -> [f32; 3] {
    match (flow, level) {
        ("into", _) => srgb(CALLER),
        ("journey", Some(1)) => srgb(CALLEE),
        ("journey", _) => srgb(DEEPER),
        _ => srgb(FURTHER),
    }
}

fn srgb(color: bevy::color::Color) -> [f32; 3] {
    let c = color.to_srgba();
    [c.red, c.green, c.blue]
}

/// A color from `#rrggbb` or `#rrggbbaa`, alpha left out.
fn rgb(hex: &str) -> Option<[f32; 3]> {
    let hex = hex.trim_start_matches('#');
    let channel = |at: usize| {
        u8::from_str_radix(hex.get(at..at + 2)?, 16)
            .ok()
            .map(|c| f32::from(c) / 255.0)
    };
    Some([channel(0)?, channel(2)?, channel(4)?])
}

fn hex3(c: [f32; 3]) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "0 to 255"
    )]
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c[0]), byte(c[1]), byte(c[2]))
}

fn whitish(c: [f32; 3]) -> bool {
    c.iter().all(|&v| v >= WHITE)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    Vec3::from_array(a).distance(Vec3::from_array(b))
}

/// Hue in degrees and saturation, 0 to 1.
fn hue(c: [f32; 3]) -> (f32, f32) {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let chroma = max - min;
    if chroma < f32::EPSILON {
        return (0.0, 0.0);
    }
    let h = if (max - c[0]).abs() < f32::EPSILON {
        ((c[1] - c[2]) / chroma).rem_euclid(6.0)
    } else if (max - c[1]).abs() < f32::EPSILON {
        (c[2] - c[0]) / chroma + 2.0
    } else {
        (c[0] - c[1]) / chroma + 4.0
    };
    (h * 60.0, chroma / max)
}

/// Whether a color drawn reads as a role's: white as white, grey as grey, else the
/// same hue, however light or dark.
fn like(got: [f32; 3], want: [f32; 3]) -> bool {
    if whitish(want) {
        return whitish(got);
    }
    let ((gh, gs), (wh, ws)) = (hue(got), hue(want));
    if ws < GREY {
        return gs < GREY;
    }
    let apart = (gh - wh).abs();
    gs >= GREY / 2.0 && apart.min(360.0 - apart) <= HUE_SLACK
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

    /// A snapshot of two modules side by side, `a` holding `f` and the faint `g`, `b`
    /// holding `h`, with `f`, `h` and both modules named.
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
                  "radius": 80.0, "world": [10.0, 0.0, 0.0], "world_radius": 4.0, "on_screen": true,
                  "label": "B", "label_rect": [992.0, 405.0, 1008.0, 417.0] },
            ],
            "nodes": [
                { "name": "f", "unit": "x:a::f", "module": "a", "roles": ["centre"], "at": [380.0, 500.0],
                  "radius": 5.0, "world": [-11.0, 0.0, 0.0], "world_radius": 0.25, "on_screen": true,
                  "label": "f", "label_rect": [390.0, 494.0, 410.0, 506.0] },
                { "name": "g", "unit": "x:a::g", "module": "a", "roles": ["speck"], "at": [440.0, 500.0],
                  "radius": 2.0, "world": [-8.0, 0.0, 0.0], "world_radius": 0.1, "on_screen": true },
                { "name": "h", "unit": "x:b::h", "module": "b", "roles": ["callee"], "at": [1000.0, 500.0],
                  "radius": 4.0, "world": [10.0, 0.0, 0.0], "world_radius": 0.2, "on_screen": true,
                  "label": "h", "label_rect": [1008.0, 494.0, 1016.0, 506.0] },
            ],
            "labels": [
                { "text": "A", "rect": [380.0, 390.0, 420.0, 402.0], "names": "module", "of": "a",
                  "overlaps": [], "cut_off": false, "under": [], "size": 12.0 },
                { "text": "f", "rect": [390.0, 494.0, 410.0, 506.0], "names": "function", "of": "f",
                  "unit": "x:a::f", "overlaps": [], "cut_off": false, "under": [], "size": 12.0 },
                { "text": "h", "rect": [1008.0, 494.0, 1016.0, 506.0], "names": "function", "of": "h",
                  "unit": "x:b::h", "overlaps": [], "cut_off": false, "under": [], "size": 12.0 },
                { "text": "B", "rect": [992.0, 405.0, 1008.0, 417.0], "names": "module", "of": "b",
                  "overlaps": [], "cut_off": false, "under": [], "size": 12.0 },
            ],
            "bars": [],
            "overlays": [],
            "text": [{ "text": "codeviz", "size": 13.0 }],
        })
    }

    fn check(now: &Value, name: &str) -> Value {
        run(now, None, &Since::default())[name].clone()
    }

    fn after(before: &Value, now: &Value, kinds: &[Kind], name: &str) -> Value {
        let since = Since {
            kinds: kinds.to_vec(),
            by_hand: false,
        };
        run(now, Some(before), &since)[name].clone()
    }

    const IN: Kind = Kind::Zoom { out: false };
    const OUT: Kind = Kind::Zoom { out: true };

    fn passes(result: &Value) -> bool {
        result["pass"] == json!(true)
    }

    #[test]
    fn a_tidy_scene_passes_every_check_that_applies() {
        let results = run(&scene(), None, &Since::default());
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
        assert!(
            passes(&check(&s, "dots-apart")),
            "a faint dot with no name may sit under another"
        );
        s["nodes"][1]["roles"] = json!(["caller"]);
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
            after(&s, &s, &[Kind::FrameAll, IN], "framed")["pass"],
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
        let result = after(&before, &now, &[IN], "zoom-to-pointer");
        assert!(passes(&result), "{result}");
        assert_eq!(result["zoomed_by"], 2.0);
        // About the window's middle instead, with the pointer off it.
        now["state"]["pointer"] = json!([300.0, 500.0]);
        let mut was = before.clone();
        was["state"]["pointer"] = json!([300.0, 500.0]);
        now["modules"][0]["at"] = json!([0.0, 488.0]);
        let result = after(&was, &now, &[IN], "zoom-to-pointer");
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
        let result = after(&s, &s, &[OUT], "zoom-bounds");
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
        // A depth change may not turn the camera, nor move anything in the world.
        let result = after(&before, &now, &[Kind::Depth], "nothing-moved");
        assert_eq!(
            result["offenders"][0]["why"],
            "a depth change may only draw the camera back"
        );
        now["modules"][1]["world"] = json!([11.0, 0.0, 0.0]);
        let result = after(&before, &now, &[Kind::Depth], "nothing-moved");
        assert!(
            result["offenders"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["module"] == "b")
        );
    }

    /// The scene after a depth change that drew the camera back to twice as far,
    /// bringing in `k` at `at`.
    fn deeper(at: [f32; 2]) -> Value {
        let mut now = scene();
        now["state"]["camera"]["distance"] = json!(200.0);
        now["camera"] = json!({ "at": [0.0, 0.0, 200.0], "target_at": [800.0, 500.0] });
        let k = json!({ "name": "k", "unit": "x:b::k", "module": "b", "roles": ["journey level 2"],
            "at": at, "radius": 4.0, "world": [10.0, 1.0, 0.0], "world_radius": 0.2, "on_screen": true });
        now["nodes"].as_array_mut().unwrap().push(k);
        now
    }

    #[test]
    fn a_depth_change_draws_the_camera_back_only_when_what_came_in_needs_it() {
        let before = scene();
        // From where the camera was, k would have shown at 800 + 2 × 700.
        let needed = after(
            &before,
            &deeper([1500.0, 500.0]),
            &[Kind::Depth],
            "nothing-moved",
        );
        assert!(passes(&needed), "{needed}");
        // At 800 + 2 × 100 it would have shown anyway.
        let result = after(
            &before,
            &deeper([900.0, 500.0]),
            &[Kind::Depth],
            "nothing-moved",
        );
        assert!(!passes(&result));
        assert!(
            result["offenders"][0]["why"]
                .as_str()
                .unwrap()
                .contains("would have shown")
        );
    }

    #[test]
    fn a_name_that_goes_while_its_dot_stays_has_moved() {
        let before = scene();
        let mut now = scene();
        now["nodes"][2]["label"] = Value::Null;
        now["nodes"][2]["label_rect"] = Value::Null;
        now["labels"].as_array_mut().unwrap().remove(2);
        let result = after(&before, &now, &[Kind::Still], "nothing-moved");
        assert_eq!(result["count"], 1, "{result}");
        assert_eq!(result["offenders"][0]["gone"], true);
        assert_eq!(result["offenders"][0]["label"], "h");
        // Its dot gone with it, as a depth change hides a call: nothing moved.
        now["nodes"].as_array_mut().unwrap().pop();
        let result = after(&before, &now, &[Kind::Click], "nothing-moved");
        assert!(passes(&result), "{result}");
    }

    #[test]
    fn names_are_told_apart_by_function_not_by_what_they_say() {
        // Two functions both named `root`, each named where it is; the second's name
        // comes first in the list now.
        let two = |flip: bool| {
            let mut s = scene();
            let mut labels = vec![
                json!({ "text": "root", "rect": [100.0, 100.0, 130.0, 112.0], "names": "function", "of": "root", "unit": "x:a::root" }),
                json!({ "text": "root", "rect": [600.0, 300.0, 630.0, 312.0], "names": "function", "of": "root", "unit": "x:b::root" }),
            ];
            if flip {
                labels.reverse();
            }
            s["labels"] = json!(labels);
            s
        };
        let result = after(&two(false), &two(true), &[Kind::Still], "nothing-moved");
        assert!(passes(&result), "{result}");
    }

    #[test]
    fn a_name_nearer_another_lit_dot_than_its_own_is_named() {
        let mut s = scene();
        s["nodes"][1]["roles"] = json!(["caller"]);
        s["nodes"][1]["at"] = json!([420.0, 500.0]);
        s["nodes"][0]["label_rect"] = json!([416.0, 494.0, 440.0, 506.0]);
        let result = check(&s, "labels-near");
        assert_eq!(result["offenders"][0]["nearer_dot"], "g", "{result}");
    }

    /// The scene in a journey from `f`, the cursor on `h`, numbered 1, with its bar.
    fn journey() -> Value {
        let mut s = scene();
        s["state"]["mode"] = json!("journey");
        s["state"]["start"] = json!("f");
        s["nodes"][0]["roles"] = json!(["start"]);
        s["nodes"][2]["roles"] = json!(["journey level 1", "cursor"]);
        s["nodes"][2]["numbers"] = json!(["1"]);
        s["nodes"][2]["label"] = json!(" 1  h");
        s["bars"] = json!([
            { "number": "", "name": "f", "unit": "x:a::f", "cursor": false },
            { "number": "1", "name": "h", "unit": "x:b::h", "cursor": true },
        ]);
        s
    }

    #[test]
    fn the_panel_shows_one_cursor_bar_named_as_the_graph_names_it() {
        let mut s = journey();
        assert!(passes(&check(&s, "panel-agrees")));
        s["bars"][1]["cursor"] = json!(false);
        let result = check(&s, "panel-agrees");
        assert!(
            result["offenders"][0]["why"]
                .as_str()
                .unwrap()
                .contains("not one cursor bar")
        );
        s["bars"][0]["cursor"] = json!(true);
        s["bars"][1]["cursor"] = json!(true);
        assert!(!passes(&check(&s, "panel-agrees")), "two cursor bars");
        let mut s = journey();
        s["bars"][1]["name"] = json!("Thing.h");
        let result = check(&s, "panel-agrees");
        assert_eq!(result["offenders"][0]["bar_says"], "Thing.h", "{result}");
        assert_eq!(result["offenders"][0]["graph_says"], " 1  h");
    }

    #[test]
    fn names_under_the_key_help_are_left_alone() {
        let mut s = scene();
        s["labels"][1]["under"] = json!(["key help (H)"]);
        assert!(!passes(&check(&s, "named-on-screen")));
        s["state"]["help"] = json!(true);
        assert_eq!(check(&s, "named-on-screen")["pass"], Value::Null);
    }

    #[test]
    fn what_is_in_focus_shows_unless_the_hands_moved_the_camera() {
        let mut s = journey();
        s["overlays"] = json!([{ "name": "journey panel", "rect": [0.0, 800.0, 1600.0, 1000.0] }]);
        s["nodes"][2]["at"] = json!([1000.0, 820.0]);
        let result = check(&s, "focus-on-screen");
        assert_eq!(result["offenders"][0]["function"], "h", "{result}");
        let by_hand = Since {
            kinds: vec![Kind::Pan],
            by_hand: true,
        };
        assert_eq!(
            run(&s, None, &by_hand)["focus-on-screen"]["pass"],
            Value::Null
        );
    }

    #[test]
    fn the_camera_is_never_inside_a_sphere() {
        let mut s = scene();
        s["camera"] = json!({ "at": [0.0, 0.0, 50.0] });
        assert!(passes(&check(&s, "camera-outside")));
        s["camera"] = json!({ "at": [-9.0, 0.0, 1.0] });
        let result = check(&s, "camera-outside");
        assert_eq!(result["offenders"][0]["module"], "a", "{result}");
    }

    #[test]
    fn zoomed_out_the_map_stays_in_the_free_area() {
        let mut s = scene();
        assert!(passes(&after(&s, &s, &[OUT], "map-on-screen")));
        assert_eq!(after(&s, &s, &[IN], "map-on-screen")["pass"], Value::Null);
        // Pushed half off the right edge, though it would fit.
        s["modules"][0]["at"] = json!([1300.0, 500.0]);
        s["modules"][1]["at"] = json!([1580.0, 500.0]);
        let result = after(&s, &s, &[OUT], "map-on-screen");
        assert!(!passes(&result), "{result}");
        // Bigger than the free area, it cannot all be there.
        s["modules"][0]["radius"] = json!(900.0);
        assert_eq!(after(&s, &s, &[OUT], "map-on-screen")["pass"], Value::Null);
    }

    #[test]
    fn a_sphere_cut_off_in_open_window_is_named() {
        let mut s = scene();
        s["graph_area"] = json!([0.0, 24.0, 1600.0, 560.0]);
        s["window"] = json!([1600.0, 1000.0]);
        s["overlays"] = json!([{ "name": "journey panel", "rect": [12.0, 890.0, 1588.0, 988.0] }]);
        let result = check(&s, "spheres-unclipped");
        assert_eq!(result["count"], 2, "both cross y = 560: {result}");
        // The panel starts where the graph ends: what it cuts is under the panel.
        s["overlays"][0]["rect"] = json!([12.0, 562.0, 1588.0, 988.0]);
        assert!(passes(&check(&s, "spheres-unclipped")));
    }

    #[test]
    fn arrows_keep_to_their_width_and_head_and_one_head_each() {
        let mut s = scene();
        let call = |width: f32, head: f32| {
            json!({ "from": "f", "to": "h", "from_unit": "x:a::f", "to_unit": "x:b::h", "flow": "journey",
                    "level": 1, "color": "#4db8ffe6", "width": width, "head": head, "head_width": head, "on_screen": true })
        };
        s["arrows"] = json!([call(1.0, 8.0)]);
        assert!(passes(&check(&s, "arrow-sizes")));
        s["arrows"] = json!([call(4.0, 2.0)]);
        assert_eq!(check(&s, "arrow-sizes")["count"], 2);
        s["arrows"] = json!([call(1.0, 8.0)]);
        s["module_arrows"] = json!([{ "from": "a", "to": "b", "calls": 3, "flow": "journey", "level": 1,
            "drawn": true, "width": 2.0, "head": 8.0, "head_width": 8.0, "on_screen": true }]);
        let result = check(&s, "arrow-sizes");
        assert!(
            result["offenders"][0]["why"]
                .as_str()
                .unwrap()
                .contains("another"),
            "{result}"
        );
    }

    #[test]
    fn a_module_name_sits_centred_above_its_sphere_off_the_rest() {
        let mut s = scene();
        assert!(passes(&check(&s, "module-names")));
        // Beside its top, on its right.
        s["modules"][0]["label_rect"] = json!([430.0, 385.0, 470.0, 397.0]);
        let result = check(&s, "module-names");
        assert!(
            result["offenders"][0]["why"]
                .as_str()
                .unwrap()
                .contains("centred")
        );
        // Over its own dot.
        s["modules"][0]["label_rect"] = json!([370.0, 494.0, 390.0, 506.0]);
        let result = check(&s, "module-names");
        assert!(
            result["offenders"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["over_its_dot"] == "f")
        );
        // Over the other sphere.
        s["modules"][0]["label_rect"] = json!([930.0, 494.0, 970.0, 506.0]);
        let result = check(&s, "module-names");
        assert!(
            result["offenders"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["over_sphere"] == "b")
        );
    }

    #[test]
    fn a_lit_function_or_module_with_room_for_its_name_is_named() {
        let mut s = scene();
        s["nodes"][2]["label"] = Value::Null;
        s["nodes"][2]["label_rect"] = Value::Null;
        s["labels"].as_array_mut().unwrap().remove(2);
        let result = check(&s, "named-when-room");
        assert_eq!(result["offenders"][0]["function"], "h", "{result}");
        // Hemmed in by names on every side, it has none.
        let blocks = [
            [1004.0, 480.0, 1100.0, 520.0],
            [900.0, 480.0, 996.0, 520.0],
            [950.0, 440.0, 1050.0, 496.0],
            [950.0, 504.0, 1050.0, 560.0],
        ];
        for r in blocks {
            s["labels"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "text": "x", "rect": r, "names": "other", "size": 12.0 }));
        }
        assert!(passes(&check(&s, "named-when-room")));
        // A module with room above its sphere.
        let mut s = scene();
        s["modules"][1]["label"] = Value::Null;
        s["modules"][1]["label_rect"] = Value::Null;
        s["labels"].as_array_mut().unwrap().remove(3);
        let result = check(&s, "named-when-room");
        assert_eq!(result["offenders"][0]["module"], "b", "{result}");
    }

    #[test]
    fn a_framed_journey_spans_the_free_area() {
        let browse = scene();
        let mut s = journey();
        // f at 380 to h's name ending at 1016: 641 of 1600 across, 0.4.
        let result = after(&browse, &s, &[Kind::Moves], "journey-fills");
        assert!(!passes(&result), "{result}");
        assert_eq!(result["share"], 0.4);
        s["nodes"][0]["at"] = json!([100.0, 500.0]);
        s["nodes"][2]["at"] = json!([1300.0, 500.0]);
        assert!(passes(&after(&browse, &s, &[Kind::Moves], "journey-fills")));
        // Not framed since: a click.
        assert_eq!(
            after(&s, &s, &[Kind::Click], "journey-fills")["pass"],
            Value::Null
        );
        assert!(passes(&after(&s, &s, &[Kind::Depth], "journey-fills")));
    }

    #[test]
    fn hovering_never_looks_like_the_selection_and_arrows_wear_their_role() {
        let mut s = journey();
        s["nodes"][0]["color"] = json!("#ffffff");
        s["nodes"][0]["rings"] = json!([{ "radius": 6.0, "color": "#ffffffff" }]);
        s["nodes"][2]["color"] = json!("#ff5cb8");
        s["nodes"][1]["roles"] = json!(["hovered"]);
        s["nodes"][1]["rings"] = json!([{ "radius": 4.0, "color": "#ffffff8c" }]);
        let result = check(&s, "colours");
        assert_eq!(result["count"], 1, "{result}");
        assert_eq!(result["offenders"][0]["function"], "g");
        s["nodes"][1]["rings"] = json!([{ "radius": 4.0, "color": "#8c94a5ff" }]);
        assert!(passes(&check(&s, "colours")));
        // A call into the start drawn blue; a call into the cursor drawn pink.
        s["arrows"] = json!([
            { "from": "g", "to": "f", "to_unit": "x:a::f", "flow": "into", "color": "#4db8ff" },
            { "from": "f", "to": "h", "to_unit": "x:b::h", "flow": "journey", "level": 1, "color": "#ff5cb8" },
        ]);
        let result = check(&s, "colours");
        assert_eq!(result["count"], 1, "{result}");
        assert_eq!(result["offenders"][0]["flow"], "into");
    }

    #[test]
    fn a_wheel_turn_with_no_sphere_on_screen_is_judged_by_one_off_it() {
        let mut before = scene();
        before["modules"][0]["on_screen"] = json!(false);
        before["modules"][1]["on_screen"] = json!(false);
        let mut now = before.clone();
        now["modules"][0]["radius"] = json!(200.0);
        now["modules"][1]["radius"] = json!(160.0);
        now["modules"][1]["at"] = json!([1600.0, 500.0]);
        let result = after(&before, &now, &[IN], "zoom-to-pointer");
        assert!(passes(&result), "{result}");
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

        let ok = run(&scene(), None, &Since::default());
        let mut bad = ok.clone();
        bad["dots-apart"] = json!({ "pass": false });
        let summary = summarise(&[("01-a".to_owned(), ok), ("02-b".to_owned(), bad)]);
        assert_eq!(summary["dots-apart"]["failed_in"], json!(["02-b"]));
        assert_eq!(summary["dots-apart"]["passed"], 1);
        assert_eq!(summary["nothing-moved"]["not_applied"], 2);
        assert_eq!(summary["dots-apart"]["pass"], false);
    }
}
