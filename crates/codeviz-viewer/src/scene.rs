//! The 3D scene: one glowing sphere per function, placed inside its module's sphere,
//! with the keys that move the cursor and switch what is shown.

use bevy::prelude::*;
use bevy_egui::{
    EguiPrimaryContextPass,
    input::{egui_wants_any_keyboard_input, egui_wants_any_pointer_input},
};

use crate::{
    camera,
    disclosure::Disclose,
    explorer::{Autoplay, Explorer, Step},
    graph_view::{self, NodeMaterials},
    idle::Awake,
    notice, nucleus, pick,
    ui::{Chrome, KeyHelp},
};

pub(crate) struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            (
                graph_view::create_node_materials,
                nucleus::create_looks,
                spawn_nodes,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                stop_on_rebuild,
                (
                    pick::point,
                    pick::click.run_if(not(egui_wants_any_pointer_input)),
                )
                    .chain(),
                keyboard.run_if(not(egui_wants_any_keyboard_input)),
                autoplay,
                graph_view::track_attention,
                nucleus::double_click.run_if(not(egui_wants_any_pointer_input)),
                nucleus::follow,
                nucleus::hover,
                place_nodes,
                nucleus::place_bodies,
                graph_view::graph_keys.run_if(not(egui_wants_any_keyboard_input)),
                nucleus::frame,
                camera::restore_pose,
                graph_view::style_nodes,
                graph_view::draw_graph,
                nucleus::draw,
            )
                .chain(),
        )
        .add_systems(EguiPrimaryContextPass, notice::draw);
    }
}

/// A function in the graph, by index into the codebase map's tiles.
#[derive(Component, Debug)]
pub(crate) struct Tile(pub(crate) usize);

/// Module colors: a node's fill, its module's ring and its journey bar, distinct
/// enough that a bar finds its node by color. None is near the violet and blue that
/// links use to say which way a call runs, nor the orange and red that mark the
/// journey and removals, so a module's color never reads as one of those.
pub(crate) const PALETTE: [(f32, f32, f32); 8] = [
    (0.36, 0.80, 0.72),
    (0.93, 0.56, 0.72),
    (0.76, 0.80, 0.36),
    (0.66, 0.78, 0.60),
    (0.70, 0.70, 0.60),
    (0.90, 0.84, 0.50),
    (0.48, 0.82, 0.48),
    (0.96, 0.74, 0.86),
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
        // What the pointer is on is worked out by `pick`, from where nodes show.
        commands.spawn((
            Tile(index),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(at)
                .with_scale(Vec3::splat(graph_view::node_size(graph, index))),
        ));
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

/// Digit keys, for opening the call with that number.
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
fn journey_step(keys: &ButtonInput<KeyCode>, shift: bool) -> Option<Step> {
    // With shift, ] steps over a call and ↑ steps out of it, as a debugger does.
    if shift && keys.just_pressed(KeyCode::BracketRight) {
        return Some(Step::Over);
    }
    if shift && keys.just_pressed(KeyCode::ArrowUp) {
        return Some(Step::Out);
    }
    [
        (KeyCode::ArrowRight, Step::Next),
        (KeyCode::ArrowLeft, Step::Previous),
        (KeyCode::ArrowDown, Step::Child),
        (KeyCode::ArrowUp, Step::Parent),
        (KeyCode::BracketRight, Step::Into),
        (KeyCode::BracketLeft, Step::Back),
    ]
    .into_iter()
    .find(|&(key, _)| keys.just_pressed(key))
    .map(|(_, step)| step)
}

/// The graph's keys: digits open the call numbered so on screen, Backspace goes back
/// to the view before and Shift+Backspace forward again.
fn graph_navigation(keys: &ButtonInput<KeyCode>, explorer: &mut Explorer, shift: bool) {
    if let Some(index) = DIGITS.iter().position(|&key| keys.just_pressed(key)) {
        explorer.open_numbered(index + 1);
    }
    if keys.just_pressed(KeyCode::Backspace) {
        if shift {
            explorer.go_forward();
        } else {
            explorer.go_back();
        }
    }
    // , and . disclose a level less or more of the journey; V switches between
    // disclosing by depth and by time.
    let disclose = explorer.disclose();
    let disclose = if keys.just_pressed(KeyCode::Comma) {
        match disclose {
            Disclose::Depth(depth) => Disclose::Depth(depth.saturating_sub(1).max(1)),
            Disclose::Time => Disclose::Time,
        }
    } else if keys.just_pressed(KeyCode::Period) {
        match disclose {
            Disclose::Depth(depth) => Disclose::Depth(depth + 1),
            Disclose::Time => Disclose::Time,
        }
    } else if keys.just_pressed(KeyCode::KeyV) {
        match disclose {
            Disclose::Depth(_) => Disclose::Time,
            Disclose::Time => Disclose::default(),
        }
    } else {
        disclose
    };
    if disclose != explorer.disclose() {
        explorer.set_disclose(disclose);
    }
}

fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut explorer: ResMut<Explorer>,
    mut autoplay: ResMut<Autoplay>,
    mut help: ResMut<KeyHelp>,
    mut chrome: ResMut<Chrome>,
) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if let Some(step) = journey_step(&keys, shift) {
        autoplay.playing = false;
        explorer.step(step);
    }
    graph_navigation(&keys, &mut explorer, shift);
    if keys.just_pressed(KeyCode::Escape) {
        close_topmost(&mut explorer, &mut help, &mut chrome);
    }
    // In a journey, I brings in every function off it that calls one on it, or leaves
    // them out again.
    if keys.just_pressed(KeyCode::KeyI) {
        explorer.toggle_outside();
    }
    if keys.just_pressed(KeyCode::Space) {
        autoplay.playing = !autoplay.playing;
    }
    if keys.just_pressed(KeyCode::Enter) {
        explorer.trace_from_focus();
    }
    if keys.just_pressed(KeyCode::KeyH) {
        help.0 = !help.0;
    }
    nucleus::toggle(&keys, &mut explorer);
    if keys.just_pressed(KeyCode::Slash) {
        chrome.search = true;
    }
}

/// Escape closes whatever is on top: the search box, the key list, a
/// function selected off the journey, then the journey itself, for browsing with the
/// cursor's function selected.
fn close_topmost(explorer: &mut Explorer, help: &mut KeyHelp, chrome: &mut Chrome) {
    if chrome.search {
        chrome.search = false;
    } else if help.0 {
        help.0 = false;
    } else if explorer.aside().is_some() {
        explorer.clear_aside();
    } else if !explorer.browsing() {
        explorer.leave_journey();
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

/// Moves the nodes wherever the nucleus puts them, as it eases round a new middle.
fn place_nodes(
    explorer: Res<Explorer>,
    mut placed: Local<Option<u64>>,
    mut nodes: Query<(&Tile, &mut Transform)>,
    mut awake: ResMut<Awake>,
) {
    if *placed == Some(explorer.moved()) {
        return;
    }
    *placed = Some(explorer.moved());
    awake.0 = true;
    for (tile, mut transform) in &mut nodes {
        if let Some(at) = explorer.tile_point(tile.0) {
            transform.translation = at;
        }
    }
}
