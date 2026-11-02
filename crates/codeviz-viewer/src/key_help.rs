//! What the keys do: a hint along the graph's foot, and on `H` what the picture
//! shows and every key that does something in the view open.

use bevy_egui::egui;

use crate::{
    colors::{CALLEE, CALLER, CURSOR, DEEPER, EDGE, MUTED, SELECTED, SURFACE, TEXT, egui_color},
    labels::{PILL, PILL_PAD},
    text,
};

/// A hint at the keys along `foot`, browsing; `expanded`, with a module expanded.
pub(crate) fn key_hint(ctx: &egui::Context, foot: egui::Rect, expanded: bool) {
    ctx.layer_painter(egui::LayerId::background())
        .with_clip_rect(foot)
        .text(
            egui::pos2(foot.min.x + 12.0, foot.max.y - 10.0),
            egui::Align2::LEFT_BOTTOM,
            hint(true, expanded),
            egui::FontId::proportional(text::SMALL),
            MUTED,
        );
}

/// The line on the keys that matter most, browsing or in a journey; inside a module
/// expanded, how to back out of it first.
pub(crate) fn hint(browsing: bool, expanded: bool) -> String {
    let keys = if browsing {
        "click a function to select it, nothing moves · Enter opens its journey · \
         Backspace back · / search · R whole map · H help"
    } else {
        "click a call to go to it · ] [ ↓ ↑ ← → step · ↑ from the start: its callers · \
         , . less or more · I who else calls it · Esc leaves for browsing · H help"
    };
    match (browsing, expanded) {
        (_, true) => format!("X or Esc: back out · {keys}"),
        (true, false) => format!("Browsing · {keys}"),
        (false, false) => keys.to_owned(),
    }
}

/// Which view a key does something in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum When {
    Always,
    /// No journey open.
    Browse,
    /// A journey open.
    Journey,
}

impl When {
    /// Whether a key does something browsing, or in a journey.
    fn applies(self, browsing: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Browse => browsing,
            Self::Journey => !browsing,
        }
    }
}

/// Every key, what it does, and in which view; H lists those that apply to the view
/// open.
const KEYS: [(&str, &str, When); 25] = [
    ("click", "select a function; nothing moves", When::Browse),
    (
        "click",
        "a call on the journey (its dot, name or bar): go to it; any other function: \
         select it",
        When::Journey,
    ),
    ("click empty space", "clear the selection", When::Always),
    (
        "Enter",
        "open the selected function's journey",
        When::Browse,
    ),
    (
        "Enter",
        "start a journey at the function selected off it, else at the call you are on",
        When::Journey,
    ),
    ("Shift click", "open a function's journey", When::Always),
    ("↓  ↑", "into the call / out to its caller", When::Journey),
    (
        "↑ at the start",
        "to the functions that call it, in the violet row above it: ← → pick one, \
         Enter starts its journey, ↓ comes back",
        When::Journey,
    ),
    (
        "←  →",
        "the previous / next call from the same caller",
        When::Journey,
    ),
    (
        "[  ]",
        "the previous / next call in the order they run",
        When::Journey,
    ),
    (",  .", "show a level less / more", When::Journey),
    (
        "I",
        "show the functions off the journey that call the one you are on",
        When::Journey,
    ),
    (
        "/",
        "find a function by name; picking it is a click on it",
        When::Always,
    ),
    (
        "X, double-click",
        "zoom into a module; again to back out",
        When::Always,
    ),
    ("wheel", "zoom towards the pointer", When::Always),
    ("drag", "turn the view", When::Always),
    ("right drag", "move the view", When::Always),
    ("F", "fit the selection on screen", When::Browse),
    ("F", "fit the journey on screen", When::Journey),
    ("R", "the whole map, from the front", When::Browse),
    (
        "R",
        "the whole journey and its modules, from the front",
        When::Journey,
    ),
    ("Backspace", "back; with Shift, forward", When::Always),
    (
        "Esc",
        "one step out: close this or the search; back out of a module; then clear the \
         selection",
        When::Browse,
    ),
    (
        "Esc",
        "one step out: close this or the search; let go of the function selected off \
         the journey; back out of a module; then leave the journey",
        When::Journey,
    ),
    ("H", "this help", When::Always),
];

/// What each color means, as the help leads with it.
fn color_key(browsing: bool) -> Vec<(egui::Color32, &'static str)> {
    if browsing {
        return vec![
            (egui_color(SELECTED), "the function selected"),
            (egui_color(CALLER), "calls it"),
            (egui_color(CALLEE), "it calls"),
        ];
    }
    vec![
        (egui_color(SELECTED), "where the journey starts"),
        (egui_color(CALLER), "calls it"),
        (egui_color(CALLEE), "it calls"),
        (egui_color(DEEPER), "called further on, darker the deeper"),
        (egui_color(CURSOR), "the call you are on"),
    ]
}

/// What the picture shows, in plain words, as the help says it.
fn reading(browsing: bool) -> &'static [&'static str] {
    const MAP: &str = "Each sphere is a module of the code, and each dot in it one of its \
        functions; the more functions, the bigger the sphere. A grey line between two \
        spheres: functions in one call functions in the other. A name that does not fit \
        is left off: point at a dot for it.";
    const BOTH: &str = "A blue or amber dot ringed in violet is both: it is called, and \
        calls back in.";
    if browsing {
        return &[MAP, BOTH];
    }
    &[
        MAP,
        BOTH,
        "A journey shows the calls one function makes, a level at a time, and the \
         functions that call it. A number says where a call comes: 2 is the start's \
         second call, 2.1 the first call that one makes; +1 after it, the journey makes \
         the call once more later on.",
        "The panel at the foot lists the same calls: the start's bar, then a row a level \
         down, left to right as they run; above the start, the functions that call it. \
         Dashed: runs only sometimes. ▼ and a count: calls hidden under it; click to \
         show them. A green end returns, a red one can fail. ↺: calls itself.",
    ]
}

/// Width of the keys' column in the help.
const KEY_WIDTH: f32 = 96.0;

/// Widest the help grows, and its share of the window at most.
const HELP_WIDTH: f32 = 520.0;
const HELP_SHARE: f32 = 1.0 / 3.0;

/// What the picture shows and the keys that apply to the view open, on a sheet down
/// the right of `area` a third of the window wide at most, whole at 1600 × 1000, and
/// in a smaller window scrolling, with a cue at its foot while more lies below.
pub(crate) fn key_help(ctx: &egui::Context, area: egui::Rect, open: &mut bool, browsing: bool) {
    let margin = 12.0;
    let width = (ctx.content_rect().width() * HELP_SHARE)
        .min(HELP_WIDTH)
        .min(area.width() - 2.0 * margin)
        .max(240.0);
    let height = (area.height() - 2.0 * margin).max(160.0);
    let keys: Vec<(&str, &str)> = KEYS
        .iter()
        .filter(|(.., when)| when.applies(browsing))
        .map(|&(key, what, _)| (key, what))
        .collect();
    egui::Area::new("key_help".into())
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(egui::pos2(area.max.x - margin, area.min.y + margin))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(SURFACE)
                .stroke(egui::Stroke::new(1.0, EDGE))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    // Less its margins and its border.
                    ui.set_width(width - 30.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        let title = if browsing { "Browsing" } else { "In a journey" };
                        ui.label(egui::RichText::new(title).strong().size(text::TITLE));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("close (H)").clicked() {
                                *open = false;
                            }
                        });
                    });
                    let body = |ui: &mut egui::Ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        for line in reading(browsing) {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(*line).size(text::SMALL).color(MUTED),
                                )
                                .wrap(),
                            );
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for (color, what) in color_key(browsing) {
                                ui.label(egui::RichText::new("●").color(color));
                                ui.label(egui::RichText::new(what).size(text::SMALL).color(MUTED));
                                ui.add_space(8.0);
                            }
                        });
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Keys").strong().size(text::BODY));
                        for (key, what) in &keys {
                            key_line(ui, key, what);
                        }
                    };
                    // As tall as the window lets it be: a scroll area is otherwise only
                    // as tall as egui guesses an area's room to be.
                    let scroll = egui::ScrollArea::vertical()
                        .max_height(height - 40.0)
                        .min_scrolled_height(height - 40.0)
                        .auto_shrink([false, true])
                        .show(ui, body);
                    more_below(ui, &scroll);
                });
        });
}

/// Whether the help had more below its foot last frame.
fn more_below_id() -> egui::Id {
    egui::Id::new("key_help_more_below")
}

/// While more of a scrolled sheet lies below, a cue at its foot, over the sheet.
fn more_below(ui: &egui::Ui, scroll: &egui::scroll_area::ScrollAreaOutput<()>) {
    let below = scroll.content_size.y - scroll.state.offset.y - scroll.inner_rect.height();
    ui.ctx()
        .data_mut(|d| d.insert_temp(more_below_id(), below > 4.0));
    if below <= 4.0 {
        return;
    }
    let foot = scroll.inner_rect;
    let galley = ui.painter().layout_no_wrap(
        "▼ more below: scroll".to_owned(),
        egui::FontId::proportional(text::SMALL),
        TEXT,
    );
    let at = egui::pos2(
        foot.center().x - galley.size().x / 2.0,
        foot.max.y - galley.size().y - 4.0,
    );
    let pill = egui::Rect::from_min_size(at, galley.size()).expand2(PILL_PAD);
    ui.painter().rect_filled(pill, 4.0, PILL);
    ui.painter().galley(at, galley, TEXT);
}

/// One key, in a fixed column, and what it does wrapping beside it.
fn key_line(ui: &mut egui::Ui, key: &str, what: &str) {
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(KEY_WIDTH, 13.0),
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                ui.set_min_width(KEY_WIDTH);
                ui.label(egui::RichText::new(key).size(text::SMALL).color(TEXT));
            },
        );
        ui.add(egui::Label::new(egui::RichText::new(what).size(text::SMALL).color(MUTED)).wrap());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_list_fits_whole_at_1600_by_1000() {
        for browsing in [false, true] {
            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
            let area = egui::Rect::from_min_max(egui::pos2(0.0, 24.0), screen.max);
            let mut open = true;
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    key_help(ui.ctx(), area, &mut open, browsing);
                });
                output.textures_delta.clear();
            }
            let more = ctx.data(|d| d.get_temp::<bool>(more_below_id()));
            assert_eq!(
                more,
                Some(false),
                "nothing left below, browsing: {browsing}"
            );
        }
    }

    #[test]
    fn the_key_list_fits_a_small_window() {
        for browsing in [false, true] {
            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
            // Below the top bar.
            let area = egui::Rect::from_min_max(egui::pos2(0.0, 30.0), screen.max);
            let mut open = true;
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    key_help(ui.ctx(), area, &mut open, browsing);
                });
                output.textures_delta.clear();
            }
            let shown = ctx
                .memory(|m| m.area_rect(egui::Id::new("key_help")))
                .expect("the list shows");
            assert!(
                area.contains_rect(shown),
                "inside the window: {shown:?} in {area:?}"
            );
            assert!(
                shown.width() <= screen.width() * HELP_SHARE + 1.0,
                "a third of the window across at most: {shown:?}"
            );
        }
    }

    #[test]
    fn the_help_speaks_plainly() {
        for browsing in [false, true] {
            let words = reading(browsing)
                .iter()
                .copied()
                .chain(KEYS.iter().map(|(_, what, _)| *what))
                .chain(color_key(browsing).into_iter().map(|(_, what)| what));
            for line in words {
                for jargon in [" lit", "focus", "callers of the start", " of 39", "nucleus"] {
                    assert!(!line.contains(jargon), "{jargon:?} in {line:?}");
                }
            }
        }
    }
}
