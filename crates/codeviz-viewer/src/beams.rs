//! Beams tie the journey's order to the map. While they show (Z), the journey docks
//! along the bottom, full width, with a strip across its top edge holding every
//! disclosed call once, in the order they run. Pale beams rise out of the strip to
//! the nodes on the graph: brightest from the cursor's call, fainter from the two
//! calls either side of it in time, and dotted from the other calls of the cursor's
//! function. Each beam leaves its tick straight up and bends into its node, and the
//! beams turn at staggered heights so they never cross near the strip.

use std::collections::{HashMap, HashSet};

use bevy_egui::egui;
use codeviz_core::SpanId;

use crate::{explorer::Explorer, journey_panel::JourneyPanel};

/// Height of the strip of calls along the top of the journey.
pub(crate) const STRIP: f32 = 20.0;
/// How many calls either side of the cursor's get a beam.
const REACH: usize = 2;
/// Height each beam rises before it may bend, and the step between beams' bends.
const RISE: f32 = 12.0;
const RISE_STEP: f32 = 11.0;

/// One call on the strip.
#[derive(Debug, Clone)]
pub(crate) struct Tick {
    pub(crate) span: SpanId,
    pub(crate) rect: egui::Rect,
    /// The node it runs on, if it is on the graph.
    pub(crate) tile: Option<usize>,
}

/// The disclosed calls in the order they run, given where the flame graph lays them
/// out: a call starts where its caller does, so a caller comes before its first call.
pub(crate) fn sequence<S: Copy>(placed: &HashMap<S, (f32, f32, usize)>) -> Vec<S> {
    let mut calls: Vec<(f32, usize, S)> = placed
        .iter()
        .map(|(&span, &(left, _, row))| (left, row, span))
        .collect();
    calls.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    calls.into_iter().map(|(_, _, span)| span).collect()
}

/// The disclosed calls in the order they run, each with its node, and which is the
/// cursor's: the call around it when it has no bar of its own.
pub(crate) fn calls(explorer: &Explorer) -> (Vec<(SpanId, Option<usize>)>, Option<usize>) {
    let track = explorer.track();
    let Some(journey) = track.journey() else {
        return (Vec::new(), None);
    };
    let map = explorer.map();
    let calls: Vec<(SpanId, Option<usize>)> =
        sequence(&explorer.disclosure().layout(journey.root()).0)
            .into_iter()
            .map(|span| {
                let tile = track.unit_id(span).and_then(|u| map.tile_of(u));
                (span, tile)
            })
            .collect();
    let cursor = std::iter::successors(track.selected(), |&s| track.span(s)?.parent)
        .find_map(|span| calls.iter().position(|&(c, _)| c == span));
    (calls, cursor)
}

/// The nodes the beams reach, so the graph can name them over the rest.
pub(crate) fn lit(explorer: &Explorer) -> HashSet<usize> {
    let (calls, cursor) = calls(explorer);
    let tiles: Vec<Option<usize>> = calls.iter().map(|&(_, tile)| tile).collect();
    cursor.map_or_else(HashSet::new, |cursor| {
        beamed(&tiles, cursor)
            .into_iter()
            .filter_map(|(index, _)| tiles[index])
            .collect()
    })
}

/// Which calls on the strip get a beam, and how strongly: the cursor's in full, its
/// neighbors in time fading with distance, and the other calls of the same node
/// faintly.
pub(crate) fn beamed(tiles: &[Option<usize>], cursor: usize) -> Vec<(usize, Beam)> {
    let near = cursor.saturating_sub(REACH)..=(cursor + REACH).min(tiles.len().saturating_sub(1));
    let mut beams: Vec<(usize, Beam)> = near
        .clone()
        .map(|index| (index, Beam::Near(index.abs_diff(cursor))))
        .collect();
    if let Some(tile) = tiles.get(cursor).copied().flatten() {
        beams.extend(
            tiles
                .iter()
                .enumerate()
                .filter(|&(index, &t)| t == Some(tile) && !near.contains(&index))
                .map(|(index, _)| (index, Beam::Again)),
        );
    }
    beams
}

/// Why a call gets a beam: it is so many calls from the cursor's, or it runs the
/// cursor's function again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Beam {
    Near(usize),
    Again,
}

impl Beam {
    /// How strongly it draws: the cursor's in full, fading with distance.
    fn strength(self) -> f32 {
        match self {
            Beam::Near(0) => 1.0,
            Beam::Near(1) => 0.65,
            Beam::Near(_) => 0.4,
            Beam::Again => 0.32,
        }
    }
}

/// How high up each beam bends, in steps, given where it leaves the strip and where
/// its node is across. Beams bending right turn lower the further right they leave,
/// beams bending left lower the further left, so each turns clear of the beams still
/// rising beside it.
pub(crate) fn bend_levels(beams: &[(f32, f32)]) -> Vec<usize> {
    let mut levels = vec![0; beams.len()];
    let mut by_x: Vec<usize> = (0..beams.len()).collect();
    by_x.sort_by(|&a, &b| beams[a].0.total_cmp(&beams[b].0));
    let (right, left): (Vec<usize>, Vec<usize>) =
        by_x.into_iter().partition(|&i| beams[i].1 > beams[i].0);
    for (rank, &i) in right.iter().enumerate() {
        levels[i] = right.len() - 1 - rank;
    }
    for (rank, &i) in left.iter().enumerate() {
        levels[i] = rank;
    }
    levels
}

/// A beam's path: straight up out of the strip by `rise`, then a smooth bend that
/// comes into the node from below, stopping `gap` short of it.
pub(crate) fn path(from: egui::Pos2, rise: f32, to: egui::Pos2, gap: f32) -> Vec<egui::Pos2> {
    let bend = egui::pos2(from.x, from.y - rise);
    // Short tangents, so a far node is reached across the map rather than by a
    // tall arc around it.
    let tangent = ((bend.y - to.y).abs() * 0.4).clamp(20.0, 140.0);
    let controls = [
        bend,
        egui::pos2(bend.x, bend.y - tangent),
        egui::pos2(to.x, to.y + tangent),
        to,
    ];
    let mut points = vec![from];
    points.extend((0..=32).map(|i| cubic(controls, i as f32 / 32.0)));
    // Short of the node, so it shows.
    while points.len() > 2 && points.last().is_some_and(|p| p.distance(to) < gap) {
        points.pop();
    }
    points
}

/// A point along a cubic Bézier curve, by repeated blending.
fn cubic(controls: [egui::Pos2; 4], at: f32) -> egui::Pos2 {
    let [p0, p1, p2, p3] = controls;
    let (q0, q1, q2) = (p0.lerp(p1, at), p1.lerp(p2, at), p2.lerp(p3, at));
    q0.lerp(q1, at).lerp(q1.lerp(q2, at), at)
}

/// The beams from the strip to their nodes, given where a node is on screen and how
/// far its edge is from its middle.
pub(crate) fn shapes(
    ticks: &[Tick],
    cursor: Option<usize>,
    node_at: impl Fn(usize) -> Option<(egui::Pos2, f32)>,
) -> Vec<egui::Shape> {
    let Some(cursor) = cursor else {
        return Vec::new();
    };
    let tiles: Vec<Option<usize>> = ticks.iter().map(|t| t.tile).collect();
    let beams: Vec<(egui::Pos2, (egui::Pos2, f32), Beam)> = beamed(&tiles, cursor)
        .into_iter()
        .filter_map(|(index, beam)| {
            let tick = ticks.get(index)?;
            let node = node_at(tick.tile?)?;
            Some((tick.rect.center_top(), node, beam))
        })
        .collect();
    let levels = bend_levels(
        &beams
            .iter()
            .map(|(from, (to, _), _)| (from.x, to.x))
            .collect::<Vec<_>>(),
    );
    let mut shapes = Vec::new();
    // The faintest first, so the cursor's draws over the rest.
    let mut order: Vec<usize> = (0..beams.len()).collect();
    order.sort_by(|&a, &b| beams[a].2.strength().total_cmp(&beams[b].2.strength()));
    for i in order {
        let (from, (to, edge), beam) = beams[i];
        let strength = beam.strength();
        let rise = RISE + RISE_STEP * levels[i] as f32;
        let gap = edge + 3.0;
        let width = 1.0 + strength * 1.2;
        let color = egui::Color32::WHITE.gamma_multiply(strength * 0.92);
        let points = path(from, rise, to, gap);
        // A soft halo sets a beam apart from the straight links it crosses.
        shapes.push(egui::Shape::line(
            points.clone(),
            egui::Stroke::new(
                width + 5.0,
                egui::Color32::WHITE.gamma_multiply(strength * 0.07),
            ),
        ));
        if beam == Beam::Again {
            // The same function another time: dotted.
            shapes.extend(egui::Shape::dotted_line(&points, color, 5.0, width * 0.75));
        } else {
            shapes.push(egui::Shape::line(points, egui::Stroke::new(width, color)));
        }
        shapes.push(egui::Shape::circle_stroke(
            to,
            gap,
            egui::Stroke::new(width, color),
        ));
    }
    shapes
}

/// How strongly the ordinary graph's links draw: they step back while beams show.
pub(crate) fn link_strength(panel: &JourneyPanel) -> f32 {
    if panel.beaming() { 0.55 } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_run_in_the_order_the_flame_graph_lays_them_out() {
        // The root, its first call and that call's own, then the root's second call.
        let placed = HashMap::from([
            ('a', (0.0, 3.0, 0)),
            ('b', (0.0, 2.0, 1)),
            ('c', (0.0, 1.0, 2)),
            ('d', (2.0, 1.0, 1)),
        ]);
        assert_eq!(sequence(&placed), ['a', 'b', 'c', 'd']);
    }

    #[test]
    fn the_cursor_and_two_either_side_get_beams_and_its_other_calls_faint_ones() {
        let tiles = [
            Some(0),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(5),
            Some(2),
        ];
        let beams = beamed(&tiles, 2);
        assert_eq!(
            beams,
            [
                (0, Beam::Near(2)),
                (1, Beam::Near(1)),
                (2, Beam::Near(0)),
                (3, Beam::Near(1)),
                (4, Beam::Near(2)),
                (6, Beam::Again),
            ]
        );
        assert_eq!(beamed(&tiles, 0).len(), 3, "none before the first call");
    }

    #[test]
    fn beams_bend_clear_of_their_neighbors() {
        // Three bending right and two bending left, as they leave the strip.
        let levels = bend_levels(&[
            (10.0, 300.0),
            (20.0, 200.0),
            (30.0, 400.0),
            (40.0, 5.0),
            (50.0, 0.0),
        ]);
        assert_eq!(levels, [2, 1, 0, 0, 1]);
    }

    #[test]
    fn a_beam_rises_straight_then_stops_short_of_its_node() {
        let from = egui::pos2(100.0, 500.0);
        let to = egui::pos2(300.0, 100.0);
        let points = path(from, 30.0, to, 10.0);
        assert_eq!(points[0], from);
        assert_eq!(points[1], egui::pos2(100.0, 470.0));
        let end = *points.last().unwrap();
        assert!(end.distance(to) >= 10.0 && end.distance(to) < 30.0);
        assert!(
            points.windows(2).all(|w| w[1].y <= w[0].y + 0.01),
            "always up"
        );
    }
}
