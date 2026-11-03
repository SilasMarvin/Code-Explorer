//! A line said for a moment in the bar along the top, as why a key moved nothing: the
//! cursor already at the journey's start, no further call from the same caller, calls
//! folded past the depth shown. Never over the graph, where it would hide a name.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::{
    colors::{SURFACE, TEXT},
    explorer::Explorer,
    text,
};

/// Seconds a notice shows for, at most. It keeps nothing drawing: it goes with the
/// first frame drawn after, which the window draws within a second when idle.
const HOLD: f32 = 2.0;
/// Room round its text, inside the top bar.
const PAD: egui::Vec2 = egui::vec2(10.0, 2.0);

fn top_bar_id() -> egui::Id {
    egui::Id::new("top_bar_rect")
}

fn drawn_id() -> egui::Id {
    egui::Id::new("notice_rect")
}

/// Notes where the bar along the top is, for the notice to sit in.
pub(crate) fn note_top_bar(ctx: &egui::Context, bar: egui::Rect) {
    ctx.data_mut(|d| d.insert_temp(top_bar_id(), bar));
}

/// Where the notice was drawn in the egui pass before `pass`, if it was.
pub(crate) fn drawn(ctx: &egui::Context, pass: u64) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp::<(u64, egui::Rect)>(drawn_id()))
        .filter(|&(at, _)| at + 1 == pass)
        .map(|(_, rect)| rect)
}

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
    let bar = ctx
        .data(|d| d.get_temp::<egui::Rect>(top_bar_id()))
        .unwrap_or(screen);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("notice"),
    ));
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(text::BODY),
        TEXT,
    );
    // In the middle of the bar along the top.
    let at = egui::pos2(
        screen.center().x - galley.size().x / 2.0,
        bar.center().y - galley.size().y / 2.0,
    );
    let backing = egui::Rect::from_min_size(at, galley.size()).expand2(PAD);
    painter.rect_filled(backing, 6.0, SURFACE);
    painter.galley(at, galley, TEXT);
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(drawn_id(), (pass, backing)));
}
