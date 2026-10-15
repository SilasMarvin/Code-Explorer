//! Draws the open journeys, stacked: one bar per span spanning its time, colored by
//! lane, with exits and divergence markers on it and drops from caller to callee. With
//! the codebase shown, the map sits above them, lighting up whatever can reach the
//! cursor's unit and tracing each journey's path through it in the journey's accent.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_keyboard_input;
use codeviz_core::{Reach, SpanTarget};
use codeviz_ir::{ExitKind, Mode};

use crate::{
    camera::JourneyCamera,
    explorer::{Autoplay, Confidence, Explorer, SpanRef, Step, Track, is_conditional, is_handoff},
};

pub(crate) struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, create_assets).add_systems(
            Update,
            (
                rebuild_scene,
                keyboard.run_if(not(egui_wants_any_keyboard_input)),
                autoplay,
                track_focus,
                follow_cursor,
                style_bars,
                style_tiles,
                draw_overlays,
            )
                .chain(),
        );
    }
}

/// Anything `rebuild_scene` spawned, so it can be cleared on the next rebuild.
#[derive(Component, Debug, Default)]
struct Spawned;

/// A bar for one span.
#[derive(Component, Debug)]
#[require(Spawned)]
struct Bar(SpanRef);

/// A clickable marker where another outcome splits off.
#[derive(Component, Debug)]
#[require(Spawned)]
struct DivergenceMarker {
    track: usize,
    scenario: usize,
}

/// Each journey's accent: its stripe, and its path across the codebase map. Kept
/// clear of the lane palette's amber (callers) and red (errors).
const ACCENTS: [(f32, f32, f32); 5] = [
    (0.55, 0.85, 1.0),
    (0.98, 0.55, 0.88),
    (0.62, 0.95, 0.5),
    (0.72, 0.62, 1.0),
    (0.4, 0.95, 0.85),
];

pub(crate) fn accent(track: usize) -> Color {
    let (r, g, b) = ACCENTS[track % ACCENTS.len()];
    Color::srgb(r, g, b)
}

/// A function on the codebase map, by index into its tiles.
#[derive(Component, Debug)]
#[require(Spawned)]
struct Tile(usize);

/// How a map tile is drawn right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TileLook {
    /// The cursor's unit.
    Focus,
    /// Calls the cursor's unit directly.
    Caller,
    /// Reaches the cursor's unit through other calls.
    Upstream,
    /// Run by the journey, but cannot reach the cursor's unit; drawn in its lane color.
    Journey(usize),
    Other,
}

const CALLER: Color = Color::srgb(1.0, 0.72, 0.25);

/// Lane colors: distinct, mid-saturation hues that read on a dark background.
pub(crate) const PALETTE: [(f32, f32, f32); 8] = [
    (0.36, 0.62, 0.95),
    (0.95, 0.63, 0.28),
    (0.45, 0.80, 0.52),
    (0.82, 0.48, 0.86),
    (0.95, 0.83, 0.35),
    (0.40, 0.80, 0.82),
    (0.93, 0.45, 0.55),
    (0.62, 0.66, 0.76),
];

pub(crate) fn lane_color(lane: usize) -> Color {
    let (r, g, b) = PALETTE[lane % PALETTE.len()];
    Color::srgb(r, g, b)
}

/// How a bar is drawn right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Look {
    /// At or before the cursor in time.
    Past,
    /// After the cursor: not reached yet.
    Future,
    /// Guessed edges, unresolved calls and handoffs.
    Faint,
}

#[derive(Resource)]
struct Assets3d {
    bar: Handle<Mesh>,
    marker: Handle<Mesh>,
    exit: Handle<Mesh>,
    materials: HashMap<(usize, Look), Handle<StandardMaterial>>,
    error: Handle<StandardMaterial>,
    success: Handle<StandardMaterial>,
    tile: Handle<Mesh>,
    tiles: HashMap<TileLook, Handle<StandardMaterial>>,
}

const BAR_HEIGHT: f32 = 0.8;
const BAR_DEPTH: f32 = 0.8;
/// Space left between consecutive bars on one row.
const BAR_GAP: f32 = 0.25;

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut table = HashMap::new();
    for lane in 0..PALETTE.len() {
        let color = lane_color(lane);
        for (look, alpha, glow) in [
            (Look::Past, 1.0, 0.25),
            (Look::Future, 0.35, 0.0),
            (Look::Faint, 0.18, 0.0),
        ] {
            let material = materials.add(StandardMaterial {
                base_color: color.with_alpha(alpha),
                emissive: LinearRgba::from(color) * glow,
                alpha_mode: if alpha < 1.0 {
                    AlphaMode::Blend
                } else {
                    AlphaMode::Opaque
                },
                perceptual_roughness: 0.8,
                ..default()
            });
            table.insert((lane, look), material);
        }
    }
    let flat = |materials: &mut Assets<StandardMaterial>, color: Color| {
        materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * 0.6,
            unlit: true,
            ..default()
        })
    };
    let mut tiles = HashMap::new();
    let soft = |materials: &mut Assets<StandardMaterial>, color: Color, glow: f32| {
        let alpha = color.alpha();
        materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * glow,
            alpha_mode: if alpha < 1.0 {
                AlphaMode::Blend
            } else {
                AlphaMode::Opaque
            },
            perceptual_roughness: 0.8,
            ..default()
        })
    };
    tiles.insert(
        TileLook::Focus,
        soft(&mut materials, Color::srgb(0.95, 0.96, 1.0), 0.5),
    );
    tiles.insert(TileLook::Caller, soft(&mut materials, CALLER, 0.45));
    tiles.insert(
        TileLook::Upstream,
        soft(&mut materials, Color::srgba(0.85, 0.6, 0.25, 0.75), 0.15),
    );
    tiles.insert(
        TileLook::Other,
        soft(&mut materials, Color::srgba(0.32, 0.35, 0.42, 0.45), 0.0),
    );
    for lane in 0..PALETTE.len() {
        tiles.insert(
            TileLook::Journey(lane),
            soft(&mut materials, lane_color(lane).with_alpha(0.4), 0.0),
        );
    }
    commands.insert_resource(Assets3d {
        tile: meshes.add(Cuboid::new(1.0, 1.0, 0.3)),
        tiles,
        bar: meshes.add(Cuboid::new(1.0, BAR_HEIGHT, BAR_DEPTH)),
        marker: meshes.add(Sphere::new(0.28)),
        exit: meshes.add(Cuboid::new(0.18, BAR_HEIGHT + 0.3, BAR_DEPTH + 0.1)),
        error: flat(&mut materials, Color::srgb(1.0, 0.32, 0.32)),
        success: flat(&mut materials, Color::srgb(0.45, 0.9, 0.55)),
        materials: table,
    });
}

/// Despawns and respawns everything when the journey changes.
fn rebuild_scene(
    mut commands: Commands,
    mut built: Local<Option<u64>>,
    explorer: Res<Explorer>,
    assets: Res<Assets3d>,
    existing: Query<Entity, With<Spawned>>,
    mut cameras: Query<&mut JourneyCamera>,
    mut autoplay: ResMut<Autoplay>,
) {
    if *built == Some(explorer.revision()) {
        return;
    }
    *built = Some(explorer.revision());
    autoplay.playing = false;

    for entity in &existing {
        commands.entity(entity).despawn();
    }
    for (index, track) in explorer.tracks().iter().enumerate() {
        spawn_track(
            &mut commands,
            &assets,
            explorer.layout.tick_spacing,
            index,
            track,
        );
    }

    let (min, mut max) = explorer.bounds();
    max += Vec3::X * explorer.layout.tick_spacing;
    let (mut min, mut max) = (min, max);
    if explorer.codebase() {
        let map = explorer.map();
        let size = Vec3::new(MAP_TILE.x, MAP_TILE.y, 1.0);
        for (index, tile) in map.tiles.iter().enumerate() {
            commands
                .spawn((
                    Tile(index),
                    Mesh3d(assets.tile.clone()),
                    MeshMaterial3d(assets.tiles[&TileLook::Other].clone()),
                    Transform::from_translation(explorer.map_point(tile.position)).with_scale(size),
                ))
                .observe(on_tile_click)
                .observe(on_tile_over)
                .observe(on_tile_out);
        }
        let (map_min, map_max) = map.bounds;
        min = min.min(explorer.map_point(map_min));
        max = max.max(explorer.map_point(map_max));
    }
    for mut camera in &mut cameras {
        camera.frame(min, max);
    }
}

fn spawn_track(commands: &mut Commands, assets: &Assets3d, tick: f32, index: usize, track: &Track) {
    let origin = track.offset();
    for placed in &track.scene().nodes {
        let Some(span) = track.span(placed.span) else {
            continue;
        };
        let length = (placed.length - BAR_GAP).max(tick * 0.4);
        let at = Vec3::from(placed.position) + origin;
        commands
            .spawn((
                Bar(SpanRef {
                    track: index,
                    span: placed.span,
                }),
                Mesh3d(assets.bar.clone()),
                MeshMaterial3d(
                    assets.materials[&(track.color(placed.span) % PALETTE.len(), Look::Past)]
                        .clone(),
                ),
                Transform::from_translation(at + Vec3::X * length / 2.0)
                    .with_scale(Vec3::new(length, 1.0, 1.0)),
            ))
            .observe(on_click)
            .observe(on_over)
            .observe(on_out);

        for exit in &span.exits {
            let material = match exit.kind {
                ExitKind::Return => assets.success.clone(),
                _ => assets.error.clone(),
            };
            commands.spawn((
                ExitMark,
                Mesh3d(assets.exit.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(
                    exit.tick as f32 * tick - BAR_GAP / 2.0 + origin.x,
                    at.y,
                    at.z,
                ),
                Pickable::IGNORE,
            ));
        }
    }

    let divergences = track.divergences();
    for divergence in divergences {
        let Some(placed) = track.scene().nodes.get(divergence.span.index()) else {
            continue;
        };
        let at = Vec3::from(placed.position) + origin;
        let nudge = 0.6
            * divergences
                .iter()
                .take_while(|d| d != &divergence)
                .filter(|d| d.span == divergence.span)
                .count() as f32;
        commands
            .spawn((
                DivergenceMarker {
                    track: index,
                    scenario: divergence.scenario,
                },
                Mesh3d(assets.marker.clone()),
                MeshMaterial3d(assets.error.clone()),
                Transform::from_translation(
                    at + Vec3::new(
                        placed.length - BAR_GAP - 0.35 - nudge,
                        0.0,
                        BAR_DEPTH / 2.0 + 0.1,
                    ),
                ),
            ))
            .observe(on_divergence_click);
    }
}

/// Matches [`codeviz_layout::MapConfig`]'s default tile size.
const MAP_TILE: Vec2 = Vec2::new(4.4, 0.7);

/// Click starts the active journey here; shift-click opens another journey here.
fn on_tile_click(
    click: On<PointerClick>,
    tiles: Query<&Tile>,
    keys: Res<ButtonInput<KeyCode>>,
    mut explorer: ResMut<Explorer>,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    let Some(unit) = tiles
        .get(click.entity)
        .ok()
        .and_then(|tile| explorer.map().tile(tile.0))
        .map(|tile| tile.unit.clone())
    else {
        return;
    };
    if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        explorer.add_track(unit);
    } else {
        explorer.start_from(unit);
    }
}

fn on_tile_over(over: On<PointerOver>, tiles: Query<&Tile>, mut explorer: ResMut<Explorer>) {
    if let Ok(tile) = tiles.get(over.entity) {
        explorer.hovered_tile = Some(tile.0);
    }
}

fn on_tile_out(out: On<PointerOut>, tiles: Query<&Tile>, mut explorer: ResMut<Explorer>) {
    if let Ok(tile) = tiles.get(out.entity)
        && explorer.hovered_tile == Some(tile.0)
    {
        explorer.hovered_tile = None;
    }
}

/// An exit drawn on a bar.
#[derive(Component, Debug)]
#[require(Spawned)]
struct ExitMark;

/// Click puts that journey's cursor on the bar; shift-click opens the bar's unit as
/// another journey.
fn on_click(
    click: On<PointerClick>,
    bars: Query<&Bar>,
    keys: Res<ButtonInput<KeyCode>>,
    mut explorer: ResMut<Explorer>,
) {
    if click.button == PointerButton::Primary
        && let Ok(bar) = bars.get(click.entity)
    {
        explorer.select_in(bar.0);
        if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
            explorer.pull_out_selected();
        }
    }
}

fn on_over(over: On<PointerOver>, bars: Query<&Bar>, mut explorer: ResMut<Explorer>) {
    if let Ok(bar) = bars.get(over.entity) {
        explorer.hovered = Some(bar.0);
    }
}

fn on_out(out: On<PointerOut>, bars: Query<&Bar>, mut explorer: ResMut<Explorer>) {
    if let Ok(bar) = bars.get(out.entity)
        && explorer.hovered == Some(bar.0)
    {
        explorer.hovered = None;
    }
}

fn on_divergence_click(
    click: On<PointerClick>,
    markers: Query<&DivergenceMarker>,
    mut explorer: ResMut<Explorer>,
) {
    if let Ok(marker) = markers.get(click.entity)
        && let Some(span) = explorer
            .tracks()
            .get(marker.track)
            .and_then(Track::selected)
    {
        explorer.select_in(SpanRef {
            track: marker.track,
            span,
        });
        explorer.set_scenario(Some(marker.scenario));
    }
}

fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut explorer: ResMut<Explorer>,
    mut autoplay: ResMut<Autoplay>,
    mut cameras: Query<&mut JourneyCamera>,
) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let step = if keys.just_pressed(KeyCode::ArrowRight) {
        Some(if shift { Step::Over } else { Step::Into })
    } else if keys.just_pressed(KeyCode::ArrowDown) {
        Some(Step::Into)
    } else if keys.just_pressed(KeyCode::ArrowLeft) {
        Some(Step::Back)
    } else if keys.just_pressed(KeyCode::ArrowUp) {
        Some(Step::Out)
    } else if keys.just_pressed(KeyCode::Home) {
        Some(Step::Start)
    } else {
        None
    };
    if let Some(step) = step {
        autoplay.playing = false;
        explorer.step(step);
    }
    if keys.just_pressed(KeyCode::Space) {
        autoplay.playing = !autoplay.playing;
    }
    if keys.just_pressed(KeyCode::Enter) {
        if shift {
            explorer.pull_out_selected();
        } else {
            explorer.trace_from_selected();
        }
    }
    if keys.just_pressed(KeyCode::Tab) {
        let next = (explorer.active() + 1) % explorer.tracks().len().max(1);
        if let Some(span) = explorer.tracks().get(next).and_then(Track::selected) {
            explorer.select_in(SpanRef { track: next, span });
        }
    }
    if keys.just_pressed(KeyCode::KeyX) {
        let active = explorer.active();
        explorer.close_track(active);
    }
    if keys.just_pressed(KeyCode::Backspace) {
        explorer.back();
    }
    if keys.just_pressed(KeyCode::KeyC) {
        explorer.toggle_codebase();
    }
    if keys.just_pressed(KeyCode::Digit3) {
        for mut camera in &mut cameras {
            camera.toggle();
        }
        explorer.rebuild();
    }
}

fn autoplay(time: Res<Time>, mut autoplay: ResMut<Autoplay>, mut explorer: ResMut<Explorer>) {
    if !autoplay.playing {
        autoplay.elapsed = 0.0;
        return;
    }
    autoplay.elapsed += time.delta_secs();
    let interval = 1.0 / autoplay.steps_per_second;
    if autoplay.elapsed >= interval {
        autoplay.elapsed -= interval;
        if explorer.step(Step::Into).is_none() {
            autoplay.playing = false;
        }
    }
}

/// Keeps what reaches the cursor's unit current while the map is shown.
fn track_focus(mut explorer: ResMut<Explorer>) {
    if explorer.codebase() && explorer.bypass_change_detection().refresh_upstream() {
        explorer.set_changed();
    }
}

/// Keeps the cursor in view by easing the camera towards it when it leaves the
/// middle of the screen horizontally.
fn follow_cursor(explorer: Res<Explorer>, mut cameras: Query<&mut JourneyCamera>) {
    if !explorer.is_changed() {
        return;
    }
    let track = explorer.track();
    let Some((start, placed)) = track
        .selected()
        .and_then(|id| Some((track.position(id)?, track.scene().nodes.get(id.index())?)))
    else {
        return;
    };
    for mut camera in &mut cameras {
        if !camera.shows(start, 1.5) {
            let half_width = camera.extent * camera.aspect() / 2.0;
            camera.target.x = start.x + placed.length.min(half_width) / 2.0;
            camera.target.y = start.y;
        }
    }
}

fn style_bars(
    explorer: Res<Explorer>,
    assets: Res<Assets3d>,
    mut bars: Query<(&Bar, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    if !explorer.is_changed() {
        return;
    }
    // Each journey dims what lies after its own cursor.
    let cursors: Vec<u32> = explorer
        .tracks()
        .iter()
        .map(|t| {
            t.selected()
                .and_then(|id| t.span(id))
                .map_or(0, |s| s.enter)
        })
        .collect();
    for (bar, mut material) in &mut bars {
        let Some(track) = explorer.tracks().get(bar.0.track) else {
            continue;
        };
        let (Some(span), Some(placed), Some(&cursor)) = (
            track.span(bar.0.span),
            track.scene().nodes.get(bar.0.span.index()),
            cursors.get(bar.0.track),
        ) else {
            continue;
        };
        let look = if Confidence::of(span) == Confidence::Guessed || is_handoff(span) {
            Look::Faint
        } else if span.enter > cursor {
            Look::Future
        } else {
            Look::Past
        };
        let wanted = &assets.materials[&(track.color(placed.span) % PALETTE.len(), look)];
        if &material.0 != wanted {
            material.0 = wanted.clone();
        }
    }
}

fn style_tiles(
    explorer: Res<Explorer>,
    assets: Res<Assets3d>,
    mut tiles: Query<(&Tile, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    if !explorer.is_changed() || !explorer.codebase() {
        return;
    }
    for (tile, mut material) in &mut tiles {
        let look = tile_look(&explorer, tile.0);
        let wanted = &assets.tiles[&look];
        if &material.0 != wanted {
            material.0 = wanted.clone();
        }
    }
}

fn tile_look(explorer: &Explorer, index: usize) -> TileLook {
    let map = explorer.map();
    if explorer.focus().and_then(|f| map.tile_of(f)) == Some(index) {
        return TileLook::Focus;
    }
    match explorer.tile_distance(index) {
        Some(1) => TileLook::Caller,
        Some(_) => TileLook::Upstream,
        None => explorer
            .journey_lane(index)
            .map_or(TileLook::Other, |lane| {
                TileLook::Journey(lane % PALETTE.len())
            }),
    }
}

/// Group boxes, each journey's path in its accent, every call path into the cursor's
/// unit, and a tether from its tile down to the cursor.
fn draw_map(explorer: &Explorer, gizmos: &mut Gizmos) {
    let map = explorer.map();
    for group in &map.groups {
        let min = explorer.map_point(group.min);
        let max = explorer.map_point(group.max);
        gizmos.rect(
            Isometry3d::from_translation((min + max) / 2.0 - Vec3::Z * 0.2),
            (max - min).truncate(),
            Color::srgba(0.6, 0.65, 0.75, 0.3),
        );
    }
    let center = |index: usize| map.tile(index).map(|t| explorer.map_point(t.position));

    // Each journey as a slice of the graph: its calls between tiles. Journeys bow
    // their arcs by different amounts so shared calls stay distinguishable.
    for (index, track) in explorer.tracks().iter().enumerate() {
        let alpha = if index == explorer.active() {
            0.8
        } else {
            0.45
        };
        let mut drawn = HashSet::new();
        for edge in &track.scene().edges {
            let (Some(from), Some(to)) = (
                track.unit_id(edge.caller).and_then(|u| map.tile_of(u)),
                track.unit_id(edge.callee).and_then(|u| map.tile_of(u)),
            ) else {
                continue;
            };
            if from == to || !drawn.insert((from, to)) {
                continue;
            }
            if let (Some(a), Some(b)) = (center(from), center(to)) {
                let bow = -0.1 - 0.08 * index as f32;
                arc(gizmos, a, b, accent(index).with_alpha(alpha), bow, false);
            }
        }
    }

    let mut drawn = HashSet::new();
    if let Some(up) = explorer.upstream() {
        for edge in &up.edges {
            let (Some(from), Some(to)) = (map.tile_of(&edge.caller), map.tile_of(&edge.callee))
            else {
                continue;
            };
            if from == to || !drawn.insert((from, to)) {
                continue;
            }
            let (Some(a), Some(b)) = (center(from), center(to)) else {
                continue;
            };
            let color = match edge.reach {
                Reach::Guessed => CALLER.with_alpha(0.35),
                Reach::Call | Reach::Encloses => CALLER.with_alpha(0.9),
            };
            arc(gizmos, a, b, color, 0.18, edge.reach == Reach::Guessed);
        }
    }
    let focus_tile = explorer.focus().and_then(|f| map.tile_of(f));
    if let Some(index) = focus_tile
        && let Some(at) = center(index)
    {
        gizmos.rect(
            Isometry3d::from_translation(at + Vec3::Z * 0.2),
            MAP_TILE + Vec2::splat(0.2),
            Color::WHITE,
        );
        let track = explorer.track();
        if let Some(bar) = track.selected().and_then(|id| track.position(id)) {
            dashed(
                gizmos,
                at - Vec3::Y * MAP_TILE.y / 2.0,
                bar + Vec3::new(0.4, BAR_HEIGHT / 2.0, 0.0),
                Color::srgba(1.0, 1.0, 1.0, 0.35),
            );
        }
    }
    if let Some(index) = explorer.hovered_tile
        && Some(index) != focus_tile
        && let Some(at) = center(index)
    {
        gizmos.rect(
            Isometry3d::from_translation(at + Vec3::Z * 0.2),
            MAP_TILE + Vec2::splat(0.15),
            Color::srgba(1.0, 1.0, 1.0, 0.5),
        );
    }
}

/// A call drawn as a curve bowed sideways by `bow` of its length, so calls within one
/// column stay apart, with an arrowhead at the callee.
fn arc(gizmos: &mut Gizmos, from: Vec3, to: Vec3, color: Color, bow: f32, dashed_line: bool) {
    const SEGMENTS: usize = 16;
    let lift = Vec3::Z * 0.25;
    let (from, to) = (from + lift, to + lift);
    let along = to - from;
    let side = Vec3::new(-along.y, along.x, 0.0).normalize_or_zero();
    let bend = (along.length() * bow).clamp(-3.0, 3.0);
    let control = (from + to) / 2.0 + side * bend;
    let point = |t: f32| from.lerp(control, t).lerp(control.lerp(to, t), t);
    for i in 0..SEGMENTS {
        if dashed_line && i % 2 == 1 {
            continue;
        }
        let (a, b) = (
            point(i as f32 / SEGMENTS as f32),
            point((i + 1) as f32 / SEGMENTS as f32),
        );
        gizmos.line(a, b, color);
    }
    let tip = to - (to - point(0.9)).normalize_or_zero() * (MAP_TILE.y / 2.0);
    gizmos.arrow(point(0.85), tip, color).with_tip_length(0.35);
}

/// Per journey: caller → callee drops and an accent stripe down its left edge. Then
/// the cursors, and every other bar that runs the cursor's unit, in any journey.
fn draw_overlays(explorer: Res<Explorer>, mut gizmos: Gizmos) {
    if explorer.codebase() {
        draw_map(&explorer, &mut gizmos);
    }
    let tick = explorer.layout.tick_spacing;
    for (index, track) in explorer.tracks().iter().enumerate() {
        draw_drops(&mut gizmos, track);
        let (min, _) = track.scene().bounds;
        let origin = track.offset();
        let top = origin + Vec3::new(-0.9, BAR_HEIGHT / 2.0, 0.0);
        let bottom = Vec3::new(top.x, origin.y + min[1] - BAR_HEIGHT / 2.0, 0.0);
        let width = if index == explorer.active() { 3 } else { 1 };
        for n in 0..width {
            let shift = Vec3::X * 0.06 * n as f32;
            gizmos.line(top - shift, bottom - shift, accent(index));
        }
    }

    let outline = |gizmos: &mut Gizmos, at: SpanRef, color: Color| {
        let Some(track) = explorer.tracks().get(at.track) else {
            return;
        };
        if let (Some(placed), Some(position)) = (
            track.scene().nodes.get(at.span.index()),
            track.position(at.span),
        ) {
            let length = (placed.length - BAR_GAP).max(tick * 0.4);
            gizmos.cube(
                Transform::from_translation(position + Vec3::X * length / 2.0).with_scale(
                    Vec3::new(length + 0.15, BAR_HEIGHT + 0.15, BAR_DEPTH + 0.15),
                ),
                color,
            );
        }
    };
    // The same function wherever else it runs.
    if let Some(focus) = explorer.focus() {
        for (index, track) in explorer.tracks().iter().enumerate() {
            let Some(journey) = track.journey() else {
                continue;
            };
            for (span, _) in journey
                .spans()
                .filter(|(_, s)| s.target == SpanTarget::Unit(focus.clone()))
            {
                let at = SpanRef { track: index, span };
                if Some(at.span) != explorer.selected() || index != explorer.active() {
                    outline(&mut gizmos, at, Color::srgba(1.0, 1.0, 1.0, 0.6));
                }
            }
        }
    }
    if let Some(hovered) = explorer.hovered {
        outline(&mut gizmos, hovered, Color::srgba(1.0, 1.0, 1.0, 0.45));
    }
    let track = explorer.track();
    if let Some(selected) = track.selected() {
        let at = SpanRef {
            track: explorer.active(),
            span: selected,
        };
        outline(&mut gizmos, at, Color::WHITE);
        if let Some(position) = track.position(selected) {
            let (min, max) = track.scene().bounds;
            let origin = track.offset();
            gizmos.line(
                Vec3::new(position.x, origin.y + max[1] + 1.2, position.z),
                Vec3::new(position.x, origin.y + min[1] - 1.2, position.z),
                Color::srgba(1.0, 1.0, 1.0, 0.25),
            );
        }
    }
}

fn draw_drops(gizmos: &mut Gizmos, track: &Track) {
    let scene = track.scene();
    let origin = track.offset();
    for edge in &scene.edges {
        let (Some(from_bar), Some(to_bar), Some(span)) = (
            scene.nodes.get(edge.caller.index()),
            scene.nodes.get(edge.callee.index()),
            track.span(edge.callee),
        ) else {
            continue;
        };
        let x = to_bar.position[0] + 0.15;
        let from = origin
            + Vec3::new(
                x,
                from_bar.position[1] - BAR_HEIGHT / 2.0,
                from_bar.position[2],
            );
        let to = origin + Vec3::new(x, to_bar.position[1] + BAR_HEIGHT / 2.0, to_bar.position[2]);
        let color = match span.mode {
            Mode::Spawn => Color::srgb(0.55, 0.85, 1.0),
            Mode::Register => Color::srgba(0.55, 0.9, 0.4, 0.6),
            _ => Color::srgba(0.85, 0.87, 0.92, 0.7),
        };
        if is_conditional(span) {
            dashed(gizmos, from, to, color);
        } else {
            gizmos.line(from, to, color);
        }
    }
}

fn dashed(gizmos: &mut Gizmos, from: Vec3, to: Vec3, color: Color) {
    const DASH: f32 = 0.25;
    let length = from.distance(to);
    let direction = (to - from).normalize_or_zero();
    let mut at = 0.0;
    while at < length {
        let end = (at + DASH).min(length);
        gizmos.line(from + direction * at, from + direction * end, color);
        at += DASH * 2.0;
    }
}
