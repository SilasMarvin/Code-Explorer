//! Time as depth: the journey's calls lie along the line of sight in the order they
//! run, so stepping through the journey flies through it. The camera faces the map
//! head-on; the cursor's call sits on the map's plane at its usual size, calls still to
//! come lie deeper ahead, smaller and hazed, receding towards the middle of the view
//! along a thread that joins the next few in order, and calls already made come nearer,
//! larger and see-through, as if passed by. `Shift Z` turns time the other way round.
//! Everything off the journey stays on the plane, so the map reads as it always does;
//! a soft pool of light marks the cursor's node on it.
//!
//! A node is one function but may be called several times; it sits at whichever of its
//! calls is nearest the cursor's, so it comes back towards the plane as its next call
//! nears.
//!
//! Two ways to place a call off the plane, on `Z`: `fly` moves it straight towards or
//! away from the map, so it drifts out from the middle of the view as it nears, as
//! stars do in a fly-through; `held` slides it along the line from the eye through its
//! place on the map, so it stays on its spot on screen and only its size, glow and
//! fade say when it runs.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_keyboard_input;
use codeviz_core::SpanId;

use crate::{
    camera::{FRONT, OrbitCamera},
    explorer::Explorer,
    graph_view::{LEVELS, NodeLook, NodeMaterials, node_color, node_look, node_size},
    scene::Tile,
    ui::Chrome,
};

/// How the journey's order shows in depth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TimeMode {
    #[default]
    Off,
    /// Calls move straight off the map's plane, by when they run.
    Fly,
    /// Calls move along the line of sight, so each stays on its spot on screen.
    Held,
}

impl TimeMode {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Off => Self::Fly,
            Self::Fly => Self::Held,
            Self::Held => Self::Off,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Fly => "fly",
            Self::Held => "held",
        }
    }

    pub(crate) fn parse(name: &str) -> Self {
        match name {
            "fly" => Self::Fly,
            "held" => Self::Held,
            _ => Self::Off,
        }
    }
}

/// Which way time runs along the line of sight.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Ahead {
    /// Calls to come lie deeper, receding into the distance, and calls made come
    /// nearer as they are passed: stepping on flies forward into the journey.
    #[default]
    Deep,
    /// Calls to come hang nearer, and calls made sink behind the map.
    Near,
}

impl Ahead {
    pub(crate) fn flip(self) -> Self {
        match self {
            Self::Deep => Self::Near,
            Self::Near => Self::Deep,
        }
    }

    /// How many calls nearer the eye than the cursor's a call `step` calls from it
    /// stands, measured as steps: positive is towards the eye.
    pub(crate) fn nearness(self, step: f32) -> f32 {
        match self {
            Self::Deep => -step,
            Self::Near => step,
        }
    }
}

/// Where time puts each node, eased as the cursor moves.
#[derive(Resource, Debug, Default)]
pub(crate) struct Flight {
    /// How far each node is drawn from where the simulation puts it.
    offsets: HashMap<usize, Vec3>,
    /// How strongly each node on the journey draws, by how far it runs from the cursor.
    fades: HashMap<usize, f32>,
    /// Calls from the cursor's to each node's nearest, eased: negative is past.
    steps: HashMap<usize, f32>,
    /// The mode last drawn, so turning it on turns the camera to face the map.
    mode: TimeMode,
    /// Each node's call nearest the cursor's, by its number, and how many times the
    /// journey calls it.
    nearest: HashMap<usize, (String, usize)>,
    /// The map's plane, as far along the head-on view as the node the camera orbits.
    plane: f32,
    /// Which way time runs (`Shift Z`).
    pub(crate) ahead: Ahead,
    /// The cursor's node, then the nodes of the next few calls, in the order they run.
    corridor: Vec<usize>,
}

impl Flight {
    /// Whether time shows as depth.
    pub(crate) fn on(&self) -> bool {
        self.mode != TimeMode::Off
    }

    /// How strongly a node draws: in full off the journey and at the cursor.
    pub(crate) fn fade(&self, tile: usize) -> f32 {
        self.fades.get(&tile).copied().unwrap_or(1.0)
    }

    /// The number of a node's call nearest the cursor's, the one it sits at, and how
    /// many times the journey calls it.
    pub(crate) fn nearest(&self, tile: usize) -> Option<(&str, usize)> {
        self.nearest
            .get(&tile)
            .map(|(number, count)| (number.as_str(), *count))
    }

    /// How much further from the middle of the view the calls the camera frames can be
    /// drawn than where they sit on the map: flying, calls to come drift outwards.
    pub(crate) fn spread(&self) -> f32 {
        match self.mode {
            #[expect(clippy::cast_precision_loss, reason = "a small count")]
            TimeMode::Fly => 1.0 / (1.0 - depth(WINDOW as f32)),
            TimeMode::Held | TimeMode::Off => 1.0,
        }
    }

    /// Calls between the cursor's and a node's nearest, when it is on the journey.
    pub(crate) fn step(&self, tile: usize) -> Option<f32> {
        self.steps.get(&tile).copied()
    }

    /// How much larger than on the map a node looks, for sizing its name.
    pub(crate) fn size(&self, tile: usize) -> f32 {
        self.step(tile)
            .map_or(1.0, |step| apparent_size(self.ahead.nearness(step)))
    }

    /// The node of the call right after the cursor's, whose name always shows.
    pub(crate) fn next(&self) -> Option<usize> {
        self.corridor.get(1).copied()
    }
}

pub(crate) struct FlyPlugin;

impl Plugin for FlyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Flight>()
            .init_gizmo_group::<CorridorGizmos>()
            .add_systems(Startup, (spawn_pool, corridor_width))
            .add_systems(
                Update,
                (
                    flip_ahead.run_if(not(egui_wants_any_keyboard_input)),
                    haze,
                    light_pool,
                ),
            );
    }
}

/// The disclosed journey's calls as they run, depth first, by span.
pub(crate) fn run_order<S: Copy>(root: S, children: impl Fn(S) -> Vec<S>) -> Vec<S> {
    let mut order = Vec::new();
    let mut stack = vec![root];
    while let Some(span) = stack.pop() {
        order.push(span);
        stack.extend(children(span).into_iter().rev());
    }
    order
}

/// For each node in `calls`, the calls between `cursor` and its nearest call: negative
/// when that call has run, positive when it is to come. An equally near past call
/// wins, as it has been seen.
pub(crate) fn steps_from(calls: &[usize], cursor: usize) -> HashMap<usize, i32> {
    let mut steps: HashMap<usize, i32> = HashMap::new();
    for (index, &tile) in calls.iter().enumerate() {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "a journey's calls number far fewer than i32::MAX"
        )]
        let step = index as i32 - cursor as i32;
        steps
            .entry(tile)
            .and_modify(|best| {
                if step.abs() < best.abs() {
                    *best = step;
                }
            })
            .or_insert(step);
    }
    steps
}

/// How far towards the camera a call `near` calls nearer the eye than the cursor's
/// sits (see [`Ahead::nearness`]), as a share of the camera's distance: calls on the
/// far side fall away behind the map, quickly at first, those on the near side rise
/// towards the eye but never even half way, so none fills the view.
pub(crate) fn depth(near: f32) -> f32 {
    if near >= 0.0 {
        NEAREST * (1.0 - (-near / 2.5).exp())
    } else {
        -DEEPEST * (1.0 - (near / 3.0).exp())
    }
}

/// How much larger than on the map a call `near` calls nearer the eye looks, for
/// sizing its name as its node: within reason, so a name stays legible.
pub(crate) fn apparent_size(near: f32) -> f32 {
    (1.0 / (1.0 - depth(near))).clamp(0.8, 1.3)
}

/// The furthest towards the eye a call comes, and the furthest behind the map it falls,
/// as shares of the camera's distance.
const NEAREST: f32 = 0.45;
const DEEPEST: f32 = 1.4;

/// How strongly a call `near` calls nearer the eye than the cursor's draws: in full
/// at the cursor, fading with each call, to a trace four or five calls away.
pub(crate) fn strength(near: f32) -> f32 {
    let away = near.abs();
    // What is nearer fades faster: it would hide the map.
    let rate = if near > 0.0 { 0.22 } else { 0.16 };
    (1.0 - away * rate).clamp(TRACE, 1.0)
}

/// What is left of a call far from the cursor.
const TRACE: f32 = 0.12;

/// The disclosed journey's calls in the order they run, by node and span, and where the
/// cursor is among them.
fn journey_calls(explorer: &Explorer) -> (Vec<(usize, SpanId)>, Option<usize>) {
    let Some(journey) = explorer.journey() else {
        return (Vec::new(), None);
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
    let cursor = explorer.selected();
    let mut calls = Vec::new();
    let mut at = None;
    for span in run_order(journey.root(), shown) {
        // A call whose node the graph lacks takes no place.
        if let Some(tile) = explorer.unit_id(span).and_then(|u| map.tile_of(u)) {
            if Some(span) == cursor {
                at = Some(calls.len());
            }
            calls.push((tile, span));
        }
    }
    (calls, at)
}

/// The nodes of the calls within [`WINDOW`] of the cursor's, as the camera frames them.
pub(crate) fn window(explorer: &Explorer) -> Vec<usize> {
    let (calls, cursor) = journey_calls(explorer);
    let Some(cursor) = cursor else {
        return Vec::new();
    };
    let from = cursor.saturating_sub(WINDOW);
    calls
        .into_iter()
        .skip(from)
        .take(WINDOW * 2 + 1)
        .map(|(tile, _)| tile)
        .collect()
}

/// Calls either side of the cursor's the camera keeps in view.
const WINDOW: usize = 2;

/// For each node `steps` places, the number of the call it sits at and how many calls
/// the journey makes to it.
fn nearest_numbers(
    explorer: &Explorer,
    calls: &[(usize, SpanId)],
    cursor: Option<usize>,
    steps: &HashMap<usize, i32>,
) -> HashMap<usize, (String, usize)> {
    let Some(cursor) = cursor else {
        return HashMap::new();
    };
    let disclosed = explorer.disclosure();
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for &(tile, _) in calls {
        *counts.entry(tile).or_default() += 1;
    }
    steps
        .iter()
        .filter_map(|(&tile, &step)| {
            let index = cursor.checked_add_signed(isize::try_from(step).ok()?)?;
            let &(_, span) = calls.get(index)?;
            let number = disclosed.number(span)?.to_owned();
            Some((tile, (number, counts.get(&tile).copied().unwrap_or(1))))
        })
        .collect()
}

/// The corridor ahead of the cursor: its node, then the nodes of the calls after it in
/// the order they run, a node called twice running only once, up to [`CORRIDOR`] on.
pub(crate) fn corridor(from: &[usize]) -> Vec<usize> {
    let mut path: Vec<usize> = Vec::new();
    for &tile in from {
        if path.len() > CORRIDOR {
            break;
        }
        if path.last() != Some(&tile) {
            path.push(tile);
        }
    }
    path
}

/// The corridor's thread, drawn thicker than links.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct CorridorGizmos;

fn corridor_width(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<CorridorGizmos>().0.line.width = 3.5;
}

/// Calls after the cursor's the corridor runs through.
const CORRIDOR: usize = 4;

/// Where a point on the map's plane at `flat` is drawn `rise` towards the eye: straight
/// up off the plane, or held on its line of sight so it stays on its spot on screen.
fn lift(mode: TimeMode, flat: Vec3, rise: f32, eye: Vec3) -> Vec3 {
    match mode {
        TimeMode::Held => {
            let sight = eye - flat;
            flat + sight * (rise / sight.z.max(1.0))
        }
        _ => flat + Vec3::Z * rise,
    }
}

/// Lays the journey out in depth by when each call runs, easing as the cursor moves,
/// and fades each call by how far it runs from the cursor's; eases everything back to
/// the map when off.
#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
pub(crate) fn fly(
    time: Res<Time>,
    chrome: Res<Chrome>,
    mut flight: ResMut<Flight>,
    mut explorer: ResMut<Explorer>,
    materials: Res<NodeMaterials>,
    diff: Option<Res<crate::diff_view::DiffView>>,
    mut cameras: Query<(&mut OrbitCamera, &Transform)>,
    mut nodes: Query<
        (&Tile, &mut Transform, &mut MeshMaterial3d<StandardMaterial>),
        Without<OrbitCamera>,
    >,
) {
    let flight = &mut *flight;
    let mode = chrome.time;
    let turned_on = mode != TimeMode::Off && flight.mode == TimeMode::Off;
    let turned_off = mode == TimeMode::Off && flight.mode != TimeMode::Off;
    flight.mode = mode;
    if mode == TimeMode::Off && flight.offsets.is_empty() && flight.fades.is_empty() {
        return;
    }
    let Ok((mut camera, view)) = cameras.single_mut() else {
        return;
    };
    if turned_on {
        camera.turn_to(FRONT);
    }
    let explorer_ref = explorer.bypass_change_detection();
    let (calls, cursor) = journey_calls(explorer_ref);
    let tiles: Vec<usize> = calls.iter().map(|&(tile, _)| tile).collect();
    let steps = match cursor.filter(|_| mode != TimeMode::Off) {
        Some(cursor) => steps_from(&tiles, cursor),
        None => HashMap::new(),
    };
    flight.nearest = nearest_numbers(explorer_ref, &calls, cursor, &steps);
    flight.corridor = match cursor.filter(|_| mode != TimeMode::Off) {
        Some(cursor) => corridor(&tiles[cursor..]),
        None => Vec::new(),
    };
    // The map's plane: through the node the camera orbits, square to the head-on view.
    let plane = explorer_ref
        .anchor()
        .and_then(|t| explorer_ref.force_point(t))
        .map_or(0.0, |p| p.z);
    let eye = view.translation;
    let reach = camera.distance;
    let ease = 1.0 - (-time.delta_secs() * 5.0).exp();

    // Eased steps, so a node glides from one place in time to the next.
    for (&tile, &step) in &steps {
        #[expect(clippy::cast_precision_loss, reason = "a small count")]
        let step = step as f32;
        let eased = flight.steps.entry(tile).or_insert(step);
        *eased += (step - *eased) * ease;
    }
    flight.steps.retain(|tile, _| steps.contains_key(tile));
    flight.plane = plane;

    let mut offsets = std::mem::take(&mut flight.offsets);
    for tile in 0..explorer_ref.map().tiles.len() {
        let Some(home) = explorer_ref.force_point(tile) else {
            continue;
        };
        let goal = if mode == TimeMode::Off {
            Vec3::ZERO
        } else {
            let flat = Vec3::new(home.x, home.y, plane);
            let rise = flight
                .steps
                .get(&tile)
                .map_or(0.0, |&s| depth(flight.ahead.nearness(s)) * reach);
            lift(mode, flat, rise, eye) - home
        };
        let offset = offsets.entry(tile).or_insert(Vec3::ZERO);
        *offset += (goal - *offset) * ease;
    }
    offsets.retain(|_, offset| mode != TimeMode::Off || offset.length() > 0.01);
    flight.offsets = offsets;
    explorer_ref.set_nudges(flight.offsets.clone());

    // Fades ease too; one left at full is dropped once it has been drawn so.
    for (&tile, &step) in &flight.steps {
        let fade = flight.fades.entry(tile).or_insert(1.0);
        *fade += (strength(flight.ahead.nearness(step)) - *fade) * ease;
    }
    for (&tile, fade) in &mut flight.fades {
        if !flight.steps.contains_key(&tile) {
            *fade += (1.0 - *fade) * ease;
        }
    }
    let styling = diff.is_none();
    for (tile, mut transform, mut material) in &mut nodes {
        if let Some(at) = explorer_ref.tile_point(tile.0) {
            transform.translation = at;
        }
        let Some(&fade) = flight.fades.get(&tile.0) else {
            continue;
        };
        if !styling {
            continue;
        }
        let near = flight.steps.get(&tile.0).map(|&s| flight.ahead.nearness(s));
        let Some((look, level)) = time_look(explorer_ref, tile.0, fade, near) else {
            continue;
        };
        if let Some(wanted) = materials.0.get(&(look, level))
            && material.0 != *wanted
        {
            material.0 = wanted.clone();
        }
    }
    flight
        .fades
        .retain(|tile, fade| flight.steps.contains_key(tile) || (*fade - 1.0).abs() > 0.01);
    if turned_off {
        // Everything restyles as it was.
        explorer.set_changed();
    }
}

/// How a node `near` calls nearer the eye than the cursor's is drawn, at `fade`: what
/// stands nearer is a see-through ghost, so it never hides the map behind it, and the
/// rest of the journey a lit ball, shaded by the light, so it reads as solid. Nothing
/// for a node the lens leaves out.
fn time_look(
    explorer: &Explorer,
    tile: usize,
    fade: f32,
    near: Option<f32>,
) -> Option<(NodeLook, u8)> {
    let look = node_look(explorer, tile);
    if look == NodeLook::Hidden {
        return None;
    }
    let look = match (look, near) {
        (NodeLook::Near(color, _) | NodeLook::Faded(color) | NodeLook::Plain(color), Some(n))
            if n > 0.5 =>
        {
            NodeLook::Ghost(color)
        }
        (NodeLook::Near(color, _) | NodeLook::Faded(color) | NodeLook::Plain(color), Some(_)) => {
            NodeLook::Lit(color)
        }
        (look, _) => look,
    };
    let shown = explorer.visibility(tile) * fade;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=LEVELS"
    )]
    let level = (shown * f32::from(LEVELS))
        .round()
        .clamp(0.0, f32::from(LEVELS)) as u8;
    Some((look, level))
}

/// From each call off the map's plane, a pale stem down to its spot on the map, so it
/// is plain where it belongs however far it has drifted; and a thread from the
/// cursor's call through the next few, in order, so the way ahead reads as a path.
pub(crate) fn draw_flight(
    flight: Res<Flight>,
    explorer: Res<Explorer>,
    cameras: Query<(&Transform, &OrbitCamera)>,
    mut gizmos: Gizmos,
    mut thread: Gizmos<CorridorGizmos>,
) {
    if !flight.on() {
        return;
    }
    let Ok((view, camera)) = cameras.single() else {
        return;
    };
    let graph = explorer.graph();
    let facing = |at: Vec3| Isometry3d::new(at, view.rotation);
    let points: Vec<(Vec3, f32)> = flight
        .corridor
        .iter()
        .filter_map(|&tile| Some((explorer.tile_point(tile)?, flight.fade(tile))))
        .collect();
    for (index, pair) in points.windows(2).enumerate() {
        let ((a, fa), (b, fb)) = (pair[0], pair[1]);
        #[expect(clippy::cast_precision_loss, reason = "a few legs")]
        let along = 1.0 - index as f32 / CORRIDOR as f32;
        let color = |fade: f32| Color::srgba(0.92, 0.94, 1.0, 0.85 * along * fade.max(0.5));
        thread.line_gradient(a, b, color(fa), color(fb));
        // A ring where the thread reaches each call, so its stations read.
        thread.circle(facing(b), camera.distance * 0.006, color(fb));
    }
    for &tile in flight.steps.keys() {
        let fade = flight.fade(tile);
        let (Some(home), Some(at)) = (explorer.force_point(tile), explorer.tile_point(tile)) else {
            continue;
        };
        let home = Vec3::new(home.x, home.y, flight.plane);
        // Held on its line of sight, a call's stem is hidden behind it.
        if flight.mode == TimeMode::Held || fade <= TRACE + 0.02 || home.distance(at) < 0.3 {
            continue;
        }
        let color = Color::srgba(0.85, 0.88, 0.95, 0.15 + 0.4 * fade);
        gizmos.line(home, at, color);
        gizmos.circle(facing(home), node_size(graph, tile) * 0.22, color);
    }
}

/// `Shift Z` turns which way time runs along the line of sight.
fn flip_ahead(keys: Res<ButtonInput<KeyCode>>, mut flight: ResMut<Flight>) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if shift && keys.just_pressed(KeyCode::KeyZ) {
        flight.ahead = flight.ahead.flip();
    }
}

/// The window's background, which haze fades towards.
const BACKDROP: Color = Color::srgb(0.06, 0.07, 0.09);

/// Haze on the scene while time shows as depth: none on the map's plane, thickening
/// behind it, so calls deep in the distance grey into the background.
fn haze(
    mut commands: Commands,
    flight: Res<Flight>,
    cameras: Query<(Entity, &OrbitCamera, Option<&DistanceFog>)>,
) {
    for (entity, camera, fog) in &cameras {
        if !flight.on() {
            if fog.is_some() {
                commands.entity(entity).remove::<DistanceFog>();
            }
            continue;
        }
        commands.entity(entity).insert(DistanceFog {
            color: BACKDROP.with_alpha(HAZE),
            directional_light_color: Color::NONE,
            directional_light_exponent: 8.0,
            falloff: FogFalloff::Linear {
                start: camera.distance * 1.02,
                end: camera.distance * (1.0 + DEEPEST),
            },
        });
    }
}

/// The most haze there is: the deepest calls dim and grey but never vanish.
const HAZE: f32 = 0.8;

/// The soft pool of light on the map round the cursor's node.
#[derive(Component)]
struct Pool;

fn spawn_pool(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(images.add(spot_image(64))),
        alpha_mode: AlphaMode::Add,
        unlit: true,
        fog_enabled: false,
        ..default()
    });
    commands.spawn((
        Pool,
        Mesh3d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial3d(material),
        Transform::default(),
        Visibility::Hidden,
        bevy::picking::Pickable::IGNORE,
    ));
}

/// How opaque a soft round spot is `distance` from its middle, 1 being its edge.
pub(crate) fn spot_alpha(distance: f32) -> f32 {
    let t = (1.0 - distance).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A white spot fading out to its edge, `size` pixels across.
fn spot_image(size: u32) -> Image {
    use bevy::{
        asset::RenderAssetUsages,
        render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    };
    let across = |i: u32| {
        #[expect(clippy::cast_precision_loss, reason = "a small image")]
        let at = (i as f32 + 0.5) / size as f32;
        at * 2.0 - 1.0
    };
    let mut data = Vec::new();
    for y in 0..size {
        for x in 0..size {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "within a channel"
            )]
            let alpha = (spot_alpha(across(x).hypot(across(y))) * 255.0).round() as u8;
            data.extend([255, 255, 255, alpha]);
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

/// Lays the pool of light on the map round the cursor's node, in its color, so where
/// the journey is now reads at a glance.
fn light_pool(
    flight: Res<Flight>,
    explorer: Res<Explorer>,
    mut pools: Query<
        (
            &mut Transform,
            &mut Visibility,
            &MeshMaterial3d<StandardMaterial>,
        ),
        With<Pool>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cameras: Query<&OrbitCamera>,
) {
    let reach = cameras.single().map_or(60.0, |c| c.distance);
    let Ok((mut transform, mut visibility, material)) = pools.single_mut() else {
        return;
    };
    let cursor = flight.corridor.first().copied().filter(|_| flight.on());
    let Some((tile, home)) = cursor.and_then(|t| Some((t, explorer.force_point(t)?))) else {
        *visibility = Visibility::Hidden;
        return;
    };
    let size = node_size(explorer.graph(), tile);
    *visibility = Visibility::Visible;
    *transform = Transform::from_translation(Vec3::new(home.x, home.y, flight.plane - size))
        .with_scale(Vec3::splat(POOL * reach).with_z(1.0));
    if let Some(mut material) = materials.get_mut(&material.0) {
        let tint = node_color(&explorer, None, tile).mix(&Color::WHITE, 0.25);
        material.base_color = tint.with_alpha(0.22);
    }
}

/// The pool's width, as a share of the camera's distance, so it reads at any zoom.
const POOL: f32 = 0.07;

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
        assert_eq!(run_order(0, tree), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_node_called_again_sits_at_its_call_nearest_the_cursor() {
        // Node 7 runs at calls 1 and 5; node 3 at 2 only.
        let calls = [0, 7, 3, 8, 9, 7];
        let at = steps_from(&calls, 2);
        assert_eq!(at[&3], 0, "the cursor's node sits on the plane");
        assert_eq!(at[&7], -1, "nearer its first call");
        assert_eq!(at[&0], -2);
        assert_eq!(steps_from(&calls, 4)[&7], 1, "nearer its second");
        assert_eq!(steps_from(&calls, 3)[&7], -2, "a tie goes to the past");
    }

    #[test]
    fn the_past_falls_away_and_the_future_comes_near_but_not_too_near() {
        assert!(depth(0.0).abs() < 1e-6, "the cursor's call is on the plane");
        assert!(
            depth(1.0) > 0.0 && depth(2.0) > depth(1.0),
            "to come is nearer"
        );
        assert!(
            depth(-1.0) < 0.0 && depth(-2.0) < depth(-1.0),
            "past is further"
        );
        assert!(depth(100.0) <= NEAREST, "never most of the way to the eye");
        assert!(depth(-100.0) >= -DEEPEST);
    }

    #[test]
    fn held_keeps_a_call_on_its_line_of_sight_and_fly_moves_it_straight_up() {
        let (eye, flat) = (Vec3::new(2.0, 1.0, 50.0), Vec3::new(10.0, -6.0, 0.0));
        let held = lift(TimeMode::Held, flat, 20.0, eye);
        assert!((held.z - 20.0).abs() < 1e-4, "as far towards the eye");
        let sight = (flat - eye).normalize();
        assert!(
            (held - eye).normalize().dot(sight) > 1.0 - 1e-5,
            "where the map put it on screen"
        );
        let flown = lift(TimeMode::Fly, flat, 20.0, eye);
        assert_eq!(flown, flat + Vec3::Z * 20.0);
    }

    #[test]
    fn names_size_with_their_calls_within_reason() {
        assert!((apparent_size(0.0) - 1.0).abs() < 1e-6);
        assert!(apparent_size(1.0) > 1.0 && apparent_size(-1.0) < 1.0);
        assert!(apparent_size(50.0) <= 1.3 && apparent_size(-50.0) >= 0.8);
    }

    #[test]
    fn the_corridor_runs_on_from_the_cursor_a_few_calls() {
        assert_eq!(corridor(&[3, 5, 5, 7, 3, 9, 11, 13]), vec![3, 5, 7, 3, 9]);
        assert_eq!(corridor(&[3]), vec![3]);
    }

    #[test]
    fn ahead_deep_puts_what_is_to_come_behind_the_map() {
        assert!(depth(Ahead::Deep.nearness(2.0)) < 0.0, "to come is deeper");
        assert!(depth(Ahead::Deep.nearness(-2.0)) > 0.0, "passed is nearer");
        assert!(depth(Ahead::Near.nearness(2.0)) > 0.0);
        assert_eq!(Ahead::Deep.flip(), Ahead::Near);
    }

    #[test]
    fn calls_fade_away_from_the_cursor() {
        assert!((strength(0.0) - 1.0).abs() < 1e-6);
        assert!(strength(1.0) < 1.0 && strength(-1.0) < 1.0);
        assert!(
            strength(2.0) < strength(-2.0),
            "what is to come fades faster"
        );
        assert!(strength(3.0) > strength(4.0));
        assert!((strength(40.0) - TRACE).abs() < 1e-6, "a trace, never gone");
    }
}
