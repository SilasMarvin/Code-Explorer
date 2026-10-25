//! Call order on the depth axis: pillars and a "now" plane. No node moves; the map
//! stays where the simulation puts it. Each node on the disclosed journey grows a
//! thin pillar out of the map, and each call of it is a bead on its pillar at the
//! height of the call's time, so a function called three times has three beads.
//!
//! A translucent plane parallel to the map sits at the cursor's time and moves with
//! it. The cursor's bead, on the plane, lights up and is named; beads below it, in the
//! past, are dim, and beads above it, still to come, are faint. A call lasts until it
//! returns, so the pillar is lit from its bead up to its return while the call is on
//! the stack: the plane cuts every call open now. Only a slab of a few steps either
//! side of the plane shows in full, so the pillars read as a time-lapse round now
//! rather than a forest. A ruler at the plane's corner counts the steps.
//!
//! `Z` raises and lowers the pillars, easing the camera to a gentle tilt and back;
//! [`Chrome`] holds which.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::SpanId;

use crate::{
    camera::{ANGLED, OrbitCamera},
    explorer::Explorer,
    scene::lane_color,
    ui::{Chrome, node_name, short},
};

/// A color for egui.
fn egui_color(color: Color) -> egui::Color32 {
    let [r, g, b, a] = color.to_srgba().to_u8_array();
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Whether time shows on the depth axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TimeMode {
    #[default]
    Flat,
    Pillars,
}

impl TimeMode {
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Flat => Self::Pillars,
            Self::Pillars => Self::Flat,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Pillars => "pillars",
        }
    }
}

/// The view the pillars are read from, head-on but tipped forward a little, so the map
/// still reads as the map and height shows as up the screen. Raising the pillars only
/// tips the camera to this pitch, keeping its turn.
pub(crate) const PILLARS_VIEW: (f32, f32) = (0.0, -0.5);

/// Which way the pillars grow: away from the camera, behind the map, which a camera
/// tipped forward sees as up the screen.
const UP: Vec3 = Vec3::NEG_Z;

/// The pillars as they stand, eased.
#[derive(Resource, Debug, Default)]
pub(crate) struct Pillars {
    /// How far the pillars are up, from 0 (flat) to 1.
    rise: f32,
    /// Whether the pillars were last asked up, to notice a toggle.
    up: bool,
    /// The camera's angles before the pillars went up, to go back to.
    returns_to: Option<(f32, f32)>,
    /// The disclosed journey in the order it runs.
    calls: Vec<Call>,
    /// Each call's height at full rise, eased as the cursor moves.
    heights: Vec<f32>,
    /// The cursor's call.
    cursor: Option<usize>,
    /// Where the plane stands, as a step, eased so it sweeps.
    now: f32,
    /// The plane's corners on the map, round the journey.
    plane: Option<(Vec2, Vec2)>,
    /// Where the map lies along depth under the journey: its nodes' mean depth, as
    /// the layout leaves the map not quite flat.
    ground: f32,
}

/// One call, a bead on its node's pillar.
#[derive(Debug, Clone, PartialEq)]
struct Call {
    tile: usize,
    /// The last call under it, when it returns.
    end: usize,
    /// The call's number in the journey.
    number: String,
}

pub(crate) struct PillarsPlugin;

impl Plugin for PillarsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pillars>()
            .init_gizmo_group::<PillarGizmos>()
            .add_systems(Startup, (line_widths, spawn_plane));
    }
}

/// Pillars and the ruler, drawn thin.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct PillarGizmos;

fn line_widths(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<PillarGizmos>().0.line.width = 2.0;
}

/// The "now" plane's surface.
#[derive(Component)]
pub(crate) struct NowPlane;

fn spawn_plane(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        NowPlane,
        Mesh3d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: PLANE,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
    ));
}

/// Neutral: the pillars say when, never direction or module.
const PILLAR: Color = Color::srgba(0.82, 0.86, 0.95, 0.75);
const LIT: Color = Color::srgb(1.0, 1.0, 1.0);
const PLANE: Color = Color::srgba(0.75, 0.8, 0.95, 0.01);

/// `root`, then each call under it in order, depth first, each with how deep it is.
pub(crate) fn preorder<S: Copy>(root: S, children: impl Fn(S) -> Vec<S>) -> Vec<(S, usize)> {
    let mut order = Vec::new();
    let mut stack = vec![(root, 0)];
    while let Some((next, depth)) = stack.pop() {
        order.push((next, depth));
        stack.extend(children(next).into_iter().rev().map(|c| (c, depth + 1)));
    }
    order
}

/// For each call of a depth-first order given by depth, the last call under it: it
/// returns after that one.
pub(crate) fn returns(depths: &[usize]) -> Vec<usize> {
    (0..depths.len())
        .map(|index| {
            depths[index + 1..]
                .iter()
                .position(|&d| d <= depths[index])
                .map_or(depths.len() - 1, |after| index + after)
        })
        .collect()
}

/// A call's height as a share of how far the journey spreads over the map, when it
/// is short; a long journey's steps close up so the whole stack stays within
/// `TALLEST` of that spread, all but those round the cursor, which keep up to
/// `CLOSE_SHARE` of it between them.
const STEP: f32 = 0.12;
const TALLEST: f32 = 1.0;
const CLOSE_SHARE: f32 = 0.6;

/// Steps either side of the plane that draw in full: the slab round now.
const SLAB: usize = 3;

/// The height of each of `count` calls over a journey spreading `reach`: evenly
/// apart when they fit, else wide round the `cursor` and close up elsewhere.
pub(crate) fn heights(count: usize, cursor: Option<usize>, reach: f32) -> Vec<f32> {
    let (full, tallest) = (reach * STEP, reach * TALLEST);
    let climbs = count.saturating_sub(1);
    let cursor = cursor.unwrap_or(0);
    // The climb up to call `index` is near the cursor when either end of it is.
    let near = |index: usize| index.abs_diff(cursor) <= SLAB + 1;
    let near_climbs = (1..count).filter(|&i| near(i)).count();
    #[expect(clippy::cast_precision_loss, reason = "step counts")]
    let (wide, close) = if climbs as f32 * full <= tallest {
        (full, full)
    } else {
        let even = tallest / climbs as f32;
        let wide = full.min(tallest * CLOSE_SHARE / near_climbs.max(1) as f32);
        let rest = (climbs - near_climbs).max(1) as f32;
        let close = (tallest - wide * near_climbs as f32) / rest;
        // Too few steps away from the cursor to be worth closing up: evenly apart.
        if close < wide {
            (wide, close)
        } else {
            (even, even)
        }
    };
    let mut height = 0.0;
    (0..count)
        .map(|index| {
            if index > 0 {
                height += if near(index) { wide } else { close };
            }
            height
        })
        .collect()
}

/// How present something at step `at` is with the plane at step `now`: full within
/// the slab, fading out over a step past it to a trace.
pub(crate) fn presence(at: f32, now: f32) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a small step count")]
    let beyond = (at - now).abs() - SLAB as f32;
    (1.0 - beyond).clamp(TRACE, 1.0)
}

/// What is left of the pillars outside the slab.
const TRACE: f32 = 0.08;

/// When a call happens, against now: past calls dim, those to come faint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum When {
    Past,
    Now,
    Open,
    Later,
}

/// When call `index`, returning after `end`, stands with the cursor at `cursor`: now,
/// still open under it (on the stack), past or later.
pub(crate) fn when(index: usize, end: usize, cursor: usize) -> When {
    if index == cursor {
        When::Now
    } else if index > cursor {
        When::Later
    } else if end >= cursor {
        When::Open
    } else {
        When::Past
    }
}

/// The disclosed journey in the order it runs, by node, with each call's depth;
/// calls the graph lacks are left out.
fn journey_calls(explorer: &Explorer) -> Vec<(usize, SpanId, usize, String)> {
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
    preorder(journey.root(), shown)
        .into_iter()
        .filter_map(|(span, depth)| {
            let tile = explorer.unit_id(span).and_then(|u| map.tile_of(u))?;
            let number = disclosed.number(span).unwrap_or_default().to_owned();
            Some((tile, span, depth, number))
        })
        .collect()
}

/// Raises the pillars while they are on and lowers them otherwise, sweeps the plane to
/// the cursor's time, and turns the camera to see them and back.
pub(crate) fn sweep(
    time: Res<Time>,
    chrome: Res<Chrome>,
    mut pillars: ResMut<Pillars>,
    explorer: Res<Explorer>,
    mut cameras: Query<&mut OrbitCamera>,
    mut planes: Query<
        (
            &mut Transform,
            &mut Visibility,
            &MeshMaterial3d<StandardMaterial>,
        ),
        With<NowPlane>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let pillars = &mut *pillars;
    let up = chrome.time == TimeMode::Pillars;
    if !up && pillars.rise == 0.0 && !pillars.up {
        return;
    }
    let found = journey_calls(&explorer);
    let depths: Vec<usize> = found.iter().map(|&(_, _, depth, _)| depth).collect();
    let ends = returns(&depths);
    let cursor = explorer.selected();
    pillars.cursor = found
        .iter()
        .position(|&(_, span, _, _)| Some(span) == cursor);
    pillars.calls = found
        .into_iter()
        .zip(ends)
        .map(|((tile, _, _, number), end)| Call { tile, end, number })
        .collect();
    let feet: Vec<Vec3> = pillars
        .calls
        .iter()
        .filter_map(|c| explorer.tile_point(c.tile))
        .collect();
    #[expect(clippy::cast_precision_loss, reason = "a node count")]
    let ground = feet.iter().map(|p| p.z).sum::<f32>() / feet.len().max(1) as f32;
    pillars.ground = ground;
    let points: Vec<Vec2> = feet.iter().map(|p| p.truncate()).collect();
    let (low, high) = points.iter().fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
        |(low, high), &p| (low.min(p), high.max(p)),
    );
    let reach = if points.is_empty() {
        0.0
    } else {
        (high - low).max_element().max(12.0)
    };
    let margin = Vec2::splat(reach * 0.08 + 2.0);
    pillars.plane = (!points.is_empty()).then_some((low - margin, high + margin));
    let goals = heights(pillars.calls.len(), pillars.cursor, reach);

    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    if up != pillars.up {
        pillars.up = up;
        if up {
            let (yaw, pitch) = camera.angles();
            pillars.returns_to = Some((yaw, pitch));
            camera.turn_to((yaw, PILLARS_VIEW.1));
        } else {
            camera.turn_to(pillars.returns_to.take().unwrap_or(ANGLED));
        }
    }
    let ease = 1.0 - (-time.delta_secs() * 5.0).exp();
    let goal = if up { 1.0 } else { 0.0 };
    pillars.rise += (goal - pillars.rise) * ease;
    if (goal - pillars.rise).abs() < 1e-3 {
        pillars.rise = goal;
    }
    if pillars.heights.len() == goals.len() {
        for (height, goal) in pillars.heights.iter_mut().zip(&goals) {
            *height += (goal - *height) * ease;
        }
    } else {
        pillars.heights = goals;
    }
    #[expect(clippy::cast_precision_loss, reason = "a step count")]
    let now_goal = pillars.cursor.unwrap_or(0) as f32;
    pillars.now += (now_goal - pillars.now) * ease;
    if (now_goal - pillars.now).abs() < 1e-3 {
        pillars.now = now_goal;
    }

    let showing = pillars.showing();
    for (mut transform, mut visibility, material) in &mut planes {
        *visibility = if showing {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        let Some((low, high)) = pillars.plane.filter(|_| showing) else {
            continue;
        };
        let center = (low + high) * 0.5;
        transform.translation = center.extend(pillars.ground) + UP * pillars.at(pillars.now);
        transform.scale = (high - low).extend(1.0);
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.base_color = PLANE.with_alpha(PLANE.alpha() * pillars.rise);
        }
    }
}

impl Pillars {
    /// Whether the pillars are up, or rising.
    pub(crate) fn up(&self) -> bool {
        self.up
    }

    /// How high the camera looks above the node it orbits: part way up to the plane,
    /// so the plane and the map both stay in view.
    pub(crate) fn lift(&self) -> Vec3 {
        UP * self.at(self.now) * 0.4
    }

    /// How strongly the graph's links draw: quieter while the pillars are up, so the
    /// pillars stand out over the map.
    pub(crate) fn links(&self) -> f32 {
        1.0 - 0.45 * self.rise
    }

    /// The height of call `index`, as drawn now.
    fn level(&self, index: usize) -> f32 {
        self.heights.get(index).copied().unwrap_or_default() * self.rise
    }

    /// The height at a fractional step, between the calls either side of it.
    fn at(&self, step: f32) -> f32 {
        let below = step.floor().max(0.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a small, non-negative step"
        )]
        let index = below as usize;
        let last = self.calls.len().saturating_sub(1);
        let (a, b) = (
            self.level(index.min(last)),
            self.level((index + 1).min(last)),
        );
        a + (b - a) * (step - below)
    }

    /// Whether the pillars show at all.
    fn showing(&self) -> bool {
        self.rise > 0.02 && !self.calls.is_empty()
    }
}

/// Draws each journey node's pillar out of the map, lit while one of its calls is
/// open, and the ruler with the plane's edge on it.
pub(crate) fn draw_pillars(
    pillars: Res<Pillars>,
    explorer: Res<Explorer>,
    mut gizmos: Gizmos<PillarGizmos>,
    mut bright: Gizmos,
) {
    if !pillars.showing() {
        return;
    }
    let rise = pillars.rise;
    // Every call of each node, by step.
    let mut calls: HashMap<usize, Vec<usize>> = HashMap::new();
    for (index, call) in pillars.calls.iter().enumerate() {
        calls.entry(call.tile).or_default().push(index);
    }
    // The whole pillar, as a trace: the slab's beads and their stems draw over it.
    let trace = PILLAR.with_alpha(PILLAR.alpha() * TRACE * 2.0 * rise);
    for (&tile, indices) in &calls {
        let Some(foot) = explorer.tile_point(tile) else {
            continue;
        };
        let top = pillars.level(*indices.last().unwrap_or(&0));
        if top > 0.05 {
            gizmos.line(foot, foot + UP * top, trace);
        }
    }
    // Where the cursor's call stands on the map.
    if let Some(foot) = pillars
        .calls
        .get(pillars.cursor.unwrap_or(0))
        .and_then(|c| explorer.tile_point(c.tile))
    {
        bright.circle(
            Isometry3d::from_translation(foot),
            1.6,
            LIT.with_alpha(0.5 * rise),
        );
    }
    draw_plane_edge(&pillars, &mut gizmos, &mut bright);
}

/// The plane's edge, and the ruler standing at the middle of its left edge with a tick per
/// step.
fn draw_plane_edge(pillars: &Pillars, gizmos: &mut Gizmos<PillarGizmos>, bright: &mut Gizmos) {
    let Some((low, high)) = pillars.plane else {
        return;
    };
    let (rise, now) = (pillars.rise, pillars.now);
    let level = pillars.at(now);
    let corners = [
        Vec2::new(low.x, low.y),
        Vec2::new(high.x, low.y),
        Vec2::new(high.x, high.y),
        Vec2::new(low.x, high.y),
    ];
    let edge = LIT.with_alpha(0.22 * rise);
    for (i, &a) in corners.iter().enumerate() {
        let b = corners[(i + 1) % 4];
        let ground = pillars.ground;
        bright.line(
            a.extend(ground) + UP * level,
            b.extend(ground) + UP * level,
            edge,
        );
    }
    let base = Vec2::new(low.x, f32::midpoint(low.y, high.y)).extend(pillars.ground);
    let count = pillars.calls.len();
    let top = pillars.level(count.saturating_sub(1));
    let pole = PILLAR.with_alpha(PILLAR.alpha() * 0.5 * rise);
    if top > 0.05 {
        gizmos.line(base, base + UP * top, pole);
    }
    let tick = (high - low).max_element() * 0.012;
    #[expect(clippy::cast_precision_loss, reason = "step counts")]
    for index in 0..count {
        let at = base + UP * pillars.level(index);
        let color = pole.with_alpha(pole.alpha() * presence(index as f32, now).max(0.3));
        gizmos.line(at, at - Vec3::X * tick, color);
    }
}

/// The beads, as dots that face the viewer whatever the tilt: the cursor's lit and
/// named, past ones dim, those to come faint rings; and the ruler's numbers round now.
pub(crate) fn draw_labels(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    pillars: &Pillars,
    explorer: &Explorer,
) {
    if !pillars.showing() {
        return;
    }
    let (alpha, now) = (pillars.rise, pillars.now);
    let cursor = pillars.cursor.unwrap_or(0);
    let mut lit = None;
    let mut named: Vec<egui::Rect> = Vec::new();
    for (index, call) in pillars.calls.iter().enumerate() {
        let Some(foot) = explorer.tile_point(call.tile) else {
            continue;
        };
        let Some(point) = to_screen(foot + UP * pillars.level(index)) else {
            continue;
        };
        #[expect(clippy::cast_precision_loss, reason = "a step count")]
        let present = presence(index as f32, now) * alpha;
        // A bead takes its node's color, as its bar in the journey does.
        let color = egui_color(lane_color(explorer.tile_color(call.tile)));
        let moment = when(index, call.end, cursor);
        stem(painter, to_screen, pillars, foot, point, moment, present);
        match moment {
            When::Now => {
                lit = Some((point, to_screen(foot), call));
                continue;
            }
            When::Open => {
                painter.circle_filled(point, 4.5, color.gamma_multiply(present));
                painter.circle_stroke(
                    point,
                    6.5,
                    egui::Stroke::new(1.2, egui::Color32::WHITE.gamma_multiply(0.8 * present)),
                );
            }
            When::Past => {
                painter.circle_filled(point, 4.5, color.gamma_multiply(0.8 * present));
            }
            When::Later => {
                painter.circle_stroke(
                    point,
                    4.5,
                    egui::Stroke::new(1.6, color.gamma_multiply(0.9 * present)),
                );
            }
        }
        // Its number beside it, round now and where there is room.
        if present < 0.5 * alpha || call.number.is_empty() {
            continue;
        }
        let galley = painter.layout_no_wrap(
            call.number.clone(),
            egui::FontId::monospace(10.0),
            egui::Color32::from_rgb(170, 176, 190).gamma_multiply(present),
        );
        let at = point + egui::vec2(7.0, -galley.size().y * 0.5);
        let rect = egui::Rect::from_min_size(at, galley.size());
        if named.iter().any(|r| r.intersects(rect)) {
            continue;
        }
        named.push(rect);
        painter.galley(at, galley, egui::Color32::WHITE);
    }
    draw_ruler_numbers(painter, to_screen, pillars);
    let Some((point, node, call)) = lit else {
        return;
    };
    painter.circle_filled(point, 5.5, egui::Color32::WHITE.gamma_multiply(alpha));
    painter.circle_stroke(
        point,
        9.0,
        egui::Stroke::new(1.5, egui::Color32::WHITE.gamma_multiply(0.6 * alpha)),
    );
    // Named on its pillar only when it stands clear of its node, which the map names.
    if node.is_some_and(|node| node.distance(point) < NAMED_APART) {
        return;
    }
    let name = short(&node_name(explorer, call.tile), 32);
    let text = if call.number.is_empty() {
        name
    } else {
        format!("{}  {name}", call.number)
    };
    let font = egui::FontId::proportional(13.0);
    let galley = painter.layout_no_wrap(text, font, egui::Color32::WHITE.gamma_multiply(alpha));
    let at = point + egui::vec2(13.0, -galley.size().y * 0.5);
    let back = egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(5.0, 2.0));
    painter.rect_filled(
        back,
        3.0,
        egui::Color32::from_rgb(12, 14, 20).gamma_multiply(0.85 * alpha),
    );
    painter.galley(at, galley, egui::Color32::WHITE);
}

/// The bead's stem down its pillar to its node, round now: white for the cursor's
/// call, neutral grey for the rest; an open call's lit on up to the plane.
fn stem(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    pillars: &Pillars,
    foot: Vec3,
    bead: egui::Pos2,
    moment: When,
    present: f32,
) {
    if present < 0.15 * pillars.rise {
        return;
    }
    let grey = egui::Color32::from_rgb(200, 206, 220);
    let (width, color) = match moment {
        When::Now => (2.0, egui::Color32::WHITE.gamma_multiply(0.9 * present)),
        When::Open => (1.2, grey.gamma_multiply(0.45 * present)),
        When::Past => (1.2, grey.gamma_multiply(0.35 * present)),
        When::Later => (1.0, grey.gamma_multiply(0.25 * present)),
    };
    if let Some(node) = to_screen(foot) {
        painter.line_segment([node, bead], egui::Stroke::new(width, color));
    }
    if moment == When::Open
        && let Some(plane) = to_screen(foot + UP * pillars.at(pillars.now))
    {
        painter.line_segment(
            [bead, plane],
            egui::Stroke::new(1.8, egui::Color32::WHITE.gamma_multiply(0.7 * present)),
        );
    }
}

/// How far, in pixels, the cursor's bead stands from its node before it is named.
const NAMED_APART: f32 = 40.0;

/// The journey's own numbers up the ruler, within the slab; now's brightest.
fn draw_ruler_numbers(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    pillars: &Pillars,
) {
    /// Pixels kept between numbers up the ruler.
    const ROOM: f32 = 12.0;
    let Some((low, high)) = pillars.plane else {
        return;
    };
    let (alpha, now) = (pillars.rise, pillars.now);
    let base = Vec2::new(low.x, f32::midpoint(low.y, high.y)).extend(pillars.ground);
    let at = |index: usize| to_screen(base + UP * pillars.level(index));
    let mut placed: Vec<f32> = Vec::new();
    if let Some((cursor, point)) = pillars.cursor.and_then(|c| Some((c, at(c)?))) {
        let number = &pillars.calls[cursor].number;
        painter.text(
            point - egui::vec2(8.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            if number.is_empty() { "start" } else { number },
            egui::FontId::monospace(12.0),
            egui::Color32::WHITE.gamma_multiply(alpha),
        );
        placed.push(point.y);
    }
    let grey = egui::Color32::from_rgb(150, 156, 170).gamma_multiply(alpha);
    for (index, call) in pillars.calls.iter().enumerate() {
        #[expect(clippy::cast_precision_loss, reason = "a step count")]
        let present = presence(index as f32, now);
        if present < 0.5 || call.number.is_empty() {
            continue;
        }
        let Some(point) = at(index) else {
            continue;
        };
        if placed.iter().any(|y| (y - point.y).abs() < ROOM) {
            continue;
        }
        painter.text(
            point - egui::vec2(6.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            &call.number,
            egui::FontId::monospace(10.0),
            grey.gamma_multiply(present),
        );
        placed.push(point.y);
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
        assert_eq!(
            preorder(0, tree),
            vec![(0, 0), (1, 1), (2, 2), (3, 2), (4, 1)]
        );
    }

    #[test]
    fn a_call_returns_after_the_last_call_under_it() {
        assert_eq!(returns(&[0, 1, 2, 2, 1]), vec![4, 3, 2, 3, 4]);
        assert_eq!(returns(&[0]), vec![0]);
        assert_eq!(returns(&[]), Vec::<usize>::new());
    }

    #[test]
    fn calls_are_past_open_now_or_later() {
        // The cursor at 2, under 1, under 0; 3 is 1's later call.
        let ends = [4, 3, 2, 3, 4];
        let at = |i: usize| when(i, ends[i], 2);
        assert_eq!(at(0), When::Open);
        assert_eq!(at(1), When::Open);
        assert_eq!(at(2), When::Now);
        assert_eq!(at(3), When::Later);
        assert_eq!(when(1, 1, 2), When::Past, "returned before the cursor");
    }

    #[test]
    fn long_journeys_close_their_steps_up_away_from_the_cursor() {
        let short = heights(4, Some(1), 10.0);
        let step = 10.0 * STEP;
        assert!(short.windows(2).all(|w| (w[1] - w[0] - step).abs() < 1e-5));
        let long = heights(200, Some(100), 10.0);
        assert!(long[0].abs() < 1e-6, "the first call on the map");
        assert!((long[199] - 10.0 * TALLEST).abs() < 1e-3);
        let climb = |i: usize| long[i] - long[i - 1];
        assert!(climb(101) > climb(150) * 3.0, "wide by the cursor");
        assert!(long.windows(2).all(|w| w[1] > w[0]), "always rising");
        assert_eq!(heights(0, None, 10.0), Vec::<f32>::new());
    }

    #[test]
    fn only_the_slab_round_now_shows_in_full() {
        assert!((presence(10.0, 10.0) - 1.0).abs() < 1e-6);
        assert!((presence(13.0, 10.0) - 1.0).abs() < 1e-6);
        assert!((presence(7.0, 10.0) - 1.0).abs() < 1e-6);
        assert!(presence(13.5, 10.0) < 1.0);
        assert!(
            (presence(30.0, 10.0) - TRACE).abs() < 1e-6,
            "a trace beyond"
        );
    }
}
