//! egui panels around the journey canvas: call stack on top, units / outcomes / outline
//! on the left, source and details on the right, stepping controls at the bottom.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use codeviz_core::{Level, SpanId, SpanTarget};
use codeviz_ir::{ExitKind, Role, UnitId};

use crate::{
    camera::{CameraMode, JourneyCamera},
    explorer::{Autoplay, Confidence, Explorer, Step, is_conditional},
    scene::lane_color,
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
/// Above this many spans, only the cursor's call stack is labeled.
const LABEL_ALL_LIMIT: usize = 300;

fn draw_ui(
    mut contexts: EguiContexts,
    mut explorer: ResMut<Explorer>,
    mut autoplay: ResMut<Autoplay>,
    mut search: ResMut<Search>,
    mut sources: ResMut<Sources>,
    mut cameras: Query<(&mut Camera, &GlobalTransform, &mut JourneyCamera)>,
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

    let mut toggle_mode = false;
    let mode = cameras
        .single()
        .map_or(CameraMode::Flat, |(_, _, c)| c.mode);
    let top = egui::Panel::top("stack")
        .show(&mut root_ui, |ui| {
            breadcrumb(ui, &mut explorer, mode, &mut toggle_mode);
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
    if let Ok((mut camera, transform, mut journey_camera)) = cameras.single_mut() {
        if toggle_mode {
            journey_camera.toggle();
            explorer.rebuild();
        }
        fit_viewport(&mut camera, &window, central);
        draw_labels(ctx, central, &explorer, &camera, transform);
    }
    Ok(())
}

fn breadcrumb(ui: &mut egui::Ui, explorer: &mut Explorer, mode: CameraMode, toggle: &mut bool) {
    ui.horizontal(|ui| {
        let flat = mode == CameraMode::Flat;
        if ui.selectable_label(flat, "Flat").clicked() && !flat {
            *toggle = true;
        }
        if ui.selectable_label(!flat, "3D").clicked() && flat {
            *toggle = true;
        }
        ui.separator();
        let Some(selected) = explorer.selected else {
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
            explorer.selected = Some(frame);
        }
    });
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
        let mut position = explorer.selected.map_or(0, SpanId::index);
        if total > 1
            && ui
                .add(egui::Slider::new(&mut position, 0..=total - 1).text(format!("of {total}")))
                .changed()
        {
            explorer.selected = explorer
                .journey()
                .and_then(|j| j.spans().nth(position))
                .map(|(id, _)| id);
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
            let lane = explorer.scene().nodes.get(id.index()).map(|n| n.lane);
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
                let selected = explorer.selected == Some(id);
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
        explorer.selected = Some(id);
    }
}

/// The caller's source around the line that made the selected call; for the root,
/// its own definition.
fn source(ui: &mut egui::Ui, explorer: &Explorer, sources: &mut Sources) {
    let Some(selected) = explorer.selected else {
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
    let Some(selected) = explorer.selected else {
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

/// Names on bars, exit labels on the cursor, and where outcomes split off.
fn draw_labels(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &Explorer,
    camera: &Camera,
    transform: &GlobalTransform,
) {
    let painter = ctx
        .layer_painter(egui::LayerId::background())
        .with_clip_rect(area);
    let to_screen = |position: Vec3| {
        camera
            .world_to_viewport(transform, position)
            .ok()
            .map(|p| egui::pos2(p.x, p.y))
    };
    let scene = explorer.scene();
    let tick = explorer.layout.tick_spacing;
    let label_all = scene.nodes.len() <= LABEL_ALL_LIMIT;
    let cursor_stack = explorer
        .selected
        .map(|s| explorer.stack(s))
        .unwrap_or_default();
    for placed in &scene.nodes {
        let id = placed.span;
        let Some(span) = explorer.span(id) else {
            continue;
        };
        let emphasized = explorer.selected == Some(id) || explorer.hovered == Some(id);
        if !(label_all || emphasized || cursor_stack.contains(&id)) {
            continue;
        }
        let [x, y, z] = placed.position;
        let Some(anchor) = to_screen(Vec3::new(x + 0.1, y + 0.55, z)) else {
            continue;
        };
        let mut text = explorer.label(id);
        if let Some(condition) = span.conditions.last() {
            text = format!("{text}  if {}", condition.arm);
        }
        // Fit the label over its own bar unless it is the one being looked at.
        let bar_end = to_screen(Vec3::new(x + placed.length - 0.2, y, z));
        if !emphasized && let Some(end) = bar_end {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a small, clamped character count"
            )]
            let room = ((end.x - anchor.x) / 6.5).clamp(3.0, 200.0) as usize;
            text = short(&text, room);
        }
        let color = if emphasized {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_rgb(210, 215, 228)
        };
        let size = if emphasized { 14.0 } else { 12.0 };
        painter.text(
            anchor,
            egui::Align2::LEFT_BOTTOM,
            text,
            egui::FontId::proportional(size),
            color,
        );
        if emphasized {
            for exit in &span.exits {
                let point = Vec3::new(exit.tick as f32 * tick, y - 0.6, z);
                if let Some(at) = to_screen(point) {
                    let color = if exit.kind == ExitKind::Return {
                        OK
                    } else {
                        ERROR
                    };
                    painter.text(
                        at,
                        egui::Align2::CENTER_TOP,
                        exit.label.as_deref().unwrap_or("returns"),
                        egui::FontId::proportional(11.0),
                        color,
                    );
                }
            }
        }
    }
    // Only for the bar under the pointer or cursor; the red dot alone marks the rest.
    for divergence in explorer.divergences() {
        if explorer.hovered != Some(divergence.span) && explorer.selected != Some(divergence.span) {
            continue;
        }
        let Some(placed) = scene.nodes.get(divergence.span.index()) else {
            continue;
        };
        let [x, y, z] = placed.position;
        if let Some(at) = to_screen(Vec3::new(x + placed.length - 0.2, y + 1.0, z)) {
            painter.text(
                at,
                egui::Align2::RIGHT_BOTTOM,
                format!("× {}", short(&divergence.label, 48)),
                egui::FontId::proportional(11.0),
                ERROR,
            );
        }
    }
}

fn short(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

fn egui_color(color: Color) -> egui::Color32 {
    let [r, g, b, a] = color.to_srgba().to_u8_array();
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}
