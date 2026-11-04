//! The calls on the graph, drawn flat over the scene at one size on screen however
//! near the view: each call a line from its caller's dot to just short of its
//! callee's, with one filled head there, straight where no dot is in its way and
//! bowed gently round one that is; browsing with nothing selected, a thin faint line
//! from each module to every module it calls, behind the spheres it passes. A call's
//! line runs faint where it crosses a sphere it neither leaves nor enters, breaks
//! round any dot (and its ring) no gentle bow clears, and bows a little off a call it
//! would otherwise run along, so it never reads as passing through a dot or as
//! another call. A lit call crossing the view with both its ends off it is faint.
//!
//! Where each line runs is worked out once for how things lie and where the camera
//! stands ([`route`]), against every dot the journey lays out at any depth, so a
//! level more or less, the callers from outside, a click or the pointer bend no line;
//! names are placed after, keeping off the lines, and off the last stretch of each
//! line into their own dot ([`arrival`]).

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bevy_egui::egui;

use crate::{
    colors::{
        ACROSS_STRENGTH, CALL_STRENGTH, CURSOR, FOLDED_STRENGTH, FURTHER, GUESSED_STRENGTH,
        INNER_STRENGTH, MAP_STRENGTH, OUTSIDE_STRENGTH, PICKED_STRENGTH, PILL, egui_color,
    },
    disclosure,
    drive::{ArrowShape, CallMark, ExitMark, Marks, TubeMark},
    explorer::Explorer,
    graph_view::{RING, drawn_radius, radius_as},
    key_help,
    labels::{CallLine, GraphNames, PILL_PAD, node_name},
    nucleus::{Flow, Link, Nucleus},
    sizes::Dot,
    text,
};

/// A call's line, in points; the one into the cursor or out of the function set
/// aside, a little wider; a call among the expanded module's functions, thinner.
pub(crate) const CALL_WIDTH: f32 = 1.5;
const BRIGHT_WIDTH: f32 = 2.0;
const INNER_WIDTH: f32 = 1.0;
/// A call's head: its length along the line and its width, in points; and the least
/// it is drawn, for a call between two dots too near for the whole head.
const HEAD: (f32, f32) = (8.0, 7.0);
const SHORT_HEAD: (f32, f32) = (5.0, 5.0);
/// A line between modules on the whole map, in points: one width, however many
/// calls it carries, and a head smaller than a call's.
const MAP_WIDTH: f32 = 1.0;
const MAP_HEAD: (f32, f32) = (6.0, 5.0);
/// Between a line's end and the dot or sphere it ends at, in points; and between two
/// dots too near for that.
const GAP: f32 = 2.5;
const SHORT_GAP: f32 = 1.0;
/// How much of its strength a line keeps where it crosses a sphere it does not touch.
const CROSSING: f32 = 0.3;
/// Kept clear round a dot a line passes without ending there, past its ring where it
/// has one, in points: bowed round, or else the line's gap there.
const DOT_CLEAR: f32 = 3.0;
/// What running along another call costs a line, and running over a lit dot,
/// against a plain dot's 1: a line keeps off the calls beside it before all, since
/// no gap would tell the two apart, then off every lit dot it can, and off as many
/// plain ones as it can; it breaks round those it still runs over.
const SHARED_COST: usize = 10_000;
const LIT_COST: usize = 100;
/// Two calls from or into one dot run along one line, reading as a chain through
/// the other's far dot, where one comes within [`SHARED_NEAR`] points of the other
/// for more than [`SHARED_RUN`] points within [`SHARED_FAR`] of that far dot, and
/// [`SHARED_ENDS`] from its own ends. Calls fanning out from one dot to dots near
/// each other run close by it, but apart where each ends.
pub(crate) const SHARED_NEAR: f32 = 3.0;
pub(crate) const SHARED_RUN: f32 = 16.0;
pub(crate) const SHARED_ENDS: f32 = 20.0;
pub(crate) const SHARED_FAR: f32 = 48.0;
/// How many ways clear of everything a line keeps, the gentlest first and bowed no
/// further than the first, for the one drawn to keep off its own ends' names.
const CLEAR_WAYS: usize = 4;
/// The last stretch of a line into its callee's dot, in points: no name of that dot
/// lies over it, so the head shows and the name does not read as where the call
/// comes from.
pub(crate) const ARRIVAL: f32 = 30.0;
/// How far each of the two lines between the same two ends moves to its right when
/// calls go both ways, in points: far enough that their heads, side by side, do not
/// touch, so neither hides the other.
const BOTH_WAYS: f32 = 4.0;
/// How far a line may bow out to clear a dot, in points, tried in turn from the
/// least; and against its length, at most.
const BENDS: [f32; 9] = [5.0, 9.0, 14.0, 22.0, 32.0, 46.0, 64.0, 90.0, 128.0];
const MOST_BEND: f32 = 0.6;
/// How much longer than straight a bowed line may run: past that it goes straight,
/// broken round the dot in its way.
pub(crate) const DETOUR: f32 = 1.3;
/// Where along a bent line it bows out most: evenly, nearer its start, nearer its
/// end, close by either, or one way then the other: where along it each of its two
/// pulls stands, as a share of its length, and the share of the bend each takes.
const BOWS: [Bow; 9] = [
    [(1.0 / 3.0, 1.0), (2.0 / 3.0, 1.0)],
    [(1.0 / 3.0, 1.6), (2.0 / 3.0, 0.2)],
    [(1.0 / 3.0, 0.2), (2.0 / 3.0, 1.6)],
    [(0.1, 1.4), (0.4, 0.6)],
    [(0.6, 0.6), (0.9, 1.4)],
    [(1.0 / 3.0, 1.2), (2.0 / 3.0, -1.2)],
    [(0.15, 1.2), (0.45, -0.6)],
    [(0.55, 0.6), (0.85, -1.2)],
    [(0.1, 0.8), (0.9, 0.8)],
];
/// Past this many far functions, the exit labels name the modules the calls leave
/// for, with the first few functions in each.
const EXITS_APART: usize = 6;
const EXIT_NAMED: usize = 2;
/// How far apart along the free area's edge an exit label tries its places, in
/// points; and how far inside the edge it sits.
const EXIT_STEP: f32 = 12.0;
const EXIT_IN: f32 = 3.0;
/// How many places near where its calls leave an exit label tries for one no line
/// crosses, before taking the nearest clear of names and dots.
const CLEAR_TRIES: usize = 60;
/// Points a bent line is drawn through.
const STEPS: usize = 24;

/// One end of a call's line: the middle of its dot (or sphere), and how far from it
/// the line stops.
type End = (egui::Pos2, f32);

/// A bent line's two pulls: each where along it, and how far out, as shares.
type Bow = [(f32, f32); 2];

/// A call routed before that shares a dot with the one routed now: its line, and
/// where it ends at its other dot.
type Beside<'a> = (&'a [egui::Pos2], egui::Pos2);

/// A call to route: its caller and callee, whether its line keeps off dots, whether
/// names keep off it, and its ends.
type Call = ((usize, usize), bool, bool, [End; 2]);

/// What a stretch of a line crosses: a sphere it is not part of, drawn faint; or a
/// dot it does not end at, left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Crossing {
    Sphere,
    Dot,
}

/// A dot as lines keep off it: its middle on screen, how far, and whether it is a
/// plain function's of the module expanded, which matters least.
type Obstacle = (egui::Pos2, f32, bool);

/// Where things show on screen this frame, as the calls are drawn among them.
struct Screen<'a> {
    explorer: &'a Explorer,
    camera: &'a Camera,
    view: &'a GlobalTransform,
    /// Every sphere shown, by cluster: its middle and radius on screen.
    spheres: Vec<(usize, egui::Pos2, f32)>,
    /// Every dot drawn, specks too, by tile: its middle and radius on screen, how far
    /// from its middle a line passing it leaves a gap, and whether it is a speck.
    dots: Vec<Passed>,
}

/// A dot drawn, as calls passing it see it.
#[derive(Debug, Clone, Copy)]
struct Passed {
    tile: usize,
    at: egui::Pos2,
    radius: f32,
    gap: f32,
    speck: bool,
}

impl<'a> Screen<'a> {
    fn new(explorer: &'a Explorer, camera: &'a Camera, view: &'a GlobalTransform) -> Self {
        let mut screen = Self {
            explorer,
            camera,
            view,
            spheres: Vec::new(),
            dots: Vec::new(),
        };
        let nucleus = explorer.nucleus();
        screen.spheres = nucleus
            .balls()
            .filter(|(_, ball)| nucleus.shows_body(ball.body))
            .filter_map(|(cluster, ball)| {
                let (at, radius) = screen.disc(ball.center, ball.radius)?;
                Some((cluster, at, radius))
            })
            .collect();
        // A ring round a dot is part of it: the start's, the cursor's, the one set
        // aside's, a call's that calls back. Not the pointer's, which comes and goes.
        let ringed = |tile: usize| {
            [nucleus.start(), explorer.cursor(), explorer.aside()].contains(&Some(tile))
                || nucleus.calls_back(tile)
        };
        screen.dots = (0..explorer.map().tiles.len())
            .filter(|&tile| nucleus.drawn(tile))
            .filter_map(|tile| {
                let world = explorer.tile_point(tile)?;
                let (at, radius) =
                    screen.disc(world, drawn_radius(explorer, tile, camera, view))?;
                let reach = if ringed(tile) { radius * RING } else { radius };
                Some(Passed {
                    tile,
                    at,
                    radius,
                    gap: reach + DOT_CLEAR,
                    speck: nucleus.speck(tile),
                })
            })
            .collect();
        screen
    }

    fn project(&self, world: Vec3) -> Option<egui::Pos2> {
        self.camera
            .world_to_viewport(self.view, world)
            .ok()
            .map(|p| egui::pos2(p.x, p.y))
    }

    /// A disc facing the camera, `radius` round `world`, as it shows.
    fn disc(&self, world: Vec3, radius: f32) -> Option<(egui::Pos2, f32)> {
        let at = self.project(world)?;
        let edge = self.project(world + self.view.up() * radius)?;
        Some((at, at.distance(edge)))
    }

    /// A function's dot as big as it shows in `dot`'s role.
    fn dot_as(&self, tile: usize, dot: Dot) -> Option<(egui::Pos2, f32)> {
        let world = self.explorer.tile_point(tile)?;
        self.disc(
            world,
            radius_as(self.explorer, tile, dot, self.camera, self.view),
        )
    }

    fn sphere(&self, cluster: usize) -> Option<(egui::Pos2, f32)> {
        self.spheres
            .iter()
            .find(|(c, ..)| *c == cluster)
            .map(|&(_, at, radius)| (at, radius))
    }

    /// Whether a function is one the open journey lays out at some depth (its start,
    /// its callers, its calls at any depth): what its lines keep off, and end at, as
    /// big as the cursor's whatever it is now, so moving the cursor moves no line.
    fn laid_out(&self, tile: usize) -> bool {
        let nucleus = self.explorer.nucleus();
        if nucleus.browsing() {
            return nucleus.drawn(tile) && !nucleus.speck(tile);
        }
        nucleus.start() == Some(tile)
            || nucleus.journey_level(tile).is_some()
            || (nucleus.level(tile) == Some(-1) && !nucleus.folded(tile) && !nucleus.outside(tile))
    }

    /// Every dot a call's line keeps off: browsing, every one drawn, the specks
    /// plain; in a journey, every one it lays out at any depth, and the plain
    /// functions of the module expanded; never a caller from outside or the function
    /// set aside off the journey, which come and go. Each as big as the cursor's, the
    /// start and a call that calls back as far as the ring round it reaches; a plain
    /// one as it is drawn.
    fn obstacles(&self) -> Vec<(usize, Obstacle)> {
        let nucleus = self.explorer.nucleus();
        (0..self.explorer.map().tiles.len())
            .filter_map(|tile| {
                let member = self.laid_out(tile);
                let plain = !member
                    && if nucleus.browsing() {
                        nucleus.drawn(tile)
                    } else {
                        nucleus.opened_up(tile)
                    };
                if !member && !plain {
                    return None;
                }
                // A speck browsing; one of the module expanded's, drawn lit.
                let (at, reach) = if plain {
                    let dot = if nucleus.browsing() {
                        Dot::Speck
                    } else {
                        Dot::Lit
                    };
                    self.dot_as(tile, dot)?
                } else {
                    // Rings that come and go with the cursor or the function set aside
                    // are left to the gap the line breaks off with there.
                    let (at, radius) = self.dot_as(tile, Dot::Focus)?;
                    let ringed = nucleus.start() == Some(tile) || nucleus.calls_back(tile);
                    (at, if ringed { radius * RING } else { radius })
                };
                Some((tile, (at, reach + DOT_CLEAR, plain)))
            })
            .collect()
    }

    /// Where a call's line runs between, and how far short of each end it stops: its
    /// caller's dot and its callee's, a function the journey lays out as big as the
    /// cursor's, or its module's sphere where the function there is folded away.
    fn ends(&self, (from, to): (usize, usize)) -> Option<[End; 2]> {
        let nucleus = self.explorer.nucleus();
        let graph = self.explorer.graph();
        let end = |tile: usize| {
            let (at, radius) = if self.laid_out(tile) && !nucleus.browsing() {
                self.dot_as(tile, Dot::Focus)?
            } else if nucleus.placed(tile) {
                self.dot_as(tile, graph_dot(self.explorer, tile))?
            } else {
                self.sphere(graph.cluster(tile)?)?
            };
            Some((at, radius + GAP))
        };
        Some([end(from)?, end(to)?])
    }

    /// The stretches of `path`, as spans of the share of each of its pieces, that
    /// cross a sphere other than `own` or a dot (and its ring, and the gap round it)
    /// other than its ends'.
    fn crossings(
        &self,
        (from, to): (egui::Pos2, egui::Pos2),
        own: [Option<usize>; 2],
        ends: [Option<usize>; 2],
    ) -> Vec<(f32, f32, Crossing)> {
        let spheres = self
            .spheres
            .iter()
            .filter(|(c, ..)| !own.contains(&Some(*c)))
            .filter_map(|&(_, at, radius)| {
                let (t0, t1) = across(from, to, at, radius)?;
                Some((t0, t1, Crossing::Sphere))
            });
        let dots = self
            .dots
            .iter()
            .filter(|d| !ends.contains(&Some(d.tile)))
            .filter_map(|d| {
                let (t0, t1) = across(from, to, d.at, d.gap)?;
                Some((t0, t1, Crossing::Dot))
            });
        spheres.chain(dots).collect()
    }
}

/// How a function's dot is drawn now.
fn graph_dot(explorer: &Explorer, tile: usize) -> Dot {
    crate::graph_view::dot(explorer, tile)
}

/// A call's line as worked out: the way it runs, and the other ways, should the
/// first run through its own end's name: as clear of everything, gentlest first,
/// then straight, broken round the dots in its way, where it runs along no call.
#[derive(Debug, Clone)]
struct Route {
    path: Vec<egui::Pos2>,
    others: Vec<Vec<egui::Pos2>>,
    /// Whether names keep off it.
    kept: bool,
}

/// Where every call the view could draw runs, by caller and callee, for where the
/// camera stands and how things lie: the calls drawn, and in a journey every call it
/// makes at any depth, which names keep off.
#[derive(Debug, Clone, Default)]
pub(crate) struct Routes {
    lines: HashMap<(usize, usize), Route>,
}

impl Routes {
    /// The line of every call the journey makes at any depth, and the calls into its
    /// start (browsing, the selection's), each caller and callee: what names would
    /// rather not lie across, at every depth alike.
    pub(crate) fn lines(&self) -> Vec<CallLine> {
        let mut lines: Vec<_> = self
            .lines
            .iter()
            .filter(|(_, route)| route.kept)
            .map(|(&(from, to), route)| (from, to, route.path.clone()))
            .collect();
        lines.sort_unstable_by_key(|l| (l.0, l.1));
        lines
    }
}

/// The last [`ARRIVAL`] points of the line through `path`, into its callee's dot:
/// where that dot's name never goes.
pub(crate) fn arrival(path: &[egui::Pos2]) -> Vec<egui::Pos2> {
    let Some(&tip) = path.last() else {
        return Vec::new();
    };
    let mut stretch = vec![tip];
    let mut left = ARRIVAL;
    for pair in path.windows(2).rev() {
        let piece = pair[0].distance(pair[1]);
        if piece >= left {
            stretch.push(pair[1] + (pair[0] - pair[1]) * (left / piece.max(f32::EPSILON)));
            break;
        }
        stretch.push(pair[0]);
        left -= piece;
    }
    stretch.reverse();
    stretch
}

/// Works out where every call runs, as [`Routes`] says.
pub(crate) fn route(explorer: &Explorer, (camera, view): (&Camera, &GlobalTransform)) -> Routes {
    let screen = Screen::new(explorer, camera, view);
    let nucleus = explorer.nucleus();
    // Each call once: whether its line keeps off the dots in its way (not the faint
    // ones among an expanded module's functions), and whether names keep off it (not
    // the calls from outside or among the expanded module's own, which come and go).
    let mut calls: HashMap<(usize, usize), (bool, bool)> = HashMap::new();
    for call in nucleus.every_call() {
        calls.insert(call, (true, true));
    }
    for link in nucleus.links() {
        calls
            .entry((link.from, link.to))
            .or_insert((link.flow != Flow::Inner, false));
    }
    let pairs: HashSet<(usize, usize)> = calls.keys().copied().collect();
    let obstacles = screen.obstacles();
    // The calls names keep off first, the shortest first, each keeping off those
    // before it; the rest after, keeping off those, so what comes and goes bends
    // none of them.
    let mut todo: Vec<Call> = calls
        .into_iter()
        .filter_map(|(pair, (avoid, kept))| Some((pair, avoid, kept, screen.ends(pair)?)))
        .collect();
    let span = |ends: &[End; 2]| ends[0].0.distance(ends[1].0);
    todo.sort_by(|a, b| {
        (!a.2)
            .cmp(&!b.2)
            .then(span(&a.3).total_cmp(&span(&b.3)))
            .then(a.0.cmp(&b.0))
    });
    let mut lines: HashMap<(usize, usize), Route> = HashMap::new();
    let mut taken: Vec<((usize, usize), bool)> = Vec::new();
    for ((from, to), avoid, kept, ends) in todo {
        let side = if pairs.contains(&(to, from)) {
            BOTH_WAYS
        } else {
            0.0
        };
        let near = |tile: usize| tile != from && tile != to;
        // How far off its straight line a bow takes it, and what lies that near.
        let corridor = BENDS[BENDS.len() - 1].min(span(&ends) * MOST_BEND);
        let obstacles: Vec<Obstacle> = if avoid {
            obstacles
                .iter()
                .filter(|&&(tile, (at, reach, _))| {
                    near(tile) && off_line(ends[0].0, ends[1].0, at) < corridor + reach
                })
                .map(|&(_, o)| o)
                .collect()
        } else {
            Vec::new()
        };
        // The calls before it sharing one of its dots, and where each ends at its
        // other: for one names keep off, only those like it, so what comes and goes
        // bends none of them.
        let beside: Vec<Beside> = if avoid {
            taken
                .iter()
                .filter(|&&(pair, before)| (before || !kept) && share_a_dot((from, to), pair))
                .filter_map(|(pair, _)| {
                    let path = lines.get(pair)?.path.as_slice();
                    // Its caller shared, it ends at its callee; else at its caller.
                    let far = if [from, to].contains(&pair.0) {
                        *path.last()?
                    } else {
                        *path.first()?
                    };
                    Some((path, far))
                })
                .collect()
        } else {
            Vec::new()
        };
        let Some(mut route) = route_one(ends, side, &obstacles, &beside) else {
            continue;
        };
        route.kept = kept;
        if avoid {
            taken.push(((from, to), kept));
        }
        lines.insert((from, to), route);
    }
    Routes { lines }
}

/// Whether the calls `pair` and `other` share a dot, and so can run along each other
/// from it, or a bow bend one into the other's way: both ways between the same two
/// dots lie side by side apart already.
fn share_a_dot(pair: (usize, usize), other: (usize, usize)) -> bool {
    other != (pair.1, pair.0)
        && (pair.0 == other.0 || pair.0 == other.1 || pair.1 == other.0 || pair.1 == other.1)
}

/// The line of a call between `ends`, moved `side` points to its right: straight
/// where it passes no dot of `obstacles` and runs along none of the calls `beside`
/// it; else bowed off them by the gentlest bend, no more than [`DETOUR`] times as
/// long, that runs along none of the calls beside it, then clears the most lit
/// dots, then the most plain ones, broken round those it still runs over; straight
/// where no bow does better. It may pass under a name, whose pill hides it there. `None` where its
/// ends are too near for a head.
fn route_one(
    ends: [End; 2],
    side: f32,
    obstacles: &[Obstacle],
    beside: &[Beside],
) -> Option<Route> {
    let [(a, ra), (b, rb)] = ends;
    let along = (b - a).normalized();
    let right = egui::vec2(-along.y, along.x);
    // Two dots near each other keep a smaller gap, so the call still shows.
    let short = a.distance(b) - ra - rb <= SHORT_HEAD.0 + 2.0;
    let (ra, rb) = if short {
        (ra - GAP + SHORT_GAP, rb - GAP + SHORT_GAP)
    } else {
        (ra, rb)
    };
    let straight = vec![a + along * ra + right * side, b - along * rb + right * side];
    if (straight[1] - straight[0]).dot(along) <= SHORT_HEAD.0 {
        return None;
    }
    let cost = |path: &[egui::Pos2], enough: usize| {
        let hits = cost(path, obstacles, enough);
        if hits >= enough {
            return hits;
        }
        hits + SHARED_COST * beside.iter().filter(|&&other| shares(path, other)).count()
    };
    let routed = |path: Vec<egui::Pos2>, others: Vec<Vec<egui::Pos2>>| Route {
        path,
        others,
        kept: true,
    };
    let first = cost(&straight, usize::MAX);
    if first == 0 {
        return Some(routed(straight, Vec::new()));
    }
    let span = a.distance(b);
    let most = length(&straight) * DETOUR;
    // A line both ways bows only to its own right, away from the other.
    let signs: &[f32] = if side > 0.0 { &[1.0] } else { &[1.0, -1.0] };
    let mut best = (first, None);
    let mut clear: Vec<Vec<egui::Pos2>> = Vec::new();
    'search: for &bend in BENDS.iter().filter(|&&bend| bend <= span * MOST_BEND) {
        if !clear.is_empty() {
            break;
        }
        for bow in &BOWS {
            for &sign in signs {
                let pull = |(along, share): (f32, f32)| (along, sign * (bend * share + side));
                let Some(path) = bowed((a, ra), (b, rb), [pull(bow[0]), pull(bow[1])]) else {
                    continue;
                };
                if length(&path) > most {
                    continue;
                }
                let hits = cost(&path, best.0.max(1));
                if hits == 0 {
                    clear.push(path);
                    if clear.len() == CLEAR_WAYS {
                        break 'search;
                    }
                    best.0 = 0;
                } else if hits < best.0 {
                    best = (hits, Some(path));
                }
            }
        }
    }
    // Straight is a way too where it runs along no call.
    let fallback = (first < SHARED_COST).then(|| straight.clone());
    if !clear.is_empty() {
        let path = clear.remove(0);
        return Some(routed(path, clear.into_iter().chain(fallback).collect()));
    }
    // The least in the way, broken round what it still runs over; straight where
    // no bow does better.
    Some(match best.1 {
        Some(path) => routed(path, fallback.into_iter().collect()),
        None => routed(straight, Vec::new()),
    })
}

/// Whether the line through `path` runs along `other`, which shares a dot with it
/// and ends at `far` at its other: as [`SHARED_RUN`] says. Each piece of it is
/// looked at only beside the pieces of `other` near enough.
fn shares(path: &[egui::Pos2], (other, far): Beside) -> bool {
    let (first, last) = (path[0], path[path.len() - 1]);
    let there = egui::Rect::from_center_size(far, egui::Vec2::splat(2.0 * SHARED_FAR));
    let theirs: Vec<(egui::Pos2, egui::Pos2, egui::Rect)> = other
        .windows(2)
        .map(|o| {
            let bounds = egui::Rect::from_two_pos(o[0], o[1]).expand(SHARED_NEAR);
            (o[0], o[1], bounds)
        })
        .filter(|o| o.2.intersects(there))
        .collect();
    let mut run = 0.0;
    for pair in path.windows(2) {
        let bounds = egui::Rect::from_two_pos(pair[0], pair[1]);
        let near: Vec<_> = theirs.iter().filter(|o| o.2.intersects(bounds)).collect();
        if near.is_empty() {
            continue;
        }
        let piece = pair[0].distance(pair[1]);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a few samples"
        )]
        let samples = (piece / 4.0).ceil().max(1.0) as usize;
        #[expect(clippy::cast_precision_loss, reason = "a few samples")]
        let step = piece / samples as f32;
        for k in 0..samples {
            #[expect(clippy::cast_precision_loss, reason = "a few samples")]
            let at = pair[0] + (pair[1] - pair[0]) * (k as f32 / samples as f32);
            if at.distance(far) < SHARED_FAR
                && at.distance(first) >= SHARED_ENDS
                && at.distance(last) >= SHARED_ENDS
                && near.iter().any(|o| off_line(o.0, o.1, at) < SHARED_NEAR)
            {
                run += step;
                if run > SHARED_RUN {
                    return true;
                }
            }
        }
    }
    false
}

/// How near the piece of line from `a` to `b` comes to `at`.
fn off_line(a: egui::Pos2, b: egui::Pos2, at: egui::Pos2) -> f32 {
    let along = b - a;
    let t = ((at - a).dot(along) / along.length_sq().max(f32::EPSILON)).clamp(0.0, 1.0);
    (a + along * t).distance(at)
}

/// What the line through `path` running over `obstacles` costs: [`LIT_COST`] a lit
/// dot, 1 a plain one; counted no further than `enough`.
fn cost(path: &[egui::Pos2], obstacles: &[Obstacle], enough: usize) -> usize {
    let bounds = egui::Rect::from_points(path);
    let mut total = 0;
    for &(at, radius, plain) in obstacles {
        if bounds.distance_to_pos(at) >= radius {
            continue;
        }
        if path
            .windows(2)
            .any(|p| across(p[0], p[1], at, radius).is_some())
        {
            total += if plain { 1 } else { LIT_COST };
            if total >= enough {
                return total;
            }
        }
    }
    total
}

/// Whether the line from `a` to `b` runs through `rect`.
fn through(a: egui::Pos2, b: egui::Pos2, rect: egui::Rect) -> bool {
    // Where it runs inside each pair of the rectangle's sides, as shares of it.
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    let along = b - a;
    for (start, delta, low, high) in [
        (a.x, along.x, rect.min.x, rect.max.x),
        (a.y, along.y, rect.min.y, rect.max.y),
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

/// A line from one dot to another bowed out to its right by its two `pulls`, each
/// where along it and how far out, in points, ending each dot's reach short of it;
/// `None` where too little of it is left for a head.
fn bowed(
    (from, from_reach): End,
    (to, to_reach): End,
    pulls: [(f32, f32); 2],
) -> Option<Vec<egui::Pos2>> {
    let along = to - from;
    let right = egui::vec2(-along.y, along.x).normalized();
    let pulls = pulls.map(|(share, out)| from + along * share + right * out);
    #[expect(clippy::cast_precision_loss, reason = "a few steps")]
    let curve: Vec<egui::Pos2> = (0..=STEPS)
        .map(|step| {
            let t = step as f32 / STEPS as f32;
            let rest = 1.0 - t;
            let point = from.to_vec2() * (rest * rest * rest)
                + pulls[0].to_vec2() * (3.0 * rest * rest * t)
                + pulls[1].to_vec2() * (3.0 * rest * t * t)
                + to.to_vec2() * (t * t * t);
            point.to_pos2()
        })
        .collect();
    // Out of each dot's reach: from where the curve leaves the first to where it
    // comes to the last.
    let first = curve.iter().position(|p| p.distance(from) >= from_reach)?;
    let last = curve.iter().rposition(|p| p.distance(to) >= to_reach)?;
    if first == 0 || last + 1 >= curve.len() || last <= first {
        return None;
    }
    let edge = |inside: egui::Pos2, outside: egui::Pos2, (dot, reach): End| {
        // Where the step between them crosses the reach, near enough.
        let (near, far) = (inside.distance(dot), outside.distance(dot));
        let share = ((reach - near) / (far - near).max(f32::EPSILON)).clamp(0.0, 1.0);
        inside + (outside - inside) * share
    };
    let mut path = vec![edge(curve[first - 1], curve[first], (from, from_reach))];
    path.extend_from_slice(&curve[first..=last]);
    path.push(edge(curve[last + 1], curve[last], (to, to_reach)));
    (length(&path) > SHORT_HEAD.0).then_some(path)
}

/// How long the line through `path` is.
fn length(path: &[egui::Pos2]) -> f32 {
    path.windows(2).map(|p| p[0].distance(p[1])).sum()
}

/// The point `back` along `path` from its end, and the pieces of it before that.
fn cut_back(path: &[egui::Pos2], back: f32) -> (egui::Pos2, Vec<egui::Pos2>) {
    let mut left = back;
    for i in (1..path.len()).rev() {
        let piece = path[i - 1].distance(path[i]);
        if piece >= left {
            let at = path[i] + (path[i - 1] - path[i]).normalized() * left;
            let mut before = path[..i].to_vec();
            before.push(at);
            return (at, before);
        }
        left -= piece;
    }
    (path[0], vec![path[0], path[0]])
}

/// Where the line from `from` to `to` runs inside the circle round `at`, as shares of
/// its length, if it does.
fn across(from: egui::Pos2, to: egui::Pos2, at: egui::Pos2, radius: f32) -> Option<(f32, f32)> {
    let along = to - from;
    let length = along.length_sq();
    if length < f32::EPSILON {
        return None;
    }
    // Where along the line it comes nearest the middle, and how near.
    let t = (at - from).dot(along) / length;
    let nearest = from + along * t;
    let off = (radius * radius - nearest.distance_sq(at)).max(0.0);
    if off <= 0.0 {
        return None;
    }
    let half = off.sqrt() / length.sqrt();
    let (t0, t1) = ((t - half).max(0.0), (t + half).min(1.0));
    (t0 < t1).then_some((t0, t1))
}

/// How a line is drawn: its width and color, its head's length and width, and how it
/// shows where it crosses a sphere it is no part of: faint, or not at all.
#[derive(Debug, Clone, Copy)]
struct Ink {
    width: f32,
    color: Color,
    head: (f32, f32),
    hidden_in_spheres: bool,
}

/// An arrow along `path`, with a filled head at its end, its line strong where it
/// crosses nothing, faint (or hidden) across a sphere, broken round a dot, as
/// `crossings` has each of its pieces, into `shapes`. Returns how it shows.
fn paint(
    shapes: &mut Vec<egui::Shape>,
    path: &[egui::Pos2],
    ink: Ink,
    crossings: &dyn Fn(egui::Pos2, egui::Pos2) -> Vec<(f32, f32, Crossing)>,
) -> ArrowShape {
    let total = length(path);
    // A short line takes a smaller head, as small as can still be seen.
    let head = if total >= ink.head.0 + 2.0 {
        ink.head
    } else {
        SHORT_HEAD
    };
    let to = path[path.len() - 1];
    let (back, shaft) = cut_back(path, head.0);
    let along = (to - back).normalized();
    let across_head = egui::vec2(-along.y, along.x);
    let strong = egui_color(ink.color);
    let faint = egui_color(ink.color.with_alpha(ink.color.alpha() * CROSSING));
    // Where the line breaks round a dot, as how far along it from its start.
    let mut gaps: Vec<[f32; 2]> = Vec::new();
    let mut run = 0.0;
    for piece in shaft.windows(2) {
        let (from, until) = (piece[0], piece[1]);
        let long = from.distance(until);
        let crossed = crossings(from, until);
        let mut cuts: Vec<f32> = crossed
            .iter()
            .flat_map(|&(t0, t1, _)| [t0, t1])
            .filter(|&t| t > 0.0 && t < 1.0)
            .chain([0.0, 1.0])
            .collect();
        cuts.sort_by(f32::total_cmp);
        cuts.dedup();
        for pair in cuts.windows(2) {
            let (t0, t1) = (pair[0], pair[1]);
            let middle = f32::midpoint(t0, t1);
            let over = |kind: Crossing| {
                crossed
                    .iter()
                    .any(|&(a, b, k)| k == kind && a <= middle && middle <= b)
            };
            let in_sphere = over(Crossing::Sphere);
            if over(Crossing::Dot) {
                let (s0, s1) = (run + t0 * long, run + t1 * long);
                match gaps.last_mut() {
                    Some(gap) if gap[1] >= s0 - 0.01 => gap[1] = s1,
                    _ => gaps.push([s0, s1]),
                }
                continue;
            }
            if in_sphere && ink.hidden_in_spheres {
                continue;
            }
            let color = if in_sphere { faint } else { strong };
            shapes.push(egui::Shape::line_segment(
                [from + (until - from) * t0, from + (until - from) * t1],
                egui::Stroke::new(ink.width, color),
            ));
        }
        run += long;
    }
    let half = head.1 / 2.0;
    shapes.push(egui::Shape::convex_polygon(
        vec![to, back + across_head * half, back - across_head * half],
        strong,
        egui::Stroke::NONE,
    ));
    ArrowShape {
        width: ink.width,
        head: head.0,
        head_width: head.1,
        tip: Vec2::new(to.x, to.y),
        path: path.iter().map(|p| Vec2::new(p.x, p.y)).collect(),
        gaps,
    }
}

/// Every call the nucleus names along its line in `routes`, and browsing with
/// nothing selected every line between modules, among the `names` placed; then where
/// calls leave the free area, the functions at their far ends. Noted in `marks`;
/// returns what to paint. The calls followed are drawn last, over the rest: the
/// journey's into the cursor, and the calls of a function set aside.
pub(crate) fn draw(
    painter: &egui::Painter,
    explorer: &Explorer,
    (camera, view): (&Camera, &GlobalTransform),
    routes: &Routes,
    names: &GraphNames,
    marks: &mut Marks,
) -> Vec<egui::Shape> {
    marks.calls.clear();
    marks.tubes.clear();
    let mut shapes = Vec::new();
    let screen = Screen::new(explorer, camera, view);
    let nucleus = explorer.nucleus();
    let graph = explorer.graph();
    draw_map(&mut shapes, &screen, nucleus, marks);
    // The faint calls inside the module expanded first, the ones followed last.
    let mut order: Vec<(bool, &Link)> = nucleus
        .links()
        .iter()
        .map(|link| (followed(explorer, link), link))
        .collect();
    order.sort_by_key(|&(bright, link)| (bright, link.flow != Flow::Inner));
    let mut drawn = Vec::new();
    for (bright, link) in order {
        let Some(route) = routes.lines.get(&(link.from, link.to)) else {
            continue;
        };
        // A line through its own end's name goes another way as clear, if it has one.
        // Into its callee's name: over its head, or, bowed, looping past it.
        let own_name = |path: &[egui::Pos2]| {
            let line = if path.len() > 2 {
                path.to_vec()
            } else {
                arrival(path)
            };
            names.pills().any(|(pill, tile)| {
                tile == link.to && line.windows(2).any(|p| through(p[0], p[1], pill))
            })
        };
        let path = std::iter::once(&route.path)
            .chain(&route.others)
            .find(|path| !own_name(path))
            .unwrap_or(&route.path);
        // A lit call crossing the view with neither end in it is faint: no label
        // says what it joins.
        let passing = link.flow != Flow::Inner
            && !names.area.contains(path[0])
            && !names.area.contains(path[path.len() - 1]);
        let bright = bright && !passing;
        let color = link_color(explorer, link, bright);
        let color = if passing {
            color.with_alpha(color.alpha().min(ACROSS_STRENGTH))
        } else {
            color
        };
        let (width, head) = match link.flow {
            _ if bright => (BRIGHT_WIDTH, HEAD),
            Flow::Inner => (INNER_WIDTH, MAP_HEAD),
            _ => (CALL_WIDTH, HEAD),
        };
        let ink = Ink {
            width,
            color,
            head,
            hidden_in_spheres: false,
        };
        let modules = [graph.cluster(link.from), graph.cluster(link.to)];
        let tiles = [Some(link.from), Some(link.to)];
        let shape = paint(&mut shapes, path, ink, &|from, to| {
            screen.crossings((from, to), modules, tiles)
        });
        drawn.push((*link, path.clone()));
        marks.calls.push(CallMark {
            from: link.from,
            to: link.to,
            flow: link.flow,
            color,
            shape,
        });
    }
    marks.exits = exits(painter, (explorer, &screen, names), &drawn, &mut shapes);
    shapes
}

/// Where the line through `path` leaves `area`, from the end inside it to the end
/// outside: the point on the edge, the way into the area there, and whether the end
/// outside is the line's last (its callee's).
fn leaves(area: egui::Rect, path: &[egui::Pos2]) -> Option<(egui::Pos2, egui::Vec2, bool)> {
    let (first, last) = (*path.first()?, *path.last()?);
    let (forward, points): (bool, Vec<egui::Pos2>) =
        match (area.contains(first), area.contains(last)) {
            (true, false) => (true, path.to_vec()),
            (false, true) => (false, path.iter().rev().copied().collect()),
            _ => return None,
        };
    let piece = points
        .windows(2)
        .find(|p| area.contains(p[0]) && !area.contains(p[1]))?;
    // Halve the piece down to where it crosses the edge.
    let (mut inside, mut outside) = (piece[0], piece[1]);
    for _ in 0..16 {
        let middle = inside + (outside - inside) / 2.0;
        if area.contains(middle) {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    let edges = [
        (inside.x - area.min.x, egui::vec2(1.0, 0.0)),
        (area.max.x - inside.x, egui::vec2(-1.0, 0.0)),
        (inside.y - area.min.y, egui::vec2(0.0, 1.0)),
        (area.max.y - inside.y, egui::vec2(0.0, -1.0)),
    ];
    let inward = edges
        .iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(egui::vec2(0.0, -1.0), |e| e.1);
    Some((inside, inward, forward))
}

/// The calls leaving the free area for one function, or past [`EXITS_APART`] for one
/// module: what one exit label names.
struct Far {
    /// The functions off the free area, first by the panel's order.
    fars: Vec<usize>,
    module: Option<usize>,
    /// Each call, the function at its far end, where it crosses the edge and the way
    /// into the free area there.
    calls: Vec<(Link, usize, egui::Pos2, egui::Vec2)>,
    /// Whether calls go out to it, and whether they come in from it.
    out: bool,
    into: bool,
}

/// The number the panel shows a function by, on the cursor's path where it runs
/// there, else its first; none for the start, which goes by its name.
pub(crate) fn path_number(explorer: &Explorer, tile: usize) -> Option<String> {
    if explorer.start() == Some(tile) {
        return None;
    }
    let disclosed = explorer.disclosure();
    let numbers = disclosed.tiles().get(&tile)?;
    let cursor = explorer
        .cursor_span()
        .and_then(|span| disclosed.number(span))
        .unwrap_or_default()
        .to_owned();
    let on_path = |n: &&String| {
        !cursor.is_empty()
            && (**n == cursor
                || n.starts_with(&format!("{cursor}."))
                || cursor.starts_with(&format!("{n}.")))
    };
    numbers
        .iter()
        .find(on_path)
        .or_else(|| numbers.iter().min_by_key(|n| disclosure::number_order(n)))
        .cloned()
}

/// Where calls leave the free area, labelled once per function at their far ends,
/// or past [`EXITS_APART`] of those, once per module: with a module expanded, every
/// call lit; collapsed in a journey, the calls in from outside. Each label sits
/// just inside the edge where its calls leave, slid along it clear of names, dots,
/// the labels before and, where it can, the lines.
fn exits(
    painter: &egui::Painter,
    (explorer, screen, names): (&Explorer, &Screen, &GraphNames),
    drawn: &[(Link, Vec<egui::Pos2>)],
    shapes: &mut Vec<egui::Shape>,
) -> Vec<ExitMark> {
    let nucleus = explorer.nucleus();
    let area = names.area;
    let mut marks: Vec<ExitMark> = Vec::new();
    let mut placed: Vec<egui::Rect> = Vec::new();
    for far in &leaving(explorer, area, drawn) {
        let text = exit_text(explorer, far);
        let (link, _, at, inward) = far.calls[0];
        // In the color of the calls it labels: pink where the cursor's path runs in.
        let path = into_cursor(explorer);
        let shade = if far.calls.iter().any(|c| Some((c.0.from, c.0.to)) == path) {
            CURSOR
        } else {
            link_color(explorer, &link, false)
        };
        let shade = egui_color(shade.with_alpha(1.0));
        let galley =
            painter.layout_no_wrap(text.clone(), egui::FontId::proportional(text::SMALL), shade);
        let size = galley.size() + 2.0 * PILL_PAD;
        // Where its calls leave along the edge the first leaves by.
        let edge: Vec<egui::Pos2> = far
            .calls
            .iter()
            .filter(|c| c.3 == inward)
            .map(|c| c.2)
            .collect();
        #[expect(clippy::cast_precision_loss, reason = "a few calls")]
        let anchor = edge
            .iter()
            .fold(egui::Pos2::ZERO, |sum, p| sum + p.to_vec2())
            / edge.len().max(1) as f32;
        let anchor = if edge.is_empty() { at } else { anchor };
        let Some(rect) = exit_spot(
            (area, anchor, inward, size),
            &|rect| {
                names.every_pill().any(|pill| pill.intersects(rect))
                    || placed.iter().any(|p| p.expand(1.0).intersects(rect))
                    || screen
                        .dots
                        .iter()
                        .any(|d| !d.speck && rect.distance_to_pos(d.at) < d.radius + 1.0)
                    || (nucleus.browsing()
                        && key_help::hint_rect(painter, area, nucleus.expanded()).intersects(rect))
            },
            drawn,
        ) else {
            continue;
        };
        placed.push(rect);
        shapes.push(egui::Shape::rect_filled(rect, 4.0, PILL));
        shapes.push(egui::Shape::rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, shade),
            egui::StrokeKind::Inside,
        ));
        shapes.push(egui::Shape::galley(rect.min + PILL_PAD, galley, shade));
        marks.extend(far.calls.iter().map(|&(link, far, at, _)| ExitMark {
            from: link.from,
            to: link.to,
            far,
            text: text.clone(),
            rect,
            at: Vec2::new(at.x, at.y),
        }));
    }
    marks
}

/// The calls leaving the free area, by the function at their far end, or past
/// [`EXITS_APART`] of those, by its module.
fn leaving(explorer: &Explorer, area: egui::Rect, drawn: &[(Link, Vec<egui::Pos2>)]) -> Vec<Far> {
    let nucleus = explorer.nucleus();
    let graph = explorer.graph();
    let mut fars: Vec<Far> = Vec::new();
    for (link, path) in drawn {
        // Expanded, every call lit; collapsed, the calls in from outside.
        let labelled = match nucleus.expanded_module() {
            Some(_) => link.flow != Flow::Inner,
            None => link.flow == Flow::Outside,
        };
        if !labelled {
            continue;
        }
        let Some((at, inward, out)) = leaves(area, path) else {
            continue;
        };
        let far = if out { link.to } else { link.from };
        let call = (*link, far, at, inward);
        if let Some(known) = fars.iter_mut().find(|f| f.fars[0] == far) {
            known.calls.push(call);
            known.out |= out;
            known.into |= !out;
        } else {
            fars.push(Far {
                fars: vec![far],
                module: graph.cluster(far),
                calls: vec![call],
                out,
                into: !out,
            });
        }
    }
    let order = |tile: usize| {
        let number = path_number(explorer, tile).unwrap_or_default();
        (disclosure::number_order(&number), tile)
    };
    fars.sort_by_cached_key(|f| order(f.fars[0]));
    if fars.len() > EXITS_APART {
        let mut modules: Vec<Far> = Vec::new();
        for far in fars {
            if let Some(known) = modules.iter_mut().find(|m| m.module == far.module) {
                known.fars.extend(far.fars);
                known.calls.extend(far.calls);
                known.out |= far.out;
                known.into |= far.into;
            } else {
                modules.push(far);
            }
        }
        fars = modules;
    }
    fars
}

/// What an exit label says, one wording for one far function or several: the way
/// its calls go, then the function with the number the panel shows it by and its
/// module (`to 2 tile_of · MAP`); or their module and the first few (`to MAP: 2
/// tile_of, 3 point +2`).
fn exit_text(explorer: &Explorer, far: &Far) -> String {
    let module = far
        .module
        .map(|c| explorer.cluster_name(c).to_uppercase())
        .unwrap_or_default();
    let way = match (far.out, far.into) {
        (true, true) => "to and from",
        (true, false) => "to",
        _ => "from",
    };
    let called = |tile: usize| {
        let name = node_name(explorer, tile);
        path_number(explorer, tile).map_or(name.clone(), |n| format!("{n} {name}"))
    };
    if let [one] = far.fars[..] {
        format!("{way} {} · {module}", called(one))
    } else {
        let first: Vec<String> = far
            .fars
            .iter()
            .take(EXIT_NAMED)
            .map(|&t| called(t))
            .collect();
        let more = far.fars.len().saturating_sub(EXIT_NAMED);
        let more = if more > 0 {
            format!(" +{more}")
        } else {
            String::new()
        };
        format!("{way} {module}: {}{more}", first.join(", "))
    }
}

/// Where an exit label of `size` goes: just inside `area`'s edge at `anchor` (the
/// way in from it `inward`), else slid along the edge, else a row further in, the
/// nearest first; where `taken` says it is clear, and among the first
/// [`CLEAR_TRIES`] places, crossed by no line drawn where one is.
fn exit_spot(
    (area, anchor, inward, size): (egui::Rect, egui::Pos2, egui::Vec2, egui::Vec2),
    taken: &dyn Fn(egui::Rect) -> bool,
    drawn: &[(Link, Vec<egui::Pos2>)],
) -> Option<egui::Rect> {
    let along = egui::vec2(-inward.y, inward.x);
    let into = f32::midpoint(size.x * inward.x.abs(), size.y * inward.y.abs()) + EXIT_IN;
    let row = (size * inward).length() + 2.0;
    let reach = (area.size() * along).length();
    let mut tries: Vec<(f32, f32)> = Vec::new();
    for rows in [0.0, 1.0, 2.0] {
        let mut by = 0.0;
        while by <= reach {
            tries.push((rows * row, by));
            if by > 0.0 {
                tries.push((rows * row, -by));
            }
            by += EXIT_STEP;
        }
    }
    tries.sort_by(|a, b| (a.1.abs() + a.0 * 2.0).total_cmp(&(b.1.abs() + b.0 * 2.0)));
    // The lines near enough to cross a place among the first tried.
    let near = egui::Rect::from_center_size(anchor, size).expand(EXIT_STEP * 12.0 + 3.0 * row);
    let pieces: Vec<(egui::Pos2, egui::Pos2)> = drawn
        .iter()
        .flat_map(|(_, path)| path.windows(2).map(|p| (p[0], p[1])))
        .filter(|&(p, q)| egui::Rect::from_two_pos(p, q).intersects(near))
        .collect();
    let mut free = None;
    for (tried, (row, by)) in tries.into_iter().enumerate() {
        let rect = egui::Rect::from_center_size(anchor + inward * (into + row) + along * by, size);
        let rect = rect.translate(egui::vec2(
            (area.min.x - rect.min.x).max(0.0) - (rect.max.x - area.max.x).max(0.0),
            (area.min.y - rect.min.y).max(0.0) - (rect.max.y - area.max.y).max(0.0),
        ));
        if taken(rect) {
            continue;
        }
        if tried >= CLEAR_TRIES {
            return free.or(Some(rect));
        }
        free = free.or(Some(rect));
        if !pieces.iter().any(|&(p, q)| through(p, q, rect)) {
            return Some(rect);
        }
    }
    free
}

/// Browsing with nothing selected, a thin faint line from each module's sphere to
/// every module's it calls, one width, behind the spheres it passes.
fn draw_map(shapes: &mut Vec<egui::Shape>, screen: &Screen, nucleus: &Nucleus, marks: &mut Marks) {
    let tubes = nucleus.tubes();
    for (index, tube) in tubes.iter().enumerate() {
        let (Some((a, ra)), Some((b, rb))) = (screen.sphere(tube.from), screen.sphere(tube.to))
        else {
            continue;
        };
        let along = (b - a).normalized();
        let (from, to) = (a + along * (ra + GAP), b - along * (rb + GAP));
        if (to - from).dot(along) <= MAP_HEAD.0 {
            continue;
        }
        let back = tubes.iter().any(|t| t.from == tube.to && t.to == tube.from);
        let (from, to) = if back {
            beside(from, to, BOTH_WAYS)
        } else {
            (from, to)
        };
        let own = [Some(tube.from), Some(tube.to)];
        let ink = Ink {
            width: MAP_WIDTH,
            color: FURTHER.with_alpha(MAP_STRENGTH),
            head: MAP_HEAD,
            hidden_in_spheres: true,
        };
        let shape = paint(shapes, &[from, to], ink, &|from, to| {
            screen
                .crossings((from, to), own, [None; 2])
                .into_iter()
                .filter(|c| c.2 == Crossing::Sphere)
                .collect()
        });
        marks.tubes.push(TubeMark { index, shape });
    }
}

/// Moves a line `by` points to its right, so two running opposite ways between the
/// same ends lie side by side.
fn beside(from: egui::Pos2, to: egui::Pos2, by: f32) -> (egui::Pos2, egui::Pos2) {
    let along = (to - from).normalized();
    let right = egui::vec2(-along.y, along.x) * by;
    (from + right, to + right)
}

/// The call on the cursor's path into the cursor, caller and callee: from the call
/// the cursor's is made in (the start, for one of the start's own) to the cursor's
/// function. The one call drawn pink; others into that function keep their role's
/// color.
fn into_cursor(explorer: &Explorer) -> Option<(usize, usize)> {
    let disclosed = explorer.disclosure();
    let number = disclosed.number(explorer.cursor_span()?)?;
    if number.is_empty() {
        return None;
    }
    let from = match number.rsplit_once('.') {
        Some((parent, _)) => disclosed
            .tiles()
            .iter()
            .filter(|(_, numbers)| numbers.iter().any(|n| n == parent))
            .map(|(&tile, _)| tile)
            .min()?,
        None => explorer.start()?,
    };
    Some((from, explorer.cursor()?))
}

/// Whether a call is one followed, drawn over the rest: the journey's into the
/// cursor along its path, and the calls of the function set aside. Pointing at a
/// function lights none: hovering only rings it.
fn followed(explorer: &Explorer, link: &Link) -> bool {
    let into_cursor = matches!(link.flow, Flow::Journey(_))
        && into_cursor(explorer) == Some((link.from, link.to));
    // A call among the expanded module's functions stays faint, out of the function
    // set aside too: its role's color.
    let aside =
        link.flow != Flow::Inner && explorer.aside().is_some_and(|aside| aside == link.from);
    into_cursor || aside
}

/// A call's color: its flow's, the cursor's for the journey's call into the cursor
/// along its path;
/// faint for a call only guessed, a call among the expanded module's functions, one
/// from off the journey and a caller of the start folded into its module's count;
/// the calls of a function set aside in their flow's color at full strength, grey
/// ones a little stronger.
fn link_color(explorer: &Explorer, link: &Link, bright: bool) -> Color {
    let nucleus = explorer.nucleus();
    let color = link.flow.color();
    match link.flow {
        Flow::Outside if bright => color.with_alpha(PICKED_STRENGTH),
        Flow::Outside => color.with_alpha(OUTSIDE_STRENGTH),
        Flow::Inner => color.with_alpha(INNER_STRENGTH),
        _ if nucleus.guessed(link) => color.with_alpha(GUESSED_STRENGTH),
        Flow::Into if !bright && nucleus.folded(link.from) => color.with_alpha(FOLDED_STRENGTH),
        Flow::Journey(_) if bright && into_cursor(explorer) == Some((link.from, link.to)) => CURSOR,
        _ if bright => color,
        _ => color.with_alpha(CALL_STRENGTH),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_crosses_a_circle_where_it_runs_inside_it() {
        let (from, to) = (egui::pos2(0.0, 0.0), egui::pos2(100.0, 0.0));
        let (t0, t1) = across(from, to, egui::pos2(50.0, 0.0), 10.0).expect("crosses");
        assert!((t0 - 0.4).abs() < 1e-4 && (t1 - 0.6).abs() < 1e-4);
        assert!(
            across(from, to, egui::pos2(50.0, 20.0), 10.0).is_none(),
            "passes by"
        );
        let (t0, _) = across(from, to, egui::pos2(0.0, 0.0), 10.0).expect("starts inside");
        assert!(t0.abs() < 1e-6);
    }

    #[test]
    fn a_line_runs_through_a_rectangle_it_crosses_and_not_one_beside_it() {
        let rect = egui::Rect::from_min_max(egui::pos2(40.0, -5.0), egui::pos2(60.0, 5.0));
        let (from, to) = (egui::pos2(0.0, 0.0), egui::pos2(100.0, 0.0));
        assert!(through(from, to, rect));
        assert!(!through(
            from,
            egui::pos2(100.0, 20.0),
            rect.translate(egui::vec2(0.0, 30.0))
        ));
        assert!(!through(from, egui::pos2(30.0, 0.0), rect), "stops short");
    }

    #[test]
    fn a_bent_line_clears_a_dot_in_the_way_and_ends_short_of_its_dots() {
        let (a, b) = (egui::pos2(0.0, 0.0), egui::pos2(200.0, 0.0));
        let obstacles = [(egui::pos2(100.0, 0.0), 6.0, false)];
        assert_eq!(cost(&[a, b], &obstacles, usize::MAX), LIT_COST);
        let route = route_one([(a, 8.0), (b, 8.0)], 0.0, &obstacles, &[]).expect("long enough");
        assert!(route.path.len() > 2, "bent");
        assert_eq!(cost(&route.path, &obstacles, usize::MAX), 0);
        assert!(length(&route.path) <= 184.0 * DETOUR);
        // The other ways: as clear, bowed, then straight, which runs along no call.
        let (straight, bowed) = route.others.split_last().expect("straight last");
        assert_eq!(straight.len(), 2);
        for other in bowed {
            assert_eq!(cost(other, &obstacles, usize::MAX), 0, "as clear");
        }
        assert!((route.path[0].distance(a) - 8.0).abs() < 0.5);
        assert!((route.path[route.path.len() - 1].distance(b) - 8.0).abs() < 0.5);
    }

    #[test]
    fn a_line_no_gentle_bow_clears_goes_straight() {
        let (a, b) = (egui::pos2(0.0, 0.0), egui::pos2(200.0, 0.0));
        // A wall of dots across the line: clearing it takes more than the bound.
        let obstacles: Vec<Obstacle> = (-12..=12_i16)
            .map(|y| (egui::pos2(100.0, f32::from(y) * 10.0), 6.0, false))
            .collect();
        let route = route_one([(a, 8.0), (b, 8.0)], 0.0, &obstacles, &[]).expect("long enough");
        assert_eq!(route.path.len(), 2, "straight");
    }

    #[test]
    fn a_line_bows_round_lit_dots_before_plain_ones() {
        let (a, b) = (egui::pos2(0.0, 0.0), egui::pos2(200.0, 0.0));
        // Plain dots everywhere a bow could go, a lit one on the straight line.
        let mut obstacles: Vec<Obstacle> = (1..20_i16)
            .flat_map(|x| {
                [-1_i16, 1].map(|s| {
                    (
                        egui::pos2(f32::from(x) * 10.0, f32::from(s) * 12.0),
                        3.0,
                        true,
                    )
                })
            })
            .collect();
        obstacles.push((egui::pos2(100.0, 0.0), 4.0, false));
        let route = route_one([(a, 8.0), (b, 8.0)], 0.0, &obstacles, &[]).expect("long enough");
        assert!(
            cost(&route.path, &obstacles, usize::MAX) < LIT_COST,
            "keeps off the lit dot"
        );
    }

    #[test]
    fn a_call_that_would_run_along_another_bows_off_it() {
        // Two calls from one dot the same way, one ending short of the other's end.
        let a = egui::pos2(0.0, 0.0);
        let near = route_one([(a, 8.0), (egui::pos2(0.0, -150.0), 8.0)], 0.0, &[], &[])
            .expect("long enough");
        assert_eq!(near.path.len(), 2, "the first runs straight");
        let far_end = [(a, 8.0), (egui::pos2(1.0, -300.0), 8.0)];
        let alone = route_one(far_end, 0.0, &[], &[]).expect("long enough");
        assert!(
            shares(&alone.path, (&near.path, near.path[1])),
            "straight, it runs along it through its end"
        );
        // Calls fanning out to two dots near each other run close only by their
        // shared dot.
        let fan = route_one([(a, 8.0), (egui::pos2(30.0, -300.0), 8.0)], 0.0, &[], &[])
            .expect("long enough");
        let other = route_one([(a, 8.0), (egui::pos2(20.0, -300.0), 8.0)], 0.0, &[], &[])
            .expect("long enough");
        assert!(!shares(&fan.path, (&other.path, other.path[1])));
        let near_end = near.path[1];
        let beside = route_one(far_end, 0.0, &[], &[(&near.path, near_end)]).expect("long enough");
        assert!(beside.path.len() > 2, "bowed");
        assert!(!shares(&beside.path, (&near.path, near_end)));
        assert!(length(&beside.path) <= length(&alone.path) * DETOUR);
        // Only calls sharing a dot can, and not both ways between two.
        assert!(share_a_dot((0, 1), (0, 2)));
        assert!(share_a_dot((0, 1), (2, 1)));
        assert!(!share_a_dot((0, 1), (2, 3)));
        assert!(!share_a_dot((0, 1), (1, 0)));
    }

    #[test]
    fn a_lines_arrival_is_its_last_stretch_into_its_dot() {
        let path = [
            egui::pos2(0.0, 0.0),
            egui::pos2(100.0, 0.0),
            egui::pos2(100.0, 20.0),
        ];
        let end = arrival(&path);
        assert_eq!(end.len(), 3);
        assert!((end[0].x - 90.0).abs() < 1e-3 && end[0].y.abs() < 1e-3);
        assert!((length(&end) - ARRIVAL).abs() < 1e-3);
        assert_eq!(arrival(&path[..2]), vec![egui::pos2(70.0, 0.0), path[1]]);
    }

    #[test]
    fn the_head_sits_at_the_end_of_the_line_cut_back_its_length() {
        let path = [
            egui::pos2(0.0, 0.0),
            egui::pos2(10.0, 0.0),
            egui::pos2(20.0, 0.0),
        ];
        let (back, before) = cut_back(&path, 8.0);
        assert!((back.x - 12.0).abs() < 1e-4);
        assert_eq!(before.len(), 3);
    }
}
