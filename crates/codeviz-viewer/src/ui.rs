//! egui panels: function picker, trace settings, playback, inspector, and node labels.

use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use codeviz_graph::{Function, FunctionId, TraceNodeId};
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
    entry_points_only: bool,
}

impl Default for PickerState {
    fn default() -> Self {
        Self {
            query: String::new(),
            entry_points_only: true,
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
    let left = egui::Panel::left("functions")
        .resizable(true)
        .default_size(300.0)
        .show(&mut root_ui, |ui| {
            function_picker(ui, &mut explorer, &mut picker);
        })
        .response
        .rect
        .width();
    let right = egui::Panel::right("inspector")
        .resizable(true)
        .default_size(340.0)
        .show(&mut root_ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                trace_settings(ui, &mut explorer);
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
        let id = placed.trace_node;
        let Some(node) = explorer.node(id) else {
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

fn function_picker(ui: &mut egui::Ui, explorer: &mut Explorer, picker: &mut PickerState) {
    ui.heading("Functions");
    ui.add(egui::TextEdit::singleline(&mut picker.query).hint_text("filter…"));
    ui.checkbox(&mut picker.entry_points_only, "Entry points only")
        .on_hover_text("Functions nothing in the analyzed source calls");
    ui.separator();

    let graph = explorer.graph();
    let query = picker.query.to_lowercase();
    let mut matches: Vec<(FunctionId, &Function, String)> = graph
        .functions()
        .filter(|(id, _)| !picker.entry_points_only || graph.callers(*id).is_empty())
        .map(|(id, function)| (id, function, function.qualified_name()))
        .filter(|(_, _, name)| name.to_lowercase().contains(&query))
        .collect();
    matches.sort_by(|a, b| a.2.cmp(&b.2));

    let mut chosen = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, function, _) in &matches {
            let selected = explorer.root() == Some(*id);
            let response = ui
                .selectable_label(selected, function.short_name())
                .on_hover_text(format!(
                    "{}\n{}",
                    function.qualified_name(),
                    function.location
                ));
            ui.label(egui::RichText::new(function.module_path()).small().weak());
            if response.clicked() {
                chosen = Some(*id);
            }
        }
    });
    if let Some(id) = chosen {
        explorer.set_root(id);
    }
}

fn trace_settings(ui: &mut egui::Ui, explorer: &mut Explorer) {
    ui.heading("Trace");
    let mut changed = false;
    changed |= ui
        .checkbox(
            &mut explorer.trace_options.include_external,
            "Show external calls",
        )
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut explorer.trace_options.max_depth, 1..=64).text("max depth"))
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
    if let Some(trace) = explorer.trace() {
        ui.label(format!(
            "{} invocations · depth {} · {} lanes",
            trace.len(),
            trace.max_depth(),
            explorer.scene().lanes.len()
        ));
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
    let Some(node) = explorer.node(selected).cloned() else {
        return;
    };

    ui.label(
        egui::RichText::new(explorer.label(selected))
            .strong()
            .size(16.0),
    );
    ui.label(
        egui::RichText::new(NodeKind::of(&node).label())
            .color(egui_color(NodeKind::of(&node).color())),
    );
    if let Some(function) = explorer.function(selected) {
        ui.label(egui::RichText::new(function.qualified_name()).small());
        ui.label(egui::RichText::new(&function.signature).monospace());
        ui.label(format!("defined at {}", function.location));
    }
    if let Some(site) = &node.call_site {
        ui.label(format!("called from {site}"));
    }
    ui.label(format!(
        "ticks {}..{} · depth {}",
        node.enter, node.exit, node.depth
    ));
    if let codeviz_graph::TraceTarget::Ambiguous(ids) = &node.target {
        ui.label("Candidates:");
        for id in ids {
            if let Some(function) = explorer.graph().function(*id) {
                ui.label(egui::RichText::new(function.qualified_name()).monospace());
            }
        }
    }

    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                explorer.function_id(selected).is_some() && node.parent.is_some(),
                egui::Button::new("Trace from here"),
            )
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

    if !node.children.is_empty() {
        ui.label("Calls, in order:");
        let mut chosen: Option<TraceNodeId> = None;
        for child in &node.children {
            if ui.link(explorer.label(*child)).clicked() {
                chosen = Some(*child);
            }
        }
        if let Some(child) = chosen {
            explorer.selected = Some(child);
            if let Some(child_node) = explorer.node(child) {
                playback.tick = playback.tick.max(child_node.enter as f32);
            }
        }
    }

    if let Some(function_id) = explorer.function_id(selected) {
        let callers: Vec<String> = explorer
            .graph()
            .callers(function_id)
            .iter()
            .filter_map(|id| explorer.graph().function(*id))
            .map(Function::qualified_name)
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
