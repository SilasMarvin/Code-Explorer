//! A line said for a moment at the top of the graph, as why a key moved nothing: the
//! cursor already at the journey's start, no further call from the same caller, calls
//! folded past the depth shown.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::{explorer::Explorer, text};

/// Seconds a notice shows for, at most. It keeps nothing drawing: it goes with the
/// first frame drawn after, which the window draws within a second when idle.
const HOLD: f32 = 2.0;
/// Below the top bar.
const TOP: f32 = 44.0;

/// The notice on screen: which, when it was said, and in which mode.
pub(crate) struct Said {
    count: u64,
    at: f32,
    browsing: bool,
    /// Whether the next key or click, or a change of mode, has put it away.
    gone: bool,
}

/// Draws the explorer's last notice for a moment after it is said, until the next key
/// or click, or until the view leaves the journey or enters one.
pub(crate) fn draw(
    time: Res<Time>,
    explorer: Res<Explorer>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut contexts: EguiContexts,
    mut said: Local<Option<Said>>,
) {
    let Some((count, text)) = explorer.notice() else {
        return;
    };
    let now = time.elapsed_secs();
    let browsing = explorer.browsing();
    match &mut *said {
        Some(said) if said.count == count => {
            let acted = keys.get_just_pressed().next().is_some()
                || buttons.get_just_pressed().next().is_some();
            said.gone |= acted || said.browsing != browsing;
        }
        // Said this frame, by the key just pressed.
        _ => {
            *said = Some(Said {
                count,
                at: now,
                browsing,
                gone: false,
            });
        }
    }
    if said.as_ref().is_none_or(|s| s.gone || now - s.at > HOLD) {
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
        egui::FontId::proportional(text::TITLE),
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
