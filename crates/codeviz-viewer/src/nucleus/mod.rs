//! The graph as module spheres, every function inside its module's, all where
//! [`codeviz_layout::place`] puts them once per document: browsing, in a journey and
//! with a module expanded alike, nothing is laid out again. Browsing, with no journey
//! open, a click lights the selection, white, its callers, violet, and its calls,
//! blue, where they sit, and moves nothing. In a journey, the start, what calls it
//! and the journey's calls down to the disclosed depth, the same depth the journey
//! panel shows, light where they sit, and the rest dims. Calls into the start are
//! violet; its own calls out blue, and every journey call further down amber, darker
//! the deeper, function to function. Space says only where things are: what calls
//! what sits near it, and no axis stands for order, rank or distance in calls.
//!
//! Expanded (`X`, a double click on a module, or `--expand`), a module opens where it
//! stands, browsing or in a journey alike: every one of its functions drawn, with the
//! calls among them. Nothing moves; the camera comes in until it is as big as its
//! functions need to sit comfortably.
//!
//! This file holds what the nucleus is and answers; [`browse`], [`journey`] and
//! [`inside`] light it for each mode, and [`draw`] puts it in the scene.

mod browse;
mod draw;
mod inside;
mod journey;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use codeviz_layout::{
    CallGraph,
    place::{self, Layout},
};

pub(crate) use draw::{create_looks, draw, place_bodies};
pub(crate) use journey::rank_callers;

use crate::{
    colors::{CALLEE, CALLER, FURTHER, deeper},
    disclosure::number_order,
};

/// How a module shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Body {
    /// In a journey, the start's module.
    Nucleus,
    /// Holds functions the view lights, or the selection's.
    Open,
    /// A module nothing lit lives in: faint glass, its functions specks.
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
    /// Whether every run of it shown is only guessed, or handed off: drawn faint.
    pub(crate) guessed: bool,
}

/// The open journey from its start down to the disclosed depth, each call once, in
/// the journey's order.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Trail {
    pub(crate) calls: Vec<Call>,
    /// Every function below the start at any depth, with the fewest calls it is
    /// from it: what folds the start's callers, so a new depth folds none anew.
    pub(crate) whole: HashMap<usize, u32>,
    /// Every call the journey makes at any depth, caller to callee, once: what names
    /// keep off at every depth alike.
    pub(crate) reach: Vec<(usize, usize)>,
}

impl Trail {
    /// Adds a call the journey makes, `guessed` when only guessed or handed off; one
    /// already there keeps its shallowest level, and is guessed only if every run is.
    pub(crate) fn add(&mut self, from: usize, to: usize, level: u32, guessed: bool) {
        match self.calls.iter_mut().find(|c| c.from == from && c.to == to) {
            Some(call) => {
                call.level = call.level.min(level);
                call.guessed &= guessed;
            }
            None => self.calls.push(Call {
                from,
                to,
                level,
                guessed,
            }),
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

/// The angle the camera turns to when the view opens, and R.
pub(crate) const VIEW: (f32, f32) = (-0.45, -0.38);
/// Browsing, how far the camera may turn off [`VIEW`], in yaw and in pitch.
pub(crate) const BROWSE_REACH: (f32, f32) = (0.6, 0.3);

/// What the view lights, for one start.
#[derive(Debug, Clone, Default)]
struct Arrangement {
    /// The function lit round: the journey's start, or browsing, the selection.
    start: Option<usize>,
    /// The start's module.
    middle: usize,
    /// By cluster.
    balls: Vec<Ball>,
    /// The functions lit for what they are to the start: its callers and the
    /// journey's calls, each with how many calls it is from the start, -1 for a caller.
    named: HashMap<usize, i32>,
    /// By cluster: the functions it holds that the view names, in the order it
    /// reaches them.
    columns: Vec<Vec<usize>>,
    /// In a journey with I on, every function off it that calls the one the cursor
    /// is on.
    outside: HashSet<usize>,
    /// In a journey, the start's callers beyond the few nearest it: drawn quietly and
    /// named only under the pointer, counted on their module's name instead.
    folded: HashSet<usize>,
    tubes: Vec<Tube>,
    links: Vec<Link>,
}

/// The graph as module spheres where the layout has them, lighting what the view is
/// on: in a journey, its start and calls; browsing, what the selection touches.
/// Browsing has no journey, so the selection stands where a start would: what is
/// said of the start below says it of the selection too.
#[derive(Debug, Clone)]
pub(crate) struct Nucleus {
    /// Where every module and function sits: worked out once.
    layout: Layout,
    /// Functions in each module, by cluster.
    counts: Vec<usize>,
    /// Each function's module, by tile.
    module_of: Vec<Option<usize>>,
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
    /// In a journey, the function set aside: lit where it is.
    aside: Option<usize>,
    /// Whether the graph is browsed rather than lit round a journey.
    browse: bool,
    /// In a journey, whether every outside caller into it shows (I).
    outside: bool,
    /// The outside callers shown whose call into the journey has no line where the
    /// camera stands: drawn nowhere.
    lineless: HashSet<usize>,
}

impl Nucleus {
    /// Lays the graph out, once, and lights what `start` touches: browsing (`browse`)
    /// its callers and calls, else `trail`, the open journey from it.
    pub(crate) fn gather(
        graph: &CallGraph,
        start: Option<usize>,
        trail: Trail,
        browse: bool,
    ) -> Self {
        let clusters = graph.cluster_count();
        let mut counts = vec![0; clusters];
        for tile in 0..graph.len() {
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
        let mut nucleus = Self {
            layout: place::layout(graph),
            counts,
            module_of: (0..graph.len()).map(|t| graph.cluster(t)).collect(),
            flows,
            expanded: None,
            now: Arrangement::default(),
            hovered: None,
            trail,
            cursor: None,
            aside: None,
            browse,
            outside: false,
            lineless: HashSet::new(),
        };
        nucleus.now = nucleus.arrange(graph, start);
        nucleus
    }

    /// The function lit round, drawn white: the journey's start,
    /// or browsing, the selection.
    pub(crate) fn start(&self) -> Option<usize> {
        self.now.start
    }

    /// The start's module.
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

    /// Expands `cluster`, or with none shrinks back what is expanded.
    pub(crate) fn expand(&mut self, graph: &CallGraph, cluster: Option<usize>) {
        if cluster == self.expanded {
            return;
        }
        self.expanded = cluster;
        self.recenter(graph, self.now.start);
    }

    /// Whether a function is one of the module expanded's: every one of them is
    /// drawn, those nothing lights plainly, as the module opens.
    pub(crate) fn opened_up(&self, tile: usize) -> bool {
        self.expanded.is_some() && self.module_of.get(tile).copied().flatten() == self.expanded
    }

    /// Lights what `start` touches instead.
    pub(crate) fn recenter(&mut self, graph: &CallGraph, start: Option<usize>) {
        self.now = self.arrange(graph, start);
    }

    /// Lights the open journey from the start instead. Returns whether that changed
    /// anything.
    pub(crate) fn retrace(&mut self, graph: &CallGraph, trail: &Trail) -> bool {
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

    /// Browses the graph, or lights a journey in it. Returns whether that changed.
    pub(crate) fn set_browse(&mut self, graph: &CallGraph, browse: bool) -> bool {
        if browse == self.browse {
            return false;
        }
        self.browse = browse;
        self.expanded = None;
        self.recenter(graph, self.now.start);
        true
    }

    /// Lights every outside caller into the journey where it is, or no longer.
    pub(crate) fn set_outside(&mut self, graph: &CallGraph, on: bool) {
        if on != self.outside {
            self.outside = on;
            self.recenter(graph, self.now.start);
        }
    }

    /// Whether a function is an outside caller into the journey, shown.
    pub(crate) fn outside(&self, tile: usize) -> bool {
        self.now.outside.contains(&tile)
    }

    /// How many callers from off the journey of the function the cursor is on (I)
    /// wait, undrawn, for a camera that sees their call's line.
    pub(crate) fn outside_waiting(&self) -> usize {
        self.now.outside.intersection(&self.lineless).count()
    }

    /// Leaves out the outside callers `tiles` whose call has no line where the camera
    /// stands: a caller with no line reads as a function of its own. Returns whether
    /// that changed.
    pub(crate) fn set_lineless(&mut self, tiles: HashSet<usize>) -> bool {
        if tiles == self.lineless {
            return false;
        }
        self.lineless = tiles;
        true
    }

    fn lineless(&self, tile: usize) -> bool {
        self.lineless.contains(&tile) && self.outside(tile)
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

    /// Marks the journey's cursor where it sits; with I on, lights the callers from
    /// off the journey of the function it is on instead. Returns whether that
    /// changed.
    pub(crate) fn set_cursor(&mut self, graph: &CallGraph, tile: Option<usize>) -> bool {
        if tile == self.cursor {
            return false;
        }
        self.cursor = tile;
        if self.outside && !self.browse {
            self.recenter(graph, self.now.start);
        }
        true
    }

    /// Sets a function aside in a journey: lit where it is. Returns whether that
    /// changed.
    pub(crate) fn set_aside(&mut self, tile: Option<usize>) -> bool {
        if tile == self.aside {
            return false;
        }
        self.aside = tile;
        true
    }

    /// The journey's cursor, marked where it sits.
    pub(crate) fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    /// The function set aside in a journey.
    pub(crate) fn aside(&self) -> Option<usize> {
        self.aside
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

    /// Whether a function is drawn at all: it has a module, and is not an outside
    /// caller with no line. The one test of what is drawn: what is pointed at,
    /// clicked and named is drawn.
    pub(crate) fn drawn(&self, tile: usize) -> bool {
        self.placed(tile) && !self.lineless(tile)
    }

    /// Whether a function placed where it shows is only a small, faint speck: not
    /// the start, and lit by nothing.
    pub(crate) fn speck(&self, tile: usize) -> bool {
        self.placed(tile) && !self.lit(tile) && self.now.start != Some(tile)
    }

    /// Where a function sits: inside its module's sphere.
    pub(crate) fn point(&self, tile: usize) -> Option<Vec3> {
        self.placed(tile).then(|| self.layout.functions[tile])
    }

    /// In a journey, where its start, its cursor and every function it lights but
    /// the callers folded into their module's count sit: what framing takes in.
    pub(crate) fn journey_points(&self) -> Vec<Vec3> {
        let lit = self.now.named.keys().copied().filter(|&t| !self.folded(t));
        self.now
            .start
            .into_iter()
            .chain(self.cursor)
            .chain(lit)
            .filter_map(|tile| self.point(tile))
            .collect()
    }

    /// A module's sphere.
    pub(crate) fn ball(&self, cluster: usize) -> Option<Ball> {
        self.now.balls.get(cluster).copied()
    }

    /// The radius of the ball each of a module's functions keeps to itself: no two
    /// come nearer than twice it.
    pub(crate) fn room(&self, cluster: usize) -> Option<f32> {
        self.layout.rooms.get(cluster).copied()
    }

    /// Every module's sphere as drawn now, by cluster.
    pub(crate) fn balls(&self) -> impl Iterator<Item = (usize, Ball)> + '_ {
        (0..self.now.balls.len()).filter_map(|c| self.ball(c).map(|b| (c, b)))
    }

    /// Whether a function has a place: inside its module's sphere.
    pub(crate) fn placed(&self, tile: usize) -> bool {
        self.module_of(tile).is_some()
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

    /// Whether a call of the journey is only guessed, or handed off, every time it
    /// shows: drawn faint, as its bar is.
    pub(crate) fn guessed(&self, link: &Link) -> bool {
        matches!(link.flow, Flow::Journey(_))
            && self
                .trail
                .calls
                .iter()
                .any(|c| c.from == link.from && c.to == link.to && c.guessed)
    }

    /// Every call the view could draw at any depth, caller to callee: what names
    /// would rather not lie across, at every depth alike.
    pub(crate) fn every_call(&self) -> Vec<(usize, usize)> {
        let reach = self.trail.reach.iter().copied().filter(|_| !self.browse);
        self.now
            .links
            .iter()
            // Not what I brings in, nor the expanded module's own: those come and go.
            .filter(|l| matches!(l.flow, Flow::Into | Flow::Journey(_)))
            .map(|l| (l.from, l.to))
            .chain(reach)
            .collect()
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

    /// Points around every sphere that should be on screen, the nucleus and the open
    /// modules.
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

    /// The modules a journey runs through, by cluster, and the one holding the
    /// function set aside: what framing it shows whole.
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

    /// The top of each of `modules`' spheres: where the module's name sits above it.
    pub(crate) fn tops(&self, rotation: Quat, modules: &[usize]) -> Vec<Vec3> {
        let up = rotation * Vec3::Y;
        self.balls_of(modules)
            .map(|b| b.center + up * b.radius)
            .collect()
    }

    /// The sides and foot of each of `modules`' spheres: with [`Self::tops`], what
    /// shows it whole.
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

    /// `modules`' spheres.
    pub(crate) fn balls_of<'a>(&'a self, modules: &'a [usize]) -> impl Iterator<Item = &'a Ball> {
        modules.iter().filter_map(|&c| self.now.balls.get(c))
    }

    /// Points round the expanded module alone: what the view takes in to fill
    /// itself with it.
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

    /// Points round every sphere: the whole codebase.
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

    /// What the view lights round `start`, browsing or in a journey, and the module
    /// expanded opened out where it is.
    fn arrange(&self, graph: &CallGraph, start: Option<usize>) -> Arrangement {
        let mut now = if self.browse {
            self.arrange_browse(graph, start)
        } else {
            self.arrange_journey(graph, start)
        };
        if let Some(cluster) = self.expanded {
            Self::open_out(graph, cluster, &mut now);
        }
        now
    }

    /// Every module's sphere where the layout has it, faint.
    fn faint_balls(&self) -> Vec<Ball> {
        (self.layout.modules.iter())
            .map(|&(center, radius)| Ball {
                center,
                radius,
                body: Body::Faint,
            })
            .collect()
    }
}

/// The usual view's rotation.
pub(crate) fn view_rotation() -> Quat {
    Quat::from_euler(EulerRot::YXZ, VIEW.0, VIEW.1, 0.0)
}
