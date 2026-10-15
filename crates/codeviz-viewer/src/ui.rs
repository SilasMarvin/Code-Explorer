//! egui around the graph: call stack and journeys on top, units / outcomes / outline
//! on the left, source and details on the right, stepping controls at the bottom; on
//! the canvas, names beside nodes, the focused node's calls in order, and how each
//! container's calls stay inside it or cross out.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use codeviz_core::{Level, SpanId, SpanTarget};
use codeviz_ir::{ExitKind, Role, UnitId};

use crate::{
    camera::OrbitCamera,
    explorer::{Autoplay, Clustering, Confidence, Explorer, SpanRef, Step, Track, is_conditional},
    graph_view::{self, Relation, relation},
    journey_panel::{self, JourneyPanel, PanelMode},
    scene::{accent, lane_color},
};

pub(crate) struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Search>()
            .init_resource::<Sources>()
            .add_systems(EguiPrimaryContextPass, draw_ui);
    }
}

#[derive(Resource, Debug, Default)]
struct Search {
    query: String,
}

/// Source files read so far; `None` when a file could not be read.
#[derive(Resource, Debug, Default)]
struct Sources(HashMap<PathBuf, Option<Vec<String>>>);

impl Sources {
    fn lines(&mut self, path: &Path) -> Option<&[String]> {
        self.0
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                std::fs::read_to_string(path)
                    .ok()
                    .map(|text| text.lines().map(str::to_owned).collect())
            })
            .as_deref()
    }
}

const ERROR: egui::Color32 = egui::Color32::from_rgb(240, 110, 100);
const OK: egui::Color32 = egui::Color32::from_rgb(120, 210, 140);
const MUTED: egui::Color32 = egui::Color32::from_rgb(140, 148, 165);

#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn draw_ui(
    mut contexts: EguiContexts,
    mut explorer: ResMut<Explorer>,
    mut autoplay: ResMut<Autoplay>,
    mut search: ResMut<Search>,
    mut sources: ResMut<Sources>,
    mut panel: ResMut<JourneyPanel>,
    mut cameras: Query<(&mut Camera, &GlobalTransform), With<OrbitCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let mut root_ui = egui::Ui::new(
        ctx.clone(),
        "codeviz".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );

    let top = egui::Panel::top("stack")
        .show(&mut root_ui, |ui| {
            breadcrumb(ui, &mut explorer, &mut panel);
        })
        .response
        .rect
        .height();
    let bottom = egui::Panel::bottom("controls")
        .show(&mut root_ui, |ui| {
            controls(ui, &mut explorer, &mut autoplay);
        })
        .response
        .rect
        .height();
    let left = egui::Panel::left("navigation")
        .resizable(true)
        .default_size(320.0)
        .show(&mut root_ui, |ui| {
            unit_search(ui, &mut explorer, &mut search);
            ui.separator();
            outcomes(ui, &mut explorer);
            ui.separator();
            outline(ui, &mut explorer);
        })
        .response
        .rect
        .width();
    let right = egui::Panel::right("source")
        .resizable(true)
        .default_size(460.0)
        .show(&mut root_ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                source(ui, &explorer, &mut sources);
                ui.separator();
                details(ui, &mut explorer);
            });
        })
        .response
        .rect
        .width();

    let screen = ctx.viewport_rect();
    let central = egui::Rect::from_min_max(
        egui::pos2(screen.min.x + left, screen.min.y + top),
        egui::pos2(screen.max.x - right, screen.max.y - bottom),
    );
    let (graph_area, panel_area) =
        journey_panel::split(panel.mode, central, journey_panel::row_count(&explorer));
    if let Ok((mut camera, transform)) = cameras.single_mut() {
        fit_viewport(&mut camera, &window, graph_area);
        let to_screen = |position: Vec3| {
            camera
                .world_to_viewport(transform, position)
                .ok()
                .map(|p| egui::pos2(p.x, p.y))
                .filter(|p| graph_area.contains(*p))
        };
        let painter = ctx
            .layer_painter(egui::LayerId::background())
            .with_clip_rect(graph_area);
        draw_graph_labels(&painter, &to_screen, &explorer, transform.up().into());
        focus_card(ctx, graph_area, &mut explorer);
        if panel.mode != PanelMode::Full {
            legend(ctx, graph_area, &mut explorer);
        }
        if let Some(area) = panel_area {
            let bar = journey_panel::draw(ctx, area, &mut explorer, &mut panel);
            // From the cursor's bar up to its node.
            let node = explorer
                .focus()
                .and_then(|unit| explorer.map().tile_of(unit))
                .and_then(|tile| explorer.tile_point(tile))
                .and_then(to_screen);
            if let (Some(bar), Some(node)) = (bar, node) {
                let tether = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("tether"),
                ));
                let from = egui::pos2(bar.min.x + 2.0, bar.min.y);
                tether.line_segment(
                    [from, node],
                    egui::Stroke::new(1.0, egui::Color32::from_white_alpha(90)),
                );
                tether.circle_filled(from, 2.5, egui::Color32::WHITE);
            }
        }
    }
    Ok(())
}

fn breadcrumb(ui: &mut egui::Ui, explorer: &mut Explorer, panel: &mut JourneyPanel) {
    ui.horizontal(|ui| {
        for (mode, label, hint) in [
            (
                PanelMode::Hidden,
                "Graph",
                "The graph alone (J hides the journey)",
            ),
            (
                PanelMode::Docked,
                "Graph + journey",
                "The journey docked over the graph (J)",
            ),
            (
                PanelMode::Full,
                "Journey",
                "The journey below a strip of graph (M)",
            ),
        ] {
            if ui
                .selectable_label(panel.mode == mode, label)
                .on_hover_text(hint)
                .clicked()
            {
                if mode != PanelMode::Hidden && panel.mode == PanelMode::Hidden {
                    explorer.journey_for_focus();
                }
                panel.mode = mode;
            }
        }
        ui.separator();
        journeys(ui, explorer);
        let Some(selected) = explorer.selected() else {
            ui.label("Pick a unit on the left.");
            return;
        };
        let mut chosen = None;
        for (index, frame) in explorer.stack(selected).into_iter().enumerate() {
            if index > 0 {
                ui.label(egui::RichText::new("›").color(MUTED));
            }
            let text = egui::RichText::new(explorer.label(frame));
            let text = if frame == selected {
                text.strong()
            } else {
                text
            };
            if ui.link(text).clicked() {
                chosen = Some(frame);
            }
        }
        if let Some(frame) = chosen {
            explorer.select(Some(frame));
        }
    });
}

/// One chip per open journey, in its accent: click to make it active, × to close.
/// Shown once there is more than one.
fn journeys(ui: &mut egui::Ui, explorer: &mut Explorer) {
    let count = explorer.tracks().len();
    if count < 2 {
        return;
    }
    let mut activate = None;
    let mut close = None;
    for index in 0..count {
        let text = egui::RichText::new(format!("■ {}", track_title(explorer, index)))
            .color(egui_color(accent(index)));
        if ui
            .selectable_label(index == explorer.active(), text)
            .on_hover_text("Make active (Tab cycles)")
            .clicked()
        {
            activate = Some(index);
        }
        if ui
            .small_button("×")
            .on_hover_text("Close (X closes the active one)")
            .clicked()
        {
            close = Some(index);
        }
    }
    ui.separator();
    if let Some(track) = activate
        && let Some(span) = explorer.tracks().get(track).and_then(Track::selected)
    {
        explorer.select_in(SpanRef { track, span });
    }
    if let Some(index) = close {
        explorer.close_track(index);
    }
}

fn controls(ui: &mut egui::Ui, explorer: &mut Explorer, autoplay: &mut Autoplay) {
    ui.horizontal(|ui| {
        let mut step = None;
        for (label, hint, action) in [
            ("⏮", "Start (Home)", Step::Start),
            ("◀", "Back (←)", Step::Back),
            (
                "Out",
                "Step out: to what runs after the caller returns (Up)",
                Step::Out,
            ),
            (
                "Over",
                "Step over: skip this call's callees (Shift + Right)",
                Step::Over,
            ),
            (
                "Into",
                "Step into: the next call in time (Right)",
                Step::Into,
            ),
        ] {
            if ui.button(label).on_hover_text(hint).clicked() {
                step = Some(action);
            }
        }
        if let Some(step) = step {
            autoplay.playing = false;
            explorer.step(step);
        }
        let play = if autoplay.playing {
            "⏸ Pause"
        } else {
            "⏵ Play"
        };
        if ui.button(play).on_hover_text("Space").clicked() {
            autoplay.playing = !autoplay.playing;
        }
        ui.add(
            egui::Slider::new(&mut autoplay.steps_per_second, 0.5..=10.0)
                .logarithmic(true)
                .text("steps/s"),
        );
        ui.separator();
        let total = explorer.journey().map_or(0, codeviz_core::Journey::len);
        let mut position = explorer.selected().map_or(0, SpanId::index);
        if total > 1
            && ui
                .add(egui::Slider::new(&mut position, 0..=total - 1).text(format!("of {total}")))
                .changed()
        {
            explorer.select(
                explorer
                    .journey()
                    .and_then(|j| j.spans().nth(position))
                    .map(|(id, _)| id),
            );
        }
    });
}

fn unit_search(ui: &mut egui::Ui, explorer: &mut Explorer, search: &mut Search) {
    ui.heading("Start from");
    ui.add(egui::TextEdit::singleline(&mut search.query).hint_text("search functions…"));
    let query = search.query.to_lowercase();
    let system = explorer.system();
    let mut rows: Vec<(UnitId, String, String)> = if query.is_empty() {
        system
            .entries()
            .iter()
            .map(|e| (e.unit.clone(), e.label.clone(), e.kind.clone()))
            .collect()
    } else {
        system
            .units()
            .filter(|u| u.role == Role::Invocable)
            .filter(|u| u.id.as_str().to_lowercase().contains(&query))
            .map(|u| (u.id.clone(), explorer.unit_name(&u.id), u.kind.clone()))
            .take(200)
            .collect()
    };
    rows.sort_by(|a, b| a.1.cmp(&b.1));
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .id_salt("units")
        .max_height(170.0)
        .show(ui, |ui| {
            for (id, title, kind) in &rows {
                ui.horizontal(|ui| {
                    let selected = explorer.root() == Some(id);
                    if ui
                        .selectable_label(selected, title)
                        .on_hover_text(id.as_str())
                        .clicked()
                    {
                        chosen = Some(id.clone());
                    }
                    ui.label(egui::RichText::new(kind).small().color(MUTED));
                });
            }
        });
    if let Some(id) = chosen {
        explorer.set_root(id);
    }
}

fn outcomes(ui: &mut egui::Ui, explorer: &mut Explorer) {
    ui.heading("Outcomes");
    let mut chosen = None;
    let current = explorer.scenario();
    for (index, scenario) in explorer.scenarios().iter().enumerate() {
        let (mark, color) = match scenario.outcome.kind {
            ExitKind::Return => ("✔", OK),
            _ => ("×", ERROR),
        };
        let text = egui::RichText::new(format!("{mark} {}", scenario.outcome.label)).color(color);
        let via: Vec<String> = scenario
            .chain
            .iter()
            .skip(1)
            .map(|site| explorer.unit_name(&site.unit))
            .collect();
        let response = ui.selectable_label(current == Some(index), text);
        let response = if via.is_empty() {
            response
        } else {
            response.on_hover_text(format!("via {}", via.join(" → ")))
        };
        if response.clicked() {
            chosen = Some(Some(index));
        }
    }
    let all = egui::RichText::new("All paths at once").color(MUTED);
    if ui.selectable_label(current.is_none(), all).clicked() {
        chosen = Some(None);
    }
    if let Some(scenario) = chosen {
        explorer.set_scenario(scenario);
    }
}

fn outline(ui: &mut egui::Ui, explorer: &mut Explorer) {
    ui.heading("Journey");
    let Some(journey) = explorer.journey() else {
        return;
    };
    let rows: Vec<(SpanId, u32, String, bool, Option<usize>)> = journey
        .spans()
        .map(|(id, span)| {
            let lane = Some(explorer.track().color(id));
            (
                id,
                span.depth,
                explorer.label(id),
                is_conditional(span),
                lane,
            )
        })
        .collect();
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .id_salt("outline")
        .show(ui, |ui| {
            for (id, depth, label, conditional, lane) in rows {
                let selected = explorer.selected() == Some(id);
                ui.horizontal(|ui| {
                    ui.add_space(depth as f32 * 12.0);
                    if let Some(lane) = lane {
                        ui.label(egui::RichText::new("■").color(egui_color(lane_color(lane))));
                    }
                    let text = if conditional {
                        egui::RichText::new(label).italics()
                    } else {
                        egui::RichText::new(label)
                    };
                    let response = ui.selectable_label(selected, text);
                    if selected {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        chosen = Some(id);
                    }
                });
            }
        });
    if let Some(id) = chosen {
        explorer.select(Some(id));
    }
}

/// The caller's source around the line that made the selected call; for the root,
/// its own definition.
fn source(ui: &mut egui::Ui, explorer: &Explorer, sources: &mut Sources) {
    let Some(selected) = explorer.selected() else {
        return;
    };
    let (owner, location, title) = if let Some((caller, location)) = explorer.call_site(selected) {
        let title = format!(
            "{} calls {}",
            explorer.unit_name(caller),
            explorer.label(selected)
        );
        (caller.clone(), location.clone(), title)
    } else {
        let Some(unit) = explorer.unit(selected) else {
            return;
        };
        let Some(location) = unit.location.clone() else {
            ui.label(egui::RichText::new("No source location.").color(MUTED));
            return;
        };
        let title = format!("{} starts here", explorer.label(selected));
        (unit.id.clone(), location, title)
    };
    ui.label(egui::RichText::new(title).strong());
    ui.label(
        egui::RichText::new(location.to_string())
            .small()
            .color(MUTED),
    );
    let Some(path) = explorer.system().local_path(&owner, &location.uri) else {
        return;
    };
    let Some(lines) = sources.lines(&path) else {
        ui.label(egui::RichText::new(format!("Cannot read {}", path.display())).color(MUTED));
        return;
    };
    let line = location.range.map_or(1, |r| r.start_line) as usize;
    // The whole enclosing function when known, otherwise a window around the line.
    let (first, last) = explorer
        .system()
        .unit(&owner)
        .and_then(|u| u.location.as_ref())
        .and_then(|l| l.range)
        .filter(|r| (r.start_line as usize..=r.end_line as usize).contains(&line))
        .map_or((line.saturating_sub(8).max(1), line + 8), |r| {
            (r.start_line as usize, r.end_line as usize)
        });
    let font = egui::FontId::monospace(12.5);
    for number in first..=last.min(lines.len()) {
        let Some(text) = lines.get(number - 1) else {
            break;
        };
        let current = number == line;
        let row = egui::RichText::new(format!("{number:>4} {text}")).font(font.clone());
        let row = if current {
            row.color(egui::Color32::WHITE)
                .background_color(egui::Color32::from_rgb(52, 70, 110))
        } else {
            row.color(egui::Color32::from_rgb(190, 196, 210))
        };
        let response = ui.label(row);
        if current {
            response.scroll_to_me(Some(egui::Align::Center));
        }
    }
}

fn details(ui: &mut egui::Ui, explorer: &mut Explorer) {
    let Some(selected) = explorer.selected() else {
        return;
    };
    let Some(span) = explorer.span(selected).cloned() else {
        return;
    };
    ui.label(
        egui::RichText::new(explorer.label(selected))
            .strong()
            .size(15.0),
    );
    if let Some(unit) = explorer.unit(selected) {
        ui.label(egui::RichText::new(unit.id.as_str()).small().color(MUTED));
        if let Some(signature) = unit.attributes.get("signature") {
            ui.label(egui::RichText::new(signature.to_string()).monospace());
        }
        if let Some(doc) = &unit.doc {
            ui.label(egui::RichText::new(doc).italics());
        }
    }
    if Confidence::of(&span) == Confidence::Guessed {
        ui.label(egui::RichText::new("Guessed: matched by name, not resolved").color(MUTED));
    }
    if let SpanTarget::Ambiguous(ids) = &span.target {
        ui.label("Could be any of:");
        for id in ids {
            ui.label(egui::RichText::new(explorer.unit_name(id)).monospace());
        }
    }
    for condition in &span.conditions {
        ui.label(format!(
            "only when {} ⇒ {}",
            condition.guard.as_deref().unwrap_or("?"),
            condition.arm
        ));
    }
    if span.repeats {
        ui.label("inside a loop: may run many times");
    }
    for exit in &span.exits {
        let color = if exit.kind == ExitKind::Return {
            OK
        } else {
            ERROR
        };
        let text = format!("exits: {}", exit.label.as_deref().unwrap_or("returns"));
        ui.label(egui::RichText::new(text).color(color));
    }
    let divergences: Vec<(usize, String)> = explorer
        .divergences()
        .iter()
        .filter(|d| d.span == selected)
        .map(|d| (d.scenario, d.label.clone()))
        .collect();
    for (scenario, label) in divergences {
        let text = egui::RichText::new(format!("× can fail here: {label}")).color(ERROR);
        if ui.link(text).clicked() {
            explorer.set_scenario(Some(scenario));
        }
    }

    ui.horizontal(|ui| {
        let can_reroot = explorer.unit_id(selected).is_some() && span.parent.is_some();
        if ui
            .add_enabled(can_reroot, egui::Button::new("Start here"))
            .clicked()
        {
            explorer.trace_from_selected();
        }
        if ui
            .add_enabled(explorer.can_go_back(), egui::Button::new("Back"))
            .clicked()
        {
            explorer.back();
        }
        if let (Some(level), SpanTarget::Unit(unit)) = (span.zoom, &span.target) {
            let label = match level {
                Level::Summary => "Zoom in",
                Level::Detail => "Zoom out",
            };
            if ui.button(label).clicked() {
                explorer.toggle_zoom(unit.clone());
            }
        }
    });
}

/// Restricts 3D rendering to the area between the panels.
fn fit_viewport(camera: &mut Camera, window: &Window, central: egui::Rect) {
    let scale = window.scale_factor();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to non-negative pixel counts"
    )]
    let to_physical = |logical: f32| (logical * scale).max(0.0).round() as u32;
    let position = UVec2::new(to_physical(central.min.x), to_physical(central.min.y));
    let size = UVec2::new(to_physical(central.width()), to_physical(central.height()))
        .min(window.physical_size().saturating_sub(position));
    if size.x == 0 || size.y == 0 {
        return;
    }
    let unchanged = camera
        .viewport
        .as_ref()
        .is_some_and(|v| v.physical_position == position && v.physical_size == size);
    if !unchanged {
        camera.viewport = Some(Viewport {
            physical_position: position,
            physical_size: size,
            ..default()
        });
    }
}

/// The journey's entry route when it starts at one, else its root's name.
pub(crate) fn track_title(explorer: &Explorer, index: usize) -> String {
    let Some(root) = explorer.tracks().get(index).and_then(|t| t.root()) else {
        return String::new();
    };
    explorer
        .system()
        .entries()
        .iter()
        .find(|e| &e.unit == root)
        .map_or_else(|| explorer.unit_name(root), |e| e.label.clone())
}

const CALLER_TEXT: egui::Color32 = egui::Color32::from_rgb(255, 184, 96);
const CALLEE_TEXT: egui::Color32 = egui::Color32::from_rgb(120, 190, 255);
const PILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(9, 11, 16, 205);

/// The 1-based places in `caller`'s calls, in written order, where it calls `callee`.
fn call_steps(explorer: &Explorer, from: usize, to_tile: usize) -> Vec<usize> {
    explorer
        .graph()
        .sequence(from)
        .iter()
        .enumerate()
        .filter(|&(_, &to)| to == to_tile)
        .map(|(index, _)| index + 1)
        .collect()
}

/// `#2` or `#2, #5`.
fn numbered(steps: &[usize]) -> String {
    steps
        .iter()
        .map(|s| format!("#{s}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A function's name, or the route of an entry point.
fn node_name(explorer: &Explorer, tile: usize) -> String {
    let Some(unit) = explorer.tile_unit(tile) else {
        return String::new();
    };
    explorer
        .system()
        .entries()
        .iter()
        .find(|e| &e.unit == unit)
        .map_or_else(
            || {
                explorer
                    .system()
                    .unit(unit)
                    .map_or_else(|| unit.to_string(), |u| u.name.clone())
            },
            |e| e.label.clone(),
        )
}

/// What a graph label names.
#[derive(Clone, Copy)]
enum Named {
    Node(usize),
    Group(usize),
}

/// Tier, then bigger first, then a stable tiebreak.
type LabelRank = (u8, std::cmp::Reverse<usize>, usize);

/// Names beside graph nodes, most important first, each on a dark pill so it reads
/// over links and glow. The focus's callees carry the order it calls them in, and its
/// callers where in their own calls they call it. A name that would overlap one
/// already placed is left out; zooming in makes room for more. Container names float
/// at the middle of their functions.
#[expect(clippy::too_many_lines, reason = "one ranked pass")]
fn draw_graph_labels(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    up: Vec3,
) {
    let (graph, map) = (explorer.graph(), explorer.map());
    let center = explorer.attention();
    let reach = explorer.reach();
    let related = |tile: usize| reach.and_then(|r| relation(r, tile));
    let stack: Vec<usize> = explorer
        .selected()
        .map(|s| explorer.stack(s))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|span| explorer.unit_id(span).and_then(|u| map.tile_of(u)))
        .collect();

    // Lower ranks are placed first and win any overlap.
    let mut order: Vec<(LabelRank, Named)> = Vec::new();
    for tile in 0..map.tiles.len() {
        let tier = match related(tile) {
            _ if center == Some(tile) => 0,
            Some(Relation::Upstream(1) | Relation::Downstream(1)) => 1,
            _ if stack.contains(&tile) => 2,
            Some(Relation::Upstream(2) | Relation::Downstream(2)) => 3,
            _ if explorer.journey_lane(tile).is_some() => 5,
            Some(_) => 6,
            None if center.is_none() => 6,
            None => continue,
        };
        order.push((
            (tier, std::cmp::Reverse(graph.degree(tile)), tile),
            Named::Node(tile),
        ));
    }
    let mut centers: Vec<(Vec3, usize)> = vec![(Vec3::ZERO, 0); map.groups.len()];
    for tile in 0..map.tiles.len() {
        if let (Some(group), Some(at)) = (graph.group(tile), explorer.tile_point(tile))
            && let Some(slot) = centers.get_mut(group)
        {
            slot.0 += at;
            slot.1 += 1;
        }
    }
    for (group, &(_, count)) in centers.iter().enumerate() {
        if count >= 2 {
            order.push(((4, std::cmp::Reverse(count), group), Named::Group(group)));
        }
    }
    order.sort_by_key(|&(key, _)| key);

    let mut taken: Vec<egui::Rect> = Vec::new();
    let mut place = |at: egui::Pos2, galley: std::sync::Arc<egui::Galley>, color, force: bool| {
        let rect = egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(5.0, 2.0));
        if !force && taken.iter().any(|r| r.intersects(rect)) {
            return;
        }
        taken.push(rect);
        painter.rect_filled(rect, 4.0, PILL);
        painter.galley(at, galley, color);
    };
    for ((tier, _, _), named) in order {
        match named {
            Named::Group(group) => {
                let (sum, count) = centers[group];
                let Some(at) = to_screen(sum / count as f32) else {
                    continue;
                };
                let color = explorer
                    .group_color(group)
                    .map_or(MUTED, |c| egui_color(lane_color(c)));
                let label = map.groups.get(group).map_or("", |g| g.label.as_str());
                let galley = painter.layout_no_wrap(
                    label.to_uppercase(),
                    egui::FontId::proportional(11.5),
                    color.gamma_multiply(0.85),
                );
                let at = at - galley.size() / 2.0;
                place(at, galley, color, false);
            }
            Named::Node(tile) => {
                let Some(world) = explorer.tile_point(tile) else {
                    continue;
                };
                let Some(node) = to_screen(world) else {
                    continue;
                };
                // Start the name just past the node's edge, however near it is.
                let radius = graph_view::node_size(graph, tile) / 2.0;
                let edge = to_screen(world + up * radius).map_or(8.0, |p| p.distance(node));
                let name = node_name(explorer, tile);
                let focus = explorer.graph_focus();
                let text = match (related(tile), focus) {
                    (Some(Relation::Downstream(1)), Some(focus)) if center == Some(focus) => {
                        format!("{}  {name}", numbered(&call_steps(explorer, focus, tile)))
                    }
                    (Some(Relation::Upstream(1)), Some(focus)) if center == Some(focus) => {
                        let steps = call_steps(explorer, tile, focus);
                        let total = graph.sequence(tile).len();
                        format!("{name}  · calls it {} of {total}", numbered(&steps))
                    }
                    _ if explorer.hovered_tile == Some(tile) => {
                        format!("{name}  · click to focus, shift-click for its journey")
                    }
                    _ => name,
                };
                let color = match (tier, related(tile)) {
                    (0, _) => egui::Color32::WHITE,
                    (_, Some(Relation::Upstream(1))) => CALLER_TEXT,
                    (_, Some(Relation::Downstream(1))) => CALLEE_TEXT,
                    (_, Some(Relation::Upstream(_))) => CALLER_TEXT.gamma_multiply(0.75),
                    (_, Some(Relation::Downstream(_))) => CALLEE_TEXT.gamma_multiply(0.75),
                    (2 | 5, None) => egui::Color32::from_rgb(225, 229, 238),
                    _ => egui::Color32::from_rgb(170, 177, 192),
                };
                let size = if tier == 0 { 16.0 } else { 13.0 };
                let galley = painter.layout_no_wrap(text, egui::FontId::proportional(size), color);
                let at = node + egui::vec2(edge + 6.0, -galley.size().y / 2.0);
                place(at, galley, color, tier == 0);
            }
        }
    }
}

#[expect(clippy::too_many_lines, reason = "one card, top to bottom")]
/// The focused node's name, then what calls it and what it calls, in the order the
/// calls are written. Rows focus their node.
fn focus_card(ctx: &egui::Context, area: egui::Rect, explorer: &mut Explorer) {
    let Some(focus) = explorer.graph_focus() else {
        return;
    };
    let graph = explorer.graph();
    let mut callers: Vec<usize> = graph.callers(focus).collect();
    callers.sort_unstable();
    callers.dedup();
    let calls = graph.sequence(focus).to_vec();
    let reach = graph.reach(focus);
    let routes: Vec<String> = explorer
        .system()
        .entries()
        .iter()
        .filter(|e| {
            explorer
                .map()
                .tile_of(&e.unit)
                .is_some_and(|t| t == focus || reach.upstream.contains_key(&t))
        })
        .map(|e| e.label.clone())
        .collect();
    let title = explorer
        .tile_unit(focus)
        .map_or_else(String::new, |u| explorer.unit_name(u));
    let mut chosen = None;
    egui::Area::new("focus_card".into())
        .fixed_pos(area.min + egui::vec2(12.0, 10.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 225))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_max_width(320.0);
                    ui.label(
                        egui::RichText::new(title)
                            .strong()
                            .size(15.0)
                            .color(egui::Color32::WHITE),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "{} upstream · {} downstream{}",
                            reach.upstream.len(),
                            reach.downstream.len(),
                            if routes.is_empty() {
                                String::new()
                            } else {
                                format!(" · reached from {}", routes.join(", "))
                            }
                        ))
                        .small()
                        .color(MUTED),
                    );
                    if !callers.is_empty() {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("CALLED BY").small().color(CALLER_TEXT));
                        for &caller in &callers {
                            let steps = call_steps(explorer, caller, focus);
                            let total = explorer.graph().sequence(caller).len();
                            let text = format!(
                                "{}  · calls it {} of {total}",
                                node_name(explorer, caller),
                                numbered(&steps)
                            );
                            if ui
                                .add(
                                    egui::Label::new(egui::RichText::new(text).color(CALLER_TEXT))
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                chosen = Some(caller);
                            }
                        }
                    }
                    if !calls.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("CALLS, IN ORDER")
                                .small()
                                .color(CALLEE_TEXT),
                        );
                        for (index, &callee) in calls.iter().enumerate().take(14) {
                            let text = format!("{:>2}  {}", index + 1, node_name(explorer, callee));
                            if ui
                                .add(
                                    egui::Label::new(egui::RichText::new(text).color(CALLEE_TEXT))
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                chosen = Some(callee);
                            }
                        }
                        if calls.len() > 14 {
                            ui.label(
                                egui::RichText::new(format!("+{} more", calls.len() - 14))
                                    .small()
                                    .color(MUTED),
                            );
                        }
                    }
                });
        });
    if let Some(tile) = chosen {
        explorer.focus_node(tile);
    }
}

/// How the graph is clustered, and per container how many of its calls stay inside
/// it: a container whose functions mostly call each other holds together when only
/// calls pull, and one that leaks calls everywhere scatters.
fn legend(ctx: &egui::Context, area: egui::Rect, explorer: &mut Explorer) {
    let stats = explorer.graph().group_stats();
    let total: usize = stats.iter().map(|s| s.internal + s.outgoing).sum();
    let inside: usize = stats.iter().map(|s| s.internal).sum();
    let mut rows: Vec<(usize, String, usize, usize, usize)> = stats
        .iter()
        .enumerate()
        .filter(|(_, s)| s.nodes > 0)
        .map(|(group, s)| {
            let label = explorer
                .map()
                .groups
                .get(group)
                .map_or_else(String::new, |g| g.label.clone());
            (
                group,
                label,
                s.nodes,
                s.internal,
                s.internal + s.outgoing + s.incoming,
            )
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.2));
    let mut clustering = explorer.clustering();
    egui::Area::new("legend".into())
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(egui::pos2(area.min.x + 12.0, area.max.y - 30.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 215))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Cluster by").small().color(MUTED));
                        ui.selectable_value(&mut clustering, Clustering::Modules, "Modules")
                            .on_hover_text("Functions in the same container pull together (K)");
                        ui.selectable_value(&mut clustering, Clustering::Calls, "Calls")
                            .on_hover_text("Only calls pull; containers that hold together mostly call themselves (K)");
                    });
                    if let Some(share) = (inside * 100).checked_div(total) {
                        ui.label(
                            egui::RichText::new(format!(
                                "{share}% of calls stay inside their container"
                            ))
                            .small()
                            .color(MUTED),
                        );
                    }
                    egui::CollapsingHeader::new(egui::RichText::new("Containers").small())
                        .default_open(false)
                        .show(ui, |ui| {
                    egui::Grid::new("legend_rows").spacing(egui::vec2(8.0, 1.0)).show(ui, |ui| {
                        for (group, label, nodes, internal, touching) in rows.iter().take(10) {
                            let color = explorer
                                .group_color(*group)
                                .map_or(MUTED, |c| egui_color(lane_color(c)));
                            ui.label(egui::RichText::new("■").color(color));
                            ui.label(egui::RichText::new(label).small());
                            ui.label(egui::RichText::new(format!("{nodes} fn")).small().color(MUTED));
                            let share = if *touching == 0 {
                                "—".to_owned()
                            } else {
                                format!("{}% inside", internal * 100 / touching)
                            };
                            ui.label(egui::RichText::new(share).small().color(MUTED));
                            ui.end_row();
                        }
                    });
                    if rows.len() > 10 {
                        ui.label(egui::RichText::new(format!("+{} more", rows.len() - 10)).small().color(MUTED));
                    }
                        });
                });
        });
    if clustering != explorer.clustering() {
        explorer.set_clustering(clustering);
    }
    ctx.layer_painter(egui::LayerId::background())
        .with_clip_rect(area)
        .text(
            egui::pos2(area.min.x + 12.0, area.max.y - 10.0),
            egui::Align2::LEFT_BOTTOM,
            "click a node to focus · shift-click opens its journey · Enter traces from it · \
             J journey · M swap · K clustering · F front · drag to orbit · scroll or pinch to zoom",
            egui::FontId::proportional(12.0),
            MUTED,
        );
}

pub(crate) fn short(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

pub(crate) fn egui_color(color: Color) -> egui::Color32 {
    let [r, g, b, a] = color.to_srgba().to_u8_array();
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}
