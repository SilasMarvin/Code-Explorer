//! Call order as depth: the disclosed journey hangs over the map as a ribbon. No node
//! moves. Each call, not each function, is a marker floating over its node at a height
//! that grows with its place in the journey, a thin drop line down to the node, and a
//! light thread joins the markers in the order they run. A function called three times
//! has three markers stacked over it, so repeats read as a column.
//!
//! `Z` raises and lowers it; while it is up the camera turns to look across the map
//! from low in front, so heights read as heights, and turns back when it comes down.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::SpanId;

use crate::{
    camera::OrbitCamera,
    cues::{self, Eye},
    explorer::Explorer,
    graph_view::{node_color, node_size},
    ui::{Chrome, egui_color, node_name},
};

/// How call order shows on the graph.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TimeMode {
    #[default]
    Off,
    /// The journey as a ribbon of calls over the map, later calls higher.
    Ribbon,
}

impl TimeMode {
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Off => Self::Ribbon,
            Self::Ribbon => Self::Off,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Ribbon => "ribbon",
        }
    }
}

/// The ribbon as it stands this frame.
#[derive(Resource, Debug, Default)]
pub(crate) struct Ribbon {
    /// How far it has risen: 0 flat on the map, 1 at full height.
    lift: f32,
    /// The camera's angles before the ribbon turned it, to turn back to.
    before: Option<(f32, f32)>,
    /// Whether it was up last frame.
    up: bool,
    /// Each call's height, in units of the scale, eased.
    heights: Vec<f32>,
    /// How tall the ribbon stood when the camera last stepped back to fit it.
    fitted: f32,
    /// The world unit heights are measured in, eased so the ribbon never jumps.
    scale: f32,
    /// The calls in the order they run, each at its marker.
    pub(crate) calls: Vec<Call>,
    /// The cursor's call, by index into `calls`.
    pub(crate) cursor: Option<usize>,
}

/// One call on the ribbon.
#[derive(Debug, Clone)]
pub(crate) struct Call {
    pub(crate) tile: usize,
    pub(crate) number: String,
    /// Where its marker floats.
    pub(crate) marker: Vec3,
    /// Where its drop line meets its node.
    pub(crate) foot: Vec3,
}

impl Ribbon {
    /// How far above the map the camera should look while the ribbon is up: a little
    /// under half way up it, so the ribbon and the map round its foot share the view.
    pub(crate) fn middle(&self) -> f32 {
        let top = self.calls.iter().map(|c| c.marker.z).fold(0.0, f32::max);
        top * 0.4
    }

    /// Where the cursor's marker floats, while the ribbon shows.
    pub(crate) fn cursor_marker(&self) -> Option<Vec3> {
        let call = self.calls.get(self.cursor?)?;
        self.showing().then_some(call.marker)
    }

    /// Whether call `index` is another call to the cursor's function.
    pub(crate) fn again(&self, index: usize) -> bool {
        let tile = |i: usize| self.calls.get(i).map(|c| c.tile);
        self.cursor
            .is_some_and(|c| c != index && tile(c).is_some() && tile(c) == tile(index))
    }

    /// Which label call `index` carries: as [`label`] says, except that other calls
    /// to the cursor's function are numbered as such.
    pub(crate) fn label_of(&self, index: usize) -> Option<Label> {
        let kind = label(index, self.calls.len(), self.cursor);
        if kind != Some(Label::Cursor) && self.again(index) {
            Some(Label::Again)
        } else {
            kind
        }
    }

    /// How far it has risen: 0 flat on the map, 1 at full height.
    pub(crate) fn lift(&self) -> f32 {
        self.lift.clamp(0.0, 1.0)
    }

    /// How strongly the map's links draw under it: quieter while it is up, so the
    /// ribbon reads over them.
    pub(crate) fn map_strength(&self) -> f32 {
        1.0 - 0.45 * self.lift()
    }

    /// Whether anything of it shows.
    pub(crate) fn showing(&self) -> bool {
        self.lift > 0.01 && !self.calls.is_empty()
    }
}

pub(crate) struct RibbonPlugin;

impl Plugin for RibbonPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Ribbon>()
            .init_gizmo_group::<ThreadGizmos>()
            .add_systems(Startup, line_widths);
    }
}

/// The ribbon's thread and drop lines, drawn thin.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct ThreadGizmos;

fn line_widths(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<ThreadGizmos>().0.line.width = 1.5;
}

/// The view across the map while the ribbon is up: a gentle tilt from in front and a
/// little to the side, so the map keeps its shape and height reads by the depth cues
/// (shadows, haze, size, the rock) more than by the angle.
pub(crate) const RIBBON_VIEW: (f32, f32) = (-0.2, 0.6);
/// The most the view tilts further for a journey bunched in a sliver of a big map,
/// where the ribbon's foot would otherwise sink into the names round it.
const BUNCHED_TILT: f32 = 0.12;

/// The view to read the ribbon from, for a journey spreading `spread` over a map
/// reaching `reach` from where it starts: the gentle tilt, a little steeper the
/// smaller a share of the map the journey covers.
pub(crate) fn ribbon_view(spread: f32, reach: f32) -> (f32, f32) {
    let share = spread / reach.max(1e-3);
    let bunched = ((0.5 - share) / 0.3).clamp(0.0, 1.0);
    (RIBBON_VIEW.0, RIBBON_VIEW.1 + BUNCHED_TILT * bunched)
}

/// How far the map showing reaches from the journey's start.
fn map_reach(explorer: &Explorer, tiles: &[usize]) -> f32 {
    let Some(start) = tiles.first().and_then(|&t| explorer.tile_point(t)) else {
        return 1.0;
    };
    (0..explorer.map().tiles.len())
        .filter(|&tile| explorer.visibility(tile) >= 0.5)
        .filter_map(|tile| explorer.tile_point(tile))
        .map(|p| p.truncate().distance(start.truncate()))
        .fold(1.0, f32::max)
}
/// How far back to stand, in ribbon heights, to see it whole.
const FIT: f32 = 2.6;

// ---- layout ----

/// The journey as it runs: `root`, then each call under it in order, depth first.
/// `children` lists the shown calls a call makes.
pub(crate) fn in_order<S: Copy>(root: S, children: impl Fn(S) -> Vec<S>) -> Vec<S> {
    let mut order = Vec::new();
    let mut stack = vec![root];
    while let Some(call) = stack.pop() {
        order.push(call);
        stack.extend(children(call).into_iter().rev());
    }
    order
}

/// The first call's height over the map, and the most a step between calls or the
/// whole ribbon rises, as shares of the scale.
const BASE: f32 = 1.0;
const STEP: f32 = 0.2;
const TALL: f32 = 1.8;

/// Steps either side of the cursor that keep their room in a long journey, and the
/// share of the ribbon's height they may take between them.
const ZOOM: usize = 4;
const ZOOM_SHARE: f32 = 0.35;

/// How many steps from the cursor's call the step from call `index` to the next is:
/// the steps into and out of it are none.
fn steps_away(index: usize, cursor: usize) -> usize {
    if index < cursor {
        cursor - index - 1
    } else {
        index - cursor
    }
}

/// How high each of `count` calls floats, in units of the scale: a step apart while
/// they fit under `TALL`. A longer journey packs closer, except the steps around the
/// cursor, which keep room enough to read, so the ribbon opens where the cursor is.
pub(crate) fn heights(count: usize, cursor: Option<usize>) -> Vec<f32> {
    let gaps = count.saturating_sub(1);
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let even = STEP.min(TALL / gaps.max(1) as f32);
    let mut steps = vec![even; gaps];
    if let Some(cursor) = cursor {
        let near: Vec<usize> = (0..gaps)
            .filter(|&i| steps_away(i, cursor) < ZOOM)
            .collect();
        let rest = gaps - near.len();
        #[expect(clippy::cast_precision_loss, reason = "small counts")]
        let (near_count, rest_count) = (near.len().max(1) as f32, rest.max(1) as f32);
        let wide = STEP.min(TALL * ZOOM_SHARE / near_count);
        let narrow = (TALL - wide * near_count) / rest_count;
        if wide > even && narrow < wide {
            steps.fill(narrow);
            for i in near {
                steps[i] = wide;
            }
        }
    }
    std::iter::once(BASE)
        .chain(steps.iter().scan(BASE, |at, step| {
            *at += step;
            Some(*at)
        }))
        .take(count)
        .collect()
}

/// How much a call's label says, if it has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Label {
    /// The cursor's call, in full.
    Cursor,
    /// Calls either side of the cursor, by number and name.
    Near,
    /// The first and last calls, so the ribbon's ends say where it runs from and to.
    End,
    /// Another call to the cursor's function, by number.
    Again,
    /// Every few calls along the rest, by number and name, so the climb is named
    /// end to end.
    Mark,
}

/// Calls named along the ribbon besides the cursor's and its ends, about.
const MARKS: usize = 6;

/// How many calls apart the marks along a ribbon of `count` calls are named.
pub(crate) fn marks_every(count: usize) -> usize {
    count.div_ceil(MARKS).max(2)
}

/// Calls either side of the cursor that are named.
const NEAR: usize = 2;

/// Which label call `index` of `count` carries with the cursor at `cursor`.
pub(crate) fn label(index: usize, count: usize, cursor: Option<usize>) -> Option<Label> {
    if cursor == Some(index) {
        return Some(Label::Cursor);
    }
    if cursor.is_some_and(|c| c.abs_diff(index) <= NEAR) {
        return Some(Label::Near);
    }
    if index == 0 || index + 1 == count {
        return Some(Label::End);
    }
    index
        .is_multiple_of(marks_every(count))
        .then_some(Label::Mark)
}

/// How strongly the thread from call `index` to the next draws with the cursor at
/// `cursor`: in full beside it, fading with each step further, never quite gone.
pub(crate) fn thread_strength(index: usize, cursor: Option<usize>) -> f32 {
    let Some(cursor) = cursor else {
        return 0.6;
    };
    // Steps before the cursor end at or before it; steps after start at or after it.
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let away = steps_away(index, cursor) as f32;
    (1.0 - away * 0.1).max(0.12)
}

/// The disclosed journey's calls in order, each with its node and number; calls whose
/// node the graph lacks are left out.
fn journey_calls(explorer: &Explorer) -> Vec<(SpanId, usize, String)> {
    let Some(journey) = explorer.journey() else {
        return Vec::new();
    };
    let disclosed = explorer.disclosure();
    let map = explorer.map();
    let shown = |span: SpanId| -> Vec<SpanId> {
        disclosed
            .children(span)
            .iter()
            .copied()
            .filter(|&c| disclosed.number(c).is_some())
            .collect()
    };
    in_order(journey.root(), shown)
        .into_iter()
        .filter_map(|span| {
            let tile = explorer.unit_id(span).and_then(|u| map.tile_of(u))?;
            let number = disclosed.number(span).unwrap_or_default().to_owned();
            Some((span, tile, number))
        })
        .collect()
}

/// The world unit heights are measured in: how far the journey spreads over the map
/// from where it starts, so the ribbon stands about as tall as the journey is wide.
fn journey_spread(explorer: &Explorer, tiles: &[usize]) -> f32 {
    let Some(start) = tiles.first().and_then(|&t| explorer.tile_point(t)) else {
        return 10.0;
    };
    tiles
        .iter()
        .filter_map(|&t| explorer.tile_point(t))
        .map(|p| p.truncate().distance(start.truncate()))
        .fold(6.0, f32::max)
}

/// Raises and lowers the ribbon as `Z` asks, turns the camera to read it and back, and
/// lays its calls out over their nodes.
pub(crate) fn rise(
    time: Res<Time>,
    chrome: Res<Chrome>,
    explorer: Res<Explorer>,
    mut ribbon: ResMut<Ribbon>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let up = chrome.time == TimeMode::Ribbon;
    let turned = up != ribbon.up;
    ribbon.up = up;
    if turned && !up {
        for mut camera in &mut cameras {
            if let Some(before) = ribbon.before.take() {
                camera.turn_to(before);
            }
        }
    }
    let ease = 1.0 - (-time.delta_secs() * 4.0).exp();
    let goal = if up { 1.0 } else { 0.0 };
    ribbon.lift += (goal - ribbon.lift) * ease;
    if !up && ribbon.lift < 0.005 {
        ribbon.lift = 0.0;
        ribbon.calls.clear();
        ribbon.cursor = None;
        return;
    }
    let calls = journey_calls(&explorer);
    let tiles: Vec<usize> = calls.iter().map(|&(_, tile, _)| tile).collect();
    let spread = journey_spread(&explorer, &tiles);
    ribbon.scale = if ribbon.scale > 0.0 {
        ribbon.scale + (spread - ribbon.scale) * ease
    } else {
        spread
    };
    let index: HashMap<SpanId, usize> = calls
        .iter()
        .enumerate()
        .map(|(i, &(span, _, _))| (span, i))
        .collect();
    // The cursor's call, or the nearest one shown that it runs under.
    ribbon.cursor = explorer.selected().and_then(|span| {
        explorer
            .stack(span)
            .into_iter()
            .rev()
            .find_map(|s| index.get(&s).copied())
    });
    let (scale, lift, count) = (ribbon.scale, ribbon.lift, calls.len());
    // Back far enough to see the ribbon whole once it stands, and again when more of
    // the journey opens and it grows, unless the view was moved by hand.
    let goals = heights(count, ribbon.cursor);
    let top = goals.last().copied().unwrap_or(BASE) * spread;
    let grown = top > ribbon.fitted * 1.1;
    if up && (turned || grown) {
        for mut camera in &mut cameras {
            if turned {
                ribbon.before = Some(camera.angles());
                camera.turn_to(ribbon_view(spread, map_reach(&explorer, &tiles)));
            } else if camera.free {
                continue;
            }
            let distance = camera.goal_distance().max(top * FIT);
            let target = camera.target;
            camera.glide_to(target, distance);
        }
    }
    if up {
        ribbon.fitted = top;
    }
    // Heights ease, so as the cursor moves the ribbon opens round it smoothly.
    if ribbon.heights.len() == count {
        for (now, goal) in ribbon.heights.iter_mut().zip(&goals) {
            *now += (goal - *now) * ease;
        }
    } else {
        ribbon.heights = goals;
    }
    let graph = explorer.graph();
    let eased = &ribbon.heights;
    let placed: Vec<Call> = calls
        .into_iter()
        .enumerate()
        .filter_map(|(i, (_, tile, number))| {
            let at = explorer.tile_point(tile)?;
            let foot = at + Vec3::Z * node_size(graph, tile) * 0.5;
            let rise = eased[i] * scale * lift;
            // Heights count from the map's plane, so calls at one time share a level.
            let marker = at.with_z(foot.z.max(rise));
            Some(Call {
                tile,
                number,
                marker,
                foot,
            })
        })
        .collect();
    ribbon.calls = placed;
}

/// While the ribbon is up, the camera looks half way up it rather than at the map.
pub(crate) fn lift_view(
    explorer: Res<Explorer>,
    ribbon: Res<Ribbon>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    if !ribbon.showing() || explorer.anchor().is_none() {
        return;
    }
    let raise = ribbon.middle();
    for mut camera in &mut cameras {
        if !camera.free {
            camera.target.z += raise;
        }
    }
}

// ---- drawing ----

/// The thread: pale, so it says nothing about direction to the focus or module.
const THREAD: egui::Color32 = egui::Color32::from_rgb(232, 236, 248);
/// The cursor's drop line: pale.
const DROP: Color = Color::srgb(0.85, 0.88, 0.95);

/// Where light comes from: from over the map, a little to the left and from its far
/// side, so markers light on top and their shadows fall under them.
const LIGHT: Vec3 = Vec3::new(-0.45, 0.5, 1.0);

/// How strongly a drop line draws at its marker and where it meets its node, for a
/// call `away` calls from the cursor's: the cursor's reaches its node, its neighbors'
/// fade out on the way, and the rest have none, their footprints joining them to
/// their nodes instead.
pub(crate) fn drop_alphas(away: Option<usize>) -> (f32, f32) {
    match away {
        Some(0) => (0.55, 0.3),
        Some(1..=NEAR) => (0.22, 0.0),
        _ => (0.0, 0.0),
    }
}

/// Draws drop lines from the calls round the cursor to their nodes, and a ring flat
/// on the map round the cursor's node, where its footprint lies.
pub(crate) fn draw_thread(ribbon: Res<Ribbon>, mut gizmos: Gizmos<ThreadGizmos>) {
    if !ribbon.showing() {
        return;
    }
    let fade = ribbon.lift.clamp(0.0, 1.0);
    for (index, call) in ribbon.calls.iter().enumerate() {
        // The cursor's function called again hangs from the same node, lit as it is.
        let away = ribbon.cursor.map(|c| {
            if ribbon.again(index) {
                0
            } else {
                c.abs_diff(index)
            }
        });
        let (top, bottom) = drop_alphas(away);
        if top > 0.0 {
            gizmos.line_gradient(
                call.marker,
                call.foot,
                DROP.with_alpha(top * fade),
                DROP.with_alpha(bottom * fade),
            );
        }
    }
    if let Some(call) = ribbon.cursor.and_then(|c| ribbon.calls.get(c)) {
        let ring = Isometry3d::from_translation(call.foot);
        gizmos.circle(ring, 1.4, Color::srgba(1.0, 1.0, 1.0, 0.8 * fade));
    }
}

/// How high call `index` floats, as a share of the ribbon's height.
pub(crate) fn rise_share(ribbon: &Ribbon, index: usize) -> f32 {
    let top = ribbon
        .calls
        .iter()
        .map(|c| c.marker.z - c.foot.z)
        .fold(1e-3, f32::max);
    ribbon
        .calls
        .get(index)
        .map_or(0.0, |c| ((c.marker.z - c.foot.z) / top).clamp(0.0, 1.0))
}

/// How big a call's footprint is, as a share of its marker's size, and how bright at
/// its middle, for a call `rise` of the way up the ribbon: the higher it floats the
/// wider and fainter its footprint, as under a broad light.
pub(crate) fn shadow_shape(rise: f32) -> (f32, f32) {
    (1.8 + 2.6 * rise, 0.9 - 0.45 * rise)
}

/// How many pixels across a marker's ball is at `nearness`, of `count` calls, the
/// cursor's bigger.
pub(crate) fn marker_radius(nearness: f32, cursor: bool, count: usize) -> f32 {
    // A long journey's balls are smaller, so a crowd of them doesn't clump.
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let crowd = (24.0 / count.max(1) as f32).sqrt().clamp(0.5, 1.0);
    let base = if cursor { 10.0 } else { 7.0 * crowd };
    (base * nearness).clamp(base * 0.85, base * 2.0)
}

/// Draws the thread from call to call, each call's marker as a ball lit from above,
/// filled with its node's color as its bar is, and the labels the cursor and its
/// neighbors carry. Nearer markers are bigger and cover farther ones; haze dims them
/// with distance.
pub(crate) fn draw_markers(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    ribbon: &Ribbon,
    eye: Eye,
) {
    if !ribbon.showing() {
        return;
    }
    let fade = ribbon.lift.clamp(0.0, 1.0);
    let backdrop = egui_color(cues::BACKDROP);
    let screen: Vec<Option<egui::Pos2>> =
        ribbon.calls.iter().map(|c| to_screen(c.marker)).collect();
    for (index, pair) in ribbon.calls.windows(2).enumerate() {
        let (Some(a), Some(b)) = (screen[index], screen[index + 1]) else {
            continue;
        };
        let middle = pair[0].marker.lerp(pair[1].marker, 0.5);
        let width = (1.4 * eye.nearness(middle)).clamp(0.8, 3.5);
        // Faded towards the background rather than made see-through, which over the
        // overlay darkens a pale line instead of thinning it.
        let strength = 0.3 + 0.5 * thread_strength(index, ribbon.cursor);
        let color = cues::mix(THREAD, backdrop, 1.0 - strength * (1.0 - eye.haze(middle)))
            .gamma_multiply(fade);
        painter.line_segment([a, b], egui::Stroke::new(width, color));
    }
    // Farthest first, so nearer balls cover them; the cursor's over all.
    let mut order: Vec<usize> = (0..ribbon.calls.len()).collect();
    order.sort_by(|&a, &b| {
        let near = |i: usize| eye.nearness(ribbon.calls[i].marker);
        near(a).total_cmp(&near(b))
    });
    order.sort_by_key(|&i| ribbon.cursor == Some(i));
    for index in order {
        let (call, Some(at)) = (&ribbon.calls[index], screen[index]) else {
            continue;
        };
        let cursor = ribbon.cursor == Some(index);
        let radius = marker_radius(eye.nearness(call.marker), cursor, ribbon.calls.len());
        let light = to_screen(call.marker + LIGHT.normalize())
            .map_or(egui::vec2(-0.6, -0.8), |p| (p - at).normalized());
        // Far along the journey from the cursor, a ball steps back into the haze, as
        // its thread fades.
        let near = ribbon.cursor.map_or(1.0, |c| {
            let away = if index < c {
                index
            } else {
                index.saturating_sub(1)
            };
            0.55 + 0.45 * thread_strength(away, Some(c))
        });
        let haze = 1.0 - (1.0 - eye.haze(call.marker)) * near;
        let fill = egui_color(node_color(explorer, None, call.tile));
        let fill = cues::mix(fill, backdrop, haze);
        if cursor {
            painter.circle_filled(at, radius + 4.0, egui::Color32::from_white_alpha(36));
        }
        cues::sphere(painter, at, radius, fill, light, fade);
        if cursor || ribbon.again(index) {
            let width = if cursor { 2.0 } else { 1.5 };
            painter.circle_stroke(
                at,
                radius + 1.0,
                egui::Stroke::new(width, egui::Color32::WHITE.gamma_multiply(fade)),
            );
        }
    }
    let placed = name_calls(painter, explorer, ribbon, &screen, fade);
    number_calls(painter, ribbon, &screen, fade, placed);
}

/// Ribbon labels are light pills with dark text, the map's inverted, so a name over
/// the map reads as the ribbon's and not another node's.
const INK: egui::Color32 = egui::Color32::from_rgb(14, 16, 22);

/// Names the calls [`Ribbon::label_of`] picks, the cursor's first so it always wins,
/// each on a light pill right of its marker, else left, else not at all. Returns
/// where the pills went.
fn name_calls(
    painter: &egui::Painter,
    explorer: &Explorer,
    ribbon: &Ribbon,
    screen: &[Option<egui::Pos2>],
    fade: f32,
) -> Vec<egui::Rect> {
    let mut placed: Vec<egui::Rect> = Vec::new();
    let mut labelled: Vec<(usize, Label)> = (0..ribbon.calls.len())
        .filter_map(|i| ribbon.label_of(i).map(|l| (i, l)))
        .collect();
    // The cursor's first, then its function's other calls, its neighbors, the ends,
    // and the marks between where room is left.
    let rank = |l: Label| match l {
        Label::Cursor => 0,
        Label::Again => 1,
        Label::Near => 2,
        Label::End => 3,
        Label::Mark => 4,
    };
    labelled.sort_by_key(|&(i, l)| (rank(l), i));
    for (index, kind) in labelled {
        let (Some(call), Some(at)) = (ribbon.calls.get(index), screen[index]) else {
            continue;
        };
        let text = match kind {
            Label::Again => call.number.clone(),
            _ if call.number.is_empty() => node_name(explorer, call.tile),
            _ => format!("{} {}", call.number, node_name(explorer, call.tile)),
        };
        let pill = match kind {
            Label::Cursor | Label::Again => egui::Color32::from_rgb(245, 247, 252),
            Label::Near => egui::Color32::from_rgb(178, 184, 198),
            Label::End | Label::Mark => egui::Color32::from_rgb(120, 127, 142),
        };
        let size = if kind == Label::Cursor { 13.0 } else { 11.0 };
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(size), INK);
        let pad = egui::vec2(4.0, 1.5);
        let gap = if kind == Label::Cursor { 11.0 } else { 8.0 };
        let width = galley.size().x + pad.x * 2.0;
        let spot = [at.x + gap, at.x - gap - width].into_iter().find_map(|x| {
            let pill = egui::Rect::from_min_size(
                egui::pos2(x, at.y - galley.size().y / 2.0 - pad.y),
                galley.size() + pad * 2.0,
            );
            (kind == Label::Cursor || !placed.iter().any(|p| p.intersects(pill))).then_some(pill)
        });
        let Some(spot) = spot else { continue };
        placed.push(spot);
        painter.rect_filled(spot, 3.0, pill.gamma_multiply(fade));
        painter.galley(spot.min + pad, galley, INK);
    }
    placed
}

/// Every call left unnamed carries its number alone, as its bar does, nearest the
/// cursor first, where there is room.
fn number_calls(
    painter: &egui::Painter,
    ribbon: &Ribbon,
    screen: &[Option<egui::Pos2>],
    fade: f32,
    mut placed: Vec<egui::Rect>,
) {
    let mut rest: Vec<usize> = (0..ribbon.calls.len())
        .filter(|&i| ribbon.label_of(i).is_none())
        .collect();
    rest.sort_by_key(|&i| ribbon.cursor.map_or(i, |c| c.abs_diff(i)));
    let tag = egui::Color32::from_rgb(196, 202, 216).gamma_multiply(fade);
    for index in rest {
        let (Some(call), Some(at)) = (ribbon.calls.get(index), screen[index]) else {
            continue;
        };
        if call.number.is_empty() {
            continue;
        }
        let galley =
            painter.layout_no_wrap(call.number.clone(), egui::FontId::proportional(10.0), tag);
        let spot = egui::Rect::from_min_size(
            egui::pos2(at.x + 6.0, at.y - galley.size().y / 2.0),
            galley.size(),
        );
        if placed.iter().any(|p| p.intersects(spot)) {
            continue;
        }
        placed.push(spot);
        painter.galley(spot.min, galley, tag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_journey_runs_depth_first() {
        // 0 calls 1 then 4; 1 calls 2 then 3.
        let tree = |n: u32| match n {
            0 => vec![1, 4],
            1 => vec![2, 3],
            _ => vec![],
        };
        assert_eq!(in_order(0, tree), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn later_calls_float_higher_and_long_journeys_stay_in_bounds() {
        let few = heights(5, Some(2));
        assert!(few.windows(2).all(|w| w[1] > w[0]), "later is higher");
        assert!(
            (few[1] - few[0] - STEP).abs() < 1e-6,
            "a full step while they fit"
        );
        assert!(few[0] > 0.0, "the first call clears the map");
        let many = heights(200, Some(100));
        assert!(many.windows(2).all(|w| w[1] > w[0]));
        assert!(many[199] <= BASE + TALL + 1e-4, "never taller than the cap");
        assert_eq!(heights(1, None), vec![BASE], "a lone call");
        assert_eq!(heights(0, None), Vec::<f32>::new());
    }

    #[test]
    fn a_long_journey_opens_round_the_cursor() {
        let many = heights(200, Some(100));
        let step = |i: usize| many[i + 1] - many[i];
        assert!(step(100) > step(10) * 4.0, "room round the cursor");
        assert!((step(99) - step(100)).abs() < 1e-6, "either side alike");
        let still = heights(200, None);
        assert!(
            (still[1] - still[0] - (still[199] - still[198])).abs() < 1e-6,
            "evenly without a cursor"
        );
        let short = heights(12, Some(5));
        let gaps: Vec<f32> = short.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            gaps.iter().all(|g| (g - gaps[0]).abs() < 1e-6),
            "a journey that fits stays even"
        );
    }

    #[test]
    fn a_function_called_again_is_picked_out_with_the_cursor() {
        // Calls to nodes 4, 7, 4, 9, 4, with the cursor on the second call to 4.
        let calls = [4, 7, 4, 9, 4]
            .into_iter()
            .map(|tile| Call {
                tile,
                number: String::new(),
                marker: Vec3::ZERO,
                foot: Vec3::ZERO,
            })
            .collect();
        let ribbon = Ribbon {
            calls,
            cursor: Some(2),
            ..default()
        };
        let again: Vec<bool> = (0..5).map(|i| ribbon.again(i)).collect();
        assert_eq!(again, vec![true, false, false, false, true]);
    }

    #[test]
    fn the_cursor_and_its_neighbors_are_named() {
        let kinds: Vec<Option<Label>> = (0..10).map(|i| label(i, 10, Some(5))).collect();
        assert_eq!(kinds[5], Some(Label::Cursor));
        assert_eq!(kinds[3], Some(Label::Near));
        assert_eq!(kinds[7], Some(Label::Near));
        assert_eq!(kinds[1], None);
        assert_eq!(
            kinds[2],
            Some(Label::Mark),
            "every other call in a short one"
        );
        assert_eq!(kinds[0], Some(Label::End));
        assert_eq!(kinds[9], Some(Label::End));
        assert_eq!(
            label(0, 10, Some(0)),
            Some(Label::Cursor),
            "the cursor wins"
        );
    }

    #[test]
    fn the_thread_fades_away_from_the_cursor() {
        assert!((thread_strength(4, Some(5)) - 1.0).abs() < 1e-6, "into it");
        assert!(
            (thread_strength(5, Some(5)) - 1.0).abs() < 1e-6,
            "out of it"
        );
        assert!(thread_strength(9, Some(5)) < thread_strength(8, Some(5)));
        assert!(thread_strength(0, Some(5)) < 1.0, "behind it too");
        assert!(thread_strength(400, Some(5)) > 0.0, "never quite gone");
    }

    #[test]
    fn higher_calls_leave_wider_fainter_footprints() {
        let (low_size, low_dark) = shadow_shape(0.0);
        let (high_size, high_dark) = shadow_shape(1.0);
        assert!(high_size > low_size);
        assert!(high_dark < low_dark);
        assert!(high_dark > 0.0, "still there");
    }

    #[test]
    fn only_the_cursor_and_its_neighbors_drop_to_the_map() {
        assert!(
            drop_alphas(Some(0)).1 > 0.0,
            "the cursor's reaches its node"
        );
        assert!(drop_alphas(Some(1)).0 > 0.0);
        assert!(drop_alphas(Some(1)).1 < 1e-6, "a neighbor's fades out");
        assert!(
            drop_alphas(Some(NEAR + 1)).0 < 1e-6,
            "the rest have footprints"
        );
        assert!(drop_alphas(None).0 < 1e-6);
    }

    #[test]
    fn a_crowd_of_balls_is_smaller_but_not_the_cursors() {
        assert!(marker_radius(1.0, false, 200) < marker_radius(1.0, false, 10));
        assert!((marker_radius(1.0, true, 200) - marker_radius(1.0, true, 10)).abs() < 1e-6);
        assert!(
            marker_radius(1.5, false, 10) > marker_radius(1.0, false, 10),
            "nearer, bigger"
        );
    }

    #[test]
    fn marks_name_the_climb_end_to_end() {
        assert_eq!(marks_every(19), 4);
        assert_eq!(marks_every(5), 2, "never every call");
        let marked: Vec<usize> = (0..19)
            .filter(|&i| label(i, 19, Some(10)) == Some(Label::Mark))
            .collect();
        assert_eq!(marked, vec![4, 16], "8 and 12 are the cursor's neighbors");
    }

    #[test]
    fn a_bunched_journey_tilts_the_view_a_little_more() {
        let (_, spread_out) = ribbon_view(60.0, 100.0);
        let (_, bunched) = ribbon_view(10.0, 100.0);
        assert!((spread_out - RIBBON_VIEW.1).abs() < 1e-6, "the gentle tilt");
        assert!(bunched > spread_out);
        assert!(
            bunched <= RIBBON_VIEW.1 + BUNCHED_TILT + 1e-6,
            "never much more"
        );
    }
}
