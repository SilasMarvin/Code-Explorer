//! How far the reveal reaches, as a slim bar at the right edge of the graph, like a
//! volume indicator: a segment per step out from where it starts, filled to the
//! current one, the top segment being everything. It shows when `-`, `=`, `L` or `G`
//! turn the dial or change what it measures, fades out after a moment, and shows
//! again while the pointer is over it.

use bevy_egui::egui;

use crate::{
    explorer::Explorer,
    lens::{Measure, Source},
    ui::{MUTED, node_name},
};

/// Seconds the meter stays fully shown after a key turns the dial.
const HOLD: f32 = 1.6;
/// Seconds it then takes to fade out.
const FADE: f32 = 0.4;
/// The bar's width, and its tallest.
const WIDTH: f32 = 8.0;
const TALLEST: f32 = 220.0;
/// Space between segments, while they are tall enough to spare it.
const GAP: f32 = 2.0;
/// From the edge of the graph.
const INSET: f32 = 14.0;
/// Room under the bar for its label.
const LABEL: f32 = 34.0;

/// How opaque the meter is `now`, given when a key last turned the dial.
pub(crate) fn opacity(shown_at: Option<f32>, now: f32, hovered: bool) -> f32 {
    if hovered {
        return 1.0;
    }
    let Some(at) = shown_at else {
        return 0.0;
    };
    let since = now - at;
    if since < 0.0 {
        return 1.0;
    }
    (1.0 - (since - HOLD) / FADE).clamp(0.0, 1.0)
}

/// What the meter says: where the dial stands out of its top, in what, from where.
pub(crate) fn caption(level: u32, top: u32, measure: Measure, from: &str) -> String {
    let unit = match measure {
        Measure::Calls => "calls",
        Measure::Space => "steps",
        Measure::Blend => "blended steps",
    };
    if level >= top {
        format!("everything · {top} {unit} from {from}")
    } else {
        format!("{level} of {top} {unit} from {from}")
    }
}

/// What the reveal starts from, in words.
fn origin(explorer: &Explorer) -> String {
    match explorer.lens().source {
        Source::Focus => explorer
            .graph_focus()
            .map_or_else(|| "the focus".to_owned(), |tile| node_name(explorer, tile)),
        Source::Journey => "the journey".to_owned(),
    }
}

/// Draws the meter at the right edge of `clear`, the part of the graph nothing else
/// covers, when a key turned the dial lately or the pointer is over it.
pub(crate) fn draw(
    ctx: &egui::Context,
    clear: egui::Rect,
    explorer: &Explorer,
    shown_at: Option<f32>,
    now: f32,
) {
    let height = (clear.height() - LABEL - 2.0 * INSET).min(TALLEST);
    if height < 40.0 {
        return;
    }
    let bar = egui::Rect::from_center_size(
        egui::pos2(
            clear.max.x - INSET - WIDTH / 2.0,
            clear.center().y - LABEL / 2.0,
        ),
        egui::vec2(WIDTH, height),
    );
    let hovered = ctx
        .pointer_hover_pos()
        .is_some_and(|p| bar.expand2(egui::vec2(10.0, 6.0)).contains(p));
    let alpha = opacity(shown_at, now, hovered);
    if alpha <= 0.0 {
        return;
    }
    let lens = explorer.lens();
    let top = explorer.lens_top();
    let level = lens.level(top);
    let fade = |color: egui::Color32| color.gamma_multiply(alpha);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Middle,
        egui::Id::new("depth_meter"),
    ));

    painter.rect_filled(
        bar.expand(4.0),
        6.0,
        fade(egui::Color32::from_rgba_premultiplied(11, 13, 19, 215)),
    );
    let steps = top as f32;
    let gap = if height / steps - GAP >= 3.0 {
        GAP
    } else {
        0.0
    };
    let pitch = (height + gap) / steps;
    for step in 1..=top {
        // Level one at the bottom, everything at the top.
        let bottom = bar.max.y - (step - 1) as f32 * pitch;
        let segment = egui::Rect::from_min_max(
            egui::pos2(bar.min.x, bottom - pitch + gap),
            egui::pos2(bar.max.x, bottom),
        );
        let color = if step <= level {
            egui::Color32::from_gray(235)
        } else {
            egui::Color32::from_gray(62)
        };
        painter.rect_filled(segment, 1.5, fade(color));
    }

    let text = caption(level, top, lens.measure, &origin(explorer));
    let galley = painter.layout(
        text,
        egui::FontId::proportional(12.0),
        fade(egui::Color32::WHITE),
        (clear.width() - 2.0 * INSET).max(80.0),
    );
    let at = egui::pos2(bar.max.x - galley.size().x, bar.max.y + 10.0);
    let backing = egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(6.0, 3.0));
    painter.rect_filled(
        backing,
        6.0,
        fade(egui::Color32::from_rgba_premultiplied(11, 13, 19, 215)),
    );
    painter.galley(at, galley, fade(MUTED));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_then_fades_unless_hovered() {
        assert!((opacity(Some(10.0), 10.5, false) - 1.0).abs() < f32::EPSILON);
        let fading = opacity(Some(10.0), 10.0 + HOLD + FADE / 2.0, false);
        assert!(fading > 0.0 && fading < 1.0, "halfway through the fade");
        assert!(
            opacity(Some(10.0), 12.5, false).abs() < f32::EPSILON,
            "gone"
        );
        assert!(opacity(None, 0.0, false).abs() < f32::EPSILON);
        assert!(
            (opacity(None, 0.0, true) - 1.0).abs() < f32::EPSILON,
            "hovering shows it"
        );
    }

    #[test]
    fn says_where_the_dial_stands_and_from_what() {
        assert_eq!(
            caption(2, 6, Measure::Calls, "orders::place"),
            "2 of 6 calls from orders::place"
        );
        assert_eq!(
            caption(6, 6, Measure::Space, "the changes"),
            "everything · 6 steps from the changes"
        );
    }
}
