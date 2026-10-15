//! The journey camera. **Flat** looks straight at the time × depth plane with an
//! orthographic projection: drag pans, scroll zooms. **Orbit** is a perspective view
//! that reveals lanes along Z: left-drag orbits, right-drag pans, scroll zooms.

use bevy::{
    camera::{CameraOutputMode, ScalingMode, visibility::RenderLayers},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
    render::render_resource::BlendState,
};
use bevy_egui::{EguiGlobalSettings, PrimaryEguiContext, input::egui_wants_any_pointer_input};

pub(crate) struct JourneyCameraPlugin;

impl Plugin for JourneyCameraPlugin {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum CameraMode {
    #[default]
    Flat,
    Orbit,
}

#[derive(Component, Debug)]
pub(crate) struct JourneyCamera {
    pub(crate) mode: CameraMode,
    /// Where the camera is easing its focus towards.
    pub(crate) target: Vec3,
    focus: Vec3,
    /// World units visible vertically (flat) or distance from the focus (orbit).
    pub(crate) extent: f32,
    yaw: f32,
    pitch: f32,
    /// Viewport width / height, kept current so framing fits both axes.
    aspect: f32,
    /// The box last framed, refit when the viewport's shape changes.
    framed: Option<(Vec3, Vec3)>,
}

impl Default for JourneyCamera {
    fn default() -> Self {
        Self {
            mode: CameraMode::default(),
            target: Vec3::ZERO,
            focus: Vec3::ZERO,
            extent: 20.0,
            yaw: -0.5,
            pitch: -0.35,
            aspect: 1.0,
            framed: None,
        }
    }
}

impl JourneyCamera {
    /// Frames an axis-aligned box, leaving room around it.
    pub(crate) fn frame(&mut self, min: Vec3, max: Vec3) {
        self.framed = Some((min, max));
        self.target = (min + max) / 2.0;
        let size = max - min;
        self.extent = match self.mode {
            // Fit height or width, whichever needs more, with a margin.
            CameraMode::Flat => ((size.y + 4.0).max((size.x + 4.0) / self.aspect)).max(8.0),
            CameraMode::Orbit => (size.length() / 2.0 / 0.3_f32.sin()).max(12.0),
        };
    }

    pub(crate) fn aspect(&self) -> f32 {
        self.aspect
    }

    /// Whether a world point is inside the flat view, with `margin` world units to spare.
    pub(crate) fn shows(&self, point: Vec3, margin: f32) -> bool {
        let half = Vec2::new(self.extent * self.aspect, self.extent) / 2.0 - margin;
        let offset = (point - self.target).truncate().abs();
        offset.x <= half.x && offset.y <= half.y
    }

    pub(crate) fn set_mode(&mut self, mode: CameraMode) {
        self.mode = mode;
    }

    pub(crate) fn set_angles(&mut self, yaw: f32, pitch: f32) {
        self.yaw = yaw;
        self.pitch = pitch;
    }

    pub(crate) fn toggle(&mut self) {
        self.mode = match self.mode {
            CameraMode::Flat => CameraMode::Orbit,
            CameraMode::Orbit => CameraMode::Flat,
        };
    }
}

const ORBIT_SPEED: f32 = 0.005;
const PITCH_LIMIT: f32 = 1.5;
const FOCUS_EASING: f32 = 8.0;
const FLAT_DISTANCE: f32 = 200.0;

fn spawn_camera(mut commands: Commands, mut egui_settings: ResMut<EguiGlobalSettings>) {
    commands.spawn((
        Camera3d::default(),
        JourneyCamera::default(),
        Projection::Orthographic(OrthographicProjection::default_3d()),
        Transform::default(),
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
    mut cameras: Query<(&mut JourneyCamera, &Camera, &Transform)>,
) {
    for (mut camera, view, transform) in &mut cameras {
        let lines = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        if lines != 0.0 {
            camera.extent = (camera.extent * (1.0 - lines * 0.1)).clamp(2.0, 4_000.0);
        }
        let viewport_height = view
            .logical_viewport_size()
            .map_or(800.0, |size| size.y.max(1.0));
        match camera.mode {
            CameraMode::Flat => {
                if buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Left) {
                    let per_pixel = camera.extent / viewport_height;
                    let offset = Vec3::new(-motion.delta.x, motion.delta.y, 0.0) * per_pixel;
                    camera.target += offset;
                    camera.focus += offset;
                }
            }
            CameraMode::Orbit => {
                if buttons.pressed(MouseButton::Left) {
                    camera.yaw -= motion.delta.x * ORBIT_SPEED;
                    camera.pitch = (camera.pitch - motion.delta.y * ORBIT_SPEED)
                        .clamp(-PITCH_LIMIT, PITCH_LIMIT);
                }
                if buttons.pressed(MouseButton::Right) {
                    let scale = camera.extent * 0.0015;
                    let offset = (transform.right() * -motion.delta.x
                        + transform.up() * motion.delta.y)
                        * scale;
                    camera.target += offset;
                    camera.focus += offset;
                }
            }
        }
    }
}

fn apply_camera(
    time: Res<Time>,
    mut cameras: Query<(&mut JourneyCamera, &Camera, &mut Transform, &mut Projection)>,
) {
    let ease = 1.0 - (-FOCUS_EASING * time.delta_secs()).exp();
    for (mut camera, view, mut transform, mut projection) in &mut cameras {
        if let Some(size) = view.logical_viewport_size().filter(|s| s.y > 0.0) {
            let aspect = size.x / size.y;
            if (aspect - camera.aspect).abs() > 0.01 {
                camera.aspect = aspect;
                if let Some((min, max)) = camera.framed {
                    camera.frame(min, max);
                    camera.focus = camera.target;
                }
            }
        }
        camera.focus = camera.focus.lerp(camera.target, ease);
        match camera.mode {
            CameraMode::Flat => {
                *transform = Transform::from_translation(camera.focus + Vec3::Z * FLAT_DISTANCE)
                    .looking_at(camera.focus, Vec3::Y);
                let height = camera.extent;
                match &mut *projection {
                    Projection::Orthographic(ortho) => {
                        ortho.scaling_mode = ScalingMode::FixedVertical {
                            viewport_height: height,
                        };
                    }
                    other => {
                        *other = Projection::Orthographic(OrthographicProjection {
                            scaling_mode: ScalingMode::FixedVertical {
                                viewport_height: height,
                            },
                            ..OrthographicProjection::default_3d()
                        });
                    }
                }
            }
            CameraMode::Orbit => {
                let rotation = Quat::from_euler(EulerRot::YXZ, camera.yaw, camera.pitch, 0.0);
                transform.translation = camera.focus + rotation * Vec3::Z * camera.extent;
                transform.rotation = rotation;
                if !matches!(*projection, Projection::Perspective(_)) {
                    *projection = Projection::Perspective(PerspectiveProjection::default());
                }
            }
        }
    }
}
