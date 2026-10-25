//! The journey standing in the scene: its flame graph as a wall square to the graph,
//! beside it, with time running along the depth the graph does not use and each level
//! of calls a row up the wall. The graph does not move. Beams tie the bars around the
//! cursor to their nodes, so the order the wall reads and the places the graph shows
//! meet without a line through every call.
//!
//! `Z` stands it up and turns the camera to look into the corner the wall makes with
//! the graph, where both read; [`Chrome`] holds whether it stands.

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
    ui::{Chrome, short},
};

/// Whether the wall stands, and whether it lit the graph's hovered node, so it can
/// clear it again.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TimeWall {
    pub(crate) on: bool,
    lit: bool,
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

/// The field of view while the wall stands, narrower than the usual so the camera
/// stands further back: the wall runs towards the viewer, and from close by its near
/// end would loom over its far one.
const FOV: f32 = 0.38;

/// Where the wall stands, in the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Wall {
    /// Its corner nearest the graph at the bottom, where the journey starts.
    foot: Vec3,
    /// How far time runs along z, start to end.
    length: f32,
    /// A row's height.
    row: f32,
    rows: usize,
}

impl Wall {
    /// A wall for `rows` levels of calls and `slots` across, standing just past the
    /// right edge of `points`, the graph as it shows, centered on it top to bottom,
    /// and running from the graph's plane towards the viewer.
    pub(crate) fn beside(points: &[Vec3], rows: usize, slots: f32) -> Option<Self> {
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
        Some(Self {
            foot: Vec3::new(max.x + GAP * extent, middle.y - height / 2.0, middle.z),
            length: (slots * SLOT).clamp(SHORT, LONG) * extent,
            row,
            rows,
        })
    }

    /// How high the wall stands, in rows.
    fn top(&self) -> f32 {
        #[expect(clippy::cast_precision_loss, reason = "a few rows")]
        level(self.rows as f32)
    }

    /// The point `along` the way from the journey's start to its end, `up` rows high.
    pub(crate) fn at(&self, along: f32, up: f32) -> Vec3 {
        self.foot + Vec3::new(0.0, up * self.row, along * self.length)
    }

    /// The wall's outline: along the bottom, then back along the top.
    pub(crate) fn corners(&self) -> [Vec3; 4] {
        let top = self.top();
        [
            self.at(0.0, 0.0),
            self.at(1.0, 0.0),
            self.at(1.0, top),
            self.at(0.0, top),
        ]
    }

    /// A bar's corners, `left` and `width` in shares of the journey on `row`, the
    /// root's at the bottom: bottom start, bottom end, top end, top start.
    pub(crate) fn bar(&self, left: f32, width: f32, row: usize) -> [Vec3; 4] {
        #[expect(clippy::cast_precision_loss, reason = "a few rows")]
        let row = row as f32;
        let (bottom, top) = (level(row + ROW_GAP / 2.0), level(row + 1.0 - ROW_GAP / 2.0));
        let end = left + width * (1.0 - BAR_GAP);
        [
            self.at(left, bottom),
            self.at(end, bottom),
            self.at(end, top),
            self.at(left, top),
        ]
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

/// Calls either side of the cursor that get a beam.
const NEAR: usize = 3;

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
    1.0 - away * 0.22
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

/// Where each span sits in the flame graph: left edge and width in slots, and row.
type Placed = HashMap<SpanId, (f32, f32, usize)>;

/// The journey's bars as disclosed, their slots across, and the wall to stand them on.
fn stand(explorer: &Explorer) -> Option<(Wall, Placed, f32)> {
    let disclosed = explorer.disclosure();
    let (placed, slots) = disclosed.layout(explorer.journey()?.root());
    let wall = Wall::beside(&journey_points(explorer), disclosed.rows(), slots)?;
    Some((wall, placed, slots.max(1.0)))
}

/// Where the camera looks into the corner from, and how far back: far enough for the
/// wall and the journey's nodes on the graph.
fn corner_view(explorer: &Explorer, perspective: &PerspectiveProjection) -> Option<(Vec3, f32)> {
    let (wall, _, _) = stand(explorer)?;
    let mut points = wall.corners().to_vec();
    points.extend(journey_points(explorer));
    let (min, max) = points.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(min, max), &p| (min.min(p), max.max(p)),
    );
    let center = (min + max) / 2.0;
    let rotation = Quat::from_euler(EulerRot::YXZ, VIEW.0, VIEW.1, 0.0);
    Some((center, fit_distance(&points, center, rotation, perspective)))
}

/// What the corner view was last worked out for, and where it looks from.
#[derive(Default)]
pub(crate) struct Framed {
    on: bool,
    key: Option<(Option<usize>, u64, usize, u32)>,
    view: Option<(Vec3, f32)>,
    /// The wall the view was fitted to.
    wall: Option<Wall>,
}

impl Wall {
    /// Whether the wall has moved or grown far enough from `before` to refit the view,
    /// as the graph settles: a share of its length either way.
    fn moved_from(&self, before: &Self) -> bool {
        let shift = self.foot.distance(before.foot) + (self.length - before.length).abs();
        shift > 0.12 * before.length
    }
}

/// Turns to look into the corner when the wall stands up, and holds that view while
/// it stands, refitting it to a new journey or a level more or less, until the viewer
/// pans or zooms away; turns back to the journey's start when it goes.
pub(crate) fn frame(
    chrome: Res<Chrome>,
    explorer: Res<Explorer>,
    mut last: Local<Framed>,
    time: Res<Time>,
    mut cameras: Query<(&mut OrbitCamera, &mut Projection)>,
) {
    let raised = chrome.wall.on && !last.on;
    let lowered = !chrome.wall.on && last.on;
    last.on = chrome.wall.on;
    let ease = 1.0 - (-6.0 * time.delta_secs()).exp();
    for (mut camera, mut projection) in &mut cameras {
        let Projection::Perspective(perspective) = &mut *projection else {
            continue;
        };
        let fov = if chrome.wall.on {
            FOV
        } else {
            PerspectiveProjection::default().fov
        };
        if (perspective.fov - fov).abs() > 1e-4 {
            perspective.fov += (fov - perspective.fov) * ease;
        }
        let narrow = PerspectiveProjection {
            fov: FOV,
            ..perspective.clone()
        };
        if lowered {
            camera.turn_to(ANGLED);
            orbit_anchor(&explorer, &mut camera);
        }
        if !chrome.wall.on {
            last.view = None;
            continue;
        }
        let disclosed = explorer.disclosure();
        let slots = explorer
            .journey()
            .map_or(0.0, |j| disclosed.layout(j.root()).1);
        let key = (
            explorer.anchor(),
            explorer.aim(),
            disclosed.rows(),
            slots.to_bits(),
        );
        let wall = stand(&explorer).map(|(wall, _, _)| wall);
        let moved = match (&wall, &last.wall) {
            (Some(wall), Some(before)) => wall.moved_from(before),
            (now, before) => now.is_some() != before.is_some(),
        };
        if raised || last.key != Some(key) || moved && !camera.free {
            last.key = Some(key);
            last.wall = wall;
            last.view = corner_view(&explorer, &narrow);
            if raised {
                camera.turn_to(VIEW);
            }
            if last.view.is_some() {
                camera.free = false;
            }
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
    /// Its corners: bottom start, bottom end, top end, top start.
    quad: [egui::Pos2; 4],
    fill: egui::Color32,
    text: egui::Color32,
}

impl Bar {
    /// Where its call starts: midway up its near edge, a little in.
    fn start(&self) -> egui::Pos2 {
        let near = self.quad[0] + (self.quad[3] - self.quad[0]) / 2.0;
        let far = self.quad[1] + (self.quad[2] - self.quad[1]) / 2.0;
        near + (far - near).normalized() * 4.0
    }
}

/// The wall as it stands this frame, on screen.
struct Standing {
    outline: [egui::Pos2; 4],
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
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer: Option<egui::Pos2>,
    ) -> Option<Self> {
        let (wall, placed, slots) = stand(explorer)?;
        let on_screen = |corners: [Vec3; 4]| -> Option<[egui::Pos2; 4]> {
            Some([
                project(corners[0])?,
                project(corners[1])?,
                project(corners[2])?,
                project(corners[3])?,
            ])
        };
        let outline = on_screen(wall.corners())?;
        let track = explorer.track();
        let map = explorer.map();
        let cursor_span = explorer.selected();
        let cursor_enter = cursor_span
            .and_then(|c| track.span(c))
            .map_or(0, |s| s.enter);
        let stepping =
            cursor_span.is_some_and(|c| explorer.journey().is_some_and(|j| j.root() != c));
        let mut bars = Vec::new();
        for span in running_order(&placed) {
            let (left, width, row) = placed[&span];
            let Some(quad) = on_screen(wall.bar(left / slots, width / slots, row)) else {
                continue;
            };
            let future = stepping && track.span(span).is_some_and(|s| s.enter > cursor_enter);
            let (fill, text) = bar_look(explorer, diff, span, future);
            bars.push(Bar {
                span,
                tile: track.unit_id(span).and_then(|u| map.tile_of(u)),
                quad,
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
            let top = wall.top();
            Some([
                project(wall.at(left / slots, 0.0))?,
                project(wall.at(left / slots, top))?,
            ])
        });
        let hovered = pointer.and_then(|p| bars.iter().position(|b| inside(p, &b.quad)));
        // Edge-on, the wall is a sliver with nothing to read.
        let breadth = (outline[1] - outline[0]).length();
        (breadth > 40.0).then_some(Self {
            outline,
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
        painter.add(egui::Shape::convex_polygon(
            self.outline.to_vec(),
            BACKING,
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(40)),
        ));
        // Time runs along the top edge, away from the graph.
        if let Some(fit) = room((self.outline[2] - self.outline[3]).length(), 20.0) {
            label_along(
                painter,
                [self.outline[3], self.outline[2]],
                fit,
                -1.0,
                "time →",
                egui::Color32::from_white_alpha(120),
            );
        }
        let disclosed = explorer.disclosure();
        for (index, bar) in self.bars.iter().enumerate() {
            painter.add(egui::Shape::convex_polygon(
                bar.quad.to_vec(),
                bar.fill,
                egui::Stroke::NONE,
            ));
            let lit = Some(index) == self.hovered
                || bar.tile.is_some() && bar.tile == explorer.hovered_tile;
            if lit {
                painter.add(egui::Shape::closed_line(
                    bar.quad.to_vec(),
                    egui::Stroke::new(1.5, egui::Color32::from_white_alpha(170)),
                ));
            }
            let number = disclosed.number(bar.span).unwrap_or_default();
            // The function's own name: the bar's color already says its module.
            let label = explorer.label(bar.span);
            let name = label
                .rsplit_once('.')
                .map_or(label.as_str(), |(_, name)| name);
            let label = if number.is_empty() {
                name.to_owned()
            } else {
                format!("{number}  {name}")
            };
            label_bar(painter, &bar.quad, &label, bar.text);
        }
        if let Some(now) = self.now {
            painter.line_segment(
                now,
                egui::Stroke::new(1.0, egui::Color32::from_white_alpha(70)),
            );
        }
        if let Some(bar) = self.cursor.map(|c| &self.bars[c]) {
            painter.add(egui::Shape::closed_line(
                bar.quad.to_vec(),
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
    let standing = wall
        .on
        .then(|| Standing::of(explorer, diff, project, hover))
        .flatten();
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

/// The font size and the characters a name gets on a line `length` long in a band
/// `height` across; `None` where it would be too small to read.
pub(crate) fn room(length: f32, height: f32) -> Option<(f32, usize)> {
    let size = (height * 0.62).min(13.0);
    if size < 7.5 || length < 24.0 {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a small, clamped character count"
    )]
    let chars = ((length - 10.0) / (size * 0.56)).clamp(0.0, 300.0) as usize;
    (chars >= 3).then_some((size, chars))
}

/// Writes `text` along the line `ends` at `size`, cut to `chars`; `outside` lifts it
/// that many of its own heights off the line, below it.
fn label_along(
    painter: &egui::Painter,
    [from, to]: [egui::Pos2; 2],
    (size, chars): (f32, usize),
    outside: f32,
    text: &str,
    color: egui::Color32,
) {
    let galley =
        painter.layout_no_wrap(short(text, chars), egui::FontId::proportional(size), color);
    let direction = (to - from).normalized();
    // Square to the line, pointing down the screen as text reads.
    let down = egui::vec2(-direction.y, direction.x);
    let shift = galley.size().y * (outside - 0.5);
    let at = from + direction * 5.0 + down * shift;
    painter.add(
        egui::epaint::TextShape::new(at, galley, color).with_angle(direction.y.atan2(direction.x)),
    );
}

/// Names a bar along it, or up it like a book's spine where a narrow bar holds more
/// that way.
fn label_bar(painter: &egui::Painter, quad: &[egui::Pos2; 4], text: &str, color: egui::Color32) {
    let middle = |a: egui::Pos2, b: egui::Pos2| a + (b - a) / 2.0;
    let along = [middle(quad[0], quad[3]), middle(quad[1], quad[2])];
    let up = [middle(quad[0], quad[1]), middle(quad[3], quad[2])];
    let length = |[a, b]: [egui::Pos2; 2]| (b - a).length();
    let height = (quad[3] - quad[0])
        .length()
        .min((quad[2] - quad[1]).length());
    let width = (quad[1] - quad[0])
        .length()
        .min((quad[2] - quad[3]).length());
    let chars = text.chars().count();
    let line = match (room(length(along), height), room(length(up), width)) {
        (Some(a), Some(u)) if a.1 < chars && u.1 > a.1 + 2 => Some((up, u)),
        (Some(a), _) => Some((along, a)),
        (None, Some(u)) => Some((up, u)),
        (None, None) => None,
    };
    if let Some((line, fit)) = line {
        label_along(painter, line, fit, 0.0, text, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wall_stands_past_the_graph_and_runs_towards_the_viewer() {
        let points = [Vec3::new(-50.0, -20.0, 1.0), Vec3::new(50.0, 40.0, -1.0)];
        let wall = Wall::beside(&points, 3, 4.0).unwrap();
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
    fn a_long_journey_lengthens_the_wall_up_to_a_point() {
        let points = [Vec3::ZERO, Vec3::new(100.0, 100.0, 0.0)];
        let length = |slots| {
            let wall = Wall::beside(&points, 2, slots).unwrap();
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
        let wall = Wall::beside(&points, 3, 4.0).unwrap();
        let [start, end, top_end, _] = wall.bar(0.25, 0.5, 1);
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
        assert!(beam_strength(8, 5) > 0.0);
        assert!(beam_strength(9, 5).abs() < f32::EPSILON, "no hairball");
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
