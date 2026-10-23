//! The active journey as a flame graph over the graph: time left to right, each call
//! on the row below its caller. Docked in a corner while the graph has the screen, or
//! filling the screen below a strip of graph. Each bar is painted the color of its
//! node, and the cursor, hover and focus are shared with the graph, so a bar and its
//! node light up together.

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::{Change, Span, SpanId};
use codeviz_ir::ExitKind;

use crate::{
    diff_view::{DiffView, change_color, sign},
    disclosure::Disclose,
    explorer::{Confidence, Explorer, SpanRef, Track, is_conditional, is_handoff},
    graph_view::node_color,
    scene::{PALETTE, lane_color},
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

/// The most of the screen's height a full-size journey takes from the graph.
const FULL_SHARE: f32 = 0.62;
/// The most of the screen's width and height a docked journey covers.
const DOCKED_WIDTH: f32 = 0.6;
const DOCKED_HEIGHT: f32 = 0.45;
const MARGIN: f32 = 12.0;
const HEADER: f32 = 28.0;
const PAD: f32 = 8.0;
const ROW: f32 = 22.0;
const ROW_GAP: f32 = 5.0;
/// Share of a slot left between consecutive bars on a row.
const BAR_GAP: f32 = 0.06;
/// Pixels a slot takes at the least, and at the most to name its call.
const MIN_SLOT: f32 = 40.0;
const MAX_SLOT: f32 = 240.0;
/// Roughly how wide a character of a bar's name is, what a bar leaves either side of
/// it, and what a badge of folded calls takes.
const CHAR: f32 = 6.8;
const LABEL_PAD: f32 = 10.0;
const BADGE: f32 = 38.0;

const BACKGROUND: egui::Color32 = egui::Color32::from_rgba_premultiplied(11, 13, 19, 238);
const MUTED: egui::Color32 = egui::Color32::from_rgb(140, 148, 165);
const ERROR: egui::Color32 = egui::Color32::from_rgb(240, 110, 100);
const OK: egui::Color32 = egui::Color32::from_rgb(120, 210, 140);

/// Where the graph renders and where the panel sits, within the space between the
/// side panels: each as big as the journey needs, `rows` deep and `width` wide, up to
/// a share of the screen. Docked, it floats in the bottom-right corner, leaving
/// `beside` clear to its left when it can (for the legend); full, it takes the bottom
/// across the whole width and the graph renders above it.
pub(crate) fn split(
    mode: PanelMode,
    area: egui::Rect,
    (rows, width): (usize, f32),
    beside: f32,
) -> (egui::Rect, Option<egui::Rect>) {
    let height = HEADER + PAD * 2.0 + rows.max(2) as f32 * (ROW + ROW_GAP);
    match mode {
        PanelMode::Hidden => (area, None),
        PanelMode::Docked => {
            let size = egui::vec2(
                (width + PAD * 2.0)
                    .min(area.width() * DOCKED_WIDTH)
                    .min(area.width() - beside - MARGIN * 2.0)
                    .max(380.0)
                    .min(area.width() - MARGIN * 2.0),
                height
                    .min((area.height() * DOCKED_HEIGHT).max(200.0))
                    .min(area.height() - MARGIN * 2.0),
            );
            let min = area.max - size - egui::vec2(MARGIN, MARGIN);
            (area, Some(egui::Rect::from_min_size(min, size)))
        }
        PanelMode::Full => {
            let height = height.clamp(160.0, area.height() * FULL_SHARE);
            let top = area.max.y - height;
            let graph = egui::Rect::from_min_max(area.min, egui::pos2(area.max.x, top));
            let panel = egui::Rect::from_min_max(egui::pos2(area.min.x, top), area.max);
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

/// How many rows the active journey takes, and how wide it wants to be: wide enough
/// that each bar names its call, as far as a slot may grow.
pub(crate) fn extent(explorer: &Explorer) -> (usize, f32) {
    let disclosed = explorer.disclosure();
    let track = explorer.track();
    let Some(journey) = explorer.journey() else {
        return (disclosed.rows(), 0.0);
    };
    let (placed, slots) = disclosed.layout(journey.root());
    let slot = placed
        .iter()
        .filter_map(|(&span, &(_, width, _))| {
            let span_data = track.span(span)?;
            let number = disclosed.number(span).unwrap_or_default();
            let label = bar_label(explorer, track, span_data, span, number, None);
            let badge = if disclosed.folded(span) > 0 {
                BADGE
            } else {
                0.0
            };
            let wanted = label.chars().count() as f32 * CHAR + LABEL_PAD + badge;
            Some(wanted / (width - BAR_GAP).max(0.5))
        })
        .fold(MIN_SLOT, f32::max)
        .min(MAX_SLOT);
    (disclosed.rows(), slot * slots)
}

/// What a bar says: its number, what changed, its name and the condition it runs on.
fn bar_label(
    explorer: &Explorer,
    track: &Track,
    span: &Span,
    id: SpanId,
    number: &str,
    change: Option<Change>,
) -> String {
    let sign = change.map_or_else(String::new, |c| format!("{} ", sign(c)));
    let mut label = format!("{sign}{}", explorer.label_in(track, id));
    if !number.is_empty() {
        label = format!("{number}  {label}");
    }
    if let Some(condition) = span.conditions.last() {
        label = format!("{label}  if {}", condition.arm);
    }
    label
}

/// Draws the panel and handles its input. Returns the bar under the pointer, to
/// tether to its node in the graph.
pub(crate) fn draw(
    ctx: &egui::Context,
    rect: egui::Rect,
    explorer: &mut Explorer,
    panel: &mut JourneyPanel,
    diff: Option<&DiffView>,
) -> Option<egui::Rect> {
    let mut hovered = None;
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
            hovered = canvas(ui, content, explorer, panel, diff);
        });
    hovered
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
    let active = explorer.active();
    ui.add(
        egui::Label::new(
            egui::RichText::new(track_title(explorer, active))
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
    let cursor = track.selected().map(|span| SpanRef {
        track: explorer.active(),
        span,
    });
    let followed = cursor.map(|c| (explorer.revision(), c));
    if followed != panel.followed {
        panel.followed = followed;
        if let Some(&(x, _, row)) = cursor.and_then(|c| placed.get(&c.span)) {
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
    let cursor_enter = cursor
        .and_then(|c| track.span(c.span))
        .map_or(0, |s| s.enter);
    let graph_focus = explorer.graph_focus();
    let stepping = cursor.is_some_and(|c| track.journey().is_some_and(|j| j.root() != c.span));
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
        let (fill, text) = bar_colors(base, faint, future);
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
            let badge = fold_badge(&painter, bar.rect, folded);
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
        let room = ((bar.rect.width() - if badge.is_some() { BADGE } else { LABEL_PAD }) / CHAR)
            .clamp(0.0, 300.0) as usize;
        let painter = painter.with_clip_rect(bar.rect.intersect(content));
        if room >= number.len() + 6 {
            let label = bar_label(explorer, track, span, bar.span, number, change);
            bar_name(&painter, bar.rect, &short(&label, room), text);
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
        if tile.is_some() && tile == graph_focus && Some(bar.span) != cursor.map(|c| c.span) {
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
        let mark = fail_end(&painter, rect);
        if mouse.is_some_and(|p| mark.expand(3.0).contains(p)) {
            divergence_hit = Some((divergence.scenario, divergence.label.clone()));
        }
    }

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

    let hovered_rect = hovered
        .and_then(rect_of)
        .filter(|rect| content.intersects(*rect));
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
        if let Some(span) = fold_hit {
            explorer.toggle_expanded(span);
        } else if let Some((scenario, _)) = divergence_hit {
            explorer.set_scenario(Some(scenario));
        } else if let Some(at) = hovered_ref {
            explorer.select_in(at);
            if shift {
                explorer.pull_out_selected();
            }
        }
    }
    hovered_rect
}

/// A bar's fill and the color of its name: faint for a guess or a handoff, dimmed
/// when it runs after the cursor.
fn bar_colors(base: egui::Color32, faint: bool, future: bool) -> (egui::Color32, egui::Color32) {
    if faint {
        (base.gamma_multiply(0.22), MUTED)
    } else if future {
        (
            base.gamma_multiply(0.38),
            egui::Color32::from_rgb(200, 205, 216),
        )
    } else {
        (base, egui::Color32::from_rgb(14, 16, 22))
    }
}

/// A bar's name, from its left end.
fn bar_name(painter: &egui::Painter, bar: egui::Rect, name: &str, color: egui::Color32) {
    painter.text(
        bar.left_center() + egui::vec2(5.0, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(12.5),
        color,
    );
}

/// The badge at a bar's right end counting the calls folded under it.
fn fold_badge(painter: &egui::Painter, bar: egui::Rect, folded: usize) -> egui::Rect {
    let badge = egui::Rect::from_min_max(
        egui::pos2(bar.max.x - 26.0, bar.min.y + 3.0),
        egui::pos2(bar.max.x - 3.0, bar.max.y - 3.0),
    );
    painter.rect_filled(badge, 3.0, egui::Color32::from_black_alpha(120));
    painter.text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        format!("+{folded}"),
        egui::FontId::proportional(10.5),
        egui::Color32::from_rgb(220, 224, 234),
    );
    badge
}

/// The red end of a bar where another outcome splits off, inside the bar.
fn fail_end(painter: &egui::Painter, bar: egui::Rect) -> egui::Rect {
    let mark = egui::Rect::from_min_max(egui::pos2(bar.max.x - 5.0, bar.min.y), bar.max);
    painter.rect_filled(mark, 2.0, ERROR);
    mark
}

/// What the marks on a bar mean, each drawn as the journey draws it: for the key.
pub(crate) fn bar_key(ui: &mut egui::Ui) {
    let sample = |ui: &mut egui::Ui, draw: &dyn Fn(&egui::Painter, egui::Rect), meaning: &str| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(170.0, ROW), egui::Sense::hover());
            draw(ui.painter(), rect);
            ui.label(egui::RichText::new(meaning).color(MUTED));
        });
    };
    let color = |index: usize| egui_color(lane_color(index % PALETTE.len()));
    let plain = |painter: &egui::Painter, rect: egui::Rect, index: usize, name: &str| {
        let (fill, text) = bar_colors(color(index), false, false);
        painter.rect_filled(rect, 3.0, fill);
        bar_name(painter, rect, name, text);
    };
    sample(
        ui,
        &|p, r| {
            plain(p, r, 0, "2  check_quantity");
            fail_end(p, r);
        },
        "red end: can fail here (click it to follow that outcome)",
    );
    sample(
        ui,
        &|p, r| {
            plain(p, r, 1, "3  quote");
            fold_badge(p, r, 2);
        },
        "+N: calls folded under it (click to open)",
    );
    sample(
        ui,
        &|p, r| {
            plain(p, r, 2, "3.2.1  discount  if bulk");
            dashed_outline(
                p,
                r.shrink(0.75),
                egui::Stroke::new(1.2, egui::Color32::from_white_alpha(190)),
            );
        },
        "dashed: only runs sometimes, on the condition named",
    );
    sample(
        ui,
        &|p, r| {
            let (fill, text) = bar_colors(color(3), true, false);
            p.rect_filled(r, 3.0, fill);
            bar_name(p, r, "→ on_message", text);
        },
        "faint: handed off to run later (→), or a guessed call",
    );
    sample(
        ui,
        &|p, r| {
            let (fill, text) = bar_colors(color(4), false, true);
            p.rect_filled(r, 3.0, fill);
            bar_name(p, r, "4  insert_order", text);
        },
        "dimmed: runs after the cursor",
    );
    sample(
        ui,
        &|p, r| {
            plain(p, r, 5, "1  find_user");
            p.rect_stroke(
                r.expand(1.5),
                3.0,
                egui::Stroke::new(2.0, egui::Color32::WHITE),
                egui::StrokeKind::Outside,
            );
        },
        "white outline: the cursor, or the node focused in the graph",
    );
    sample(
        ui,
        &|p, r| plain(p, r, 6, "walk ↻   visit …"),
        "↻ calls itself again; … goes deeper than the journey follows",
    );
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

    #[test]
    fn the_panel_is_as_big_as_the_journey_up_to_a_share_of_the_screen() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
        let (graph, docked) = split(PanelMode::Docked, screen, (3, 500.0), 0.0);
        let docked = docked.unwrap();
        assert_eq!(graph, screen, "docked, it floats over the graph");
        assert_eq!(docked.max, screen.max - egui::vec2(MARGIN, MARGIN));
        assert!((docked.width() - (500.0 + PAD * 2.0)).abs() < 1e-3);
        assert!((docked.height() - (HEADER + PAD * 2.0 + 3.0 * (ROW + ROW_GAP))).abs() < 1e-3);
        let (_, wide) = split(PanelMode::Docked, screen, (40, 5000.0), 0.0);
        let wide = wide.unwrap();
        assert!(wide.width() <= screen.width() * DOCKED_WIDTH + 1e-3);
        assert!(wide.height() <= screen.height() * DOCKED_HEIGHT + 1e-3);
        let (_, narrower) = split(PanelMode::Docked, screen, (3, 5000.0), 900.0);
        assert!(
            narrower.unwrap().min.x >= 900.0,
            "it leaves the legend its corner"
        );
        let (_, least) = split(PanelMode::Docked, screen, (3, 5000.0), 1400.0);
        assert!(
            (least.unwrap().width() - 380.0).abs() < 1e-3,
            "but not below the least"
        );

        // Full, it takes the bottom as deep as its rows, and the graph the rest.
        let (graph, full) = split(PanelMode::Full, screen, (4, 500.0), 0.0);
        let full = full.unwrap();
        assert_eq!(full.width(), screen.width());
        assert_eq!(graph.max.y, full.min.y);
        assert!(
            full.height() < 200.0,
            "four rows leave the graph most of it"
        );
        let (graph, _) = split(PanelMode::Full, screen, (60, 500.0), 0.0);
        assert!(graph.height() >= screen.height() * (1.0 - FULL_SHARE) - 1e-3);
    }

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
