//! egui panels: entry picker, journey settings, scenarios, playback, inspector, and labels.

use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use codeviz_core::{SpanId, SpanTarget};
use codeviz_ir::{ExitKind, Role, UnitId};
use codeviz_layout::LaneKey;

use crate::{
    camera::OrbitCamera,
    explorer::{Explorer, NodeKind, Playback},
};

pub(crate) struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PickerState>()
            .add_systems(EguiPrimaryContextPass, draw_ui);
    }
}

#[derive(Resource, Debug)]
struct PickerState {
    query: String,
    entries_only: bool,
}

impl Default for PickerState {
    fn default() -> Self {
        Self {
            query: String::new(),
            entries_only: true,
        }
    }
}

/// Above this many invocations, only the root, selection and hover are labeled.
const LABEL_ALL_LIMIT: usize = 250;

fn draw_ui(
    mut contexts: EguiContexts,
    mut explorer: ResMut<Explorer>,
    mut playback: ResMut<Playback>,
    mut picker: ResMut<PickerState>,
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
    let left = egui::Panel::left("entries")
        .resizable(true)
        .default_size(300.0)
        .show(&mut root_ui, |ui| {
            entry_picker(ui, &mut explorer, &mut picker);
        })
        .response
        .rect
        .width();
    let right = egui::Panel::right("inspector")
        .resizable(true)
        .default_size(340.0)
        .show(&mut root_ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                journey_settings(ui, &mut explorer);
                ui.separator();
                scenario_picker(ui, &mut explorer);
                ui.separator();
                playback_controls(ui, &explorer, &mut playback);
                ui.separator();
                inspector(ui, &mut explorer, &mut playback);
                ui.separator();
                legend(ui);
            });
        })
        .response
        .rect
        .width();

    let screen = ctx.viewport_rect();
    let central = egui::Rect::from_min_max(
        egui::pos2(screen.min.x + left, screen.min.y),
        egui::pos2(screen.max.x - right, screen.max.y),
    );
    if let Ok((mut camera, transform)) = cameras.single_mut() {
        fit_viewport(&mut camera, &window, central);
        draw_labels(ctx, central, &explorer, &playback, &camera, transform);
    }
    Ok(())
}

/// Restricts 3D rendering to the area between the panels so framing and picking match
/// what is visible.
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

fn draw_labels(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &Explorer,
    playback: &Playback,
    camera: &Camera,
    transform: &GlobalTransform,
) {
    let painter = ctx
        .layer_painter(egui::LayerId::background())
        .with_clip_rect(area);
    // Bevy reports viewport positions in window coordinates, which egui shares.
    let to_screen = |position: Vec3| {
        camera
            .world_to_viewport(transform, position)
            .ok()
            .map(|p| egui::pos2(p.x, p.y))
    };
    let scene = explorer.scene();
    let label_all = scene.nodes.len() <= LABEL_ALL_LIMIT;
    for placed in &scene.nodes {
        let id = placed.span;
        let Some(node) = explorer.span(id) else {
            continue;
        };
        let emphasized = explorer.selected == Some(id) || explorer.hovered == Some(id);
        if !(label_all || emphasized || node.parent.is_none()) {
            continue;
        }
        let Some(screen) = to_screen(Vec3::from(placed.position) + Vec3::Y * 0.5) else {
            continue;
        };
        let alpha = if playback.has_entered(node) { 230 } else { 50 };
        let color = if emphasized {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_white_alpha(alpha)
        };
        painter.text(
            screen,
            egui::Align2::CENTER_BOTTOM,
            explorer.label(id),
            egui::FontId::proportional(if emphasized { 15.0 } else { 12.0 }),
            color,
        );
    }

    let (min, max) = scene.bounds;
    for lane in &scene.lanes {
        let anchor = Vec3::new(min[0] - 1.5, max[1] + 1.0, lane.z);
        if let Some(screen) = to_screen(anchor) {
            painter.text(
                screen,
                egui::Align2::RIGHT_CENTER,
                &lane.label,
                egui::FontId::monospace(11.0),
                egui::Color32::from_rgb(150, 165, 190),
            );
        }
    }
}

fn entry_picker(ui: &mut egui::Ui, explorer: &mut Explorer, picker: &mut PickerState) {
    ui.heading("Entries");
    ui.add(egui::TextEdit::singleline(&mut picker.query).hint_text("filter…"));
    ui.checkbox(&mut picker.entries_only, "Entry points only")
        .on_hover_text("Where execution starts: routes, main, triggers, consumers");
    ui.separator();

    let system = explorer.system();
    let query = picker.query.to_lowercase();
    let mut rows: Vec<(UnitId, String, String)> = if picker.entries_only {
        system
            .entries()
            .iter()
            .map(|e| {
                (
                    e.unit.clone(),
                    e.label.clone(),
                    format!("{} · {}", e.kind, e.unit),
                )
            })
            .collect()
    } else {
        system
            .units()
            .filter(|u| u.role == Role::Invocable)
            .map(|u| (u.id.clone(), explorer.unit_name(&u.id), u.id.to_string()))
            .collect()
    };
    rows.retain(|(_, title, detail)| {
        title.to_lowercase().contains(&query) || detail.to_lowercase().contains(&query)
    });
    rows.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.1.cmp(&b.1)));

    let mut chosen = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, title, detail) in &rows {
            let selected = explorer.root() == Some(id);
            if ui.selectable_label(selected, title).clicked() {
                chosen = Some(id.clone());
            }
            ui.label(egui::RichText::new(detail).small().weak());
        }
    });
    if let Some(id) = chosen {
        explorer.set_root(id);
    }
}

fn journey_settings(ui: &mut egui::Ui, explorer: &mut Explorer) {
    ui.heading("Journey");
    let mut changed = false;
    changed |= ui
        .checkbox(
            &mut explorer.options.include_unresolved,
            "Show unresolved calls",
        )
        .changed();
    changed |= ui
        .checkbox(
            &mut explorer.options.follow_registrations,
            "Follow handoffs",
        )
        .on_hover_text("Expand registered routes, callbacks and subscriptions inline")
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut explorer.options.max_depth, 1..=64).text("max depth"))
        .changed();
    egui::ComboBox::from_label("Z lanes")
        .selected_text(explorer.layout.lane_key.label())
        .show_ui(ui, |ui| {
            for key in LaneKey::ALL {
                changed |= ui
                    .selectable_value(&mut explorer.layout.lane_key, key, key.label())
                    .changed();
            }
        });
    if changed {
        explorer.rebuild();
    }
    if let Some(journey) = explorer.journey() {
        ui.label(format!(
            "{} spans · {} ticks · depth {} · {} lanes",
            journey.len(),
            journey.duration(),
            journey.max_depth(),
            explorer.scene().lanes.len()
        ));
    }
}

fn scenario_picker(ui: &mut egui::Ui, explorer: &mut Explorer) {
    ui.heading("Scenarios");
    let mut chosen = None;
    let current = explorer.scenario();
    if ui
        .selectable_label(current.is_none(), "All paths")
        .clicked()
    {
        chosen = Some(None);
    }
    for (index, scenario) in explorer.scenarios().iter().enumerate() {
        let (mark, color) = match scenario.outcome.kind {
            ExitKind::Return => ("✔", egui::Color32::from_rgb(120, 210, 140)),
            ExitKind::Panic => ("‼", egui::Color32::from_rgb(240, 90, 90)),
            ExitKind::Error | ExitKind::Unknown => ("✗", egui::Color32::from_rgb(240, 120, 110)),
        };
        let text = egui::RichText::new(format!("{mark} {}", scenario.outcome.label)).color(color);
        let response = ui.selectable_label(current == Some(index), text);
        let via: Vec<String> = scenario
            .chain
            .iter()
            .skip(1)
            .map(|site| explorer.unit_name(&site.unit))
            .collect();
        let response = if via.is_empty() {
            response
        } else {
            response.on_hover_text(format!("via {}", via.join(" → ")))
        };
        if response.clicked() {
            chosen = Some(Some(index));
        }
    }
    if let Some(scenario) = chosen {
        explorer.set_scenario(scenario);
    }
}

fn playback_controls(ui: &mut egui::Ui, explorer: &Explorer, playback: &mut Playback) {
    let duration = explorer.duration();
    ui.heading("Time");
    ui.horizontal(|ui| {
        let label = if playback.playing {
            "⏸ Pause"
        } else {
            "▶ Play"
        };
        if ui.button(label).clicked() {
            playback.toggle(duration);
        }
        if ui.button("⏮").clicked() {
            playback.tick = 0.0;
        }
        if ui.button("⏭").clicked() {
            playback.tick = duration;
            playback.playing = false;
        }
    });
    let mut tick = playback.tick.min(duration);
    if ui
        .add(egui::Slider::new(&mut tick, 0.0..=duration).text("tick"))
        .changed()
    {
        playback.tick = tick;
        playback.playing = false;
    }
    ui.add(
        egui::Slider::new(&mut playback.ticks_per_second, 0.25..=20.0)
            .logarithmic(true)
            .text("ticks/s"),
    );
}

fn inspector(ui: &mut egui::Ui, explorer: &mut Explorer, playback: &mut Playback) {
    ui.heading("Selection");
    let Some(selected) = explorer.selected else {
        ui.label("Click a node.");
        return;
    };
    let Some(span) = explorer.span(selected).cloned() else {
        return;
    };

    let kind = NodeKind::of(&span);
    ui.label(
        egui::RichText::new(explorer.label(selected))
            .strong()
            .size(16.0),
    );
    ui.label(egui::RichText::new(kind.label()).color(egui_color(kind.color())));
    if let Some(unit) = explorer.unit(selected) {
        unit_details(ui, unit);
    }
    span_context(ui, explorer, &span);

    ui.horizontal(|ui| {
        let can_reroot = explorer.unit_id(selected).is_some() && span.parent.is_some();
        if ui
            .add_enabled(can_reroot, egui::Button::new("Trace from here"))
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
    });

    if !span.children.is_empty() {
        ui.label("Calls, in order:");
        let mut chosen: Option<SpanId> = None;
        for child in &span.children {
            if ui.link(explorer.label(*child)).clicked() {
                chosen = Some(*child);
            }
        }
        if let Some(child) = chosen {
            explorer.selected = Some(child);
            if let Some(child_span) = explorer.span(child) {
                playback.tick = playback.tick.max(child_span.enter as f32);
            }
        }
    }

    if let Some(unit) = explorer.unit_id(selected) {
        let callers: Vec<String> = explorer
            .system()
            .callers(unit)
            .iter()
            .map(|id| explorer.unit_name(id))
            .collect();
        if !callers.is_empty() {
            ui.collapsing(format!("Called by ({})", callers.len()), |ui| {
                for caller in callers {
                    ui.label(egui::RichText::new(caller).monospace().small());
                }
            });
        }
    }
}

fn unit_details(ui: &mut egui::Ui, unit: &codeviz_ir::Unit) {
    ui.label(egui::RichText::new(unit.id.as_str()).small());
    ui.label(format!("{} · {:?}", unit.kind, unit.role).to_lowercase());
    if let Some(doc) = &unit.doc {
        ui.label(egui::RichText::new(doc).italics());
    }
    for (key, value) in &unit.attributes {
        if key == "signature" {
            ui.label(egui::RichText::new(value.to_string()).monospace());
        } else {
            ui.label(format!("{key}: {value}"));
        }
    }
    if let Some(location) = &unit.location {
        ui.label(format!("defined at {location}"));
    }
}

fn span_context(ui: &mut egui::Ui, explorer: &Explorer, span: &codeviz_core::Span) {
    if let Some((caller, key)) = &span.call {
        let site = explorer
            .system()
            .unit(caller)
            .and_then(|u| u.call(key))
            .and_then(|call| call.location.as_ref());
        match site {
            Some(location) => ui.label(format!("called as `{key}` at {location}")),
            None => ui.label(format!("called as `{key}`")),
        };
    }
    for condition in &span.conditions {
        ui.label(format!(
            "when {} ⇒ {}",
            condition.guard.as_deref().unwrap_or("?"),
            condition.arm
        ));
    }
    if span.repeats {
        ui.label("inside a loop");
    }
    for exit in &span.exits {
        ui.label(
            egui::RichText::new(format!(
                "{:?}: {}",
                exit.kind,
                exit.label.as_deref().unwrap_or("")
            ))
            .color(egui::Color32::from_rgb(240, 120, 110)),
        );
    }
    ui.label(format!(
        "ticks {}..{} · depth {}",
        span.enter, span.exit, span.depth
    ));
    if let SpanTarget::Ambiguous(ids) = &span.target {
        ui.label("Candidates:");
        for id in ids {
            ui.label(egui::RichText::new(id.as_str()).monospace());
        }
    }
}

fn legend(ui: &mut egui::Ui) {
    ui.collapsing("Legend & controls", |ui| {
        for kind in NodeKind::ALL {
            ui.label(
                egui::RichText::new(format!("● {}", kind.label())).color(egui_color(kind.color())),
            );
        }
        ui.separator();
        ui.label("X: time · Y: stack depth · Z: lane");
        ui.label("Left-drag orbit · right-drag pan · scroll zoom");
        ui.label("←/→ prev/next call in time · ↑ caller · ↓ first callee");
        ui.label("F focus · Space play · Enter trace from selection · Backspace back");
    });
}

fn egui_color(color: Color) -> egui::Color32 {
    let [r, g, b, a] = color.to_srgba().to_u8_array();
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}
