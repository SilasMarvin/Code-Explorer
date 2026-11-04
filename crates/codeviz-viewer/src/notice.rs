//! A line said in the bar along the top until the next key, as why a key moved nothing: the
//! cursor already at the journey's start, no further call from the same caller, calls
//! folded past the depth shown; while none is, how many functions lit have no room
//! for a name, and how many callers I brings in no room to be drawn. Never over the
//! graph, where it would hide a name.

use bevy::{input::mouse::AccumulatedMouseScroll, prelude::*};
use bevy_egui::{EguiContexts, egui};

use crate::{
    colors::{MUTED, SURFACE, TEXT},
    explorer::Explorer,
    text,
};

/// Room round its text, inside the top bar.
const PAD: egui::Vec2 = egui::vec2(10.0, 2.0);

fn top_bar_id() -> egui::Id {
    egui::Id::new("top_bar_rect")
}

fn drawn_id() -> egui::Id {
    egui::Id::new("notice_rect")
}

fn unnamed_id() -> egui::Id {
    egui::Id::new("unnamed_note")
}

/// Notes where in the bar along the top the notice may sit: between what the bar
/// holds on its left and on its right.
pub(crate) fn note_top_bar(ctx: &egui::Context, free: egui::Rect) {
    ctx.data_mut(|d| d.insert_temp(top_bar_id(), free));
}

/// Notes how many functions lit and on screen have no room for a name this pass, of
/// how many, for the bar along the top to say while no notice is said.
pub(crate) fn note_unnamed(ctx: &egui::Context, unnamed: usize, lit: usize) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(unnamed_id(), (pass, unnamed, lit)));
}

/// Where the notice was drawn in the egui pass before `pass`, if it was.
pub(crate) fn drawn(ctx: &egui::Context, pass: u64) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp::<(u64, egui::Rect)>(drawn_id()))
        .filter(|&(at, _)| at + 1 == pass)
        .map(|(_, rect)| rect)
}

/// The notice on screen: which, and in which mode.
pub(crate) struct Said {
    count: u64,
    browsing: bool,
    /// Whether the next key, click or wheel turn, or a change of mode, has put it
    /// away.
    gone: bool,
}

/// Draws the explorer's last notice from when it is said until the next key, click
/// or wheel turn that does not say it again, or until the view leaves the journey or
/// enters one; while none shows, how many functions lit have no room for a name,
/// quietly. Nothing times it out: a still window draws nothing. Runs after the graph
/// is drawn, which counts the names left out.
pub(crate) fn draw(
    explorer: Res<Explorer>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut contexts: EguiContexts,
    mut said: Local<Option<Said>>,
) {
    let browsing = explorer.browsing();
    let notice = explorer.notice().filter(|&(count, _)| {
        match &mut *said {
            Some(said) if said.count == count => {
                let acted = keys.get_just_pressed().next().is_some()
                    || buttons.get_just_pressed().next().is_some()
                    || scroll.delta != Vec2::ZERO;
                said.gone |= acted || said.browsing != browsing;
            }
            // Said this frame, by the key just pressed.
            _ => {
                *said = Some(Said {
                    count,
                    browsing,
                    gone: false,
                });
            }
        }
        said.as_ref().is_some_and(|s| !s.gone)
    });
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let pass = ctx.cumulative_pass_nr();
    // Quietly, what is left out: names with no room, and callers I brings in with no
    // room to be drawn.
    let unnamed = ctx
        .data(|d| d.get_temp::<(u64, usize, usize)>(unnamed_id()))
        .filter(|&(at, unnamed, _)| at == pass && unnamed > 0)
        .map(|(_, unnamed, lit)| {
            format!("{unnamed} of {lit} lit names left out · zoom in, or point at one")
        });
    let waiting = Some(explorer.nucleus().outside_waiting())
        .filter(|&waiting| waiting > 0 && explorer.outside_shown())
        .map(|waiting| format!("{waiting} more call it with no room here: X shows them"));
    let left_out: Vec<String> = unnamed.into_iter().chain(waiting).collect();
    let (text, color, size) = match notice {
        Some((_, text)) => (text.to_owned(), TEXT, text::BODY),
        None if !left_out.is_empty() => (left_out.join(" · "), MUTED, text::SMALL),
        None => return,
    };
    let screen = ctx.viewport_rect();
    let free = ctx
        .data(|d| d.get_temp::<egui::Rect>(top_bar_id()))
        .unwrap_or(screen);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("notice"),
    ));
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(size), color);
    // In the middle of the bar's free stretch, or from its left where it is short.
    let at = egui::pos2(
        (free.center().x - galley.size().x / 2.0).max(free.min.x + PAD.x),
        free.center().y - galley.size().y / 2.0,
    );
    let backing = egui::Rect::from_min_size(at, galley.size()).expand2(PAD);
    painter.rect_filled(backing, 6.0, SURFACE);
    painter.galley(at, galley, color);
    ctx.data_mut(|d| d.insert_temp(drawn_id(), (pass, backing)));
}
