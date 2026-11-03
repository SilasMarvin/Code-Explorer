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
//! Expanded (`X`, a double click on a module, or `--expand`), a module opens where it
//! stands, browsing or in a journey alike: every one of its functions drawn, spread in
//! rows inside its sphere, with the calls among them. Nothing else moves; the camera
//! comes in until it fills the view.
//!
//! This file holds what the nucleus is and answers; [`browse`], [`journey`] and
//! [`inside`] arrange it for each mode, and [`draw`] puts it in the scene. The browsing map itself is [`codeviz_layout::atlas`]'s.

mod browse;
mod draw;
mod inside;
mod journey;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use codeviz_layout::{
    ForceGraph,
    atlas::{self, module_homes, pack, turned},
};

pub(crate) use draw::{create_looks, draw, follow, place_bodies};
pub(crate) use journey::rank_callers;

use crate::{
    camera::{ease, transition_step},
    colors::{CALLEE, CALLER, FURTHER, deeper},
    disclosure::number_order,
    sizes::module_radius,
};

/// How a module shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Body {
    /// The start's module, in the middle, with every function inside it.
    Nucleus,
    /// Holds functions near the start: opened to show those alone, by name.
    Open,
    /// In a journey, a module it does not run through: left out, where the map has
    /// it.
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
    /// A call between two functions of the module expanded that no other flow names.
    Inner,
}

/// Levels of the journey drawn in a shade of their own: the start's calls blue, then
/// amber, darker a level at a time; deeper than this, as dark as the last.
pub(crate) const SHADES: u32 = 4;

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
            Self::Journey(level) => deeper(level, SHADES),
            Self::Outside | Self::Inner => FURTHER,
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

/// The open journey from its start down to the disclosed depth, each call once, in
/// the journey's order.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Trail {
    pub(crate) calls: Vec<Call>,
    /// Every function below the start at any depth, with the fewest calls it is
    /// from it: what the modules round the start are laid out for, so a new depth
    /// moves nothing.
    pub(crate) whole: HashMap<usize, u32>,
    /// Every function below the start at any depth, in the order the journey first
    /// runs it, which orders the modules round the nucleus.
    pub(crate) order: Vec<usize>,
}

impl Trail {
    /// Adds a call the journey makes; one already there keeps its shallowest level.
    pub(crate) fn add(&mut self, from: usize, to: usize, level: u32) {
        match self.calls.iter_mut().find(|c| c.from == from && c.to == to) {
            Some(call) => call.level = call.level.min(level),
            None => self.calls.push(Call { from, to, level }),
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

/// Browsing with nothing selected, every call from one module to another, drawn as
/// one faint line between their spheres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tube {
    pub(crate) from: usize,
    pub(crate) to: usize,
    /// Every call from one module to the other.
    pub(crate) calls: usize,
}

/// A call this view names, from function to function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Link {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) flow: Flow,
}

/// The angle the camera turns to when the nucleus opens.
pub(crate) const VIEW: atlas::View = (-0.45, -0.38);
/// Browsing, how far the camera may turn off [`VIEW`], in yaw and in pitch: the map
/// is kept apart for views up to an eighth of a turn either side, and is flattened
/// along the view, so further round it would turn edge-on; turned both ways at once,
/// names crowd well before that.
pub(crate) const BROWSE_REACH: (f32, f32) = (0.6, 0.3);

/// Where everything is, for one start.
#[derive(Debug, Clone, Default)]
struct Arrangement {
    /// The function gathered round: the journey's start, or browsing, the selection.
    start: Option<usize>,
    /// The module in the middle: the start's.
    middle: usize,
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
    /// The modules round the nucleus, clockwise from the top.
    ring: Vec<usize>,
    /// In a journey with I on, every function off it that calls one on it.
    outside: HashSet<usize>,
    /// In a journey, the function set aside, when it is drawn only for that: picked
    /// by a search where the journey draws nothing of it.
    aside: Option<usize>,
    /// In a journey, the start's callers beyond the few nearest it: drawn quietly and
    /// named only under the pointer, counted on their module's name instead.
    folded: HashSet<usize>,
    tubes: Vec<Tube>,
    links: Vec<Link>,
    /// By cluster, how big its dots are drawn against their role's size, where its
    /// functions sit closer than the biggest two need: 1 where it is not listed.
    dots: HashMap<usize, f32>,
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
    /// The module expanded, if any: it opens where it is, and only the camera moves.
    expanded: Option<usize>,
    now: Arrangement,
    /// The function the pointer has rested on.
    hovered: Option<usize>,
    /// The open journey from the start.
    trail: Trail,
    /// The journey's cursor: marked where it sits.
    cursor: Option<usize>,
    /// In a journey, the function set aside: drawn, wherever it is.
    aside: Option<usize>,
    /// Whether the graph is browsed, laid out as the atlas has it, rather than
    /// gathered round a journey.
    browse: bool,
    /// Where everything is browsing: one layout of the whole codebase.
    atlas: Atlas,
    /// In a journey, whether every outside caller into it shows (I).
    outside: bool,
    /// Where the spheres and functions were when the last re-centering began.
    from: Option<(Vec<Ball>, Vec<Vec3>)>,
    /// How far along that re-centering is, 0 to 1.
    progress: f32,
}

impl Nucleus {
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
        let homes = module_homes(clusters, flows.iter().map(|(&pair, &calls)| (pair, calls)));
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); clusters];
        for tile in 0..tiles {
            if let Some(cluster) = graph.cluster(tile) {
                members[cluster].push(tile);
            }
        }
        let mut offsets = vec![Vec3::ZERO; tiles];
        for (cluster, members) in members.iter().enumerate() {
            for (tile, at) in pack(graph, members, module_radius(counts[cluster]), VIEW) {
                offsets[tile] = at;
            }
        }
        let module_of: Vec<Option<usize>> = (0..tiles).map(|t| graph.cluster(t)).collect();
        let atlas = browse::atlas(&homes, &counts, &offsets, &module_of);
        let mut nucleus = Self {
            homes,
            counts,
            module_of,
            offsets,
            flows,
            expanded: None,
            now: Arrangement::default(),
            hovered: None,
            trail,
            cursor: None,
            aside: None,
            browse,
            atlas,
            outside: false,
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
    #[cfg(test)]
    pub(crate) fn middle(&self) -> usize {
        self.now.middle
    }

    /// Whether a module is expanded.
    pub(crate) fn expanded(&self) -> bool {
        self.expanded.is_some()
    }

    /// The module expanded, if any.
    pub(crate) fn expanded_module(&self) -> Option<usize> {
        self.expanded
    }

    /// Expands `cluster`, or with none shrinks back what is expanded, easing there.
    pub(crate) fn expand(&mut self, graph: &ForceGraph, cluster: Option<usize>) {
        if cluster == self.expanded {
            return;
        }
        self.expanded = cluster;
        self.recenter(graph, self.now.start);
    }

    /// How big a function's dot is drawn against its role's size: smaller in a
    /// module whose functions sit too close for their dots at full size.
    pub(crate) fn dot_scale(&self, tile: usize) -> f32 {
        self.module_of
            .get(tile)
            .copied()
            .flatten()
            .and_then(|cluster| self.now.dots.get(&cluster))
            .copied()
            .unwrap_or(1.0)
    }

    /// Whether a function is one of the module expanded's: every one of them is
    /// drawn, those nothing lights plainly, as the module opens.
    pub(crate) fn opened_up(&self, tile: usize) -> bool {
        self.expanded.is_some() && self.module_of.get(tile).copied().flatten() == self.expanded
    }

    /// Arranges everything around `start` instead, easing there from wherever things
    /// are now.
    pub(crate) fn recenter(&mut self, graph: &ForceGraph, start: Option<usize>) {
        let mut balls: Vec<Ball> = (0..self.now.balls.len())
            .filter_map(|c| self.ball(c))
            .collect();
        let mut points: Vec<Vec3> = (0..self.now.points.len())
            .filter_map(|t| self.point(t))
            .collect();
        let placed: Vec<bool> = (0..points.len()).map(|t| self.placed(t)).collect();
        let shown: Vec<bool> = balls.iter().map(|b| self.shows_body(b.body)).collect();
        self.progress = 0.0;
        self.now = self.arrange(graph, start);
        // What was not drawn before shows where it goes, rather than flying in.
        for (tile, point) in points.iter_mut().enumerate() {
            if !placed[tile] {
                *point = self.now.points.get(tile).copied().unwrap_or(*point);
            }
        }
        for (cluster, ball) in balls.iter_mut().enumerate() {
            if !shown[cluster] {
                *ball = self.now.balls.get(cluster).copied().unwrap_or(*ball);
            }
        }
        self.from = Some((balls, points));
    }

    /// Names the open journey from the start instead, easing there. Returns whether
    /// that changed anything.
    pub(crate) fn retrace(&mut self, graph: &ForceGraph, trail: &Trail) -> bool {
        if *trail == self.trail {
            return false;
        }
        self.trail = trail.clone();
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
        self.expanded = None;
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

    /// Whether a function is an outside caller into the journey, shown.
    pub(crate) fn outside(&self, tile: usize) -> bool {
        self.now.outside.contains(&tile)
    }

    /// The outside callers into the journey shown (I).
    pub(crate) fn outside_callers(&self) -> impl Iterator<Item = usize> + '_ {
        self.now.outside.iter().copied()
    }

    /// Whether a function is one of the start's callers folded into its module's
    /// count: drawn quietly, named only under the pointer.
    pub(crate) fn folded(&self, tile: usize) -> bool {
        self.now.folded.contains(&tile)
    }

    /// Marks the journey's cursor where it sits, without moving anything else; with
    /// I on, brings in the callers from off the journey of the function it is on
    /// instead. Returns whether that changed.
    pub(crate) fn set_cursor(&mut self, graph: &ForceGraph, tile: Option<usize>) -> bool {
        if tile == self.cursor {
            return false;
        }
        self.cursor = tile;
        if self.outside && !self.browse {
            self.recenter(graph, self.now.start);
        }
        true
    }

    /// Sets a function aside in a journey: drawn where it is, or where the journey
    /// draws nothing of it, put where it shows moving nothing else. Returns whether
    /// that changed.
    pub(crate) fn set_aside(&mut self, graph: &ForceGraph, tile: Option<usize>) -> bool {
        if tile == self.aside {
            return false;
        }
        self.aside = tile;
        let placing = tile.is_some_and(|t| !self.placed(t)) || self.now.aside.is_some();
        if placing && !self.browse {
            self.recenter(graph, self.now.start);
        }
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
    /// (it has a [`Self::level`]), the journey's cursor, the function set aside, or
    /// an outside caller into the journey.
    pub(crate) fn lit(&self, tile: usize) -> bool {
        self.level(tile).is_some()
            || self.is_cursor(tile)
            || (!self.browse && self.aside == Some(tile))
            || self.outside(tile)
            || self.opened_up(tile)
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

    /// Moves a re-centering along by `dt` seconds. Returns whether anything moved.
    pub(crate) fn advance(&mut self, dt: f32) -> bool {
        if self.progress >= 1.0 {
            return false;
        }
        self.progress = (self.progress + transition_step(dt)).min(1.0);
        if self.progress >= 1.0 {
            self.from = None;
        }
        true
    }

    /// The share of the way to where things are going, eased in and out.
    fn eased(&self) -> f32 {
        ease(self.progress)
    }

    /// Where a function is drawn: inside its module's sphere, or at its middle when
    /// it is folded away. The cursor only ever rests on a call drawn.
    pub(crate) fn point(&self, tile: usize) -> Option<Vec3> {
        let to = *self.now.points.get(tile)?;
        Some(match &self.from {
            Some((_, points)) => points.get(tile).map_or(to, |&p| p.lerp(to, self.eased())),
            None => to,
        })
    }

    /// In a journey, where its start, its cursor and every function it lights but
    /// the callers folded into their module's count are going to rest: what framing
    /// it takes in.
    pub(crate) fn journey_points(&self) -> Vec<Vec3> {
        let lit = self.now.named.keys().copied().filter(|&t| !self.folded(t));
        self.now
            .start
            .into_iter()
            .chain(self.cursor)
            .chain(lit)
            .filter_map(|tile| self.resting_point(tile))
            .collect()
    }

    /// Where a function is going to be drawn once a re-centering ends: what the
    /// camera frames, rather than where it passes on the way.
    pub(crate) fn resting_point(&self, tile: usize) -> Option<Vec3> {
        self.now.points.get(tile).copied()
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
    /// its module.
    pub(crate) fn placed(&self, tile: usize) -> bool {
        self.now.shown.get(tile).copied().unwrap_or(false)
    }

    /// Every function the open journey could light at some depth: what it runs below
    /// the start, wherever it shows.
    pub(crate) fn could_light(&self) -> impl Iterator<Item = usize> + '_ {
        self.trail.whole.keys().copied().filter(|_| !self.browse)
    }

    /// How many calls below the start the open journey first runs a function, at
    /// any depth.
    pub(crate) fn journey_level(&self, tile: usize) -> Option<u32> {
        self.trail
            .whole
            .get(&tile)
            .copied()
            .filter(|_| !self.browse)
    }

    /// How many calls a lit function is from the start: -1 for a caller, else its
    /// shallowest level in the journey.
    pub(crate) fn level(&self, tile: usize) -> Option<i32> {
        self.now.named.get(&tile).copied()
    }

    /// Whether a function both calls the start (or the selection) and is called on
    /// its journey: colored as the call, and ringed in the caller's violet besides.
    pub(crate) fn calls_back(&self, tile: usize) -> bool {
        self.level(tile).is_some_and(|level| level > 0)
            && self
                .now
                .links
                .iter()
                .any(|l| l.flow == Flow::Into && l.from == tile)
    }

    /// How many functions a module holds.
    pub(crate) fn count(&self, cluster: usize) -> usize {
        self.counts.get(cluster).copied().unwrap_or(0)
    }

    /// How many functions an open module names.
    pub(crate) fn opened(&self, cluster: usize) -> usize {
        self.now.columns.get(cluster).map_or(0, Vec::len)
    }

    /// In a journey, the modules round the nucleus at any depth, clockwise from the
    /// top.
    pub(crate) fn ring(&self) -> &[usize] {
        &self.now.ring
    }

    pub(crate) fn tubes(&self) -> &[Tube] {
        &self.now.tubes
    }

    pub(crate) fn links(&self) -> &[Link] {
        &self.now.links
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
                [
                    b.center + right * b.radius,
                    b.center - right * b.radius,
                    b.center + up * b.radius,
                    b.center - up * b.radius,
                ]
            })
            .collect()
    }

    /// The modules a journey runs through, by cluster, once a re-centering ends, and
    /// the one holding the function set aside, wherever it is drawn: what framing it
    /// shows whole.
    pub(crate) fn open_modules(&self) -> Vec<usize> {
        let aside = self
            .aside
            .filter(|_| !self.browse)
            .and_then(|tile| self.module_of(tile));
        (0..self.now.balls.len())
            .filter(|&c| {
                matches!(self.now.balls[c].body, Body::Nucleus | Body::Open) || Some(c) == aside
            })
            .collect()
    }

    /// The top of each of `modules`' spheres, as it will be once a re-centering
    /// ends: where the module's name sits above it.
    pub(crate) fn tops(&self, rotation: Quat, modules: &[usize]) -> Vec<Vec3> {
        let up = rotation * Vec3::Y;
        self.balls_of(modules)
            .map(|b| b.center + up * b.radius)
            .collect()
    }

    /// The sides and foot of each of `modules`' spheres, as it will be once a
    /// re-centering ends: with [`Self::tops`], what shows it whole.
    pub(crate) fn rims(&self, rotation: Quat, modules: &[usize]) -> Vec<Vec3> {
        let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
        self.balls_of(modules)
            .flat_map(|b| {
                [
                    b.center + right * b.radius,
                    b.center - right * b.radius,
                    b.center - up * b.radius,
                ]
            })
            .collect()
    }

    fn balls_of<'a>(&'a self, modules: &'a [usize]) -> impl Iterator<Item = &'a Ball> {
        modules.iter().filter_map(|&c| self.now.balls.get(c))
    }

    /// Points round the expanded module alone, as it will be once a re-centering
    /// ends: what the view takes in to fill itself with it.
    pub(crate) fn nucleus_points(&self, rotation: Quat) -> Vec<Vec3> {
        let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
        self.now
            .balls
            .get(self.expanded.unwrap_or(self.now.middle))
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

    /// Points round every sphere shown, as it will be once a re-centering ends:
    /// browsing, the whole codebase.
    pub(crate) fn atlas_points(&self, rotation: Quat) -> Vec<Vec3> {
        let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
        self.now
            .balls
            .iter()
            .filter(|b| self.shows_body(b.body))
            .flat_map(|b| {
                [
                    b.center + (right + up) * b.radius,
                    b.center - (right + up) * b.radius,
                ]
            })
            .collect()
    }

    /// Everything placed around `start`, browsing or in a journey, and the module
    /// expanded opened out where it is.
    fn arrange(&mut self, graph: &ForceGraph, start: Option<usize>) -> Arrangement {
        let mut now = if self.browse {
            self.arrange_browse(graph, start)
        } else {
            self.arrange_modules(graph, start)
        };
        if let Some(cluster) = self.expanded {
            self.open_out(graph, cluster, &mut now);
        }
        now
    }
}

/// The usual view's rotation.
pub(crate) fn view_rotation() -> Quat {
    turned(VIEW, 0.0)
}
