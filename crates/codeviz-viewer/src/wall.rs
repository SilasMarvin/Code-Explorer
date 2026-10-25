//! The journey standing in the scene: its flame graph as a wall beside the graph, with
//! time running along the wall's length and each level of calls a row up it. The graph
//! does not move. Beams tie the bars around the cursor to their nodes, so the order the
//! wall reads and the places the graph shows meet without a line through every call.
//!
//! The wall can stand three ways ([`Shape`]): square to the graph, along the depth the
//! graph does not use; slanted to face the usual view; or curved around the graph, a
//! stretch of cylinder whose arc runs along time.
//!
//! `Z` stands it up and turns the camera to look into the corner the wall makes with
//! the graph, where both read, `Shift+Z` changes its shape; [`Chrome`] holds both.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::SpanId;

use crate::{
    camera::{ANGLED, OrbitCamera},
    diff_view::DiffView,
    explorer::Explorer,
    graph_view::{fit_distance, orbit_anchor},
    journey_panel::{PanelMode, bar_look},
    ui::Chrome,
};

/// Whether the wall stands and how, and the state its drawing keeps: whether it lit
/// the graph's hovered node, so it can clear it again, and whether `F` asked to look
/// into the corner again.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TimeWall {
    pub(crate) on: bool,
    pub(crate) shape: Shape,
    lit: bool,
    pub(crate) refit: bool,
    /// How far round from the usual corner view the camera turns to look at it, in
    /// radians of yaw, to see the wall from elsewhere.
    pub(crate) turn: f32,
    /// The stretch of the journey, in slots, the wall widens around the cursor, as it
    /// eases after it.
    lens: Option<(f32, f32)>,
}

impl TimeWall {
    /// How the journey panel shows: the wall stands in for it docked, where the
    /// wall's near end would be.
    pub(crate) fn panel(self, mode: PanelMode) -> PanelMode {
        if self.on && mode == PanelMode::Docked {
            PanelMode::Hidden
        } else {
            mode
        }
    }
}

/// How the wall stands beside the graph.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Shape {
    /// Square to the graph, time running along its depth towards the viewer.
    Square,
    /// Turned to face the usual view head-on, so neither end looms.
    #[default]
    Slant,
    /// Curved around the graph, a stretch of cylinder about its middle.
    Arc,
}

impl Shape {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Slant => Self::Arc,
            Self::Arc => Self::Square,
            Self::Square => Self::Slant,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Square => "wall",
            Self::Slant => "slant",
            Self::Arc => "arc",
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        [Self::Square, Self::Slant, Self::Arc]
            .into_iter()
            .find(|s| s.name() == name)
    }

    /// The field of view while the wall stands. Square, the wall runs towards the
    /// viewer and from close by its near end would loom over its far one, so the
    /// camera stands further back with a narrower lens.
    fn fov(self) -> f32 {
        match self {
            Self::Square => 0.38,
            Self::Slant | Self::Arc => 0.6,
        }
    }
}

/// The camera's yaw and pitch looking into the corner: from the graph's front left,
/// a little above, so the graph and the wall both face it at about the same slant.
pub(crate) const VIEW: (f32, f32) = (-0.78, -0.26);

/// The wall's measures, as shares of the graph's extent: the gap between the graph's
/// edge and the wall, a row's height, its length for a short journey, the length each
/// slot asks for so a long journey still names its calls, and the longest it gets.
const GAP: f32 = 0.22;
const ROW: f32 = 0.3;
/// The tallest the wall gets, a share of the graph's extent, however deep the journey.
const TALL: f32 = 1.0;

/// The root's row, as a share of the others: it spans the whole journey and says
/// least, so it is the wall's footing rather than a slab across it.
const ROOT: f32 = 0.45;

/// How high up the wall, in rows, row `row` starts; fractions reach into it.
pub(crate) fn level(row: f32) -> f32 {
    if row <= 1.0 {
        row * ROOT
    } else {
        ROOT + row - 1.0
    }
}
const SHORT: f32 = 1.1;
const SLOT: f32 = 0.11;
const LONG: f32 = 2.6;

/// Shares of a slot and of a row left between bars, as in the panel.
const BAR_GAP: f32 = 0.06;
const ROW_GAP: f32 = 0.12;

/// The wall's face: as dark as the journey panel and as solid, since the graph
/// behind it is hidden by it; dimmed bars are see-through.
const BACKING: egui::Color32 = egui::Color32::from_rgba_premultiplied(14, 16, 23, 240);

/// The slanted wall's direction: the usual view's right, so it faces that view.
fn slant() -> Vec3 {
    Vec3::new(VIEW.0.cos(), 0.0, -VIEW.0.sin())
}

/// The most the arc turns, start to end: a little short of a quarter, so its far end
/// never comes round in front of the graph.
const ARC: f32 = 1.45;

/// How many pieces a curved bar is drawn in per radian it turns.
const PIECES: f32 = 18.0;

/// Where the wall stands, in the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Wall {
    shape: Shape,
    /// Its corner nearest the graph at the bottom, where the journey starts.
    foot: Vec3,
    /// The middle of the journey's nodes, level with the foot: the arc's axis.
    middle: Vec3,
    /// How far time runs, start to end, unless the arc must stop short.
    length: f32,
    /// A row's height.
    row: f32,
    rows: usize,
}

impl Wall {
    /// A wall for `rows` levels of calls and `slots` across, standing just past the
    /// right edge of `points`, the journey's nodes, centered on them top to bottom.
    pub(crate) fn beside(shape: Shape, points: &[Vec3], rows: usize, slots: f32) -> Option<Self> {
        if points.is_empty() || rows == 0 {
            return None;
        }
        let (min, max) = points.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(min, max), &p| (min.min(p), max.max(p)),
        );
        let size = max - min;
        let extent = size.x.max(size.y).max(10.0);
        #[expect(clippy::cast_precision_loss, reason = "a few rows")]
        let rows_f = rows as f32;
        let row = (ROW * extent).min(TALL * extent / level(rows_f));
        let height = row * level(rows_f);
        let middle = (min + max) / 2.0;
        let foot = Vec3::new(max.x + GAP * extent, middle.y - height / 2.0, middle.z);
        Some(Self {
            shape,
            foot,
            middle: Vec3::new(middle.x, foot.y, middle.z),
            length: (slots * SLOT).clamp(SHORT, LONG) * extent,
            row,
            rows,
        })
    }

    /// The wall moved out along x, if need be, to stand clear past `edge`.
    pub(crate) fn clear_of(mut self, edge: f32) -> Self {
        self.foot.x = self.foot.x.max(edge + GAP * self.length / LONG);
        self
    }

    /// How high the wall stands, in rows.
    fn top(&self) -> f32 {
        #[expect(clippy::cast_precision_loss, reason = "a few rows")]
        level(self.rows as f32)
    }

    /// The arc's radius and how far it turns, start to end.
    fn arc(&self) -> (f32, f32) {
        let radius = (self.foot.x - self.middle.x).max(1.0);
        (radius, (self.length / radius).min(ARC))
    }

    /// The point `along` the way from the journey's start to its end, `up` rows high.
    pub(crate) fn at(&self, along: f32, up: f32) -> Vec3 {
        let rise = Vec3::Y * up * self.row;
        match self.shape {
            Shape::Square => self.foot + Vec3::Z * along * self.length + rise,
            Shape::Slant => self.foot + slant() * along * self.length + rise,
            Shape::Arc => {
                let (radius, turn) = self.arc();
                let angle = along * turn;
                self.middle + Vec3::new(angle.cos(), 0.0, angle.sin()) * radius + rise
            }
        }
    }

    /// The wall's corners: along the bottom, then back along the top.
    #[cfg(test)]
    fn corners(&self) -> [Vec3; 4] {
        let top = self.top();
        [
            self.at(0.0, 0.0),
            self.at(1.0, 0.0),
            self.at(1.0, top),
            self.at(0.0, top),
        ]
    }

    /// The stretch of wall from `left` for `width`, in shares of the journey, between
    /// heights `bottom` and `top`, in pieces flat enough to draw as quads: one on a
    /// flat wall, more the further a curved one turns. Each is bottom start, bottom
    /// end, top end, top start.
    fn strip(&self, left: f32, width: f32, bottom: f32, top: f32) -> Vec<[Vec3; 4]> {
        let pieces = match self.shape {
            Shape::Arc => (width * self.arc().1 * PIECES).ceil().max(1.0),
            Shape::Square | Shape::Slant => 1.0,
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a small, positive count"
        )]
        let count = pieces as usize;
        (0..count)
            .map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "a small count")]
                let (a, b) = (
                    left + width * i as f32 / pieces,
                    left + width * (i + 1) as f32 / pieces,
                );
                [
                    self.at(a, bottom),
                    self.at(b, bottom),
                    self.at(b, top),
                    self.at(a, top),
                ]
            })
            .collect()
    }

    /// The whole wall, in pieces.
    fn face(&self) -> Vec<[Vec3; 4]> {
        self.strip(0.0, 1.0, 0.0, self.top())
    }

    /// A bar's pieces, `left` and `width` in shares of the journey on `row`, the
    /// root's at the bottom.
    pub(crate) fn bar(&self, left: f32, width: f32, row: usize) -> Vec<[Vec3; 4]> {
        #[expect(clippy::cast_precision_loss, reason = "a few rows")]
        let row = row as f32;
        self.strip(
            left,
            width * (1.0 - BAR_GAP),
            level(row + ROW_GAP / 2.0),
            level(row + 1.0 - ROW_GAP / 2.0),
        )
    }
}

/// The spans laid out, in the order they run: by where they start, a caller before
/// its first call.
pub(crate) fn running_order<K: Copy>(placed: &HashMap<K, (f32, f32, usize)>) -> Vec<K> {
    let mut order: Vec<(K, f32, usize)> = placed
        .iter()
        .map(|(&key, &(left, _, row))| (key, left, row))
        .collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.2.cmp(&b.2)));
    order.into_iter().map(|(key, _, _)| key).collect()
}

/// Slots across past which a journey is too crowded to name every call evenly, and
/// the wall widens the stretch around the cursor instead.
const CROWDED: f32 = 12.0;
/// That stretch, in slots before and after the cursor's start: more ahead, where
/// stepping goes next.
const BEHIND: f32 = 2.0;
const AHEAD: f32 = 6.0;
/// The least share of the wall's length the stretch gets.
const FOCUS: f32 = 0.6;

/// Where slot `slot` of `total` falls along the wall, as a share of its length: evenly,
/// or with the stretch `lens` given at least [`FOCUS`] of the length and the rest
/// sharing what is left, so the calls around the cursor have room for their names
/// while the whole journey still shows.
pub(crate) fn warp(slot: f32, total: f32, lens: Option<(f32, f32)>) -> f32 {
    let Some((from, to)) = lens.filter(|_| total > 0.0) else {
        return slot / total.max(1.0);
    };
    let inside = to - from;
    let outside = total - inside;
    if inside <= 0.0 || outside <= 0.0 {
        return slot / total;
    }
    let focus = FOCUS.max(inside / total);
    let rest = 1.0 - focus;
    let before = from / outside * rest;
    if slot < from {
        slot / outside * rest
    } else if slot <= to {
        before + (slot - from) / inside * focus
    } else {
        before + focus + (slot - to) / outside * rest
    }
}

/// The stretch to widen around the cursor's call starting at `start`, in a journey
/// `total` slots across; none when every call has room as it is.
pub(crate) fn lens_around(start: f32, total: f32) -> Option<(f32, f32)> {
    (total > CROWDED).then(|| {
        let from = (start - BEHIND).max(0.0);
        let to = (from + BEHIND + AHEAD).min(total);
        ((to - BEHIND - AHEAD).max(0.0), to)
    })
}

/// Calls either side of the cursor that get a beam: enough to see where it came from
/// and goes next; more cross the wall in a tangle.
const NEAR: usize = 2;

/// How strongly the bar `index` along the running order beams to its node with the
/// cursor on bar `cursor`: in full for the cursor's, fading over the few either side,
/// and not at all further away, so a long journey never draws a hairball.
pub(crate) fn beam_strength(index: usize, cursor: usize) -> f32 {
    let away = index.abs_diff(cursor);
    if away > NEAR {
        return 0.0;
    }
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let away = away as f32;
    1.0 - away * 0.3
}

/// How strongly the other calls to the cursor's function beam to it, so a function
/// the journey runs more than once shows each time.
const AGAIN: f32 = 0.3;

/// Whether `point` lies inside the convex quad `corners`, either way round.
pub(crate) fn inside(point: egui::Pos2, corners: &[egui::Pos2; 4]) -> bool {
    let sides: Vec<f32> = (0..4)
        .map(|i| {
            let (a, b) = (corners[i], corners[(i + 1) % 4]);
            (b - a).x * (point - a).y - (b - a).y * (point - a).x
        })
        .collect();
    sides.iter().all(|&s| s >= 0.0) || sides.iter().all(|&s| s <= 0.0)
}

/// Where the journey runs on the graph: its nodes as disclosed, and where it starts.
/// The wall stands beside these, not the whole graph, whose far reaches would push it
/// out of sight.
fn journey_points(explorer: &Explorer) -> Vec<Vec3> {
    explorer
        .disclosure()
        .tiles()
        .keys()
        .chain(explorer.anchor().as_ref())
        .filter_map(|&tile| explorer.tile_point(tile))
        .collect()
}

/// How far right the graph reaches level with the wall, give or take half its height:
/// the wall stands past it so as not to hide the graph behind it. A few far outliers do
/// not push it out.
fn map_edge(explorer: &Explorer, wall: &Wall) -> f32 {
    let (low, high) = (wall.at(0.0, 0.0).y, wall.at(0.0, wall.top()).y);
    let reach = (high - low) / 2.0;
    let mut edges: Vec<f32> = (0..explorer.map().tiles.len())
        .filter(|&tile| explorer.visibility(tile) >= 0.5)
        .filter_map(|tile| explorer.tile_point(tile))
        .filter(|p| p.y > low - reach && p.y < high + reach)
        .map(|p| p.x)
        .collect();
    edges.sort_by(f32::total_cmp);
    edges
        .get(edges.len() * 19 / 20)
        .copied()
        .unwrap_or(f32::NEG_INFINITY)
}

/// Where each span sits in the flame graph: left edge and width in slots, and row.
type Placed = HashMap<SpanId, (f32, f32, usize)>;

/// The journey's bars as disclosed, their slots across, and the wall to stand them on.
fn stand(explorer: &Explorer, shape: Shape) -> Option<(Wall, Placed, f32)> {
    let disclosed = explorer.disclosure();
    let (placed, slots) = disclosed.layout(explorer.journey()?.root());
    let wall = Wall::beside(shape, &journey_points(explorer), disclosed.rows(), slots)?;
    Some((
        wall.clear_of(map_edge(explorer, &wall)),
        placed,
        slots.max(1.0),
    ))
}

/// What the corner view must keep on screen: the wall and the journey's nodes.
fn view_points(explorer: &Explorer, shape: Shape) -> Option<Vec<Vec3>> {
    let (wall, _, _) = stand(explorer, shape)?;
    let mut points: Vec<Vec3> = wall.face().into_iter().flatten().collect();
    points.extend(journey_points(explorer));
    Some(points)
}

/// Where the camera turned to `rotation` looks from, and how far back, to see the
/// wall and the journey's nodes.
fn corner_view(
    explorer: &Explorer,
    shape: Shape,
    rotation: Quat,
    perspective: &PerspectiveProjection,
) -> Option<(Vec3, f32)> {
    let points = view_points(explorer, shape)?;
    let (min, max) = points.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(min, max), &p| (min.min(p), max.max(p)),
    );
    let center = (min + max) / 2.0;
    Some((center, fit_distance(&points, center, rotation, perspective)))
}

/// What the corner view was last worked out for, and where it looks from.
#[derive(Default)]
pub(crate) struct Framed {
    on: bool,
    shape: Shape,
    key: Option<(Option<usize>, u64)>,
    view: Option<(Vec3, f32)>,
}

/// How much further back than the view stands the wall may need before it refits.
const OUTGROWN: f32 = 1.05;

/// Turns to look into the corner when the wall stands up, changes shape, or `F` asks,
/// and holds that view while it stands, until the viewer pans or zooms away. A new
/// journey looks again; the wall growing past the view, as a level more opens or the
/// graph settles, only pulls back from where the camera is turned. Turns back to the
/// journey's start when the wall goes.
pub(crate) fn frame(
    mut chrome: ResMut<Chrome>,
    explorer: Res<Explorer>,
    mut last: Local<Framed>,
    time: Res<Time>,
    mut cameras: Query<(&mut OrbitCamera, &mut Projection, &Transform)>,
) {
    let wall = chrome.wall;
    let raised = wall.on && (!last.on || last.shape != wall.shape);
    let lowered = !wall.on && last.on;
    (last.on, last.shape) = (wall.on, wall.shape);
    if wall.refit {
        chrome.wall.refit = false;
    }
    let ease = 1.0 - (-6.0 * time.delta_secs()).exp();
    for (mut camera, mut projection, transform) in &mut cameras {
        let Projection::Perspective(perspective) = &mut *projection else {
            continue;
        };
        let fov = if wall.on {
            wall.shape.fov()
        } else {
            PerspectiveProjection::default().fov
        };
        if (perspective.fov - fov).abs() > 1e-4 {
            perspective.fov += (fov - perspective.fov) * ease;
        }
        let lens = PerspectiveProjection {
            fov,
            ..perspective.clone()
        };
        if lowered {
            camera.turn_to(ANGLED);
            orbit_anchor(&explorer, &mut camera);
        }
        if !wall.on {
            last.view = None;
            continue;
        }
        let key = (explorer.anchor(), explorer.aim());
        if raised || wall.refit || last.key != Some(key) {
            last.key = Some(key);
            let angles = (VIEW.0 + wall.turn, VIEW.1);
            let corner = Quat::from_euler(EulerRot::YXZ, angles.0, angles.1, 0.0);
            last.view = corner_view(&explorer, wall.shape, corner, &lens);
            camera.turn_to(angles);
            if last.view.is_some() {
                camera.free = false;
            }
        } else if let Some((target, distance)) = last.view
            && !camera.free
            && let Some(points) = view_points(&explorer, wall.shape)
            && fit_distance(&points, target, transform.rotation, &lens) > distance * OUTGROWN
        {
            last.view = corner_view(&explorer, wall.shape, transform.rotation, &lens);
        }
        if let Some((target, distance)) = last.view
            && !camera.free
        {
            camera.glide_to(target, distance);
        }
    }
}

/// A bar on the wall, on screen.
struct Bar {
    span: SpanId,
    tile: Option<usize>,
    /// Its pieces, each bottom start, bottom end, top end, top start.
    quads: Vec<[egui::Pos2; 4]>,
    fill: egui::Color32,
    text: egui::Color32,
}

impl Bar {
    /// Where its call starts: midway up its near edge, a little in.
    fn start(&self) -> egui::Pos2 {
        let first = self.quads[0];
        let near = first[0] + (first[3] - first[0]) / 2.0;
        let far = first[1] + (first[2] - first[1]) / 2.0;
        near + (far - near).normalized() * 4.0
    }

    fn contains(&self, point: egui::Pos2) -> bool {
        self.quads.iter().any(|quad| inside(point, quad))
    }
}

/// The outline of a strip of pieces: along the bottom, then back along the top.
fn outline(quads: &[[egui::Pos2; 4]]) -> Vec<egui::Pos2> {
    let bottom = quads.iter().map(|q| q[0]).chain(quads.last().map(|q| q[1]));
    let top = quads
        .iter()
        .rev()
        .map(|q| q[2])
        .chain(quads.first().map(|q| q[3]));
    bottom.chain(top).collect()
}

/// The upright rectangle inside a strip on screen, where its name goes horizontally,
/// whichever way the strip runs.
pub(crate) fn upright(quads: &[[egui::Pos2; 4]]) -> Option<egui::Rect> {
    let (first, last) = (quads.first()?, quads.last()?);
    let start = f32::midpoint(first[0].x, first[3].x);
    let end = f32::midpoint(last[1].x, last[2].x);
    let (left, right) = if start <= end {
        (first[0].x.max(first[3].x), last[1].x.min(last[2].x))
    } else {
        (last[1].x.max(last[2].x), first[0].x.min(first[3].x))
    };
    // Its top edge's lowest point and its bottom edge's highest, across the pieces.
    let top = quads
        .iter()
        .flat_map(|q| [q[2].y, q[3].y])
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = quads
        .iter()
        .flat_map(|q| [q[0].y, q[1].y])
        .fold(f32::INFINITY, f32::min);
    let (top, bottom) = (top.min(bottom), top.max(bottom));
    (right > left && bottom > top)
        .then(|| egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom)))
}

/// The wall as it stands this frame, on screen.
struct Standing {
    face: Vec<[egui::Pos2; 4]>,
    /// The bars in the order they run.
    bars: Vec<Bar>,
    /// The cursor's bar, by index into `bars`.
    cursor: Option<usize>,
    /// Where the cursor's call starts, bottom to top of the wall.
    now: Option<[egui::Pos2; 2]>,
    /// The bar under the pointer.
    hovered: Option<usize>,
}

impl Standing {
    /// Stands the wall for the journey as disclosed, through `project`, the camera's
    /// view of the world; `None` while there is no journey or the wall is edge-on.
    fn of(
        explorer: &Explorer,
        diff: Option<&DiffView>,
        (wall, placed, slots): &(Wall, Placed, f32),
        lens: Option<(f32, f32)>,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer: Option<egui::Pos2>,
    ) -> Option<Self> {
        let (wall, slots) = (*wall, *slots);
        let along = |slot: f32| warp(slot, slots, lens);
        let on_screen = |pieces: Vec<[Vec3; 4]>| -> Option<Vec<[egui::Pos2; 4]>> {
            pieces
                .into_iter()
                .map(|q| {
                    Some([
                        project(q[0])?,
                        project(q[1])?,
                        project(q[2])?,
                        project(q[3])?,
                    ])
                })
                .collect()
        };
        let face = on_screen(wall.face())?;
        let track = explorer.track();
        let map = explorer.map();
        let cursor_span = explorer.selected();
        let cursor_enter = cursor_span
            .and_then(|c| track.span(c))
            .map_or(0, |s| s.enter);
        let stepping =
            cursor_span.is_some_and(|c| explorer.journey().is_some_and(|j| j.root() != c));
        let mut bars = Vec::new();
        for span in running_order(placed) {
            let (left, width, row) = placed[&span];
            let (start, end) = (along(left), along(left + width));
            let Some(quads) = on_screen(wall.bar(start, end - start, row)) else {
                continue;
            };
            let future = stepping && track.span(span).is_some_and(|s| s.enter > cursor_enter);
            let (fill, text) = bar_look(explorer, diff, span, future);
            bars.push(Bar {
                span,
                tile: track.unit_id(span).and_then(|u| map.tile_of(u)),
                quads,
                fill,
                text,
            });
        }
        // A closure has no bar of its own: the cursor on one is on its function's.
        let cursor = cursor_span.and_then(|mut span| {
            loop {
                if let Some(index) = bars.iter().position(|b| b.span == span) {
                    return Some(index);
                }
                span = track.span(span)?.parent?;
            }
        });
        let now = cursor.and_then(|index| {
            let (left, _, _) = placed[&bars[index].span];
            Some([
                project(wall.at(along(left), 0.0))?,
                project(wall.at(along(left), wall.top()))?,
            ])
        });
        let hovered = pointer.and_then(|p| bars.iter().position(|b| b.contains(p)));
        // Edge-on, the wall is a sliver with nothing to read.
        let breadth: f32 = face.iter().map(|q| (q[1] - q[0]).length()).sum();
        (breadth > 40.0).then_some(Self {
            face,
            bars,
            cursor,
            now,
            hovered,
        })
    }

    /// The beams from the bars around the cursor to their nodes, pale so they say
    /// nothing about direction or module, and strongest at the cursor.
    fn draw_beams(
        &self,
        painter: &egui::Painter,
        explorer: &Explorer,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        let cursor_tile = self.cursor.and_then(|c| self.bars[c].tile);
        let hovered_tile = self.hovered.and_then(|h| self.bars[h].tile);
        let mut beams: Vec<(usize, f32)> = Vec::new();
        for (index, bar) in self.bars.iter().enumerate() {
            let near = self.cursor.map_or(0.0, |c| beam_strength(index, c));
            let again = if bar.tile.is_some() && bar.tile == cursor_tile {
                AGAIN
            } else {
                0.0
            };
            let hovered =
                if Some(index) == self.hovered || bar.tile.is_some() && bar.tile == hovered_tile {
                    0.8
                } else {
                    0.0
                };
            let strength = near.max(again).max(hovered);
            if strength > 0.0 {
                beams.push((index, strength));
            }
        }
        // The strongest last, on top.
        beams.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (index, strength) in beams {
            let bar = &self.bars[index];
            let Some(node) = bar
                .tile
                .and_then(|tile| explorer.tile_point(tile))
                .and_then(project)
            else {
                continue;
            };
            let from = bar.start();
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "an alpha in 50..=230"
            )]
            let alpha = (50.0 + strength * 180.0) as u8;
            let stroke =
                egui::Stroke::new(0.8 + strength * 1.6, egui::Color32::from_white_alpha(alpha));
            painter.line_segment([from, node], stroke);
            painter.circle_filled(from, 1.5 + strength * 1.5, stroke.color);
            painter.circle_stroke(node, 4.0 + strength * 4.0, stroke);
        }
    }

    /// The wall, its bars and their names, and the cursor's line across it.
    fn draw_bars(&self, painter: &egui::Painter, explorer: &Explorer) {
        for quad in &self.face {
            painter.add(egui::Shape::convex_polygon(
                quad.to_vec(),
                BACKING,
                egui::Stroke::NONE,
            ));
        }
        painter.add(egui::Shape::closed_line(
            outline(&self.face),
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(40)),
        ));
        // Time runs away from the graph: said over the wall's end, clear of the
        // cursor's name over its start.
        if let Some(last) = self.face.last() {
            painter.text(
                last[2] + egui::vec2(-4.0, -4.0),
                egui::Align2::RIGHT_BOTTOM,
                "time →",
                egui::FontId::proportional(12.0),
                egui::Color32::from_white_alpha(130),
            );
        }
        let disclosed = explorer.disclosure();
        for (index, bar) in self.bars.iter().enumerate() {
            for quad in &bar.quads {
                painter.add(egui::Shape::convex_polygon(
                    quad.to_vec(),
                    bar.fill,
                    egui::Stroke::new(0.5, bar.fill),
                ));
            }
            let lit = Some(index) == self.hovered
                || bar.tile.is_some() && bar.tile == explorer.hovered_tile;
            if lit {
                painter.add(egui::Shape::closed_line(
                    outline(&bar.quads),
                    egui::Stroke::new(1.5, egui::Color32::from_white_alpha(170)),
                ));
            }
            let number = disclosed.number(bar.span).unwrap_or_default();
            // The function's own name: the bar's color already says its module.
            let label = explorer.label(bar.span);
            let name = label
                .rsplit_once('.')
                .map_or(label.as_str(), |(_, name)| name);
            let named = label_bar(painter, &bar.quads, number, name, bar.text);
            // The cursor's call is always named: over the wall where its bar has no
            // room.
            if Some(index) == self.cursor
                && !named
                && let Some([_, top]) = self.now
            {
                painter.text(
                    top + egui::vec2(0.0, -6.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{number}  {name}").trim(),
                    egui::FontId::proportional(13.0),
                    egui::Color32::WHITE,
                );
            }
        }
        if let Some(now) = self.now {
            painter.line_segment(
                now,
                egui::Stroke::new(1.0, egui::Color32::from_white_alpha(70)),
            );
        }
        if let Some(bar) = self.cursor.map(|c| &self.bars[c]) {
            painter.add(egui::Shape::closed_line(
                outline(&bar.quads),
                egui::Stroke::new(2.0, egui::Color32::WHITE),
            ));
        }
    }

    /// Lights the hovered bar's node in the graph, and a click on a bar moves the
    /// cursor to it.
    fn interact(&self, ctx: &egui::Context, explorer: &mut Explorer, lit: &mut bool) {
        let hovered = self.hovered.map(|h| &self.bars[h]);
        if let Some(tile) = hovered.and_then(|b| b.tile) {
            if explorer.hovered_tile != Some(tile) {
                explorer.hovered_tile = Some(tile);
            }
            *lit = true;
        } else if *lit {
            explorer.hovered_tile = None;
            *lit = false;
        }
        if let Some(bar) = hovered
            && ctx.input(|i| i.pointer.primary_clicked())
        {
            explorer.select(Some(bar.span));
        }
    }
}

/// Stands the wall while it is on, under the graph's names and over the graph: its
/// bars, then the beams over them, so each can be traced to its own bar. A bar under
/// the pointer lights its node, and a click on one moves the cursor there.
pub(crate) fn draw(
    painter: &egui::Painter,
    (camera, transform): (&Camera, &GlobalTransform),
    area: egui::Rect,
    explorer: &mut Explorer,
    diff: Option<&DiffView>,
    wall: &mut TimeWall,
) {
    // The wall reaches past the screen's edges, so its corners project unclipped.
    let project = &|position: Vec3| {
        camera
            .world_to_viewport(transform, position)
            .ok()
            .map(|p| egui::pos2(p.x, p.y))
    };
    let ctx = painter.ctx();
    let hover = ctx
        .pointer_hover_pos()
        .filter(|p| area.contains(*p) && !ctx.egui_is_using_pointer());
    let stood = wall.on.then(|| stand(explorer, wall.shape)).flatten();
    // The stretch around the cursor eases after it as it moves.
    let goal = stood.as_ref().and_then(|(_, placed, slots)| {
        let cursor = explorer.selected()?;
        lens_around(placed.get(&cursor)?.0, *slots)
    });
    wall.lens = match (wall.lens, goal) {
        (Some(lens), Some(goal)) => {
            let ease = 1.0 - (-8.0 * ctx.input(|i| i.stable_dt).min(0.1)).exp();
            Some((
                lens.0 + (goal.0 - lens.0) * ease,
                lens.1 + (goal.1 - lens.1) * ease,
            ))
        }
        (_, goal) => goal,
    };
    let standing =
        stood.and_then(|stood| Standing::of(explorer, diff, &stood, wall.lens, project, hover));
    let Some(standing) = standing else {
        if wall.lit {
            explorer.hovered_tile = None;
            wall.lit = false;
        }
        return;
    };
    standing.draw_bars(painter, explorer);
    standing.draw_beams(painter, explorer, project);
    standing.interact(ctx, explorer, &mut wall.lit);
}

/// Font sizes a bar's name tries, largest first.
const SIZES: [f32; 4] = [13.0, 11.5, 10.0, 9.0];

/// The pieces a name breaks into across lines: after each `_` or `::`, and before
/// each capital that follows a small letter.
fn words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut last = ' ';
    for c in name.chars() {
        if c.is_uppercase() && last.is_lowercase() && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        word.push(c);
        if c == '_' || c == ':' && last == ':' {
            words.push(std::mem::take(&mut word));
        }
        last = c;
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// `words` packed into at most `lines` lines of `width` characters, a word too long
/// for a line cut across lines, and the last line ending in `…` when they do not all
/// fit. Returns the lines and whether everything fit with no word cut.
pub(crate) fn pack(words: &[String], width: usize, lines: usize) -> (Vec<String>, bool) {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut cut = false;
    for word in words {
        let fits = line.chars().count() + word.chars().count() <= width;
        if !fits && !line.is_empty() {
            out.push(std::mem::take(&mut line));
        }
        line.push_str(word);
        while line.chars().count() > width {
            let head: String = line.chars().take(width).collect();
            line = line.chars().skip(width).collect();
            out.push(head);
            cut = true;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    let whole = out.len() <= lines && !cut;
    if out.len() > lines {
        out.truncate(lines);
        if let Some(last) = out.last_mut() {
            let kept: String = last.chars().take(width.saturating_sub(1)).collect();
            *last = format!("{kept}…");
        }
    }
    (out, whole)
}

/// Names a bar upright, whatever way the wall turns: its number, then its name
/// broken at its words across as many lines as the bar is tall, as large as fits
/// whole; where it fits at no size, the number alone, as the panel does. Returns
/// whether the name showed.
fn label_bar(
    painter: &egui::Painter,
    quads: &[[egui::Pos2; 4]],
    number: &str,
    name: &str,
    color: egui::Color32,
) -> bool {
    let Some(rect) = upright(quads).map(|r| r.shrink2(egui::vec2(3.0, 2.0))) else {
        return false;
    };
    let mut parts = words(name);
    if !number.is_empty() {
        parts.insert(0, format!("{number} "));
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "small, clamped counts"
    )]
    let fit = |size: f32| {
        let width = (rect.width() / (size * 0.5)).clamp(0.0, 200.0) as usize;
        let lines = (rect.height() / (size * 1.2)).clamp(0.0, 20.0) as usize;
        (width >= 3 && lines >= 1).then(|| (size, pack(&parts, width, lines)))
    };
    let whole = SIZES
        .into_iter()
        .filter_map(fit)
        .find(|(_, (_, whole))| *whole);
    let named = whole.is_some();
    let number_alone = || {
        let size = SIZES[SIZES.len() - 1];
        #[expect(clippy::cast_precision_loss, reason = "a short number")]
        let wide = number.chars().count() as f32 * size * 0.5;
        (!number.is_empty() && wide <= rect.width() && size * 1.2 <= rect.height())
            .then(|| (size, (vec![number.to_owned()], true)))
    };
    let Some((size, (lines, _))) = whole.or_else(number_alone) else {
        return false;
    };
    let font = egui::FontId::proportional(size);
    #[expect(clippy::cast_precision_loss, reason = "a few lines")]
    let block = lines.len() as f32 * size * 1.2;
    let painter = painter.with_clip_rect(rect.expand(2.0).intersect(painter.clip_rect()));
    let mut y = rect.center().y - block / 2.0;
    for line in lines {
        painter.text(
            egui::pos2(rect.left(), y),
            egui::Align2::LEFT_TOP,
            line,
            font.clone(),
            color,
        );
        y += size * 1.2;
    }
    named
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wall_stands_past_the_graph_and_runs_towards_the_viewer() {
        let points = [Vec3::new(-50.0, -20.0, 1.0), Vec3::new(50.0, 40.0, -1.0)];
        let wall = Wall::beside(Shape::Square, &points, 3, 4.0).unwrap();
        let [start, end, top_end, top_start] = wall.corners();
        assert!(start.x > 50.0, "clear of the graph's right edge");
        assert_eq!(start.x, end.x, "square to the graph");
        assert!(end.z > start.z, "time runs towards the viewer");
        assert!(top_start.y > start.y && (top_end.y - top_start.y).abs() < 1e-4);
        let middle = f32::midpoint(start.y, top_start.y);
        assert!(
            (middle - 10.0).abs() < 1e-4,
            "centered on the graph top to bottom"
        );
    }

    #[test]
    fn the_wall_moves_out_past_the_graph_but_never_in() {
        let points = [Vec3::ZERO, Vec3::new(100.0, 100.0, 0.0)];
        let wall = Wall::beside(Shape::Square, &points, 2, 4.0).unwrap();
        let foot = wall.at(0.0, 0.0).x;
        assert!((wall.clear_of(0.0).at(0.0, 0.0).x - foot).abs() < 1e-4);
        assert!(wall.clear_of(300.0).at(0.0, 0.0).x > 300.0);
    }

    #[test]
    fn a_long_journey_lengthens_the_wall_up_to_a_point() {
        let points = [Vec3::ZERO, Vec3::new(100.0, 100.0, 0.0)];
        let length = |slots| {
            let wall = Wall::beside(Shape::Square, &points, 2, slots).unwrap();
            wall.at(1.0, 0.0).z - wall.at(0.0, 0.0).z
        };
        assert_eq!(
            length(2.0),
            length(5.0),
            "a short journey keeps the least length"
        );
        assert!(length(20.0) > length(5.0));
        assert_eq!(length(200.0), length(400.0), "and the longest stops there");
    }

    #[test]
    fn a_bar_sits_on_its_row_within_its_share_of_time() {
        let points = [Vec3::ZERO, Vec3::new(100.0, 100.0, 0.0)];
        let wall = Wall::beside(Shape::Square, &points, 3, 4.0).unwrap();
        let [start, end, top_end, _] = wall.bar(0.25, 0.5, 1)[0];
        let (bottom, top) = (wall.at(0.0, level(1.0)).y, wall.at(0.0, level(2.0)).y);
        assert!(
            start.y > bottom && top_end.y < top,
            "inside its row, with a gap"
        );
        assert!((start.z - wall.at(0.25, 0.0).z).abs() < 1e-4);
        assert!(end.z < wall.at(0.75, 0.0).z, "a gap before the next bar");
    }

    #[test]
    fn the_root_is_a_footing_below_full_rows() {
        assert!(level(1.0) < 1.0, "the root's row is short");
        assert!(
            (level(3.0) - level(2.0) - 1.0).abs() < 1e-6,
            "the rest are full"
        );
        assert!((level(0.5) - level(1.0) / 2.0).abs() < 1e-6);
    }

    #[test]
    fn the_lens_widens_the_stretch_round_the_cursor_and_keeps_the_rest() {
        let lens = Some((10.0, 22.0));
        let width = |a: f32, b: f32| warp(b, 100.0, lens) - warp(a, 100.0, lens);
        assert!(warp(0.0, 100.0, lens).abs() < 1e-6);
        assert!(
            (warp(100.0, 100.0, lens) - 1.0).abs() < 1e-6,
            "the whole journey shows"
        );
        assert!(width(10.0, 22.0) >= FOCUS - 1e-6);
        assert!(
            width(11.0, 12.0) > 4.0 * width(50.0, 51.0),
            "a call near the cursor gets room"
        );
        let mut last = -1.0;
        for slot in 0..=100 {
            #[expect(clippy::cast_precision_loss, reason = "small numbers")]
            let at = warp(slot as f32, 100.0, lens);
            assert!(at > last, "in order");
            last = at;
        }
        assert!(
            (warp(30.0, 100.0, None) - 0.3).abs() < 1e-6,
            "evenly without it"
        );
    }

    #[test]
    fn a_short_journey_needs_no_lens_and_a_long_one_looks_ahead() {
        assert_eq!(lens_around(3.0, 10.0), None);
        assert_eq!(lens_around(40.0, 100.0), Some((38.0, 46.0)));
        assert_eq!(
            lens_around(0.0, 100.0),
            Some((0.0, 8.0)),
            "held inside the start"
        );
        assert_eq!(lens_around(99.0, 100.0), Some((92.0, 100.0)), "and the end");
    }

    #[test]
    fn the_arc_curves_round_the_graph_towards_the_viewer_and_stops_short() {
        let points = [Vec3::new(-50.0, -50.0, 0.0), Vec3::new(50.0, 50.0, 0.0)];
        let wall = Wall::beside(Shape::Arc, &points, 2, 400.0).unwrap();
        let (start, end) = (wall.at(0.0, 0.0), wall.at(1.0, 0.0));
        let axis = Vec3::new(0.0, start.y, 0.0);
        assert!(
            (start.distance(axis) - end.distance(axis)).abs() < 1e-3,
            "on a circle"
        );
        assert!(end.z > start.z, "towards the viewer");
        assert!(end.x > 0.0, "never round in front of the graph");
        assert!(
            wall.bar(0.0, 1.0, 1).len() > 4,
            "a long curved bar comes in pieces"
        );
    }

    #[test]
    fn the_slant_faces_the_usual_view() {
        let points = [Vec3::ZERO, Vec3::new(100.0, 100.0, 0.0)];
        let wall = Wall::beside(Shape::Slant, &points, 2, 4.0).unwrap();
        let run = (wall.at(1.0, 0.0) - wall.at(0.0, 0.0)).normalize();
        let view = Quat::from_euler(EulerRot::YXZ, VIEW.0, VIEW.1, 0.0) * Vec3::Z;
        assert!(run.dot(view).abs() < 1e-4, "square to the line of sight");
        assert_eq!(wall.bar(0.0, 1.0, 1).len(), 1);
    }

    #[test]
    fn a_name_breaks_at_its_words_and_is_cut_only_when_it_must() {
        assert_eq!(words("draw_graph_labels"), ["draw_", "graph_", "labels"]);
        assert_eq!(words("tilePoint"), ["tile", "Point"]);
        let parts = words("draw_graph_labels");
        let (lines, whole) = pack(&parts, 7, 3);
        assert_eq!(lines, ["draw_", "graph_", "labels"]);
        assert!(whole);
        let (lines, whole) = pack(&parts, 12, 1);
        assert_eq!(lines, ["draw_graph_…"]);
        assert!(!whole);
        let (lines, whole) = pack(&["abcdefgh".to_owned()], 3, 3);
        assert!(!whole, "a cut word counts against it");
        assert_eq!(
            lines,
            ["abc", "def", "gh"],
            "a long word is cut across lines"
        );
    }

    #[test]
    fn a_name_stands_upright_inside_its_bar_either_way_round() {
        let quad = [
            egui::pos2(0.0, 20.0),
            egui::pos2(40.0, 24.0),
            egui::pos2(40.0, 4.0),
            egui::pos2(0.0, 0.0),
        ];
        let rect = upright(&[quad]).unwrap();
        assert_eq!(
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 4.0), egui::pos2(40.0, 20.0))
        );
        let mirrored = quad.map(|p| egui::pos2(-p.x, p.y));
        assert_eq!(upright(&[mirrored]).unwrap().width(), 40.0);
    }

    #[test]
    fn calls_run_in_the_order_they_start_a_caller_first() {
        let placed = HashMap::from([
            ('a', (0.0, 3.0, 0)),
            ('c', (1.0, 2.0, 1)),
            ('b', (0.0, 1.0, 1)),
            ('d', (1.0, 1.0, 2)),
        ]);
        assert_eq!(running_order(&placed), vec!['a', 'b', 'c', 'd']);
    }

    #[test]
    fn beams_fade_from_the_cursor_and_stop_a_few_calls_away() {
        assert!((beam_strength(5, 5) - 1.0).abs() < f32::EPSILON);
        assert!(beam_strength(4, 5) < 1.0);
        assert!((beam_strength(4, 5) - beam_strength(6, 5)).abs() < f32::EPSILON);
        assert!(beam_strength(7, 5) > 0.0);
        assert!(beam_strength(8, 5).abs() < f32::EPSILON, "no hairball");
    }

    #[test]
    fn a_point_is_inside_a_quad_either_way_round() {
        let quad = [
            egui::pos2(0.0, 0.0),
            egui::pos2(10.0, 2.0),
            egui::pos2(10.0, 8.0),
            egui::pos2(0.0, 10.0),
        ];
        let mut reversed = quad;
        reversed.reverse();
        for quad in [quad, reversed] {
            assert!(inside(egui::pos2(5.0, 5.0), &quad));
            assert!(!inside(egui::pos2(5.0, 0.5), &quad));
            assert!(!inside(egui::pos2(11.0, 5.0), &quad));
        }
    }
}
