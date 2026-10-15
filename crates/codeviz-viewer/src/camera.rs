//! The graph camera: a perspective view orbiting a point. Left-drag orbits, right-drag
//! pans, scroll or pinch zooms. Moves to a new point, distance or angle ease in.

use bevy::{
    camera::{CameraOutputMode, visibility::RenderLayers},
    input::{
        gestures::PinchGesture,
        mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    },
    post_process::bloom::Bloom,
    prelude::*,
    render::render_resource::BlendState,
};
use bevy_egui::{EguiGlobalSettings, PrimaryEguiContext, input::egui_wants_any_pointer_input};

pub(crate) struct OrbitCameraPlugin;

impl Plugin for OrbitCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_camera).add_systems(
            Update,
            (
                camera_input.run_if(not(egui_wants_any_pointer_input)),
                apply_camera,
            )
                .chain(),
        );
    }
}

/// Facing the graph head-on.
pub(crate) const FRONT: (f32, f32) = (0.0, 0.0);
/// The default three-quarter view.
pub(crate) const ANGLED: (f32, f32) = (-0.35, -0.2);

#[derive(Component, Debug)]
pub(crate) struct OrbitCamera {
    /// Where the camera is easing its focus towards.
    pub(crate) target: Vec3,
    focus: Vec3,
    /// Distance from the focus.
    pub(crate) distance: f32,
    yaw: f32,
    pitch: f32,
    /// A distance being eased towards, until the viewer zooms by hand.
    goal_distance: Option<f32>,
    /// Angles being eased towards, until the viewer orbits by hand.
    goal_angles: Option<(f32, f32)>,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            focus: Vec3::ZERO,
            distance: 60.0,
            yaw: ANGLED.0,
            pitch: ANGLED.1,
            goal_distance: None,
            goal_angles: None,
        }
    }
}

impl OrbitCamera {
    /// Eases towards looking at `target` from `distance` away.
    pub(crate) fn glide_to(&mut self, target: Vec3, distance: f32) {
        self.target = target;
        self.goal_distance = Some(distance);
    }

    /// Eases round to `(yaw, pitch)`.
    pub(crate) fn turn_to(&mut self, (yaw, pitch): (f32, f32)) {
        // The short way round.
        let turns = ((self.yaw - yaw) / std::f32::consts::TAU).round();
        self.yaw -= turns * std::f32::consts::TAU;
        self.goal_angles = Some((yaw, pitch));
    }
}

const ORBIT_SPEED: f32 = 0.005;
const PITCH_LIMIT: f32 = 1.5;
const EASING: f32 = 8.0;

fn spawn_camera(
    mut commands: Commands,
    mut egui_settings: ResMut<EguiGlobalSettings>,
    mut gizmos: ResMut<GizmoConfigStore>,
) {
    gizmos.config_mut::<DefaultGizmoConfigGroup>().0.line.width = 2.0;
    commands.spawn((
        Camera3d::default(),
        OrbitCamera::default(),
        Projection::Perspective(PerspectiveProjection::default()),
        Transform::default(),
        Bloom::NATURAL,
    ));
    // egui gets its own full-window camera so the 3D camera's viewport can shrink to
    // the space between the panels.
    egui_settings.auto_create_primary_context = false;
    commands.spawn((
        PrimaryEguiContext,
        Camera2d,
        RenderLayers::none(),
        Camera {
            order: 1,
            output_mode: CameraOutputMode::Write {
                blend_state: Some(BlendState::ALPHA_BLENDING),
                clear_color: ClearColorConfig::None,
            },
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 5_000.0,
            ..default()
        },
        Transform::from_xyz(2.0, 6.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        brightness: 900.0,
        ..default()
    });
}

fn camera_input(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut pinches: MessageReader<PinchGesture>,
    mut cameras: Query<(&mut OrbitCamera, &Transform)>,
) {
    // A trackpad pinch zooms like the wheel: spread to zoom in.
    let pinch: f32 = pinches.read().map(|p| p.0).sum();
    for (mut camera, transform) in &mut cameras {
        let lines = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        let zoom = (1.0 - lines * 0.1) * (1.0 - pinch * 1.5);
        if (zoom - 1.0).abs() > f32::EPSILON {
            camera.goal_distance = None;
            camera.distance = (camera.distance * zoom.clamp(0.5, 2.0)).clamp(2.0, 4_000.0);
        }
        if buttons.pressed(MouseButton::Left) && motion.delta != Vec2::ZERO {
            camera.goal_angles = None;
            camera.yaw -= motion.delta.x * ORBIT_SPEED;
            camera.pitch =
                (camera.pitch - motion.delta.y * ORBIT_SPEED).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        }
        if buttons.pressed(MouseButton::Right) {
            let scale = camera.distance * 0.0015;
            let offset =
                (transform.right() * -motion.delta.x + transform.up() * motion.delta.y) * scale;
            camera.target += offset;
            camera.focus += offset;
        }
    }
}

fn apply_camera(time: Res<Time>, mut cameras: Query<(&mut OrbitCamera, &mut Transform)>) {
    let ease = 1.0 - (-EASING * time.delta_secs()).exp();
    for (mut camera, mut transform) in &mut cameras {
        camera.focus = camera.focus.lerp(camera.target, ease);
        if let Some(goal) = camera.goal_distance {
            camera.distance += (goal - camera.distance) * ease;
            if (goal - camera.distance).abs() < 0.01 {
                camera.goal_distance = None;
            }
        }
        if let Some((yaw, pitch)) = camera.goal_angles {
            camera.yaw += (yaw - camera.yaw) * ease;
            camera.pitch += (pitch - camera.pitch) * ease;
            if (yaw - camera.yaw).abs() + (pitch - camera.pitch).abs() < 1e-4 {
                camera.goal_angles = None;
            }
        }
        let rotation = Quat::from_euler(EulerRot::YXZ, camera.yaw, camera.pitch, 0.0);
        transform.translation = camera.focus + rotation * Vec3::Z * camera.distance;
        transform.rotation = rotation;
    }
}
