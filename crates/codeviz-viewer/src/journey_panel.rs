//! The journey as a flame graph over the graph: time left to right, each call
//! on the row below its caller. Docked in a corner while the graph has the screen, or
//! filling the screen below a strip of graph. Each bar is painted the color of its
//! node, and the cursor, hover and focus are shared with the graph, so a bar and its
//! node light up together.

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::{Change, SpanId};
use codeviz_ir::ExitKind;

use crate::{
    beams::{self, Tick},
    diff_view::{DiffView, change_color, sign},
    disclosure::Disclose,
    explorer::{Confidence, Explorer, is_conditional, is_handoff},
    graph_view::node_color,
    scene::{PALETTE, lane_color},
    ui::{egui_color, journey_title, short},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PanelMode {
    Hidden,
    /// In the bottom-right corner over the graph.
    #[default]
    Docked,
    /// Below the graph, which shrinks to a strip above it.
    Full,
}

#[derive(Resource, Debug, Default)]
pub(crate) struct JourneyPanel {
    pub(crate) mode: PanelMode,
    /// Pixels per world unit of time; `None` fits the whole journey across.
    zoom: Option<f32>,
    /// The world time at the left edge.
    left: f32,
    /// The first row shown.
    top_row: f32,
    /// The cursor the view last scrolled to.
    followed: Option<(u64, SpanId)>,
    /// Whether the panel set the graph's hovered node, so it can clear it again.
    lit_node: bool,
    /// Whether beams tie the journey's calls to their nodes (Z).
    pub(crate) beams: bool,
}

impl JourneyPanel {
    /// Whether beams show: they need the journey on screen.
    pub(crate) fn beaming(&self) -> bool {
        self.beams && self.mode != PanelMode::Hidden
    }
}

/// Share of the screen's height the graph keeps above a full-size journey.
const GRAPH_STRIP: f32 = 0.34;
const MARGIN: f32 = 12.0;
const HEADER: f32 = 28.0;
const PAD: f32 = 8.0;
const ROW: f32 = 22.0;
const ROW_GAP: f32 = 5.0;
/// Share of a slot left between consecutive bars on a row.
const BAR_GAP: f32 = 0.06;
/// Pixels a slot needs to name its call.
const NAMED_SLOT: f32 = 110.0;

const BACKGROUND: egui::Color32 = egui::Color32::from_rgba_premultiplied(11, 13, 19, 238);
const MUTED: egui::Color32 = egui::Color32::from_rgb(140, 148, 165);
const ERROR: egui::Color32 = egui::Color32::from_rgb(240, 110, 100);
const OK: egui::Color32 = egui::Color32::from_rgb(120, 210, 140);

/// Where the graph renders and where the panel sits, within the space between the
/// side panels.
pub(crate) fn split(
    panel: &JourneyPanel,
    area: egui::Rect,
    (rows, slots): (usize, f32),
) -> (egui::Rect, Option<egui::Rect>) {
    match panel.mode {
        PanelMode::Hidden => (area, None),
        // Beams rise from the journey's top edge, so it takes the bottom full width,
        // as tall as it needs, and the graph the space above it.
        PanelMode::Docked if panel.beams => {
            let height = (beams::STRIP + HEADER + PAD * 2.0 + rows.max(2) as f32 * (ROW + ROW_GAP))
                .min((area.height() * 0.4).max(160.0))
                .min(area.height() * 0.6);
            let top = area.max.y - height;
            (
                egui::Rect::from_min_max(area.min, egui::pos2(area.max.x, top)),
                Some(egui::Rect::from_min_max(
                    egui::pos2(area.min.x, top),
                    area.max,
                )),
            )
        }
        PanelMode::Docked => {
            let size = egui::vec2(
                // Wide enough to name each call, up to the whole width.
                (slots * NAMED_SLOT + PAD * 2.0)
                    .clamp(380.0, 960.0_f32.max(area.width() * 0.52))
                    .min(area.width() - MARGIN * 2.0),
                // As tall as the journey needs, up to a share of the screen.
                (HEADER + PAD * 2.0 + rows.max(2) as f32 * (ROW + ROW_GAP))
                    .min((area.height() * 0.45).max(200.0))
                    .min(area.height() - MARGIN * 2.0),
            );
            let min = area.max - size - egui::vec2(MARGIN, MARGIN);
            (area, Some(egui::Rect::from_min_size(min, size)))
        }
        PanelMode::Full => {
            let strip = area.height() * GRAPH_STRIP;
            let graph =
                egui::Rect::from_min_max(area.min, egui::pos2(area.max.x, area.min.y + strip));
            let panel = egui::Rect::from_min_max(egui::pos2(area.min.x, graph.max.y), area.max);
            (graph, Some(panel))
        }
    }
}

/// One bar on screen.
struct Bar {
    span: SpanId,
    rect: egui::Rect,
    /// Whether it is disclosed; folded bars keep their place but are not drawn.
    shown: bool,
}

/// How many rows the journey takes, and how many slots across.
pub(crate) fn extent(explorer: &Explorer) -> (usize, f32) {
    let disclosed = explorer.disclosure();
    let slots = explorer
        .journey()
        .map_or(0.0, |j| disclosed.layout(j.root()).1);
    (disclosed.rows(), slots)
}

/// The bars a tether runs from: the cursor's, and the one under the pointer; and
/// while beams show, the strip of calls they rise from, with the cursor's place on it.
#[derive(Default)]
pub(crate) struct Tethers {
    pub(crate) cursor: Option<egui::Rect>,
    pub(crate) hovered: Option<egui::Rect>,
    pub(crate) ticks: Vec<Tick>,
    pub(crate) cursor_tick: Option<usize>,
}

/// Draws the panel and handles its input. Returns the bars to tether to their nodes
/// in the graph.
pub(crate) fn draw(
    ctx: &egui::Context,
    rect: egui::Rect,
    explorer: &mut Explorer,
    panel: &mut JourneyPanel,
    diff: Option<&DiffView>,
) -> Tethers {
    let mut tethers = Tethers::default();
    egui::Area::new("journey_panel".into())
        .fixed_pos(rect.min)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_clip_rect(rect);
            ui.allocate_rect(rect, egui::Sense::hover());
            let rounding = if panel.mode == PanelMode::Full || panel.beams {
                0.0
            } else {
                8.0
            };
            ui.painter().rect(
                rect,
                rounding,
                BACKGROUND,
                egui::Stroke::new(1.0, egui::Color32::from_white_alpha(28)),
                egui::StrokeKind::Inside,
            );
            let strip = if panel.beams { beams::STRIP } else { 0.0 };
            let header = egui::Rect::from_min_size(
                rect.min + egui::vec2(0.0, strip),
                egui::vec2(rect.width(), HEADER),
            );
            header_bar(ui, header.shrink2(egui::vec2(10.0, 2.0)), explorer, panel);
            let content = egui::Rect::from_min_max(egui::pos2(rect.min.x, header.max.y), rect.max)
                .shrink(PAD);
            tethers = canvas(ui, content, explorer, panel, diff);
            if panel.beams {
                let strip = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), strip));
                (tethers.ticks, tethers.cursor_tick) = draw_strip(ui, strip, explorer, diff);
            }
        });
    tethers
}

/// Every disclosed call once, in the order they run, each filled like its bar; what
/// runs after the cursor dims, as its bars do. Returns where each call sits on it and
/// which is the cursor's.
fn draw_strip(
    ui: &egui::Ui,
    rect: egui::Rect,
    explorer: &Explorer,
    diff: Option<&DiffView>,
) -> (Vec<Tick>, Option<usize>) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(90));
    let track = explorer.track();
    let disclosed = explorer.disclosure();
    let (calls, cursor) = beams::calls(explorer);
    let inner = rect.shrink2(egui::vec2(PAD, 3.0));
    let pitch = inner.width() / calls.len().max(1) as f32;
    let gap = if pitch > 5.0 { 1.0 } else { 0.0 };
    let ticks: Vec<Tick> = calls
        .iter()
        .enumerate()
        .map(|(index, &(span, tile))| Tick {
            span,
            rect: egui::Rect::from_min_size(
                egui::pos2(inner.min.x + index as f32 * pitch, inner.min.y),
                egui::vec2((pitch - gap).max(1.0), inner.height()),
            ),
            tile,
        })
        .collect();
    for (index, tick) in ticks.iter().enumerate() {
        let base = egui_color(tick.tile.map_or_else(
            || lane_color(track.color(tick.span) % PALETTE.len()),
            |tile| node_color(explorer, diff, tile),
        ));
        let future = cursor.is_some_and(|c| index > c);
        let fill = if future {
            base.gamma_multiply(0.38)
        } else {
            base
        };
        painter.rect_filled(tick.rect, 2.0, fill);
        let number = disclosed.number(tick.span).unwrap_or_default();
        if !number.is_empty() && tick.rect.width() >= 6.5 * number.len() as f32 + 4.0 {
            let text = if future {
                egui::Color32::from_rgb(200, 205, 216)
            } else {
                egui::Color32::from_rgb(14, 16, 22)
            };
            painter.text(
                tick.rect.center(),
                egui::Align2::CENTER_CENTER,
                number,
                egui::FontId::proportional(11.0),
                text,
            );
        }
    }
    if let Some(tick) = cursor.and_then(|c| ticks.get(c)) {
        painter.rect_stroke(
            tick.rect.expand(1.0),
            2.0,
            egui::Stroke::new(2.0, egui::Color32::WHITE),
            egui::StrokeKind::Outside,
        );
    }
    (ticks, cursor)
}

/// How much of the journey shows: levels of calls below the root, or everything up to
/// the cursor in time.
fn disclosure(ui: &mut egui::Ui, explorer: &mut Explorer) {
    let mut disclose = explorer.disclose();
    let by_time = disclose == Disclose::Time;
    if ui
        .selectable_label(by_time, "time")
        .on_hover_text("Show calls as the cursor reaches them: ] and [ step, Space plays (V)")
        .clicked()
    {
        disclose = if by_time {
            Disclose::default()
        } else {
            Disclose::Time
        };
    }
    if let Disclose::Depth(depth) = disclose {
        if ui
            .small_button("+")
            .on_hover_text("One more level of calls (.)")
            .clicked()
        {
            disclose = Disclose::Depth(depth + 1);
        }
        ui.label(egui::RichText::new(format!("{depth}")).small());
        if ui
            .small_button("−")
            .on_hover_text("One level of calls fewer (,)")
            .clicked()
        {
            disclose = Disclose::Depth(depth.saturating_sub(1).max(1));
        }
        ui.label(egui::RichText::new("depth").small().color(MUTED));
    }
    if disclose != explorer.disclose() {
        explorer.set_disclose(disclose);
    }
}

fn header_bar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    explorer: &mut Explorer,
    panel: &mut JourneyPanel,
) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            // The controls first, so the title takes what room is left.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("×")
                    .on_hover_text("Hide the journey (J)")
                    .clicked()
                {
                    panel.mode = PanelMode::Hidden;
                }
                let (label, hint, next) = match panel.mode {
                    PanelMode::Full => ("⤡", "Give the graph the screen (M)", PanelMode::Docked),
                    _ => ("⤢", "Give the journey the screen (M)", PanelMode::Full),
                };
                if ui.small_button(label).on_hover_text(hint).clicked() {
                    panel.mode = next;
                }
                if ui
                    .small_button("Fit")
                    .on_hover_text("Fit the whole journey (double-click)")
                    .clicked()
                {
                    panel.zoom = None;
                }
                disclosure(ui, explorer);
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    title(ui, explorer);
                });
            });
        },
    );
}

/// The journey's name and, when one is picked, the outcome it follows.
fn title(ui: &mut egui::Ui, explorer: &Explorer) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(journey_title(explorer))
                .color(egui::Color32::WHITE)
                .strong(),
        )
        .truncate(),
    );
    // The outcome only where it fits whole: the outline names it too.
    if let Some(scenario) = explorer
        .scenario()
        .and_then(|i| explorer.scenarios().get(i))
        .filter(|s| ui.available_width() > 16.0 + 6.0 * s.outcome.label.chars().count() as f32)
    {
        let (mark, color) = match scenario.outcome.kind {
            ExitKind::Return => ("✔", OK),
            _ => ("×", ERROR),
        };
        ui.label(
            egui::RichText::new(format!("{mark} {}", scenario.outcome.label))
                .color(color)
                .small(),
        );
    }
}

#[expect(clippy::too_many_lines, reason = "one pass over the bars")]
fn canvas(
    ui: &mut egui::Ui,
    content: egui::Rect,
    explorer: &mut Explorer,
    panel: &mut JourneyPanel,
    diff: Option<&DiffView>,
) -> Tethers {
    let response = ui.interact(
        content,
        ui.id().with("journey_canvas"),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter_at(content);
    let track = explorer.track();
    let scene = track.scene();
    if scene.nodes.is_empty() {
        painter.text(
            content.center(),
            egui::Align2::CENTER_CENTER,
            "No journey here. Focus a node and press Enter to trace from it.",
            egui::FontId::proportional(13.0),
            MUTED,
        );
        return Tethers::default();
    }
    // Only what is disclosed is laid out, a slot per call.
    let disclosed = explorer.disclosure();
    let (placed, slots) = track
        .journey()
        .map(|j| disclosed.layout(j.root()))
        .unwrap_or_default();
    let start = 0.0;
    let end = slots.max(1.0);
    let fit = content.width() / (end - start);
    let scale = panel.zoom.unwrap_or(fit);
    if panel.zoom.is_none() {
        panel.left = start;
    }
    let pitch = ROW + ROW_GAP;
    let visible_rows = (content.height() / pitch).floor().max(1.0);
    let max_top = (disclosed.rows() as f32 - visible_rows).max(0.0);

    // Zoom about the pointer: scroll or pinch. Drag pans; double-click fits.
    if response.hovered() {
        let (scroll, spread) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        let factor = spread * (scroll.y * 0.004).exp();
        if (factor - 1.0).abs() > 1e-4
            && let Some(pointer) = response.hover_pos()
        {
            let at = panel.left + (pointer.x - content.min.x) / scale;
            let zoomed = (scale * factor).clamp(fit.min(0.5), 400.0);
            panel.zoom = Some(zoomed);
            panel.left = at - (pointer.x - content.min.x) / zoomed;
        }
        if scroll.x.abs() > 0.0 {
            panel.zoom = Some(scale);
            panel.left -= scroll.x / scale;
        }
    }
    if response.dragged() {
        let delta = response.drag_delta();
        panel.zoom = Some(panel.zoom.unwrap_or(fit));
        panel.left -= delta.x / scale;
        panel.top_row -= delta.y / pitch;
    }
    if response.double_clicked() {
        panel.zoom = None;
        panel.left = start;
        panel.top_row = 0.0;
    }

    // Keep the cursor in view when it moves.
    let cursor = track.selected();
    let followed = cursor.map(|c| (explorer.revision(), c));
    if followed != panel.followed {
        panel.followed = followed;
        if let Some(&(x, _, row)) = cursor.and_then(|c| placed.get(&c)) {
            let width = content.width() / scale;
            if x < panel.left || x > panel.left + width * 0.9 {
                panel.left = x - width * 0.15;
            }
            let row = row as f32;
            if row < panel.top_row || row > panel.top_row + visible_rows - 1.0 {
                panel.top_row = row - (visible_rows / 3.0).floor();
            }
        }
    }
    panel.top_row = panel.top_row.clamp(0.0, max_top);
    let scale = panel.zoom.unwrap_or(fit);
    let x_of = |x: f32| content.min.x + (x - panel.left) * scale;
    let y_of = |row: f32| content.min.y + (row - panel.top_row) * pitch;

    let map = explorer.map();
    let cursor_enter = cursor.and_then(|c| track.span(c)).map_or(0, |s| s.enter);
    let graph_focus = explorer.graph_focus();
    let stepping = cursor.is_some_and(|c| track.journey().is_some_and(|j| j.root() != c));
    let tile_of = |span: SpanId| track.unit_id(span).and_then(|u| map.tile_of(u));

    let bars: Vec<Bar> = scene
        .nodes
        .iter()
        .map(|node| match placed.get(&node.span) {
            Some(&(left, slots, row)) => Bar {
                span: node.span,
                rect: egui::Rect::from_min_size(
                    egui::pos2(x_of(left), y_of(row as f32)),
                    egui::vec2(((slots - BAR_GAP) * scale).max(3.0), ROW),
                ),
                shown: true,
            },
            None => Bar {
                span: node.span,
                rect: egui::Rect::NOTHING,
                shown: false,
            },
        })
        .collect();
    // A closure has no bar of its own: it is drawn as part of the function around it.
    let rect_of = |span: SpanId| {
        let mut at = Some(span);
        while let Some(span) = at {
            if let Some(bar) = bars.get(span.index()).filter(|b| b.shown) {
                return Some(bar.rect);
            }
            at = track.span(span).and_then(|s| s.parent);
        }
        None
    };

    // A node hovered in the graph lights its bars here.
    let graph_hover = explorer.hovered_tile.filter(|_| !panel.lit_node);
    // Caller to callee: a short drop from the caller's row.
    for edge in &scene.edges {
        if !disclosed.shows(edge.callee) {
            continue;
        }
        let (Some(from), Some(to), Some(span)) = (
            rect_of(edge.caller),
            rect_of(edge.callee),
            track.span(edge.callee),
        ) else {
            continue;
        };
        let x = to.min.x + 2.0;
        let points = [egui::pos2(x, from.max.y), egui::pos2(x, to.min.y)];
        // Dashes along an unbounded line never end.
        if !points.iter().all(|p| p.is_finite()) {
            continue;
        }
        let stroke = egui::Stroke::new(1.0, egui::Color32::from_white_alpha(70));
        if is_conditional(span) {
            painter.extend(egui::Shape::dashed_line(&points, stroke, 3.0, 3.0));
        } else {
            painter.line_segment(points, stroke);
        }
    }

    let mouse = response.hover_pos();
    let mut hovered = None;
    let mut fold_hit = None;
    for bar in &bars {
        let Some(span) = track.span(bar.span) else {
            continue;
        };
        if !bar.shown || !bar.rect.intersects(content) {
            continue;
        }
        if mouse.is_some_and(|p| bar.rect.contains(p)) {
            hovered = Some(bar.span);
        }
        let tile = tile_of(bar.span);
        let base = egui_color(tile.map_or_else(
            || lane_color(track.color(bar.span) % PALETTE.len()),
            |tile| node_color(explorer, diff, tile),
        ));
        let faint = Confidence::of(span) == Confidence::Guessed || is_handoff(span);
        // What runs after the cursor dims once you step through it; at the start,
        // everything shows in full.
        let future = stepping && span.enter > cursor_enter;
        let (fill, text) = if faint {
            (base.gamma_multiply(0.22), MUTED)
        } else if future {
            (
                base.gamma_multiply(0.38),
                egui::Color32::from_rgb(200, 205, 216),
            )
        } else {
            (base, egui::Color32::from_rgb(14, 16, 22))
        };
        painter.rect_filled(bar.rect, 3.0, fill);

        // Under a diff, what changed is outlined too; a call that is new, though what
        // it calls is not, counts as added.
        let new_call = diff
            .zip(span.parent.and_then(tile_of))
            .zip(tile)
            .is_some_and(|((diff, from), to)| diff.call_change(from, to) == Some(Change::Added));
        let change = diff
            .zip(tile)
            .and_then(|(diff, tile)| diff.change(tile))
            .or(new_call.then_some(Change::Added));
        // A call that only sometimes runs is outlined in dashes, in its change's color
        // under a diff.
        let outline = change.map(|c| egui::Stroke::new(1.5, egui_color(change_color(c))));
        if is_conditional(span) {
            let stroke =
                outline.unwrap_or(egui::Stroke::new(1.2, egui::Color32::from_white_alpha(190)));
            dashed_outline(&painter, bar.rect.shrink(0.75), stroke);
        } else if let Some(stroke) = outline {
            painter.rect_stroke(bar.rect, 3.0, stroke, egui::StrokeKind::Inside);
        }

        // How it can end, marked at its right end: green returns, red fails.
        for exit in &span.exits {
            let x = bar.rect.max.x;
            let mark = egui::Rect::from_min_max(
                egui::pos2(x - 3.0, bar.rect.min.y),
                egui::pos2(x, bar.rect.max.y),
            );
            let color = if exit.kind == ExitKind::Return {
                OK
            } else {
                ERROR
            };
            painter.rect_filled(mark.intersect(bar.rect), 0.0, color);
        }

        // What is folded under the bar, as a badge that opens it.
        let folded = disclosed.folded(bar.span);
        let badge = (folded > 0 && bar.rect.width() > 40.0).then(|| {
            let badge = egui::Rect::from_min_max(
                egui::pos2(bar.rect.max.x - 26.0, bar.rect.min.y + 3.0),
                egui::pos2(bar.rect.max.x - 3.0, bar.rect.max.y - 3.0),
            );
            painter.rect_filled(badge, 3.0, egui::Color32::from_black_alpha(120));
            painter.text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                format!("+{folded}"),
                egui::FontId::proportional(10.5),
                egui::Color32::from_rgb(220, 224, 234),
            );
            if mouse.is_some_and(|p| badge.contains(p)) {
                fold_hit = Some(bar.span);
            }
            badge
        });
        if graph_hover.is_some() && tile == graph_hover {
            painter.rect_stroke(
                bar.rect.expand(1.5),
                3.0,
                egui::Stroke::new(2.0, egui::Color32::from_white_alpha(200)),
                egui::StrokeKind::Outside,
            );
        }
        // The same number the graph puts on the node, then what changed and the name;
        // where the name would not fit, the number alone.
        let number = disclosed.number(bar.span).unwrap_or_default();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a small, clamped character count"
        )]
        let room = ((bar.rect.width() - if badge.is_some() { 38.0 } else { 10.0 }) / 6.8)
            .clamp(0.0, 300.0) as usize;
        let painter = painter.with_clip_rect(bar.rect.intersect(content));
        if room >= number.len() + 6 {
            let sign = change.map_or_else(String::new, |c| format!("{} ", sign(c)));
            let mut label = format!("{sign}{}", explorer.label(bar.span));
            if !number.is_empty() {
                label = format!("{number}  {label}");
            }
            if let Some(condition) = span.conditions.last() {
                label = format!("{label}  if {}", condition.arm);
            }
            painter.text(
                bar.rect.left_center() + egui::vec2(5.0, 0.0),
                egui::Align2::LEFT_CENTER,
                short(&label, room),
                egui::FontId::proportional(12.5),
                text,
            );
        } else {
            // Beside the badge, not under it.
            let free = badge.map_or(bar.rect, |badge| {
                egui::Rect::from_min_max(bar.rect.min, egui::pos2(badge.min.x, bar.rect.max.y))
            });
            if free.width() < 6.5 * number.len() as f32 + 3.0 {
                continue;
            }
            painter.text(
                free.center(),
                egui::Align2::CENTER_CENTER,
                number,
                egui::FontId::proportional(11.5),
                text,
            );
        }
        if tile.is_some() && tile == graph_focus && Some(bar.span) != cursor {
            painter.rect_stroke(
                bar.rect.expand(1.5),
                3.0,
                egui::Stroke::new(1.5, egui::Color32::WHITE),
                egui::StrokeKind::Outside,
            );
        }
    }

    // A red end on the bars where other outcomes split off, inside the bar so it
    // cannot be read as its neighbor's; click one to follow it.
    let mut divergence_hit = None;
    for divergence in track.divergences() {
        let Some(rect) = rect_of(divergence.span) else {
            continue;
        };
        let mark = egui::Rect::from_min_max(egui::pos2(rect.max.x - 5.0, rect.min.y), rect.max);
        painter.rect_filled(mark, 2.0, ERROR);
        if mouse.is_some_and(|p| mark.expand(3.0).contains(p)) {
            divergence_hit = Some((divergence.scenario, divergence.label.clone()));
        }
    }

    // With the focus on a node off the journey, the journey steps back behind it; the
    // cursor still shows where an arrow key returns to.
    if explorer.aside().is_some() {
        painter.rect_filled(content, 0.0, egui::Color32::from_black_alpha(140));
    }

    let mut cursor_rect = None;
    if let Some(rect) = cursor.and_then(rect_of) {
        painter.line_segment(
            [
                egui::pos2(rect.min.x, content.min.y),
                egui::pos2(rect.min.x, content.max.y),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(60)),
        );
        painter.rect_stroke(
            rect.expand(1.5),
            3.0,
            egui::Stroke::new(2.0, egui::Color32::WHITE),
            egui::StrokeKind::Outside,
        );
        cursor_rect = content.intersects(rect).then_some(rect);
    }

    if let Some((_, label)) = &divergence_hit {
        response
            .clone()
            .on_hover_text(format!("× {label} (click to follow)"));
    } else if let Some(span) = hovered {
        if let Some(rect) = rect_of(span) {
            painter.rect_stroke(
                rect.expand(1.0),
                3.0,
                egui::Stroke::new(1.0, egui::Color32::from_white_alpha(150)),
                egui::StrokeKind::Outside,
            );
        }
        let mut tip = explorer.label(span);
        if let Some(span) = track.span(span) {
            for condition in &span.conditions {
                let guard = condition.guard.as_deref().unwrap_or("?");
                tip = format!("{tip}\nonly when {guard} ⇒ {}", condition.arm);
            }
            for exit in &span.exits {
                let label = exit.label.as_deref().unwrap_or("returns");
                tip = format!("{tip}\nexits: {label}");
            }
        }
        response.clone().on_hover_text(tip);
    }

    let hovered_rect = hovered
        .and_then(rect_of)
        .filter(|rect| content.intersects(*rect));
    // Hovering a bar lights its node in the graph.
    let hovered_tile = hovered.and_then(tile_of);
    if hovered_tile.is_some() {
        if explorer.hovered_tile != hovered_tile {
            explorer.hovered_tile = hovered_tile;
        }
        panel.lit_node = true;
    } else if panel.lit_node {
        explorer.hovered_tile = None;
        panel.lit_node = false;
    }

    // Only a pointer's click: egui also clicks a widget it focuses on Enter or Space,
    // and those keys belong to the graph.
    if response.clicked_by(egui::PointerButton::Primary) {
        if let Some(span) = fold_hit {
            explorer.toggle_expanded(span);
        } else if let Some((scenario, _)) = divergence_hit {
            explorer.set_scenario(Some(scenario));
        } else if let Some(span) = hovered {
            explorer.select(Some(span));
        }
    }
    Tethers {
        cursor: cursor_rect,
        hovered: hovered_rect,
        ..Tethers::default()
    }
}

/// A rectangle's outline in dashes.
fn dashed_outline(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke) {
    // Dashes along an unbounded line never end.
    if !rect.is_finite() {
        return;
    }
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(&corners, stroke, 4.0, 3.0));
}

#[cfg(test)]
mod tests {
    use codeviz_core::{JourneyOptions, System};
    use codeviz_ir::{Document, UnitId};

    use super::*;

    /// `main` calls `a` from inside a closure: the closure runs on `main`'s node, so it
    /// has no bar of its own.
    fn explorer() -> Explorer {
        let unit = |id: &str, parent: Option<&str>, calls: &[&str]| {
            let calls: Vec<String> = calls
                .iter()
                .map(|c| {
                    format!(
                        r#"{{ "key": "{c}#0", "text": "{c}", "targets": [{{ "unit": "{c}", "certainty": "exact" }}] }}"#
                    )
                })
                .collect();
            let parent = parent.map_or_else(String::new, |p| format!(r#""parent": "{p}","#));
            format!(
                r#"{{ "id": "{id}", "name": "{id}", "kind": "fn", "role": "invocable", {parent} "calls": [{}] }}"#,
                calls.join(",")
            )
        };
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}, {}, {}] }}"#,
            unit("x:main", None, &["x:main::{closure#0}"]),
            unit("x:main::{closure#0}", Some("x:main"), &["x:a"]),
            unit("x:a", None, &[]),
        );
        let system = System::new([Document::from_json(&json).unwrap()]);
        Explorer::new(
            system,
            Some(UnitId::new("x:main")),
            JourneyOptions::default(),
        )
    }

    #[test]
    fn a_call_from_a_closure_draws_from_its_function() {
        let mut explorer = explorer();
        let journey = explorer.journey().unwrap();
        let closure = journey.span(journey.root()).unwrap().children[0];
        let disclosed = explorer.disclosure();
        assert!(disclosed.shows(closure), "the closure shows with main");
        assert_eq!(
            disclosed.number(journey.span(closure).unwrap().children[0]),
            Some("1"),
            "a is main's first call"
        );

        let ctx = egui::Context::default();
        let mut panel = JourneyPanel::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 300.0));
        // An area is sized on its first frame and drawn from the second.
        let mut frame = || {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| {
                    draw(ui.ctx(), screen, &mut explorer, &mut panel, None);
                },
            );
            output.textures_delta.clear();
            output
        };
        frame();
        let output = frame();
        let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
        let vertices: Vec<egui::Pos2> = meshes
            .iter()
            .filter_map(|m| match &m.primitive {
                egui::epaint::Primitive::Mesh(mesh) => Some(mesh.vertices.iter().map(|v| v.pos)),
                egui::epaint::Primitive::Callback(_) => None,
            })
            .flatten()
            .collect();
        assert!(vertices.len() > 4, "the panel draws");
        assert!(
            vertices.iter().all(|p| p.is_finite()),
            "nothing is drawn from a bar the closure does not have"
        );
    }
}
