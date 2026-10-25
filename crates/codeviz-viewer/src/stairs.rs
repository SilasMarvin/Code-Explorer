//! Call order on the depth axis: the staircase. The map stays where the simulation
//! puts it, in its plane; each node on the disclosed journey rises out of it by when
//! it is first called, so the journey climbs like a staircase from where it starts.
//! Nodes off the journey stay on the map.
//!
//! A node stands on a thin stem down to its place on the map, so its height reads as
//! a time and its footprint as a place. A function called again keeps its first
//! call's height; its stem runs on up through each later call, with a ring at each.
//! A ruler beside the stairs counts the steps, the cursor's step is marked on it and
//! levelled across to its node, and stems fade with distance from the cursor.
//!
//! `Z` raises and lowers the stairs, easing the camera round to a view that shows
//! them and back; [`Chrome`] holds which.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::SpanId;

use crate::{
    camera::{ANGLED, OrbitCamera},
    explorer::Explorer,
    graph_view::node_size,
    scene::Tile,
    ui::Chrome,
};

/// Whether time shows on the depth axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TimeMode {
    #[default]
    Flat,
    Stairs,
}

impl TimeMode {
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Flat => Self::Stairs,
            Self::Stairs => Self::Flat,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Stairs => "stairs",
        }
    }
}

/// The view the stairs are read from: the map tipped back like a table seen from its
/// near edge, so up the screen is up the stairs, and turned a little to show depth.
pub(crate) const STAIRS_VIEW: (f32, f32) = (-0.12, 1.1);

/// The staircase as it stands, eased.
#[derive(Resource, Debug, Default)]
pub(crate) struct Stairs {
    /// How far the stairs are up, from 0 (flat) to 1.
    rise: f32,
    /// Whether the stairs were last asked up, to notice a toggle.
    up: bool,
    /// The camera's angles before the stairs went up, to go back to.
    returns_to: Option<(f32, f32)>,
    /// How high the camera looks above the anchor, eased towards the cursor's step.
    lift: f32,
    /// The disclosed journey in the order it runs, as drawn this frame.
    steps: Vec<Step>,
    /// Each step's height at full rise, eased as the cursor moves.
    heights: Vec<f32>,
    /// Where the ruler stands on the map.
    ruler: Option<Vec3>,
    /// The cursor's step.
    cursor: Option<usize>,
}

/// One call on the stairs.
#[derive(Debug, Clone, PartialEq)]
struct Step {
    tile: usize,
    /// The step of this node's first call, where it stands.
    first: usize,
    /// The call's number in the journey.
    number: String,
}

pub(crate) struct StairsPlugin;

impl Plugin for StairsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stairs>()
            .init_gizmo_group::<StemGizmos>()
            .add_systems(Startup, line_widths);
    }
}

/// Stems, rings and the ruler, drawn thin.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct StemGizmos;

fn line_widths(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<StemGizmos>().0.line.width = 1.5;
}

/// Neutral: the stairs say when, never direction or module.
const STEM: Color = Color::srgba(0.85, 0.88, 0.95, 0.8);
const MARK: Color = Color::srgb(1.0, 1.0, 1.0);

/// `root`, then each call under it in order, depth first.
pub(crate) fn preorder<S: Copy>(root: S, children: impl Fn(S) -> Vec<S>) -> Vec<S> {
    let mut order = Vec::new();
    let mut stack = vec![root];
    while let Some(next) = stack.pop() {
        order.push(next);
        stack.extend(children(next).into_iter().rev());
    }
    order
}

/// For each step, the step at which its node is first called.
pub(crate) fn first_calls(tiles: &[usize]) -> Vec<usize> {
    let mut first: HashMap<usize, usize> = HashMap::new();
    tiles
        .iter()
        .enumerate()
        .map(|(index, &tile)| *first.entry(tile).or_insert(index))
        .collect()
}

/// A step's height as a share of how far the journey spreads over the map, when it
/// is short; a long journey's steps close up so the whole climb stays within
/// `TALLEST` of that spread, all but those around the cursor, which keep up to
/// `CLOSE_SHARE` of it between them.
const STEP: f32 = 0.16;
const TALLEST: f32 = 1.0;
const CLOSE_SHARE: f32 = 0.5;

/// The height of each of `count` steps over a journey spreading `reach`: evenly
/// apart when they fit, else wide around the `cursor` and close up elsewhere.
pub(crate) fn heights(count: usize, cursor: Option<usize>, reach: f32) -> Vec<f32> {
    let (full, tallest) = (reach * STEP, reach * TALLEST);
    let climbs = count.saturating_sub(1);
    let cursor = cursor.unwrap_or(0);
    // The climb up to step `index` is near the cursor when either end of it is.
    let near = |index: usize| index.abs_diff(cursor) <= NEAR_STEPS;
    let near_climbs = (1..count).filter(|&i| near(i)).count();
    #[expect(clippy::cast_precision_loss, reason = "step counts")]
    let (wide, close) = if climbs as f32 * full <= tallest {
        (full, full)
    } else {
        let even = tallest / climbs as f32;
        let wide = full.min(tallest * CLOSE_SHARE / near_climbs.max(1) as f32);
        let rest = (climbs - near_climbs).max(1) as f32;
        let close = (tallest - wide * near_climbs as f32) / rest;
        // Too few steps away from the cursor to be worth closing up: evenly apart.
        if close < wide {
            (wide, close)
        } else {
            (even, even)
        }
    };
    let mut height = 0.0;
    (0..count)
        .map(|index| {
            if index > 0 {
                height += if near(index) { wide } else { close };
            }
            height
        })
        .collect()
}

/// Steps either side of the cursor that draw in full; further away fades.
const NEAR_STEPS: usize = 5;

/// How strongly what stands at step `index` draws with the cursor at `cursor`.
pub(crate) fn strength(index: usize, cursor: Option<usize>) -> f32 {
    let Some(cursor) = cursor else {
        return 1.0;
    };
    #[expect(clippy::cast_precision_loss, reason = "a step count")]
    let beyond = index.abs_diff(cursor).saturating_sub(NEAR_STEPS) as f32;
    (1.0 - beyond * 0.08).max(0.2)
}

/// The disclosed journey in the order it runs, by node; calls the graph lacks are
/// left out.
fn journey_steps(explorer: &Explorer) -> Vec<(usize, SpanId, String)> {
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
    preorder(journey.root(), shown)
        .into_iter()
        .filter_map(|span| {
            let tile = explorer.unit_id(span).and_then(|u| map.tile_of(u))?;
            let number = disclosed.number(span).unwrap_or_default().to_owned();
            Some((tile, span, number))
        })
        .collect()
}

/// Raises the journey into stairs while they are on and lowers it otherwise, moving
/// the nodes' meshes with it; turns the camera to see them and back.
pub(crate) fn rise(
    time: Res<Time>,
    chrome: Res<Chrome>,
    mut stairs: ResMut<Stairs>,
    mut explorer: ResMut<Explorer>,
    mut cameras: Query<(&mut OrbitCamera, &Transform)>,
    mut nodes: Query<(&Tile, &mut Transform), Without<OrbitCamera>>,
) {
    let stairs = &mut *stairs;
    let up = chrome.time == TimeMode::Stairs;
    if !up && stairs.rise == 0.0 && !stairs.up {
        return;
    }
    let explorer = explorer.bypass_change_detection();
    let found = journey_steps(explorer);
    let tiles: Vec<usize> = found.iter().map(|&(tile, _, _)| tile).collect();
    let firsts = first_calls(&tiles);
    let cursor = explorer.selected();
    stairs.cursor = found.iter().position(|&(_, span, _)| Some(span) == cursor);
    stairs.steps = found
        .into_iter()
        .zip(firsts)
        .map(|((tile, _, number), first)| Step {
            tile,
            first,
            number,
        })
        .collect();
    let base = stairs
        .steps
        .first()
        .and_then(|s| explorer.force_point(s.tile));
    let reach = base.map_or(0.0, |base| {
        stairs
            .steps
            .iter()
            .filter_map(|s| explorer.force_point(s.tile))
            .map(|p| p.truncate().distance(base.truncate()))
            .fold(6.0, f32::max)
    });
    let goals = heights(stairs.steps.len(), stairs.cursor, reach);

    let Ok((mut camera, view)) = cameras.single_mut() else {
        return;
    };
    if up != stairs.up {
        stairs.up = up;
        if up {
            stairs.returns_to = Some(camera.angles());
            camera.turn_to(STAIRS_VIEW);
            let height = goals.last().copied().unwrap_or_default();
            let target = camera.target;
            let distance = camera.distance.max(height * 1.8);
            camera.glide_to(target, distance);
        } else {
            camera.turn_to(stairs.returns_to.take().unwrap_or(ANGLED));
        }
    }
    let ease = 1.0 - (-time.delta_secs() * 4.0).exp();
    let goal = if up { 1.0 } else { 0.0 };
    stairs.rise += (goal - stairs.rise) * ease;
    if (goal - stairs.rise).abs() < 1e-3 {
        stairs.rise = goal;
    }
    if stairs.heights.len() == goals.len() {
        for (height, goal) in stairs.heights.iter_mut().zip(&goals) {
            *height += (goal - *height) * ease;
        }
    } else {
        stairs.heights = goals;
    }
    let level = |step: usize| stairs.heights.get(step).copied().unwrap_or_default() * stairs.rise;
    // Mid-way up a short climb; on a long one, following the cursor up it.
    let height = level(stairs.steps.len().saturating_sub(1));
    let lift_goal = stairs.cursor.map_or(height * 0.5, |c| {
        level(c).clamp(height * 0.35, height * 0.65)
    });
    stairs.lift += (lift_goal - stairs.lift) * ease;

    // The ruler stands left of everything on the stairs, as seen, level with the start.
    let right = view.right().as_vec3();
    stairs.ruler = base.map(|base| {
        let leftmost = stairs
            .steps
            .iter()
            .filter_map(|s| explorer.force_point(s.tile))
            .map(|p| (p - base).dot(right))
            .fold(0.0, f32::min);
        let along = Vec3::new(right.x, right.y, 0.0).normalize_or_zero();
        base + along * (leftmost - reach * 0.1)
    });

    let mut nudges: HashMap<usize, Vec3> = HashMap::new();
    if stairs.rise > 0.0 {
        for step in &stairs.steps {
            nudges
                .entry(step.tile)
                .or_insert(Vec3::Z * level(step.first));
        }
    }
    let before: Vec<usize> = explorer.nudged().collect();
    explorer.set_nudges(nudges);
    for (tile, mut transform) in &mut nodes {
        if (stairs.rise > 0.0 || before.contains(&tile.0))
            && let Some(at) = explorer.tile_point(tile.0)
        {
            transform.translation = at;
        }
    }
}

impl Stairs {
    /// Whether the stairs are up, or rising.
    pub(crate) fn up(&self) -> bool {
        self.up
    }

    /// How high the camera looks above the node it orbits.
    pub(crate) fn lift(&self) -> Vec3 {
        Vec3::Z * self.lift
    }

    /// The height of step `index`, as drawn now.
    fn level(&self, index: usize) -> f32 {
        self.heights.get(index).copied().unwrap_or_default() * self.rise
    }

    /// Whether the stairs show at all.
    fn showing(&self) -> bool {
        self.rise > 0.02 && !self.steps.is_empty()
    }
}

/// Draws each stair node's stem down to the map, rings where it is called again, the
/// ruler, and the cursor's level.
pub(crate) fn draw_stairs(
    stairs: Res<Stairs>,
    explorer: Res<Explorer>,
    mut gizmos: Gizmos<StemGizmos>,
    mut bright: Gizmos,
) {
    if !stairs.showing() {
        return;
    }
    let graph = explorer.graph();
    let rise = stairs.rise;
    // Every call each node makes, by step.
    let mut calls: HashMap<usize, Vec<usize>> = HashMap::new();
    for (index, step) in stairs.steps.iter().enumerate() {
        calls.entry(step.tile).or_default().push(index);
    }
    for (&tile, steps) in &calls {
        let Some(foot) = explorer.force_point(tile) else {
            continue;
        };
        let near = steps
            .iter()
            .map(|&s| strength(s, stairs.cursor))
            .fold(0.0, f32::max);
        let color = STEM.with_alpha(STEM.alpha() * near * rise);
        let radius = node_size(graph, tile) * 0.5;
        let first = stairs.level(steps[0]);
        let last = stairs.level(*steps.last().unwrap_or(&steps[0]));
        // Down to the map, where a small ring marks its place.
        if first > radius {
            gizmos.line(foot, foot + Vec3::Z * (first - radius), color);
        }
        gizmos.circle(Isometry3d::from_translation(foot), radius * 0.8, color);
        gizmos.circle(Isometry3d::from_translation(foot), radius * 0.3, color);
        // On up through every later call.
        if last > first + radius {
            gizmos.line(
                foot + Vec3::Z * (first + radius),
                foot + Vec3::Z * last,
                color,
            );
        }
        for &later in &steps[1..] {
            let at = foot + Vec3::Z * stairs.level(later);
            let mark = if Some(later) == stairs.cursor {
                MARK
            } else {
                STEM.with_alpha(STEM.alpha() * strength(later, stairs.cursor) * rise * 1.4)
            };
            gizmos.circle(Isometry3d::from_translation(at), radius * 0.7, mark);
        }
    }
    draw_ruler(&stairs, &explorer, &mut gizmos, &mut bright);
}

/// The ruler: a pole with a tick per step, and the cursor's step levelled across to
/// where its call stands.
fn draw_ruler(
    stairs: &Stairs,
    explorer: &Explorer,
    gizmos: &mut Gizmos<StemGizmos>,
    bright: &mut Gizmos,
) {
    let Some(base) = stairs.ruler else {
        return;
    };
    let count = stairs.steps.len();
    let top = stairs.level(count.saturating_sub(1));
    let rise = stairs.rise;
    let pole = STEM.with_alpha(STEM.alpha() * rise);
    gizmos.line(base, base + Vec3::Z * top, pole);
    let tick = (stairs.level(count.saturating_sub(1)) * 0.02).clamp(0.15, 0.8);
    for index in 0..count {
        let at = base + Vec3::Z * stairs.level(index);
        let long = if index % 5 == 0 { 2.0 } else { 1.0 };
        let color = pole.with_alpha(pole.alpha() * strength(index, stairs.cursor));
        gizmos.line(at, at + Vec3::X * tick * long, color);
    }
    let Some(cursor) = stairs.cursor else {
        return;
    };
    let Some(step) = stairs.steps.get(cursor) else {
        return;
    };
    let level = stairs.level(cursor);
    let mark = MARK.with_alpha(rise);
    let at = base + Vec3::Z * level;
    bright.line(at - Vec3::X * tick, at + Vec3::X * tick * 2.5, mark);
    // Across to the call, at its height: its node, or the ring on its stem.
    if let Some(foot) = explorer.force_point(step.tile) {
        let to = foot + Vec3::Z * level;
        dashed(
            gizmos,
            at + Vec3::X * tick * 2.5,
            to,
            tick,
            mark.with_alpha(0.7 * rise),
        );
    }
}

/// A dashed line, dashes `dash` long.
fn dashed(gizmos: &mut Gizmos<StemGizmos>, a: Vec3, b: Vec3, dash: f32, color: Color) {
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

/// The ruler's numbers, the journey's own for the call at each height, as many as
/// fit; the cursor's first and brightest.
pub(crate) fn draw_labels(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    stairs: &Stairs,
) {
    /// Pixels kept between numbers up the ruler.
    const ROOM: f32 = 13.0;
    if !stairs.showing() {
        return;
    }
    let Some(base) = stairs.ruler else {
        return;
    };
    let alpha = stairs.rise;
    let at = |index: usize| to_screen(base + Vec3::Z * stairs.level(index));
    let mut placed: Vec<f32> = Vec::new();
    if let Some((cursor, point)) = stairs.cursor.and_then(|c| Some((c, at(c)?))) {
        let number = &stairs.steps[cursor].number;
        painter.text(
            point - egui::vec2(6.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            if number.is_empty() { "start" } else { number },
            egui::FontId::monospace(13.0),
            egui::Color32::WHITE.gamma_multiply(alpha),
        );
        placed.push(point.y);
    }
    let grey = egui::Color32::from_rgb(160, 166, 180).gamma_multiply(alpha);
    for (index, step) in stairs.steps.iter().enumerate() {
        let Some(point) = at(index).filter(|_| !step.number.is_empty()) else {
            continue;
        };
        if placed.iter().any(|y| (y - point.y).abs() < ROOM) {
            continue;
        }
        painter.text(
            point - egui::vec2(4.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            &step.number,
            egui::FontId::monospace(10.0),
            grey.gamma_multiply(strength(index, stairs.cursor)),
        );
        placed.push(point.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_journey_runs_depth_first() {
        // 0 calls 1 then 4; 1 calls 2 then 3.
        let tree = |n: u32| match n {
            0 => vec![1, 4],
            1 => vec![2, 3],
            _ => vec![],
        };
        assert_eq!(preorder(0, tree), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_function_called_again_stands_at_its_first_call() {
        assert_eq!(first_calls(&[7, 3, 5, 3, 7, 9]), vec![0, 1, 2, 1, 0, 5]);
    }

    #[test]
    fn long_journeys_close_their_steps_up_away_from_the_cursor() {
        let short = heights(4, Some(1), 10.0);
        let step = 10.0 * STEP;
        assert!(
            short.windows(2).all(|w| (w[1] - w[0] - step).abs() < 1e-5),
            "a short climb evenly at full height: {short:?}"
        );
        let long = heights(200, Some(100), 10.0);
        assert!((long[0]).abs() < 1e-6, "the start stays on the map");
        assert!(
            (long[199] - 10.0 * TALLEST).abs() < 1e-3,
            "no taller than the cap: {}",
            long[199]
        );
        let climb = |i: usize| long[i] - long[i - 1];
        assert!(
            climb(101) > climb(150) * 3.0,
            "wide by the cursor, close away"
        );
        assert!((climb(100) - climb(101)).abs() < 1e-5);
        assert!(long.windows(2).all(|w| w[1] > w[0]), "always climbing");
        let few = heights(19, Some(10), 10.0);
        let climbs: Vec<f32> = few.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            climbs.iter().all(|c| *c <= climbs[9] + 1e-5),
            "never closer by the cursor than away from it: {climbs:?}"
        );
        assert_eq!(heights(1, None, 10.0), vec![0.0], "a lone step on the map");
        assert_eq!(heights(0, None, 10.0), Vec::<f32>::new());
    }

    #[test]
    fn the_stairs_fade_away_from_the_cursor() {
        assert!((strength(10, Some(10)) - 1.0).abs() < 1e-6);
        assert!((strength(10 + NEAR_STEPS, Some(10)) - 1.0).abs() < 1e-6);
        assert!(strength(30, Some(10)) < strength(20, Some(10)));
        assert!(strength(0, Some(30)) < 1.0, "behind it too");
        assert!(strength(900, Some(0)) > 0.0, "never quite gone");
        assert!((strength(900, None) - 1.0).abs() < 1e-6);
    }
}
