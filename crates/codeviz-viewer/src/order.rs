//! Call order on the graph itself, two ways, to try side by side. The graph has one
//! node per function, while order belongs to calls: a function can be called twice,
//! or reached by two paths.
//!
//! - The fan lines the focus's calls up beside it, first at the top, in the screen's
//!   plane whatever the orbit. A function called more than once sits at its first
//!   call, marked `×2`. Everything else stays where the simulation puts it.
//! - The route moves nothing: it threads the disclosed journey through the nodes in
//!   the order it runs, as one line with chevrons, solid where a call goes in and
//!   dotted where execution returns and moves on to the next call. Passes over the
//!   same pair of nodes run side by side.
//!
//! `E` cycles off, fan, route; [`Chrome`] holds which.

use std::collections::HashMap;

use bevy::prelude::*;
use codeviz_core::SpanId;

use crate::{
    camera::OrbitCamera, disclosure::Disclose, explorer::Explorer, graph_view::node_size,
    scene::Tile, ui::Chrome,
};

/// How call order shows on the graph.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum OrderMode {
    #[default]
    Off,
    Fan,
    Route,
}

impl OrderMode {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Off => Self::Fan,
            Self::Fan => Self::Route,
            Self::Route => Self::Off,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Fan => "fan",
            Self::Route => "route",
        }
    }
}

/// The state call order's animations ease; [`Chrome`] says which shows.
#[derive(Resource, Debug, Default)]
pub(crate) struct Order {
    /// How far each fanned node is drawn from its place, eased towards its slot.
    offsets: HashMap<usize, Vec3>,
    /// The fan as it stands: each node in it, and how many times the focus calls it.
    pub(crate) fanned: Vec<(usize, usize)>,
    /// Where the route's cursor marker rides, in stops along the line.
    marker: Option<f32>,
}

pub(crate) struct OrderPlugin;

impl Plugin for OrderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Order>()
            .init_gizmo_group::<RouteGizmos>()
            .init_gizmo_group::<ThenGizmos>()
            .add_systems(Startup, line_widths);
    }
}

/// The route's calls, drawn thick.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct RouteGizmos;

/// The route's returns to the next call, drawn thin.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct ThenGizmos;

fn line_widths(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<RouteGizmos>().0.line.width = 3.5;
    store.config_mut::<ThenGizmos>().0.line.width = 1.5;
}

/// The route's line: pale, so it says nothing about direction to the focus or module.
const LINE: Color = Color::srgba(0.93, 0.95, 1.0, 0.9);

// ---- the fan ----

/// The nodes `calls` runs, in order, each once at its first call with how many times
/// it is called; the focus calling itself stays put.
pub(crate) fn fan_slots(
    calls: impl IntoIterator<Item = usize>,
    focus: usize,
) -> Vec<(usize, usize)> {
    let mut slots: Vec<(usize, usize)> = Vec::new();
    for tile in calls {
        if tile == focus {
            continue;
        }
        match slots.iter_mut().find(|(t, _)| *t == tile) {
            Some((_, count)) => *count += 1,
            None => slots.push((tile, 1)),
        }
    }
    slots
}

/// Calls a column of the fan holds before the next one starts beside it.
const COLUMN: usize = 24;

/// The fan's measures, as shares of the camera's distance, so it keeps its size on
/// screen as the view zooms: a slot's height when there are few, the most a column
/// spans, the gap from the focus to the nearest column, and from column to column,
/// room for a name.
const SLOT: f32 = 0.055;
const COLUMN_SPAN: f32 = 0.62;
const FAN_GAP: f32 = 0.16;
const COLUMN_GAP: f32 = 0.25;

/// Where slot `index` of `count` sits from the focus, as shares of the camera's
/// distance: right and up. Columns read like text, top to bottom, then on to the
/// column to the right, level with the focus; to its right, or `leftward` when there
/// is more room on that side.
pub(crate) fn slot_offset(index: usize, count: usize, leftward: bool) -> Vec2 {
    let columns = count.div_ceil(COLUMN).max(1);
    let rows = count.div_ceil(columns).max(1);
    let (column, row) = (index / rows, index % rows);
    // A column nearer the focus comes first on the right, last on the left.
    let out = if leftward {
        columns - 1 - column
    } else {
        column
    };
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let (out, row, rows) = (out as f32, row as f32, rows as f32);
    let height = SLOT.min(COLUMN_SPAN / rows);
    let x = FAN_GAP + out * COLUMN_GAP;
    Vec2::new(
        if leftward { -x } else { x },
        ((rows - 1.0) / 2.0 - row) * height,
    )
}

/// What the focus calls, in the order the journey makes them: the cursor's calls, or
/// for a focus off the journey, its calls as written.
fn focus_calls(explorer: &Explorer, focus: usize) -> Vec<usize> {
    let map = explorer.map();
    let on_journey = explorer.aside().is_none();
    let disclosed = explorer.disclosure();
    let calls: Vec<usize> = explorer
        .selected()
        .filter(|_| on_journey)
        .map(|span| disclosed.children(span).to_vec())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|span| explorer.unit_id(span).and_then(|u| map.tile_of(u)))
        .collect();
    if calls.is_empty() {
        explorer.graph().sequence(focus).to_vec()
    } else {
        calls
    }
}

/// Eases the focus's calls into a fan beside it while the fan is on, and everything
/// back to its place otherwise; moves the nodes' meshes with them.
pub(crate) fn fan(
    time: Res<Time>,
    chrome: Res<Chrome>,
    mut order: ResMut<Order>,
    mut explorer: ResMut<Explorer>,
    cameras: Query<(&OrbitCamera, &Transform)>,
    mut nodes: Query<(&Tile, &mut Transform), Without<OrbitCamera>>,
) {
    let order = &mut *order;
    let fanning = chrome.order == OrderMode::Fan;
    if !fanning && order.offsets.is_empty() {
        order.fanned.clear();
        return;
    }
    let Ok((camera, view)) = cameras.single() else {
        return;
    };
    // The fan's calls are numbered and lit as the journey's: their bars open with it.
    if fanning
        && explorer.aside().is_none()
        && explorer.disclose() != Disclose::Time
        && let Some(span) = explorer.selected()
        && explorer.bypass_change_detection().open_calls(span)
    {
        explorer.set_changed();
    }
    let explorer_ref = explorer.bypass_change_detection();
    let focus = explorer_ref.graph_focus();
    order.fanned = match focus.filter(|_| fanning) {
        Some(focus) => fan_slots(focus_calls(explorer_ref, focus), focus),
        None => Vec::new(),
    };
    let (right, up) = (view.right().as_vec3(), view.up().as_vec3());
    let center = focus.and_then(|f| explorer_ref.force_point(f));
    let mut goals: HashMap<usize, Vec3> = HashMap::new();
    if let Some(center) = center {
        // Towards the middle of the view when the focus sits well right of it.
        let leftward = (center - camera.target).dot(right) > camera.distance * 0.2;
        for (index, &(tile, _)) in order.fanned.iter().enumerate() {
            let at = slot_offset(index, order.fanned.len(), leftward) * camera.distance;
            if let Some(home) = explorer_ref.force_point(tile) {
                goals.insert(tile, center + right * at.x + up * at.y - home);
            }
        }
    }
    let ease = 1.0 - (-time.delta_secs() * 6.0).exp();
    let mut offsets = std::mem::take(&mut order.offsets);
    for &tile in goals.keys() {
        offsets.entry(tile).or_insert(Vec3::ZERO);
    }
    offsets.retain(|tile, offset| {
        let goal = goals.get(tile).copied().unwrap_or_default();
        *offset += (goal - *offset) * ease;
        goals.contains_key(tile) || offset.length() > 0.01
    });
    order.offsets = offsets;
    let before: Vec<usize> = explorer_ref.nudged().collect();
    explorer_ref.set_nudges(order.offsets.clone());
    for (tile, mut transform) in &mut nodes {
        if (order.offsets.contains_key(&tile.0) || before.contains(&tile.0))
            && let Some(at) = explorer_ref.tile_point(tile.0)
        {
            transform.translation = at;
        }
    }
}

// ---- the route ----

/// How the route reaches a stop from the one before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Leg {
    /// The stop before calls it.
    Call,
    /// Execution returned from the stop before and moves on to this call.
    Then,
}

/// The journey as it runs: `root`, then each call under it in order, depth first,
/// each with how it is reached from the stop before. `children` lists the shown calls
/// a call makes.
pub(crate) fn route<S: Copy + Eq>(
    root: S,
    children: impl Fn(S) -> Vec<S>,
) -> Vec<(S, Option<Leg>)> {
    let mut stops = Vec::new();
    // Each call to visit with its caller.
    let mut stack: Vec<(S, Option<S>)> = vec![(root, None)];
    let mut previous: Option<S> = None;
    while let Some((stop, caller)) = stack.pop() {
        let leg = previous.map(|p| {
            if Some(p) == caller {
                Leg::Call
            } else {
                Leg::Then
            }
        });
        stops.push((stop, leg));
        previous = Some(stop);
        stack.extend(children(stop).into_iter().rev().map(|c| (c, Some(stop))));
    }
    stops
}

/// Legs either side of the cursor's stop that draw in full; further along the line in
/// either direction fades, so on a busy graph the stretch around the cursor reads.
const NEAR_LEGS: usize = 4;

/// How strongly leg `index`, from stop `index` to the next, draws with the cursor at
/// stop `cursor`: in full near it, fading with each leg further, never quite gone.
pub(crate) fn leg_strength(index: usize, cursor: Option<usize>) -> f32 {
    let Some(cursor) = cursor else {
        return 1.0;
    };
    // Legs before the cursor end at or before it; legs after start at or after it.
    let away = if index < cursor {
        cursor - index - 1
    } else {
        index - cursor
    };
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let beyond = away.saturating_sub(NEAR_LEGS) as f32;
    (1.0 - beyond * 0.15).max(0.12)
}

/// For each leg between two nodes, how far to set it aside, in lanes, so passes over
/// the same pair run side by side: centered on the line between them, measured across
/// it as seen from the lower-numbered node, so a pass and its return share one frame.
pub(crate) fn lanes(legs: &[(usize, usize)]) -> Vec<f32> {
    let key = |&(a, b): &(usize, usize)| (a.min(b), a.max(b));
    let mut totals: HashMap<(usize, usize), usize> = HashMap::new();
    for leg in legs {
        *totals.entry(key(leg)).or_default() += 1;
    }
    let mut seen: HashMap<(usize, usize), usize> = HashMap::new();
    legs.iter()
        .map(|leg| {
            let total = totals[&key(leg)];
            let nth = seen.entry(key(leg)).or_default();
            #[expect(clippy::cast_precision_loss, reason = "small counts")]
            let lane = *nth as f32 - (total as f32 - 1.0) / 2.0;
            *nth += 1;
            lane
        })
        .collect()
}

/// The disclosed journey's stops, by node, with how each is reached and its span.
fn journey_route(explorer: &Explorer) -> Vec<(usize, SpanId, Option<Leg>)> {
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
    let mut stops = Vec::new();
    // A call whose node the graph lacks is skipped; what follows it is reached by return.
    let mut skipped = false;
    for (span, leg) in route(journey.root(), shown) {
        match explorer.unit_id(span).and_then(|u| map.tile_of(u)) {
            Some(tile) => {
                let leg = if skipped { leg.map(|_| Leg::Then) } else { leg };
                stops.push((tile, span, leg));
                skipped = false;
            }
            None => skipped = true,
        }
    }
    stops
}

/// One leg as drawn: its two ends, beside the line between the nodes' centers by its
/// lane and clear of the nodes, and how it is reached.
struct Drawn {
    from: Vec3,
    to: Vec3,
    leg: Leg,
}

/// Lays the route's legs out for a camera looking along `forward`, `lane` apart.
fn drawn_legs(
    explorer: &Explorer,
    stops: &[(usize, SpanId, Option<Leg>)],
    forward: Vec3,
    lane: f32,
) -> Vec<Option<Drawn>> {
    let pairs: Vec<(usize, usize)> = stops.windows(2).map(|w| (w[0].0, w[1].0)).collect();
    let offsets = lanes(&pairs);
    let graph = explorer.graph();
    stops
        .windows(2)
        .zip(offsets)
        .map(|(pair, offset)| {
            let (a, b) = (pair[0].0, pair[1].0);
            let (low, high) = (a.min(b), a.max(b));
            let (pa, pb) = (explorer.tile_point(a)?, explorer.tile_point(b)?);
            let (pl, ph) = (explorer.tile_point(low)?, explorer.tile_point(high)?);
            let across = (ph - pl).cross(forward).normalize_or_zero() * offset * lane;
            let along = (pb - pa).normalize_or_zero();
            let (ra, rb) = (
                node_size(graph, a) * 0.5 + 0.25,
                node_size(graph, b) * 0.5 + 0.25,
            );
            if pa.distance(pb) <= ra + rb {
                return None;
            }
            Some(Drawn {
                from: pa + along * ra + across,
                to: pb - along * rb + across,
                leg: pair[1].2.unwrap_or(Leg::Then),
            })
        })
        .collect()
}

/// Draws the disclosed journey as one line through its nodes, in the order it runs,
/// with the cursor's marker riding it.
pub(crate) fn draw_route(
    time: Res<Time>,
    chrome: Res<Chrome>,
    explorer: Res<Explorer>,
    mut order: ResMut<Order>,
    cameras: Query<(&OrbitCamera, &Transform)>,
    mut calls: Gizmos<RouteGizmos>,
    mut thens: Gizmos<ThenGizmos>,
) {
    if chrome.order != OrderMode::Route {
        order.marker = None;
        return;
    }
    let Ok((camera, view)) = cameras.single() else {
        return;
    };
    let scale = camera.distance;
    let forward = view.forward().as_vec3();
    let stops = journey_route(&explorer);
    let legs = drawn_legs(&explorer, &stops, forward, scale * 0.007);
    let chevron = scale * 0.011;
    let cursor = explorer.selected();
    let current = stops.iter().position(|&(_, span, _)| Some(span) == cursor);
    for (index, drawn) in legs.iter().enumerate() {
        let Some(drawn) = drawn else {
            continue;
        };
        let (a, b) = (drawn.from, drawn.to);
        let along = (b - a).normalize_or_zero();
        let side = along.cross(forward).normalize_or_zero();
        let line = LINE.with_alpha(LINE.alpha() * leg_strength(index, current));
        match drawn.leg {
            Leg::Call => {
                calls.line(a, b, line);
                chevrons(&mut calls, a, b, chevron, scale * 0.06, side, line);
            }
            Leg::Then => {
                let faint = line.with_alpha(line.alpha() * 0.8);
                dotted(&mut thens, a, b, scale * 0.006, faint);
                chevrons(&mut thens, a, b, chevron * 0.8, scale * 0.08, side, faint);
            }
        }
        // A station tick across the line where it reaches its stop.
        calls.line(b - side * chevron * 0.7, b + side * chevron * 0.7, line);
    }
    // The cursor's marker rides the line to its stop.
    let Some(target) = current else {
        order.marker = None;
        return;
    };
    #[expect(clippy::cast_precision_loss, reason = "a stop index")]
    let target = target as f32;
    let ease = 1.0 - (-time.delta_secs() * 5.0).exp();
    let at = order.marker.map_or(target, |m| m + (target - m) * ease);
    order.marker = Some(at);
    if let Some(point) = along_route(&explorer, &stops, &legs, at) {
        // Rings facing the camera, a dot inside a halo, so it reads as a marker on the
        // line rather than another node.
        let facing = Isometry3d::new(point, view.rotation);
        let radius = scale * 0.006;
        for (grow, alpha) in [(0.4, 1.0), (1.0, 1.0), (2.0, 0.5)] {
            calls.circle(facing, radius * grow, Color::srgba(1.0, 1.0, 1.0, alpha));
        }
    }
}

/// Where `at` stops along the route falls: on a stop, where its incoming leg reaches
/// it, else part way along the leg.
fn along_route(
    explorer: &Explorer,
    stops: &[(usize, SpanId, Option<Leg>)],
    legs: &[Option<Drawn>],
    at: f32,
) -> Option<Vec3> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a stop index, never negative"
    )]
    let stop = at.floor().max(0.0) as usize;
    let part = at - at.floor();
    if part < 1e-3 {
        return match stop.checked_sub(1).and_then(|leg| legs.get(leg)) {
            Some(Some(drawn)) => Some(drawn.to),
            _ => explorer.tile_point(stops.get(stop)?.0),
        };
    }
    match legs.get(stop) {
        Some(Some(drawn)) => Some(drawn.from.lerp(drawn.to, part)),
        _ => explorer.tile_point(stops.get(stop)?.0),
    }
}

/// Chevrons pointing from `a` to `b`, about `every` apart, at least one, spread
/// `across` the line.
fn chevrons<T: GizmoConfigGroup>(
    gizmos: &mut Gizmos<T>,
    a: Vec3,
    b: Vec3,
    size: f32,
    every: f32,
    across: Vec3,
    color: Color,
) {
    let length = a.distance(b);
    if length < size * 3.0 {
        return;
    }
    let along = (b - a) / length;
    let count = (length / every).floor().clamp(1.0, 12.0);
    let mut index = 0.0;
    while index < count {
        let tip = a.lerp(b, (index + 0.5) / count) + along * size * 0.5;
        let back = tip - along * size;
        gizmos.line(back + across * size * 0.6, tip, color);
        gizmos.line(back - across * size * 0.6, tip, color);
        index += 1.0;
    }
}

/// A line of short dashes, `dash` long.
fn dotted<T: GizmoConfigGroup>(gizmos: &mut Gizmos<T>, a: Vec3, b: Vec3, dash: f32, color: Color) {
    let length = a.distance(b);
    let dashes = (length / dash).floor().max(1.0);
    let mut at = 0.0;
    while at < dashes {
        gizmos.line(
            a.lerp(b, at / dashes),
            a.lerp(b, ((at + 1.0) / dashes).min(1.0)),
            color,
        );
        at += 2.0;
    }
}

/// How strongly the ordinary graph's links draw: the route fades them back.
pub(crate) fn link_strength(chrome: &Chrome) -> f32 {
    if chrome.order == OrderMode::Route {
        0.3
    } else {
        1.0
    }
}

/// `×2` under each node the fan holds that the focus calls more than once, given
/// where each node sits on screen and how far its edge is from its middle.
pub(crate) fn draw_marks(
    painter: &bevy_egui::egui::Painter,
    discs: &[Option<(bevy_egui::egui::Pos2, f32)>],
    fanned: &[(usize, usize)],
) {
    use bevy_egui::egui;
    for &(tile, count) in fanned {
        let Some((center, edge)) = discs.get(tile).copied().flatten().filter(|_| count > 1) else {
            continue;
        };
        // Under the node, where its name rarely goes.
        painter.text(
            center + egui::vec2(0.0, edge + 1.0),
            egui::Align2::CENTER_TOP,
            format!("×{count}"),
            egui::FontId::proportional(11.0),
            egui::Color32::from_rgb(235, 238, 245),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_function_called_twice_fans_once_at_its_first_call() {
        // The focus (0) calls 3, 5, 3, itself, then 7.
        assert_eq!(fan_slots([3, 5, 3, 0, 7], 0), vec![(3, 2), (5, 1), (7, 1)]);
    }

    #[test]
    fn the_fan_reads_top_to_bottom_then_left_to_right() {
        for leftward in [false, true] {
            let few: Vec<Vec2> = (0..3).map(|i| slot_offset(i, 3, leftward)).collect();
            assert!(
                few.iter().all(|p| (p.x - few[0].x).abs() < 1e-6),
                "one column"
            );
            assert!(
                few[0].y > few[1].y && few[1].y > few[2].y,
                "first at the top"
            );
            assert!((few[0].y + few[2].y).abs() < 1e-6, "level with the focus");
            assert_eq!(few[0].x < 0.0, leftward, "on its side of the focus");
            let many: Vec<Vec2> = (0..30).map(|i| slot_offset(i, 30, leftward)).collect();
            assert!(
                many[29].x > many[0].x,
                "a long list wraps into columns to the right"
            );
            assert!(many.windows(2).all(|w| w[1].x > w[0].x || w[1].y < w[0].y));
            let most = (0..100).map(|i| slot_offset(i, 100, leftward).y.abs());
            assert!(
                most.fold(0.0, f32::max) <= COLUMN_SPAN / 2.0,
                "a column fits the view"
            );
        }
    }

    #[test]
    fn the_route_runs_depth_first_calling_in_and_moving_on() {
        // 0 calls 1 then 4; 1 calls 2 then 3.
        let tree = |n: u32| match n {
            0 => vec![1, 4],
            1 => vec![2, 3],
            _ => vec![],
        };
        assert_eq!(
            route(0, tree),
            vec![
                (0, None),
                (1, Some(Leg::Call)),
                (2, Some(Leg::Call)),
                (3, Some(Leg::Then)),
                (4, Some(Leg::Then)),
            ]
        );
    }

    #[test]
    fn the_route_fades_away_from_the_cursor() {
        assert!(
            (leg_strength(9, Some(10)) - 1.0).abs() < 1e-6,
            "the leg into it"
        );
        assert!(
            (leg_strength(10, Some(10)) - 1.0).abs() < 1e-6,
            "the leg out of it"
        );
        assert!(leg_strength(16, Some(10)) < leg_strength(15, Some(10)));
        assert!(leg_strength(3, Some(10)) < 1.0, "behind it too");
        assert!(leg_strength(400, Some(10)) > 0.0, "never quite gone");
        assert!(
            (leg_strength(400, None) - 1.0).abs() < 1e-6,
            "without a cursor, all of it"
        );
    }

    #[test]
    fn passes_over_the_same_pair_run_side_by_side() {
        // Out to 2 and back, then on to 3, then 2 again from the other side.
        let offsets = lanes(&[(1, 2), (2, 1), (1, 3), (3, 2)]);
        assert_eq!(offsets[0], -0.5);
        assert_eq!(offsets[1], 0.5, "the return runs beside the way out");
        assert_eq!(offsets[2], 0.0, "a pair passed once stays on the line");
        assert_eq!(offsets[3], 0.0);
        let thrice = lanes(&[(4, 5), (5, 4), (4, 5)]);
        assert_eq!(thrice, vec![-1.0, 0.0, 1.0]);
    }
}
