//! Call order in depth, as a spiral staircase. No node moves: the calls the cursor
//! steps through (those of its caller, or the root's) stand as markers on a helix
//! whose axis rises out of the caller's node towards the viewer. Call `k` sits at
//! `k` o'clock, one hour a call, and `k` steps up, so head-on the helix reads as a
//! clock face, first call at one o'clock, and tilted it becomes a stair, its height
//! the time. A list longer than twelve winds round again, a little wider, so its
//! turns part on the face as well as in height.
//!
//! Each marker is its callee's color with its number and short name, and a thin line
//! runs from it down to the callee's real node on the map: pale, white for the
//! cursor's call.
//!
//! [`Chrome`] holds whether it shows; [`Helix`] eases it up and down.

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::SpanId;

use crate::{
    camera::OrbitCamera,
    explorer::Explorer,
    scene::{PALETTE, lane_color},
    ui::{Chrome, egui_color, node_name},
};

/// How call order shows in depth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TimeMode {
    #[default]
    Off,
    Helix,
}

impl TimeMode {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Off => Self::Helix,
            Self::Helix => Self::Off,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Helix => "helix",
        }
    }
}

/// The helix as it stands.
#[derive(Resource, Debug)]
pub(crate) struct Helix {
    /// How far it has risen: 0 flat on the map, 1 at full height.
    rise: f32,
    /// Whether it was on last frame, to notice the toggle.
    on: bool,
    /// The camera's angles before the helix turned it, to turn back to.
    before: Option<(f32, f32)>,
    /// The camera's turn while the helix shows.
    pub(crate) tilt: (f32, f32),
    /// Each call's marker, in order, for the labels.
    pub(crate) markers: Vec<Marker>,
    /// Whose calls it holds, and how far it has grown since that last changed, so a
    /// new list winds out from its caller rather than jumping into place.
    caller: Option<usize>,
    grown: f32,
    /// Where the helix stands, and which of a long list it shows.
    pub(crate) hub: Option<Vec3>,
    pub(crate) note: Option<String>,
}

impl Helix {
    /// A helix that turns the camera to `pitch` from below while it shows, or
    /// head-on.
    pub(crate) fn tilted(pitch: Option<f32>) -> Self {
        Self {
            tilt: pitch.map_or(TILT, |p| (0.0, p)),
            ..default()
        }
    }
}

impl Default for Helix {
    fn default() -> Self {
        Self {
            rise: 0.0,
            on: false,
            before: None,
            tilt: TILT,
            markers: Vec::new(),
            caller: None,
            grown: 1.0,
            hub: None,
            note: None,
        }
    }
}

/// One call on the helix.
#[derive(Debug, Clone)]
pub(crate) struct Marker {
    pub(crate) at: Vec3,
    /// The axis at the marker's height, so its name can lean away from it.
    pub(crate) axis: Vec3,
    pub(crate) number: String,
    pub(crate) name: String,
    pub(crate) color: Color,
    pub(crate) cursor: bool,
    /// Whether it stands on the cursor's own small helix.
    pub(crate) small: bool,
    /// The callee's node.
    pub(crate) tile: usize,
}

/// What the markers took on screen, and whose calls they are, for the map's names.
#[derive(Debug, Default)]
pub(crate) struct Drawn {
    pub(crate) rects: Vec<egui::Rect>,
    pub(crate) tiles: Vec<usize>,
}

pub(crate) struct HelixPlugin;

impl Plugin for HelixPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Helix>()
            .init_gizmo_group::<StringGizmos>()
            .init_gizmo_group::<RailGizmos>()
            .init_gizmo_group::<CursorGizmos>()
            .add_systems(Startup, line_widths);
    }
}

/// The face, the axis, the treads and the strings down to the map.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct StringGizmos;

/// The rail the calls stand on, drawn heavier than the map's links so the order
/// reads through them.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct RailGizmos;

/// The cursor's string, drawn thicker.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct CursorGizmos;

fn line_widths(mut store: ResMut<GizmoConfigStore>) {
    let set = |line: &mut GizmoLineConfig, width: f32| {
        line.width = width;
        line.joints = GizmoLineJoint::Round(4);
    };
    set(&mut store.config_mut::<StringGizmos>().0.line, 1.2);
    set(&mut store.config_mut::<CursorGizmos>().0.line, 2.6);
    set(&mut store.config_mut::<RailGizmos>().0.line, 2.2);
}

/// Neutral, so it says nothing of direction or module.
const LINE: Color = Color::srgb(0.86, 0.89, 0.95);

/// The camera's turn while the helix shows: head-on, so the helix reads as a clock
/// face and the map keeps its shape. Orbiting off it shows the stair; a tilt, even a
/// gentle one, slides the later turns over the earlier on a busy graph.
const TILT: (f32, f32) = (0.0, 0.0);

/// Calls a turn: one an hour.
pub(crate) const PER_TURN: usize = 12;

/// The face's radius, as a share of the camera's distance, so it keeps its size on
/// screen however far the view is zoomed out.
const RADIUS: f32 = 0.15;

/// How much wider each further turn winds, as a share of the first's radius.
const WIDENING: f32 = 0.15;

/// How far each call steps up, as a share of the camera's distance, and how high
/// the stair stands at most.
const STEP: f32 = 0.05;
const HEIGHT: f32 = 0.25;

/// Where call `index` (from 0) of `count` stands on a helix of unit radius and unit
/// step: its angle clockwise from twelve o'clock, its radius and its height. Calls
/// past a turn wind round wider.
pub(crate) fn place(index: f32, count: usize) -> (f32, f32, f32) {
    let hour = index + 1.0;
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let turn = hour / PER_TURN as f32;
    let widening = if count > PER_TURN { WIDENING } else { 0.0 };
    // One turn for the first twelve, wider from there on.
    let radius = 1.0 + widening * (turn - 1.0 / PER_TURN as f32).max(0.0);
    (turn * std::f32::consts::TAU, radius, hour)
}

/// Where on a face of `radius` and `step`, standing on `base`, a call at `index` of
/// `count` sits: twelve o'clock is the world's up, the map's +y.
pub(crate) fn point(base: Vec3, radius: f32, step: f32, index: f32, count: usize) -> Vec3 {
    let (angle, wide, high) = place(index, count);
    base + Vec3::new(angle.sin(), angle.cos(), 0.0) * radius * wide + Vec3::Z * step * high
}

/// One call to place: its caller's node, its callee's, its span, its caller's span
/// and its number.
struct Call {
    from: usize,
    to: usize,
    span: SpanId,
    caller: SpanId,
    number: String,
}

/// The calls under `root` in the order they run, depth first, each with its caller.
/// `children` lists the calls a call makes, in order.
pub(crate) fn calls_in_order<S: Copy>(root: S, children: impl Fn(S) -> Vec<S>) -> Vec<(S, S)> {
    let mut calls = Vec::new();
    let mut stack: Vec<(Option<S>, S)> = vec![(None, root)];
    while let Some((caller, call)) = stack.pop() {
        if let Some(caller) = caller {
            calls.push((caller, call));
        }
        stack.extend(children(call).into_iter().rev().map(|c| (Some(call), c)));
    }
    calls
}

/// The disclosed journey's calls in the order they run, between nodes the graph has.
fn journey_calls(explorer: &Explorer) -> Vec<Call> {
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
    let tile = |span: SpanId| explorer.unit_id(span).and_then(|u| map.tile_of(u));
    calls_in_order(journey.root(), shown)
        .into_iter()
        .filter_map(|(caller, call)| {
            Some(Call {
                from: tile(caller)?,
                to: tile(call)?,
                span: call,
                caller,
                number: disclosed.number(call)?.to_owned(),
            })
        })
        .collect()
}

/// The calls the cursor steps through: those its caller makes, or the root's when it
/// is on none, with which of them it is on. A cursor inside a call that is drawn as
/// none of its own, as in a closure, counts as on that call.
fn in_step<'a>(explorer: &Explorer, calls: &'a [Call]) -> (Vec<&'a Call>, Option<usize>) {
    let Some(journey) = explorer.journey() else {
        return (Vec::new(), None);
    };
    let on = std::iter::successors(explorer.selected(), |&s| journey.span(s)?.parent)
        .find(|&s| calls.iter().any(|c| c.span == s));
    let caller = on
        .and_then(|s| calls.iter().find(|c| c.span == s))
        .map_or(journey.root(), |c| c.caller);
    let listed: Vec<&Call> = calls.iter().filter(|c| c.caller == caller).collect();
    let at = on.and_then(|s| listed.iter().position(|c| c.span == s));
    (listed, at)
}

/// Eases the helix up while it shows and down when it doesn't, and turns the camera
/// as it comes on, back as it goes.
pub(crate) fn rise(
    time: Res<Time>,
    chrome: Res<Chrome>,
    mut helix: ResMut<Helix>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let on = chrome.time == TimeMode::Helix;
    if on != helix.on {
        helix.on = on;
        for mut camera in &mut cameras {
            if on {
                helix.before = Some(camera.angles());
                camera.turn_to(helix.tilt);
            } else if let Some(before) = helix.before.take() {
                camera.turn_to(before);
            }
        }
    }
    let goal = if on { 1.0 } else { 0.0 };
    let ease = 1.0 - (-time.delta_secs() * 4.0).exp();
    helix.rise += (goal - helix.rise) * ease;
    if (goal - helix.rise).abs() < 1e-3 {
        helix.rise = goal;
    }
}

/// Stands the cursor's list of calls on a helix over their caller's node, joins each
/// to its callee on the map, and notes the markers for the labels. The calls the
/// cursor's own call makes, where the journey shows them, wind round its marker on a
/// small helix of their own.
pub(crate) fn draw_helix(
    time: Res<Time>,
    explorer: Res<Explorer>,
    mut helix: ResMut<Helix>,
    cameras: Query<&OrbitCamera>,
    mut strings: Gizmos<StringGizmos>,
    mut rails: Gizmos<RailGizmos>,
    mut bright: Gizmos<CursorGizmos>,
) {
    helix.markers.clear();
    helix.hub = None;
    if helix.rise <= 0.0 {
        return;
    }
    let Ok(camera) = cameras.single() else {
        return;
    };
    let all = journey_calls(&explorer);
    let (mut calls, mut cursor) = in_step(&explorer, &all);
    // A long list shows the turns round the cursor's, each call still at its hour.
    let shown = turns_shown(calls.len(), cursor);
    helix.note = (shown.len() < calls.len())
        .then(|| format!("{}–{} of {}", shown.start + 1, shown.end, calls.len()));
    cursor = cursor.and_then(|i| i.checked_sub(shown.start));
    calls.truncate(shown.end);
    calls.drain(..shown.start);
    let Some(base) = calls.first().and_then(|c| explorer.tile_point(c.from)) else {
        return;
    };
    let caller = calls.first().map(|c| c.from);
    if caller != helix.caller {
        helix.caller = caller;
        helix.grown = 0.0;
    }
    helix.grown = (helix.grown + time.delta_secs() * 2.5).min(1.0);
    let grown = ease_in_out(helix.rise) * (0.35 + 0.65 * ease_in_out(helix.grown));
    let mut pen = Pen {
        explorer: &explorer,
        fade: helix.rise.min(1.0),
        lines: Vec::new(),
        markers: Vec::new(),
    };
    let stair = Stair::new(base, camera.distance * RADIUS * grown, calls.len());
    pen.face(&stair);
    pen.stand(&stair, &calls, cursor, false);
    // The cursor's own calls, on a small helix round its marker.
    if let Some(on) = cursor.map(|i| calls[i]) {
        let own: Vec<&Call> = all.iter().filter(|c| c.caller == on.span).collect();
        if let Some(&at) = pen.markers.iter().find(|m| m.cursor).map(|m| &m.at) {
            let small = Stair::new(at, stair.radius * SMALL, own.len());
            pen.stand(&small, &own, None, true);
        }
    }
    for (layer, points, color) in pen.lines {
        match layer {
            Layer::String => strings.linestrip(points, color),
            Layer::Rail => rails.linestrip(points, color),
            Layer::Cursor => bright.linestrip(points, color),
        }
    }
    helix.markers = pen.markers;
    helix.hub = Some(base);
}

/// The most turns a helix shows at once.
const TURNS: usize = 3;

/// Which of `count` calls a helix shows: all when they fit in [`TURNS`] turns, else
/// whole turns round the cursor's, from the first when it is on none.
pub(crate) fn turns_shown(count: usize, cursor: Option<usize>) -> std::ops::Range<usize> {
    let turns = count.div_ceil(PER_TURN);
    if turns <= TURNS {
        return 0..count;
    }
    let first = cursor
        .map_or(0, |c| (c / PER_TURN).saturating_sub(1))
        .min(turns - TURNS);
    first * PER_TURN..((first + TURNS) * PER_TURN).min(count)
}

/// Which lines a line is drawn with.
#[derive(Debug, Clone, Copy)]
enum Layer {
    String,
    Rail,
    Cursor,
}

/// How big the cursor's own helix is beside the one it stands on.
const SMALL: f32 = 0.3;

/// Where a helix stands and how big it is.
struct Stair {
    base: Vec3,
    radius: f32,
    step: f32,
    count: usize,
}

impl Stair {
    /// A helix of `count` calls over `base`, `radius` wide, as tall as its radius
    /// allows.
    fn new(base: Vec3, radius: f32, count: usize) -> Self {
        #[expect(clippy::cast_precision_loss, reason = "a small count")]
        let step = radius / RADIUS * STEP.min(HEIGHT / count.max(1) as f32);
        Self {
            base,
            radius,
            step,
            count,
        }
    }

    fn point(&self, index: f32) -> Vec3 {
        point(self.base, self.radius, self.step, index, self.count)
    }
}

/// What draws a helix: the lines it gathers, and its markers.
struct Pen<'a> {
    explorer: &'a Explorer,
    fade: f32,
    lines: Vec<(Layer, Vec<Vec3>, Color)>,
    markers: Vec<Marker>,
}

impl Pen<'_> {
    fn line(&mut self, layer: Layer, a: Vec3, b: Vec3, color: Color) {
        self.lines.push((layer, vec![a, b], color));
    }

    /// The face on the map round the caller, an hour mark a call, twelve longer.
    fn face(&mut self, stair: &Stair) {
        let dial = LINE.with_alpha(0.2 * self.fade);
        let round = (0..=96).map(|i| {
            #[expect(clippy::cast_precision_loss, reason = "a small count")]
            let angle = i as f32 / 96.0 * std::f32::consts::TAU;
            stair.base + Vec3::new(angle.sin(), angle.cos(), 0.0) * stair.radius
        });
        self.lines.push((Layer::String, round.collect(), dial));
        for hour in 1..=PER_TURN {
            #[expect(clippy::cast_precision_loss, reason = "a small count")]
            let angle = hour as f32 / PER_TURN as f32 * std::f32::consts::TAU;
            let out = Vec3::new(angle.sin(), angle.cos(), 0.0) * stair.radius;
            let tick = if hour == PER_TURN { 0.84 } else { 0.92 };
            self.line(
                Layer::String,
                stair.base + out * tick,
                stair.base + out,
                dial,
            );
        }
    }

    /// The axis standing up from the base, the rail the calls stand on, a tread out
    /// to each and a string down to its callee's node.
    fn stand(&mut self, stair: &Stair, calls: &[&Call], cursor: Option<usize>, small: bool) {
        let Some(last) = stair.count.checked_sub(1) else {
            return;
        };
        let fade = self.fade * if small { 0.8 } else { 1.0 };
        #[expect(clippy::cast_precision_loss, reason = "a small count")]
        let top = stair.base + Vec3::Z * stair.step * stair.count as f32;
        self.line(Layer::String, stair.base, top, LINE.with_alpha(0.35 * fade));
        // The rail, brighter as far as the cursor, so what has run reads as climbed.
        let run = cursor.unwrap_or(0);
        let rail = |from: usize, to: usize| -> Vec<Vec3> {
            (from * 12..=to * 12)
                .map(|i| {
                    #[expect(clippy::cast_precision_loss, reason = "a small count")]
                    let along = i as f32 / 12.0;
                    stair.point(along)
                })
                .collect()
        };
        if run > 0 {
            self.lines
                .push((Layer::Rail, rail(0, run), LINE.with_alpha(0.6 * fade)));
        }
        if run < last {
            self.lines
                .push((Layer::Rail, rail(run, last), LINE.with_alpha(0.3 * fade)));
        }
        for (index, call) in calls.iter().enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "a small count")]
            let at = stair.point(index as f32);
            let is_cursor = cursor == Some(index);
            // A tread from the axis out to it, so its height reads as a step.
            let axis = stair.base + Vec3::Z * (at.z - stair.base.z);
            // The cursor's is the clock's hand.
            if is_cursor {
                self.line(
                    Layer::Cursor,
                    axis,
                    at,
                    Color::WHITE.with_alpha(0.75 * fade),
                );
            } else {
                self.line(Layer::String, axis, at, LINE.with_alpha(0.16 * fade));
            }
            if let Some(node) = self.explorer.tile_point(call.to) {
                if is_cursor {
                    self.line(Layer::Cursor, at, node, Color::WHITE.with_alpha(0.9 * fade));
                } else {
                    let pale = if small { 0.18 } else { 0.28 };
                    self.line(Layer::String, at, node, LINE.with_alpha(pale * fade));
                }
            }
            self.markers.push(Marker {
                at,
                axis,
                number: call.number.clone(),
                name: node_name(self.explorer, call.to),
                // Its module's color even on the focus, which the map paints white, so
                // only the cursor's marker is white.
                color: lane_color(self.explorer.tile_color(call.to) % PALETTE.len()),
                cursor: is_cursor,
                small,
                tile: call.to,
            });
        }
    }
}

/// Slow at both ends, so the helix grows and settles gently.
fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Whether the helix is up far enough that the map goes quiet, so its names read.
pub(crate) fn up(helix: &Helix) -> bool {
    helix.rise > 0.5
}

/// How strongly the ordinary graph's links draw: the helix fades them back as it
/// rises.
pub(crate) fn link_strength(helix: &Helix) -> f32 {
    1.0 - 0.45 * helix.rise
}

/// The markers, furthest first so nearer ones cover them: a disc in the callee's
/// color with its number, its name beside it on the side away from the axis where
/// that is clear. The cursor's is white and always named. Returns where they drew, for
/// the map's names to keep off.
pub(crate) fn draw_labels(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    eye: Vec3,
    helix: &Helix,
) -> Drawn {
    let mut taken: Vec<egui::Rect> = Vec::new();
    let shown: Vec<(&Marker, egui::Pos2, egui::Vec2)> = helix
        .markers
        .iter()
        .filter_map(|m| {
            let at = to_screen(m.at)?;
            let away =
                to_screen(m.axis).map_or(egui::vec2(1.0, 0.0), |axis| (at - axis).normalized());
            Some((m, at, away))
        })
        .collect();
    // Discs first, so names keep off every one.
    for &(m, at, _) in &shown {
        taken.push(egui::Rect::from_center_size(
            at,
            egui::Vec2::splat(disc(m) * 2.0),
        ));
    }
    // Names: the cursor's first, then in order.
    let mut names: Vec<&(&Marker, egui::Pos2, egui::Vec2)> = shown.iter().collect();
    names.sort_by_key(|(m, _, _)| (!m.cursor, m.small));
    let mut placed: Vec<(egui::Rect, egui::text::LayoutJob, bool)> = Vec::new();
    for &&(m, at, away) in &names {
        let (size, color) = match (m.cursor, m.small) {
            (true, _) => (13.0, egui::Color32::WHITE),
            (false, false) => (11.5, egui::Color32::from_rgb(214, 219, 230)),
            (false, true) => (10.0, egui::Color32::from_rgb(170, 176, 190)),
        };
        let job = egui::text::LayoutJob::simple_singleline(
            crate::ui::short(&m.name, 22),
            egui::FontId::proportional(size),
            color,
        );
        let galley = painter.layout_job(job.clone());
        let extent = galley.size() + egui::vec2(8.0, 2.0);
        let gap = disc(m) + 4.0;
        // Out from the axis, else to either side of that, else in towards it; the
        // cursor's comes what may.
        let across = egui::vec2(-away.y, away.x);
        let clear = [away, across, -across, -away]
            .into_iter()
            .map(|way| name_rect(at, way, gap, extent))
            .find(|rect| !taken.iter().any(|r| r.intersects(rect.expand(1.0))));
        let Some(rect) = clear.or_else(|| m.cursor.then(|| name_rect(at, away, gap, extent)))
        else {
            continue;
        };
        taken.push(rect);
        placed.push((rect, job, m.cursor));
    }
    // Furthest first.
    let mut order: Vec<&(&Marker, egui::Pos2, egui::Vec2)> = shown.iter().collect();
    order.sort_by(|a, b| {
        b.0.at
            .distance_squared(eye)
            .total_cmp(&a.0.at.distance_squared(eye))
    });
    // The cursor's name last, over every disc.
    let (under, over): (Vec<_>, Vec<_>) = placed.into_iter().partition(|p| !p.2);
    for (rect, job, cursor) in under {
        pill(painter, rect, job, cursor);
    }
    for &&(m, at, _) in &order {
        draw_disc(painter, m, at);
    }
    for (rect, job, cursor) in over {
        pill(painter, rect, job, cursor);
    }
    // Which stretch of a long list shows, under the hub.
    if let (Some(note), Some(hub)) = (&helix.note, helix.hub.and_then(to_screen)) {
        let galley = painter.layout_no_wrap(
            note.clone(),
            egui::FontId::proportional(11.0),
            egui::Color32::from_rgb(170, 176, 190),
        );
        let rect = egui::Rect::from_center_size(
            hub + egui::vec2(0.0, 22.0),
            galley.size() + egui::vec2(8.0, 2.0),
        );
        painter.rect_filled(rect, 4.0, BACK);
        painter.galley(
            rect.center() - galley.size() * 0.5,
            galley,
            egui::Color32::WHITE,
        );
        taken.push(rect);
    }
    Drawn {
        rects: taken,
        tiles: helix.markers.iter().map(|m| m.tile).collect(),
    }
}

/// How big a marker's disc is: the cursor's biggest, those on its own small helix
/// smallest.
fn disc(marker: &Marker) -> f32 {
    match (marker.cursor, marker.small) {
        (true, _) => 11.0,
        (false, false) => 9.0,
        (false, true) => 6.5,
    }
}

/// Behind a name, dark enough to read over the map.
const BACK: egui::Color32 = egui::Color32::from_rgba_premultiplied(12, 14, 19, 215);

/// A marker's disc in its callee's color, the cursor's white, with the last part of
/// its number.
fn draw_disc(painter: &egui::Painter, marker: &Marker, at: egui::Pos2) {
    let r = disc(marker);
    let fill = if marker.cursor {
        egui::Color32::WHITE
    } else {
        egui_color(marker.color)
    };
    painter.circle_filled(at, r, fill);
    let ring = if marker.cursor {
        egui::Stroke::new(2.0, egui::Color32::WHITE)
    } else {
        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160))
    };
    painter.circle_stroke(at, r, ring);
    let digits = marker.number.rsplit('.').next().unwrap_or(&marker.number);
    painter.text(
        at,
        egui::Align2::CENTER_CENTER,
        digits,
        egui::FontId::proportional(r + 1.5),
        egui::Color32::from_rgb(12, 14, 18),
    );
}

/// A name on its dark pill, ringed for the cursor's.
fn pill(painter: &egui::Painter, rect: egui::Rect, job: egui::text::LayoutJob, cursor: bool) {
    painter.rect_filled(rect, 4.0, BACK);
    if cursor {
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(170)),
            egui::StrokeKind::Outside,
        );
    }
    let galley = painter.layout_job(job);
    painter.galley(
        rect.center() - galley.size() * 0.5,
        galley,
        egui::Color32::WHITE,
    );
}

/// Where a name of `extent` goes beside a marker at `at`: out along `away` from the
/// axis, `gap` clear of the disc, flush to the side it leans to.
pub(crate) fn name_rect(
    at: egui::Pos2,
    away: egui::Vec2,
    gap: f32,
    extent: egui::Vec2,
) -> egui::Rect {
    let anchor = at + away * gap;
    // Leaning right, the name starts there; left, it ends there; straight up or down,
    // it centres over or under.
    let x = if away.x > 0.35 {
        anchor.x
    } else if away.x < -0.35 {
        anchor.x - extent.x
    } else {
        anchor.x - extent.x * 0.5
    };
    let y = if away.y > 0.35 {
        anchor.y
    } else if away.y < -0.35 {
        anchor.y - extent.y
    } else {
        anchor.y - extent.y * 0.5
    };
    egui::Rect::from_min_size(egui::pos2(x, y), extent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_run_depth_first_each_with_its_caller() {
        let tree = |n: u32| match n {
            0 => vec![1, 4],
            1 => vec![2, 3],
            4 => vec![2],
            _ => vec![],
        };
        assert_eq!(
            calls_in_order(0, tree),
            vec![(0, 1), (1, 2), (1, 3), (0, 4), (4, 2)]
        );
    }

    #[test]
    fn calls_read_round_a_clock() {
        // The first at one o'clock, the third at three, the twelfth at twelve.
        let first = point(Vec3::ZERO, 1.0, 0.0, 0.0, 4);
        assert!((first.x - 0.5).abs() < 1e-5 && first.y > 0.8);
        let third = point(Vec3::ZERO, 1.0, 0.0, 2.0, 4);
        assert!((third - Vec3::X).length() < 1e-5);
        let twelfth = point(Vec3::ZERO, 1.0, 0.0, 11.0, 12);
        assert!((twelfth - Vec3::Y).length() < 1e-5);
    }

    #[test]
    fn each_call_stands_a_step_higher() {
        let heights: Vec<f32> = (0..19)
            .map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "a small count")]
                let i = i as f32;
                point(Vec3::ZERO, 1.0, 2.0, i, 19).z
            })
            .collect();
        assert!(heights.windows(2).all(|w| (w[1] - w[0] - 2.0).abs() < 1e-5));
    }

    #[test]
    fn a_second_turn_winds_wider_and_one_turn_stays_round() {
        let (_, inner, _) = place(0.0, 19);
        let (_, outer, _) = place(12.0, 19);
        assert!(outer > inner + 0.1);
        assert!((place(11.0, 12).1 - 1.0).abs() < 1e-6, "a clock is round");
    }

    #[test]
    fn a_long_list_shows_whole_turns_round_the_cursor() {
        assert_eq!(turns_shown(19, Some(18)), 0..19, "all, when they fit");
        assert_eq!(turns_shown(70, None), 0..36);
        assert_eq!(turns_shown(70, Some(2)), 0..36);
        assert_eq!(
            turns_shown(70, Some(30)),
            12..48,
            "the turn before and after"
        );
        assert_eq!(turns_shown(70, Some(69)), 36..70, "kept whole at the end");
    }

    #[test]
    fn names_lean_away_from_the_axis() {
        let at = egui::pos2(100.0, 100.0);
        let size = egui::vec2(40.0, 10.0);
        let right = name_rect(at, egui::vec2(1.0, 0.0), 5.0, size);
        assert!(right.min.x >= 105.0 - 1e-3);
        let left = name_rect(at, egui::vec2(-1.0, 0.0), 5.0, size);
        assert!(left.max.x <= 95.0 + 1e-3);
        let over = name_rect(at, egui::vec2(0.0, -1.0), 5.0, size);
        assert!(over.max.y <= 95.0 + 1e-3 && (over.center().x - 100.0).abs() < 1e-3);
    }
}
