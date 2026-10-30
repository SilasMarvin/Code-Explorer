//! The nucleus (`Z`, or `--z nucleus`): the focus's module as a large glass sphere in
//! the middle with every one of its functions inside, and every other module a sphere
//! around it. Modules holding what calls the focus or what it calls come into a near
//! ring, opened just far enough to name those functions; a module holding what calls
//! those, or what they call, sits just beyond the module it reaches the focus
//! through. Everything else stays a small solid sphere further out, in the direction a
//! stable 3D layout of the modules has it, so a module keeps its bearing however the
//! focus moves. Thick arrows carry the calls from one module to another, violet into
//! the focus's module and blue out of it, and the calls this view names run inside
//! them, function to function. Space says only where things are: no axis stands for
//! order, rank or distance in calls. A new focus in another module re-centers on it,
//! easing every sphere to its new place.
//!
//! Expanded (`X`, a double click on the nucleus, or `--expand`), the nucleus grows to
//! several times its size as the camera comes in, and its functions are laid out by
//! the calls among them: what is within two calls of the focus, up or down, in the
//! middle, the focus's callers leaning one way and its callees the other, the rest
//! small towards the wall. Only the focus's own calls are drawn, and its callers and
//! callees named; the modules it calls or is called by shrink onto a ring just outside
//! the wall, at the bearings they had, and each of its calls leaving the module runs
//! out through the wall towards its module. With the pointer over another function
//! inside, that function's own calls in and out are drawn too, pale, and every end of
//! them named.

use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use bevy::prelude::*;
use codeviz_layout::ForceGraph;

use crate::{
    camera::{ANGLED, OrbitCamera},
    explorer::{Explorer, Hidden, Side},
    graph_view::{
        CALLEE, CALLER, FURTHER, fit_distance, flow, hidden_speck, node_size, orbit_anchor,
    },
    idle::Awake,
    scene::{PALETTE, lane_color},
};

/// How a module shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Body {
    /// The focus's module, in the middle, with every function inside it.
    Nucleus,
    /// Holds functions near the focus: opened to show those alone, by name.
    Open,
    /// Everything else: a small solid sphere.
    Closed,
}

/// A module's sphere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ball {
    pub(crate) center: Vec3,
    pub(crate) radius: f32,
    pub(crate) body: Body,
}

/// Which way a named call runs, seen from the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Flow {
    /// Calls the focus.
    Into,
    /// Called by the focus.
    OutOf,
    /// One call further out: into a caller, or out of a callee.
    Further,
}

impl Flow {
    pub(crate) fn color(self) -> Color {
        match self {
            Self::Into => CALLER,
            Self::OutOf => CALLEE,
            Self::Further => FURTHER,
        }
    }
}

/// Every call from one module to another, drawn as one arrow as thick as there are
/// calls, whenever it carries a call this view names.
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

/// Callers, and callees, of the focus named at most.
const NEAR_CAP: usize = 12;
/// Functions two calls away named at most, on each side.
const FAR_CAP: usize = 8;
/// Of those, at most this many through any one caller or callee.
const PER_STEP: usize = 3;
/// Functions an open module names at most.
const OPEN_CAP: usize = 7;
/// Between two functions named in an open module.
const SPACING: f32 = 2.8;
/// Between the nucleus and a module in the near ring, for its arrows to show.
const NEAR_GAP: f32 = 15.0;
/// Between a module in the near ring and one beyond it.
const BEYOND_GAP: f32 = 10.0;
/// Space kept clear between two spheres.
const MARGIN: f32 = 6.0;
/// Seconds a re-centering takes.
const RECENTER_SECONDS: f32 = 0.9;
/// How big a function is drawn, against its size in the flat graph.
const NODE_SCALE: f32 = 0.8;
/// How big a function in the nucleus that relates to nothing named is drawn.
const QUIET_SCALE: f32 = 0.3;
/// The angle the camera turns to when the nucleus opens.
pub(crate) const VIEW: (f32, f32) = (-0.45, -0.38);
/// How many times its size the nucleus grows to when expanded.
const GROWTH: f32 = 3.5;
/// How far outside the expanded nucleus's wall the modules round it sit, against its
/// radius.
const RIM_GAP: f32 = 0.14;
/// Kept clear between two modules round the expanded nucleus, against its radius.
const RIM_MARGIN: f32 = 0.08;
/// How much bigger a function is drawn inside the expanded nucleus.
const INSIDE_GROWTH: f32 = 2.2;
/// Calls up or down from the focus, within its module, that the expanded nucleus names.
const INSIDE_HOPS: i32 = 2;
/// Functions a module on the ring round the expanded nucleus names at most.
const RIM_NAMED: usize = 4;
/// Between two functions named in a module on that ring.
const RIM_SPACING: f32 = 5.0;
/// Between two named calls running side by side along one arrow.
const STRAND_GAP: f32 = 0.45;

/// Where everything is, for one focus.
#[derive(Debug, Clone, Default)]
struct Arrangement {
    focus: Option<usize>,
    /// The module in the middle.
    center: usize,
    /// Whether the nucleus is expanded.
    expanded: bool,
    /// By cluster.
    balls: Vec<Ball>,
    /// By tile.
    points: Vec<Vec3>,
    /// Functions drawn, by tile: the nucleus's and those opened around it.
    shown: Vec<bool>,
    /// Functions named outside the focus: callers, callees and those two calls away.
    named: HashSet<usize>,
    /// By cluster: how many functions an open module names.
    opened: Vec<usize>,
    tubes: Vec<Tube>,
    links: Vec<Link>,
}

/// One end of a call the function under the pointer makes or takes, expanded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum End {
    /// A function drawn: inside the nucleus, or named on the ring round it.
    Function(usize),
    /// Another module, where the call leaves the nucleus towards it.
    Module(usize),
}

/// A call the function under the pointer makes or takes, expanded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Touch {
    pub(crate) from: End,
    pub(crate) to: End,
}

/// The graph gathered round the focus's module.
#[derive(Debug, Clone)]
pub(crate) struct Nucleus {
    /// Each module's place in a stable 3D layout of modules, by cluster.
    homes: Vec<Vec3>,
    /// Functions in each module, by cluster.
    counts: Vec<usize>,
    /// Calls in or out of each function, by tile.
    degrees: Vec<usize>,
    /// Each function's module, by tile.
    module_of: Vec<Option<usize>>,
    /// Calls from one module to another, by `(from, to)` cluster.
    flows: HashMap<(usize, usize), usize>,
    /// Whether the focus's module is expanded to show how calls run inside it.
    expanded: bool,
    now: Arrangement,
    /// The function under the pointer, and, expanded, the calls it makes or takes.
    hovered: Option<usize>,
    touches: Vec<Touch>,
    /// Which sides of the focus are left out, with their calls.
    hidden: Hidden,
    /// The journey's cursor, when it is not the focus: marked where it sits, and
    /// drawn there though the arrangement folds it away.
    marked: Option<usize>,
    /// Whether the functions named by nothing are left out altogether, rather than
    /// drawn as faint specks: in the module view, and expanded.
    specks_hidden: [bool; 2],
    /// Where the spheres and functions were when the last re-centering began.
    from: Option<(Vec<Ball>, Vec<Vec3>)>,
    /// How far along that re-centering is, 0 to 1.
    progress: f32,
}

impl Nucleus {
    #[cfg(test)]
    pub(crate) fn new(graph: &ForceGraph, focus: Option<usize>) -> Self {
        Self::with_hidden(graph, focus, Hidden::default())
    }

    /// Gathers the graph round `focus`'s module, leaving out the sides `hidden` says.
    pub(crate) fn with_hidden(graph: &ForceGraph, focus: Option<usize>, hidden: Hidden) -> Self {
        let clusters = graph.cluster_count();
        let tiles = graph.positions.len();
        let mut counts = vec![0; clusters];
        for tile in 0..tiles {
            if let Some(cluster) = graph.cluster(tile) {
                counts[cluster] += 1;
            }
        }
        let mut degrees = vec![0; tiles];
        for &(a, b) in &graph.links {
            degrees[a] += 1;
            degrees[b] += 1;
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
        let mut nucleus = Self {
            homes,
            counts,
            degrees,
            module_of: (0..tiles).map(|t| graph.cluster(t)).collect(),
            flows,
            expanded: false,
            now: Arrangement::default(),
            hovered: None,
            touches: Vec::new(),
            hidden,
            marked: None,
            // Expanded, the specks crowd round what the reader points at.
            specks_hidden: [false, true],
            from: None,
            progress: 1.0,
        };
        nucleus.now = nucleus.arrange(graph, focus);
        nucleus
    }

    /// The function the nucleus is arranged around.
    pub(crate) fn focus(&self) -> Option<usize> {
        self.now.focus
    }

    /// The module in the middle.
    pub(crate) fn center(&self) -> usize {
        self.now.center
    }

    /// Whether the focus's module is expanded.
    pub(crate) fn expanded(&self) -> bool {
        self.expanded
    }

    /// Expands the focus's module, or shrinks it back among the others, easing there.
    pub(crate) fn expand(&mut self, graph: &ForceGraph, on: bool) {
        if on != self.expanded {
            self.expanded = on;
            self.recenter(graph, self.now.focus);
        }
    }

    /// Arranges everything around `focus` instead, easing there from wherever things
    /// are now.
    pub(crate) fn recenter(&mut self, graph: &ForceGraph, focus: Option<usize>) {
        let balls = (0..self.now.balls.len())
            .filter_map(|c| self.ball(c))
            .collect();
        let points = (0..self.now.points.len())
            .filter_map(|t| self.point(t))
            .collect();
        self.from = Some((balls, points));
        self.progress = 0.0;
        self.now = self.arrange(graph, focus);
        self.touches = self.touching(graph, self.hovered);
    }

    /// Leaves out a side of the focus, with its calls, or brings it back, easing
    /// there.
    pub(crate) fn set_hidden(&mut self, graph: &ForceGraph, hidden: Hidden) {
        if hidden != self.hidden {
            self.hidden = hidden;
            self.recenter(graph, self.now.focus);
        }
    }

    /// Whether the functions named by nothing are left out altogether in the view
    /// the nucleus is in, rather than drawn as faint specks.
    pub(crate) fn specks_hidden(&self) -> bool {
        self.specks_hidden[usize::from(self.expanded)]
    }

    /// Leaves out the functions named by nothing, or draws them again as specks, in
    /// the view the nucleus is in.
    pub(crate) fn toggle_specks(&mut self) {
        let hidden = &mut self.specks_hidden[usize::from(self.expanded)];
        *hidden = !*hidden;
    }

    /// Marks the journey's cursor where it sits, without moving anything else.
    /// Returns whether that changed.
    pub(crate) fn mark(&mut self, graph: &ForceGraph, tile: Option<usize>) -> bool {
        if tile == self.marked {
            return false;
        }
        self.marked = tile;
        // A call of the function under the pointer may now end at it.
        self.touches = self.touching(graph, self.hovered);
        true
    }

    /// Whether a function is the journey's cursor, marked where it sits.
    pub(crate) fn marks(&self, tile: usize) -> bool {
        self.marked == Some(tile)
    }

    /// The function the pointer has settled on.
    pub(crate) fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// Marks the function under the pointer. Returns whether that changed.
    pub(crate) fn hover(&mut self, graph: &ForceGraph, tile: Option<usize>) -> bool {
        if tile == self.hovered {
            return false;
        }
        self.hovered = tile;
        self.touches = self.touching(graph, tile);
        true
    }

    /// Expanded, the calls `tile` makes or takes when it is a function inside the
    /// nucleus other than the focus, whose own are drawn already; one for each
    /// function, or each other module where the far end is folded away.
    fn touching(&self, graph: &ForceGraph, tile: Option<usize>) -> Vec<Touch> {
        let Some(tile) = tile.filter(|&t| {
            self.now.expanded
                && Some(t) != self.now.focus
                && self.module_of.get(t).copied().flatten() == Some(self.now.center)
        }) else {
            return Vec::new();
        };
        let mut touches = Vec::new();
        for &(from, to) in &graph.calls {
            let other = match () {
                () if from == tile && to != tile => to,
                () if to == tile && from != tile => from,
                () => continue,
            };
            if Some(other) == self.now.focus {
                continue;
            }
            let end = match self.module_of.get(other).copied().flatten() {
                _ if self.shows(other) => End::Function(other),
                Some(cluster) => End::Module(cluster),
                None => continue,
            };
            let touch = if from == tile {
                Touch {
                    from: End::Function(tile),
                    to: end,
                }
            } else {
                Touch {
                    from: end,
                    to: End::Function(tile),
                }
            };
            if !touches.contains(&touch) {
                touches.push(touch);
            }
        }
        touches
    }

    /// Whether a function is drawn full size: named, the journey's cursor, or an end
    /// of a call the function under the pointer makes or takes.
    pub(crate) fn lit(&self, tile: usize) -> bool {
        self.names(tile) || self.marks(tile) || self.touched(tile)
    }

    /// Expanded, the calls the function under the pointer makes or takes.
    pub(crate) fn touches(&self) -> &[Touch] {
        &self.touches
    }

    /// Whether a function is an end of a call drawn: the focus's own, or the
    /// function under the pointer's. Such a function is always named.
    pub(crate) fn ends(&self, tile: usize) -> bool {
        self.now.expanded
            && self.shows(tile)
            && (self
                .now
                .links
                .iter()
                .any(|l| l.from == tile || l.to == tile)
                || self.touched(tile))
    }

    /// Whether a function is an end of a call the function under the pointer makes or
    /// takes.
    pub(crate) fn touched(&self, tile: usize) -> bool {
        self.touches
            .iter()
            .any(|t| t.from == End::Function(tile) || t.to == End::Function(tile))
    }

    /// Where a call the function under the pointer makes or takes ends, as drawn now,
    /// coming from `toward`: at a function, at the near side of a module on the ring,
    /// or else on the nucleus's wall on the way to that module.
    pub(crate) fn end_point(&self, end: End, toward: Vec3) -> Option<Vec3> {
        match end {
            End::Function(tile) => self.point(tile),
            End::Module(cluster) => {
                let ball = self.ball(cluster)?;
                if ball.body == Body::Open {
                    let back = (toward - ball.center).normalize_or_zero();
                    return Some(ball.center + back * ball.radius);
                }
                self.wall(cluster)
            }
        }
    }

    /// Where the nucleus's wall faces a module, as drawn now.
    pub(crate) fn wall(&self, cluster: usize) -> Option<Vec3> {
        let core = self.ball(self.now.center)?;
        let ball = self.ball(cluster)?;
        let out = (ball.center - core.center).try_normalize()?;
        Some(core.center + out * core.radius)
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
        if self.marks(tile) && !self.now.shown.get(tile).copied().unwrap_or(false) {
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

    /// Whether a function is drawn.
    pub(crate) fn shows(&self, tile: usize) -> bool {
        self.marks(tile) || self.now.shown.get(tile).copied().unwrap_or(false)
    }

    /// Whether a function is named as calling the focus, called by it, or two calls
    /// from it.
    pub(crate) fn names(&self, tile: usize) -> bool {
        self.now.named.contains(&tile)
    }

    /// How big a function is drawn against its size in the flat graph: small when it
    /// relates to nothing named, and bigger inside the expanded nucleus.
    pub(crate) fn node_scale(&self, quiet: bool) -> f32 {
        let scale = if quiet { QUIET_SCALE } else { NODE_SCALE };
        if self.now.expanded {
            scale * INSIDE_GROWTH
        } else {
            scale
        }
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
        self.now.opened.get(cluster).copied().unwrap_or(0)
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

    /// The path a named call is drawn along: straight within a module, else from the
    /// caller to its module's surface, along the arrow between the two modules, and
    /// in to the callee, each call on its own strand beside the others along the
    /// arrow. An end folded away inside its module stops at the module's surface.
    pub(crate) fn route(&self, link: &Link) -> Vec<Vec3> {
        let (Some(a), Some(b)) = (self.point(link.from), self.point(link.to)) else {
            return Vec::new();
        };
        let Some((from, to)) = self.tube_of(link) else {
            return vec![a, b];
        };
        let tube = self.now.tubes.iter().find(|t| t.from == from && t.to == to);
        let Some((tube, (start, end, _))) = tube.and_then(|t| Some((t, self.tube_ends(t)?))) else {
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
        let mut route = Vec::with_capacity(4);
        if self.shows(link.from) {
            route.push(a);
        }
        route.extend([start + offset, end + offset]);
        if self.shows(link.to) {
            route.push(b);
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
            .filter(|b| b.body != Body::Closed)
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

    /// Everything placed around `focus`, the nucleus expanded or not.
    fn arrange(&self, graph: &ForceGraph, focus: Option<usize>) -> Arrangement {
        let modules = self.arrange_modules(graph, focus);
        if self.expanded {
            self.arrange_inside(graph, modules)
        } else {
            modules
        }
    }

    /// Everything placed around `focus`, as the module view has it.
    fn arrange_modules(&self, graph: &ForceGraph, focus: Option<usize>) -> Arrangement {
        let clusters = self.homes.len();
        let center = focus
            .and_then(|f| graph.cluster(f))
            .or_else(|| (0..clusters).max_by_key(|&c| (self.counts[c], Reverse(c))))
            .unwrap_or(0);
        let cluster_of = |tile: usize| graph.cluster(tile).unwrap_or(center);
        let links = focus.map_or_else(Vec::new, |f| pick(graph, f, &self.degrees, self.hidden));
        let (links, opened, named) = open_up(links, focus, clusters, center, &cluster_of);

        let mut balls: Vec<Ball> = (0..clusters)
            .map(|c| Ball {
                center: Vec3::ZERO,
                radius: closed_radius(self.counts[c]),
                body: Body::Closed,
            })
            .collect();
        balls[center] = Ball {
            center: Vec3::ZERO,
            radius: nucleus_radius(self.counts[center]),
            body: Body::Nucleus,
        };
        let bearings = Bearings {
            homes: &self.homes,
            center,
            back: view_rotation() * Vec3::Z,
        };
        place_open(&mut balls, &opened, &links, &bearings, &cluster_of);
        place_closed(&mut balls, &bearings);
        let (points, shown) = place_functions(graph, &balls, &opened, center, &cluster_of);
        let tubes = self.join(&links, &cluster_of);
        Arrangement {
            focus,
            center,
            expanded: false,
            balls,
            points,
            shown,
            named,
            opened: opened.iter().map(Vec::len).collect(),
            tubes,
            links,
        }
    }

    /// The nucleus grown from `modules`, the module view, to several times its size,
    /// its functions laid out by the calls among them; the modules it calls or is
    /// called by on a ring just outside its wall, at the bearings they had, shrunk;
    /// and every other module pushed out by as much as it grew.
    fn arrange_inside(&self, graph: &ForceGraph, modules: Arrangement) -> Arrangement {
        let (center, focus) = (modules.center, modules.focus);
        let cluster_of = |tile: usize| graph.cluster(tile).unwrap_or(center);
        let before = modules.balls[center].radius;
        let core = before * GROWTH;
        let members: Vec<usize> = (0..graph.positions.len())
            .filter(|&t| cluster_of(t) == center)
            .collect();
        let calls: Vec<(usize, usize)> = graph
            .calls
            .iter()
            .copied()
            .filter(|&(a, b)| a != b && cluster_of(a) == center && cluster_of(b) == center)
            .collect();
        let reach = focus.map_or_else(HashMap::new, |f| within(&calls, f));
        let mut links = inside_links(&calls, focus, self.hidden);
        let exits = exits(graph, focus, center, &cluster_of, self.hidden);
        let mut rim: Vec<usize> = exits
            .iter()
            .map(|l| {
                let (from, to) = (cluster_of(l.from), cluster_of(l.to));
                if from == center { to } else { from }
            })
            .collect();
        rim.sort_unstable();
        rim.dedup();
        links.extend(exits);

        let mut balls = modules.balls.clone();
        for (cluster, ball) in balls.iter_mut().enumerate() {
            if cluster == center {
                ball.radius = core;
            } else {
                // Out of the way by as much as the nucleus grew.
                let out = ball.center.try_normalize().unwrap_or(Vec3::Y);
                ball.center += out * (core - before);
                ball.radius = closed_radius(self.counts[cluster]);
                ball.body = Body::Closed;
            }
        }
        let columns = named_outside(&links, self.homes.len(), center, &cluster_of);
        let radii: Vec<f32> = rim
            .iter()
            .map(|&c| (closed_radius(self.counts[c]) * 2.0).max(rim_radius(columns[c].len())))
            .collect();
        place_rim(&mut balls, &rim, &radii, core);

        // Laid out in a sphere of radius one, from where the module view had them.
        let start: Vec<Vec3> = members
            .iter()
            .map(|&t| modules.points[t] / before)
            .collect();
        let inside = lay_members(&members, &start, &calls, &reach);

        let tiles = graph.positions.len();
        let mut points: Vec<Vec3> = (0..tiles).map(|t| balls[cluster_of(t)].center).collect();
        let mut shown = vec![false; tiles];
        for (&tile, at) in members.iter().zip(inside) {
            points[tile] = at * core;
            shown[tile] = true;
        }
        for (cluster, column) in columns.iter().enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "a short list")]
            let top = (column.len() as f32 - 1.0) * RIM_SPACING / 2.0;
            for (row, &tile) in column.iter().enumerate() {
                #[expect(clippy::cast_precision_loss, reason = "a short list")]
                let y = top - row as f32 * RIM_SPACING;
                points[tile] = balls[cluster].center + Vec3::Y * y;
                shown[tile] = true;
            }
        }
        let tubes = self.join(&links, &cluster_of);
        Arrangement {
            focus,
            center,
            expanded: true,
            balls,
            points,
            shown,
            // The focus's callers and callees alone: what is further along is drawn
            // only while the pointer is over what leads to it.
            named: links
                .iter()
                .flat_map(|l| [l.from, l.to])
                .chain(columns.iter().flatten().copied())
                .filter(|&t| Some(t) != focus)
                .collect(),
            opened: columns.iter().map(Vec::len).collect(),
            tubes,
            links,
        }
    }

    /// An arrow for each pair of modules a named call runs between, as thick as all
    /// the calls between them.
    fn join(&self, links: &[Link], cluster_of: &impl Fn(usize) -> usize) -> Vec<Tube> {
        let mut tubes: Vec<Tube> = Vec::new();
        for link in links {
            let (from, to) = (cluster_of(link.from), cluster_of(link.to));
            if from == to {
                continue;
            }
            match tubes.iter_mut().find(|t| t.from == from && t.to == to) {
                // Violet or blue wins over pale, so an arrow touching the focus says so.
                Some(tube) if tube.flow == Flow::Further => tube.flow = link.flow,
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
    center: usize,
    back: Vec3,
}

impl Bearings<'_> {
    /// The way to a module's home, split into across the usual view and towards it.
    fn split(&self, cluster: usize) -> (Vec3, f32) {
        let full = (self.homes[cluster] - self.homes[self.center])
            .try_normalize()
            .unwrap_or_else(|| fibonacci(cluster, self.homes.len()));
        let toward = full.dot(self.back);
        let across = (full - self.back * toward)
            .try_normalize()
            .unwrap_or_else(|| sideways(self.back));
        (across, toward)
    }

    /// For the near ring: round the nucleus as the usual view sees it, tilted towards
    /// and away from the camera by only part of the way, so no module in it hides in
    /// front of or behind the nucleus.
    fn near(&self, cluster: usize) -> Vec3 {
        let (across, toward) = self.split(cluster);
        (across + self.back * toward * 0.6).normalize()
    }

    /// For the rest of the codebase: standing back behind the nucleus, a bowl seen
    /// from the usual view, so it never crowds in front of what matters.
    fn behind(&self, cluster: usize) -> Vec3 {
        let (across, toward) = self.split(cluster);
        (across - self.back * (0.3 + 0.7 * toward.abs())).normalize()
    }

    /// How far a module's home is from the middle's, against the typical module's.
    fn remoteness(&self, cluster: usize) -> f32 {
        let distance = |c: usize| self.homes[c].distance(self.homes[self.center]);
        let mut spans: Vec<f32> = (0..self.homes.len())
            .filter(|&c| c != self.center)
            .map(distance)
            .collect();
        spans.sort_by(f32::total_cmp);
        let typical = spans.get(spans.len() / 2).copied().unwrap_or(1.0).max(1.0);
        distance(cluster) / typical
    }
}

/// Which functions each module outside the middle opens to name, as many as fit,
/// and the links left between what is named; and every function named but the focus.
fn open_up(
    mut links: Vec<Link>,
    focus: Option<usize>,
    clusters: usize,
    center: usize,
    cluster_of: &impl Fn(usize) -> usize,
) -> (Vec<Link>, Vec<Vec<usize>>, HashSet<usize>) {
    let mut opened: Vec<Vec<usize>> = vec![Vec::new(); clusters];
    let mut kept: HashSet<usize> = focus.into_iter().collect();
    for link in &links {
        for tile in [link.from, link.to] {
            let cluster = cluster_of(tile);
            if kept.contains(&tile) {
                continue;
            }
            if cluster == center {
                kept.insert(tile);
            } else if opened[cluster].len() < OPEN_CAP {
                opened[cluster].push(tile);
                kept.insert(tile);
            }
        }
    }
    links.retain(|l| kept.contains(&l.from) && kept.contains(&l.to));
    let named = kept.into_iter().filter(|&t| Some(t) != focus).collect();
    (links, opened, named)
}

/// Opens the modules that name something into a ring round the nucleus: those
/// holding a caller or callee of the focus out along their own bearing, those that
/// reach the focus only through another just beyond that one; then holds them apart.
fn place_open(
    balls: &mut [Ball],
    opened: &[Vec<usize>],
    links: &[Link],
    bearings: &Bearings,
    cluster_of: &impl Fn(usize) -> usize,
) {
    let core = balls[bearings.center].radius;
    let open: Vec<usize> = (0..balls.len())
        .filter(|&c| c != bearings.center && !opened[c].is_empty())
        .collect();
    for &cluster in &open {
        let radius = open_radius(opened[cluster].len());
        balls[cluster] = Ball {
            center: bearings.near(cluster) * (core + NEAR_GAP + radius),
            radius,
            body: Body::Open,
        };
    }
    let touches_focus = |cluster: usize| {
        links.iter().any(|l| {
            l.flow != Flow::Further
                && (cluster_of(l.from) == cluster || cluster_of(l.to) == cluster)
        })
    };
    for &cluster in open.iter().filter(|&&c| !touches_focus(c)) {
        let through = links.iter().find_map(|l| {
            let (a, b) = (cluster_of(l.from), cluster_of(l.to));
            match () {
                () if a == cluster && b != cluster => Some(b),
                () if b == cluster && a != cluster => Some(a),
                () => None,
            }
        });
        let Some(through) = through.filter(|&t| t != bearings.center) else {
            continue;
        };
        let base = balls[through];
        let out =
            (base.center.normalize_or_zero() * 2.0 + bearings.near(cluster)).normalize_or_zero();
        let radius = balls[cluster].radius;
        balls[cluster].center = base.center + out * (base.radius + BEYOND_GAP + radius);
    }
    // Apart as the usual view sees them, and as it sees them turned an eighth of the
    // way either side, so none hides another or its names.
    let views: Vec<Vec3> = [0.0, -1.0, 1.0]
        .into_iter()
        .map(|turn| turned_view(turn * std::f32::consts::FRAC_PI_4) * Vec3::Z)
        .collect();
    relax(balls, |b| b.body == Body::Open, MARGIN, &views, 300);
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
    relax(balls, |b| b.body == Body::Closed, 1.5, &[], 60);
}

/// Where each function is drawn, and whether it is: the nucleus's own where the flat
/// graph has them, filling its sphere; those an open module names in a column down
/// its middle; the rest folded away at their module's middle.
fn place_functions(
    graph: &ForceGraph,
    balls: &[Ball],
    opened: &[Vec<usize>],
    center: usize,
    cluster_of: &impl Fn(usize) -> usize,
) -> (Vec<Vec3>, Vec<bool>) {
    let tiles = graph.positions.len();
    let mut points: Vec<Vec3> = (0..tiles).map(|t| balls[cluster_of(t)].center).collect();
    let mut shown = vec![false; tiles];
    let members: Vec<usize> = (0..tiles).filter(|&t| cluster_of(t) == center).collect();
    for (tile, at) in interior(graph, &members, balls[center].radius) {
        points[tile] = at;
        shown[tile] = true;
    }
    for (cluster, column) in opened.iter().enumerate() {
        #[expect(clippy::cast_precision_loss, reason = "a short list")]
        let top = (column.len() as f32 - 1.0) * SPACING / 2.0;
        for (row, &tile) in column.iter().enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "a short list")]
            let y = top - row as f32 * SPACING;
            points[tile] = balls[cluster].center + Vec3::Y * y;
            shown[tile] = true;
        }
    }
    (points, shown)
}

/// The calls named around `focus`: what calls it and what it calls, the busiest
/// first, then what calls those callers and what those callees call, a few through
/// each; none on a side `hidden` leaves out.
fn pick(graph: &ForceGraph, focus: usize, degrees: &[usize], hidden: Hidden) -> Vec<Link> {
    let busiest = |tiles: &mut dyn Iterator<Item = usize>| {
        let mut list: Vec<usize> = tiles.filter(|&t| t != focus).collect();
        list.sort_unstable();
        list.dedup();
        list.sort_by_key(|&t| (Reverse(degrees.get(t).copied().unwrap_or(0)), t));
        list
    };
    let side = |side: Side, tiles: &mut dyn Iterator<Item = usize>| {
        let mut list = if hidden.hides(side) {
            Vec::new()
        } else {
            busiest(tiles)
        };
        list.truncate(NEAR_CAP);
        list
    };
    let inward = side(Side::Callers, &mut graph.callers(focus));
    let outward = side(Side::Callees, &mut graph.callees(focus));
    let mut links: Vec<Link> = inward
        .iter()
        .map(|&from| Link {
            from,
            to: focus,
            flow: Flow::Into,
        })
        .chain(outward.iter().map(|&to| Link {
            from: focus,
            to,
            flow: Flow::OutOf,
        }))
        .collect();
    let mut taken: HashSet<usize> = inward.iter().chain(&outward).copied().collect();
    taken.insert(focus);
    let mut further = 0;
    for &near in &inward {
        for outer in busiest(&mut graph.callers(near)).into_iter().take(PER_STEP) {
            if further < FAR_CAP && taken.insert(outer) {
                further += 1;
                links.push(Link {
                    from: outer,
                    to: near,
                    flow: Flow::Further,
                });
            }
        }
    }
    further = 0;
    for &near in &outward {
        for outer in busiest(&mut graph.callees(near)).into_iter().take(PER_STEP) {
            if further < FAR_CAP && taken.insert(outer) {
                further += 1;
                links.push(Link {
                    from: near,
                    to: outer,
                    flow: Flow::Further,
                });
            }
        }
    }
    links
}

/// Functions within [`INSIDE_HOPS`] calls of `focus` along `calls`, by how many calls
/// up from it (negative) or down from it (positive); the focus itself is 0. Upstream
/// wins a tie, as it does for the graph's own reach.
fn within(calls: &[(usize, usize)], focus: usize) -> HashMap<usize, i32> {
    let walk = |upward: bool| {
        let mut seen: HashMap<usize, i32> = HashMap::new();
        let mut ring = vec![focus];
        for hop in 1..=INSIDE_HOPS {
            let mut next = Vec::new();
            for &(a, b) in calls {
                let (from, to) = if upward { (b, a) } else { (a, b) };
                if to != focus && ring.contains(&from) && !seen.contains_key(&to) {
                    seen.insert(to, hop);
                    next.push(to);
                }
            }
            ring = next;
        }
        seen
    };
    let mut reach = walk(false);
    for (tile, up) in walk(true) {
        if reach.get(&tile).is_none_or(|&down| up <= down) {
            reach.insert(tile, -up);
        }
    }
    reach.insert(focus, 0);
    reach
}

/// The calls inside the module into the focus and out of it, but for a side
/// `hidden` leaves out.
fn inside_links(calls: &[(usize, usize)], focus: Option<usize>, hidden: Hidden) -> Vec<Link> {
    calls
        .iter()
        .filter_map(|&(from, to)| {
            let flow = match () {
                () if Some(to) == focus => Flow::Into,
                () if Some(from) == focus => Flow::OutOf,
                () => return None,
            };
            Some(Link { from, to, flow }).filter(|l| shown(l, hidden))
        })
        .collect()
}

/// Whether a call into or out of the focus is on a side `hidden` keeps.
fn shown(link: &Link, hidden: Hidden) -> bool {
    match link.flow {
        Flow::Into => !hidden.callers,
        Flow::OutOf => !hidden.callees,
        Flow::Further => true,
    }
}

/// The focus's calls leaving the module, or coming into it, each once, but for a
/// side `hidden` leaves out.
fn exits(
    graph: &ForceGraph,
    focus: Option<usize>,
    center: usize,
    cluster_of: &impl Fn(usize) -> usize,
    hidden: Hidden,
) -> Vec<Link> {
    let mut seen = HashSet::new();
    let mut links = Vec::new();
    for &(from, to) in &graph.calls {
        let flow = match () {
            () if Some(from) == focus && cluster_of(to) != center => Flow::OutOf,
            () if Some(to) == focus && cluster_of(from) != center => Flow::Into,
            () => continue,
        };
        let link = Link { from, to, flow };
        if shown(&link, hidden) && seen.insert((from, to)) {
            links.push(link);
        }
    }
    links
}

/// The functions outside the module that the focus calls or is called by, a few in
/// each module, by cluster: they stay named in their modules round the nucleus.
fn named_outside(
    links: &[Link],
    clusters: usize,
    center: usize,
    cluster_of: &impl Fn(usize) -> usize,
) -> Vec<Vec<usize>> {
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); clusters];
    for link in links.iter().filter(|l| l.flow != Flow::Further) {
        let outside = if cluster_of(link.from) == center {
            link.to
        } else {
            link.from
        };
        if cluster_of(outside) == center {
            continue;
        }
        let column = &mut columns[cluster_of(outside)];
        if column.len() < RIM_NAMED && !column.contains(&outside) {
            column.push(outside);
        }
    }
    columns
}

/// [`lay_inside`] for a module's `members`, by tile, from `start`, with the `calls`
/// among them and how far each is from the focus by `reach`; turned to the usual view.
fn lay_members(
    members: &[usize],
    start: &[Vec3],
    calls: &[(usize, usize)],
    reach: &HashMap<usize, i32>,
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
    lay_inside(
        &start,
        &calls,
        &hops,
        rotation * Vec3::X,
        rotation * Vec3::Z,
    )
}

/// Puts the modules in `rim` on a ring just outside the expanded nucleus's wall, as
/// the usual view sees it, each at the bearing it had and shrunk, then spreads them
/// round it until none crowds another.
fn place_rim(balls: &mut [Ball], rim: &[usize], radii: &[f32], core: f32) {
    let rotation = view_rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    let widest = radii.iter().copied().fold(0.0, f32::max);
    let ring = core * (1.0 + RIM_GAP) + widest;
    let mut angles: Vec<f32> = rim
        .iter()
        .map(|&c| {
            let at = balls[c].center;
            at.dot(up).atan2(at.dot(right))
        })
        .collect();
    let mut order: Vec<usize> = (0..rim.len()).collect();
    order.sort_by(|&a, &b| angles[a].total_cmp(&angles[b]));
    if rim.len() > 1 {
        for _ in 0..200 {
            let mut moved = false;
            for k in 0..order.len() {
                let (i, j) = (order[k], order[(k + 1) % order.len()]);
                let need = (radii[i] + radii[j] + core * RIM_MARGIN) / ring;
                let gap = (angles[j] - angles[i]).rem_euclid(std::f32::consts::TAU);
                if gap < need {
                    angles[i] -= (need - gap) / 2.0;
                    angles[j] += (need - gap) / 2.0;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
    }
    for ((&cluster, &radius), angle) in rim.iter().zip(radii).zip(angles) {
        balls[cluster] = Ball {
            center: (right * angle.cos() + up * angle.sin()) * (core * (1.0 + RIM_GAP) + radius),
            radius,
            body: Body::Open,
        };
    }
}

/// Lays a module's functions out in a sphere of radius one by the calls among them,
/// from where `start` has them. What is near the focus, by `hops` (none for the rest),
/// gathers round it in the middle with room between, and the focus's callers lean
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
                    let lean = hops as f32 * 0.22 - at[i].dot(right);
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

/// The nucleus's size, roomy enough for all its functions, and never so small that
/// it does not read as the middle.
fn nucleus_radius(count: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a function count")]
    (5.0 + 1.25 * (count as f32).sqrt()).clamp(8.0, 18.0)
}

/// The usual view's rotation.
fn view_rotation() -> Quat {
    turned_view(0.0)
}

/// The usual view turned round by `yaw`.
fn turned_view(yaw: f32) -> Quat {
    Quat::from_euler(EulerRot::YXZ, VIEW.0 + yaw, VIEW.1, 0.0)
}

/// A module's size on the ring round the expanded nucleus, just enough for the
/// column it names.
fn rim_radius(named: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a short list")]
    (named.saturating_sub(1) as f32 * RIM_SPACING / 2.0 + 2.6)
}

/// An open module's size, just enough for the column it names.
fn open_radius(named: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a short list")]
    ((named as f32 - 1.0) * SPACING / 2.0 + 2.2).max(2.6)
}

/// A closed module's size, growing slowly with what it holds.
fn closed_radius(count: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a function count")]
    (0.5 + 0.3 * (count as f32).cbrt())
}

/// An arrow's thickness, for this many calls: growing slowly, and never so thick that
/// the calls running along it are lost.
pub(crate) fn tube_radius(calls: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a call count")]
    (0.08 + 0.05 * (calls as f32).ln_1p()).min(0.3)
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
/// sees them, moving across it; given none, in space.
fn relax(
    balls: &mut [Ball],
    movable: impl Fn(&Ball) -> bool,
    margin: f32,
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
                let (move_a, move_b) = (movable(&balls[i]), movable(&balls[j]));
                if !move_a && !move_b {
                    continue;
                }
                for &seen_from in &seen {
                    let (a, b) = (balls[i], balls[j]);
                    let need = a.radius + b.radius + margin;
                    let apart = b.center - a.center;
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

/// Where the nucleus's functions sit inside a sphere of `radius`: as the flat graph
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

/// Z gathers the graph round the focus's module, or lays it flat again; X expands the
/// nucleus, or shrinks it back, gathering the graph first when it lies flat; N leaves
/// out the functions named by nothing, or draws them again as specks.
pub(crate) fn toggle(keys: &ButtonInput<KeyCode>, explorer: &mut Explorer) {
    if keys.just_pressed(KeyCode::KeyN) {
        explorer.toggle_specks();
    }
    if keys.just_pressed(KeyCode::KeyZ) {
        let on = explorer.nucleus().is_none();
        explorer.set_nucleus(on);
    }
    if keys.just_pressed(KeyCode::KeyX) {
        let on = !explorer.nucleus().is_some_and(Nucleus::expanded);
        explorer.expand_nucleus(on);
    }
}

/// Longest between the two clicks of a double click, in seconds.
const DOUBLE_CLICK: f32 = 0.4;

/// A double click on the nucleus, off any function, expands it or shrinks it back.
/// A press that drags to orbit is no click.
pub(crate) fn double_click(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<OrbitCamera>>,
    mut explorer: ResMut<Explorer>,
    mut pressed: Local<Option<Vec2>>,
    mut last: Local<Option<f32>>,
) {
    let cursor = window.cursor_position();
    if buttons.just_pressed(MouseButton::Left) {
        *pressed = cursor;
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let clicked = matches!((pressed.take(), cursor), (Some(a), Some(b)) if a.distance(b) < 4.0);
    let on_nucleus = || {
        let nucleus = explorer.nucleus()?;
        let ball = nucleus.ball(nucleus.center())?;
        let (camera, global) = cameras.single().ok()?;
        let corner = camera.logical_viewport_rect().map_or(Vec2::ZERO, |r| r.min);
        let ray = camera.viewport_to_world(global, cursor? - corner).ok()?;
        let to_middle = ball.center - ray.origin;
        let nearest = ray.origin + *ray.direction * to_middle.dot(*ray.direction);
        Some(nearest.distance(ball.center) <= ball.radius)
    };
    if !clicked || explorer.hovered_tile.is_some() || on_nucleus() != Some(true) {
        *last = None;
        return;
    }
    let now = time.elapsed_secs();
    if last.is_some_and(|at| now - at <= DOUBLE_CLICK) {
        *last = None;
        let on = !explorer.nucleus().is_some_and(Nucleus::expanded);
        explorer.expand_nucleus(on);
    } else {
        *last = Some(now);
    }
}

/// Seconds the names round the function under the pointer take to settle once the
/// pointer moves onto it or off it.
const HOVER_SETTLE: f32 = 0.4;

/// Seconds the pointer rests on a function, or off it, before the nucleus takes it
/// as pointed at or no longer: passing over one on the way elsewhere draws nothing.
const HOVER_DELAY: f32 = 0.15;

/// Holds a change of what the pointer is over until it has lasted [`HOVER_DELAY`].
#[derive(Debug, Default)]
pub(crate) struct Settle {
    /// What the pointer moved onto, and since when, in seconds of app time.
    pending: Option<(Option<usize>, f32)>,
}

impl Settle {
    /// Given what the pointer is over, what is taken as pointed at now and the time:
    /// whether to take what the pointer is over instead, as it has stayed there long
    /// enough.
    fn settle(&mut self, over: Option<usize>, taken: Option<usize>, now: f32) -> bool {
        if over == taken {
            self.pending = None;
            return false;
        }
        match self.pending {
            Some((tile, since)) if tile == over => {
                let landed = now - since >= HOVER_DELAY;
                if landed {
                    self.pending = None;
                }
                landed
            }
            _ => {
                self.pending = Some((over, now));
                false
            }
        }
    }

    /// Whether a change is being held, so the frames keep coming until it lands.
    fn waiting(&self) -> bool {
        self.pending.is_some()
    }
}

/// Expanded, draws the calls of the function the pointer has rested on, and keeps
/// drawing while the names at their ends fade in or out. A speck left out is never
/// pointed at, though the pointer was on it when it went.
pub(crate) fn hover(
    time: Res<Time>,
    mut explorer: ResMut<Explorer>,
    mut awake: ResMut<Awake>,
    mut held: Local<Settle>,
    mut settling: Local<f32>,
) {
    let Some(taken) = explorer.nucleus().map(Nucleus::hovered) else {
        *held = Settle::default();
        return;
    };
    let over = explorer
        .hovered_tile
        .filter(|&t| !hidden_speck(&explorer, t));
    if held.settle(over, taken, time.elapsed_secs())
        && explorer.bypass_change_detection().hover_nucleus(over)
    {
        explorer.set_changed();
        *settling = HOVER_SETTLE;
    }
    if held.waiting() {
        awake.0 = true;
    }
    if *settling > 0.0 {
        *settling -= time.delta_secs();
        awake.0 = true;
    }
}

/// Re-centers on the focus's module when the focus moves, and eases the spheres
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

/// Looks at the nucleus from far enough back to see it and every open module, turning
/// to a three-quarter view from above when it first opens; again whenever the view is
/// pointed somewhere new. Turns back to the usual view when the graph lies flat again.
pub(crate) fn frame(
    explorer: Res<Explorer>,
    mut last: Local<Option<(Option<usize>, u64, bool)>>,
    mut cameras: Query<(&mut OrbitCamera, &Transform, &Projection)>,
) {
    let Some(nucleus) = explorer.nucleus() else {
        // Laid flat again: back to the usual angle, round the journey's start.
        if last.take().is_some() {
            for (mut camera, ..) in &mut cameras {
                camera.turn_to(ANGLED);
                orbit_anchor(&explorer, &mut camera);
            }
        }
        return;
    };
    let key = (nucleus.focus(), explorer.aim(), nucleus.expanded());
    // Expanding or shrinking back turns to the usual view too, where the ring of
    // modules round the nucleus keeps the bearings they had.
    let opening = last.is_none_or(|(.., expanded)| expanded != nucleus.expanded());
    if *last == Some(key) {
        return;
    }
    *last = Some(key);
    for (mut camera, transform, projection) in &mut cameras {
        if opening {
            camera.turn_to(VIEW);
        }
        let rotation = if opening {
            view_rotation()
        } else {
            transform.rotation
        };
        fit(nucleus, &mut camera, rotation, projection);
    }
}

/// Glides to the nucleus from far enough back, turned to `rotation`, to see it all.
pub(crate) fn fit(
    nucleus: &Nucleus,
    camera: &mut OrbitCamera,
    rotation: Quat,
    projection: &Projection,
) {
    let Projection::Perspective(perspective) = projection else {
        return;
    };
    let points = nucleus.frame_points(rotation);
    camera.glide_to(
        Vec3::ZERO,
        fit_distance(&points, Vec3::ZERO, rotation, perspective),
    );
}

/// A module's sphere in the scene.
#[derive(Component, Debug)]
pub(crate) struct ModuleSphere(usize);

/// One of the arrows between modules in the scene: its shaft, or its head.
#[derive(Component, Debug)]
pub(crate) struct Arrow {
    index: usize,
    head: bool,
}

/// What spheres and arrows are drawn with.
#[derive(Resource, Debug)]
pub(crate) struct Looks {
    sphere: Handle<Mesh>,
    shaft: Handle<Mesh>,
    head: Handle<Mesh>,
    /// By color in [`PALETTE`]: glass for the nucleus, for an open module, and a
    /// solid for a closed one.
    nucleus: Vec<Handle<StandardMaterial>>,
    open: Vec<Handle<StandardMaterial>>,
    closed: Vec<Handle<StandardMaterial>>,
    /// By [`Flow`]: an arrow's shaft and its head.
    shafts: HashMap<Flow, Handle<StandardMaterial>>,
    heads: HashMap<Flow, Handle<StandardMaterial>>,
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
    let colors: Vec<Color> = (0..PALETTE.len()).map(lane_color).collect();
    let flows = [Flow::Into, Flow::OutOf, Flow::Further];
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
        shafts: flows
            .iter()
            .map(|&f| {
                let alpha = if f == Flow::Further { 0.18 } else { 0.32 };
                (f, materials.add(see_through(f.color(), alpha)))
            })
            .collect(),
        heads: flows
            .iter()
            .map(|&f| {
                let alpha = if f == Flow::Further { 0.45 } else { 0.8 };
                (f, materials.add(see_through(f.color(), alpha)))
            })
            .collect(),
    };
    commands.insert_resource(looks);
}

/// Spawns a sphere per module once the nucleus first opens, and an arrow per pair of
/// modules as many as are needed, then keeps each where the nucleus has it, drawn as
/// its body asks; hides them all while the graph is flat.
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
) {
    let Some(nucleus) = explorer.nucleus() else {
        for (_, _, mut visibility, _) in &mut spheres {
            visibility.set_if_neq(Visibility::Hidden);
        }
        for (_, _, mut visibility, _) in &mut arrows {
            visibility.set_if_neq(Visibility::Hidden);
        }
        return;
    };
    let clusters = explorer.graph().cluster_count();
    if spheres.is_empty() {
        for cluster in 0..clusters {
            commands.spawn((
                ModuleSphere(cluster),
                Mesh3d(looks.sphere.clone()),
                MeshMaterial3d(looks.closed[cluster % PALETTE.len()].clone()),
                Transform::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
            ));
        }
    }
    for (sphere, mut transform, mut visibility, mut material) in &mut spheres {
        let Some(ball) = nucleus.ball(sphere.0) else {
            continue;
        };
        let color = sphere.0 % PALETTE.len();
        let wanted = match ball.body {
            Body::Nucleus => &looks.nucleus[color],
            Body::Open => &looks.open[color],
            Body::Closed => &looks.closed[color],
        };
        if &material.0 != wanted {
            material.0 = wanted.clone();
        }
        let placed = Transform::from_translation(ball.center).with_scale(Vec3::splat(ball.radius));
        transform.set_if_neq(placed);
        visibility.set_if_neq(Visibility::Inherited);
    }

    let tubes = nucleus.tubes();
    let have = arrows.iter().filter(|(a, ..)| !a.head).count();
    for index in have..tubes.len() {
        for head in [false, true] {
            commands.spawn((
                Arrow { index, head },
                Mesh3d(if head {
                    looks.head.clone()
                } else {
                    looks.shaft.clone()
                }),
                MeshMaterial3d(looks.shafts[&Flow::Further].clone()),
                Transform::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
            ));
        }
    }
    for (arrow, mut transform, mut visibility, mut material) in &mut arrows {
        let Some((tube, (start, end, radius))) = tubes
            .get(arrow.index)
            .and_then(|t| Some((t, nucleus.tube_ends(t)?)))
        else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        let along = end - start;
        let length = along.length();
        if length < 0.5 {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        }
        let direction = along / length;
        // Small and narrow, so the calls running in beside it stay in view.
        let zoom = nucleus.zoom();
        let head = (radius * 6.0).max(0.9 * zoom).min(length * 0.4);
        let width = (radius * 3.5).max(0.35 * zoom);
        let rotation = Quat::from_rotation_arc(Vec3::Y, direction);
        let placed = if arrow.head {
            Transform {
                translation: end - direction * head / 2.0,
                rotation,
                scale: Vec3::new(width, head, width),
            }
        } else {
            Transform {
                translation: start + along * ((length - head) / length) / 2.0,
                rotation,
                scale: Vec3::new(radius, length - head, radius),
            }
        };
        transform.set_if_neq(placed);
        let wanted = if arrow.head {
            &looks.heads[&tube.flow]
        } else {
            &looks.shafts[&tube.flow]
        };
        if &material.0 != wanted {
            material.0 = wanted.clone();
        }
        visibility.set_if_neq(Visibility::Inherited);
    }
}

/// A rim round each glass sphere, facing the camera, so its edge reads as one
/// outline; and the named calls, function to function along their arrows, with dots
/// flowing along those that touch the focus.
pub(crate) fn draw(
    explorer: Res<Explorer>,
    clock: Res<Time>,
    cameras: Query<&Transform, With<OrbitCamera>>,
    mut gizmos: Gizmos,
) {
    let Some(nucleus) = explorer.nucleus() else {
        return;
    };
    let Ok(camera) = cameras.single() else {
        return;
    };
    for (cluster, ball) in nucleus.balls() {
        let color = lane_color(cluster % PALETTE.len());
        // Brighter towards the edge, as glass is, with a faint halo outside it.
        let rims: &[(f32, f32)] = match ball.body {
            Body::Nucleus => &[
                (0.9, 0.05),
                (0.94, 0.09),
                (0.97, 0.16),
                (0.99, 0.32),
                (1.0, 0.85),
                (1.025, 0.18),
            ],
            Body::Open => &[(0.92, 0.06), (0.96, 0.12), (0.985, 0.25), (1.0, 0.7)],
            Body::Closed => &[],
        };
        for &(grow, alpha) in rims {
            gizmos
                .circle(
                    Isometry3d::new(ball.center, camera.rotation),
                    ball.radius * grow,
                    color.with_alpha(alpha),
                )
                .resolution(96);
        }
    }
    let graph = explorer.graph();
    let time = clock.elapsed_secs();
    let zoom = nucleus.zoom();
    // Short of a function's surface, so the head stays visible.
    let short = |from: Vec3, to: Vec3, tile: usize| {
        if !nucleus.shows(tile) {
            return to;
        }
        let quiet = !nucleus.lit(tile) && nucleus.focus() != Some(tile);
        to - (to - from).normalize_or_zero()
            * node_size(graph, tile)
            * nucleus.node_scale(quiet)
            * 0.5
    };
    // Expanded, the calls of the function under the pointer, pale.
    for touch in nucleus.touches() {
        let toward = |end: End| match end {
            End::Function(function) => nucleus.point(function),
            End::Module(_) => None,
        };
        let Some(near) = toward(touch.from).or_else(|| toward(touch.to)) else {
            continue;
        };
        let (Some(a), Some(b)) = (
            nucleus.end_point(touch.from, near),
            nucleus.end_point(touch.to, near),
        ) else {
            continue;
        };
        let b = match touch.to {
            End::Function(callee) => short(a, b, callee),
            End::Module(_) => b,
        };
        gizmos
            .arrow(a, b, FURTHER.with_alpha(0.8))
            .with_tip_length(0.7 * zoom);
    }
    for (index, link) in nucleus.links().iter().enumerate() {
        let route = nucleus.route(link);
        let [.., last_but_one, last] = route.as_slice() else {
            continue;
        };
        let strength = if link.flow == Flow::Further {
            0.55
        } else {
            0.95
        };
        let color = link.flow.color().with_alpha(strength);
        gizmos.linestrip(route[..route.len() - 1].iter().copied(), color);
        gizmos
            .arrow(*last_but_one, short(*last_but_one, *last, link.to), color)
            .with_tip_length(0.8 * zoom);
        if link.flow != Flow::Further {
            for pair in route.windows(2) {
                flow(&mut gizmos, pair[0], pair[1], color, time, index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use codeviz_core::System;
    use codeviz_ir::{Document, UnitId};
    use codeviz_layout::{CodebaseMap, ForceConfig, MapConfig, map};

    use super::*;

    /// Seven modules: `web::route` calls `core::run`, which calls `db::put`,
    /// `log::say` and `core::help`; `core::help` calls `core::idle` and `ext::lib`;
    /// `db::put` calls `disk::write`; `far` calls nothing.
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
        let map = map(&system, &MapConfig::default());
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

    #[test]
    fn the_focus_module_sits_in_the_middle_holding_all_its_functions() {
        let (graph, map) = codebase();
        let nucleus = Nucleus::new(&graph, Some(tile(&map, "core::run")));
        let core = module(&graph, &map, "core::run");
        let ball = nucleus.ball(core).expect("a sphere");
        assert_eq!(ball.body, Body::Nucleus);
        assert!(ball.center.length() < 1e-3, "the nucleus is the middle");
        for id in ["core::run", "core::help", "core::idle"] {
            assert!(nucleus.shows(tile(&map, id)), "every function inside shows");
            let at = nucleus.point(tile(&map, id)).expect("a point");
            assert!(at.length() <= ball.radius, "inside its sphere");
        }
    }

    #[test]
    fn modules_near_the_focus_open_and_the_rest_stay_closed_further_out() {
        let (graph, map) = codebase();
        let nucleus = Nucleus::new(&graph, Some(tile(&map, "core::run")));
        for id in ["web::route", "db::put", "log::say", "disk::write"] {
            assert!(nucleus.names(tile(&map, id)), "{id} is named");
            let ball = nucleus.ball(module(&graph, &map, id));
            assert_eq!(
                ball.map(|b| b.body),
                Some(Body::Open),
                "{id}'s module opens"
            );
        }
        assert!(
            !nucleus.shows(tile(&map, "db::other")),
            "what an open module does not name stays folded"
        );
        let near = nucleus
            .ball(module(&graph, &map, "db::put"))
            .expect("a sphere");
        let beyond = nucleus
            .ball(module(&graph, &map, "disk::write"))
            .expect("a sphere");
        assert!(
            beyond.center.length() > near.center.length(),
            "what a callee calls sits beyond the callee's module"
        );
        let far = nucleus
            .ball(module(&graph, &map, "far::alone"))
            .expect("a sphere");
        assert_eq!(far.body, Body::Closed);
        assert!(far.center.length() > beyond.center.length());
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
        let nucleus = Nucleus::new(&graph, Some(tile(&map, "core::run")));
        let [web, core, db, disk] = ["web::route", "core::run", "db::put", "disk::write"]
            .map(|id| module(&graph, &map, id));
        let has = |from, to, flow| {
            nucleus
                .tubes()
                .iter()
                .any(|t| t.from == from && t.to == to && t.flow == flow)
        };
        assert!(has(web, core, Flow::Into));
        assert!(has(core, db, Flow::OutOf));
        assert!(has(db, disk, Flow::Further));
        let into = nucleus
            .links()
            .iter()
            .find(|l| l.flow == Flow::Into)
            .expect("a call into the focus");
        assert_eq!(
            nucleus.route(into).len(),
            4,
            "a call between modules runs along their arrow"
        );
    }

    #[test]
    fn a_new_focus_recenters_easing_there() {
        let (graph, map) = codebase();
        let mut nucleus = Nucleus::new(&graph, Some(tile(&map, "core::run")));
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
        let mut nucleus = Nucleus::new(&graph, Some(tile(&map, "core::run")));
        let core = module(&graph, &map, "core::run");
        let small = nucleus.ball(core).expect("a sphere").radius;
        let web = module(&graph, &map, "web::route");
        let bearing = nucleus.ball(web).expect("a sphere").center.normalize();
        nucleus.expand(&graph, true);
        while nucleus.advance(0.1) {}
        let big = nucleus.ball(core).expect("a sphere");
        assert!(big.radius >= small * 3.0, "grows several times over");
        for id in ["web::route", "db::put", "log::say"] {
            let ball = nucleus.ball(module(&graph, &map, id)).expect("a sphere");
            assert_eq!(ball.body, Body::Open, "{id}'s module is on the ring");
            assert!(
                ball.center.length() > big.radius + ball.radius,
                "outside the wall"
            );
            assert!(ball.radius < big.radius / 4.0, "shrunk");
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
            assert!(nucleus.shows(tile(&map, id)));
            assert!(at.length() < big.radius, "{id} inside");
        }
        assert!(
            nucleus.shows(tile(&map, "db::put")),
            "what the focus calls outside stays named in its module"
        );
        assert!(
            !nucleus.shows(tile(&map, "db::other")),
            "the rest folds away"
        );
        nucleus.expand(&graph, false);
        while nucleus.advance(0.1) {}
        assert!((nucleus.ball(core).expect("a sphere").radius - small).abs() < 1e-3);
    }

    #[test]
    fn expanded_calls_leave_through_the_wall_towards_their_module() {
        let (graph, map) = codebase();
        let mut nucleus = Nucleus::new(&graph, Some(tile(&map, "core::run")));
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
        assert_eq!(inside.flow, Flow::OutOf);
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
        assert!(nucleus.names(help), "called by the focus, named");
        assert!(
            !nucleus.names(tile(&map, "core::idle")),
            "two calls along, not named"
        );
    }

    #[test]
    fn expanded_draws_only_the_focus_calls_until_the_pointer_asks() {
        let (graph, map) = codebase();
        let mut nucleus = Nucleus::new(&graph, Some(tile(&map, "core::run")));
        nucleus.expand(&graph, true);
        while nucleus.advance(0.1) {}
        assert!(
            nucleus.links().iter().all(|l| l.flow != Flow::Further),
            "nothing further along"
        );
        let (help, idle) = (tile(&map, "core::help"), tile(&map, "core::idle"));
        let ext = module(&graph, &map, "ext::lib");
        assert_eq!(
            nucleus.ball(ext).map(|b| b.body),
            Some(Body::Closed),
            "only the focus's own calls put a module on the ring"
        );
        assert_eq!(nucleus.touches(), &[]);
        assert!(!nucleus.ends(idle));

        assert!(nucleus.hover(&graph, Some(help)));
        let touches = nucleus.touches();
        assert!(touches.contains(&Touch {
            from: End::Function(help),
            to: End::Function(idle),
        }));
        assert!(
            touches.contains(&Touch {
                from: End::Function(help),
                to: End::Module(ext),
            }),
            "a call to a function folded away runs to its module"
        );
        assert_eq!(touches.len(), 2, "the focus's call to it is drawn already");
        assert!(nucleus.ends(idle), "the end of a call drawn is named");
        let core = nucleus
            .ball(module(&graph, &map, "core::run"))
            .expect("a sphere");
        let wall = nucleus.wall(ext).expect("a wall");
        assert!((wall.distance(core.center) - core.radius).abs() < 1e-3);

        assert!(nucleus.hover(&graph, None));
        assert!(nucleus.touches().is_empty() && !nucleus.ends(idle));
    }

    #[test]
    fn within_reaches_two_calls_either_way_upstream_first() {
        // 0 -> 1 -> 2 -> 3 -> 4, and 2 -> 1.
        let calls = [(0, 1), (1, 2), (2, 3), (3, 4), (2, 1)];
        let reach = within(&calls, 2);
        assert_eq!(reach.get(&2), Some(&0));
        assert_eq!(reach.get(&1), Some(&-1), "upstream wins a tie");
        assert_eq!(reach.get(&0), Some(&-2));
        assert_eq!(reach.get(&3), Some(&1));
        assert_eq!(reach.get(&4), Some(&2));
        assert_eq!(within(&calls, 0).get(&2), Some(&2));
        assert_eq!(within(&calls, 0).get(&3), None, "three calls away");
    }

    #[test]
    fn inside_callers_lean_one_way_and_callees_the_other() {
        // Focus 0; 1 and 2 call it; it calls 3 and 4; 5 relates to nothing.
        let start: Vec<Vec3> = (0..6).map(|i| fibonacci(i, 6) * 0.3).collect();
        let calls = [(1, 0), (2, 0), (0, 3), (0, 4)];
        let hops = [Some(0), Some(-1), Some(-1), Some(1), Some(1), None];
        let at = lay_inside(&start, &calls, &hops, Vec3::X, Vec3::Z);
        assert_eq!(at[0], Vec3::ZERO, "the focus in the middle");
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
            Nucleus::new(&graph, None).homes,
            Nucleus::new(&graph, None).homes
        );
    }

    #[test]
    fn hiding_callers_leaves_out_their_calls_and_modules() {
        let (graph, map) = codebase();
        let run = tile(&map, "core::run");
        let web = module(&graph, &map, "web::route");
        let hidden = Hidden {
            callers: true,
            callees: false,
        };
        let mut nucleus = Nucleus::with_hidden(&graph, Some(run), hidden);
        assert!(nucleus.links().iter().all(|l| l.flow != Flow::Into));
        assert!(!nucleus.names(tile(&map, "web::route")));
        assert_eq!(nucleus.ball(web).map(|b| b.body), Some(Body::Closed));
        assert!(nucleus.names(tile(&map, "db::put")), "callees stay");

        nucleus.expand(&graph, true);
        assert!(nucleus.links().iter().all(|l| l.flow == Flow::OutOf));
        nucleus.set_hidden(
            &graph,
            Hidden {
                callers: false,
                callees: true,
            },
        );
        assert!(nucleus.links().iter().all(|l| l.flow == Flow::Into));
        assert!(nucleus.names(tile(&map, "web::route")));
    }

    #[test]
    fn the_cursor_shows_where_it_sits_without_moving_anything() {
        let (graph, map) = codebase();
        let mut nucleus = Nucleus::new(&graph, Some(tile(&map, "core::run")));
        let other = tile(&map, "db::other");
        let db = module(&graph, &map, "db::put");
        assert!(!nucleus.shows(other));
        let before: Vec<Ball> = nucleus.balls().map(|(_, b)| b).collect();
        assert!(nucleus.mark(&graph, Some(other)));
        assert!(nucleus.shows(other) && nucleus.lit(other));
        let after: Vec<Ball> = nucleus.balls().map(|(_, b)| b).collect();
        assert_eq!(before, after, "nothing moves");
        let ball = nucleus.ball(db).expect("a sphere");
        let at = nucleus.point(other).expect("a point");
        assert!(
            at.distance(ball.center) > ball.radius,
            "just outside its module"
        );
        assert!(at.distance(ball.center) < ball.radius + 2.0);
        assert!(!nucleus.mark(&graph, Some(other)), "no change");
        assert!(nucleus.mark(&graph, None));
        assert!(!nucleus.shows(other));
    }

    #[test]
    fn specks_hide_by_default_only_when_expanded() {
        let (graph, map) = codebase();
        let mut nucleus = Nucleus::new(&graph, Some(tile(&map, "core::run")));
        assert!(!nucleus.specks_hidden());
        nucleus.expand(&graph, true);
        assert!(nucleus.specks_hidden());
        nucleus.toggle_specks();
        assert!(!nucleus.specks_hidden());
        nucleus.expand(&graph, false);
        assert!(!nucleus.specks_hidden(), "each view keeps its own");
        nucleus.toggle_specks();
        assert!(nucleus.specks_hidden());
    }

    #[test]
    fn the_pointer_counts_once_it_rests() {
        let mut held = Settle::default();
        assert!(!held.settle(Some(3), None, 0.0), "not at once");
        assert!(held.waiting());
        assert!(!held.settle(Some(3), None, 0.1));
        assert!(held.settle(Some(3), None, 0.16), "after a rest");
        assert!(!held.waiting());
        // Passing over one function on the way to another takes neither.
        assert!(!held.settle(Some(4), Some(3), 1.0));
        assert!(!held.settle(Some(5), Some(3), 1.1));
        assert!(!held.settle(Some(5), Some(3), 1.2));
        assert!(held.settle(Some(5), Some(3), 1.26));
        // Off and back on before the delay keeps it.
        assert!(!held.settle(None, Some(5), 2.0));
        assert!(!held.settle(Some(5), Some(5), 2.05));
        assert!(!held.waiting());
    }
}
