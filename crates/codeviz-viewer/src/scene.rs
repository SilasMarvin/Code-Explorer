//! The 3D scene: one glowing sphere per function, placed by the graph simulation, with
//! the keys that move the cursor and switch what is shown.

use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_keyboard_input;

use crate::{
    explorer::{Autoplay, Clustering, Explorer, SpanRef, Step, Track},
    graph_view::{self, NodeMaterials},
    journey_panel::{JourneyPanel, PanelMode},
    ui::KeyHelp,
};

pub(crate) struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            (graph_view::create_node_materials, spawn_nodes).chain(),
        )
        .add_systems(
            Update,
            (
                stop_on_rebuild,
                keyboard.run_if(not(egui_wants_any_keyboard_input)),
                autoplay,
                graph_view::track_attention,
                simulate,
                graph_view::graph_keys.run_if(not(egui_wants_any_keyboard_input)),
                graph_view::follow_focus,
                graph_view::style_nodes,
                graph_view::draw_graph,
            )
                .chain(),
        );
    }
}

/// Each journey's accent: its chip, and its path through the graph. Saturated, and
/// clear of the amber and blue that mark what calls in and out, so a path stands out
/// from the pale container colors.
pub(crate) const ACCENTS: [(f32, f32, f32); 4] = [
    (0.3, 1.0, 0.62),
    (1.0, 0.42, 0.86),
    (0.68, 0.52, 1.0),
    (0.78, 1.0, 0.32),
];

pub(crate) fn accent(track: usize) -> Color {
    let (r, g, b) = ACCENTS[track % ACCENTS.len()];
    Color::srgb(r, g, b)
}

/// A function in the graph, by index into the codebase map's tiles.
#[derive(Component, Debug)]
pub(crate) struct Tile(pub(crate) usize);

/// Container colors: pale, low-chroma hues that tell containers apart without being
/// mistaken for the saturated amber, blue and accents that carry meaning.
pub(crate) const PALETTE: [(f32, f32, f32); 8] = [
    (0.66, 0.62, 0.84),
    (0.62, 0.78, 0.64),
    (0.84, 0.62, 0.68),
    (0.56, 0.76, 0.78),
    (0.82, 0.77, 0.58),
    (0.64, 0.67, 0.76),
    (0.72, 0.74, 0.52),
    (0.80, 0.67, 0.61),
];

pub(crate) fn lane_color(lane: usize) -> Color {
    let (r, g, b) = PALETTE[lane % PALETTE.len()];
    Color::srgb(r, g, b)
}

/// A sphere per function, sized by how connected it is.
fn spawn_nodes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    explorer: Res<Explorer>,
    materials: Res<NodeMaterials>,
) {
    let mesh = meshes.add(
        Sphere::new(0.5)
            .mesh()
            .ico(3)
            .unwrap_or_else(|_| Sphere::new(0.5).mesh().uv(24, 16)),
    );
    let material = materials.0.values().next().cloned().unwrap_or_default();
    let graph = explorer.graph();
    for index in 0..explorer.map().tiles.len() {
        let Some(at) = explorer.tile_point(index) else {
            continue;
        };
        commands
            .spawn((
                Tile(index),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(at)
                    .with_scale(Vec3::splat(graph_view::node_size(graph, index))),
            ))
            .observe(on_node_click)
            .observe(on_node_over)
            .observe(on_node_out);
    }
}

/// Click focuses a node; shift-click opens a journey from it.
fn on_node_click(
    click: On<PointerClick>,
    tiles: Query<&Tile>,
    keys: Res<ButtonInput<KeyCode>>,
    mut explorer: ResMut<Explorer>,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    let Ok(tile) = tiles.get(click.entity) else {
        return;
    };
    if !explorer.shown(tile.0) {
        return;
    }
    if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        if let Some(unit) = explorer.tile_unit(tile.0).cloned() {
            explorer.add_track(unit);
        }
    } else {
        explorer.focus_node(tile.0);
    }
}

fn on_node_over(over: On<PointerOver>, tiles: Query<&Tile>, mut explorer: ResMut<Explorer>) {
    if let Ok(tile) = tiles.get(over.entity)
        && explorer.shown(tile.0)
    {
        explorer.hovered_tile = Some(tile.0);
    }
}

fn on_node_out(out: On<PointerOut>, tiles: Query<&Tile>, mut explorer: ResMut<Explorer>) {
    if let Ok(tile) = tiles.get(out.entity)
        && explorer.hovered_tile == Some(tile.0)
    {
        explorer.hovered_tile = None;
    }
}

/// Stops autoplay when a journey is rebuilt under it.
fn stop_on_rebuild(
    explorer: Res<Explorer>,
    mut built: Local<Option<u64>>,
    mut autoplay: ResMut<Autoplay>,
) {
    if *built != Some(explorer.revision()) {
        *built = Some(explorer.revision());
        autoplay.playing = false;
    }
}

/// Digit keys, for picking the nth call or caller.
const DIGITS: [KeyCode; 9] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

/// The arrows walk the journey's call tree: left and right between the calls one
/// caller makes, down into a call, up to its caller. The brackets step through time
/// one call at a time, the way the journey actually runs.
fn journey_step(keys: &ButtonInput<KeyCode>) -> Option<Step> {
    [
        (KeyCode::ArrowRight, Step::Next),
        (KeyCode::ArrowLeft, Step::Previous),
        (KeyCode::ArrowDown, Step::Child),
        (KeyCode::ArrowUp, Step::Parent),
        (KeyCode::BracketRight, Step::Into),
        (KeyCode::BracketLeft, Step::Back),
        (KeyCode::Home, Step::Start),
        (KeyCode::End, Step::End),
    ]
    .into_iter()
    .find(|&(key, _)| keys.just_pressed(key))
    .map(|(_, step)| step)
}

/// The graph's keys: digits walk to the focus's nth call (with shift, its nth caller),
/// Backspace goes back to the node focused before, Escape hands the focus back to the
/// journey's cursor, Q quiets everything far from the focus.
fn graph_navigation(keys: &ButtonInput<KeyCode>, explorer: &mut Explorer, shift: bool) {
    if let Some(index) = DIGITS.iter().position(|&key| keys.just_pressed(key))
        && let Some(focus) = explorer.graph_focus()
    {
        let target = if shift {
            explorer.callers_of(focus).get(index).copied()
        } else {
            explorer.graph().sequence(focus).get(index).copied()
        };
        if let Some(tile) = target {
            explorer.focus_node(tile);
        }
    }
    if keys.just_pressed(KeyCode::Backspace) && !explorer.focus_back() {
        explorer.back();
    }
    if keys.just_pressed(KeyCode::Escape) {
        explorer.unfocus();
    }
    if keys.just_pressed(KeyCode::KeyQ) {
        let next = explorer.quiet().next();
        explorer.set_quiet(next);
    }
    if keys.just_pressed(KeyCode::KeyK) {
        let next = match explorer.clustering() {
            Clustering::Modules => Clustering::Calls,
            Clustering::Calls => Clustering::Modules,
        };
        explorer.set_clustering(next);
    }
}

fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut explorer: ResMut<Explorer>,
    mut autoplay: ResMut<Autoplay>,
    mut panel: ResMut<JourneyPanel>,
    mut help: ResMut<KeyHelp>,
) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if let Some(step) = journey_step(&keys) {
        autoplay.playing = false;
        explorer.step(step);
    }
    graph_navigation(&keys, &mut explorer, shift);
    if keys.just_pressed(KeyCode::Space) {
        autoplay.playing = !autoplay.playing;
    }
    if keys.just_pressed(KeyCode::Enter) {
        if shift {
            explorer.pull_out_selected();
        } else {
            explorer.trace_from_focus();
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
    if keys.just_pressed(KeyCode::KeyJ) {
        if panel.mode == PanelMode::Hidden {
            explorer.journey_for_focus();
            panel.mode = PanelMode::Docked;
        } else {
            panel.mode = PanelMode::Hidden;
        }
    }
    if keys.just_pressed(KeyCode::KeyM) {
        if panel.mode != PanelMode::Full {
            explorer.journey_for_focus();
        }
        panel.mode = match panel.mode {
            PanelMode::Full => PanelMode::Docked,
            PanelMode::Hidden | PanelMode::Docked => PanelMode::Full,
        };
    }
    if keys.just_pressed(KeyCode::Slash) || keys.just_pressed(KeyCode::KeyH) {
        help.0 = !help.0;
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

/// Lets the graph settle, moving its nodes as it does.
fn simulate(mut explorer: ResMut<Explorer>, mut nodes: Query<(&Tile, &mut Transform)>) {
    if !explorer.bypass_change_detection().simulate(2) {
        return;
    }
    for (tile, mut transform) in &mut nodes {
        if let Some(at) = explorer.tile_point(tile.0) {
            transform.translation = at;
        }
    }
}
