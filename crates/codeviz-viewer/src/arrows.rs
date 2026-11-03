//! The calls on the graph, drawn flat over the scene at one size on screen however
//! near the view: each call a line from its caller's dot to just short of its
//! callee's, with one filled head there, straight where no dot is in its way and
//! bowed gently round one that is; browsing with nothing selected, a thin faint line
//! from each module to every module it calls, behind the spheres it passes. A call's
//! line runs faint where it crosses a sphere it neither leaves nor enters, and breaks
//! round a dot no gentle bow clears, so it never reads as passing through either.
//!
//! Where each line runs is worked out once for how things lie and where the camera
//! stands ([`route`]), against every dot the journey lays out at any depth, so a
//! level more or less, the callers from outside, a click or the pointer bend no line;
//! names are placed after, keeping off the lines.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bevy_egui::egui;

use crate::{
    colors::{
        CALL_STRENGTH, CURSOR, FOLDED_STRENGTH, FURTHER, GUESSED_STRENGTH, INNER_STRENGTH,
        MAP_STRENGTH, OUTSIDE_STRENGTH, PICKED_STRENGTH, PILL, egui_color,
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
/// Kept clear round a dot a line passes without ending there, in points.
const DOT_CLEAR: f32 = 2.0;
/// What running over a lit dot costs a line, against a plain one's 1: a line keeps
/// off every lit dot, and off as many plain ones as it can.
const LIT_COST: usize = 100;
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
    /// Every dot drawn full size, by tile: its middle and radius on screen.
    dots: Vec<(usize, egui::Pos2, f32)>,
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
        screen.dots = (0..explorer.map().tiles.len())
            .filter(|&tile| nucleus.drawn(tile) && !nucleus.speck(tile))
            .filter_map(|tile| {
                let world = explorer.tile_point(tile)?;
                let (at, radius) =
                    screen.disc(world, drawn_radius(explorer, tile, camera, view))?;
                Some((tile, at, radius))
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

    /// Every dot a call's line keeps off: browsing, every one lit; in a journey, every
    /// one it lays out at any depth, and the plain functions of the module expanded;
    /// never a caller from outside or the function set aside off the journey, which
    /// come and go. Each as far as a ring round it reaches, as big as the cursor's,
    /// and a plain one as it is drawn.
    fn obstacles(&self) -> Vec<(usize, Obstacle)> {
        let nucleus = self.explorer.nucleus();
        (0..self.explorer.map().tiles.len())
            .filter_map(|tile| {
                let member = self.laid_out(tile);
                // Browsing, every function of the module expanded is drawn; those
                // nothing lights are plain.
                let plain = nucleus.opened_up(tile)
                    && if nucleus.browsing() {
                        nucleus.level(tile).is_none() && nucleus.start() != Some(tile)
                    } else {
                        !member
                    };
                if !member && !plain {
                    return None;
                }
                let dot = if plain { Dot::Lit } else { Dot::Focus };
                let (at, radius) = self.dot_as(tile, dot)?;
                let reach = if plain { radius } else { radius * RING };
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
    /// cross a sphere other than `own` or a dot other than its ends'.
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
            .filter(|(t, ..)| !ends.contains(&Some(*t)))
            .filter_map(|&(_, at, radius)| {
                let (t0, t1) = across(from, to, at, radius + DOT_CLEAR)?;
                Some((t0, t1, Crossing::Dot))
            });
        spheres.chain(dots).collect()
    }
}

/// How a function's dot is drawn now.
fn graph_dot(explorer: &Explorer, tile: usize) -> Dot {
    crate::graph_view::dot(explorer, tile)
}

/// A call's line as worked out: bowed round what is in its way, and straight.
#[derive(Debug, Clone)]
struct Route {
    path: Vec<egui::Pos2>,
    straight: Vec<egui::Pos2>,
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
    let lines = calls
        .into_iter()
        .filter_map(|((from, to), (avoid, kept))| {
            let ends = screen.ends((from, to))?;
            let side = if pairs.contains(&(to, from)) {
                BOTH_WAYS
            } else {
                0.0
            };
            let near = |tile: usize| tile != from && tile != to;
            let reach =
                egui::Rect::from_two_pos(ends[0].0, ends[1].0).expand(BENDS[BENDS.len() - 1]);
            let obstacles: Vec<Obstacle> = obstacles
                .iter()
                .filter(|&&(tile, (at, ..))| avoid && near(tile) && reach.contains(at))
                .map(|&(_, o)| o)
                .collect();
            let mut route = route_one(ends, side, &obstacles)?;
            route.kept = kept;
            Some(((from, to), route))
        })
        .collect();
    Routes { lines }
}

/// The line of a call between `ends`, moved `side` points to its right: straight
/// where it passes no dot of `obstacles`; else bowed round them by the gentlest bend,
/// no more than [`DETOUR`] times as long, that clears every lit dot and the most
/// plain ones; else straight, broken round the dot. It may pass under a name, whose
/// pill hides it there. `None` where its ends are too near for a head.
fn route_one(ends: [End; 2], side: f32, obstacles: &[Obstacle]) -> Option<Route> {
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
    let routed = |path: Vec<egui::Pos2>| Route {
        path,
        straight: straight.clone(),
        kept: true,
    };
    let first = cost(&straight, obstacles, usize::MAX);
    if first == 0 {
        return Some(routed(straight.clone()));
    }
    let span = a.distance(b);
    let most = length(&straight) * DETOUR;
    // A line both ways bows only to its own right, away from the other.
    let signs: &[f32] = if side > 0.0 { &[1.0] } else { &[1.0, -1.0] };
    let mut best = (first, None);
    for &bend in BENDS.iter().filter(|&&bend| bend <= span * MOST_BEND) {
        for bow in &BOWS {
            for &sign in signs {
                let pull = |(along, share): (f32, f32)| (along, sign * (bend * share + side));
                let Some(path) = bowed((a, ra), (b, rb), [pull(bow[0]), pull(bow[1])]) else {
                    continue;
                };
                if length(&path) > most {
                    continue;
                }
                let hits = cost(&path, obstacles, best.0);
                if hits == 0 {
                    return Some(routed(path));
                }
                if hits < best.0 {
                    best = (hits, Some(path));
                }
            }
        }
    }
    // A line no bow keeps off every lit dot goes straight, broken round it.
    Some(routed(match best {
        (hits, Some(path)) if hits < LIT_COST => path,
        _ => straight.clone(),
    }))
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
    for piece in shaft.windows(2) {
        let (from, until) = (piece[0], piece[1]);
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
            if over(Crossing::Dot) || (in_sphere && ink.hidden_in_spheres) {
                continue;
            }
            let color = if in_sphere { faint } else { strong };
            shapes.push(egui::Shape::line_segment(
                [from + (until - from) * t0, from + (until - from) * t1],
                egui::Stroke::new(ink.width, color),
            ));
        }
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
        // A line bowed through its own end's name goes straight instead.
        let own_name = names.pills().any(|(pill, tile)| {
            (tile == link.from || tile == link.to)
                && route.path.windows(2).any(|p| through(p[0], p[1], pill))
        });
        let path = if route.path.len() > 2 && own_name {
            &route.straight
        } else {
            &route.path
        };
        let color = link_color(explorer, link, bright);
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
fn path_number(explorer: &Explorer, tile: usize) -> Option<String> {
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
        let shade = egui_color(link_color(explorer, &link, true).with_alpha(1.0));
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
                        .any(|&(_, dot, radius)| rect.distance_to_pos(dot) < radius + 1.0)
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

/// What an exit label says: the way its calls go, and the function at their far end
/// with the number the panel shows it by and its module; or, for several, their
/// module and the first few.
fn exit_text(explorer: &Explorer, far: &Far) -> String {
    let module = far
        .module
        .map(|c| explorer.cluster_name(c).to_uppercase())
        .unwrap_or_default();
    if let [one] = far.fars[..] {
        let name = node_name(explorer, one);
        let called = path_number(explorer, one).map_or(name.clone(), |n| format!("{n} {name}"));
        let way = match (far.out, far.into) {
            (true, true) => "to and from",
            (true, false) => "to",
            _ => "from",
        };
        format!("{way} {called} · {module}")
    } else {
        let way = match (far.out, far.into) {
            (true, true) => "↔",
            (true, false) => "→",
            _ => "←",
        };
        let first: Vec<String> = far
            .fars
            .iter()
            .take(EXIT_NAMED)
            .map(|&t| node_name(explorer, t))
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

/// Whether a call is one followed, drawn over the rest: the journey's into the
/// cursor, and the calls of the function set aside. Pointing at a function lights
/// none: hovering only rings it.
fn followed(explorer: &Explorer, link: &Link) -> bool {
    let into_cursor = matches!(link.flow, Flow::Journey(_)) && explorer.cursor() == Some(link.to);
    // A call among the expanded module's functions stays faint, out of the function
    // set aside too: its role's color.
    let aside =
        link.flow != Flow::Inner && explorer.aside().is_some_and(|aside| aside == link.from);
    into_cursor || aside
}

/// A call's color: its flow's, the cursor's for the journey's call into the cursor;
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
        Flow::Journey(_) if bright && explorer.cursor() == Some(link.to) => CURSOR,
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
        let route = route_one([(a, 8.0), (b, 8.0)], 0.0, &obstacles).expect("long enough");
        assert!(route.path.len() > 2, "bent");
        assert_eq!(cost(&route.path, &obstacles, usize::MAX), 0);
        assert!(length(&route.path) <= length(&route.straight) * DETOUR);
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
        let route = route_one([(a, 8.0), (b, 8.0)], 0.0, &obstacles).expect("long enough");
        assert_eq!(route.path, route.straight);
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
        let route = route_one([(a, 8.0), (b, 8.0)], 0.0, &obstacles).expect("long enough");
        assert!(
            cost(&route.path, &obstacles, usize::MAX) < LIT_COST,
            "keeps off the lit dot"
        );
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
