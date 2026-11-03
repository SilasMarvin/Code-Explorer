//! The calls on the graph, drawn flat over the scene at one size on screen however
//! near the view: each call a line from its caller's dot to just short of its
//! callee's, with one filled head there, straight where nothing is in its way and
//! bent round a dot or a name it would otherwise run through; browsing with nothing
//! selected, a thin faint line from each module to every module it calls, behind the
//! spheres it passes. A call's line runs faint where it crosses a sphere it neither
//! leaves nor enters, and breaks round a dot only where no bend clears it, so it
//! never reads as passing through either.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::{
    colors::{
        CALL_STRENGTH, CURSOR, FOLDED_STRENGTH, FURTHER, GUESSED_STRENGTH, INNER_STRENGTH,
        MAP_STRENGTH, OUTSIDE_STRENGTH, PICKED_STRENGTH, PILL, egui_color,
    },
    drive::{ArrowShape, CallMark, ExitMark, Marks, TubeMark},
    explorer::Explorer,
    graph_view::drawn_radius,
    labels::{GraphNames, PILL_PAD, node_name},
    nucleus::{Flow, Link, Nucleus},
    text,
};

/// A call's line, in points; the one into the cursor or out of the function set
/// aside, a little wider; a call among the expanded module's functions, thinner.
const CALL_WIDTH: f32 = 1.5;
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
/// Between a line's end and the dot or sphere it ends at, in points.
const GAP: f32 = 2.5;
/// How much of its strength a line keeps where it crosses a sphere it does not touch.
const CROSSING: f32 = 0.3;
/// Kept clear round a dot a line passes without ending there, and round a name, in
/// points.
const DOT_CLEAR: f32 = 2.0;
const NAME_CLEAR: f32 = 1.0;
/// How far each of the two lines between the same two ends moves to its right when
/// calls go both ways, in points: far enough that their heads, side by side, do not
/// touch, so neither hides the other.
const BOTH_WAYS: f32 = 4.0;
/// How far a line may bow out to clear a dot or a name, in points, tried in turn from
/// the least; and against its length, at most.
const BENDS: [f32; 8] = [8.0, 14.0, 22.0, 32.0, 46.0, 64.0, 90.0, 128.0];
const MOST_BEND: f32 = 0.6;
/// Where along a bent line it bows out most: evenly, nearer its start, nearer its
/// end, and close by either: where along it each of its two pulls stands, as a share
/// of its length, and the share of the bend each takes.
const BOWS: [Bow; 5] = [
    [(1.0 / 3.0, 1.0), (2.0 / 3.0, 1.0)],
    [(1.0 / 3.0, 1.6), (2.0 / 3.0, 0.2)],
    [(1.0 / 3.0, 0.2), (2.0 / 3.0, 1.6)],
    [(0.1, 1.4), (0.4, 0.6)],
    [(0.6, 0.6), (0.9, 1.4)],
];
/// How far along the free area's edge a call's exit label may slide to find room,
/// in points, tried in turn; and how far inside the edge it sits.
const EXIT_SLIDES: [f32; 7] = [0.0, 24.0, -24.0, 48.0, -48.0, 72.0, -72.0];
const EXIT_IN: f32 = 3.0;
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

/// Where things show on screen this frame, as the calls are drawn among them.
struct Screen<'a> {
    explorer: &'a Explorer,
    camera: &'a Camera,
    view: &'a GlobalTransform,
    /// Every sphere shown, by cluster: its middle and radius on screen.
    spheres: Vec<(usize, egui::Pos2, f32)>,
    /// Every dot drawn full size, by tile: its middle and radius on screen.
    dots: Vec<(usize, egui::Pos2, f32)>,
    /// The names of the dots drawn full size: each pill, by tile.
    names: Vec<(egui::Rect, usize)>,
}

impl<'a> Screen<'a> {
    fn new(
        explorer: &'a Explorer,
        camera: &'a Camera,
        view: &'a GlobalTransform,
        names: &GraphNames,
    ) -> Self {
        let mut screen = Self {
            explorer,
            camera,
            view,
            spheres: Vec::new(),
            dots: Vec::new(),
            names: Vec::new(),
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
                let (at, radius) = screen.dot(tile)?;
                Some((tile, at, radius))
            })
            .collect();
        screen.names = names
            .pills()
            .filter(|&(_, tile)| nucleus.drawn(tile) && !nucleus.speck(tile))
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

    /// A function's dot as it shows.
    fn dot(&self, tile: usize) -> Option<(egui::Pos2, f32)> {
        let world = self.explorer.tile_point(tile)?;
        self.disc(
            world,
            drawn_radius(self.explorer, tile, self.camera, self.view),
        )
    }

    fn sphere(&self, cluster: usize) -> Option<(egui::Pos2, f32)> {
        self.spheres
            .iter()
            .find(|(c, ..)| *c == cluster)
            .map(|&(_, at, radius)| (at, radius))
    }

    /// Where a call's line runs between, and how far short of each end it stops: its
    /// caller's dot and its callee's, or its module's sphere where the function there
    /// is folded away; and the modules it leaves and enters.
    fn ends(&self, link: &Link) -> Option<([End; 2], [Option<usize>; 2])> {
        let nucleus = self.explorer.nucleus();
        let graph = self.explorer.graph();
        let modules = [graph.cluster(link.from), graph.cluster(link.to)];
        let end = |tile: usize, module: Option<usize>| {
            let (at, radius) = if nucleus.placed(tile) {
                self.dot(tile)?
            } else {
                self.sphere(module?)?
            };
            Some((at, radius + GAP))
        };
        Some((
            [end(link.from, modules[0])?, end(link.to, modules[1])?],
            modules,
        ))
    }

    /// The line of a call between `ends`, moved `side` points to its right: straight
    /// where it passes no dot or name but those of `tiles`, its ends; else bowed
    /// round them by the least bend that clears every dot, and every name too where
    /// one does; else straight. `None` where its ends are too near for a head.
    fn route(
        &self,
        ends: [End; 2],
        side: f32,
        tiles: [Option<usize>; 2],
        avoid: bool,
    ) -> Option<Vec<egui::Pos2>> {
        let [(a, ra), (b, rb)] = ends;
        let along = (b - a).normalized();
        let right = egui::vec2(-along.y, along.x);
        let straight = vec![a + along * ra + right * side, b - along * rb + right * side];
        if (straight[1] - straight[0]).dot(along) <= SHORT_HEAD.0 {
            return None;
        }
        if !avoid {
            return Some(straight);
        }
        let obstacles = self.obstacles(a, b, tiles);
        let blocked = |path: &[egui::Pos2]| blocked(path, &obstacles);
        let first = blocked(&straight);
        if first == (0, 0) {
            return Some(straight);
        }
        let length = a.distance(b);
        // A line both ways bows only to its own right, away from the other.
        let signs: &[f32] = if side > 0.0 { &[1.0] } else { &[1.0, -1.0] };
        let mut best = (first, straight);
        for &bend in BENDS.iter().filter(|&&bend| bend <= length * MOST_BEND) {
            for bow in &BOWS {
                for &sign in signs {
                    let pull = |(along, share): (f32, f32)| (along, sign * (bend * share + side));
                    let pulls = [pull(bow[0]), pull(bow[1])];
                    let Some(path) = bowed((a, ra), (b, rb), pulls) else {
                        continue;
                    };
                    let hits = blocked(&path);
                    if hits == (0, 0) {
                        return Some(path);
                    }
                    if hits < best.0 {
                        best = (hits, path);
                    }
                }
            }
        }
        Some(best.1)
    }

    /// The dots and names a line from `a` to `b` may come near, but those of its
    /// ends: each dot as far as lines keep off it, each name its pill and the room
    /// kept round it.
    fn obstacles(&self, a: egui::Pos2, b: egui::Pos2, tiles: [Option<usize>; 2]) -> Obstacles {
        let reach = egui::Rect::from_two_pos(a, b).expand(BENDS[BENDS.len() - 1] + DOT_CLEAR);
        let own = |tile: usize| tiles.contains(&Some(tile));
        Obstacles {
            dots: self
                .dots
                .iter()
                .filter(|&&(tile, at, _)| !own(tile) && reach.contains(at))
                .map(|&(_, at, radius)| (at, radius + DOT_CLEAR))
                .collect(),
            names: self
                .names
                .iter()
                .filter(|&&(pill, tile)| !own(tile) && reach.intersects(pill))
                .map(|&(pill, _)| pill.expand(NAME_CLEAR))
                .collect(),
        }
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

/// What a call's line keeps off: dots, as circles, and names, as rectangles.
struct Obstacles {
    dots: Vec<(egui::Pos2, f32)>,
    names: Vec<egui::Rect>,
}

/// How many dots and names in `obstacles` the line through `path` runs over.
fn blocked(path: &[egui::Pos2], obstacles: &Obstacles) -> (usize, usize) {
    let pieces = || path.windows(2).map(|p| (p[0], p[1]));
    let dots = obstacles
        .dots
        .iter()
        .filter(|&&(at, radius)| pieces().any(|(a, b)| across(a, b, at, radius).is_some()))
        .count();
    let names = obstacles
        .names
        .iter()
        .filter(|&&pill| pieces().any(|(a, b)| through(a, b, pill)))
        .count();
    (dots, names)
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
/// `crossings` has each of its pieces. Returns how it shows.
fn paint(
    painter: &egui::Painter,
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
            painter.line_segment(
                [from + (until - from) * t0, from + (until - from) * t1],
                egui::Stroke::new(ink.width, color),
            );
        }
    }
    let half = head.1 / 2.0;
    painter.add(egui::Shape::convex_polygon(
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

/// Every call the nucleus names, and browsing with nothing selected every line
/// between modules, drawn on `painter` among the `names` placed and noted in `marks`.
/// The calls followed are drawn last, over the rest: the journey's into the cursor,
/// and the calls of a function set aside.
pub(crate) fn draw(
    painter: &egui::Painter,
    explorer: &Explorer,
    (camera, view): (&Camera, &GlobalTransform),
    names: &GraphNames,
    marks: &mut Marks,
) {
    marks.calls.clear();
    marks.tubes.clear();
    marks.exits.clear();
    let screen = Screen::new(explorer, camera, view, names);
    let nucleus = explorer.nucleus();
    // Expanded in a journey, a call that leaves the free area is labelled where it
    // does with the function at its far end.
    let exits = nucleus.expanded() && !nucleus.browsing();
    draw_map(painter, &screen, nucleus, marks);
    let links = nucleus.links();
    let both_ways = |link: &Link| links.iter().any(|l| l.from == link.to && l.to == link.from);
    // The faint calls inside the module expanded first, the ones followed last.
    let mut order: Vec<(bool, &Link)> = links
        .iter()
        .map(|link| (followed(explorer, link), link))
        .collect();
    order.sort_by_key(|&(bright, link)| (bright, link.flow != Flow::Inner));
    for (bright, link) in order {
        let Some((ends, modules)) = screen.ends(link) else {
            continue;
        };
        let side = if both_ways(link) { BOTH_WAYS } else { 0.0 };
        let tiles = [Some(link.from), Some(link.to)];
        // The faint calls among an expanded module's functions go straight.
        let Some(path) = screen.route(ends, side, tiles, link.flow != Flow::Inner) else {
            continue;
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
        let shape = paint(painter, &path, ink, &|from, to| {
            screen.crossings((from, to), modules, tiles)
        });
        if exits
            && link.flow != Flow::Inner
            && let Some(exit) = exit_label(painter, (explorer, &screen, names), link, &path, marks)
        {
            marks.exits.push(exit);
        }
        marks.calls.push(CallMark {
            from: link.from,
            to: link.to,
            flow: link.flow,
            color,
            shape,
        });
    }
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

/// Expanded in a journey, the label where a call's line along `path` leaves the
/// free area: the function at its far end, its number and its module, on a pill
/// just inside the edge, slid along it clear of names, dots and the labels before.
fn exit_label(
    painter: &egui::Painter,
    (explorer, screen, names): (&Explorer, &Screen, &GraphNames),
    link: &Link,
    path: &[egui::Pos2],
    marks: &Marks,
) -> Option<ExitMark> {
    let (at, inward, outward) = leaves(names.area, path)?;
    let far = if outward { link.to } else { link.from };
    let number = explorer
        .disclosure()
        .tiles()
        .get(&far)
        .and_then(|numbers| numbers.first().cloned());
    let module = explorer
        .graph()
        .cluster(far)
        .map(|c| explorer.cluster_name(c).to_uppercase())
        .unwrap_or_default();
    let name = node_name(explorer, far);
    let called = number.map_or(name.clone(), |n| format!("{n} {name}"));
    let text = format!(
        "{} {called} · {module}",
        if outward { "to" } else { "from" }
    );
    let shade = egui_color(link_color(explorer, link, true).with_alpha(1.0));
    let galley =
        painter.layout_no_wrap(text.clone(), egui::FontId::proportional(text::SMALL), shade);
    let size = galley.size() + 2.0 * PILL_PAD;
    let along = egui::vec2(-inward.y, inward.x);
    let into = f32::midpoint(size.x * inward.x.abs(), size.y * inward.y.abs()) + EXIT_IN;
    let taken = |rect: egui::Rect| {
        names.pills().any(|(pill, _)| pill.intersects(rect))
            || marks.exits.iter().any(|e| e.rect.intersects(rect))
            || screen
                .dots
                .iter()
                .any(|&(_, dot, radius)| rect.distance_to_pos(dot) < radius + 1.0)
    };
    let rect = EXIT_SLIDES.iter().find_map(|&by| {
        let middle = at + inward * into + along * by;
        let rect = egui::Rect::from_center_size(middle, size);
        let rect = rect.translate(egui::vec2(
            (names.area.min.x - rect.min.x).max(0.0) - (rect.max.x - names.area.max.x).max(0.0),
            (names.area.min.y - rect.min.y).max(0.0) - (rect.max.y - names.area.max.y).max(0.0),
        ));
        (!taken(rect)).then_some(rect)
    })?;
    painter.rect_filled(rect, 4.0, PILL);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, shade),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.min + PILL_PAD, galley, shade);
    Some(ExitMark {
        from: link.from,
        to: link.to,
        far,
        text,
        rect,
        at: Vec2::new(at.x, at.y),
    })
}

/// Browsing with nothing selected, a thin faint line from each module's sphere to
/// every module's it calls, one width, behind the spheres it passes.
fn draw_map(painter: &egui::Painter, screen: &Screen, nucleus: &Nucleus, marks: &mut Marks) {
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
        let shape = paint(painter, &[from, to], ink, &|from, to| {
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
    into_cursor || explorer.aside().is_some_and(|aside| aside == link.from)
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
        Flow::Outside | Flow::Inner if bright => color.with_alpha(PICKED_STRENGTH),
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
        let obstacles = Obstacles {
            dots: vec![(egui::pos2(100.0, 0.0), 6.0)],
            names: Vec::new(),
        };
        assert_eq!(blocked(&[a, b], &obstacles), (1, 0));
        let path =
            bowed((a, 8.0), (b, 8.0), [(1.0 / 3.0, 22.0), (2.0 / 3.0, 22.0)]).expect("long enough");
        assert_eq!(blocked(&path, &obstacles), (0, 0));
        assert!((path[0].distance(a) - 8.0).abs() < 0.5);
        assert!((path[path.len() - 1].distance(b) - 8.0).abs() < 0.5);
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
