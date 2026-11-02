//! Where the camera is sent, in one place: F and R, the view pointed somewhere new
//! (a journey entered or left, a new start, Enter, expanding or shrinking back), a
//! level more or less of the journey, the camera put back where it stood (Back,
//! Esc, X again), and how near and far the wheel may take it. The camera's own
//! easing and the pointer's orbit, pan and zoom are [`crate::camera`]'s.

use bevy::prelude::*;
use bevy_egui::{egui, input::EguiWantsInput};

use crate::{
    camera::{Bounds, Lens, OrbitCamera, Reach},
    disclosure::Disclose,
    explorer::{Explorer, Return},
    nucleus::{BROWSE_REACH, Nucleus, VIEW, view_rotation},
    ui,
};

/// The graph's view in the window, in points: the camera's viewport, from under the
/// top bar to the window's foot. Set as the window is laid out; nothing is framed
/// until it is.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub(crate) struct GraphView(pub(crate) Option<egui::Rect>);

/// The free area as the camera sees it, and the share of it, across and up, that
/// leaves [`NAME_ROOM`] round its edges: what a journey's functions are framed into,
/// names and all.
#[derive(Debug, Clone, Copy)]
struct Views {
    free: Lens,
    named: Vec2,
}

/// What the nucleus was last framed for: whether it was browsed; in a journey, where
/// it starts and how much of it showed; the hand that last pointed the view; the
/// module expanded and, expanded, the journey's cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Framed {
    browsing: bool,
    start: Option<usize>,
    aim: u64,
    expanded: Option<usize>,
    disclose: Option<Disclose>,
    cursor: Option<usize>,
}

/// How much of the free area the expanded module's diameter fills, of its smaller
/// side.
const FILL: f32 = 0.75;

/// Zoomed all the way out, how much further back the camera is than where R puts
/// it: everything shown then takes a little over half the free area.
const FAR_MARGIN: f32 = 1.35;

/// The share of the free area, about its middle, a framed point may use: the rest
/// is room for its name.
const ROOM: f32 = 0.72;

/// Room left round a journey's functions for their names when it is framed, across
/// and up, in points.
const NAME_ROOM: egui::Vec2 = egui::vec2(150.0, 24.0);

/// The share of the free area, about its middle, the cursor is kept in when the view
/// pans to it.
const CURSOR_ROOM: f32 = 0.85;

/// Sends the camera where the view asks, into the free area (the graph's view less
/// the journey panel), in this order, each having the last word over the one before:
///
/// - Whenever the view is pointed somewhere new, it looks at the nucleus from far
///   enough back to see it, every open module and the journey's cursor, turning to
///   the usual view. Expanding turns there too and comes in until the expanded module
///   fills the view. Browsing, it holds the camera's angle near the usual view, so the
///   flattened map never turns edge-on, and frames the selection and what it touches
///   only then, so a click moves nothing. Expanded in a journey, it pans just enough
///   to keep the cursor in view wherever it goes; a level more or less moves nothing.
///   Otherwise a level more or less of the journey moves nothing, unless what it
///   brings in would show off the free area, and then it only pulls back.
/// - F frames the focus from the angle the camera has: browsing, the selection and
///   what it lights; in a journey, the journey and what the view is on
///   ([`Explorer::current`]); expanded, the module, filling the view, and the cursor.
///   R shrinks back what is expanded, turns back to the usual three-quarter view and
///   frames everything shown. Either has the last word over the view pointed anew.
/// - When the explorer asks for the camera back (Back to a place, Esc to browsing,
///   shrinking back), it returns to where it stood then, or with no such place,
///   browsing, takes in the whole codebase.
pub(crate) fn frame(
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<EguiWantsInput>,
    view: Res<GraphView>,
    mut explorer: ResMut<Explorer>,
    mut last: Local<Option<Framed>>,
    mut cameras: Query<(&mut OrbitCamera, &Projection)>,
) {
    let Some(view) = view.0 else {
        return;
    };
    let Ok((mut camera, projection)) = cameras.single_mut() else {
        return;
    };
    let keyed = |key| !wants.wants_any_keyboard_input() && keys.just_pressed(key);
    let (focus, whole) = (keyed(KeyCode::KeyF), keyed(KeyCode::KeyR));
    if whole && explorer.nucleus().expanded() {
        explorer.collapse();
    }
    let returning = explorer.bypass_change_detection().take_return_pose();
    let explorer = &*explorer;
    let free = ui::areas(view, explorer).0;
    let Some(lens) = lens(projection, view, free) else {
        return;
    };
    let named = (Vec2::ONE
        - Vec2::new(NAME_ROOM.x, NAME_ROOM.y) * 2.0 / Vec2::new(free.width(), free.height()))
    .max(Vec2::splat(0.5));
    let views = Views { free: lens, named };
    let nucleus = explorer.nucleus();
    let browsing = nucleus.browsing();
    let resting = |tile: usize| nucleus.resting_point(tile);

    // The orbit is held near the usual view, so the map, flattened along it, never
    // turns edge-on, nor the ring round a journey; the wheel, between a module filling
    // the view and the whole scene with room round it.
    camera.bound(Some(Bounds {
        middle: VIEW,
        reach: BROWSE_REACH,
    }));
    if let Some(reach) = reach(nucleus, &camera, lens) {
        camera.limit(reach);
    }
    let expanded = nucleus.expanded_module();
    let cursor = explorer.cursor();
    let now = Framed {
        browsing,
        start: nucleus.start().filter(|_| !browsing),
        aim: explorer.aim(),
        expanded,
        disclose: (!browsing).then(|| explorer.disclose()),
        cursor: cursor.filter(|_| expanded.is_some()),
    };
    let before = last.replace(now);
    let cursor = cursor.and_then(resting);
    if focus {
        let rotation = camera.aim_rotation();
        if nucleus.expanded() {
            fill(nucleus, cursor, &mut camera, rotation, lens);
        } else {
            let current = explorer.current().and_then(resting);
            fit(nucleus, current, &mut camera, rotation, views);
        }
    } else if whole {
        // Everything shown, as it was first seen: from the front; in a journey,
        // everything it lights.
        camera.turn_to(VIEW);
        if browsing {
            fit_all(nucleus, &mut camera, view_rotation(), lens);
        } else {
            let mut points = journey_points(nucleus, cursor, view_rotation(), views);
            let spheres = nucleus.atlas_points(view_rotation());
            points.extend(spheres.into_iter().map(|p| (p, Vec2::ONE)));
            glide(&points, &mut camera, view_rotation(), lens);
        }
    } else if before != Some(now) {
        follow_view(before, now, nucleus, cursor, &mut camera, views);
    }

    if let Some(pose) = returning {
        match pose {
            Return::To(pose) => camera.return_to(pose),
            Return::Whole if browsing => {
                let rotation = camera.aim_rotation();
                fit_all(nucleus, &mut camera, rotation, lens);
            }
            Return::Whole => {}
        }
    }
}

/// The camera's view through `projection`, filling `view`, with `free` the part of
/// it nothing lies over.
fn lens(projection: &Projection, view: egui::Rect, free: egui::Rect) -> Option<Lens> {
    let Projection::Perspective(perspective) = projection else {
        return None;
    };
    if view.width() < 1.0 || view.height() < 1.0 || free.height() < 1.0 {
        return None;
    }
    let across = |x: f32| (x - view.min.x) / view.width() * 2.0 - 1.0;
    let up = |y: f32| 1.0 - (y - view.min.y) / view.height() * 2.0;
    let (low, high) = (
        Vec2::new(across(free.min.x), up(free.max.y)),
        Vec2::new(across(free.max.x), up(free.min.y)),
    );
    Some(Lens {
        centre: (low + high) / 2.0,
        half: (high - low) / 2.0,
        ..Lens::whole(perspective.fov, view.width() / view.height())
    })
}

/// The view pointed somewhere new, the expanded view's cursor moved, or anything
/// else [`Framed`] holds changed since `before`: see [`frame`].
fn follow_view(
    before: Option<Framed>,
    now: Framed,
    nucleus: &Nucleus,
    cursor: Option<Vec3>,
    camera: &mut OrbitCamera,
    views: Views,
) {
    let lens = views.free;
    // Entering or leaving a journey, expanding or shrinking back, turns to the usual
    // view too, where the ring of modules round the nucleus keeps its order.
    let opening = before.is_none_or(|b| (b.browsing, b.expanded) != (now.browsing, now.expanded));
    let pointed = opening || before.is_some_and(|b| (b.start, b.aim) != (now.start, now.aim));
    if pointed {
        if opening {
            camera.turn_to(VIEW);
        }
        let rotation = if opening {
            view_rotation()
        } else {
            camera.aim_rotation()
        };
        if now.expanded.is_some() {
            // Expanding blows the module up to fill the view; shrinking back returns
            // to where the camera stood before (the explorer's pose).
            fill(nucleus, cursor.filter(|_| !opening), camera, rotation, lens);
        } else {
            fit(nucleus, cursor, camera, rotation, views);
        }
    } else if now.expanded.is_some() {
        if before.is_some_and(|b| b.cursor != now.cursor)
            && let Some(at) = cursor
        {
            pan_into_view(at, camera, lens);
        }
    } else {
        reveal(nucleus, cursor, camera, lens);
    }
}

/// Pans, from as far as the camera is going to be, just far enough that `at` shows
/// in the free area, clear of its edges; when it shows already, nothing moves.
fn pan_into_view(at: Vec3, camera: &mut OrbitCamera, lens: Lens) {
    let (target, distance, angles) = camera.pose().parts();
    let rotation = Quat::from_euler(EulerRot::YXZ, angles.0, angles.1, 0.0);
    let eye = target + rotation * Vec3::Z * distance;
    let Some((shows, depth)) = lens.project(rotation, eye, at) else {
        return;
    };
    let (low, high) = lens.free(CURSOR_ROOM);
    let by = shows.clamp(low, high) - shows;
    if by != Vec2::ZERO {
        let shift = rotation * Vec3::new(by.x * depth * lens.tan.x, by.y * depth * lens.tan.y, 0.0);
        camera.pan_to(target - shift);
    }
}

/// Pulls back, round wherever the camera looks, just far enough that every function
/// the journey lights, and `also`, shows in the free area; when they all show
/// already, nothing moves.
fn reveal(nucleus: &Nucleus, also: Option<Vec3>, camera: &mut OrbitCamera, lens: Lens) {
    let rotation = camera.aim_rotation();
    let mut points = nucleus.journey_points();
    points.extend(also);
    let needed = distance_to_show(&points, camera.target, rotation, lens, Vec2::ONE);
    if needed > camera.goal_distance() {
        camera.pull_back(needed);
    }
}

/// Glides to frame, turned to `rotation`, `also` (the journey's cursor, or what the
/// view is on) and in a journey every function it lights, with room for their names,
/// so they fill the free area, but never nearer than the start's module filling it;
/// browsing, the selection and every module it touches, or with none, the whole
/// codebase.
fn fit(
    nucleus: &Nucleus,
    also: Option<Vec3>,
    camera: &mut OrbitCamera,
    rotation: Quat,
    views: Views,
) {
    if nucleus.browsing() {
        let mut points = nucleus.frame_points(rotation);
        if points.is_empty() {
            points = nucleus.atlas_points(rotation);
        }
        points.extend(also);
        glide(&within(&points, ROOM), camera, rotation, views.free);
    } else {
        let points = journey_points(nucleus, also, rotation, views);
        glide(&points, camera, rotation, views.free);
    }
}

/// In a journey, every function it lights and `also`, each with the share of the
/// free area it may use, leaving room for names; and with them, when they lie so
/// close that framing them alone would come nearer, the start's module filling
/// [`FILL`] of it.
fn journey_points(
    nucleus: &Nucleus,
    also: Option<Vec3>,
    rotation: Quat,
    views: Views,
) -> Vec<(Vec3, Vec2)> {
    let mut lit = nucleus.journey_points();
    lit.extend(also);
    let mut points: Vec<(Vec3, Vec2)> = lit.into_iter().map(|p| (p, views.named)).collect();
    let module = within(&nucleus.nucleus_points(rotation), FILL);
    let least = place(&module, rotation, views.free).map_or(0.0, |(_, d)| d);
    if place(&points, rotation, views.free).is_none_or(|(_, d)| d < least) {
        points.extend(module);
    }
    points
}

/// Expanded, glides to the expanded module from just far enough back that its
/// diameter fills [`FILL`] of the free area, turned to `rotation`, and `also` (the
/// cursor) shows with it.
fn fill(
    nucleus: &Nucleus,
    also: Option<Vec3>,
    camera: &mut OrbitCamera,
    rotation: Quat,
    lens: Lens,
) {
    let mut points = nucleus.nucleus_points(rotation);
    if points.is_empty() {
        return;
    }
    points.extend(also);
    glide(&within(&points, FILL), camera, rotation, lens);
}

/// Glides to take in everything shown from where the camera turns to.
fn fit_all(nucleus: &Nucleus, camera: &mut OrbitCamera, rotation: Quat, lens: Lens) {
    let points = within(&nucleus.atlas_points(rotation), ROOM);
    glide(&points, camera, rotation, lens);
}

/// `points`, each to show within `share` of the free area about its middle.
fn within(points: &[Vec3], share: f32) -> Vec<(Vec3, Vec2)> {
    points.iter().map(|&p| (p, Vec2::splat(share))).collect()
}

/// Glides to show every one of `points` within its share of the free area, turned
/// to `rotation`, centred in it.
fn glide(points: &[(Vec3, Vec2)], camera: &mut OrbitCamera, rotation: Quat, lens: Lens) {
    if let Some((middle, distance)) = place(points, rotation, lens) {
        camera.glide_to(middle - lens.shift(rotation) * distance, distance);
    }
}

/// How near and far the wheel may take the camera turned as it is, and what it keeps
/// out of and in view: out, until everything shown takes a little over half the free
/// area, held in it on the way.
fn reach(nucleus: &Nucleus, camera: &OrbitCamera, lens: Lens) -> Option<Reach> {
    let spheres: Vec<(Vec3, f32)> = nucleus
        .balls()
        .filter(|(_, ball)| nucleus.shows_body(ball.body))
        .map(|(_, ball)| (ball.center, ball.radius))
        .collect();
    let near = spheres.iter().map(|&(_, radius)| radius).reduce(f32::min)?;
    let rotation = camera.aim_rotation();
    let map = nucleus.atlas_points(rotation);
    let (home, fits) = place(&within(&map, ROOM), rotation, lens)?;
    let half = |axis: Vec3| {
        map.iter()
            .map(|p| (p - home).dot(axis).abs())
            .fold(0.0, f32::max)
    };
    Some(Reach {
        near,
        far: (fits * FAR_MARGIN).max(near),
        home,
        extent: Vec2::new(half(rotation * Vec3::X), half(rotation * Vec3::Y)),
        spheres,
        lens,
    })
}

/// The closest a framing brings the camera to the middle of what it frames.
const NEAREST_FRAMING: f32 = 1.0;

/// The middle of `points` across the view turned to `rotation`, and how far back
/// from it the camera must be, looking past it as [`Lens::shift`] says, for each of
/// them to show within its share of the free area about its middle, across and up.
fn place(points: &[(Vec3, Vec2)], rotation: Quat, lens: Lens) -> Option<(Vec3, f32)> {
    if points.is_empty() {
        return None;
    }
    let (right, up, back) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z);
    let middle_along = |axis: Vec3| {
        let (low, high) = points
            .iter()
            .fold((f32::MAX, f32::MIN), |(low, high), (p, _)| {
                (low.min(p.dot(axis)), high.max(p.dot(axis)))
            });
        axis * (low + high) / 2.0
    };
    let middle = middle_along(right) + middle_along(up) + middle_along(back);
    let (centre, tan) = (lens.centre, lens.tan);
    let distance = points
        .iter()
        .map(|&(p, share)| {
            let offset = p - middle;
            // A point this far towards the camera needs the camera that much further.
            let toward = offset.dot(back);
            let across = (offset.dot(right) + centre.x * tan.x * toward).abs()
                / (lens.half.x * share.x * tan.x);
            let high = (offset.dot(up) + centre.y * tan.y * toward).abs()
                / (lens.half.y * share.y * tan.y);
            across.max(high) + toward
        })
        .fold(NEAREST_FRAMING, f32::max);
    Some((middle, distance))
}

/// How far back a camera turned to `rotation` and looking at `looking_at` must be
/// for every one of `points` to show within `share` of the free area about its
/// middle, or as near it as backing off can bring them.
fn distance_to_show(
    points: &[Vec3],
    looking_at: Vec3,
    rotation: Quat,
    lens: Lens,
    share: Vec2,
) -> f32 {
    let (low, high) = (
        lens.centre - lens.half * share,
        lens.centre + lens.half * share,
    );
    // Backing off brings everything towards the view's middle, so a side of the free
    // area past the middle is out of reach.
    let (low, high) = (low.min(Vec2::splat(-0.05)), high.max(Vec2::splat(0.05)));
    let (right, up, back) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z);
    points
        .iter()
        .map(|&p| {
            let offset = p - looking_at;
            let need = |along: f32, low: f32, high: f32, tan: f32| {
                along / (if along >= 0.0 { high } else { low } * tan)
            };
            need(offset.dot(right), low.x, high.x, lens.tan.x).max(need(
                offset.dot(up),
                low.y,
                high.y,
                lens.tan.y,
            )) + offset.dot(back)
        })
        .fold(0.0, f32::max)
}

/// The module whose middle shows nearest the free area's middle, of those shown:
/// what X expands with nothing selected.
pub(crate) fn module_nearest_middle(
    nucleus: &Nucleus,
    camera: &Camera,
    view: &GlobalTransform,
    free: egui::Rect,
) -> Option<usize> {
    let middle = Vec2::new(free.center().x, free.center().y);
    nucleus
        .balls()
        .filter(|(_, ball)| nucleus.shows_body(ball.body))
        .filter_map(|(cluster, ball)| {
            let at = camera.world_to_viewport(view, ball.center).ok()?;
            free.contains(egui::pos2(at.x, at.y))
                .then(|| (cluster, at.distance_squared(middle)))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(cluster, _)| cluster)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The view of a 1600 by 976 window, its foot from `free_to` down under a panel.
    fn lens(free_to: f32) -> Lens {
        let view = egui::Rect::from_min_max(egui::pos2(0.0, 24.0), egui::pos2(1600.0, 1000.0));
        let free = egui::Rect::from_min_max(view.min, egui::pos2(1600.0, free_to));
        super::lens(
            &Projection::Perspective(PerspectiveProjection::default()),
            view,
            free,
        )
        .unwrap()
    }

    /// Where `point` shows, in window points, seen from where `place` puts the camera.
    fn shows(points: &[Vec3], lens: Lens, share: f32, point: Vec3) -> Vec2 {
        let rotation = view_rotation();
        let (middle, distance) = place(&within(points, share), rotation, lens).unwrap();
        let eye = middle - lens.shift(rotation) * distance + rotation * Vec3::Z * distance;
        let (at, _) = lens.project(rotation, eye, point).unwrap();
        Vec2::new(
            f32::midpoint(at.x, 1.0) * 1600.0,
            24.0 + (1.0 - at.y) / 2.0 * 976.0,
        )
    }

    #[test]
    fn framing_centres_what_it_frames_in_the_free_area_and_fills_its_share() {
        let points = [Vec3::new(-10.0, -6.0, 0.0), Vec3::new(10.0, 6.0, 0.0)];
        let ball = |c: Vec3, r: f32| {
            let rotation = view_rotation();
            let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
            [c + right * r, c - right * r, c + up * r, c - up * r]
        };
        let rims: Vec<Vec3> = points.iter().flat_map(|&p| ball(p, 2.0)).collect();
        for free_to in [1000.0, 760.0] {
            let lens = lens(free_to);
            let spots: Vec<Vec2> = rims.iter().map(|&p| shows(&rims, lens, ROOM, p)).collect();
            let (low, high) = spots.iter().fold(
                (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
                |(l, h), &s| (l.min(s), h.max(s)),
            );
            let middle = (low + high) / 2.0;
            let free_middle = f32::midpoint(24.0, free_to);
            assert!(
                (middle.x - 800.0).abs() < 25.0 && (middle.y - free_middle).abs() < 25.0,
                "centred in the free area above {free_to}: {middle}"
            );
            let used = ((high.y - low.y) / (free_to - 24.0)).max((high.x - low.x) / 1600.0);
            assert!(
                (ROOM - 0.08..=ROOM + 0.02).contains(&used),
                "it uses its share of the free area: {used}"
            );
            assert!(high.y <= free_to, "nothing under the panel: {high}");
        }
    }

    #[test]
    fn pulls_back_only_as_far_as_what_it_must_show() {
        let lens = lens(1000.0);
        let head_on = Quat::IDENTITY;
        let near = [Vec3::new(3.0, 2.0, 0.0), Vec3::new(-4.0, -1.0, 0.0)];
        let far = [Vec3::new(0.0, 40.0, 0.0)];
        let needed = distance_to_show(&far, Vec3::ZERO, head_on, lens, Vec2::splat(ROOM));
        assert!(
            distance_to_show(&near, Vec3::ZERO, head_on, lens, Vec2::splat(ROOM)) < needed,
            "what is close needs less"
        );
        let half_height = needed * lens.tan.y * ROOM;
        assert!(
            (half_height - 40.0).abs() < 1e-2,
            "far enough back that it shows, with room: {needed}"
        );
        let wide = [Vec3::new(40.0, 0.0, 0.0)];
        assert!(
            distance_to_show(&wide, Vec3::ZERO, head_on, lens, Vec2::splat(ROOM)) < needed,
            "the view is wider than it is tall"
        );
    }
}
