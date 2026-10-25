//! Call order as a second view of the same map. Time runs along depth: each call the
//! journey discloses gets a marker at its function's node, pushed back by when it
//! runs, evenly spaced. Head-on, the map, every marker sits on its node and nothing
//! has moved. Swing the camera a quarter turn to look along the map's x axis and depth
//! becomes the screen's horizontal: the journey reads left to right, each call at its
//! node's height, so modules keep their bands, and a function called twice is there
//! twice. Under a lens that narrows as the camera nears the side, that view is near
//! orthographic, so the calls stay evenly spaced on screen.
//!
//! In between, at any angle, is a 3D view worth stopping at: the map in perspective,
//! quieted, and the calls standing out of it along time in one upright sheet, each
//! caller's lifeline a rail running out from the map, each call dropping straight from
//! its caller's rail to its marker, a string back to its node, and fog to say what is
//! further; [`crate::stage`] lights it so it reads as space. A
//! call hangs from its caller's lifeline for as long as the caller runs: from the side
//! a sequence diagram; head-on, the map's own links. Turning back folds each call into
//! its node.
//!
//! `Z` steps from the map to the 3D view to the side and back; [`Chrome`] holds which.

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::SpanId;

use crate::{
    camera::{FRONT, OrbitCamera},
    explorer::Explorer,
    graph_view::{
        CALLEE, CALLER, FURTHER, LEVELS, NodeMaterials, node_look, node_size, orbit_anchor, regions,
    },
    ui::Chrome,
};

/// Which way the camera looks at call order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TimeView {
    /// The map as the simulation lays it out; time folded away.
    #[default]
    Map,
    /// Turned 45° and looking down a little: the map in perspective, the calls
    /// standing out of it along time.
    Rails,
    /// Swung a quarter turn: time runs left to right.
    Side,
}

impl TimeView {
    /// `Z` steps from the map to the 3D view to the side, and back to the map.
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Map => Self::Rails,
            Self::Rails => Self::Side,
            Self::Side => Self::Map,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Rails => "3d",
            Self::Side => "side",
        }
    }

    /// The camera's yaw and pitch for this view: from the side looking along x, time
    /// to the right.
    fn angles(self) -> (f32, f32) {
        match self {
            Self::Map => FRONT,
            // Level, so the rail of time runs flat across the screen.
            Self::Rails => (-RAILS_YAW.to_radians(), 0.0),
            Self::Side => (-std::f32::consts::FRAC_PI_2, 0.0),
        }
    }
}

/// How far round from head-on the 3D view turns, in degrees.
const RAILS_YAW: f32 = 55.0;

/// One call on the timeline.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Call {
    pub(crate) span: SpanId,
    pub(crate) tile: usize,
    /// The call that makes it, by index; none for the journey's start.
    pub(crate) caller: Option<usize>,
    /// The last call that runs under it, by index; itself when it calls nothing shown.
    pub(crate) last: usize,
}

/// The timeline as it stands.
#[derive(Resource, Debug, Default)]
pub(crate) struct Timeline {
    /// How far time has unfolded: 0 on the map, 1 seen from the side.
    pub(crate) unfold: f32,
    /// Whether the time view is on, eased.
    weight: f32,
    /// The disclosed calls, in the order they run.
    pub(crate) calls: Vec<Call>,
    /// The depth the first call sits at, unfolded, and the step to the next.
    start: f32,
    step: f32,
    /// The plane across x the calls settle into from the side: the start's node's.
    plane: f32,
    /// How far along time from the map the camera looks, unfolded.
    pub(crate) pan: f32,
    /// The cursor's call, by index.
    cursor: Option<usize>,
    /// The calls standing out, by index: a long journey shows only those round the
    /// cursor's, and [`Self::slide`] eases towards where they start.
    window: (usize, usize),
    slide: f32,
    /// Where the camera stands, and its lens, as of the last frame.
    eye: Vec3,
    lens: f32,
    /// How far from the eye the fog starts and is thickest.
    haze: (f32, f32),
    markers: Vec<Entity>,
}

impl Timeline {
    /// How far into the side view the camera has come: 0 until it is nearly there.
    pub(crate) fn side(&self) -> f32 {
        side(self.unfold)
    }

    /// How much the view is the 3D one: none on the map or from the side.
    pub(crate) fn depth_view(&self) -> f32 {
        quiet(self.unfold) * (1.0 - self.side())
    }

    /// Where the camera stood as of the last frame.
    pub(crate) fn eye(&self) -> Vec3 {
        self.eye
    }

    /// The cursor's call, by index.
    pub(crate) fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    /// How far from the eye haze starts and thickens to its most.
    pub(crate) fn haze_range(&self) -> (f32, f32) {
        self.haze
    }

    /// The calls a node runs that stand out, by index.
    pub(crate) fn calls_of(&self, tile: usize) -> impl Iterator<Item = usize> + '_ {
        calls_of(self.calls.iter().map(|c| c.tile), tile).filter(|&i| self.shown(i))
    }

    /// The calls standing out, by index.
    pub(crate) fn window(&self) -> std::ops::Range<usize> {
        self.window.0..self.window.1
    }

    /// Whether call `index` stands out: in the window round the cursor's.
    pub(crate) fn shown(&self, index: usize) -> bool {
        self.window().contains(&index)
    }

    /// Where call `index` is drawn: its node's place, pushed back along time as far
    /// as the view has unfolded; nowhere outside the window.
    pub(crate) fn point(&self, explorer: &Explorer, index: usize) -> Option<Vec3> {
        let call = self.calls.get(index).filter(|_| self.shown(index))?;
        let home = explorer.tile_point(call.tile)?;
        Some(self.at(home, index))
    }

    /// Where a node shows: on the map its place, unfolded its first call's marker, or
    /// for the cursor's node the cursor's call.
    pub(crate) fn tile_point(&self, explorer: &Explorer, tile: usize) -> Option<Vec3> {
        if self.unfold <= 0.0 {
            return explorer.tile_point(tile);
        }
        let cursor = self.cursor.filter(|&c| self.calls[c].tile == tile);
        let index = cursor.or_else(|| self.calls_of(tile).next());
        match index {
            Some(index) => self.point(explorer, index),
            None => explorer.tile_point(tile),
        }
    }

    /// How wide call `index`'s marker is drawn: larger than its node on the map, as the
    /// side view is sparse, but clear of its neighbors, and growing as time unfolds.
    /// Never more than a small share of the view, so a call near the camera does not
    /// balloon; in the 3D view never less than half that, so a lit ball reads as one
    /// however small its node.
    pub(crate) fn marker_size(&self, explorer: &Explorer, index: usize) -> f32 {
        let Some(call) = self.calls.get(index) else {
            return 0.0;
        };
        let size = (node_size(explorer.graph(), call.tile) * 1.6).min(self.step * 0.6);
        let size = if self.cursor == Some(index) {
            size * 1.3
        } else {
            size
        };
        let size = size * self.unfold.sqrt();
        match self.point(explorer, index) {
            Some(at) => {
                let cap = marker_cap(self.eye.distance(at), self.lens);
                size.max(cap * 0.5 * self.depth_view())
                    .min(cap)
                    .min(self.step * 0.8)
            }
            None => size,
        }
    }

    /// `home` moved to call `index`'s moment. As time unfolds, calls also settle into
    /// one sheet across x, standing out of the map from the start's node: every call
    /// down from a lifeline is upright, so none crosses another, and from the side the
    /// camera looks straight through it, so no call sits nearer than another.
    pub(crate) fn at(&self, home: Vec3, index: usize) -> Vec3 {
        #[expect(clippy::cast_precision_loss, reason = "a call index")]
        let moment = index as f32;
        self.at_moment(home, moment)
    }

    /// `home` moved to `moment`, counted in calls from the journey's start; the
    /// window's first call stands where the first would.
    fn at_moment(&self, home: Vec3, moment: f32) -> Vec3 {
        let depth = self.start + self.step * (moment - self.slide);
        let settle = sheet(self.unfold);
        Vec3::new(
            home.x + (self.plane - home.x) * settle,
            home.y,
            // All the way out by the 3D view, so the map's own slight depth does not
            // tilt a call down from its lifeline.
            home.z + (depth - home.z) * settle,
        )
    }
}

impl Timeline {
    /// A long journey stands out only round the cursor's call, sliding along as it
    /// steps; at once when the view swings or the window jumps far.
    fn slide_window(&mut self, swung: bool, ease: f32) {
        self.window = window(self.calls.len(), self.cursor, WINDOW);
        #[expect(clippy::cast_precision_loss, reason = "a call index")]
        let (first, half) = (self.window.0 as f32, WINDOW as f32 / 2.0);
        if swung || (first - self.slide).abs() > half {
            self.slide = first;
        } else {
            self.slide += (first - self.slide) * ease;
        }
    }
}

/// One of the timeline's straight lines: a caller's lifeline, or a call down from it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Line {
    pub(crate) from: Vec3,
    pub(crate) to: Vec3,
    pub(crate) color: Color,
    /// A lifeline, drawn heavier.
    pub(crate) rail: bool,
}

impl Timeline {
    /// Each caller's lifeline along time and each call down or up from it to its
    /// marker, faded in as time unfolds. Calls into and out of the cursor's call take
    /// the caller and callee colors; the rest are pale.
    pub(crate) fn lines(&self, explorer: &Explorer) -> Vec<Line> {
        let unfold = self.unfold;
        let mut lines = Vec::new();
        if unfold <= 0.0 {
            return lines;
        }
        let graph = explorer.graph();
        let pale = FURTHER.with_alpha(0.4 * unfold);
        let (first, end) = self.window;
        for (index, call) in self.calls.iter().enumerate() {
            let Some(home) = explorer.tile_point(call.tile) else {
                continue;
            };
            // The lifeline: from the call to the last call under it, inside the window.
            let (from, to) = (index.max(first), call.last.min(end.saturating_sub(1)));
            if call.last > index && to > from {
                let color = if self.cursor == Some(index) {
                    CALLEE.with_alpha(0.7 * unfold)
                } else {
                    pale
                };
                lines.push(Line {
                    from: self.at(home, from),
                    to: self.at(home, to),
                    color,
                    rail: true,
                });
            }
            if !self.shown(index) {
                continue;
            }
            let Some(source) = call
                .caller
                .and_then(|c| self.calls.get(c))
                .and_then(|caller| explorer.tile_point(caller.tile))
            else {
                continue;
            };
            // At this call's moment, from the caller's lifeline to this call's marker.
            let radius = node_size(graph, call.tile) * 0.5;
            let (from, to) = (self.at(source, index), self.at(home, index));
            let length = from.distance(to);
            if length <= radius {
                continue;
            }
            let color = if self.cursor == Some(index) {
                CALLER.with_alpha(0.9 * unfold)
            } else if self.cursor == call.caller {
                CALLEE.with_alpha(0.9 * unfold)
            } else {
                pale
            };
            lines.push(Line {
                from,
                to: from + (to - from) * ((length - radius) / length),
                color,
                rail: false,
            });
        }
        lines
    }
}

/// A call's marker, by index into [`Timeline::calls`].
#[derive(Component, Debug)]
pub(crate) struct Marker(usize);

pub(crate) struct TimelinePlugin;

impl Plugin for TimelinePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Timeline>()
            .init_gizmo_group::<TimeGizmos>()
            .init_gizmo_group::<RailGizmos>()
            .add_systems(Startup, line_widths);
    }
}

/// The timeline's lifelines and calls.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct TimeGizmos;

/// The lifelines, drawn heavier: the rails the calls hang from.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct RailGizmos;

fn line_widths(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<TimeGizmos>().0.line.width = 1.5;
    store.config_mut::<RailGizmos>().0.line.width = 3.0;
}

/// The calls under `root` in the order they run, depth first, each with the index of
/// the call that makes it and of the last call under it. `children` lists the shown
/// calls a call makes.
pub(crate) fn run_order<S: Copy>(
    root: S,
    children: impl Fn(S) -> Vec<S>,
) -> Vec<(S, Option<usize>, usize)> {
    let mut calls: Vec<(S, Option<usize>, usize)> = Vec::new();
    let mut stack: Vec<(S, Option<usize>)> = vec![(root, None)];
    while let Some((call, caller)) = stack.pop() {
        let index = calls.len();
        calls.push((call, caller, index));
        // Each caller up the chain now runs at least until this call.
        let mut up = caller;
        while let Some(at) = up {
            calls[at].2 = index;
            up = calls[at].1;
        }
        stack.extend(children(call).into_iter().rev().map(|c| (c, Some(index))));
    }
    calls
}

/// The most calls that stand out at once; a longer journey shows that many round the
/// cursor's.
pub(crate) const WINDOW: usize = 40;

/// Which of `count` calls stand out, by index from and to: all of them when no more
/// than `size`, else `size` of them with the cursor's a third of the way in.
pub(crate) fn window(count: usize, cursor: Option<usize>, size: usize) -> (usize, usize) {
    if count <= size {
        return (0, count);
    }
    let first = cursor
        .unwrap_or(0)
        .saturating_sub(size / 3)
        .min(count - size);
    (first, first + size)
}

/// The fewest world units between two calls, so neighbors never touch.
const MIN_STEP: f32 = 2.4;

/// How long time runs, in world units, for `count` calls over a journey reaching
/// `reach` across the map: about as wide as the map shows it, and no tighter than
/// [`MIN_STEP`] a call.
pub(crate) fn time_length(count: usize, reach: f32) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a call count")]
    let gaps = count.saturating_sub(1) as f32;
    (reach * 3.0).max(gaps * MIN_STEP)
}

/// How far into the side view `unfold` is: nothing short of about 35° from it, all of
/// it the last few degrees, so every angle on the way stays in perspective.
pub(crate) fn side(unfold: f32) -> f32 {
    let t = ((unfold - 0.82) / 0.15).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How far calls have settled into the one sheet they stand in out of the map: none
/// on the map, all of it well before the 3D view.
pub(crate) fn sheet(unfold: f32) -> f32 {
    let t = ((unfold - 0.1) / 0.55).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much haze lies `distance` from the eye, given where it starts and is thickest.
pub(crate) fn haze(distance: f32, (start, end): (f32, f32)) -> f32 {
    ((distance - start) / (end - start).max(1e-3)).clamp(0.0, 1.0)
}

/// How quiet the map has gone, from 0 head-on to all of it by about 35° round.
fn quiet(unfold: f32) -> f32 {
    (unfold / 0.6).min(1.0)
}

/// Which of the calls, given the node each runs, run `tile`, by index.
pub(crate) fn calls_of(
    tiles: impl Iterator<Item = usize>,
    tile: usize,
) -> impl Iterator<Item = usize> {
    tiles
        .enumerate()
        .filter(move |&(_, t)| t == tile)
        .map(|(i, _)| i)
}

/// The widest a marker `distance` from the camera through `lens` may be: a share of
/// the view's height, whatever its distance.
pub(crate) fn marker_cap(distance: f32, lens: f32) -> f32 {
    const SHARE: f32 = 0.025;
    distance * (FIELD / 2.0).tan() / lens.max(1.0) * 2.0 * SHARE
}

/// How far the camera's lens narrows as time unfolds: only near the side, where it
/// looks from eight times as far, through an eighth of the angle, nearly flat, so
/// calls are evenly spaced however near the camera they are. Elsewhere, perspective.
pub(crate) fn lens(unfold: f32) -> f32 {
    1.0 + 7.0 * side(unfold)
}

/// Where along time to look, from its middle, to keep the cursor's call in sight when
/// time runs wider than the view's `half_width`.
pub(crate) fn time_pan(cursor: f32, length: f32, half_width: f32) -> f32 {
    let room = (length / 2.0 - half_width * 0.8).max(0.0);
    cursor.clamp(-room, room)
}

/// The disclosed journey's calls, by node, in the order they run; a call whose node
/// the graph lacks is left out and what it calls hangs from its caller.
fn disclosed_calls(explorer: &Explorer) -> Vec<Call> {
    let Some(journey) = explorer.journey() else {
        return Vec::new();
    };
    let disclosed = explorer.disclosure();
    let map = explorer.map();
    let tile = |span: SpanId| explorer.unit_id(span).and_then(|u| map.tile_of(u));
    // Calls without a node are skipped through to what they call.
    let shown = |span: SpanId| -> Vec<SpanId> {
        let mut out = Vec::new();
        let mut queue: Vec<SpanId> = disclosed.children(span).to_vec();
        queue.reverse();
        while let Some(child) = queue.pop() {
            if disclosed.number(child).is_none() {
                continue;
            }
            if tile(child).is_some() {
                out.push(child);
            } else {
                queue.extend(disclosed.children(child).iter().rev());
            }
        }
        out
    };
    let root = journey.root();
    run_order(root, shown)
        .into_iter()
        .filter_map(|(span, caller, last)| {
            Some(Call {
                span,
                tile: tile(span)?,
                caller,
                last,
            })
        })
        .collect()
}

/// Swings the camera when the view changes, lays the timeline out for the disclosed
/// journey, and unfolds it as far as the camera has turned.
pub(crate) fn swing(
    time: Res<Time>,
    chrome: Res<Chrome>,
    explorer: Res<Explorer>,
    mut timeline: ResMut<Timeline>,
    mut last: Local<Option<TimeView>>,
    mut cameras: Query<(&mut OrbitCamera, &Transform, &Projection, &mut DistanceFog)>,
) {
    let Ok((mut camera, view, projection, mut fog)) = cameras.single_mut() else {
        return;
    };
    let wanted = chrome.time;
    // A camera angle asked for, in place of the view's own.
    let angles = chrome
        .angle
        .filter(|_| wanted != TimeView::Map)
        .unwrap_or_else(|| wanted.angles());
    let calls = disclosed_calls(&explorer);
    let swung = *last != Some(wanted);
    if swung {
        // Opening on the map keeps the default angle; back on it, the usual view.
        if last.is_some() || wanted != TimeView::Map {
            camera.turn_to(angles);
        }
        if last.is_some() && wanted == TimeView::Map {
            orbit_anchor(&explorer, &mut camera);
        }
    }
    *last = Some(wanted);
    let on = if wanted == TimeView::Map { 0.0 } else { 1.0 };
    let ease = 1.0 - (-time.delta_secs() * 6.0).exp();
    timeline.weight += (on - timeline.weight) * ease;
    if (on - timeline.weight).abs() < 1e-3 {
        timeline.weight = on;
    }
    // How much the camera looks along x: all of it from the side.
    let unfold = (timeline.weight * view.back().x.abs()).clamp(0.0, 1.0);
    timeline.unfold = if unfold < 1e-3 { 0.0 } else { unfold };
    camera.lens = lens(timeline.unfold);
    timeline.eye = view.translation;
    timeline.lens = camera.lens;
    // Fog in the 3D views, so what is further reads as further; none head-on, where
    // nothing is, nor from the side, where everything is as far.
    let depth = camera.distance * camera.lens;
    timeline.haze = (depth * 0.85, depth * 1.9);
    fog.falloff = FogFalloff::Linear {
        start: depth * 0.85,
        end: depth * 1.9,
    };
    fog.color = FOG.with_alpha(0.85 * timeline.unfold * (1.0 - timeline.side()));

    timeline.calls = calls;
    let anchor = explorer.anchor().and_then(|t| explorer.tile_point(t));
    let Some(anchor) = anchor else {
        return;
    };
    // On a call the timeline leaves out, as a closure, the nearest call it is under.
    let stack = explorer
        .selected()
        .map(|s| explorer.stack(s))
        .unwrap_or_default();
    timeline.cursor = stack
        .iter()
        .rev()
        .find_map(|&span| timeline.calls.iter().position(|c| c.span == span));
    timeline.slide_window(swung, ease);
    let reach = timeline
        .window()
        .filter_map(|i| explorer.tile_point(timeline.calls[i].tile))
        .map(|p| p.truncate().distance(anchor.truncate()))
        .fold(10.0, f32::max);
    let count = timeline.window().len();
    let length = time_length(count, reach);
    #[expect(clippy::cast_precision_loss, reason = "a call count")]
    let gaps = count.saturating_sub(1).max(1) as f32;
    timeline.step = length / gaps;
    // Time runs out of the map, towards the camera head-on, from a little clear of it
    // so the first calls stand apart from the map in the 3D views.
    let gap = length * GAP;
    timeline.start = anchor.z + gap;
    timeline.plane = anchor.x;
    let aspect = match projection {
        Projection::Perspective(p) => p.aspect_ratio,
        _ => 1.6,
    };
    // Turned to time, the view fits it, until the viewer pans or zooms away.
    if wanted != TimeView::Map && !camera.free {
        // Most of the calls: a few far outliers do not push the view back.
        let mut heights: Vec<f32> = timeline
            .window()
            .filter_map(|i| explorer.tile_point(timeline.calls[i].tile))
            .map(|p| (p.y - anchor.y).abs())
            .collect();
        heights.sort_by(f32::total_cmp);
        let height = heights
            .get(heights.len() * 9 / 10)
            .or(heights.last())
            .copied()
            .unwrap_or_default();
        let mut distance = time_distance(length, count, height, aspect);
        if wanted == TimeView::Rails {
            // Room for the map as well as the calls, which it sees across.
            distance = distance.max(reach * 2.6) * 1.3;
        }
        if (camera.goal_distance().unwrap_or(camera.distance) - distance).abs() > distance * 0.01 {
            let target = camera.target;
            camera.glide_to(target, distance);
        }
    }
    let half_width = camera.distance * (FIELD / 2.0).tan() * aspect;
    #[expect(clippy::cast_precision_loss, reason = "a call index")]
    let at = timeline.cursor.map_or(0.0, |i| {
        timeline.step * (i as f32 - timeline.slide) - length / 2.0
    });
    // From the side, the middle of time; in 3D, a little back towards the map, so
    // both stay in view.
    let towards_map = 0.82 + 0.18 * timeline.side();
    timeline.pan = (gap + length / 2.0 + time_pan(at, length, half_width))
        * sheet(timeline.unfold)
        * towards_map;
}

/// How far clear of the map time starts, as a share of its length.
const GAP: f32 = 0.3;

/// The fog's color: the background's.
const FOG: Color = Color::srgb(0.06, 0.07, 0.09);

/// Calls the side view takes in across its width at most; a longer journey pans.
const CALLS_ACROSS: f32 = 60.0;

/// How far back to look from the side to see time `length` long holding `count`
/// calls, `height` above or below the start, at `aspect`, with room for names: all of
/// time when it holds no more than [`CALLS_ACROSS`] calls, else that many.
pub(crate) fn time_distance(length: f32, count: usize, height: f32, aspect: f32) -> f32 {
    let tan = (FIELD / 2.0).tan();
    #[expect(clippy::cast_precision_loss, reason = "a call count")]
    let share = (CALLS_ACROSS / count.max(1) as f32).min(1.0);
    // Room beside time for the modules' names on the left and the last call's.
    let across = (length * share / 2.0) * 1.3 / (tan * aspect);
    let tall = height * 1.3 / tan;
    across.max(tall).max(20.0)
}

/// The camera's field of view before the lens narrows it.
pub(crate) const FIELD: f32 = std::f32::consts::FRAC_PI_4;

/// Places a marker for each call, as its node looks, while time is unfolded; shrinks
/// the map's own nodes away as it unfolds.
#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
pub(crate) fn place_markers(
    mut commands: Commands,
    mut timeline: ResMut<Timeline>,
    explorer: Res<Explorer>,
    materials: Res<NodeMaterials>,
    mut assets: ResMut<Assets<StandardMaterial>>,
    mut lit: Local<
        std::collections::HashMap<crate::graph_view::NodeLook, Handle<StandardMaterial>>,
    >,
    mut meshes: ResMut<Assets<Mesh>>,
    mut shrunk: Local<bool>,
    mut markers: Query<
        (
            &Marker,
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<crate::scene::Tile>,
    >,
    mut nodes: Query<(&crate::scene::Tile, &mut Transform), Without<Marker>>,
) {
    let unfold = timeline.unfold;
    if unfold > 0.0 || *shrunk {
        let graph = explorer.graph();
        let disclosed = explorer.disclosure();
        let gone = 1.0 - side(unfold);
        for (tile, mut transform) in &mut nodes {
            let size = node_scale(&explorer, tile.0) * node_size(graph, tile.0);
            // The map stays a map until the side, what the journey runs in full and
            // the rest quieter; from the side the map's nodes would only pile up.
            let left = if disclosed.tiles().contains_key(&tile.0) {
                gone
            } else {
                (1.0 - 0.35 * quiet(unfold)) * gone
            };
            transform.scale = Vec3::splat(size * left.max(0.001));
        }
        *shrunk = unfold > 0.0;
    }
    let wanted = if unfold > 0.0 {
        timeline.calls.len()
    } else {
        0
    };
    if timeline.markers.len() < wanted {
        let mesh = meshes.add(
            Sphere::new(0.5)
                .mesh()
                .ico(3)
                .unwrap_or_else(|_| Sphere::new(0.5).mesh().uv(24, 16)),
        );
        let material = materials.0.values().next().cloned().unwrap_or_default();
        for index in timeline.markers.len()..wanted {
            let entity = commands
                .spawn((
                    Marker(index),
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .observe(on_marker_click)
                .observe(on_marker_over)
                .observe(on_marker_out)
                .id();
            timeline.markers.push(entity);
        }
    }
    for (marker, mut transform, mut visibility, mut material) in &mut markers {
        let index = marker.0;
        let at = (index < wanted)
            .then(|| timeline.point(&explorer, index))
            .flatten();
        visibility.set_if_neq(if at.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        let Some(at) = at else {
            continue;
        };
        let call = timeline.calls[index];
        transform.translation = at;
        transform.scale = Vec3::splat(timeline.marker_size(&explorer, index));
        // As its node looks, but glowing less, so the light shades it as a ball in
        // space: a depth cue the map's flat glow does not need.
        let look = node_look(&explorer, call.tile);
        if !lit.contains_key(&look)
            && let Some(base) = materials.0.get(&(look, LEVELS)).and_then(|h| assets.get(h))
        {
            let shaded = StandardMaterial {
                emissive: base.emissive * 0.3,
                perceptual_roughness: 0.35,
                reflectance: 0.6,
                ..base.clone()
            };
            lit.insert(look, assets.add(shaded));
        }
        if let Some(wanted) = lit.get(&look)
            && &material.0 != wanted
        {
            material.0 = wanted.clone();
        }
    }
}

/// A node's scale for what the lens shows, as `style_nodes` sets it.
fn node_scale(explorer: &Explorer, tile: usize) -> f32 {
    if node_look(explorer, tile) == crate::graph_view::NodeLook::Hidden {
        0.35
    } else {
        0.35 + 0.65 * explorer.visibility(tile)
    }
}

/// A click on a marker puts the journey's cursor on its call.
fn on_marker_click(
    click: On<PointerClick>,
    markers: Query<&Marker>,
    timeline: Res<Timeline>,
    mut explorer: ResMut<Explorer>,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    if let Ok(marker) = markers.get(click.entity)
        && let Some(call) = timeline.calls.get(marker.0)
    {
        explorer.select(Some(call.span));
    }
}

fn on_marker_over(
    over: On<PointerOver>,
    markers: Query<&Marker>,
    timeline: Res<Timeline>,
    mut explorer: ResMut<Explorer>,
) {
    if let Ok(marker) = markers.get(over.entity)
        && let Some(call) = timeline.calls.get(marker.0)
    {
        explorer.hovered_tile = Some(call.tile);
    }
}

fn on_marker_out(
    out: On<PointerOut>,
    markers: Query<&Marker>,
    timeline: Res<Timeline>,
    mut explorer: ResMut<Explorer>,
) {
    if let Ok(marker) = markers.get(out.entity)
        && let Some(call) = timeline.calls.get(marker.0)
        && explorer.hovered_tile == Some(call.tile)
    {
        explorer.hovered_tile = None;
    }
}

/// Draws each call's string back to its node, and its [`Timeline::lines`] as lines
/// on the map and from the side.
pub(crate) fn draw_lines(
    timeline: Res<Timeline>,
    explorer: Res<Explorer>,
    cameras: Query<&Transform, With<OrbitCamera>>,
    mut gizmos: Gizmos<TimeGizmos>,
    mut rails: Gizmos<RailGizmos>,
) {
    let unfold = timeline.unfold;
    if unfold <= 0.0 {
        return;
    }
    let graph = explorer.graph();
    let strength = string_strength(unfold);
    for index in timeline.window() {
        let call = timeline.calls[index];
        let Some(home) = explorer.tile_point(call.tile) else {
            continue;
        };
        let radius = node_size(graph, call.tile) * 0.5;
        // The node under the pointer, or one of its calls: its strings lit.
        let lit = explorer.hovered_tile == Some(call.tile);
        let string = if lit {
            Color::WHITE.with_alpha(0.7 * strength)
        } else {
            FURTHER.with_alpha(0.18 * strength)
        };
        // Back along time to the node it unfolded from.
        if string.alpha() > 0.02 {
            let at = timeline.at(home, index);
            if at.distance(home) > radius * 2.0 {
                dashed(&mut gizmos, home, at, radius, string);
            }
        }
    }
    // In the 3D view the rails and calls are tubes in space instead.
    let flat = 1.0 - timeline.depth_view();
    for line in timeline.lines(&explorer) {
        let color = line.color.with_alpha(line.color.alpha() * flat);
        if line.rail {
            rails.line(line.from, line.to, color);
        } else {
            gizmos.line(line.from, line.to, color);
        }
    }
    // The journey's own lifeline ends in an arrowhead: time runs this way.
    if let Some(first) = timeline
        .calls
        .first()
        .filter(|c| c.last > 0 && c.last < timeline.window.1)
        && let Some(home) = explorer.tile_point(first.tile)
    {
        let to = timeline.at(home, first.last);
        let size = timeline.step.min(4.0) * 0.5;
        let back = Vec3::NEG_Z * size;
        let color = FURTHER.with_alpha(0.5 * unfold);
        rails.line(to, to + back + Vec3::Y * size * 0.6, color);
        rails.line(to, to + back - Vec3::Y * size * 0.6, color);
    }
    // The node under the pointer: every call it runs and its place on the map, ringed.
    let (Some(tile), Ok(view)) = (explorer.hovered_tile, cameras.single()) else {
        return;
    };
    let ring = Color::WHITE.with_alpha(0.9 * unfold);
    let facing = |at: Vec3| Isometry3d::new(at, view.rotation);
    if let Some(home) = explorer.tile_point(tile) {
        gizmos.circle(facing(home), node_size(graph, tile), ring);
    }
    for index in timeline.calls_of(tile) {
        if let Some(at) = timeline.point(&explorer, index) {
            gizmos.circle(facing(at), timeline.marker_size(&explorer, index), ring);
        }
    }
}

/// A line of dashes `dash` long, from `a` to `b`.
fn dashed(gizmos: &mut Gizmos<TimeGizmos>, a: Vec3, b: Vec3, dash: f32, color: Color) {
    let dashes = (a.distance(b) / dash).floor().clamp(1.0, 200.0);
    let mut at = 0.0;
    while at < dashes {
        gizmos.line(
            a.lerp(b, at / dashes),
            a.lerp(b, ((at + 1.0) / dashes).min(1.0)),
            color,
        );
        at += 2.0;
    }
}

/// How strongly the map's links and rings draw: quieter as time unfolds, to a little
/// over half by 35° round so the map stays a map under the calls, and gone from the
/// side, where they would collapse into a column.
pub(crate) fn map_strength(timeline: &Timeline) -> f32 {
    (1.0 - 0.45 * quiet(timeline.unfold)) * (1.0 - timeline.side())
}

/// How strongly a call's string back to its node draws: through the 3D views, where
/// it says which node a call came from, and not at all on the map or from the side.
pub(crate) fn string_strength(unfold: f32) -> f32 {
    unfold * (1.0 - side(unfold))
}

/// Names the calls on the timeline, angled up from each marker so neighbors fit: the
/// cursor's call first, then what makes it and what it calls, then the rest by
/// nearness to it or to the window's start, each in full above its marker or below
/// it, else by its short name, wherever it clears the names already placed. Says how
/// many calls the window leaves out at either end.
pub(crate) fn draw_labels(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    timeline: &Timeline,
) {
    let alpha = ((timeline.unfold - 0.25) / 0.3).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    let disclosed = explorer.disclosure();
    let callers: Vec<Option<usize>> = timeline.calls.iter().map(|c| c.caller).collect();
    let mut taken: Vec<(egui::Pos2, f32)> = Vec::new();
    // Below the top bar and short of the right edge, so no name is cut off.
    let bounds = painter.clip_rect().shrink2(egui::vec2(16.0, 16.0));
    let along = egui::vec2(LABEL_ANGLE.cos(), LABEL_ANGLE.sin());
    for index in label_order(&callers, timeline.cursor, timeline.window.0) {
        let call = timeline.calls[index];
        let Some(point) = timeline.point(explorer, index) else {
            continue;
        };
        let Some(at) = to_screen(point) else {
            continue;
        };
        let number = disclosed.number(call.span).unwrap_or_default();
        let name = explorer.label(call.span);
        let numbered = |name: &str| {
            if number.is_empty() {
                name.to_owned()
            } else {
                format!("{number}  {name}")
            }
        };
        let current = timeline.cursor == Some(index);
        let color = if current {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_rgb(200, 205, 215)
        };
        let font = egui::FontId::proportional(if current { 13.0 } else { 11.5 });
        // Clear of the marker, however large it shows.
        let radius = to_screen(point + Vec3::Y * timeline.marker_size(explorer, index) * 0.5)
            .map_or(4.0, |edge| (at.y - edge.y).abs());
        let above = at + egui::vec2(radius * 0.8 + 3.0, -radius * 0.8 - 7.0);
        let below = at + egui::vec2(radius + 2.0, radius + 4.0);
        let texts = [numbered(&name), numbered(short_name(&name))];
        let placed = texts.iter().enumerate().find_map(|(i, text)| {
            // The short name only where the full one is longer.
            if i > 0 && *text == texts[0] {
                return None;
            }
            let galley =
                painter.layout_no_wrap(text.clone(), font.clone(), color.gamma_multiply(alpha));
            let width = galley.size().x;
            [above, below]
                .into_iter()
                .find(|&start| {
                    bounds.contains(start + along * width)
                        && !taken
                            .iter()
                            .any(|&(other, length)| angled_overlap(other, length, start, width))
                })
                .map(|start| (start, galley))
        });
        let Some((start, galley)) = placed else {
            continue;
        };
        taken.push((start, galley.size().x));
        painter.add(egui::epaint::TextShape::new(start, galley, color).with_angle(LABEL_ANGLE));
    }
    draw_window_ends(painter, to_screen, explorer, timeline, alpha);
    // The map's rings name its modules until the side, where they fold away.
    draw_modules(painter, to_screen, explorer, timeline, timeline.side());
}

/// A name without its module or type: `draw` for `journey_panel.draw`.
fn short_name(name: &str) -> &str {
    name.rsplit(['.', ':']).next().unwrap_or(name)
}

/// Says how many calls run before and after the window, at the ends of the journey's
/// own lifeline.
fn draw_window_ends(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    timeline: &Timeline,
    alpha: f32,
) {
    let (first, end) = timeline.window;
    let count = timeline.calls.len();
    let Some(home) = timeline
        .calls
        .first()
        .and_then(|root| explorer.tile_point(root.tile))
    else {
        return;
    };
    let color = egui::Color32::from_rgb(170, 176, 190).gamma_multiply(alpha);
    let font = egui::FontId::proportional(12.0);
    #[expect(clippy::cast_precision_loss, reason = "a call index")]
    let (first_at, end_at) = (first as f32, end as f32);
    let bounds = painter.clip_rect().shrink(8.0);
    // On a backing, as it sits over the map or the rail's far end, and on screen.
    let note = |at: egui::Pos2, align: egui::Align2, text: String| {
        let galley = painter.layout_no_wrap(text, font.clone(), color);
        let rect = align.anchor_size(at, galley.size());
        let rect = rect.translate(egui::vec2(
            (bounds.right() - rect.right()).min(0.0) + (bounds.left() - rect.left()).max(0.0),
            0.0,
        ));
        painter.rect_filled(
            rect.expand(4.0),
            4.0,
            egui::Color32::from_rgb(15, 17, 23).gamma_multiply(0.85 * alpha),
        );
        painter.galley(rect.min, galley, color);
    };
    if first > 0
        && let Some(at) = to_screen(timeline.at_moment(home, first_at - 0.3))
    {
        // Under the rail's start, clear of the map behind it.
        note(
            at + egui::vec2(0.0, 14.0),
            egui::Align2::LEFT_TOP,
            format!("‹ {first} earlier"),
        );
    }
    if end < count
        && let Some(at) = to_screen(timeline.at_moment(home, end_at - 0.4))
    {
        note(
            at + egui::vec2(0.0, 14.0),
            egui::Align2::LEFT_TOP,
            format!("{} later ›", count - end),
        );
    }
}

/// Names the modules the calls reach down the left edge, each level with the middle
/// of its ring on the map: from the side a module keeps its band of the map's height.
fn draw_modules(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    timeline: &Timeline,
    alpha: f32,
) {
    let graph = explorer.graph();
    // How often the calls reach each module.
    let mut visits: Vec<(usize, usize)> = Vec::new();
    for call in &timeline.calls {
        let Some(cluster) = graph.cluster(call.tile) else {
            continue;
        };
        match visits.iter_mut().find(|(c, _)| *c == cluster) {
            Some((_, count)) => *count += 1,
            None => visits.push((cluster, 1)),
        }
    }
    visits.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
    // A point in view to measure heights beside: the cursor's call, else the first.
    let Some(beside) = timeline.point(explorer, timeline.cursor.unwrap_or(0)) else {
        return;
    };
    let rings = regions(explorer);
    // Only beside the calls: a ring's middle far from all of them says little.
    let heights: Vec<f32> = (0..timeline.calls.len())
        .filter_map(|i| timeline.point(explorer, i).and_then(to_screen))
        .map(|at| at.y)
        .collect();
    let top = heights.iter().copied().fold(f32::INFINITY, f32::min) - 20.0;
    let bottom = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max) + 20.0;
    let left = painter.clip_rect().left() + 12.0;
    let mut taken: Vec<f32> = Vec::new();
    for (cluster, _) in visits {
        let Some(y) = rings
            .iter()
            .find(|(c, _, _)| *c == cluster)
            .and_then(|(_, middle, _)| to_screen(beside.with_y(middle.y)))
            .map(|at| at.y)
        else {
            continue;
        };
        if !(top..=bottom).contains(&y) || taken.iter().any(|t| (t - y).abs() < 15.0) {
            continue;
        }
        taken.push(y);
        let color =
            crate::ui::egui_color(crate::scene::lane_color(cluster)).gamma_multiply(0.75 * alpha);
        // On a backing, for when the view has panned along time under them.
        let galley = painter.layout_no_wrap(
            explorer.cluster_name(cluster).to_uppercase(),
            egui::FontId::proportional(11.0),
            color,
        );
        let at = egui::pos2(left, y - galley.size().y / 2.0);
        let backing = egui::Rect::from_min_size(at, galley.size()).expand(3.0);
        painter.rect_filled(
            backing,
            3.0,
            egui::Color32::from_rgb(15, 17, 23).gamma_multiply(0.8 * alpha),
        );
        painter.galley(at, galley, color);
    }
}

/// How far names tilt up from the horizontal, so neighbors in time fit side by side.
const LABEL_ANGLE: f32 = -0.6;

/// Whether two names tilted alike, starting at `a` and `b` and `a_length` and
/// `b_length` long, would overlap: they run parallel, so only if they start within a
/// line of each other across the tilt and their runs along it overlap.
pub(crate) fn angled_overlap(a: egui::Pos2, a_length: f32, b: egui::Pos2, b_length: f32) -> bool {
    const LINE: f32 = 13.0;
    let along = egui::vec2(LABEL_ANGLE.cos(), LABEL_ANGLE.sin());
    let across = egui::vec2(-along.y, along.x);
    let offset = b - a;
    offset.dot(across).abs() < LINE && offset.dot(along) < a_length && offset.dot(along) > -b_length
}

/// The order calls are offered a name, given each call's caller: the cursor's call,
/// its caller chain, the calls it makes, then everything else by nearness to it or
/// to `first`, where the window starts, so the first calls
/// on the rail are named as surely as the cursor's neighbors; earlier first on a tie.
pub(crate) fn label_order(
    callers: &[Option<usize>],
    cursor: Option<usize>,
    first: usize,
) -> Vec<usize> {
    let mut order: Vec<usize> = Vec::new();
    let push = |i: usize, order: &mut Vec<usize>| {
        if !order.contains(&i) {
            order.push(i);
        }
    };
    let cursor = cursor.filter(|&c| c < callers.len());
    if let Some(cursor) = cursor {
        push(cursor, &mut order);
        let mut up = callers[cursor];
        while let Some(at) = up {
            push(at, &mut order);
            up = callers[at];
        }
        for (i, caller) in callers.iter().enumerate() {
            if *caller == Some(cursor) {
                push(i, &mut order);
            }
        }
    }
    let mut rest: Vec<usize> = (0..callers.len()).collect();
    rest.sort_by_key(|&i| i.abs_diff(cursor.unwrap_or(first)).min(i.abs_diff(first)));
    for i in rest {
        push(i, &mut order);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_run_depth_first_each_hanging_from_its_caller() {
        // 0 calls 1 then 4; 1 calls 2 then 3.
        let tree = |n: u32| match n {
            0 => vec![1, 4],
            1 => vec![2, 3],
            _ => vec![],
        };
        assert_eq!(
            run_order(0, tree),
            vec![
                (0, None, 4),
                (1, Some(0), 3),
                (2, Some(1), 2),
                (3, Some(1), 3),
                (4, Some(0), 4),
            ]
        );
    }

    #[test]
    fn a_long_journey_stands_out_round_the_cursor() {
        assert_eq!(window(10, Some(9), 40), (0, 10), "short: all of it");
        assert_eq!(window(200, None, 40), (0, 40), "from the start");
        assert_eq!(window(200, Some(5), 40), (0, 40));
        let (first, end) = window(200, Some(100), 40);
        assert!(
            first < 100 && 100 < end && end - first == 40,
            "round the cursor"
        );
        assert!(100 - first < end - 100, "with more ahead than behind");
        assert_eq!(window(200, Some(199), 40), (160, 200), "never past the end");
    }

    #[test]
    fn a_function_called_twice_is_on_the_timeline_twice() {
        let tree = |n: u32| if n == 0 { vec![7, 8, 7] } else { vec![] };
        let calls: Vec<u32> = run_order(0, tree).into_iter().map(|c| c.0).collect();
        assert_eq!(calls, vec![0, 7, 8, 7]);
    }

    #[test]
    fn time_spans_the_map_but_never_crowds_its_calls() {
        assert!(
            (time_length(5, 20.0) - 60.0).abs() < 1e-3,
            "about as wide as the map"
        );
        let many = time_length(200, 20.0);
        assert!(
            many / 199.0 >= MIN_STEP - 1e-3,
            "no two calls closer than the least step"
        );
    }

    #[test]
    fn the_side_view_fits_time_but_pans_a_long_journey() {
        let short = time_distance(60.0, 20, 5.0, 1.6);
        assert!(short * (FIELD / 2.0).tan() * 1.6 > 30.0, "all of it across");
        let long = time_distance(1000.0, 400, 5.0, 1.6);
        assert!(
            long * (FIELD / 2.0).tan() * 1.6 < 200.0,
            "only part of a long one"
        );
        assert!(
            time_distance(10.0, 5, 80.0, 1.6) > time_distance(10.0, 5, 5.0, 1.6),
            "tall too"
        );
    }

    #[test]
    fn the_lens_flattens_only_near_the_side() {
        assert!(
            (lens(0.0) - 1.0).abs() < 1e-6,
            "the map keeps its perspective"
        );
        let three_d = std::f32::consts::FRAC_PI_4.sin();
        assert!((lens(three_d) - 1.0).abs() < 1e-6, "so does the 3D view");
        assert!(lens(1.0) > 7.0, "the side is near flat");
        assert!(
            lens(0.9) > 1.0 && lens(0.9) < lens(1.0),
            "eased in on the way"
        );
    }

    #[test]
    fn calls_settle_into_one_sheet_by_the_3d_view() {
        assert!(sheet(0.0) < 1e-6, "on their nodes head-on");
        let three_d = RAILS_YAW.to_radians().sin();
        assert!(
            (sheet(three_d) - 1.0).abs() < 1e-6,
            "all in the sheet in 3D"
        );
        assert!((sheet(1.0) - 1.0).abs() < 1e-6, "and from the side");
    }

    #[test]
    fn haze_thickens_only_past_where_it_starts() {
        assert!(haze(5.0, (10.0, 20.0)) < 1e-6);
        assert!((haze(15.0, (10.0, 20.0)) - 0.5).abs() < 1e-6);
        assert!((haze(50.0, (10.0, 20.0)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_call_near_the_camera_stays_small() {
        assert!(
            marker_cap(10.0, 1.0) < marker_cap(100.0, 1.0),
            "further may be wider"
        );
        let on_screen = |size: f32, distance: f32| size / (distance * (FIELD / 2.0).tan());
        assert!(
            (on_screen(marker_cap(10.0, 1.0), 10.0) - on_screen(marker_cap(80.0, 1.0), 80.0)).abs()
                < 1e-6,
            "the same share of the view at any distance"
        );
    }

    #[test]
    fn the_map_stays_a_map_until_the_side() {
        let at = |unfold: f32| {
            map_strength(&Timeline {
                unfold,
                ..default()
            })
        };
        assert!((at(0.0) - 1.0).abs() < 1e-6);
        assert!((at(0.7) - 0.55).abs() < 0.01, "a little over half in 3D");
        assert!(at(1.0) < 1e-3, "gone from the side");
    }

    #[test]
    fn a_node_lights_every_call_it_runs() {
        let tiles = [4, 7, 4, 9, 4];
        let lit: Vec<usize> = calls_of(tiles.into_iter(), 4).collect();
        assert_eq!(lit, vec![0, 2, 4]);
        assert_eq!(calls_of(tiles.into_iter(), 5).count(), 0);
    }

    #[test]
    fn the_view_follows_the_cursor_only_when_time_runs_wider_than_it() {
        assert!(time_pan(30.0, 50.0, 40.0).abs() < 1e-6, "it all fits");
        let pan = time_pan(90.0, 200.0, 40.0);
        assert!(pan > 0.0 && pan <= 90.0, "towards the cursor");
        assert!(
            time_pan(-500.0, 200.0, 40.0) >= -100.0,
            "never past the start"
        );
    }

    #[test]
    fn tilted_names_clash_only_when_they_share_a_line() {
        let a = egui::pos2(100.0, 100.0);
        assert!(
            angled_overlap(a, 80.0, egui::pos2(110.0, 100.0), 80.0),
            "side by side, level"
        );
        // Further along the tilt, on the same line: clear once past its end.
        let along = egui::vec2(LABEL_ANGLE.cos(), LABEL_ANGLE.sin());
        assert!(angled_overlap(a, 80.0, a + along * 60.0, 80.0));
        assert!(!angled_overlap(a, 80.0, a + along * 90.0, 80.0));
        assert!(
            !angled_overlap(a, 80.0, egui::pos2(100.0, 140.0), 80.0),
            "a line below"
        );
        assert!(!angled_overlap(
            a,
            80.0,
            egui::pos2(130.0, 100.0 + 30.0),
            80.0
        ));
    }

    #[test]
    fn the_cursor_and_its_neighbors_are_named_first() {
        // 0 calls 1, 3, 4; 1 calls 2.
        let calls = [None, Some(0), Some(1), Some(0), Some(0)];
        assert_eq!(label_order(&calls, Some(3), 0), vec![3, 0, 1, 2, 4]);
        assert_eq!(label_order(&calls, None, 0), vec![0, 1, 2, 3, 4]);
        // The rest by nearness to the cursor's, or to the window's start.
        let flat: Vec<Option<usize>> = std::iter::once(None)
            .chain((1..20).map(|_| Some(0)))
            .collect();
        let order = label_order(&flat, Some(12), 0);
        assert_eq!(
            &order[..8],
            &[12, 0, 1, 11, 13, 2, 10, 14],
            "the start as soon as the cursor's neighbors"
        );
    }

    #[test]
    fn a_short_name_drops_its_module() {
        assert_eq!(short_name("journey_panel.draw"), "draw");
        assert_eq!(short_name("ui::Chrome::draw"), "draw");
        assert_eq!(short_name("main"), "main");
    }
}
