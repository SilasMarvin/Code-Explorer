//! Spawns invocation meshes for the current layout and draws edges with gizmos.

use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_keyboard_input;
use codeviz_core::SpanId;
use codeviz_ir::ExitKind;

use crate::{
    camera::OrbitCamera,
    explorer::{Explorer, NodeKind, Playback, SelectionStep},
};

pub(crate) struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, create_assets).add_systems(
            Update,
            (
                rebuild_scene,
                keyboard.run_if(not(egui_wants_any_keyboard_input)),
                advance_playback,
                update_nodes,
                draw_edges,
            )
                .chain(),
        );
    }
}

/// An entity representing one invocation in the current trace.
#[derive(Component, Debug)]
struct Invocation(SpanId);

#[derive(Resource)]
struct Assets3d {
    function: Handle<Mesh>,
    external: Handle<Mesh>,
    /// Indexed like [`NodeKind::ALL`]: `(lit, dimmed)`.
    materials: Vec<(Handle<StandardMaterial>, Handle<StandardMaterial>)>,
}

impl Assets3d {
    fn material(&self, kind: NodeKind, lit: bool) -> Handle<StandardMaterial> {
        let index = NodeKind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
        self.materials
            .get(index)
            .map(|(on, off)| if lit { on.clone() } else { off.clone() })
            .unwrap_or_default()
    }
}

const NODE_RADIUS: f32 = 0.35;
const EXTERNAL_RADIUS: f32 = 0.2;
const DIMMED_ALPHA: f32 = 0.12;

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let materials = NodeKind::ALL
        .iter()
        .map(|kind| {
            let color = kind.color();
            let lit = materials.add(StandardMaterial {
                base_color: color,
                emissive: LinearRgba::from(color) * 0.4,
                ..default()
            });
            let dimmed = materials.add(StandardMaterial {
                base_color: color.with_alpha(DIMMED_ALPHA),
                alpha_mode: AlphaMode::Blend,
                ..default()
            });
            (lit, dimmed)
        })
        .collect();
    commands.insert_resource(Assets3d {
        function: meshes.add(Sphere::new(NODE_RADIUS)),
        external: meshes.add(Cuboid::from_length(EXTERNAL_RADIUS * 2.0)),
        materials,
    });
}

fn rebuild_scene(
    mut commands: Commands,
    mut built: Local<Option<u64>>,
    explorer: Res<Explorer>,
    assets: Res<Assets3d>,
    existing: Query<Entity, With<Invocation>>,
    mut cameras: Query<&mut OrbitCamera>,
    mut playback: ResMut<Playback>,
) {
    if *built == Some(explorer.revision()) {
        return;
    }
    *built = Some(explorer.revision());
    *playback = Playback::default();

    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let scene = explorer.scene();
    for placed in &scene.nodes {
        let Some(node) = explorer.span(placed.span) else {
            continue;
        };
        let kind = NodeKind::of(node);
        let mesh = if matches!(kind, NodeKind::Unresolved | NodeKind::Handoff) {
            assets.external.clone()
        } else {
            assets.function.clone()
        };
        commands
            .spawn((
                Invocation(placed.span),
                Mesh3d(mesh),
                MeshMaterial3d(assets.material(kind, true)),
                Transform::from_translation(Vec3::from(placed.position)),
            ))
            .observe(on_click)
            .observe(on_over)
            .observe(on_out);
    }

    let (min, max) = scene.bounds;
    for mut camera in &mut cameras {
        camera.frame(Vec3::from(min), Vec3::from(max));
    }
}

fn on_click(
    click: On<PointerClick>,
    invocations: Query<&Invocation>,
    mut explorer: ResMut<Explorer>,
    mut playback: ResMut<Playback>,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    if let Ok(invocation) = invocations.get(click.entity) {
        explorer.selected = Some(invocation.0);
        if let Some(node) = explorer.span(invocation.0) {
            playback.tick = playback.tick.max(node.enter as f32);
        }
    }
}

fn on_over(over: On<PointerOver>, invocations: Query<&Invocation>, mut explorer: ResMut<Explorer>) {
    if let Ok(invocation) = invocations.get(over.entity) {
        explorer.hovered = Some(invocation.0);
    }
}

fn on_out(out: On<PointerOut>, invocations: Query<&Invocation>, mut explorer: ResMut<Explorer>) {
    if let Ok(invocation) = invocations.get(out.entity)
        && explorer.hovered == Some(invocation.0)
    {
        explorer.hovered = None;
    }
}

fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut explorer: ResMut<Explorer>,
    mut playback: ResMut<Playback>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let duration = explorer.duration();
    if keys.just_pressed(KeyCode::Space) {
        playback.toggle(duration);
    }
    if keys.just_pressed(KeyCode::Enter) {
        explorer.trace_from_selected();
    }
    if keys.just_pressed(KeyCode::Backspace) {
        explorer.back();
    }

    let step = [
        (KeyCode::ArrowRight, SelectionStep::Next),
        (KeyCode::ArrowLeft, SelectionStep::Previous),
        (KeyCode::ArrowUp, SelectionStep::Parent),
        (KeyCode::ArrowDown, SelectionStep::FirstChild),
    ]
    .into_iter()
    .find_map(|(key, step)| keys.just_pressed(key).then_some(step));
    let moved = step.and_then(|step| explorer.step_selection(step));
    if let Some(node) = moved.and_then(|id| explorer.span(id)) {
        playback.playing = false;
        playback.tick = node.enter as f32;
    }

    if (moved.is_some() || keys.just_pressed(KeyCode::KeyF))
        && let Some(position) = explorer
            .selected
            .and_then(|id| explorer.scene().nodes.get(id.index()))
            .map(|placed| Vec3::from(placed.position))
    {
        for mut camera in &mut cameras {
            camera.target = position;
        }
    }
}

fn advance_playback(time: Res<Time>, explorer: Res<Explorer>, mut playback: ResMut<Playback>) {
    let duration = explorer.duration();
    if playback.playing {
        playback.tick += time.delta_secs() * playback.ticks_per_second;
        if playback.tick >= duration {
            playback.tick = duration;
            playback.playing = false;
        }
    }
}

fn update_nodes(
    time: Res<Time>,
    explorer: Res<Explorer>,
    playback: Res<Playback>,
    assets: Res<Assets3d>,
    mut invocations: Query<(
        &Invocation,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Transform,
    )>,
) {
    let pulse = 1.25 + 0.15 * (time.elapsed_secs() * 6.0).sin();
    for (invocation, mut material, mut transform) in &mut invocations {
        let Some(node) = explorer.span(invocation.0) else {
            continue;
        };
        let wanted = assets.material(NodeKind::of(node), playback.has_entered(node));
        if material.0 != wanted {
            material.0 = wanted;
        }
        let active = playback.is_active(node) && playback.tick < explorer.duration();
        let emphasized =
            explorer.selected == Some(invocation.0) || explorer.hovered == Some(invocation.0);
        let scale = match (active, emphasized) {
            (true, _) => pulse,
            (false, true) => 1.3,
            (false, false) => 1.0,
        };
        transform.scale = Vec3::splat(scale);
    }
}

/// Each caller gets a lifeline along X spanning its invocation; each call drops from
/// the caller's lifeline at the moment of the call to the callee.
fn draw_edges(explorer: Res<Explorer>, playback: Res<Playback>, mut gizmos: Gizmos) {
    let scene = explorer.scene();
    let lit = |entered: bool, color: Color| {
        if entered {
            color
        } else {
            color.with_alpha(DIMMED_ALPHA)
        }
    };

    for placed in &scene.nodes {
        let Some(node) = explorer.span(placed.span) else {
            continue;
        };
        if node.children.is_empty() {
            continue;
        }
        let start = Vec3::from(placed.position);
        let tick_spacing = explorer.layout.tick_spacing;
        let span_end = start + Vec3::X * (placed.length - tick_spacing).max(0.0);
        let color = lit(playback.has_entered(node), NodeKind::of(node).color());
        gizmos.line(start, span_end, color.with_alpha(0.35));
    }

    for edge in &scene.edges {
        let (Some(from_node), Some(to_node), Some(callee)) = (
            scene.nodes.get(edge.caller.index()),
            scene.nodes.get(edge.callee.index()),
            explorer.span(edge.callee),
        ) else {
            continue;
        };
        let to = Vec3::from(to_node.position);
        let from = Vec3::new(to.x, from_node.position[1], from_node.position[2]);
        let mut color = lit(playback.has_entered(callee), NodeKind::of(callee).color());
        if !callee.conditions.is_empty() {
            // Conditional calls are fainter than calls that always happen.
            color = color.with_alpha(color.alpha() * 0.45);
        }
        gizmos.line(from, to, color);
    }

    // Exits sit on the lifeline of the span whose body leaves, at the tick they happen.
    for placed in &scene.nodes {
        let Some(span) = explorer.span(placed.span) else {
            continue;
        };
        for exit in &span.exits {
            let at = Vec3::new(
                exit.tick as f32 * explorer.layout.tick_spacing,
                placed.position[1],
                placed.position[2],
            );
            let color = match exit.kind {
                ExitKind::Return => Color::srgb(0.5, 0.85, 0.55),
                _ => Color::srgb(1.0, 0.3, 0.3),
            };
            let color = lit(exit.tick as f32 <= playback.tick, color);
            let size = NODE_RADIUS * 0.8;
            gizmos.line(
                at + Vec3::new(-size, -size, 0.0),
                at + Vec3::new(size, size, 0.0),
                color,
            );
            gizmos.line(
                at + Vec3::new(-size, size, 0.0),
                at + Vec3::new(size, -size, 0.0),
                color,
            );
        }
    }

    if let Some(selected) = explorer.selected.and_then(|id| scene.nodes.get(id.index())) {
        gizmos.sphere(
            Isometry3d::from_translation(Vec3::from(selected.position)),
            NODE_RADIUS * 2.0,
            Color::WHITE,
        );
    }

    let duration = explorer.duration();
    if playback.tick < duration {
        let (min, max) = scene.bounds;
        let x = playback.tick * explorer.layout.tick_spacing;
        let margin = 1.0;
        let color = Color::srgba(1.0, 1.0, 1.0, 0.5);
        let corners = [
            Vec3::new(x, max[1] + margin, max[2] + margin),
            Vec3::new(x, min[1] - margin, max[2] + margin),
            Vec3::new(x, min[1] - margin, min[2] - margin),
            Vec3::new(x, max[1] + margin, min[2] - margin),
        ];
        gizmos.linestrip(corners.into_iter().chain([corners[0]]), color);
    }

    for lane in &scene.lanes {
        let (min, max) = scene.bounds;
        let y = max[1] + 1.0;
        gizmos.line(
            Vec3::new(min[0] - 1.0, y, lane.z),
            Vec3::new(max[0] + 1.0, y, lane.z),
            Color::srgba(0.6, 0.65, 0.75, 0.12),
        );
    }
}
