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
    /// How far above the map the camera should look while the ribbon is up: half way
    /// up it, so the whole ribbon sits in view.
    fn middle(&self) -> f32 {
        let top = self
            .calls
            .iter()
            .map(|c| c.marker.z - c.foot.z)
            .fold(0.0, f32::max);
        top * 0.5
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

/// The view across the map while the ribbon is up: from low in front and a little to
/// the side, so the map lies back like a floor and height reads upwards.
pub(crate) const RIBBON_VIEW: (f32, f32) = (-0.35, 1.1);
/// How far back to stand, in ribbon heights, to see it whole.
const FIT: f32 = 2.4;

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
const BASE: f32 = 0.6;
const STEP: f32 = 0.3;
const TALL: f32 = 2.4;

/// How high call `index` of `count` floats, in units of the scale: evenly spaced, a
/// step apart while they fit, closer together once a long journey would rise past
/// `TALL`.
pub(crate) fn height(index: usize, count: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let (index, gaps) = (index as f32, count.saturating_sub(1).max(1) as f32);
    BASE + index * STEP.min(TALL / gaps)
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
    (index == 0 || index + 1 == count).then_some(Label::End)
}

/// How strongly the thread from call `index` to the next draws with the cursor at
/// `cursor`: in full beside it, fading with each step further, never quite gone.
pub(crate) fn thread_strength(index: usize, cursor: Option<usize>) -> f32 {
    let Some(cursor) = cursor else {
        return 0.6;
    };
    // Steps before the cursor end at or before it; steps after start at or after it.
    let away = if index < cursor {
        cursor - index - 1
    } else {
        index - cursor
    };
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let away = away as f32;
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
    if turned && up {
        // Back far enough to see the ribbon whole once it stands.
        let top = height(count.saturating_sub(1), count) * scale;
        for mut camera in &mut cameras {
            ribbon.before = Some(camera.angles());
            camera.turn_to(RIBBON_VIEW);
            let distance = camera.distance.max(top * FIT);
            let target = camera.target;
            camera.glide_to(target, distance);
        }
    }
    let graph = explorer.graph();
    ribbon.calls = calls
        .into_iter()
        .enumerate()
        .filter_map(|(i, (_, tile, number))| {
            let at = explorer.tile_point(tile)?;
            let foot = at + Vec3::Z * node_size(graph, tile) * 0.5;
            let rise = height(i, count) * scale * lift;
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
const THREAD: Color = Color::srgba(0.92, 0.94, 1.0, 0.55);
/// Drop lines: pale, and how strongly is [`drop_alphas`]'s.
const DROP: Color = Color::srgb(0.85, 0.88, 0.95);

/// How strongly a drop line draws at its marker and where it meets its node, for a
/// call `away` calls from the cursor's, of `count`. The cursor's runs all the way
/// down to the ring round its node; the rest fade out before they reach the map, and
/// the more calls there are the fainter, so the map under a long journey stays clear.
pub(crate) fn drop_alphas(away: Option<usize>, count: usize) -> (f32, f32) {
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let crowd = (24.0 / count.max(1) as f32).min(1.0);
    match away {
        Some(0) => (0.9, 0.7),
        Some(1..=NEAR) => (0.4, 0.0),
        _ => ((0.16 * crowd).max(0.06), 0.0),
    }
}

/// Draws the thread from call to call and each call's drop line to its node.
pub(crate) fn draw_thread(ribbon: Res<Ribbon>, mut gizmos: Gizmos<ThreadGizmos>) {
    if !ribbon.showing() {
        return;
    }
    let fade = ribbon.lift.clamp(0.0, 1.0);
    for (index, pair) in ribbon.calls.windows(2).enumerate() {
        let strength = thread_strength(index, ribbon.cursor) * fade;
        gizmos.line(
            pair[0].marker,
            pair[1].marker,
            THREAD.with_alpha(THREAD.alpha() * strength),
        );
    }
    for (index, call) in ribbon.calls.iter().enumerate() {
        // The cursor's function called again hangs from the same node, lit as it is.
        let away = ribbon.cursor.map(|c| {
            if ribbon.again(index) {
                0
            } else {
                c.abs_diff(index)
            }
        });
        let (top, bottom) = drop_alphas(away, ribbon.calls.len());
        gizmos.line_gradient(
            call.marker,
            call.foot,
            DROP.with_alpha(top * fade),
            DROP.with_alpha(bottom * fade),
        );
    }
    // Where the cursor's drop line lands, a ring flat on the map round its node.
    if let Some(call) = ribbon.cursor.and_then(|c| ribbon.calls.get(c)) {
        let ring = Isometry3d::from_translation(call.foot);
        gizmos.circle(ring, 1.4, Color::srgba(1.0, 1.0, 1.0, 0.8 * fade));
    }
}

/// Ribbon labels are light pills with dark text, the map's inverted, so a name over
/// the map reads as the ribbon's and not another node's.
const INK: egui::Color32 = egui::Color32::from_rgb(14, 16, 22);

/// Draws each call's marker, filled with its node's color as its bar is, and the
/// labels the cursor and its neighbors carry.
pub(crate) fn draw_markers(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    ribbon: &Ribbon,
) {
    if !ribbon.showing() {
        return;
    }
    let fade = ribbon.lift.clamp(0.0, 1.0);
    let screen: Vec<Option<egui::Pos2>> =
        ribbon.calls.iter().map(|c| to_screen(c.marker)).collect();
    // Plain markers first, so the cursor's draws over them.
    for (index, (call, at)) in ribbon.calls.iter().zip(&screen).enumerate() {
        let Some(at) = *at else { continue };
        if ribbon.cursor == Some(index) {
            continue;
        }
        let fill = egui_color(node_color(explorer, None, call.tile)).gamma_multiply(fade);
        painter.circle(
            at,
            4.0,
            fill,
            egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
        );
    }
    let placed = name_calls(painter, explorer, ribbon, &screen, fade);
    number_calls(painter, ribbon, &screen, fade, placed);
}

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
    labelled.sort_by_key(|&(i, l)| (l != Label::Cursor, l != Label::Again, l == Label::End, i));
    for (index, kind) in labelled {
        let (Some(call), Some(at)) = (ribbon.calls.get(index), screen[index]) else {
            continue;
        };
        let fill = egui_color(node_color(explorer, None, call.tile));
        match kind {
            Label::Cursor => {
                painter.circle_filled(at, 9.0, egui::Color32::from_white_alpha(40));
                painter.circle(at, 6.0, fill, egui::Stroke::new(2.0, egui::Color32::WHITE));
            }
            Label::Again => {
                painter.circle(at, 4.5, fill, egui::Stroke::new(1.5, egui::Color32::WHITE));
            }
            Label::Near | Label::End => {}
        }
        let text = match kind {
            Label::Again => call.number.clone(),
            _ if call.number.is_empty() => node_name(explorer, call.tile),
            _ => format!("{} {}", call.number, node_name(explorer, call.tile)),
        };
        let pill = match kind {
            Label::Cursor | Label::Again => egui::Color32::from_rgb(245, 247, 252),
            Label::Near => egui::Color32::from_rgb(178, 184, 198),
            Label::End => egui::Color32::from_rgb(120, 127, 142),
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
        let few: Vec<f32> = (0..5).map(|i| height(i, 5)).collect();
        assert!(few.windows(2).all(|w| w[1] > w[0]), "later is higher");
        assert!(
            (few[1] - few[0] - STEP).abs() < 1e-6,
            "a full step while they fit"
        );
        assert!(few[0] > 0.0, "the first call clears the map");
        let many: Vec<f32> = (0..200).map(|i| height(i, 200)).collect();
        assert!(many.windows(2).all(|w| w[1] > w[0]));
        assert!(many[199] <= BASE + TALL + 1e-4, "never taller than the cap");
        assert!((height(0, 1) - BASE).abs() < 1e-6, "a lone call");
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
        assert_eq!(kinds[2], None);
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
    fn only_the_cursors_drop_line_reaches_the_map() {
        assert!(drop_alphas(Some(0), 10).1 > 0.5);
        for away in [Some(1), Some(NEAR), Some(9), None] {
            assert!(drop_alphas(away, 10).1 < 1e-6, "{away:?} fades out");
        }
        assert!(
            drop_alphas(Some(1), 10).0 > drop_alphas(Some(9), 10).0,
            "neighbors brighter"
        );
        assert!(
            drop_alphas(Some(9), 70).0 < drop_alphas(Some(9), 10).0,
            "fainter in a crowd"
        );
        assert!(drop_alphas(Some(9), 1000).0 > 0.0, "never gone");
    }
}
