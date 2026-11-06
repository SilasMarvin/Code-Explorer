//! The calls on the graph, drawn flat over the scene at one size on screen however
//! near the view: each call one straight line from its caller's dot to just short of
//! its callee's, with one filled head there; browsing with nothing selected, a thin
//! faint line from each module to every module it calls, behind the spheres it
//! passes. A call's line is never broken: across a dot it passes, the dot shows over
//! it. A lit call crossing the view with both its ends off it is faint. In a journey
//! the cursor's path down from the start is drawn pink ([`Spine`]), and once two
//! levels show, the calls off it but the cursor's own are drawn dimmer.
//!
//! Where each line runs is worked out once for how things lie and where the camera
//! stands ([`route`]), each end as big as the journey ever draws it, so a level more
//! or less, a click or the pointer moves no line; names are placed after, keeping off
//! the lines, and off the last stretch of each line into their own dot
//! ([`arrival`]).

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bevy_egui::egui;

use crate::{
    colors::{
        ACROSS_STRENGTH, CALL_STRENGTH, CURSOR, DIMMED_STRENGTH, FOLDED_STRENGTH, FURTHER,
        GUESSED_STRENGTH, INNER_STRENGTH, MAP_STRENGTH, OUTSIDE_STRENGTH, PICKED_STRENGTH, PILL,
        egui_color,
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

/// A call's line, in points; one on the cursor's path or out of the function set
/// aside, a little wider; the one into the cursor widest; a call among the expanded
/// module's functions, thinner.
pub(crate) const CALL_WIDTH: f32 = 1.5;
pub(crate) const BRIGHT_WIDTH: f32 = 2.0;
pub(crate) const LAST_WIDTH: f32 = 2.5;
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
/// How much of its strength a line keeps across a dot it runs under, so the dot
/// shows over it; and how near a dot's edge, or its ring's, a line passing it runs
/// under it, in points.
const UNDER_DOT: f32 = 0.15;
const GRAZE: f32 = 2.0;
/// The last stretch of a line into its callee's dot, in points: no name of that dot
/// lies over it, so the head shows and the name does not read as where the call
/// comes from.
pub(crate) const ARRIVAL: f32 = 30.0;
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

/// One end of a call's line: the middle of its dot (or sphere), and how far from it
/// the line stops.
type End = (egui::Pos2, f32);

/// Where things show on screen this frame, as the calls are drawn among them.
struct Screen<'a> {
    explorer: &'a Explorer,
    camera: &'a Camera,
    view: &'a GlobalTransform,
    /// Every sphere shown, by cluster: its middle and radius on screen.
    spheres: Vec<(usize, egui::Pos2, f32)>,
    /// `spheres` by where they show.
    sphere_cells: Cells,
    /// Every dot drawn bigger than a speck, by tile: its middle and radius on screen,
    /// and how far from its middle a line passing it runs under it.
    dots: Vec<Passed>,
}

/// A dot drawn, as calls passing it see it.
#[derive(Debug, Clone, Copy)]
struct Passed {
    tile: usize,
    at: egui::Pos2,
    radius: f32,
    reach: f32,
}

impl<'a> Screen<'a> {
    fn new(explorer: &'a Explorer, camera: &'a Camera, view: &'a GlobalTransform) -> Self {
        let mut screen = Self {
            explorer,
            camera,
            view,
            spheres: Vec::new(),
            sphere_cells: Cells::default(),
            dots: Vec::new(),
        };
        let nucleus = explorer.nucleus();
        screen.spheres = nucleus
            .balls()
            .filter_map(|(cluster, ball)| {
                let (at, radius) = screen.disc(ball.center, ball.radius)?;
                Some((cluster, at, radius))
            })
            .collect();
        screen.sphere_cells = Cells::new(&screen.spheres);
        // A ring round a dot is part of it: the start's, the cursor's, the one set
        // aside's, a call's that calls back. Not the pointer's, which comes and goes.
        let ringed = |tile: usize| {
            [nucleus.start(), explorer.cursor(), explorer.aside()].contains(&Some(tile))
                || nucleus.calls_back(tile)
        };
        // A speck is never in a line's way: the line runs over it.
        screen.dots = (0..explorer.map().tiles.len())
            .filter(|&tile| nucleus.drawn(tile) && !nucleus.speck(tile))
            .filter_map(|tile| {
                let world = explorer.tile_point(tile)?;
                let (at, radius) =
                    screen.disc(world, drawn_radius(explorer, tile, camera, view))?;
                let reach = if ringed(tile) { radius * RING } else { radius };
                Some(Passed {
                    tile,
                    at,
                    radius,
                    reach,
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
    /// its callers, its calls at any depth): what its lines end at as big as the
    /// cursor's whatever it is now, so moving the cursor moves no line.
    fn laid_out(&self, tile: usize) -> bool {
        let nucleus = self.explorer.nucleus();
        if nucleus.browsing() {
            return nucleus.drawn(tile) && !nucleus.speck(tile);
        }
        nucleus.start() == Some(tile)
            || nucleus.journey_level(tile).is_some()
            || (nucleus.level(tile) == Some(-1) && !nucleus.folded(tile) && !nucleus.outside(tile))
    }

    /// Where a call's line runs between, and how far short of each end it stops: its
    /// caller's dot and its callee's, a function the journey lays out as big as the
    /// cursor's where `steady` (else as drawn), or its module's sphere where the
    /// function there is folded away.
    fn ends(&self, (from, to): (usize, usize), steady: bool) -> Option<[End; 2]> {
        let nucleus = self.explorer.nucleus();
        let graph = self.explorer.graph();
        let end = |tile: usize| {
            let (at, radius) = if steady && self.laid_out(tile) && !nucleus.browsing() {
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

    /// The stretches of the line from `from` to `to`, as spans of the share of it,
    /// that cross a sphere other than `own`: where a line between modules hides.
    fn in_spheres(
        &self,
        (from, to): (egui::Pos2, egui::Pos2),
        own: [Option<usize>; 2],
    ) -> Vec<(f32, f32)> {
        (self
            .sphere_cells
            .along(from, to, self.spheres.len())
            .into_iter())
        .map(|index| self.spheres[index])
        .filter(|(c, ..)| !own.contains(&Some(*c)))
        .filter_map(|(_, at, radius)| across(from, to, at, radius))
        .collect()
    }

    /// The dots drawn bigger than a speck that the line through `path` runs under,
    /// or grazes, but its own ends' `ends`: each shows over it.
    fn under(&self, path: &[egui::Pos2], ends: [usize; 2]) -> Vec<Passed> {
        let bounds = egui::Rect::from_points(path);
        let reach = |d: &Passed| d.reach + GRAZE;
        self.dots
            .iter()
            .filter(|d| !ends.contains(&d.tile) && bounds.distance_to_pos(d.at) < reach(d))
            .filter(|d| {
                path.windows(2)
                    .any(|p| across(p[0], p[1], d.at, reach(d)).is_some())
            })
            .copied()
            .collect()
    }
}

/// Spheres on screen filed by the square cells they cover, so a line finds those it
/// may cross without trying every one; one covering too many cells is tried by
/// every line.
#[derive(Debug, Default)]
struct Cells {
    cells: HashMap<(i32, i32), Vec<usize>>,
    big: Vec<usize>,
}

impl Cells {
    /// A cell's side, in points; and the most cells a sphere is filed in.
    const SIDE: f32 = 16.0;
    const MOST: i32 = 256;

    #[expect(clippy::cast_possible_truncation, reason = "a cell index, clamped")]
    fn cell(at: f32) -> i32 {
        (at / Self::SIDE).floor().clamp(-1e6, 1e6) as i32
    }

    fn new(spheres: &[(usize, egui::Pos2, f32)]) -> Self {
        let mut cells = Self::default();
        for (index, &(_, at, radius)) in spheres.iter().enumerate() {
            let reach = radius + 1.0;
            let (x0, x1) = (Self::cell(at.x - reach), Self::cell(at.x + reach));
            let (y0, y1) = (Self::cell(at.y - reach), Self::cell(at.y + reach));
            if (x1 - x0 + 1) * (y1 - y0 + 1) > Self::MOST {
                cells.big.push(index);
                continue;
            }
            for x in x0..=x1 {
                for y in y0..=y1 {
                    cells.cells.entry((x, y)).or_default().push(index);
                }
            }
        }
        cells
    }

    /// The spheres, by index, filed in any cell the line from `from` to `to` passes
    /// through: every one it crosses, and some it does not, each once, in order; all
    /// `count` of them when the line passes through more cells than that.
    fn along(&self, from: egui::Pos2, to: egui::Pos2, count: usize) -> Vec<usize> {
        let (a, b) = if from.x <= to.x {
            (from, to)
        } else {
            (to, from)
        };
        let (c0, c1) = (Self::cell(a.x), Self::cell(b.x));
        let rows = (Self::cell(b.y) - Self::cell(a.y)).unsigned_abs() as usize;
        let columns = (c1 - c0).unsigned_abs() as usize + 1;
        if rows + 3 * columns > count {
            return (0..count).collect();
        }
        let wide = b.x - a.x;
        let y_at = |x: f32| {
            if wide <= f32::EPSILON {
                a.y
            } else {
                a.y + (b.y - a.y) * ((x - a.x) / wide).clamp(0.0, 1.0)
            }
        };
        let mut found = self.big.clone();
        for column in c0..=c1 {
            #[expect(clippy::cast_precision_loss, reason = "a cell index")]
            let left = column as f32 * Self::SIDE;
            let (x0, x1) = (left.max(a.x), (left + Self::SIDE).min(b.x));
            let (ya, yb) = if wide <= f32::EPSILON {
                (a.y, b.y)
            } else {
                (y_at(x0), y_at(x1))
            };
            let (low, high) = (ya.min(yb) - 1.0, ya.max(yb) + 1.0);
            for row in Self::cell(low)..=Self::cell(high) {
                if let Some(list) = self.cells.get(&(column, row)) {
                    found.extend(list);
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }
}

/// How a function's dot is drawn now.
fn graph_dot(explorer: &Explorer, tile: usize) -> Dot {
    crate::graph_view::dot(explorer, tile)
}

/// A call's line as worked out: from its caller's dot to just short of its callee's,
/// and whether names keep off it.
#[derive(Debug, Clone)]
struct Route {
    path: Vec<egui::Pos2>,
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

    /// The outside callers shown whose call has no line: none runs clear of the
    /// journey's lines, or the dots are too near for a head.
    pub(crate) fn lineless(&self, nucleus: &Nucleus) -> HashSet<usize> {
        nucleus
            .links()
            .iter()
            .filter(|l| l.flow == Flow::Outside && nucleus.outside(l.from))
            .filter(|l| !self.lines.contains_key(&(l.from, l.to)))
            .map(|l| l.from)
            .collect()
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

/// Works out where every call runs, as [`Routes`] says: each one straight between
/// its ends.
pub(crate) fn route(explorer: &Explorer, (camera, view): (&Camera, &GlobalTransform)) -> Routes {
    let screen = Screen::new(explorer, camera, view);
    let nucleus = explorer.nucleus();
    // Each call once, and whether names keep off it: not the calls from outside or
    // among the expanded module's own, which come and go.
    let mut calls: HashMap<(usize, usize), bool> = HashMap::new();
    for call in nucleus.every_call() {
        calls.insert(call, true);
    }
    for link in nucleus.links() {
        calls.entry((link.from, link.to)).or_insert(false);
    }
    let lines = calls
        .into_iter()
        .filter_map(|(pair, kept)| {
            // Two dots too near for that take their line from the dots as drawn.
            let path = straight(screen.ends(pair, true)?)
                .or_else(|| straight(screen.ends(pair, false)?))?;
            Some((pair, Route { path, kept }))
        })
        .collect();
    Routes { lines }
}

/// A call's straight line between `ends`, on the line through their middles, each
/// end's reach short of it; two dots too near for that keep a smaller gap, so the
/// call still shows. `None` where its ends are too near for a head.
fn straight([(a, ra), (b, rb)]: [End; 2]) -> Option<Vec<egui::Pos2>> {
    let along = (b - a).normalized();
    let short = a.distance(b) - ra - rb <= SHORT_HEAD.0 + 2.0;
    let (ra, rb) = if short {
        (ra - GAP + SHORT_GAP, rb - GAP + SHORT_GAP)
    } else {
        (ra, rb)
    };
    let path = vec![a + along * ra, b - along * rb];
    ((path[1] - path[0]).dot(along) > SHORT_HEAD.0).then_some(path)
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

/// How a line is drawn: its width and color, and its head's length and width.
#[derive(Debug, Clone, Copy)]
struct Ink {
    width: f32,
    color: Color,
    head: (f32, f32),
}

/// An arrow along `path`, with a filled head at its end, into `shapes`: its line
/// whole, but where `muted` has it (spans of the share of each of its pieces, and how
/// much of its strength it keeps there), fainter, or hidden where it keeps none.
/// Returns how it shows.
fn paint(
    shapes: &mut Vec<egui::Shape>,
    path: &[egui::Pos2],
    ink: Ink,
    muted: &dyn Fn(egui::Pos2, egui::Pos2) -> Vec<(f32, f32, f32)>,
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
    let color = egui_color(ink.color);
    // Where the line does not show, as how far along it from its start.
    let mut gaps: Vec<[f32; 2]> = Vec::new();
    let mut run = 0.0;
    for piece in shaft.windows(2) {
        let (from, until) = (piece[0], piece[1]);
        let long = from.distance(until);
        let spans = muted(from, until);
        let mut cuts: Vec<f32> = spans
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
            let keep = spans
                .iter()
                .filter(|&&(a, b, _)| a <= middle && middle <= b)
                .map(|&(.., keep)| keep)
                .fold(1.0, f32::min);
            if keep <= 0.0 {
                let (s0, s1) = (run + t0 * long, run + t1 * long);
                match gaps.last_mut() {
                    Some(gap) if gap[1] >= s0 - 0.01 => gap[1] = s1,
                    _ => gaps.push([s0, s1]),
                }
                continue;
            }
            shapes.push(egui::Shape::line_segment(
                [from + (until - from) * t0, from + (until - from) * t1],
                egui::Stroke::new(ink.width, color.gamma_multiply(keep)),
            ));
        }
        run += long;
    }
    let half = head.1 / 2.0;
    shapes.push(egui::Shape::convex_polygon(
        vec![to, back + across_head * half, back - across_head * half],
        color,
        egui::Stroke::NONE,
    ));
    ArrowShape {
        width: ink.width,
        head: head.0,
        head_width: head.1,
        tip: Vec2::new(to.x, to.y),
        path: path.iter().map(|p| Vec2::new(p.x, p.y)).collect(),
        gaps,
        under: Vec::new(),
    }
}

/// Every call the nucleus names along its line in `routes`, and browsing with
/// nothing selected every line between modules, among the `names` placed; then where
/// calls leave the free area, the functions at their far ends. Noted in `marks`; returns
/// what to paint. The calls are drawn the weakest first, so the ones followed lie
/// over the rest: the cursor's path, the one into the cursor last.
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
    let spine = Spine::of(explorer);
    draw_map(&mut shapes, &screen, nucleus, marks);
    // A call is drawn only between two dots drawn, or the sphere a caller of the
    // start is folded away in: never from or to a dot that is not there, as a caller
    // from outside with no room to be drawn.
    let shown = |link: &Link, tile: usize| {
        nucleus.drawn(tile) || (link.flow != Flow::Outside && !nucleus.placed(tile))
    };
    let mut order: Vec<(Weight, &Link)> = nucleus
        .links()
        .iter()
        .filter(|link| shown(link, link.from) && shown(link, link.to))
        .map(|link| (weight(explorer, &spine, link), link))
        .collect();
    order.sort_by_key(|&(weight, _)| weight);
    let mut drawn = Vec::new();
    for (weight, link) in order {
        let Some(route) = routes.lines.get(&(link.from, link.to)) else {
            continue;
        };
        let path = &route.path;
        // A lit call crossing the view with neither end in it is faint: no label
        // says what it joins.
        let passing = link.flow != Flow::Inner
            && !names.area.contains(path[0])
            && !names.area.contains(path[path.len() - 1]);
        let weight = if passing {
            weight.min(Weight::Plain)
        } else {
            weight
        };
        let color = link_color(explorer, link, weight);
        let color = if passing {
            color.with_alpha(color.alpha().min(ACROSS_STRENGTH))
        } else {
            color
        };
        let (width, head) = match weight {
            Weight::Last => (LAST_WIDTH, HEAD),
            Weight::Path | Weight::Followed => (BRIGHT_WIDTH, HEAD),
            Weight::Inner => (INNER_WIDTH, MAP_HEAD),
            Weight::Dimmed | Weight::Plain => (CALL_WIDTH, HEAD),
        };
        let ink = Ink { width, color, head };
        // Across each dot it runs under, the dot shows through it.
        let under = screen.under(path, [link.from, link.to]);
        let mut shape = paint(&mut shapes, path, ink, &|from, to| {
            (under.iter())
                .filter_map(|d| across(from, to, d.at, d.radius))
                .map(|(t0, t1)| (t0, t1, UNDER_DOT))
                .collect()
        });
        shape.under = under.iter().map(|d| d.tile).collect();
        drawn.push((*link, path.clone()));
        marks.calls.push(CallMark {
            from: link.from,
            to: link.to,
            flow: link.flow,
            color,
            shape,
        });
    }
    marks.exits = exits(
        painter,
        (explorer, &screen, names),
        (&spine, &drawn),
        &mut shapes,
    );
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
    (spine, drawn): (&Spine, &[(Link, Vec<egui::Pos2>)]),
    shapes: &mut Vec<egui::Shape>,
) -> Vec<ExitMark> {
    let nucleus = explorer.nucleus();
    let area = names.area;
    let mut marks: Vec<ExitMark> = Vec::new();
    let mut placed: Vec<egui::Rect> = Vec::new();
    for far in &leaving(explorer, area, drawn) {
        let text = exit_text(explorer, far);
        let (link, _, at, inward) = far.calls[0];
        // In the color of the calls it labels: pink where the cursor's path runs.
        let shade = if far.calls.iter().any(|c| spine.along(&c.0).is_some()) {
            CURSOR
        } else {
            link_color(explorer, &link, Weight::Plain)
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
                        .any(|d| rect.distance_to_pos(d.at) < d.radius + 1.0)
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
        let own = [Some(tube.from), Some(tube.to)];
        let ink = Ink {
            width: MAP_WIDTH,
            color: FURTHER.with_alpha(MAP_STRENGTH),
            head: MAP_HEAD,
        };
        let shape = paint(shapes, &[from, to], ink, &|from, to| {
            let hidden = screen.in_spheres((from, to), own);
            hidden.into_iter().map(|(t0, t1)| (t0, t1, 0.0)).collect()
        });
        marks.tubes.push(TubeMark { index, shape });
    }
}

/// The journey cursor's path down from the start, the spine a reader follows: each
/// call on it drawn pink, the last, into the cursor, widest; the cursor's own calls
/// in their role's color as ever; and once two levels or more show, every other
/// call of the journey and its dot dimmer, its name placed after theirs and the
/// callers'.
#[derive(Debug, Default)]
pub(crate) struct Spine {
    /// Each call on the path, caller and callee, from the start's own down to the
    /// one into the cursor.
    calls: Vec<(usize, usize)>,
    /// The functions along it, the start's and the cursor's among them.
    on: HashSet<usize>,
    /// The functions the cursor's calls, and the function set aside.
    kept: HashSet<usize>,
    cursor: Option<usize>,
    /// Whether the rest of the journey is drawn dimmer: two levels or more show.
    dims: bool,
}

impl Spine {
    pub(crate) fn of(explorer: &Explorer) -> Self {
        let disclosed = explorer.disclosure();
        let (Some(start), Some(cursor), Some(number)) = (
            explorer.start(),
            explorer.cursor(),
            explorer
                .cursor_span()
                .and_then(|span| disclosed.number(span)),
        ) else {
            return Self::default();
        };
        let by_number: HashMap<&str, usize> = disclosed
            .tiles()
            .iter()
            .flat_map(|(&tile, numbers)| numbers.iter().map(move |n| (n.as_str(), tile)))
            .collect();
        let mut spine = Self {
            on: HashSet::from([start, cursor]),
            cursor: Some(cursor),
            dims: disclosed.rows() > 2,
            ..Self::default()
        };
        // Each number down to the cursor's, `2`, `2.3`, `2.3.1`: the call it is.
        let ends = number.match_indices('.').map(|(at, _)| at);
        let mut from = start;
        for end in ends.chain([number.len()]).filter(|_| !number.is_empty()) {
            let Some(&to) = by_number.get(&number[..end]) else {
                break;
            };
            spine.calls.push((from, to));
            spine.on.insert(to);
            from = to;
        }
        let nucleus = explorer.nucleus();
        spine.kept = nucleus
            .links()
            .iter()
            .filter(|link| matches!(link.flow, Flow::Journey(_)) && link.from == cursor)
            .map(|link| link.to)
            .chain(explorer.aside())
            .collect();
        spine
    }

    /// Whether a call is on the path, and if so whether it is the last, into the
    /// cursor.
    pub(crate) fn along(&self, link: &Link) -> Option<bool> {
        if !matches!(link.flow, Flow::Journey(_)) {
            return None;
        }
        let at = self.calls.iter().position(|&c| c == (link.from, link.to))?;
        Some(at + 1 == self.calls.len())
    }

    /// Whether a call of the journey is drawn dimmer: off the path, and not the
    /// cursor's own.
    fn dims_call(&self, link: &Link) -> bool {
        self.dims
            && matches!(link.flow, Flow::Journey(_))
            && self.along(link).is_none()
            && self.cursor != Some(link.from)
    }

    /// Whether a function the journey calls is drawn dimmer, and named after the
    /// path, the cursor's calls and the callers: off the path, not one the cursor
    /// calls, nor set aside.
    pub(crate) fn dims(&self, nucleus: &Nucleus, tile: usize) -> bool {
        self.dims
            && nucleus.level(tile).is_some_and(|level| level > 0)
            && !self.on.contains(&tile)
            && !self.kept.contains(&tile)
    }
}

/// How strongly a call is drawn, the weakest first, as they are painted: one among
/// the expanded module's functions; one of the journey's off the cursor's path;
/// any other; one out of the function set aside; one on the cursor's path; and the
/// one into the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Weight {
    Inner,
    Dimmed,
    Plain,
    Followed,
    Path,
    Last,
}

/// How strongly `link` is drawn, as [`Weight`] has it. Pointing at a function
/// lights none: hovering only rings it.
fn weight(explorer: &Explorer, spine: &Spine, link: &Link) -> Weight {
    match spine.along(link) {
        Some(true) => return Weight::Last,
        Some(false) => return Weight::Path,
        None => {}
    }
    if link.flow == Flow::Inner {
        // Out of the function set aside too: its role's color, faint.
        Weight::Inner
    } else if explorer.aside() == Some(link.from) {
        Weight::Followed
    } else if spine.dims_call(link) {
        Weight::Dimmed
    } else {
        Weight::Plain
    }
}

/// A call's color: its flow's, pink along the cursor's path, guessed or not (the
/// path never breaks; its bar says it is guessed); faint for a call only guessed
/// off the path, a call among the expanded module's functions, one from off the
/// journey and a caller of the start folded into its module's count; dimmer for one of the
/// journey's off the path; the calls of a function set aside in their flow's color at
/// full strength, grey ones a little stronger.
fn link_color(explorer: &Explorer, link: &Link, weight: Weight) -> Color {
    let nucleus = explorer.nucleus();
    let color = link.flow.color();
    match link.flow {
        Flow::Outside if weight == Weight::Followed => color.with_alpha(PICKED_STRENGTH),
        Flow::Outside => color.with_alpha(OUTSIDE_STRENGTH),
        Flow::Inner => color.with_alpha(INNER_STRENGTH),
        _ if weight >= Weight::Path => CURSOR,
        _ if nucleus.guessed(link) => color.with_alpha(GUESSED_STRENGTH),
        Flow::Into if weight < Weight::Followed && nucleus.folded(link.from) => {
            color.with_alpha(FOLDED_STRENGTH)
        }
        _ if weight == Weight::Followed => color,
        _ if weight == Weight::Dimmed => color.with_alpha(DIMMED_STRENGTH),
        _ => color.with_alpha(CALL_STRENGTH),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cells_a_line_passes_through_hold_every_sphere_it_crosses() {
        // A deterministic scatter of spheres, small and big, and lines every way.
        let mut seed = 7_u32;
        let mut next = move |most: f32| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            #[expect(clippy::cast_precision_loss, reason = "a test's numbers")]
            let share = (seed >> 8) as f32 / (1 << 24) as f32;
            share * most
        };
        let spheres: Vec<(usize, egui::Pos2, f32)> = (0..400)
            .map(|i| {
                let radius = if i % 50 == 0 { 300.0 } else { next(40.0) };
                (i, egui::pos2(next(1600.0), next(1000.0)), radius)
            })
            .collect();
        let cells = Cells::new(&spheres);
        for _ in 0..2000 {
            let from = egui::pos2(next(1800.0) - 100.0, next(1200.0) - 100.0);
            let to = if next(1.0) < 0.2 {
                egui::pos2(from.x, next(1000.0))
            } else {
                egui::pos2(next(1800.0) - 100.0, next(1200.0) - 100.0)
            };
            let found = cells.along(from, to, spheres.len());
            for &(index, at, radius) in &spheres {
                if across(from, to, at, radius).is_some() {
                    assert!(found.contains(&index), "{from:?} {to:?} misses {index}");
                }
            }
        }
    }

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
    fn a_line_runs_straight_between_its_dots_short_of_each() {
        let (a, b) = (egui::pos2(0.0, 0.0), egui::pos2(200.0, 100.0));
        let path = straight([(a, 8.0), (b, 6.0)]).expect("long enough");
        assert_eq!(path.len(), 2, "one segment");
        assert!((path[0].distance(a) - 8.0).abs() < 1e-3);
        assert!((path[1].distance(b) - 6.0).abs() < 1e-3);
        let off = |p: egui::Pos2| ((p - a).x * (b - a).y - (p - a).y * (b - a).x).abs();
        assert!(
            off(path[0]) < 1e-2 && off(path[1]) < 1e-2,
            "on the line through both"
        );
        assert!(
            straight([(a, 8.0), (egui::pos2(14.0, 0.0), 8.0)]).is_none(),
            "too near for a head"
        );
    }

    #[test]
    fn a_line_shows_whole_but_where_it_is_hidden() {
        let path = [egui::pos2(0.0, 0.0), egui::pos2(100.0, 0.0)];
        let ink = Ink {
            width: CALL_WIDTH,
            color: CURSOR,
            head: HEAD,
        };
        let mut shapes = Vec::new();
        let whole = paint(&mut shapes, &path, ink, &|_, _| Vec::new());
        assert_eq!(whole.gaps.len(), 0);
        let hidden = paint(&mut shapes, &path, ink, &|_, _| vec![(0.4, 0.6, 0.0)]);
        let under = paint(&mut shapes, &path, ink, &|_, _| vec![(0.4, 0.6, UNDER_DOT)]);
        assert_eq!(under.gaps.len(), 0, "fainter, not broken");
        assert_eq!(hidden.gaps.len(), 1);
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
