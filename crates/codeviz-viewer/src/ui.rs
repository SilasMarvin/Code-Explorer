//! egui around the graph: call stack and journeys on top, and a side panel, hidden at
//! first, with the view's settings, outcomes, source and details; on the canvas, the
//! focus with what calls it and what it calls, names beside nodes, a key to the
//! colors, and a search box on `/`.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use codeviz_core::{Change, Level, SpanTarget};
use codeviz_ir::ExitKind;

use crate::{
    camera::OrbitCamera,
    diff_view::{self, DiffStyle, DiffView},
    explorer::{Clustering, Confidence, Explorer, Side, SpanRef, Track},
    graph_view::{self, CALLEE, CALLER, Relation, relation},
    journey_panel::{self, JourneyPanel, PanelMode},
    lens::{self, Measure, Source},
    routes::Start,
    scene::{PALETTE, accent, lane_color},
    search::{self, Search},
    source_diff,
};

pub(crate) struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Search>()
            .init_resource::<KeyHelp>()
            .init_resource::<Chrome>()
            .init_resource::<Sources>()
            .add_systems(EguiPrimaryContextPass, (fallback_fonts, draw_ui).chain());
    }
}

/// egui's proportional font has no arrows; its monospace one does, so it fills in.
fn fallback_fonts(mut contexts: EguiContexts, mut done: Local<bool>) -> Result {
    if *done {
        return Ok(());
    }
    let mut fonts = egui::FontDefinitions::default();
    let monospace = fonts
        .families
        .get(&egui::FontFamily::Monospace)
        .and_then(|family| family.first())
        .cloned();
    if let (Some(name), Some(family)) = (
        monospace,
        fonts.families.get_mut(&egui::FontFamily::Proportional),
    ) {
        family.push(name);
    }
    contexts.ctx_mut()?.set_fonts(fonts);
    *done = true;
    Ok(())
}

/// Whether the key reference is showing.
#[derive(Resource, Debug, Default)]
pub(crate) struct KeyHelp(pub(crate) bool);

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
pub(crate) const MUTED: egui::Color32 = egui::Color32::from_rgb(140, 148, 165);

/// What draws around the graph: everything, or bare, with only the graph and the
/// journey; and whether the side panel and the search box are open.
#[derive(Resource, Default)]
pub(crate) struct Chrome {
    pub(crate) bare: bool,
    pub(crate) side: bool,
    pub(crate) search: bool,
}

#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn draw_ui(
    mut contexts: EguiContexts,
    mut explorer: ResMut<Explorer>,
    mut search: ResMut<Search>,
    mut sources: ResMut<Sources>,
    mut panel: ResMut<JourneyPanel>,
    mut help: ResMut<KeyHelp>,
    mut chrome: ResMut<Chrome>,
    time: Res<Time>,
    mut diff: Option<ResMut<DiffView>>,
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

    let [top, bottom, left, right] = if chrome.bare {
        [0.0; 4]
    } else {
        panels(
            &mut root_ui,
            &mut explorer,
            &mut panel,
            &mut sources,
            &mut chrome,
            diff.as_deref(),
            time.elapsed_secs(),
        )
    };

    let screen = ctx.viewport_rect();
    let central = egui::Rect::from_min_max(
        egui::pos2(screen.min.x + left, screen.min.y + top),
        egui::pos2(screen.max.x - right, screen.max.y - bottom),
    );
    let (graph_area, panel_area) =
        journey_panel::split(panel.mode, central, journey_panel::extent(&explorer));
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
        draw_graph_labels(
            &painter,
            &to_screen,
            &explorer,
            diff.as_deref(),
            transform.up().into(),
        );
        if let Some(diff) = diff.as_deref_mut().filter(|_| !chrome.bare) {
            diff_view::diff_card(ctx, graph_area, &mut explorer, diff);
        }
        if !chrome.bare {
            routes_card(ctx, graph_area, &mut explorer);
        }
        if chrome.search && !chrome.bare {
            search::search_box(
                ctx,
                graph_area,
                &mut explorer,
                &mut search,
                &mut chrome.search,
                time.elapsed_secs(),
            );
        }
        if panel.mode != PanelMode::Full && !chrome.bare {
            legend(ctx, above(graph_area, panel_area), diff.is_some());
        }
        if help.0 {
            key_help(ctx, graph_area, &mut help.0);
        }
        if let Some(area) = panel_area {
            let tethers =
                journey_panel::draw(ctx, area, &mut explorer, &mut panel, diff.as_deref());
            if let (Some(bar), Some(node)) = (tethers.cursor, cursor_node(&explorer, &to_screen)) {
                tether(ctx, bar, node, false);
            }
            // The bar under the pointer, to the node it stands for.
            let hovered = explorer
                .hovered_tile
                .and_then(|tile| explorer.tile_point(tile))
                .and_then(to_screen);
            if let (Some(bar), Some(node)) = (tethers.hovered, hovered) {
                tether(ctx, bar, node, true);
            }
        }
    }
    Ok(())
}

/// The panels around the graph, and how much of each edge they take: top, bottom, left
/// and right. The side panel shows only when opened (S).
fn panels(
    root_ui: &mut egui::Ui,
    explorer: &mut Explorer,
    panel: &mut JourneyPanel,
    sources: &mut Sources,
    chrome: &mut Chrome,
    diff: Option<&DiffView>,
    now: f32,
) -> [f32; 4] {
    let top = egui::Panel::top("stack")
        .show(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (label, hint) = if chrome.side {
                        ("Hide panel", "Hide the side panel (S)")
                    } else {
                        ("Panel", "Settings, outcomes, source and details (S)")
                    };
                    if ui
                        .selectable_label(chrome.side, label)
                        .on_hover_text(hint)
                        .clicked()
                    {
                        chrome.side = !chrome.side;
                    }
                    if ui
                        .selectable_label(chrome.search, "Search")
                        .on_hover_text("Find a function, or where an error is raised (/)")
                        .clicked()
                    {
                        chrome.search = !chrome.search;
                    }
                    focus_sides(ui, explorer, now);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        breadcrumb(ui, explorer, panel);
                    });
                });
            });
        })
        .response
        .rect
        .height();
    let right = if chrome.side {
        egui::Panel::right("source")
            .resizable(true)
            .default_size(460.0)
            .show(root_ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    view_settings(ui, explorer, diff.is_some());
                    ui.separator();
                    outcomes(ui, explorer);
                    ui.separator();
                    match diff {
                        Some(view) => changed_source(ui, explorer, view, sources),
                        None => source(ui, explorer, sources),
                    }
                    ui.separator();
                    details(ui, explorer);
                });
            })
            .response
            .rect
            .width()
    } else {
        0.0
    };
    [top, 0.0, 0.0, right]
}

/// How the graph is drawn: what the lens shows, and what pulls nodes together.
fn view_settings(ui: &mut egui::Ui, explorer: &mut Explorer, diffing: bool) {
    ui.heading("View");
    reveal_controls(ui, explorer, diffing);
    let mut clustering = explorer.clustering();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Cluster by").small().color(MUTED));
        ui.selectable_value(&mut clustering, Clustering::Modules, "Modules")
            .on_hover_text("Functions in the same container pull together (K)");
        ui.selectable_value(&mut clustering, Clustering::Calls, "Calls")
            .on_hover_text(
                "Only calls pull; containers that hold together mostly call themselves (K)",
            );
    });
    if clustering != explorer.clustering() {
        explorer.set_clustering(clustering);
    }
}

/// Where the cursor's node is on screen, for the tether from its bar.
fn cursor_node(
    explorer: &Explorer,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
) -> Option<egui::Pos2> {
    explorer
        .focus()
        .and_then(|unit| explorer.map().tile_of(unit))
        .and_then(|tile| explorer.tile_point(tile))
        .and_then(to_screen)
}

/// The part of the graph above a docked journey, for what sits at its bottom.
fn above(graph: egui::Rect, panel: Option<egui::Rect>) -> egui::Rect {
    panel.map_or(graph, |p| {
        egui::Rect::from_min_max(graph.min, egui::pos2(graph.max.x, p.min.y))
    })
}

/// A line from a bar in the journey up to its node in the graph: faint for the
/// cursor's, bright for the one under the pointer.
fn tether(ctx: &egui::Context, bar: egui::Rect, node: egui::Pos2, bright: bool) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("tether"),
    ));
    let from = if bright {
        egui::pos2(bar.center().x, bar.min.y)
    } else {
        egui::pos2(bar.min.x + 2.0, bar.min.y)
    };
    let alpha = if bright { 220 } else { 90 };
    painter.line_segment(
        [from, node],
        egui::Stroke::new(
            if bright { 1.5 } else { 1.0 },
            egui::Color32::from_white_alpha(alpha),
        ),
    );
    painter.circle_filled(from, 2.5, egui::Color32::WHITE);
    if bright {
        painter.circle_stroke(node, 9.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
    }
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

/// The focused function as the diff changed it: its old and new source as one
/// unified diff.
fn changed_source(ui: &mut egui::Ui, explorer: &Explorer, view: &DiffView, sources: &mut Sources) {
    let Some((tile, unit)) = explorer
        .graph_focus()
        .and_then(|tile| Some((tile, explorer.tile_unit(tile)?.clone())))
    else {
        ui.label(egui::RichText::new("Click a node to see how it changed.").color(MUTED));
        return;
    };
    let change = view.change(tile);
    let heading = format!(
        "{}  {}",
        change.map_or(" ", diff_view::sign),
        explorer.unit_name(&unit)
    );
    let color = change.map_or(egui::Color32::WHITE, |c| {
        egui_color(diff_view::change_color(c))
    });
    ui.label(
        egui::RichText::new(heading)
            .strong()
            .size(15.0)
            .color(color),
    );
    let mut side = |system: &codeviz_core::System| -> Option<(Vec<String>, usize, String)> {
        let location = system.unit(&unit)?.location.as_ref()?;
        let range = location.range?;
        let path = system.local_path(&unit, &location.uri)?;
        let lines = sources.lines(&path)?;
        let (first, last) = (range.start_line as usize, range.end_line as usize);
        let body = lines
            .get(first.checked_sub(1)?..last.min(lines.len()))?
            .to_vec();
        Some((body, first, location.uri.clone()))
    };
    let (old, new) = (side(view.old()), side(explorer.system()));
    let place = match (&old, &new) {
        (Some((_, a, uri_a)), Some((_, b, uri_b))) if uri_a != uri_b || a != b => {
            format!("was {uri_a}:{a}, now {uri_b}:{b}")
        }
        (_, Some((_, line, uri))) | (Some((_, line, uri)), None) => format!("{uri}:{line}"),
        (None, None) => {
            ui.label(egui::RichText::new("No source to show.").color(MUTED));
            return;
        }
    };
    ui.label(egui::RichText::new(place).small().color(MUTED));
    if let Some(was) = view.moved_from(tile) {
        ui.label(
            egui::RichText::new(format!("was {was}"))
                .small()
                .color(color),
        );
    }
    if change.is_none() {
        ui.label(egui::RichText::new("Unchanged.").small().color(MUTED));
    }
    source_diff::show(ui, as_side(old.as_ref()), as_side(new.as_ref()));
}

fn as_side(side: Option<&(Vec<String>, usize, String)>) -> Option<source_diff::Side<'_>> {
    side.map(|(lines, first, _)| source_diff::Side {
        lines: lines.as_slice(),
        first: *first,
    })
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

const CALLER_TEXT: egui::Color32 = egui::Color32::from_rgb(206, 170, 255);
const CALLEE_TEXT: egui::Color32 = egui::Color32::from_rgb(130, 196, 255);
/// Text on a bar or a badge filled with a node's color.
const ON_FILL: egui::Color32 = egui::Color32::from_rgb(14, 16, 22);
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
pub(crate) fn node_name(explorer: &Explorer, tile: usize) -> String {
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
                    .any_unit(unit)
                    .map_or_else(|| unit.to_string(), |u| u.name.clone())
            },
            |e| e.label.clone(),
        )
}

/// What a graph label names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Named {
    Node(usize),
    Group(usize),
}

/// Tier, then bigger first, then a stable tiebreak.
type LabelRank = (u8, std::cmp::Reverse<usize>, usize);

/// Where a label may go around its node, in order of preference.
const SIDES: [egui::Align2; 4] = [
    egui::Align2::LEFT_CENTER,
    egui::Align2::RIGHT_CENTER,
    egui::Align2::CENTER_BOTTOM,
    egui::Align2::CENTER_TOP,
];

/// Room kept around a label's text: its pill.
const PILL_PAD: egui::Vec2 = egui::vec2(5.0, 2.0);
/// How fast a label slides to a new side: the share of the way left it covers in a
/// second is `1 - e^-SLIDE`.
const SLIDE: f32 = 14.0;
/// Seconds a label takes to fade in or out.
const FADE: f32 = 0.18;

/// A label as last drawn, so the next frame eases it instead of jumping it.
#[derive(Clone)]
struct Shown {
    /// Index into [`SIDES`].
    side: usize,
    /// Where its text sits, from its node.
    offset: egui::Vec2,
    opacity: f32,
    galley: std::sync::Arc<egui::Galley>,
    stroke: Option<egui::Color32>,
}

/// Every label drawn last frame.
type ShownLabels = HashMap<Named, Shown>;

/// Places labels so none overlaps another or a node that matters, and draws them
/// eased from where they were: a label keeps its side while that side stays free,
/// slides when it must change sides, and fades in and out instead of popping.
struct Placer<'a> {
    painter: &'a egui::Painter,
    taken: Vec<egui::Rect>,
    last: ShownLabels,
    next: ShownLabels,
    /// Where each label placed this frame sits.
    anchors: HashMap<Named, egui::Pos2>,
    /// The share of the way to its new side a label covers this frame.
    slide: f32,
    /// How much opacity a label gains or loses this frame.
    fade: f32,
}

impl<'a> Placer<'a> {
    fn new(painter: &'a egui::Painter, taken: Vec<egui::Rect>) -> Self {
        let dt = painter.ctx().input(|i| i.stable_dt).min(0.1);
        Self {
            painter,
            taken,
            last: painter
                .ctx()
                .data_mut(|d| d.get_temp::<ShownLabels>(Self::id()))
                .unwrap_or_default(),
            next: HashMap::new(),
            anchors: HashMap::new(),
            slide: 1.0 - (-SLIDE * dt).exp(),
            fade: dt / FADE,
        }
    }

    fn id() -> egui::Id {
        egui::Id::new("graph_labels")
    }

    /// Places `job` on a side of `node` (a disc of `edge` points) where it overlaps
    /// nothing already placed: the side it had last frame if that is still free,
    /// else the first free one, else the first anyway when `force` is set. Returns
    /// whether it was placed.
    fn place(
        &mut self,
        key: Named,
        node: egui::Pos2,
        edge: f32,
        job: egui::text::LayoutJob,
        stroke: Option<egui::Color32>,
        force: bool,
    ) -> bool {
        let galley = self.painter.layout_job(job);
        let extent = galley.size();
        let gap = edge + 6.0;
        let spot = |side: usize| {
            let anchor = match SIDES[side] {
                egui::Align2::LEFT_CENTER => node + egui::vec2(gap, 0.0),
                egui::Align2::RIGHT_CENTER => node - egui::vec2(gap, 0.0),
                egui::Align2::CENTER_BOTTOM => node - egui::vec2(0.0, gap),
                _ => node + egui::vec2(0.0, gap),
            };
            SIDES[side].anchor_size(anchor, extent)
        };
        // Inside the graph, so no label is cut off at its edge.
        let bounds = self.painter.clip_rect();
        let fits = |rect: egui::Rect, pad: egui::Vec2| {
            bounds.contains_rect(rect.expand2(pad))
                && !self.taken.iter().any(|r| r.intersects(rect.expand2(pad)))
        };
        let previous = self.last.get(&key);
        // A label holds its side until something actually covers it; a new side must
        // be clear with room to spare. That margin keeps a label from flipping back
        // and forth as the view moves.
        let kept = previous
            .map(|p| p.side)
            .filter(|&side| fits(spot(side), egui::Vec2::ZERO));
        let side = kept
            .or_else(|| (0..SIDES.len()).find(|&side| fits(spot(side), PILL_PAD)))
            .or_else(|| force.then(|| previous.map_or(0, |p| p.side)));
        let Some(side) = side else {
            return false;
        };
        let rect = spot(side);
        self.taken.push(rect.expand2(PILL_PAD));
        let target = rect.min - node;
        let (offset, opacity) = match previous {
            Some(p) => (
                p.offset + (target - p.offset) * self.slide,
                (p.opacity + self.fade).min(1.0),
            ),
            // A new label fades in where it belongs.
            None => (target, self.fade.min(1.0)),
        };
        self.anchors.insert(key, node);
        self.next.insert(
            key,
            Shown {
                side,
                offset,
                opacity,
                galley,
                stroke,
            },
        );
        true
    }

    /// Draws what was placed, and fades out what was not where its node, found by
    /// `anchor`, still shows. Remembers it all for the next frame.
    fn finish(mut self, anchor: impl Fn(Named) -> Option<egui::Pos2>) {
        let mut leaving = Vec::new();
        for (key, mut shown) in std::mem::take(&mut self.last) {
            if self.next.contains_key(&key) {
                continue;
            }
            shown.opacity -= self.fade;
            if shown.opacity > 0.0
                && let Some(node) = anchor(key)
            {
                leaving.push((key, node, shown));
            }
        }
        // Leaving labels go underneath the ones that hold their place.
        for (_, node, shown) in &leaving {
            self.paint(*node, shown);
        }
        for (key, shown) in &self.next {
            if let Some(&node) = self.anchors.get(key) {
                self.paint(node, shown);
            }
        }
        self.next
            .extend(leaving.into_iter().map(|(key, _, shown)| (key, shown)));
        let next = self.next;
        self.painter
            .ctx()
            .data_mut(|d| d.insert_temp(Self::id(), next));
    }

    /// A label on its pill, at its eased place beside `node`.
    fn paint(&self, node: egui::Pos2, shown: &Shown) {
        let mut painter = self.painter.clone();
        painter.multiply_opacity(shown.opacity.clamp(0.0, 1.0));
        let min = egui::emath::GuiRounding::round_to_pixels(
            node + shown.offset,
            painter.pixels_per_point(),
        );
        let pill = egui::Rect::from_min_size(min, shown.galley.size()).expand2(PILL_PAD);
        painter.rect_filled(pill, 4.0, PILL);
        if let Some(color) = shown.stroke {
            painter.rect_stroke(
                pill,
                4.0,
                egui::Stroke::new(1.5, color),
                egui::StrokeKind::Inside,
            );
        }
        painter.galley(min, shown.galley.clone(), egui::Color32::WHITE);
    }
}

/// What leads a label: text in a color, on a fill when it is a badge.
type Lead = (String, egui::Color32, Option<egui::Color32>);

/// A label in two parts: the name, then a quieter note.
fn label_job(
    lead: Option<Lead>,
    name: &str,
    note: Option<String>,
    color: egui::Color32,
    size: f32,
    struck: bool,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let format = |color, size| egui::TextFormat {
        font_id: egui::FontId::proportional(size),
        color,
        ..default()
    };
    if let Some((lead, lead_color, fill)) = lead {
        let mut lead_format = format(lead_color, size);
        if let Some(fill) = fill {
            lead_format.background = fill;
        }
        job.append(&lead, 0.0, lead_format);
        job.append(" ", 0.0, format(color, size));
    }
    let mut name_format = format(color, size);
    if struck {
        name_format.strikethrough = egui::Stroke::new(1.2, color);
    }
    job.append(name, 0.0, name_format);
    if let Some(note) = note {
        job.append(&note, 6.0, format(MUTED, size - 1.5));
    }
    job
}

/// What the lens shows: how far, around what, and measured how.
fn reveal_controls(ui: &mut egui::Ui, explorer: &mut Explorer, diffing: bool) {
    let mut lens = explorer.lens();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Show").small().color(MUTED));
        if lens.changes_only && diffing {
            ui.label("only changes")
                .on_hover_text("Everything else is hidden; C or the Changes card shows it");
            return;
        }
        if ui
            .selectable_label(lens.reach.is_none(), "All")
            .on_hover_text("Every function (=, until it shows all)")
            .clicked()
        {
            lens.reach = None;
        }
        let mut reach = lens.reach.unwrap_or(lens::MAX_REACH + 1);
        let unit = match (lens.measure, lens.reach) {
            (Measure::Calls, Some(1)) => "call",
            (Measure::Calls, _) => "calls",
            (_, Some(1)) => "step",
            _ => "steps",
        };
        if ui
            .add(
                egui::Slider::new(&mut reach, 0..=lens::MAX_REACH + 1)
                    .show_value(false)
                    .custom_formatter(|_, _| String::new()),
            )
            .on_hover_text("How far out from the start shows in full (- and =)")
            .changed()
        {
            lens.reach = (reach <= lens::MAX_REACH).then_some(reach);
        }
        if let Some(reach) = lens.reach {
            ui.label(egui::RichText::new(format!("{reach} {unit}")).small());
        }
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Around").small().color(MUTED));
        for source in [Source::Focus, Source::Changes, Source::Journey] {
            if source == Source::Changes && !diffing {
                continue;
            }
            ui.selectable_value(&mut lens.source, source, source.label())
                .on_hover_text("What the reveal starts from (L)");
        }
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Near by").small().color(MUTED));
        ui.selectable_value(&mut lens.measure, Measure::Calls, "calls")
            .on_hover_text("Calls between, along call chains (G)");
        ui.selectable_value(&mut lens.measure, Measure::Space, "space")
            .on_hover_text("Distance in the graph, in steps of a typical call's length (G)");
        ui.selectable_value(&mut lens.measure, Measure::Blend, "blend")
            .on_hover_text(
                "Distance in the graph, with each call between counting against it; \
                 direct callers and callees always show (G)",
            );
    });
    if lens != explorer.lens() {
        explorer.set_lens(lens);
    }
}

/// Names beside graph nodes, most important first, each on a dark pill so it reads
/// over links and glow. The focus's callees lead with the order it calls them in, and
/// its callers note where in their own calls they call it. A name tries each side of
/// its node and is left out when none is free, and never covers a node that matters;
/// zooming in makes room for more. Container names fill whatever space is left.
#[expect(clippy::too_many_lines, reason = "one ranked pass")]
fn draw_graph_labels(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    diff: Option<&DiffView>,
    up: Vec3,
) {
    let (graph, map) = (explorer.graph(), explorer.map());
    // Under a diff, what changed is named first and what it reaches next.
    let diff_tier = |tile: usize| {
        let diff = diff?;
        if diff.presence(tile) < 0.3 {
            return Some(u8::MAX);
        }
        match (diff.change(tile), diff.style, diff.impact(tile)) {
            (Some(_), _, _) => Some(1),
            (None, DiffStyle::Impact, Some(_)) if diff.affected.iter().any(|a| a.tile == tile) => {
                Some(1)
            }
            (None, DiffStyle::Impact, Some(_)) => Some(3),
            _ => None,
        }
    };
    let center = explorer.attention();
    let reach = explorer.reach();
    let related = |tile: usize| reach.and_then(|r| relation(r, tile));
    let spotlit = explorer.spotlit_hops();
    // Where the spotlight's routes start, and what reaches the focus from there.
    let starts = match (explorer.spotlight(), explorer.graph_focus()) {
        (Some(spotlight), Some(focus)) if spotlight.side == Side::Callers => explorer.starts(focus),
        _ => Arc::default(),
    };
    let route_starts: Vec<usize> = starts
        .iter()
        .map(|s| s.tile)
        .filter(|t| spotlit.contains_key(t))
        .collect();
    let disclosed = explorer.disclosure();
    let journey = disclosed.tiles();
    let opened = disclosed.opened();
    let stack: Vec<usize> = explorer
        .selected()
        .map(|s| explorer.stack(s))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|span| explorer.unit_id(span).and_then(|u| map.tile_of(u)))
        .collect();
    // Where each shown node sits on screen, and how far its edge is from its middle.
    let discs: Vec<Option<(egui::Pos2, f32)>> = (0..map.tiles.len())
        .map(|tile| {
            // Only what the lens shows at least half way gets a name.
            let world = explorer
                .tile_point(tile)
                .filter(|_| explorer.visibility(tile) >= 0.5)?;
            let node = to_screen(world)?;
            let radius = graph_view::node_size(graph, tile) / 2.0;
            let edge = to_screen(world + up * radius).map_or(8.0, |p| p.distance(node));
            Some((node, edge))
        })
        .collect();

    // Lower ranks are placed first and win any overlap.
    let mut order: Vec<(LabelRank, Named)> = Vec::new();
    for (tile, disc) in discs.iter().enumerate() {
        if disc.is_none() {
            continue;
        }
        let tier = match related(tile) {
            _ if center == Some(tile) => 0,
            // What the spotlight picks out is named, whatever else would be.
            _ if spotlit.contains_key(&tile) => 1,
            _ if diff_tier(tile) == Some(u8::MAX) => continue,
            // What the open journey discloses is named first, with its numbers; with
            // a journey open, what it folds away and callers further up go unnamed.
            _ if journey.contains_key(&tile) => 1,
            // A change is named even where the journey folds it away, after what the
            // journey shows.
            _ if diff_tier(tile).is_some() => diff_tier(tile).map_or(2, |tier| tier.max(2)),
            Some(Relation::Downstream(_)) if opened => continue,
            Some(Relation::Upstream(hops)) if opened && hops > 1 => continue,
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
    // Modules are named at the top of their rings, ahead of most function names.
    let regions = graph_view::regions(explorer);
    let ring_top = |cluster: usize| {
        let &(_, middle, radius) = regions.iter().find(|r| r.0 == cluster)?;
        to_screen(middle + up * radius)
    };
    if explorer.lens().reach.is_none() {
        for &(cluster, _, radius) in &regions {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a rough size for ordering"
            )]
            let size = radius as usize;
            let tier = if center.is_some() { 4 } else { 1 };
            order.push((
                (tier, std::cmp::Reverse(size), cluster),
                Named::Group(cluster),
            ));
        }
    }
    order.sort_by_key(|&(key, _)| key);

    // Labels keep off the cards over the graph, as last laid out.
    let cards: Vec<egui::Rect> = ["diff_card", "legend", "search", "routes"]
        .into_iter()
        .filter_map(|id| painter.ctx().memory(|m| m.area_rect(egui::Id::new(id))))
        .collect();
    // And off the nodes that matter, so they never hide a neighbor.
    let mut placer = Placer::new(
        painter,
        cards
            .into_iter()
            .chain(
                order
                    .iter()
                    .filter(|((tier, _, _), _)| *tier <= 3)
                    .filter_map(|&(_, named)| match named {
                        Named::Node(tile) => discs[tile],
                        Named::Group(_) => None,
                    })
                    .map(|(node, edge)| {
                        egui::Rect::from_center_size(node, egui::Vec2::splat(edge * 2.0))
                    }),
            )
            .collect(),
    );
    let focus = explorer.graph_focus();
    for ((tier, _, _), named) in order {
        match named {
            Named::Group(group) => {
                let Some(at) = ring_top(group) else {
                    continue;
                };
                let color = egui_color(lane_color(group % PALETTE.len()))
                    .gamma_multiply(if center.is_some() { 0.6 } else { 0.9 });
                let label = explorer.cluster_name(group);
                let job = label_job(None, &label.to_uppercase(), None, color, 12.0, false);
                placer.place(named, at, 0.0, job, None, false);
            }
            Named::Node(tile) => {
                let Some((node, edge)) = discs[tile] else {
                    continue;
                };
                let change = diff.and_then(|d| d.change(tile));
                // A change is named in full: `new` alone could be any type's.
                let name = match (change, explorer.tile_unit(tile)) {
                    (Some(_), Some(unit)) => explorer.unit_name(unit),
                    _ => node_name(explorer, tile),
                };
                let relation = related(tile);
                let color = match (tier, relation) {
                    (0, _) => egui::Color32::WHITE,
                    (_, Some(Relation::Upstream(1))) => CALLER_TEXT,
                    (_, Some(Relation::Downstream(1))) => CALLEE_TEXT,
                    // Only what touches the focus is colored, as its links are.
                    (_, Some(_)) => egui::Color32::from_rgb(200, 205, 216),
                    (2 | 5, None) => egui::Color32::from_rgb(225, 229, 238),
                    _ => egui::Color32::from_rgb(160, 167, 182),
                };
                let focused = focus.filter(|&f| center == Some(f));
                let impacted =
                    diff.is_some_and(|d| d.style == DiffStyle::Impact && d.impact(tile).is_some());
                let color = match change {
                    Some(change) if tier != 0 => egui_color(diff_view::change_color(change)),
                    _ if impacted && tier != 0 => {
                        egui_color(diff_view::MODIFIED).gamma_multiply(0.8)
                    }
                    // Amber and blue are not drawn under a diff, and what did not change
                    // steps back.
                    _ if diff.is_some() && tier != 0 => egui::Color32::from_rgb(150, 156, 172),
                    _ => color,
                };
                let impact_note = diff
                    .filter(|d| d.style == DiffStyle::Impact)
                    .and_then(|d| d.affected.iter().find(|a| a.tile == tile))
                    .map(|a| match a.hops {
                        0 => "changed".to_owned(),
                        1 => "calls a change".to_owned(),
                        n => format!("{n} calls from a change"),
                    });
                let (lead, aside) = match (relation, focused) {
                    _ if change.is_some() => (
                        change.map(|c| (diff_view::sign(c).to_owned(), color, None)),
                        diff.and_then(|d| d.moved_from(tile))
                            .map(|was| format!("was {}", tail(was))),
                    ),
                    _ if impact_note.is_some() => (None, impact_note),
                    (Some(Relation::Downstream(1)), Some(focus)) => {
                        let steps = call_steps(explorer, focus, tile);
                        (Some((numbered(&steps), color, None)), None)
                    }
                    // With the spotlight on, names alone, so more of them fit.
                    (Some(Relation::Upstream(1)), Some(_)) if !spotlit.is_empty() => (None, None),
                    (Some(Relation::Upstream(1)), Some(focus)) => {
                        let steps = call_steps(explorer, tile, focus);
                        let total = graph.sequence(tile).len();
                        (
                            None,
                            Some(format!("calls it {} of {total}", numbered(&steps))),
                        )
                    }
                    _ => (None, None),
                };
                // How the journey runs it, after anything else said of it.
                let marks: Vec<&str> = [
                    (disclosed.tile_sometimes(tile), "sometimes"),
                    (disclosed.tile_fails(tile), "can fail"),
                ]
                .into_iter()
                .filter_map(|(on, mark)| on.then_some(mark))
                .collect();
                // The focus says where its routes start, once they are picked out.
                let routes = (tier == 0 && focused.is_some() && !starts.is_empty())
                    .then(|| reached_from(&starts));
                let aside = [
                    aside,
                    routes,
                    (!marks.is_empty()).then(|| marks.join(" · ")),
                ]
                .into_iter()
                .flatten()
                .reduce(|a, b| format!("{a} · {b}"));
                // A node the journey runs leads with its place in it, as its bar does.
                let order = journey
                    .get(&tile)
                    .filter(|numbers| !numbers.is_empty())
                    .map(|numbers| match numbers.as_slice() {
                        [one] => one.clone(),
                        [first, second] => format!("{first}, {second}"),
                        [first, .., last] => format!("{first} … {last}"),
                        [] => String::new(),
                    });
                // The number on a badge filled like the node and its bar, so the two
                // read as one; what changed after it.
                let fill = egui_color(graph_view::node_color(explorer, diff, tile));
                let lead = match (order, change) {
                    (Some(order), Some(change)) => Some((
                        format!(" {order} {} ", diff_view::sign(change)),
                        ON_FILL,
                        Some(fill),
                    )),
                    (Some(order), None) => Some((format!(" {order} "), ON_FILL, Some(fill))),
                    (None, _) => lead,
                };
                let size = match tier {
                    0 => 14.0,
                    1 => 12.0,
                    _ => 11.0,
                };
                // The pill's border says which way it relates to the focus, as its link
                // does.
                let stroke = match relation {
                    _ if tier == 0 => Some(egui::Color32::from_white_alpha(140)),
                    Some(Relation::Upstream(1)) => Some(egui_color(CALLER)),
                    Some(Relation::Downstream(1)) => Some(egui_color(CALLEE)),
                    // Where a route starts is ringed pale, as its node is.
                    _ if route_starts.contains(&tile) => Some(egui_color(graph_view::FURTHER)),
                    _ => None,
                };
                // What is gone is struck through, which reads at any size.
                let struck = change == Some(Change::Removed);
                let job = label_job(lead, &name, aside, color, size, struck);
                // The focus, what the spotlight picks out next to it, and where its routes
                // start, are named come what may.
                let force =
                    tier == 0 || spotlit.get(&tile) == Some(&1) || route_starts.contains(&tile);
                placer.place(named, node, edge, job, stroke, force);
            }
        }
    }
    placer.finish(|named| match named {
        Named::Node(tile) => discs.get(tile).copied().flatten().map(|(node, _)| node),
        Named::Group(group) => ring_top(group),
    });
}

/// A button for each side of the focus: what calls it (I) and what it calls (O), each
/// in the color its links are drawn in, and how far the spotlight reaches on it. A
/// button puts its side in the spotlight, a call further with each press, which
/// frames everything it picks out. Then where the routes into the focus start
/// (Shift I), which picks out every one of them.
fn focus_sides(ui: &mut egui::Ui, explorer: &mut Explorer, now: f32) {
    let Some(focus) = explorer.graph_focus() else {
        return;
    };
    let calling = explorer.callers_of(focus).len();
    let mut called: Vec<usize> = explorer
        .graph()
        .callees(focus)
        .filter(|&t| t != focus)
        .collect();
    called.sort_unstable();
    called.dedup();
    let spotlight = explorer.spotlight();
    let picked = explorer.spotlit().len();
    let starts = explorer.starts(focus);
    let mut pressed = None;
    ui.horizontal(|ui| {
        if !starts.is_empty() {
            let all = spotlight.is_some_and(|s| s.side == Side::Callers && s.reach.is_none());
            let text = egui::RichText::new(format!("↑ {}", reached_from(&starts)));
            if ui
                .add(egui::Button::selectable(all, text))
                .on_hover_text(
                    "Every route into it, from the entry points down; pick one, or E, to \
                     follow it (Shift I)",
                )
                .clicked()
            {
                pressed = Some((Side::Callers, true));
            }
        }
        for (side, count, noun, key, color) in [
            (Side::Callers, calling, "caller", "I", CALLER),
            (Side::Callees, called.len(), "call", "O", CALLEE),
        ] {
            let plural = if count == 1 { "" } else { "s" };
            let lit = spotlight.filter(|s| s.side == side);
            // How far the spotlight reaches past the direct ones.
            let extent = match lit.map(|s| s.reach) {
                Some(None) if picked > count => format!(" · all {picked}"),
                Some(Some(reach)) if reach > 1 => format!(" · {picked} within {reach}"),
                _ => String::new(),
            };
            let text = egui::RichText::new(format!("● {count} {noun}{plural}{extent}  {key}"))
                .color(egui_color(color));
            let hint = match side {
                Side::Callers => {
                    "Pick out what calls it, then two calls up, then every route in, and bring \
                     it all into view (I; Shift I all the way)"
                }
                Side::Callees => {
                    "Pick out what it calls, then two calls down, then everything it runs, and \
                     bring it all into view (O; Shift O all the way)"
                }
            };
            if ui
                .add_enabled(count > 0, egui::Button::selectable(lit.is_some(), text))
                .on_hover_text(hint)
                .clicked()
            {
                pressed = Some((side, false));
            }
        }
    });
    match pressed {
        Some((side, true)) => explorer.spotlight_all(side, now),
        Some((side, false)) => explorer.widen_spotlight(side, now),
        None => {}
    }
}

/// `reached from 3 entries`, or from the functions nothing calls when no declared
/// entry reaches it.
fn reached_from(starts: &[Start]) -> String {
    let entries = starts.iter().filter(|s| s.declared).count();
    let uncalled = starts.len() - entries;
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match (entries, uncalled) {
        (0, 0) => "reached from nowhere".to_owned(),
        (0, _) => format!(
            "reached from {}",
            plural(
                uncalled,
                "function nothing calls",
                "functions nothing calls"
            )
        ),
        (_, 0) => format!("reached from {}", plural(entries, "entry", "entries")),
        _ => format!(
            "reached from {} and {} nothing calls",
            plural(entries, "entry", "entries"),
            plural(uncalled, "function", "functions")
        ),
    }
}

/// Where the routes into the focus start, while its callers are in the spotlight:
/// top left of the graph. A row opens that start's journey with its cursor on the
/// focus.
fn routes_card(ctx: &egui::Context, area: egui::Rect, explorer: &mut Explorer) {
    let (Some(spotlight), Some(focus)) = (explorer.spotlight(), explorer.graph_focus()) else {
        return;
    };
    if spotlight.side != Side::Callers {
        return;
    }
    let starts = explorer.starts(focus);
    if starts.is_empty() {
        return;
    }
    let root = explorer.root().and_then(|u| explorer.map().tile_of(u));
    let mut chosen = None;
    egui::Area::new("routes".into())
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(egui::pos2(area.min.x + 12.0, area.min.y + 10.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 225))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_max_width(320.0);
                    let title = reached_from(&starts);
                    let mut title = title.chars();
                    let title: String = title
                        .next()
                        .map(|c| c.to_uppercase().chain(title).collect())
                        .unwrap_or_default();
                    ui.label(egui::RichText::new(title).strong().size(14.0));
                    ui.label(
                        egui::RichText::new("pick one, or E, to follow its journey here")
                            .small()
                            .color(MUTED),
                    );
                    egui::ScrollArea::vertical()
                        .max_height(area.height() * 0.4)
                        .show(ui, |ui| {
                            for start in starts.iter() {
                                let shown = spotlight.holds(start.hops);
                                let calls = if start.hops == 1 { "call" } else { "calls" };
                                let mut job = egui::text::LayoutJob::default();
                                let format = |color, size| egui::TextFormat {
                                    font_id: egui::FontId::proportional(size),
                                    color,
                                    ..default()
                                };
                                let color = if shown {
                                    egui::Color32::WHITE
                                } else {
                                    egui::Color32::from_rgb(190, 196, 210)
                                };
                                job.append(&short(&start.label, 40), 0.0, format(color, 13.0));
                                job.append(
                                    &format!("{} {calls}", start.hops),
                                    8.0,
                                    format(MUTED, 11.5),
                                );
                                if ui
                                    .selectable_label(root == Some(start.tile), job)
                                    .on_hover_text(format!(
                                        "Open the journey from here, with the cursor on {}",
                                        node_name(explorer, focus)
                                    ))
                                    .clicked()
                                {
                                    chosen = Some(start.tile);
                                }
                            }
                        });
                });
        });
    if let Some(start) = chosen {
        explorer.follow_route(start);
    }
}

/// What the colors mean, in two lines: what a node's fill says, then which way a
/// link runs. A hint at the keys below.
fn legend(ctx: &egui::Context, area: egui::Rect, diffing: bool) {
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
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let swatch = |ui: &mut egui::Ui, mark: &str, color: Color, text: &str| {
                            ui.label(egui::RichText::new(mark).color(egui_color(color)));
                            ui.label(egui::RichText::new(text).size(12.5));
                            ui.add_space(6.0);
                        };
                        if diffing {
                            swatch(ui, "●", diff_view::ADDED, "added");
                            swatch(ui, "●", diff_view::MODIFIED, "rewritten");
                            swatch(ui, "●", diff_view::REMOVED, "removed");
                            swatch(ui, "●", diff_view::MOVED, "moved");
                            swatch(ui, "●", graph_view::UNCHANGED, "unchanged");
                        } else {
                            ui.label(
                                egui::RichText::new("node color = its module, as its ring")
                                    .size(12.5),
                            );
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (color, text) in [
                            (CALLER, "calls into the focus"),
                            (CALLEE, "calls out of it"),
                        ] {
                            ui.label(egui::RichText::new("→").color(egui_color(color)));
                            ui.label(egui::RichText::new(text).size(12.5));
                            ui.add_space(6.0);
                        }
                        ui.label(egui::RichText::new("→").color(egui_color(graph_view::FURTHER)));
                        ui.label(egui::RichText::new("further along").size(12.5));
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("journey bars match their nodes")
                                .size(12.5)
                                .color(MUTED),
                        );
                    });
                });
        });
    ctx.layer_painter(egui::LayerId::background())
        .with_clip_rect(area)
        .text(
            egui::pos2(area.min.x + 12.0, area.max.y - 10.0),
            egui::Align2::LEFT_BOTTOM,
            "click a node to open it · I callers · O calls · , . less or more of the journey \
             · / search · S panel · H keys",
            egui::FontId::proportional(12.0),
            MUTED,
        );
}

/// Every key, grouped by what it moves.
const KEYS: [(&str, &[(&str, &str)]); 4] = [
    (
        "Journey",
        &[
            ("← →", "previous / next call by the same caller"),
            ("↓", "into the first call this one makes"),
            ("↑", "out to its caller"),
            ("[  ]", "back / forward one call in time"),
            ("Shift ]", "step over this call's calls"),
            ("Shift ↑", "step out to what runs after its caller"),
            ("Home  End", "first / last call"),
            (",  .", "show a level less / more of it"),
            ("V", "show calls by level / as the cursor reaches them"),
            ("Space", "play"),
            ("Tab", "next open journey"),
            ("Enter", "trace a journey from the focus"),
            (
                "Shift Enter",
                "pull the cursor's call out as its own journey",
            ),
            ("X", "close the journey"),
        ],
    ),
    (
        "Graph",
        &[
            (
                "I",
                "what calls the focus, then 2 calls up, then all, then off",
            ),
            (
                "Shift I",
                "every route into the focus, from its entry points",
            ),
            (
                "E  Shift E",
                "follow the next / previous route into the focus",
            ),
            (
                "O",
                "what the focus calls, then 2 calls down, then all, then off",
            ),
            ("Shift O", "everything the focus can run"),
            ("1 – 9", "focus the focus's nth call"),
            ("Shift 1 – 9", "focus its nth caller"),
            ("Backspace", "back to the previous focus"),
            ("Esc", "focus the journey's cursor again"),
            ("- / =", "show less / more around the start"),
            ("Q", "show all → 2 steps → 1 step"),
            ("L", "reveal around: focus → changes → journey"),
            ("G", "nearness by: calls → space → blend"),
            ("C", "diff: only what changed / everything"),
            ("K", "cluster by modules / calls"),
            ("F", "back to the focus, head-on"),
            ("R", "frame the whole graph"),
            ("D", "diff: status → impact → morph"),
            ("T", "play or pause the morph"),
        ],
    ),
    (
        "Camera",
        &[
            ("two fingers", "orbit"),
            ("Shift two fingers", "pan"),
            ("pinch, Ctrl two fingers", "zoom towards the pointer"),
            ("wheel", "zoom towards the pointer"),
            ("left / middle drag", "orbit"),
            ("right / Shift drag", "pan"),
        ],
    ),
    (
        "Layout",
        &[
            ("J", "show or hide the journey"),
            ("M", "swap graph and journey"),
            ("S", "show or hide the side panel"),
            ("/", "search functions, errors, conditions, calls"),
            ("H", "this list"),
        ],
    ),
];

fn key_help(ctx: &egui::Context, area: egui::Rect, open: &mut bool) {
    egui::Area::new("key_help".into())
        .pivot(egui::Align2::CENTER_CENTER)
        .fixed_pos(area.center())
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 240))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(60)))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(18, 14))
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        for (title, keys) in KEYS {
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(title).strong().size(14.0));
                                ui.add_space(4.0);
                                egui::Grid::new(title).spacing(egui::vec2(12.0, 4.0)).show(
                                    ui,
                                    |ui| {
                                        for (key, what) in keys {
                                            ui.label(
                                                egui::RichText::new(*key)
                                                    .monospace()
                                                    .color(egui::Color32::WHITE),
                                            );
                                            ui.label(egui::RichText::new(*what).color(MUTED));
                                            ui.end_row();
                                        }
                                    },
                                );
                            });
                            ui.add_space(16.0);
                        }
                    });
                    if ui.small_button("close").clicked() {
                        *open = false;
                    }
                });
        });
}

/// The last two segments of a path: `orders::Order::total` is `Order.total`.
fn tail(path: &str) -> String {
    let segments: Vec<&str> = path.split("::").collect();
    segments[segments.len().saturating_sub(2)..].join(".")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame that places a label for `key` at `node` with `blocked` taken,
    /// and returns the side it took and where its text was drawn from the node.
    fn frame(ctx: &egui::Context, node: egui::Pos2, blocked: &[egui::Rect]) -> (usize, egui::Vec2) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx();
            let painter = ctx.layer_painter(egui::LayerId::background());
            let mut placer = Placer::new(&painter, blocked.to_vec());
            let job = label_job(None, "draw_graph", None, egui::Color32::WHITE, 12.5, false);
            assert!(placer.place(Named::Node(0), node, 5.0, job, None, false));
            placer.finish(|_| Some(node));
        });
        output.textures_delta.clear();
        let shown = ctx
            .data_mut(|d| d.get_temp::<ShownLabels>(Placer::id()))
            .and_then(|labels| labels.get(&Named::Node(0)).cloned())
            .unwrap_or_else(|| panic!("the label was not remembered"));
        (shown.side, shown.offset)
    }

    #[test]
    fn labels_hold_their_side_and_ease_to_a_new_one() {
        let ctx = egui::Context::default();
        let node = egui::pos2(400.0, 300.0);
        let right = egui::Rect::from_min_max(egui::pos2(405.0, 280.0), egui::pos2(600.0, 320.0));
        let left = egui::Rect::from_min_max(egui::pos2(200.0, 280.0), egui::pos2(395.0, 320.0));

        let (side, start) = frame(&ctx, node, &[right]);
        assert_eq!(side, 1, "the right is taken, so it goes left");
        let (side, offset) = frame(&ctx, node, &[]);
        assert_eq!(side, 1, "the right freeing up does not flip it back");
        assert_eq!(offset, start);

        let (side, offset) = frame(&ctx, node, &[left]);
        assert_eq!(side, 0, "covered, it moves");
        assert!(
            offset.x > start.x && offset.x < 0.0,
            "it slides over rather than jumping: {start:?} → {offset:?}"
        );
        let mut settled = offset;
        for _ in 0..60 {
            settled = frame(&ctx, node, &[left]).1;
        }
        assert!(settled.x > 0.0, "and gets there: {settled:?}");
    }
}
