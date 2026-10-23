//! The graph camera: a perspective view orbiting a point, driven as Blender's viewport
//! is. On a trackpad, two fingers orbit, shift and two fingers pan, and a pinch or
//! ctrl and two fingers zoom; with a mouse, the wheel zooms, left- or middle-drag
//! orbits and right- or shift-drag pans. Zoom goes towards the pointer, so you can
//! zoom into any part of the graph. Panning or zooming away lets go of the focused
//! node until you click one or press F. Moves to a new point, distance or angle ease
//! in. What floats over the graph (the docked journey, the legend, cards) leaves it a
//! smaller room: the camera centers on the middle of that room and frames to fit it.

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
    /// camera stops holding it in place.
    pub(crate) free: bool,
    /// The part of the view left clear of what floats over it.
    room: Room,
    /// Where on screen the point orbited sits, from the room's middle: off it when
    /// what is framed spreads further one way than the other, so that is centered.
    aim: Vec2,
    /// Where on screen the point orbited sits, easing towards where it is aimed.
    shift: Vec2,
    /// What was last framed, refitted when the room changes until the viewer zooms or
    /// pans.
    framed: Option<Framed>,
    /// The vertical field of view and the view's width over its height.
    fov: f32,
    aspect: f32,
}

/// The part of the view clear of overlays: its middle and half its size, in the
/// view's normalized coordinates (-1 to 1 across, y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Room {
    pub(crate) middle: Vec2,
    pub(crate) half: Vec2,
}

impl Default for Room {
    fn default() -> Self {
        Self {
            middle: Vec2::ZERO,
            half: Vec2::ONE,
        }
    }
}

impl Room {
    /// The room `clear` leaves within a view filling `view`, both in pixels with y
    /// down.
    pub(crate) fn within(view: Rect, clear: Rect) -> Self {
        let size = view.size().max(Vec2::ONE);
        let to_ndc = |p: Vec2| {
            let share = (p - view.min) / size;
            Vec2::new(share.x * 2.0 - 1.0, 1.0 - share.y * 2.0)
        };
        Self {
            middle: to_ndc(clear.center()),
            half: (clear.size() / size).clamp(Vec2::splat(0.05), Vec2::ONE),
        }
    }
}

/// Points framed, and the closest the camera may come while framing them.
#[derive(Debug, Clone)]
struct Framed {
    points: Vec<Vec3>,
    nearest: f32,
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
            room: Room::default(),
            aim: Vec2::ZERO,
            shift: Vec2::ZERO,
            framed: None,
            fov: PerspectiveProjection::default().fov,
            aspect: 1.6,
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

    /// Eases towards orbiting `target` from just far enough back to show all of
    /// `points` in the room, and no nearer than `nearest`, with what they span centered
    /// in it. Refits them if the room changes, until the viewer moves the camera.
    pub(crate) fn frame(&mut self, target: Vec3, points: Vec<Vec3>, nearest: f32) {
        let (aim, distance) = self.aim_at(&points, target, nearest);
        self.aim = aim;
        self.glide_to(target, distance);
        self.framed = Some(Framed { points, nearest });
    }

    /// How far back, orbiting `center` and turned where it is turning to, the camera
    /// must be for every one of `points` to sit inside the room, clear of its edges.
    pub(crate) fn fit_distance(&self, points: &[Vec3], center: Vec3) -> f32 {
        self.fit_aimed(points, center, self.aim)
    }

    /// Where to put the point orbited, `center`, and how far back to be, so `points`
    /// are centered in the room and fit it, no nearer than `nearest`.
    fn aim_at(&self, points: &[Vec3], center: Vec3, nearest: f32) -> (Vec2, f32) {
        // The point orbited stays inside the part of the room what is framed may use.
        let limit = self.room.half * MARGIN * 0.99;
        let mut aim = Vec2::ZERO;
        let mut distance = self.fit_aimed(points, center, aim).max(nearest);
        // Centered, they need the camera back only as far as what they span takes to
        // fill the room. Perspective moves where they land as the camera moves, so a
        // few rounds settle both.
        for _ in 0..3 {
            let Some((low, high)) = self.on_screen(points, center, distance, aim) else {
                break;
            };
            let (low, high) = (low * distance, high * distance);
            let needed = ((high - low) / (2.0 * self.room.half * MARGIN))
                .max_element()
                .max(nearest);
            aim = (-(low + high) / (2.0 * needed)).clamp(-limit, limit);
            distance = self.fit_aimed(points, center, aim).max(nearest);
        }
        (aim, distance)
    }

    /// [`Self::fit_distance`] with the point orbited `aim` off the room's middle.
    fn fit_aimed(&self, points: &[Vec3], center: Vec3, aim: Vec2) -> f32 {
        let tan = self.tangents();
        let shift = (self.room.middle + aim) * tan;
        // How far each way a point may land from the point orbited, in tangents.
        let reach = self.room.half * MARGIN;
        let (low, high) = ((-reach - aim) * tan, (reach - aim) * tan);
        let need = |v: f32, low: f32, high: f32| if v >= 0.0 { v / high } else { v / low };
        let (right, up, back) = self.axes();
        points
            .iter()
            .map(|&p| {
                let offset = p - center;
                // A point this far towards the camera needs the camera that much further,
                // and lands that much off the point orbited.
                let toward = offset.dot(back);
                let across = offset.dot(right) + shift.x * toward;
                let along = offset.dot(up) + shift.y * toward;
                need(across, low.x, high.x).max(need(along, low.y, high.y)) + toward
            })
            .fold(0.0, f32::max)
    }

    /// Where `points` land on screen, from the point orbited and in the view's
    /// normalized coordinates, orbiting `center` from `distance` with it `aim` off the
    /// room's middle: their least and greatest.
    fn on_screen(
        &self,
        points: &[Vec3],
        center: Vec3,
        distance: f32,
        aim: Vec2,
    ) -> Option<(Vec2, Vec2)> {
        let tan = self.tangents();
        let shift = (self.room.middle + aim) * tan;
        let (right, up, back) = self.axes();
        let landed: Vec<Vec2> = points
            .iter()
            .filter_map(|&p| {
                let offset = p - center;
                let toward = offset.dot(back);
                let depth = distance - toward;
                (depth > 1e-3).then(|| {
                    let at = Vec2::new(offset.dot(right), offset.dot(up)) + shift * toward;
                    at / (depth * tan)
                })
            })
            .collect();
        let low = landed.iter().copied().reduce(Vec2::min)?;
        let high = landed.iter().copied().reduce(Vec2::max)?;
        Some((low, high))
    }

    /// The middle of `points` as the camera sees them, turned where it is turning to:
    /// halfway between their extremes across and up the view, at their average depth.
    pub(crate) fn view_middle(&self, points: &[Vec3]) -> Option<Vec3> {
        if points.is_empty() {
            return None;
        }
        let rotation = self.heading();
        let inverse = rotation.inverse();
        let local: Vec<Vec3> = points.iter().map(|&p| inverse * p).collect();
        let min = local.iter().copied().fold(Vec3::INFINITY, Vec3::min);
        let max = local.iter().copied().fold(Vec3::NEG_INFINITY, Vec3::max);
        let depth = local.iter().map(|p| p.z).sum::<f32>() / local.len() as f32;
        Some(rotation * ((min + max) / 2.0).with_z(depth))
    }

    /// Takes the room left clear of overlays, refitting what was framed when it
    /// changes.
    pub(crate) fn set_room(&mut self, room: Room) {
        let moved = (room.middle - self.room.middle).abs().max_element();
        let grew = (room.half - self.room.half).abs().max_element();
        if moved.max(grew) < 0.01 {
            return;
        }
        self.room = room;
        if let Some(framed) = &self.framed {
            let (aim, distance) = self.aim_at(&framed.points, self.target, framed.nearest);
            self.aim = aim;
            self.goal_distance = Some(distance);
        }
    }

    /// Where the camera faces once it has turned where it is turning.
    fn heading(&self) -> Quat {
        let (yaw, pitch) = self.goal_angles.unwrap_or((self.yaw, self.pitch));
        Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0)
    }

    /// Right, up and back, as the camera faces once it has turned.
    fn axes(&self) -> (Vec3, Vec3, Vec3) {
        let rotation = self.heading();
        (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z)
    }

    /// The tangents of half the field of view, across and up.
    fn tangents(&self) -> Vec2 {
        let up = (self.fov / 2.0).tan();
        Vec2::new(up * self.aspect, up)
    }

    /// Moves the point orbited by `offset` at once, letting go of the focus.
    fn pan(&mut self, offset: Vec3) {
        self.target += offset;
        self.focus += offset;
        self.free = true;
        self.framed = None;
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
        self.framed = None;
    }

    /// Turns by a drag of `delta` pixels.
    fn orbit(&mut self, delta: Vec2) {
        self.goal_angles = None;
        self.framed = None;
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
/// The share of the room what is framed may use: the rest is room for its names.
const MARGIN: f32 = 0.8;
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
    mut cameras: Query<(&mut OrbitCamera, &mut Transform, &Projection)>,
) {
    let ease = 1.0 - (-EASING * time.delta_secs()).exp();
    for (mut camera, mut transform, projection) in &mut cameras {
        if let Projection::Perspective(perspective) = projection {
            camera.fov = perspective.fov;
            camera.aspect = perspective.aspect_ratio;
        }
        camera.focus = camera.focus.lerp(camera.target, ease);
        camera.shift = camera.shift.lerp(camera.room.middle + camera.aim, ease);
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
        // Stepping sideways, not turning, puts the point orbited in the room's middle:
        // it stays put on screen as the camera orbits or zooms.
        let off_middle = -camera.shift * camera.tangents();
        transform.translation = camera.focus + rotation * off_middle.extend(1.0) * camera.distance;
        transform.rotation = rotation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head_on() -> OrbitCamera {
        OrbitCamera {
            yaw: 0.0,
            pitch: 0.0,
            ..default()
        }
    }

    #[test]
    fn pulls_back_only_as_far_as_what_it_must_show() {
        let camera = head_on();
        let tall = [Vec3::new(0.0, 40.0, 0.0)];
        let needed = camera.fit_distance(&tall, Vec3::ZERO);
        let half_height = needed * (camera.fov / 2.0).tan();
        assert!(
            half_height > 40.0,
            "far enough back that it shows, with room: {needed}"
        );
        let wide = [Vec3::new(40.0, 0.0, 0.0)];
        assert!(
            camera.fit_distance(&wide, Vec3::ZERO) < needed,
            "the view is wider than it is tall"
        );
    }

    #[test]
    fn a_smaller_room_needs_the_camera_further_back() {
        let mut camera = head_on();
        let points = [Vec3::new(30.0, 20.0, 0.0), Vec3::new(-30.0, -20.0, 0.0)];
        let whole = camera.fit_distance(&points, Vec3::ZERO);
        // The top half of the view, as when the journey docks across the bottom.
        camera.set_room(Room::within(
            Rect::new(0.0, 0.0, 1600.0, 1000.0),
            Rect::new(0.0, 0.0, 1600.0, 500.0),
        ));
        assert_eq!(camera.room.middle, Vec2::new(0.0, 0.5));
        assert!(camera.fit_distance(&points, Vec3::ZERO) > whole * 1.4);
    }

    #[test]
    fn refits_what_it_framed_when_the_room_changes() {
        let mut camera = head_on();
        let points = vec![Vec3::new(0.0, 40.0, 0.0), Vec3::new(0.0, -40.0, 0.0)];
        camera.frame(Vec3::ZERO, points, 1.0);
        let before = camera.goal_distance.unwrap();
        camera.set_room(Room {
            middle: Vec2::ZERO,
            half: Vec2::new(1.0, 0.5),
        });
        assert!(camera.goal_distance.unwrap() > before * 1.9);
        // Once the viewer zooms by hand, the room no longer moves the camera.
        camera.zoom(0.9, None);
        camera.set_room(Room::default());
        assert!(camera.goal_distance.is_none());
    }

    #[test]
    fn what_spreads_one_way_is_centered_with_the_point_orbited_off_the_middle() {
        let mut camera = head_on();
        let lopsided = vec![Vec3::ZERO, Vec3::new(40.0, 10.0, 0.0)];
        let around = camera.fit_aimed(&lopsided, Vec3::ZERO, Vec2::ZERO);
        camera.frame(Vec3::ZERO, lopsided.clone(), 1.0);
        let distance = camera.goal_distance.unwrap();
        assert!(
            camera.aim.x < -0.3 && camera.aim.y < 0.0,
            "{:?}",
            camera.aim
        );
        assert!(
            distance < around * 0.7,
            "closer once centered: {distance} vs {around}"
        );
        let (low, high) = camera
            .on_screen(&lopsided, Vec3::ZERO, distance, camera.aim)
            .unwrap();
        let middle = (low + high) / 2.0 + camera.aim;
        assert!(middle.length() < 0.02, "centered in the room: {middle:?}");
    }

    #[test]
    fn the_middle_is_between_the_extremes() {
        let camera = head_on();
        let points = [
            Vec3::new(-10.0, 0.0, 0.0),
            Vec3::new(-8.0, 0.0, 0.0),
            Vec3::new(30.0, 4.0, 0.0),
        ];
        let middle = camera.view_middle(&points).unwrap();
        assert!((middle - Vec3::new(10.0, 2.0, 0.0)).length() < 1e-4);
    }
}
