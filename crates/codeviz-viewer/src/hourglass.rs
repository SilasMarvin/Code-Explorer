//! The hourglass (Z, `--z hourglass`): the map lies flat as a floor and height says
//! how many calls a node is from the focus. What calls the focus stands a level above
//! it, what calls those two levels; what the focus calls hangs a level below, what
//! those call two. Everything else stays on the floor, dimmed. Calls run downhill, so
//! control flow falls through the focus like sand through the neck of an hourglass.
//! Each lifted node keeps its place on the map: a stem in its module's color drops
//! from it to the floor, inside its module's ring, so where it lives still reads.
//! When the focus moves, nodes rise and sink to their new levels.

use std::collections::HashSet;

use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_keyboard_input;
use codeviz_layout::Reach;

use crate::{
    camera::{ANGLED, OVER_FLOOR, OrbitCamera},
    explorer::Explorer,
    graph_view::{
        CALLEE, CALLER, Relation, hourglass_center, hourglass_distance, near, node_size,
        orbit_anchor, relation,
    },
    scene::{PALETTE, Tile, lane_color},
};

/// What height means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Height {
    /// Whatever the graph's layout gives it.
    #[default]
    Layout,
    /// Calls from the focus: callers above, callees below.
    Hourglass,
}

pub(crate) struct HourglassPlugin;

impl Plugin for HourglassPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                toggle.run_if(not(egui_wants_any_keyboard_input)),
                lay_floor,
                rise,
                keep_levels_in_view,
                draw_hourglass,
            )
                .chain()
                .after(crate::graph_view::track_attention)
                .before(crate::graph_view::follow_focus),
        );
    }
}

/// Levels either side of the focus: a call further still sits on the last.
const DEEPEST: u32 = 3;

/// How many levels above the focus a node stands (below when negative): one per call
/// between them, for what the graph lights around the focus; zero for the focus and
/// for what is unrelated or not lit.
pub(crate) fn level(explorer: &Explorer, reach: &Reach, tile: usize) -> i8 {
    if explorer.graph_focus() == Some(tile) || near(explorer, reach, tile).is_none() {
        return 0;
    }
    let steps = |hops: u32| i8::try_from(hops.min(DEEPEST)).unwrap_or_default();
    match relation(reach, tile) {
        Some(Relation::Upstream(hops)) => steps(hops),
        Some(Relation::Downstream(hops)) => -steps(hops),
        None => 0,
    }
}

/// How high one level stands: about half as far as what is lifted spreads from the
/// focus across the map (`spread`, each one's distance), so the levels stand as far
/// apart as the neighborhood is wide; never more than a share of the whole map's
/// width (`positions`), so they do not tower over it.
pub(crate) fn level_height(positions: &[[f32; 3]], spread: &[f32]) -> f32 {
    let (mut low, mut high) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for p in positions {
        let p = Vec2::new(p[0], p[1]);
        low = low.min(p);
        high = high.max(p);
    }
    let width = (high - low).max_element();
    if !width.is_finite() {
        return MIN_LEVEL;
    }
    let tallest = (width * 0.25).max(MIN_LEVEL);
    let mut spread = spread.to_vec();
    spread.sort_by(f32::total_cmp);
    let wide = spread
        .get(spread.len() * 9 / 10)
        .or(spread.last())
        .copied()
        .unwrap_or(tallest);
    (wide * 0.42).clamp(MIN_LEVEL, tallest)
}

/// The least a level stands, so a small map's levels still part.
const MIN_LEVEL: f32 = 10.0;

/// How fast heights ease: the share of the way left covered in a second is
/// `1 - e^-EASE`.
const EASE: f32 = 5.0;

/// Eases each value towards its target. Returns whether any moved.
pub(crate) fn ease(values: &mut [f32], targets: &[f32], dt: f32) -> bool {
    let share = 1.0 - (-EASE * dt).exp();
    let mut moved = false;
    for (value, &target) in values.iter_mut().zip(targets) {
        let gap = target - *value;
        if gap == 0.0 {
            continue;
        }
        *value = if gap.abs() < 0.01 {
            target
        } else {
            *value + gap * share
        };
        moved = true;
    }
    moved
}

/// Z turns the hourglass on and off.
fn toggle(keys: Res<ButtonInput<KeyCode>>, mut explorer: ResMut<Explorer>) {
    if keys.just_pressed(KeyCode::KeyZ) {
        let on = !explorer.hourglass();
        explorer.set_hourglass(on);
    }
}

/// Lays the map down as a floor when the hourglass comes on, and stands it back up
/// when it goes off. Opening with it on starts there, with nodes at their levels.
fn lay_floor(
    mut explorer: ResMut<Explorer>,
    mut last: Local<Option<bool>>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let on = explorer.hourglass();
    if *last == Some(on) {
        return;
    }
    let opening = last.is_none();
    *last = Some(on);
    if opening && !on {
        return;
    }
    if opening {
        explorer.bypass_change_detection().settle_lift();
    }
    for mut camera in &mut cameras {
        camera.lay_down(on, if on { OVER_FLOOR } else { ANGLED }, opening);
        if !camera.free {
            orbit_anchor(&explorer, &mut camera);
        }
    }
}

/// Moves nodes towards their levels.
fn rise(
    time: Res<Time>,
    mut explorer: ResMut<Explorer>,
    mut nodes: Query<(&Tile, &mut Transform)>,
) {
    if !explorer
        .bypass_change_detection()
        .ease_lift(time.delta_secs())
    {
        return;
    }
    for (tile, mut transform) in &mut nodes {
        if let Some(at) = explorer.tile_point(tile.0) {
            transform.translation = at;
        }
    }
}

/// Pulls back as levels grow, as a graph still settling spreads them, so all of
/// them stay in view; never nearer, and not once the viewer has moved the camera.
fn keep_levels_in_view(explorer: Res<Explorer>, mut cameras: Query<&mut OrbitCamera>) {
    for mut camera in &mut cameras {
        if camera.free {
            continue;
        }
        if let Some(needed) = hourglass_distance(&explorer, &camera)
            && needed > camera.goal_distance().unwrap_or(camera.distance) * 1.02
            && let Some(at) = hourglass_center(&explorer)
        {
            camera.glide_to(at, needed);
        }
    }
}

/// Under the hourglass: a stem from each lifted node down to its place on the floor,
/// and the axis through the focus with a ring at each level.
fn draw_hourglass(explorer: Res<Explorer>, mut gizmos: Gizmos) {
    if !explorer.hourglass() {
        return;
    }
    let graph = explorer.graph();
    for tile in 0..explorer.map().tiles.len() {
        let (Some(at), Some(floor)) = (explorer.tile_point(tile), explorer.floor_point(tile))
        else {
            continue;
        };
        if (at.z - floor.z).abs() < 0.5 || !explorer.shown(tile) {
            continue;
        }
        let color = lane_color(explorer.tile_color(tile) % PALETTE.len());
        // From the node's surface, so the stem does not cross it.
        let surface = node_size(graph, tile) * 0.5 * (floor.z - at.z).signum();
        gizmos.line(at + Vec3::Z * surface, floor, color.with_alpha(0.35));
        gizmos
            .circle(
                Isometry3d::from_translation(floor),
                0.5,
                color.with_alpha(0.6),
            )
            .resolution(12);
    }
    let Some(focus) = explorer.graph_focus().and_then(|t| explorer.floor_point(t)) else {
        return;
    };
    let rung = explorer.level_height();
    let top = f32::from(DEEPEST_DRAWN) * rung;
    gizmos.line(
        focus + Vec3::Z * top,
        focus - Vec3::Z * top,
        Color::srgba(1.0, 1.0, 1.0, 0.12),
    );
    for step in 1..=DEEPEST_DRAWN {
        let height = f32::from(step) * rung;
        for (sign, color) in [(1.0, CALLER), (-1.0, CALLEE)] {
            gizmos
                .circle(
                    Isometry3d::from_translation(focus + Vec3::Z * height * sign),
                    1.2,
                    color.with_alpha(0.5),
                )
                .resolution(24);
        }
    }
}

/// Under the hourglass, the modules the focus and whatever is lifted live in.
pub(crate) fn lifted_clusters(explorer: &Explorer) -> Option<HashSet<usize>> {
    if !explorer.hourglass() {
        return None;
    }
    let graph = explorer.graph();
    Some(
        (0..explorer.map().tiles.len())
            .filter(|&t| explorer.level(t) != 0 || explorer.graph_focus() == Some(t))
            .filter_map(|t| graph.cluster(t))
            .collect(),
    )
}

/// Levels the axis through the focus marks.
const DEEPEST_DRAWN: u8 = 2;

/// What each level on the axis holds, for its name beside it: above the focus, then
/// below.
pub(crate) const LEVEL_NAMES: [(i8, &str); 4] = [
    (2, "calls its callers"),
    (1, "calls it"),
    (-1, "it calls"),
    (-2, "they call"),
];

/// Where each level's name goes: beside the axis through the focus, that level up.
pub(crate) fn level_marks(explorer: &Explorer) -> Vec<(i8, &'static str, Vec3)> {
    if !explorer.hourglass() {
        return Vec::new();
    }
    let Some(focus) = explorer.graph_focus().and_then(|t| explorer.floor_point(t)) else {
        return Vec::new();
    };
    let level = explorer.level_height();
    LEVEL_NAMES
        .iter()
        .map(|&(step, name)| (step, name, focus + Vec3::Z * f32::from(step) * level))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_all_the_way_and_then_rests() {
        let mut values = [0.0, 5.0];
        let targets = [10.0, 5.0];
        assert!(ease(&mut values, &targets, 0.1), "the first moves");
        assert!(values[0] > 0.0 && values[0] < 10.0, "part of the way");
        assert!((values[1] - 5.0).abs() < f32::EPSILON, "the second stays");
        for _ in 0..200 {
            ease(&mut values, &targets, 0.1);
        }
        assert!((values[0] - 10.0).abs() < f32::EPSILON, "arrives exactly");
        assert!(!ease(&mut values, &targets, 0.1), "and rests");
    }

    #[test]
    fn levels_stand_as_far_apart_as_the_neighborhood_is_wide() {
        let map = [[0.0, 0.0, 0.0], [400.0, 100.0, 0.0]];
        let near = level_height(&map, &[20.0, 30.0, 40.0]);
        let far = level_height(&map, &[80.0, 120.0, 160.0]);
        assert!(
            near < far,
            "a wider neighborhood stands taller: {near} {far}"
        );
        assert!(
            level_height(&map, &[1_000.0]) <= 100.0,
            "never over a quarter of the map"
        );
        assert!((level_height(&map, &[1.0]) - MIN_LEVEL).abs() < f32::EPSILON);
        assert!((level_height(&[], &[]) - MIN_LEVEL).abs() < f32::EPSILON);
    }
}
