//! Depth cues for the timeline's 3D view, so it reads as space at a glance: the map
//! stands on a lit, faintly gridded plane that fades out at its rim, the grid's lines
//! running away in perspective; each node a call is strung back to glows with a soft
//! pool of its color's light, brightest for the cursor's; time's rails and calls are
//! lit tubes, thickening towards the eye and greying into haze away from it; and the camera rocks a few degrees about
//! the map, so the calls standing out of it slide against it. `Shift Z` holds the rock
//! still. None of it shows on the map or from the side.

use std::f32::consts::TAU;

use bevy::{
    asset::RenderAssetUsages,
    picking::Pickable,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

use crate::{
    camera::OrbitCamera,
    explorer::Explorer,
    graph_view::{node_color, node_size},
    timeline::{Timeline, haze},
};

pub(crate) struct StagePlugin;

impl Plugin for StagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stage>()
            .init_resource::<Rock>()
            .init_gizmo_group::<GridGizmos>()
            .add_systems(Startup, spawn_stage)
            .add_systems(
                Update,
                (lay_plane, light_pools, lay_tubes, rock).after(crate::timeline::place_markers),
            );
    }
}

/// The map's grid, drawn thinner than links.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct GridGizmos;

/// The map's plane: a shade lighter than the background, so the map reads as lying on
/// something and the pools have somewhere to fall.
const SURFACE: Color = Color::srgb(0.13, 0.14, 0.175);

/// The plane under the map and the pools on it.
#[derive(Resource, Debug, Default)]
struct Stage {
    min: Vec2,
    max: Vec2,
    /// How far back from the map it lies.
    z: f32,
    laid: bool,
    material: Handle<StandardMaterial>,
    spot: Handle<Image>,
    quad: Handle<Mesh>,
    /// One per node lit, spare ones hidden.
    pools: Vec<(Entity, Handle<StandardMaterial>)>,
    tube: Option<Handle<Mesh>>,
    /// One per line, spare ones hidden.
    tubes: Vec<(Entity, Handle<StandardMaterial>)>,
}

#[derive(Component)]
struct Plane;

#[derive(Component)]
struct Pool;

fn spawn_stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut stage: ResMut<Stage>,
    mut store: ResMut<GizmoConfigStore>,
) {
    store.config_mut::<GridGizmos>().0.line.width = 1.0;
    stage.quad = meshes.add(Rectangle::new(1.0, 1.0));
    stage.spot = images.add(spot_image(64));
    let material = materials.add(StandardMaterial {
        base_color: SURFACE.with_alpha(0.0),
        base_color_texture: Some(images.add(plane_image(128))),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        // Its own rim fades it; fog over a map far back would take it away.
        fog_enabled: false,
        ..default()
    });
    commands.spawn((
        Plane,
        Mesh3d(stage.quad.clone()),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        Visibility::Hidden,
        Pickable::IGNORE,
    ));
    stage.material = material;
}

// ---- the plane ----

/// How much of the plane's own shade shows at `(u, v)` across it, each from -1 to 1
/// edge to edge: all of it over the middle, falling away to none at its rim, so the
/// plane has no hard edge.
pub(crate) fn plane_glow(u: f32, v: f32) -> f32 {
    let rim = (u.powi(4) + v.powi(4)).sqrt().sqrt();
    let t = ((rim - 0.55) / 0.45).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// A square image `size` pixels across, each pixel's channels from `pixel` given its
/// place from -1 to 1 across.
fn square_image(size: u32, format: TextureFormat, pixel: impl Fn(f32, f32) -> [u8; 4]) -> Image {
    #[expect(clippy::cast_precision_loss, reason = "a small image")]
    let across = |i: u32| (i as f32 + 0.5) / size as f32 * 2.0 - 1.0;
    let mut data = Vec::new();
    for y in 0..size {
        for x in 0..size {
            data.extend(pixel(across(x), across(y)));
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        format,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// A share as a channel's value.
fn channel(share: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "within a channel"
    )]
    let value = (share.clamp(0.0, 1.0) * 255.0).round() as u8;
    value
}

/// The plane's shading: solid over the middle, clearing to nothing at the rim, so
/// it melts into the background whatever the renderer does to near-black.
fn plane_image(size: u32) -> Image {
    square_image(size, TextureFormat::Rgba8Unorm, |u, v| {
        [255, 255, 255, channel(plane_glow(u, v))]
    })
}

/// Grid lines across the plane, and the pieces each is drawn in so haze can fade it.
const GRID_LINES: usize = 18;
const GRID_PIECES: usize = 10;
/// How strongly the grid draws where nothing dims it.
const GRID: f32 = 0.14;

/// Fits the plane behind the journey's nodes, fades it in with the 3D view, and draws
/// its grid.
fn lay_plane(
    time: Res<Time>,
    explorer: Res<Explorer>,
    timeline: Res<Timeline>,
    mut stage: ResMut<Stage>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut planes: Query<(&mut Transform, &mut Visibility), With<Plane>>,
    mut gizmos: Gizmos<GridGizmos>,
) {
    let strength = timeline.depth_view();
    let Ok((mut transform, mut visibility)) = planes.single_mut() else {
        return;
    };
    // Round the nodes the calls come from, so on a big map it lights the part the
    // journey runs in and its rim stays in view.
    let points: Vec<Vec3> = timeline
        .calls
        .iter()
        .filter_map(|call| explorer.tile_point(call.tile))
        .collect();
    if strength < 0.01 || points.is_empty() {
        *visibility = Visibility::Hidden;
        stage.laid = false;
        return;
    }
    let (min, max, low) = points.iter().fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN), f32::MAX),
        |(min, max, low), p| (min.min(p.truncate()), max.max(p.truncate()), low.min(p.z)),
    );
    let margin = (max - min).max_element() * 0.45 + 8.0;
    let (min, max, z) = (min - margin, max + margin, low - 1.5);
    if stage.laid {
        let ease = 1.0 - (-time.delta_secs() * 3.0).exp();
        stage.min = stage.min.lerp(min, ease);
        stage.max = stage.max.lerp(max, ease);
        stage.z += (z - stage.z) * ease;
    } else {
        (stage.min, stage.max, stage.z, stage.laid) = (min, max, z, true);
    }
    let size = stage.max - stage.min;
    let middle = (stage.min + stage.max) / 2.0;
    *visibility = Visibility::Visible;
    *transform = Transform::from_translation(middle.extend(stage.z)).with_scale(size.extend(1.0));
    if let Some(mut material) = materials.get_mut(&stage.material) {
        material.base_color = SURFACE.with_alpha(strength);
    }
    let eye = timeline.eye();
    let range = timeline.haze_range();
    #[expect(clippy::cast_precision_loss, reason = "a few lines")]
    let step = size.max_element() / GRID_LINES as f32;
    let shade = |p: Vec3| {
        let across = (p.truncate() - middle) / (size / 2.0);
        let glow = plane_glow(across.x, across.y);
        Color::WHITE.with_alpha(GRID * strength * glow * (1.0 - 0.4 * haze(eye.distance(p), range)))
    };
    let mut line = |from: Vec3, to: Vec3| {
        #[expect(clippy::cast_precision_loss, reason = "a few pieces")]
        let at = |i: usize| from.lerp(to, i as f32 / GRID_PIECES as f32);
        for i in 0..GRID_PIECES {
            let (a, b) = (at(i), at(i + 1));
            gizmos.line_gradient(a, b, shade(a), shade(b));
        }
    };
    let z = stage.z + 0.01;
    for i in 0..=GRID_LINES {
        #[expect(clippy::cast_precision_loss, reason = "a few lines")]
        let offset = step * i as f32;
        let x = stage.min.x + offset;
        if x <= stage.max.x + 1e-3 {
            line(Vec3::new(x, stage.min.y, z), Vec3::new(x, stage.max.y, z));
        }
        let y = stage.min.y + offset;
        if y <= stage.max.y + 1e-3 {
            line(Vec3::new(stage.min.x, y, z), Vec3::new(stage.max.x, y, z));
        }
    }
}

// ---- pools ----

/// How opaque a soft round spot is `distance` from its middle, 1 being its edge.
pub(crate) fn spot_alpha(distance: f32) -> f32 {
    let t = (1.0 - distance).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A white spot fading out to its edge, `size` pixels across.
fn spot_image(size: u32) -> Image {
    square_image(size, TextureFormat::Rgba8UnormSrgb, |u, v| {
        [255, 255, 255, channel(spot_alpha(u.hypot(v)))]
    })
}

/// How brightly a node's pool glows, given how many calls from the cursor's the
/// nearest of its calls is, of `count` calls: the cursor's in full, its neighbors'
/// less, the rest faintly, and fainter in a long journey, so a crowd of pools does not
/// run together.
pub(crate) fn pool_share(away: Option<usize>, count: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let crowd = (12.0 / count.max(1) as f32).clamp(0.4, 1.0);
    match away {
        Some(0) => 1.0,
        Some(1..=2) => 0.65,
        _ => 0.5 * crowd,
    }
}

/// How bright a pool is at its middle, at most.
const POOL: f32 = 0.6;

/// Lights each node the calls are strung back to with a soft pool of its color, on
/// the map just in front of it: additive meshes, since egui would apply their alpha
/// twice.
fn light_pools(
    mut commands: Commands,
    explorer: Res<Explorer>,
    timeline: Res<Timeline>,
    mut stage: ResMut<Stage>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut drawn: Query<(&mut Transform, &mut Visibility), With<Pool>>,
) {
    let strength = timeline.depth_view();
    // Each node once, at its call nearest the cursor's.
    let mut lit: Vec<(usize, Option<usize>)> = Vec::new();
    if strength > 0.01 {
        for (index, call) in timeline.calls.iter().enumerate() {
            let away = timeline.cursor().map(|c| c.abs_diff(index));
            match lit.iter_mut().find(|(tile, _)| *tile == call.tile) {
                Some((_, best)) => *best = (*best).min(away),
                None => lit.push((call.tile, away)),
            }
        }
    }
    while stage.pools.len() < lit.len() {
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(stage.spot.clone()),
            alpha_mode: AlphaMode::Add,
            unlit: true,
            fog_enabled: false,
            ..default()
        });
        let entity = commands
            .spawn((
                Pool,
                Mesh3d(stage.quad.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
            ))
            .id();
        stage.pools.push((entity, material));
    }
    let eye = timeline.eye();
    let range = timeline.haze_range();
    let graph = explorer.graph();
    for (i, (entity, material)) in stage.pools.iter().enumerate() {
        let Ok((mut place, mut visibility)) = drawn.get_mut(*entity) else {
            continue;
        };
        let Some((tile, home)) = lit
            .get(i)
            .and_then(|&(tile, _)| Some((tile, explorer.tile_point(tile)?)))
        else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let away = lit[i].1;
        let cursor = away == Some(0);
        // Wider than the node, and than its call's ball, however small the node.
        let ball = timeline
            .calls_of(tile)
            .map(|index| timeline.marker_size(&explorer, index))
            .fold(0.0, f32::max);
        let size = node_size(graph, tile).max(ball * 0.8) * if cursor { 7.0 } else { 5.0 };
        // Behind the node, so it glows round it rather than over it.
        let at = home - Vec3::Z * node_size(graph, tile) * 0.6;
        *visibility = Visibility::Visible;
        *place = Transform::from_translation(at).with_scale(Vec3::new(size, size, 1.0));
        if let Some(mut material) = materials.get_mut(material) {
            let tint = node_color(&explorer, None, tile).mix(&Color::WHITE, 0.35);
            let glow = POOL
                * pool_share(away, timeline.calls.len())
                * strength
                * (1.0 - 0.4 * haze(eye.distance(at), range));
            material.base_color = tint.with_alpha(glow);
        }
    }
}

// ---- tubes ----

/// One of the timeline's lines as a tube in space.
#[derive(Component)]
struct Tube;

/// How thick tubes are, in pixels, at the point orbited: rails heavier than calls.
const RAIL: f32 = 4.0;
const CALL: f32 = 2.2;

/// In the 3D view, draws the timeline's rails and calls as lit tubes in space, so a
/// rail thickens towards the eye, thins away down time and greys into the haze; on
/// the map and from the side they are flat lines.
#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn lay_tubes(
    mut commands: Commands,
    explorer: Res<Explorer>,
    timeline: Res<Timeline>,
    mut stage: ResMut<Stage>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut drawn: Query<(&mut Transform, &mut Visibility), With<Tube>>,
    cameras: Query<(&Camera, &OrbitCamera)>,
) {
    let strength = timeline.depth_view();
    let lines = if strength > 0.01 {
        timeline.lines(&explorer)
    } else {
        Vec::new()
    };
    if stage.tube.is_none() {
        stage.tube = Some(meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(10)));
    }
    let mesh = stage.tube.clone().unwrap_or_default();
    while stage.tubes.len() < lines.len() {
        let material = materials.add(StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.4,
            ..default()
        });
        let entity = commands
            .spawn((
                Tube,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
            ))
            .id();
        stage.tubes.push((entity, material));
    }
    let Ok((camera, orbit)) = cameras.single() else {
        return;
    };
    // World units a pixel spans at the point orbited.
    let height = camera.logical_viewport_size().map_or(1000.0, |size| size.y);
    let pixel = 2.0 * orbit.distance * (crate::timeline::FIELD / 2.0).tan() / height;
    for (i, (entity, material)) in stage.tubes.iter().enumerate() {
        let Ok((mut place, mut visibility)) = drawn.get_mut(*entity) else {
            continue;
        };
        let Some(line) = lines.get(i) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let length = line.from.distance(line.to);
        if length < 1e-3 {
            *visibility = Visibility::Hidden;
            continue;
        }
        let radius = pixel * if line.rail { RAIL } else { CALL } / 2.0;
        *visibility = Visibility::Visible;
        *place = Transform::from_translation(line.from.midpoint(line.to))
            .with_rotation(Quat::from_rotation_arc(
                Vec3::Y,
                (line.to - line.from) / length,
            ))
            .with_scale(Vec3::new(radius, length, radius));
        let color = line
            .color
            .with_alpha((line.color.alpha() * 1.6).min(1.0) * strength);
        if let Some(mut material) = materials.get_mut(material)
            && material.base_color != color
        {
            material.base_color = color;
            material.emissive = (LinearRgba::from(color) * 0.5).with_alpha(1.0);
        }
    }
}

// ---- the rock ----

/// How far the camera rocks each way, in radians, and how long a cycle takes.
const ROCK: f32 = 0.045;
const ROCK_PERIOD: f32 = 7.0;

/// Whether the 3D view rocks the camera, and where in its rock to hold it.
#[derive(Resource, Debug, Default)]
pub(crate) struct Rock {
    /// Held still rather than rocking (`Shift Z`).
    pub(crate) still: bool,
    /// Held at this share of a cycle, for screenshots.
    pub(crate) held_at: Option<f32>,
}

/// How far the camera is turned at `phase` of its rock, a share of a cycle.
pub(crate) fn sway(phase: f32) -> f32 {
    ROCK * (phase * TAU).sin()
}

/// While the 3D view is up, rocks the camera gently from side to side about the map,
/// so the calls standing out of it slide against it; still while a hand moves the
/// camera, and held still by `Shift Z`.
fn rock(
    time: Res<Time>,
    settings: Res<Rock>,
    timeline: Res<Timeline>,
    mut amount: Local<f32>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    let on = if settings.still || camera.free {
        0.0
    } else {
        timeline.depth_view()
    };
    let ease = 1.0 - (-time.delta_secs() * 2.0).exp();
    *amount += (on - *amount) * ease;
    let phase = settings
        .held_at
        .unwrap_or(time.elapsed_secs() / ROCK_PERIOD);
    camera.sway = sway(phase) * *amount;
    // About the map, so it holds still and what stands out of it sways.
    camera.pivot = Vec3::NEG_Z * timeline.pan;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plane_fades_out_to_its_rim() {
        assert!(
            (plane_glow(0.0, 0.0) - 1.0).abs() < 1e-6,
            "full in the middle"
        );
        assert!((plane_glow(0.3, -0.2) - 1.0).abs() < 1e-6, "and round it");
        assert!(plane_glow(0.8, 0.0) < plane_glow(0.6, 0.0));
        assert!(plane_glow(1.0, 1.0) < 1e-6, "none at the rim");
    }

    #[test]
    fn a_spot_is_brightest_in_the_middle_and_gone_at_its_edge() {
        assert!((spot_alpha(0.0) - 1.0).abs() < 1e-6);
        assert!(spot_alpha(0.5) < 1.0 && spot_alpha(0.5) > 0.0);
        assert!(spot_alpha(1.0) < 1e-6);
    }

    #[test]
    fn the_cursors_pool_glows_brightest_and_a_crowd_dimmer() {
        assert!(pool_share(Some(0), 10) > pool_share(Some(1), 10));
        assert!(pool_share(Some(1), 10) > pool_share(Some(5), 10));
        assert!(pool_share(Some(5), 50) < pool_share(Some(5), 10));
        assert!(pool_share(None, 500) > 0.0, "never gone");
    }

    #[test]
    fn the_rock_swings_both_ways_and_back() {
        assert!(sway(0.0).abs() < 1e-6, "starts square");
        assert!((sway(0.25) - ROCK).abs() < 1e-6, "one end");
        assert!((sway(0.75) + ROCK).abs() < 1e-6, "the other");
        assert!(sway(1.0).abs() < 1e-5, "and back");
    }
}
