//! The calls on the graph, drawn flat over the scene at one size on screen however
//! near the view: each call a straight line from its caller's dot to just short of
//! its callee's, with one filled head there; browsing with nothing selected, a thin
//! faint line from each module to every module it calls. A line runs faint where it
//! crosses a sphere it neither leaves nor enters, and breaks round a dot it does not
//! end at, so it never reads as passing through either.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::{
    colors::{CURSOR, FURTHER, egui_color},
    drive::{ArrowShape, CallMark, Marks, TubeMark},
    explorer::Explorer,
    graph_view::drawn_radius,
    nucleus::{Flow, Link, Nucleus},
};

/// A call's line, in points; the one being followed, a little wider; a call among
/// the expanded module's functions, thinner.
const CALL_WIDTH: f32 = 1.5;
const BRIGHT_WIDTH: f32 = 2.0;
const INNER_WIDTH: f32 = 1.0;
/// A call's head: its length along the line and its width, in points.
const HEAD: (f32, f32) = (8.0, 7.0);
/// A line between modules, in points; one carrying [`MANY_CALLS`] or more, a little
/// wider. Its head is smaller than a call's.
const MAP_WIDTH: f32 = 1.0;
const MAP_WIDE: f32 = 1.6;
const MANY_CALLS: usize = 10;
const MAP_HEAD: (f32, f32) = (6.0, 5.0);
/// How strong every line between modules is: one, so the map is no hairball.
const MAP_ALPHA: f32 = 0.16;
/// Between a line's end and the dot or sphere it ends at, in points.
const GAP: f32 = 2.5;
/// How much of its strength a line keeps where it crosses a sphere it does not touch.
const CROSSING: f32 = 0.3;
/// Kept clear round a dot a line passes over without ending there, in points.
const DOT_CLEAR: f32 = 2.0;
/// How far apart, in points, the two lines between the same two ends run when calls
/// go both ways.
const BOTH_WAYS: f32 = 2.5;

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
                let (at, radius) = screen.dot(tile)?;
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

    /// Where a call's line runs: from its caller's dot to its callee's, each end on
    /// its dot's edge, or on its module's sphere when the function there is folded
    /// away; and the modules it leaves and enters.
    fn ends(&self, link: &Link) -> Option<(egui::Pos2, egui::Pos2, [Option<usize>; 2])> {
        let nucleus = self.explorer.nucleus();
        let graph = self.explorer.graph();
        let modules = [graph.cluster(link.from), graph.cluster(link.to)];
        let end = |tile: usize, module: Option<usize>| {
            if nucleus.placed(tile) {
                self.dot(tile)
            } else {
                self.sphere(module?)
            }
        };
        let (a, ra) = end(link.from, modules[0])?;
        let (b, rb) = end(link.to, modules[1])?;
        let along = (b - a).normalized();
        let (from, to) = (a + along * (ra + GAP), b - along * (rb + GAP));
        ((to - from).length() > HEAD.0 && (to - from).dot(along) > 0.0)
            .then_some((from, to, modules))
    }

    /// The stretches of the line from `from` to `to`, as shares of its length, that
    /// cross a sphere other than `own` or a dot other than its ends'.
    fn crossings(
        &self,
        from: egui::Pos2,
        to: egui::Pos2,
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

/// An arrow from `from` to `to`, `width` wide in `color`, with a filled head `head`
/// long and wide at `to`; strong where it crosses nothing, faint across a sphere,
/// broken round a dot, as `crossings` has it. Returns how it shows.
fn paint(
    painter: &egui::Painter,
    (from, to): (egui::Pos2, egui::Pos2),
    width: f32,
    color: Color,
    head: (f32, f32),
    crossings: &[(f32, f32, Crossing)],
) -> ArrowShape {
    let along = (to - from).normalized();
    let across = egui::vec2(-along.y, along.x);
    let back = to - along * head.0;
    let ink = egui_color(color);
    let faint = egui_color(color.with_alpha(color.alpha() * CROSSING));
    // The shaft, cut where what it crosses changes, up to the head's back.
    let shaft = (back - from).length() / (to - from).length();
    let mut cuts: Vec<f32> = crossings
        .iter()
        .flat_map(|&(t0, t1, _)| [t0, t1])
        .filter(|&t| t > 0.0 && t < shaft)
        .chain([0.0, shaft])
        .collect();
    cuts.sort_by(f32::total_cmp);
    cuts.dedup();
    for pair in cuts.windows(2) {
        let (t0, t1) = (pair[0], pair[1]);
        let middle = f32::midpoint(t0, t1);
        let over = |kind: Crossing| {
            crossings
                .iter()
                .any(|&(a, b, k)| k == kind && a <= middle && middle <= b)
        };
        if over(Crossing::Dot) {
            continue;
        }
        let stroke = egui::Stroke::new(width, if over(Crossing::Sphere) { faint } else { ink });
        painter.line_segment([from + (to - from) * t0, from + (to - from) * t1], stroke);
    }
    let half = head.1 / 2.0;
    painter.add(egui::Shape::convex_polygon(
        vec![to, back + across * half, back - across * half],
        ink,
        egui::Stroke::NONE,
    ));
    ArrowShape {
        width,
        head: head.0,
        head_width: head.1,
        tip: Vec2::new(to.x, to.y),
    }
}

/// Moves a line `by` points to its right, so two running opposite ways between the
/// same ends lie side by side.
fn beside(from: egui::Pos2, to: egui::Pos2, by: f32) -> (egui::Pos2, egui::Pos2) {
    let along = (to - from).normalized();
    let right = egui::vec2(-along.y, along.x) * by;
    (from + right, to + right)
}

/// Every call the nucleus names, and browsing with nothing selected every line
/// between modules, drawn on `painter` and noted in `marks`. The calls being followed
/// are drawn last, over the rest: the journey's into the cursor or into what the
/// pointer is on, and the calls in of a function set aside, or of an outside caller
/// or a folded caller of the start the pointer is on.
pub(crate) fn draw(
    painter: &egui::Painter,
    explorer: &Explorer,
    (camera, view): (&Camera, &GlobalTransform),
    marks: &mut Marks,
) {
    marks.calls.clear();
    marks.tubes.clear();
    let screen = Screen::new(explorer, camera, view);
    let nucleus = explorer.nucleus();
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
        let Some((from, to, modules)) = screen.ends(link) else {
            continue;
        };
        let (from, to) = if both_ways(link) {
            beside(from, to, BOTH_WAYS)
        } else {
            (from, to)
        };
        let crossings = screen.crossings(from, to, modules, [Some(link.from), Some(link.to)]);
        let color = link_color(explorer, link, bright);
        let (width, head) = match link.flow {
            _ if bright => (BRIGHT_WIDTH, HEAD),
            Flow::Inner => (INNER_WIDTH, MAP_HEAD),
            _ => (CALL_WIDTH, HEAD),
        };
        let shape = paint(painter, (from, to), width, color, head, &crossings);
        marks.calls.push(CallMark {
            from: link.from,
            to: link.to,
            flow: link.flow,
            color,
            shape,
        });
    }
}

/// Browsing with nothing selected, a thin faint line from each module's sphere to
/// every module's it calls.
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
        let crossings = screen.crossings(from, to, [Some(tube.from), Some(tube.to)], [None; 2]);
        let width = if tube.calls >= MANY_CALLS {
            MAP_WIDE
        } else {
            MAP_WIDTH
        };
        let color = FURTHER.with_alpha(MAP_ALPHA);
        let shape = paint(painter, (from, to), width, color, MAP_HEAD, &crossings);
        marks.tubes.push(TubeMark { index, shape });
    }
}

/// Whether a call is the one being followed, drawn bright over the rest.
fn followed(explorer: &Explorer, link: &Link) -> bool {
    let nucleus = explorer.nucleus();
    let hovered = explorer.hovered();
    let into = link.flow != Flow::Into
        && link.flow != Flow::Outside
        && (explorer.cursor() == Some(link.to) || hovered == Some(link.to));
    let pointed =
        (link.flow == Flow::Outside || nucleus.folded(link.from)) && hovered == Some(link.from);
    let aside = explorer.aside();
    into || pointed || (aside.is_some() && Some(link.from) == aside)
}

/// A call's color: its flow's, but the cursor's for the call into the cursor, and
/// lighter for the others being followed.
fn link_color(explorer: &Explorer, link: &Link, bright: bool) -> Color {
    let nucleus = explorer.nucleus();
    match link.flow {
        Flow::Outside if bright => Color::WHITE.with_alpha(0.9),
        Flow::Outside => link.flow.color().with_alpha(0.35),
        Flow::Inner if !bright => link.flow.color().with_alpha(0.28),
        // A caller of the start folded into its module's count: there, but quiet.
        Flow::Into if !bright && nucleus.folded(link.from) => link.flow.color().with_alpha(0.25),
        _ if bright && explorer.cursor() == Some(link.to) => CURSOR,
        _ if bright => link.flow.color().mix(&Color::WHITE, 0.35),
        _ => link.flow.color().with_alpha(0.85),
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
}
