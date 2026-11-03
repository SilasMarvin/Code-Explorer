//! Every color that means something, defined once, each meaning one thing everywhere:
//! white the selection or the journey's start, violet calls into it, blue its own
//! calls, amber the journey's calls further down, pink the journey's cursor, grey
//! everything else; and a muted hue per module, for telling modules apart alone.

use bevy::color::{Color, ColorToPacked, Mix};
use bevy_egui::egui;

/// The selection and the journey's start; and the ring round a function set aside in
/// a journey, whose fill stays its role's.
pub(crate) const SELECTED: Color = Color::WHITE;
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
/// What the pointer rests on, the one way everywhere: a thin ring round its dot, an
/// outline round its bar, a line from its bar to its dot. Grey, never the
/// selection's white.
pub(crate) const HOVER: Color = Color::srgb(0.5, 0.53, 0.6);

/// [`DEEPER`] at a level of the journey, 2 and down: darker a level at a time, as dark
/// as `last` at most.
pub(crate) fn deeper(level: u32, last: u32) -> Color {
    #[expect(clippy::cast_precision_loss, reason = "a small level")]
    let darker = (level.clamp(2, last) - 2) as f32;
    DEEPER.mix(&Color::BLACK, 0.2 * darker)
}

/// How much of its color a call's line keeps, by what it is: a call the view names;
/// one among the expanded module's functions no role names; a caller of the start
/// folded into its module's count; a call in from off the journey (I); one only
/// guessed, or handed off; a line between modules on the whole map; at most, a call
/// crossing the view with both its ends off it. The calls of a
/// function set aside keep all of it, a grey one [`PICKED_STRENGTH`]: grey still,
/// never the selection's white.
pub(crate) const CALL_STRENGTH: f32 = 0.85;
pub(crate) const INNER_STRENGTH: f32 = 0.28;
pub(crate) const FOLDED_STRENGTH: f32 = 0.25;
pub(crate) const OUTSIDE_STRENGTH: f32 = 0.35;
pub(crate) const GUESSED_STRENGTH: f32 = 0.3;
pub(crate) const MAP_STRENGTH: f32 = 0.12;
pub(crate) const PICKED_STRENGTH: f32 = 0.6;
pub(crate) const ACROSS_STRENGTH: f32 = 0.3;

/// Behind the scene.
pub(crate) const GROUND: Color = Color::srgb(0.06, 0.07, 0.09);

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
/// The dark pill a name on the graph sits on, so it reads over the arrows: opaque,
/// so no dot or line shows through its letters.
pub(crate) const PILL: egui::Color32 = egui::Color32::from_rgb(11, 13, 19);
/// A plain function's name: quieter than a lit one's.
pub(crate) const PLAIN_TEXT: egui::Color32 = egui::Color32::from_rgb(178, 184, 196);
/// The name of a function faded beside what is selected: quieter still.
pub(crate) const FADED_TEXT: egui::Color32 = egui::Color32::from_rgb(112, 118, 130);
/// Text on a bar or a badge filled with a node's color.
pub(crate) const ON_FILL: egui::Color32 = egui::Color32::from_rgb(14, 16, 22);
/// Text that is only text, and a name in the top bar.
pub(crate) const TEXT: egui::Color32 = egui::Color32::WHITE;
/// What sits over the graph: the journey panel, the search box, the help, a notice;
/// and the edge round it.
pub(crate) const SURFACE: egui::Color32 = egui::Color32::from_rgb(11, 13, 19);
pub(crate) const EDGE: egui::Color32 = egui::Color32::from_gray(60);
/// A chip in the top bar: where the view is, and a function set aside.
pub(crate) const CHIP: egui::Color32 = egui::Color32::from_rgb(30, 34, 46);
/// The chip for a function set aside that calls the journey's start: violet, as
/// calling it is.
pub(crate) const CALLER_CHIP: egui::Color32 = egui::Color32::from_rgb(70, 52, 104);
/// Quiet marks in the panel: the line from a call to its caller's row, the track
/// along a row that scrolls and its thumb, the panel's edge, the line under the cursor.
pub(crate) const FAINT_LINE: egui::Color32 = egui::Color32::from_rgba_premultiplied(70, 70, 70, 70);
pub(crate) const TRACK: egui::Color32 = egui::Color32::from_rgba_premultiplied(18, 18, 18, 18);
pub(crate) const THUMB: egui::Color32 = egui::Color32::from_rgba_premultiplied(110, 110, 110, 110);
pub(crate) const PANEL_EDGE: egui::Color32 = egui::Color32::from_rgba_premultiplied(28, 28, 28, 28);
pub(crate) const CURSOR_LINE: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(60, 60, 60, 60);
/// A call that only sometimes runs, outlined in dashes; a badge of calls folded
/// under a bar, and its ink.
pub(crate) const SOMETIMES: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(190, 190, 190, 190);
pub(crate) const BADGE_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 120);
pub(crate) const BADGE_INK: egui::Color32 = egui::Color32::from_rgb(220, 224, 234);
/// How much of its color a bar keeps for a call only guessed, or handed off.
pub(crate) const GUESSED: f32 = 0.22;

/// Module colors: a module's sphere, its rim and its name. Six hues far enough apart
/// to tell neighbouring modules by, all muted and none near a role's color: jade,
/// lime, brick, green, stone and sand read as none of white, violet, blue, amber or
/// pink.
const MODULES: [Color; 6] = [
    Color::srgb(0.38, 0.74, 0.58),
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
