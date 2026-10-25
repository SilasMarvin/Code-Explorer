//! Depth cues for the two floors, so a gentle tilt shows the deck floating over the
//! map without the camera turning steeply. The map gets a lit floor, and a soft pool
//! of light where each string lands, in its node's color; haze thickens with distance
//! from the eye, so the far side of the map recedes; balls are lit spheres, nearer
//! ones bigger; and the camera rocks a few degrees from side to side, so the deck
//! slides against the map under it. `Shift Z` holds it still.

use std::f32::consts::TAU;

use bevy::{
    asset::RenderAssetUsages,
    picking::Pickable,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use bevy_egui::egui;

use crate::{
    camera::OrbitCamera,
    explorer::Explorer,
    floors::Floors,
    graph_view::{node_color, node_size},
};

pub(crate) struct CuesPlugin;

impl Plugin for CuesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Floor>()
            .init_resource::<Rock>()
            .init_resource::<Footprints>()
            .init_gizmo_group::<FloorGizmos>()
            .add_systems(Startup, spawn_floor)
            .add_systems(
                Update,
                ((lay_floor, cast_footprints).chain(), haze_view, rock).after(crate::floors::raise),
            );
    }
}

/// The floor's grid, drawn thinner than links.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct FloorGizmos;

/// The window's background, which haze fades towards.
pub(crate) const BACKDROP: Color = Color::srgb(0.06, 0.07, 0.09);
/// The floor under the map: a shade lighter than the background, so the map reads as
/// lying on something and shadows have somewhere to fall.
const SURFACE: Color = Color::srgb(0.13, 0.14, 0.175);

// ---- the eye ----

/// Where the camera looks from, to measure how near things are.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Eye {
    at: Vec3,
    forward: Vec3,
    /// How far the point orbited is, against which nearness is measured.
    reach: f32,
}

impl Eye {
    pub(crate) fn of(transform: &GlobalTransform, reach: f32) -> Self {
        Self {
            at: transform.translation(),
            forward: transform.forward().into(),
            reach: reach.max(1e-3),
        }
    }

    /// How far in front of the eye `point` is.
    pub(crate) fn depth(&self, point: Vec3) -> f32 {
        (point - self.at).dot(self.forward)
    }

    /// How much bigger than at the point orbited something at `point` looks: above 1
    /// nearer, below 1 farther.
    pub(crate) fn nearness(&self, point: Vec3) -> f32 {
        self.reach / self.depth(point).max(self.reach * 0.05)
    }

    /// How much haze lies between the eye and `point`, at full strength.
    pub(crate) fn haze(&self, point: Vec3) -> f32 {
        haze(self.depth(point) / self.reach)
    }
}

/// Haze starts a little past the point orbited and thickens to its most farther out,
/// as a share of how far that point is.
const HAZE_NEAR: f32 = 0.75;
const HAZE_FAR: f32 = 1.5;
/// The most haze there is: the far map dims and greys but never vanishes.
const HAZE: f32 = 0.7;

/// How much haze lies in front of something at `share` of the point orbited's
/// distance.
pub(crate) fn haze(share: f32) -> f32 {
    ((share - HAZE_NEAR) / (HAZE_FAR - HAZE_NEAR)).clamp(0.0, 1.0) * HAZE
}

/// Haze on the scene's meshes, nodes and floor, while the upper floor is up.
fn haze_view(
    mut commands: Commands,
    floors: Res<Floors>,
    mut cameras: Query<(Entity, &OrbitCamera, Option<&mut DistanceFog>)>,
) {
    let lift = floors.lift();
    for (entity, camera, fog) in &mut cameras {
        let color = BACKDROP.with_alpha(HAZE * lift);
        let falloff = FogFalloff::Linear {
            start: camera.distance * HAZE_NEAR,
            end: camera.distance * HAZE_FAR,
        };
        match fog {
            Some(mut fog) => {
                fog.color = color;
                fog.falloff = falloff;
            }
            None => {
                commands.entity(entity).insert(DistanceFog {
                    color,
                    directional_light_color: Color::NONE,
                    directional_light_exponent: 8.0,
                    falloff,
                });
            }
        }
    }
}

// ---- the floor ----

/// The floor under the map, fitted round the nodes showing and eased as they move.
#[derive(Resource, Debug, Default)]
struct Floor {
    min: Vec2,
    max: Vec2,
    /// How deep under the map it lies.
    z: f32,
    laid: bool,
    material: Handle<StandardMaterial>,
}

#[derive(Component)]
struct Ground;

fn spawn_floor(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut floor: ResMut<Floor>,
    mut footprints: ResMut<Footprints>,
    mut store: ResMut<GizmoConfigStore>,
) {
    footprints.mesh = meshes.add(Rectangle::new(1.0, 1.0));
    footprints.spot = images.add(spot_image(64));
    store.config_mut::<FloorGizmos>().0.line.width = 1.0;
    let material = materials.add(StandardMaterial {
        base_color: BACKDROP,
        base_color_texture: Some(images.add(floor_image(128))),
        unlit: true,
        ..default()
    });
    commands.spawn((
        Ground,
        Mesh3d(meshes.add(Plane3d::new(Vec3::Z, Vec2::splat(0.5)))),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        Visibility::Hidden,
        Pickable::IGNORE,
    ));
    floor.material = material;
}

/// How much of the floor's own shade shows at `(u, v)` across it, each from -1 to 1
/// edge to edge: all of it over the middle, falling away to none at its rim, so the
/// floor has no hard edge.
pub(crate) fn floor_glow(u: f32, v: f32) -> f32 {
    let rim = (u.powi(4) + v.powi(4)).sqrt().sqrt();
    let t = ((rim - 0.55) / 0.45).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// What the floor's surface is scaled by at its rim: the background's share of it, in
/// linear light.
const FLOOR_RIM: f32 = 0.3;

/// The floor's shading, `size` pixels square: white over the middle, darkening to
/// the background towards the rim.
fn floor_image(size: u32) -> Image {
    let mut data = Vec::new();
    let across = |i: u32| (i as f32 + 0.5) / size as f32 * 2.0 - 1.0;
    for y in 0..size {
        for x in 0..size {
            let shade = FLOOR_RIM + (1.0 - FLOOR_RIM) * floor_glow(across(x), across(y));
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "within a channel"
            )]
            let value = (shade * 255.0).round() as u8;
            data.extend([value, value, value, 255]);
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
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Grid lines across the floor, and the pieces each is drawn in so haze can fade it.
const GRID_LINES: usize = 16;
const GRID_PIECES: usize = 10;
/// How strongly the grid draws where no haze dims it.
const GRID: f32 = 0.09;

/// Fits the floor under the nodes showing, fades it in with the upper floor, and draws
/// its grid.
#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn lay_floor(
    time: Res<Time>,
    explorer: Res<Explorer>,
    floors: Res<Floors>,
    mut floor: ResMut<Floor>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut grounds: Query<(&mut Transform, &mut Visibility), With<Ground>>,
    cameras: Query<(&GlobalTransform, &OrbitCamera)>,
    mut gizmos: Gizmos<FloorGizmos>,
) {
    let lift = floors.lift();
    let Ok((mut transform, mut visibility)) = grounds.single_mut() else {
        return;
    };
    let points: Vec<Vec3> = (0..explorer.map().tiles.len())
        .filter(|&tile| explorer.visibility(tile) >= 0.5)
        .filter_map(|tile| explorer.tile_point(tile))
        .collect();
    if lift < 0.01 || points.is_empty() {
        *visibility = Visibility::Hidden;
        floor.laid = false;
        return;
    }
    let (min, max, low) = points.iter().fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN), f32::MAX),
        |(min, max, low), p| (min.min(p.truncate()), max.max(p.truncate()), low.min(p.z)),
    );
    let margin = (max - min).max_element() * 0.2 + 4.0;
    let (min, max, z) = (min - margin, max + margin, low - 1.5);
    if floor.laid {
        let ease = 1.0 - (-time.delta_secs() * 3.0).exp();
        floor.min = floor.min.lerp(min, ease);
        floor.max = floor.max.lerp(max, ease);
        floor.z += (z - floor.z) * ease;
    } else {
        (floor.min, floor.max, floor.z, floor.laid) = (min, max, z, true);
    }
    let size = floor.max - floor.min;
    *visibility = Visibility::Visible;
    *transform = Transform::from_translation(((floor.min + floor.max) / 2.0).extend(floor.z))
        .with_scale(size.extend(1.0));
    if let Some(mut material) = materials.get_mut(&floor.material) {
        material.base_color = BACKDROP.mix(&SURFACE, lift);
    }
    let Ok((camera, orbit)) = cameras.single() else {
        return;
    };
    let eye = Eye::of(camera, orbit.distance);
    let step = size.max_element() / GRID_LINES as f32;
    let middle = (floor.min + floor.max) / 2.0;
    let line = |from: Vec3, to: Vec3, gizmos: &mut Gizmos<FloorGizmos>| {
        let at = |i: usize| from.lerp(to, i as f32 / GRID_PIECES as f32);
        for i in 0..GRID_PIECES {
            let (a, b) = (at(i), at(i + 1));
            let shade = |p: Vec3| {
                let across = (p.truncate() - middle) / (size / 2.0);
                let glow = floor_glow(across.x, across.y);
                Color::WHITE.with_alpha(GRID * lift * glow * (1.0 - eye.haze(p) / HAZE))
            };
            gizmos.line_gradient(a, b, shade(a), shade(b));
        }
    };
    let z = floor.z + 0.01;
    for i in 0..=GRID_LINES {
        let offset = step * i as f32;
        let x = floor.min.x + offset;
        if x <= floor.max.x + 1e-3 {
            line(
                Vec3::new(x, floor.min.y, z),
                Vec3::new(x, floor.max.y, z),
                &mut gizmos,
            );
        }
        let y = floor.min.y + offset;
        if y <= floor.max.y + 1e-3 {
            line(
                Vec3::new(floor.min.x, y, z),
                Vec3::new(floor.max.x, y, z),
                &mut gizmos,
            );
        }
    }
}

// ---- footprints ----

/// Each call's footprint on the floor under its node: a soft pool of its color's
/// light, as if its marker glowed down onto the map, so its height is the gap between
/// the two. Light, since a dark shadow barely shows on a dark floor.
#[derive(Resource, Debug, Default)]
struct Footprints {
    mesh: Handle<Mesh>,
    spot: Handle<Image>,
    /// One per call, spare ones hidden.
    drawn: Vec<(Entity, Handle<StandardMaterial>)>,
}

#[derive(Component)]
struct Footprint;

/// How bright a footprint is at its middle, at most.
const FOOTPRINT: f32 = 0.55;

/// How opaque a soft round spot is `distance` from its middle, 1 being its edge.
pub(crate) fn spot_alpha(distance: f32) -> f32 {
    let t = (1.0 - distance).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How brightly a footprint `away` calls from the cursor's glows, of `count` calls:
/// the cursor's in full, its neighbors' less, the rest faintly, and fainter in a long
/// journey, so the pools under a crowd of calls don't run together.
pub(crate) fn pool_share(away: Option<usize>, count: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let crowd = (12.0 / count.max(1) as f32).clamp(0.5, 1.0);
    match away {
        Some(0) => 1.0,
        Some(1..=2) => 0.7,
        _ => 0.6 * crowd,
    }
}

/// A white spot fading out to its edge, `size` pixels across.
fn spot_image(size: u32) -> Image {
    let mut data = Vec::new();
    let across = |i: u32| (i as f32 + 0.5) / size as f32 * 2.0 - 1.0;
    for y in 0..size {
        for x in 0..size {
            let alpha = spot_alpha(across(x).hypot(across(y)));
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "within a channel"
            )]
            data.extend([255, 255, 255, (alpha * 255.0).round() as u8]);
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
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Lays a pool of light round each shown call's node, where its string lands.
fn cast_footprints(
    mut commands: Commands,
    explorer: Res<Explorer>,
    floors: Res<Floors>,
    mut footprints: ResMut<Footprints>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut drawn: Query<(&mut Transform, &mut Visibility), With<Footprint>>,
    cameras: Query<(&GlobalTransform, &OrbitCamera)>,
) {
    let lift = floors.lift();
    let count = if floors.showing() {
        floors.calls.len()
    } else {
        0
    };
    while footprints.drawn.len() < count {
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(footprints.spot.clone()),
            alpha_mode: AlphaMode::Add,
            unlit: true,
            fog_enabled: false,
            ..default()
        });
        let entity = commands
            .spawn((
                Footprint,
                Mesh3d(footprints.mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
            ))
            .id();
        footprints.drawn.push((entity, material));
    }
    let Ok((transform, orbit)) = cameras.single() else {
        return;
    };
    let eye = Eye::of(transform, orbit.distance);
    let shown = floors.window.len();
    for (index, (entity, material)) in footprints.drawn.iter().enumerate() {
        let Ok((mut place, mut visibility)) = drawn.get_mut(*entity) else {
            continue;
        };
        let Some(call) = floors
            .calls
            .get(index)
            .filter(|_| index < count && floors.shows(index))
        else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let away = if floors.hovered(index) {
            Some(0)
        } else {
            floors.away(index)
        };
        let size = node_size(explorer.graph(), call.tile);
        let radius = size * if away == Some(0) { 3.2 } else { 2.2 };
        // Round the node itself rather than on the floor straight under it: the map
        // is not quite flat, and a tilted view would part the two.
        let at = call.foot - Vec3::Z * size * 0.5;
        *visibility = Visibility::Visible;
        *place =
            Transform::from_translation(at).with_scale(Vec3::new(radius * 2.0, radius * 2.0, 1.0));
        if let Some(mut material) = materials.get_mut(material) {
            let tint = node_color(&explorer, None, call.tile).mix(&Color::WHITE, 0.4);
            let strength = FOOTPRINT * pool_share(away, shown) * lift * (1.0 - eye.haze(at));
            material.base_color = tint.with_alpha(strength);
        }
    }
}

// ---- the rock ----

/// How far the camera rocks each way, in radians, and how long a cycle takes.
const ROCK: f32 = 0.045;
const ROCK_PERIOD: f32 = 6.0;

/// Whether the time view rocks the camera, and where in its rock to hold it.
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

/// While the upper floor is up, rocks the view gently from side to side about the
/// map, so the deck over it slides against it; easing in and out, and held still by
/// `Shift Z`.
fn rock(
    time: Res<Time>,
    settings: Res<Rock>,
    floors: Res<Floors>,
    mut amount: Local<f32>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let on = if settings.still { 0.0 } else { floors.lift() };
    let ease = 1.0 - (-time.delta_secs() * 2.0).exp();
    *amount += (on - *amount) * ease;
    let phase = settings
        .held_at
        .unwrap_or(time.elapsed_secs() / ROCK_PERIOD);
    for mut camera in &mut cameras {
        camera.sway = sway(phase) * *amount;
        // About the map under the view's middle, so the map holds still and the deck
        // over it sways.
        camera.pivot = if camera.free {
            Vec3::ZERO
        } else {
            Vec3::NEG_Z * floors.middle()
        };
    }
}

// ---- painting ----

/// `color` a share `t` of the way to `to`, alpha kept.
pub(crate) fn mix(color: egui::Color32, to: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "stays within a channel"
        )]
        let value = (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        value
    };
    egui::Color32::from_rgba_unmultiplied(
        channel(color.r(), to.r()),
        channel(color.g(), to.g()),
        channel(color.b(), to.b()),
        color.a(),
    )
}

/// Points round a circle, for the meshes below.
const ROUND: usize = 28;

/// A lit sphere at `at` of `radius` pixels in `fill`, the light coming from `light`
/// across the screen: bright towards it, dark at the far rim, with a glint.
pub(crate) fn sphere(
    painter: &egui::Painter,
    at: egui::Pos2,
    radius: f32,
    fill: egui::Color32,
    light: egui::Vec2,
    alpha: f32,
) {
    let lit = mix(fill, egui::Color32::WHITE, 0.4);
    let dark = mix(fill, egui::Color32::BLACK, 0.7);
    let mut mesh = egui::Mesh::default();
    let hot = at + light * radius * 0.4;
    mesh.colored_vertex(hot, lit.gamma_multiply(alpha));
    for i in 0..=ROUND {
        let angle = i as f32 / ROUND as f32 * TAU;
        let out = egui::vec2(angle.cos(), angle.sin());
        // The rim facing the light keeps some of the fill; the far rim falls dark.
        let facing = f32::midpoint(out.dot(light), 1.0);
        let rim = mix(dark, fill, facing * 0.8);
        mesh.colored_vertex(at + out * radius, rim.gamma_multiply(alpha));
    }
    #[expect(clippy::cast_possible_truncation, reason = "a few points")]
    for i in 0..ROUND as u32 {
        mesh.add_triangle(0, i + 1, i + 2);
    }
    painter.add(mesh);
    painter.circle_stroke(
        at,
        radius,
        egui::Stroke::new(
            0.8,
            egui::Color32::from_black_alpha(140).gamma_multiply(alpha),
        ),
    );
    painter.circle_filled(
        at + light * radius * 0.5,
        (radius * 0.2).max(1.0),
        egui::Color32::from_white_alpha(170).gamma_multiply(alpha),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haze_thickens_with_distance_and_stops_short_of_hiding() {
        assert!(haze(0.5) < 1e-6, "clear near the eye");
        assert!(haze(HAZE_NEAR) < 1e-6, "clear at the point orbited");
        assert!(haze(1.2) > haze(1.0));
        assert!((haze(10.0) - HAZE).abs() < 1e-6, "never more than the most");
    }

    #[test]
    fn the_rock_swings_both_ways_and_back() {
        assert!(sway(0.0).abs() < 1e-6, "starts square");
        assert!((sway(0.25) - ROCK).abs() < 1e-6, "one end");
        assert!((sway(0.75) + ROCK).abs() < 1e-6, "the other");
        assert!(sway(1.0).abs() < 1e-5, "and back");
    }

    #[test]
    fn mixing_goes_part_way_and_keeps_alpha() {
        let from = egui::Color32::from_rgba_unmultiplied(0, 100, 200, 255);
        let to = egui::Color32::from_rgba_unmultiplied(100, 100, 0, 255);
        let half = mix(from, to, 0.5);
        assert_eq!(
            (half.r(), half.g(), half.b(), half.a()),
            (50, 100, 100, 255)
        );
        assert_eq!(mix(from, to, 2.0), to, "never past where it goes");
    }

    #[test]
    fn a_spot_is_solid_in_the_middle_and_gone_at_its_edge() {
        assert!((spot_alpha(0.0) - 1.0).abs() < 1e-6);
        assert!(spot_alpha(0.5) < spot_alpha(0.2));
        assert!(spot_alpha(1.0).abs() < 1e-6);
        assert!(spot_alpha(1.5).abs() < 1e-6, "nothing past it");
    }

    #[test]
    fn the_cursors_pool_glows_brightest_and_a_crowd_dimmer() {
        assert!(pool_share(Some(0), 10) > pool_share(Some(1), 10));
        assert!(pool_share(Some(1), 10) > pool_share(Some(5), 10));
        assert!(
            pool_share(Some(5), 50) < pool_share(Some(5), 10),
            "a crowd dims"
        );
        assert!(pool_share(None, 500) > 0.0, "never gone");
    }

    #[test]
    fn the_floor_fades_out_to_its_rim() {
        assert!(
            (floor_glow(0.0, 0.0) - 1.0).abs() < 1e-6,
            "full in the middle"
        );
        assert!((floor_glow(0.3, -0.2) - 1.0).abs() < 1e-6, "and round it");
        assert!(floor_glow(0.8, 0.0) < floor_glow(0.6, 0.0));
        assert!(floor_glow(1.0, 0.0).abs() < 1e-6, "none at the rim");
        assert!(floor_glow(1.0, 1.0).abs() < 1e-6, "nor in the corners");
    }

    #[test]
    fn nearer_looks_bigger() {
        let eye = Eye {
            at: Vec3::new(0.0, 0.0, 10.0),
            forward: Vec3::NEG_Z,
            reach: 10.0,
        };
        assert!(
            (eye.nearness(Vec3::ZERO) - 1.0).abs() < 1e-6,
            "the point orbited"
        );
        assert!(
            eye.nearness(Vec3::new(0.0, 0.0, 5.0)) > 1.9,
            "half way is twice"
        );
        assert!(eye.haze(Vec3::new(0.0, 0.0, -20.0)) > eye.haze(Vec3::ZERO));
    }
}
