//! Call order as a second view of the same map. Time runs along depth: each call the
//! journey discloses gets a marker at its function's node, pushed back by when it
//! runs, evenly spaced. Head-on, the map, every marker sits on its node and nothing
//! has moved. Swing the camera a quarter turn to look along the map's x axis and depth
//! becomes the screen's horizontal: the journey reads left to right, each call at its
//! node's height, so modules keep their bands, and a function called twice is there
//! twice. Under a lens that narrows as the camera swings, the side view is near
//! orthographic, so the calls stay evenly spaced on screen.
//!
//! The markers unfold with the swing and the map's nodes, links and rings fade with
//! it, so turning back shows each call folding into its node. A call hangs from its
//! caller's lifeline, which runs along time at the caller's node for as long as it
//! runs: seen from the side, a sequence diagram; head-on, the map's own links.
//!
//! `Z` swings between the two views; [`Chrome`] holds which.

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::SpanId;

use crate::{
    camera::{FRONT, OrbitCamera},
    explorer::Explorer,
    graph_view::{
        CALLEE, CALLER, FURTHER, LEVELS, NodeMaterials, node_look, node_size, orbit_anchor,
    },
    ui::Chrome,
};

/// Which way the camera looks at call order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TimeView {
    /// The map as the simulation lays it out; time folded away.
    #[default]
    Map,
    /// Swung a quarter turn: time runs left to right.
    Side,
    /// Halfway round, to see the calls unfold from their nodes.
    Half,
}

impl TimeView {
    /// `Z` swings between the map and the side.
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Map => Self::Side,
            Self::Side | Self::Half => Self::Map,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Side => "side",
            Self::Half => "half",
        }
    }

    /// The camera's yaw and pitch for this view: looking along x, time to the right.
    fn angles(self) -> (f32, f32) {
        match self {
            Self::Map => FRONT,
            Self::Side => (-std::f32::consts::FRAC_PI_2, 0.0),
            Self::Half => (-std::f32::consts::FRAC_PI_6, -0.12),
        }
    }
}

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
    /// How far along time the camera looks, from the timeline's middle, unfolded.
    pub(crate) pan: f32,
    /// The cursor's call, by index.
    cursor: Option<usize>,
    markers: Vec<Entity>,
}

impl Timeline {
    /// Where call `index` is drawn: its node's place, pushed back along time as far
    /// as the view has unfolded.
    pub(crate) fn point(&self, explorer: &Explorer, index: usize) -> Option<Vec3> {
        let call = self.calls.get(index)?;
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
        let index = cursor.or_else(|| self.calls.iter().position(|c| c.tile == tile));
        match index {
            Some(index) => self.point(explorer, index),
            None => explorer.tile_point(tile),
        }
    }

    /// `home` moved to call `index`'s moment.
    fn at(&self, home: Vec3, index: usize) -> Vec3 {
        #[expect(clippy::cast_precision_loss, reason = "a call index")]
        let depth = self.start + self.step * index as f32;
        home.with_z(home.z + (depth - home.z) * self.unfold)
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
            .add_systems(Startup, line_widths);
    }
}

/// The timeline's lifelines and calls.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct TimeGizmos;

fn line_widths(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<TimeGizmos>().0.line.width = 1.5;
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

/// How far the camera's lens narrows as time unfolds: by the side it looks from eight
/// times as far, through an eighth of the angle, nearly flat, so calls are evenly
/// spaced however near the camera they are.
pub(crate) fn lens(unfold: f32) -> f32 {
    1.0 + 7.0 * unfold
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
    mut last: Local<Option<(TimeView, usize)>>,
    mut cameras: Query<(&mut OrbitCamera, &Transform, &Projection)>,
) {
    let Ok((mut camera, view, projection)) = cameras.single_mut() else {
        return;
    };
    let wanted = chrome.time;
    let calls = disclosed_calls(&explorer);
    let swung = last.is_none_or(|(view, _)| view != wanted);
    // Turned to time, or more or fewer calls in it: refit the view to them.
    let refit =
        wanted != TimeView::Map && last.is_none_or(|(_, count)| swung || count != calls.len());
    if swung {
        // Opening on the map keeps the default angle; back on it, the usual view.
        if last.is_some() || wanted != TimeView::Map {
            camera.turn_to(wanted.angles());
        }
        if last.is_some() && wanted == TimeView::Map {
            orbit_anchor(&explorer, &mut camera);
        }
    }
    *last = Some((wanted, calls.len()));
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

    timeline.calls = calls;
    let anchor = explorer.anchor().and_then(|t| explorer.tile_point(t));
    let Some(anchor) = anchor else {
        return;
    };
    let reach = timeline
        .calls
        .iter()
        .filter_map(|c| explorer.tile_point(c.tile))
        .map(|p| p.truncate().distance(anchor.truncate()))
        .fold(10.0, f32::max);
    let count = timeline.calls.len();
    let length = time_length(count, reach);
    #[expect(clippy::cast_precision_loss, reason = "a call count")]
    let gaps = count.saturating_sub(1).max(1) as f32;
    timeline.step = length / gaps;
    timeline.start = anchor.z - length / 2.0;
    let cursor = explorer.selected();
    timeline.cursor = timeline.calls.iter().position(|c| Some(c.span) == cursor);
    let aspect = match projection {
        Projection::Perspective(p) => p.aspect_ratio,
        _ => 1.6,
    };
    if refit {
        let height = timeline
            .calls
            .iter()
            .filter_map(|c| explorer.tile_point(c.tile))
            .map(|p| (p.y - anchor.y).abs())
            .fold(0.0, f32::max);
        let distance = time_distance(length, count, height, aspect);
        let target = camera.target;
        camera.glide_to(target, distance);
    }
    let half_width = camera.distance * (FIELD / 2.0).tan() * aspect;
    #[expect(clippy::cast_precision_loss, reason = "a call index")]
    let at = timeline
        .cursor
        .map_or(0.0, |i| timeline.step * i as f32 - length / 2.0);
    timeline.pan = time_pan(at, length, half_width) * timeline.unfold;
}

/// Calls the side view takes in across its width at most; a longer journey pans.
const CALLS_ACROSS: f32 = 60.0;

/// How far back to look from the side to see time `length` long holding `count`
/// calls, `height` above or below the start, at `aspect`, with room for names: all of
/// time when it holds no more than [`CALLS_ACROSS`] calls, else that many.
pub(crate) fn time_distance(length: f32, count: usize, height: f32, aspect: f32) -> f32 {
    let tan = (FIELD / 2.0).tan();
    #[expect(clippy::cast_precision_loss, reason = "a call count")]
    let share = (CALLS_ACROSS / count.max(1) as f32).min(1.0);
    let across = (length * share / 2.0) * 1.15 / (tan * aspect);
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
        for (tile, mut transform) in &mut nodes {
            let size = node_scale(&explorer, tile.0) * node_size(graph, tile.0);
            // What the journey runs stays a little longer, for its calls to unfold from.
            let left = if disclosed.tiles().contains_key(&tile.0) {
                1.0 - unfold
            } else {
                (1.0 - unfold).powi(3)
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
    let graph = explorer.graph();
    for (marker, mut transform, mut visibility, mut material) in &mut markers {
        let index = marker.0;
        let shown = index < wanted;
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if !shown {
            continue;
        }
        let call = timeline.calls[index];
        let Some(at) = timeline.point(&explorer, index) else {
            continue;
        };
        transform.translation = at;
        // Larger than on the map, as the side view is sparse, but clear of neighbors.
        let size = (node_size(graph, call.tile) * 1.6).min(timeline.step * 0.6);
        let size = if timeline.cursor == Some(index) {
            size * 1.3
        } else {
            size
        };
        transform.scale = Vec3::splat(size * unfold.sqrt());
        if let Some(wanted) = materials.0.get(&(node_look(&explorer, call.tile), LEVELS))
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

/// Draws each caller's lifeline along time and each call down or up from it to its
/// marker, faded in as time unfolds. Calls into and out of the cursor's call take the
/// caller and callee colors; the rest are pale.
pub(crate) fn draw_lines(
    timeline: Res<Timeline>,
    explorer: Res<Explorer>,
    mut gizmos: Gizmos<TimeGizmos>,
) {
    let unfold = timeline.unfold;
    if unfold <= 0.0 {
        return;
    }
    let graph = explorer.graph();
    let pale = FURTHER.with_alpha(0.4 * unfold);
    let string = FURTHER.with_alpha(0.35 * string_strength(unfold));
    for (index, call) in timeline.calls.iter().enumerate() {
        let Some(home) = explorer.tile_point(call.tile) else {
            continue;
        };
        let radius = node_size(graph, call.tile) * 0.5;
        // Back along time to the node it unfolded from.
        if string.alpha() > 0.02 {
            let at = timeline.at(home, index);
            if at.distance(home) > radius * 2.0 {
                dashed(&mut gizmos, home, at, radius, string);
            }
        }
        // The lifeline: from the call to the last call under it.
        if call.last > index {
            let (from, to) = (timeline.at(home, index), timeline.at(home, call.last));
            let color = if timeline.cursor == Some(index) {
                CALLEE.with_alpha(0.7 * unfold)
            } else {
                pale
            };
            gizmos.line(from, to, color);
        }
        let Some(caller) = call.caller.and_then(|c| timeline.calls.get(c)) else {
            continue;
        };
        let Some(source) = explorer.tile_point(caller.tile) else {
            continue;
        };
        // At this call's moment, from the caller's lifeline to this call's marker.
        let (from, to) = (timeline.at(source, index), timeline.at(home, index));
        let length = from.distance(to);
        if length <= radius {
            continue;
        }
        let to = from + (to - from) * ((length - radius) / length);
        let color = if timeline.cursor == Some(index) {
            CALLER.with_alpha(0.9 * unfold)
        } else if timeline.cursor == call.caller {
            CALLEE.with_alpha(0.9 * unfold)
        } else {
            pale
        };
        gizmos.line(from, to, color);
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

/// How strongly the map's links and rings draw: they fade as time unfolds.
pub(crate) fn map_strength(timeline: &Timeline) -> f32 {
    (1.0 - timeline.unfold).powi(2)
}

/// How strongly a call's string back to its node draws: most mid-swing, where it
/// says which node a call came from, and not at all on the map or from the side.
pub(crate) fn string_strength(unfold: f32) -> f32 {
    4.0 * unfold * (1.0 - unfold)
}

/// Names the calls on the timeline, angled up from each marker so neighbors fit:
/// the cursor's call first, then what makes it and what it calls, then the rest, each
/// only where it clears the names already placed.
pub(crate) fn draw_labels(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    timeline: &Timeline,
) {
    let alpha = ((timeline.unfold - 0.5) * 2.0).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    let disclosed = explorer.disclosure();
    let callers: Vec<Option<usize>> = timeline.calls.iter().map(|c| c.caller).collect();
    let (order, near) = label_order(&callers, timeline.cursor);
    // Where calls stand closer than a name is tall, only those near the cursor's are
    // named, so the names that show are the ones that say where it is.
    let pixels = |index: usize| timeline.point(explorer, index).and_then(to_screen);
    let first = timeline
        .cursor
        .unwrap_or(0)
        .min(timeline.calls.len().saturating_sub(2));
    let crowded = (first..timeline.calls.len().saturating_sub(1))
        .chain((0..first).rev())
        .find_map(|i| Some((pixels(i)?, pixels(i + 1)?)))
        .is_some_and(|(a, b)| (b.x - a.x).abs() < 16.0);
    let order = if crowded { &order[..near] } else { &order[..] };
    let mut taken: Vec<(egui::Pos2, f32)> = Vec::new();
    for &index in order {
        let call = timeline.calls[index];
        let Some(at) = timeline.point(explorer, index).and_then(to_screen) else {
            continue;
        };
        let number = disclosed.number(call.span).unwrap_or_default();
        let name = explorer.label(call.span);
        let text = if number.is_empty() {
            name
        } else {
            format!("{number}  {name}")
        };
        let current = timeline.cursor == Some(index);
        let color = if current {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_rgb(200, 205, 215)
        };
        let galley = painter.layout_no_wrap(
            crate::ui::short(&text, 36),
            egui::FontId::proportional(if current { 13.0 } else { 11.5 }),
            color.gamma_multiply(alpha),
        );
        let start = at + egui::vec2(5.0, -9.0);
        let width = galley.size().x;
        if taken
            .iter()
            .any(|&(other, length)| angled_overlap(other, length, start, width))
        {
            continue;
        }
        taken.push((start, width));
        painter.add(egui::epaint::TextShape::new(start, galley, color).with_angle(LABEL_ANGLE));
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
/// its caller chain, the calls it makes, its siblings by nearness, then everything
/// else in time order; with how many lead the list as near the cursor.
pub(crate) fn label_order(callers: &[Option<usize>], cursor: Option<usize>) -> (Vec<usize>, usize) {
    let mut order: Vec<usize> = Vec::new();
    let push = |i: usize, order: &mut Vec<usize>| {
        if !order.contains(&i) {
            order.push(i);
        }
    };
    if let Some(cursor) = cursor.filter(|&c| c < callers.len()) {
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
        let mut siblings: Vec<usize> = (0..callers.len())
            .filter(|&i| callers[i] == callers[cursor])
            .collect();
        siblings.sort_by_key(|&i| i.abs_diff(cursor));
        for i in siblings {
            push(i, &mut order);
        }
    }
    let near = order.len();
    for i in 0..callers.len() {
        push(i, &mut order);
    }
    (order, near)
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
    fn the_lens_flattens_only_as_time_unfolds() {
        assert!(
            (lens(0.0) - 1.0).abs() < 1e-6,
            "the map keeps its perspective"
        );
        assert!(lens(1.0) > lens(0.5) && lens(0.5) > 1.0);
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
        assert_eq!(label_order(&calls, Some(3)), (vec![3, 0, 4, 1, 2], 4));
        assert_eq!(label_order(&calls, None), (vec![0, 1, 2, 3, 4], 0));
    }
}
