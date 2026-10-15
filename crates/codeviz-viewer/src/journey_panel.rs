//! The active journey as a flame graph over the graph: time left to right, each call
//! on the row below its caller. Docked in a corner while the graph has the screen, or
//! filling the screen below a strip of graph. The cursor, hover and focus are shared
//! with the graph, so a bar and its node light up together.

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::SpanId;
use codeviz_ir::ExitKind;

use crate::{
    diff_view::{DiffView, change_color},
    explorer::{Confidence, Explorer, SpanRef, is_conditional, is_handoff},
    graph_view::{CALLEE, CALLER, Relation, relation},
    scene::{PALETTE, accent, lane_color},
    ui::{egui_color, short, track_title},
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
    followed: Option<(u64, SpanRef)>,
    /// Whether the panel set the graph's hovered node, so it can clear it again.
    lit_node: bool,
}

impl JourneyPanel {
    pub(crate) fn new(mode: PanelMode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }
}

/// Share of the screen's height the graph keeps above a full-size journey.
const GRAPH_STRIP: f32 = 0.34;
const MARGIN: f32 = 12.0;
const HEADER: f32 = 28.0;
const PAD: f32 = 8.0;
const ROW: f32 = 22.0;
const ROW_GAP: f32 = 5.0;
/// World units left between consecutive bars on a row.
const BAR_GAP: f32 = 0.2;

const BACKGROUND: egui::Color32 = egui::Color32::from_rgba_premultiplied(11, 13, 19, 238);
const MUTED: egui::Color32 = egui::Color32::from_rgb(140, 148, 165);
const ERROR: egui::Color32 = egui::Color32::from_rgb(240, 110, 100);
const OK: egui::Color32 = egui::Color32::from_rgb(120, 210, 140);

/// Where the graph renders and where the panel sits, within the space between the
/// side panels.
pub(crate) fn split(
    mode: PanelMode,
    area: egui::Rect,
    rows: usize,
) -> (egui::Rect, Option<egui::Rect>) {
    match mode {
        PanelMode::Hidden => (area, None),
        PanelMode::Docked => {
            let size = egui::vec2(
                (area.width() * 0.52)
                    .clamp(380.0, 960.0)
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
}

/// How many rows the active journey takes.
pub(crate) fn row_count(explorer: &Explorer) -> usize {
    rows(explorer).len()
}

/// Rows from top to bottom: each distinct layout height, highest first.
fn rows(explorer: &Explorer) -> Vec<f32> {
    let mut heights: Vec<f32> = explorer
        .track()
        .scene()
        .nodes
        .iter()
        .map(|n| n.position[1])
        .collect();
    heights.sort_by(|a, b| b.total_cmp(a));
    heights.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    heights
}

/// Draws the panel and handles its input. Returns the cursor bar's rect, for a
/// tether to its node in the graph.
pub(crate) fn draw(
    ctx: &egui::Context,
    rect: egui::Rect,
    explorer: &mut Explorer,
    panel: &mut JourneyPanel,
    diff: Option<&DiffView>,
) -> Option<egui::Rect> {
    let mut cursor_bar = None;
    egui::Area::new("journey_panel".into())
        .fixed_pos(rect.min)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_clip_rect(rect);
            ui.allocate_rect(rect, egui::Sense::hover());
            let rounding = if panel.mode == PanelMode::Full {
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
            let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEADER));
            header_bar(ui, header.shrink2(egui::vec2(10.0, 2.0)), explorer, panel);
            let content =
                egui::Rect::from_min_max(egui::pos2(rect.min.x, rect.min.y + HEADER), rect.max)
                    .shrink(PAD);
            cursor_bar = canvas(ui, content, explorer, panel, diff);
        });
    cursor_bar
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
            let active = explorer.active();
            ui.label(
                egui::RichText::new(format!("■ {}", track_title(explorer, active)))
                    .color(egui_color(accent(active)))
                    .strong(),
            );
            if let Some(scenario) = explorer
                .scenario()
                .and_then(|i| explorer.scenarios().get(i))
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
            });
        },
    );
}

#[expect(clippy::too_many_lines, reason = "one pass over the bars")]
fn canvas(
    ui: &mut egui::Ui,
    content: egui::Rect,
    explorer: &mut Explorer,
    panel: &mut JourneyPanel,
    diff: Option<&DiffView>,
) -> Option<egui::Rect> {
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
        return None;
    }
    let heights = rows(explorer);
    let row_of = |y: f32| {
        heights
            .iter()
            .position(|h| (h - y).abs() < 1e-3)
            .unwrap_or_default() as f32
    };
    let start = scene.bounds.0[0];
    let end = scene
        .nodes
        .iter()
        .map(|n| n.position[0] + n.length)
        .fold(start + 1.0, f32::max);
    let fit = content.width() / (end - start);
    let scale = panel.zoom.unwrap_or(fit);
    if panel.zoom.is_none() {
        panel.left = start;
    }
    let pitch = ROW + ROW_GAP;
    let visible_rows = (content.height() / pitch).floor().max(1.0);
    let max_top = (heights.len() as f32 - visible_rows).max(0.0);

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
    let cursor = track.selected().map(|span| SpanRef {
        track: explorer.active(),
        span,
    });
    let followed = cursor.map(|c| (explorer.revision(), c));
    if followed != panel.followed {
        panel.followed = followed;
        if let Some(node) = cursor.and_then(|c| scene.nodes.get(c.span.index())) {
            let width = content.width() / scale;
            let x = node.position[0];
            if x < panel.left || x > panel.left + width * 0.9 {
                panel.left = x - width * 0.15;
            }
            let row = row_of(node.position[1]);
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
    let tick = explorer.layout.tick_spacing;
    let cursor_enter = cursor
        .and_then(|c| track.span(c.span))
        .map_or(0, |s| s.enter);
    let graph_focus = explorer.graph_focus();
    let reach = explorer.reach();
    let tile_of = |span: SpanId| track.unit_id(span).and_then(|u| map.tile_of(u));

    let bars: Vec<Bar> = scene
        .nodes
        .iter()
        .map(|node| {
            let x = x_of(node.position[0]);
            let width = ((node.length - BAR_GAP) * scale).max(3.0);
            let y = y_of(row_of(node.position[1]));
            Bar {
                span: node.span,
                rect: egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, ROW)),
            }
        })
        .collect();
    let rect_of = |span: SpanId| bars.get(span.index()).map(|b| b.rect);

    // Caller to callee: a short drop from the caller's row.
    for edge in &scene.edges {
        let (Some(from), Some(to), Some(span)) = (
            rect_of(edge.caller),
            rect_of(edge.callee),
            track.span(edge.callee),
        ) else {
            continue;
        };
        let x = to.min.x + 2.0;
        let points = [egui::pos2(x, from.max.y), egui::pos2(x, to.min.y)];
        let stroke = egui::Stroke::new(1.0, egui::Color32::from_white_alpha(70));
        if is_conditional(span) {
            painter.extend(egui::Shape::dashed_line(&points, stroke, 3.0, 3.0));
        } else {
            painter.line_segment(points, stroke);
        }
    }

    let mouse = response.hover_pos();
    let mut hovered = None;
    for bar in &bars {
        let Some(span) = track.span(bar.span) else {
            continue;
        };
        if !bar.rect.intersects(content) {
            continue;
        }
        if mouse.is_some_and(|p| bar.rect.contains(p)) {
            hovered = Some(bar.span);
        }
        let base = egui_color(lane_color(track.color(bar.span) % PALETTE.len()));
        let faint = Confidence::of(span) == Confidence::Guessed || is_handoff(span);
        let future = span.enter > cursor_enter;
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

        // The focus's direct callers and callees, underlined in their colors.
        let tile = tile_of(bar.span);
        let related = tile
            .zip(reach)
            .and_then(|(tile, reach)| relation(reach, tile));
        // Under a diff, what changed instead, outlined too.
        let change = diff.zip(tile).and_then(|(diff, tile)| diff.change(tile));
        let underline = match (change, related) {
            (Some(change), _) => Some(change_color(change)),
            _ if diff.is_some() => None,
            (None, Some(Relation::Upstream(1))) => Some(CALLER),
            (None, Some(Relation::Downstream(1))) => Some(CALLEE),
            _ => None,
        };
        if let Some(change) = change {
            painter.rect_stroke(
                bar.rect,
                3.0,
                egui::Stroke::new(1.5, egui_color(change_color(change))),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(color) = underline {
            let line = egui::Rect::from_min_max(
                egui::pos2(bar.rect.min.x, bar.rect.max.y - 3.0),
                bar.rect.max,
            );
            painter.rect_filled(line, 1.0, egui_color(color));
        }

        for exit in &span.exits {
            let x = content.min.x + (exit.tick as f32 * tick - panel.left) * scale;
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

        if bar.rect.width() > 26.0 {
            let mut label = explorer.label_in(track, bar.span);
            if let Some(condition) = span.conditions.last() {
                label = format!("{label}  if {}", condition.arm);
            }
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a small, clamped character count"
            )]
            let room = ((bar.rect.width() - 10.0) / 6.8).clamp(1.0, 300.0) as usize;
            painter.with_clip_rect(bar.rect.intersect(content)).text(
                bar.rect.left_center() + egui::vec2(5.0, 0.0),
                egui::Align2::LEFT_CENTER,
                short(&label, room),
                egui::FontId::proportional(12.5),
                text,
            );
        }
        if tile.is_some() && tile == graph_focus && Some(bar.span) != cursor.map(|c| c.span) {
            painter.rect_stroke(
                bar.rect.expand(1.5),
                3.0,
                egui::Stroke::new(1.5, egui::Color32::WHITE),
                egui::StrokeKind::Outside,
            );
        }
    }

    // Red dots where other outcomes split off; click one to follow it.
    let mut divergence_hit = None;
    for divergence in track.divergences() {
        let Some(rect) = rect_of(divergence.span) else {
            continue;
        };
        let dot = egui::pos2(rect.max.x - 6.0, rect.min.y - 1.0);
        painter.circle_filled(dot, 4.0, ERROR);
        if mouse.is_some_and(|p| p.distance(dot) < 7.0) {
            divergence_hit = Some((divergence.scenario, divergence.label.clone()));
        }
    }

    let mut cursor_rect = None;
    if let Some(rect) = cursor.and_then(|c| rect_of(c.span)) {
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
        let mut tip = explorer.label_in(track, span);
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

    // Hovering a bar lights its node in the graph.
    let hovered_ref = hovered.map(|span| SpanRef {
        track: explorer.active(),
        span,
    });
    let hovered_tile = hovered.and_then(tile_of);
    if explorer.hovered != hovered_ref {
        explorer.hovered = hovered_ref;
    }
    if hovered_tile.is_some() {
        if explorer.hovered_tile != hovered_tile {
            explorer.hovered_tile = hovered_tile;
        }
        panel.lit_node = true;
    } else if panel.lit_node {
        explorer.hovered_tile = None;
        panel.lit_node = false;
    }

    if response.clicked() {
        let shift = ui.input(|i| i.modifiers.shift);
        if let Some((scenario, _)) = divergence_hit {
            explorer.set_scenario(Some(scenario));
        } else if let Some(at) = hovered_ref {
            explorer.select_in(at);
            if shift {
                explorer.pull_out_selected();
            }
        }
    }
    cursor_rect
}
