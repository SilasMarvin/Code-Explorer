//! The 3D scene: one sphere per function, placed inside its module's sphere, and the
//! systems that run each frame in order, from the pointer and the keys to what is
//! drawn; with the keys that move the cursor and change what is shown.

use bevy::{prelude::*, transform::TransformSystems};
use bevy_egui::{
    EguiPrimaryContextPass,
    input::{egui_wants_any_keyboard_input, egui_wants_any_pointer_input},
};

use crate::{
    camera::{self, OrbitCamera},
    explorer::{Explorer, Step},
    framing::{self, GraphView},
    graph_view::{self, NodeMaterials},
    idle::Awake,
    notice, nucleus, pick,
    ui::{self, Chrome, KeyHelp},
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
                (
                    pick::point,
                    pick::click.run_if(not(egui_wants_any_pointer_input)),
                )
                    .chain(),
                keyboard.run_if(not(egui_wants_any_keyboard_input)),
                pick::double_click.run_if(not(egui_wants_any_pointer_input)),
                nucleus::follow,
                pick::hover,
                place_nodes,
                nucleus::place_bodies,
                framing::frame,
                graph_view::style_nodes,
                graph_view::draw_graph,
                nucleus::draw,
            )
                .chain(),
        )
        // Sized once the camera has moved for the frame, before anything is placed
        // where it is drawn.
        .add_systems(
            PostUpdate,
            graph_view::size_nodes
                .after(camera::apply_camera)
                .before(TransformSystems::Propagate),
        )
        .add_systems(EguiPrimaryContextPass, notice::draw);
    }
}

/// A function in the graph, by index into the codebase map's tiles.
#[derive(Component, Debug)]
pub(crate) struct Tile(pub(crate) usize);

/// A sphere per function, sized by [`graph_view::size_nodes`].
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
    for index in 0..explorer.map().tiles.len() {
        let Some(at) = explorer.tile_point(index) else {
            continue;
        };
        // What the pointer is on is worked out by `pick`, from where nodes show.
        commands.spawn((
            Tile(index),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(at),
        ));
    }
}

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
    ]
    .into_iter()
    .find(|&(key, _)| keys.just_pressed(key))
    .map(|(_, step)| step)
}

/// The graph's keys: Backspace goes back to the view before and Shift+Backspace
/// forward again.
fn graph_navigation(keys: &ButtonInput<KeyCode>, explorer: &mut Explorer, shift: bool) {
    if keys.just_pressed(KeyCode::Backspace) {
        if shift {
            explorer.go_forward();
        } else {
            explorer.go_back();
        }
    }
    // , and . disclose a level less or more of the journey.
    if keys.just_pressed(KeyCode::Comma) {
        explorer.shallower();
    } else if keys.just_pressed(KeyCode::Period) {
        explorer.deeper();
    }
}

fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut explorer: ResMut<Explorer>,
    mut help: ResMut<KeyHelp>,
    mut chrome: ResMut<Chrome>,
    view: Res<GraphView>,
    cameras: Query<(&Camera, &GlobalTransform), With<OrbitCamera>>,
) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if let Some(step) = journey_step(&keys) {
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
    // Enter opens a journey at what the view is on.
    if keys.just_pressed(KeyCode::Enter)
        && let Some(tile) = explorer.current()
    {
        explorer.open_at(tile);
    }
    if keys.just_pressed(KeyCode::KeyH) {
        help.0 = !help.0;
    }
    // X expands the focus's module, with none the one in the middle of the view, or
    // shrinks it back.
    if keys.just_pressed(KeyCode::KeyX) {
        let middle = || {
            let (camera, at) = cameras.single().ok()?;
            let free = ui::areas(view.0?, &explorer).0;
            framing::module_nearest_middle(explorer.nucleus(), camera, at, free)
        };
        let module = if explorer.nucleus().expanded() {
            None
        } else {
            explorer.focus_module().or_else(middle)
        };
        explorer.toggle_expand(module);
    }
    if keys.just_pressed(KeyCode::Slash) {
        chrome.search = true;
    }
}

/// Escape steps out one level, closing whatever is on top: the search box, the key
/// list, the function set aside, the expanded module, the journey, for browsing with
/// its start selected, and then the selection.
fn close_topmost(explorer: &mut Explorer, help: &mut KeyHelp, chrome: &mut Chrome) {
    if chrome.search {
        chrome.search = false;
    } else if help.0 {
        help.0 = false;
    } else if explorer.aside().is_some() {
        explorer.clear_aside();
    } else if explorer.nucleus().expanded() {
        explorer.toggle_expand(None);
    } else if !explorer.browsing() {
        explorer.leave_journey();
    } else {
        explorer.deselect();
    }
}

/// Moves the nodes wherever the nucleus puts them, as it eases round a new start.
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
