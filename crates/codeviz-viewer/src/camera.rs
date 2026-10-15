//! A minimal orbit camera: left-drag orbits, right-drag pans, scroll zooms.

use bevy::{
    camera::{CameraOutputMode, visibility::RenderLayers},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
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
                orbit_input.run_if(not(egui_wants_any_pointer_input)),
                apply_orbit,
            )
                .chain(),
        );
    }
}

#[derive(Component, Debug)]
pub(crate) struct OrbitCamera {
    /// Where the camera is easing its focus towards.
    pub(crate) target: Vec3,
    focus: Vec3,
    pub(crate) radius: f32,
    yaw: f32,
    pitch: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            focus: Vec3::ZERO,
            radius: 30.0,
            yaw: -0.3,
            pitch: -0.4,
        }
    }
}

impl OrbitCamera {
    /// Frames an axis-aligned box by fitting its bounding sphere in the narrower of the
    /// two fields of view, assuming a viewport no narrower than 3:4.
    pub(crate) fn frame(&mut self, min: Vec3, max: Vec3) {
        const NARROW_HALF_FOV: f32 = 0.30;
        self.target = (min + max) / 2.0;
        let sphere = (max - min).length() / 2.0;
        self.radius = (sphere / NARROW_HALF_FOV.sin()).max(12.0);
    }
}

const ORBIT_SPEED: f32 = 0.005;
const PAN_SPEED: f32 = 0.0015;
const PITCH_LIMIT: f32 = 1.5;
const FOCUS_EASING: f32 = 8.0;

fn spawn_camera(mut commands: Commands, mut egui_settings: ResMut<EguiGlobalSettings>) {
    commands.spawn((
        Camera3d::default(),
        OrbitCamera::default(),
        Transform::default(),
    ));
    // egui gets its own full-window camera so the 3D camera's viewport can shrink to the
    // space between the panels.
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
            illuminance: 6_000.0,
            ..default()
        },
        Transform::from_xyz(4.0, 10.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        brightness: 400.0,
        ..default()
    });
}

fn orbit_input(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cameras: Query<(&mut OrbitCamera, &Transform)>,
) {
    for (mut orbit, transform) in &mut cameras {
        if buttons.pressed(MouseButton::Left) {
            orbit.yaw -= motion.delta.x * ORBIT_SPEED;
            orbit.pitch =
                (orbit.pitch - motion.delta.y * ORBIT_SPEED).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        }
        if buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle) {
            let scale = orbit.radius * PAN_SPEED;
            let offset =
                (transform.right() * -motion.delta.x + transform.up() * motion.delta.y) * scale;
            orbit.target += offset;
            orbit.focus += offset;
        }
        let lines = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        if lines != 0.0 {
            orbit.radius = (orbit.radius * (1.0 - lines * 0.1)).clamp(2.0, 2_000.0);
        }
    }
}

fn apply_orbit(time: Res<Time>, mut cameras: Query<(&mut OrbitCamera, &mut Transform)>) {
    let ease = 1.0 - (-FOCUS_EASING * time.delta_secs()).exp();
    for (mut orbit, mut transform) in &mut cameras {
        orbit.focus = orbit.focus.lerp(orbit.target, ease);
        let rotation = Quat::from_euler(EulerRot::YXZ, orbit.yaw, orbit.pitch, 0.0);
        transform.translation = orbit.focus + rotation * Vec3::Z * orbit.radius;
        transform.rotation = rotation;
    }
}
