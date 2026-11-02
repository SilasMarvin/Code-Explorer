//! The graph camera: a perspective view orbiting a point, driven as Blender's viewport
//! is. On a trackpad, two fingers orbit, shift and two fingers pan, and a pinch or
//! ctrl and two fingers zoom; with a mouse, the wheel zooms, left- or middle-drag
//! orbits and right- or shift-drag pans. Zoom goes towards the pointer, so you can
//! zoom into any part of the graph. Moves to a new point, distance or angle ease in.

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

use crate::{explorer::Explorer, nucleus};

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
                (apply_camera, remember_pose)
                    .chain()
                    .before(TransformSystems::Propagate),
            );
    }
}

#[derive(Component, Debug)]
pub(crate) struct OrbitCamera {
    /// Where the camera is easing towards looking at.
    pub(crate) target: Vec3,
    /// Where it looks now.
    looking_at: Vec3,
    /// Distance from where it looks.
    pub(crate) distance: f32,
    yaw: f32,
    pitch: f32,
    /// A distance being eased towards, until the viewer zooms by hand.
    goal_distance: Option<f32>,
    /// Angles being eased towards, until the viewer orbits by hand.
    goal_angles: Option<(f32, f32)>,
    /// The angles it may turn to, when held near one: browsing, the flattened map
    /// never turns edge-on.
    bounds: Option<Bounds>,
}

/// Angles held near `middle`: yaw and pitch each at most `reach` off it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bounds {
    pub(crate) middle: (f32, f32),
    pub(crate) reach: (f32, f32),
}

impl Bounds {
    /// `angles` turned no further than the bounds allow, counting whole turns round
    /// as the yaw `from` does, so a long drag stops at the edge it went past.
    fn hold(self, (yaw, pitch): (f32, f32), from: f32) -> (f32, f32) {
        let tau = std::f32::consts::TAU;
        let turns = ((from - self.middle.0) / tau).round() * tau;
        let off = yaw - self.middle.0;
        (
            self.middle.0 + turns + (off - turns).clamp(-self.reach.0, self.reach.0),
            pitch.clamp(self.middle.1 - self.reach.1, self.middle.1 + self.reach.1),
        )
    }
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            looking_at: Vec3::ZERO,
            distance: 60.0,
            yaw: nucleus::VIEW.0,
            pitch: nucleus::VIEW.1,
            goal_distance: None,
            goal_angles: None,
            bounds: None,
        }
    }
}

impl OrbitCamera {
    /// Whether the camera is still easing towards where it was sent.
    pub(crate) fn easing(&self) -> bool {
        self.goal_distance.is_some()
            || self.goal_angles.is_some()
            || self.looking_at.distance(self.target) > self.distance * 1e-4
    }

    /// Eases towards looking at `target` from `distance` away.
    pub(crate) fn glide_to(&mut self, target: Vec3, distance: f32) {
        self.target = target;
        self.goal_distance = Some(distance);
    }

    /// Eases back to `distance`, around wherever it is looking: a panned or zoomed
    /// view stays where it was put.
    pub(crate) fn pull_back(&mut self, distance: f32) {
        let goal = self.goal_distance.unwrap_or(self.distance);
        self.goal_distance = Some(goal.max(distance));
    }

    /// Moves the point orbited by `offset` at once.
    fn pan(&mut self, offset: Vec3) {
        self.target += offset;
        self.looking_at += offset;
    }

    /// Zooms by `factor` (below 1 is nearer), towards `toward` when given: the whole
    /// view scales about that point, so it stays where it is on screen.
    fn zoom(&mut self, factor: f32, toward: Option<Vec3>) {
        let distance = (self.distance * factor).clamp(2.0, 4_000.0);
        if let Some(point) = toward {
            self.pan((self.looking_at - point) * (distance / self.distance - 1.0));
        }
        self.distance = distance;
        self.goal_distance = None;
    }

    /// Turns by a drag of `delta` pixels, no further than the bounds allow.
    fn orbit(&mut self, delta: Vec2) {
        self.goal_angles = None;
        let yaw = self.yaw - delta.x * ORBIT_SPEED;
        let pitch = (self.pitch - delta.y * ORBIT_SPEED).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        (self.yaw, self.pitch) = self
            .bounds
            .map_or((yaw, pitch), |bounds| bounds.hold((yaw, pitch), self.yaw));
    }

    /// `angles` within the bounds, when there are any.
    fn held(&self, angles: (f32, f32)) -> (f32, f32) {
        self.bounds
            .map_or(angles, |bounds| bounds.hold(angles, angles.0))
    }

    /// Holds the angles near one from now on, or lets them go; when it is turned past
    /// new bounds, eases back inside them.
    pub(crate) fn bound(&mut self, bounds: Option<Bounds>) {
        if self.bounds == bounds {
            return;
        }
        self.bounds = bounds;
        let angles = self.goal_angles.unwrap_or((self.yaw, self.pitch));
        let held = self.held(angles);
        if held != angles {
            self.turn_to(held);
        }
    }

    /// Which way the camera faces once it has turned to where it is turning.
    pub(crate) fn aim_rotation(&self) -> Quat {
        let (yaw, pitch) = self.goal_angles.unwrap_or((self.yaw, self.pitch));
        Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0)
    }

    /// Eases round to `(yaw, pitch)`, or as near it as the bounds allow.
    pub(crate) fn turn_to(&mut self, angles: (f32, f32)) {
        let (yaw, pitch) = self.held(angles);
        // The short way round.
        let turns = ((self.yaw - yaw) / std::f32::consts::TAU).round();
        self.yaw -= turns * std::f32::consts::TAU;
        self.goal_angles = Some((yaw, pitch));
    }

    /// Eases across to look at `target` from as far away as it is going to be.
    pub(crate) fn pan_to(&mut self, target: Vec3) {
        self.target = target;
    }

    /// The distance it is easing to, or is at.
    pub(crate) fn goal_distance(&self) -> f32 {
        self.goal_distance.unwrap_or(self.distance)
    }

    /// Where the camera is going to be once it stops easing.
    pub(crate) fn pose(&self) -> Pose {
        Pose {
            target: self.target,
            distance: self.goal_distance(),
            angles: self.goal_angles.unwrap_or((self.yaw, self.pitch)),
        }
    }

    /// Eases back to a pose it had.
    pub(crate) fn return_to(&mut self, pose: Pose) {
        self.target = pose.target;
        self.goal_distance = Some(pose.distance);
        self.turn_to(pose.angles);
    }
}

/// Where the camera stands: what it looks at, from how far and at what angle. Kept
/// with each place in the history, so going back looks at it as it was seen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pose {
    target: Vec3,
    distance: f32,
    angles: (f32, f32),
}

impl Pose {
    /// What it looks at, from how far, and its yaw and pitch.
    pub(crate) fn parts(&self) -> (Vec3, f32, (f32, f32)) {
        (self.target, self.distance, self.angles)
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
        // The window's cursor as it is: the conversion allows for where the viewport
        // sits in the window itself.
        let pointer = window.cursor_position().and_then(|cursor| {
            let ray = view.viewport_to_world(global, cursor).ok()?;
            let ahead = transform.forward();
            let along = ray.direction.dot(*ahead);
            (along.abs() > 1e-4).then(|| {
                ray.origin + *ray.direction * ((camera.looking_at - ray.origin).dot(*ahead) / along)
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

pub(crate) fn apply_camera(
    time: Res<Time>,
    mut cameras: Query<(&mut OrbitCamera, &mut Transform)>,
) {
    let ease = 1.0 - (-EASING * time.delta_secs()).exp();
    for (mut camera, mut transform) in &mut cameras {
        camera.looking_at = camera.looking_at.lerp(camera.target, ease);
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
        transform.translation = camera.looking_at + rotation * Vec3::Z * camera.distance;
        transform.rotation = rotation;
    }
}

/// Tells the explorer where the camera stands, for its history and for leaving a
/// journey, without counting as a change to it.
fn remember_pose(mut explorer: ResMut<Explorer>, cameras: Query<&OrbitCamera>) {
    if let Ok(camera) = cameras.single() {
        explorer.bypass_change_detection().see(camera.pose());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_hold_the_angles_near_the_middle_the_same_way_round() {
        let bounds = Bounds {
            middle: (-0.45, -0.38),
            reach: (0.8, 0.4),
        };
        assert_eq!(
            bounds.hold((-0.3, -0.5), -0.3),
            (-0.3, -0.5),
            "inside, as it was"
        );
        let (yaw, pitch) = bounds.hold((-2.0, 0.5), -2.0);
        assert!((yaw - -1.25).abs() < 1e-5 && (pitch - 0.02).abs() < 1e-5);
        let turned = std::f32::consts::TAU;
        let (yaw, _) = bounds.hold((turned + 1.0, -0.38), turned + 1.0);
        assert!(
            (yaw - (turned + 0.35)).abs() < 1e-4,
            "a whole turn round stays round"
        );

        let mut camera = OrbitCamera::default();
        camera.bound(Some(bounds));
        camera.orbit(Vec2::new(2_000.0, 0.0));
        let (yaw, _) = camera.pose().angles;
        assert!(
            (yaw - (-0.45 - 0.8)).abs() < 1e-4,
            "a long drag stops at the edge"
        );
    }
}
