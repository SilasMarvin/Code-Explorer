//! The graph gathered into module spheres, in one of two modes. Browsing, with no
//! journey open, every module is a sphere at its place in one layout of the whole
//! codebase, worked out once per document, with every one of its functions inside; a
//! click lights the selection, white, its callers, violet, and its calls, blue, where
//! they sit, and moves nothing. In a journey, the start's module is a large glass
//! sphere in the middle and the modules the journey enters stand round it clockwise in
//! the order it first enters them; shown are the start, what calls it, and the
//! journey's calls down to the disclosed depth, the same depth the journey panel
//! shows, and nothing else. Calls into the start are violet; its own calls out blue,
//! and every journey call further down amber, darker the deeper, function to function
//! along thin arrows between modules. Space says only where things are: no axis
//! stands for order, rank or distance in calls.
//!
//! Expanded (`X`, a double click on the nucleus, or `--expand`), the nucleus grows to
//! several times its size as the camera comes in, and its functions are laid out by
//! the calls among them, what is named in the middle, callers leaning one way and the
//! journey the other, the rest small towards the wall. The same functions are named
//! as in the module view: those in other modules sit in their module's sphere on a
//! ring just outside the wall, and a call to one runs out through the wall.

use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use bevy::prelude::*;
use codeviz_layout::ForceGraph;

use crate::{
    camera::OrbitCamera,
    colors::{CALLEE, CALLER, CURSOR, DEEPER, FURTHER, MODULE_SHADES, module_color, module_shade},
    disclosure::number_order,
    explorer::Explorer,
    graph_view::{Dot, drawn_radius},
    idle::Awake,
};

/// How a module shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Body {
    /// The start's module, in the middle, with every function inside it.
    Nucleus,
    /// Holds functions near the start: opened to show those alone, by name.
    Open,
    /// Everything else: a small solid sphere, left out of a journey altogether.
    Closed,
    /// Browsing, a module nothing named lives in: faint glass, its functions specks.
    Faint,
}

/// A module's sphere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ball {
    pub(crate) center: Vec3,
    pub(crate) radius: f32,
    pub(crate) body: Body,
}

/// What a named call is, seen from the start (or the selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Flow {
    /// Calls the start.
    Into,
    /// A call of the journey, this many levels below the start: 1 for its own.
    Journey(u32),
    /// A call into a function on the journey from a function off it (I).
    Outside,
    /// Browsing with nothing selected, every call from one module to another.
    Module,
}

/// Levels of the journey drawn in a shade of their own: the start's calls blue, then
/// amber, darker a level at a time; deeper than this, as dark as the last.
const SHADES: u32 = 4;

impl Flow {
    /// The journey's call this many levels below the start.
    pub(crate) fn journey(level: u32) -> Self {
        Self::Journey(level.clamp(1, SHADES))
    }

    /// Violet into the start, blue for its own calls out, amber for the calls
    /// further down the journey, darker the deeper; pale for a call in from outside.
    pub(crate) fn color(self) -> Color {
        match self {
            Self::Into => CALLER,
            Self::Journey(1) => CALLEE,
            Self::Journey(level) => {
                #[expect(clippy::cast_precision_loss, reason = "a small level")]
                let deeper = (level.min(SHADES) - 2) as f32;
                DEEPER.mix(&Color::BLACK, 0.2 * deeper)
            }
            Self::Outside | Self::Module => FURTHER,
        }
    }

    /// Which of two flows an arrow carrying both says: into the start, else the
    /// shallowest level, and a call in from outside last.
    fn rank(self) -> u32 {
        match self {
            Self::Into => 0,
            Self::Journey(level) => level,
            Self::Outside => SHADES + 1,
            Self::Module => SHADES + 2,
        }
    }
}

/// One call of the open journey, from caller to callee, by tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Call {
    pub(crate) from: usize,
    pub(crate) to: usize,
    /// Levels below the start: 1 for a call the start makes.
    pub(crate) level: u32,
}

/// The open journey from its start down to the disclosed depth: each call once, in
/// the journey's order, and each function's numbers in it, as the panel numbers them.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Trail {
    pub(crate) calls: Vec<Call>,
    pub(crate) numbers: HashMap<usize, Vec<String>>,
    /// Every function below the start at any depth, with the fewest calls it is
    /// from it: what the expanded nucleus is laid out for, so a new depth moves
    /// nothing.
    pub(crate) whole: HashMap<usize, u32>,
    /// Every function below the start at any depth, in the order the journey first
    /// runs it, which orders the modules round the nucleus.
    pub(crate) order: Vec<usize>,
}

impl Trail {
    /// Adds a call the journey makes, numbered `number`; one already there keeps its
    /// shallowest level.
    pub(crate) fn add(&mut self, from: usize, to: usize, level: u32, number: &str) {
        match self.calls.iter_mut().find(|c| c.from == from && c.to == to) {
            Some(call) => call.level = call.level.min(level),
            None => self.calls.push(Call { from, to, level }),
        }
        let numbers = self.numbers.entry(to).or_default();
        if !numbers.iter().any(|n| n == number) {
            numbers.push(number.to_owned());
            numbers.sort_by_key(|n| number_order(n));
        }
    }
}

/// A function's numbers on its badge: the call `at` names when given, the cursor's
/// call, which the panel names too; else its first call in the panel's order, nearest
/// the root; and how many more times the journey runs it. Where the cursor is never
/// changes another function's badge, and a deeper level disclosed only adds to the
/// count.
pub(crate) fn badge<'a>(
    numbers: impl IntoIterator<Item = &'a String>,
    at: Option<&str>,
) -> Option<String> {
    let mut numbers: Vec<&str> = numbers.into_iter().map(String::as_str).collect();
    numbers.sort_by_key(|n| number_order(n));
    numbers.dedup();
    let first = match at {
        Some(at) => at,
        None => numbers.first()?,
    };
    Some(match numbers.iter().filter(|&&n| n != first).count() {
        0 => first.to_owned(),
        more => format!("{first} +{more}"),
    })
}

/// Every call from one module to another, drawn as one arrow as thick as there are
/// calls, whenever it carries a call this view names; colored as the call nearest the
/// start it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tube {
    pub(crate) from: usize,
    pub(crate) to: usize,
    /// Every call from one module to the other, named or not.
    pub(crate) calls: usize,
    pub(crate) flow: Flow,
}

/// A call this view names, from function to function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Link {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) flow: Flow,
}

/// Above this many functions lit, only those within two calls of the start keep
/// their names; the rest are named under the pointer.
const NAMED_ALL: usize = 60;
/// Rows of names an open module stacks before it starts another column.
const ROWS: usize = 8;
/// Between two columns of names in an open module, room for a name.
const COLUMN_GAP: f32 = 11.0;
/// Between two functions named in an open module.
const SPACING: f32 = 2.8;
/// Between the nucleus and a module in the near ring, for its arrows to show.
const NEAR_GAP: f32 = 15.0;
/// Space kept clear between two spheres.
const MARGIN: f32 = 6.0;
/// Seconds a re-centering takes.
const RECENTER_SECONDS: f32 = 0.9;
/// The angle the camera turns to when the nucleus opens.
pub(crate) const VIEW: (f32, f32) = (-0.45, -0.38);
/// Browsing, how far the camera may turn off [`VIEW`], in yaw and in pitch: the map
/// is kept apart for views up to an eighth of a turn either side, and is flattened
/// along the view, so further round it would turn edge-on; turned both ways at once,
/// names crowd well before that.
pub(crate) const BROWSE_REACH: (f32, f32) = (0.6, 0.3);
/// How many times its size the nucleus grows to when expanded.
const GROWTH: f32 = 3.5;
/// How far outside the expanded nucleus's wall the modules round it sit, against its
/// radius.
const RIM_GAP: f32 = 0.14;
/// Kept clear between two modules round the expanded nucleus, against its radius.
const RIM_MARGIN: f32 = 0.08;
/// Between two named calls running side by side along one arrow.
const STRAND_GAP: f32 = 0.45;
/// Kept clear round a sphere an arrow bends round, before the expanded view's zoom.
const DETOUR_CLEAR: f32 = 3.0;
/// Kept clear round a function a call bends round, before the expanded view's zoom.
const NODE_CLEAR: f32 = 0.5;
/// Most bends an arrow takes round what lies in its way.
const DETOUR_BENDS: usize = 4;
/// Kept clear between two module spheres browsing.
const ATLAS_MARGIN: f32 = 5.0;
/// Browsing, how much wider than high the codebase is laid out, as a window is.
const ATLAS_ASPECT: f32 = 1.5;
/// Browsing, how much of its depth along the usual view the layout of modules keeps.
const ATLAS_DEPTH: f32 = 0.35;
/// Browsing, how much further apart across the view two modules are kept for each
/// unit one lies behind the other: about how far off the middle of the window a
/// camera's perspective moves what is further back, at the window's edge.
const ATLAS_SLANT: f32 = 0.7;
/// Between the ring round the nucleus and a module beyond it holding only calls in
/// from outside the journey.
const OUTER_GAP: f32 = 12.0;
/// Of a module's callers from outside the journey (I), how many are drawn and named,
/// those nearest the journey; the rest are a count on the module. An open module keeps
/// this many places free below its names for them, so they come and go moving nothing.
const OUTSIDE_SHOWN: usize = 2;
/// In a journey, how many of the start's callers are named; the rest fold into a
/// count on their module's name, so the journey stays what reads.
const CALLERS_NAMED: usize = 5;
/// The radius of a module of one function: every sphere is a multiple of it.
const SPHERE_UNIT: f32 = 4.0;
/// How fast a sphere grows with the square root of what it holds, past one function.
const SPHERE_GROWTH: f32 = 0.3;
/// The biggest sphere against the smallest: past it, more functions pack closer.
const SPHERE_RATIO: f32 = 3.95;
/// Of a sphere's radius, how much the disc its functions spread over takes, less
/// [`PACK_INSET`]: room at the wall for a dot drawn big when the view is far off.
const PACK_SHARE: f32 = 0.86;
const PACK_INSET: f32 = 0.5;
/// How far apart a module's functions are kept, against the disc's radius over the
/// square root of how many: as near a honeycomb as pushing them apart reaches.
const PACKING: f32 = 1.8;
/// How far the disc a module's functions spread over bows towards the usual view,
/// against how far it could and stay inside.
const PACK_BOW: f32 = 0.3;

/// Where everything is, for one start.
#[derive(Debug, Clone, Default)]
struct Arrangement {
    /// The function gathered round: the journey's start, or browsing, the selection.
    start: Option<usize>,
    /// The module in the middle: the start's.
    middle: usize,
    /// Whether the nucleus is expanded.
    expanded: bool,
    /// By cluster.
    balls: Vec<Ball>,
    /// By tile.
    points: Vec<Vec3>,
    /// Functions drawn, by tile: the nucleus's and those opened around it.
    shown: Vec<bool>,
    /// The functions lit for what they are to the start: its callers and the
    /// journey's calls, each with how many calls it is from the start, -1 for a caller.
    named: HashMap<usize, i32>,
    /// By cluster: the functions an open module names, in the order it lists them.
    columns: Vec<Vec<usize>>,
    /// By cluster: every function an open module could name in this journey, at any
    /// depth, in the order its names are laid out.
    reserved: Vec<Vec<usize>>,
    /// The modules round the nucleus, clockwise from the top.
    ring: Vec<usize>,
    /// In a journey with I on, every function off it that calls one on it.
    outside: HashSet<usize>,
    /// In a journey, the start's callers beyond the few nearest it: drawn quietly and
    /// named only under the pointer, counted on their module's name instead.
    folded: HashSet<usize>,
    /// In a journey, the places each open module keeps free for its callers from
    /// outside it, which names keep off so those come and go covering none.
    free: Vec<Vec3>,
    tubes: Vec<Tube>,
    links: Vec<Link>,
}

/// Browsing, where every module and function sits: one layout of the whole codebase,
/// worked out once, whatever is clicked.
#[derive(Debug, Clone)]
struct Atlas {
    /// By cluster.
    balls: Vec<Ball>,
    /// By tile.
    points: Vec<Vec3>,
}

/// The expanded nucleus's layout for one start: where its functions sit in a sphere
/// of radius one, and the angle round it of each module that could be named there.
#[derive(Debug, Clone)]
struct Laid {
    /// The start, and everything below it at any depth.
    key: (Option<usize>, HashMap<usize, u32>),
    inside: Vec<(usize, Vec3)>,
    angles: HashMap<usize, f32>,
}

/// The graph gathered into module spheres: in a journey, round the start's module;
/// browsing, the whole codebase, lighting what the selection touches. Browsing has no
/// journey, so the selection stands where a start would: what is said of the start
/// below says it of the selection too.
#[derive(Debug, Clone)]
pub(crate) struct Nucleus {
    /// Each module's place in a stable 3D layout of modules, by cluster.
    homes: Vec<Vec3>,
    /// Functions in each module, by cluster.
    counts: Vec<usize>,
    /// Each function's module, by tile.
    module_of: Vec<Option<usize>>,
    /// Where each function sits in its module's sphere, from its middle, by tile:
    /// the same in every view that shows it there, browsing or in a journey.
    offsets: Vec<Vec3>,
    /// Calls from one module to another, by `(from, to)` cluster.
    flows: HashMap<(usize, usize), usize>,
    /// Whether the start's module is expanded to show how calls run inside it.
    expanded: bool,
    now: Arrangement,
    /// The function the pointer has rested on.
    hovered: Option<usize>,
    /// The open journey from the start.
    trail: Trail,
    /// The journey's cursor: marked where it sits, and drawn there though the
    /// arrangement folds it away.
    cursor: Option<usize>,
    /// Whether the graph is browsed, laid out as the atlas has it, rather than
    /// gathered round a journey.
    browse: bool,
    /// The layout browsing uses, once worked out.
    atlas: Option<Atlas>,
    /// In a journey, whether every outside caller into it shows (I).
    outside: bool,
    /// The expanded layout, kept while the start stays.
    laid: Option<Laid>,
    /// Where the spheres and functions were when the last re-centering began.
    from: Option<(Vec<Ball>, Vec<Vec3>)>,
    /// How far along that re-centering is, 0 to 1.
    progress: f32,
}

impl Nucleus {
    #[cfg(test)]
    pub(crate) fn new(graph: &ForceGraph, start: Option<usize>, trail: Trail) -> Self {
        Self::gather(graph, start, trail, false)
    }

    /// Gathers the graph round `start`'s module, naming its callers and `trail`, the
    /// open journey from it; or, to `browse`,
    /// lays every module out where the atlas has it, lighting what `start` touches.
    pub(crate) fn gather(
        graph: &ForceGraph,
        start: Option<usize>,
        trail: Trail,
        browse: bool,
    ) -> Self {
        let clusters = graph.cluster_count();
        let tiles = graph.positions.len();
        let mut counts = vec![0; clusters];
        for tile in 0..tiles {
            if let Some(cluster) = graph.cluster(tile) {
                counts[cluster] += 1;
            }
        }
        let mut flows = HashMap::new();
        for &(from, to) in &graph.calls {
            if let (Some(a), Some(b)) = (graph.cluster(from), graph.cluster(to))
                && a != b
            {
                *flows.entry((a, b)).or_insert(0) += 1;
            }
        }
        let homes = module_homes(clusters, &flows);
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); clusters];
        for tile in 0..tiles {
            if let Some(cluster) = graph.cluster(tile) {
                members[cluster].push(tile);
            }
        }
        let mut offsets = vec![Vec3::ZERO; tiles];
        for (cluster, members) in members.iter().enumerate() {
            for (tile, at) in pack(graph, members, module_radius(counts[cluster])) {
                offsets[tile] = at;
            }
        }
        let mut nucleus = Self {
            homes,
            counts,
            module_of: (0..tiles).map(|t| graph.cluster(t)).collect(),
            offsets,
            flows,
            expanded: false,
            now: Arrangement::default(),
            hovered: None,
            trail,
            cursor: None,
            browse,
            atlas: None,
            outside: false,
            laid: None,
            from: None,
            progress: 1.0,
        };
        nucleus.now = nucleus.arrange(graph, start);
        nucleus
    }

    /// The function the nucleus is gathered round, drawn white: the journey's start,
    /// or browsing, the selection.
    pub(crate) fn start(&self) -> Option<usize> {
        self.now.start
    }

    /// The module in the middle: the start's.
    pub(crate) fn middle(&self) -> usize {
        self.now.middle
    }

    /// Whether the start's module is expanded.
    pub(crate) fn expanded(&self) -> bool {
        self.expanded
    }

    /// Expands the start's module, or shrinks it back among the others, easing there.
    pub(crate) fn expand(&mut self, graph: &ForceGraph, on: bool) {
        if on != self.expanded {
            self.expanded = on;
            self.recenter(graph, self.now.start);
        }
    }

    /// Arranges everything around `start` instead, easing there from wherever things
    /// are now.
    pub(crate) fn recenter(&mut self, graph: &ForceGraph, start: Option<usize>) {
        let balls = (0..self.now.balls.len())
            .filter_map(|c| self.ball(c))
            .collect();
        let points = (0..self.now.points.len())
            .filter_map(|t| self.point(t))
            .collect();
        self.from = Some((balls, points));
        self.progress = 0.0;
        self.now = self.arrange(graph, start);
    }

    /// Names the open journey from the start instead, easing there. Returns whether
    /// that changed anything.
    pub(crate) fn retrace(&mut self, graph: &ForceGraph, trail: Trail) -> bool {
        if trail == self.trail {
            return false;
        }
        self.trail = trail;
        self.recenter(graph, self.now.start);
        true
    }

    /// Whether the graph is browsed rather than gathered round a journey.
    pub(crate) fn browsing(&self) -> bool {
        self.browse
    }

    /// Browses the graph, laid out as the atlas has it, or gathers it round a
    /// journey, easing there. Returns whether that changed.
    pub(crate) fn set_browse(&mut self, graph: &ForceGraph, browse: bool) -> bool {
        if browse == self.browse {
            return false;
        }
        self.browse = browse;
        if browse {
            self.expanded = false;
        }
        self.recenter(graph, self.now.start);
        true
    }

    /// Shows every outside caller into the journey, or leaves them out, without
    /// moving anything already there.
    pub(crate) fn set_outside(&mut self, graph: &ForceGraph, on: bool) {
        if on != self.outside {
            self.outside = on;
            self.recenter(graph, self.now.start);
        }
    }

    /// In a journey, the places the open modules keep free for callers from outside
    /// it, as they will be once a re-centering ends.
    pub(crate) fn kept_free(&self) -> &[Vec3] {
        &self.now.free
    }

    /// Whether a function is an outside caller into the journey, shown.
    pub(crate) fn outside(&self, tile: usize) -> bool {
        self.now.outside.contains(&tile)
    }

    /// Whether a function is one of the start's callers folded into its module's
    /// count: drawn quietly, named only under the pointer.
    pub(crate) fn folded(&self, tile: usize) -> bool {
        self.now.folded.contains(&tile)
    }

    /// How many of a module's functions are the start's callers folded into a count.
    pub(crate) fn folded_in(&self, cluster: usize) -> usize {
        self.now
            .folded
            .iter()
            .filter(|&&t| self.module_of.get(t).copied().flatten() == Some(cluster))
            .count()
    }

    /// Marks the journey's cursor where it sits, without moving anything else.
    /// Returns whether that changed.
    pub(crate) fn set_cursor(&mut self, tile: Option<usize>) -> bool {
        if tile == self.cursor {
            return false;
        }
        self.cursor = tile;
        true
    }

    /// The journey's cursor, marked where it sits.
    pub(crate) fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    fn is_cursor(&self, tile: usize) -> bool {
        self.cursor == Some(tile)
    }

    /// The function the pointer has rested on.
    pub(crate) fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// Marks the function the pointer has rested on. Returns whether that changed.
    pub(crate) fn hover(&mut self, tile: Option<usize>) -> bool {
        if tile == self.hovered {
            return false;
        }
        self.hovered = tile;
        true
    }

    /// Whether a function is drawn full size: a caller or a call of the journey's
    /// (it has a [`Self::level`]), the journey's cursor, or an outside caller into
    /// the journey.
    pub(crate) fn lit(&self, tile: usize) -> bool {
        self.level(tile).is_some() || self.is_cursor(tile) || self.outside(tile)
    }

    /// Whether a function is drawn at all: placed where it shows, and in a journey,
    /// not a speck, which only browsing draws. The one test of what is drawn: what
    /// is pointed at, clicked and named is drawn.
    pub(crate) fn drawn(&self, tile: usize) -> bool {
        self.placed(tile) && (self.browse || !self.speck(tile))
    }

    /// Whether a function placed where it shows is only a small, faint speck: not
    /// the start, and lit by nothing.
    pub(crate) fn speck(&self, tile: usize) -> bool {
        self.placed(tile) && !self.lit(tile) && self.now.start != Some(tile)
    }

    /// Whether a function carries its name without the pointer on it: every lit one
    /// does, unless so many are lit that only those within two calls of the start
    /// keep theirs; and none of the start's callers folded into a count. The one
    /// test of what is named.
    pub(crate) fn named(&self, tile: usize) -> bool {
        !self.folded(tile)
            && self
                .level(tile)
                .is_some_and(|level| self.now.named.len() <= NAMED_ALL || level.abs() <= 2)
    }

    /// Moves a re-centering along by `dt` seconds. Returns whether anything moved.
    pub(crate) fn advance(&mut self, dt: f32) -> bool {
        if self.progress >= 1.0 {
            return false;
        }
        self.progress = (self.progress + dt / RECENTER_SECONDS).min(1.0);
        if self.progress >= 1.0 {
            self.from = None;
        }
        true
    }

    /// The share of the way to where things are going, eased in and out.
    fn eased(&self) -> f32 {
        let t = self.progress.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Where a function is drawn: inside its module's sphere, or at its middle when
    /// it is folded away; the journey's cursor, folded away, just under its module.
    pub(crate) fn point(&self, tile: usize) -> Option<Vec3> {
        if self.is_cursor(tile) && !self.now.shown.get(tile).copied().unwrap_or(false) {
            return self.under(tile);
        }
        let to = *self.now.points.get(tile)?;
        Some(match &self.from {
            Some((_, points)) => points.get(tile).map_or(to, |&p| p.lerp(to, self.eased())),
            None => to,
        })
    }

    /// Just under a function's module as the usual view sees it, where a function
    /// the arrangement folds away shows without moving anything: clear of the
    /// module's name on top and of the arrows leaving its sides.
    fn under(&self, tile: usize) -> Option<Vec3> {
        let ball = self.ball(self.module_of.get(tile).copied().flatten()?)?;
        let down = view_rotation() * Vec3::NEG_Y;
        Some(ball.center + down * (ball.radius + 0.8 * self.zoom()))
    }

    /// A module's sphere as drawn now.
    pub(crate) fn ball(&self, cluster: usize) -> Option<Ball> {
        let to = *self.now.balls.get(cluster)?;
        Some(match self.from.as_ref().and_then(|(b, _)| b.get(cluster)) {
            Some(from) => Ball {
                center: from.center.lerp(to.center, self.eased()),
                radius: from.radius + (to.radius - from.radius) * self.eased(),
                body: to.body,
            },
            None => to,
        })
    }

    /// Every module's sphere as drawn now, by cluster.
    pub(crate) fn balls(&self) -> impl Iterator<Item = (usize, Ball)> + '_ {
        (0..self.now.balls.len()).filter_map(|c| self.ball(c).map(|b| (c, b)))
    }

    /// Whether a module sphere of this body is drawn: in a journey, only those it
    /// runs through.
    pub(crate) fn shows_body(&self, body: Body) -> bool {
        self.browse || body != Body::Closed
    }

    /// Whether a function is placed where it shows, rather than folded away inside
    /// its module: what the arrangement shows, and the cursor wherever it is.
    fn placed(&self, tile: usize) -> bool {
        self.is_cursor(tile) || self.now.shown.get(tile).copied().unwrap_or(false)
    }

    /// How many calls a lit function is from the start: -1 for a caller, else its
    /// shallowest level in the journey.
    pub(crate) fn level(&self, tile: usize) -> Option<i32> {
        self.now.named.get(&tile).copied()
    }

    /// A function's numbers in the open journey from the start, in order.
    pub(crate) fn numbers(&self, tile: usize) -> &[String] {
        self.trail.numbers.get(&tile).map_or(&[], Vec::as_slice)
    }

    /// How big a function of `dot`'s role is drawn in the world, before the bounds
    /// its size keeps to on screen: bigger in the expanded nucleus, as the view is
    /// nearer.
    pub(crate) fn dot_radius(&self, dot: Dot) -> f32 {
        dot.radius() * self.zoom()
    }

    /// How much bigger arrows and their heads are drawn, so they keep their size on
    /// screen in the expanded nucleus.
    pub(crate) fn zoom(&self) -> f32 {
        if self.now.expanded { GROWTH * 0.5 } else { 1.0 }
    }

    /// How many functions a module holds.
    pub(crate) fn count(&self, cluster: usize) -> usize {
        self.counts.get(cluster).copied().unwrap_or(0)
    }

    /// How many functions an open module names.
    pub(crate) fn opened(&self, cluster: usize) -> usize {
        self.now.columns.get(cluster).map_or(0, Vec::len)
    }

    pub(crate) fn tubes(&self) -> &[Tube] {
        &self.now.tubes
    }

    pub(crate) fn links(&self) -> &[Link] {
        &self.now.links
    }

    /// Where the arrow from one module to another starts and ends, on their spheres
    /// as drawn now, and how thick it is. An arrow running each way between the same
    /// two modules keeps to its own side.
    pub(crate) fn tube_ends(&self, tube: &Tube) -> Option<(Vec3, Vec3, f32)> {
        let (a, b) = (self.ball(tube.from)?, self.ball(tube.to)?);
        let along = (b.center - a.center).normalize_or_zero();
        if along == Vec3::ZERO {
            return None;
        }
        let radius = tube_radius(tube.calls) * self.zoom();
        let both_ways = self
            .now
            .tubes
            .iter()
            .any(|t| t.from == tube.to && t.to == tube.from);
        let side = if both_ways {
            sideways(along) * (self.half_width(tube) + 0.4 * self.zoom())
        } else {
            Vec3::ZERO
        };
        // Leaving and entering where the spheres' surfaces are, shifted to the side.
        let start = a.center + along * a.radius + side;
        let end = b.center - along * b.radius + side;
        Some((start, end, radius))
    }

    /// How many named calls run along an arrow.
    fn strands(&self, tube: &Tube) -> usize {
        self.now
            .links
            .iter()
            .filter(|l| self.tube_of(l) == Some((tube.from, tube.to)))
            .count()
    }

    /// Half the width the named calls along an arrow take, side by side, or its own
    /// when that is wider.
    fn half_width(&self, tube: &Tube) -> f32 {
        #[expect(clippy::cast_precision_loss, reason = "a short list")]
        let spread = self.strands(tube).saturating_sub(1) as f32 * STRAND_GAP / 2.0;
        (spread * self.zoom()).max(tube_radius(tube.calls) * self.zoom())
    }

    /// The pair of modules a named call runs between, when they differ.
    fn tube_of(&self, link: &Link) -> Option<(usize, usize)> {
        let cluster = |tile: usize| self.module_of.get(tile).copied().flatten();
        let (from, to) = (cluster(link.from)?, cluster(link.to)?);
        (from != to).then_some((from, to))
    }

    /// The path an arrow between modules runs along, from where it leaves one to where
    /// it enters the other, bent round every sphere drawn that lies in its way as the
    /// usual view sees it, and how thick it is.
    pub(crate) fn tube_path(&self, tube: &Tube) -> Option<(Vec<Vec3>, f32)> {
        let (start, end, radius) = self.tube_ends(tube)?;
        let spheres: Vec<(Vec3, f32)> = self
            .balls()
            .filter(|&(c, b)| c != tube.from && c != tube.to && self.shows_body(b.body))
            .map(|(_, b)| (b.center, b.radius))
            .collect();
        Some((
            detour(vec![start, end], &spheres, DETOUR_CLEAR * self.zoom()),
            radius,
        ))
    }

    /// The functions drawn full size whose disc a call should keep off: lit, the
    /// start, the cursor, and the outside callers shown; each with where it is drawn
    /// and how big.
    fn discs_in_way(&self, except: [usize; 2]) -> Vec<(Vec3, f32)> {
        self.now
            .named
            .keys()
            .chain(&self.now.outside)
            .chain(self.now.start.iter())
            .chain(self.cursor.iter())
            .filter(|t| !except.contains(t) && self.placed(**t))
            .filter_map(|&t| Some((self.point(t)?, self.dot_radius(Dot::Lit))))
            .collect()
    }

    /// The path a named call is drawn along: within a module, from one function to
    /// the other; else from the caller to its module's surface, along the arrow
    /// between the two modules, and in to the callee, each call on its own strand
    /// beside the others along the arrow. An end folded away inside its module stops
    /// at the module's surface. Within a module it bends round the other functions
    /// drawn there, so a call never reads as another's.
    pub(crate) fn route(&self, link: &Link) -> Vec<Vec3> {
        let (Some(a), Some(b)) = (self.point(link.from), self.point(link.to)) else {
            return Vec::new();
        };
        let discs = self.discs_in_way([link.from, link.to]);
        let clear = NODE_CLEAR * self.zoom();
        let Some((from, to)) = self.tube_of(link) else {
            return detour(vec![a, b], &discs, clear);
        };
        let tube = self.now.tubes.iter().find(|t| t.from == from && t.to == to);
        let Some((tube, (path, _))) = tube.and_then(|t| Some((t, self.tube_path(t)?))) else {
            return detour(vec![a, b], &discs, clear);
        };
        let (Some(&start), Some(&end)) = (path.first(), path.last()) else {
            return vec![a, b];
        };
        let strand = self
            .now
            .links
            .iter()
            .take_while(|l| *l != link)
            .filter(|l| self.tube_of(l) == Some((from, to)))
            .count();
        let along = (end - start).normalize_or_zero();
        #[expect(clippy::cast_precision_loss, reason = "a short list")]
        let spread =
            (strand as f32 - self.strands(tube).saturating_sub(1) as f32 / 2.0) * STRAND_GAP;
        let offset = sideways(along).cross(along) * spread * self.zoom();
        let mut route = Vec::with_capacity(path.len() + 4);
        if self.placed(link.from) {
            route.extend(detour(vec![a, start + offset], &discs, clear));
            route.pop();
        }
        route.extend(path.iter().map(|&p| p + offset));
        if self.placed(link.to) {
            route.extend(
                detour(vec![end + offset, b], &discs, clear)
                    .into_iter()
                    .skip(1),
            );
        }
        route
    }

    /// Points around every sphere that should be on screen, the nucleus and the open
    /// modules, as they will be once a re-centering ends.
    pub(crate) fn frame_points(&self, rotation: Quat) -> Vec<Vec3> {
        let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
        self.now
            .balls
            .iter()
            .filter(|b| matches!(b.body, Body::Nucleus | Body::Open))
            .flat_map(|b| {
                // Names hang to the right of an open module's functions.
                let room = if b.body == Body::Open { 2.2 } else { 1.0 };
                [
                    b.center + right * b.radius * room,
                    b.center - right * b.radius,
                    b.center + up * b.radius,
                    b.center - up * b.radius,
                ]
            })
            .collect()
    }

    /// Points round the nucleus alone, as it will be once a re-centering ends: what
    /// the view takes in to fill itself with the expanded module.
    pub(crate) fn nucleus_points(&self, rotation: Quat) -> Vec<Vec3> {
        let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
        self.now
            .balls
            .get(self.now.middle)
            .map(|b| {
                vec![
                    b.center + right * b.radius,
                    b.center - right * b.radius,
                    b.center + up * b.radius,
                    b.center - up * b.radius,
                ]
            })
            .unwrap_or_default()
    }

    /// Points round every sphere browsing shows, the whole codebase.
    pub(crate) fn atlas_points(&self, rotation: Quat) -> Vec<Vec3> {
        let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
        self.now
            .balls
            .iter()
            .flat_map(|b| {
                [
                    b.center + (right + up) * b.radius,
                    b.center - (right + up) * b.radius,
                ]
            })
            .collect()
    }

    /// Expanded in a journey, the function the view is centred on: the cursor while
    /// it is inside the expanded module, so whatever moves it there, a click, a key
    /// or a bar, centres the view on it the same way; else the start.
    pub(crate) fn centre(&self) -> Option<usize> {
        if !self.now.expanded || self.browse {
            return None;
        }
        let inside =
            |tile: usize| self.module_of.get(tile).copied().flatten() == Some(self.now.middle);
        match self.cursor {
            Some(cursor) if inside(cursor) => Some(cursor),
            _ => self.now.start,
        }
    }

    /// Where the camera looks: expanded in a journey, the function it centres on; the
    /// nucleus in a journey otherwise; browsing, the middle of the modules the
    /// selection touches.
    pub(crate) fn frame_target(&self) -> Vec3 {
        if let Some(at) = self.centre().and_then(|t| self.now.points.get(t).copied()) {
            return at;
        }
        let open: Vec<Vec3> = self
            .now
            .balls
            .iter()
            .filter(|b| self.browse && b.body == Body::Open)
            .map(|b| b.center)
            .collect();
        if open.is_empty() {
            return Vec3::ZERO;
        }
        #[expect(clippy::cast_precision_loss, reason = "a module count")]
        let middle = open.iter().copied().sum::<Vec3>() / open.len() as f32;
        middle
    }

    /// Everything placed around `start`, the nucleus expanded or not, or browsing.
    fn arrange(&mut self, graph: &ForceGraph, start: Option<usize>) -> Arrangement {
        if self.browse {
            return self.arrange_browse(graph, start);
        }
        let modules = self.arrange_modules(graph, start);
        if self.expanded {
            let laid = self.laid_out(graph, &modules);
            self.arrange_inside(graph, modules, &laid)
        } else {
            modules
        }
    }

    /// Where the expanded nucleus puts its functions and the modules round it, worked
    /// out once for each start from everything it could name at any depth: a new
    /// depth, outside callers or the pointer moving then only names and draws, and
    /// moves nothing.
    fn laid_out(&mut self, graph: &ForceGraph, modules: &Arrangement) -> Laid {
        let key = (modules.start, self.trail.whole.clone());
        if let Some(laid) = self.laid.as_ref().filter(|l| l.key == key) {
            return laid.clone();
        }
        let middle = modules.middle;
        let cluster_of = |tile: usize| graph.cluster(tile).unwrap_or(middle);
        let before = modules.balls[middle].radius;
        // Everything below the start at any depth, and every caller.
        let mut reach: HashMap<usize, i32> = HashMap::new();
        if let Some(start) = modules.start {
            reach.extend(
                self.trail
                    .whole
                    .iter()
                    .map(|(&t, &l)| (t, i32::try_from(l).unwrap_or(i32::MAX))),
            );
            for caller in graph.callers(start) {
                reach.entry(caller).or_insert(-1);
            }
            reach.insert(start, 0);
        }
        let members: Vec<usize> = (0..graph.positions.len())
            .filter(|&t| cluster_of(t) == middle)
            .collect();
        let calls: Vec<(usize, usize)> = graph
            .calls
            .iter()
            .copied()
            .filter(|&(a, b)| a != b && cluster_of(a) == middle && cluster_of(b) == middle)
            .collect();
        // From where they sit collapsed, in a sphere of radius one.
        let seeded: Vec<Vec3> = members.iter().map(|&t| self.offsets[t] / before).collect();
        // How big each is drawn there at most, against the sphere, with room for a
        // ring.
        let core = before * GROWTH;
        let size = (Dot::Focus.radius() * GROWTH * 0.5 + 1.0) / core;
        let inside = lay_members(&members, &seeded, &calls, &reach, size);
        // Each module that could ever be named round it, clockwise in the same order
        // as round the nucleus collapsed, each as big as all it could name.
        let rim: Vec<usize> = modules
            .ring
            .iter()
            .copied()
            .filter(|&c| !modules.reserved[c].is_empty())
            .collect();
        let radii: Vec<f32> = rim.iter().map(|&c| modules.balls[c].radius).collect();
        let angles = rim_angles(&radii, before * GROWTH);
        let laid = Laid {
            key,
            inside: members.into_iter().zip(inside).collect(),
            angles: rim.into_iter().zip(angles).collect(),
        };
        self.laid = Some(laid.clone());
        laid
    }

    /// Everything placed around `start`, as the module view has it.
    fn arrange_modules(&self, graph: &ForceGraph, start: Option<usize>) -> Arrangement {
        let clusters = self.homes.len();
        let middle = start
            .and_then(|f| graph.cluster(f))
            .or_else(|| (0..clusters).max_by_key(|&c| (self.counts[c], Reverse(c))))
            .unwrap_or(0);
        let cluster_of = |tile: usize| graph.cluster(tile).unwrap_or(middle);
        // Laid out for every depth, so showing a level more or less only draws or
        // stops drawing, and moves nothing.
        let mut links = start.map_or_else(Vec::new, |f| self.named_calls(graph, f));
        let every = links.clone();
        let named = distances(&links, start);
        let folded = start.map_or_else(HashSet::new, |f| self.fold_callers(graph, f, &cluster_of));
        let laid = self.reserve(&every, start, clusters, middle, &cluster_of);
        let columns = open_up(&links, start, clusters, middle, &cluster_of);

        let mut balls: Vec<Ball> = (0..clusters)
            .map(|c| Ball {
                center: Vec3::ZERO,
                radius: module_radius(self.counts[c]),
                body: Body::Closed,
            })
            .collect();
        balls[middle].body = Body::Nucleus;
        let bearings = Bearings {
            homes: &self.homes,
            middle,
            back: view_rotation() * Vec3::Z,
        };
        for (cluster, column) in laid.iter().enumerate() {
            if cluster != middle && !column.is_empty() {
                balls[cluster].body = Body::Open;
            }
        }
        let order = self.ring_order(&every, middle, &cluster_of);
        place_ring(&mut balls, &order, middle);
        place_closed(&mut balls, &bearings);
        let (mut points, mut shown) = self.place_functions(&balls, &laid, middle, &cluster_of);
        // What only a deeper level names keeps its place, undrawn.
        for (cluster, column) in laid.iter().enumerate() {
            for tile in column.iter().filter(|t| !columns[cluster].contains(t)) {
                shown[*tile] = false;
            }
            if cluster != middle && columns[cluster].is_empty() {
                balls[cluster].body = Body::Closed;
            }
        }
        let calls_in = self.outside_calls(graph, &links, start);
        let outside = self.place_outside(
            &mut balls,
            (&mut points, &mut shown),
            (&calls_in, &laid),
            middle,
            &cluster_of,
        );
        links.extend(calls_in);
        let tubes = self.join(&links, &cluster_of);
        let free = kept_free(&balls, &laid);
        Arrangement {
            start,
            middle,
            expanded: false,
            balls,
            points,
            shown,
            named,
            columns,
            reserved: laid,
            ring: order,
            outside,
            folded,
            free,
            tubes,
            links,
        }
    }

    /// Browsing: every module where the atlas has it, every function inside its
    /// module, and what `start` touches lit where it sits: its callers and its own
    /// calls, named, and their modules opened. Nothing moves for a new start: the
    /// atlas leaves each function room enough to be lit and named where it is.
    fn arrange_browse(&mut self, graph: &ForceGraph, start: Option<usize>) -> Arrangement {
        let (homes, counts) = (&self.homes, &self.counts);
        let (offsets, module_of) = (&self.offsets, &self.module_of);
        let atlas = self
            .atlas
            .get_or_insert_with(|| atlas(homes, counts, offsets, module_of))
            .clone();
        let clusters = self.homes.len();
        let middle = start.and_then(|f| graph.cluster(f)).unwrap_or(0);
        let cluster_of = |tile: usize| graph.cluster(tile).unwrap_or(middle);
        let links = start.map_or_else(Vec::new, |f| Self::browse_calls(graph, f));
        let named = distances(&links, start);
        // Every module counts what it names, the start's own among them.
        let columns = open_up(&links, start, clusters, usize::MAX, &cluster_of);
        let mut balls = atlas.balls;
        for (cluster, ball) in balls.iter_mut().enumerate() {
            if (start.is_some() && cluster == middle) || !columns[cluster].is_empty() {
                ball.body = Body::Open;
            }
        }
        let shown = (0..graph.positions.len())
            .map(|t| graph.cluster(t).is_some())
            .collect();
        let tubes = if start.is_some() {
            self.join(&links, &cluster_of)
        } else {
            self.map_tubes()
        };
        Arrangement {
            start,
            middle,
            expanded: false,
            balls,
            points: atlas.points,
            shown,
            named,
            reserved: columns.clone(),
            ring: Vec::new(),
            columns,
            outside: HashSet::new(),
            folded: HashSet::new(),
            free: Vec::new(),
            tubes,
            links,
        }
    }

    /// Browsing with nothing selected, how modules depend on each other: an arrow
    /// from each module to every module it calls, as thick and as plain as the calls
    /// it carries, so a weak dependency shows faint rather than not at all.
    fn map_tubes(&self) -> Vec<Tube> {
        let mut tubes: Vec<Tube> = self
            .flows
            .iter()
            .map(|(&(from, to), &calls)| Tube {
                from,
                to,
                calls,
                flow: Flow::Module,
            })
            .collect();
        tubes.sort_unstable_by_key(|t| (t.from, t.to));
        tubes
    }

    /// Each of `start`'s callers' calls into it, once.
    fn calls_into(graph: &ForceGraph, start: usize) -> Vec<Link> {
        graph
            .callers(start)
            .map(|from| Link {
                from,
                to: start,
                flow: Flow::Into,
            })
            .collect()
    }

    /// Browsing, the calls named round `start`: each of its callers' into it, then each
    /// of its own calls, in the order written.
    fn browse_calls(graph: &ForceGraph, start: usize) -> Vec<Link> {
        let mut links = Self::calls_into(graph, start);
        for &to in graph.sequence(start) {
            if !links.iter().any(|l| l.from == start && l.to == to) {
                links.push(Link {
                    from: start,
                    to,
                    flow: Flow::Journey(1),
                });
            }
        }
        links
    }

    /// In a journey with I on, a call into each function shown on it, the start
    /// included, from each function that calls it and is not shown: of each module's
    /// such callers, only the few nearest the journey.
    fn outside_calls(&self, graph: &ForceGraph, links: &[Link], start: Option<usize>) -> Vec<Link> {
        self.outside_split(graph, links, start).0
    }

    /// In a journey with I on, how many callers from outside it each module holds
    /// beyond those drawn, by cluster.
    pub(crate) fn outside_left_out(&self, graph: &ForceGraph) -> HashMap<usize, usize> {
        let links: Vec<Link> = self
            .now
            .links
            .iter()
            .filter(|l| l.flow != Flow::Outside)
            .copied()
            .collect();
        self.outside_split(graph, &links, self.now.start).1
    }

    /// The calls in from outside the journey that are drawn, and how many callers
    /// each module holds beyond them. A module's callers are ranked by the shallowest
    /// function on the journey they call, the start first, then by how many of its
    /// functions they call.
    fn outside_split(
        &self,
        graph: &ForceGraph,
        links: &[Link],
        start: Option<usize>,
    ) -> (Vec<Link>, HashMap<usize, usize>) {
        let Some(start) = start.filter(|_| self.outside && !self.browse) else {
            return (Vec::new(), HashMap::new());
        };
        // Each function on the journey, with its level: the start's 0.
        let mut on: Vec<(usize, u32)> = vec![(start, 0)];
        for link in links {
            if let Flow::Journey(level) = link.flow {
                match on.iter_mut().find(|(tile, _)| *tile == link.to) {
                    Some(known) => known.1 = known.1.min(level),
                    None => on.push((link.to, level)),
                }
            }
        }
        let shown: HashSet<usize> = links
            .iter()
            .flat_map(|l| [l.from, l.to])
            .chain([start])
            .collect();
        let mut calls = Vec::new();
        // Each caller's nearest level on the journey, and how many functions on it it
        // calls.
        let mut near: HashMap<usize, (u32, usize)> = HashMap::new();
        for &(to, level) in &on {
            let callers: Vec<usize> = graph.callers(to).filter(|c| !shown.contains(c)).collect();
            for &from in &callers {
                let known = near.entry(from).or_insert((level, 0));
                known.0 = known.0.min(level);
                known.1 += 1;
            }
            calls.extend(callers.into_iter().map(|from| Link {
                from,
                to,
                flow: Flow::Outside,
            }));
        }
        let mut by_module: HashMap<usize, Vec<usize>> = HashMap::new();
        for &caller in near.keys() {
            if let Some(cluster) = graph.cluster(caller) {
                by_module.entry(cluster).or_default().push(caller);
            }
        }
        let mut kept = HashSet::new();
        let mut left_out = HashMap::new();
        for (cluster, mut callers) in by_module {
            callers.sort_by_key(|c| (near[c].0, Reverse(near[c].1), *c));
            if callers.len() > OUTSIDE_SHOWN {
                left_out.insert(cluster, callers.len() - OUTSIDE_SHOWN);
            }
            kept.extend(callers.into_iter().take(OUTSIDE_SHOWN));
        }
        calls.retain(|l| kept.contains(&l.from) || graph.cluster(l.from).is_none());
        (calls, left_out)
    }

    /// The nucleus grown from `modules`, the module view, to several times its size,
    /// its functions laid out by the calls among them; the modules naming anything on
    /// a ring just outside its wall, at the bearings they had, shrunk; and every other
    /// module pushed out by as much as it grew. It names what the module view names.
    fn arrange_inside(&self, graph: &ForceGraph, modules: Arrangement, laid: &Laid) -> Arrangement {
        let middle = modules.middle;
        let cluster_of = |tile: usize| graph.cluster(tile).unwrap_or(middle);
        let before = modules.balls[middle].radius;
        let core = before * GROWTH;
        // Every module the journey could name round it at any depth keeps its place
        // there; one naming nothing yet is left out, undrawn, where it stands.
        let rim: Vec<usize> = (0..self.homes.len())
            .filter(|&c| c != middle && !modules.reserved[c].is_empty())
            .collect();

        let mut balls = modules.balls.clone();
        for (cluster, ball) in balls.iter_mut().enumerate() {
            if cluster == middle {
                ball.radius = core;
            } else {
                // Out of the way by as much as the nucleus grew.
                let out = ball.center.try_normalize().unwrap_or(Vec3::Y);
                ball.center += out * (core - before);
                ball.body = Body::Closed;
            }
        }
        place_rim(&mut balls, &rim, &laid.angles, core);

        let tiles = graph.positions.len();
        let mut points: Vec<Vec3> = (0..tiles).map(|t| balls[cluster_of(t)].center).collect();
        let mut shown = vec![false; tiles];
        for &(tile, at) in &laid.inside {
            points[tile] = at * core;
            shown[tile] = true;
        }
        for &cluster in &rim {
            let column = &modules.reserved[cluster];
            for (&tile, at) in column
                .iter()
                .zip(names_grid(column.len(), balls[cluster].radius))
            {
                points[tile] = balls[cluster].center + at;
                shown[tile] = modules.columns[cluster].contains(&tile);
            }
            if modules.columns[cluster].is_empty() {
                balls[cluster].body = Body::Closed;
            }
        }
        let calls_in: Vec<Link> = modules
            .links
            .iter()
            .copied()
            .filter(|l| l.flow == Flow::Outside)
            .collect();
        let outside = self.place_outside(
            &mut balls,
            (&mut points, &mut shown),
            (&calls_in, &modules.reserved),
            middle,
            &cluster_of,
        );
        let tubes = self.join(&modules.links, &cluster_of);
        let free = kept_free(&balls, &modules.reserved);
        Arrangement {
            expanded: true,
            balls,
            points,
            shown,
            outside,
            free,
            tubes,
            ..modules
        }
    }

    /// Of the callers of `start`, the journey's start, those past the few nearest the
    /// journey: those that call most of the journey's functions, then call the start
    /// most often, then live in its module, are named; the rest fold into a count on
    /// their module's name. Worked out from every caller and the journey at every
    /// depth, so a new depth or the cursor never changes which.
    fn fold_callers(
        &self,
        graph: &ForceGraph,
        start: usize,
        cluster_of: &impl Fn(usize) -> usize,
    ) -> HashSet<usize> {
        // One the journey runs too is named as the journey's.
        let mut callers: Vec<usize> = graph
            .callers(start)
            .filter(|t| !self.trail.whole.contains_key(t))
            .collect();
        // Folding one or two away would save nothing.
        if callers.len() <= CALLERS_NAMED + 2 {
            return HashSet::new();
        }
        let home = cluster_of(start);
        callers.sort_by_cached_key(|&caller| {
            let into_journey: HashSet<usize> = graph
                .callees(caller)
                .filter(|t| self.trail.whole.contains_key(t))
                .collect();
            let sites = graph
                .sequence(caller)
                .iter()
                .filter(|&&t| t == start)
                .count();
            (
                Reverse(into_journey.len()),
                Reverse(sites),
                cluster_of(caller) != home,
                caller,
            )
        });
        callers.split_off(CALLERS_NAMED).into_iter().collect()
    }

    /// The calls named round `start`: each of its callers' into it, then each of the
    /// open journey's from it, in the journey's order.
    fn named_calls(&self, graph: &ForceGraph, start: usize) -> Vec<Link> {
        let mut links = Self::calls_into(graph, start);
        for call in &self.trail.calls {
            if call.from != call.to && !links.iter().any(|l| l.from == call.from && l.to == call.to)
            {
                links.push(Link {
                    from: call.from,
                    to: call.to,
                    flow: Flow::journey(call.level),
                });
            }
        }
        links
    }

    /// An arrow for each pair of modules a named call runs between, as thick as all
    /// the calls between them. A call in from outside the journey runs straight, on
    /// no arrow, so the journey's own keep the arrows to themselves.
    fn join(&self, links: &[Link], cluster_of: &impl Fn(usize) -> usize) -> Vec<Tube> {
        let mut tubes: Vec<Tube> = Vec::new();
        for link in links.iter().filter(|l| l.flow != Flow::Outside) {
            let (from, to) = (cluster_of(link.from), cluster_of(link.to));
            if from == to {
                continue;
            }
            match tubes.iter_mut().find(|t| t.from == from && t.to == to) {
                // The call nearest the start colors the arrow.
                Some(tube) if link.flow.rank() < tube.flow.rank() => tube.flow = link.flow,
                Some(_) => {}
                None => tubes.push(Tube {
                    from,
                    to,
                    calls: self.flows.get(&(from, to)).copied().unwrap_or(1),
                    flow: link.flow,
                }),
            }
        }
        tubes
    }
}

/// Which way each module lies from the one in the middle, by the stable layout of
/// modules, as the usual view sees it from `back`.
struct Bearings<'a> {
    homes: &'a [Vec3],
    middle: usize,
    back: Vec3,
}

impl Bearings<'_> {
    /// The way to a module's home, split into across the usual view and towards it.
    fn split(&self, cluster: usize) -> (Vec3, f32) {
        let full = (self.homes[cluster] - self.homes[self.middle])
            .try_normalize()
            .unwrap_or_else(|| fibonacci(cluster, self.homes.len()));
        let toward = full.dot(self.back);
        let across = (full - self.back * toward)
            .try_normalize()
            .unwrap_or_else(|| sideways(self.back));
        (across, toward)
    }

    /// For the rest of the codebase: standing back behind the nucleus, a bowl seen
    /// from the usual view, so it never crowds in front of what matters.
    fn behind(&self, cluster: usize) -> Vec3 {
        let (across, toward) = self.split(cluster);
        (across - self.back * (0.3 + 0.7 * toward.abs())).normalize()
    }

    /// How far a module's home is from the middle's, against the typical module's.
    fn remoteness(&self, cluster: usize) -> f32 {
        let distance = |c: usize| self.homes[c].distance(self.homes[self.middle]);
        let mut spans: Vec<f32> = (0..self.homes.len())
            .filter(|&c| c != self.middle)
            .map(distance)
            .collect();
        spans.sort_by(f32::total_cmp);
        let typical = spans.get(spans.len() / 2).copied().unwrap_or(1.0).max(1.0);
        distance(cluster) / typical
    }
}

/// How many calls each function `links` names is from `start`: -1 for a caller,
/// else its shallowest level in the journey, capped at [`SHADES`]; the start left out.
fn distances(links: &[Link], start: Option<usize>) -> HashMap<usize, i32> {
    let mut named: HashMap<usize, i32> = HashMap::new();
    for link in links {
        let (tile, level) = match link.flow {
            Flow::Into => (link.from, -1),
            #[expect(clippy::cast_possible_wrap, reason = "a small level")]
            Flow::Journey(level) => (link.to, level as i32),
            // An outside caller is shown, not named.
            Flow::Outside | Flow::Module => continue,
        };
        if Some(tile) == start {
            continue;
        }
        let at = named.entry(tile).or_insert(level);
        if level.abs() < at.abs() {
            *at = level;
        }
    }
    named
}

/// The functions each module outside the middle one opens to name, by cluster, in the
/// order `links` reaches them.
fn open_up(
    links: &[Link],
    start: Option<usize>,
    clusters: usize,
    middle: usize,
    cluster_of: &impl Fn(usize) -> usize,
) -> Vec<Vec<usize>> {
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); clusters];
    for link in links {
        for tile in [link.from, link.to] {
            let cluster = cluster_of(tile);
            if Some(tile) != start && cluster != middle && !columns[cluster].contains(&tile) {
                columns[cluster].push(tile);
            }
        }
    }
    columns
}

impl Nucleus {
    /// Every function below the start at any depth, in the order the journey first
    /// runs it, then any `links` name that the trail's order leaves out.
    fn journey_order<'a>(&'a self, links: &'a [Link]) -> impl Iterator<Item = usize> + 'a {
        self.trail.order.iter().copied().chain(
            links
                .iter()
                .filter(|l| matches!(l.flow, Flow::Journey(_)))
                .map(|l| l.to),
        )
    }

    /// The functions each module outside the middle one could ever name in this
    /// journey, by cluster: the start's callers in `links` first, then everything the
    /// journey runs below the start at any depth, the shallowest first and, level by level,
    /// in the order the journey first runs them. Each module is laid out for all of
    /// them once, so a level more or less shows or hides names in place, and the
    /// names a shallower depth shows come first, at the top.
    fn reserve(
        &self,
        links: &[Link],
        start: Option<usize>,
        clusters: usize,
        middle: usize,
        cluster_of: &impl Fn(usize) -> usize,
    ) -> Vec<Vec<usize>> {
        let mut columns: Vec<Vec<usize>> = vec![Vec::new(); clusters];
        let callers = links
            .iter()
            .filter(|l| l.flow == Flow::Into)
            .map(|l| l.from);
        let mut journey: Vec<usize> = Vec::new();
        for tile in self.journey_order(links) {
            if !journey.contains(&tile) {
                journey.push(tile);
            }
        }
        // A stable sort: level by level, in the journey's order within each.
        journey.sort_by_key(|t| self.trail.whole.get(t).copied().unwrap_or(u32::MAX));
        for tile in callers.chain(journey) {
            let cluster = cluster_of(tile);
            if Some(tile) != start && cluster != middle && !columns[cluster].contains(&tile) {
                columns[cluster].push(tile);
            }
        }
        columns
    }

    /// The modules round the nucleus, in order: those the journey enters at any
    /// depth, in the order it first enters them, the shallower first; then those
    /// holding only callers of the start, from `links`. The same at every depth,
    /// collapsed or expanded.
    fn ring_order(
        &self,
        links: &[Link],
        middle: usize,
        cluster_of: &impl Fn(usize) -> usize,
    ) -> Vec<usize> {
        let callers = links
            .iter()
            .filter(|l| l.flow == Flow::Into)
            .map(|l| l.from);
        // Level by level, as the panel's rows run, so the ring reads clockwise as the
        // first row numbers its calls, and a module first entered deeper comes after.
        let mut journey: Vec<usize> = self.journey_order(links).collect();
        journey.sort_by_key(|t| self.trail.whole.get(t).copied().unwrap_or(u32::MAX));
        let mut order: Vec<usize> = Vec::new();
        for tile in journey.into_iter().chain(callers) {
            let cluster = cluster_of(tile);
            if cluster != middle && !order.contains(&cluster) {
                order.push(cluster);
            }
        }
        order
    }
}

/// Stands the modules in `order` on a ring round the nucleus, `middle`, clockwise
/// from the top as the usual view sees it, each as big as it is, the ring just wide
/// enough to hold them all with room between.
fn place_ring(balls: &mut [Ball], order: &[usize], middle: usize) {
    let radii: Vec<f32> = order.iter().map(|&c| balls[c].radius).collect();
    let widest = radii.iter().copied().fold(0.0, f32::max);
    let least = balls[middle].radius + NEAR_GAP + widest;
    let (angles, ring) = clockwise(&radii, least, MARGIN);
    let rotation = view_rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    for (&cluster, angle) in order.iter().zip(angles) {
        balls[cluster].center = (right * angle.cos() + up * angle.sin()) * ring;
    }
}

/// Angles round a ring, clockwise from the top, for spheres as big as `radii` in
/// order, `margin` apart at least and the room left shared evenly between them; and
/// the ring's radius, `least` or more when they need it.
fn clockwise(radii: &[f32], least: f32, margin: f32) -> (Vec<f32>, f32) {
    let widths: Vec<f32> = radii.iter().map(|r| 2.0 * r + margin).collect();
    let total: f32 = widths.iter().sum();
    let ring = least.max(total / std::f32::consts::TAU);
    #[expect(clippy::cast_precision_loss, reason = "a short list")]
    let spare = (std::f32::consts::TAU * ring - total).max(0.0) / widths.len().max(1) as f32;
    let mut along = 0.0;
    let mut angles = Vec::with_capacity(widths.len());
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            along += (widths[index - 1] + width) / 2.0 + spare;
        }
        angles.push(std::f32::consts::FRAC_PI_2 - along / ring);
    }
    (angles, ring)
}

impl Nucleus {
    /// Puts the outside callers `calls` brings in where they show without moving
    /// anything already placed: inside the nucleus where they sit; in a module on the
    /// ring, in the places it keeps free below the names `reserved` lays out in it;
    /// else in their own module beyond the ring, towards what they call, where they sit
    /// in it. Returns them.
    fn place_outside(
        &self,
        balls: &mut [Ball],
        (points, shown): (&mut [Vec3], &mut [bool]),
        (calls, reserved): (&[Link], &[Vec<usize>]),
        middle: usize,
        cluster_of: &impl Fn(usize) -> usize,
    ) -> HashSet<usize> {
        let mut by_module: Vec<(usize, Vec<usize>)> = Vec::new();
        for call in calls {
            let cluster = cluster_of(call.from);
            match by_module.iter_mut().find(|(c, _)| *c == cluster) {
                Some((_, tiles)) if !tiles.contains(&call.from) => tiles.push(call.from),
                Some(_) => {}
                None => by_module.push((cluster, vec![call.from])),
            }
        }
        let core = balls[middle].center;
        let placed: Vec<usize> = (0..balls.len())
            .filter(|&c| matches!(balls[c].body, Body::Nucleus | Body::Open))
            .collect();
        let outer = placed
            .iter()
            .map(|&c| balls[c].center.distance(core) + balls[c].radius)
            .fold(0.0, f32::max)
            + OUTER_GAP;
        let back = view_rotation() * Vec3::Z;
        let mut beyond = Vec::new();
        for (cluster, tiles) in &by_module {
            let ball = balls[*cluster];
            if *cluster == middle {
                for &tile in tiles {
                    shown[tile] = true;
                }
                continue;
            }
            if ball.body == Body::Open {
                // One the journey could name deeper has its place; the rest take the
                // places kept free below the names.
                let names = &reserved[*cluster];
                let mut free = names_grid(names.len(), ball.radius)
                    .into_iter()
                    .skip(names.len());
                for &tile in tiles {
                    if !names.contains(&tile) {
                        points[tile] = ball.center + free.next().unwrap_or(self.offsets[tile]);
                    }
                    shown[tile] = true;
                }
                continue;
            }
            // Towards the functions on the journey it calls, as the usual view sees them.
            let toward = calls
                .iter()
                .filter(|c| tiles.contains(&c.from))
                .map(|c| points[c.to] - core)
                .sum::<Vec3>();
            let toward = (toward - back * toward.dot(back))
                .try_normalize()
                .unwrap_or_else(|| sideways(back));
            // Faint, as everything brought in from outside the journey is.
            balls[*cluster] = Ball {
                center: core + toward * (outer + ball.radius),
                radius: ball.radius,
                body: Body::Faint,
            };
            beyond.push(*cluster);
        }
        let views: Vec<Vec3> = [0.0, -1.0, 1.0]
            .into_iter()
            .map(|turn| turned_view(turn * std::f32::consts::FRAC_PI_4) * Vec3::Z)
            .collect();
        relax(
            balls,
            |c, _| beyond.contains(&c),
            (MARGIN, 0.0),
            &views,
            200,
        );
        for (cluster, tiles) in &by_module {
            if beyond.contains(cluster) {
                for &tile in tiles {
                    points[tile] = balls[*cluster].center + self.offsets[tile];
                    shown[tile] = true;
                }
            }
        }
        by_module.into_iter().flat_map(|(_, tiles)| tiles).collect()
    }

    /// Where each function is drawn, and whether it is: the nucleus's own where they
    /// sit in it; those an open module could name, by `opened`, in columns down its
    /// middle; the rest folded away at their module's middle.
    fn place_functions(
        &self,
        balls: &[Ball],
        opened: &[Vec<usize>],
        middle: usize,
        cluster_of: &impl Fn(usize) -> usize,
    ) -> (Vec<Vec3>, Vec<bool>) {
        let tiles = self.offsets.len();
        let mut points: Vec<Vec3> = (0..tiles).map(|t| balls[cluster_of(t)].center).collect();
        let mut shown = vec![false; tiles];
        for tile in (0..tiles).filter(|&t| cluster_of(t) == middle) {
            points[tile] = balls[middle].center + self.offsets[tile];
            shown[tile] = true;
        }
        for (cluster, column) in opened.iter().enumerate() {
            let grid = names_grid(column.len(), balls[cluster].radius);
            for (&tile, at) in column.iter().zip(grid) {
                points[tile] = balls[cluster].center + at;
                shown[tile] = true;
            }
        }
        (points, shown)
    }
}

/// Puts every module that names nothing further out than the ring, behind it, the
/// farther the further its home is from the middle's.
fn place_closed(balls: &mut [Ball], bearings: &Bearings) {
    let outer = balls
        .iter()
        .filter(|b| b.body != Body::Closed)
        .map(|b| b.center.length() + b.radius)
        .fold(0.0, f32::max)
        + 14.0;
    for (cluster, ball) in balls.iter_mut().enumerate() {
        if ball.body == Body::Closed {
            let far = bearings.remoteness(cluster).min(2.5);
            ball.center = bearings.behind(cluster) * (outer + 10.0 * far);
        }
    }
    relax(balls, |_, b| b.body == Body::Closed, (1.5, 0.0), &[], 60);
}

/// [`lay_inside`] for a module's `members`, by tile, from `start`, with the `calls`
/// among them and how far each is from the start by `reach`; turned to the usual view,
/// and what is near the start, each as big as `size`, kept from touching as that view
/// sees it.
fn lay_members(
    members: &[usize],
    start: &[Vec3],
    calls: &[(usize, usize)],
    reach: &HashMap<usize, i32>,
    size: f32,
) -> Vec<Vec3> {
    let local: HashMap<usize, usize> = members.iter().enumerate().map(|(i, &t)| (t, i)).collect();
    let calls: Vec<(usize, usize)> = calls.iter().map(|(a, b)| (local[a], local[b])).collect();
    let hops: Vec<Option<i32>> = members.iter().map(|t| reach.get(t).copied()).collect();
    // A nudge apart, so no two start in the same place.
    let start: Vec<Vec3> = start
        .iter()
        .enumerate()
        .map(|(i, &at)| at + fibonacci(i, start.len()) * 0.01)
        .collect();
    let rotation = view_rotation();
    let mut at = lay_inside(
        &start,
        &calls,
        &hops,
        rotation * Vec3::X,
        rotation * Vec3::Z,
    );
    keep_apart(&mut at, &hops, size, rotation * Vec3::Z);
    at
}

/// Pushes apart the functions near the start, by `hops`, that touch as seen from
/// `back`, each as big as `size`, until a gap shows between every two; the start
/// stays in the middle. They come most of the way towards the plane the view looks
/// across first, so the camera's perspective, which spreads what is nearer it, does
/// not bring two together again.
fn keep_apart(at: &mut [Vec3], hops: &[Option<i32>], size: f32, back: Vec3) {
    let near: Vec<usize> = (0..at.len()).filter(|&i| hops[i].is_some()).collect();
    for &i in &near {
        at[i] -= back * at[i].dot(back) * 0.7;
    }
    for _ in 0..200 {
        let mut moved = false;
        for (k, &i) in near.iter().enumerate() {
            for &j in &near[k + 1..] {
                let apart = at[j] - at[i];
                let seen = apart - back * apart.dot(back);
                let need = 2.0 * size;
                let distance = seen.length();
                if distance >= need {
                    continue;
                }
                let away = seen
                    .try_normalize()
                    .unwrap_or_else(|| fibonacci(i * 31 + j, 97).cross(back).normalize_or(Vec3::X));
                let gap = need - distance;
                let (share_i, share_j) = match (hops[i], hops[j]) {
                    (Some(0), _) => (0.0, 1.0),
                    (_, Some(0)) => (1.0, 0.0),
                    _ => (0.5, 0.5),
                };
                at[i] = (at[i] - away * gap * share_i).clamp_length_max(0.92);
                at[j] = (at[j] + away * gap * share_j).clamp_length_max(0.92);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
}

/// Puts the modules in `rim` on a ring just outside the expanded nucleus's wall, as
/// the usual view sees it, each at its angle round it from `angles`; one without an
/// angle at its bearing.
fn place_rim(balls: &mut [Ball], rim: &[usize], angles: &HashMap<usize, f32>, core: f32) {
    let rotation = view_rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    for &cluster in rim {
        let ball = &mut balls[cluster];
        let angle = angles.get(&cluster).copied().unwrap_or_else(|| {
            let at = ball.center;
            at.dot(up).atan2(at.dot(right))
        });
        ball.center =
            (right * angle.cos() + up * angle.sin()) * (core * (1.0 + RIM_GAP) + ball.radius);
        ball.body = Body::Open;
    }
}

/// The angle round the expanded nucleus, as the usual view sees it, of each module on
/// the ring round it, as big as `radii`, in order clockwise from the top.
fn rim_angles(radii: &[f32], core: f32) -> Vec<f32> {
    let widest = radii.iter().copied().fold(0.0, f32::max);
    clockwise(radii, core * (1.0 + RIM_GAP) + widest, core * RIM_MARGIN).0
}

/// Lays a module's functions out in a sphere of radius one by the calls among them,
/// from where `start` has them. What is near the start, by `hops` (none for the rest),
/// gathers round it in the middle with room between, and the start's callers lean
/// away from `right` and its callees towards it, only as a tendency; calls pull
/// together; everything else keeps out towards the wall. Depth along `back`, the way
/// the usual view looks, counts for half, so what lies in line there spreads across.
fn lay_inside(
    start: &[Vec3],
    calls: &[(usize, usize)],
    hops: &[Option<i32>],
    right: Vec3,
    back: Vec3,
) -> Vec<Vec3> {
    const ROUNDS: usize = 300;
    let count = start.len();
    let mut at = start.to_vec();
    let weight: Vec<f32> = hops
        .iter()
        .map(|h| if h.is_some() { 1.0 } else { 0.35 })
        .collect();
    for round in 0..ROUNDS {
        let mut push = vec![Vec3::ZERO; count];
        for i in 0..count {
            for j in i + 1..count {
                let apart = at[i] - at[j];
                let seen = apart - back * apart.dot(back) * 0.5;
                let squared = seen.length_squared().max(1e-4);
                let force =
                    seen / squared.sqrt() * (0.0016 * weight[i] * weight[j] / squared).min(0.05);
                push[i] += force;
                push[j] -= force;
            }
        }
        for &(a, b) in calls {
            let apart = at[b] - at[a];
            let length = apart.length().max(1e-3);
            let strength = if hops[a].is_some() && hops[b].is_some() {
                0.08
            } else {
                0.02
            };
            let pull = apart / length * (length - 0.3) * strength;
            push[a] += pull;
            push[b] -= pull;
        }
        for (i, push) in push.iter_mut().enumerate() {
            *push += match hops[i] {
                #[expect(clippy::cast_precision_loss, reason = "a hop count")]
                Some(hops) => {
                    let lean = hops.clamp(-1, 3) as f32 * 0.22 - at[i].dot(right);
                    right * lean * 0.03 - at[i] * 0.02
                }
                None => at[i].normalize_or_zero() * (0.82 - at[i].length()) * 0.05,
            };
        }
        #[expect(clippy::cast_precision_loss, reason = "a step count")]
        let cooling = 1.0 - round as f32 / ROUNDS as f32;
        for (i, point) in at.iter_mut().enumerate() {
            *point = if hops[i] == Some(0) {
                Vec3::ZERO
            } else {
                (*point + push[i].clamp_length_max(0.05) * cooling).clamp_length_max(0.92)
            };
        }
    }
    at
}

/// Browsing, where every module and function sits: each module where the stable
/// layout of modules has it, spread so none hides another as the usual view, or one
/// turned an eighth of the way either side, sees them; each function where `offsets`
/// has it in its module, by `module_of`.
fn atlas(homes: &[Vec3], counts: &[usize], offsets: &[Vec3], module_of: &[Option<usize>]) -> Atlas {
    let balls = atlas_balls(homes, counts);
    let points = offsets
        .iter()
        .zip(module_of)
        .map(|(&at, module)| module.map_or(at, |c| balls[c].center + at))
        .collect();
    Atlas { balls, points }
}

/// Browsing, each module's sphere, where [`atlas`] has it.
fn atlas_balls(homes: &[Vec3], counts: &[usize]) -> Vec<Ball> {
    let clusters = homes.len();
    let radii: Vec<f32> = counts.iter().map(|&n| module_radius(n)).collect();
    let median = |mut values: Vec<f32>| {
        values.sort_by(f32::total_cmp);
        values.get(values.len() / 2).copied()
    };
    // As far apart as the typical module needs, nearest neighbour to nearest.
    let nearest: Vec<f32> = (0..clusters)
        .map(|i| {
            (0..clusters)
                .filter(|&j| j != i)
                .map(|j| homes[i].distance(homes[j]))
                .fold(f32::INFINITY, f32::min)
        })
        .filter(|d| d.is_finite())
        .collect();
    let gap = median(nearest).unwrap_or(1.0).max(1e-3);
    let room = 2.0 * median(radii.clone()).unwrap_or(1.0) + ATLAS_MARGIN;
    #[expect(clippy::cast_precision_loss, reason = "a module count")]
    let middle = homes.iter().copied().sum::<Vec3>() / clusters.max(1) as f32;
    // Shallow as the usual view sees it, so a module further back does not slide
    // under one in front as the camera comes in.
    let back = view_rotation() * Vec3::Z;
    let mut balls: Vec<Ball> = homes
        .iter()
        .zip(&radii)
        .map(|(&home, &radius)| {
            let at = (home - middle) * (room / gap);
            Ball {
                center: at - back * at.dot(back) * (1.0 - ATLAS_DEPTH),
                radius,
                body: Body::Faint,
            }
        })
        .collect();
    let views: Vec<Vec3> = [0.0, -1.0, 1.0]
        .into_iter()
        .map(|turn| turned_view(turn * std::f32::consts::FRAC_PI_4) * Vec3::Z)
        .collect();
    relax(
        &mut balls,
        |_, _| true,
        (ATLAS_MARGIN, ATLAS_SLANT),
        &views,
        2000,
    );
    #[expect(clippy::cast_precision_loss, reason = "a module count")]
    let middle = balls.iter().map(|b| b.center).sum::<Vec3>() / clusters.max(1) as f32;
    // As wide as a window is, against its height, as the usual view sees it: pulled
    // apart sideways, never together, so none comes to hide another.
    let rotation = view_rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    let spread = |along: Vec3| {
        balls
            .iter()
            .map(|b| (b.center - middle).dot(along).abs())
            .fold(0.0, f32::max)
    };
    let (wide, high) = (spread(right), spread(up));
    let stretch = (ATLAS_ASPECT * high / wide.max(1e-3)).clamp(1.0, 3.0);
    for ball in &mut balls {
        let offset = ball.center - middle;
        ball.center = offset + right * offset.dot(right) * (stretch - 1.0);
    }
    balls
}

/// A module's sphere, by how many functions it holds: the one rule for every module
/// in every view, browsing, in a journey and round the expanded one. It grows with
/// the square root of the count, so a module twice the size is far less than twice as
/// big, and no more than [`SPHERE_RATIO`] times the smallest; past that its functions
/// pack closer.
pub(crate) fn module_radius(count: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a function count")]
    let root = (count.max(1) as f32).sqrt();
    SPHERE_UNIT * (1.0 + SPHERE_GROWTH * (root - 1.0)).min(SPHERE_RATIO)
}

/// Where a module's `members` sit inside its sphere of `radius`, about its middle:
/// spread evenly over a disc square to the usual view, so none hides or touches
/// another from there, or from as far round as the camera turns, at any zoom that
/// shows them apart at all; each as near where the settled graph has it, against the
/// others, as that allows. The disc bows a little towards the view, so the module
/// reads as round.
fn pack(graph: &ForceGraph, members: &[usize], radius: f32) -> Vec<(usize, Vec3)> {
    if members.len() < 2 {
        return members.iter().map(|&t| (t, Vec3::ZERO)).collect();
    }
    let rotation = view_rotation();
    let (right, up, back) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z);
    let disc = radius * PACK_SHARE - PACK_INSET;
    #[expect(clippy::cast_precision_loss, reason = "a function count")]
    let count = members.len() as f32;
    // As far apart as the disc holds them evenly, nearly as close as a honeycomb.
    let apart = (disc * PACKING / count.sqrt()).min(disc);
    let mut at: Vec<Vec2> = interior(graph, members, disc)
        .into_iter()
        .map(|(_, at)| Vec2::new(at.dot(right), at.dot(up)))
        .collect();
    spread(&mut at, apart, disc);
    members
        .iter()
        .zip(at)
        .map(|(&tile, at)| {
            let bow = PACK_BOW * (disc * disc - at.length_squared()).max(0.0).sqrt();
            (tile, right * at.x + up * at.y + back * bow)
        })
        .collect()
}

/// Pushes `at` apart until every two are `apart` or more, each kept within `disc` of
/// the middle: a pair too near moves away from each other, half each, again and
/// again; near pairs are found through a grid of cells `apart` wide.
fn spread(at: &mut [Vec2], apart: f32, disc: f32) {
    const ROUNDS: usize = 600;
    let cell = |p: Vec2| {
        #[expect(clippy::cast_possible_truncation, reason = "a cell index")]
        let index = ((p.x / apart).floor() as i32, (p.y / apart).floor() as i32);
        index
    };
    for round in 0..ROUNDS {
        let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, &p) in at.iter().enumerate() {
            cells.entry(cell(p)).or_default().push(i);
        }
        let mut moved = false;
        for i in 0..at.len() {
            let (x, y) = cell(at[i]);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let Some(near) = cells.get(&(x + dx, y + dy)) else {
                        continue;
                    };
                    for &j in near.iter().filter(|&&j| j > i) {
                        let between = at[j] - at[i];
                        let distance = between.length();
                        if distance >= apart {
                            continue;
                        }
                        let away = between.try_normalize().unwrap_or_else(|| {
                            let turn = fibonacci(i * 31 + j + round, 97);
                            Vec2::new(turn.x, turn.y).normalize_or(Vec2::X)
                        });
                        let push = away * (apart - distance) * 0.5;
                        at[i] = (at[i] - push).clamp_length_max(disc);
                        at[j] = (at[j] + push).clamp_length_max(disc);
                        moved = true;
                    }
                }
            }
        }
        if !moved {
            break;
        }
    }
}

/// The usual view's rotation.
pub(crate) fn view_rotation() -> Quat {
    turned_view(0.0)
}

/// The usual view turned round by `yaw`.
fn turned_view(yaw: f32) -> Quat {
    Quat::from_euler(EulerRot::YXZ, VIEW.0 + yaw, VIEW.1, 0.0)
}

/// Where `count` names sit about their module's middle: a column [`SPACING`] apart,
/// top to bottom, and past [`ROWS`] more columns side by side, [`COLUMN_GAP`] apart,
/// as the usual view sees them; drawn in closer where that would not fit a sphere of
/// `radius`.
fn grid(count: usize, radius: f32) -> Vec<Vec3> {
    let columns = count.div_ceil(ROWS).max(1);
    let rows = count.div_ceil(columns).max(1);
    let right = view_rotation() * Vec3::X;
    #[expect(clippy::cast_precision_loss, reason = "short lists")]
    let (high, wide) = (
        (rows as f32 - 1.0) * SPACING / 2.0,
        (columns as f32 - 1.0) * COLUMN_GAP / 2.0,
    );
    let room = radius * PACK_SHARE - PACK_INSET;
    let fit = (room / high.hypot(wide).max(f32::EPSILON)).min(1.0);
    #[expect(clippy::cast_precision_loss, reason = "short lists")]
    (0..count)
        .map(|i| {
            let (column, row) = (i / rows, i % rows);
            let x = (column as f32 - (columns as f32 - 1.0) / 2.0) * COLUMN_GAP;
            let y = ((rows as f32 - 1.0) / 2.0 - row as f32) * SPACING;
            (right * x + Vec3::Y * y) * fit
        })
        .collect()
}

/// The places each open module of `balls` keeps free below the names `reserved`
/// lays out in it, in the world.
fn kept_free(balls: &[Ball], reserved: &[Vec<usize>]) -> Vec<Vec3> {
    balls
        .iter()
        .zip(reserved)
        .filter(|(ball, names)| ball.body == Body::Open && !names.is_empty())
        .flat_map(|(ball, names)| {
            names_grid(names.len(), ball.radius)
                .into_iter()
                .skip(names.len())
                .map(move |at| ball.center + at)
        })
        .collect()
}

/// Where an open module's `count` names sit in its sphere of `radius`, by [`grid`],
/// and after them the [`OUTSIDE_SHOWN`] places it keeps free for its callers from
/// outside the journey, so those come and go moving no name.
fn names_grid(count: usize, radius: f32) -> Vec<Vec3> {
    grid(count + OUTSIDE_SHOWN, radius)
}

/// An arrow's thickness, for this many calls: growing slowly, and never so thick that
/// the calls running along it are lost.
pub(crate) fn tube_radius(calls: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a call count")]
    (0.08 + 0.05 * (calls as f32).ln_1p()).min(0.3)
}

/// `path` bent round each of `obstacles` (a middle and a radius) it runs across as
/// the usual view sees it, keeping `clear` off it: at the point that runs deepest
/// into one, a bend out past its edge, again for whatever the bent path still runs
/// across, up to [`DETOUR_BENDS`] bends. One holding an end of the path, which no
/// bend can keep off, is left be.
fn detour(mut path: Vec<Vec3>, obstacles: &[(Vec3, f32)], clear: f32) -> Vec<Vec3> {
    let back = view_rotation() * Vec3::Z;
    let flat = |v: Vec3| v - back * v.dot(back);
    for _ in 0..DETOUR_BENDS {
        // The segment, the bend that clears it, and how far into the obstacle it ran.
        let mut worst: Option<(usize, Vec3, f32)> = None;
        for (k, pair) in path.windows(2).enumerate() {
            let (a, b) = (pair[0], pair[1]);
            let along = flat(b - a);
            let length = along.length_squared();
            if length < 1e-6 {
                continue;
            }
            for &(middle, radius) in obstacles {
                let reach = radius + clear;
                if flat(a - middle).length() < reach || flat(b - middle).length() < reach {
                    continue;
                }
                let t = flat(middle - a).dot(along) / length;
                if !(0.0..=1.0).contains(&t) {
                    continue;
                }
                let nearest = a + (b - a) * t;
                let off = flat(nearest - middle);
                let into = reach - off.length();
                if into <= 0.0 || worst.is_some_and(|w| w.2 >= into) {
                    continue;
                }
                let out = off
                    .try_normalize()
                    .unwrap_or_else(|| along.cross(back).normalize_or(Vec3::X));
                worst = Some((k, nearest + out * (into + clear * 0.5), into));
            }
        }
        let Some((k, bend, _)) = worst else {
            break;
        };
        path.insert(k + 1, bend);
    }
    path
}

/// A direction square to `along`, the same every time for the same direction.
fn sideways(along: Vec3) -> Vec3 {
    along
        .cross(Vec3::Y)
        .try_normalize()
        .unwrap_or_else(|| along.any_orthonormal_vector())
}

/// The `index`th of `count` points spread evenly over the unit sphere.
fn fibonacci(index: usize, count: usize) -> Vec3 {
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let (i, n) = (index as f32 + 0.5, count.max(1) as f32);
    let y = 1.0 - 2.0 * i / n;
    let ring = (1.0 - y * y).max(0.0).sqrt();
    let turn = i * std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    Vec3::new(ring * turn.cos(), y, ring * turn.sin())
}

/// A layout of the modules alone in 3D, the same every time for the same code:
/// modules repel, and calls between two pull them together, the harder the more.
fn module_homes(count: usize, flows: &HashMap<(usize, usize), usize>) -> Vec<Vec3> {
    let mut weights: HashMap<(usize, usize), usize> = HashMap::new();
    for (&(a, b), &calls) in flows {
        *weights.entry((a.min(b), a.max(b))).or_insert(0) += calls;
    }
    let mut weights: Vec<((usize, usize), usize)> = weights.into_iter().collect();
    weights.sort_unstable();
    #[expect(clippy::cast_precision_loss, reason = "a module count")]
    let spread = (count as f32).sqrt() * 6.0;
    let mut at: Vec<Vec3> = (0..count).map(|i| fibonacci(i, count) * spread).collect();
    for step in 0..300 {
        let mut push = vec![Vec3::ZERO; count];
        for i in 0..count {
            for j in i + 1..count {
                let apart = at[i] - at[j];
                let squared = apart.length_squared().max(0.25);
                let force = apart / squared.sqrt() * (400.0 / squared);
                push[i] += force;
                push[j] -= force;
            }
        }
        for &((a, b), calls) in &weights {
            let apart = at[b] - at[a];
            let length = apart.length().max(0.01);
            #[expect(clippy::cast_precision_loss, reason = "a call count")]
            let pull = apart / length * (length - 10.0) * 0.04 * (1.0 + (calls as f32).ln());
            push[a] += pull;
            push[b] -= pull;
        }
        #[expect(clippy::cast_precision_loss, reason = "a step count")]
        let cooling = 1.0 - step as f32 / 300.0;
        for (point, push) in at.iter_mut().zip(push) {
            let push = push - *point * 0.01;
            *point += push.clamp_length_max(3.0) * cooling;
        }
    }
    at
}

/// Pushes overlapping spheres apart, moving only those `movable` picks: each pair is
/// pulled apart along the line between them until `margin` clears between them.
/// Given the directions views look from, they are kept apart as each of those views
/// sees them, moving across it, and as the first of them sees them by `slant` more
/// for each unit one lies behind the other, so a camera's perspective from there does
/// not bring them together; given none, in space.
fn relax(
    balls: &mut [Ball],
    movable: impl Fn(usize, &Ball) -> bool,
    (margin, slant): (f32, f32),
    views: &[Vec3],
    rounds: usize,
) {
    let in_space = [None];
    let seen: Vec<Option<Vec3>> = if views.is_empty() {
        in_space.to_vec()
    } else {
        views.iter().copied().map(Some).collect()
    };
    for _ in 0..rounds {
        let mut moved = false;
        for i in 0..balls.len() {
            for j in i + 1..balls.len() {
                let (move_a, move_b) = (movable(i, &balls[i]), movable(j, &balls[j]));
                if !move_a && !move_b {
                    continue;
                }
                for (view, &seen_from) in seen.iter().enumerate() {
                    let (a, b) = (balls[i], balls[j]);
                    let apart = b.center - a.center;
                    // One further back shows nearer the middle of the view than it
                    // would side by side, so a gap in depth needs one across too.
                    let deeper = match seen_from {
                        Some(back) if view == 0 => apart.dot(back).abs(),
                        _ => 0.0,
                    };
                    let need = a.radius + b.radius + margin + slant * deeper;
                    let apart = seen_from.map_or(apart, |back| apart - back * apart.dot(back));
                    let distance = apart.length();
                    if distance >= need {
                        continue;
                    }
                    let away = apart.try_normalize().unwrap_or_else(|| {
                        seen_from.map_or_else(|| fibonacci(i * 31 + j, 97), sideways)
                    });
                    let gap = need - distance;
                    let (share_a, share_b) = match (move_a, move_b) {
                        (true, true) => (0.5, 0.5),
                        (true, false) => (1.0, 0.0),
                        _ => (0.0, 1.0),
                    };
                    balls[i].center -= away * gap * share_a;
                    balls[j].center += away * gap * share_b;
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
}

/// Where the nucleus's functions sit inside a sphere of `radius`: as the settled graph
/// has them about their middle, stretched to fill the sphere in all three
/// directions, so the module's own map is kept.
fn interior(graph: &ForceGraph, members: &[usize], radius: f32) -> Vec<(usize, Vec3)> {
    let at = |tile: usize| Vec3::from(graph.positions[tile]);
    if members.len() < 2 {
        return members.iter().map(|&t| (t, Vec3::ZERO)).collect();
    }
    #[expect(clippy::cast_precision_loss, reason = "a function count")]
    let count = members.len() as f32;
    let middle = members.iter().map(|&t| at(t)).sum::<Vec3>() / count;
    let spread = (members
        .iter()
        .map(|&t| (at(t) - middle).powf(2.0))
        .sum::<Vec3>()
        / count)
        .powf(0.5)
        .max(Vec3::splat(1e-3));
    let offsets: Vec<Vec3> = members.iter().map(|&t| (at(t) - middle) / spread).collect();
    let mut lengths: Vec<f32> = offsets.iter().map(|o| o.length()).collect();
    lengths.sort_by(f32::total_cmp);
    let typical = lengths[(lengths.len() * 85 / 100).min(lengths.len() - 1)].max(1e-3);
    members
        .iter()
        .zip(offsets)
        .map(|(&tile, offset)| {
            (
                tile,
                (offset * 0.72 / typical).clamp_length_max(0.88) * radius,
            )
        })
        .collect()
}

/// X expands the nucleus, or shrinks it back.
pub(crate) fn toggle(keys: &ButtonInput<KeyCode>, explorer: &mut Explorer) {
    if keys.just_pressed(KeyCode::KeyX) {
        let on = !explorer.nucleus().expanded();
        explorer.expand_nucleus(on);
    }
}

/// Re-centers on the start's module when the start moves, and eases the spheres
/// there.
pub(crate) fn follow(time: Res<Time>, mut explorer: ResMut<Explorer>, mut awake: ResMut<Awake>) {
    if explorer
        .bypass_change_detection()
        .ease_nucleus(time.delta_secs())
    {
        explorer.set_changed();
        awake.0 = true;
    }
}

/// A module's sphere in the scene.
#[derive(Component, Debug)]
pub(crate) struct ModuleSphere(usize);

/// One of the arrows between modules in the scene: a stretch of its shaft, between
/// two of its bends, or its head.
#[derive(Component, Debug)]
pub(crate) struct Arrow {
    index: usize,
    /// Which stretch of the shaft, from where it leaves.
    part: usize,
    head: bool,
}

/// What spheres and arrows are drawn with.
#[derive(Resource, Debug)]
pub(crate) struct Looks {
    sphere: Handle<Mesh>,
    shaft: Handle<Mesh>,
    head: Handle<Mesh>,
    /// By module color: glass for the nucleus, for an open module, and a
    /// solid for a closed one.
    nucleus: Vec<Handle<StandardMaterial>>,
    open: Vec<Handle<StandardMaterial>>,
    closed: Vec<Handle<StandardMaterial>>,
    /// Browsing, glass so faint the module only just reads as one, by color, for
    /// each of [`DEPTH_SHADES`] from the nearest to the furthest back.
    faint: Vec<Vec<Handle<StandardMaterial>>>,
    /// By [`Flow`]: an arrow's shaft and its head.
    shafts: HashMap<Flow, Handle<StandardMaterial>>,
    heads: HashMap<Flow, Handle<StandardMaterial>>,
    /// Browsing with nothing selected, an arrow between modules by how many calls it
    /// carries ([`module_strength`]): its shaft and its head, plainer the more.
    module_shafts: Vec<Handle<StandardMaterial>>,
    module_heads: Vec<Handle<StandardMaterial>>,
}

/// Browsing, how plain a module's glass is, from the nearest module to the furthest
/// back: further back reads fainter, so the map reads as having depth in a still.
const DEPTH_SHADES: [f32; 4] = [0.06, 0.038, 0.022, 0.01];

/// Browsing, how far back each module sits among them all, as `camera` sees them:
/// 0 for the nearest, 1 for the furthest back.
fn depths(nucleus: &Nucleus, camera: &Transform) -> Vec<f32> {
    let forward = *camera.forward();
    let along: Vec<f32> = (0..nucleus.now.balls.len())
        .map(|c| {
            nucleus
                .ball(c)
                .map_or(0.0, |b| (b.center - camera.translation).dot(forward))
        })
        .collect();
    let (near, far) = along
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &d| {
            (lo.min(d), hi.max(d))
        });
    let span = (far - near).max(1e-3);
    along
        .iter()
        .map(|d| ((d - near) / span).clamp(0.0, 1.0))
        .collect()
}

/// How plain an arrow between modules is drawn, browsing with nothing selected, for
/// each [`module_strength`]: its shaft's and its head's opacity.
const MODULE_STRENGTHS: [(f32, f32); 4] = [(0.03, 0.06), (0.06, 0.12), (0.11, 0.22), (0.2, 0.38)];

/// Which of [`MODULE_STRENGTHS`] an arrow carrying `calls` calls is drawn with: one
/// call, a few, up to ten, more.
fn module_strength(calls: usize) -> usize {
    match calls {
        0 | 1 => 0,
        2..=3 => 1,
        4..=9 => 2,
        _ => 3,
    }
}

pub(crate) fn create_looks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let glass = |color: Color, alpha: f32| StandardMaterial {
        base_color: color.with_alpha(alpha),
        perceptual_roughness: 0.25,
        reflectance: 0.6,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        double_sided: true,
        ..default()
    };
    let solid = |color: Color| {
        let linear = LinearRgba::from(color);
        StandardMaterial {
            // Dark and matte, so the rest of the codebase stays back.
            base_color: Color::from(LinearRgba::rgb(
                linear.red * 0.14,
                linear.green * 0.14,
                linear.blue * 0.16,
            )),
            perceptual_roughness: 0.8,
            reflectance: 0.15,
            ..default()
        }
    };
    let see_through = |color: Color, alpha: f32| StandardMaterial {
        base_color: color.with_alpha(alpha),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    };
    let colors: Vec<Color> = (0..MODULE_SHADES).map(module_color).collect();
    let flows: Vec<Flow> = [Flow::Into, Flow::Outside, Flow::Module]
        .into_iter()
        .chain((1..=SHADES).map(Flow::Journey))
        .collect();
    let looks = Looks {
        sphere: meshes.add(Sphere::new(1.0).mesh().uv(64, 40)),
        shaft: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(24)),
        head: meshes.add(Cone::new(1.0, 1.0).mesh().resolution(24)),
        nucleus: colors
            .iter()
            .map(|&c| materials.add(glass(c, 0.05)))
            .collect(),
        open: colors
            .iter()
            .map(|&c| materials.add(glass(c, 0.08)))
            .collect(),
        closed: colors.iter().map(|&c| materials.add(solid(c))).collect(),
        faint: DEPTH_SHADES
            .iter()
            .map(|&alpha| {
                colors
                    .iter()
                    .map(|&c| materials.add(glass(c, alpha)))
                    .collect()
            })
            .collect(),
        shafts: flows
            .iter()
            .map(|&f| {
                let alpha = match f {
                    Flow::Module => 0.14,
                    _ if f.rank() > 1 => 0.2,
                    _ => 0.32,
                };
                (f, materials.add(see_through(f.color(), alpha)))
            })
            .collect(),
        heads: flows
            .iter()
            .map(|&f| {
                let alpha = match f {
                    Flow::Module => 0.3,
                    _ if f.rank() > 1 => 0.55,
                    _ => 0.8,
                };
                (f, materials.add(see_through(f.color(), alpha)))
            })
            .collect(),
        module_shafts: MODULE_STRENGTHS
            .iter()
            .map(|&(shaft, _)| materials.add(see_through(Flow::Module.color(), shaft)))
            .collect(),
        module_heads: MODULE_STRENGTHS
            .iter()
            .map(|&(_, head)| materials.add(see_through(Flow::Module.color(), head)))
            .collect(),
    };
    commands.insert_resource(looks);
}

/// The orbit camera, apart from the spheres and arrows placed.
type OnlyTheCamera = (With<OrbitCamera>, Without<ModuleSphere>, Without<Arrow>);

/// Spawns a sphere per module, and an arrow per pair of modules as many as are
/// needed, then keeps each where the nucleus has it, drawn as its body asks.
pub(crate) fn place_bodies(
    mut commands: Commands,
    explorer: Res<Explorer>,
    looks: Res<Looks>,
    mut spheres: Query<
        (
            &ModuleSphere,
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<Arrow>,
    >,
    mut arrows: Query<
        (
            &Arrow,
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<ModuleSphere>,
    >,
    cameras: Query<&Transform, OnlyTheCamera>,
) {
    let nucleus = explorer.nucleus();
    let clusters = explorer.graph().cluster_count();
    if spheres.is_empty() {
        for cluster in 0..clusters {
            commands.spawn((
                ModuleSphere(cluster),
                Mesh3d(looks.sphere.clone()),
                MeshMaterial3d(looks.closed[module_shade(cluster)].clone()),
                Transform::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
            ));
        }
    }
    let depths = cameras
        .single()
        .ok()
        .filter(|_| nucleus.browsing())
        .map(|camera| depths(nucleus, camera));
    for (sphere, mut transform, mut visibility, mut material) in &mut spheres {
        if let Some(ball) = nucleus.ball(sphere.0) {
            let shown = nucleus.shows_body(ball.body);
            let placed =
                Transform::from_translation(ball.center).with_scale(Vec3::splat(ball.radius));
            transform.set_if_neq(placed);
            let depth = depths
                .as_ref()
                .and_then(|d| d.get(sphere.0).copied())
                .unwrap_or(0.0);
            dress(&looks, sphere.0, ball.body, depth, &mut material);
            // A journey shows the modules it runs through, and nothing else.
            visibility.set_if_neq(if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
    }

    place_arrows(&mut commands, nucleus, &looks, &mut arrows);
}

/// Spawns as many arrows as the nucleus has between modules, each a shaft in
/// stretches round what lies in its way and a head, and keeps each where its path
/// runs, colored as its flow; hides those left over.
fn place_arrows(
    commands: &mut Commands,
    nucleus: &Nucleus,
    looks: &Looks,
    arrows: &mut Query<
        (
            &Arrow,
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<ModuleSphere>,
    >,
) {
    let tubes = nucleus.tubes();
    let have = arrows
        .iter()
        .filter(|(a, ..)| !a.head && a.part == 0)
        .count();
    for index in have..tubes.len() {
        for (part, head) in (0..=DETOUR_BENDS)
            .map(|part| (part, false))
            .chain([(DETOUR_BENDS, true)])
        {
            commands.spawn((
                Arrow { index, part, head },
                Mesh3d(if head {
                    looks.head.clone()
                } else {
                    looks.shaft.clone()
                }),
                MeshMaterial3d(looks.shafts[&Flow::Into].clone()),
                Transform::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
            ));
        }
    }
    let paths: Vec<Option<(Vec<Vec3>, f32)>> = tubes.iter().map(|t| nucleus.tube_path(t)).collect();
    let zoom = nucleus.zoom();
    for (arrow, mut transform, mut visibility, mut material) in arrows.iter_mut() {
        let Some((tube, (path, radius))) = tubes
            .get(arrow.index)
            .zip(paths.get(arrow.index).and_then(Option::as_ref))
        else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        let last = path.len().saturating_sub(2);
        let part = if arrow.head { last } else { arrow.part };
        let (Some(&start), Some(&end)) = (path.get(part), path.get(part + 1)) else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        let along = end - start;
        let length = along.length();
        let total: f32 = path.windows(2).map(|p| p[0].distance(p[1])).sum();
        if length < 1e-3 || total < 0.5 {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        }
        let direction = along / length;
        // Small and narrow, so the calls running in beside it stay in view.
        let head = (radius * 6.0).max(0.9 * zoom).min(length * 0.4);
        let width = (radius * 3.5).max(0.35 * zoom);
        let rotation = Quat::from_rotation_arc(Vec3::Y, direction);
        // The last stretch stops where the head begins.
        let shaft = if part == last { length - head } else { length };
        let placed = if arrow.head {
            Transform {
                translation: end - direction * head / 2.0,
                rotation,
                scale: Vec3::new(width, head, width),
            }
        } else {
            Transform {
                translation: start + direction * shaft / 2.0,
                rotation,
                scale: Vec3::new(*radius, shaft, *radius),
            }
        };
        transform.set_if_neq(placed);
        let strength = module_strength(tube.calls);
        let wanted = match (tube.flow, arrow.head) {
            (Flow::Module, true) => &looks.module_heads[strength],
            (Flow::Module, false) => &looks.module_shafts[strength],
            (flow, true) => &looks.heads[&flow],
            (flow, false) => &looks.shafts[&flow],
        };
        if &material.0 != wanted {
            material.0 = wanted.clone();
        }
        visibility.set_if_neq(Visibility::Inherited);
    }
}

/// Gives a module's sphere the look its body asks for, `depth` back among them all
/// (0 nearest, 1 furthest).
fn dress(
    looks: &Looks,
    cluster: usize,
    body: Body,
    depth: f32,
    material: &mut MeshMaterial3d<StandardMaterial>,
) {
    let color = module_shade(cluster);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a share of a short list"
    )]
    let shade = ((depth * DEPTH_SHADES.len() as f32) as usize).min(DEPTH_SHADES.len() - 1);
    let wanted = match body {
        Body::Nucleus => &looks.nucleus[color],
        Body::Open => &looks.open[color],
        Body::Closed => &looks.closed[color],
        Body::Faint => &looks.faint[shade][color],
    };
    if &material.0 != wanted {
        material.0 = wanted.clone();
    }
}

/// A rim round each glass sphere, facing the camera, so its edge reads as one
/// outline; and the named calls, function to function along their arrows.
pub(crate) fn draw(
    explorer: Res<Explorer>,
    cameras: Query<(&Camera, &GlobalTransform, &Transform), With<OrbitCamera>>,
    mut gizmos: Gizmos,
) {
    let nucleus = explorer.nucleus();
    let Ok((lens, view, camera)) = cameras.single() else {
        return;
    };
    draw_rims(nucleus, camera, &mut gizmos);
    let zoom = nucleus.zoom();
    // Short of a function's surface, so the head stays visible.
    let short = |from: Vec3, to: Vec3, tile: usize| {
        if !nucleus.placed(tile) {
            return to;
        }
        to - (to - from).normalize_or_zero() * drawn_radius(&explorer, tile, lens, view)
    };
    // The journey's call into the cursor, in the cursor's color, or into the function
    // the pointer is on, and the calls in of a function selected off the journey, or
    // of an outside caller or a folded caller of the start the pointer is on, are
    // drawn last and brightest, so they read over the rest.
    let aside = explorer.aside();
    let picked = |link: &Link| {
        let into = link.flow != Flow::Into
            && link.flow != Flow::Outside
            && (nucleus.is_cursor(link.to) || nucleus.hovered() == Some(link.to));
        let pointed = (link.flow == Flow::Outside || nucleus.folded(link.from))
            && nucleus.hovered() == Some(link.from);
        into || pointed || (aside.is_some() && Some(link.from) == aside)
    };
    let (picked, rest): (Vec<_>, Vec<_>) = nucleus
        .links()
        .iter()
        .enumerate()
        .partition(|(_, link)| picked(link));
    for (index, link) in rest.into_iter().chain(picked.iter().copied()) {
        let route = nucleus.route(link);
        let [.., last_but_one, last] = route.as_slice() else {
            continue;
        };
        let bright = picked.iter().any(|(i, _)| *i == index);
        let color = link_color(nucleus, link, bright);
        let end = short(*last_but_one, *last, link.to);
        gizmos.linestrip(route[..route.len() - 1].iter().copied(), color);
        gizmos
            .arrow(*last_but_one, end, color)
            .with_tip_length(if bright { 1.1 } else { 0.8 } * zoom);
    }
}

/// A rim round each sphere drawn, facing the camera; browsing, fainter the further
/// back.
fn draw_rims(nucleus: &Nucleus, camera: &Transform, gizmos: &mut Gizmos) {
    let facing = camera.rotation;
    let depths = nucleus.browsing().then(|| depths(nucleus, camera));
    for (cluster, ball) in nucleus.balls() {
        let near = depths
            .as_ref()
            .and_then(|d| d.get(cluster))
            .map_or(1.0, |depth| 1.0 - 0.8 * depth);
        let color = module_color(cluster);
        // One rim, so the glass's edge reads as one outline.
        let alpha = match ball.body {
            Body::Nucleus => 0.85,
            Body::Open => 0.7,
            Body::Faint => 0.2,
            Body::Closed => continue,
        };
        gizmos
            .circle(
                Isometry3d::new(ball.center, facing),
                ball.radius,
                color.with_alpha(alpha * near),
            )
            .resolution(96);
    }
}

/// A named call's color: its flow's, but the cursor's for the call into the cursor,
/// and brighter for the one into the function the pointer is on.
fn link_color(nucleus: &Nucleus, link: &Link, bright: bool) -> Color {
    match link.flow {
        Flow::Outside if bright => Color::WHITE.with_alpha(0.85),
        // Secondary: there to be found, not read first.
        Flow::Outside => link.flow.color().with_alpha(0.08),
        // A caller of the start folded into its module's count: there, but quiet.
        Flow::Into if !bright && nucleus.folded(link.from) => link.flow.color().with_alpha(0.16),
        _ if bright && nucleus.is_cursor(link.to) => CURSOR,
        _ if bright => link.flow.color().mix(&Color::WHITE, 0.45),
        _ => link.flow.color().with_alpha(0.9),
    }
}

#[cfg(test)]
mod tests {
    use codeviz_core::System;
    use codeviz_ir::{Document, UnitId};
    use codeviz_layout::{CodebaseMap, ForceConfig, map};

    use super::*;

    /// Seven modules: `web::route` calls `core::run`, which calls `db::put`,
    /// `log::say` and `core::help`; `core::help` calls `core::idle` and `ext::lib`;
    /// `db::put` calls `disk::write`; `log::tail` calls `log::say`; `far` calls nothing.
    fn codebase() -> (ForceGraph, CodebaseMap) {
        let module = |name: &str| {
            format!(
                r#"{{ "id": "m:{name}", "name": "{name}", "kind": "module", "role": "group" }}"#
            )
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

    /// What the graph says `from` calls, `depth` calls down, numbered as a journey
    /// would number it.
    fn trail(graph: &ForceGraph, from: usize, depth: u32) -> Trail {
        let mut trail = Trail::default();
        let mut stack = vec![(from, 0, String::new())];
        while let Some((tile, level, number)) = stack.pop() {
            if level == depth {
                continue;
            }
            let mut callees: Vec<usize> = Vec::new();
            for callee in graph.callees(tile) {
                if callee != tile && !callees.contains(&callee) {
                    callees.push(callee);
                }
            }
            for (index, callee) in callees.into_iter().enumerate() {
                let number = if number.is_empty() {
                    format!("{}", index + 1)
                } else {
                    format!("{number}.{}", index + 1)
                };
                trail.add(tile, callee, level + 1, &number);
                stack.push((callee, level + 1, number));
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
        Nucleus::new(graph, Some(start), trail(graph, start, depth))
    }

    /// `core::start` calls `db::put`, which calls `db::deep`; nine functions call
    /// `core::start`: `web::c0` to `web::c5`, of which `web::c0` calls `db::put` too,
    /// and `core::c6` (twice) to `core::c8`.
    fn crowded() -> (ForceGraph, CodebaseMap) {
        let module = |name: &str| {
            format!(
                r#"{{ "id": "m:{name}", "name": "{name}", "kind": "module", "role": "group" }}"#
            )
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
        let web = module(&graph, &map, "web::c0");
        for depth in [1, 2] {
            let nucleus = around(&graph, &map, "core::start", depth);
            // Calling into the journey, calling it most, and living by it come first.
            for id in ["web::c0", "core::c6", "core::c7", "core::c8"] {
                let caller = tile(&map, id);
                assert!(!nucleus.folded(caller), "{id} is named at depth {depth}");
                assert!(nucleus.named(caller));
            }
            let folded: Vec<usize> = (0..graph.positions.len())
                .filter(|&t| nucleus.folded(t))
                .collect();
            assert_eq!(folded.len(), 4, "nine callers, five named");
            assert_eq!(nucleus.folded_in(web), 4, "counted on their module");
            for &caller in &folded {
                assert!(!nucleus.named(caller), "unnamed but under the pointer");
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
        let balls: Vec<Ball> = nucleus.balls().map(|(_, b)| b).collect();
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
    fn arrows_run_into_and_out_of_the_nucleus() {
        let (graph, map) = codebase();
        let nucleus = around(&graph, &map, "core::run", 2);
        let [web, core, db, disk] = ["web::route", "core::run", "db::put", "disk::write"]
            .map(|id| module(&graph, &map, id));
        let has = |from, to, flow| {
            nucleus
                .tubes()
                .iter()
                .any(|t| t.from == from && t.to == to && t.flow == flow)
        };
        assert!(has(web, core, Flow::Into));
        assert!(has(core, db, Flow::Journey(1)));
        assert!(has(db, disk, Flow::Journey(2)));
        let into = nucleus
            .links()
            .iter()
            .find(|l| l.flow == Flow::Into)
            .expect("a call into the start");
        assert_eq!(
            nucleus.route(into).len(),
            4,
            "a call between modules runs along their arrow"
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
    fn arrows_stay_thin_however_many_calls() {
        assert!(tube_radius(1) < tube_radius(10));
        assert!(tube_radius(10_000) <= 0.3);
    }

    #[test]
    fn expanding_grows_the_nucleus_and_rings_its_neighbours_outside() {
        let (graph, map) = codebase();
        let mut nucleus = around(&graph, &map, "core::run", 1);
        let core = module(&graph, &map, "core::run");
        let small = nucleus.ball(core).expect("a sphere").radius;
        let web = module(&graph, &map, "web::route");
        let bearing = nucleus.ball(web).expect("a sphere").center.normalize();
        nucleus.expand(&graph, true);
        while nucleus.advance(0.1) {}
        let big = nucleus.ball(core).expect("a sphere");
        assert!(big.radius >= small * 3.0, "grows several times over");
        for id in ["web::route", "db::put", "log::say"] {
            let cluster = module(&graph, &map, id);
            let ball = nucleus.ball(cluster).expect("a sphere");
            assert_eq!(ball.body, Body::Open, "{id}'s module is on the ring");
            assert!(
                ball.center.length() > big.radius + ball.radius,
                "outside the wall"
            );
            assert!(
                (ball.radius - module_radius(nucleus.count(cluster))).abs() < 1e-3,
                "as big as it is everywhere"
            );
        }
        let ring = nucleus.ball(web).expect("a sphere").center.normalize();
        assert!(ring.dot(bearing) > 0.0, "keeps to the side it was on");
        let disk = nucleus
            .ball(module(&graph, &map, "disk::write"))
            .expect("a sphere");
        assert_eq!(
            disk.body,
            Body::Closed,
            "two modules away is not on the ring"
        );
        for id in ["core::run", "core::help", "core::idle"] {
            let at = nucleus.point(tile(&map, id)).expect("a point");
            assert!(nucleus.placed(tile(&map, id)));
            assert!(at.length() < big.radius, "{id} inside");
        }
        assert!(
            nucleus.placed(tile(&map, "db::put")),
            "what the start calls outside stays named in its module"
        );
        assert!(
            !nucleus.placed(tile(&map, "db::other")),
            "the rest folds away"
        );
        nucleus.expand(&graph, false);
        while nucleus.advance(0.1) {}
        assert!((nucleus.ball(core).expect("a sphere").radius - small).abs() < 1e-3);
    }

    #[test]
    fn expanded_calls_leave_through_the_wall_towards_their_module() {
        let (graph, map) = codebase();
        let mut nucleus = around(&graph, &map, "core::run", 1);
        nucleus.expand(&graph, true);
        while nucleus.advance(0.1) {}
        let core = nucleus
            .ball(module(&graph, &map, "core::run"))
            .expect("a sphere");
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
        assert_eq!(nucleus.route(&inside).len(), 2, "straight, inside");
        let out = link(run, put).expect("the call out to db");
        let route = nucleus.route(&out);
        assert_eq!(route.len(), 4, "to the wall, to db's surface, to db::put");
        assert!(
            (route[1].length() - core.radius).abs() < 0.5,
            "leaves at the wall"
        );
        let db = nucleus
            .ball(module(&graph, &map, "db::put"))
            .expect("a sphere");
        assert!((route[2].distance(db.center) - db.radius).abs() < 0.5);
        assert!(nucleus.level(help).is_some(), "called by the start, named");
        assert!(
            nucleus.level(tile(&map, "core::idle")).is_none(),
            "two calls along, not named"
        );
    }

    #[test]
    fn what_is_named_inside_never_touches() {
        // The start and two of its callees all start in one spot.
        let mut at = vec![
            Vec3::ZERO,
            Vec3::new(0.01, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 0.3),
        ];
        let hops = [Some(0), Some(1), Some(1)];
        keep_apart(&mut at, &hops, 0.05, Vec3::Z);
        assert_eq!(at[0], Vec3::ZERO, "the start stays");
        let seen = |v: Vec3| v.truncate();
        for (i, a) in at.iter().enumerate() {
            for b in &at[i + 1..] {
                assert!(seen(*a).distance(seen(*b)) >= 0.1 - 1e-4, "apart as seen");
            }
        }
    }

    #[test]
    fn inside_callers_lean_one_way_and_callees_the_other() {
        // Focus 0; 1 and 2 call it; it calls 3 and 4; 5 relates to nothing.
        let start: Vec<Vec3> = (0..6).map(|i| fibonacci(i, 6) * 0.3).collect();
        let calls = [(1, 0), (2, 0), (0, 3), (0, 4)];
        let hops = [Some(0), Some(-1), Some(-1), Some(1), Some(1), None];
        let at = lay_inside(&start, &calls, &hops, Vec3::X, Vec3::Z);
        assert_eq!(at[0], Vec3::ZERO, "the start in the middle");
        assert!(at[1].x < 0.0 && at[2].x < 0.0, "callers to one side");
        assert!(at[3].x > 0.0 && at[4].x > 0.0, "callees to the other");
        assert!(at[5].length() > at[1].length(), "the rest towards the wall");
        for (i, a) in at.iter().enumerate() {
            assert!(a.length() <= 0.92 + 1e-4, "inside");
            for b in &at[i + 1..] {
                assert!(a.distance(*b) > 0.1, "apart");
            }
        }
    }

    #[test]
    fn modules_keep_their_homes() {
        let (graph, _) = codebase();
        assert_eq!(
            Nucleus::new(&graph, None, Trail::default()).homes,
            Nucleus::new(&graph, None, Trail::default()).homes
        );
    }

    #[test]
    fn the_cursor_shows_where_it_sits_without_moving_anything() {
        let (graph, map) = codebase();
        let mut nucleus = around(&graph, &map, "core::run", 1);
        let other = tile(&map, "db::other");
        let db = module(&graph, &map, "db::put");
        assert!(!nucleus.placed(other));
        let before: Vec<Ball> = nucleus.balls().map(|(_, b)| b).collect();
        assert!(nucleus.set_cursor(Some(other)));
        assert!(nucleus.placed(other) && nucleus.lit(other));
        let after: Vec<Ball> = nucleus.balls().map(|(_, b)| b).collect();
        assert_eq!(before, after, "nothing moves");
        let ball = nucleus.ball(db).expect("a sphere");
        let at = nucleus.point(other).expect("a point");
        assert!(
            at.distance(ball.center) > ball.radius,
            "just outside its module"
        );
        assert!(at.distance(ball.center) < ball.radius + 2.0);
        assert!(!nucleus.set_cursor(Some(other)), "no change");
        assert!(nucleus.set_cursor(None));
        assert!(!nucleus.placed(other));
    }

    #[test]
    fn specks_hide_in_a_journey_and_show_browsing() {
        let (graph, map) = codebase();
        // Two calls down from the start, past the depth shown: a speck in its module.
        let idle = tile(&map, "core::idle");
        let mut nucleus = around(&graph, &map, "core::run", 1);
        assert!(nucleus.speck(idle));
        assert!(!nucleus.drawn(idle), "a journey shows only itself");
        nucleus.expand(&graph, true);
        assert!(!nucleus.drawn(idle), "expanded too");
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
            let balls: Vec<(Vec3, f32)> =
                nucleus.balls().map(|(_, b)| (b.center, b.radius)).collect();
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

        assert!(nucleus.retrace(&graph, trail(&graph, run, 2)));
        assert!(!nucleus.retrace(&graph, trail(&graph, run, 2)), "no change");
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
        nucleus.expand(&graph, true);
        assert_eq!(named(&nucleus), modules, "the expanded view names the same");
        for &tile in &modules {
            assert!(nucleus.placed(tile), "and draws it");
        }
        assert_eq!(
            nucleus.ball(disk_module).map(|b| b.body),
            Some(Body::Open),
            "on the ring round the wall"
        );

        assert!(nucleus.retrace(&graph, trail(&graph, run, 1)));
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
        nucleus.expand(&graph, true);
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

        assert!(nucleus.retrace(&graph, trail(&graph, run, 2)));
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

        assert!(nucleus.retrace(&graph, trail(&graph, run, 2)));
        while nucleus.advance(0.1) {}
        assert!(drawn(&nucleus, disk), "a level more brings disk in");
        assert!(nucleus.placed(tile(&map, "disk::write")));
        assert_eq!(places(&nucleus), before, "where it was kept for it");

        assert!(nucleus.retrace(&graph, trail(&graph, run, 1)));
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
        nucleus.expand(&graph, true);
        while nucleus.advance(0.1) {}
        assert_eq!(clockwise(&nucleus), before, "expanded");
        nucleus.expand(&graph, false);
        while nucleus.advance(0.1) {}
        assert_eq!(clockwise(&nucleus), before, "and back");
    }

    #[test]
    fn a_function_run_twice_shows_once_with_both_numbers() {
        let mut trail = Trail::default();
        trail.add(0, 1, 1, "1");
        trail.add(0, 2, 1, "2");
        trail.add(2, 1, 2, "2.1");
        trail.add(1, 3, 2, "1.10");
        trail.add(1, 3, 2, "1.9");
        assert_eq!(trail.calls.len(), 4);
        assert_eq!(trail.numbers[&1], ["1", "2.1"]);
        assert_eq!(trail.numbers[&3], ["1.9", "1.10"], "in the journey's order");
        assert_eq!(badge(&trail.numbers[&1], None).as_deref(), Some("1 +1"));
        assert_eq!(badge(&trail.numbers[&2], None).as_deref(), Some("2"));
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
        let mut trail = Trail::default();
        trail.add(0, 1, 2, "1.1");
        trail.add(0, 1, 1, "2");
        assert_eq!(trail.numbers[&1], ["2", "1.1"], "the shallower first");
        assert_eq!(badge(&trail.numbers[&1], None).as_deref(), Some("2 +1"));
        // A level deeper adds a run further down, and the number stays.
        trail.add(1, 1, 3, "1.1.4");
        assert_eq!(badge(&trail.numbers[&1], None).as_deref(), Some("2 +2"));
    }

    #[test]
    fn many_names_keep_only_the_near_ones() {
        let (graph, map) = codebase();
        let mut nucleus = around(&graph, &map, "core::run", 2);
        let disk = tile(&map, "disk::write");
        assert!(nucleus.named(disk), "few enough to name them all");
        for tile in 1000..1000 + NAMED_ALL {
            nucleus.now.named.insert(tile, 3);
        }
        assert!(nucleus.named(disk), "two calls down keeps its name");
        assert!(
            !nucleus.named(1000),
            "three calls down is named under the pointer"
        );
    }

    #[test]
    fn names_past_a_column_spread_into_more_and_keep_inside() {
        assert_eq!(grid(3, 100.0).len(), 3);
        let many = grid(ROWS + 1, 100.0);
        let right = view_rotation() * Vec3::X;
        let across: Vec<f32> = many.iter().map(|p| p.dot(right)).collect();
        assert!(across.iter().any(|&x| x < -1.0) && across.iter().any(|&x| x > 1.0));
        for at in grid(ROWS * 3, 6.0) {
            assert!(at.length() < 6.0, "drawn in to fit a small module: {at}");
        }
        let names = names_grid(3, 100.0);
        assert_eq!(
            names.len(),
            3 + OUTSIDE_SHOWN,
            "room kept for outside callers"
        );
    }

    #[test]
    fn one_rule_sizes_every_module_growing_ever_slower() {
        let radii: Vec<f32> = [1, 2, 4, 10, 40, 100, 1000].map(module_radius).into();
        assert!(radii.is_sorted(), "more functions, no smaller: {radii:?}");
        for (pair, counts) in radii.windows(2).zip([[1, 2], [2, 4], [4, 10], [10, 40]]) {
            #[expect(clippy::cast_precision_loss, reason = "small counts")]
            let more = counts[1] as f32 / counts[0] as f32;
            assert!(pair[1] / pair[0] < more, "less than in proportion");
        }
        assert!(
            radii[6] / radii[0] <= 4.0,
            "at most four times the smallest"
        );
    }

    /// Pairs of `balls` whose outlines overlap on screen, as a camera from the usual
    /// view, looking at `target` from `distance`, sees them in perspective.
    fn overlapping(balls: &[Ball], target: Vec3, distance: f32) -> usize {
        let rotation = view_rotation();
        let (right, up, back) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z);
        let eye = target + back * distance;
        let tan = (std::f32::consts::FRAC_PI_4 / 2.0).tan();
        let seen: Vec<Option<(Vec2, f32)>> = balls
            .iter()
            .map(|b| {
                let offset = b.center - eye;
                let depth = -offset.dot(back);
                let at = Vec2::new(offset.dot(right), offset.dot(up)) / depth / tan;
                let on = depth > b.radius && at.x.abs() < 1.6 && at.y.abs() < 1.0;
                let radius = b.radius / (depth * depth - b.radius * b.radius).sqrt() / tan;
                on.then_some((at, radius))
            })
            .collect();
        let mut count = 0;
        for (i, a) in seen.iter().enumerate() {
            for b in &seen[i + 1..] {
                if let (Some((a, ra)), Some((b, rb))) = (a, b)
                    && a.distance(*b) < ra + rb
                {
                    count += 1;
                }
            }
        }
        count
    }

    #[test]
    fn browsing_no_two_modules_overlap_however_near_the_camera_comes() {
        // Modules spread through a deep ball of space, as a codebase's often are.
        let homes: Vec<Vec3> = (0..60)
            .map(|i| fibonacci(i, 60) * (20.0 + (i % 7) as f32 * 4.0))
            .collect();
        let counts: Vec<usize> = (0..60).map(|i| 1 + (i * 37) % 120).collect();
        let balls = atlas_balls(&homes, &counts);
        let middle = balls.iter().map(|b| b.center).sum::<Vec3>() / 60.0;
        let wide = balls
            .iter()
            .map(|b| b.center.distance(middle))
            .fold(0.0, f32::max);
        assert_eq!(overlapping(&balls, middle, wide * 2.5), 0, "the whole map");
        for ball in &balls {
            for distance in [80.0, 150.0] {
                assert_eq!(
                    overlapping(&balls, ball.center, distance),
                    0,
                    "framed close on one module, from {distance}"
                );
            }
        }
    }

    #[test]
    fn a_crowded_module_spreads_its_functions_apart_inside_it() {
        // A hundred functions all but on top of each other, as a module's often sit.
        let mut at: Vec<Vec2> = (0..106)
            .map(|i| {
                let p = fibonacci(i % 9, 9) * 0.3;
                Vec2::new(p.x, p.y)
            })
            .collect();
        let (disc, count) = (12.0, 106.0_f32);
        let apart = disc * PACKING / count.sqrt();
        spread(&mut at, apart, disc);
        for (i, a) in at.iter().enumerate() {
            assert!(a.length() <= disc + 1e-3, "inside");
            for b in &at[i + 1..] {
                assert!(a.distance(*b) > apart * 0.9, "apart: {}", a.distance(*b));
            }
        }
    }

    #[test]
    fn browsing_with_nothing_selected_every_module_points_to_each_it_calls() {
        let (graph, map) = codebase();
        let nucleus = Nucleus::gather(&graph, None, Trail::default(), true);
        let tubes = nucleus.tubes();
        assert!(tubes.iter().all(|t| t.flow == Flow::Module));
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
        assert!(
            module_strength(1) < module_strength(12),
            "more calls, plainer"
        );
        let selected = Nucleus::gather(
            &graph,
            Some(tile(&map, "core::run")),
            Trail::default(),
            true,
        );
        assert!(
            selected.tubes().iter().all(|t| t.flow != Flow::Module),
            "a selection's own calls take over"
        );
    }

    #[test]
    fn an_arrow_bends_round_a_sphere_in_its_way() {
        let right = view_rotation() * Vec3::X;
        let back = view_rotation() * Vec3::Z;
        let sphere = (Vec3::ZERO, 3.0);
        let path = detour(vec![-right * 10.0, right * 10.0], &[sphere], 0.5);
        assert!(path.len() > 2, "bent");
        for pair in path.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let flat = |v: Vec3| v - back * v.dot(back);
            let along = flat(b - a);
            let t = (flat(-a).dot(along) / along.length_squared()).clamp(0.0, 1.0);
            let nearest = flat(a + (b - a) * t);
            assert!(nearest.length() >= 3.0, "clear of the sphere: {nearest}");
        }
        let past = detour(
            vec![-right * 10.0 + Vec3::Y * 8.0, right * 10.0 + Vec3::Y * 8.0],
            &[sphere],
            0.5,
        );
        assert_eq!(past.len(), 2, "one already clear runs straight");
    }
}
