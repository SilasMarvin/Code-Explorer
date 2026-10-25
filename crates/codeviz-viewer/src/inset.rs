//! A lens on a crowded cursor. Where the nodes of the cursor's call and the call just
//! after it sit close together in a crowd of the map's nodes, which way one lies
//! from the other can't be read; a small inset at the map's lower left shows that
//! patch magnified, each of those calls numbered in its node's color, the cursor's in
//! white, with a ring and a leader from the patch on the map.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::{
    explorer::Explorer,
    floors::Floors,
    graph_view,
    ui::{MUTED, ON_FILL, PILL, PILL_PAD, egui_color},
};

/// A node numbered in the lens: where it is on the map, and its calls' numbers.
struct Spot {
    tile: usize,
    numbers: String,
    current: bool,
    at: egui::Pos2,
}

/// How close on screen two of the cursor's nodes crowd before the lens shows: about
/// a badge and its name, as the map's labels round a crowd push one off its node.
const NEAR: f32 = 60.0;
/// How many of the map's nodes round the cursor's make a crowd.
const CROWD: usize = 8;
/// The most the lens magnifies.
const MOST: f32 = 16.0;
/// How large the lens is on screen.
const INSET: egui::Vec2 = egui::vec2(220.0, 150.0);

/// The nodes of the cursor's call and the call after it in time, where the two crowd
/// together on screen; none when they stand apart.
fn spots(
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    floors: &Floors,
) -> Vec<Spot> {
    let Some(cursor) = floors.cursor.filter(|_| floors.showing()) else {
        return Vec::new();
    };
    let mut spots: Vec<Spot> = Vec::new();
    // Only the call after the cursor's: where the journey goes next. The call before
    // it beside the cursor would read as the next.
    for index in [cursor, cursor + 1] {
        let Some(call) = floors.calls.get(index) else {
            continue;
        };
        let current = index == cursor;
        if let Some(spot) = spots.iter_mut().find(|s| s.tile == call.tile) {
            spot.numbers = format!("{}, {}", spot.numbers, call.number);
            spot.current |= current;
            continue;
        }
        let Some(at) = explorer.tile_point(call.tile).and_then(to_screen) else {
            continue;
        };
        spots.push(Spot {
            tile: call.tile,
            numbers: call.number.clone(),
            current,
            at,
        });
    }
    // Only those crowding the cursor's, and only in a crowd of the map's nodes: on a
    // sparse map a neighbor close by still reads as it is.
    let Some(here) = spots.iter().find(|s| s.current).map(|s| s.at) else {
        return Vec::new();
    };
    spots.retain(|s| s.current || s.at.distance(here) < NEAR);
    let around = (0..explorer.map().tiles.len())
        .filter_map(|tile| explorer.tile_point(tile).and_then(to_screen))
        .filter(|at| at.distance(here) < NEAR)
        .count();
    if around < CROWD {
        return Vec::new();
    }
    let points: Vec<egui::Pos2> = spots.iter().map(|s| s.at).collect();
    if crowded(&points, NEAR) {
        spots
    } else {
        Vec::new()
    }
}

/// Whether any two of `points` sit closer than `near`.
pub(crate) fn crowded(points: &[egui::Pos2], near: f32) -> bool {
    points
        .iter()
        .enumerate()
        .flat_map(|(i, a)| points[i + 1..].iter().map(move |b| a.distance(*b)))
        .any(|d| d < near)
}

/// How much the lens magnifies points `offsets` from their middle so they fill
/// `room` either way, at most `most`.
pub(crate) fn zoom(offsets: &[egui::Vec2], room: egui::Vec2, most: f32) -> f32 {
    offsets
        .iter()
        .map(|off| {
            let off = off.abs();
            (room.x / off.x.max(1e-3)).min(room.y / off.y.max(1e-3))
        })
        .fold(most, f32::min)
}

/// Draws the lens over the map in `painter`'s lower left, clear of the legend under
/// it, when the cursor's nodes crowd.
pub(crate) fn draw(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    floors: &Floors,
) {
    let lens = spots(to_screen, explorer, floors);
    if lens.len() < 2 {
        return;
    }
    #[expect(clippy::cast_precision_loss, reason = "a few spots")]
    let middle = (lens
        .iter()
        .fold(egui::Vec2::ZERO, |sum, s| sum + s.at.to_vec2())
        / lens.len() as f32)
        .to_pos2();
    // As far as fits them in the inset with room for their numbers to the right.
    let offsets: Vec<egui::Vec2> = lens.iter().map(|s| s.at - middle).collect();
    let room = egui::vec2(INSET.x / 2.0 - 60.0, INSET.y / 2.0 - 24.0);
    let zoom = zoom(&offsets, room, MOST);
    let clip = painter.clip_rect();
    let inset = egui::Rect::from_min_size(
        egui::pos2(clip.left() + 16.0, clip.bottom() - 250.0 - INSET.y),
        INSET,
    );
    let reach = INSET.min_elem() / 2.0 / zoom;
    let line = egui::Stroke::new(1.0, egui::Color32::from_white_alpha(110));
    // Where it looks on the map, and the leader from there to the inset.
    painter.circle_stroke(middle, reach, line);
    painter.line_segment(
        [
            middle + (inset.center() - middle).normalized() * reach,
            inset.center_top(),
        ],
        line,
    );
    painter.rect_filled(
        inset,
        6.0,
        egui::Color32::from_rgb(12, 14, 19).gamma_multiply(0.94),
    );
    painter.rect_stroke(inset, 6.0, line, egui::StrokeKind::Inside);
    let centre = inset.center() + egui::vec2(-30.0, 6.0);
    let place = |at: egui::Pos2| centre + (at - middle) * zoom;
    // The map's own nodes round them, faint, for bearings.
    let lens_painter = painter.with_clip_rect(inset.shrink(4.0));
    for tile in 0..explorer.map().tiles.len() {
        if let Some(at) = explorer.tile_point(tile).and_then(to_screen)
            && at.distance(middle) < reach * 1.5
        {
            lens_painter.circle_filled(place(at), 2.0, egui::Color32::from_white_alpha(40));
        }
    }
    for spot in &lens {
        let fill = egui_color(graph_view::node_color(explorer, None, spot.tile));
        let at = place(spot.at);
        lens_painter.circle_filled(at, 5.0, fill);
        let text_color = if spot.current {
            egui::Color32::WHITE
        } else {
            ON_FILL
        };
        let galley = lens_painter.layout_no_wrap(
            format!(" {} ", spot.numbers),
            egui::FontId::proportional(12.0),
            text_color,
        );
        let min = at + egui::vec2(8.0, -galley.size().y / 2.0);
        let pill = egui::Rect::from_min_size(min, galley.size()).expand2(PILL_PAD);
        let (back, border) = if spot.current {
            (PILL, egui::Color32::WHITE)
        } else {
            (fill, fill)
        };
        lens_painter.rect_filled(pill, 4.0, back);
        lens_painter.rect_stroke(
            pill,
            4.0,
            egui::Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        lens_painter.galley(min, galley, text_color);
    }
    let here = lens
        .iter()
        .find(|s| s.current)
        .map_or("", |s| s.numbers.as_str());
    painter.text(
        inset.left_top() + egui::vec2(8.0, 6.0),
        egui::Align2::LEFT_TOP,
        format!("near {here} · ×{zoom:.0}"),
        egui::FontId::proportional(10.5),
        MUTED,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_crowd_needs_the_lens() {
        let apart = [egui::pos2(0.0, 0.0), egui::pos2(100.0, 0.0)];
        assert!(!crowded(&apart, NEAR));
        assert!(!crowded(&[egui::pos2(0.0, 0.0)], NEAR));
        assert!(crowded(
            &[egui::pos2(0.0, 0.0), egui::pos2(40.0, 30.0)],
            NEAR
        ));
    }

    #[test]
    fn the_lens_fits_its_crowd_and_no_further() {
        let room = egui::vec2(50.0, 50.0);
        assert!((zoom(&[egui::vec2(5.0, 0.0)], room, 16.0) - 10.0).abs() < 1e-4);
        assert!(
            (zoom(&[egui::vec2(0.5, 0.5)], room, 16.0) - 16.0).abs() < 1e-4,
            "at most"
        );
    }
}
