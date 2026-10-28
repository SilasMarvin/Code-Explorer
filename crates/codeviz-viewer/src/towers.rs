//! The towers view (`Z`, `--z towers`): the map lies down as the ground, and each
//! module stands on it as a tower where its region was, wider the more functions it
//! holds. Height is flow: a function stands on the floor of the longest chain of calls
//! from an entry point down to it, so entry points are at the top, everything a
//! function calls stands lower than it, and every call runs downhill except one that
//! closes a loop. The focus's tower and the towers of what calls it and what it calls
//! light up; the rest of the city stays dim.

use std::collections::HashMap;

use bevy::prelude::*;
use codeviz_layout::ForceGraph;

use crate::{
    camera::{ANGLED, OrbitCamera},
    explorer::Explorer,
    graph_view::{self, Relation, orbit_anchor, relation},
    scene::{PALETTE, lane_color},
};

/// Looking down on the city from the front, high enough to see the ground plan and
/// low enough that height still reads.
pub(crate) const TOWERS_VIEW: (f32, f32) = (-0.3, -0.32);

/// Room above a tower's top floor and below its bottom one.
const EAVES: f32 = 0.6;

/// Every node stood in its module's tower.
#[derive(Debug, Clone, Default)]
pub(crate) struct Towers {
    /// Where each node stands, by tile.
    points: Vec<Vec3>,
    /// Each node's floor: calls down from the top.
    ranks: Vec<u32>,
    /// One per module that has a node.
    pub(crate) modules: Vec<Tower>,
    /// How tall a floor is.
    pub(crate) floor: f32,
    /// The lowest floor's rank.
    pub(crate) deepest: u32,
}

/// One module, standing.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tower {
    pub(crate) cluster: usize,
    /// The middle of its footprint, on the ground.
    pub(crate) base: Vec3,
    pub(crate) radius: f32,
    /// Heights of its lowest and highest floors.
    pub(crate) bottom: f32,
    pub(crate) top: f32,
}

impl Towers {
    /// Stands the graph up: each module where the flat map put it, each function on the
    /// floor its calls give it. `entries` go first in deciding which way a loop runs.
    pub(crate) fn raise(graph: &ForceGraph, entries: &[usize]) -> Self {
        let count = graph.positions.len();
        let ranks = flow_ranks(count, &graph.calls, entries);
        let deepest = ranks.iter().copied().max().unwrap_or(0);
        let flat: Vec<Vec2> = graph
            .positions
            .iter()
            .map(|&[x, y, _]| Vec2::new(x, y))
            .collect();
        let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
        for tile in 0..count {
            members
                .entry(graph.cluster(tile).unwrap_or_default())
                .or_default()
                .push(tile);
        }
        let middle = flat.iter().copied().sum::<Vec2>() / count.max(1) as f32;
        let extent = flat.iter().map(|p| p.distance(middle)).fold(10.0, f32::max);
        // The city about as tall as the map reaches from its middle, a floor never so
        // thin that its nodes touch or so tall that a few floors tower over the map.
        #[expect(clippy::cast_precision_loss, reason = "a small count of floors")]
        let floor = (extent / (deepest + 1) as f32).clamp(3.0, 20.0);
        let height = |rank: u32| floor_height(floor, deepest, rank);
        let ground = |p: Vec2| Vec3::new(p.x, 0.0, -p.y);
        let mut points = vec![Vec3::ZERO; count];
        let mut modules: Vec<Tower> = members
            .into_iter()
            .map(|(cluster, tiles)| {
                let center = tiles.iter().map(|&t| flat[t]).sum::<Vec2>() / tiles.len() as f32;
                let mut spread: Vec<f32> =
                    tiles.iter().map(|&t| flat[t].distance(center)).collect();
                spread.sort_by(f32::total_cmp);
                let spread = spread[(spread.len() * 9 / 10).min(spread.len() - 1)].max(1.0);
                #[expect(clippy::cast_precision_loss, reason = "a module's size")]
                let radius = (1.8 + (tiles.len() as f32).sqrt() * 1.2).min(spread * 0.85 + 1.8);
                // Each function keeps its bearing from the module's middle, drawn in
                // to fit the tower.
                for &tile in &tiles {
                    let offset = (flat[tile] - center) / spread * (radius - 1.0);
                    let offset = offset.clamp_length_max(radius - 1.0);
                    points[tile] = ground(center + offset) + Vec3::Y * height(ranks[tile]);
                }
                let highest = tiles.iter().map(|&t| ranks[t]).min().unwrap_or(0);
                let lowest = tiles.iter().map(|&t| ranks[t]).max().unwrap_or(0);
                Tower {
                    cluster,
                    base: ground(center),
                    radius,
                    bottom: height(lowest) - floor * EAVES,
                    top: height(highest) + floor * EAVES,
                }
            })
            .collect();
        modules.sort_by_key(|t| t.cluster);
        Self {
            points,
            ranks,
            modules,
            floor,
            deepest,
        }
    }

    /// Where a node stands.
    pub(crate) fn point(&self, tile: usize) -> Option<Vec3> {
        self.points.get(tile).copied()
    }

    /// The floor a node stands on, counted down from the top.
    pub(crate) fn rank(&self, tile: usize) -> Option<u32> {
        self.ranks.get(tile).copied()
    }

    /// How high a floor stands.
    pub(crate) fn height(&self, rank: u32) -> f32 {
        floor_height(self.floor, self.deepest, rank)
    }

    /// A module's tower.
    pub(crate) fn tower(&self, cluster: usize) -> Option<&Tower> {
        self.modules.iter().find(|t| t.cluster == cluster)
    }
}

/// How high the floor `rank` calls down from the top stands, the lowest one floor
/// off the ground.
fn floor_height(floor: f32, deepest: u32, rank: u32) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a small count of floors")]
    let up = deepest.saturating_sub(rank) as f32;
    (up + 1.0) * floor
}

/// Each node's floor: the most calls on any path down to it from a node nothing calls
/// (or an entry), so a caller always stands above what it calls. Calls that close a
/// loop are left out, found by walking depth first from `entries`, then from what
/// nothing calls, then from whatever is left.
pub(crate) fn flow_ranks(count: usize, calls: &[(usize, usize)], entries: &[usize]) -> Vec<u32> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut called = vec![false; count];
    for &(from, to) in calls {
        if from != to && from < count && to < count {
            out[from].push(to);
            called[to] = true;
        }
    }
    for list in &mut out {
        list.sort_unstable();
        list.dedup();
    }
    // Depth first, keeping only calls that do not lead back up the path walked.
    let starts = entries
        .iter()
        .copied()
        .filter(|&t| t < count)
        .chain((0..count).filter(|&t| !called[t]))
        .chain(0..count);
    // 0 unvisited, 1 on the path, 2 done.
    let mut state = vec![0u8; count];
    let mut forward: Vec<Vec<usize>> = vec![Vec::new(); count];
    for start in starts {
        if state[start] != 0 {
            continue;
        }
        state[start] = 1;
        let mut stack = vec![(start, 0usize)];
        while let Some(&(node, next)) = stack.last() {
            if let Some(&to) = out[node].get(next) {
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                match state[to] {
                    0 => {
                        forward[node].push(to);
                        state[to] = 1;
                        stack.push((to, 0));
                    }
                    2 => forward[node].push(to),
                    _ => {}
                }
            } else {
                state[node] = 2;
                stack.pop();
            }
        }
    }
    // Longest path over what is left, which no longer loops.
    let mut waiting = vec![0usize; count];
    for list in &forward {
        for &to in list {
            waiting[to] += 1;
        }
    }
    let mut ranks = vec![0u32; count];
    let mut ready: Vec<usize> = (0..count).filter(|&t| waiting[t] == 0).collect();
    while let Some(node) = ready.pop() {
        for &to in &forward[node] {
            ranks[to] = ranks[to].max(ranks[node] + 1);
            waiting[to] -= 1;
            if waiting[to] == 0 {
                ready.push(to);
            }
        }
    }
    ranks
}

/// How brightly a module's tower shows: the focus's brightest, then those holding what
/// calls it or what it calls, then those holding a function two calls away.
pub(crate) fn tower_light(explorer: &Explorer, cluster: usize) -> f32 {
    let graph = explorer.graph();
    let focus = explorer.graph_focus();
    if focus.and_then(|t| graph.cluster(t)) == Some(cluster) {
        return 1.0;
    }
    let Some(reach) = explorer.reach() else {
        return 0.35;
    };
    let mut light: f32 = 0.12;
    for tile in reach.upstream.keys().chain(reach.downstream.keys()) {
        if graph.cluster(*tile) != Some(cluster) || !explorer.shown(*tile) {
            continue;
        }
        light = light.max(match graph_view::near(explorer, reach, *tile) {
            Some(1) => 0.75,
            Some(2) => 0.4,
            _ => continue,
        });
    }
    light
}

/// Whether a node is one call from the focus, either way.
fn next_door(explorer: &Explorer, tile: usize) -> bool {
    explorer.reach().is_some_and(|reach| {
        matches!(
            relation(reach, tile),
            Some(Relation::Upstream(1) | Relation::Downstream(1))
        )
    })
}

/// Each tower as a wireframe in its module's color: a ring at its top and bottom
/// floors and its walls between, on a faint footprint on the ground. A lit tower marks
/// the floor of the focus and of each function next to it with a ring.
pub(crate) fn draw(explorer: &Explorer, gizmos: &mut Gizmos) {
    let Some(towers) = explorer.towers() else {
        return;
    };
    let flat = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let mut floors: HashMap<(usize, u32), bool> = HashMap::new();
    let focus = explorer.graph_focus();
    for tile in 0..explorer.map().tiles.len() {
        let is_focus = focus == Some(tile);
        if !is_focus && !next_door(explorer, tile) {
            continue;
        }
        if let (Some(cluster), Some(rank)) = (explorer.graph().cluster(tile), towers.rank(tile)) {
            *floors.entry((cluster, rank)).or_default() |= is_focus;
        }
    }
    for tower in &towers.modules {
        let light = tower_light(explorer, tower.cluster);
        let color = lane_color(tower.cluster % PALETTE.len());
        let ring = |gizmos: &mut Gizmos, height: f32, alpha: f32| {
            gizmos
                .circle(
                    Isometry3d::new(tower.base + Vec3::Y * height, flat),
                    tower.radius,
                    color.with_alpha(alpha),
                )
                .resolution(40);
        };
        // Its footprint where the map has the module, and its lowest floor.
        ring(gizmos, 0.0, 0.06 + 0.3 * light);
        ring(gizmos, tower.bottom, 0.04 + 0.3 * light);
        ring(gizmos, tower.top, 0.1 + 0.8 * light);
    }
    for (&(cluster, rank), &is_focus) in &floors {
        let Some(tower) = towers.tower(cluster) else {
            continue;
        };
        let height = towers.height(rank);
        let color = if is_focus {
            Color::WHITE.with_alpha(0.7)
        } else {
            lane_color(cluster % PALETTE.len()).with_alpha(0.6)
        };
        gizmos
            .circle(
                Isometry3d::new(tower.base + Vec3::Y * height, flat),
                tower.radius,
                color,
            )
            .resolution(32);
    }
}

/// A tower's glass walls, for the module cluster it stands for, at a share of the
/// tower's light.
#[derive(Component, Debug)]
pub(crate) struct Body {
    cluster: usize,
    share: f32,
}

/// How much of a tower's light its shaft below its lowest floor gets.
const SHAFT: f32 = 0.35;

/// How opaque a tower's walls are at full light.
const GLASS: f32 = 0.16;

/// Puts up a glass cylinder for each tower whenever the towers move, and takes them
/// down when the graph lies flat. Clicks go through them to the nodes inside.
pub(crate) fn build_bodies(
    mut commands: Commands,
    explorer: Res<Explorer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    bodies: Query<Entity, With<Body>>,
    mut built: Local<Option<u64>>,
) {
    let wanted = explorer.towers().map(|_| explorer.moved());
    if *built == wanted {
        return;
    }
    *built = wanted;
    for body in &bodies {
        commands.entity(body).despawn();
    }
    let Some(towers) = explorer.towers() else {
        return;
    };
    let mesh = meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(40));
    for tower in &towers.modules {
        let light = tower_light(&explorer, tower.cluster);
        // The whole shaft from the ground, faint, and the floors its functions stand
        // on, clearer.
        for (share, low) in [(SHAFT, 0.0), (1.0, tower.bottom)] {
            let tall = (tower.top - low).max(0.1);
            commands.spawn((
                Body {
                    cluster: tower.cluster,
                    share,
                },
                Mesh3d(mesh.clone()),
                MeshMaterial3d(materials.add(glass(tower.cluster, share, light))),
                Transform::from_translation(tower.base + Vec3::Y * (low + tall / 2.0))
                    .with_scale(Vec3::new(tower.radius, tall, tower.radius)),
                Pickable::IGNORE,
            ));
        }
    }
}

/// A tower's glass in its module's color: `share` of full for the part it covers,
/// fading fast as the tower's light drops.
fn glass(cluster: usize, share: f32, light: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: lane_color(cluster % PALETTE.len())
            .with_alpha(GLASS * share * (0.03 + 0.97 * light * light)),
        alpha_mode: AlphaMode::Add,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

/// Lights each tower's glass as [`tower_light`] says, whenever the focus moves.
pub(crate) fn light_bodies(
    explorer: Res<Explorer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    bodies: Query<(&Body, &MeshMaterial3d<StandardMaterial>)>,
) {
    if !explorer.is_changed() {
        return;
    }
    for (body, material) in &bodies {
        let light = tower_light(&explorer, body.cluster);
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.base_color = glass(body.cluster, body.share, light).base_color;
        }
    }
}

/// Where a module's name goes: just above its tower.
pub(crate) fn roof(explorer: &Explorer, cluster: usize) -> Option<Vec3> {
    let towers = explorer.towers()?;
    let tower = towers.tower(cluster)?;
    Some(tower.base + Vec3::Y * (tower.top + towers.floor * 0.3))
}

/// Z stands the graph up as towers, or lays it flat again.
pub(crate) fn toggle(keys: &ButtonInput<KeyCode>, explorer: &mut Explorer) {
    if keys.just_pressed(KeyCode::KeyZ) {
        let on = explorer.towers().is_none();
        explorer.set_towers(on);
    }
}

/// Turns the camera to look down on the city when the towers go up, and back to the
/// usual angle when they come down, around where the journey starts.
pub(crate) fn frame(
    explorer: Res<Explorer>,
    mut last: Local<bool>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let on = explorer.towers().is_some();
    if on == *last {
        return;
    }
    *last = on;
    for mut camera in &mut cameras {
        camera.turn_to(if on { TOWERS_VIEW } else { ANGLED });
        orbit_anchor(&explorer, &mut camera);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callers_stand_above_what_they_call() {
        // 0 → 1 → 2, and 0 → 2 directly: 2 is two calls down at most.
        let ranks = flow_ranks(3, &[(0, 1), (1, 2), (0, 2)], &[]);
        assert_eq!(ranks, vec![0, 1, 2]);
    }

    #[test]
    fn a_loop_runs_downhill_from_where_it_is_entered() {
        // 0 → 1 → 2 → 1: the call back up from 2 is the one left out.
        let ranks = flow_ranks(3, &[(0, 1), (1, 2), (2, 1)], &[0]);
        assert_eq!(ranks, vec![0, 1, 2]);
        // A loop nothing enters still stands, from the entry given.
        let ranks = flow_ranks(2, &[(0, 1), (1, 0)], &[1]);
        assert_eq!(ranks, vec![1, 0]);
    }

    #[test]
    fn nothing_calls_the_top_floor() {
        let ranks = flow_ranks(4, &[(0, 2), (1, 2), (2, 3)], &[]);
        assert_eq!(ranks[0], 0);
        assert_eq!(ranks[1], 0);
        assert_eq!(ranks[3], 2);
    }
}
