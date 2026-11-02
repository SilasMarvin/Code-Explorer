//! Where the camera is sent, in one place: F and R, the view pointed somewhere new
//! (a journey entered or left, a new start, Enter, expanding or shrinking back), a
//! level more or less of the journey, the camera put back where it stood (Back,
//! Esc, X again), and how near and far the wheel may take it. The camera's own
//! easing and the pointer's orbit, pan and zoom are [`crate::camera`]'s.

use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;

use crate::{
    camera::{Bounds, OrbitCamera, Reach},
    disclosure::Disclose,
    explorer::{Explorer, Return},
    nucleus::{BROWSE_REACH, Nucleus, VIEW, view_rotation},
};

/// What the nucleus was last framed for: whether it was browsed; in a journey, where
/// it starts, the function the view centres on and how much of it showed; the hand
/// that last pointed the view; and the module expanded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Framed {
    browsing: bool,
    start: Option<usize>,
    centre: Option<usize>,
    aim: u64,
    expanded: Option<usize>,
    disclose: Option<Disclose>,
}

/// How much of the view the expanded module's diameter fills, of the view's
/// smaller side.
const FILL: f32 = 0.75;

/// Zoomed all the way out, how much further back the camera is than where the
/// whole scene just fits with room for names: the scene then takes a little over
/// half the view.
const FAR_MARGIN: f32 = 1.35;

/// Sends the camera where the view asks, in this order, each having the last word
/// over the one before:
///
/// - F frames the focus from the angle the camera has: browsing, the selection and
///   what it lights; in a journey, the journey and what the view is on
///   ([`Explorer::current`]); expanded, the module, filling the view. R turns back
///   to the usual three-quarter view and frames everything shown.
/// - Whenever the view is pointed somewhere new, it looks at the nucleus from far
///   enough back to see it, every open module and the journey's cursor, turning to
///   the usual view. Expanding turns there too and comes in until the expanded module
///   fills the view. Browsing, it holds the camera's angle near the usual view, so the
///   flattened map never turns edge-on, and frames the selection and what it touches
///   only then, so a click moves nothing. Expanded in a journey, it glides across to
///   the cursor wherever the cursor goes, from as far as it was. A level more or less
///   of the journey moves nothing, unless what it brings in would show off screen,
///   and then it only pulls back.
/// - When the explorer asks for the camera back (Back to a place, Esc to browsing,
///   shrinking back), it returns to where it stood then, or with no such place,
///   browsing, takes in the whole codebase.
pub(crate) fn frame(
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<EguiWantsInput>,
    mut explorer: ResMut<Explorer>,
    mut last: Local<Option<Framed>>,
    mut cameras: Query<(&mut OrbitCamera, &Projection)>,
) {
    let returning = explorer.bypass_change_detection().take_return_pose();
    let explorer = &*explorer;
    let nucleus = explorer.nucleus();
    let browsing = nucleus.browsing();
    let current = explorer.current().and_then(|t| explorer.tile_point(t));
    if !wants.wants_any_keyboard_input() {
        for (mut camera, projection) in &mut cameras {
            frame_keys(&keys, nucleus, current, &mut camera, projection);
        }
    }

    // The orbit is held near the usual view, so the map, flattened along it, never
    // turns edge-on, nor the ring round a journey; the wheel, between a module filling
    // the view and the whole scene with room round it.
    for (mut camera, projection) in &mut cameras {
        camera.bound(Some(Bounds {
            middle: VIEW,
            reach: BROWSE_REACH,
        }));
        if let Some(reach) = reach(nucleus, &camera, projection) {
            camera.limit(reach);
        }
    }
    let now = Framed {
        browsing,
        start: nucleus.start().filter(|_| !browsing),
        centre: nucleus.centre(),
        aim: explorer.aim(),
        expanded: nucleus.expanded_module(),
        disclose: (!browsing).then(|| explorer.disclose()),
    };
    let before = last.replace(now);
    if before != Some(now) {
        let cursor = explorer.cursor().and_then(|t| explorer.tile_point(t));
        for (mut camera, projection) in &mut cameras {
            follow_view(before, now, nucleus, cursor, &mut camera, projection);
        }
    }

    if let Some(pose) = returning {
        for (mut camera, projection) in &mut cameras {
            match pose {
                Return::To(pose) => camera.return_to(pose),
                Return::Whole if browsing => {
                    let rotation = camera.aim_rotation();
                    fit_all(nucleus, &mut camera, rotation, projection);
                }
                Return::Whole => {}
            }
        }
    }
}

/// F and R: see [`frame`].
fn frame_keys(
    keys: &ButtonInput<KeyCode>,
    nucleus: &Nucleus,
    current: Option<Vec3>,
    camera: &mut OrbitCamera,
    projection: &Projection,
) {
    if keys.just_pressed(KeyCode::KeyF) {
        let rotation = camera.aim_rotation();
        if nucleus.expanded() {
            fill(nucleus, camera, rotation, projection);
        } else {
            fit(nucleus, current, camera, rotation, projection);
        }
    }
    if keys.just_pressed(KeyCode::KeyR) {
        // Everything shown, as it was first seen: from the front.
        camera.turn_to(VIEW);
        fit_all(nucleus, camera, view_rotation(), projection);
    }
}

/// The view pointed somewhere new, the expanded view recentred, or anything else
/// [`Framed`] holds changed since `before`: see [`frame`].
fn follow_view(
    before: Option<Framed>,
    now: Framed,
    nucleus: &Nucleus,
    cursor: Option<Vec3>,
    camera: &mut OrbitCamera,
    projection: &Projection,
) {
    // Entering or leaving a journey, expanding or shrinking back, turns to the usual
    // view too, where the ring of modules round the nucleus keeps its order.
    let opening = before.is_none_or(|b| (b.browsing, b.expanded) != (now.browsing, now.expanded));
    let pointed = opening || before.is_some_and(|b| (b.start, b.aim) != (now.start, now.aim));
    let recentred = before.is_some_and(|b| b.centre != now.centre) && now.expanded.is_some();
    if pointed {
        if opening {
            camera.turn_to(VIEW);
        }
        let rotation = if opening {
            view_rotation()
        } else {
            camera.aim_rotation()
        };
        if opening && now.expanded.is_some() {
            // Expanding blows the module up to fill the view; shrinking back returns
            // to where the camera stood before (the explorer's pose).
            fill(nucleus, camera, rotation, projection);
        } else {
            fit(nucleus, cursor, camera, rotation, projection);
        }
    } else if recentred {
        // Only when the cursor would show off the view, or near its edge.
        let shows = cursor.is_some_and(|at| {
            let Projection::Perspective(perspective) = projection else {
                return false;
            };
            let rotation = camera.aim_rotation();
            fit_distance(&[at], camera.target, rotation, perspective) <= camera.goal_distance()
        });
        if !shows {
            camera.pan_to(nucleus.frame_target());
        }
    } else {
        reveal(nucleus, cursor, camera, projection);
    }
}

/// Pulls back, round wherever the camera looks, just far enough that everything the
/// nucleus shows, and `also`, is on screen; when it all shows already, nothing moves.
fn reveal(
    nucleus: &Nucleus,
    also: Option<Vec3>,
    camera: &mut OrbitCamera,
    projection: &Projection,
) {
    let Projection::Perspective(perspective) = projection else {
        return;
    };
    let rotation = camera.aim_rotation();
    let mut points = nucleus.frame_points(rotation);
    points.extend(also);
    let needed = fit_distance(&points, camera.target, rotation, perspective);
    if needed > camera.goal_distance() {
        camera.pull_back(needed);
    }
}

/// Glides to the nucleus from far enough back, turned to `rotation`, to see it all,
/// and `also` (the journey's cursor, or what the view is on) wherever it sits;
/// browsing, to the selection and every module it touches, or with none, the whole
/// codebase.
fn fit(
    nucleus: &Nucleus,
    also: Option<Vec3>,
    camera: &mut OrbitCamera,
    rotation: Quat,
    projection: &Projection,
) {
    let mut points = nucleus.frame_points(rotation);
    if points.is_empty() {
        points = nucleus.atlas_points(rotation);
    }
    points.extend(also);
    glide(
        &points,
        nucleus.frame_target(),
        camera,
        rotation,
        projection,
    );
}

/// Expanded, glides to the expanded module (in a journey's start module, to the
/// function the view centres on) from just far enough back that the module's
/// diameter fills [`FILL`] of the view, turned to `rotation`.
fn fill(nucleus: &Nucleus, camera: &mut OrbitCamera, rotation: Quat, projection: &Projection) {
    let Projection::Perspective(perspective) = projection else {
        return;
    };
    let points = nucleus.nucleus_points(rotation);
    if points.is_empty() {
        return;
    }
    let target = nucleus.frame_target();
    camera.glide_to(
        target,
        distance_for(&points, target, rotation, perspective, FILL),
    );
}

/// Glides to take in everything shown from where the camera turns to.
fn fit_all(nucleus: &Nucleus, camera: &mut OrbitCamera, rotation: Quat, projection: &Projection) {
    let points = nucleus.atlas_points(rotation);
    glide(&points, middle(&points), camera, rotation, projection);
}

/// The middle of `points`.
fn middle(points: &[Vec3]) -> Vec3 {
    #[expect(clippy::cast_precision_loss, reason = "a point count")]
    let count = points.len().max(1) as f32;
    points.iter().copied().sum::<Vec3>() / count
}

/// How near and far the wheel may take the camera turned as it is: in, until the
/// smallest sphere shown fills the view; out, until everything shown takes a little
/// over half of it.
fn reach(nucleus: &Nucleus, camera: &OrbitCamera, projection: &Projection) -> Option<Reach> {
    let Projection::Perspective(perspective) = projection else {
        return None;
    };
    let smallest = nucleus
        .balls()
        .filter(|(_, ball)| nucleus.shows_body(ball.body))
        .map(|(_, ball)| ball.radius)
        .reduce(f32::min)?;
    let half = (perspective.fov / 2.0).tan() * perspective.aspect_ratio.min(1.0);
    let rotation = camera.aim_rotation();
    let points = nucleus.atlas_points(rotation);
    let far = fit_distance(&points, middle(&points), rotation, perspective) * FAR_MARGIN;
    let near = smallest / half;
    Some(Reach {
        near,
        far: far.max(near),
    })
}

/// Glides to look at `target`, turned to `rotation`, from far enough back to see
/// every one of `points`.
fn glide(
    points: &[Vec3],
    target: Vec3,
    camera: &mut OrbitCamera,
    rotation: Quat,
    projection: &Projection,
) {
    let Projection::Perspective(perspective) = projection else {
        return;
    };
    camera.glide_to(target, fit_distance(points, target, rotation, perspective));
}

/// The closest a framing brings the camera to what it looks at.
const MIN_ORBIT_DISTANCE: f32 = 25.0;

/// The share of the half-view a framed point may use: the rest is room for its name.
const ROOM: f32 = 0.72;

/// How far back a camera turned to `rotation` and looking at `center` must be for
/// every one of `points` to sit inside the middle of its view, clear of the edges.
fn fit_distance(
    points: &[Vec3],
    center: Vec3,
    rotation: Quat,
    perspective: &PerspectiveProjection,
) -> f32 {
    distance_for(points, center, rotation, perspective, ROOM).max(MIN_ORBIT_DISTANCE)
}

/// How far back a camera turned to `rotation` and looking at `center` must be for
/// every one of `points` to sit within `share` of the half-view from its middle.
fn distance_for(
    points: &[Vec3],
    center: Vec3,
    rotation: Quat,
    perspective: &PerspectiveProjection,
    share: f32,
) -> f32 {
    let tan_y = (perspective.fov / 2.0).tan() * share;
    let tan_x = tan_y * perspective.aspect_ratio;
    let (right, up, back) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z);
    points
        .iter()
        .map(|&p| {
            let offset = p - center;
            // A point this far towards the camera needs the camera that much further.
            let toward = offset.dot(back);
            (offset.dot(right).abs() / tan_x).max(offset.dot(up).abs() / tan_y) + toward
        })
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulls_back_only_as_far_as_what_it_must_show() {
        let view = PerspectiveProjection {
            aspect_ratio: 1.6,
            ..default()
        };
        let head_on = Quat::IDENTITY;
        let near = [Vec3::new(3.0, 2.0, 0.0), Vec3::new(-4.0, -1.0, 0.0)];
        assert!(
            (fit_distance(&near, Vec3::ZERO, head_on, &view) - MIN_ORBIT_DISTANCE).abs() < 1e-3,
            "what is close needs no more than the closest orbit"
        );
        let far = [Vec3::new(0.0, 40.0, 0.0)];
        let needed = fit_distance(&far, Vec3::ZERO, head_on, &view);
        let half_height = needed * (view.fov / 2.0).tan();
        assert!(
            needed > MIN_ORBIT_DISTANCE && half_height > 40.0,
            "far enough back that it shows, with room: {needed}"
        );
        let wide = [Vec3::new(40.0, 0.0, 0.0)];
        assert!(
            fit_distance(&wide, Vec3::ZERO, head_on, &view) < needed,
            "the view is wider than it is tall"
        );
    }
}
