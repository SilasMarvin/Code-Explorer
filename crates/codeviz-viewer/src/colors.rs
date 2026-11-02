//! Every color that means something, defined once, each meaning one thing everywhere:
//! white the selection or the journey's start, violet calls into it, blue its own
//! calls, amber the journey's calls further down, pink the journey's cursor, grey
//! everything else; and a muted hue per module, for telling modules apart alone.

use bevy::color::{Color, ColorToPacked};
use bevy_egui::egui;

/// Calls into the selection or the journey's start.
pub(crate) const CALLER: Color = Color::srgb(0.74, 0.52, 1.0);
/// Its own calls out.
pub(crate) const CALLEE: Color = Color::srgb(0.3, 0.72, 1.0);
/// The journey's calls past the start's own: one color for every level below the
/// first, darkened a level at a time, so blue only ever means the start's own calls.
pub(crate) const DEEPER: Color = Color::srgb(1.0, 0.7, 0.22);
/// The journey's cursor: its node, its name and the call into it.
pub(crate) const CURSOR: Color = Color::srgb(1.0, 0.36, 0.72);
/// Calls that touch neither the selection nor the start: an arrow, but no color.
pub(crate) const FURTHER: Color = Color::srgb(0.82, 0.85, 0.92);

/// [`CALLER`], [`CALLEE`], [`DEEPER`] and [`CURSOR`] as a name's text: lighter, to
/// read on a dark pill.
pub(crate) const CALLER_TEXT: egui::Color32 = egui::Color32::from_rgb(206, 170, 255);
pub(crate) const CALLEE_TEXT: egui::Color32 = egui::Color32::from_rgb(130, 196, 255);
pub(crate) const DEEPER_TEXT: egui::Color32 = egui::Color32::from_rgb(255, 204, 128);
pub(crate) const CURSOR_TEXT: egui::Color32 = egui::Color32::from_rgb(255, 160, 210);
/// Quiet text, and whatever belongs to no module.
pub(crate) const MUTED: egui::Color32 = egui::Color32::from_rgb(140, 148, 165);
/// How a journey's call can end: it returns, or it fails.
pub(crate) const OK: egui::Color32 = egui::Color32::from_rgb(120, 210, 140);
pub(crate) const ERROR: egui::Color32 = egui::Color32::from_rgb(240, 110, 100);
/// Text on a bar or a badge filled with a node's color.
pub(crate) const ON_FILL: egui::Color32 = egui::Color32::from_rgb(14, 16, 22);

/// Module colors: a module's sphere, its rim and its name. Six hues far enough apart
/// to tell neighbouring modules by, all muted and none near a role's color: teal,
/// lime, brick, green, stone and sand read as none of white, violet, blue, amber or
/// pink.
const MODULES: [Color; 6] = [
    Color::srgb(0.36, 0.74, 0.76),
    Color::srgb(0.66, 0.80, 0.34),
    Color::srgb(0.84, 0.50, 0.42),
    Color::srgb(0.44, 0.78, 0.44),
    Color::srgb(0.66, 0.66, 0.62),
    Color::srgb(0.78, 0.76, 0.54),
];

/// How many module colors there are: what is drawn in one is made once for each.
pub(crate) const MODULE_SHADES: usize = MODULES.len();

/// Which of the module colors a module, by cluster, is drawn in.
pub(crate) fn module_shade(cluster: usize) -> usize {
    cluster % MODULE_SHADES
}

/// A module's color, by cluster.
pub(crate) fn module_color(cluster: usize) -> Color {
    MODULES[module_shade(cluster)]
}

pub(crate) fn egui_color(color: Color) -> egui::Color32 {
    let [r, g, b, a] = color.to_srgba().to_u8_array();
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}
