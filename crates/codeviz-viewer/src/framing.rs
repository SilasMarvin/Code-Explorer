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
    nucleus::{BROWSE_REACH, Ball, Nucleus, VIEW, view_rotation},
    sizes, ui,
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
    /// The share that leaves [`DOT_ROOM`] round its edges, for a dot to show whole.
    dots: Vec2,
    /// The free area's width and height, in points.
    size: Vec2,
}

impl Views {
    /// The free area `free`, as the camera sees it through `lens`.
    fn of(lens: Lens, free: egui::Rect) -> Self {
        let size = Vec2::new(free.width(), free.height());
        let short_of = |room: Vec2| (Vec2::ONE - room * 2.0 / size).max(Vec2::splat(0.5));
        Self {
            free: lens,
            named: short_of(Vec2::new(NAME_ROOM.x, NAME_ROOM.y)),
            dots: short_of(Vec2::splat(DOT_ROOM)),
            size,
        }
    }
}

/// What the nucleus was last framed for: whether it was browsed; in a journey, where
/// it starts and how much of it showed (its depth and the folds opened by hand); the
/// hand that last pointed the view; the module expanded and, expanded, the journey's
/// cursor; and the graph's view in the window and the free area in it, in whole
/// points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Framed {
    view: [i32; 4],
    free: [i32; 4],
    browsing: bool,
    start: Option<usize>,
    aim: u64,
    expanded: Option<usize>,
    disclose: Option<(Disclose, usize)>,
    cursor: Option<usize>,
    /// In a journey, the function set aside: one a search put off the journey's
    /// spheres draws the camera back to show it.
    aside: Option<usize>,
}

/// A journey is never framed nearer than its start's module filling this much of the
/// free area's smaller side.
const START_FILL: f32 = 0.75;

/// Zoomed all the way out, how much further back the camera is than where R puts
/// it: everything shown then takes a little over half the free area.
const FAR_MARGIN: f32 = 1.35;

/// The share of the free area, about its middle, a framed point may use: the rest
/// is room for its name.
const ROOM: f32 = 0.72;

/// Room left round a journey's functions for their names when it is framed, across
/// and up, in points.
const NAME_ROOM: egui::Vec2 = egui::vec2(150.0, 24.0);

/// Room left round the free area's edges for a dot to show whole, in points.
const DOT_ROOM: f32 = 10.0;

/// How much further back than the camera is something must need it to be, as a share
/// of its distance, for a level more or less to draw it back: rounding, not leeway.
const REVEAL_SLACK: f32 = 0.005;

/// The share of the free area, about its middle, the cursor is kept in when the view
/// pans to it.
const CURSOR_ROOM: f32 = 0.85;

/// Sends the camera where the view asks, into the free area (the graph's view less
/// the journey panel), in this order, each having the last word over the one before:
///
/// - Whenever the view is pointed somewhere new, it turns to the usual view and
///   frames it: in a journey, the start, the cursor, every
///   function the journey lights and the spheres they lie in, whole, with room for
///   their names; browsing, the selection and every module it touches, or the whole
///   codebase. Expanding turns there too and comes in until the expanded module takes
///   the share of the view its functions need ([`fill`]), as does a cursor, or a
///   pick, that takes the expansion to another module. Browsing,
///   it holds the camera's angle near the usual view, so the flattened map never
///   turns edge-on, and frames the selection only then, so a click moves nothing.
///   Expanded in a journey, it pans just enough to keep the cursor in view within the
///   module, and a level more or less moves nothing; otherwise a level more or less
///   moves nothing unless something the journey lights, or a sphere it comes to run
///   through (any, when the panel grows), would show off the free area, and then it
///   frames the journey afresh, coming no nearer.
/// - F, or the panel's Fit, frames the focus from the angle the camera has:
///   browsing, the selection and what it lights; in a journey, the journey and what
///   the view is on ([`Explorer::current`]); expanded, the module, as expanding frames it,
///   and the cursor. R shrinks back what is expanded, turns back to the usual
///   three-quarter view and frames everything shown: in a journey, its spheres and
///   what it lights. Either has the last word over the view pointed anew.
/// - When the explorer asks for the camera back (Back to a place, Esc to browsing,
///   shrinking back), it returns to where it stood then, or with no such place,
///   browsing, takes in the whole codebase.
/// - A window sized anew frames the view again as it was last framed ([`Shot`]),
///   unless the hands moved the camera since or it went back to where it stood;
///   then it stays, what it looks at in the middle.
pub(crate) fn frame(
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<EguiWantsInput>,
    view: Res<GraphView>,
    mut explorer: ResMut<Explorer>,
    mut last: Local<(Option<Framed>, Option<Shot>)>,
    mut opened: Local<Vec<usize>>,
    mut cameras: Query<(&mut OrbitCamera, &Projection)>,
) {
    let Some(view) = view.0 else {
        return;
    };
    let Ok((mut camera, projection)) = cameras.single_mut() else {
        return;
    };
    let keyed = |key| !wants.wants_any_keyboard_input() && keys.just_pressed(key);
    // The panel's Fit frames as F does.
    let asked = explorer.bypass_change_detection().take_frame_ask();
    let (focus, whole) = (keyed(KeyCode::KeyF) || asked, keyed(KeyCode::KeyR));
    if whole && explorer.nucleus().expanded() {
        explorer.collapse();
    }
    if focus {
        explorer.frame_focus();
    }
    let returning = explorer.bypass_change_detection().take_return_pose();
    let explorer = &*explorer;
    let free = ui::areas(view, explorer).0;
    let Some(lens) = lens(projection, view, free) else {
        return;
    };
    let views = Views::of(lens, free);
    // How long each function's name is, which sets how big a module expands.
    let chars = |tile: usize| {
        explorer
            .tile_unit(tile)
            .map_or(0, |unit| explorer.function_name(unit).chars().count())
    };
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
    #[expect(clippy::cast_possible_truncation, reason = "window points")]
    let points = |r: egui::Rect| [r.min.x, r.min.y, r.max.x, r.max.y].map(|v| v.round() as i32);
    let now = Framed {
        view: points(view),
        free: points(free),
        browsing,
        start: nucleus.start().filter(|_| !browsing),
        aim: explorer.aim(),
        expanded,
        disclose: (!browsing).then(|| (explorer.disclose(), explorer.folds_opened())),
        cursor: cursor.filter(|_| expanded.is_some()),
        aside: explorer.aside(),
    };
    let (last, shot) = &mut *last;
    let before = last.replace(now);
    let came = came(
        nucleus,
        &mut opened,
        before.is_some_and(|b| b.free != now.free),
    );
    let cursor = cursor.and_then(resting);
    let current = explorer.current().and_then(resting);
    let taken = if focus {
        Some(Shot::Pointed { opening: false })
    } else if whole {
        Some(Shot::Whole)
    } else if before != Some(now) {
        let aside = explorer.aside().and_then(resting);
        follow_view(
            (before, now),
            nucleus,
            (cursor, aside),
            &came,
            &mut camera,
            views,
        )
    } else {
        None
    };
    // A window sized anew frames again as the view was last framed, unless the
    // hands have moved the camera since; then what it looks at stays in the middle.
    // Asked for the whole codebase back, browsing, it frames that.
    let resized = before.is_some_and(|b| b.view != now.view);
    let taken = match returning {
        Some(Return::Whole) if browsing => Some(Shot::Whole),
        _ => taken.or_else(|| (*shot).filter(|_| resized && camera.framed())),
    };
    if let Some(taken) = taken {
        take(taken, nucleus, current, &mut camera, (views, &chars));
        *shot = Some(match taken {
            Shot::Pointed { .. } => Shot::Pointed { opening: false },
            Shot::Whole => Shot::Whole,
        });
    }
    if let Some(Return::To(pose)) = returning {
        camera.return_to(pose);
    }
}

/// The modules the journey has come to run through since the frame before, `opened`
/// then; all of them when the panel has grown or shrunk since (`resized`).
fn came(nucleus: &Nucleus, opened: &mut Vec<usize>, resized: bool) -> Vec<usize> {
    let modules = if nucleus.browsing() {
        Vec::new()
    } else {
        nucleus.open_modules()
    };
    let came = modules
        .iter()
        .copied()
        .filter(|m| resized || !opened.contains(m))
        .collect();
    *opened = modules;
    came
}

/// How the view was framed: as R does, or as F does and the view pointed somewhere
/// new (`opening` when it turns to the usual view on the way, as entering or leaving a
/// journey and expanding or shrinking back do).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Shot {
    Whole,
    Pointed { opening: bool },
}

/// Frames the view as `shot` says, with what the view is on (`current`) where it
/// rests.
fn take(
    shot: Shot,
    nucleus: &Nucleus,
    current: Option<Vec3>,
    camera: &mut OrbitCamera,
    (views, chars): (Views, &dyn Fn(usize) -> usize),
) {
    let lens = views.free;
    match shot {
        // Everything shown, as it was first seen: from the front; in a journey,
        // everything it lights.
        Shot::Whole => {
            camera.turn_to(VIEW);
            if nucleus.browsing() {
                fit_all(nucleus, camera, view_rotation(), lens);
            } else {
                let mut points = journey_points(nucleus, &[current], view_rotation(), views);
                let spheres = nucleus.atlas_points(view_rotation());
                points.extend(spheres.into_iter().map(|p| (p, Vec2::ONE)));
                glide(&points, camera, view_rotation(), lens);
            }
        }
        // Entering or leaving a journey, expanding or shrinking back, turns to the
        // usual view too, where the ring of modules round the nucleus keeps its order.
        Shot::Pointed { opening } => {
            let rotation = if opening {
                camera.turn_to(VIEW);
                view_rotation()
            } else {
                camera.aim_rotation()
            };
            if nucleus.expanded() {
                // Expanding blows the module up as its functions need; shrinking back
                // returns to where the camera stood before (the explorer's pose).
                let current = current.filter(|_| !opening);
                fill(nucleus, current, camera, rotation, (views, chars));
            } else {
                fit(nucleus, current, camera, rotation, views);
            }
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
/// else [`Framed`] holds changed since `before`, the journey now running through
/// the modules `came` too: see [`frame`]. Returns the framing the view pointed
/// somewhere new takes.
fn follow_view(
    (before, now): (Option<Framed>, Framed),
    nucleus: &Nucleus,
    (cursor, aside): (Option<Vec3>, Option<Vec3>),
    came: &[usize],
    camera: &mut OrbitCamera,
    views: Views,
) -> Option<Shot> {
    let lens = views.free;
    let opening = before.is_none_or(|b| (b.browsing, b.expanded) != (now.browsing, now.expanded));
    if opening || before.is_some_and(|b| (b.start, b.aim) != (now.start, now.aim)) {
        return Some(Shot::Pointed { opening });
    }
    if now.expanded.is_some() {
        if before.is_some_and(|b| b.cursor != now.cursor)
            && let Some(at) = cursor
        {
            pan_into_view(at, camera, lens);
        }
    } else {
        reveal(nucleus, &[cursor, aside], came, camera, views);
    }
    None
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

/// When something the journey lights, or `also` (its cursor, the function set
/// aside), would show off the free area, or not whole, or a sphere of the modules it
/// `came` to run through, or its name, frames the journey afresh, but never nearer
/// than the camera is; when it all shows already, nothing moves.
fn reveal(
    nucleus: &Nucleus,
    also: &[Option<Vec3>],
    came: &[usize],
    camera: &mut OrbitCamera,
    views: Views,
) {
    let rotation = camera.aim_rotation();
    let mut points = nucleus.journey_points();
    points.extend(also.iter().flatten());
    points.extend(nucleus.rims(rotation, came));
    // About the middle the camera frames now.
    let middle = camera.target + views.free.shift(rotation) * camera.goal_distance();
    let show = |points: &[Vec3], share: Vec2| {
        let points: Vec<(Vec3, Vec2)> = points.iter().map(|&p| (p, share)).collect();
        distance_from(&points, middle, rotation, views.free)
    };
    let needed = show(&points, views.dots).max(show(&nucleus.tops(rotation, came), views.named));
    // What a framing put on the edge of its share stays there.
    if needed <= camera.goal_distance() * (1.0 + REVEAL_SLACK) {
        return;
    }
    let framed = journey_points(nucleus, also, rotation, views);
    if let Some((middle, distance)) = place(&framed, rotation, views.free) {
        let distance = distance.max(camera.goal_distance());
        camera.glide_to(middle - views.free.shift(rotation) * distance, distance);
    }
}

/// Glides to frame, turned to `rotation`, `also` (the journey's cursor, or what the
/// view is on) and in a journey every function it lights, with room for their names,
/// so they fill the free area, but never nearer than the start's module filling
/// [`START_FILL`] of it;
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
        let points = journey_points(nucleus, &[also], rotation, views);
        glide(&points, camera, rotation, views.free);
    }
}

/// In a journey, every function it lights, `also` and the spheres they lie in,
/// whole, each with the share of the free area it may use, leaving room for names
/// by the functions and above the spheres; and with them, when they lie so close
/// that framing them alone would come nearer, the start's module filling
/// [`START_FILL`] of it.
fn journey_points(
    nucleus: &Nucleus,
    also: &[Option<Vec3>],
    rotation: Quat,
    views: Views,
) -> Vec<(Vec3, Vec2)> {
    let modules = nucleus.open_modules();
    let mut lit = nucleus.journey_points();
    lit.extend(also.iter().flatten());
    // The modules' names sit above their spheres' tops.
    lit.extend(nucleus.tops(rotation, &modules));
    let mut points: Vec<(Vec3, Vec2)> = lit.into_iter().map(|p| (p, views.named)).collect();
    let rims = nucleus.rims(rotation, &modules);
    points.extend(rims.into_iter().map(|p| (p, views.dots)));
    let module = within(&nucleus.nucleus_points(rotation), START_FILL);
    let least = place(&module, rotation, views.free).map_or(0.0, |(_, d)| d);
    if place(&points, rotation, views.free).is_none_or(|(_, d)| d < least) {
        points.extend(module);
    }
    points
}

/// Expanded, glides to the expanded module, turned to `rotation`, from just far
/// enough back that its diameter takes the share of the free area's smaller side
/// its functions need to sit comfortably, their names `chars` long
/// ([`sizes::expanded_share`]), and `also` (the cursor) shows with it: in a journey
/// and browsing alike, its neighbours left where they fall, carried by the labels
/// where calls leave the screen. Of the shares near that, it takes the nearest that
/// leaves no sphere the journey shows half under the panel.
fn fill(
    nucleus: &Nucleus,
    also: Option<Vec3>,
    camera: &mut OrbitCamera,
    rotation: Quat,
    (views, chars): (Views, &dyn Fn(usize) -> usize),
) {
    let (Some(module), Some((radius, dots))) =
        (nucleus.expanded_module(), nucleus.expanded_layout())
    else {
        return;
    };
    // Where its functions would show, in points, at the least share, and so the
    // share they need.
    let side = views.size.min_element();
    let (least, most) = sizes::EXPANDED_SHARE;
    let scale = least * side / (2.0 * radius).max(f32::EPSILON);
    let shown: Vec<(Vec2, usize)> = dots.iter().map(|&(t, at)| (at * scale, chars(t))).collect();
    let wanted = sizes::expanded_share(least, sizes::comfort(&shown));
    let framed = |share: f32| {
        let share = Vec2::splat(share * side) / views.size;
        let alone: Vec<(Vec3, Vec2)> = nucleus
            .nucleus_points(rotation)
            .into_iter()
            .chain(also)
            .map(|p| (p, share))
            .collect();
        place(&alone, rotation, views.free)
    };
    let others: Vec<usize> = if nucleus.browsing() {
        Vec::new()
    } else {
        let mut open = nucleus.open_modules();
        open.retain(|&m| m != module);
        open
    };
    let cut = |(middle, distance): (Vec3, f32)| {
        let target = middle - views.free.shift(rotation) * distance;
        under_panel(
            nucleus.balls_of(&others),
            target,
            distance,
            rotation,
            views.free,
        )
    };
    // The share wanted, else the nearest to it a step at a time either way.
    let shares = (0..=PANEL_STEPS).flat_map(|i| {
        #[expect(clippy::cast_precision_loss, reason = "a step count")]
        let by = i as f32 * (most - least) / PANEL_STEPS as f32;
        [wanted - by, wanted + by]
    });
    let chosen = shares
        .filter(|share| (least..=most).contains(share))
        .filter_map(framed)
        .find(|&framing| !cut(framing))
        .or_else(|| framed(wanted));
    if let Some((middle, distance)) = chosen {
        camera.glide_to(middle - views.free.shift(rotation) * distance, distance);
    }
}

/// How many steps between the least and most share an expanded module may take
/// [`fill`] tries, either way from the one it wants, for one that leaves no sphere
/// half under the panel.
const PANEL_STEPS: usize = 16;

/// Whether any of `balls` shows cut by the foot of the free area, where a panel
/// docks, seen turned to `rotation` looking at `target` from `distance`; never with
/// no panel.
fn under_panel<'a>(
    balls: impl Iterator<Item = &'a Ball>,
    target: Vec3,
    distance: f32,
    rotation: Quat,
    lens: Lens,
) -> bool {
    let foot = lens.centre.y - lens.half.y;
    if foot <= -1.0 + 1e-3 {
        return false;
    }
    let eye = target + rotation * Vec3::Z * distance;
    let mut balls = balls;
    balls.any(|ball| {
        lens.project(rotation, eye, ball.center)
            .is_some_and(|(at, depth)| {
                let size = Vec2::new(ball.radius, ball.radius) / (depth * lens.tan);
                at.x.abs() < 1.0 + size.x && (at.y - foot).abs() < size.y
            })
    })
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
    let shown: Vec<(usize, Ball)> = nucleus
        .balls()
        .filter(|(_, ball)| nucleus.shows_body(ball.body))
        .collect();
    let near = shown.iter().map(|(_, ball)| ball.radius).reduce(f32::min)?;
    let rotation = camera.aim_rotation();
    let (_, fits) = place(
        &within(&nucleus.atlas_points(rotation), ROOM),
        rotation,
        lens,
    )?;
    let expanded = nucleus.expanded_module();
    Some(Reach {
        near,
        far: (fits * FAR_MARGIN).max(near),
        expanded: shown.iter().position(|&(m, _)| Some(m) == expanded),
        spheres: shown.iter().map(|(_, b)| (b.center, b.radius)).collect(),
        lens,
    })
}

/// The closest a framing brings the camera to the middle of what it frames.
const NEAREST_FRAMING: f32 = 1.0;

/// The middle of `points` across the view turned to `rotation`, and how far back
/// from it the camera must be ([`distance_from`]).
fn place(points: &[(Vec3, Vec2)], rotation: Quat, lens: Lens) -> Option<(Vec3, f32)> {
    if points.is_empty() {
        return None;
    }
    let middle_along = |axis: Vec3| {
        let (low, high) = points
            .iter()
            .fold((f32::MAX, f32::MIN), |(low, high), (p, _)| {
                (low.min(p.dot(axis)), high.max(p.dot(axis)))
            });
        axis * (low + high) / 2.0
    };
    let middle = [Vec3::X, Vec3::Y, Vec3::Z]
        .into_iter()
        .map(|axis| middle_along(rotation * axis))
        .sum();
    Some((middle, distance_from(points, middle, rotation, lens)))
}

/// How far back from `middle` a camera turned to `rotation` must be, looking past it
/// as [`Lens::shift`] says, for each of `points` to show within its share of the
/// free area about its middle, across and up.
fn distance_from(points: &[(Vec3, Vec2)], middle: Vec3, rotation: Quat, lens: Lens) -> f32 {
    let (right, up, back) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z);
    let (centre, tan) = (lens.centre, lens.tan);
    points
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
        .fold(NEAREST_FRAMING, f32::max)
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
        let show =
            |points: &[Vec3]| distance_from(&within(points, ROOM), Vec3::ZERO, head_on, lens);
        let near = [Vec3::new(3.0, 2.0, 0.0), Vec3::new(-4.0, -1.0, 0.0)];
        let needed = show(&[Vec3::new(0.0, 40.0, 0.0)]);
        assert!(show(&near) < needed, "what is close needs less");
        let half_height = needed * lens.tan.y * ROOM;
        assert!(
            (half_height - 40.0).abs() < 1e-2,
            "far enough back that it shows, with room: {needed}"
        );
        assert!(
            show(&[Vec3::new(40.0, 0.0, 0.0)]) < needed,
            "the view is wider than it is tall"
        );
    }
}
