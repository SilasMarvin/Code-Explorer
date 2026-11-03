//! The journey as a flame graph docked along the foot of the window: calls left to
//! right in the order they run, each on the row below its caller, and above the
//! start's bar a row of the functions that call it. Each bar is painted the color of
//! its node, and the cursor and what is hovered are shared with the graph, so a bar
//! and its node light up together.

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::SpanId;
use codeviz_ir::ExitKind;

use crate::{
    colors::{
        BADGE_FILL, BADGE_INK, CALLER, CURSOR, CURSOR_LINE, ERROR, FAINT_LINE, GUESSED, HOVER,
        MUTED, OK, ON_FILL, PANEL_EDGE, SELECTED, SOMETIMES, SURFACE, TEXT, THUMB, TRACK,
        egui_color,
    },
    disclosure::Disclose,
    explorer::{Explorer, is_conditional},
    graph_view::{look_color, role_look},
    text,
    ui::{journey_title, short},
};

/// How the panel is zoomed and scrolled.
#[derive(Resource, Debug, Default)]
pub(crate) struct JourneyPanel {
    /// How much wider than laid out the bars are drawn; `None` fits the whole journey
    /// across, but never so narrow that a bar loses its number and the start of its
    /// name.
    zoom: Option<f32>,
    /// Where along the journey, laid out, the left edge is.
    left: f32,
    /// The first row shown.
    top_row: f32,
    /// The cursor the view last scrolled to, in the journey and layout it was in.
    followed: Option<(u64, SpanId, u32)>,
    /// Whether the panel set the function under the pointer, so it can clear it again.
    lit_node: bool,
}

const MARGIN: f32 = 12.0;
const HEADER: f32 = 28.0;
const PAD: f32 = 8.0;
const ROW: f32 = 22.0;
const ROW_GAP: f32 = 5.0;
/// Points left between consecutive bars on a row.
const BAR_GAP: f32 = 5.0;
/// How much of its name a bar keeps at the least, cut short: what a journey too wide
/// for the panel squeezes every bar down to before it scrolls, following the cursor.
/// And at the most, how wide a bar asks to be to say all it says.
const LEAST_LETTERS: usize = 4;
const FULLEST: f32 = 260.0;
/// How far in the panel zooms by hand, against the bars as laid out.
const MOST_ZOOM: f32 = 4.0;
/// The narrowest a bar in the row of the start's callers is drawn.
const MIN_SLOT: f32 = 96.0;
/// The badge on a bar that opens the calls folded under it.
const BADGE: f32 = 30.0;
/// How big a bar's name is drawn, and how small it shrinks to fit.
const BAR_TEXT: f32 = text::BODY;
const BAR_TEXT_SMALL: f32 = text::SMALL;

/// The rows of bars the panel makes room for: the journey's, a row a level with the
/// start's, as many as [`split`] gives room, past which they scroll; and above them
/// the start's callers.
pub(crate) fn rows(explorer: &Explorer) -> usize {
    explorer.disclosure().rows().max(2) + usize::from(explorer.start_called())
}

/// The graph's free area within `area`, and the strip along its foot the panel takes,
/// with `rows` of bars to show ([`rows`]): the panel as wide as the window less a
/// margin round it, and as tall as the rows.
pub(crate) fn split(area: egui::Rect, rows: usize) -> (egui::Rect, egui::Rect) {
    #[expect(clippy::cast_precision_loss, reason = "a handful of rows")]
    let height = (HEADER + PAD * 2.0 + rows as f32 * (ROW + ROW_GAP))
        .min((area.height() * 0.4).max(140.0))
        .min(area.height() - MARGIN * 2.0);
    let top = area.max.y - height - MARGIN * 2.0;
    (
        egui::Rect::from_min_max(area.min, egui::pos2(area.max.x, top)),
        egui::Rect::from_min_max(egui::pos2(area.min.x, top), area.max),
    )
}

impl JourneyPanel {
    /// Back to the journey across at its usual scale, the cursor in view.
    fn fit(&mut self) {
        self.zoom = None;
        self.top_row = 0.0;
        self.followed = None;
    }
}

/// One bar on screen.
struct Bar {
    span: SpanId,
    rect: egui::Rect,
    /// Whether it is disclosed; folded bars keep their place but are not drawn.
    shown: bool,
}

/// The bar a tether runs from to its node: the one under the pointer, the cursor's
/// as any other.
#[derive(Default)]
pub(crate) struct Tethers {
    pub(crate) hovered: Option<egui::Rect>,
}

/// Draws the panel in `strip` (see [`split`]), the hint at the keys in its header,
/// and handles its input. Returns the bar to tether to its node in the graph.
pub(crate) fn draw(
    ctx: &egui::Context,
    strip: egui::Rect,
    hint: &str,
    explorer: &mut Explorer,
    panel: &mut JourneyPanel,
) -> Tethers {
    let mut tethers = Tethers::default();
    let rect = strip.shrink(MARGIN);
    egui::Area::new("journey_panel".into())
        .fixed_pos(strip.min)
        .constrain(false)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            // The whole strip is the panel's, so nothing on the graph shows as if
            // clickable under its margin.
            ui.allocate_rect(strip, egui::Sense::hover());
            ui.set_clip_rect(rect);
            // Opaque: the graph is drawn on under the panel.
            ui.painter().rect(
                rect,
                8.0,
                SURFACE,
                egui::Stroke::new(1.0, PANEL_EDGE),
                egui::StrokeKind::Inside,
            );
            let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEADER));
            header_bar(
                ui,
                header.shrink2(egui::vec2(10.0, 2.0)),
                hint,
                explorer,
                panel,
            );
            let content =
                egui::Rect::from_min_max(egui::pos2(rect.min.x, rect.min.y + HEADER), rect.max)
                    .shrink(PAD);
            tethers = canvas(ui, content, explorer, panel);
        });
    tethers
}

/// How much of the journey shows: levels of calls below the root; − and + do as `,`
/// and `.` do.
fn disclosure(ui: &mut egui::Ui, explorer: &mut Explorer) {
    let Disclose(depth) = explorer.disclose();
    if ui
        .small_button("+")
        .on_hover_text("One more level of calls (.)")
        .clicked()
    {
        explorer.deeper();
    }
    ui.label(egui::RichText::new(format!("{depth}")).small());
    if ui
        .small_button("−")
        .on_hover_text("One level of calls fewer (,)")
        .clicked()
    {
        explorer.shallower();
    }
    ui.label(egui::RichText::new("depth").small().color(MUTED));
}

fn header_bar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    hint: &str,
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
                    .small_button("Fit")
                    .on_hover_text(
                        "The whole journey, here across as far as every bar keeps its \
                         number (double-click), and in the graph",
                    )
                    .clicked()
                {
                    panel.fit();
                    explorer.ask_to_frame();
                }
                disclosure(ui, explorer);
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    title(ui, explorer);
                    ui.add_space(16.0);
                    ui.add(
                        egui::Label::new(egui::RichText::new(hint).size(text::SMALL).color(MUTED))
                            .truncate(),
                    );
                });
            });
        },
    );
}

/// The journey's name.
fn title(ui: &mut egui::Ui, explorer: &Explorer) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(journey_title(explorer))
                .color(TEXT)
                .strong(),
        )
        .truncate(),
    );
}

#[expect(clippy::too_many_lines, reason = "one pass over the bars")]
fn canvas(
    ui: &mut egui::Ui,
    whole: egui::Rect,
    explorer: &mut Explorer,
    panel: &mut JourneyPanel,
) -> Tethers {
    let response = ui.interact(
        whole,
        ui.id().with("journey_canvas"),
        egui::Sense::click_and_drag(),
    );
    // The start's callers in a row along the top; the journey's bars below.
    let callers = explorer.start_callers();
    let (above, content) = if callers.is_empty() {
        (None, whole)
    } else {
        let row = egui::Rect::from_min_size(whole.min, egui::vec2(whole.width(), ROW));
        let below =
            egui::Rect::from_min_max(egui::pos2(whole.min.x, row.max.y + ROW_GAP), whole.max);
        (Some(row), below)
    };
    let painter = ui.painter_at(content);
    let track = explorer.track();
    let Some(journey) = track.journey() else {
        painter.text(
            content.center(),
            egui::Align2::CENTER_CENTER,
            "No journey runs from this function.",
            egui::FontId::proportional(text::BODY),
            MUTED,
        );
        return Tethers::default();
    };
    // Only what is disclosed is laid out, each bar as wide as what it says needs, the
    // whole squeezed to the panel's width where it can be.
    let disclosed = explorer.disclosure();
    let (placed, slots) = disclosed.layout(journey.root(), content.width(), &|span| {
        needs(&painter, &says(explorer, span), disclosed.folded(span))
    });
    let slots = slots.max(1.0);
    let fit = content.width() / slots;
    let usual = fit.max(1.0);
    // Another journey, or one put back, opens at the usual scale.
    if panel
        .followed
        .is_some_and(|(revision, ..)| revision != explorer.revision())
    {
        panel.zoom = None;
    }
    let scale = panel.zoom.unwrap_or(usual);
    let pitch = ROW + ROW_GAP;
    let visible_rows = (content.height() / pitch).floor().max(1.0);
    let max_top = (disclosed.rows() as f32 - visible_rows).max(0.0);

    // Zoom about the pointer: scroll or pinch, out as far as the whole journey
    // across. Drag pans; double-click goes back to the usual scale.
    if response.hovered() {
        let (scroll, spread) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        let factor = spread * (scroll.y * 0.004).exp();
        if (factor - 1.0).abs() > 1e-4
            && let Some(pointer) = response.hover_pos()
        {
            let at = panel.left + (pointer.x - content.min.x) / scale;
            let zoomed = (scale * factor).clamp(fit.min(usual), MOST_ZOOM);
            panel.zoom = Some(zoomed);
            panel.left = at - (pointer.x - content.min.x) / zoomed;
        }
        if scroll.x.abs() > 0.0 {
            panel.left -= scroll.x / scale;
        }
    }
    if response.dragged() {
        let delta = response.drag_delta();
        panel.left -= delta.x / scale;
        panel.top_row -= delta.y / pitch;
    }
    if response.double_clicked() {
        panel.fit();
    }

    // Keep the cursor in view when it moves, or the journey under it changes.
    let scale = panel.zoom.unwrap_or(usual);
    let across = content.width() / scale;
    let cursor = explorer.cursor_span();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a count of slots"
    )]
    let followed = cursor.map(|c| (explorer.revision(), c, slots as u32));
    if followed != panel.followed {
        panel.followed = followed;
        if let Some(&(x, width, row)) = cursor.and_then(|c| placed.get(&c)) {
            // As much of the cursor's bar as the panel can show, its left end first.
            let width = width.min(across);
            if x < panel.left || x + width > panel.left + across {
                panel.left = x - (across - width).min(across * 0.15);
            }
            let row = row as f32;
            if row < panel.top_row || row > panel.top_row + visible_rows - 1.0 {
                panel.top_row = row - (visible_rows / 3.0).floor();
            }
        }
    }
    panel.left = panel.left.clamp(0.0, (slots - across).max(0.0));
    panel.top_row = panel.top_row.clamp(0.0, max_top);
    let x_of = |x: f32| content.min.x + (x - panel.left) * scale;
    let y_of = |row: f32| content.min.y + (row - panel.top_row) * pitch;

    let map = explorer.map();
    let current = explorer.current();
    let tile_of = |span: SpanId| track.tile(span, map);

    // A bar per span, in the journey's order, so a span's index finds its bar.
    let bars: Vec<Bar> = journey
        .spans()
        .map(|(span, _)| match placed.get(&span) {
            Some(&(left, slots, row)) => Bar {
                span,
                rect: egui::Rect::from_min_size(
                    egui::pos2(x_of(left), y_of(row as f32)),
                    egui::vec2((slots * scale - BAR_GAP).max(3.0), ROW),
                ),
                shown: true,
            },
            None => Bar {
                span,
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
    let graph_hover = explorer.hovered().filter(|_| !panel.lit_node);
    // Caller to callee: a short drop from the caller's row.
    for (callee, span) in journey.spans() {
        let Some(caller) = span.parent.filter(|_| disclosed.shows(callee)) else {
            continue;
        };
        let (Some(from), Some(to)) = (rect_of(caller), rect_of(callee)) else {
            continue;
        };
        let x = to.min.x + 2.0;
        let points = [egui::pos2(x, from.max.y), egui::pos2(x, to.min.y)];
        // Dashes along an unbounded line never end.
        if !points.iter().all(|p| p.is_finite()) {
            continue;
        }
        let stroke = egui::Stroke::new(1.0, FAINT_LINE);
        if is_conditional(span) {
            painter.extend(egui::Shape::dashed_line(&points, stroke, 3.0, 3.0));
        } else {
            painter.line_segment(points, stroke);
        }
    }

    let mouse = response.hover_pos();
    let mut hovered = None;
    let mut fold_hit = None;
    let mut chips = Vec::new();
    let mut said = Vec::new();
    for bar in &bars {
        let Some(span) = track.span(bar.span) else {
            continue;
        };
        if !bar.shown || !bar.rect.intersects(content) {
            continue;
        }
        if mouse.is_some_and(|p| bar.rect.intersect(content).contains(p)) {
            hovered = Some(bar.span);
        }
        let tile = tile_of(bar.span);
        let fill = bar_fill(explorer, bar.span);
        let text = if guessed(explorer, bar.span) {
            MUTED
        } else {
            ON_FILL
        };
        painter.rect_filled(bar.rect, 3.0, fill);
        // A call that calls the start back, outlined violet as its dot is ringed.
        if tile.is_some_and(|t| explorer.nucleus().calls_back(t)) {
            outline(&painter, bar.rect, egui_color(CALLER), 1.5);
        }

        // A call that only sometimes runs is outlined in dashes.
        if is_conditional(span) {
            let stroke = egui::Stroke::new(1.2, SOMETIMES);
            dashed_outline(&painter, bar.rect.shrink(0.75), stroke);
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

        // The same number the graph puts on the node, then the name.
        let number = disclosed.bar_number(bar.span).unwrap_or_default();
        let number_width = text_width(&painter, number, BAR_TEXT_SMALL);
        // What is folded under the bar, as a badge that opens it, where there is room
        // for its number beside it: a number says which call it is, the badge only
        // how much more there is. A triangle, as a fold is drawn anywhere, and the
        // count.
        let folded = disclosed.folded(bar.span);
        let badge = (folded > 0 && bar.rect.width() > BADGE + number_width + 12.0).then(|| {
            let badge = egui::Rect::from_min_max(
                egui::pos2(bar.rect.max.x - BADGE, bar.rect.min.y + 3.0),
                egui::pos2(bar.rect.max.x - 3.0, bar.rect.max.y - 3.0),
            );
            let ink = BADGE_INK;
            painter.rect_filled(badge, 3.0, BADGE_FILL);
            let tip = egui::pos2(badge.min.x + 8.0, badge.center().y + 2.5);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    tip + egui::vec2(-3.5, -5.0),
                    tip + egui::vec2(3.5, -5.0),
                    tip,
                ],
                ink,
                egui::Stroke::NONE,
            ));
            painter.text(
                egui::pos2(badge.min.x + 13.5, badge.center().y),
                egui::Align2::LEFT_CENTER,
                folded.to_string(),
                egui::FontId::proportional(text::SMALL),
                ink,
            );
            if mouse.is_some_and(|p| badge.contains(p)) {
                fold_hit = Some(bar.span);
            }
            badge
        });
        chips.extend(badge.map(|badge| (bar.span, badge)));
        // A node the pointer rests on in the graph outlines its bars, as hovering a
        // bar does.
        if graph_hover.is_some() && tile == graph_hover {
            outline(&painter, bar.rect, egui_color(HOVER), 1.0);
        }
        let free = badge.map_or(bar.rect, |badge| {
            egui::Rect::from_min_max(bar.rect.min, egui::pos2(badge.min.x - 2.0, bar.rect.max.y))
        });
        let painter = painter.with_clip_rect(free.intersect(content));
        let named = bar_text(&painter, &says(explorer, bar.span), free.width() - 10.0);
        if let Some((named, size)) = &named {
            painter.text(
                free.left_center() + egui::vec2(5.0, 0.0),
                egui::Align2::LEFT_CENTER,
                named,
                egui::FontId::proportional(*size),
                text,
            );
        }
        said.push((bar.span, named.map(|(named, _)| named).unwrap_or_default()));
        if tile.is_some() && tile == current && Some(bar.span) != cursor {
            outline(&painter, bar.rect, egui_color(SELECTED), 1.5);
        }
    }

    let (caller_hovered, caller_bars) = above.map_or_else(
        || (None, Vec::new()),
        |row| callers_row(ui, row, explorer, &callers, mouse, graph_hover),
    );

    if let Some(rect) = cursor.and_then(rect_of) {
        painter.line_segment(
            [
                egui::pos2(rect.min.x, content.min.y),
                egui::pos2(rect.min.x, content.max.y),
            ],
            egui::Stroke::new(1.0, CURSOR_LINE),
        );
        outline(&painter, rect, egui_color(SELECTED), 2.0);
    }
    // Wider than the panel, a track along its foot shows which part is in view.
    let along = (slots > across + 1e-3).then(|| {
        let foot = content.with_min_y(content.max.y - 3.0);
        scroll_track(
            &painter,
            foot,
            (panel.left / slots, (panel.left + across) / slots),
        );
        foot
    });
    // Deeper than the panel shows, a track down its right side shows which rows are
    // in view.
    let rows = disclosed.rows() as f32;
    let down = (rows > visible_rows).then(|| {
        let side = content.with_min_x(content.max.x - 3.0);
        let shown = (panel.top_row / rows, (panel.top_row + visible_rows) / rows);
        scroll_track(&painter, side, shown);
        side
    });

    if let Some((tile, _)) = caller_hovered {
        let start = explorer
            .start()
            .and_then(|s| explorer.tile_unit(s))
            .map(|unit| explorer.function_name(unit))
            .unwrap_or_default();
        let name = explorer
            .tile_unit(tile)
            .map(|unit| explorer.function_name(unit))
            .unwrap_or_default();
        response.clone().on_hover_text(format!(
            "{name} calls {start}\nclick: select it · Enter or Shift-click: start a journey there"
        ));
    }
    if let Some(span) = hovered {
        if let Some(rect) = rect_of(span) {
            outline(&painter, rect, egui_color(HOVER), 1.0);
        }
        let mut tip = explorer.label(span);
        let number = disclosed.bar_number(span).unwrap_or_default();
        if !number.is_empty() {
            tip = format!("{number}  {tip}");
        }
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

    // Where each bar is, for a scripted session to click.
    crate::drive::note_bars(
        ui.ctx(),
        content,
        bars.iter()
            .filter(|bar| bar.shown)
            .map(|bar| {
                let text = said.iter().find(|(s, _)| *s == bar.span);
                (
                    bar.span,
                    bar.rect,
                    text.map(|(_, t)| t.clone()).unwrap_or_default(),
                )
            })
            .collect(),
        chips,
        caller_bars,
        (down, along),
    );
    let hovered_rect = hovered
        .and_then(rect_of)
        .filter(|rect| content.intersects(*rect))
        .or(caller_hovered.map(|(_, rect)| rect));
    // Hovering a bar lights its node in the graph.
    let bar_tile = hovered.and_then(tile_of);
    let hovered_tile = bar_tile.or(caller_hovered.map(|(tile, _)| tile));
    if hovered_tile.is_some() {
        if explorer.under_pointer != hovered_tile {
            explorer.under_pointer = hovered_tile;
        }
        panel.lit_node = true;
    } else if panel.lit_node {
        explorer.under_pointer = None;
        panel.lit_node = false;
    }

    // Only a pointer's click: egui also clicks a widget it focuses on Enter or Space,
    // and those keys belong to the graph.
    if response.clicked_by(egui::PointerButton::Primary) {
        if let Some((tile, _)) = caller_hovered {
            if ui.input(|i| i.modifiers.shift) {
                explorer.open_at(tile);
            } else {
                explorer.pick_caller(tile);
            }
        } else if let Some(span) = fold_hit {
            explorer.toggle_expanded(span);
        } else if let Some(span) = hovered {
            // Shift opens its journey, as Shift-click does anywhere.
            match bar_tile {
                Some(tile) if ui.input(|i| i.modifiers.shift) => explorer.open_at(tile),
                _ => explorer.go_to(span),
            }
        }
    }
    Tethers {
        hovered: hovered_rect,
    }
}

/// A bar in the row of the start's callers: its function, where it is, and what it
/// says.
type CallerBar = (usize, egui::Rect, String);

/// The row above the start's bar: the functions that call the start, violet (faint
/// those folded into their module's count, as their dots are), the one set aside
/// outlined white, as many across as fit at the narrowest a bar is drawn, from the
/// first, or as far along as keeps the one set aside in view. Returns the caller under
/// the pointer, and where each caller's bar is.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a count of bars across"
)]
fn callers_row(
    ui: &egui::Ui,
    row: egui::Rect,
    explorer: &Explorer,
    callers: &[usize],
    mouse: Option<egui::Pos2>,
    graph_hover: Option<usize>,
) -> (Option<(usize, egui::Rect)>, Vec<CallerBar>) {
    let painter = ui.painter_at(row.expand(2.0));
    let across = callers
        .len()
        .min(((row.width() / MIN_SLOT) as usize).max(1));
    let first = explorer
        .aside_caller()
        .map_or(0, |at| (at + 1).saturating_sub(across))
        .min(callers.len() - across);
    let slot = row.width() / across as f32;
    let mut hovered = None;
    let mut bars = Vec::new();
    for (place, &tile) in callers.iter().skip(first).take(across).enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(row.min.x + place as f32 * slot, row.min.y),
            egui::vec2(slot - BAR_GAP, ROW),
        );
        let aside = explorer.aside() == Some(tile);
        painter.rect_filled(rect, 3.0, caller_fill(explorer, tile));
        if aside {
            outline(&painter, rect, egui_color(SELECTED), 2.0);
        } else if graph_hover == Some(tile) || mouse.is_some_and(|p| rect.contains(p)) {
            outline(&painter, rect, egui_color(HOVER), 1.0);
        }
        let name = explorer
            .tile_unit(tile)
            .map(|unit| explorer.function_name(unit))
            .unwrap_or_default();
        let room = rect.width() - 10.0;
        let said = [BAR_TEXT, BAR_TEXT_SMALL]
            .into_iter()
            .find(|&size| text_width(&painter, &name, size) <= room)
            .map(|size| (name.clone(), size))
            .or_else(|| {
                fit_text(&painter, &name, BAR_TEXT_SMALL, room).map(|t| (t, BAR_TEXT_SMALL))
            });
        if let Some((text, size)) = &said {
            painter.with_clip_rect(rect).text(
                rect.left_center() + egui::vec2(5.0, 0.0),
                egui::Align2::LEFT_CENTER,
                text,
                egui::FontId::proportional(*size),
                if explorer.nucleus().folded(tile) {
                    MUTED
                } else {
                    ON_FILL
                },
            );
        }
        let said = said.map(|(text, _)| text).unwrap_or_default();
        if mouse.is_some_and(|p| rect.contains(p)) {
            hovered = Some((tile, rect));
        }
        bars.push((tile, rect, said));
    }
    // More than fit: a track along its foot shows which part is in view.
    if callers.len() > across {
        let foot = row.with_min_y(row.max.y + 1.0).with_max_y(row.max.y + 2.0);
        let share = |at: usize| at as f32 / callers.len() as f32;
        scroll_track(&painter, foot, (share(first), share(first + across)));
    }
    (hovered, bars)
}

/// A journey bar's fill: its node's color; a call with no node of its own is grey,
/// as everything that is no module's is. Only the cursor's call is pink: another run
/// of the same function is painted as its node would be were the cursor elsewhere. A
/// call only guessed, or handed off to run later, is faint.
pub(crate) fn bar_fill(explorer: &Explorer, span: SpanId) -> egui::Color32 {
    let track = explorer.track();
    let base = match track.tile(span, explorer.map()) {
        _ if Some(span) == explorer.cursor_span() => egui_color(CURSOR),
        Some(tile) => egui_color(look_color(role_look(explorer, tile))),
        None => MUTED,
    };
    if guessed(explorer, span) {
        base.gamma_multiply(GUESSED)
    } else {
        base
    }
}

/// Whether a call is only guessed, or handed off to run later: its bar is faint.
pub(crate) fn guessed(explorer: &Explorer, span: SpanId) -> bool {
    explorer.span(span).is_some_and(crate::disclosure::guessed)
}

/// A bar in the row of the start's callers: violet, as what calls the start is, even
/// one the journey runs too; faint, as its dot is, one folded into its module's count.
pub(crate) fn caller_fill(explorer: &Explorer, tile: usize) -> egui::Color32 {
    let violet = egui_color(CALLER);
    if explorer.nucleus().folded(tile) {
        violet.gamma_multiply(GUESSED)
    } else {
        violet
    }
}

/// A track along `track` with a thumb over the share of it, from and to, in view:
/// across it where it is wider than tall, else down it.
fn scroll_track(painter: &egui::Painter, track: egui::Rect, (from, to): (f32, f32)) {
    let thumb = if track.width() > track.height() {
        let at = |share: f32| track.min.x + track.width() * share.min(1.0);
        track.with_min_x(at(from)).with_max_x(at(to))
    } else {
        let at = |share: f32| track.min.y + track.height() * share.min(1.0);
        track.with_min_y(at(from)).with_max_y(at(to))
    };
    painter.rect_filled(track, 1.5, TRACK);
    painter.rect_filled(thumb, 1.5, THUMB);
}

/// A line round a bar, just outside it.
fn outline(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32, width: f32) {
    painter.rect_stroke(
        rect.expand(1.5),
        3.0,
        egui::Stroke::new(width, color),
        egui::StrokeKind::Outside,
    );
}

/// What a bar says: its number (none for the start's), its function's name as the
/// graph has it, with why the journey stops there and what it says of calling round;
/// the same with the name's type or module left off; and all of it, with when it runs
/// and that it can fail.
struct Says {
    number: String,
    name: String,
    bare: String,
    whole: String,
}

impl Says {
    /// `name` after the bar's number.
    fn numbered(&self, name: &str) -> String {
        if self.number.is_empty() {
            name.to_owned()
        } else {
            format!("{}  {name}", self.number)
        }
    }
}

fn says(explorer: &Explorer, span: SpanId) -> Says {
    let disclosed = explorer.disclosure();
    let number = disclosed.bar_number(span).unwrap_or_default().to_owned();
    let label = explorer.label(span);
    // What the graph's name says of it calling round, the same words; said so, the
    // mark of a journey stopped where it runs already would say it twice.
    let note = explorer
        .track()
        .tile(span, explorer.map())
        .and_then(|tile| crate::labels::round_note(explorer, tile));
    let mark = explorer.stop_mark(span);
    let dress = |name: &str| {
        let name = match mark {
            Some(mark @ "→") => format!("{mark} {name}"),
            Some("↻") if note.is_some() => name.to_owned(),
            Some(mark) => format!("{name} {mark}"),
            None => name.to_owned(),
        };
        note.map_or_else(|| name.clone(), |note| format!("{name}  {note}"))
    };
    // `Track.unit_id` cut short says more as `unit_…` than as `Track.…`.
    let tail = match label.rsplit_once('.') {
        Some((_, tail)) if !label.contains(char::is_whitespace) && !tail.is_empty() => tail,
        _ => label.as_str(),
    };
    let when = explorer
        .span(span)
        .and_then(crate::disclosure::when)
        .map_or_else(String::new, |w| format!("  {w}"));
    // That it can fail, in the graph's words.
    let fails = if disclosed.fails(span) {
        format!("  {}", crate::labels::FAILS)
    } else {
        String::new()
    };
    let mut says = Says {
        number,
        name: dress(&label),
        bare: dress(tail),
        whole: String::new(),
    };
    says.whole = format!("{}{when}{fails}", says.numbered(&says.name));
    says
}

/// How wide a bar needs to be, in points: at the least, for its number and a few
/// letters of its name; at the most, for all it says; with its badge of calls folded
/// under it, and room round its text.
fn needs(painter: &egui::Painter, says: &Says, folded: usize) -> (f32, f32) {
    let room = 10.0 + BAR_GAP + if folded > 0 { BADGE + 2.0 } else { 0.0 };
    let least = says.numbered(&short(&says.bare, LEAST_LETTERS));
    let least = text_width(painter, &least, BAR_TEXT_SMALL) + room;
    let most = text_width(painter, &says.whole, BAR_TEXT) + room;
    (least, most.min(FULLEST).max(least))
}

/// What a bar says in `room` points, and at what size: all of it; then smaller; then
/// without when it runs; then without its name's type or module; then cut short with
/// an ellipsis, keeping the number and a couple of letters; then the number alone,
/// as a deep call's bar zoomed out by hand is narrow, so it still says which call it
/// is; then, a bar with no number, its name cut short. `None` where not even that
/// fits.
fn bar_text(painter: &egui::Painter, says: &Says, room: f32) -> Option<(String, f32)> {
    let named = says.numbered(&says.name);
    let bare = says.numbered(&says.bare);
    let tries = [
        (says.whole.as_str(), BAR_TEXT),
        (says.whole.as_str(), BAR_TEXT_SMALL),
        (named.as_str(), BAR_TEXT_SMALL),
        (bare.as_str(), BAR_TEXT_SMALL),
    ];
    if let Some((text, size)) = tries
        .into_iter()
        .find(|(text, size)| text_width(painter, text, *size) <= room)
    {
        return Some((text.to_owned(), size));
    }
    let number = says.number.as_str();
    fit_text(painter, &bare, BAR_TEXT_SMALL, room)
        .filter(|cut| cut.chars().count() > number.chars().count() + 4)
        .or_else(|| {
            (text_width(painter, number, BAR_TEXT_SMALL) <= room).then(|| number.to_owned())
        })
        .filter(|text| !text.is_empty())
        .or_else(|| {
            fit_text(painter, &says.bare, BAR_TEXT_SMALL, room)
                .filter(|cut| number.is_empty() && (*cut == says.bare || cut.chars().count() > 3))
        })
        .map(|text| (text, BAR_TEXT_SMALL))
}

/// How wide `text` is drawn at `size`, in points.
fn text_width(painter: &egui::Painter, text: &str, size: f32) -> f32 {
    painter
        .layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(size),
            egui::Color32::WHITE,
        )
        .size()
        .x
}

/// `text` at `size`, whole if it fits in `width` points, else cut short with an
/// ellipsis to fit; `None` when not even its first letter does.
fn fit_text(painter: &egui::Painter, text: &str, size: f32, width: f32) -> Option<String> {
    if text_width(painter, text, size) <= width {
        return Some(text.to_owned());
    }
    let count = text.chars().count();
    // The most characters that fit with the ellipsis, found by halving.
    let (mut fits, mut over) = (0, count);
    while over - fits > 1 {
        let mid = usize::midpoint(fits, over);
        if text_width(painter, &short(text, mid), size) <= width {
            fits = mid;
        } else {
            over = mid;
        }
    }
    (fits > 0).then(|| short(text, fits))
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
    use codeviz_core::System;
    use codeviz_ir::Document;

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
        crate::explorer::tests::opened(system, "x:main")
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
                    draw(ui.ctx(), screen, "", &mut explorer, &mut panel);
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

    #[test]
    fn a_narrow_bar_keeps_its_number() {
        // main -> (a -> c), b
        let mut explorer = crate::explorer::tests::explorer();
        explorer.set_disclose(Disclose(2));
        let journey = explorer.journey().unwrap();
        let a = journey.span(journey.root()).unwrap().children[0];
        let c = journey.span(a).unwrap().children[0];
        let ctx = egui::Context::default();
        let mut said = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter();
            let number = text_width(painter, "1.1", BAR_TEXT_SMALL);
            said = [400.0, number + 1.0, number - 1.0]
                .map(|room| bar_text(painter, &says(&explorer, c), room).map(|(text, _)| text))
                .to_vec();
        });
        output.textures_delta.clear();
        assert_eq!(
            said,
            [Some("1.1  c".to_owned()), Some("1.1".to_owned()), None],
            "whole, then the number alone, then nothing where not even that fits"
        );
    }
}
