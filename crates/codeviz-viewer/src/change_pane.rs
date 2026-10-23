//! The focused change beside the graph, in a pane of its own so it never covers the
//! focus: where it comes in the reading order, what it affects, the calls it gained
//! and lost, and its source before and after with only what is near a change; below,
//! every change in the order to read them.

use std::fmt::Write as _;

use bevy::prelude::*;
use bevy_egui::egui;

use crate::{
    diff_view::{self, ADDED, DiffView, REMOVED},
    explorer::Explorer,
    source_diff,
    ui::{MUTED, Sources, egui_color},
};

/// Whether the pane was closed; N opens it again.
#[derive(Resource, Debug, Default)]
pub(crate) struct Closed(pub(crate) bool);

/// How wide the pane opens: a third of the window, within reason.
fn default_width(window: f32) -> f32 {
    (window * 0.34).clamp(360.0, 620.0)
}

/// The pane down the left of the window, resizable. Returns how wide it is.
pub(crate) fn panel(
    root_ui: &mut egui::Ui,
    explorer: &mut Explorer,
    view: &DiffView,
    sources: &mut Sources,
    closed: &mut bool,
) -> f32 {
    let width = default_width(root_ui.ctx().viewport_rect().width());
    egui::Panel::left("change_pane")
        .resizable(true)
        .default_size(width)
        .min_size(300.0)
        .show(root_ui, |ui| show(ui, explorer, view, sources, closed))
        .response
        .rect
        .width()
}

/// Height kept for the list of changes below the source.
const LIST: f32 = 220.0;
/// Entry points a change affects named in full; the rest are counted.
const NAMED: usize = 4;

/// The pane's content. Sets `closed` when its close button is pressed; a change
/// picked from the list opens.
fn show(
    ui: &mut egui::Ui,
    explorer: &mut Explorer,
    view: &DiffView,
    sources: &mut Sources,
    closed: &mut bool,
) {
    focused(ui, explorer, view, sources, closed);
    ui.separator();
    let height = ui.available_height();
    if let Some(tile) = diff_view::reading_list(ui, explorer, view, "pane", height) {
        explorer.open_at(tile);
    }
}

/// The focused change, or the unchanged function focused, with its source.
fn focused(
    ui: &mut egui::Ui,
    explorer: &Explorer,
    view: &DiffView,
    sources: &mut Sources,
    closed: &mut bool,
) {
    let focus = explorer.graph_focus();
    let place = focus.and_then(|f| view.place(f));
    ui.horizontal(|ui| {
        let at = place.map_or_else(
            || format!("{} changes", view.order().len()),
            |at| format!("{} of {}", at + 1, view.order().len()),
        );
        ui.label(egui::RichText::new(at).strong());
        ui.label(
            egui::RichText::new(format!("{} unread", view.unread()))
                .small()
                .color(MUTED),
        );
        ui.label(
            egui::RichText::new("N next · Shift N back")
                .small()
                .color(MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("×")
                .on_hover_text("Close; N opens it again")
                .clicked()
            {
                *closed = true;
            }
        });
    });
    let Some((tile, unit)) = focus.and_then(|t| Some((t, explorer.tile_unit(t)?.clone()))) else {
        ui.label(egui::RichText::new("Press N for the first change.").color(MUTED));
        return;
    };
    let change = view.change(tile);
    let color = change.map_or(egui::Color32::WHITE, |c| {
        egui_color(diff_view::change_color(c))
    });
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(format!(
            "{}  {}",
            change.map_or("", diff_view::sign),
            explorer.unit_name(&unit)
        ))
        .strong()
        .size(16.0)
        .color(color),
    );
    let (old, new) = (
        sources.version(view.old(), &unit),
        sources.version(explorer.system(), &unit),
    );
    let mut place_line = match (&old, &new) {
        (Some(a), Some(b)) if a.uri != b.uri || a.first != b.first => {
            format!("was {}:{}, now {}:{}", a.uri, a.first, b.uri, b.first)
        }
        (_, Some(v)) | (Some(v), None) => format!("{}:{}", v.uri, v.first),
        (None, None) => String::new(),
    };
    if change.is_some() {
        let (added, removed) = source_diff::counts(old.as_ref(), new.as_ref());
        let _ = write!(place_line, "   +{added} −{removed} lines");
    } else {
        place_line.push_str("   unchanged");
    }
    ui.label(egui::RichText::new(place_line).small().color(MUTED));
    if let Some(was) = view.moved_from(tile) {
        ui.label(
            egui::RichText::new(format!("moved from {was}"))
                .small()
                .color(color),
        );
    }
    if change.is_some() {
        affects(ui, view, tile);
        calls(ui, explorer, view, tile);
    }
    ui.separator();
    if old.is_none() && new.is_none() {
        ui.label(egui::RichText::new("No source to show.").color(MUTED));
        return;
    }
    let key = egui::Id::new(("change_pane", tile));
    let height = (ui.available_height() - LIST).max(160.0);
    if change.is_some() {
        source_diff::show(ui, old.as_ref(), new.as_ref(), true, height, key);
    } else {
        // Unchanged: the same on both sides, so one side shows it all.
        source_diff::show(ui, new.as_ref(), new.as_ref(), false, height, key);
    }
}

/// The entry points a change can affect, by name, the nearest the graph draws the way
/// up to first.
fn affects(ui: &mut egui::Ui, view: &DiffView, tile: usize) {
    let reached = view.reached_from(tile);
    let what = if view.roots {
        "functions nothing calls"
    } else {
        "entry points"
    };
    if reached.is_empty() {
        ui.label(
            egui::RichText::new(format!("No {what} reach it"))
                .small()
                .color(MUTED),
        );
        return;
    }
    let ends = view.route().map(|r| r.ends.as_slice()).unwrap_or_default();
    let mut names: Vec<(bool, &str)> = reached
        .iter()
        .filter_map(|&i| view.starts.get(i))
        .map(|start| {
            let drawn = start.tile.is_some_and(|t| ends.contains(&t));
            (!drawn, start.label.as_str())
        })
        .collect();
    names.sort_unstable();
    let mut text = names
        .iter()
        .take(NAMED)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > NAMED {
        let _ = write!(text, " and {} more", names.len() - NAMED);
    }
    ui.label(
        egui::RichText::new(format!(
            "Affects {} of {} {what}: {text}",
            reached.len(),
            view.starts.len()
        ))
        .small(),
    )
    .on_hover_text(
        names
            .iter()
            .map(|(_, name)| *name)
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// The calls a change gained, green, and lost, red, as the graph draws them.
fn calls(ui: &mut egui::Ui, explorer: &Explorer, view: &DiffView, tile: usize) {
    let (gained, lost) = view.calls_changed(tile);
    if gained.is_empty() && lost.is_empty() {
        return;
    }
    let name = |t: usize| {
        explorer
            .tile_unit(t)
            .map_or_else(String::new, |u| explorer.unit_name(u))
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(egui::RichText::new("calls").small().color(MUTED));
        for (tiles, sign, color) in [(&gained, "+", ADDED), (&lost, "−", REMOVED)] {
            for &callee in tiles {
                ui.label(
                    egui::RichText::new(format!("{sign}{}", name(callee)))
                        .small()
                        .color(egui_color(color)),
                );
            }
        }
    });
}
