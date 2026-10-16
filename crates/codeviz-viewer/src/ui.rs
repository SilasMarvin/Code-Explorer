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
    explorer::{Autoplay, Clustering, Confidence, Explorer, SpanRef, Step, Track, is_conditional},
    graph_view::{self, Relation, relation},
    journey_panel::{self, JourneyPanel, PanelMode},
    lens::{self, Measure, Source},
    scene::{PALETTE, accent, lane_color},
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
        if let Some(diff) = diff.as_deref_mut() {
            diff_view::diff_card(ctx, graph_area, &mut explorer, diff);
        }
        focus_card(ctx, graph_area, &mut explorer);
        if panel.mode != PanelMode::Full {
            legend(
                ctx,
                above(graph_area, panel_area),
                &mut explorer,
                diff.is_some(),
            );
        }
        if help.0 {
            key_help(ctx, graph_area, &mut help.0);
        }
        if let Some(area) = panel_area {
            let bar = journey_panel::draw(ctx, area, &mut explorer, &mut panel, diff.as_deref());
            if let (Some(bar), Some(node)) = (bar, cursor_node(&explorer, &to_screen)) {
                tether(ctx, bar, node);
            }
        }
    }
    Ok(())
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
    // What the journey discloses, in call order, numbered as the flame graph and the
    // graph number it.
    let disclosed = explorer.disclosure();
    let mut rows: Vec<(SpanId, u32, String, bool, Option<usize>)> = Vec::new();
    let mut stack = vec![journey.root()];
    while let Some(id) = stack.pop() {
        let Some(span) = journey.span(id) else {
            continue;
        };
        let number = disclosed.number(id).unwrap_or_default();
        let label = if number.is_empty() {
            explorer.label(id)
        } else {
            format!("{number}  {}", explorer.label(id))
        };
        let folded = disclosed.folded(id);
        let label = if folded > 0 {
            format!("{label}  +{folded}")
        } else {
            label
        };
        rows.push((
            id,
            disclosed.level(id).unwrap_or_default(),
            label,
            is_conditional(span),
            Some(explorer.track().color(id)),
        ));
        stack.extend(
            disclosed
                .children(id)
                .iter()
                .rev()
                .filter(|&&c| disclosed.shows(c)),
        );
    }
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
                egui::Stroke::new(1.0, color.gamma_multiply(0.45)),
                egui::StrokeKind::Inside,
            );
        }
        painter.galley(min, shown.galley.clone(), egui::Color32::WHITE);
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
            _ if diff_tier(tile) == Some(u8::MAX) => continue,
            // What the open journey discloses is named first, with its numbers; with
            // a journey open, what it folds away and callers further up go unnamed.
            _ if journey.contains_key(&tile) => 1,
            Some(Relation::Downstream(_)) if opened => continue,
            Some(Relation::Upstream(hops)) if opened && hops > 1 => continue,
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
    let cards: Vec<egui::Rect> = ["focus_card", "diff_card", "legend"]
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
                let job = label_job(None, &label.to_uppercase(), None, color, 12.0);
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
                let lead = match (order, change) {
                    (Some(order), Some(change)) => {
                        Some((format!("{order} {}", diff_view::sign(change)), color))
                    }
                    (Some(order), None) => Some((order, color)),
                    (None, _) => lead,
                };
                let size = match tier {
                    0 => 14.0,
                    1 => 12.0,
                    _ => 11.0,
                };
                let stroke = (tier <= 1).then_some(color);
                placer.place(
                    named,
                    node,
                    edge,
                    label_job(lead, &name, aside, color, size),
                    stroke,
                    tier == 0,
                );
            }
        }
    }
    placer.finish(|named| match named {
        Named::Node(tile) => discs.get(tile).copied().flatten().map(|(node, _)| node),
        Named::Group(group) => ring_top(group),
    });
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
        explorer.open_at(tile);
    }
}

/// How the graph is clustered, and per container how many of its calls stay inside
/// it: a container whose functions mostly call each other holds together when only
/// calls pull, and one that leaks calls everywhere scatters.
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
                    reveal_controls(ui, explorer, diffing);
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
            "click a node to open its journey · shift-click opens another · , . less or \
             more of it · drag to orbit · scroll to zoom · H for keys",
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
            let job = label_job(None, "draw_graph", None, egui::Color32::WHITE, 12.5);
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
