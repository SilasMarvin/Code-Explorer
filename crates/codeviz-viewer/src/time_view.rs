//! Call order in a second view of the same scene, beside the map. Each call the journey
//! discloses is a marker over its node, lifted out of the map along z by its place in
//! the order: z is time. The map camera looks down z, head-on, as it always has, and
//! never sees them; a second camera, in its own viewport, does.
//!
//! - In 3D (the default) the second camera stands at a three-quarter angle, z up, so
//!   the calls rise over the journey's part of the map like a tower of steps, each
//!   dropping a line to its node's spot on the floor. Shading, fog and a faint floor
//!   grid say what is near; dragging in the view turns it, scrolling zooms, a double
//!   click turns it back.
//! - Side-on, an orthographic camera looks along the map's x axis, so time runs left
//!   to right and each call sits at the height its node shows on the map: module bands
//!   line up across the two views, and a call is found on the map by looking straight
//!   across.
//!
//! The calls are spaced evenly, whatever they cost, so each reads as a step; a function
//! called three times has three markers over one node. In either view the cursor's
//! call is ringed and a dashed guide runs from its node on the map to it; the call
//! under the pointer lights its node on the map, and a click moves the cursor there.
//!
//! `Z` cycles off, 3D and side-on; [`Chrome`] holds which.

use std::ops::Range;

use bevy::{
    camera::{
        ScalingMode,
        visibility::{NoFrustumCulling, RenderLayers},
    },
    post_process::bloom::Bloom,
    prelude::*,
};
use bevy_egui::{egui, input::egui_wants_any_keyboard_input};
use codeviz_core::SpanId;

use crate::{
    camera::{FRONT, OrbitCamera},
    diff_view::DiffView,
    explorer::Explorer,
    graph_view::{self, node_color},
    journey_panel::{self, PanelMode},
    scene::{PALETTE, lane_color},
    ui::{self, Chrome, egui_color},
};

/// How call order shows beside the map.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TimeMode {
    #[default]
    Off,
    /// Calls rise over the map, seen at a three-quarter angle.
    Rise,
    /// Calls run left to right, seen from the map's side.
    Side,
}

impl TimeMode {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Off => Self::Rise,
            Self::Rise => Self::Side,
            Self::Side => Self::Off,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Rise => "3D",
            Self::Side => "side-on",
        }
    }
}

/// The layer only the time camera sees: the markers, their light and their lines.
const LAYER: usize = 1;

/// The second camera.
#[derive(Component)]
pub(crate) struct TimeCamera;

/// The time camera, as the egui pass reads it.
pub(crate) type TimeCameras<'w, 's> = Query<
    'w,
    's,
    (&'static mut Camera, &'static GlobalTransform),
    (With<TimeCamera>, Without<OrbitCamera>),
>;

/// A call's marker, by its index in [`TimeView::calls`].
#[derive(Component)]
struct Marker(usize);

/// One call on the journey, in the order it runs.
#[derive(Debug, Clone)]
pub(crate) struct Call {
    pub(crate) span: SpanId,
    pub(crate) tile: usize,
    /// The call that makes it, by index.
    pub(crate) parent: Option<usize>,
    /// Its number in the journey, `2.1`; empty for the start.
    pub(crate) number: String,
}

/// How the 3D view is turned: about the journey's middle, z up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Turn {
    /// Round the vertical, from straight in front (looking north up the map).
    pub(crate) yaw: f32,
    /// Up from the floor.
    pub(crate) pitch: f32,
    /// Nearer than fitting, above 1.
    pub(crate) zoom: f32,
}

impl Default for Turn {
    /// From the front left and a little above: time reads as height, and the floor
    /// still shows the map's layout.
    fn default() -> Self {
        Self {
            yaw: -0.5,
            pitch: 0.3,
            zoom: 1.0,
        }
    }
}

impl Turn {
    /// Turns by a drag of `delta` pixels.
    fn drag(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.008;
        self.pitch = (self.pitch + delta.y * 0.008).clamp(0.0, 1.5);
    }

    /// Where the eye is from what it looks at, `distance` away.
    pub(crate) fn eye(self, distance: f32) -> Vec3 {
        Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            -self.pitch.cos() * self.yaw.cos(),
            self.pitch.sin(),
        ) * distance
    }
}

/// The time view as it stands.
#[derive(Resource, Debug, Default)]
pub(crate) struct TimeView {
    pub(crate) calls: Vec<Call>,
    /// Each call's marker's color, as last drawn.
    colors: Vec<Color>,
    /// The calls that fit, around the cursor.
    pub(crate) window: Range<usize>,
    /// Where each call's marker sits, while it shows.
    pub(crate) points: Vec<Option<Vec3>>,
    /// The marker's radius, in world units.
    pub(crate) radius: f32,
    /// The call under the pointer in the time view.
    pub(crate) hovered: Option<usize>,
    pub(crate) turn: Turn,
    /// How it shows, as last placed.
    pub(crate) mode: TimeMode,
    /// In 3D, what the eye looks at and from how far before zooming, eased.
    fit: Option<(Vec3, f32)>,
    /// In 3D, the foot of the time ruler: the floor's corner nearest the eye.
    pub(crate) ruler: Option<Vec3>,
}

impl TimeView {
    /// The call the journey's cursor is on.
    pub(crate) fn cursor(&self, explorer: &Explorer) -> Option<usize> {
        let span = explorer.selected()?;
        self.calls.iter().position(|c| c.span == span)
    }
}

pub(crate) struct TimeViewPlugin;

impl Plugin for TimeViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TimeView>()
            .init_gizmo_group::<TimeGizmos>()
            .add_systems(Startup, spawn_time_camera)
            .add_systems(
                Update,
                (
                    head_on,
                    follow_journey,
                    turn_keys.run_if(not(egui_wants_any_keyboard_input)),
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                aim.after(crate::camera::apply_camera)
                    .before(TransformSystems::Propagate),
            );
    }
}

/// The time view's lines, which only its camera draws.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct TimeGizmos;

fn spawn_time_camera(mut commands: Commands, mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<TimeGizmos>();
    config.render_layers = RenderLayers::layer(LAYER);
    config.line.width = 1.5;
    commands.spawn((
        TimeCamera,
        Camera3d::default(),
        Camera {
            order: 1,
            is_active: false,
            clear_color: ClearColorConfig::Custom(BACKDROP),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection::default()),
        DistanceFog {
            color: BACKDROP,
            ..default()
        },
        RenderLayers::layer(LAYER),
        Transform::default(),
        Bloom::NATURAL,
    ));
    // The markers' own light, from above and behind the default eye, so they shade.
    commands.spawn((
        DirectionalLight {
            illuminance: 6_000.0,
            ..default()
        },
        Transform::from_xyz(0.4, 0.6, 1.0).looking_at(Vec3::ZERO, Vec3::Z),
        RenderLayers::layer(LAYER),
    ));
}

/// A shade off the map's, so the two views read as two.
const BACKDROP: Color = Color::srgb(0.075, 0.085, 0.11);

/// How far back the side-on camera stands, and how deep either sees.
const STAND_OFF: f32 = 4_000.0;
const FAR: f32 = 10_000.0;

/// The map faces its camera head-on whenever the time view opens.
fn head_on(
    chrome: Res<Chrome>,
    mut last: Local<Option<TimeMode>>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    if *last == Some(chrome.time) {
        return;
    }
    *last = Some(chrome.time);
    if chrome.time != TimeMode::Off {
        for mut camera in &mut cameras {
            camera.turn_to(FRONT);
        }
    }
}

/// Shift-Z turns the 3D view a quarter round, for a look from another side without
/// dragging.
fn turn_keys(keys: Res<ButtonInput<KeyCode>>, chrome: Res<Chrome>, mut view: ResMut<TimeView>) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if shift && keys.just_pressed(KeyCode::KeyZ) && chrome.time == TimeMode::Rise {
        view.turn.yaw += std::f32::consts::FRAC_PI_2;
    }
}

/// The journey as it runs: `root`, then each call under it in order, depth first,
/// each with the index of the call that makes it.
pub(crate) fn in_order<S: Copy>(
    root: S,
    children: impl Fn(S) -> Vec<S>,
) -> Vec<(S, Option<usize>)> {
    let mut calls = Vec::new();
    let mut stack: Vec<(S, Option<usize>)> = vec![(root, None)];
    while let Some((call, parent)) = stack.pop() {
        let index = calls.len();
        calls.push((call, parent));
        stack.extend(children(call).into_iter().rev().map(|c| (c, Some(index))));
    }
    calls
}

/// The disclosed journey's calls whose nodes the graph has, in the order they run.
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
    let ordered = in_order(journey.root(), shown);
    // A call whose node the graph lacks is left out; its calls hang from its caller.
    let mut kept: Vec<Option<usize>> = Vec::with_capacity(ordered.len());
    let mut calls = Vec::new();
    for &(span, parent) in &ordered {
        let tile = explorer.unit_id(span).and_then(|u| map.tile_of(u));
        let parent = parent.and_then(|p| kept[p]);
        match tile {
            Some(tile) => {
                kept.push(Some(calls.len()));
                calls.push(Call {
                    span,
                    tile,
                    parent,
                    number: disclosed.number(span).unwrap_or_default().to_owned(),
                });
            }
            None => kept.push(parent),
        }
    }
    calls
}

/// Rebuilds the calls and their markers when what the journey discloses changes.
#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn follow_journey(
    mut commands: Commands,
    chrome: Res<Chrome>,
    explorer: Res<Explorer>,
    diff: Option<Res<DiffView>>,
    mut view: ResMut<TimeView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    markers: Query<Entity, With<Marker>>,
) {
    let calls = if chrome.time == TimeMode::Off {
        Vec::new()
    } else {
        journey_calls(&explorer)
    };
    let colors: Vec<Color> = calls
        .iter()
        .map(|c| node_color(&explorer, diff.as_deref(), c.tile))
        .collect();
    let same = colors == view.colors
        && calls.len() == view.calls.len()
        && calls
            .iter()
            .zip(&view.calls)
            .all(|(a, b)| a.span == b.span && a.tile == b.tile);
    if same {
        return;
    }
    for marker in &markers {
        commands.entity(marker).despawn();
    }
    let mesh = meshes.add(Sphere::new(1.0).mesh().uv(24, 16));
    for (index, &color) in colors.iter().enumerate() {
        commands.spawn((
            Marker(index),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: color,
                // A glow of their own, so the side away from the light still reads.
                emissive: color.to_linear() * 0.35,
                perceptual_roughness: 0.45,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
            RenderLayers::layer(LAYER),
            // The orthographic camera's culling drops them, wherever they sit.
            NoFrustumCulling,
            Pickable::IGNORE,
        ));
    }
    view.calls = calls;
    view.colors = colors;
    view.points = vec![None; view.calls.len()];
}

/// The fewest pixels between two calls side-on; more calls than fit show the stretch
/// around the cursor.
const MIN_STEP: f32 = 22.0;

/// The most calls the 3D view rises through at once.
const MOST_RISING: usize = 60;

/// The calls that fit in `room`, around `cursor`: as many before it as after.
pub(crate) fn window(count: usize, cursor: Option<usize>, room: usize) -> Range<usize> {
    let room = room.max(1);
    if count <= room {
        return 0..count;
    }
    let cursor = cursor.unwrap_or(0).min(count - 1);
    let start = cursor.saturating_sub(room / 2).min(count - room);
    start..start + room
}

/// Where call `slot` of `slots` sits along time, as a share of the whole span, leaving
/// a little room before the first (for module names, side-on) and after the last.
pub(crate) fn along(slot: usize, slots: usize) -> f32 {
    const BEFORE: f32 = 0.06;
    const AFTER: f32 = 0.03;
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let (slot, slots) = (slot as f32, slots.max(1) as f32);
    BEFORE + (1.0 - BEFORE - AFTER) * (slot + 0.5) / slots
}

/// How high `point` shows in a perspective view from `eye` with a vertical field of
/// view `fov`: -1 at the bottom edge, 1 at the top; none behind it. Side-on, markers
/// take their node's height as the map shows it rather than as it is, so the two line
/// up exactly even where the map's perspective draws a nearer node further out.
pub(crate) fn apparent(eye: &Transform, fov: f32, point: Vec3) -> Option<f32> {
    let seen = eye.compute_affine().inverse().transform_point3(point);
    (seen.z < -1e-3).then(|| seen.y / (-seen.z * (fov / 2.0).tan()))
}

/// The journey's footprint on the map: its middle and how far most of it reaches
/// from there. The middle is the median each way and the reach holds four in five, so
/// one call far across the map does not shrink the rest to a needle.
pub(crate) fn footprint(points: &[Vec3]) -> Option<(Vec2, f32)> {
    let median = |mut values: Vec<f32>| {
        values.sort_by(f32::total_cmp);
        values.get(values.len() / 2).copied()
    };
    let middle = Vec2::new(
        median(points.iter().map(|p| p.x).collect())?,
        median(points.iter().map(|p| p.y).collect())?,
    );
    let mut reaches: Vec<f32> = points
        .iter()
        .map(|p| p.truncate().distance(middle))
        .collect();
    reaches.sort_by(f32::total_cmp);
    let most = (reaches.len() - 1) * 4 / 5;
    Some((middle, reaches[most]))
}

/// How the time camera sees, and where each call's marker sits.
struct Layout {
    eye: Transform,
    projection: Projection,
    /// The fog's falloff, by distance from the eye.
    fog: (f32, f32),
    points: Vec<Option<Vec3>>,
    /// Side-on, the height each call's node shows at, shown or not.
    levels: Vec<Option<f32>>,
    radius: f32,
}

/// The time camera, as it is aimed.
type TimeEyes<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Camera,
        &'static mut Transform,
        &'static mut Projection,
        &'static mut DistanceFog,
    ),
    (With<TimeCamera>, Without<OrbitCamera>, Without<Marker>),
>;

/// The markers, as they are placed.
type Markers<'w, 's> = Query<
    'w,
    's,
    (
        &'static Marker,
        &'static mut Transform,
        &'static mut Visibility,
    ),
    (Without<Camera>, Without<OrbitCamera>),
>;

/// Aims the time camera, places the markers and draws their lines.
#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn aim(
    time: Res<Time>,
    chrome: Res<Chrome>,
    explorer: Res<Explorer>,
    mut view: ResMut<TimeView>,
    maps: Query<(&OrbitCamera, &Transform, &Projection), Without<TimeCamera>>,
    mut cameras: TimeEyes,
    mut markers: Markers,
    mut gizmos: Gizmos<TimeGizmos>,
) {
    let Ok((mut camera, mut transform, mut projection, mut fog)) = cameras.single_mut() else {
        return;
    };
    camera.is_active = chrome.time != TimeMode::Off;
    let Some(size) = camera.logical_viewport_size().filter(|_| camera.is_active) else {
        return;
    };
    let cursor = view.cursor(&explorer);
    let layout = if chrome.time == TimeMode::Side {
        let Ok((orbit, map, Projection::Perspective(perspective))) = maps.single() else {
            return;
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a positive pixel count"
        )]
        let room = (size.x * 0.91 / MIN_STEP).max(1.0) as usize;
        view.window = window(view.calls.len(), cursor, room);
        side_on(&explorer, &view, size, orbit, map, perspective)
    } else {
        view.window = window(view.calls.len(), cursor, MOST_RISING);
        let ease = 1.0 - (-time.delta_secs() * 6.0).exp();
        let Some(layout) = rising(&explorer, &mut view, size, ease) else {
            return;
        };
        layout
    };
    *transform = layout.eye;
    *projection = layout.projection.clone();
    fog.falloff = FogFalloff::Linear {
        start: layout.fog.0,
        end: layout.fog.1,
    };
    view.radius = layout.radius;
    view.mode = chrome.time;
    let hovered_tile = explorer.hovered_tile;
    for (marker, mut place, mut visibility) in &mut markers {
        let point = layout.points.get(marker.0).copied().flatten();
        *visibility = if point.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let Some(point) = point else {
            continue;
        };
        let grow = if Some(marker.0) == cursor {
            1.7
        } else if hovered_tile == Some(view.calls[marker.0].tile) {
            1.4
        } else if cursor.is_some_and(|c| marker.0 > c) {
            // Yet to run.
            0.8
        } else {
            1.0
        };
        place.translation = point;
        place.scale = Vec3::splat(layout.radius * grow);
    }
    view.ruler = if chrome.time == TimeMode::Side {
        draw_levels(&view.calls, &layout, cursor, &mut gizmos);
        None
    } else {
        draw_rise(&explorer, &view, &layout, cursor, &mut gizmos)
    };
    view.points = layout.points;
}

/// A marker's radius, as a share of the side-on view's height.
const RADIUS_SHARE: f32 = 0.006;

/// Side-on: an orthographic camera along the map's x axis, seeing as much height as
/// the map does where it orbits; each marker at the height its node shows on the map.
fn side_on(
    explorer: &Explorer,
    view: &TimeView,
    size: Vec2,
    orbit: &OrbitCamera,
    map: &Transform,
    perspective: &PerspectiveProjection,
) -> Layout {
    let height = 2.0 * orbit.distance * (perspective.fov / 2.0).tan();
    let width = height * size.x / size.y;
    let shown = view.window.clone();
    let points = view
        .calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            let slot = index
                .checked_sub(shown.start)
                .filter(|_| shown.contains(&index))?;
            let at = explorer.tile_point(call.tile)?;
            let y = apparent(map, perspective.fov, at)? * height / 2.0;
            Some(Vec3::new(at.x, y, along(slot, shown.len()) * width))
        })
        .collect();
    let levels = view
        .calls
        .iter()
        .map(|call| {
            let at = explorer.tile_point(call.tile)?;
            Some(apparent(map, perspective.fov, at)? * height / 2.0)
        })
        .collect();
    Layout {
        // Looking along +x, so +z runs to the right; time starts at the left edge.
        eye: Transform::from_xyz(-STAND_OFF, 0.0, width / 2.0).looking_to(Vec3::X, Vec3::Y),
        projection: Projection::Orthographic(OrthographicProjection {
            near: 0.0,
            far: FAR,
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: height,
            },
            ..OrthographicProjection::default_3d()
        }),
        fog: (FAR * 2.0, FAR * 3.0),
        points,
        levels,
        radius: height * RADIUS_SHARE,
    }
}

/// How tall the rise is beside how far the journey spreads over the map.
const RISE: f32 = 1.6;

/// In 3D: calls rise from the floor over their nodes, seen from where the view is
/// turned, far enough back to take in the whole rise.
fn rising(explorer: &Explorer, view: &mut TimeView, size: Vec2, ease: f32) -> Option<Layout> {
    let shown = view.window.clone();
    let floor: Vec<Vec3> = view.calls[shown.clone()]
        .iter()
        .filter_map(|c| explorer.tile_point(c.tile))
        .collect();
    let (middle, reach) = footprint(&floor)?;
    let reach = reach.max(5.0);
    let rise = reach * RISE;
    let points = view
        .calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            let slot = index
                .checked_sub(shown.start)
                .filter(|_| shown.contains(&index))?;
            let at = explorer.tile_point(call.tile)?;
            Some(Vec3::new(at.x, at.y, along(slot, shown.len()) * rise))
        })
        .collect();
    let center = middle.extend(rise / 2.0);
    let fov = std::f32::consts::FRAC_PI_4;
    let aspect = size.x / size.y.max(1.0);
    // Fit the rise's bounding sphere in the narrower of the two directions.
    // The floor square reaches out to its corners.
    let bound = (reach * 1.15 * std::f32::consts::SQRT_2).hypot(rise / 2.0);
    let half = (fov / 2.0).tan() * aspect.min(1.0);
    // A new fit eases in, so opening or folding calls does not jump the view.
    let goal = (center, bound / half.atan().sin());
    let (center, fit) = match view.fit {
        Some((at, far)) => (at.lerp(goal.0, ease), far + (goal.1 - far) * ease),
        None => goal,
    };
    view.fit = Some((center, fit));
    let distance = fit / view.turn.zoom;
    Some(Layout {
        eye: Transform::from_translation(center + view.turn.eye(distance))
            .looking_at(center, Vec3::Z),
        projection: Projection::Perspective(PerspectiveProjection {
            fov,
            aspect_ratio: aspect,
            near: 0.1,
            far: FAR,
            ..default()
        }),
        fog: (distance - bound * 0.4, distance + bound * 1.6),
        points,
        levels: Vec::new(),
        radius: bound * 0.017,
    })
}

/// Lines sit behind every marker, seen from the side.
const BEHIND: f32 = STAND_OFF;

/// A pale line color, at `alpha`.
fn pale(alpha: f32) -> Color {
    Color::srgba(0.85, 0.88, 0.95, alpha)
}

/// Side-on, each caller's line along its level, from its call to its last, with a
/// drop to each call it makes; the cursor's caller and its drop to the cursor
/// brighter. Callers off the view still have their level.
fn draw_levels(
    calls: &[Call],
    layout: &Layout,
    cursor: Option<usize>,
    gizmos: &mut Gizmos<TimeGizmos>,
) {
    let (points, levels) = (&layout.points, &layout.levels);
    let behind = |y: f32, z: f32| Vec3::new(BEHIND, y, z);
    let lit_caller = cursor.and_then(|c| calls[c].parent);
    for (index, call) in calls.iter().enumerate() {
        let (Some(parent), Some(point)) = (call.parent, points[index]) else {
            continue;
        };
        let Some(level) = levels[parent] else {
            continue;
        };
        let alpha = if Some(index) == cursor { 0.8 } else { 0.25 };
        gizmos.line(
            behind(level, point.z),
            behind(point.y, point.z),
            pale(alpha),
        );
    }
    for (index, level) in levels.iter().enumerate() {
        let last = (0..calls.len())
            .rev()
            .find(|&i| calls[i].parent == Some(index) && points[i].is_some())
            .and_then(|i| points[i]);
        let (Some(last), Some(level)) = (last, *level) else {
            continue;
        };
        // From the left edge when the call itself is off it.
        let from = points[index].map_or(0.0, |p| p.z);
        let alpha = if lit_caller == Some(index) { 0.6 } else { 0.2 };
        gizmos.line(behind(level, from), behind(level, last.z), pale(alpha));
    }
}

/// In 3D: a faint grid and the modules' rings on the floor; from each marker a drop
/// to its node's spot there; over each caller a pole up to its last call, with a level
/// rung out to each call it makes, as a sequence diagram stood on the map; and a frame
/// round the cursor's moment, with the ruler. Lines fade with distance, as the fog
/// fades the markers. Returns the ruler's foot.
fn draw_rise(
    explorer: &Explorer,
    view: &TimeView,
    layout: &Layout,
    cursor: Option<usize>,
    gizmos: &mut Gizmos<TimeGizmos>,
) -> Option<Vec3> {
    let eye = layout.eye.translation;
    let (near, far) = layout.fog;
    let fade = |p: Vec3| 1.0 - ((eye.distance(p) - near) / (far - near)).clamp(0.0, 0.85);
    let floor: Vec<Vec3> = view
        .calls
        .iter()
        .zip(&layout.points)
        .filter(|(_, p)| p.is_some())
        .filter_map(|(c, _)| explorer.tile_point(c.tile))
        .collect();
    let (middle, reach) = footprint(&floor)?;
    let reach = reach.max(5.0) * 1.15;
    draw_floor(explorer, view, layout, middle, reach, gizmos);
    let lit_caller = cursor.and_then(|c| view.calls[c].parent);
    // Where a caller stands: its marker, or for one before the view, its node's spot.
    let caller_at = |index: usize| {
        layout.points[index].or_else(|| {
            let at = explorer.tile_point(view.calls[index].tile)?;
            Some(at.truncate().extend(0.0))
        })
    };
    for (index, call) in view.calls.iter().enumerate() {
        let Some(point) = layout.points[index] else {
            continue;
        };
        let lit = Some(index) == cursor;
        let spot = point.truncate().extend(0.0);
        let color = view.colors.get(index).copied().unwrap_or(Color::WHITE);
        // What is yet to run, past the cursor, steps back.
        let ahead = cursor.is_some_and(|c| index > c);
        let strength = fade(point) * if ahead { 0.55 } else { 1.0 };
        gizmos.line(
            point,
            spot,
            color.with_alpha(if lit { 0.8 } else { 0.3 } * strength),
        );
        gizmos.circle(
            Isometry3d::from_translation(spot),
            layout.radius * 0.6,
            color.with_alpha(0.7 * strength),
        );
        // A level rung from the caller's pole out to the call, at the call's height.
        if let Some(from) = call.parent.and_then(caller_at) {
            let alpha = match () {
                () if lit => 0.9,
                () if lit_caller == call.parent => 0.4,
                () => 0.2,
            };
            gizmos.line(
                from.truncate().extend(point.z),
                point,
                pale(alpha * strength),
            );
        }
    }
    // Each caller's pole, up over its node from its call to its last.
    for caller in 0..view.calls.len() {
        let last = (0..view.calls.len())
            .rev()
            .find(|&i| view.calls[i].parent == Some(caller) && layout.points[i].is_some())
            .and_then(|i| layout.points[i]);
        let (Some(last), Some(from)) = (last, caller_at(caller)) else {
            continue;
        };
        let alpha = if lit_caller == Some(caller) {
            0.6
        } else {
            0.25
        };
        gizmos.line(
            from,
            from.truncate().extend(last.z),
            pale(alpha * fade(from)),
        );
    }
    Some(draw_ruler(layout, cursor, middle, reach, gizmos))
}

/// The cursor's moment, a square across the rise at its height, and a ruler up the
/// floor's far left corner with a tick at each call. Returns the ruler's foot.
fn draw_ruler(
    layout: &Layout,
    cursor: Option<usize>,
    middle: Vec2,
    reach: f32,
    gizmos: &mut Gizmos<TimeGizmos>,
) -> Vec3 {
    let eye = layout.eye.translation;
    // The cursor's moment: a square across the rise at its height.
    if let Some(point) = cursor.and_then(|c| layout.points[c]) {
        let at = middle.extend(point.z);
        gizmos.rect(
            Isometry3d::from_translation(at),
            Vec2::splat(reach * 2.0),
            pale(0.22),
        );
    }
    let foot = ruler_foot(middle, reach, eye);
    let top = layout
        .points
        .iter()
        .flatten()
        .map(|p| p.z)
        .fold(0.0, f32::max);
    gizmos.line(foot, foot + Vec3::Z * top, pale(0.35));
    for (index, point) in layout.points.iter().enumerate() {
        let Some(point) = point else {
            continue;
        };
        let lit = Some(index) == cursor;
        let tick = reach * if lit { 0.08 } else { 0.035 };
        let at = foot + Vec3::Z * point.z;
        gizmos.line(
            at - Vec3::X * tick,
            at + Vec3::X * tick,
            pale(if lit { 0.9 } else { 0.4 }),
        );
    }
    foot
}

/// The floor under the rise: a faint grid `reach` round `middle`, and the rings of
/// the modules the shown calls touch, where the map draws them.
fn draw_floor(
    explorer: &Explorer,
    view: &TimeView,
    layout: &Layout,
    middle: Vec2,
    reach: f32,
    gizmos: &mut Gizmos<TimeGizmos>,
) {
    let cells = 10;
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let spacing = Vec2::splat(reach * 2.0 / cells as f32);
    gizmos.grid(
        Isometry3d::from_translation(middle.extend(0.0)),
        UVec2::splat(cells),
        spacing,
        pale(0.07),
    );
    // The rings of the modules the calls touch, where the map draws them.
    let graph = explorer.graph();
    let touched: Vec<usize> = view
        .calls
        .iter()
        .zip(&layout.points)
        .filter(|(_, p)| p.is_some())
        .filter_map(|(c, _)| graph.cluster(c.tile))
        .collect();
    for (cluster, ring_middle, ring) in graph_view::regions(explorer) {
        if touched.contains(&cluster) {
            let color = lane_color(cluster % PALETTE.len());
            let at = ring_middle.truncate().extend(0.0);
            gizmos
                .circle(
                    Isometry3d::from_translation(at),
                    ring,
                    color.with_alpha(0.3),
                )
                .resolution(48);
        }
    }
}

/// The floor's corner, `reach` from `middle`, on the eye's side across and the far
/// side deep: off to one side of the rise, at its middle distance, so the ruler
/// neither looms in front nor hides behind.
pub(crate) fn ruler_foot(middle: Vec2, reach: f32, eye: Vec3) -> Vec3 {
    // Looking along `ahead` over the floor; the corner beyond and to the left.
    let ahead = (middle - eye.truncate()).normalize_or_zero();
    let toward = ahead + Vec2::new(-ahead.y, ahead.x);
    let corner = Vec2::new(reach.copysign(toward.x), reach.copysign(toward.y));
    (middle + corner).extend(0.0)
}

/// Splits the graph's area into the map, left, and the time view, right, and docks
/// a docked journey on the map; without the time view, all of it is the map.
pub(crate) fn layout(
    mode: TimeMode,
    panel: PanelMode,
    graph: egui::Rect,
    journey: Option<egui::Rect>,
    extent: (usize, f32),
) -> (egui::Rect, Option<egui::Rect>, Option<egui::Rect>) {
    if mode == TimeMode::Off {
        return (graph, None, journey);
    }
    let (map, time) = split(graph);
    let journey = match panel {
        PanelMode::Docked => journey_panel::split(panel, map, extent).1,
        _ => journey,
    };
    (map, Some(time), journey)
}

/// Splits the graph's area into the map, left, and the time view, right.
pub(crate) fn split(area: egui::Rect) -> (egui::Rect, egui::Rect) {
    let cut = area.min.x + area.width() * MAP_SHARE;
    (
        egui::Rect::from_min_max(area.min, egui::pos2(cut, area.max.y)),
        egui::Rect::from_min_max(egui::pos2(cut, area.min.y), area.max),
    )
}

/// How much of the graph's width the map keeps.
const MAP_SHARE: f32 = 0.56;

/// How the map projects: anywhere, and only within its own area.
pub(crate) struct MapProjection<'a> {
    pub(crate) anywhere: &'a dyn Fn(Vec3) -> Option<egui::Pos2>,
    pub(crate) on_map: &'a dyn Fn(Vec3) -> Option<egui::Pos2>,
}

/// What the time view draws over its camera's picture and how the pointer works it:
/// a title, module bands side-on, the cursor's ring and the guide from its node on
/// the map, and a name on each call that fits. Hovering a call lights its node,
/// clicking moves the cursor there; in 3D, dragging turns the view and scrolling zooms.
pub(crate) fn draw_overlay(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &mut Explorer,
    diff: Option<&DiffView>,
    view: &mut TimeView,
    (camera, transform): (&Camera, &GlobalTransform),
    map: &MapProjection,
) {
    let mode = view.mode;
    let to_screen = |p: Vec3| {
        camera
            .world_to_viewport(transform, p)
            .ok()
            .map(|p| egui::pos2(p.x, p.y))
    };
    let painter = ctx
        .layer_painter(egui::LayerId::background())
        .with_clip_rect(area);
    let screens: Vec<Option<egui::Pos2>> =
        view.points.iter().map(|p| p.and_then(to_screen)).collect();
    // A marker's radius on screen, at the cursor's depth or the first shown.
    let sized = view
        .cursor(explorer)
        .and_then(|c| view.points[c])
        .or_else(|| view.points.iter().flatten().next().copied());
    let radius = sized
        .and_then(|p| {
            let up = transform.up().as_vec3() * view.radius;
            Some(to_screen(p)?.distance(to_screen(p + up)?))
        })
        .unwrap_or(4.0);

    title(&painter, area, mode, view);
    let cursor = view.cursor(explorer);
    let mut taken = if mode == TimeMode::Side {
        bands(&painter, area, explorer, view, map.anywhere)
    } else {
        ruler(&painter, view, cursor, &to_screen)
    };
    if let Some(at) = cursor.and_then(|c| screens[c]) {
        if mode == TimeMode::Side {
            painter.line_segment(
                [egui::pos2(at.x, area.min.y), egui::pos2(at.x, area.max.y)],
                egui::Stroke::new(1.0, egui::Color32::from_white_alpha(30)),
            );
        }
        let ring = radius * 1.7 + 4.0;
        painter.circle_stroke(at, ring, egui::Stroke::new(1.5, egui::Color32::WHITE));
        if let Some(node) = cursor
            .and_then(|c| explorer.tile_point(view.calls[c].tile))
            .and_then(map.on_map)
        {
            guide(ctx, node, at, ring);
        }
    }
    interact(ctx, area, mode, explorer, view, &screens, radius);
    let names = Names {
        painter: &painter,
        area,
        screens: &screens,
        radius,
        cursor,
    };
    names.draw(explorer, diff, view, &mut taken);
}

/// In 3D, the ruler's numbers: the first call's, the last's and the cursor's, beside
/// their ticks. Returns where they went.
fn ruler(
    painter: &egui::Painter,
    view: &TimeView,
    cursor: Option<usize>,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
) -> Vec<egui::Rect> {
    let Some(foot) = view.ruler else {
        return Vec::new();
    };
    let shown: Vec<usize> = (0..view.calls.len())
        .filter(|&i| view.points[i].is_some())
        .collect();
    let mut marked: Vec<usize> = [shown.first(), shown.last()]
        .into_iter()
        .flatten()
        .copied()
        .collect();
    marked.extend(cursor);
    let mut taken = Vec::new();
    // The cursor's last, so it wins over the ends it would crowd.
    for index in marked.into_iter().rev() {
        let Some(at) = view.points[index].and_then(|p| to_screen(foot + Vec3::Z * p.z)) else {
            continue;
        };
        let number = &view.calls[index].number;
        let text = if number.is_empty() { "start" } else { number };
        let lit = Some(index) == cursor;
        let galley = painter.layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(if lit { 12.0 } else { 10.0 }),
            if lit { egui::Color32::WHITE } else { ui::MUTED },
        );
        let rect = egui::Rect::from_min_size(
            egui::pos2(at.x - galley.size().x - 10.0, at.y - galley.size().y / 2.0),
            galley.size(),
        );
        if taken.iter().any(|t: &egui::Rect| t.intersects(rect)) {
            continue;
        }
        taken.push(rect.expand(2.0));
        painter.galley(rect.min, galley, egui::Color32::WHITE);
    }
    taken
}

/// What the view is, and how far through the journey it shows when not all of it.
fn title(painter: &egui::Painter, area: egui::Rect, mode: TimeMode, view: &TimeView) {
    let text = match mode {
        TimeMode::Side => "calls in order  →    Z: off",
        _ => "calls rise in order  ↑    drag or shift-Z to turn · Z: side-on",
    };
    painter.line_segment(
        [area.left_top(), area.left_bottom()],
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(40)),
    );
    painter.text(
        area.left_top() + egui::vec2(10.0, 8.0),
        egui::Align2::LEFT_TOP,
        text,
        egui::FontId::proportional(11.0),
        ui::MUTED,
    );
    if view.window.start > 0 || view.window.end < view.calls.len() {
        painter.text(
            area.right_top() + egui::vec2(-10.0, 8.0),
            egui::Align2::RIGHT_TOP,
            format!(
                "{}–{} of {}",
                view.window.start + 1,
                view.window.end,
                view.calls.len()
            ),
            egui::FontId::proportional(11.0),
            ui::MUTED,
        );
    }
}

/// Side-on, a faint band across the view for each module the shown calls touch, at
/// the height of its ring on the map, named at the left. Returns where the names went.
fn bands(
    painter: &egui::Painter,
    area: egui::Rect,
    explorer: &Explorer,
    view: &TimeView,
    map: &dyn Fn(Vec3) -> Option<egui::Pos2>,
) -> Vec<egui::Rect> {
    let mut taken: Vec<egui::Rect> = Vec::new();
    let graph = explorer.graph();
    let touched: Vec<usize> = view
        .calls
        .iter()
        .zip(&view.points)
        .filter(|(_, p)| p.is_some())
        .filter_map(|(c, _)| graph.cluster(c.tile))
        .collect();
    for (cluster, middle, ring) in graph_view::regions(explorer) {
        if !touched.contains(&cluster) {
            continue;
        }
        let (Some(top), Some(bottom)) =
            (map(middle + Vec3::Y * ring), map(middle - Vec3::Y * ring))
        else {
            continue;
        };
        let color = egui_color(lane_color(cluster % PALETTE.len()));
        let band = egui::Rect::from_x_y_ranges(area.x_range(), top.y..=bottom.y);
        painter.rect_filled(band, 0.0, color.gamma_multiply(0.035));
        let name = painter.layout_no_wrap(
            explorer.cluster_name(cluster).to_uppercase(),
            egui::FontId::proportional(10.0),
            color.gamma_multiply(0.7),
        );
        let at = egui::pos2(area.min.x + 8.0, band.center().y - name.size().y / 2.0);
        let rect = egui::Rect::from_min_size(at, name.size());
        if !taken.iter().any(|t| t.intersects(rect)) {
            taken.push(rect.expand(2.0));
            painter.galley(at, name, egui::Color32::WHITE);
        }
    }
    taken
}

/// The pointer in the time view: the nearest call within reach lights its node in
/// both views and a click moves the cursor to it; in 3D a drag turns the view, a
/// scroll zooms and a double click turns it back.
fn interact(
    ctx: &egui::Context,
    area: egui::Rect,
    mode: TimeMode,
    explorer: &mut Explorer,
    view: &mut TimeView,
    screens: &[Option<egui::Pos2>],
    radius: f32,
) {
    // Claims the pointer, so the map's camera leaves it alone here.
    let response = egui::Area::new(egui::Id::new("time_view"))
        .fixed_pos(area.min)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.allocate_rect(area, egui::Sense::click_and_drag())
        })
        .inner;
    if mode == TimeMode::Rise {
        if response.dragged() {
            let delta = response.drag_delta();
            view.turn.drag(Vec2::new(delta.x, delta.y));
        }
        if response.hovered() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            view.turn.zoom = (view.turn.zoom * (scroll * 0.003).exp()).clamp(0.4, 6.0);
        }
        if response.double_clicked() {
            view.turn = Turn::default();
        }
    }
    let hover = response.hover_pos();
    let near = hover.and_then(|p| {
        screens
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.map(|s| (i, s.distance(p))))
            .filter(|&(_, d)| d < (radius + 8.0).max(10.0))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    });
    if hover.is_some() {
        let tile = near.map(|i| view.calls[i].tile);
        if explorer.hovered_tile != tile {
            explorer.hovered_tile = tile;
        }
        if let Some(index) = near
            && response.clicked()
        {
            explorer.select(Some(view.calls[index].span));
        }
    } else if view.hovered.is_some() {
        explorer.hovered_tile = None;
    }
    view.hovered = near;
}

/// How many calls either side of the cursor the 3D view names.
const NAMED_AROUND: usize = 12;

/// Names on the time view's calls.
struct Names<'a> {
    painter: &'a egui::Painter,
    area: egui::Rect,
    screens: &'a [Option<egui::Pos2>],
    radius: f32,
    cursor: Option<usize>,
}

impl Names<'_> {
    /// Where a call's marker, or the cursor's ring, covers.
    fn disc(&self, index: usize, at: egui::Pos2) -> egui::Rect {
        let reach = if Some(index) == self.cursor {
            self.radius * 1.7 + 5.0
        } else {
            self.radius + 1.0
        };
        egui::Rect::from_center_size(at, egui::Vec2::splat(reach * 2.0))
    }

    /// The cursor's name first, then what the pointer is over, then outwards from the
    /// cursor in time, wherever they fit beside their markers without covering one.
    fn draw(
        &self,
        explorer: &Explorer,
        diff: Option<&DiffView>,
        view: &TimeView,
        taken: &mut Vec<egui::Rect>,
    ) {
        let hovered_tile = explorer.hovered_tile;
        let lit = |i: usize| Some(i) == self.cursor || hovered_tile == Some(view.calls[i].tile);
        let mut order: Vec<usize> = (0..view.calls.len())
            .filter(|&i| self.screens[i].is_some())
            .collect();
        let anchor = self.cursor.unwrap_or(view.window.start);
        order.sort_by_key(|&i| (!lit(i), i.abs_diff(anchor)));
        // In 3D, where calls crowd in depth, only the stretch around the cursor.
        if view.mode == TimeMode::Rise {
            order.retain(|&i| lit(i) || i.abs_diff(anchor) <= NAMED_AROUND);
        }
        taken.extend(
            self.screens
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.map(|s| self.disc(i, s))),
        );
        for index in order {
            let Some(at) = self.screens[index] else {
                continue;
            };
            let call = &view.calls[index];
            let fill = egui_color(node_color(explorer, diff, call.tile));
            let lead = (!call.number.is_empty())
                .then(|| (format!(" {} ", call.number), ui::ON_FILL, Some(fill)));
            let color = if lit(index) {
                egui::Color32::WHITE
            } else {
                egui::Color32::from_rgb(200, 205, 216)
            };
            let name = ui::node_name(explorer, call.tile);
            let size = if lit(index) { 12.0 } else { 11.0 };
            let galley = self
                .painter
                .layout_job(ui::label_job(lead, &name, None, color, size, false));
            let own = self.disc(index, at);
            let Some(rect) = self.spot(at, own, galley.size(), taken, lit(index)) else {
                continue;
            };
            taken.push(rect.expand(1.0));
            if lit(index) {
                self.painter.rect_filled(
                    rect.expand(2.0),
                    3.0,
                    egui::Color32::from_rgba_unmultiplied(12, 14, 20, 200),
                );
            }
            self.painter.galley(rect.min, galley, egui::Color32::WHITE);
        }
    }

    /// Right of the marker, else left, above or below; a lit name goes right come
    /// what may.
    fn spot(
        &self,
        at: egui::Pos2,
        own: egui::Rect,
        size: egui::Vec2,
        taken: &[egui::Rect],
        lit: bool,
    ) -> Option<egui::Rect> {
        let gap = own.width() / 2.0 + 3.0;
        let spots = [
            egui::pos2(at.x + gap, at.y - size.y / 2.0),
            egui::pos2(at.x - gap - size.x, at.y - size.y / 2.0),
            egui::pos2(at.x - size.x / 2.0, at.y - gap - size.y),
            egui::pos2(at.x - size.x / 2.0, at.y + gap),
        ]
        .map(|p| egui::Rect::from_min_size(p, size));
        spots
            .into_iter()
            .find(|&rect| {
                self.area.contains_rect(rect)
                    && !taken.iter().any(|t| *t != own && t.intersects(rect))
            })
            .or_else(|| lit.then_some(spots[0]))
    }
}

/// A dashed line from the cursor's node on the map across to its marker, stopping
/// short of the marker's ring.
fn guide(ctx: &egui::Context, node: egui::Pos2, marker: egui::Pos2, ring: f32) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("time_guide"),
    ));
    let along = marker - node;
    let length = along.length();
    if length < ring + 12.0 {
        return;
    }
    let end = node + along * ((length - ring) / length);
    let start = node + along * (12.0 / length);
    painter.extend(egui::Shape::dashed_line(
        &[start, end],
        egui::Stroke::new(1.2, egui::Color32::from_white_alpha(150)),
        6.0,
        4.0,
    ));
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
            in_order(0, tree),
            vec![
                (0, None),
                (1, Some(0)),
                (2, Some(1)),
                (3, Some(1)),
                (4, Some(0)),
                (2, Some(4)),
            ]
        );
    }

    #[test]
    fn the_window_holds_everything_that_fits_else_centers_the_cursor() {
        assert_eq!(window(5, Some(3), 10), 0..5);
        assert_eq!(window(50, Some(25), 10), 20..30);
        assert_eq!(window(50, Some(2), 10), 0..10, "clamped at the start");
        assert_eq!(window(50, Some(49), 10), 40..50, "and at the end");
        assert_eq!(window(50, None, 10), 0..10);
        assert_eq!(window(3, None, 0), 0..1, "always one");
    }

    #[test]
    fn calls_are_spaced_evenly_inside_the_span() {
        let slots: Vec<f32> = (0..7).map(|i| along(i, 7)).collect();
        assert!(slots.windows(2).all(|w| w[1] > w[0]));
        let steps: Vec<f32> = slots.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(steps.iter().all(|s| (s - steps[0]).abs() < 1e-5));
        assert!(slots[0] > 0.0 && slots[6] < 1.0);
    }

    #[test]
    fn heights_are_as_the_map_shows_them() {
        let eye = Transform::from_xyz(0.0, 0.0, 10.0);
        let fov = std::f32::consts::FRAC_PI_2;
        assert!(apparent(&eye, fov, Vec3::ZERO).is_some_and(|y| y.abs() < 1e-6));
        let top = apparent(&eye, fov, Vec3::new(3.0, 10.0, 0.0)).unwrap_or_default();
        assert!((top - 1.0).abs() < 1e-5, "at the top edge");
        let nearer = apparent(&eye, fov, Vec3::new(0.0, 5.0, 5.0)).unwrap_or_default();
        assert!(
            (nearer - 1.0).abs() < 1e-5,
            "a nearer node shows further out"
        );
        assert!(
            apparent(&eye, fov, Vec3::new(0.0, 0.0, 20.0)).is_none(),
            "behind"
        );
    }

    #[test]
    fn the_default_turn_looks_from_the_front_left_and_above() {
        let eye = Turn::default().eye(10.0);
        assert!((eye.length() - 10.0).abs() < 1e-4);
        assert!(eye.x < 0.0, "left");
        assert!(eye.y < 0.0, "in front, south of the map");
        assert!(eye.z > 0.0, "above the floor");
        let mut turn = Turn::default();
        turn.drag(Vec2::new(0.0, 10_000.0));
        assert!(turn.pitch <= 1.5, "never over the top");
    }

    #[test]
    fn the_ruler_stands_to_the_side_not_in_front() {
        // The eye in front (south) and a little left: the ruler at the far left corner.
        let foot = ruler_foot(Vec2::ZERO, 1.0, Vec3::new(-0.3, -5.0, 2.0));
        assert_eq!(foot, Vec3::new(-1.0, 1.0, 0.0));
        // From the east: the far corner is west, the side the near corner on the south.
        let foot = ruler_foot(Vec2::ZERO, 1.0, Vec3::new(5.0, 0.3, 2.0));
        assert_eq!(foot, Vec3::new(-1.0, -1.0, 0.0));
    }

    #[test]
    fn the_footprint_holds_every_node() {
        let points = [Vec3::new(0.0, 0.0, 1.0), Vec3::new(4.0, 0.0, -1.0)];
        let (middle, reach) = footprint(&points).unwrap_or_default();
        assert_eq!(middle, Vec2::new(4.0, 0.0), "the median");
        assert!(reach.abs() < 1e-6);
        let mut spread: Vec<Vec3> = (0..9).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();
        spread.push(Vec3::new(1_000.0, 0.0, 0.0));
        let (middle, reach) = footprint(&spread).unwrap_or_default();
        assert!((middle.x - 5.0).abs() < 1e-6);
        assert!(reach < 10.0, "one far call does not count");
        assert!(footprint(&[]).is_none());
    }

    #[test]
    fn the_map_keeps_the_left_of_the_graph() {
        let area = egui::Rect::from_min_max(egui::pos2(0.0, 30.0), egui::pos2(1000.0, 900.0));
        let (map, time) = split(area);
        assert_eq!(map.min, area.min);
        assert_eq!(time.max, area.max);
        assert!((map.max.x - time.min.x).abs() < f32::EPSILON);
        assert!(
            (map.height() - time.height()).abs() < f32::EPSILON,
            "same height"
        );
        assert!(map.width() > time.width());
    }
}
