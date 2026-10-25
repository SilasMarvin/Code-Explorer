//! Beams in 3D (`Z` a second time). The map lies on a faint floor, tilted gently away
//! from the eye, and the journey's calls lie along the floor's near edge as a rail of
//! named tiles, in the order they run, left to right. The beams are arcs that rise
//! from their tiles, up and over, and come down onto their nodes, each landing in a
//! soft pool of its node's light; an arc to a far node climbs higher, so beams that
//! would cross flat part by height and depth instead. Haze dims the far side of the
//! map, so the floor reads as going away.

use bevy::{
    asset::RenderAssetUsages,
    picking::Pickable,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use bevy_egui::egui;
use codeviz_core::SpanId;

use crate::{
    beams::{self, Beam},
    camera::OrbitCamera,
    explorer::{Explorer, is_conditional},
    graph_view::{self, node_color, node_size},
    journey_panel::{self, JourneyPanel},
    scene::{PALETTE, lane_color},
    ui::egui_color,
};

pub(crate) struct RailPlugin;

impl Plugin for RailPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Rail>()
            .init_gizmo_group::<FloorGizmos>()
            .add_systems(Startup, spawn_floor)
            .add_systems(
                Update,
                (stand, lay_floor, light_pools, haze_view)
                    .chain()
                    .after(graph_view::frame_spotlight),
            );
    }
}

/// The view while the rail shows: square on to it, so time runs straight across, and
/// tilted gently back, so the map keeps its shape and lies on its floor.
pub(crate) const RAIL_VIEW: (f32, f32) = (0.0, 0.52);

/// The rail and the floor it lies on, worked out from the journey each frame.
#[derive(Resource, Debug, Default)]
pub(crate) struct Rail {
    /// Whether it shows, and how far it has eased in, 0 to 1.
    up: bool,
    lift: f32,
    /// The camera's angles before it turned to the rail.
    before: Option<(f32, f32)>,
    /// Where the rail lies, while it shows.
    pub(crate) stage: Option<Stage>,
    /// How deep the rail's tiles reach towards the eye, in world units, as last drawn.
    pub(crate) depth: f32,
    floor: Floor,
    pools: Pools,
}

/// Where the rail lies: its back edge, nearest the map, runs from `left` to `right` at
/// `back` across the floor at height `z`. `middle` is the middle of the journey's
/// nodes and the rail, which the camera looks at, and `size` how big they are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Stage {
    pub(crate) left: f32,
    pub(crate) right: f32,
    pub(crate) back: f32,
    pub(crate) z: f32,
    pub(crate) middle: Vec3,
    pub(crate) size: f32,
}

/// How far in front of the journey's nearest node the rail lies, as a share of how
/// big the journey is.
const RAIL_GAP: f32 = 0.16;
/// How much deeper than its tiles the camera leaves room for in front of the rail.
const RAIL_ROOM: f32 = 0.12;

/// Where the rail of `count` calls lies under journey nodes at `points`, on a floor at
/// `z`: across the front of them, as wide as they spread, wider for a narrow journey
/// or a long one, so each call has room for its name, and a gap in front of the
/// nearest node.
pub(crate) fn stage(points: &[Vec3], count: usize, z: f32) -> Option<Stage> {
    let first = points.first()?;
    let (min, max) = points
        .iter()
        .fold((first.truncate(), first.truncate()), |(min, max), p| {
            (min.min(p.truncate()), max.max(p.truncate()))
        });
    let span = max - min;
    let size = span.max_element().max(8.0);
    let width = span.x.max(size * (count as f32 / 7.0).clamp(0.9, 2.4));
    let center = f32::midpoint(min.x, max.x);
    let back = min.y - size * RAIL_GAP;
    let front = back - size * RAIL_ROOM;
    Some(Stage {
        left: center - width / 2.0,
        right: center + width / 2.0,
        back,
        z,
        middle: Vec3::new(center, f32::midpoint(front, max.y), z.max(0.0)),
        size,
    })
}

/// How many staggered rows the rail needs so every call is named in full: `count`
/// calls across `width` pixels, the longest name `longest` pixels. A call on a row
/// spans as many slots as there are rows.
pub(crate) fn rows(count: usize, width: f32, longest: f32) -> usize {
    (1..=4)
        .find(|&rows| width / count.max(1) as f32 * rows as f32 >= longest)
        .unwrap_or(4)
}

/// A call's tile on the rail: its corners on the floor, back left first and round,
/// given its place in time, how many calls there are, how many rows, and how deep a
/// row is. Rows step towards the eye, the first nearest the map; each tile spans
/// `rows` slots, so the rows overlap and time still runs left to right.
pub(crate) fn tile(
    stage: &Stage,
    index: usize,
    count: usize,
    rows: usize,
    row_depth: f32,
) -> [Vec3; 4] {
    let pitch = (stage.right - stage.left) / (count + rows - 1).max(1) as f32;
    let gap = pitch * 0.08;
    let left = stage.left + index as f32 * pitch;
    let right = left + rows as f32 * pitch - gap;
    let row = index % rows;
    let back = stage.back - row as f32 * row_depth * 1.12;
    let front = back - row_depth;
    [
        Vec3::new(left, back, stage.z),
        Vec3::new(right, back, stage.z),
        Vec3::new(right, front, stage.z),
        Vec3::new(left, front, stage.z),
    ]
}

/// How high an arc from `from` to `to` climbs over the higher of the two: higher the
/// further it goes, and a step higher at each `level`, so arcs crossing one another
/// part.
pub(crate) fn apex(from: Vec3, to: Vec3, level: usize, size: f32) -> f32 {
    from.truncate().distance(to.truncate()) * 0.18 + size * (0.04 + 0.04 * level as f32)
}

/// The points along an arc: up out of `from`, over, and down onto `to`, `height`
/// above the higher of them at most.
pub(crate) fn arc(from: Vec3, to: Vec3, height: f32) -> Vec<Vec3> {
    let top = from.z.max(to.z) + height;
    // Straight up out of the tile, and down onto the node from the rail's side, so
    // it never hooks round behind the node.
    let back = to.truncate().lerp(from.truncate(), 0.4);
    let controls = [
        from,
        from.with_z(top * 1.25 - from.z * 0.25),
        back.extend(top),
        to,
    ];
    (0..=ARC_POINTS)
        .map(|i| {
            let at = i as f32 / ARC_POINTS as f32;
            let [p0, p1, p2, p3] = controls;
            let (q0, q1, q2) = (p0.lerp(p1, at), p1.lerp(p2, at), p2.lerp(p3, at));
            q0.lerp(q1, at).lerp(q1.lerp(q2, at), at)
        })
        .collect()
}

const ARC_POINTS: usize = 40;

/// Turns the camera to the rail when it comes up and back when it goes, eases it in
/// and out, presses the map flat, and lays the rail across the front of the journey,
/// holding the camera just far enough back to take the journey and its rail in,
/// until a hand moves it.
fn stand(
    time: Res<Time>,
    mut explorer: ResMut<Explorer>,
    panel: Res<JourneyPanel>,
    mut rail: ResMut<Rail>,
    mut cameras: Query<(&mut OrbitCamera, &Projection)>,
) {
    let up = panel.railed();
    let turned = up != rail.up;
    rail.up = up;
    for (mut camera, _) in &mut cameras {
        if turned && up {
            rail.before = Some(camera.angles());
            camera.turn_to(RAIL_VIEW);
        } else if turned && let Some(before) = rail.before.take() {
            camera.turn_to(before);
        }
    }
    let ease = 1.0 - (-time.delta_secs() * 4.0).exp();
    rail.lift += (f32::from(u8::from(up)) - rail.lift) * ease;
    // The map lies flat on the floor.
    if (explorer.flat() - rail.lift).abs() > 1e-4 {
        explorer.set_flat(rail.lift);
    }
    if !up {
        if rail.lift < 0.005 {
            rail.lift = 0.0;
            rail.stage = None;
            explorer.set_flat(0.0);
        }
        return;
    }
    let points: Vec<Vec3> = explorer
        .disclosure()
        .tiles()
        .keys()
        .copied()
        .chain(explorer.anchor())
        .filter_map(|tile| explorer.tile_point(tile))
        .collect();
    let floor = floor_height(&explorer);
    let count = beams::calls(&explorer).0.len();
    let Some(stage) = stage(&points, count, floor) else {
        rail.stage = None;
        return;
    };
    rail.stage = Some(stage);
    let rotation = Quat::from_euler(EulerRot::YXZ, RAIL_VIEW.0, RAIL_VIEW.1, 0.0);
    let mut reach = points;
    reach.extend([
        Vec3::new(stage.left, stage.back - stage.size * RAIL_ROOM, stage.z),
        Vec3::new(stage.right, stage.back - stage.size * RAIL_ROOM, stage.z),
    ]);
    for (mut camera, projection) in &mut cameras {
        let Projection::Perspective(perspective) = projection else {
            continue;
        };
        if camera.free && !turned {
            continue;
        }
        let needed = graph_view::fit_distance(&reach, stage.middle, rotation, perspective);
        camera.glide_to(stage.middle, needed * 0.9);
    }
}

/// While the rail shows, the camera looks at the middle of the journey and its rail
/// rather than at where the journey starts.
pub(crate) fn aim_view(rail: Res<Rail>, mut cameras: Query<&mut OrbitCamera>) {
    let (Some(stage), lift) = (rail.stage, rail.lift) else {
        return;
    };
    for mut camera in &mut cameras {
        if !camera.free {
            camera.target = camera.target.lerp(stage.middle, lift);
        }
    }
}

/// How deep under the map the floor lies: just under its lowest node.
fn floor_height(explorer: &Explorer) -> f32 {
    (0..explorer.map().tiles.len())
        .filter(|&tile| explorer.visibility(tile) >= 0.5)
        .filter_map(|tile| explorer.tile_point(tile))
        .map(|p| p.z)
        .fold(f32::MAX, f32::min)
        .min(0.0)
        - 1.5
}

// ---- the floor ----

/// The window's background, which haze fades towards.
const BACKDROP: Color = Color::srgb(0.06, 0.07, 0.09);
/// The floor: a shade lighter than the background, so the map reads as lying on it.
const SURFACE: Color = Color::srgb(0.12, 0.13, 0.165);

/// The floor's grid, drawn thinner than links.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct FloorGizmos;

/// The floor under the map and the rail, eased as they move.
#[derive(Debug, Default)]
struct Floor {
    min: Vec2,
    max: Vec2,
    laid: bool,
    material: Handle<StandardMaterial>,
}

#[derive(Component)]
struct Ground;

/// Each beam's pool of light where it lands, a soft spot added to what is under it.
#[derive(Debug, Default)]
struct Pools {
    mesh: Handle<Mesh>,
    spot: Handle<Image>,
    drawn: Vec<(Entity, Handle<StandardMaterial>)>,
}

#[derive(Component)]
struct Pool;

fn spawn_floor(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut rail: ResMut<Rail>,
    mut store: ResMut<GizmoConfigStore>,
) {
    store.config_mut::<FloorGizmos>().0.line.width = 1.0;
    rail.pools.mesh = meshes.add(Rectangle::new(1.0, 1.0));
    rail.pools.spot = images.add(shaded_image(64, |u, v| spot_alpha(u.hypot(v)), true));
    let material = materials.add(StandardMaterial {
        base_color: BACKDROP,
        base_color_texture: Some(images.add(shaded_image(
            128,
            |u, v| FLOOR_RIM + (1.0 - FLOOR_RIM) * floor_glow(u, v),
            false,
        ))),
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
    rail.floor.material = material;
}

/// How much of the floor's own shade shows at `(u, v)` across it, each from -1 to 1
/// edge to edge: all of it over the middle, falling away to none at its rim, so the
/// floor has no hard edge.
pub(crate) fn floor_glow(u: f32, v: f32) -> f32 {
    let rim = (u.powi(4) + v.powi(4)).sqrt().sqrt();
    let t = ((rim - 0.55) / 0.45).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// What the floor's surface is scaled by at its rim.
const FLOOR_RIM: f32 = 0.3;

/// How opaque a soft round spot is `distance` from its middle, 1 being its edge.
pub(crate) fn spot_alpha(distance: f32) -> f32 {
    let t = (1.0 - distance).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A square image `size` pixels across shaded by `shade` from -1 to 1 each way: in
/// its alpha over white when `alpha`, else in grey.
fn shaded_image(size: u32, shade: impl Fn(f32, f32) -> f32, alpha: bool) -> Image {
    let across = |i: u32| (i as f32 + 0.5) / size as f32 * 2.0 - 1.0;
    let mut data = Vec::new();
    for y in 0..size {
        for x in 0..size {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "within a channel"
            )]
            let value = (shade(across(x), across(y)).clamp(0.0, 1.0) * 255.0).round() as u8;
            data.extend(if alpha {
                [255, 255, 255, value]
            } else {
                [value, value, value, 255]
            });
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
        if alpha {
            TextureFormat::Rgba8UnormSrgb
        } else {
            TextureFormat::Rgba8Unorm
        },
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Grid lines across the floor, and the pieces each is drawn in so it fades at the rim.
const GRID_LINES: usize = 18;
const GRID_PIECES: usize = 12;
/// How strongly the grid draws at the floor's middle.
const GRID: f32 = 0.08;

/// Fits the floor under the map and the rail, fades it in with the rail, and draws
/// its grid.
fn lay_floor(
    time: Res<Time>,
    explorer: Res<Explorer>,
    mut rail: ResMut<Rail>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut grounds: Query<(&mut Transform, &mut Visibility), With<Ground>>,
    mut gizmos: Gizmos<FloorGizmos>,
) {
    let lift = rail.lift;
    let Ok((mut transform, mut visibility)) = grounds.single_mut() else {
        return;
    };
    let Some(stage) = rail.stage.filter(|_| lift > 0.01) else {
        *visibility = Visibility::Hidden;
        rail.floor.laid = false;
        return;
    };
    let (mut min, mut max) = (0..explorer.map().tiles.len())
        .filter(|&tile| explorer.visibility(tile) >= 0.5)
        .filter_map(|tile| explorer.tile_point(tile))
        .fold(
            (
                Vec2::new(stage.left, stage.back),
                Vec2::new(stage.right, stage.back),
            ),
            |(min, max), p| (min.min(p.truncate()), max.max(p.truncate())),
        );
    let margin = (max - min).max_element() * 0.12 + 4.0;
    min -= Vec2::splat(margin);
    max += Vec2::splat(margin);
    // The rail lies along the floor's near edge.
    min.y = stage.back - rail.depth - stage.size * 0.08 - margin * 0.3;
    let floor = &mut rail.floor;
    if floor.laid {
        let ease = 1.0 - (-time.delta_secs() * 3.0).exp();
        floor.min = floor.min.lerp(min, ease);
        floor.max = floor.max.lerp(max, ease);
    } else {
        (floor.min, floor.max, floor.laid) = (min, max, true);
    }
    let size = floor.max - floor.min;
    let middle = (floor.min + floor.max) / 2.0;
    *visibility = Visibility::Visible;
    *transform = Transform::from_translation(middle.extend(stage.z)).with_scale(size.extend(1.0));
    if let Some(mut material) = materials.get_mut(&floor.material) {
        material.base_color = BACKDROP.mix(&SURFACE, lift);
    }
    let z = stage.z + 0.01;
    let shade = |p: Vec3| {
        let across = (p.truncate() - middle) / (size / 2.0);
        Color::WHITE.with_alpha(GRID * lift * floor_glow(across.x, across.y))
    };
    let mut line = |from: Vec3, to: Vec3| {
        let at = |i: usize| from.lerp(to, i as f32 / GRID_PIECES as f32);
        for i in 0..GRID_PIECES {
            let (a, b) = (at(i), at(i + 1));
            gizmos.line_gradient(a, b, shade(a), shade(b));
        }
    };
    let step = size.max_element() / GRID_LINES as f32;
    for i in 0..=GRID_LINES {
        let offset = step * i as f32;
        let x = floor.min.x + offset;
        if x <= floor.max.x + 1e-3 {
            line(Vec3::new(x, floor.min.y, z), Vec3::new(x, floor.max.y, z));
        }
        let y = floor.min.y + offset;
        if y <= floor.max.y + 1e-3 {
            line(Vec3::new(floor.min.x, y, z), Vec3::new(floor.max.x, y, z));
        }
    }
}

/// How bright the pool where a beam lands is at its middle, at most.
const POOL: f32 = 0.6;

/// Lays a pool of light round each node a beam lands on, the cursor's widest and
/// brightest.
fn light_pools(
    mut commands: Commands,
    explorer: Res<Explorer>,
    mut rail: ResMut<Rail>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut drawn: Query<(&mut Transform, &mut Visibility), With<Pool>>,
) {
    let lift = rail.lift;
    let landings: Vec<(usize, Beam)> = if rail.stage.is_some() {
        let (calls, cursor) = beams::calls(&explorer);
        let tiles: Vec<Option<usize>> = calls.iter().map(|&(_, tile)| tile).collect();
        beams::beamed(&tiles, cursor)
            .into_iter()
            .filter_map(|(index, beam)| Some((tiles[index]?, beam)))
            .collect()
    } else {
        Vec::new()
    };
    let pools = &mut rail.pools;
    while pools.drawn.len() < landings.len() {
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(pools.spot.clone()),
            alpha_mode: AlphaMode::Add,
            unlit: true,
            fog_enabled: false,
            ..default()
        });
        let entity = commands
            .spawn((
                Pool,
                Mesh3d(pools.mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
            ))
            .id();
        pools.drawn.push((entity, material));
    }
    for (index, (entity, material)) in pools.drawn.iter().enumerate() {
        let Ok((mut place, mut visibility)) = drawn.get_mut(*entity) else {
            continue;
        };
        let Some((tile, beam)) = landings.get(index).copied() else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let Some(at) = explorer.tile_point(tile) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let strength = beam.strength();
        let radius = node_size(explorer.graph(), tile) * (1.6 + 2.4 * strength);
        *visibility = Visibility::Visible;
        *place = Transform::from_translation(at - Vec3::Z * 0.2).with_scale(Vec3::new(
            radius * 2.0,
            radius * 2.0,
            1.0,
        ));
        if let Some(mut material) = materials.get_mut(material) {
            let tint = node_color(&explorer, None, tile).mix(&Color::WHITE, 0.45);
            material.base_color = tint.with_alpha(POOL * strength * lift);
        }
    }
}

/// Haze starts a little past the point looked at and is thickest further out, as a
/// share of how far that point is, and the most there is.
const HAZE_NEAR: f32 = 0.8;
const HAZE_FAR: f32 = 1.6;
const HAZE: f32 = 0.65;

/// Haze on the nodes and the floor while the rail shows, so the map's far side
/// recedes.
fn haze_view(
    mut commands: Commands,
    rail: Res<Rail>,
    mut cameras: Query<(Entity, &OrbitCamera, Option<&mut DistanceFog>)>,
) {
    for (entity, camera, fog) in &mut cameras {
        let color = BACKDROP.with_alpha(HAZE * rail.lift);
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

// ---- drawing ----

impl Rail {
    /// Lays the rail of calls on the floor and the arcs from it in `beam_slot`, under
    /// the graph's names, and lets the pointer at its calls.
    pub(crate) fn lay(
        &mut self,
        painter: &egui::Painter,
        beam_slot: egui::layers::ShapeIdx,
        explorer: &mut Explorer,
        panel: &mut JourneyPanel,
        project: impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        let nodes = |tile: usize| {
            let world = explorer.tile_point(tile)?;
            let node = project(world)?;
            let radius = node_size(explorer.graph(), tile) / 2.0;
            let edge = project(world + Vec3::Z * radius).map_or(8.0, |p| p.distance(node));
            Some((node, edge))
        };
        let (arcs, placed) = draw(painter, explorer, self, &project, nodes);
        painter.set(beam_slot, arcs);
        journey_panel::rail_pointer(painter.ctx(), &placed, explorer, panel);
    }
}

/// Pixels deep a row of tiles stands on screen, and the largest a name is drawn.
const ROW_PIXELS: f32 = 24.0;
const NAME_SIZE: f32 = 12.5;

/// One call's tile on screen.
pub(crate) struct Placed {
    pub(crate) span: SpanId,
    pub(crate) tile: Option<usize>,
    pub(crate) corners: [egui::Pos2; 4],
}

impl Placed {
    /// Whether `point` is on the tile.
    pub(crate) fn contains(&self, point: egui::Pos2) -> bool {
        inside(&self.corners, point)
    }

    pub(crate) fn bounds(&self) -> egui::Rect {
        egui::Rect::from_points(&self.corners)
    }
}

/// Whether `point` is inside the four-sided shape with `corners`, taken in order
/// either way round.
pub(crate) fn inside(corners: &[egui::Pos2; 4], point: egui::Pos2) -> bool {
    let sides: Vec<f32> = (0..4)
        .map(|i| {
            let (a, b) = (corners[i], corners[(i + 1) % 4]);
            (b - a).x * (point - a).y - (b - a).y * (point - a).x
        })
        .collect();
    sides.iter().all(|&s| s >= 0.0) || sides.iter().all(|&s| s <= 0.0)
}

/// Draws the rail of calls on the floor. Returns the arcs from them to their nodes,
/// to draw under the graph's names, and where each call's tile is on screen, for the
/// pointer.
fn draw(
    painter: &egui::Painter,
    explorer: &Explorer,
    rail: &mut Rail,
    project: impl Fn(Vec3) -> Option<egui::Pos2>,
    node_at: impl Fn(usize) -> Option<(egui::Pos2, f32)>,
) -> (Vec<egui::Shape>, Vec<Placed>) {
    let Some(stage) = rail.stage else {
        return (Vec::new(), Vec::new());
    };
    let alpha = rail.lift;
    let (calls, cursor) = beams::calls(explorer);
    let count = calls.len();
    if count == 0 {
        return (Vec::new(), Vec::new());
    }
    let disclosed = explorer.disclosure();
    // How deep a row is on the floor, and how wide the rail is, in pixels here.
    let middle = Vec3::new(f32::midpoint(stage.left, stage.right), stage.back, stage.z);
    let (Some(at), Some(nearer), Some(left), Some(right)) = (
        project(middle),
        project(middle - Vec3::Y),
        project(middle.with_x(stage.left)),
        project(middle.with_x(stage.right)),
    ) else {
        return (Vec::new(), Vec::new());
    };
    let per_unit = at.distance(nearer).max(1e-3);
    let row_depth = ROW_PIXELS / per_unit;
    let names: Vec<String> = calls
        .iter()
        .map(|&(span, _)| {
            let number = disclosed.number(span).unwrap_or_default();
            let name = explorer.label(span);
            let name = name
                .rsplit(['.', ':'])
                .next()
                .unwrap_or_default()
                .to_owned();
            format!("{number} {name}")
        })
        .collect();
    let longest = names
        .iter()
        .map(|n| n.chars().count() as f32 * 6.6 + 12.0)
        .fold(0.0, f32::max);
    let rows = rows(count, left.distance(right), longest);
    rail.depth = row_depth * (rows as f32 * 1.12);
    let placed: Vec<Placed> = calls
        .iter()
        .enumerate()
        .filter_map(|(index, &(span, tile))| {
            let corners = self::tile(&stage, index, count, rows, row_depth);
            let corners = [
                project(corners[0])?,
                project(corners[1])?,
                project(corners[2])?,
                project(corners[3])?,
            ];
            Some(Placed {
                span,
                tile,
                corners,
            })
        })
        .collect();
    let beams = arcs(
        explorer, &stage, &calls, cursor, rows, row_depth, &project, &node_at, alpha,
    );
    for (index, placed) in placed.iter().enumerate() {
        let future = cursor.is_some_and(|c| index > c);
        paint_tile(
            painter,
            explorer,
            placed,
            &names[index],
            future,
            cursor == Some(index),
            alpha,
        );
    }
    (beams, placed)
}

/// Paints one call's tile on the rail, filled like its bar and named in full: what
/// runs after the cursor dims, as on the strip, dashed-edged when it only sometimes
/// runs, red-ended where it can fail, and the cursor's outlined in white.
fn paint_tile(
    painter: &egui::Painter,
    explorer: &Explorer,
    placed: &Placed,
    name: &str,
    future: bool,
    cursor: bool,
    alpha: f32,
) {
    let track = explorer.track();
    let base = egui_color(placed.tile.map_or_else(
        || lane_color(track.color(placed.span) % PALETTE.len()),
        |tile| node_color(explorer, None, tile),
    ));
    let fill = if future {
        base.gamma_multiply(0.4)
    } else {
        base
    };
    painter.add(egui::Shape::convex_polygon(
        placed.corners.to_vec(),
        fill.gamma_multiply(alpha),
        egui::Stroke::NONE,
    ));
    let edge = if cursor {
        Some((2.0, egui::Color32::WHITE))
    } else if placed.tile.is_some() && placed.tile == explorer.hovered_tile {
        Some((1.5, egui::Color32::from_white_alpha(220)))
    } else if track.span(placed.span).is_some_and(is_conditional) {
        Some((1.0, egui::Color32::from_white_alpha(120)))
    } else {
        None
    };
    if let Some((width, color)) = edge {
        let mut ring = placed.corners.to_vec();
        ring.push(ring[0]);
        painter.add(egui::Shape::line(
            ring,
            egui::Stroke::new(width, color.gamma_multiply(alpha)),
        ));
    }
    if explorer.disclosure().fails(placed.span) {
        let [_, back_right, front_right, _] = placed.corners;
        painter.line_segment(
            [back_right, front_right],
            egui::Stroke::new(
                2.5,
                egui::Color32::from_rgb(240, 110, 100).gamma_multiply(alpha),
            ),
        );
    }
    let bounds = placed.bounds();
    let size = (bounds.height() * 0.62).clamp(8.0, NAME_SIZE);
    let text = if future {
        egui::Color32::from_rgb(200, 205, 216)
    } else {
        egui::Color32::from_rgb(14, 16, 22)
    };
    painter
        .with_clip_rect(bounds.intersect(painter.clip_rect()))
        .text(
            egui::pos2(bounds.min.x + 5.0, bounds.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(size),
            text.gamma_multiply(alpha),
        );
}

/// The arcs from the rail to the nodes: the cursor's call's brightest, the two calls
/// either side fainter, and the cursor's function's other calls dotted.
#[expect(clippy::too_many_arguments, reason = "what an arc is drawn from")]
fn arcs(
    explorer: &Explorer,
    stage: &Stage,
    calls: &[(SpanId, Option<usize>)],
    cursor: Option<usize>,
    rows: usize,
    row_depth: f32,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    node_at: &impl Fn(usize) -> Option<(egui::Pos2, f32)>,
    alpha: f32,
) -> Vec<egui::Shape> {
    let tiles: Vec<Option<usize>> = calls.iter().map(|&(_, tile)| tile).collect();
    let count = calls.len();
    let beams: Vec<(Vec3, usize, Vec3, Beam)> = beams::beamed(&tiles, cursor)
        .into_iter()
        .filter_map(|(index, beam)| {
            let tile = tiles[index]?;
            let [back_left, back_right, ..] = self::tile(stage, index, count, rows, row_depth);
            let node = explorer.tile_point(tile)?;
            let top = node + Vec3::Z * node_size(explorer.graph(), tile) * 0.5;
            Some((back_left.midpoint(back_right), tile, top, beam))
        })
        .collect();
    let levels = beams::bend_levels(
        &beams
            .iter()
            .map(|&(from, _, to, _)| (from.x, to.x))
            .collect::<Vec<_>>(),
    );
    let mut order: Vec<usize> = (0..beams.len()).collect();
    order.sort_by(|&a, &b| beams[a].3.strength().total_cmp(&beams[b].3.strength()));
    let mut shapes = Vec::new();
    for i in order {
        let (from, tile, to, beam) = beams[i];
        let strength = beam.strength() * alpha;
        let height = apex(from, to, levels[i], stage.size);
        let mut points: Vec<egui::Pos2> = arc(from, to, height)
            .into_iter()
            .filter_map(project)
            .collect();
        let Some((node, edge)) = node_at(tile) else {
            continue;
        };
        let gap = edge + 3.0;
        while points.len() > 2 && points.last().is_some_and(|p| p.distance(node) < gap) {
            points.pop();
        }
        let width = 1.0 + strength * 1.4;
        let color = egui::Color32::WHITE.gamma_multiply(strength * 0.92);
        shapes.push(egui::Shape::line(
            points.clone(),
            egui::Stroke::new(
                width + 5.0,
                egui::Color32::WHITE.gamma_multiply(strength * 0.07),
            ),
        ));
        if beam == Beam::Again {
            shapes.extend(egui::Shape::dotted_line(&points, color, 5.0, width * 0.75));
        } else {
            shapes.push(egui::Shape::line(points, egui::Stroke::new(width, color)));
        }
        shapes.push(egui::Shape::circle_stroke(
            node,
            gap,
            egui::Stroke::new(width, color),
        ));
    }
    shapes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rail_lies_across_the_front_of_the_journey() {
        let points = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(40.0, 20.0, 0.0)];
        let stage = stage(&points, 4, -2.0).unwrap();
        assert!((stage.left - 0.0).abs() < 1e-3 && (stage.right - 40.0).abs() < 1e-3);
        assert!(stage.back < 0.0, "in front of the nearest node");
        assert!((stage.z + 2.0).abs() < 1e-6, "on the floor");
        assert!(stage.middle.y < 10.0, "the view takes in the rail");
        let narrow = stage_of_line();
        assert!(
            narrow.right - narrow.left >= narrow.size * 0.9 - 1e-3,
            "never a sliver"
        );
        let long = super::stage(&points, 20, -2.0).unwrap();
        assert!(long.right - long.left > 40.0, "wider for a long journey");
        assert!(super::stage(&[], 4, 0.0).is_none());
    }

    fn stage_of_line() -> Stage {
        stage(
            &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 30.0, 0.0)],
            4,
            0.0,
        )
        .unwrap()
    }

    #[test]
    fn rows_stagger_until_every_name_fits() {
        assert_eq!(rows(8, 1400.0, 120.0), 1);
        assert_eq!(rows(19, 1400.0, 120.0), 2);
        assert_eq!(rows(49, 1400.0, 120.0), 4);
    }

    #[test]
    fn tiles_run_left_to_right_and_step_towards_the_eye() {
        let stage = stage(&[Vec3::ZERO, Vec3::new(100.0, 50.0, 0.0)], 5, 0.0).unwrap();
        let tiles: Vec<[Vec3; 4]> = (0..5).map(|i| tile(&stage, i, 5, 2, 3.0)).collect();
        assert!(tiles.windows(2).all(|w| w[1][0].x > w[0][0].x), "in order");
        assert!(
            (tiles[0][0].y - stage.back).abs() < 1e-6,
            "the first row at the back"
        );
        assert!(tiles[1][0].y < tiles[0][2].y, "the second in front of it");
        assert!(
            (tiles[4][1].x - stage.right).abs() < 2.0,
            "the last reaches the end"
        );
        assert!(tiles[0][1].x > tiles[1][0].x, "rows overlap");
    }

    #[test]
    fn an_arc_rises_from_its_tile_and_lands_on_its_node() {
        let (from, to) = (Vec3::new(0.0, -10.0, 0.0), Vec3::new(30.0, 20.0, 1.0));
        let points = arc(from, to, 12.0);
        assert_eq!(points[0], from);
        assert!(points.last().unwrap().distance(to) < 1e-4);
        let top = points.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        assert!(top > 8.0 && top <= 1.0 + 12.0 + 1e-3, "up and over: {top}");
        assert!(points[1].z > from.z, "leaves upwards");
    }

    #[test]
    fn farther_and_higher_level_arcs_climb_higher() {
        let from = Vec3::ZERO;
        assert!(apex(from, Vec3::X * 50.0, 0, 10.0) > apex(from, Vec3::X * 10.0, 0, 10.0));
        assert!(apex(from, Vec3::X * 10.0, 2, 10.0) > apex(from, Vec3::X * 10.0, 0, 10.0));
    }

    #[test]
    fn a_tile_holds_the_points_inside_it() {
        let corners = [
            egui::pos2(0.0, 0.0),
            egui::pos2(10.0, 0.0),
            egui::pos2(12.0, 10.0),
            egui::pos2(-2.0, 10.0),
        ];
        assert!(inside(&corners, egui::pos2(5.0, 5.0)));
        assert!(!inside(&corners, egui::pos2(20.0, 5.0)));
        let mut backwards = corners;
        backwards.reverse();
        assert!(inside(&backwards, egui::pos2(5.0, 5.0)), "either way round");
    }

    #[test]
    fn the_floor_fades_out_to_its_rim() {
        assert!((floor_glow(0.0, 0.0) - 1.0).abs() < 1e-6);
        assert!(floor_glow(0.8, 0.0) < floor_glow(0.6, 0.0));
        assert!(floor_glow(1.0, 1.0).abs() < 1e-6);
        assert!((spot_alpha(0.0) - 1.0).abs() < 1e-6 && spot_alpha(1.0).abs() < 1e-6);
    }
}
