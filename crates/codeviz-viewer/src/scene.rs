//! The 3D scene: one glowing sphere per function, placed by the graph simulation, with
//! the keys that move the cursor and switch what is shown.

use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_keyboard_input;

use crate::{
    disclosure::Disclose,
    explorer::{Autoplay, Clustering, Explorer, Side, Step},
    graph_view::{self, NodeMaterials},
    journey_panel::{JourneyPanel, PanelMode},
    lens::Lens,
    ui::{Chrome, KeyHelp},
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
                reveal,
                graph_view::graph_keys.run_if(not(egui_wants_any_keyboard_input)),
                graph_view::follow_focus,
                graph_view::frame_reveal,
                graph_view::frame_spotlight,
                graph_view::style_nodes,
                graph_view::draw_graph,
            )
                .chain(),
        );
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

/// Click moves the focus to a node: the journey's cursor when it runs the node, else
/// the node alone, for now. Shift-click follows it, starting the journey there.
fn on_node_click(
    click: On<PointerClick>,
    tiles: Query<&Tile>,
    keys: Res<ButtonInput<KeyCode>>,
    mut explorer: ResMut<Explorer>,
    mut panel: ResMut<JourneyPanel>,
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
        explorer.open_at(tile.0);
    } else {
        explorer.focus_tile(tile.0);
    }
    // Clicking a node is asking what it does: the journey opens with it.
    if panel.mode == PanelMode::Hidden {
        panel.mode = PanelMode::Docked;
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
/// to the view before and Shift+Backspace forward again. - and = narrow and widen
/// what shows, L picks what it shows around and G how it measures nearness. Returns
/// whether one of those four was pressed, for the depth meter.
fn graph_navigation(keys: &ButtonInput<KeyCode>, explorer: &mut Explorer, shift: bool) -> bool {
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
    let lens = explorer.lens();
    let dialed =
        keys.any_just_pressed([KeyCode::Minus, KeyCode::Equal, KeyCode::KeyL, KeyCode::KeyG]);
    let lens = if keys.just_pressed(KeyCode::Minus) {
        lens.narrower(explorer.lens_top())
    } else if keys.just_pressed(KeyCode::Equal) {
        lens.wider(explorer.lens_top())
    } else if keys.just_pressed(KeyCode::KeyL) {
        Lens {
            source: lens.source.next(),
            ..lens
        }
    } else if keys.just_pressed(KeyCode::KeyG) {
        Lens {
            measure: lens.measure.next(),
            ..lens
        }
    } else {
        lens
    };
    if lens != explorer.lens() {
        explorer.set_lens(lens);
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
    if keys.just_pressed(KeyCode::KeyK) {
        let next = match explorer.clustering() {
            Clustering::Modules => Clustering::Calls,
            Clustering::Calls => Clustering::Modules,
        };
        explorer.set_clustering(next);
    }
    dialed
}

fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut explorer: ResMut<Explorer>,
    mut autoplay: ResMut<Autoplay>,
    mut panel: ResMut<JourneyPanel>,
    mut help: ResMut<KeyHelp>,
    mut chrome: ResMut<Chrome>,
) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if let Some(step) = journey_step(&keys, shift) {
        autoplay.playing = false;
        explorer.step(step);
    }
    if graph_navigation(&keys, &mut explorer, shift) {
        chrome.dial_turned = Some(time.elapsed_secs());
    }
    if keys.just_pressed(KeyCode::Escape) {
        close_topmost(&mut explorer, &mut help, &mut chrome);
    }
    // I and O pick out what calls into the focus and what it calls out to.
    for (key, side) in [
        (KeyCode::KeyI, Side::Callers),
        (KeyCode::KeyO, Side::Callees),
    ] {
        if keys.just_pressed(key) {
            explorer.toggle_spotlight(side, time.elapsed_secs());
        }
    }
    if keys.just_pressed(KeyCode::Space) {
        autoplay.playing = !autoplay.playing;
    }
    if keys.just_pressed(KeyCode::Enter) {
        explorer.trace_from_focus();
    }
    if keys.just_pressed(KeyCode::KeyJ) {
        panel.mode = if panel.mode == PanelMode::Hidden {
            PanelMode::Docked
        } else {
            PanelMode::Hidden
        };
    }
    if keys.just_pressed(KeyCode::KeyZ) {
        crate::ui::toggle_beams(&mut panel);
    }
    if keys.just_pressed(KeyCode::KeyM) {
        panel.mode = match panel.mode {
            PanelMode::Full => PanelMode::Docked,
            PanelMode::Hidden | PanelMode::Docked => PanelMode::Full,
        };
    }
    if keys.just_pressed(KeyCode::KeyH) {
        help.0 = !help.0;
    }
    if keys.just_pressed(KeyCode::KeyS) {
        chrome.side = !chrome.side;
    }
    if keys.just_pressed(KeyCode::Slash) {
        chrome.search = true;
    }
}

/// Escape closes whatever is on top: the search box, the key list, the spotlight, a
/// focus off the journey, then the side panel. It never moves the view.
fn close_topmost(explorer: &mut Explorer, help: &mut KeyHelp, chrome: &mut Chrome) {
    if chrome.search {
        chrome.search = false;
    } else if help.0 {
        help.0 = false;
    } else if explorer.spotlight().is_some() {
        explorer.clear_spotlight();
    } else if explorer.aside().is_some() {
        explorer.clear_aside();
    } else if chrome.side {
        chrome.side = false;
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

/// Fades nodes in and out as the lens moves.
fn reveal(time: Res<Time>, mut explorer: ResMut<Explorer>) {
    if explorer
        .bypass_change_detection()
        .ease_visibility(time.delta_secs())
    {
        explorer.set_changed();
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
