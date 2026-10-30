//! A line said for a moment at the top of the graph, as why a key moved nothing: the
//! cursor already at the journey's start, no further call from the same caller, calls
//! folded past the depth shown.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::explorer::Explorer;

/// Seconds a notice shows for, at least. It keeps nothing drawing: it goes with the
/// first frame drawn after, which the window draws within a second when idle.
const HOLD: f32 = 2.0;
/// Below the top bar.
const TOP: f32 = 44.0;

/// Draws the explorer's last notice for a moment after it is said.
pub(crate) fn draw(
    time: Res<Time>,
    explorer: Res<Explorer>,
    mut contexts: EguiContexts,
    mut said: Local<Option<(u64, f32)>>,
) {
    let Some((count, text)) = explorer.notice() else {
        return;
    };
    let now = time.elapsed_secs();
    let since = match *said {
        Some((seen, at)) if seen == count => at,
        _ => {
            *said = Some((count, now));
            now
        }
    };
    if now - since > HOLD {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let screen = ctx.viewport_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("notice"),
    ));
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(14.0),
        egui::Color32::WHITE,
    );
    let at = egui::pos2(
        screen.center().x - galley.size().x / 2.0,
        screen.min.y + TOP,
    );
    let backing = egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(10.0, 5.0));
    painter.rect_filled(
        backing,
        6.0,
        egui::Color32::from_rgba_premultiplied(11, 13, 19, 225),
    );
    painter.galley(at, galley, egui::Color32::WHITE);
}
