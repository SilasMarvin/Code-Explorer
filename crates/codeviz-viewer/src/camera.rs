//! The graph camera: a perspective view orbiting a point, driven as Blender's viewport
//! is. On a trackpad, two fingers orbit, shift and two fingers pan, and a pinch or
//! ctrl and two fingers zoom; with a mouse, the wheel zooms, left- or middle-drag
//! orbits and right- or shift-drag pans. Zoom goes towards the pointer, so you can
//! zoom into any part of the graph, as near as a module filling most of the view and
//! as far as the whole scene with room round it. The camera never goes inside a
//! sphere: the wheel stops once the one under the pointer fills nine tenths of the
//! free part of the view, drawn into it whole as it comes to, or with none under it,
//! while the one nearest it still shows; zoomed out, the map is drawn into the free
//! part of the view as it comes to fit there, and held inside once it does. A pan
//! keeps the map in the free part of the view too. Moves to a new point, distance or
//! angle take one short, eased [`TRANSITION_SECONDS`]; a hand on the wheel or a drag
//! stops them where they are.

use bevy::{
    camera::{CameraOutputMode, visibility::RenderLayers},
    input::{
        gestures::PinchGesture,
        mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    },
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

/// How long every move the camera is sent on takes, and every re-gathering of the
/// spheres: one length for every transition.
pub(crate) const TRANSITION_SECONDS: f32 = 0.35;

/// The longest step a transition takes in one frame, in seconds: after an idle spell
/// the first frame's time is long, and would otherwise jump to the end.
const LONGEST_STEP: f32 = 1.0 / 30.0;

/// How far along a transition `dt` seconds take it.
pub(crate) fn transition_step(dt: f32) -> f32 {
    dt / TRANSITION_SECONDS
}

/// The time this frame moves transitions along by: its own, but no more than
/// [`LONGEST_STEP`].
pub(crate) fn frame_time(time: &Time) -> f32 {
    time.delta_secs().min(LONGEST_STEP)
}

/// The one easing: the share of the way at `t` of a transition (0 to 1), slow at
/// both ends and never past the end.
pub(crate) fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Component, Debug)]
pub(crate) struct OrbitCamera {
    /// Where the camera is going to look.
    pub(crate) target: Vec3,
    /// Where it looks now.
    looking_at: Vec3,
    /// Distance from where it looks.
    pub(crate) distance: f32,
    yaw: f32,
    pitch: f32,
    /// A distance it is sent to, until it gets there or the viewer zooms by hand.
    goal_distance: Option<f32>,
    /// Angles it is sent to, until it gets there or the viewer orbits by hand.
    goal_angles: Option<(f32, f32)>,
    /// The move under way: where it set out from, where to, and how far along.
    trip: Option<Trip>,
    /// The angles it may turn to: held near the usual view, so the map never turns
    /// edge-on.
    bounds: Option<Bounds>,
    /// How near and how far the wheel may take it.
    reach: Option<Reach>,
    /// Whether the wheel, turning one way (`Some(out)`) since it last turned the
    /// other, or an orbit or pan, drew the map, or the sphere it zoomed to, into the
    /// free area or held it there, beside zooming about the pointer.
    held: Option<bool>,
}

/// Where the camera stands, for a move from one stand to another.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Stand {
    looking_at: Vec3,
    distance: f32,
    angles: (f32, f32),
}

/// A move under way.
#[derive(Debug, Clone, Copy)]
struct Trip {
    from: Stand,
    to: Stand,
    progress: f32,
}

/// How near and how far the wheel may take the camera, and what it keeps clear of.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reach {
    /// The closest it comes to what it looks at.
    pub(crate) near: f32,
    /// The furthest.
    pub(crate) far: f32,
    /// The middle of everything shown, and half how far it reaches across and up
    /// the view: the map the wheel out holds in the free area.
    pub(crate) home: Vec3,
    pub(crate) extent: Vec2,
    /// Every sphere shown, by middle and radius: the camera stays outside each.
    pub(crate) spheres: Vec<(Vec3, f32)>,
    /// The view the camera sees through.
    pub(crate) lens: Lens,
}

/// The camera's view as a share of where it looks, and the part of it nothing lies
/// over: `tan`, how far off the middle the view's edges are per unit of depth, across
/// and up; `centre` and `half`, the free area's middle and half its size, in the
/// view's own units (-1 to 1 across and up, up positive).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Lens {
    pub(crate) tan: Vec2,
    pub(crate) centre: Vec2,
    pub(crate) half: Vec2,
}

impl Lens {
    /// The whole view free, `fov` high and `aspect` times as wide.
    pub(crate) fn whole(fov: f32, aspect: f32) -> Self {
        let up = (fov / 2.0).tan();
        Self {
            tan: Vec2::new(up * aspect, up),
            centre: Vec2::ZERO,
            half: Vec2::ONE,
        }
    }

    /// How far the camera looks off a point, per unit of distance, for the point to
    /// show in the free area's middle rather than the view's.
    pub(crate) fn shift(&self, rotation: Quat) -> Vec3 {
        rotation * Vec3::new(self.centre.x * self.tan.x, self.centre.y * self.tan.y, 0.0)
    }

    /// Where `point` shows, in the view's own units, seen from `eye` turned to
    /// `rotation`, and how deep it is; none when it is behind the camera.
    pub(crate) fn project(&self, rotation: Quat, eye: Vec3, point: Vec3) -> Option<(Vec2, f32)> {
        let local = rotation.inverse() * (point - eye);
        let depth = -local.z;
        (depth > 1e-4).then(|| {
            (
                Vec2::new(
                    local.x / (depth * self.tan.x),
                    local.y / (depth * self.tan.y),
                ),
                depth,
            )
        })
    }

    /// The free area, `share` of it about its middle: its corners in the view's
    /// own units.
    pub(crate) fn free(&self, share: f32) -> (Vec2, Vec2) {
        (
            self.centre - self.half * share,
            self.centre + self.half * share,
        )
    }
}

/// Zooming out, how many times the free area the map is when the wheel starts to
/// draw it in ([`OrbitCamera::hold_map`]).
const DRAWN_FROM: f32 = 1.5;

/// How far outside a sphere's surface, as a share of its radius, the camera keeps.
const OUTSIDE: f32 = 1.1;

/// Zoomed in as far as the wheel goes, the sphere under the pointer's diameter
/// against the free area's smaller side.
const NEAREST_FILL: f32 = 0.9;

/// Zooming in, the sphere under the pointer's diameter against the free area's
/// smaller side when the wheel starts to draw it into the free area, all of it in by
/// [`NEAREST_FILL`] ([`OrbitCamera::keep_aimed`]).
const KEPT_FROM: f32 = 0.6;

/// Room kept above a sphere drawn into the free area for its name, as a share of half
/// the free area's height: about a name's height.
const NAME_ROOM: f32 = 0.05;

/// The share of a pan the map's edge must hold back for the pan to count as stopped
/// there, and say so.
const HELD_BACK: f32 = 0.25;

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
            trip: None,
            bounds: None,
            reach: None,
            held: None,
        }
    }
}

impl OrbitCamera {
    /// Whether the camera is still moving to where it was sent.
    pub(crate) fn easing(&self) -> bool {
        self.goal() != self.stand()
    }

    /// Where it stands now.
    fn stand(&self) -> Stand {
        Stand {
            looking_at: self.looking_at,
            distance: self.distance,
            angles: (self.yaw, self.pitch),
        }
    }

    /// Where it is sent.
    fn goal(&self) -> Stand {
        Stand {
            looking_at: self.target,
            distance: self.goal_distance(),
            angles: self.goal_angles.unwrap_or((self.yaw, self.pitch)),
        }
    }

    /// Stops wherever it is on its way: a hand on the wheel or a drag takes over.
    fn halt(&mut self) {
        self.target = self.looking_at;
        self.goal_distance = None;
        self.goal_angles = None;
        self.trip = None;
    }

    /// Moves along to where it is sent by `step` of a whole transition: a new goal
    /// starts a new move from where it stands.
    fn advance(&mut self, step: f32) {
        let (now, goal) = (self.stand(), self.goal());
        if now == goal {
            self.trip = None;
            return;
        }
        let trip = match self.trip {
            Some(trip) if trip.to == goal => trip,
            _ => Trip {
                from: now,
                to: goal,
                progress: 0.0,
            },
        };
        let progress = (trip.progress + step).min(1.0);
        let t = ease(progress);
        let (from, to) = (trip.from, trip.to);
        self.looking_at = from.looking_at.lerp(to.looking_at, t);
        self.distance = from.distance + (to.distance - from.distance) * t;
        self.yaw = from.angles.0 + (to.angles.0 - from.angles.0) * t;
        self.pitch = from.angles.1 + (to.angles.1 - from.angles.1) * t;
        if progress >= 1.0 {
            self.looking_at = to.looking_at;
            self.distance = to.distance;
            (self.yaw, self.pitch) = to.angles;
            self.goal_distance = None;
            self.goal_angles = None;
            self.trip = None;
        } else {
            self.trip = Some(Trip { progress, ..trip });
        }
    }

    /// Eases towards looking at `target` from `distance` away.
    pub(crate) fn glide_to(&mut self, target: Vec3, distance: f32) {
        self.target = target;
        self.goal_distance = Some(distance);
    }

    /// Moves the point orbited by `offset` at once.
    fn pan(&mut self, offset: Vec3) {
        self.halt();
        self.target += offset;
        self.looking_at += offset;
    }

    /// Pans by hand, no further than keeps the map in the free area, then backs off
    /// from any sphere that took the camera in. Returns what stopped it, when the
    /// map's edge, or zoomed all the way out the middle, held back much of the move
    /// ([`HELD_BACK`]).
    fn pan_by_hand(&mut self, offset: Vec3) -> Option<&'static str> {
        self.held = None;
        let reach = self.reach.clone()?;
        let was = self.map_shown(&reach);
        self.pan(offset);
        let asked = self.target;
        if let Some((was, _)) = was {
            self.keep_map(&reach, was);
        }
        self.back_out();
        let held = asked.distance(self.target) >= offset.length() * HELD_BACK;
        held.then(|| {
            if self.towards_far(&reach) <= 0.0 {
                "Zoomed all the way out, the map stays in the middle"
            } else {
                "The edge of the map: it pans no further that way"
            }
        })
    }

    /// Zoomed out, how far from the far end the camera is: 1 where the map just fits
    /// the free area, or nearer, 0 at the far end.
    fn towards_far(&self, reach: &Reach) -> f32 {
        self.map_seen(reach).map_or(1.0, |(_, _, fits)| {
            ((reach.far - self.distance) / (reach.far - fits).max(1e-3)).clamp(0.0, 1.0)
        })
    }

    /// Where the map shows: how far its middle is off the free area's and half its
    /// size, across and up, in the view's units, from every sphere's disc; and how
    /// deep its middle is.
    fn map_shown(&self, reach: &Reach) -> Option<((Vec2, Vec2), f32)> {
        let (rotation, eye) = (self.rotation(), self.eye());
        let (low, high) = reach
            .spheres
            .iter()
            .filter_map(|&sphere| disc(&reach.lens, rotation, eye, sphere))
            .map(|(at, _, size)| (at - size, at + size))
            .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))?;
        let (_, depth) = reach.lens.project(rotation, eye, reach.home)?;
        Some((
            ((low + high) / 2.0 - reach.lens.centre, (high - low) / 2.0),
            depth,
        ))
    }

    /// After a pan, with the map's middle `was` off the free area's before it, pans
    /// back as far as needed for the map to stay inside the free area across or up
    /// where it fits there, and where it does not, for the free area to stay on the
    /// map, its edge coming in no further than the free area's middle (by twice
    /// what it is bigger than the free area, up to that). A map already further off
    /// than that may come back, but goes no further. Zoomed out, as the wheel out
    /// holds the map ([`Self::hold_map`]), a map that fits may stray the less from
    /// the middle the nearer the far end, where it stays in the middle.
    fn keep_map(&mut self, reach: &Reach, (was, _): (Vec2, Vec2)) {
        let lens = reach.lens;
        let towards_far = self.towards_far(reach);
        // Nearer spheres move further on screen than the map's middle, so twice.
        for _ in 0..2 {
            let Some(((off, size), depth)) = self.map_shown(reach) else {
                return;
            };
            let room = |size: f32, half: f32, was: f32| {
                if size <= half {
                    (pan_room(size, half).max(was.abs())) * towards_far
                } else {
                    pan_room(size, half).max(was.abs())
                }
            };
            let room = Vec2::new(
                room(size.x, lens.half.x, was.x),
                room(size.y, lens.half.y, was.y),
            );
            let by = off.clamp(-room, room) - off;
            if by.length() < 1e-5 {
                return;
            }
            let tan = lens.tan;
            self.pan(
                self.rotation() * Vec3::new(-by.x * depth * tan.x, -by.y * depth * tan.y, 0.0),
            );
        }
    }

    /// Which way the camera faces now.
    fn rotation(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0)
    }

    /// Where the camera stands now.
    fn eye(&self) -> Vec3 {
        self.looking_at + self.rotation() * Vec3::Z * self.distance
    }

    /// Backs away from what it looks at until it is outside every sphere: an orbit
    /// or a pan may have swung it into one.
    fn back_out(&mut self) {
        let Some(reach) = &self.reach else {
            return;
        };
        let back = self.rotation() * Vec3::Z;
        let mut distance = self.distance;
        for &(middle, radius) in &reach.spheres {
            // Where along `back` from what it looks at the camera leaves the sphere.
            let to = self.looking_at - middle;
            let along = to.dot(back);
            let room = along * along - (to.length_squared() - (radius * OUTSIDE).powi(2));
            if room >= 0.0 {
                let (enter, leave) = (-along - room.sqrt(), -along + room.sqrt());
                if distance > enter && distance < leave {
                    distance = leave;
                }
            }
        }
        if distance > self.distance {
            self.distance = distance;
            self.goal_distance = None;
        }
    }

    /// Zooms by `factor` (below 1 is nearer), towards `toward` when given: the whole
    /// view scales about that point, so it stays where it is on screen. In, it stops
    /// outside every sphere, once the sphere under the pointer fills the view, and with
    /// the pointer over none, while the sphere nearest it still shows; out, it goes no
    /// further than the reach's far end, draws the map in as it comes to fit the free
    /// area and holds it inside once it does. It never jumps to a bound from outside
    /// it.
    fn zoom(&mut self, factor: f32, toward: Option<Vec3>) {
        self.halt();
        let point = toward.unwrap_or(self.looking_at);
        let Some(reach) = self.reach.clone() else {
            self.scale_about(point, factor);
            return;
        };
        if factor < 1.0 {
            let aimed = self.aimed(&reach, point);
            let scale = self.nearest_scale(&reach, point, factor, aimed);
            self.scale_about(point, scale);
            self.held = self.held.filter(|out| !out);
            if self.keep_aimed(&reach, aimed) {
                self.held = Some(false);
            }
            return;
        }
        let before = self.distance;
        let distance = (before * factor).min(reach.far.max(before));
        let was = self.map_seen(&reach);
        self.scale_about(point, distance / before);
        if let Some(was) = was {
            self.held = self.held.filter(|&out| out);
            if self.hold_map(&reach, was, before) {
                self.held = Some(true);
            }
        }
    }

    /// Where the map's middle shows, off the free area's middle, in the view's
    /// units, how deep, and how far from what it looks at the camera would just fit
    /// the map in the free area.
    fn map_seen(&self, reach: &Reach) -> Option<(Vec2, f32, f32)> {
        let lens = reach.lens;
        let (at, depth) = lens.project(self.rotation(), self.eye(), reach.home)?;
        let share = reach.extent / (depth * lens.tan) / lens.half;
        Some((at - lens.centre, depth, self.distance * share.max_element()))
    }

    /// Zoomed out from `before` to where it is, with the map seen as `was` says
    /// then, pans just enough that the map's middle is no further off the free
    /// area's than this: once the map fits the free area, across or up, so far that
    /// it is inside, and less the nearer the far end, where it is in the middle; as
    /// it comes to fit, from where it is [`DRAWN_FROM`] times the
    /// free area, half the free area at first, nothing by the time it fits, but never
    /// less than as much of how far off it was as the wheel left of the way, so it
    /// never jumps. Returns whether it panned.
    fn hold_map(
        &mut self,
        reach: &Reach,
        (off_before, _, fits): (Vec2, f32, f32),
        before: f32,
    ) -> bool {
        let Some((off, depth, _)) = self.map_seen(reach) else {
            return false;
        };
        let lens = reach.lens;
        let tan = lens.tan;
        let size = reach.extent / (depth * tan);
        // Across or up, where it fits, as far as keeps it inside, and less the
        // nearer the far end, where it is in the middle.
        let towards_far = self.towards_far(reach);
        let mut room = (lens.half - size).map(|room| {
            if room >= 0.0 {
                room * towards_far
            } else {
                f32::INFINITY
            }
        });
        let left = |d: f32| ((fits - d) / (fits - fits / DRAWN_FROM)).clamp(0.0, 1.0);
        let (was, now) = (left(before), left(self.distance));
        if now > 0.0 && now < 1.0 {
            let kept = if was > 0.0 { now / was } else { 0.0 };
            room = room.min((lens.half * now).max(off_before.abs() * kept));
        }
        let by = off.clamp(-room, room) - off;
        if by == Vec2::ZERO {
            return false;
        }
        self.pan(self.rotation() * Vec3::new(-by.x * depth * tan.x, -by.y * depth * tan.y, 0.0));
        true
    }

    /// Scales the whole view by `scale` about `point`.
    fn scale_about(&mut self, point: Vec3, scale: f32) {
        self.pan((self.looking_at - point) * (scale - 1.0));
        self.distance *= scale;
    }

    /// The sphere the wheel zooms towards with the pointer at `point`.
    fn aimed(&self, reach: &Reach, point: Vec3) -> Option<Aim> {
        let (rotation, eye) = (self.rotation(), self.eye());
        let (at, _) = reach.lens.project(rotation, eye, point)?;
        aimed_sphere(reach, rotation, eye, at)
    }

    /// Zoomed in towards the sphere under the pointer, `aimed`, pans it into the free
    /// area, with room for its name above it, as it comes to fill it: not at all while
    /// its diameter is under [`KEPT_FROM`] of the free area's smaller side, the more of
    /// the way the nearer it is to [`NEAREST_FILL`], and wholly inside once there.
    /// Returns whether it panned.
    fn keep_aimed(&mut self, reach: &Reach, aimed: Option<Aim>) -> bool {
        let Some(Aim::Under(index)) = aimed else {
            return false;
        };
        let lens = reach.lens;
        let Some((at, depth, size)) = reach
            .spheres
            .get(index)
            .and_then(|&sphere| disc(&lens, self.rotation(), self.eye(), sphere))
        else {
            return false;
        };
        let fill = size.y / smaller_side(&lens);
        let kept = ((NEAREST_FILL - fill) / (NEAREST_FILL - KEPT_FROM)).clamp(0.0, 1.0);
        let (low, high) = lens.free(1.0);
        let (low, high) = (
            low + size,
            high - size - Vec2::new(0.0, NAME_ROOM * lens.half.y),
        );
        let inside = Vec2::new(
            at.x.clamp(low.x, high.x.max(low.x)),
            at.y.clamp(low.y, high.y.max(low.y)),
        );
        let by = (inside - at) * (1.0 - kept);
        if by.length() < 1e-5 {
            return false;
        }
        let tan = lens.tan;
        self.pan(self.rotation() * Vec3::new(-by.x * depth * tan.x, -by.y * depth * tan.y, 0.0));
        self.back_out();
        true
    }

    /// The nearest scale towards `wanted` (below 1) that zooming about `point` may
    /// go to, towards the sphere `aimed`: one where the camera stands clear (see
    /// [`Self::clear_at`]) all the way from where it is; none nearer when it is not
    /// clear now.
    fn nearest_scale(&self, reach: &Reach, point: Vec3, wanted: f32, aimed: Option<Aim>) -> f32 {
        let rotation = self.rotation();
        let eye = self.eye();
        let clear = |scale: f32| {
            let eye = point + (eye - point) * scale;
            let looking_at = point + (self.looking_at - point) * scale;
            clear_at(reach, rotation, eye, looking_at, aimed)
        };
        if clear(wanted) {
            return wanted;
        }
        if !clear(1.0) {
            return 1.0;
        }
        let (mut blocked, mut free) = (wanted, 1.0);
        for _ in 0..24 {
            let mid = f32::midpoint(blocked, free);
            if clear(mid) {
                free = mid;
            } else {
                blocked = mid;
            }
        }
        free
    }

    /// How near and how far the wheel may take it from now on.
    pub(crate) fn limit(&mut self, reach: Reach) {
        self.reach = Some(reach);
    }

    /// Turns by a drag of `delta` pixels, no further than the bounds allow.
    fn orbit(&mut self, delta: Vec2) {
        self.halt();
        self.held = None;
        let yaw = self.yaw - delta.x * ORBIT_SPEED;
        let pitch = (self.pitch - delta.y * ORBIT_SPEED).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        (self.yaw, self.pitch) = self
            .bounds
            .map_or((yaw, pitch), |bounds| bounds.hold((yaw, pitch), self.yaw));
        self.back_out();
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

    /// Whether the wheel, since it last turned the other way, drew the map, or the
    /// sphere it zoomed to, into the free area or held it there.
    pub(crate) fn held_map(&self) -> bool {
        self.held.is_some()
    }

    /// Whether it stands as far out as the wheel takes it.
    pub(crate) fn farthest(&self) -> bool {
        self.reach
            .as_ref()
            .is_some_and(|reach| self.distance >= reach.far * (1.0 - 1e-3))
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

    /// Where the camera stands this frame, on its way or not.
    pub(crate) fn pose_now(&self) -> Pose {
        Pose {
            target: self.looking_at,
            distance: self.distance,
            angles: (self.yaw, self.pitch),
        }
    }

    /// Eases back to a pose it had.
    pub(crate) fn return_to(&mut self, pose: Pose) {
        self.target = pose.target;
        self.goal_distance = Some(pose.distance);
        self.turn_to(pose.angles);
    }
}

/// The sphere the wheel zooms towards, by index into the reach's spheres.
#[derive(Debug, Clone, Copy)]
enum Aim {
    /// Under the pointer (of several, the nearest the camera): the wheel stops once
    /// it fills the view.
    Under(usize),
    /// With the pointer over none, the one whose disc on screen is nearest it: the
    /// wheel stops while it still shows.
    Near(usize),
}

/// A sphere of `radius` at `middle` as it shows from `eye` turned to `rotation`: its
/// middle in the view's units, its depth, and its radius across and up.
fn disc(
    lens: &Lens,
    rotation: Quat,
    eye: Vec3,
    (middle, radius): (Vec3, f32),
) -> Option<(Vec2, f32, Vec2)> {
    let (at, depth) = lens.project(rotation, eye, middle)?;
    let size = Vec2::new(radius / (depth * lens.tan.x), radius / (depth * lens.tan.y));
    Some((at, depth, size))
}

/// The sphere the wheel zooms towards with the pointer at `pointer`, in the view's
/// units.
fn aimed_sphere(reach: &Reach, rotation: Quat, eye: Vec3, pointer: Vec2) -> Option<Aim> {
    let mut under: Option<(usize, f32)> = None;
    let mut near: Option<(usize, f32)> = None;
    for (index, &sphere) in reach.spheres.iter().enumerate() {
        let Some((at, depth, size)) = disc(&reach.lens, rotation, eye, sphere) else {
            continue;
        };
        // How far off its rim the pointer is, as a share of its radius on screen.
        let off = ((pointer - at) / size).length() - 1.0;
        let (best, by) = if off <= 0.0 {
            (&mut under, depth)
        } else {
            (&mut near, off)
        };
        if best.is_none_or(|(_, least)| by < least) {
            *best = Some((index, by));
        }
    }
    under
        .map(|(index, _)| Aim::Under(index))
        .or(near.map(|(index, _)| Aim::Near(index)))
}

/// Whether the camera may stand at `eye`, looking at `looking_at`, turned to
/// `rotation`: no nearer what it looks at than the reach's near end, outside every
/// sphere, and with the sphere `aimed` at no bigger than the view, or still showing.
fn clear_at(
    reach: &Reach,
    rotation: Quat,
    eye: Vec3,
    looking_at: Vec3,
    aimed: Option<Aim>,
) -> bool {
    if eye.distance(looking_at) < reach.near {
        return false;
    }
    if reach
        .spheres
        .iter()
        .any(|&(middle, radius)| eye.distance(middle) < radius * OUTSIDE)
    {
        return false;
    }
    let lens = &reach.lens;
    let shows = |index: usize| {
        let sphere = *reach.spheres.get(index)?;
        disc(lens, rotation, eye, sphere)
    };
    match aimed {
        None => true,
        Some(Aim::Under(index)) => shows(index).is_some_and(|(_, _, size)| {
            // Its diameter against the free area's smaller side.
            size.y <= smaller_side(lens) * NEAREST_FILL
        }),
        Some(Aim::Near(index)) => shows(index).is_some_and(|(at, _, size)| {
            let (low, high) = lens.free(1.0);
            ((at.clamp(low, high) - at) / size).length() <= 1.0
        }),
    }
}

/// Half the free area's smaller side, in the view's units up: what a sphere's radius
/// up the view, in the same units, is set against for how much of it it fills.
fn smaller_side(lens: &Lens) -> f32 {
    (lens.half.y).min(lens.half.x * lens.tan.x / lens.tan.y)
}

/// How far the map's middle may be off the free area's, across or up, after a pan:
/// with the map `size` and the free area `half` (each half of it, in the view's
/// units), inside it where it fits; where not, its edge no further in than the free
/// area's middle, by twice what it is bigger, up to that.
fn pan_room(size: f32, half: f32) -> f32 {
    if size <= half {
        half - size
    } else {
        let over = size - half;
        over + over.min(half)
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

#[expect(clippy::too_many_arguments, reason = "a system's inputs")]
fn camera_input(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut pinches: MessageReader<PinchGesture>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut cameras: Query<(&mut OrbitCamera, &Transform, &Camera, &GlobalTransform)>,
    mut explorer: ResMut<Explorer>,
    mut stopped: Local<bool>,
) {
    // A pan the map's edge stops says so once, until a pan moves freely again or
    // the hand lets go.
    if buttons.get_pressed().next().is_none() && scroll.delta == Vec2::ZERO {
        *stopped = false;
    }
    let mut panned = |camera: &mut OrbitCamera, offset: Vec3| {
        let held = camera.pan_by_hand(offset);
        if let Some(why) = held.filter(|_| !*stopped) {
            explorer.say(why);
        }
        *stopped = held.is_some();
    };
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
                    panned(&mut camera, offset);
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
            panned(&mut camera, offset);
        } else if orbiting {
            camera.orbit(motion.delta);
        }
    }
}

pub(crate) fn apply_camera(
    time: Res<Time>,
    mut cameras: Query<(&mut OrbitCamera, &mut Transform)>,
) {
    let step = transition_step(frame_time(&time));
    for (mut camera, mut transform) in &mut cameras {
        camera.advance(step);
        transform.translation = camera.eye();
        transform.rotation = camera.rotation();
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
    fn a_move_takes_one_short_transition_and_never_overshoots() {
        let mut camera = OrbitCamera::default();
        camera.glide_to(Vec3::new(10.0, 0.0, 0.0), 30.0);
        camera.turn_to((0.0, 0.0));
        let frame = 1.0 / 60.0;
        let mut frames = 0;
        let mut last = camera.looking_at.x;
        while camera.easing() {
            camera.advance(transition_step(frame));
            assert!(camera.looking_at.x >= last && camera.looking_at.x <= 10.0);
            assert!(camera.distance >= 30.0 && camera.distance <= 60.0);
            last = camera.looking_at.x;
            frames += 1;
            assert!(frames < 100, "it gets there");
        }
        #[expect(clippy::cast_precision_loss, reason = "a frame count")]
        let took = frames as f32 * frame;
        assert!(took <= TRANSITION_SECONDS + frame, "took {took} s");
        assert_eq!(
            camera.pose().parts(),
            (Vec3::new(10.0, 0.0, 0.0), 30.0, (0.0, 0.0))
        );
        camera.advance(transition_step(frame));
        assert!(!camera.easing(), "and rests there");
    }

    /// A reach for a camera looking straight down -Z, with `spheres`.
    fn reach(near: f32, far: f32, spheres: Vec<(Vec3, f32)>) -> Reach {
        Reach {
            near,
            far,
            home: Vec3::ZERO,
            extent: Vec2::new(8.0, 5.0),
            spheres,
            lens: Lens::whole(std::f32::consts::FRAC_PI_4, 1.6),
        }
    }

    /// A camera at `distance` looking head on at `at`, within `reach`.
    fn head_on(at: Vec3, distance: f32, reach: Reach) -> OrbitCamera {
        let mut camera = OrbitCamera {
            yaw: 0.0,
            pitch: 0.0,
            ..default()
        };
        camera.target = at;
        camera.looking_at = at;
        camera.distance = distance;
        camera.limit(reach);
        camera
    }

    #[test]
    fn the_wheel_stops_at_the_reach_but_never_jumps_to_it() {
        let mut camera = head_on(Vec3::ZERO, 60.0, reach(20.0, 100.0, Vec::new()));
        for _ in 0..40 {
            camera.zoom(0.8, None);
        }
        assert!((camera.distance - 20.0).abs() < 1e-2, "in to the near end");
        for _ in 0..40 {
            camera.zoom(1.25, None);
        }
        assert!((camera.distance - 100.0).abs() < 1e-4, "out to the far end");
        camera.distance = 300.0;
        camera.zoom(0.9, None);
        assert!(
            (camera.distance - 270.0).abs() < 1e-3,
            "from outside, it comes in"
        );
        camera.zoom(1.25, None);
        assert!(
            (camera.distance - 270.0).abs() < 1e-3,
            "but goes no further out"
        );
    }

    #[test]
    fn the_wheel_stops_once_the_sphere_under_the_pointer_fills_the_view() {
        let sphere = (Vec3::new(2.0, 0.0, 0.0), 5.0);
        let mut camera = head_on(Vec3::ZERO, 60.0, reach(0.1, 100.0, vec![sphere]));
        for _ in 0..60 {
            camera.zoom(0.8, Some(Vec3::new(1.0, 0.5, 0.0)));
        }
        let lens = camera.reach.as_ref().unwrap().lens;
        let (at, _, size) = disc(&lens, camera.rotation(), camera.eye(), sphere).unwrap();
        assert!(
            (NEAREST_FILL - 0.02..=NEAREST_FILL).contains(&size.y),
            "it fills the view, not quite: {size}"
        );
        assert!(
            (at.abs() + size).max_element() <= 1.0 + 1e-4,
            "drawn into the view whole: {at} {size}"
        );
        assert!(camera.eye().distance(sphere.0) > sphere.1 * OUTSIDE);
    }

    #[test]
    fn a_pan_keeps_the_map_in_the_free_area() {
        // The map: four spheres, 15 by 10 about the middle.
        let spheres: Vec<(Vec3, f32)> = [(-6.0, -3.5), (6.0, -3.5), (-6.0, 3.5), (6.0, 3.5)]
            .map(|(x, y)| (Vec3::new(x, y, 0.0), 1.5))
            .into();
        let seen = |camera: &OrbitCamera| {
            let reach = camera.reach.as_ref().unwrap();
            camera.map_shown(reach).unwrap().0
        };
        // From 60 away it fits the view: dragged as far as its edge meets the view's,
        // less the nearer the far end.
        let mut camera = head_on(Vec3::ZERO, 60.0, reach(0.1, 100.0, spheres.clone()));
        let stopped = camera.pan_by_hand(Vec3::new(500.0, 300.0, 0.0));
        assert!(stopped.is_some(), "the edge stops it, and says so");
        let towards_far = camera.towards_far(camera.reach.as_ref().unwrap());
        assert!((0.1..0.9).contains(&towards_far), "{towards_far}");
        let (at, size) = seen(&camera);
        assert!(
            (at.x.abs() - (1.0 - size.x) * towards_far).abs() < 1e-3
                && (at.y.abs() - (1.0 - size.y) * towards_far).abs() < 1e-3,
            "dragged as far as it may be, across and up alike: {at} {size}"
        );
        // A drag along one side still moves it along the other.
        let (across, up) = (seen(&camera).0.x, seen(&camera).0.y);
        camera.pan_by_hand(Vec3::new(500.0, -1.0, 0.0));
        let now = seen(&camera).0;
        assert!(
            (now.x - across).abs() < 1e-3 && now.y > up,
            "it moves where it may: {now}"
        );
        // As far out as the wheel goes, it stays in the middle.
        let mut camera = head_on(Vec3::ZERO, 100.0, reach(0.1, 100.0, spheres.clone()));
        let stopped = camera.pan_by_hand(Vec3::new(5.0, 3.0, 0.0));
        assert!(stopped.is_some_and(|why| why.contains("all the way out")));
        assert!(seen(&camera).0.length() < 1e-3, "{}", seen(&camera).0);
        // Close in, the map is far bigger than the view: its edge comes in to the
        // middle, no further.
        let mut camera = head_on(Vec3::ZERO, 5.0, reach(0.1, 100.0, spheres));
        camera.pan_by_hand(Vec3::new(-500.0, 300.0, 0.0));
        let (at, size) = seen(&camera);
        assert!(
            (at.x - size.x).abs() < 1e-3 && (at.y + size.y).abs() < 1e-3,
            "the map's corner at the middle: {at} {size}"
        );
        camera.pan_by_hand(Vec3::new(0.2, 0.0, 0.0));
        let back = seen(&camera).0;
        assert!(back.x < at.x, "it comes back: {back}");
    }

    #[test]
    fn over_nothing_the_wheel_stops_while_the_nearest_sphere_shows() {
        let sphere = (Vec3::new(-10.0, 0.0, 0.0), 1.0);
        let mut camera = head_on(Vec3::ZERO, 60.0, reach(0.1, 100.0, vec![sphere]));
        let empty = Vec3::new(10.0, 0.0, 0.0);
        for _ in 0..60 {
            camera.zoom(0.8, Some(empty));
        }
        let lens = camera.reach.as_ref().unwrap().lens;
        let (at, depth) = lens
            .project(camera.rotation(), camera.eye(), sphere.0)
            .unwrap();
        let rim = at.x + sphere.1 / (depth * lens.tan.x);
        assert!(
            (-1.0..-0.9).contains(&rim),
            "its edge is still on screen, just: {rim}"
        );
    }

    #[test]
    fn zoomed_out_the_map_comes_into_the_free_area_and_stays() {
        let mut camera = head_on(Vec3::new(2.0, 1.0, 0.0), 5.0, reach(1.0, 100.0, Vec::new()));
        let pointer = Vec3::new(3.0, 1.5, 0.0);
        camera.zoom(1.1, Some(pointer));
        assert!(
            camera.looking_at.distance(Vec3::new(1.9, 0.95, 0.0)) < 1e-3,
            "close in, out about the pointer: {}",
            camera.looking_at
        );
        let lens = camera.reach.as_ref().unwrap().lens;
        for _ in 0..60 {
            camera.zoom(1.1, Some(pointer));
            let (at, depth) = lens
                .project(camera.rotation(), camera.eye(), Vec3::ZERO)
                .unwrap();
            let size = Vec2::new(8.0, 5.0) / (depth * lens.tan);
            if size.x < 1.0 && size.y < 1.0 {
                assert!(
                    (at.abs() + size).max_element() <= 1.0 + 1e-4,
                    "once it fits, the map is inside: {at} {size}"
                );
            }
        }
        assert!((camera.distance - 100.0).abs() < 1e-3);
    }

    #[test]
    fn an_orbit_backs_out_of_a_sphere_it_swings_into() {
        let sphere = (Vec3::new(0.0, 0.0, 19.0), 3.0);
        let mut camera = head_on(Vec3::ZERO, 20.0, reach(0.1, 100.0, vec![sphere]));
        camera.orbit(Vec2::ZERO);
        assert!(camera.eye().distance(sphere.0) >= sphere.1 * OUTSIDE - 1e-3);
    }

    #[test]
    fn a_hand_on_the_wheel_stops_a_move() {
        let mut camera = OrbitCamera::default();
        camera.glide_to(Vec3::new(10.0, 0.0, 0.0), 30.0);
        camera.advance(0.5);
        camera.zoom(0.9, None);
        assert!(!camera.easing(), "it stays where the hand put it");
    }

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
