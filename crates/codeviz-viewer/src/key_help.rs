//! What the keys do: a hint along the graph's foot, and the full list on `H`, each
//! key saying what it does in the view open.

use bevy_egui::egui;

use crate::{
    colors::{CALLEE, CALLER, CURSOR, DEEPER, MUTED, egui_color},
    labels::{PILL, PILL_PAD},
    text,
};

/// A hint at the keys along `foot`, on a backing of its own so no name drawn under
/// it shows through.
pub(crate) fn key_hint(ctx: &egui::Context, foot: egui::Rect) {
    let painter = ctx
        .layer_painter(egui::LayerId::background())
        .with_clip_rect(foot);
    let galley = hint_galley(&painter);
    let at = hint_at(foot, galley.size());
    painter.rect_filled(
        egui::Rect::from_min_size(at, galley.size()).expand2(PILL_PAD),
        4.0,
        PILL,
    );
    painter.galley(at, galley, MUTED);
}

/// Where the browsing hint at the keys lies along `foot`, its backing included: names
/// on the graph keep off it.
pub(crate) fn hint_rect(painter: &egui::Painter, foot: egui::Rect) -> egui::Rect {
    let size = hint_galley(painter).size();
    egui::Rect::from_min_size(hint_at(foot, size), size).expand2(PILL_PAD)
}

fn hint_galley(painter: &egui::Painter) -> std::sync::Arc<egui::Galley> {
    painter.layout_no_wrap(
        hint(true).to_owned(),
        egui::FontId::proportional(text::SMALL),
        MUTED,
    )
}

/// The hint's top left: at the foot's left, its text's foot 10 points above the
/// foot's.
fn hint_at(foot: egui::Rect, size: egui::Vec2) -> egui::Pos2 {
    egui::pos2(foot.min.x + 12.0, foot.max.y - 10.0 - size.y)
}

/// The line on the keys that matter most, browsing or in a journey.
pub(crate) fn hint(browsing: bool) -> &'static str {
    if browsing {
        "Browsing · click a function to select it, nothing moves · Enter opens its journey · \
         Backspace back · / search · R whole map · H keys"
    } else {
        "click a call to move the cursor there · ] [ ↓ ↑ ← → step · , . less or more · \
         I calls in from outside · Esc leaves for browsing · H keys"
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

/// A group of keys by title: each key, what it does, and in which view.
type KeyGroup = (&'static str, &'static [(&'static str, &'static str, When)]);

/// Every key, grouped by what it moves, each saying what it does in the view it
/// applies in; H lists those that apply to the view open.
const KEYS: [KeyGroup; 7] = [
    (
        "Browse and journey",
        &[
            (
                "click",
                "select a function where it sits: its callers and calls light up; nothing moves",
                When::Browse,
            ),
            (
                "click",
                "a function the journey runs: move the cursor there (the run nearest it); \
                 nothing moves",
                When::Journey,
            ),
            (
                "click",
                "any other function: select it in place, with its calls into the journey",
                When::Journey,
            ),
            (
                "click a name",
                "as a click on its function; on the journey, to the call its number names",
                When::Always,
            ),
            (
                "Enter",
                "open the journey of the function selected",
                When::Browse,
            ),
            (
                "Enter",
                "start a journey at the function selected, or at the cursor",
                When::Journey,
            ),
            (
                "Shift click",
                "open the journey of a function at once, as deep as now",
                When::Always,
            ),
            (
                "Esc",
                "close search, this list, a function selected off the journey, the \
                 module expanded; then leave the journey for browsing",
                When::Journey,
            ),
            (
                "Esc",
                "close search, this list, the module expanded",
                When::Browse,
            ),
            (
                "click empty space",
                "clear the selection; the camera stays",
                When::Browse,
            ),
            (
                "click empty space",
                "let go of a function selected off the journey",
                When::Journey,
            ),
            (
                "Backspace",
                "back: the selection before, or across into the journey before (← Back)",
                When::Always,
            ),
            ("Shift Backspace", "forward again (Forward →)", When::Always),
            ("/", "search for a function", When::Always),
        ],
    ),
    (
        "Journey cursor",
        &[
            (
                "↓  ↑",
                "into the first call it makes / out to its caller",
                When::Journey,
            ),
            (
                "←  →",
                "previous / next call by the same caller",
                When::Journey,
            ),
            (
                "[  ]",
                "back / forward one call, in the order they run",
                When::Journey,
            ),
            ("click a bar", "to that call", When::Journey),
        ],
    ),
    (
        "Journey depth",
        &[(
            ",  .",
            "show a level less / more of the journey",
            When::Journey,
        )],
    ),
    (
        "Calls in",
        &[(
            "I",
            "bring in every function off the journey that calls one on it, pale",
            When::Journey,
        )],
    ),
    (
        "Spheres",
        &[
            (
                "X",
                "fly into the module under the pointer, else the focus's / back out",
                When::Always,
            ),
            (
                "dbl-click",
                "fly into the module clicked; off any function, back out",
                When::Always,
            ),
        ],
    ),
    (
        "Camera",
        &[
            ("drag", "orbit (or two fingers)", When::Always),
            (
                "right drag",
                "pan (or Shift drag, Shift two fingers)",
                When::Always,
            ),
            (
                "wheel, pinch",
                "zoom towards the pointer (or Ctrl two fingers)",
                When::Always,
            ),
            (
                "F",
                "frame the focus: the selection, the journey, or the module expanded",
                When::Always,
            ),
            ("R", "everything shown, from the front", When::Always),
        ],
    ),
    ("Help", &[("H", "this list", When::Always)]),
];

/// What each color means, as the help leads with it.
fn color_key(browsing: bool) -> Vec<(egui::Color32, &'static str)> {
    if browsing {
        return vec![
            (egui::Color32::WHITE, "the function selected"),
            (egui_color(CALLER), "calls into it"),
            (egui_color(CALLEE), "its own calls out"),
        ];
    }
    vec![
        (egui::Color32::WHITE, "the journey's start"),
        (egui_color(CALLER), "calls into it"),
        (egui_color(CALLEE), "its own calls"),
        (egui_color(DEEPER), "calls further down, darker the deeper"),
        (egui_color(CURSOR), "the journey's cursor"),
    ]
}

/// How what is drawn reads, beyond its colors, as the help says it.
fn reading(browsing: bool) -> &'static [&'static str] {
    if browsing {
        return &[
            "Each sphere is a module, in a color of its own, named above it where its \
             name fits; point at one for its name. Two modules of one name are told \
             apart by their crate (layout::graph).",
            "A function is named beside its own dot where its name fits, the most \
             important first; one left out is named when you point at it, and the lit \
             ones left out are counted at the foot. Zoom in to name more.",
        ];
    }
    &[
        "Each sphere is a module, in a color of its own; the middle one holds the start. \
         Callers of the start past the few named show small and unnamed; point at one for \
         its name.",
        "On a name, 2 +1: its first call in the journey, numbered as in the panel, and how \
         many more times the journey runs it; the cursor's shows the call it is on. Two \
         functions of one name show their types (Explorer.root, Track.root).",
        "The panel: a bar per call, left to right as they run, a row a level deeper. \
         Dashed: runs only sometimes, and says when (if …, in a loop). Pale: a call only \
         guessed at, or handed off to run later. A triangle and a count: calls folded \
         under it; a click opens them. A green end returns, a red one fails. ↻: already \
         running further up; ↺ calls itself: it calls itself straight back. Too long to \
         fit, it scrolls with the cursor: drag it, or zoom it with the wheel; the line \
         along its foot shows which part is in view.",
    ]
}

/// Width of the keys' column in the help.
const KEY_WIDTH: f32 = 84.0;

/// Widest the help grows.
const HELP_WIDTH: f32 = 820.0;

/// Roughly how wide a character of the help's text is, in points, to balance its
/// columns before they are laid out.
const HELP_CHAR: f32 = 5.6;

/// The keys that apply to the view open, on a sheet down the right of `area`, in two
/// columns where they fit: whole at 1600 × 1000, and in a smaller window scrolling,
/// with a cue at its foot while more lies below.
pub(crate) fn key_help(ctx: &egui::Context, area: egui::Rect, open: &mut bool, browsing: bool) {
    let margin = 12.0;
    let width = (area.width() - 2.0 * margin).clamp(240.0, HELP_WIDTH);
    let height = (area.height() - 2.0 * margin).max(160.0);
    let columns = if width >= 560.0 { 2 } else { 1 };
    let sections: Vec<(&str, Vec<(&str, &str)>)> = KEYS
        .iter()
        .map(|(title, keys)| {
            let keys = keys
                .iter()
                .filter(|(.., when)| when.applies(browsing))
                .map(|&(key, what, _)| (key, what))
                .collect();
            (*title, keys)
        })
        .filter(|(_, keys): &(_, Vec<_>)| !keys.is_empty())
        .collect();
    let placed = balance(&sections, columns, width);
    egui::Area::new("key_help".into())
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(egui::pos2(area.max.x - margin, area.min.y + margin))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(11, 13, 19))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(60)))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(width - 28.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        let title = if browsing {
                            "Keys while browsing: no journey open"
                        } else {
                            "Keys in a journey"
                        };
                        ui.label(egui::RichText::new(title).strong().size(text::TITLE));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("close (H)").clicked() {
                                *open = false;
                            }
                        });
                    });
                    let body = |ui: &mut egui::Ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for (color, what) in color_key(browsing) {
                                ui.label(egui::RichText::new("●").color(color));
                                ui.label(egui::RichText::new(what).size(text::SMALL).color(MUTED));
                                ui.add_space(8.0);
                            }
                        });
                        ui.label(
                            egui::RichText::new(if browsing {
                                "Browsing, a click selects a function and moves nothing. \
                                 Enter opens its journey."
                            } else {
                                "In a journey, the view gathers round where it starts and \
                                 shows only the journey; a click moves its cursor and nothing \
                                 else. Esc leaves it for browsing."
                            })
                            .size(text::SMALL)
                            .color(egui::Color32::WHITE),
                        );
                        for line in reading(browsing) {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(*line).size(text::SMALL).color(MUTED),
                                )
                                .wrap(),
                            );
                        }
                        ui.add_space(4.0);
                        ui.columns(columns, |cols| {
                            for (col, indices) in cols.iter_mut().zip(&placed) {
                                col.spacing_mut().item_spacing.y = 1.0;
                                for &index in indices {
                                    let (title, keys) = &sections[index];
                                    key_section(col, title, keys);
                                }
                            }
                        });
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

/// Which sections of keys go in which of `columns` on a sheet `width` wide: each to
/// the shortest column so far, in order, by the lines its keys wrap to.
fn balance(sections: &[(&str, Vec<(&str, &str)>)], columns: usize, width: f32) -> Vec<Vec<usize>> {
    #[expect(clippy::cast_precision_loss, reason = "a column count")]
    let column_width = (width - 40.0) / columns as f32 - KEY_WIDTH - 8.0;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a positive count of characters"
    )]
    let per_line = (column_width / HELP_CHAR).max(10.0) as usize;
    let mut placed: Vec<Vec<usize>> = vec![Vec::new(); columns];
    let mut lengths = vec![0; columns];
    for (index, (_, keys)) in sections.iter().enumerate() {
        let shortest = (0..columns).min_by_key(|&c| lengths[c]).unwrap_or(0);
        placed[shortest].push(index);
        lengths[shortest] += 2 + keys
            .iter()
            .map(|(_, what)| what.chars().count().div_ceil(per_line))
            .sum::<usize>();
    }
    placed
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
        egui::Color32::WHITE,
    );
    let at = egui::pos2(
        foot.center().x - galley.size().x / 2.0,
        foot.max.y - galley.size().y - 4.0,
    );
    let pill = egui::Rect::from_min_size(at, galley.size()).expand2(PILL_PAD);
    ui.painter().rect_filled(pill, 4.0, PILL);
    ui.painter().galley(at, galley, egui::Color32::WHITE);
}

/// One group of keys, each key in a fixed column and what it does wrapping beside it.
fn key_section(ui: &mut egui::Ui, title: &str, keys: &[(&str, &str)]) {
    ui.label(egui::RichText::new(title).strong().size(text::BODY));
    for (key, what) in keys {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(KEY_WIDTH, 13.0),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    ui.set_min_width(KEY_WIDTH);
                    ui.label(
                        egui::RichText::new(*key)
                            .monospace()
                            .size(text::SMALL)
                            .color(egui::Color32::WHITE),
                    );
                },
            );
            ui.add(
                egui::Label::new(egui::RichText::new(*what).size(text::SMALL).color(MUTED)).wrap(),
            );
        });
    }
    ui.add_space(6.0);
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
        }
    }
}
