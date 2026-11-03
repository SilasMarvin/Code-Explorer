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

use bevy::{
    camera::PerspectiveProjection,
    math::{EulerRot, Quat, Vec2, Vec3},
};
use bevy_egui::egui;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    colors::{CALLEE, CALLER, CURSOR, DEEPER, FURTHER, GROUND},
    nucleus::VIEW,
    script::Kind,
    sizes,
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
/// rim, edge to edge, in points (CORE: "≤ ~40 px"); past it a name names nothing a
/// reader can see. The placer puts names a dot's reach from it, sliding them no
/// farther than a line along its side, well inside this.
pub(crate) const LABEL_REACH: f32 = 40.0;
/// Smallest and largest font size of a function's name, in points.
const NAME_SIZES: (f32, f32) = (11.0, 18.0);
/// Most distinct font sizes on screen at once: module name, function name, panel,
/// and small print.
const MOST_TEXT_SIZES: usize = 4;
/// Most share of the free area's smaller side the expanded module's diameter fills;
/// and how far, as a share of it, the share it fills may be from the one its
/// functions need ([`sizes::expanded_share`]).
const EXPANDED_MOST: f32 = 0.9;
const EXPANDED_SLACK: f32 = 0.12;
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
/// Zoomed in, the most share of the free area's smaller side the diameter of the
/// sphere under the pointer spans; and the share past which it lies inside the free
/// area, as it does at the near bound.
const ZOOM_IN_MOST: f32 = 0.92;
const ZOOM_IN_KEPT: f32 = 0.88;
/// Least a function or sphere moves in the world, or a dot or name on screen in
/// points, or the camera turns in radians, to count as moved.
const MOVED_WORLD: f32 = 0.01;
const MOVED_SCREEN: f32 = 0.5;
const MOVED_ANGLE: f32 = 0.002;
/// How much nearer another lit dot may be to a function's name than its own, edge
/// to edge, in points, before the name reads as the other's.
const NEARER: f32 = 2.0;
/// Share of the map, by area on screen, that must lie inside the free area after
/// zooming out or R, when the whole map would fit there.
const MAP_SHOWN: f32 = 0.9;
/// How far beyond an edge of the graph's part of the window a panel may start, or
/// the window end, for the edge to hide what it cuts, in points.
const HIDDEN_EDGE: f32 = 4.0;
/// Widest an arrow's line or shaft shows, in points, a line between modules however
/// many calls it carries too.
const ARROW_WIDTH: f32 = 2.5;
/// Least and most an arrow's head shows, in points: its length along the line, or
/// half its width when that is more, as for a head seen end on.
const HEAD_SIZE: (f32, f32) = (5.0, 12.0);
/// How far a module's name may sit off its sphere's middle, across, and how far
/// above its top edge (or below it, inside, where it is smaller: at most this font
/// size), in points.
const NAME_CENTRED: f32 = 3.0;
const NAME_ABOVE: f32 = 8.0;
const INSIDE_NAME: f32 = 11.5;
/// How far from its dot a name's pill would be put to see whether it fits, the pill
/// round its text, and the room kept clear round a pill, in points: as the label
/// placer has them.
const NAME_GAP: f32 = 4.0;
const NAME_PILL: egui::Vec2 = egui::vec2(5.0, 2.0);
const NAME_CLEAR: f32 = 1.0;
/// How far a dot's radius in the snapshot may be from the placer's, rounded.
const ROUNDING: f32 = 0.25;
/// A character's width as a share of its font size, when no name drawn says: a
/// function's, and a module's in capitals, at their widest.
const CHAR_WIDTH: f32 = 0.55;
const CAPITAL_WIDTH: f32 = 0.75;
/// Least share of the free area a framed journey's lit dots and their names span,
/// along whichever of its width and height they come nearer filling.
const JOURNEY_FILL: f32 = 0.7;
/// How far a color's hue may be from its role's, in degrees, and how grey a color
/// is that carries no role's hue (saturation, 0 to 1).
const HUE_SLACK: f32 = 25.0;
const GREY: f32 = 0.3;
/// The most alpha a faint call or bar is drawn with.
const FAINT: f32 = 0.5;
/// A call's line, in points, and how much of its color it keeps, as drawn when not
/// followed: wider or stronger is followed.
const CALL_WIDTH: f32 = 1.5;
const CALL_STRENGTH: f32 = 0.85;
/// How bright a dot only guessed is drawn at most, against its role's color.
const GUESSED_DOT: f32 = 0.8;
/// A color this near white, in every channel (0 to 1), reads as the selection's.
const WHITE: f32 = 0.85;
/// Room the framing leaves round a lit dot or a sphere for it to show whole, in
/// points: a depth change draws the camera back for one nearer the free area's edge
/// than this.
const SHOWN_WHOLE: f32 = 10.0;
/// Room the framing leaves above a journey's sphere for its name, in points.
const NAME_ROOM: f32 = 24.0;
/// How far into a dot, or a name's pill, a call's line may run before it counts as
/// passing through, in points: rounding, not leeway.
const ARROW_SLACK: f32 = 1.0;
/// Offenders listed per check; `count` says how many in all.
const LISTED: usize = 12;

/// Every check, in the order a snapshot lists them.
pub(crate) const NAMES: [&str; 28] = [
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
    "arrows-clear",
    "names-distinct",
    "exits-named",
    "calls-drawn",
];

/// What happened since the snapshot before: what each step may move, and whether the
/// camera stands where the user's hands put it (a wheel turn, an orbit or a pan since
/// the view last framed itself).
#[derive(Debug, Default, Clone)]
pub(crate) struct Since {
    pub(crate) kinds: Vec<Kind>,
    pub(crate) by_hand: bool,
    /// A frame snapped as a transition plays (`frames N`), not the view at rest.
    pub(crate) playing: bool,
    /// Nothing that moves the view has happened since the session opened: what is
    /// judged is the view as it opened.
    pub(crate) opening: bool,
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
    /// Where the journey's calls a deeper level shows would be, which names keep off.
    held: Vec<Held>,
    modules: Vec<Module>,
    arrows: Vec<Arrow>,
    module_arrows: Vec<Arrow>,
    exits: Vec<Exit>,
    content: Content,
    labels: Vec<Label>,
    bars: Vec<Bar>,
    /// While the search box shows.
    search_rows: Option<Vec<SearchRow>>,
    overlays: Vec<Overlay>,
    text: Vec<Text>,
}

/// What the view means to show: the selection or the start, its callers, its calls.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Content {
    selection: Option<String>,
    start: Option<String>,
    callers: Vec<String>,
    calls: Vec<ContentCall>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ContentCall {
    name: String,
    /// The number of the call it is made from: empty for the start's own.
    parent: Option<String>,
}

/// Where a call leaves the free area, labelled with the function at its far end.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Exit {
    from_unit: Option<String>,
    to_unit: Option<String>,
    far: String,
    text: String,
    rect: [f32; 4],
}

/// A row the search box lists: what it says, and the function it finds.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SearchRow {
    text: String,
    unit: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct State {
    mode: String,
    nucleus: String,
    selection: Option<String>,
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
    drawn: Option<bool>,
    width: Option<f32>,
    head: Option<f32>,
    head_width: Option<f32>,
    on_screen: bool,
    /// A call only guessed, or handed off: drawn faint.
    guessed: bool,
    /// The points its line runs through, from its start to its tip.
    path: Vec<[f32; 2]>,
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
    /// The last wheel turn out panned to hold the map in the free area.
    held: bool,
    /// Zoomed out as far as the wheel goes.
    farthest: bool,
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
    /// How big its name was laid out, room found for it or not.
    name_size: Option<[f32; 2]>,
    /// The one place its name was tried, as the view is: the side it holds, or its
    /// home.
    name_spot: Option<[f32; 4]>,
    /// How far from its middle names keep, in points.
    kept_off: Option<f32>,
    color: Option<String>,
    rings: Vec<Ring>,
}

/// A journey's call not drawn at this depth, where it would show.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Held {
    at: [f32; 2],
    radius: f32,
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
    /// How big its name was laid out, room found for it or not.
    name_size: Option<[f32; 2]>,
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
    color: Option<String>,
    guessed: bool,
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

    /// One of the expanded module's functions that nothing else lights: a lit name
    /// may lie over it.
    fn plain(&self) -> bool {
        self.is("opened") && !self.focus()
    }

    /// Whether `other`'s dot keeps this one's name off: any lit one, but a plain one
    /// only from a plain name.
    fn kept_off_by(&self, other: &Node) -> bool {
        other.lit() && (self.plain() || !other.plain())
    }

    /// A faint dot with no name: nothing a reader tells apart.
    fn faint(&self) -> bool {
        !self.lit() && self.label.is_none()
    }

    /// Where its name comes in the order names are placed, first 0: the start or the
    /// selection, the cursor, the function set aside, (the modules,) the start's own
    /// calls, its callers, the calls further down, whatever else is lit, (the modules
    /// brought in from off the journey,) then the rest.
    fn rank(&self) -> u8 {
        let level = self
            .roles
            .iter()
            .find_map(|r| r.strip_prefix("journey level ")?.parse::<u32>().ok());
        if self.is("start") || self.is("selection") {
            0
        } else if self.is("cursor") {
            1
        } else if self.is("aside") {
            2
        } else if self.is("caller") {
            5
        } else {
            match (level, self.numbers.is_empty()) {
                (Some(1), false) => 4,
                (Some(_), false) => 6,
                (Some(_), true) => 7,
                (None, _) => 9,
            }
        }
    }

    /// Lit in a journey: the start, a call or a caller of it (not one folded into
    /// its module's count).
    fn in_journey(&self) -> bool {
        self.roles.iter().any(|r| {
            matches!(r.as_str(), "start" | "cursor" | "caller") || r.starts_with("journey level")
        })
    }
}

impl Module {
    /// Where its name comes in the order names are placed (see [`Node::rank`]).
    fn rank(&self, journey: bool) -> u8 {
        if journey && self.body == "faint" {
            8
        } else {
            3
        }
    }
}

impl Snap {
    fn expanded(&self) -> bool {
        self.state.nucleus == "expanded"
    }

    fn spheres(&self) -> impl Iterator<Item = &Module> {
        self.modules.iter().filter(|m| m.shown)
    }

    /// The function the journey's cursor is on, by unit.
    fn cursor(&self) -> Option<&str> {
        self.nodes
            .iter()
            .find(|n| n.is("cursor"))
            .and_then(|n| n.unit.as_deref())
    }

    /// Expanded in a journey, whether the cursor moved since `before`: the view keeps
    /// it in view, and takes the expansion to its module.
    fn cursor_moved_expanded(&self, before: Option<&Snap>) -> bool {
        self.expanded()
            && self.state.mode == "journey"
            && before.is_some_and(|b| b.cursor() != self.cursor())
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
        expanded_fill(&now, before, since),
        framed(&now, since),
        zoom_to_pointer(&now, before, since),
        zoom_bounds(&now, since),
        nothing_moved(&now, before, since),
        focus_on_screen(&now, before, since),
        camera_outside(&now),
        map_on_screen(&now, since),
        spheres_unclipped(&now, before, since),
        arrow_sizes(&now),
        module_names(&now),
        named_when_room(&now),
        journey_fills(&now, before, since),
        colours(&now),
        arrows_clear(&now),
        names_distinct(&now),
        exits_named(&now),
        calls_drawn(&now),
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
        // The lit dot nearest the name, but its own (and for a lit name, a plain one).
        let nearest = lit
            .iter()
            .filter(|other| other.unit != n.unit && n.kept_off_by(other))
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
        // A notice is said over nothing named.
        if label.under.iter().any(|o| o == "notice") {
            let offender = json!({ "label": label.text, "under": "notice" });
            offenders.push((rect(label.rect).area(), offender));
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
            if std::ptr::eq(*other, n) || !n.kept_off_by(other) {
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
        // The search box opens over the graph for a moment, as the key help does.
        let under: Vec<&String> = label
            .under
            .iter()
            .filter(|o| o.as_str() != "search box")
            .collect();
        if !under.is_empty() {
            let offender = json!({ "label": label.text, "of": label.of, "rect": rect_json(label.rect), "under": under });
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

fn expanded_fill(s: &Snap, before: Option<&Snap>, since: &Since) -> Value {
    // Expanded in a journey, the cursor takes the expansion to its module; browsing,
    // a pick takes it to the selection's.
    let followed = before.is_some_and(|b| b.expanded() && b.state.expanded != s.state.expanded);
    if !since.has(Kind::Expand) && !followed {
        return skip(
            "no expand (x, double click), nor another module expanded, since the last snapshot",
        );
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
    // Its functions where they show, and how long their names are: the share they
    // need to sit comfortably.
    let dots: Vec<(Vec2, usize)> = s
        .nodes
        .iter()
        .filter(|n| n.module.as_ref() == Some(&m.name))
        .map(|n| (v2(n.at), n.name.chars().count()))
        .collect();
    let wanted = sizes::expanded_share(fill, sizes::comfort(&dots));
    // Small in a journey, it may stay smaller, to keep the modules beside it on screen.
    let small = s.state.mode == "journey" && m.functions <= sizes::SMALL_MODULE;
    let mut offenders = Vec::new();
    let off = fill / wanted.max(f32::EPSILON) - 1.0;
    if fill > EXPANDED_MOST || off > EXPANDED_SLACK || (!small && off < -EXPANDED_SLACK) {
        let offender = json!({
            "module": m.name, "fill": r2(fill), "wanted": r2(wanted), "most": EXPANDED_MOST,
            "functions": m.functions, "diameter": r1(2.0 * m.radius),
            "free_area": [r1(free.width()), r1(free.height())],
        });
        offenders.push((off.abs(), offender));
    }
    let out = circle_out(free, v2(m.at), m.radius);
    if out > EDGE {
        let offender = json!({ "module": m.name, "sticks_out_by": r1(out), "at": pt(m.at), "radius": r1(m.radius) });
        offenders.push((out, offender));
    }
    if small {
        for near in neighbours(s, &m.name) {
            let out = circle_out(free, v2(near.at), near.radius);
            if out > EDGE {
                let offender = json!({
                    "why": "a small module expanded in a journey leaves a module it calls or is called from off the free area",
                    "module": near.name, "out_by": r1(out),
                });
                offenders.push((out, offender));
            }
        }
    }
    with(verdict(offenders), "fill", json!(r2(fill)))
}

/// The spheres shown that hold a function calling, or called from, one of `module`'s
/// along a journey's call or a call into its start, drawn.
fn neighbours<'a>(s: &'a Snap, module: &str) -> Vec<&'a Module> {
    let module_of = |unit: &Option<String>| {
        let unit = unit.as_deref()?;
        s.node(unit).and_then(|n| n.module.as_deref())
    };
    let mut near: Vec<&str> = s
        .arrows
        .iter()
        .filter(|a| matches!(a.flow.as_str(), "into" | "journey"))
        .filter_map(|a| match (module_of(&a.from_unit), module_of(&a.to_unit)) {
            (Some(from), Some(to)) if from == module && to != module => Some(to),
            (Some(from), Some(to)) if to == module && from != module => Some(from),
            _ => None,
        })
        .collect();
    near.sort_unstable();
    near.dedup();
    s.spheres()
        .filter(|m| near.contains(&m.name.as_str()))
        .collect()
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
    if s.state.camera.is_some_and(|c| c.held) {
        return skip(
            "the wheel turn panned to hold the map, or the sphere it zoomed to, in the free area",
        );
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
    // Zoomed in, the sphere under the pointer, nearest the camera (biggest on screen).
    let under = s.state.pointer.and_then(|p| {
        spheres
            .iter()
            .filter(|m| v2(m.at).distance(v2(p)) <= m.radius)
            .max_by(|a, b| a.radius.total_cmp(&b.radius))
    });
    if since.has(Kind::Zoom { out: false })
        && let Some(m) = under
    {
        let side = free.width().min(free.height());
        let fill = 2.0 * m.radius / side;
        if fill > ZOOM_IN_MOST {
            let offender = json!({ "why": "zoomed in past the sphere under the pointer filling the free area", "module": m.name, "fill": r2(fill), "most": ZOOM_IN_MOST });
            offenders.push((fill, offender));
        } else if fill >= ZOOM_IN_KEPT {
            let out = circle_out(free, v2(m.at), m.radius);
            if out > EDGE {
                let offender = json!({ "why": "zoomed in to the near bound, the sphere under the pointer is not all in the free area", "module": m.name, "out_by": r1(out) });
                offenders.push((out, offender));
            }
        }
    }
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
    if s.cursor_moved_expanded(Some(before)) {
        return skip(
            "the cursor moved expanded in a journey: the camera may pan to keep it in view, or fly to its module",
        );
    }
    // A depth change may draw the camera back to show what it brought in, when that
    // would be off the free area, and no more.
    let depth = since.has(Kind::Depth);
    let (summary, mut offenders) = compare(before, s, depth);
    if depth {
        offenders.extend(depth_camera(before, s));
    }
    // A click that selects another function lights others, and names them instead;
    // a depth change that drew the camera back shows everything smaller, and names
    // what fits now; any other step here only reads, and takes no name away.
    let drew_back = depth && summary["camera"]["moved"] == json!(true);
    if before.state.selection == s.state.selection && !drew_back {
        offenders.extend(gone_labels(before, s));
    }
    verdict(offenders)
}

/// How the camera moved on a depth change, when it should not have: anything but
/// drawing back (and framing the journey afresh as it does), or drawing back when
/// all that came in would have shown anyway.
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
    // Drawing back, it may frame the journey afresh, and so look somewhere else.
    if turned > MOVED_ANGLE || back < -MOVED_WORLD || back <= MOVED_WORLD && by > MOVED_WORLD {
        let why = "a depth change may only draw the camera back";
        return vec![(f32::MAX, json!({ "camera": camera, "why": why }))];
    }
    let old: Vec<&str> = before
        .nodes
        .iter()
        .filter_map(|n| n.unit.as_deref())
        .collect();
    // The free area as the depth change left it: a taller panel takes more of it.
    let free = now.free().shrink(SHOWN_WHOLE);
    let came: Vec<&Node> = now
        .nodes
        .iter()
        .filter(|n| n.unit.as_deref().is_some_and(|u| !old.contains(&u)))
        .collect();
    // What came in, or anything of the journey's, as the camera from before would
    // have shown it: off the free area, or too near its edge to show whole, draws
    // the camera back.
    let judged = now
        .nodes
        .iter()
        .filter(|n| n.in_journey() || n.unit.as_deref().is_some_and(|u| !old.contains(&u)));
    let off = judged.into_iter().any(|n| {
        n.world.is_some_and(|world| {
            shown_from(a, rect(now.graph_area), Vec3::from_array(world))
                .is_none_or(|was| !free.contains(egui::pos2(was.x, was.y)))
        })
    });
    // A sphere that came in, or any while the panel has grown, cut by the free area
    // as the camera from before would have shown it, or with no room above it for
    // its name, draws the camera back too.
    let grown = now.free() != before.free();
    let free = now.free();
    let named = egui::Rect::from_min_max(free.min + egui::vec2(0.0, NAME_ROOM), free.max);
    let cut = now
        .spheres()
        .filter(|m| grown || !before.spheres().any(|b| b.name == m.name))
        .any(|m| {
            let world = Vec3::from_array(m.world);
            let right = Quat::from_euler(EulerRot::YXZ, a.yaw, a.pitch, 0.0) * Vec3::X;
            let graph = rect(now.graph_area);
            match (
                shown_from(a, graph, world),
                shown_from(a, graph, world + right * m.world_radius),
            ) {
                (Some(at), Some(rim)) => {
                    let radius = at.distance(rim);
                    circle_out(free.shrink(SHOWN_WHOLE), at, radius) > 0.0
                        || circle_out(named, at, radius) > 0.0
                }
                _ => true,
            }
        });
    if off || cut {
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

/// Where `world` shows, in window points, seen by `camera` filling `graph` with the
/// viewer's field of view; none behind it.
fn shown_from(camera: Camera, graph: egui::Rect, world: Vec3) -> Option<Vec2> {
    let rotation = Quat::from_euler(EulerRot::YXZ, camera.yaw, camera.pitch, 0.0);
    let eye = Vec3::from_array(camera.target) + rotation * Vec3::Z * camera.distance;
    let local = rotation.inverse() * (world - eye);
    let depth = -local.z;
    if depth <= f32::EPSILON || graph.height() <= 0.0 {
        return None;
    }
    let up = (PerspectiveProjection::default().fov / 2.0).tan();
    let across = up * graph.width() / graph.height();
    Some(Vec2::new(
        graph.center().x + local.x / (depth * across) * graph.width() / 2.0,
        graph.center().y - local.y / (depth * up) * graph.height() / 2.0,
    ))
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

fn focus_on_screen(s: &Snap, before: Option<&Snap>, since: &Since) -> Value {
    // Wherever the hands put the camera, a cursor that moves expanded in a journey is
    // kept in view.
    let cursor_only = since.by_hand && s.cursor_moved_expanded(before);
    if since.by_hand && !cursor_only {
        return skip("the camera is where the user's hands put it");
    }
    if since.has(Kind::Expand) {
        return skip("an expand frames the module, wherever the focus is");
    }
    let free = s.free();
    // A function picked browsing since (a click, a search, Back) is in view wherever
    // it is: the expansion follows it.
    let picked = s.state.mode == "browse"
        && s.state.selection.is_some()
        && before.is_some_and(|b| b.state.selection != s.state.selection);
    // Expanded, the view is on the module expanded: what is in focus in another may
    // lie off it.
    let in_view = |n: &&Node| {
        (picked && n.is("selection"))
            || s.state
                .expanded
                .as_ref()
                .filter(|_| s.expanded())
                .is_none_or(|m| n.module.as_ref() == Some(m))
    };
    let focus: Vec<&Node> = s
        .nodes
        .iter()
        .filter(|n| {
            if cursor_only {
                n.is("cursor")
            } else {
                n.focus()
            }
        })
        .filter(in_view)
        .collect();
    if picked && !s.nodes.iter().any(|n| n.is("selection")) {
        return fail("the function picked is selected but not drawn");
    }
    if focus.is_empty() {
        return skip("no selection, start or cursor drawn, in the module expanded if one is");
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
        .any(|k| matches!(k, Kind::Zoom { out: true } | Kind::FrameAll | Kind::Pan));
    if !out {
        return skip("no wheel turn out, pan or R since the last snapshot");
    }
    let free = s.free();
    let spheres: Vec<&Module> = s.spheres().filter(|m| m.radius > 0.0).collect();
    let Some(extent) = map_extent(s) else {
        return skip("no module spheres drawn");
    };
    if extent.width() > free.width() || extent.height() > free.height() {
        // Dragged, a map bigger than the free area keeps over its middle.
        if since.has(Kind::Pan) {
            let mut offenders = Vec::new();
            if !extent.contains(free.center()) {
                let offender = json!({
                    "why": "dragged off the middle of the free area",
                    "map": rect_json([extent.min.x, extent.min.y, extent.max.x, extent.max.y]),
                    "middle": pt([free.center().x, free.center().y]),
                });
                offenders.push((1.0, offender));
            }
            return verdict(offenders);
        }
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
    // Dragged, a map that fits stays in the free area whole.
    let least = if since.has(Kind::Pan) { 1.0 } else { MAP_SHOWN };
    let mut offenders = Vec::new();
    // Zoomed all the way out, the map is held over the free area's middle, however
    // it was dragged.
    if s.state.camera.is_some_and(|c| c.farthest) && !extent.contains(free.center()) {
        let offender = json!({
            "why": "zoomed all the way out, the map is off the middle of the free area",
            "map": rect_json([extent.min.x, extent.min.y, extent.max.x, extent.max.y]),
            "middle": pt([free.center().x, free.center().y]),
        });
        offenders.push((f32::MAX, offender));
    }
    if share < least {
        let outside: Vec<&str> = spheres
            .iter()
            .filter(|m| circle_out(free, v2(m.at), m.radius) > EDGE)
            .map(|m| m.name.as_str())
            .collect();
        let offender = json!({
            "why": "the whole map would fit in the free area, but not this much of it is there",
            "inside": r2(share), "least": least, "map": rect_json([extent.min.x, extent.min.y, extent.max.x, extent.max.y]),
            "outside": outside,
        });
        offenders.push((least - share, offender));
    }
    with(verdict(offenders), "inside", json!(r2(share)))
}

/// Where every sphere shown lies on screen, together.
fn map_extent(s: &Snap) -> Option<egui::Rect> {
    s.spheres()
        .filter(|m| m.radius > 0.0)
        .map(|m| egui::Rect::from_center_size(pos(m.at), egui::Vec2::splat(2.0 * m.radius)))
        .reduce(egui::Rect::union)
}

fn spheres_unclipped(s: &Snap, before: Option<&Snap>, since: &Since) -> Value {
    let graph = rect(s.graph_area);
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(s.window[0], s.window[1]));
    if !graph.is_positive() {
        return skip("no graph area");
    }
    // The top of a panel docked along the graph's foot, where the view framed itself:
    // what it cuts shows half under the panel. Zoomed into a module, the modules round
    // it lie where they fall, as at the window's edges; shrunk back, the camera stands
    // where it stood before, whatever came in meanwhile.
    let free = s.free();
    let collapsed = before.is_some_and(Snap::expanded) && !s.expanded();
    let panel = (free.max.y < graph.max.y - EDGE && !since.by_hand && !s.expanded() && !collapsed)
        .then_some(free.max.y);
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
    let eye = s.camera.map(|e| Vec3::from_array(e.at));
    // A faint sphere is left alone at the panel, but the one holding the function
    // set aside, which the view brings in whole.
    let aside = s
        .nodes
        .iter()
        .find(|n| n.is("aside"))
        .and_then(|n| n.module.as_ref());
    let mut offenders = Vec::new();
    for m in s.spheres().filter(|m| m.radius > 0.0) {
        let faint = m.body == "faint" && aside != Some(&m.name);
        // Zoomed past it, the camera inside, it shows as no disc to cut.
        if eye.is_some_and(|eye| eye.distance(Vec3::from_array(m.world)) < m.world_radius) {
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
            (
                panel.is_some_and(|y| !faint && across && (c.y - y).abs() < m.radius),
                "panel",
            ),
            (left && down && (c.x - graph.min.x).abs() < m.radius, "left"),
            (
                right && down && (c.x - graph.max.x).abs() < m.radius,
                "right",
            ),
        ];
        for (_, edge) in cut.iter().filter(|(cut, _)| *cut) {
            let by = if *edge == "panel" {
                "the top of the panel".to_owned()
            } else {
                format!("the graph's {edge} edge, in open window")
            };
            let offender = json!({
                "module": m.name, "cut_by": by,
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
            let most = ARROW_WIDTH;
            let width = a.width.unwrap_or(0.0);
            if width > most {
                let offender = json!({ "arrow": kind, "from": a.from, "to": a.to, "width": r1(width), "most": most });
                offenders.push((width - most, offender));
            }
            let head = a.head.unwrap_or(0.0).max(a.head_width.unwrap_or(0.0) / 2.0);
            // The whole map's faint links between modules need no head to be seen.
            let (least, most) = HEAD_SIZE;
            let least = if a.flow == "module" { 0.0 } else { least };
            if !(least..=most).contains(&head) {
                let offender = json!({ "arrow": kind, "from": a.from, "to": a.to, "head": r1(head), "allowed": [least, most] });
                offenders.push(((least - head).max(head - most), offender));
            }
        }
    }
    // The whole map's links between modules are one width, however many calls each
    // carries.
    let widths: Vec<f32> = s
        .module_arrows
        .iter()
        .filter(shown)
        .filter_map(|a| a.width)
        .collect();
    let (thinnest, widest) = widths
        .iter()
        .fold((f32::INFINITY, 0.0_f32), |(lo, hi), &w| {
            (lo.min(w), hi.max(w))
        });
    if widest - thinnest > 0.1 {
        let offender = json!({ "arrow": "module", "why": "the whole map's links differ in width", "widths": [r1(thinnest), r1(widest)] });
        offenders.push((widest - thinnest, offender));
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
        let top = m.at[1] - m.radius;
        let above = top - r.max.y;
        // Browsing, where the room above is taken, smaller just inside its top.
        let size = s
            .labels
            .iter()
            .find(|l| l.names == "module" && l.of.as_ref() == Some(&m.name))
            .map_or(0.0, |l| l.size);
        let within = s.state.mode == "browse"
            && size <= INSIDE_NAME
            && (0.0..=NAME_ABOVE).contains(&(r.min.y - top));
        if off_middle > NAME_CENTRED || !((-EDGE..=NAME_ABOVE).contains(&above) || within) {
            let offender = json!({
                "module": m.name, "label": label,
                "why": "not centred just above its sphere's top edge, nor smaller just inside it",
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
        let own = s.nodes.iter().filter(|n| {
            n.on_screen
                && n.module.as_ref() == Some(&m.name)
                && n.radius > 0.0
                && (!within || n.lit())
        });
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
    // How far the placer kept names from a dot, leaning to less room either way.
    let kept = |n: &Node| n.kept_off.unwrap_or(n.radius) + ROUNDING;
    // Its own as drawn: the placer takes it as small as it is drawn lit.
    let own_kept = |n: &Node| n.radius - ROUNDING;
    let journey = s.state.mode == "journey";
    let taken = names_by_rank(s, journey);
    let guess =
        |text: &str, kind: &str, size: f32, width: f32| guess_size(s, text, kind, (size, width));
    let lit: Vec<&Node> = s.nodes.iter().filter(|n| n.on_screen && n.lit()).collect();
    let spheres: Vec<&Module> = s.spheres().filter(|m| m.radius > 0.0).collect();
    // Whether a pill at `r` would fit: in the free area, under no overlay, over no
    // name placed as early, off every other dot that keeps its name off (as far as
    // the placer keeps names, the calls a deeper level shows among them) and nearer
    // none of them than its own; a module's off every other sphere and its own dots.
    let clear = |r: egui::Rect, rank: u8, own: Option<&Node>, module: Option<(&str, bool)>| {
        // Over a dot judged by the pill, nearer one by the text, as labels-near does.
        let text = r.shrink2(NAME_PILL);
        let gap = |at: [f32; 2], radius: f32| text.distance_to_pos(pos(at)) - radius;
        let mine = own.map_or(0.0, |n| gap(n.at, own_kept(n)));
        let near = |at: [f32; 2], radius: f32| {
            r.distance_to_pos(pos(at)) - radius < NAME_CLEAR || gap(at, radius) < mine - NEARER
        };
        let keeps_off = |n: &Node| own.is_none_or(|o| o.kept_off_by(n));
        free.contains_rect(r)
            && !s.under_overlay(r)
            && !taken
                .iter()
                .any(|&(t, by)| by <= rank && t.intersects(r.expand(NAME_CLEAR)))
            && !lit.iter().any(|n| {
                n.unit != own.and_then(|o| o.unit.clone()) && keeps_off(n) && near(n.at, kept(n))
            })
            && !s.held.iter().any(|h| near(h.at, h.radius + ROUNDING))
            && module.is_none_or(|(module, within)| {
                !spheres
                    .iter()
                    .any(|m| m.name != module && r.distance_to_pos(pos(m.at)) < m.radius - 0.5)
                    && !s.nodes.iter().any(|n| {
                        n.on_screen
                            && n.module.as_deref() == Some(module)
                            && (!within || n.lit())
                            && near(n.at, n.radius)
                    })
            })
    };
    let mut offenders = Vec::new();
    for n in &lit {
        if n.label.is_some() || n.unit.is_none() {
            continue;
        }
        let size = n.name_size.map_or_else(
            || {
                let text = match n.numbers.first() {
                    Some(number) => format!("{number}  {}", n.name),
                    None => n.name.clone(),
                };
                guess(&text, "function", 12.5, CHAR_WIDTH)
            },
            |[w, h]| egui::vec2(w, h),
        );
        let size = size + 2.0 * NAME_PILL;
        let at = pos(n.at);
        let reach = n.radius + NAME_GAP;
        let spots = [
            egui::Rect::from_min_size(egui::pos2(at.x + reach, at.y - size.y / 2.0), size),
            egui::Rect::from_min_size(egui::pos2(at.x - reach - size.x, at.y - size.y / 2.0), size),
            egui::Rect::from_min_size(egui::pos2(at.x - size.x / 2.0, at.y - reach - size.y), size),
            egui::Rect::from_min_size(egui::pos2(at.x - size.x / 2.0, at.y + reach), size),
        ];
        // Where the placer tried it, the one place a name may go that stays put.
        let tried = n.name_spot.map(rect);
        let spots = tried
            .as_slice()
            .iter()
            .chain(spots.iter().filter(|_| tried.is_none()));
        let rank = n.rank();
        if let Some(spot) = spots.into_iter().find(|r| clear(**r, rank, Some(n), None)) {
            let offender = json!({ "function": n.name, "roles": n.roles, "would_fit_at": rect_json([spot.min.x, spot.min.y, spot.max.x, spot.max.y]) });
            offenders.push((1.0 + f32::from(u8::from(n.focus())), offender));
        }
    }
    for m in s
        .spheres()
        .filter(|m| m.on_screen && m.radius > 0.0 && m.label.is_none())
    {
        let size = m.name_size.map_or_else(
            || guess(&m.name.to_uppercase(), "module", 13.5, CAPITAL_WIDTH),
            |[w, h]| egui::vec2(w, h),
        );
        let [spot, inside] = module_spots(m, size);
        let fits = if clear(spot, m.rank(journey), None, Some((&m.name, false))) {
            Some(spot)
        } else {
            (!journey && clear(inside, u8::MAX, None, Some((&m.name, true)))).then_some(inside)
        };
        if let Some(spot) = fits {
            let offender = json!({ "module": m.name, "would_fit_at": rect_json([spot.min.x, spot.min.y, spot.max.x, spot.max.y]) });
            offenders.push((0.5, offender));
        }
    }
    verdict(offenders)
}

/// Where a module's name of `size` (its text, at the size above its sphere) would go:
/// just above its sphere, and browsing where that is taken, smaller just inside its
/// top.
fn module_spots(m: &Module, size: egui::Vec2) -> [egui::Rect; 2] {
    let top = m.at[1] - m.radius;
    let pill = size + 2.0 * NAME_PILL;
    let above =
        egui::Rect::from_min_size(egui::pos2(m.at[0] - pill.x / 2.0, top - 2.0 - pill.y), pill);
    let small = size * (INSIDE_NAME / 13.0) + 2.0 * NAME_PILL;
    let inside = egui::Rect::from_min_size(egui::pos2(m.at[0] - small.x / 2.0, top + 3.0), small);
    [above, inside]
}

/// Each name drawn, its pill round it, with the rank of what it names: only those
/// placed before what is left out, or as early, could have taken its room.
fn names_by_rank(s: &Snap, journey: bool) -> Vec<(egui::Rect, u8)> {
    s.labels
        .iter()
        .map(|l| {
            let rank = match l.names.as_str() {
                "function" => l
                    .unit
                    .as_deref()
                    .and_then(|u| s.node(u))
                    .map_or(0, Node::rank),
                "module" => s
                    .modules
                    .iter()
                    .find(|m| Some(&m.name) == l.of.as_ref())
                    .map_or(0, |m| m.rank(journey)),
                _ => 0,
            };
            (rect(l.rect).expand2(NAME_PILL), rank)
        })
        .collect()
}

/// How big a name of `kind` saying `text` would show, for one the snapshot gives no
/// size of: as wide a character as the widest of the names drawn of that kind, else
/// `fallback`'s width, at their size, else its size.
fn guess_size(s: &Snap, text: &str, kind: &str, fallback: (f32, f32)) -> egui::Vec2 {
    let drawn = || s.labels.iter().filter(|l| l.names == kind && l.size > 0.0);
    let size = drawn().next().map_or(fallback.0, |l| l.size);
    let width = drawn()
        .map(|l| rect(l.rect).width() / (l.text.trim().chars().count().max(1) as f32 * l.size))
        .fold(None, |most: Option<f32>, w| {
            Some(most.map_or(w, |m| m.max(w)))
        })
        .unwrap_or(fallback.1);
    egui::vec2(text.chars().count() as f32 * size * width, size * 1.2)
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
    if since.playing {
        return skip("a frame of a transition still playing");
    }
    let opened = since.opening
        || before.is_none_or(|b| b.state.mode != "journey" || b.state.start != s.state.start);
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
    let disc = |at: [f32; 2], radius: f32| {
        egui::Rect::from_center_size(pos(at), egui::Vec2::splat(2.0 * radius))
    };
    // The spheres holding what the journey lights, and the function set aside; a
    // faint one holds none of it, but for the one set aside.
    let aside = s
        .nodes
        .iter()
        .find(|n| n.is("aside"))
        .and_then(|n| n.module.as_ref());
    let spheres: Vec<&Module> = s
        .spheres()
        .filter(|m| m.radius > 0.0 && (m.body != "faint" || aside == Some(&m.name)))
        .collect();
    let span = lit
        .iter()
        .flat_map(|n| std::iter::once(disc(n.at, n.radius)).chain(n.label_rect.map(rect)))
        .chain(spheres.iter().map(|m| disc(m.at, m.radius)))
        .reduce(egui::Rect::union)
        .unwrap_or(egui::Rect::NOTHING);
    let share = (span.width() / free.width()).max(span.height() / free.height());
    let mut offenders = Vec::new();
    for m in spheres {
        let out = circle_out(free, v2(m.at), m.radius);
        if out > EDGE {
            let offender = json!({ "module": m.name, "out_of_the_free_area_by": r1(out) });
            offenders.push((out, offender));
        }
    }
    if share < JOURNEY_FILL {
        let offender = json!({
            "why": "the journey's spheres, functions and names take too little of the free area",
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
    // The function set aside keeps its role's fill, and a white ring says it is.
    for n in s.nodes.iter().filter(|n| n.is("aside") && !n.is("cursor")) {
        let white = n.rings.iter().filter_map(|r| rgb(&r.color)).any(whitish);
        if !white {
            let offender = json!({ "function": n.name, "why": "set aside, but no white ring", "rings": n.rings.iter().map(|r| &r.color).collect::<Vec<_>>() });
            offenders.push((1.0, offender));
        }
        if n.color.as_deref().and_then(rgb).is_some_and(whitish) {
            let offender = json!({ "function": n.name, "why": "set aside, but filled white as the start is", "fill": n.color });
            offenders.push((1.0, offender));
        }
    }
    // A call only guessed is drawn faint, as its bar is: darker than its role's.
    for n in s.nodes.iter().filter(|n| n.is("guessed")) {
        let (Some(got), Some(want)) = (n.color.as_deref().and_then(rgb), node_role_color(n)) else {
            continue;
        };
        let bright = |c: [f32; 3]| c[0].max(c[1]).max(c[2]);
        if bright(got) > GUESSED_DOT * bright(want) {
            let offender = json!({ "function": n.name, "why": "only guessed, but drawn as bright as a sure call", "fill": n.color, "role_color": hex3(want) });
            offenders.push((1.0, offender));
        }
    }
    // A call that calls the start back is ringed in the caller's violet.
    for n in s.nodes.iter().filter(|n| n.is("calls back") && !n.focus()) {
        let violet = n
            .rings
            .iter()
            .filter_map(|r| rgb(&r.color))
            .any(|c| like(c, srgb(CALLER)));
        if !violet {
            let offender = json!({ "function": n.name, "why": "calls back, but no violet ring", "rings": n.rings.iter().map(|r| &r.color).collect::<Vec<_>>() });
            offenders.push((1.0, offender));
        }
    }
    // A bar is painted as its node is: the cursor's other runs as it would be were
    // the cursor elsewhere; a call only guessed in the same hue, faint.
    for bar in s.bars.iter().filter(|b| !b.cursor) {
        let (Some(unit), Some(got)) = (&bar.unit, bar.color.as_deref().and_then(rgb)) else {
            continue;
        };
        let strength = bar.color.as_deref().and_then(alpha).unwrap_or(1.0);
        if bar.guessed && strength > FAINT {
            let offender = json!({ "bar": bar.number, "name": bar.name, "bar_color": bar.color, "why": "only guessed, but not faint", "most_alpha": FAINT });
            offenders.push((1.0, offender));
        }
        let Some(n) = s.node(unit).filter(|n| !n.is("cursor")) else {
            continue;
        };
        let want = if n.is("guessed") {
            node_role_color(n)
        } else {
            n.color.as_deref().and_then(rgb)
        };
        if let Some(want) = want
            && !like(got, want)
        {
            let offender = json!({ "bar": bar.number, "name": bar.name, "bar_color": bar.color, "node_color": n.color });
            offenders.push((1.0, offender));
        }
    }
    call_colours(s, cursor.as_deref(), &mut offenders);
    verdict(offenders)
}

/// Each call's color is its role's: none reads white; only the journey's call into
/// the `cursor` and the calls of the function set aside are drawn followed; a call
/// off the journey among the expanded module's functions, and one only guessed, are
/// faint.
fn call_colours(s: &Snap, cursor: Option<&str>, offenders: &mut Vec<(f32, Value)>) {
    let aside = s
        .nodes
        .iter()
        .find(|n| n.is("aside"))
        .and_then(|n| n.unit.as_deref());
    for a in &s.arrows {
        let Some(got) = a.color.as_deref().and_then(rgb) else {
            continue;
        };
        let strength = a.color.as_deref().and_then(alpha).unwrap_or(1.0);
        let from_aside = aside.is_some() && a.from_unit.as_deref() == aside;
        let into_cursor = cursor.is_some() && a.to_unit.as_deref() == cursor && a.flow == "journey";
        // White is the selection's and the start's alone: no call reads white on the
        // ground, a grey one from off the journey followed least of all.
        let shown = over_ground(got, strength);
        if whitish(shown) {
            let offender = json!({ "arrow": [a.from, a.to], "flow": a.flow, "color": a.color, "why": "a call that reads white, as the selection does", "shows": hex3(shown) });
            offenders.push((1.0, offender));
            continue;
        }
        // Only the journey's call into the cursor and the calls of the function set
        // aside are drawn stronger or wider than the rest: pointing at a function
        // lights none.
        let bright = strength > CALL_STRENGTH + 0.05 || a.width.unwrap_or(0.0) > CALL_WIDTH + 0.25;
        if bright && !from_aside && !into_cursor {
            let offender = json!({ "arrow": [a.from, a.to], "flow": a.flow, "color": a.color, "width": a.width, "why": "drawn as followed, but neither into the cursor nor from the function set aside" });
            offenders.push((1.0, offender));
            continue;
        }
        // A call among the expanded module's functions that the journey does not make,
        // out of the function set aside too, and a call only guessed, are faint.
        let faint = a.flow == "inner" || a.guessed;
        if faint && strength > FAINT {
            let why = if a.guessed {
                "a call only guessed drawn strong"
            } else {
                "a call off the journey drawn bright"
            };
            let offender = json!({ "arrow": [a.from, a.to], "flow": a.flow, "color": a.color, "why": why, "most_alpha": FAINT });
            offenders.push((1.0, offender));
            continue;
        }
        let want = flow_color(&a.flow, a.level);
        let pink = into_cursor && like(got, srgb(CURSOR));
        if !like(got, want) && !pink {
            let offender = json!({ "arrow": [a.from, a.to], "flow": a.flow, "level": a.level, "color": a.color, "role_color": hex3(want) });
            offenders.push((1.0, offender));
        }
    }
}

/// No call's line runs through a lit dot, or a lit function's name in a sphere it
/// leaves or enters, but its own ends': it goes round them. Left out: the faint
/// calls, among an expanded module's functions or only guessed; faint unnamed dots;
/// an expanded module's plain functions, dots and names, which in a crowded one no
/// line could keep off (it breaks round a dot); and browsing, names, which sit where
/// the map's dots leave room.
fn arrows_clear(s: &Snap) -> Value {
    let dots: Vec<&Node> = s
        .nodes
        .iter()
        .filter(|n| n.on_screen && n.lit() && !n.plain())
        .collect();
    // Browsing, every function stays where the map has it and its name where the
    // placer finds room: a line round a crowd of calls is judged by the dots alone.
    let browsing = s.state.mode == "browse";
    let lit = |unit: &Option<String>| {
        !browsing && dots.iter().any(|n| n.unit.is_some() && n.unit == *unit)
    };
    let names: Vec<&Label> = s
        .labels
        .iter()
        .filter(|l| l.names == "function" && lit(&l.unit))
        .collect();
    let mut offenders = Vec::new();
    let mut judged = 0;
    for a in s
        .arrows
        .iter()
        .filter(|a| a.flow != "inner" && !a.guessed && a.path.len() >= 2)
    {
        judged += 1;
        let own =
            |unit: &Option<String>| unit.is_some() && (*unit == a.from_unit || *unit == a.to_unit);
        let pieces: Vec<(Vec2, Vec2)> = a.path.windows(2).map(|p| (v2(p[0]), v2(p[1]))).collect();
        for n in dots.iter().filter(|n| !own(&n.unit)) {
            let near = pieces
                .iter()
                .map(|&(p, q)| piece_distance(p, q, v2(n.at)))
                .fold(f32::INFINITY, f32::min);
            if near < n.radius - ARROW_SLACK {
                let offender = json!({ "arrow": [a.from, a.to], "through": "dot", "function": n.name, "at": pt(n.at), "off_by": r1(n.radius - near) });
                offenders.push((n.radius - near, offender));
            }
        }
        // Names in the spheres it leaves and enters: where its ends' neighbours are
        // laid out round it.
        let module =
            |unit: &Option<String>| unit.as_deref().and_then(|u| s.node(u)?.module.clone());
        let ends = [module(&a.from_unit), module(&a.to_unit)];
        let near = |l: &&&Label| ends.contains(&module(&l.unit));
        for l in names.iter().filter(|l| !own(&l.unit)).filter(near) {
            let pill = rect(l.rect).shrink(ARROW_SLACK);
            if pieces.iter().any(|&(p, q)| crosses_rect(p, q, pill)) {
                let offender = json!({ "arrow": [a.from, a.to], "through": "name", "label": l.text, "rect": rect_json(l.rect) });
                offenders.push((1.0, offender));
            }
        }
    }
    if judged == 0 {
        return skip("no call drawn");
    }
    verdict(offenders)
}

/// No two functions read the same name: on the graph within one module, in the
/// journey's bars, or among the search's rows, each judged on its own. Two alike are
/// told apart by their type or module.
fn names_distinct(s: &Snap) -> Value {
    let module = |unit: &str| {
        s.node(unit)
            .and_then(|n| n.module.clone())
            .unwrap_or_default()
    };
    let graph = s
        .labels
        .iter()
        .filter(|l| l.names == "function")
        .filter_map(|l| {
            let unit = l.unit.clone()?;
            Some((format!("{} in {}", l.of.clone()?, module(&unit)), unit))
        });
    let bars = s
        .bars
        .iter()
        .filter_map(|b| Some((b.name.clone(), b.unit.clone()?)));
    let rows = s
        .search_rows
        .iter()
        .flatten()
        .filter_map(|r| Some((r.text.clone(), r.unit.clone()?)));
    let lists: [(&str, Vec<(String, String)>); 3] = [
        ("graph", graph.collect()),
        ("bars", bars.collect()),
        ("search rows", rows.collect()),
    ];
    let mut offenders = Vec::new();
    let mut judged = 0;
    for (place, names) in &lists {
        judged += names.len();
        let mut by_name: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (name, unit) in names {
            let units = by_name.entry(name.as_str()).or_default();
            if !units.contains(&unit.as_str()) {
                units.push(unit);
            }
        }
        for (name, units) in by_name.into_iter().filter(|(_, u)| u.len() > 1) {
            let offender = json!({ "in": place, "name": name, "functions": units });
            offenders.push((1.0, offender));
        }
    }
    if judged == 0 {
        return skip("no function named");
    }
    verdict(offenders)
}

/// Expanded in a journey, every call drawn from a function in the free area to one
/// off it is labelled where its line leaves, with the far function's name, on a pill
/// inside the free area.
fn exits_named(s: &Snap) -> Value {
    if !(s.expanded() && s.state.mode == "journey") {
        return skip("not expanded in a journey");
    }
    let free = s.free();
    let inside = |at: [f32; 2]| free.contains(pos(at));
    let mut offenders = Vec::new();
    let mut judged = 0;
    for a in s
        .arrows
        .iter()
        .filter(|a| a.flow != "inner" && a.path.len() >= 2)
    {
        let (first, last) = (a.path[0], a.path[a.path.len() - 1]);
        if inside(first) == inside(last) {
            continue;
        }
        judged += 1;
        let far = if inside(first) { &a.to } else { &a.from };
        let exit = s
            .exits
            .iter()
            .find(|e| e.from_unit == a.from_unit && e.to_unit == a.to_unit);
        match exit {
            Some(e)
                if e.far == *far
                    && e.text.contains(far.as_str())
                    && free.contains_rect(rect(e.rect)) => {}
            Some(e) => {
                let offender = json!({ "arrow": [a.from, a.to], "why": "its exit label does not name the far function inside the free area", "label": e.text, "rect": rect_json(e.rect) });
                offenders.push((1.0, offender));
            }
            None => {
                let offender = json!({ "arrow": [a.from, a.to], "why": "leaves the free area unlabelled", "far": far });
                offenders.push((1.0, offender));
            }
        }
    }
    if judged == 0 {
        return skip("no call leaves the free area");
    }
    verdict(offenders)
}

/// Every call into the selection (or the start) and each of its own, between two dots
/// drawn on screen apart, is drawn as an arrow: however near the two dots, so a
/// function that both calls it and is called by it shows both.
fn calls_drawn(s: &Snap) -> Value {
    let Some(centre) = s.content.start.as_ref().or(s.content.selection.as_ref()) else {
        return skip("nothing selected and no journey");
    };
    let node = |name: &str| s.nodes.iter().find(|n| n.name == name && n.on_screen);
    if node(centre).is_none() {
        return skip("the selection or the start is not on screen");
    }
    let own = s
        .content
        .calls
        .iter()
        .filter(|c| c.parent.as_deref().is_none_or(str::is_empty))
        .map(|c| (centre.as_str(), c.name.as_str()));
    let into = s
        .content
        .callers
        .iter()
        .map(|c| (c.as_str(), centre.as_str()));
    let mut offenders = Vec::new();
    let mut judged = 0;
    for (from, to) in own.chain(into).filter(|(from, to)| from != to) {
        let (Some(a), Some(b)) = (node(from), node(to)) else {
            continue;
        };
        // Two dots touching leave no room for a line, its gaps and its head.
        if v2(a.at).distance(v2(b.at)) <= a.radius + b.radius + HEAD_SIZE.0 + 2.0 {
            continue;
        }
        judged += 1;
        if !s.arrows.iter().any(|x| x.from == from && x.to == to) {
            let offender = json!({ "call": [from, to], "apart": r1(v2(a.at).distance(v2(b.at))), "why": "not drawn" });
            offenders.push((1.0, offender));
        }
    }
    if judged == 0 {
        return skip("no call between two dots on screen");
    }
    verdict(offenders)
}

/// How near the piece of line from `p` to `q` comes to `at`.
fn piece_distance(p: Vec2, q: Vec2, at: Vec2) -> f32 {
    let along = q - p;
    let t = ((at - p).dot(along) / along.length_squared().max(f32::EPSILON)).clamp(0.0, 1.0);
    (p + along * t).distance(at)
}

/// Whether the piece of line from `p` to `q` runs through `r`.
fn crosses_rect(p: Vec2, q: Vec2, r: egui::Rect) -> bool {
    if r.width() <= 0.0 || r.height() <= 0.0 {
        return false;
    }
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    let along = q - p;
    for (start, delta, low, high) in [
        (p.x, along.x, r.min.x, r.max.x),
        (p.y, along.y, r.min.y, r.max.y),
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
}

/// How `color` drawn at `strength` shows over the ground behind the graph.
fn over_ground(color: [f32; 3], strength: f32) -> [f32; 3] {
    let ground = srgb(GROUND);
    [0, 1, 2].map(|i| color[i] * strength + ground[i] * (1.0 - strength))
}

/// The color a function's role gives its fill, if its roles name one.
fn node_role_color(n: &Node) -> Option<[f32; 3]> {
    if n.is("cursor") {
        return Some(srgb(CURSOR));
    }
    if n.is("start") || n.is("selection") {
        return Some([1.0, 1.0, 1.0]);
    }
    if n.is("caller") {
        return Some(srgb(CALLER));
    }
    let level = n
        .roles
        .iter()
        .find_map(|r| r.strip_prefix("journey level ")?.parse::<u32>().ok());
    match level {
        Some(level) => Some(flow_color("journey", Some(level))),
        // Set aside off the journey: grey, its ring alone white.
        None if n.is("aside") => Some(srgb(FURTHER)),
        None => None,
    }
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
/// A `#rrggbbaa` color's alpha, 0 to 1.
fn alpha(hex: &str) -> Option<f32> {
    let hex = hex.trim_start_matches('#');
    u8::from_str_radix(hex.get(6..8)?, 16)
        .ok()
        .map(|a| f32::from(a) / 255.0)
}

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
            ..Since::default()
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
    fn a_call_through_a_third_dot_or_name_is_named_and_one_bent_round_it_passes() {
        let mut s = scene();
        s["state"]["mode"] = json!("journey");
        // From f straight to h, over f's own name, through g's spot made a lit dot.
        s["nodes"][1]["roles"] = json!(["callee"]);
        s["nodes"][1]["radius"] = json!(4.0);
        s["arrows"] = json!([{ "from": "f", "to": "h", "from_unit": "x:a::f", "to_unit": "x:b::h",
            "flow": "journey", "level": 1, "path": [[386.0, 500.0], [994.0, 500.0]] }]);
        let result = check(&s, "arrows-clear");
        assert_eq!(result["count"], 1, "{result}");
        assert_eq!(result["offenders"][0]["function"], "g");
        // A name in the way is named too.
        s["labels"]
            .as_array_mut()
            .expect("labels")
            .push(json!({ "text": "g", "rect": [700.0, 494.0, 720.0, 506.0],
                "names": "function", "of": "g", "unit": "x:a::g" }));
        assert_eq!(check(&s, "arrows-clear")["count"], 2);
        // Bent round both, it passes.
        s["arrows"][0]["path"] = json!([
            [386.0, 500.0],
            [440.0, 530.0],
            [710.0, 530.0],
            [994.0, 500.0]
        ]);
        assert!(
            passes(&check(&s, "arrows-clear")),
            "{}",
            check(&s, "arrows-clear")
        );
    }

    #[test]
    fn two_functions_reading_the_same_name_are_named() {
        let mut s = scene();
        s["labels"][2]["of"] = json!("f");
        // Two in different modules read apart by their spheres.
        assert!(passes(&check(&s, "names-distinct")));
        s["nodes"][2]["module"] = json!("a");
        let result = check(&s, "names-distinct");
        assert_eq!(result["count"], 1, "{result}");
        assert_eq!(result["offenders"][0]["in"], "graph");
        let mut s = scene();
        s["search_rows"] = json!([
            { "text": "reroot", "unit": "x:a::reroot" },
            { "text": "reroot", "unit": "x:b::reroot" },
        ]);
        assert_eq!(
            check(&s, "names-distinct")["offenders"][0]["in"],
            "search rows"
        );
        s["search_rows"][1]["text"] = json!("Track.reroot");
        assert!(passes(&check(&s, "names-distinct")));
    }

    #[test]
    fn expanded_in_a_journey_a_call_off_the_free_area_is_labelled_where_it_leaves() {
        let mut s = scene();
        s["state"]["mode"] = json!("journey");
        s["state"]["nucleus"] = json!("expanded");
        s["arrows"] = json!([{ "from": "f", "to": "far", "from_unit": "x:a::f", "to_unit": "x:c::far",
            "flow": "journey", "level": 1, "path": [[386.0, 500.0], [386.0, 1200.0]] }]);
        assert_eq!(check(&s, "exits-named")["count"], 1);
        s["exits"] = json!([{ "from_unit": "x:a::f", "to_unit": "x:c::far", "far": "far",
            "text": "to far · C", "rect": [350.0, 975.0, 420.0, 995.0] }]);
        assert!(
            passes(&check(&s, "exits-named")),
            "{}",
            check(&s, "exits-named")
        );
        s["state"]["nucleus"] = json!("modules");
        assert_eq!(check(&s, "exits-named")["pass"], Value::Null);
    }

    #[test]
    fn a_call_between_dots_on_screen_is_drawn_however_near_they_are() {
        let mut s = scene();
        s["content"] = json!({ "selection": "f", "callers": ["h"], "calls": [{ "name": "h" }] });
        s["arrows"] = json!([{ "from": "f", "to": "h" }]);
        let result = check(&s, "calls-drawn");
        assert_eq!(result["count"], 1, "{result}");
        assert_eq!(result["offenders"][0]["call"], json!(["h", "f"]));
        s["arrows"] = json!([{ "from": "f", "to": "h" }, { "from": "h", "to": "f" }]);
        assert!(passes(&check(&s, "calls-drawn")));
    }

    #[test]
    fn the_whole_maps_links_are_one_width() {
        let mut s = scene();
        s["module_arrows"] = json!([
            { "from": "a", "to": "b", "flow": "module", "drawn": true, "on_screen": true, "width": 1.0, "head": 6.0 },
            { "from": "b", "to": "a", "flow": "module", "drawn": true, "on_screen": true, "width": 2.0, "head": 6.0 },
        ]);
        assert!(!passes(&check(&s, "arrow-sizes")));
        s["module_arrows"][1]["width"] = json!(1.0);
        assert!(
            passes(&check(&s, "arrow-sizes")),
            "{}",
            check(&s, "arrow-sizes")
        );
    }

    #[test]
    fn the_function_set_aside_keeps_its_fill_and_wears_a_white_ring() {
        let mut s = journey();
        let grey = "#d1d9ebff";
        s["nodes"][1]["roles"] = json!(["aside"]);
        s["nodes"][1]["color"] = json!("#ffffffff");
        s["nodes"][1]["rings"] = json!([{ "radius": 6.0, "color": "#ffffffff" }]);
        assert!(
            !passes(&check(&s, "colours")),
            "a white fill is the start's alone"
        );
        s["nodes"][1]["color"] = json!(grey);
        assert!(passes(&check(&s, "colours")), "{}", check(&s, "colours"));
        s["nodes"][1]["rings"] = json!([]);
        assert!(!passes(&check(&s, "colours")), "no white ring");
    }

    #[test]
    fn a_call_reads_white_nowhere_and_only_the_followed_are_drawn_bright() {
        let mut s = journey();
        // A grey call from off the journey at full strength reads white.
        s["arrows"] = json!([{ "from": "g", "to": "f", "from_unit": "x:a::g", "to_unit": "x:a::f",
            "flow": "outside", "color": "#d1d9ebff", "width": 1.5 }]);
        assert_eq!(check(&s, "colours")["count"], 1);
        s["arrows"][0]["color"] = json!("#d1d9eb59");
        assert!(passes(&check(&s, "colours")), "{}", check(&s, "colours"));
        // Pointing at it lights nothing: wider is followed.
        s["arrows"][0]["width"] = json!(2.0);
        assert_eq!(check(&s, "colours")["count"], 1);
    }

    #[test]
    fn a_call_only_guessed_is_faint_on_the_graph_and_in_the_panel() {
        let mut s = journey();
        s["arrows"] = json!([{ "from": "f", "to": "h", "from_unit": "x:a::f", "to_unit": "x:b::h",
            "flow": "journey", "level": 1, "color": "#4db8ffd9", "width": 1.5, "guessed": true }]);
        assert_eq!(check(&s, "colours")["count"], 1, "{}", check(&s, "colours"));
        s["arrows"][0]["color"] = json!("#4db8ff4d");
        assert!(passes(&check(&s, "colours")), "{}", check(&s, "colours"));
        s["bars"] = json!([{ "number": "1", "name": "h", "unit": "x:b::h", "color": "#4DB8FF", "guessed": true }]);
        assert_eq!(check(&s, "colours")["count"], 1, "{}", check(&s, "colours"));
        s["bars"][0]["color"] = json!("#4DB8FF38");
        assert!(passes(&check(&s, "colours")), "{}", check(&s, "colours"));
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

    /// `s` with module `a` expanded in the middle, `radius` across on screen, its
    /// functions `f` and `g` half that either side of its middle, `f` named `name`.
    fn expanded(mut s: Value, radius: f32, name: &str) -> Value {
        s["state"]["nucleus"] = json!("expanded");
        s["modules"][0]["body"] = json!("nucleus");
        s["modules"][0]["at"] = json!([800.0, 512.0]);
        s["modules"][0]["radius"] = json!(radius);
        s["nodes"][0]["name"] = json!(name);
        s["nodes"][0]["at"] = json!([800.0 - radius / 2.0, 512.0]);
        s["nodes"][1]["at"] = json!([800.0 + radius / 2.0, 512.0]);
        s
    }

    #[test]
    fn the_expanded_module_takes_what_its_functions_need_after_expanding() {
        let fill = |s: &Value| after(s, s, &[Kind::Expand], "expanded-fill");
        // Two short names: the least share is room enough.
        let result = fill(&expanded(scene(), 400.0, "f"));
        assert!(
            !passes(&result),
            "far bigger than two short names need: {result}"
        );
        assert_eq!(result["offenders"][0]["wanted"], 0.35);
        let result = fill(&expanded(scene(), 171.0, "f"));
        assert!(passes(&result), "{result}");
        // A long name beside its neighbour needs the most.
        let long = "a_function_name_long_enough_to_need_the_most_room_there_is";
        let mut s = expanded(scene(), 400.0, long);
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
        s["modules"][1]["radius"] = json!(171.0);
        let result = after(&s, &s, &[Kind::Expand], "expanded-fill");
        assert!(
            passes(&result),
            "b fills the view, not the nucleus a: {result}"
        );
    }

    #[test]
    fn a_small_module_expanded_in_a_journey_keeps_its_neighbours_on_screen() {
        let mut s = journey();
        s["state"]["nucleus"] = json!("expanded");
        s["state"]["expanded"] = json!("b");
        s["arrows"] = json!([{ "from": "f", "to": "h", "from_unit": "x:a::f", "to_unit": "x:b::h",
                               "flow": "journey", "level": 1, "on_screen": true }]);
        let fill = |s: &Value| after(s, s, &[Kind::Expand], "expanded-fill");
        assert!(passes(&fill(&s)), "small, beside the module calling it");
        s["modules"][0]["at"] = json!([-200.0, 500.0]);
        let result = fill(&s);
        assert_eq!(result["offenders"][0]["module"], "a", "{result}");
        s["modules"][0]["at"] = json!([400.0, 500.0]);
        s["modules"][1]["functions"] = json!(12);
        assert!(!passes(&fill(&s)), "a bigger one takes its share");
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
    fn zooming_in_stops_with_the_sphere_under_the_pointer_in_the_free_area() {
        let mut s = scene();
        // a, under the pointer, 880 across of the free area's 976 down: 0.9.
        s["modules"][0]["radius"] = json!(440.0);
        s["modules"][0]["at"] = json!([500.0, 512.0]);
        assert!(passes(&after(&s, &s, &[IN], "zoom-bounds")));
        s["modules"][0]["at"] = json!([400.0, 600.0]);
        let result = after(&s, &s, &[IN], "zoom-bounds");
        assert!(
            result["offenders"][0]["why"]
                .as_str()
                .unwrap()
                .contains("not all in the free area"),
            "{result}"
        );
        s["modules"][0]["radius"] = json!(600.0);
        let result = after(&s, &s, &[IN], "zoom-bounds");
        assert_eq!(result["offenders"][0]["module"], "a", "{result}");
        assert_eq!(
            after(&s, &s, &[OUT], "zoom-bounds")["pass"],
            json!(true),
            "out, the bound out alone"
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
    /// bringing in `k` at `world`, `along` the usual view's right from the middle.
    fn deeper(along: f32) -> Value {
        let mut now = scene();
        now["state"]["camera"]["distance"] = json!(200.0);
        let world = (crate::nucleus::view_rotation() * Vec3::X * along).to_array();
        let k = json!({ "name": "k", "unit": "x:b::k", "module": "b", "roles": ["journey level 2"],
            "at": [800.0, 500.0], "radius": 4.0, "world": world, "world_radius": 0.2, "on_screen": true });
        now["nodes"].as_array_mut().unwrap().push(k);
        now
    }

    #[test]
    fn a_depth_change_draws_the_camera_back_only_when_what_came_in_needs_it() {
        let before = scene();
        let camera = Camera {
            target: [0.0; 3],
            distance: 100.0,
            yaw: VIEW.0,
            pitch: VIEW.1,
            ..Camera::default()
        };
        let graph = egui::Rect::from_min_max(egui::pos2(0.0, 24.0), egui::pos2(1600.0, 1000.0));
        let shows = |along: f32| {
            let world = crate::nucleus::view_rotation() * Vec3::X * along;
            shown_from(camera, graph, world).unwrap()
        };
        assert!((shows(0.0) - Vec2::new(800.0, 512.0)).length() < 1e-3);
        // From where the camera was, k would have shown off the right edge.
        assert!(shows(80.0).x > 1600.0);
        let needed = after(&before, &deeper(80.0), &[Kind::Depth], "nothing-moved");
        assert!(passes(&needed), "{needed}");
        // Near the middle it would have shown anyway.
        assert!(shows(10.0).x < 1400.0);
        let result = after(&before, &deeper(10.0), &[Kind::Depth], "nothing-moved");
        assert!(!passes(&result));
        assert!(
            result["offenders"][0]["why"]
                .as_str()
                .unwrap()
                .contains("would have shown")
        );
        // A sphere that came in, cut by the free area's edge from where the camera was.
        let mut now = deeper(10.0);
        let world = (crate::nucleus::view_rotation() * Vec3::X * 75.0).to_array();
        now["modules"].as_array_mut().unwrap().push(json!({
            "name": "c", "body": "open", "shown": true, "functions": 1, "at": [1200.0, 500.0],
            "radius": 40.0, "world": world, "world_radius": 6.0, "on_screen": true,
        }));
        let result = after(&before, &now, &[Kind::Depth], "nothing-moved");
        assert!(passes(&result), "{result}");
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
        // A click that selects another function names what that lights instead.
        let mut selected = now.clone();
        selected["state"]["selection"] = json!("g");
        let result = after(&before, &selected, &[Kind::Click], "nothing-moved");
        assert!(passes(&result), "{result}");
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
            ..Since::default()
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
        assert!(
            !passes(&after(&s, &s, &[Kind::Pan], "map-on-screen")),
            "nor dragged out"
        );
        // Bigger than the free area, it cannot all be there.
        s["modules"][0]["radius"] = json!(900.0);
        assert_eq!(after(&s, &s, &[OUT], "map-on-screen")["pass"], Value::Null);
        // But dragged, it stays over the free area's middle.
        assert!(passes(&after(&s, &s, &[Kind::Pan], "map-on-screen")));
        s["modules"][0]["at"] = json!([2300.0, 500.0]);
        s["modules"][1]["at"] = json!([3300.0, 500.0]);
        let result = after(&s, &s, &[Kind::Pan], "map-on-screen");
        assert!(!passes(&result), "{result}");
    }

    #[test]
    fn a_sphere_cut_off_in_open_window_is_named() {
        let mut s = scene();
        s["graph_area"] = json!([0.0, 24.0, 1600.0, 560.0]);
        s["window"] = json!([1600.0, 1000.0]);
        s["overlays"] = json!([{ "name": "journey panel", "rect": [12.0, 890.0, 1588.0, 988.0] }]);
        let result = check(&s, "spheres-unclipped");
        assert_eq!(result["count"], 2, "both cross y = 560: {result}");
        // However big it shows, while the camera is outside it.
        s["modules"][0]["radius"] = json!(900.0);
        assert_eq!(check(&s, "spheres-unclipped")["count"], 2);
        s["camera"] = json!({ "at": [-10.0, 0.0, 1.0] });
        assert_eq!(
            check(&s, "spheres-unclipped")["count"],
            1,
            "inside a, it shows no disc"
        );
        // The panel starts where the graph ends: what it cuts is under the panel.
        s["overlays"][0]["rect"] = json!([12.0, 562.0, 1588.0, 988.0]);
        assert!(passes(&check(&s, "spheres-unclipped")));
    }

    #[test]
    fn a_sphere_half_under_the_panel_is_named_unless_the_hands_put_it_there() {
        let mut s = scene();
        s["window"] = json!([1600.0, 1000.0]);
        s["graph_area"] = json!([0.0, 24.0, 1600.0, 1000.0]);
        s["overlays"] = json!([{ "name": "journey panel", "rect": [0.0, 560.0, 1600.0, 1000.0] }]);
        let result = check(&s, "spheres-unclipped");
        assert_eq!(result["count"], 2, "{result}");
        assert_eq!(result["offenders"][0]["cut_by"], "the top of the panel");
        let mut faint = s.clone();
        faint["modules"][0]["body"] = json!("faint");
        faint["modules"][1]["body"] = json!("faint");
        assert!(
            passes(&check(&faint, "spheres-unclipped")),
            "faint, left alone"
        );
        faint["nodes"][2]["roles"] = json!(["aside"]);
        let result = check(&faint, "spheres-unclipped");
        assert_eq!(
            result["offenders"][0]["module"], "b",
            "but the aside's: {result}"
        );
        let by_hand = Since {
            kinds: vec![Kind::Zoom { out: false }],
            by_hand: true,
            ..Since::default()
        };
        assert!(passes(&run(&s, None, &by_hand)["spheres-unclipped"]));
        s["state"]["nucleus"] = json!("expanded");
        assert!(
            passes(&check(&s, "spheres-unclipped")),
            "zoomed into a module, those round it fall where they do"
        );
    }

    #[test]
    fn expanded_the_focus_off_the_module_and_a_hand_placed_camera_are_left_alone() {
        let mut s = journey();
        s["overlays"] = json!([{ "name": "journey panel", "rect": [0.0, 800.0, 1600.0, 1000.0] }]);
        s["nodes"][2]["at"] = json!([1000.0, 820.0]);
        s["state"]["nucleus"] = json!("expanded");
        s["state"]["expanded"] = json!("a");
        assert!(
            passes(&check(&s, "focus-on-screen")),
            "the cursor is in b, not the module expanded"
        );
        s["state"]["expanded"] = json!("b");
        assert!(!passes(&check(&s, "focus-on-screen")));
        // After an orbit, a cursor step expanded in a journey still keeps it in view.
        let mut before = s.clone();
        before["nodes"][2]["roles"] = json!(["journey level 1"]);
        before["nodes"][0]["roles"] = json!(["start", "cursor"]);
        let orbited = Since {
            kinds: vec![Kind::Still],
            by_hand: true,
            ..Since::default()
        };
        let result = run(&s, Some(&before), &orbited)["focus-on-screen"].clone();
        assert_eq!(result["offenders"][0]["function"], "h", "{result}");
        assert_eq!(
            run(&s, Some(&s), &orbited)["focus-on-screen"]["pass"],
            Value::Null,
            "the cursor stayed: the hands' camera stands"
        );
    }

    #[test]
    fn a_cursor_that_takes_the_expansion_along_fills_the_view_with_its_module() {
        let mut before = journey();
        before["state"]["nucleus"] = json!("expanded");
        before["state"]["expanded"] = json!("a");
        let mut now = before.clone();
        now["state"]["expanded"] = json!("b");
        now["modules"][1]["functions"] = json!(12);
        let result = after(&before, &now, &[Kind::Still], "expanded-fill");
        assert!(!passes(&result), "b fills too little: {result}");
        assert_eq!(
            after(&now, &now, &[Kind::Still], "expanded-fill")["pass"],
            Value::Null
        );
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
    fn only_names_placed_as_early_excuse_one_left_out() {
        let mut s = journey();
        s["nodes"][2]["label"] = Value::Null;
        s["nodes"][2]["label_rect"] = Value::Null;
        s["labels"].as_array_mut().unwrap().remove(2);
        // Hemmed in by the names of g, plain, placed after the cursor: still room.
        let blocks = [
            [1004.0, 480.0, 1100.0, 520.0],
            [900.0, 480.0, 996.0, 520.0],
            [950.0, 440.0, 1050.0, 496.0],
            [950.0, 504.0, 1050.0, 560.0],
        ];
        for r in blocks {
            s["labels"].as_array_mut().unwrap().push(
                json!({ "text": "g", "rect": r, "names": "function", "unit": "x:a::g", "size": 12.0 }),
            );
        }
        let result = check(&s, "named-when-room");
        assert_eq!(result["offenders"][0]["function"], "h", "{result}");
        // The size the placer laid its name out at, too wide for any side.
        s["nodes"][2]["name_size"] = json!([2000.0, 14.0]);
        assert!(passes(&check(&s, "named-when-room")));
    }

    #[test]
    fn a_plain_dot_or_name_never_excuses_a_lit_name_left_out() {
        let mut s = journey();
        s["state"]["nucleus"] = json!("expanded");
        s["nodes"][2]["label"] = Value::Null;
        s["nodes"][2]["label_rect"] = Value::Null;
        s["labels"].as_array_mut().unwrap().remove(2);
        // Where the placer tried it, right of its dot: a plain dot and a plain name
        // lie there.
        let tried = [1009.0, 492.0, 1049.0, 508.0];
        s["nodes"][2]["name_spot"] = json!(tried);
        s["nodes"].as_array_mut().unwrap().push(json!({
            "name": "p", "unit": "x:b::p", "module": "b", "roles": ["opened"],
            "at": [1030.0, 500.0], "radius": 4.0, "world": [11.0, 0.0, 0.0],
            "world_radius": 0.2, "on_screen": true, "label": "p",
            "label_rect": [1020.0, 486.0, 1040.0, 498.0],
        }));
        s["labels"].as_array_mut().unwrap().push(json!({
            "text": "p", "rect": [1020.0, 486.0, 1040.0, 498.0], "names": "function",
            "of": "p", "unit": "x:b::p", "size": 12.0,
        }));
        let result = check(&s, "named-when-room");
        assert_eq!(result["offenders"][0]["function"], "h", "{result}");
        assert_eq!(result["offenders"][0]["would_fit_at"], json!(tried));
        // A dot a deeper level shows there keeps it off.
        s["held"] = json!([{ "at": [1030.0, 500.0], "radius": 4.0 }]);
        assert!(passes(&check(&s, "named-when-room")));
        // Lit, the dot keeps it off too.
        s["held"] = json!([]);
        s["nodes"][3]["roles"] = json!(["journey level 2"]);
        assert!(passes(&check(&s, "named-when-room")));
    }

    #[test]
    fn a_lit_name_may_lie_over_a_plain_dot_but_a_plain_one_may_not() {
        let mut s = scene();
        s["state"]["nucleus"] = json!("expanded");
        s["nodes"][1]["roles"] = json!(["opened"]);
        s["nodes"][1]["at"] = json!([400.0, 500.0]);
        s["nodes"][1]["radius"] = json!(4.0);
        // f's name, lit, over g's plain dot.
        s["nodes"][0]["roles"] = json!(["callee"]);
        assert!(passes(&check(&s, "labels-clear")));
        // f plain too: over it.
        s["nodes"][0]["roles"] = json!(["opened"]);
        assert!(!passes(&check(&s, "labels-clear")));
    }

    #[test]
    fn a_name_under_a_notice_is_not_clear() {
        let mut s = scene();
        s["labels"][1]["under"] = json!(["notice"]);
        let result = check(&s, "labels-clear");
        assert_eq!(result["offenders"][0]["under"], "notice", "{result}");
    }

    #[test]
    fn a_depth_change_that_drew_the_camera_back_may_take_a_name_away() {
        let before = scene();
        let mut now = deeper(80.0);
        now["nodes"][2]["label"] = Value::Null;
        now["nodes"][2]["label_rect"] = Value::Null;
        now["labels"].as_array_mut().unwrap().remove(2);
        let result = after(&before, &now, &[Kind::Depth], "nothing-moved");
        assert!(passes(&result), "{result}");
        // Without drawing the camera back, it may not.
        now["state"]["camera"]["distance"] = json!(100.0);
        assert!(!passes(&after(
            &before,
            &now,
            &[Kind::Depth],
            "nothing-moved"
        )));
    }

    #[test]
    fn a_frame_of_a_transition_is_not_judged_for_filling() {
        let s = journey();
        let playing = Since {
            playing: true,
            ..Since::default()
        };
        assert_eq!(
            run(&s, None, &playing)["journey-fills"]["pass"],
            Value::Null
        );
        assert_eq!(check(&s, "journey-fills")["pass"], json!(false));
    }

    #[test]
    fn a_name_under_the_open_search_box_is_left_alone() {
        let mut s = scene();
        s["labels"][1]["under"] = json!(["search box"]);
        assert!(passes(&check(&s, "named-on-screen")));
        s["labels"][1]["under"] = json!(["search box", "journey panel"]);
        assert_eq!(check(&s, "named-on-screen")["count"], 1);
    }

    #[test]
    fn a_call_off_the_journey_inside_the_expanded_module_stays_faint() {
        let mut s = journey();
        s["arrows"] = json!([
            { "from": "g", "to": "h", "to_unit": "x:b::h", "flow": "inner", "color": "#ff5cb8" },
            { "from": "f", "to": "g", "to_unit": "x:a::g", "flow": "inner", "color": "#8c94a547" },
        ]);
        let result = check(&s, "colours");
        assert_eq!(result["count"], 1, "{result}");
        assert_eq!(result["offenders"][0]["arrow"], json!(["g", "h"]));
        // Pointed at, it stays faint: hovering lights no call.
        s["nodes"][2]["roles"] = json!(["journey level 1", "cursor", "hovered"]);
        assert_eq!(check(&s, "colours")["count"], 1);
        // Out of the function set aside, too: its calls take their role's colors.
        s["nodes"][2]["roles"] = json!(["journey level 1", "cursor"]);
        s["nodes"][1]["roles"] = json!(["aside"]);
        s["nodes"][1]["rings"] = json!([{ "radius": 4.0, "color": "#ffffffff" }]);
        s["arrows"][0]["from_unit"] = json!("x:a::g");
        let result = check(&s, "colours");
        assert_eq!(result["count"], 1, "{result}");
        assert_eq!(result["offenders"][0]["arrow"], json!(["g", "h"]));
    }

    #[test]
    fn a_framed_journey_spans_the_free_area() {
        let browse = scene();
        let mut s = journey();
        // a's left edge at 300 to b's right at 1080: 780 of 1600 across, 0.49.
        let result = after(&browse, &s, &[Kind::Moves], "journey-fills");
        assert!(!passes(&result), "{result}");
        assert_eq!(result["share"], 0.49);
        s["nodes"][0]["at"] = json!([100.0, 500.0]);
        s["nodes"][2]["at"] = json!([1300.0, 500.0]);
        assert!(passes(&after(&browse, &s, &[Kind::Moves], "journey-fills")));
        // Not framed since: a click.
        assert_eq!(
            after(&s, &s, &[Kind::Click], "journey-fills")["pass"],
            Value::Null
        );
        assert!(passes(&after(&s, &s, &[Kind::Depth], "journey-fills")));
        // A sphere half under the panel.
        s["overlays"] = json!([{ "name": "journey panel", "rect": [0.0, 560.0, 1600.0, 1000.0] }]);
        let result = after(&browse, &s, &[Kind::Moves], "journey-fills");
        assert_eq!(result["offenders"][0]["module"], "a", "{result}");
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
    fn a_bar_wears_its_node_s_color_and_a_call_back_its_violet_ring() {
        let mut s = journey();
        s["nodes"][0]["color"] = json!("#ffffff");
        s["nodes"][1]["roles"] = json!(["journey level 1", "calls back"]);
        s["nodes"][1]["color"] = json!("#4db8ff");
        s["nodes"][1]["rings"] = json!([{ "radius": 6.0, "color": "#bd85ffff" }]);
        s["bars"] = json!([
            { "number": "", "name": "f", "unit": "x:a::f", "cursor": false, "color": "#ffffff" },
            { "number": null, "name": "g", "unit": "x:a::g", "caller": true, "color": "#4db8ff" },
        ]);
        assert!(passes(&check(&s, "colours")), "{}", check(&s, "colours"));
        // Its caller's bar painted violet while its dot is blue.
        s["bars"][1]["color"] = json!("#bd85ff");
        let result = check(&s, "colours");
        assert_eq!(result["offenders"][0]["name"], "g", "{result}");
        // A call back with no violet ring.
        s["bars"][1]["color"] = json!("#4db8ff");
        s["nodes"][1]["rings"] = json!([]);
        let result = check(&s, "colours");
        assert_eq!(result["offenders"][0]["function"], "g", "{result}");
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
