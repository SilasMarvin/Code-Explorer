//! The graph camera: a perspective view orbiting a point, driven as Blender's viewport
//! is. On a trackpad, two fingers orbit, shift and two fingers pan, and a pinch or
//! ctrl and two fingers zoom; with a mouse, the wheel zooms, left- or middle-drag
//! orbits and right- or shift-drag pans. Zoom goes towards the pointer, so you can
//! zoom into any part of the graph. Panning or zooming away lets go of the focused
//! node until you click one or press F. Moves to a new point, distance or angle ease
//! in.

use bevy::{
    camera::{CameraOutputMode, visibility::RenderLayers},
    input::{
        gestures::PinchGesture,
        mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    },
    post_process::bloom::Bloom,
    prelude::*,
    render::render_resource::BlendState,
    window::PrimaryWindow,
};
use bevy_egui::{EguiGlobalSettings, PrimaryEguiContext, input::egui_wants_any_pointer_input};

pub(crate) struct OrbitCameraPlugin;

impl Plugin for OrbitCameraPlugin {
    fn build(&self, app: &mut App) {
        // The camera moves last, once everything that aims it has had its say.
        app.add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                camera_input.run_if(not(egui_wants_any_pointer_input)),
            )
            .add_systems(
                PostUpdate,
                (crate::graph_view::hold_center, apply_camera)
                    .chain()
                    .before(TransformSystems::Propagate),
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
    /// Whether the viewer has panned or zoomed away from the focused node, so the
    /// camera stops holding it in the middle.
    pub(crate) free: bool,
    /// How far the lens narrows: the camera stands this many times further back,
    /// through a field as much narrower, so what it frames keeps its size but flattens
    /// towards orthographic.
    pub(crate) lens: f32,
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
            free: false,
            lens: 1.0,
        }
    }
}

impl OrbitCamera {
    /// Eases towards looking at `target` from `distance` away.
    pub(crate) fn glide_to(&mut self, target: Vec3, distance: f32) {
        self.target = target;
        self.goal_distance = Some(distance);
        self.free = false;
    }

    /// Moves the point orbited by `offset` at once, letting go of the focus.
    fn pan(&mut self, offset: Vec3) {
        self.target += offset;
        self.focus += offset;
        self.free = true;
    }

    /// Zooms by `factor` (below 1 is nearer), towards `toward` when given: the whole
    /// view scales about that point, so it stays where it is on screen.
    fn zoom(&mut self, factor: f32, toward: Option<Vec3>) {
        let distance = (self.distance * factor).clamp(2.0, 4_000.0);
        if let Some(point) = toward {
            self.pan((self.focus - point) * (distance / self.distance - 1.0));
        }
        self.distance = distance;
        self.goal_distance = None;
    }

    /// Turns by a drag of `delta` pixels.
    fn orbit(&mut self, delta: Vec2) {
        self.goal_angles = None;
        self.yaw -= delta.x * ORBIT_SPEED;
        self.pitch = (self.pitch - delta.y * ORBIT_SPEED).clamp(-PITCH_LIMIT, PITCH_LIMIT);
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
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut pinches: MessageReader<PinchGesture>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut cameras: Query<(&mut OrbitCamera, &Transform, &Camera, &GlobalTransform)>,
) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    // A trackpad pinch zooms: spread to zoom in.
    let pinch: f32 = pinches.read().map(|p| p.0).sum();
    for (mut camera, transform, view, global) in &mut cameras {
        // What a pixel of drag moves the view by, at the depth of the point orbited.
        let pan_scale = camera.distance * 0.0015;
        let pan =
            |delta: Vec2| (transform.right() * -delta.x + transform.up() * delta.y) * pan_scale;
        // Where the pointer is, at the depth of the point orbited.
        let pointer = window.cursor_position().and_then(|cursor| {
            let corner = view.logical_viewport_rect().map_or(Vec2::ZERO, |r| r.min);
            let ray = view.viewport_to_world(global, cursor - corner).ok()?;
            let ahead = transform.forward();
            let along = ray.direction.dot(*ahead);
            (along.abs() > 1e-4).then(|| {
                ray.origin + *ray.direction * ((camera.focus - ray.origin).dot(*ahead) / along)
            })
        });
        let mut zoom = 1.0 - pinch * 1.5;
        match scroll.unit {
            // A mouse wheel zooms.
            MouseScrollUnit::Line => zoom *= 1.0 - scroll.delta.y * 0.1,
            // Two fingers on a trackpad: zoom with ctrl, pan with shift, else orbit.
            MouseScrollUnit::Pixel if scroll.delta != Vec2::ZERO => {
                if ctrl {
                    zoom *= 1.0 - scroll.delta.y * 0.0025;
                } else if shift {
                    let offset = pan(scroll.delta);
                    camera.pan(offset);
                } else {
                    camera.orbit(scroll.delta);
                }
            }
            MouseScrollUnit::Pixel => {}
        }
        if (zoom - 1.0).abs() > f32::EPSILON {
            camera.zoom(zoom.clamp(0.5, 2.0), pointer);
        }
        if motion.delta == Vec2::ZERO {
            continue;
        }
        let orbiting = buttons.any_pressed([MouseButton::Left, MouseButton::Middle]) && !shift;
        let panning = buttons.pressed(MouseButton::Right)
            || shift && buttons.any_pressed([MouseButton::Left, MouseButton::Middle]);
        if panning {
            let offset = pan(motion.delta);
            camera.pan(offset);
        } else if orbiting {
            camera.orbit(motion.delta);
        }
    }
}

fn apply_camera(
    time: Res<Time>,
    mut cameras: Query<(&mut OrbitCamera, &mut Transform, &mut Projection)>,
) {
    let ease = 1.0 - (-EASING * time.delta_secs()).exp();
    for (mut camera, mut transform, mut projection) in &mut cameras {
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
        let lens = camera.lens.max(1.0);
        transform.translation = camera.focus + rotation * Vec3::Z * camera.distance * lens;
        transform.rotation = rotation;
        if let Projection::Perspective(perspective) = &mut *projection {
            let fov = 2.0 * ((crate::timeline::FIELD / 2.0).tan() / lens).atan();
            if (perspective.fov - fov).abs() > 1e-5 {
                perspective.fov = fov;
            }
            perspective.far = perspective.far.max(camera.distance * lens * 4.0);
        }
    }
}
