//! Call order in depth, as time bridges. No node moves: each call the disclosed
//! journey makes is drawn as an arc from its caller to the function it calls, lifting
//! off the map towards the viewer, higher the later it runs, so the journey reads as a
//! stack of bridges, first low and last high. A function called twice gets two arcs.
//! The stack holds the calls the cursor steps through, one list at a time, and of a
//! long list the stretch around the cursor.
//!
//! Depth hardly shows head-on, so `Z` tilts the camera a little, from below, and height
//! reads as rising up the screen while the map stays nearly face on. An arc whose link
//! runs up or down the screen would rise along itself, so it leans sideways and opens
//! into a loop. The stack stands as high as the view allows. The cursor's arc is
//! bright and numbered, with a pier down to its link; the others are pale, numbered at
//! their tops where there is room.
//!
//! [`Chrome`] holds whether they show; [`Bridges`] eases them up and down.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::SpanId;

use crate::{camera::OrbitCamera, explorer::Explorer, graph_view::node_size, ui::Chrome};

/// How call order shows in depth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TimeMode {
    #[default]
    Off,
    Bridges,
}

impl TimeMode {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Off => Self::Bridges,
            Self::Bridges => Self::Off,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Bridges => "bridges",
        }
    }
}

/// The bridges as they stand.
#[derive(Resource, Debug, Default)]
pub(crate) struct Bridges {
    /// How far the arcs have risen: 0 flat on the map, 1 at full height.
    rise: f32,
    /// Whether they were on last frame, to notice the toggle.
    on: bool,
    /// The camera's angles before the bridges tilted it, to turn back to.
    before: Option<(f32, f32)>,
    /// Each arc's height as a share of the stack's, by call, eased towards where it
    /// stands, so arcs rise into place rather than jump as the cursor moves the stack.
    heights: HashMap<SpanId, f32>,
    /// Each arc's top, with its number, for the labels.
    pub(crate) tops: Vec<Top>,
}

/// Where an arc peaks, and what its label says.
#[derive(Debug, Clone)]
pub(crate) struct Top {
    pub(crate) at: Vec3,
    pub(crate) number: String,
    /// Where the arc leaves and lands.
    pub(crate) ends: (Vec3, Vec3),
    /// How strongly the arc draws, 1 for the cursor's.
    pub(crate) strength: f32,
    pub(crate) cursor: bool,
}

pub(crate) struct BridgesPlugin;

impl Plugin for BridgesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Bridges>()
            .init_gizmo_group::<ArcGizmos>()
            .init_gizmo_group::<CursorGizmos>()
            .add_systems(Startup, line_widths);
    }
}

/// The arcs.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct ArcGizmos;

/// The cursor's arc, drawn thicker.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct CursorGizmos;

fn line_widths(mut store: ResMut<GizmoConfigStore>) {
    let set = |line: &mut GizmoLineConfig, width: f32| {
        line.width = width;
        line.joints = GizmoLineJoint::Round(4);
    };
    set(&mut store.config_mut::<ArcGizmos>().0.line, 1.6);
    set(&mut store.config_mut::<CursorGizmos>().0.line, 3.5);
}

/// The arcs' color: neutral, so it says nothing of direction or module.
const LINE: Color = Color::srgb(0.86, 0.89, 0.95);

/// The camera's turn while the bridges show: square to the map side to side, tilted up
/// from below so that what lifts towards the viewer rises on screen.
const TILT: (f32, f32) = (0.0, 0.3);

/// How high the last call's arc stands at most, as a share of the camera's distance,
/// so the stack keeps its height on screen however far the view is zoomed out; lower
/// when that would take a top off screen.
const TOP: f32 = 0.45;

/// The first call's height as a share of the last's, so it still clears its link.
const FLOOR: f32 = 0.25;

/// How far an arc running up or down the screen leans to the right of its way for
/// each unit it lifts, so it opens into a loop rather than doubling back over its
/// link.
const LEAN: f32 = 0.6;

/// The calls under `root` in the order they run, depth first, each with its caller.
/// `children` lists the calls a call makes, in order.
pub(crate) fn calls_in_order<S: Copy>(root: S, children: impl Fn(S) -> Vec<S>) -> Vec<(S, S)> {
    let mut calls = Vec::new();
    let mut stack: Vec<(Option<S>, S)> = vec![(None, root)];
    while let Some((caller, call)) = stack.pop() {
        if let Some(caller) = caller {
            calls.push((caller, call));
        }
        stack.extend(children(call).into_iter().rev().map(|c| (Some(call), c)));
    }
    calls
}

/// How high call `index` of `count` stands, as a share of the last's: evenly up from
/// the floor, in the order they run.
pub(crate) fn height_share(index: usize, count: usize) -> f32 {
    if count < 2 {
        return 1.0;
    }
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let along = index as f32 / (count - 1) as f32;
    FLOOR + (1.0 - FLOOR) * along
}

/// How strongly call `index` draws with the cursor on call `cursor`: in full on it,
/// pale near it, paler further off, never gone; what is still to come fainter than
/// what has run, so the stack fills in as the cursor climbs. All pale when the cursor
/// is on none.
pub(crate) fn arc_strength(index: usize, cursor: Option<usize>) -> f32 {
    let Some(cursor) = cursor else {
        return 0.5;
    };
    let near = match index.abs_diff(cursor) {
        0 => return 1.0,
        1..=2 => 0.55,
        #[expect(clippy::cast_precision_loss, reason = "a small count")]
        away => (0.55 - (away - 2) as f32 * 0.04).max(0.25),
    };
    if index > cursor { near * 0.6 } else { near }
}

/// Points along an arc from `from` to `to` that peaks `lift` off the straight line
/// between them, halfway along, `segments` long.
pub(crate) fn arc(from: Vec3, to: Vec3, lift: Vec3, segments: usize) -> Vec<Vec3> {
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let steps = segments.max(1) as f32;
    (0..=segments.max(1))
        .map(|i| {
            #[expect(clippy::cast_precision_loss, reason = "a small count")]
            let t = i as f32 / steps;
            from.lerp(to, t) + lift * (t * std::f32::consts::PI).sin()
        })
        .collect()
}

/// Which way an arc from `from` to `to` lifts, per unit of height: towards the viewer
/// off the map, and leaning to the right of its way as much as it runs up or down the
/// screen, whose `right` is given. A link across the screen already shows its lift
/// as height; one up or down it would hide it, and leans level instead, so how high
/// an arc's top stands still says when it runs, and arcs over one link nest.
pub(crate) fn lift_direction(from: Vec3, to: Vec3, right: Vec3) -> Vec3 {
    let right = Vec3::new(right.x, right.y, 0.0).normalize_or_zero();
    let up = Vec3::Z.cross(right);
    let way = (to - from).normalize_or_zero();
    let side = way.cross(Vec3::Z).normalize_or_zero();
    Vec3::Z + side * LEAN * way.dot(up).abs()
}

/// One call to draw: its caller's node, its callee's, its span, its caller's span and
/// its number.
struct Call {
    from: usize,
    to: usize,
    span: SpanId,
    caller: SpanId,
    number: String,
}

/// The disclosed journey's calls in the order they run, between nodes the graph has.
fn journey_calls(explorer: &Explorer) -> Vec<Call> {
    let Some(journey) = explorer.journey() else {
        return Vec::new();
    };
    let disclosed = explorer.disclosure();
    let map = explorer.map();
    let shown = |span: SpanId| -> Vec<SpanId> {
        disclosed
            .children(span)
            .iter()
            .copied()
            .filter(|&c| disclosed.number(c).is_some())
            .collect()
    };
    let tile = |span: SpanId| explorer.unit_id(span).and_then(|u| map.tile_of(u));
    calls_in_order(journey.root(), shown)
        .into_iter()
        .filter_map(|(caller, call)| {
            Some(Call {
                from: tile(caller)?,
                to: tile(call)?,
                span: call,
                caller,
                number: disclosed.number(call)?.to_owned(),
            })
        })
        .collect()
}

/// The calls the cursor steps through: those its caller makes, or the root's when it
/// is on none, so the stack holds one list of calls at a time, however deep the
/// journey is open; stepping into a call shows the calls it makes. A cursor inside a
/// call that draws no arc of its own, as in a closure, counts as on that call.
fn in_step(explorer: &Explorer, calls: Vec<Call>) -> (Vec<Call>, Option<SpanId>) {
    let Some(journey) = explorer.journey() else {
        return (Vec::new(), None);
    };
    let on = std::iter::successors(explorer.selected(), |&s| journey.span(s)?.parent)
        .find(|&s| calls.iter().any(|c| c.span == s));
    let caller = on
        .and_then(|s| calls.iter().find(|c| c.span == s))
        .map_or(journey.root(), |c| c.caller);
    let mut calls: Vec<Call> = calls.into_iter().filter(|c| c.caller == caller).collect();
    // A long list shows the stretch around the cursor.
    let at = on.and_then(|s| calls.iter().position(|c| c.span == s));
    let shown = window(calls.len(), at, WINDOW);
    calls.truncate(shown.end);
    calls.drain(..shown.start);
    (calls, on)
}

/// The most calls the stack holds at once.
const WINDOW: usize = 15;

/// Which of `count` calls the stack holds, at most `size` of them: all when they fit,
/// else a run centered on the cursor's, or from the first when it is on none, kept
/// whole at either end.
pub(crate) fn window(count: usize, cursor: Option<usize>, size: usize) -> std::ops::Range<usize> {
    if count <= size {
        return 0..count;
    }
    let start = cursor
        .unwrap_or(0)
        .saturating_sub(size / 2)
        .min(count - size);
    start..start + size
}

/// Eases the arcs up while the bridges show and down when they don't, and tilts the
/// camera as they come on, back as they go.
pub(crate) fn rise(
    time: Res<Time>,
    chrome: Res<Chrome>,
    mut bridges: ResMut<Bridges>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let on = chrome.time == TimeMode::Bridges;
    if on != bridges.on {
        bridges.on = on;
        for mut camera in &mut cameras {
            if on {
                bridges.before = Some(camera.angles());
                camera.turn_to(TILT);
            } else if let Some(before) = bridges.before.take() {
                camera.turn_to(before);
            }
        }
    }
    let goal = if on { 1.0 } else { 0.0 };
    let ease = 1.0 - (-time.delta_secs() * 4.0).exp();
    bridges.rise += (goal - bridges.rise) * ease;
    if (goal - bridges.rise).abs() < 1e-3 {
        bridges.rise = goal;
    }
}

/// Draws each call of the disclosed journey as an arc from caller to callee, as high
/// as it is late, and notes where each peaks for its number.
pub(crate) fn draw_bridges(
    time: Res<Time>,
    explorer: Res<Explorer>,
    mut bridges: ResMut<Bridges>,
    cameras: Query<(&OrbitCamera, &Transform, &Projection)>,
    mut arcs: Gizmos<ArcGizmos>,
    mut bright: Gizmos<CursorGizmos>,
) {
    bridges.tops.clear();
    if bridges.rise <= 0.0 {
        return;
    }
    let Ok((camera, view, projection)) = cameras.single() else {
        return;
    };
    let (calls, on) = in_step(&explorer, journey_calls(&explorer));
    let graph = explorer.graph();
    let right = view.right().as_vec3();
    let ease = 1.0 - (-time.delta_secs() * 6.0).exp();
    let heights = &mut bridges.heights;
    heights.retain(|span, _| calls.iter().any(|c| c.span == *span));
    for (index, call) in calls.iter().enumerate() {
        let height = heights.entry(call.span).or_default();
        *height += (height_share(index, calls.len()) - *height) * ease;
    }
    // Each call's ends, from the caller's surface to the callee's, and which way and
    // how far it lifts per unit of the stack's height.
    let spans: Vec<Option<(Vec3, Vec3, Vec3)>> = calls
        .iter()
        .map(|call| {
            let (a, b) = (
                explorer.tile_point(call.from)?,
                explorer.tile_point(call.to)?,
            );
            if a.distance(b) < 1e-3 {
                return None;
            }
            let along = (b - a).normalize_or_zero();
            let from = a + along * node_size(graph, call.from) * 0.5;
            let to = b - along * node_size(graph, call.to) * 0.5;
            let lift = lift_direction(a, b, right) * bridges.heights[&call.span];
            Some((from, to, lift))
        })
        .collect();
    let tan_half = match projection {
        Projection::Perspective(perspective) => (perspective.fov / 2.0).tan(),
        _ => 0.4,
    };
    let eye = Eye {
        at: view.translation,
        up: view.up().as_vec3(),
        forward: view.forward().as_vec3(),
        tan_half,
    };
    let tops: Vec<(Vec3, Vec3)> = spans
        .iter()
        .flatten()
        .map(|&(from, to, lift)| (from.lerp(to, 0.5), lift))
        .collect();
    let top = fitting_top(&eye, &tops, camera.distance * TOP) * ease_in_out(bridges.rise);
    let cursor = on.and_then(|s| calls.iter().position(|c| c.span == s));
    let forward = eye.forward;
    let fade = bridges.rise.min(1.0);
    for (index, (call, span)) in calls.iter().zip(&spans).enumerate() {
        let Some((from, to, lift)) = *span else {
            continue;
        };
        let lift = lift * top;
        let points = arc(from, to, lift, 32);
        let strength = arc_strength(index, cursor);
        let is_cursor = cursor == Some(index);
        let color = if is_cursor {
            Color::WHITE.with_alpha(fade)
        } else {
            LINE.with_alpha(strength * 0.8 * fade)
        };
        let head = camera.distance * if is_cursor { 0.014 } else { 0.009 };
        if is_cursor {
            bright.linestrip(points.iter().copied(), color);
            arrowhead(&mut bright, &points, forward, head, color);
            // A pier from its top down to its link, so its height reads as a length.
            let pier = color.with_alpha(0.45 * fade);
            dotted(
                &mut arcs,
                from.lerp(to, 0.5) + lift,
                from.lerp(to, 0.5),
                head * 0.5,
                pier,
            );
        } else {
            // Faint where it leaves the caller, where a busy caller's arcs crowd, and
            // whole from its top down to where it lands.
            #[expect(clippy::cast_precision_loss, reason = "a small count")]
            let last = (points.len() - 1).max(1) as f32;
            arcs.linestrip_gradient(points.iter().enumerate().map(|(i, &p)| {
                #[expect(clippy::cast_precision_loss, reason = "a small count")]
                let along = i as f32 / last;
                (p, color.with_alpha(color.alpha() * emerging(along)))
            }));
            arrowhead(&mut arcs, &points, forward, head, color);
        }
        bridges.tops.push(Top {
            at: from.lerp(to, 0.5) + lift,
            number: call.number.clone(),
            ends: (from, to),
            strength,
            cursor: is_cursor,
        });
    }
}

/// Where a camera looks from and how wide it sees.
pub(crate) struct Eye {
    pub(crate) at: Vec3,
    pub(crate) up: Vec3,
    pub(crate) forward: Vec3,
    /// The tangent of half the vertical field of view.
    pub(crate) tan_half: f32,
}

/// The share of the half-view above the middle an arc's top may reach.
const ROOM: f32 = 0.82;

/// How high the stack may stand, at most `most`, so that every arc's top stays on
/// screen: each given by the middle of its link and which way it lifts per unit of
/// height. Arcs whose link is already off screen don't hold it down.
pub(crate) fn fitting_top(view: &Eye, arcs: &[(Vec3, Vec3)], most: f32) -> f32 {
    let limit = ROOM * view.tan_half;
    arcs.iter()
        .filter_map(|&(middle, lift)| {
            let offset = middle - view.at;
            let (high, deep) = (offset.dot(view.up), offset.dot(view.forward));
            if deep <= 0.0 || high > limit * deep {
                return None;
            }
            let (rising, nearing) = (lift.dot(view.up), lift.dot(view.forward));
            // On screen while high + t·rising ≤ limit·(deep + t·nearing), and well in
            // front of the camera.
            let climb = rising - limit * nearing;
            let fits = if climb > 0.0 {
                (limit * deep - high) / climb
            } else {
                f32::INFINITY
            };
            let near = if nearing < 0.0 {
                deep * 0.5 / -nearing
            } else {
                f32::INFINITY
            };
            Some(fits.min(near))
        })
        .fold(most, f32::min)
}

/// How much of an arc's strength shows `along` it, 0 at the caller to 1 at the
/// callee: a fifth where it leaves, rising to all of it by its top.
pub(crate) fn emerging(along: f32) -> f32 {
    0.2 + 0.8 * ease_in_out(along * 2.0)
}

/// Slow at both ends, so the arcs lift off and settle gently.
fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A chevron where an arc lands, pointing along it, facing a camera looking along
/// `forward`.
fn arrowhead<T: GizmoConfigGroup>(
    gizmos: &mut Gizmos<T>,
    points: &[Vec3],
    forward: Vec3,
    size: f32,
    color: Color,
) {
    let [.., before, tip] = points else {
        return;
    };
    let along = (*tip - *before).normalize_or_zero();
    let across = along.cross(forward).normalize_or_zero();
    let back = *tip - along * size;
    gizmos.line(back + across * size * 0.6, *tip, color);
    gizmos.line(back - across * size * 0.6, *tip, color);
}

/// A line of short dashes, about `dash` long.
fn dotted<T: GizmoConfigGroup>(gizmos: &mut Gizmos<T>, a: Vec3, b: Vec3, dash: f32, color: Color) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a small positive count"
    )]
    let dashes = (a.distance(b) / dash.max(1e-3)).clamp(1.0, 200.0) as usize;
    for i in (0..dashes).step_by(2) {
        #[expect(clippy::cast_precision_loss, reason = "a small count")]
        let (t0, t1) = (i as f32 / dashes as f32, (i + 1) as f32 / dashes as f32);
        gizmos.line(a.lerp(b, t0), a.lerp(b, t1), color);
    }
}

/// Whether the bridges are up far enough that the map names only what the journey
/// touches, so their numbers read.
pub(crate) fn up(bridges: &Bridges) -> bool {
    bridges.rise > 0.5
}

/// How strongly the ordinary graph's links draw: the bridges fade them back as they
/// rise.
pub(crate) fn link_strength(bridges: &Bridges) -> f32 {
    1.0 - 0.45 * bridges.rise
}

/// The arcs' numbers at their tops: the cursor's always, bright on a dark pill, the
/// others pale where they clear every number already placed, nearest the cursor first.
pub(crate) fn draw_numbers(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    tops: &[Top],
) {
    let mut order: Vec<&Top> = tops.iter().collect();
    order.sort_by(|a, b| b.strength.total_cmp(&a.strength));
    let mut placed: Vec<egui::Rect> = Vec::new();
    for top in order {
        let Some(at) = to_screen(top.at) else {
            continue;
        };
        // An arc too low to clear its nodes leaves its number to theirs.
        let (from, to) = (to_screen(top.ends.0), to_screen(top.ends.1));
        if [from, to]
            .into_iter()
            .flatten()
            .any(|end| end.distance(at) < LOW)
        {
            continue;
        }
        let size = if top.cursor { 13.0 } else { 11.0 };
        let text = if top.cursor {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_gray(200).gamma_multiply(top.strength.max(0.6))
        };
        let galley =
            painter.layout_no_wrap(top.number.clone(), egui::FontId::proportional(size), text);
        // Just above the top, clear of the line.
        let rect = egui::Rect::from_center_size(
            at - egui::vec2(0.0, galley.size().y * 0.5 + 4.0),
            galley.size() + egui::vec2(8.0, 2.0),
        );
        if !top.cursor && !clear_of(rect, &placed) {
            continue;
        }
        placed.push(rect);
        let fill = egui::Color32::from_rgba_unmultiplied(14, 16, 22, 210);
        painter.rect_filled(rect, 4.0, fill);
        if top.cursor {
            painter.rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.0, egui::Color32::from_white_alpha(170)),
                egui::StrokeKind::Outside,
            );
        }
        painter.galley(rect.center() - galley.size() * 0.5, galley, text);
    }
}

/// How near an arc's top may come to either of its nodes on screen, in points, and
/// still be numbered.
const LOW: f32 = 22.0;

/// Whether `rect` keeps a little off every one of `placed`.
pub(crate) fn clear_of(rect: egui::Rect, placed: &[egui::Rect]) -> bool {
    let room = rect.expand(2.0);
    placed.iter().all(|p| !p.intersects(room))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_run_depth_first_each_with_its_caller() {
        // 0 calls 1 then 4; 1 calls 2 then 3; 4 calls 2 again.
        let tree = |n: u32| match n {
            0 => vec![1, 4],
            1 => vec![2, 3],
            4 => vec![2],
            _ => vec![],
        };
        assert_eq!(
            calls_in_order(0, tree),
            vec![(0, 1), (1, 2), (1, 3), (0, 4), (4, 2)]
        );
    }

    #[test]
    fn later_calls_stand_higher() {
        let heights: Vec<f32> = (0..19).map(|i| height_share(i, 19)).collect();
        assert!(heights.windows(2).all(|w| w[1] > w[0]));
        assert!(
            (heights[0] - FLOOR).abs() < 1e-6,
            "the first clears its link"
        );
        assert!(
            (heights[18] - 1.0).abs() < 1e-6,
            "the last stands at the top"
        );
        assert!(
            (height_share(0, 1) - 1.0).abs() < 1e-6,
            "a lone call at the top"
        );
    }

    #[test]
    fn the_cursor_draws_brightest_and_nothing_vanishes() {
        assert!((arc_strength(5, Some(5)) - 1.0).abs() < 1e-6);
        assert!(arc_strength(4, Some(5)) < 1.0);
        assert!(arc_strength(12, Some(5)) < arc_strength(6, Some(5)));
        assert!(arc_strength(400, Some(5)) > 0.0);
        assert!(
            arc_strength(6, Some(5)) < arc_strength(4, Some(5)),
            "to come, fainter"
        );
        assert!(arc_strength(3, None) < 1.0, "no cursor, none bright");
    }

    #[test]
    fn an_arc_runs_end_to_end_and_peaks_halfway() {
        let (from, to, lift) = (Vec3::ZERO, Vec3::X * 10.0, Vec3::Z * 4.0);
        let points = arc(from, to, lift, 32);
        assert!(points[0].distance(from) < 1e-5);
        assert!(points[32].distance(to) < 1e-5);
        assert!(points[16].distance(Vec3::new(5.0, 0.0, 4.0)) < 1e-4);
        assert!(points.iter().all(|p| p.z <= 4.0 + 1e-5));
    }

    #[test]
    fn arcs_lift_towards_the_viewer_and_lean_right_of_their_way() {
        let lift = lift_direction(Vec3::ZERO, Vec3::Y, Vec3::X);
        assert!((lift.z - 1.0).abs() < 1e-6);
        assert!(
            lift.x > 0.0 && lift.y.abs() < 1e-6,
            "up the screen, leaning right"
        );
        let down = lift_direction(Vec3::Y, Vec3::ZERO, Vec3::X);
        assert!(down.x < 0.0, "the way back leans the other side");
        let level = lift_direction(Vec3::ZERO, Vec3::X, Vec3::X);
        assert!((level - Vec3::Z).length() < 1e-6, "across the screen, none");
    }

    #[test]
    fn the_stack_stands_no_higher_than_the_view() {
        // Looking down -z from 100 away, a quarter-turn view: tan(fov / 2) = 1.
        let view = Eye {
            at: Vec3::Z * 100.0,
            up: Vec3::Y,
            forward: -Vec3::Z,
            tan_half: 1.0,
        };
        // Lifting straight up the screen, from the middle.
        let top = fitting_top(&view, &[(Vec3::ZERO, Vec3::Y)], 1000.0);
        assert!((top - ROOM * 100.0).abs() < 1e-3);
        assert!((fitting_top(&view, &[(Vec3::ZERO, Vec3::Y)], 10.0) - 10.0).abs() < 1e-6);
        // Lifting towards the camera keeps it in front.
        let toward = fitting_top(&view, &[(Vec3::ZERO, Vec3::Z)], 1000.0);
        assert!(toward <= 50.0 + 1e-3);
        // A link already off screen doesn't hold the rest down.
        let off = fitting_top(&view, &[(Vec3::Y * 500.0, Vec3::Y)], 30.0);
        assert!((off - 30.0).abs() < 1e-6);
    }

    #[test]
    fn an_arc_emerges_from_its_caller() {
        assert!((emerging(0.0) - 0.2).abs() < 1e-6);
        assert!(emerging(0.25) > emerging(0.1));
        assert!((emerging(0.5) - 1.0).abs() < 1e-6, "whole by its top");
        assert!((emerging(1.0) - 1.0).abs() < 1e-6, "and where it lands");
    }

    #[test]
    fn a_long_list_shows_the_stretch_around_the_cursor() {
        assert_eq!(window(10, Some(3), 15), 0..10, "all, when they fit");
        assert_eq!(window(49, Some(20), 15), 13..28);
        assert_eq!(window(49, Some(1), 15), 0..15, "kept whole at the start");
        assert_eq!(window(49, Some(48), 15), 34..49, "and at the end");
        assert_eq!(window(49, None, 15), 0..15);
    }

    #[test]
    fn numbers_keep_off_each_other() {
        let a = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(20.0, 10.0));
        let near = a.translate(egui::vec2(10.0, 0.0));
        let far = a.translate(egui::vec2(40.0, 0.0));
        assert!(!clear_of(near, &[a]));
        assert!(clear_of(far, &[a]));
    }
}
