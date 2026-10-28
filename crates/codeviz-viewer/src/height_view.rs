//! Drawing height: with `Z` on, the graph stands upright with each node at its rank in
//! the flow (see [`crate::height`]). What is lit round the focus stands on a faint stalk
//! down to a floor just under it, where each module lies as a ring, its shadow, so where
//! a node sits on the map reads apart from how high it stands. A faint line runs across
//! at each rank, level with the focus and named at its left end, to read heights
//! against.

use bevy::prelude::*;

use crate::{
    camera::{ANGLED, MAP_UP, OrbitCamera, TILTED, height_up},
    explorer::Explorer,
    graph_view::{NodeLook, node_color, node_look, orbit_anchor},
};

/// How far under the lowest lit node the floor lies, in ranks.
const FLOOR_GAP: f32 = 0.6;
/// How far the rank lines run past the lit nodes, left and right.
const RULE_LEFT: f32 = 16.0;
const RULE_RIGHT: f32 = 4.0;

/// What height draws round the focus.
#[derive(Debug, Clone)]
pub(crate) struct Stage {
    /// The nodes lit round the focus, with how strongly to draw each one's stalk.
    pub(crate) lit: Vec<(usize, f32)>,
    /// The height of the highest of them.
    pub(crate) top: f32,
    /// The floor under them.
    pub(crate) floor: f32,
    /// The ranks they stand at, the focus's included, shallowest first.
    pub(crate) ranks: Vec<u32>,
    /// The focus's rank.
    pub(crate) focus_rank: u32,
    /// Where the rank lines run: from where to where across, and how far back, level
    /// with the focus so it sits on its own line.
    pub(crate) rule: (f32, f32, f32),
}

impl Stage {
    /// Where a rank's line starts, at its left end, and ends.
    pub(crate) fn rung(&self, explorer: &Explorer, rank: u32) -> (Vec3, Vec3) {
        let lift = explorer.lift();
        let z = lift.rank_height(rank) * lift.share();
        let (start, end, back) = self.rule;
        (Vec3::new(start, back, z), Vec3::new(end, back, z))
    }
}

/// What height draws round the focus, while it shows any.
pub(crate) fn stage(explorer: &Explorer) -> Option<Stage> {
    let height = explorer.lift();
    if !height.lifted() {
        return None;
    }
    let focus = explorer.graph_focus()?;
    let focus_rank = height.rank(focus)?;
    let lit: Vec<(usize, f32)> = (0..explorer.map().tiles.len())
        .filter_map(|tile| {
            let strength = match node_look(explorer, tile) {
                NodeLook::Focus => 0.45,
                NodeLook::Near(_, 1) => 0.3,
                NodeLook::Near(..) => 0.16,
                _ => return None,
            };
            Some((tile, strength))
        })
        .collect();
    let points: Vec<Vec3> = lit
        .iter()
        .filter_map(|&(tile, _)| explorer.tile_point(tile))
        .collect();
    let fold = |pick: fn(Vec3) -> f32, f: fn(f32, f32) -> f32, from: f32| {
        points.iter().map(|&p| pick(p)).fold(from, f)
    };
    let top = fold(|p| p.z, f32::max, f32::MIN);
    let bottom = fold(|p| p.z, f32::min, f32::MAX);
    let mut ranks: Vec<u32> = lit
        .iter()
        .filter_map(|&(tile, _)| height.rank(tile))
        .collect();
    ranks.sort_unstable();
    ranks.dedup();
    let step = height.rank_height(0) - height.rank_height(1);
    Some(Stage {
        lit,
        top,
        floor: bottom - step * FLOOR_GAP * height.share(),
        ranks,
        focus_rank,
        rule: (
            fold(|p| p.x, f32::min, f32::MAX) - RULE_LEFT,
            fold(|p| p.x, f32::max, f32::MIN) + RULE_RIGHT,
            explorer.tile_point(focus)?.y,
        ),
    })
}

/// The stalks under what is lit, and a line across it at each rank.
pub(crate) fn draw(explorer: &Explorer, gizmos: &mut Gizmos) {
    let Some(stage) = stage(explorer) else {
        return;
    };
    let share = explorer.lift().share();
    for &(tile, strength) in &stage.lit {
        let Some(at) = explorer.tile_point(tile) else {
            continue;
        };
        let color = node_color(explorer, None, tile);
        let foot = at.with_z(stage.floor);
        gizmos.line(at, foot, color.with_alpha(strength * share));
        gizmos
            .circle(
                Isometry3d::from_translation(foot),
                0.5,
                color.with_alpha(0.6 * share),
            )
            .resolution(12);
    }
    for &rank in &stage.ranks {
        let (left, right) = stage.rung(explorer, rank);
        let alpha = if rank == stage.focus_rank { 0.32 } else { 0.12 };
        gizmos.line(left, right, Color::srgba(0.7, 0.74, 0.84, alpha * share));
    }
}

/// Stands the graph upright when height comes on, turned to read it, and lays it back
/// down when it goes off; once nodes reach their heights, refits the view round them.
pub(crate) fn pose(
    explorer: Res<Explorer>,
    mut last: Local<Option<(bool, bool)>>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let lift = explorer.lift();
    let upright = lift.by().is_some();
    let settled = lift.settled();
    let Some((was_upright, was_settled)) = last.replace((upright, settled)) else {
        // Opening upright, the camera starts turned to read it; flat, it is left be.
        if upright {
            for mut camera in &mut cameras {
                camera.stand(height_up());
                camera.turn_to(TILTED);
                orbit_anchor(&explorer, &mut camera);
            }
        }
        return;
    };
    for mut camera in &mut cameras {
        if upright != was_upright {
            camera.stand(if upright { height_up() } else { MAP_UP });
            camera.turn_to(if upright { TILTED } else { ANGLED });
        }
        if settled && !was_settled && !camera.free {
            orbit_anchor(&explorer, &mut camera);
        }
    }
}

/// Whether the call from `from` to `to` runs uphill: into a function ranked above its
/// caller, as a callback or a call into an entry point does.
pub(crate) fn uphill(explorer: &Explorer, from: usize, to: usize) -> bool {
    let lift = explorer.lift();
    lift.lifted() && lift.rank(to) < lift.rank(from)
}

/// How the camera turns to read height.
pub(crate) fn tilted() -> Quat {
    height_up() * Quat::from_euler(EulerRot::YXZ, TILTED.0, TILTED.1, 0.0)
}

/// What a rank's line on the wall is called.
pub(crate) fn rank_name(rank: u32) -> String {
    match rank {
        0 => "entry points".to_owned(),
        1 => "1 call down".to_owned(),
        n => format!("{n} calls down"),
    }
}
