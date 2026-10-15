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
    diff_view::{self, DiffStyle, DiffView},
    explorer::{
        Autoplay, Clustering, Confidence, Explorer, Quiet, SpanRef, Step, Track, is_conditional,
    },
    graph_view::{self, Relation, relation},
    journey_panel::{self, JourneyPanel, PanelMode},
    scene::{accent, lane_color},
    source_diff,
};

pub(crate) struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Search>()
            .init_resource::<KeyHelp>()
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
pub(crate) const MUTED: egui::Color32 = egui::Color32::from_rgb(140, 148, 165);

#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn draw_ui(
    mut contexts: EguiContexts,
    mut explorer: ResMut<Explorer>,
    mut autoplay: ResMut<Autoplay>,
    mut search: ResMut<Search>,
    mut sources: ResMut<Sources>,
    mut panel: ResMut<JourneyPanel>,
    mut help: ResMut<KeyHelp>,
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
                match diff.as_deref() {
                    Some(view) => changed_source(ui, &explorer, view, &mut sources),
                    None => source(ui, &explorer, &mut sources),
                }
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
        draw_graph_labels(
            &painter,
            &to_screen,
            &explorer,
            diff.as_deref(),
            transform.up().into(),
        );
        if let Some(diff) = diff.as_deref_mut() {
            diff_view::diff_card(ctx, graph_area, &mut explorer, diff);
        }
        focus_card(ctx, graph_area, &mut explorer);
        if panel.mode != PanelMode::Full {
            legend(ctx, graph_area, &mut explorer, diff.is_some());
        }
        if help.0 {
            key_help(ctx, graph_area, &mut help.0);
        }
        if let Some(area) = panel_area {
            let bar = journey_panel::draw(ctx, area, &mut explorer, &mut panel, diff.as_deref());
            // From the cursor's bar up to its node.
            let node = explorer
                .focus()
                .and_then(|unit| explorer.map().tile_of(unit))
                .and_then(|tile| explorer.tile_point(tile))
                .and_then(to_screen);
            if let (Some(bar), Some(node)) = (bar, node) {
                tether(ctx, bar, node);
            }
        }
    }
    Ok(())
}

/// A line from the cursor's bar in the journey up to its node in the graph.
fn tether(ctx: &egui::Context, bar: egui::Rect, node: egui::Pos2) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("tether"),
    ));
    let from = egui::pos2(bar.min.x + 2.0, bar.min.y);
    painter.line_segment(
        [from, node],
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(90)),
    );
    painter.circle_filled(from, 2.5, egui::Color32::WHITE);
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
                    .any_unit(unit)
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

/// Where a label may go around its node, in order of preference.
const SIDES: [egui::Align2; 4] = [
    egui::Align2::LEFT_CENTER,
    egui::Align2::RIGHT_CENTER,
    egui::Align2::CENTER_BOTTOM,
    egui::Align2::CENTER_TOP,
];

/// Labels already placed and nodes they must not cover.
struct Placer<'a> {
    painter: &'a egui::Painter,
    taken: Vec<egui::Rect>,
}

impl Placer<'_> {
    /// Paints `job` on a pill on the first side of `node` (a disc of `edge` points)
    /// where it overlaps nothing already placed, or the first side anyway when
    /// `force` is set. Returns whether it was placed.
    fn place(
        &mut self,
        node: egui::Pos2,
        edge: f32,
        job: egui::text::LayoutJob,
        stroke: Option<egui::Color32>,
        force: bool,
    ) -> bool {
        let galley = self.painter.layout_job(job);
        let size = galley.size();
        let gap = edge + 6.0;
        let ppp = self.painter.pixels_per_point();
        let spot = |side: egui::Align2| {
            let anchor = match side {
                egui::Align2::LEFT_CENTER => node + egui::vec2(gap, 0.0),
                egui::Align2::RIGHT_CENTER => node - egui::vec2(gap, 0.0),
                egui::Align2::CENTER_BOTTOM => node - egui::vec2(0.0, gap),
                _ => node + egui::vec2(0.0, gap),
            };
            let rect = side.anchor_size(anchor, size);
            egui::Rect::from_min_size(
                egui::emath::GuiRounding::round_to_pixels(rect.min, ppp),
                size,
            )
        };
        let pad = egui::vec2(5.0, 2.0);
        // Inside the graph, so no label is cut off at its edge.
        let bounds = self.painter.clip_rect();
        let free = SIDES.iter().map(|&side| spot(side)).find(|rect| {
            bounds.contains_rect(rect.expand2(pad))
                && !self.taken.iter().any(|r| r.intersects(rect.expand2(pad)))
        });
        let Some(rect) = free.or_else(|| force.then(|| spot(SIDES[0]))) else {
            return false;
        };
        let pill = rect.expand2(pad);
        self.taken.push(pill);
        self.painter.rect_filled(pill, 4.0, PILL);
        if let Some(color) = stroke {
            self.painter.rect_stroke(
                pill,
                4.0,
                egui::Stroke::new(1.0, color.gamma_multiply(0.45)),
                egui::StrokeKind::Inside,
            );
        }
        self.painter.galley(rect.min, galley, egui::Color32::WHITE);
        true
    }
}

/// A label in two parts: the name, then a quieter note.
fn label_job(
    lead: Option<(String, egui::Color32)>,
    name: &str,
    note: Option<String>,
    color: egui::Color32,
    size: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let format = |color, size| egui::TextFormat {
        font_id: egui::FontId::proportional(size),
        color,
        ..default()
    };
    if let Some((lead, lead_color)) = lead {
        job.append(&lead, 0.0, format(lead_color, size));
        job.append(" ", 0.0, format(color, size));
    }
    job.append(name, 0.0, format(color, size));
    if let Some(note) = note {
        job.append(&note, 6.0, format(MUTED, size - 1.5));
    }
    job
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
            let world = explorer.tile_point(tile).filter(|_| explorer.shown(tile))?;
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
            _ if diff_tier(tile) == Some(u8::MAX) => continue,
            _ if diff_tier(tile).is_some() => diff_tier(tile).unwrap_or(1),
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
        // A container counts only the functions showing.
        if diff.is_some_and(|d| d.presence(tile) < 0.3) {
            continue;
        }
        if let (Some(group), Some(at)) = (graph.group(tile), explorer.tile_point(tile))
            && let Some(slot) = centers.get_mut(group)
        {
            slot.0 += at;
            slot.1 += 1;
        }
    }
    if explorer.quiet().radius().is_none() {
        for (group, &(_, count)) in centers.iter().enumerate() {
            if count >= 2 {
                order.push(((7, std::cmp::Reverse(count), group), Named::Group(group)));
            }
        }
    }
    order.sort_by_key(|&(key, _)| key);

    // Labels keep off the cards over the graph, as last laid out.
    let cards: Vec<egui::Rect> = ["focus_card", "diff_card", "legend"]
        .into_iter()
        .filter_map(|id| painter.ctx().memory(|m| m.area_rect(egui::Id::new(id))))
        .collect();
    // And off the nodes that matter, so they never hide a neighbor.
    let mut placer = Placer {
        painter,
        taken: cards
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
    };
    let focus = explorer.graph_focus();
    for ((tier, _, _), named) in order {
        match named {
            Named::Group(group) => {
                let (sum, count) = centers[group];
                let Some(at) = to_screen(sum / count as f32) else {
                    continue;
                };
                let color = explorer
                    .group_color(group)
                    .map_or(MUTED, |c| egui_color(lane_color(c)))
                    .gamma_multiply(if center.is_some() { 0.6 } else { 0.9 });
                let label = map.groups.get(group).map_or("", |g| g.label.as_str());
                let job = label_job(None, &label.to_uppercase(), None, color, 11.0);
                placer.place(at, 0.0, job, None, false);
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
                    (_, Some(Relation::Upstream(_))) => CALLER_TEXT.gamma_multiply(0.7),
                    (_, Some(Relation::Downstream(_))) => CALLEE_TEXT.gamma_multiply(0.7),
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
                        change.map(|c| (diff_view::sign(c).to_owned(), color)),
                        diff.and_then(|d| d.moved_from(tile))
                            .map(|was| format!("was {}", tail(was))),
                    ),
                    _ if impact_note.is_some() => (None, impact_note),
                    (Some(Relation::Downstream(1)), Some(focus)) => {
                        let steps = call_steps(explorer, focus, tile);
                        (Some((numbered(&steps), color)), None)
                    }
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
                let size = match tier {
                    0 => 16.0,
                    1 => 14.0,
                    _ => 12.5,
                };
                let stroke = (tier <= 1).then_some(color);
                placer.place(
                    node,
                    edge,
                    label_job(lead, &name, aside, color, size),
                    stroke,
                    tier == 0,
                );
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
    let callers = explorer.callers_of(focus);
    let graph = explorer.graph();
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
                        ui.label(
                            egui::RichText::new("CALLED BY  (shift + number)")
                                .small()
                                .color(CALLER_TEXT),
                        );
                        for (index, &caller) in callers.iter().enumerate() {
                            let steps = call_steps(explorer, caller, focus);
                            let total = explorer.graph().sequence(caller).len();
                            let key = if index < 9 {
                                format!("{}", index + 1)
                            } else {
                                "  ".to_owned()
                            };
                            let text = format!(
                                "{key}  {}  · calls it {} of {total}",
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
                            egui::RichText::new("CALLS, IN ORDER  (number)")
                                .small()
                                .color(CALLEE_TEXT),
                        );
                        for (index, &callee) in calls.iter().enumerate().take(14) {
                            let text =
                                format!("{:>2}   {}", index + 1, node_name(explorer, callee));
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
#[expect(clippy::too_many_lines, reason = "one panel, top to bottom")]
fn legend(ctx: &egui::Context, area: egui::Rect, explorer: &mut Explorer, diffing: bool) {
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
                    if diffing {
                        diff_key(ui);
                    } else {
                        color_key(ui, explorer.active());
                    }
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Show").small().color(MUTED));
                        let mut quiet = explorer.quiet();
                        ui.selectable_value(&mut quiet, Quiet::Off, "All")
                            .on_hover_text("Every function (Q)");
                        ui.selectable_value(&mut quiet, Quiet::Near, "2 calls")
                            .on_hover_text("Only what is within two calls of the focus (Q)");
                        ui.selectable_value(&mut quiet, Quiet::Direct, "1 call")
                            .on_hover_text("Only what the focus calls and what calls it (Q)");
                        if quiet != explorer.quiet() {
                            explorer.set_quiet(quiet);
                        }
                    });
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
            "click a node to focus · shift-click opens its journey · drag to orbit · \
             scroll or pinch to zoom · H for keys",
            egui::FontId::proportional(12.0),
            MUTED,
        );
}

/// What each color in the graph means.
fn color_key(ui: &mut egui::Ui, active: usize) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let swatch = |ui: &mut egui::Ui, color: egui::Color32, text: &str| {
            ui.label(egui::RichText::new("■").color(color));
            ui.label(egui::RichText::new(text).small());
            ui.add_space(6.0);
        };
        swatch(ui, egui::Color32::WHITE, "focus");
        swatch(ui, CALLER_TEXT, "calls in");
        swatch(ui, CALLEE_TEXT, "calls out");
        swatch(ui, egui_color(accent(active)), "journey");
    });
    ui.label(
        egui::RichText::new("bright = one call away, dimmer = further")
            .small()
            .color(MUTED),
    );
}

/// What each color means under a diff.
fn diff_key(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let swatch = |ui: &mut egui::Ui, color: Color, text: &str| {
            ui.label(egui::RichText::new("■").color(egui_color(color)));
            ui.label(egui::RichText::new(text).small());
            ui.add_space(6.0);
        };
        swatch(ui, diff_view::ADDED, "added");
        swatch(ui, diff_view::MODIFIED, "rewritten");
        swatch(ui, diff_view::REMOVED, "removed");
        swatch(ui, diff_view::MOVED, "moved");
        swatch(ui, Color::WHITE, "focus");
    });
    ui.label(
        egui::RichText::new("dashed = call removed · dim gold (Impact) = reaches a change")
            .small()
            .color(MUTED),
    );
}

/// Every key, grouped by what it moves.
const KEYS: [(&str, &[(&str, &str)]); 3] = [
    (
        "Journey",
        &[
            ("← →", "previous / next call by the same caller"),
            ("↓", "into the first call this one makes"),
            ("↑", "out to its caller"),
            ("[  ]", "back / forward one call in time"),
            ("Home  End", "first / last call"),
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
            ("1 – 9", "focus the focus's nth call"),
            ("Shift 1 – 9", "focus its nth caller"),
            ("Backspace", "back to the previous focus"),
            ("Esc", "focus the journey's cursor again"),
            ("Q", "quiet: all → within 2 calls → within 1"),
            ("K", "cluster by modules / calls"),
            ("F", "face the focus head-on"),
            ("R", "frame the whole graph"),
            ("D", "diff: status → impact → morph"),
            ("T", "play or pause the morph"),
        ],
    ),
    (
        "Layout",
        &[
            ("J", "show or hide the journey"),
            ("M", "swap graph and journey"),
            ("H  /", "this list"),
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
