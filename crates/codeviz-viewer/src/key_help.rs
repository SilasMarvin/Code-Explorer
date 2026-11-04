//! What the keys do: a hint along the graph's foot, and on `H` what the picture
//! shows and every key that does something in the view open.

use bevy_egui::egui;

use crate::{
    colors::{
        CALLEE, CALLER, CURSOR, DEEPER, EDGE, MUTED, OK, PILL, SELECTED, SURFACE, TEXT, egui_color,
    },
    labels::PILL_PAD,
    text,
};

/// A hint at the keys along `foot`, browsing; `expanded`, with a module expanded;
/// on a backing of its own so no name drawn under it shows through.
pub(crate) fn key_hint(ctx: &egui::Context, foot: egui::Rect, expanded: bool) {
    let painter = ctx
        .layer_painter(egui::LayerId::background())
        .with_clip_rect(foot);
    let galley = hint_galley(&painter, expanded);
    let at = hint_at(foot, galley.size());
    let backing = egui::Rect::from_min_size(at, galley.size()).expand2(PILL_PAD);
    painter.rect_filled(backing, 4.0, PILL);
    painter.galley(at, galley, MUTED);
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(hint_drawn_id(), (pass, backing)));
}

fn hint_drawn_id() -> egui::Id {
    egui::Id::new("key_hint_drawn")
}

/// Where the hint at the keys was drawn in the egui pass before `pass`, if it was:
/// what it covers on the graph a session reads as under it.
pub(crate) fn hint_drawn(ctx: &egui::Context, pass: u64) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp::<(u64, egui::Rect)>(hint_drawn_id()))
        .filter(|&(at, _)| at + 1 == pass)
        .map(|(_, rect)| rect)
}

/// Where the browsing hint at the keys lies along `foot`, its backing included: names
/// on the graph keep off it.
pub(crate) fn hint_rect(painter: &egui::Painter, foot: egui::Rect, expanded: bool) -> egui::Rect {
    let size = hint_galley(painter, expanded).size();
    egui::Rect::from_min_size(hint_at(foot, size), size).expand2(PILL_PAD)
}

fn hint_galley(painter: &egui::Painter, expanded: bool) -> std::sync::Arc<egui::Galley> {
    painter.layout_no_wrap(
        hint(true, expanded),
        egui::FontId::proportional(text::SMALL),
        MUTED,
    )
}

/// The hint's top left: at the foot's left, its text's foot 10 points above the
/// foot's.
fn hint_at(foot: egui::Rect, size: egui::Vec2) -> egui::Pos2 {
    egui::pos2(foot.min.x + 12.0, foot.max.y - 10.0 - size.y)
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
/// open. One line each.
const KEYS: [(&str, &str, When); 25] = [
    ("click", "select a function; nothing moves", When::Browse),
    (
        "click",
        "a call on the journey: go to it; another function: pick it",
        When::Journey,
    ),
    ("click empty space", "clear the selection", When::Browse),
    (
        "click empty space",
        "let go of the function picked",
        When::Journey,
    ),
    (
        "Enter",
        "open the selected function's journey",
        When::Browse,
    ),
    (
        "Enter",
        "a journey at the function picked, else at the call you are on",
        When::Journey,
    ),
    ("Shift click", "open a function's journey", When::Always),
    (
        "↓  ↑",
        "into the call, its calls opened / out to its caller",
        When::Journey,
    ),
    (
        "↑ at the start",
        "its callers' row: ← → pick one, Enter opens it, ↓ back",
        When::Journey,
    ),
    (
        "←  →",
        "the previous / next call from the same caller",
        When::Journey,
    ),
    (
        "[  ]",
        "the previous / next call as they run",
        When::Journey,
    ),
    (",  .", "every branch a level less / more", When::Journey),
    ("I", "who else calls the call you are on", When::Journey),
    (
        "/",
        "find a function; picking it is a click on it",
        When::Always,
    ),
    (
        "X, double-click",
        "zoom into a module; again to back out",
        When::Browse,
    ),
    (
        "X, double-click",
        "zoom into a module, which follows the cursor; again to back out",
        When::Journey,
    ),
    ("wheel", "zoom towards the pointer", When::Always),
    ("drag, right drag", "turn the view, move it", When::Always),
    ("F", "fit the selection on screen", When::Browse),
    ("F", "fit the journey on screen", When::Journey),
    ("R", "the whole map, from the front", When::Browse),
    ("R", "the whole journey, from the front", When::Journey),
    ("Backspace", "back; with Shift, forward", When::Always),
    (
        "Esc",
        "one step out: this, search, module, selection",
        When::Browse,
    ),
    (
        "Esc",
        "one step out: this, search, the function picked, module, journey",
        When::Journey,
    ),
];

/// What the picture shows, one line each: a mark as drawn, in its color, and what
/// it means. Browsing, then in a journey.
fn legend(browsing: bool) -> Vec<(&'static str, egui::Color32, &'static str)> {
    let white = egui_color(SELECTED);
    let violet = egui_color(CALLER);
    let mut lines = vec![
        (
            "○ sphere",
            MUTED,
            "a module, bigger the more functions it holds",
        ),
        ("● dot", MUTED, "a function; point at it for its name"),
        (
            "— grey line",
            MUTED,
            "functions in one module call the other's",
        ),
    ];
    if browsing {
        lines.extend([
            ("● white", white, "the function selected"),
            ("● violet", violet, "calls it"),
            ("● blue", egui_color(CALLEE), "it calls"),
            ("◎ violet ring", violet, "it calls, and calls it back"),
            ("faint sphere", MUTED, "neither calls it nor is called"),
            (
                "faint dot, line",
                MUTED,
                "a call only guessed, or handed off",
            ),
        ]);
        return lines;
    }
    lines.extend([
        ("● white", white, "where the journey starts"),
        ("● pink", egui_color(CURSOR), "the call you are on"),
        ("● blue", egui_color(CALLEE), "a call the start makes"),
        (
            "● amber",
            egui_color(DEEPER),
            "called further on, darker the deeper",
        ),
        ("● violet", violet, "calls the start: the row above its bar"),
        (
            "◎ violet ring",
            violet,
            "on the journey, and calls the start back",
        ),
        ("◎ white ring", white, "a function picked off the journey"),
        ("● grey", MUTED, "off the journey; I shows who calls in"),
        (
            "faint",
            MUTED,
            "only guessed, handed off, or counted on its module",
        ),
        (
            "2.1  +1",
            TEXT,
            "call 2's first call; +1, made once more later",
        ),
        ("▼ 3", TEXT, "calls folded under a bar: click to step in"),
        ("dashed bar", TEXT, "runs only sometimes"),
        ("green, red end", OK, "returns, can fail"),
        ("↺", TEXT, "calls itself, or calls the start again"),
    ]);
    lines
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
                        for (mark, color, what) in legend(browsing) {
                            line(ui, (mark, color), what);
                        }
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Keys").strong().size(text::BODY));
                        for (key, what) in &keys {
                            line(ui, (key, TEXT), what);
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

/// One key or mark, in its color in a fixed column, and what it means beside it.
fn line(ui: &mut egui::Ui, (key, color): (&str, egui::Color32), what: &str) {
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(KEY_WIDTH, 13.0),
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                ui.set_min_width(KEY_WIDTH);
                ui.label(egui::RichText::new(key).size(text::SMALL).color(color));
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
    fn each_colour_state_and_key_takes_one_line() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter();
            let wide = |text: &str| {
                painter
                    .layout_no_wrap(
                        text.to_owned(),
                        egui::FontId::proportional(text::SMALL),
                        TEXT,
                    )
                    .size()
                    .x
            };
            // The sheet at its widest, less its margins, the keys' column and a gap.
            let room = HELP_WIDTH - 30.0 - KEY_WIDTH - 8.0;
            for browsing in [false, true] {
                let lines = legend(browsing)
                    .into_iter()
                    .map(|(mark, _, what)| (mark, what))
                    .chain(KEYS.iter().map(|&(key, what, _)| (key, what)));
                for (key, what) in lines {
                    assert!(wide(key) <= KEY_WIDTH, "{key:?} fits its column");
                    assert!(wide(what) <= room, "{what:?} fits one line");
                }
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_help_speaks_plainly() {
        for browsing in [false, true] {
            let words = legend(browsing)
                .into_iter()
                .map(|(_, _, what)| what)
                .chain(KEYS.iter().map(|(_, what, _)| *what));
            for line in words {
                for jargon in [" lit", "focus", "callers of the start", " of 39", "nucleus"] {
                    assert!(!line.contains(jargon), "{jargon:?} in {line:?}");
                }
            }
        }
    }
}
