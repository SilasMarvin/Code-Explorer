//! The graph as terrain (Z): each container stands as a plateau on the one it sits in,
//! crate at the bottom, then module, nested module and type, and each function sits on
//! top of its own container's plateau. Height means one thing, how deep a function's
//! container is nested, so a module boundary is a cliff edge you can see, and a call
//! that leaves its module arcs over the edge to land on another plateau. The map keeps
//! its place across the ground; the camera looks at it from low down, as at a relief
//! map.

use std::collections::HashMap;

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use codeviz_ir::UnitId;
use codeviz_layout::{ForceGraph, Nesting};

use crate::{
    camera::{ANGLED, OrbitCamera},
    explorer::Explorer,
    graph_view::orbit_anchor,
    scene::{PALETTE, lane_color},
};

/// How much higher each level of container stands than the one it sits in.
pub(crate) const STEP: f32 = 2.4;
/// How far above its plateau a function floats, so its sphere sits on the surface.
const LIFT: f32 = 0.7;
/// The camera's turn and tilt over the terraces: low and from the south, turned a
/// little, so cliffs show their faces and the map's north recedes.
pub(crate) const RELIEF: (f32, f32) = (-0.32, 0.9);

/// How high each node stands: on top of its innermost container's plateau.
pub(crate) fn heights(graph: &ForceGraph) -> Vec<f32> {
    let nesting = graph.nesting();
    (0..nesting.paths.len())
        .map(|tile| {
            // Outside every container, on the ground under the outermost plateaus.
            let ground = nesting
                .container(tile)
                .map_or(-STEP, |n| top(nesting.nests[n].depth));
            ground + LIFT
        })
        .collect()
}

/// The height of the top of a plateau `depth` containers deep: an outermost one tops
/// out at zero.
#[expect(clippy::cast_precision_loss, reason = "a small depth")]
fn top(depth: usize) -> f32 {
    depth as f32 * STEP
}

/// One container's plateau: its outline, how high it stands, and what it looks like.
#[derive(Debug, Clone)]
pub(crate) struct Plateau {
    /// The container.
    pub(crate) unit: UnitId,
    /// The functions standing on it, or on what stands on it.
    pub(crate) tiles: Vec<usize>,
    /// Its edge, counter-clockwise, on the ground.
    pub(crate) outline: Vec<Vec2>,
    pub(crate) bottom: f32,
    pub(crate) top: f32,
    pub(crate) look: Ground,
}

impl Plateau {
    /// How far across it is, at its widest.
    pub(crate) fn extent(&self) -> f32 {
        let (min, max) = self.outline.iter().fold(
            (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
            |(min, max), &p| (min.min(p), max.max(p)),
        );
        (max - min).max_element().max(0.0)
    }
}

/// What a plateau is filled with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Ground {
    /// A module, in its color.
    Module(usize),
    /// A type in a module, a lighter shade of the module's color.
    Type(usize),
    /// A crate, package or a module holding only modules: neutral, lighter the
    /// deeper it is.
    Bare(usize),
}

impl Ground {
    fn color(self) -> Color {
        let slate = LinearRgba::from(Color::srgb(0.14, 0.16, 0.21));
        let mix = |color: Color, share: f32| {
            let c = LinearRgba::from(color);
            Color::from(LinearRgba::rgb(
                slate.red + (c.red - slate.red) * share,
                slate.green + (c.green - slate.green) * share,
                slate.blue + (c.blue - slate.blue) * share,
            ))
        };
        match self {
            Self::Module(cluster) => mix(lane_color(cluster % PALETTE.len()), 0.3),
            Self::Type(cluster) => mix(lane_color(cluster % PALETTE.len()), 0.5),
            #[expect(clippy::cast_precision_loss, reason = "a small depth")]
            Self::Bare(depth) => mix(Color::srgb(0.5, 0.55, 0.66), 0.06 * depth as f32),
        }
    }

    /// The color of the plateau's rim, its boundary.
    pub(crate) fn rim(self) -> Color {
        match self {
            Self::Module(cluster) | Self::Type(cluster) => {
                lane_color(cluster % PALETTE.len()).with_alpha(0.75)
            }
            Self::Bare(_) => Color::srgba(0.62, 0.68, 0.8, 0.45),
        }
    }
}

/// Room around a container's functions to its edge: wider the more containers it
/// holds, so each plateau sits inside the one under it.
fn margin(levels_inside: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let levels = levels_inside as f32;
    2.6 + 1.6 * levels
}

/// Every container's plateau, from the functions that show where the graph puts them.
pub(crate) fn plateaus(explorer: &Explorer) -> Vec<Plateau> {
    let graph = explorer.graph();
    let nesting = graph.nesting();
    let mut members: Vec<Vec<Vec2>> = vec![Vec::new(); nesting.nests.len()];
    let mut tiles: Vec<Vec<usize>> = vec![Vec::new(); nesting.nests.len()];
    for (tile, path) in nesting.paths.iter().enumerate() {
        let Some(at) = explorer.tile_point(tile) else {
            continue;
        };
        for &nest in path {
            members[nest].push(at.truncate());
            tiles[nest].push(tile);
        }
    }
    let inside = levels_inside(nesting);
    let cluster_of = clusters(graph);
    let mut holds_modules = vec![false; nesting.nests.len()];
    for (n, nest) in nesting.nests.iter().enumerate() {
        if let (Some(parent), Some(&(_, true))) = (nest.parent, cluster_of.get(&n)) {
            holds_modules[parent] = true;
        }
    }
    let mut plateaus: Vec<Plateau> = nesting
        .nests
        .iter()
        .enumerate()
        .filter(|(n, nest)| !nest.loose && !members[*n].is_empty())
        .map(|(n, nest)| {
            let corners = hull(&members[n]);
            let outline = hull(&rounded(&corners, margin(inside[n])));
            let look = match cluster_of.get(&n) {
                // A module holding modules is ground for them, whatever it holds itself.
                Some(&(cluster, true)) if !holds_modules[n] => Ground::Module(cluster),
                Some(&(cluster, false)) => Ground::Type(cluster),
                _ => Ground::Bare(nest.depth),
            };
            Plateau {
                unit: nest.unit.clone(),
                tiles: std::mem::take(&mut tiles[n]),
                outline,
                bottom: if nest.depth == 0 {
                    -STEP
                } else {
                    top(nest.depth - 1)
                },
                top: top(nest.depth),
                look,
            }
        })
        .collect();
    // Lowest first, so what stands on a plateau draws after it.
    plateaus.sort_by(|a, b| a.top.total_cmp(&b.top));
    plateaus
}

/// For each nest, how many levels of container it holds.
fn levels_inside(nesting: &Nesting) -> Vec<usize> {
    let mut inside = vec![0; nesting.nests.len()];
    let mut order: Vec<usize> = (0..nesting.nests.len()).collect();
    order.sort_by_key(|&n| std::cmp::Reverse(nesting.nests[n].depth));
    for n in order {
        let nest = &nesting.nests[n];
        if nest.loose {
            continue;
        }
        if let Some(parent) = nest.parent {
            inside[parent] = inside[parent].max(inside[n] + 1);
        }
    }
    inside
}

/// For each nest that is a module cluster or sits in one, the cluster, and whether
/// the nest is the module itself rather than a type inside it.
fn clusters(graph: &ForceGraph) -> HashMap<usize, (usize, bool)> {
    let nesting = graph.nesting();
    let mut found = HashMap::new();
    for (tile, path) in nesting.paths.iter().enumerate() {
        let Some(cluster) = graph.cluster(tile) else {
            continue;
        };
        let Some(module) = graph.cluster_unit(cluster) else {
            continue;
        };
        let mut within = false;
        for &n in path {
            let nest = &nesting.nests[n];
            if nest.loose {
                continue;
            }
            if &nest.unit == module {
                within = true;
                found.insert(n, (cluster, true));
            } else if within {
                found.entry(n).or_insert((cluster, false));
            }
        }
    }
    found
}

/// The convex hull of `points`, counter-clockwise.
fn hull(points: &[Vec2]) -> Vec<Vec2> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    sorted.dedup_by(|a, b| a.distance_squared(*b) < 1e-6);
    if sorted.len() < 3 {
        return sorted;
    }
    let turn = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut lower: Vec<Vec2> = Vec::new();
    for &p in &sorted {
        while lower.len() >= 2 && turn(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<Vec2> = Vec::new();
    for &p in sorted.iter().rev() {
        while upper.len() >= 2 && turn(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Points round each of `corners` at `radius`, whose hull is the corners' hull grown
/// by `radius` with rounded corners.
fn rounded(corners: &[Vec2], radius: f32) -> Vec<Vec2> {
    const SIDES: u16 = 20;
    corners
        .iter()
        .flat_map(|&c| {
            (0..SIDES).map(move |i| {
                let angle = f32::from(i) / f32::from(SIDES) * std::f32::consts::TAU;
                c + Vec2::from_angle(angle) * radius
            })
        })
        .collect()
}

/// A plateau as a solid: its top, and walls down to what it stands on.
fn solid(plateau: &Plateau) -> Mesh {
    let outline = &plateau.outline;
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let middle = outline.iter().copied().sum::<Vec2>() / outline.len().max(1) as f32;
    let index = |positions: &Vec<[f32; 3]>| u32::try_from(positions.len()).unwrap_or(u32::MAX);
    // The top, as a fan round its middle: the outline is convex.
    let center = index(&positions);
    positions.push([middle.x, middle.y, plateau.top]);
    normals.push([0.0, 0.0, 1.0]);
    for point in outline {
        positions.push([point.x, point.y, plateau.top]);
        normals.push([0.0, 0.0, 1.0]);
    }
    let count = u32::try_from(outline.len()).unwrap_or(0);
    for i in 0..count {
        indices.extend([center, center + 1 + i, center + 1 + (i + 1) % count]);
    }
    // The walls, each side its own face so it shades flat.
    for (i, &a) in outline.iter().enumerate() {
        let b = outline[(i + 1) % outline.len()];
        let out = (b - a).perp().normalize_or_zero() * -1.0;
        let base = index(&positions);
        for (p, z) in [
            (a, plateau.bottom),
            (b, plateau.bottom),
            (b, plateau.top),
            (a, plateau.top),
        ] {
            positions.push([p.x, p.y, z]);
            normals.push([out.x, out.y, 0.0]);
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

/// A plateau standing in the scene.
#[derive(Component, Debug)]
pub(crate) struct Slab;

/// The plateaus as last built, and when: the nodes' moves and whether terraced.
#[derive(Resource, Debug, Default)]
pub(crate) struct Terrain {
    pub(crate) plateaus: Vec<Plateau>,
    built: Option<u64>,
}

/// Rebuilds the plateaus whenever the nodes move, and clears them when the graph lies
/// flat.
pub(crate) fn build_terrain(
    mut commands: Commands,
    explorer: Res<Explorer>,
    mut terrain: ResMut<Terrain>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut grounds: Local<HashMap<Ground, Handle<StandardMaterial>>>,
    slabs: Query<(Entity, &Mesh3d), With<Slab>>,
) {
    let wanted = explorer.terraced().then(|| explorer.moved());
    if terrain.built == wanted {
        return;
    }
    terrain.built = wanted;
    for (entity, mesh) in &slabs {
        meshes.remove(&mesh.0);
        commands.entity(entity).despawn();
    }
    terrain.plateaus = if explorer.terraced() {
        plateaus(&explorer)
    } else {
        Vec::new()
    };
    for plateau in &terrain.plateaus {
        let material = grounds
            .entry(plateau.look)
            .or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color: plateau.look.color(),
                    perceptual_roughness: 0.95,
                    reflectance: 0.1,
                    ..default()
                })
            })
            .clone();
        commands.spawn((
            Slab,
            Mesh3d(meshes.add(solid(plateau))),
            MeshMaterial3d(material),
            Transform::default(),
            Pickable::IGNORE,
        ));
    }
}

/// The rim of each plateau, its boundary, in its container's color.
pub(crate) fn draw_rims(terrain: &Terrain, gizmos: &mut Gizmos) {
    for plateau in &terrain.plateaus {
        let z = plateau.top + 0.02;
        let rim = plateau.outline.iter().map(|p| p.extend(z));
        gizmos.linestrip(rim.clone().chain(rim.take(1)), plateau.look.rim());
    }
}

/// Where a call between `a` and `b` runs: along the plateau within one, else in an
/// arc over the edges between them, higher the further it goes.
pub(crate) fn course(a: Vec3, b: Vec3) -> Vec<Vec3> {
    let span = a.truncate().distance(b.truncate());
    if (a.z - b.z).abs() < 0.01 && span < 1.0 {
        return vec![a, b];
    }
    let rise = a.z.max(b.z) + 1.5 + span * 0.22;
    let control = ((a + b) / 2.0).truncate().extend(rise);
    let steps = 16_u16;
    (0..=steps)
        .map(|i| {
            let t = f32::from(i) / f32::from(steps);
            a * (1.0 - t) * (1.0 - t) + control * 2.0 * t * (1.0 - t) + b * t * t
        })
        .collect()
}

/// Whether a call should arc: it leaves its module's plateau.
pub(crate) fn arcs(explorer: &Explorer, from: usize, to: usize) -> bool {
    let graph = explorer.graph();
    let nesting = graph.nesting();
    nesting.container(from) != nesting.container(to)
}

/// Z stands the graph up as terraces or lays it flat again, turning the camera to suit.
pub(crate) fn terrace_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut explorer: ResMut<Explorer>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    if !keys.just_pressed(KeyCode::KeyZ) {
        return;
    }
    let terraced = !explorer.terraced();
    explorer.set_terraced(terraced);
    for mut camera in &mut cameras {
        pose(&explorer, &mut camera);
    }
}

/// Turns to look at the terraces as at a relief map, or back to the usual angle.
pub(crate) fn pose(explorer: &Explorer, camera: &mut OrbitCamera) {
    camera.turn_to(if explorer.terraced() { RELIEF } else { ANGLED });
    orbit_anchor(explorer, camera);
}

/// Poses the camera for terraces when the view opens on them.
pub(crate) fn open_posed(
    explorer: Res<Explorer>,
    mut done: Local<bool>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    if *done {
        return;
    }
    *done = true;
    if explorer.terraced() {
        for mut camera in &mut cameras {
            pose(&explorer, &mut camera);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hull_holds_every_point_and_keeps_only_corners() {
        let points = [
            Vec2::new(0.0, 0.0),
            Vec2::new(4.0, 0.0),
            Vec2::new(4.0, 3.0),
            Vec2::new(0.0, 3.0),
            Vec2::new(2.0, 1.0),
        ];
        let corners = hull(&points);
        assert_eq!(corners.len(), 4, "the middle point is not a corner");
        let area: f32 = (0..corners.len())
            .map(|i| corners[i].perp_dot(corners[(i + 1) % corners.len()]))
            .sum::<f32>()
            / 2.0;
        assert!((area - 12.0).abs() < 1e-4, "counter-clockwise, got {area}");
    }

    #[test]
    fn a_wider_margin_holds_a_narrower_one() {
        let corners = hull(&[Vec2::ZERO, Vec2::new(6.0, 0.0), Vec2::new(0.0, 5.0)]);
        let inner = hull(&rounded(&corners, margin(0)));
        let outer = hull(&rounded(&corners, margin(1)));
        let inside = |p: Vec2| {
            (0..outer.len())
                .all(|i| (outer[(i + 1) % outer.len()] - outer[i]).perp_dot(p - outer[i]) >= -1e-3)
        };
        assert!(inner.iter().all(|&p| inside(p)));
    }

    #[test]
    fn a_call_across_modules_rises_over_both_ends() {
        let (a, b) = (Vec3::new(0.0, 0.0, 2.0), Vec3::new(20.0, 0.0, 4.0));
        let path = course(a, b);
        assert_eq!(path.first(), Some(&a));
        assert_eq!(path.last(), Some(&b));
        let peak = path.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        assert!(peak > 4.0, "clears the higher end, got {peak}");
    }
}
