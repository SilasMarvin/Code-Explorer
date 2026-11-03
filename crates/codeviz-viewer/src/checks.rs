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
//!
//! They come in families sharing one geometry: what must lie in the free area
//! ([`Disc`], [`outside`]); names, near their own dot and off the rest ([`Node::gap`],
//! [`Node::overlooks`], [`Room`]); and calls ([`Arrow::pieces`]).

use std::collections::{BTreeMap, HashMap};

use bevy::{
    camera::PerspectiveProjection,
    math::{EulerRot, Quat, Vec2, Vec3},
};
use bevy_egui::egui;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    arrows,
    colors::{CALL_STRENGTH, CALLEE, CALLER, CURSOR, DEEPER, FURTHER, GROUND},
    framing,
    labels::{self, CLEAR as NAME_CLEAR, GAP as NAME_GAP, PILL_PAD as NAME_PILL},
    nucleus::VIEW,
    script::Kind,
    sizes::EXPANDED_SHARE,
    text,
};

// What the checks allow, each in one place. Points are window points, as in the PNG;
// world sizes are the scene's units. What the product sets itself (a call's width, the
// room the framing leaves) is the product's own constant, not a copy.

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
/// How far past the expanded module's band ([`EXPANDED_SHARE`]) it may show: a
/// sphere's disc seen in perspective is a little bigger than its rim points framed.
const EXPANDED_SLACK: f32 = 0.04;
/// Expanded below the band's top, how near, in points, two of its dots may show
/// for a name to fit between them, and the share of its dots that may be nearer.
const CROWDED_GAP: f32 = 18.0;
const CROWDED_SHARE: f32 = 0.25;
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
/// area, as it does at the near bound. The wheel stops at nine tenths framed by the
/// sphere's rim; its disc seen in perspective shows a little bigger.
const ZOOM_IN_MOST: f32 = 0.95;
const ZOOM_IN_KEPT: f32 = 0.88;
/// Least a function or sphere moves in the world, or a dot or name on screen in
/// points, or the camera turns in radians, to count as moved.
const MOVED_WORLD: f32 = 0.01;
const MOVED_SCREEN: f32 = 0.5;
const MOVED_ANGLE: f32 = 0.002;
/// How much nearer another dot may be to a function's name than its own, edge to
/// edge, in points, before the name reads as the other's: the placer's, and half a
/// point for where the text lands once rounded to pixels.
const NEARER: f32 = labels::NEARER + 2.0 * ROUNDING;
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
/// above its top edge (or below it, inside, where it is smaller: at most the small
/// font's size), in points.
const NAME_CENTRED: f32 = 3.0;
const NAME_ABOVE: f32 = 8.0;
const INSIDE_NAME: f32 = text::SMALL + 0.5;
/// How far a dot's radius in the snapshot may be from the placer's, rounded.
const ROUNDING: f32 = 0.25;
/// A character's width as a share of its font size, when no name drawn says, for a
/// module's name in capitals; a function's is [`text::CHAR_WIDTH`].
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
/// How bright a dot only guessed is drawn at most, against its role's color.
const GUESSED_DOT: f32 = 0.8;
/// A color this near white, in every channel (0 to 1), reads as the selection's.
const WHITE: f32 = 0.85;
/// How far into a dot, or a name's pill, a call's line may run before it counts as
/// passing through, in points: rounding, not leeway.
const ARROW_SLACK: f32 = 1.0;
/// How much longer than straight a call's line may run: the product's bound, with
/// rounding. A loop round a dot or a name is a detour, not a bend.
const ARROW_DETOUR: f32 = arrows::DETOUR + 0.05;
/// How far clear of a lit dot's ring a bent line passes, in points.
const RING_MARGIN: f32 = 1.0;
/// How far a call drawn in two snapshots may run elsewhere before it counts as moved,
/// in points: a dot growing as the cursor lands on it shortens its line by less.
const ARROW_MOVED: f32 = 3.0;
/// Offenders listed per check; `count` says how many in all.
const LISTED: usize = 12;

/// Why a check of where the view puts things leaves the view alone.
const BY_HAND: &str = "the camera is where the user's hands put it";

/// Why it leaves Back and Forward alone.
const BACK: &str = "Back or Forward put back the camera the place had, wherever that shows things";

/// A check: the snapshot, the one before it, and what happened between them.
type Check = fn(&Snap, Option<&Snap>, &Since) -> Value;

/// Every check, by the name testers read, in the order a snapshot lists them.
const CHECKS: [(&str, Check); 30] = [
    ("dots-in-spheres", |s, _, _| dots_in_spheres(s)),
    ("dots-apart", |s, _, _| dots_apart(s)),
    ("dot-sizes", |s, _, _| dot_sizes(s)),
    ("spheres-apart", |s, _, _| spheres_apart(s)),
    ("sphere-sizes", |s, _, _| sphere_sizes(s)),
    ("labels-near", |s, _, _| labels_near(s)),
    ("labels-clear", |s, _, _| labels_clear(s)),
    ("named-on-screen", |s, _, _| named_on_screen(s)),
    ("text-sizes", |s, _, _| text_sizes(s)),
    ("panel-agrees", |s, _, _| panel_agrees(s)),
    ("expanded-fill", expanded_fill),
    ("expanded-drawn", |s, _, _| expanded_drawn(s)),
    ("framed", framed),
    ("zoom-to-pointer", zoom_to_pointer),
    ("zoom-bounds", |s, _, since| zoom_bounds(s, since)),
    ("nothing-moved", nothing_moved),
    ("focus-on-screen", focus_on_screen),
    ("camera-outside", |s, _, _| camera_outside(s)),
    ("map-on-screen", |s, _, since| map_on_screen(s, since)),
    ("spheres-unclipped", |s, _, since| {
        spheres_unclipped(s, since)
    }),
    ("arrow-sizes", |s, _, _| arrow_sizes(s)),
    ("module-names", |s, _, _| module_names(s)),
    ("named-when-room", |s, _, _| named_when_room(s)),
    ("journey-fills", journey_fills),
    ("colours", |s, _, _| colours(s)),
    ("arrows-clear", |s, _, _| arrows_clear(s)),
    ("names-distinct", |s, _, _| names_distinct(s)),
    ("exits-named", |s, _, _| exits_named(s)),
    ("calls-drawn", |s, _, _| calls_drawn(s)),
    ("bars-in-view", |s, _, _| bars_in_view(s)),
];

/// Every check's name, as `expect check NAME` takes it.
pub(crate) const NAMES: [&str; CHECKS.len()] = {
    let mut names = [""; CHECKS.len()];
    let mut i = 0;
    while i < names.len() {
        names[i] = CHECKS[i].0;
        i += 1;
    }
    names
};

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
    /// The last step since that moves the view, if any: a window sized anew frames
    /// the view again as the step before it did, so it is passed over.
    fn last_move(&self) -> Option<Kind> {
        self.kinds
            .iter()
            .rev()
            .copied()
            .find(|k| !matches!(k, Kind::Still | Kind::Resize))
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
    /// The graph's part of the window that nothing lies over, where names are drawn:
    /// the graph less a docked panel.
    free_area: Option<[f32; 4]>,
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
    /// How much of the journey the panel shows, while it shows.
    panel: Option<Panel>,
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
    far_unit: Option<String>,
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
    /// In a journey, the function set aside.
    aside: Option<String>,
    /// The module expanded, by name.
    expanded: Option<String>,
    /// In a journey, the levels it shows, and how many below the start its deepest
    /// call is.
    depth: u32,
    deepest: Option<u32>,
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
    /// In the row of the start's callers.
    caller: bool,
    cursor: bool,
    color: Option<String>,
    guessed: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Panel {
    /// Bars below what the panel shows, cut off or out of view.
    below: usize,
    /// Whether a track down its side shows which rows are in view.
    rows_track: bool,
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

/// Something drawn as a disc on screen that must lie in the free area: a function's
/// dot or a module's sphere.
struct Disc<'a> {
    kind: &'static str,
    name: &'a str,
    at: [f32; 2],
    radius: f32,
    on_screen: bool,
}

/// Each disc off screen, or sticking out of `area` by more than `margin` points.
fn outside<'a>(area: egui::Rect, margin: f32, discs: impl IntoIterator<Item = Disc<'a>>) -> Found {
    discs
        .into_iter()
        .filter_map(|d| {
            let out = circle_out(area, v2(d.at), d.radius);
            (out > margin || !d.on_screen).then(|| {
                let offender = json!({ d.kind: d.name, "at": pt(d.at), "radius": r1(d.radius), "out_by": r1(out) });
                (out, offender)
            })
        })
        .collect()
}

/// Offenders, each with how bad it is: the worst are listed first.
type Found = Vec<(f32, Value)>;

/// Something drawn in two snapshots that a wheel turn is judged by: its name, where
/// it shows and how big.
type Probe = (String, [f32; 2], f32);

impl Node {
    /// Whether the dot is lit: anything but a speck, faint or named.
    fn lit(&self) -> bool {
        !self.roles.iter().all(|r| r == "speck" || r == "plain")
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

    /// One of the expanded module's functions that nothing else lights, or a speck
    /// named: a lit name may hide it.
    fn plain(&self) -> bool {
        (self.is("opened") || self.is("plain")) && !self.focus()
    }

    /// Nothing lights it: a speck, or plain.
    fn unlit(&self) -> bool {
        !self.lit() || self.plain()
    }

    /// The journey level its role gives it, if any.
    fn level(&self) -> Option<u32> {
        self.roles
            .iter()
            .find_map(|r| r.strip_prefix("journey level ")?.parse::<u32>().ok())
    }

    /// Whether `other`'s dot keeps this one's name off: any lit one, but a plain one
    /// only from a plain name.
    fn kept_off_by(&self, other: &Node) -> bool {
        other.lit() && (self.plain() || !other.plain())
    }

    /// Whether this one's name, lit, at `pill`, pays `other`'s dot no mind: one drawn
    /// faded, or one nothing lights that the opaque pill hides wholly.
    fn overlooks(&self, other: &Node, pill: egui::Rect) -> bool {
        !self.unlit()
            && (other.faded()
                || other.unlit()
                    && pill
                        .shrink(NAME_CLEAR)
                        .contains_rect(disc(other.at, other.radius - ROUNDING)))
    }

    /// How far a name's text at `r` lies from the dot's edge, the dot taken `radius`
    /// big, in points.
    fn gap(&self, r: egui::Rect, radius: f32) -> f32 {
        r.distance_to_pos(pos(self.at)) - radius
    }

    /// A dot nothing lights drawn faded beside what is lit: no lit name keeps off it.
    fn faded(&self) -> bool {
        self.unlit()
            && self
                .color
                .as_deref()
                .and_then(alpha)
                .is_some_and(|a| a < FAINT)
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
        if self.is("start") || self.is("selection") {
            0
        } else if self.is("cursor") {
            1
        } else if self.is("aside") {
            2
        } else if self.is("caller") {
            5
        } else {
            match (self.level(), self.numbers.is_empty()) {
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

    fn disc(&self) -> Disc<'_> {
        Disc {
            kind: "function",
            name: &self.name,
            at: self.at,
            radius: self.radius,
            on_screen: self.on_screen,
        }
    }
}

impl Module {
    /// Where its name comes in the order names are placed (see [`Node::rank`]): one
    /// nothing lit lives in after every function lit.
    fn rank(&self) -> u8 {
        if self.body == "faint" { 8 } else { 3 }
    }

    fn disc(&self) -> Disc<'_> {
        Disc {
            kind: "module",
            name: &self.name,
            at: self.at,
            radius: self.radius,
            on_screen: self.on_screen,
        }
    }
}

impl Arrow {
    /// Whether it shows on screen.
    fn shown(&self) -> bool {
        self.on_screen && self.drawn != Some(false)
    }

    /// The straight pieces its line runs along.
    fn pieces(&self) -> Vec<(Vec2, Vec2)> {
        self.path.windows(2).map(|p| (v2(p[0]), v2(p[1]))).collect()
    }

    /// Whether `unit` is one of its own ends.
    fn ends_at(&self, unit: Option<&str>) -> bool {
        unit.is_some() && (unit == self.from_unit.as_deref() || unit == self.to_unit.as_deref())
    }
}

impl Snap {
    fn expanded(&self) -> bool {
        self.state.nucleus == "expanded"
    }

    fn spheres(&self) -> impl Iterator<Item = &Module> {
        self.modules.iter().filter(|m| m.shown)
    }

    /// The spheres drawn with some size on screen.
    fn sized(&self) -> impl Iterator<Item = &Module> {
        self.spheres().filter(|m| m.radius > 0.0)
    }

    /// The function the journey's cursor is on, by unit.
    fn cursor(&self) -> Option<&str> {
        self.nodes
            .iter()
            .find(|n| n.is("cursor"))
            .and_then(|n| n.unit.as_deref())
    }

    /// The function set aside, drawn.
    fn aside(&self) -> Option<&Node> {
        self.nodes.iter().find(|n| n.is("aside"))
    }

    /// The module holding the function set aside.
    fn aside_module(&self) -> Option<&str> {
        self.aside().and_then(|n| n.module.as_deref())
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

    /// The module of the function drawn for a unit.
    fn module_of(&self, unit: Option<&str>) -> Option<&str> {
        self.node(unit?)?.module.as_deref()
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

    /// The graph's part of the window that nothing lies over, as the view laid its
    /// names out in it; in a snapshot from before it was recorded, the graph.
    fn free(&self) -> egui::Rect {
        rect(self.free_area.unwrap_or(self.graph_area))
    }

    /// How much of the free area's smaller side a sphere's diameter spans.
    fn fill(&self, m: &Module) -> f32 {
        let free = self.free();
        2.0 * m.radius / free.width().min(free.height())
    }
}

/// Runs every check on the snapshot `now`, given the snapshot before it, if any, and
/// what happened between them.
pub(crate) fn run(now: &Value, before: Option<&Value>, since: &Since) -> Value {
    let read = |v: &Value| serde_json::from_value::<Snap>(v.clone());
    let results: Map<String, Value> = match (read(now), before.map(read).transpose()) {
        (Ok(now), Ok(before)) => CHECKS
            .iter()
            .map(|(name, check)| ((*name).to_owned(), check(&now, before.as_ref(), since)))
            .collect(),
        (Err(error), _) | (_, Err(error)) => {
            let why = format!("the snapshot's JSON could not be read: {error}");
            NAMES
                .iter()
                .map(|&name| (name.to_owned(), fail(&why)))
                .collect()
        }
    };
    results.into()
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

// What any one picture shows: dots, spheres and their sizes.

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
                let offender = json!({
                    "function": n.name, "module": m.name, "at": pt(n.at),
                    "out_by": r3(out), "sphere_radius": r3(m.world_radius),
                });
                (out / m.world_radius.max(f32::EPSILON), offender)
            })
        })
        .collect();
    verdict(offenders)
}

fn dots_apart(s: &Snap) -> Value {
    // A faint dot with no name is nothing a reader tells apart.
    let dots: Vec<&Node> = s
        .nodes
        .iter()
        .filter(|n| n.on_screen && n.radius > 0.0 && !n.faint())
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
            Some((off, json!({ "function": n.name, "roles": n.roles, "radius": r1(n.radius), "allowed": bound })))
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
            let share = overlap / a.world_radius.min(b.world_radius).max(f32::EPSILON);
            if overlap > SPHERE_OVERLAP {
                let offender = json!({ "modules": [a.name, b.name], "overlap": r3(overlap), "share_of_smaller": r2(share) });
                offenders.push((share, offender));
            }
        }
    }
    verdict(offenders)
}

fn sphere_sizes(s: &Snap) -> Value {
    // Expanded, the module in the middle is meant to be bigger than the rest.
    let sized: Vec<&Module> = s
        .spheres()
        .filter(|m| !(s.expanded() && m.body == "nucleus") && m.world_radius > 0.0)
        .collect();
    let by_size = |a: &&&Module, b: &&&Module| a.world_radius.total_cmp(&b.world_radius);
    let (Some(small), Some(big)) = (sized.iter().min_by(by_size), sized.iter().max_by(by_size))
    else {
        return skip("no module spheres drawn");
    };
    let ratio = big.world_radius / small.world_radius;
    let mut offenders = Vec::new();
    if ratio > SPHERE_RATIO {
        let offender = json!({ "largest": big.name, "smallest": small.name, "ratio": r2(ratio), "most": SPHERE_RATIO });
        offenders.push((ratio, offender));
    }
    // Bigger with more functions, but less than in proportion.
    for a in &sized {
        for b in sized
            .iter()
            .filter(|b| a.functions > b.functions && b.functions > 0)
        {
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

fn text_sizes(s: &Snap) -> Value {
    let mut offenders = Vec::new();
    let (least, most) = NAME_SIZES;
    for label in s.labels.iter().filter(|l| l.names == "function") {
        let off = (least - label.size).max(label.size - most);
        if off > 0.01 {
            let offender =
                json!({ "label": label.text, "size": label.size, "allowed": [least, most] });
            offenders.push((off, offender));
        }
    }
    // Each size seen, to the half point, with a text drawn at it.
    let mut sizes: BTreeMap<i32, &str> = BTreeMap::new();
    let texts = s.labels.iter().map(|l| (l.size, l.text.as_str()));
    let texts = texts.chain(s.text.iter().map(|t| (t.size, t.text.as_str())));
    for (size, text) in texts.filter(|(size, _)| *size > 0.0) {
        #[expect(clippy::cast_possible_truncation, reason = "a font size")]
        sizes.entry((size * 2.0).round() as i32).or_insert(text);
    }
    if sizes.len() > MOST_TEXT_SIZES {
        let seen: Vec<Value> = sizes
            .iter()
            .map(|(&half, text)| json!({ "size": f64::from(half) / 2.0, "e.g.": text }))
            .collect();
        let offender =
            json!({ "distinct_sizes": sizes.len(), "most": MOST_TEXT_SIZES, "sizes": seen });
        offenders.push((f32::MAX, offender));
    }
    let seen: Vec<f64> = sizes.keys().map(|&h| f64::from(h) / 2.0).collect();
    with(verdict(offenders), "sizes", json!(seen))
}

// The journey panel against the graph.

fn panel_agrees(s: &Snap) -> Value {
    if s.bars.is_empty() {
        return skip("no journey bars drawn");
    }
    let mut offenders = Vec::new();
    // The depth the panel reads is one the journey has: from 1 to its deepest call.
    if let Some(deepest) = s.state.deepest
        && !(1..=deepest.max(1)).contains(&s.state.depth)
    {
        let why = "the depth shown is not one of the journey's levels";
        let offender = json!({ "depth": s.state.depth, "deepest": deepest, "why": why });
        offenders.push((3.0, offender));
    }
    for bar in &s.bars {
        let drawn = bar.unit.as_deref().and_then(|u| s.node(u));
        // Each numbered bar has its call on the graph: a call only guessed among
        // several functions, with no one function of its own, on each of them. The
        // start's bar carries no number.
        if let Some(number) = bar.number.as_ref().filter(|n| !n.is_empty()) {
            if !s.nodes.iter().any(|n| n.numbers.contains(number)) {
                let why = "numbered in the panel, but no function on the graph carries the number";
                offenders.push((2.0, json!({ "bar": number, "name": bar.name, "why": why })));
            }
            if bar.unit.is_some() {
                if let Some(n) = drawn.filter(|n| !n.numbers.contains(number)) {
                    let offender =
                        json!({ "bar": number, "name": bar.name, "node_numbers": n.numbers });
                    offenders.push((1.0, offender));
                }
                for n in s
                    .nodes
                    .iter()
                    .filter(|n| n.numbers.contains(number) && n.unit != bar.unit)
                {
                    let offender =
                        json!({ "bar": number, "name": bar.name, "but_on_the_graph": n.name });
                    offenders.push((1.0, offender));
                }
            }
        }
        // The same name on the bar as on the graph.
        if let Some(n) = drawn.filter(|n| !same_name(&bar.name, n)) {
            let says = n.label.as_deref().unwrap_or(&n.name);
            let offender = json!({ "bar": bar.number, "bar_says": bar.name, "graph_says": says });
            offenders.push((0.5, offender));
        }
    }
    let marked: Vec<&String> = s
        .nodes
        .iter()
        .filter(|n| n.is("cursor"))
        .map(|n| &n.name)
        .collect();
    let cursor_bars: Vec<&Bar> = s.bars.iter().filter(|b| b.cursor).collect();
    if !marked.is_empty() && cursor_bars.len() != 1 {
        let offender = json!({
            "why": "the graph has a cursor, but the panel shows not one cursor bar",
            "cursor_on_the_graph": marked,
            "cursor_bars": cursor_bars.iter().map(|b| &b.name).collect::<Vec<_>>(),
        });
        offenders.push((3.0, offender));
    }
    if let [bar] = cursor_bars.as_slice() {
        let drawn = bar.unit.as_deref().and_then(|u| s.node(u));
        let agrees = s
            .nodes
            .iter()
            .filter(|n| n.is("cursor"))
            .all(|n| n.unit == bar.unit)
            && drawn.is_none_or(|n| n.is("cursor"));
        if !agrees {
            let offender = json!({ "cursor_bar": bar.number, "name": bar.name, "cursor_on_the_graph": marked });
            offenders.push((2.0, offender));
        }
    }
    verdict(offenders)
}

/// Bars the panel cannot show for its height, below what it shows, are said to be
/// there: a track down its side shows which rows are in view.
fn bars_in_view(s: &Snap) -> Value {
    let Some(panel) = &s.panel else {
        return skip("no journey panel");
    };
    if panel.below == 0 {
        return skip("every bar the panel shows is in view down to its foot");
    }
    let mut offenders = Vec::new();
    if !panel.rows_track {
        let why = "bars lie below the panel's foot, and nothing says so";
        offenders.push((1.0, json!({ "below": panel.below, "why": why })));
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

// In view: what must lie in the free area, after the steps that put it there. Each
// judges discs against the free area ([`outside`]); they differ in what must be
// inside and after which steps, and all leave a camera the user's hands put alone.

fn expanded_fill(s: &Snap, before: Option<&Snap>, since: &Since) -> Value {
    // Expanded in a journey, the cursor or the function set aside takes the expansion
    // to its module; browsing, a pick takes it to the selection's.
    let followed = before.is_some_and(|b| b.expanded() && b.state.expanded != s.state.expanded);
    if !since.has(Kind::Expand) && !followed {
        return skip(
            "no expand (x, double click), nor another module expanded, since the last snapshot",
        );
    }
    if !s.expanded() {
        return skip("collapsed");
    }
    if since.by_hand {
        return skip(BY_HAND);
    }
    let Some(m) = s.expanded_sphere() else {
        return fail("expanded, but no module drawn expanded");
    };
    let free = s.free();
    let fill = s.fill(m);
    let (least, most) = EXPANDED_SHARE;
    let mut offenders = outside(free, EDGE, [m.disc()]);
    let about = |why: &str| {
        json!({
            "module": m.name, "fill": r2(fill), "band": [least, most], "functions": m.functions,
            "diameter": r1(2.0 * m.radius), "free_area": [r1(free.width()), r1(free.height())], "why": why,
        })
    };
    if fill < least - EXPANDED_SLACK || fill > most + EXPANDED_SLACK {
        let off = (fill - fill.clamp(least, most)).abs();
        offenders.push((off, about("outside the band")));
    }
    // Below the band's top it has room to grow: its dots must have room for names.
    let dots: Vec<Vec2> = s
        .nodes
        .iter()
        .filter(|n| n.module.as_ref() == Some(&m.name))
        .map(|n| v2(n.at))
        .collect();
    let crowded = dots
        .iter()
        .enumerate()
        .filter(|&(i, &at)| {
            (dots.iter().enumerate()).any(|(j, &other)| i != j && at.distance(other) < CROWDED_GAP)
        })
        .count();
    #[expect(clippy::cast_precision_loss, reason = "dot counts")]
    let crowded_share = crowded as f32 / dots.len().max(1) as f32;
    if fill < most - EXPANDED_SLACK && crowded_share > CROWDED_SHARE {
        let why = format!(
            "its dots crowd ({crowded} of {} nearer than {CROWDED_GAP} points) though it could grow",
            dots.len()
        );
        offenders.push((crowded_share, about(&why)));
    }
    // An expand from the collapsed view zooms in.
    if since.has(Kind::Expand)
        && let Some(b) = before.filter(|b| !b.expanded())
        && let Some(was) = b.spheres().find(|w| w.name == m.name)
    {
        let was_fill = b.fill(was);
        if fill <= was_fill * 1.05 && was_fill < most - EXPANDED_SLACK {
            let why = "the expand did not zoom in";
            let offender = json!({ "why": why, "module": m.name, "fill_before": r2(was_fill), "fill": r2(fill) });
            offenders.push((was_fill - fill + 1.0, offender));
        }
    }
    with(verdict(offenders), "fill", json!(r2(fill)))
}

/// Expanded, the module expanded is drawn, and some of its functions show in the
/// free area: an expansion never leaves a blank graph.
fn expanded_drawn(s: &Snap) -> Value {
    if !s.expanded() {
        return skip("collapsed");
    }
    let Some(name) = &s.state.expanded else {
        return fail("expanded, but the state names no module");
    };
    let Some(m) = s.modules.iter().find(|m| &m.name == name) else {
        return fail(&format!("expanded {name} is no module drawn"));
    };
    let free = s.free();
    let dots = s
        .nodes
        .iter()
        .filter(|n| n.module.as_ref() == Some(name) && n.on_screen && free.contains(pos(n.at)))
        .count();
    if !m.shown || matches!(m.body.as_str(), "closed" | "faint") || dots == 0 {
        let offender = json!({
            "why": "the module expanded is not drawn, or none of its functions shows",
            "module": m.name, "body": m.body, "shown": m.shown, "dots_in_view": dots,
            "at": pt(m.at), "radius": r1(m.radius),
        });
        return verdict(vec![(1.0, offender)]);
    }
    with(verdict(Vec::new()), "dots_in_view", json!(dots))
}

fn framed(s: &Snap, before: Option<&Snap>, since: &Since) -> Value {
    // A window sized anew with the camera where the hands put it leaves it there.
    if since.by_hand && since.has(Kind::Resize) && since.last_move().is_none() {
        return verdict(before.map_or_else(Vec::new, |b| hands_kept(b, s)));
    }
    let free = s.free();
    let mut offenders = Vec::new();
    match since.last_move() {
        Some(Kind::FrameAll) => {
            offenders.extend(outside(free, EDGE, s.spheres().map(Module::disc)));
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
        Some(Kind::FrameFocus) if s.expanded() => {
            offenders.extend(outside(free, EDGE, s.expanded_sphere().map(Module::disc)));
        }
        Some(Kind::FrameFocus) => {
            let lit: Vec<&Node> = s.nodes.iter().filter(|n| n.lit()).collect();
            if lit.is_empty() {
                return skip("nothing lit for F to frame");
            }
            offenders.extend(outside(
                free.shrink(FRAME_MARGIN),
                0.0,
                lit.into_iter().map(Node::disc),
            ));
        }
        _ => {
            return skip(
                "the last step since the snapshot before that moves the view is not F or R",
            );
        }
    }
    verdict(offenders)
}

/// The camera moved off what it looked at, or nearer or further, since `before`.
fn hands_kept(before: &Snap, now: &Snap) -> Found {
    let (Some(a), Some(b)) = (before.state.camera, now.state.camera) else {
        return Vec::new();
    };
    let by = Vec3::from_array(a.target).distance(Vec3::from_array(b.target));
    let further = b.distance - a.distance;
    if by <= MOVED_WORLD && further.abs() <= MOVED_WORLD {
        return Vec::new();
    }
    let why = "a window sized anew moved a camera the hands had put";
    let offender = json!({ "target_by": r3(by), "distance_by": r3(further), "why": why });
    vec![(by + further.abs(), offender)]
}

fn zoom_to_pointer(s: &Snap, before: Option<&Snap>, since: &Since) -> Value {
    let turns = since.zooms();
    if turns == 0 {
        return skip("no wheel turn since the last snapshot");
    }
    let others = (since.kinds.iter()).any(|k| !matches!(k, Kind::Zoom { .. } | Kind::Still));
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
    let spheres = |snap: &Snap, on_screen: bool| -> Vec<Probe> {
        snap.sized()
            .filter(|m| m.on_screen || !on_screen)
            .map(|m| (m.name.clone(), m.at, m.radius))
            .collect()
    };
    let dots = |snap: &Snap| -> Vec<Probe> {
        (snap.nodes.iter().filter(|n| n.radius > 0.0))
            .filter_map(|n| Some((n.unit.clone()?, n.at, n.radius)))
            .collect()
    };
    let from_pointer = |at: [f32; 2]| v2(at).distance(pointer);
    let probe = |was: Vec<Probe>, now: Vec<Probe>| {
        now.into_iter()
            .filter_map(|(name, at, radius)| {
                let then = was.iter().find(|w| w.0 == name)?;
                Some((name, at, radius, then.1, then.2))
            })
            .min_by(|a, b| from_pointer(a.3).total_cmp(&from_pointer(b.3)))
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
    let Some(extent) = map_extent(s) else {
        return skip("no module spheres drawn");
    };
    let free = s.free();
    let mut offenders = Vec::new();
    // Zoomed in, the sphere under the pointer, nearest the camera (biggest on screen).
    let under = s.state.pointer.and_then(|p| {
        s.spheres()
            .filter(|m| v2(m.at).distance(v2(p)) <= m.radius)
            .max_by(|a, b| a.radius.total_cmp(&b.radius))
    });
    if since.has(Kind::Zoom { out: false })
        && let Some(m) = under
    {
        let fill = s.fill(m);
        if fill > ZOOM_IN_MOST {
            let why = "zoomed in past the sphere under the pointer filling the free area";
            let offender =
                json!({ "why": why, "module": m.name, "fill": r2(fill), "most": ZOOM_IN_MOST });
            offenders.push((fill, offender));
        } else if fill >= ZOOM_IN_KEPT {
            let why = "zoomed in to the near bound, the sphere under the pointer is not all in the free area";
            offenders.extend(
                outside(free, EDGE, [m.disc()])
                    .into_iter()
                    .map(|(out, o)| (out, with(o, "why", json!(why)))),
            );
        }
    }
    let discs: Vec<egui::Rect> = s.sized().map(|m| disc(m.at, m.radius)).collect();
    if !discs.iter().any(|d| d.intersects(free)) {
        let offender = json!({ "why": "nothing of the map is on screen" });
        offenders.push((f32::MAX, offender));
    } else if discs.iter().all(|d| free.contains_rect(*d)) {
        let span = extent.width().max(extent.height());
        let least = ZOOM_OUT_LEAST * free.width().min(free.height());
        if span < least {
            let offender = json!({ "why": "zoomed out past the whole map", "map_spans": r1(span), "least": r1(least) });
            offenders.push((least - span, offender));
        }
    }
    verdict(offenders)
}

fn focus_on_screen(s: &Snap, before: Option<&Snap>, since: &Since) -> Value {
    // An expand or a shrink back picks nothing: browsing, the selection stays.
    let only_expands = (since.kinds.iter()).all(|k| matches!(k, Kind::Still | Kind::Expand));
    let browsed = before.filter(|b| {
        since.has(Kind::Expand)
            && only_expands
            && b.state.mode == "browse"
            && s.state.mode == "browse"
    });
    if let Some(b) = browsed
        && b.state.selection != s.state.selection
    {
        let why = "an expand or shrink back changed the selection";
        let offender = json!({ "why": why, "was": b.state.selection, "now": s.state.selection });
        return verdict(vec![(1.0, offender)]);
    }
    // Wherever the hands put the camera, a cursor that moves expanded in a journey is
    // kept in view.
    let cursor_only = since.by_hand && s.cursor_moved_expanded(before);
    if since.by_hand && !cursor_only {
        return skip(BY_HAND);
    }
    if since.has(Kind::Expand) && s.expanded() {
        // What an expand must keep, the selection, it kept.
        if browsed.is_some() {
            return verdict(Vec::new());
        }
        return skip("an expand frames the module, wherever the focus is");
    }
    if since.has(Kind::Back) {
        return skip(BACK);
    }
    // A function picked browsing since (a click, a search, Back), or set aside in a
    // journey, is in view wherever it is: the expansion follows it.
    let picked = before.is_some_and(|b| {
        (s.state.mode == "browse"
            && s.state.selection.is_some()
            && b.state.selection != s.state.selection)
            || (s.state.aside.is_some() && b.state.aside != s.state.aside)
    });
    if picked && !s.nodes.iter().any(|n| n.is("selection") || n.is("aside")) {
        return fail("the function picked is selected but not drawn");
    }
    // Expanded, the view is on the module expanded: what is in focus in another may
    // lie off it.
    let expanded = s.state.expanded.as_ref().filter(|_| s.expanded());
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
        .filter(|n| {
            (picked && (n.is("selection") || n.is("aside")))
                || expanded.is_none_or(|m| n.module.as_ref() == Some(m))
        })
        .collect();
    if focus.is_empty() {
        return skip("no selection, start or cursor drawn, in the module expanded if one is");
    }
    verdict(outside(s.free(), EDGE, focus.into_iter().map(Node::disc)))
}

fn map_on_screen(s: &Snap, since: &Since) -> Value {
    let out = (since.kinds.iter())
        .any(|k| matches!(k, Kind::Zoom { out: true } | Kind::FrameAll | Kind::Pan));
    if !out {
        return skip("no wheel turn out, pan or R since the last snapshot");
    }
    let Some(extent) = map_extent(s) else {
        return skip("no module spheres drawn");
    };
    let free = s.free();
    let map = rect_json(corners(extent));
    let middle = pt([free.center().x, free.center().y]);
    let mut offenders = Vec::new();
    if extent.width() > free.width() || extent.height() > free.height() {
        // Dragged, a map bigger than the free area keeps over its middle.
        if !since.has(Kind::Pan) {
            return skip("the map is bigger than the free area at this zoom");
        }
        if !extent.contains(free.center()) {
            let why = "dragged off the middle of the free area";
            offenders.push((1.0, json!({ "why": why, "map": map, "middle": middle })));
        }
        return verdict(offenders);
    }
    let area = |m: &&Module| m.radius * m.radius;
    let all: f32 = s.sized().map(|m| area(&m)).sum();
    let out: Vec<&Module> = s
        .sized()
        .filter(|m| circle_out(free, v2(m.at), m.radius) > EDGE)
        .collect();
    let share = 1.0 - out.iter().map(area).sum::<f32>() / all.max(f32::EPSILON);
    // Dragged, a map that fits stays in the free area whole.
    let least = if since.has(Kind::Pan) { 1.0 } else { MAP_SHOWN };
    // Zoomed all the way out, the map is held over the free area's middle, however
    // it was dragged.
    if s.state.camera.is_some_and(|c| c.farthest) && !extent.contains(free.center()) {
        let why = "zoomed all the way out, the map is off the middle of the free area";
        let offender = json!({ "why": why, "map": map, "middle": middle });
        offenders.push((f32::MAX, offender));
    }
    if share < least {
        let offender = json!({
            "why": "the whole map would fit in the free area, but not this much of it is there",
            "inside": r2(share), "least": least, "map": map,
            "outside": out.iter().map(|m| &m.name).collect::<Vec<_>>(),
        });
        offenders.push((least - share, offender));
    }
    with(verdict(offenders), "inside", json!(r2(share)))
}

/// Where every sphere shown lies on screen, together.
fn map_extent(s: &Snap) -> Option<egui::Rect> {
    s.sized()
        .map(|m| disc(m.at, m.radius))
        .reduce(egui::Rect::union)
}

fn spheres_unclipped(s: &Snap, since: &Since) -> Value {
    let graph = rect(s.graph_area);
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(s.window[0], s.window[1]));
    if !graph.is_positive() {
        return skip("no graph area");
    }
    // The top of a panel docked along the graph's foot, where the view framed itself,
    // expanded too: what it cuts shows half under the panel.
    let free = s.free();
    let panel = (free.max.y < graph.max.y - EDGE && !since.by_hand).then_some(free.max.y);
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
    let aside = s.aside_module();
    let mut offenders = Vec::new();
    for m in s.sized() {
        // Zoomed past it, the camera inside, it shows as no disc to cut.
        if eye.is_some_and(|eye| eye.distance(Vec3::from_array(m.world)) < m.world_radius) {
            continue;
        }
        let faint = m.body == "faint" && aside != Some(m.name.as_str());
        let c = v2(m.at);
        let across = c.x + m.radius > graph.min.x && c.x - m.radius < graph.max.x;
        let down = c.y + m.radius > graph.min.y && c.y - m.radius < graph.max.y;
        let cut = [
            (
                bottom && across && (c.y - graph.max.y).abs() < m.radius,
                "the graph's bottom edge, in open window",
            ),
            (
                panel.is_some_and(|y| !faint && across && (c.y - y).abs() < m.radius),
                "the top of the panel",
            ),
            (
                left && down && (c.x - graph.min.x).abs() < m.radius,
                "the graph's left edge, in open window",
            ),
            (
                right && down && (c.x - graph.max.x).abs() < m.radius,
                "the graph's right edge, in open window",
            ),
        ];
        for (_, by) in cut.into_iter().filter(|(cut, _)| *cut) {
            let offender = json!({
                "module": m.name, "cut_by": by,
                "at": pt(m.at), "radius": r1(m.radius), "graph_area": rect_json(s.graph_area),
            });
            offenders.push((m.radius, offender));
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
        return skip(BY_HAND);
    }
    if since.playing {
        return skip("a frame of a transition still playing");
    }
    if since.has(Kind::Back) {
        return skip(BACK);
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
    // The spheres holding what the journey lights, and the function set aside; a
    // faint one holds none of it, but for the one set aside.
    let aside = s.aside_module();
    let spheres: Vec<&Module> = s
        .sized()
        .filter(|m| m.body != "faint" || aside == Some(m.name.as_str()))
        .collect();
    let span = lit
        .iter()
        .flat_map(|n| std::iter::once(disc(n.at, n.radius)).chain(n.label_rect.map(rect)))
        .chain(spheres.iter().map(|m| disc(m.at, m.radius)))
        .reduce(egui::Rect::union)
        .unwrap_or(egui::Rect::NOTHING);
    let share = (span.width() / free.width()).max(span.height() / free.height());
    let mut offenders = outside(free, EDGE, spheres.into_iter().map(Module::disc));
    offenders.extend(outside(free, EDGE, lit.into_iter().map(Node::disc)));
    if share < JOURNEY_FILL {
        let offender = json!({
            "why": "the journey's spheres, functions and names take too little of the free area",
            "share": r2(share), "least": JOURNEY_FILL, "span": [r1(span.width()), r1(span.height())],
            "free_area": [r1(free.width()), r1(free.height())],
        });
        offenders.push((JOURNEY_FILL - share, offender));
    }
    with(verdict(offenders), "share", json!(r2(share)))
}

// What moved since the snapshot before.

fn nothing_moved(s: &Snap, before: Option<&Snap>, since: &Since) -> Value {
    let Some(before) = before else {
        return skip("no snapshot before");
    };
    if since.kinds.is_empty() {
        return skip("no step since the last snapshot");
    }
    // Expanding or shrinking back in a journey moves the camera, never the journey.
    let journey = s.state.mode == "journey" && before.state.mode == "journey";
    let only = |kinds: &[Kind]| since.kinds.iter().all(|k| kinds.contains(k));
    if journey && since.has(Kind::Expand) && only(&[Kind::Still, Kind::Expand]) {
        return verdict(journey_kept(before, s));
    }
    if let Some(kind) =
        (since.kinds.iter()).find(|k| !matches!(k, Kind::Still | Kind::Click | Kind::Depth))
    {
        return skip(&format!(
            "a step since the last snapshot may move the view ({kind:?})"
        ));
    }
    let expanded_journey = journey && s.expanded();
    if expanded_journey && before.state.expanded != s.state.expanded {
        return skip("the cursor or the function set aside took the expansion to another module");
    }
    // A depth change may draw the camera back to show what it brought in, when that
    // would be off the free area, and no more. Expanded in a journey, a click may
    // re-centre on what it clicks, and a cursor that moves may pan to keep it in
    // view: the camera has leave to move there, nothing else has.
    let depth = since.has(Kind::Depth);
    let recentres =
        expanded_journey && (since.has(Kind::Click) || s.cursor_moved_expanded(Some(before)));
    let (summary, mut offenders) = compare(before, s, depth || recentres);
    // A click that selects another function lights others, named first: a lit
    // function's name whose side they take tries its others rather than go, and a
    // module's name its spot inside its sphere.
    let selected = before.state.selection != s.state.selection;
    if selected {
        offenders.retain(|(_, o)| {
            o["label"].is_null()
                || o["unit"]
                    .as_str()
                    .and_then(|u| s.node(u))
                    .is_some_and(Node::unlit)
        });
    }
    if depth {
        offenders.extend(depth_camera(before, s));
    }
    // A click that selects another function lights others, and names them instead;
    // a camera that drew back or panned shows things elsewhere, and names what fits
    // now; any other step here only reads, and takes no name away.
    let camera_moved = summary["camera"]["moved"] == json!(true);
    if !selected && !((depth || recentres) && camera_moved) {
        offenders.extend(gone_labels(before, s));
    }
    verdict(offenders)
}

/// Each of the journey's dots drawn before and now that moved in the world.
fn journey_kept(before: &Snap, now: &Snap) -> Found {
    now.nodes
        .iter()
        .filter(|n| n.in_journey())
        .filter_map(|n| {
            let was = before.nodes.iter().find(|b| b.unit == n.unit)?;
            let by = Vec3::from_array(n.world?).distance(Vec3::from_array(was.world?));
            let why = "expanding in a journey moves the camera, not the journey's dots";
            (by > MOVED_WORLD).then(|| {
                (
                    by,
                    json!({ "function": n.name, "moved_in_world": r3(by), "why": why }),
                )
            })
        })
        .collect()
}

/// How the camera moved on a depth change, when it should not have: anything but
/// drawing back (and framing the journey afresh as it does), or drawing back when
/// all that came in would have shown anyway.
fn depth_camera(before: &Snap, now: &Snap) -> Found {
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
    let old: Vec<&str> = (before.nodes.iter())
        .filter_map(|n| n.unit.as_deref())
        .collect();
    let came_in = |n: &&Node| n.unit.as_deref().is_some_and(|u| !old.contains(&u));
    let graph = rect(now.graph_area);
    // The free area as the depth change left it: a taller panel takes more of it.
    let free = now.free();
    let whole = free.shrink(framing::DOT_ROOM);
    // What came in, or anything of the journey's, as the camera from before would
    // have shown it: off the free area, or too near its edge to show whole, draws
    // the camera back.
    let off = (now.nodes.iter())
        .filter(|n| n.in_journey() || came_in(n))
        .any(|n| {
            n.world.is_some_and(|world| {
                shown_from(a, graph, Vec3::from_array(world))
                    .is_none_or(|was| !whole.contains(egui::pos2(was.x, was.y)))
            })
        });
    // A sphere that came in, or any while the panel has grown, cut by the free area
    // as the camera from before would have shown it, or with no room above it for
    // its name, draws the camera back too.
    let grown = free != before.free();
    let named =
        egui::Rect::from_min_max(free.min + egui::vec2(0.0, framing::NAME_ROOM.y), free.max);
    let right = Quat::from_euler(EulerRot::YXZ, a.yaw, a.pitch, 0.0) * Vec3::X;
    let cut = (now.spheres())
        .filter(|m| grown || !before.spheres().any(|b| b.name == m.name))
        .any(|m| {
            let world = Vec3::from_array(m.world);
            let rim = world + right * m.world_radius;
            match (shown_from(a, graph, world), shown_from(a, graph, rim)) {
                (Some(at), Some(rim)) => {
                    let radius = at.distance(rim);
                    circle_out(whole, at, radius) > 0.0 || circle_out(named, at, radius) > 0.0
                }
                _ => true,
            }
        });
    if off || cut {
        return Vec::new();
    }
    let came = now.nodes.iter().filter(came_in).count();
    let why = if came == 0 {
        "the camera drew back, but nothing came in"
    } else {
        "the camera drew back, but what came in would have shown from where it was"
    };
    vec![(
        f32::MAX,
        json!({ "camera": camera, "why": why, "came_in": came }),
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
fn gone_labels(before: &Snap, now: &Snap) -> Found {
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
            // A name shown only under the pointer goes with it.
            if kind == "function" {
                !before.node(of).is_some_and(|n| n.is("hovered"))
                    && now.node(of).is_some_and(|n| n.on_screen)
            } else {
                now.spheres().any(|m| &m.name == of && m.on_screen)
            }
        })
        .map(|(_, l)| {
            (
                MOVED_SCREEN,
                json!({ "label": l.text, "gone": true, "was_at": rect_json(l.rect) }),
            )
        })
        .collect()
}

/// What moved between two snapshots, and, as offenders, everything that did: when
/// `camera_free`, only what moved in the world, the camera having had leave to move.
fn compare(before: &Snap, now: &Snap, camera_free: bool) -> (Value, Found) {
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

    let was: HashMap<&str, &Node> = (before.nodes.iter())
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
    // A click that selects another function lights other calls, which keep off
    // other dots.
    let (arrows, moved_arrows) = arrows_moved(before, now);
    if screen_counts {
        offenders.extend(moved_labels.iter().cloned());
        if before.state.selection == now.state.selection {
            offenders.extend(moved_arrows.iter().cloned());
        }
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
        "arrows": { "compared": arrows, "moved": moved_arrows.len() },
        "appeared": new.iter().filter(|u| !old.contains(u)).count(),
        "gone": old.iter().filter(|u| !new.contains(u)).count(),
    });
    (summary, offenders)
}

/// How many calls are drawn in both snapshots, and those whose line runs elsewhere,
/// with how far.
fn arrows_moved(before: &Snap, now: &Snap) -> (usize, Found) {
    let key = |a: &Arrow| (a.from_unit.clone(), a.to_unit.clone(), a.flow.clone());
    let lines = |a: &&Arrow| a.path.len() >= 2;
    let was: HashMap<_, &Arrow> = before
        .arrows
        .iter()
        .filter(lines)
        .map(|a| (key(a), a))
        .collect();
    let both: Vec<(&Arrow, &Arrow)> = (now.arrows.iter().filter(lines))
        .filter_map(|a| Some((a, *was.get(&key(a))?)))
        .collect();
    let moved = both
        .iter()
        .filter_map(|(a, then)| {
            let by = path_gap(&a.path, &then.path).max(path_gap(&then.path, &a.path));
            (by > ARROW_MOVED).then(|| (by, json!({ "arrow": [a.from, a.to], "by": r1(by) })))
        })
        .collect();
    (both.len(), moved)
}

/// How far the point of `a` farthest from the line through `b` is from it.
fn path_gap(a: &[[f32; 2]], b: &[[f32; 2]]) -> f32 {
    a.iter()
        .map(|&p| {
            b.windows(2)
                .map(|q| piece_distance(v2(q[0]), v2(q[1]), v2(p)))
                .fold(f32::INFINITY, f32::min)
        })
        .fold(0.0, f32::max)
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
fn labels_moved(before: &Snap, now: &Snap) -> (usize, Found) {
    let named: HashMap<(String, Option<String>), &Label> = (before.labels.iter())
        .filter_map(|l| Some((label_key(l)?, l)))
        .collect();
    let both: Vec<(&Label, &Label)> = (now.labels.iter())
        .filter_map(|l| Some((l, *named.get(&label_key(l)?)?)))
        .collect();
    let moved = both
        .iter()
        .filter_map(|(label, then)| {
            let by = label_shift(rect(then.rect), rect(label.rect));
            (by > MOVED_SCREEN).then(|| {
                let offender = json!({
                    "label": label.text, "unit": label.unit, "from": rect_json(then.rect),
                    "to": rect_json(label.rect), "by": r1(by),
                });
                (by, offender)
            })
        })
        .collect();
    (both.len(), moved)
}

// Names: each by its own dot or sphere, clear of the rest, on screen, and shown
// wherever there is room. They share how far a name's text lies from a dot
// ([`Node::gap`]), which dots a lit name pays no mind ([`Node::overlooks`]), and
// where a name would fit ([`Room`]).

fn labels_near(s: &Snap) -> Value {
    let mut offenders = Vec::new();
    // The start, the cursor, the selection and the function set aside are named come
    // what may: one with room nowhere round its dot lies where it covers least.
    let room = std::cell::OnceCell::new();
    let crowded =
        |n: &Node| n.focus() && room.get_or_init(|| Room::new(s)).function_spot(n).is_none();
    let drawn: Vec<&Node> = s.nodes.iter().filter(|n| n.on_screen).collect();
    for n in &s.nodes {
        let (Some(label), Some(r)) = (&n.label, n.label_rect.map(rect)) else {
            continue;
        };
        let mine = n.gap(r, n.radius).max(0.0);
        // The dot nearest the name, any but its own and those it pays no mind.
        let pill = r.expand2(NAME_PILL);
        let nearest = (drawn.iter())
            .filter(|other| other.unit != n.unit && !n.overlooks(other, pill))
            .map(|other| (other, other.gap(r, other.radius).max(0.0)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if !n.on_screen {
            let why = "its dot is off screen";
            let offender = json!({ "function": n.name, "label": label, "why": why });
            offenders.push((f32::MAX, offender));
        } else if mine > LABEL_REACH {
            let offender =
                json!({ "function": n.name, "label": label, "gap": r1(mine), "most": LABEL_REACH });
            offenders.push((mine, offender));
        } else if let Some((other, theirs)) = nearest
            && theirs + NEARER < mine
            && !crowded(n)
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

/// Whether a label names a function or a module.
fn naming(l: &Label) -> bool {
    matches!(l.names.as_str(), "function" | "module")
}

fn labels_clear(s: &Snap) -> Value {
    let mut offenders = Vec::new();
    for (i, label) in s.labels.iter().enumerate().filter(|(_, l)| naming(l)) {
        // A notice is said over nothing named.
        if label.under.iter().any(|o| o == "notice") {
            let offender = json!({ "label": label.text, "under": "notice" });
            offenders.push((rect(label.rect).area(), offender));
        }
        // Each pair of names once.
        for (j, other) in (label.overlaps.iter()).filter_map(|&j| Some((j, s.labels.get(j)?))) {
            if naming(other) && j < i {
                continue;
            }
            let both = rect(label.rect).intersect(rect(other.rect));
            let offender = json!({ "label": label.text, "over_label": other.text, "overlap": [r1(both.width()), r1(both.height())] });
            offenders.push((both.area(), offender));
        }
    }
    // The labels where calls leave the free area lie over no name either.
    for e in &s.exits {
        for label in s.labels.iter().filter(|l| naming(l)) {
            let both = rect(e.rect).intersect(rect(label.rect));
            if both.width() > 0.0 && both.height() > 0.0 {
                let offender = json!({ "exit": e.text, "over_label": label.text, "overlap": [r1(both.width()), r1(both.height())] });
                offenders.push((both.area(), offender));
            }
        }
    }
    let lit: Vec<&Node> = s.nodes.iter().filter(|n| n.on_screen && n.lit()).collect();
    for n in &s.nodes {
        let (Some(label), Some(r)) = (&n.label, n.label_rect) else {
            continue;
        };
        let pill = rect(r).shrink(1.0);
        for other in lit
            .iter()
            .filter(|o| !std::ptr::eq(**o, n) && n.kept_off_by(o))
        {
            let into = -other.gap(pill, other.radius);
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
    for label in s.labels.iter().filter(|l| naming(l)) {
        let about = json!({ "label": label.text, "of": label.of, "rect": rect_json(label.rect) });
        if label.cut_off {
            offenders.push((
                2.0,
                with(about.clone(), "why", json!("cut off by the graph's edge")),
            ));
        }
        // The search box opens over the graph for a moment, as the key help does.
        let under: Vec<&String> = (label.under.iter())
            .filter(|o| o.as_str() != "search box")
            .collect();
        if !under.is_empty() {
            offenders.push((1.0, with(about, "under", json!(under))));
        }
    }
    verdict(offenders)
}

fn module_names(s: &Snap) -> Value {
    let spheres: Vec<&Module> = s.spheres().filter(|m| m.on_screen).collect();
    let mut offenders = Vec::new();
    let mut named = 0;
    for m in s.spheres() {
        let (Some(label), Some(r)) = (&m.label, m.label_rect.map(rect)) else {
            continue;
        };
        named += 1;
        let off_middle = (r.center().x - m.at[0]).abs();
        let above = m.at[1] - m.radius - r.max.y;
        // Where the room above is taken, smaller inside the top of its sphere: its
        // corners within the rim, over the sphere's middle.
        let size = (s.labels.iter())
            .find(|l| l.names == "module" && l.of.as_ref() == Some(&m.name))
            .map_or(0.0, |l| l.size);
        let corner = [
            r.left_top(),
            r.right_top(),
            r.left_bottom(),
            r.right_bottom(),
        ]
        .iter()
        .map(|c| c.distance(pos(m.at)))
        .fold(0.0, f32::max);
        let within = size <= INSIDE_NAME && corner <= m.radius + EDGE && r.max.y <= m.at[1];
        if off_middle > NAME_CENTRED || !((-EDGE..=NAME_ABOVE).contains(&above) || within) {
            let offender = json!({
                "module": m.name, "label": label,
                "why": "not centred just above its sphere's top edge, nor smaller inside its top",
                "off_middle": r1(off_middle), "above_top_by": r1(above),
            });
            offenders.push((off_middle.max(above.abs()), offender));
        }
        for other in spheres.iter().filter(|o| o.name != m.name) {
            let into = other.radius - r.distance_to_pos(pos(other.at));
            if into > 1.0 {
                let offender = json!({ "module": m.name, "label": label, "over_sphere": other.name, "by": r1(into) });
                offenders.push((into + 100.0, offender));
            }
        }
        // Over none of its dots: inside its sphere, none lit, its opaque pill hiding
        // the others.
        let own = |n: &&Node| n.on_screen && n.module.as_ref() == Some(&m.name) && n.radius > 0.0;
        for n in s
            .nodes
            .iter()
            .filter(own)
            .filter(|n| !(within && n.unlit()))
        {
            let into = -n.gap(r.shrink(1.0), n.radius);
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

/// Where a name could go, as the placer has room for it: in the free area, under no
/// overlay, over no name placed as early, a function's off the dots drawn and those a
/// deeper level shows, a module's off other spheres and its own dots.
struct Room<'a> {
    s: &'a Snap,
    free: egui::Rect,
    taken: Vec<(egui::Rect, u8, Option<&'a str>)>,
    drawn: Vec<&'a Node>,
}

impl<'a> Room<'a> {
    fn new(s: &'a Snap) -> Self {
        Self {
            s,
            free: s.free(),
            taken: names_by_rank(s),
            drawn: s.nodes.iter().filter(|n| n.on_screen).collect(),
        }
    }

    /// Whether a pill at `r` would fit: in the free area, under no overlay, over no
    /// name placed as early but `own`'s; a function's off every other dot drawn (as
    /// far as the placer keeps names, the calls a deeper level shows among them) but
    /// those it would pay no mind, and nearer none of them than its own; a module's
    /// off every other sphere and its own dots, but those it would hide inside its
    /// sphere.
    fn clear(
        &self,
        r: egui::Rect,
        rank: u8,
        own: Option<&Node>,
        module: Option<(&str, bool)>,
    ) -> bool {
        // How far the placer kept names from a dot, leaning to less room either way.
        let kept = |n: &Node| n.kept_off.unwrap_or(n.radius) + ROUNDING;
        // Over a dot judged by the pill, nearer one by the text, as labels-near does;
        // its own as drawn, as small as the placer takes it lit.
        let text = r.shrink2(NAME_PILL);
        let mine = own.map_or(0.0, |n| n.gap(text, n.radius - ROUNDING));
        let near = |n: &Node, radius: f32| {
            n.gap(r, radius) < NAME_CLEAR || n.gap(text, radius) < mine - NEARER
        };
        let held = |h: &Held| {
            let gap = |area: egui::Rect| area.distance_to_pos(pos(h.at)) - h.radius - ROUNDING;
            gap(r) < NAME_CLEAR || gap(text) < mine - NEARER
        };
        let unit = own.and_then(|n| n.unit.as_deref());
        self.free.contains_rect(r)
            && !self.s.under_overlay(r)
            && !self.taken.iter().any(|&(t, by, of)| {
                by <= rank && (unit.is_none() || of != unit) && t.intersects(r.expand(NAME_CLEAR))
            })
            && own.is_none_or(|own| {
                !(self.drawn.iter())
                    .any(|n| n.unit != own.unit && !own.overlooks(n, r) && near(n, kept(n)))
            })
            && !self.s.held.iter().any(held)
            && module.is_none_or(|(module, within)| {
                !(self.s.sized())
                    .any(|m| m.name != module && r.distance_to_pos(pos(m.at)) < m.radius - 0.5)
                    && !self.drawn.iter().any(|n| {
                        n.module.as_deref() == Some(module)
                            && !(within && n.unlit())
                            && near(n, kept(n))
                    })
            })
    }

    /// The first place round `n`'s dot the placer tries where its name would fit, as
    /// big as the placer laid it out, its own name drawn aside.
    fn function_spot(&self, n: &Node) -> Option<egui::Rect> {
        let size = n.name_size.map_or_else(
            || {
                let text = match n.numbers.first() {
                    Some(number) => format!("{number}  {}", n.name),
                    None => n.name.clone(),
                };
                guess_size(self.s, &text, "function", (12.5, text::CHAR_WIDTH))
            },
            |[w, h]| egui::vec2(w, h),
        ) + 2.0 * NAME_PILL;
        let reach = n.kept_off.unwrap_or(n.radius) + NAME_GAP;
        // Every place round its dot the placer tries.
        (0..labels::SPOTS.len())
            .filter(|&i| labels::spot_takes(i, size))
            .flat_map(|i| {
                (labels::SLIDES.iter()).map(move |&by| labels::slid(i, pos(n.at), reach, size, by))
            })
            .find(|r| self.clear(*r, n.rank(), Some(n), None))
    }
}

fn named_when_room(s: &Snap) -> Value {
    let room = Room::new(s);
    let mut offenders = Vec::new();
    // A dot off the free area, under the panel or past its edge, is no one's to name
    // beside it: an exit label carries it.
    let unnamed = |n: &&&Node| {
        !n.unlit() && room.free.contains(pos(n.at)) && n.label.is_none() && n.unit.is_some()
    };
    for n in room.drawn.iter().filter(unnamed) {
        if let Some(spot) = room.function_spot(n) {
            let offender = json!({ "function": n.name, "roles": n.roles, "would_fit_at": rect_json(corners(spot)) });
            offenders.push((1.0 + f32::from(u8::from(n.focus())), offender));
        }
    }
    for m in s.sized().filter(|m| m.on_screen && m.label.is_none()) {
        let size = m.name_size.map_or_else(
            || guess_size(s, &m.name.to_uppercase(), "module", (13.5, CAPITAL_WIDTH)),
            |[w, h]| egui::vec2(w, h),
        );
        let [above, inside] = module_spots(m, size);
        let fits = [(above, false), (inside, true)]
            .into_iter()
            .find_map(|(spot, within)| {
                spot.filter(|&spot| room.clear(spot, m.rank(), None, Some((&m.name, within))))
            });
        if let Some(spot) = fits {
            let offender = json!({ "module": m.name, "would_fit_at": rect_json(corners(spot)) });
            offenders.push((0.5, offender));
        }
    }
    verdict(offenders)
}

/// Where a module's name of `size` (its text, at the size above its sphere) would go:
/// just above its sphere, and where that is taken, smaller inside its top, where it
/// fits the sphere's width there.
fn module_spots(m: &Module, size: egui::Vec2) -> [Option<egui::Rect>; 2] {
    let pill = size + 2.0 * NAME_PILL;
    let top = egui::pos2(
        m.at[0] - pill.x / 2.0,
        m.at[1] - m.radius - labels::ABOVE - pill.y,
    );
    let small = size * (text::SMALL / text::TITLE) + 2.0 * NAME_PILL;
    [
        Some(egui::Rect::from_min_size(top, pill)),
        labels::inside(pos(m.at), m.radius, small),
    ]
}

/// Each name drawn, its pill round it, with the rank of what it names: only those
/// placed before what is left out, or as early, could have taken its room.
fn names_by_rank(s: &Snap) -> Vec<(egui::Rect, u8, Option<&str>)> {
    s.labels
        .iter()
        .map(|l| {
            let rank = match l.names.as_str() {
                "function" => l
                    .unit
                    .as_deref()
                    .and_then(|u| s.node(u))
                    .map_or(0, Node::rank),
                "module" => (s.modules.iter())
                    .find(|m| Some(&m.name) == l.of.as_ref())
                    .map_or(0, Module::rank),
                _ => 0,
            };
            (rect(l.rect).expand2(NAME_PILL), rank, l.unit.as_deref())
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
        .reduce(f32::max)
        .unwrap_or(fallback.1);
    egui::vec2(text.chars().count() as f32 * size * width, size * 1.2)
}

/// No two functions read the same name: on the graph within one module, in the
/// journey's bars, or among the search's rows, each judged on its own. Two alike are
/// told apart by their type or module.
fn names_distinct(s: &Snap) -> Value {
    let graph = (s.labels.iter().filter(|l| l.names == "function")).filter_map(|l| {
        let unit = l.unit.as_deref()?;
        Some((
            format!(
                "{} in {}",
                l.of.as_ref()?,
                s.module_of(Some(unit)).unwrap_or_default()
            ),
            unit,
        ))
    });
    let bars = s
        .bars
        .iter()
        .filter_map(|b| Some((b.name.clone(), b.unit.as_deref()?)));
    let rows =
        (s.search_rows.iter().flatten()).filter_map(|r| Some((r.text.clone(), r.unit.as_deref()?)));
    let lists: [(&str, Vec<(String, &str)>); 3] = [
        ("graph", graph.collect()),
        ("bars", bars.collect()),
        ("search rows", rows.collect()),
    ];
    let mut offenders = Vec::new();
    for (place, names) in &lists {
        let mut by_name: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (name, unit) in names {
            let units = by_name.entry(name.as_str()).or_default();
            if !units.contains(unit) {
                units.push(*unit);
            }
        }
        for (name, units) in by_name.into_iter().filter(|(_, u)| u.len() > 1) {
            let offender = json!({ "in": place, "name": name, "functions": units });
            offenders.push((1.0, offender));
        }
    }
    if lists.iter().all(|(_, names)| names.is_empty()) {
        return skip("no function named");
    }
    verdict(offenders)
}

// Calls: of a size, clear of what they pass, labelled where they leave, drawn
// wherever the selection's are due, and in their role's color.

fn arrow_sizes(s: &Snap) -> Value {
    let mut offenders = Vec::new();
    let shown = || {
        (s.arrows.iter().map(|a| ("call", a))).chain(s.module_arrows.iter().map(|a| ("module", a)))
    };
    let shown: Vec<(&str, &Arrow)> = shown().filter(|(_, a)| a.shown()).collect();
    if shown.is_empty() {
        return skip("no arrow drawn on screen");
    }
    for &(kind, a) in &shown {
        let width = a.width.unwrap_or(0.0);
        if width > ARROW_WIDTH {
            let offender = json!({ "arrow": kind, "from": a.from, "to": a.to, "width": r1(width), "most": ARROW_WIDTH });
            offenders.push((width - ARROW_WIDTH, offender));
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
    // The whole map's links between modules are one width, however many calls each
    // carries.
    let widths = (shown.iter())
        .filter(|(kind, _)| *kind == "module")
        .filter_map(|(_, a)| a.width);
    let (thinnest, widest) = widths.fold((f32::INFINITY, 0.0_f32), |(lo, hi), w| {
        (lo.min(w), hi.max(w))
    });
    if widest - thinnest > 0.1 {
        let offender = json!({ "arrow": "module", "why": "the whole map's links differ in width", "widths": [r1(thinnest), r1(widest)] });
        offenders.push((widest - thinnest, offender));
    }
    // One head per arrow: a module arrow ends in a head at the sphere's edge only
    // when no call drawn along it goes on to a dot with a head of its own.
    for (_, m) in shown.iter().filter(|(kind, _)| *kind == "module") {
        let along = |(kind, a): &&(&str, &Arrow)| {
            *kind == "call"
                && s.module_of(a.from_unit.as_deref()) == Some(m.from.as_str())
                && s.module_of(a.to_unit.as_deref()) == Some(m.to.as_str())
        };
        if let Some((_, call)) = shown.iter().find(along) {
            let offender = json!({
                "arrow": "module", "from": m.from, "to": m.to,
                "why": "a head at the sphere's edge, and another where the call goes on to its dot",
                "call": [call.from, call.to],
            });
            offenders.push((1.0, offender));
        }
    }
    verdict(offenders)
}

/// No call's line runs more than [`ARROW_DETOUR`] as long as straight; a bent one
/// runs through no lit dot nor the ring round one, but its own ends', nor through
/// its own ends' names away from where it leaves its dot. A line may pass under any
/// other name: the pill hides it there, and bending round names would loop past the
/// bound. Left out: the faint calls, among an expanded module's functions or only
/// guessed; faint unnamed dots; and an expanded module's plain functions, which in a
/// crowded one no line could keep off (it breaks round a dot).
fn arrows_clear(s: &Snap) -> Value {
    // Lines are routed without the callers I brings in, so I bends none.
    let dots: Vec<&Node> = (s.nodes.iter())
        .filter(|n| n.on_screen && !n.unlit() && (!n.is("outside") || n.focus()))
        .collect();
    let lit = |unit: &Option<String>| dots.iter().any(|n| n.unit.is_some() && n.unit == *unit);
    let names: Vec<&Label> = (s.labels.iter())
        .filter(|l| l.names == "function" && lit(&l.unit))
        .collect();
    let calls: Vec<&Arrow> = (s.arrows.iter())
        .filter(|a| a.flow != "inner" && !a.guessed && a.path.len() >= 2)
        .collect();
    if calls.is_empty() {
        return skip("no call drawn");
    }
    let mut offenders = Vec::new();
    for a in calls {
        let pieces = a.pieces();
        let run: f32 = pieces.iter().map(|(p, q)| p.distance(*q)).sum();
        let chord = v2(a.path[0]).distance(v2(a.path[a.path.len() - 1]));
        if run > chord * ARROW_DETOUR + ARROW_SLACK {
            let detour = run / chord.max(1.0);
            let offender = json!({ "arrow": [a.from, a.to], "detour": r2(detour), "run": r1(run), "straight": r1(chord) });
            offenders.push((detour, offender));
        }
        // A straight line through a dot is drawn broken round it, as no gentle bend
        // clears it.
        if a.path.len() > 2 {
            offenders.extend(through_dots(a, &pieces, &dots));
            offenders.extend(through_names(s, a, &pieces, &names));
        }
    }
    verdict(offenders)
}

/// The lit dots a bent call's line runs through, or the ring round one (but the
/// pointer's).
fn through_dots(a: &Arrow, pieces: &[(Vec2, Vec2)], dots: &[&Node]) -> Found {
    let mut offenders = Vec::new();
    for n in dots.iter().filter(|n| !a.ends_at(n.unit.as_deref())) {
        let rings = n.rings.iter().map(|r| r.radius + RING_MARGIN);
        let ring = if n.is("hovered") {
            0.0
        } else {
            rings.fold(0.0, f32::max)
        };
        let reach = n.radius.max(ring);
        let near = (pieces.iter())
            .map(|&(p, q)| piece_distance(p, q, v2(n.at)))
            .fold(f32::INFINITY, f32::min);
        if near < reach - ARROW_SLACK {
            let offender = json!({ "arrow": [a.from, a.to], "through": "dot", "function": n.name, "at": pt(n.at), "off_by": r1(reach - near) });
            offenders.push((reach - near, offender));
        }
    }
    offenders
}

/// The names of a bent call's own ends its line runs through, away from where it
/// leaves its dot.
fn through_names(s: &Snap, a: &Arrow, pieces: &[(Vec2, Vec2)], names: &[&Label]) -> Found {
    let mut offenders = Vec::new();
    for l in names.iter().filter(|l| a.ends_at(l.unit.as_deref())) {
        let dot = l.unit.as_deref().and_then(|u| s.node(u));
        let off_dot = |p: Vec2, q: Vec2| {
            dot.is_none_or(|n| {
                let clear = n.radius + NAME_GAP + ARROW_SLACK;
                p.distance(v2(n.at)) > clear && q.distance(v2(n.at)) > clear
            })
        };
        let pill = rect(l.rect).shrink(ARROW_SLACK);
        if pieces
            .iter()
            .any(|&(p, q)| off_dot(p, q) && crosses_rect(p, q, pill))
        {
            let offender = json!({ "arrow": [a.from, a.to], "through": "its own name", "label": l.text, "rect": rect_json(l.rect) });
            offenders.push((1.0, offender));
        }
    }
    offenders
}

/// With a module expanded, every lit call drawn from a function in the free area to
/// one off it, or in a journey every call in from outside, is labelled where its line
/// leaves, with the far function's name (or past a few, its module's), on a pill
/// inside the free area; no other call is, and no two labels name one far function
/// or lie over each other.
fn exits_named(s: &Snap) -> Value {
    if !s.expanded() && s.state.mode != "journey" {
        return skip("browsing with no module expanded");
    }
    let free = s.free();
    let inside = |at: [f32; 2]| free.contains(pos(at));
    let module = |unit: &Option<String>| s.module_of(unit.as_deref());
    let expanded = s.state.expanded.as_deref().filter(|_| s.expanded());
    let in_expanded = |unit: &Option<String>| {
        module(unit).is_some_and(|m| expanded.is_some_and(|e| m.eq_ignore_ascii_case(e)))
    };
    // With a module expanded, every call lit (the plain ones among its functions
    // only with one end in it); else, in a journey, the calls in from outside.
    let relevant = |from: &Option<String>, to: &Option<String>, flow: &str| match expanded {
        Some(_) => flow != "inner" || in_expanded(from) != in_expanded(to),
        None => flow == "outside",
    };
    let mut offenders = Vec::new();
    let mut judged = 0;
    for a in (s.arrows.iter())
        .filter(|a| a.flow != "inner" && a.path.len() >= 2)
        .filter(|a| relevant(&a.from_unit, &a.to_unit, &a.flow))
    {
        let (first, last) = (a.path[0], a.path[a.path.len() - 1]);
        if inside(first) == inside(last) {
            continue;
        }
        judged += 1;
        let (far, far_unit) = if inside(first) {
            (&a.to, &a.to_unit)
        } else {
            (&a.from, &a.from_unit)
        };
        let exit = (s.exits.iter()).find(|e| e.from_unit == a.from_unit && e.to_unit == a.to_unit);
        // A label names its far function, or, past a few, that function's module.
        let names_far = |e: &Exit| {
            e.text.contains(far.as_str())
                || module(far_unit).is_some_and(|m| e.text.contains(&m.to_uppercase()))
        };
        match exit {
            Some(e) if names_far(e) && free.contains_rect(rect(e.rect)) => {}
            Some(e) => {
                let why = "its exit label does not name the far function inside the free area";
                let offender = json!({ "arrow": [a.from, a.to], "why": why, "label": e.text, "rect": rect_json(e.rect) });
                offenders.push((1.0, offender));
            }
            None => {
                let why = "leaves the free area unlabelled";
                let offender = json!({ "arrow": [a.from, a.to], "why": why, "far": far });
                offenders.push((1.0, offender));
            }
        }
    }
    let mut pills: Vec<(egui::Rect, &Exit)> = Vec::new();
    for e in &s.exits {
        let flow = (s.arrows.iter())
            .find(|a| a.from_unit == e.from_unit && a.to_unit == e.to_unit)
            .map_or("", |a| a.flow.as_str());
        if !relevant(&e.from_unit, &e.to_unit, flow) {
            let why = "labels a call that should have none: a plain one among the expanded module's, or collapsed, one not in from outside";
            offenders.push((1.0, json!({ "label": e.text, "why": why })));
        }
        // One label names every call leaving for its function or module.
        let r = rect(e.rect);
        if pills.iter().any(|(p, _)| *p == r) {
            continue;
        }
        for (p, other) in &pills {
            if other.far_unit.is_some() && other.far_unit == e.far_unit {
                let why = "names a far function another label names";
                let offender = json!({ "label": e.text, "why": why, "other": other.text });
                offenders.push((1.0, offender));
            } else if p.intersects(r) {
                let both = p.intersect(r);
                let offender = json!({ "label": e.text, "over_label": other.text, "overlap": [r1(both.width()), r1(both.height())] });
                offenders.push((both.area().max(1.0), offender));
            }
        }
        pills.push((r, e));
    }
    if judged == 0 && s.exits.is_empty() {
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
    let own = (s.content.calls.iter())
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
        let apart = v2(a.at).distance(v2(b.at));
        if apart <= a.radius + b.radius + HEAD_SIZE.0 + 2.0 {
            continue;
        }
        judged += 1;
        if !s.arrows.iter().any(|x| x.from == from && x.to == to) {
            let offender = json!({ "call": [from, to], "apart": r1(apart), "why": "not drawn" });
            offenders.push((1.0, offender));
        }
    }
    if judged == 0 {
        return skip("no call between two dots on screen");
    }
    verdict(offenders)
}

// Colors.

fn colours(s: &Snap) -> Value {
    let mut offenders = Vec::new();
    let rings = |n: &Node| n.rings.iter().map(|r| r.color.clone()).collect::<Vec<_>>();
    let ringed = |n: &Node, test: &dyn Fn([f32; 3]) -> bool| {
        n.rings.iter().filter_map(|r| rgb(&r.color)).any(test)
    };
    let fill = |n: &Node| n.color.as_deref().and_then(rgb);
    // Hovering never looks like the selection: no white fill, no ring the selection's
    // color.
    let marked: Vec<[f32; 3]> = (s.nodes.iter())
        .filter(|n| n.is("start") || n.is("selection"))
        .flat_map(|n| n.rings.iter().filter_map(|r| rgb(&r.color)))
        .collect();
    for n in s.nodes.iter().filter(|n| n.is("hovered") && !n.focus()) {
        let selection_like =
            |c: [f32; 3]| whitish(c) || marked.iter().any(|m| distance(*m, c) < 0.1);
        if fill(n).is_some_and(whitish) || ringed(n, &selection_like) {
            let why = "hovered, it looks like the selection";
            let offender =
                json!({ "function": n.name, "why": why, "fill": n.color, "rings": rings(n) });
            offenders.push((2.0, offender));
        }
    }
    // Each function's fill is its role's.
    for n in &s.nodes {
        if let (Some(want), Some(got)) = (node_role_color(n), fill(n))
            && !like(got, want)
        {
            let offender = json!({ "function": n.name, "roles": n.roles, "fill": n.color, "role_color": hex3(want) });
            offenders.push((1.0, offender));
        }
    }
    // The function set aside keeps its role's fill, and a white ring says it is.
    for n in s.nodes.iter().filter(|n| n.is("aside") && !n.is("cursor")) {
        if !ringed(n, &whitish) {
            let why = "set aside, but no white ring";
            let offender = json!({ "function": n.name, "why": why, "rings": rings(n) });
            offenders.push((1.0, offender));
        }
        if fill(n).is_some_and(whitish) {
            let why = "set aside, but filled white as the start is";
            let offender = json!({ "function": n.name, "why": why, "fill": n.color });
            offenders.push((1.0, offender));
        }
    }
    // A call only guessed is drawn faint, as its bar is: darker than its role's.
    let bright = |c: [f32; 3]| c[0].max(c[1]).max(c[2]);
    for n in s.nodes.iter().filter(|n| n.is("guessed")) {
        if let (Some(got), Some(want)) = (fill(n), node_role_color(n))
            && bright(got) > GUESSED_DOT * bright(want)
        {
            let why = "only guessed, but drawn as bright as a sure call";
            offenders.push((1.0, json!({ "function": n.name, "why": why, "fill": n.color, "role_color": hex3(want) })));
        }
    }
    // A call that calls the start back is ringed in the caller's violet.
    for n in s.nodes.iter().filter(|n| n.is("calls back") && !n.focus()) {
        if !ringed(n, &|c: [f32; 3]| like(c, srgb(CALLER))) {
            let why = "calls back, but no violet ring";
            let offender = json!({ "function": n.name, "why": why, "rings": rings(n) });
            offenders.push((1.0, offender));
        }
    }
    bar_colours(s, &mut offenders);
    call_colours(s, &mut offenders);
    verdict(offenders)
}

/// A bar is painted as its node is: the cursor's other runs as it would be were the
/// cursor elsewhere; a call only guessed in the same hue, faint. A bar in the row of
/// the start's callers is violet, whatever else its function is; faint, as its dot
/// is, one folded into its module's count.
fn bar_colours(s: &Snap, offenders: &mut Found) {
    for bar in &s.bars {
        let Some(got) = bar.color.as_deref().and_then(rgb) else {
            continue;
        };
        let strength = bar.color.as_deref().and_then(alpha).unwrap_or(1.0);
        let node = bar.unit.as_deref().and_then(|u| s.node(u));
        if bar.caller {
            let folded = node.is_some_and(|n| n.roles.iter().any(|r| r.contains("folded")));
            if !like(got, srgb(CALLER)) || folded != (strength <= FAINT) {
                let why =
                    "a caller's bar is violet, faint only when folded into its module's count";
                offenders.push((1.0, json!({ "caller_bar": bar.name, "bar_color": bar.color, "folded": folded, "why": why })));
            }
            continue;
        }
        if bar.cursor || bar.unit.is_none() {
            continue;
        }
        if bar.guessed && strength > FAINT {
            let why = "only guessed, but not faint";
            let offender = json!({ "bar": bar.number, "name": bar.name, "bar_color": bar.color, "why": why, "most_alpha": FAINT });
            offenders.push((1.0, offender));
        }
        let Some(n) = node.filter(|n| !n.is("cursor")) else {
            continue;
        };
        let want = if n.is("guessed") {
            node_role_color(n)
        } else {
            n.color.as_deref().and_then(rgb)
        };
        if want.is_some_and(|want| !like(got, want)) {
            let offender = json!({ "bar": bar.number, "name": bar.name, "bar_color": bar.color, "node_color": n.color });
            offenders.push((1.0, offender));
        }
    }
}

/// Each call's color is its role's: none reads white; only the journey's call into
/// the cursor and the calls of the function set aside are drawn followed; a call
/// off the journey among the expanded module's functions, and one only guessed, are
/// faint.
fn call_colours(s: &Snap, offenders: &mut Found) {
    let cursor = s.cursor();
    let aside = s.aside().and_then(|n| n.unit.as_deref());
    for a in &s.arrows {
        let Some(got) = a.color.as_deref().and_then(rgb) else {
            continue;
        };
        let strength = a.color.as_deref().and_then(alpha).unwrap_or(1.0);
        let from_aside = aside.is_some() && a.from_unit.as_deref() == aside;
        let into_cursor = cursor.is_some() && a.to_unit.as_deref() == cursor && a.flow == "journey";
        let about = |why: &str| json!({ "arrow": [a.from, a.to], "flow": a.flow, "color": a.color, "why": why });
        // White is the selection's and the start's alone: no call reads white on the
        // ground, a grey one from off the journey followed least of all.
        let shown = over_ground(got, strength);
        if whitish(shown) {
            let why = "a call that reads white, as the selection does";
            offenders.push((1.0, with(about(why), "shows", json!(hex3(shown)))));
            continue;
        }
        // Only the journey's call into the cursor and the calls of the function set
        // aside are drawn stronger or wider than the rest: pointing at a function
        // lights none.
        let followed =
            strength > CALL_STRENGTH + 0.05 || a.width.unwrap_or(0.0) > arrows::CALL_WIDTH + 0.25;
        if followed && !from_aside && !into_cursor {
            let why =
                "drawn as followed, but neither into the cursor nor from the function set aside";
            offenders.push((1.0, with(about(why), "width", json!(a.width))));
            continue;
        }
        // A call among the expanded module's functions that the journey does not make,
        // out of the function set aside too, and a call only guessed, are faint.
        if (a.flow == "inner" || a.guessed) && strength > FAINT {
            let why = if a.guessed {
                "a call only guessed drawn strong"
            } else {
                "a call off the journey drawn bright"
            };
            offenders.push((1.0, with(about(why), "most_alpha", json!(FAINT))));
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

/// How `color` drawn at `strength` shows over the ground behind the graph.
fn over_ground(color: [f32; 3], strength: f32) -> [f32; 3] {
    let ground = srgb(GROUND);
    [0, 1, 2].map(|i| color[i] * strength + ground[i] * (1.0 - strength))
}

/// The color a function's role gives its fill, if its roles name one.
fn node_role_color(n: &Node) -> Option<[f32; 3]> {
    if n.is("cursor") {
        Some(srgb(CURSOR))
    } else if n.is("start") || n.is("selection") {
        Some([1.0, 1.0, 1.0])
    } else if n.is("caller") {
        Some(srgb(CALLER))
    } else if let Some(level) = n.level() {
        Some(flow_color("journey", Some(level)))
    } else {
        // Set aside off the journey: grey, its ring alone white.
        n.is("aside").then(|| srgb(FURTHER))
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

/// A `#rrggbbaa` color's alpha, 0 to 1.
fn alpha(hex: &str) -> Option<f32> {
    let hex = hex.trim_start_matches('#');
    u8::from_str_radix(hex.get(6..8)?, 16)
        .ok()
        .map(|a| f32::from(a) / 255.0)
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

// Geometry, and how results read.

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
    (near - radius).max(radius - far).max(0.0)
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

/// The square on screen a circle fills.
fn disc(at: [f32; 2], radius: f32) -> egui::Rect {
    egui::Rect::from_center_size(pos(at), egui::Vec2::splat(2.0 * radius))
}

/// Passes with no offenders; else fails, listing the worst first.
fn verdict(mut offenders: Found) -> Value {
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

/// A rectangle's corners, as the JSON gives them: `[x0, y0, x1, y1]`.
fn corners(r: egui::Rect) -> [f32; 4] {
    [r.min.x, r.min.y, r.max.x, r.max.y]
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
    fn a_bent_call_through_a_third_dot_or_its_own_name_is_named() {
        let mut s = scene();
        s["state"]["mode"] = json!("journey");
        s["nodes"][0]["roles"] = json!(["start"]);
        s["nodes"][1]["roles"] = json!(["journey level 1"]);
        s["nodes"][1]["radius"] = json!(4.0);
        s["nodes"][2]["roles"] = json!(["journey level 1"]);
        s["labels"]
            .as_array_mut()
            .expect("labels")
            .push(json!({ "text": "g", "rect": [700.0, 494.0, 720.0, 506.0],
                "names": "function", "of": "g", "unit": "x:a::g" }));
        // Straight from f to h through g and under its name: drawn broken round the
        // dot, hidden by the pill, so it passes, browsing too.
        s["arrows"] = json!([{ "from": "f", "to": "h", "from_unit": "x:a::f", "to_unit": "x:b::h",
            "flow": "journey", "level": 1, "path": [[386.0, 500.0], [994.0, 500.0]] }]);
        assert!(
            passes(&check(&s, "arrows-clear")),
            "{}",
            check(&s, "arrows-clear")
        );
        s["state"]["mode"] = json!("browse");
        assert!(passes(&check(&s, "arrows-clear")));
        // Bent, but through g's dot.
        s["arrows"][0]["path"] = json!([
            [386.0, 500.0],
            [440.0, 500.0],
            [700.0, 520.0],
            [994.0, 500.0]
        ]);
        let result = check(&s, "arrows-clear");
        assert_eq!(result["count"], 1, "{result}");
        assert_eq!(result["offenders"][0]["function"], "g");
        // Bent round g, it passes, over its own name only where it leaves its dot.
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
        // Bent back through its own end's name, away from the dot.
        s["arrows"][0]["path"] = json!([
            [386.0, 500.0],
            [440.0, 530.0],
            [1012.0, 520.0],
            [1012.0, 500.0],
            [1006.0, 500.0]
        ]);
        let result = check(&s, "arrows-clear");
        assert!(
            result["offenders"]
                .as_array()
                .expect("offenders")
                .iter()
                .any(|o| o["through"] == "its own name"),
            "{result}"
        );
        // Too near a ring round g.
        s["arrows"][0]["path"] = json!([
            [386.0, 500.0],
            [440.0, 530.0],
            [710.0, 530.0],
            [994.0, 500.0]
        ]);
        s["nodes"][1]["rings"] = json!([{ "radius": 40.0, "color": "#ffffffff" }]);
        assert_eq!(check(&s, "arrows-clear")["offenders"][0]["function"], "g");
        // A caller I brings in: lines are routed without it.
        let roles = s["nodes"][1]["roles"].clone();
        s["nodes"][1]["roles"] = json!(["outside"]);
        assert!(passes(&check(&s, "arrows-clear")));
        s["nodes"][1]["roles"] = roles;
        // A loop more than a third longer than straight is a detour.
        s["nodes"][1]["rings"] = json!([]);
        s["arrows"][0]["path"] = json!([[386.0, 500.0], [700.0, 800.0], [994.0, 500.0]]);
        let result = check(&s, "arrows-clear");
        assert_eq!(result["count"], 1, "{result}");
        assert!(result["offenders"][0]["detour"].as_f64().expect("detour") > 1.35);
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
    fn expanded_a_call_off_the_free_area_is_labelled_where_it_leaves() {
        let mut s = scene();
        s["state"]["mode"] = json!("journey");
        s["state"]["nucleus"] = json!("expanded");
        s["state"]["expanded"] = json!("a");
        s["arrows"] = json!([{ "from": "f", "to": "far", "from_unit": "x:a::f", "to_unit": "x:c::far",
            "flow": "journey", "level": 1, "path": [[386.0, 500.0], [386.0, 1200.0]] }]);
        assert_eq!(check(&s, "exits-named")["count"], 1);
        s["exits"] = json!([{ "from_unit": "x:a::f", "to_unit": "x:c::far", "far_unit": "x:c::far",
            "far": "far", "text": "to far · C", "rect": [350.0, 975.0, 420.0, 995.0] }]);
        assert!(
            passes(&check(&s, "exits-named")),
            "{}",
            check(&s, "exits-named")
        );
        // Neither end in the module expanded: a lit call leaving is labelled too.
        s["state"]["expanded"] = json!("b");
        s["exits"] = json!([]);
        assert_eq!(check(&s, "exits-named")["count"], 1);
        s["exits"] = json!([{ "from_unit": "x:a::f", "to_unit": "x:c::far", "far_unit": "x:c::far",
            "far": "far", "text": "to far · C", "rect": [350.0, 975.0, 420.0, 995.0] }]);
        s["state"]["expanded"] = json!("a");
        // Browsing expanded too.
        s["state"]["mode"] = json!("browse");
        assert!(passes(&check(&s, "exits-named")));
        // Not over a name.
        s["exits"][0]["rect"] = json!([385.0, 490.0, 455.0, 510.0]);
        let result = check(&s, "labels-clear");
        assert_eq!(result["offenders"][0]["exit"], "to far · C", "{result}");
        s["exits"][0]["rect"] = json!([350.0, 975.0, 420.0, 995.0]);
        // A second label for the same far function, over the first.
        let mut exits = s["exits"].as_array().expect("exits").clone();
        exits.push(
            json!({ "from_unit": "x:a::g", "to_unit": "x:c::far", "far_unit": "x:c::far",
            "far": "far", "text": "to far · C", "rect": [360.0, 980.0, 430.0, 1000.0] }),
        );
        // And one for a plain call among the expanded module's own.
        exits.push(
            json!({ "from_unit": "x:a::f", "to_unit": "x:a::g", "far_unit": "x:a::g",
            "far": "g", "text": "to g · A", "rect": [500.0, 975.0, 560.0, 995.0] }),
        );
        s["exits"] = json!(exits);
        s["arrows"].as_array_mut().expect("arrows").push(json!({ "from": "f", "to": "g",
            "from_unit": "x:a::f", "to_unit": "x:a::g", "flow": "inner", "path": [[386.0, 500.0], [436.0, 500.0]] }));
        let whys: Vec<String> = check(&s, "exits-named")["offenders"]
            .as_array()
            .expect("offenders")
            .iter()
            .filter_map(|o| o["why"].as_str().map(str::to_owned))
            .collect();
        assert!(whys.iter().any(|w| w.contains("another label")), "{whys:?}");
        assert!(
            whys.iter().any(|w| w.contains("should have none")),
            "{whys:?}"
        );
        // Collapsed and browsing, nothing to label.
        s["state"]["nucleus"] = json!("modules");
        assert_eq!(check(&s, "exits-named")["pass"], Value::Null);
    }

    #[test]
    fn a_call_bent_anew_on_a_step_that_moves_nothing_has_moved() {
        let mut before = scene();
        before["state"]["mode"] = json!("journey");
        before["arrows"] = json!([{ "from": "f", "to": "h", "from_unit": "x:a::f", "to_unit": "x:b::h",
            "flow": "journey", "level": 1, "path": [[386.0, 500.0], [994.0, 500.0]] }]);
        // The callee grown as the cursor lands on it: its line a little shorter.
        let mut now = before.clone();
        now["arrows"][0]["path"] = json!([[386.0, 500.0], [992.0, 500.0]]);
        let result = after(&before, &now, &[Kind::Still], "nothing-moved");
        assert!(passes(&result), "{result}");
        now["arrows"][0]["path"] = json!([[386.0, 500.0], [690.0, 530.0], [994.0, 500.0]]);
        let result = after(&before, &now, &[Kind::Still], "nothing-moved");
        assert_eq!(result["count"], 1, "{result}");
        assert_eq!(result["offenders"][0]["arrow"], json!(["f", "h"]));
        // A depth change that drew the camera back moves every line with it.
        now["state"]["camera"]["distance"] = json!(120.0);
        let result = after(&before, &now, &[Kind::Depth], "nothing-moved");
        assert!(
            result["offenders"]
                .as_array()
                .is_none_or(|o| o.iter().all(|o| o["arrow"].is_null())),
            "{result}"
        );
    }

    #[test]
    fn a_name_shown_only_under_the_pointer_goes_with_it() {
        let mut before = scene();
        before["nodes"][2]["roles"] = json!(["callee", "hovered"]);
        let mut now = scene();
        now["labels"].as_array_mut().expect("labels").remove(2);
        let result = after(&before, &now, &[Kind::Still], "nothing-moved");
        assert!(passes(&result), "{result}");
        before["nodes"][2]["roles"] = json!(["callee"]);
        let result = after(&before, &now, &[Kind::Still], "nothing-moved");
        assert_eq!(result["offenders"][0]["gone"], true, "{result}");
    }

    #[test]
    fn in_a_journey_a_call_in_from_outside_off_the_free_area_is_labelled() {
        let mut s = scene();
        s["state"]["mode"] = json!("journey");
        s["arrows"] = json!([{ "from": "far", "to": "f", "from_unit": "x:c::far", "to_unit": "x:a::f",
            "flow": "outside", "path": [[386.0, 1200.0], [386.0, 506.0]] }]);
        assert_eq!(check(&s, "exits-named")["count"], 1);
        s["exits"] = json!([{ "from_unit": "x:c::far", "to_unit": "x:a::f", "far_unit": "x:c::far",
            "far": "far", "text": "from far · C", "rect": [350.0, 975.0, 430.0, 995.0] }]);
        assert!(
            passes(&check(&s, "exits-named")),
            "{}",
            check(&s, "exits-named")
        );
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
        assert_eq!(result["count"], 3, "{result}");
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
    fn the_expanded_module_fills_the_band_after_expanding() {
        let fill = |s: &Value| after(s, s, &[Kind::Expand], "expanded-fill");
        let result = fill(&expanded(scene(), 171.0, "f"));
        assert!(
            !passes(&result),
            "a third of the view is too little: {result}"
        );
        assert_eq!(result["offenders"][0]["why"], "outside the band");
        let result = fill(&expanded(scene(), 300.0, "f"));
        assert!(passes(&result), "{result}");
        assert_eq!(result["fill"], 0.61);
        assert!(
            !passes(&fill(&expanded(scene(), 460.0, "f"))),
            "nor past it"
        );
        let mut s = expanded(scene(), 400.0, "f");
        s["overlays"] = json!([{ "name": "journey panel", "rect": [0.0, 800.0, 1600.0, 1000.0] }]);
        s["free_area"] = json!([0.0, 24.0, 1600.0, 800.0]);
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
    fn the_expanded_module_grows_while_its_dots_crowd() {
        let fill = |s: &Value| after(s, s, &[Kind::Expand], "expanded-fill");
        let mut s = expanded(scene(), 260.0, "f");
        s["nodes"][1]["at"] = json!([s["nodes"][0]["at"][0].as_f64().unwrap() + 10.0, 512.0]);
        let result = fill(&s);
        assert!(!passes(&result), "two dots 10 points apart: {result}");
        let mut s = expanded(scene(), 430.0, "f");
        s["nodes"][1]["at"] = json!([s["nodes"][0]["at"][0].as_f64().unwrap() + 10.0, 512.0]);
        assert!(passes(&fill(&s)), "at the band's top it can grow no more");
    }

    #[test]
    fn an_expand_zooms_in() {
        let mut before = scene();
        before["modules"][0]["radius"] = json!(300.0);
        let now = expanded(scene(), 280.0, "f");
        let result = after(&before, &now, &[Kind::Expand], "expanded-fill");
        assert_eq!(
            result["offenders"][0]["why"], "the expand did not zoom in",
            "{result}"
        );
        before["modules"][0]["radius"] = json!(100.0);
        assert!(passes(&after(
            &before,
            &now,
            &[Kind::Expand],
            "expanded-fill"
        )));
    }

    #[test]
    fn an_expansion_never_leaves_a_blank_graph() {
        let mut s = expanded(scene(), 300.0, "f");
        s["state"]["expanded"] = json!("a");
        assert!(passes(&check(&s, "expanded-drawn")));
        s["modules"][0]["body"] = json!("closed");
        s["nodes"][0]["at"] = json!([-358.0, 914.0]);
        s["nodes"][1]["at"] = json!([-300.0, 914.0]);
        let result = check(&s, "expanded-drawn");
        assert_eq!(result["offenders"][0]["module"], "a", "{result}");
        assert_eq!(
            check(&scene(), "expanded-drawn")["pass"],
            Value::Null,
            "collapsed"
        );
    }

    #[test]
    fn the_expanded_module_is_the_one_the_state_names() {
        let mut s = scene();
        s["state"]["nucleus"] = json!("expanded");
        s["state"]["expanded"] = json!("b");
        s["modules"][0]["body"] = json!("nucleus");
        s["modules"][1]["at"] = json!([800.0, 512.0]);
        s["modules"][1]["radius"] = json!(300.0);
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
        assert_eq!(result["offenders"][1]["out_by"], 60.0);
        let resized = after(&s, &s, &[Kind::FrameAll, Kind::Resize], "framed");
        assert_eq!(resized["count"], 2, "a window sized anew frames as R did");
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

    #[test]
    fn a_name_shown_come_what_may_with_room_nowhere_may_lie_nearer_another_dot() {
        let mut s = scene();
        s["nodes"][0]["roles"] = json!(["start"]);
        s["nodes"][1]["roles"] = json!(["caller"]);
        s["nodes"][1]["at"] = json!([420.0, 500.0]);
        s["nodes"][0]["label_rect"] = json!([416.0, 494.0, 440.0, 506.0]);
        let result = check(&s, "labels-near");
        assert_eq!(
            result["offenders"][0]["nearer_dot"], "g",
            "room on its left: {result}"
        );

        s["graph_area"] = json!([360.0, 496.0, 440.0, 504.0]);
        let result = check(&s, "labels-near");
        assert!(passes(&result), "room nowhere: {result}");
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
    fn expanded_in_a_journey_a_click_may_pan_but_moves_nothing_else() {
        let mut before = journey();
        before["state"]["nucleus"] = json!("expanded");
        before["state"]["expanded"] = json!("b");
        let mut now = before.clone();
        now["state"]["camera"]["target"] = json!([1.0, 0.0, 0.0]);
        for (i, at) in [
            (0, [330.0, 500.0]),
            (1, [390.0, 500.0]),
            (2, [950.0, 500.0]),
        ] {
            now["nodes"][i]["at"] = json!(at);
        }
        for (i, rect) in [
            (1, [340.0, 494.0, 360.0, 506.0]),
            (2, [958.0, 494.0, 966.0, 506.0]),
        ] {
            now["labels"][i]["rect"] = json!(rect);
        }
        assert!(
            passes(&after(&before, &now, &[Kind::Click], "nothing-moved")),
            "panned: the camera has leave"
        );
        now["modules"][1]["world"] = json!([11.0, 0.0, 0.0]);
        let result = after(&before, &now, &[Kind::Click], "nothing-moved");
        assert_eq!(result["offenders"][0]["module"], "b", "{result}");
        now["state"]["expanded"] = json!("a");
        assert_eq!(
            after(&before, &now, &[Kind::Still], "nothing-moved")["pass"],
            Value::Null,
            "the cursor took the expansion along"
        );
    }

    #[test]
    fn a_sphere_half_under_the_panel_just_after_shrinking_back_is_named() {
        let mut s = scene();
        s["overlays"] = json!([{ "name": "journey panel", "rect": [0.0, 560.0, 1600.0, 1000.0] }]);
        s["free_area"] = json!([0.0, 24.0, 1600.0, 560.0]);
        let mut before = s.clone();
        before["state"]["nucleus"] = json!("expanded");
        let result = after(&before, &s, &[Kind::Expand], "spheres-unclipped");
        assert_eq!(result["count"], 2, "{result}");
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
        // A call guessed among several functions: on the graph, or the panel says
        // what the graph does not.
        let mut s = journey();
        s["bars"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "number": "2", "name": "g → one of 2", "cursor": false }));
        let result = check(&s, "panel-agrees");
        assert_eq!(result["offenders"][0]["bar"], "2", "{result}");
        s["nodes"][1]["numbers"] = json!(["2"]);
        assert!(passes(&check(&s, "panel-agrees")));
    }

    #[test]
    fn expanding_in_a_journey_moves_none_of_its_dots_in_the_world() {
        let before = journey();
        let mut now = journey();
        now["state"]["nucleus"] = json!("expanded");
        // The camera came in: on screen everything grew and moved.
        now["nodes"][2]["at"] = json!([900.0, 300.0]);
        let result = after(&before, &now, &[Kind::Expand], "nothing-moved");
        assert!(passes(&result), "{result}");
        now["nodes"][2]["world"] = json!([12.0, 1.0, 0.0]);
        let result = after(&before, &now, &[Kind::Expand], "nothing-moved");
        assert_eq!(result["offenders"][0]["function"], "h", "{result}");
    }

    #[test]
    fn a_dot_off_the_free_area_is_left_to_an_exit_label() {
        let mut s = scene();
        s["nodes"][2]["label"] = Value::Null;
        s["nodes"][2]["label_rect"] = Value::Null;
        let named = |s: &Value| {
            check(s, "named-when-room")["offenders"]
                .as_array()
                .is_some_and(|o| o.iter().any(|o| o["function"] == "h"))
        };
        assert!(named(&s), "room beside it, and unnamed");
        s["nodes"][2]["at"] = json!([1000.0, 1003.0]);
        assert!(!named(&s), "just past the free area's foot");
    }

    #[test]
    fn bars_below_the_panel_s_foot_are_said_to_be_there() {
        let mut s = journey();
        assert_eq!(check(&s, "bars-in-view")["pass"], Value::Null);
        s["panel"] = json!({ "below": 3, "rows_track": false });
        let result = check(&s, "bars-in-view");
        assert_eq!(result["offenders"][0]["below"], 3, "{result}");
        s["panel"]["rows_track"] = json!(true);
        assert!(passes(&check(&s, "bars-in-view")));
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
        s["free_area"] = json!([0.0, 24.0, 1600.0, 800.0]);
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
    fn a_shrink_back_or_back_is_judged_as_such() {
        let mut s = journey();
        s["overlays"] = json!([{ "name": "journey panel", "rect": [0.0, 800.0, 1600.0, 1000.0] }]);
        s["free_area"] = json!([0.0, 24.0, 1600.0, 800.0]);
        s["nodes"][2]["at"] = json!([1000.0, 820.0]);
        // X shrinking back must leave what is in focus in view; expanding need not.
        let result = after(&s, &s, &[Kind::Expand], "focus-on-screen");
        assert_eq!(result["offenders"][0]["function"], "h", "{result}");
        s["state"]["nucleus"] = json!("expanded");
        s["state"]["expanded"] = json!("b");
        let result = after(&s, &s, &[Kind::Expand], "focus-on-screen");
        assert_eq!(result["pass"], Value::Null, "{result}");
        // Back puts back the camera a place had, wherever that shows things.
        s["state"]["nucleus"] = json!("modules");
        for name in ["focus-on-screen", "journey-fills"] {
            let result = after(&s, &s, &[Kind::Back], name);
            assert_eq!(result["pass"], Value::Null, "{name}: {result}");
        }
    }

    #[test]
    fn an_expand_keeps_the_selection() {
        let mut before = scene();
        before["state"]["selection"] = json!("f");
        let mut now = before.clone();
        assert_ne!(
            after(&before, &now, &[Kind::Expand], "focus-on-screen")["pass"],
            json!(false)
        );
        now["state"]["selection"] = json!("h");
        now["nodes"][2]["roles"] = json!(["selection"]);
        let result = after(&before, &now, &[Kind::Expand], "focus-on-screen");
        assert!(!passes(&result), "{result}");
        assert_ne!(
            after(
                &before,
                &now,
                &[Kind::Click, Kind::Expand],
                "focus-on-screen"
            )["pass"],
            json!(false),
            "a click picks"
        );
    }

    #[test]
    fn a_window_sized_anew_leaves_the_hands_camera_be() {
        let before = scene();
        let mut now = before.clone();
        let resized = Since {
            kinds: vec![Kind::Resize],
            by_hand: true,
            ..Since::default()
        };
        assert!(passes(&run(&now, Some(&before), &resized)["framed"]));
        now["state"]["camera"]["distance"] = json!(140.0);
        let result = run(&now, Some(&before), &resized)["framed"].clone();
        assert!(!passes(&result), "{result}");
    }

    #[test]
    fn the_depth_shown_is_one_the_journey_has() {
        let mut s = journey();
        s["state"]["deepest"] = json!(3);
        s["state"]["depth"] = json!(3);
        assert_ne!(
            check(&s, "panel-agrees")["offenders"][0]["why"],
            json!("the depth shown is not one of the journey's levels")
        );
        s["state"]["depth"] = json!(5);
        let result = check(&s, "panel-agrees");
        assert_eq!(result["offenders"][0]["deepest"], 3, "{result}");
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
        s["free_area"] = json!([0.0, 24.0, 1600.0, 560.0]);
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
            !passes(&check(&s, "spheres-unclipped")),
            "zoomed into a module, the framing keeps those round it off the panel's edge"
        );
    }

    #[test]
    fn a_function_set_aside_since_shows_wherever_the_expansion_is() {
        let mut s = journey();
        s["state"]["nucleus"] = json!("expanded");
        s["state"]["expanded"] = json!("a");
        let before = s.clone();
        s["state"]["aside"] = json!("h");
        s["nodes"][2]["roles"] = json!(["aside"]);
        s["nodes"][2]["at"] = json!([1772.0, -959.0]);
        let result = after(&before, &s, &[Kind::Moves], "focus-on-screen");
        assert_eq!(result["offenders"][0]["function"], "h", "{result}");
    }

    #[test]
    fn expanded_the_focus_off_the_module_and_a_hand_placed_camera_are_left_alone() {
        let mut s = journey();
        s["overlays"] = json!([{ "name": "journey panel", "rect": [0.0, 800.0, 1600.0, 1000.0] }]);
        s["free_area"] = json!([0.0, 24.0, 1600.0, 800.0]);
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
        // Right of its dot, a plain dot it would hide, and a plain name.
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
        // Dots a deeper level shows all round it keep it off every side.
        let at = s["nodes"][2]["at"].clone();
        s["held"] = json!([{ "at": at, "radius": 120.0 }]);
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
        s["free_area"] = json!([0.0, 24.0, 1600.0, 560.0]);
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
        // Its caller's bar painted blue as its dot is, not violet.
        let result = check(&s, "colours");
        assert_eq!(result["offenders"][0]["caller_bar"], "g", "{result}");
        s["bars"][1]["color"] = json!("#bd85ff");
        assert!(passes(&check(&s, "colours")), "{}", check(&s, "colours"));
        // A call back with no violet ring.
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
