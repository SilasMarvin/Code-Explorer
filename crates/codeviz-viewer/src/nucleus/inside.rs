//! A module expanded: every one of its functions drawn, in rows across the view
//! wider apart along a row than between rows, so each name has room beside its own
//! dot, with the calls among them. It opens where it is, browsing or in a journey; in
//! a journey, the journey's functions keep the places it gave them, and the rest fill
//! in round them.

use std::collections::HashSet;

use bevy::prelude::*;
use codeviz_layout::{ForceGraph, atlas};

use super::{Arrangement, Body, Flow, Link, Nucleus, view_rotation};

/// How much further apart two places along a row are than two rows.
pub(super) const ROW_WIDE: f32 = 4.0;

impl Nucleus {
    /// Opens `cluster`, the module expanded, in `now`: every one of its functions
    /// drawn, spread in rows where it is, the same whatever is selected; and every
    /// call among them that nothing else names, drawn faintly.
    pub(super) fn open_out(&self, graph: &ForceGraph, cluster: usize, now: &mut Arrangement) {
        let members: Vec<usize> = (0..graph.positions.len())
            .filter(|&t| graph.cluster(t) == Some(cluster))
            .collect();
        // Spread over the whole of it, its dots are drawn their full size, and its
        // sphere drawn open, though nothing on the journey lies in it.
        now.dots.remove(&cluster);
        if let Some(ball) = now.balls.get_mut(cluster)
            && matches!(ball.body, Body::Closed | Body::Faint)
        {
            ball.body = Body::Open;
        }
        if let Some(ball) = now.balls.get(cluster).copied() {
            // In a journey, what it lays out in the sphere keeps its place, at any
            // depth; the rest fill in round it.
            let journey = self.spread.as_ref().filter(|_| !self.browse);
            let kept: Vec<(usize, Vec3)> = journey
                .into_iter()
                .flat_map(|(_, placed)| &placed.points)
                .filter(|&(&t, _)| graph.cluster(t) == Some(cluster))
                .map(|(&t, &at)| (t, at - ball.center))
                .collect();
            let rest: Vec<(usize, Vec3)> = members
                .iter()
                .filter(|t| !kept.iter().any(|(k, _)| k == *t))
                .map(|&t| (t, self.offsets[t]))
                .collect();
            for (tile, offset) in
                kept.iter()
                    .copied()
                    .chain(spread(&rest, &kept, atlas::disc(ball.radius)))
            {
                now.points[tile] = ball.center + offset;
                now.shown[tile] = true;
            }
        }
        let mut named: HashSet<(usize, usize)> = now.links.iter().map(|l| (l.from, l.to)).collect();
        let inside: HashSet<usize> = members.into_iter().collect();
        for &(from, to) in &graph.calls {
            if from != to
                && inside.contains(&from)
                && inside.contains(&to)
                && named.insert((from, to))
            {
                now.links.push(Link {
                    from,
                    to,
                    flow: Flow::Inner,
                });
            }
        }
    }

    /// The module expanded, laid out as it opens browsing, whatever the view lights:
    /// its sphere's radius, and each of its functions with where it sits across and
    /// up the usual view from the sphere's middle, in the world. What it holds alone
    /// sets how big it expands, so it expands alike wherever it is.
    pub(crate) fn expanded_layout(&self) -> Option<(f32, Vec<(usize, Vec2)>)> {
        let cluster = self.expanded?;
        let radius = atlas::module_radius(*self.counts.get(cluster)?);
        let members: Vec<(usize, Vec3)> = (0..self.offsets.len())
            .filter(|&t| self.module_of(t) == Some(cluster))
            .map(|t| (t, self.offsets[t]))
            .collect();
        let rotation = view_rotation();
        let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
        let dots = spread(&members, &[], atlas::disc(radius))
            .into_iter()
            .map(|(t, at)| (t, Vec2::new(at.dot(right), at.dot(up))))
            .collect();
        Some((radius, dots))
    }

    /// The module a function lives in.
    pub(crate) fn module_of(&self, tile: usize) -> Option<usize> {
        self.module_of.get(tile).copied().flatten()
    }
}

/// Where `members`, each a tile and where it sits about its module's middle, go when
/// the module opens: on [`lattice`]'s places over a disc of radius `reach` square to
/// the usual view, those near a dot `kept` where it is aside, each as near where it
/// sat, against the others, as is free, the nearest the middle first.
pub(super) fn spread(
    members: &[(usize, Vec3)],
    kept: &[(usize, Vec3)],
    reach: f32,
) -> Vec<(usize, Vec3)> {
    let rotation = view_rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    let flat = |at: Vec3| Vec2::new(at.dot(right), at.dot(up));
    let kept: Vec<Vec2> = kept.iter().map(|&(_, at)| flat(at)).collect();
    // More places, closer, until enough are clear of the dots kept: no nearer one
    // than half the way to the next place.
    let mut count = members.len();
    let places = loop {
        let places = lattice(count, reach, ROW_WIDE);
        let apart = places
            .iter()
            .enumerate()
            .flat_map(|(i, a)| places[i + 1..].iter().map(move |b| a.distance(*b)))
            .fold(reach, f32::min);
        let clear: Vec<Vec2> = places
            .into_iter()
            .filter(|p| kept.iter().all(|k| k.distance(*p) >= apart / 2.0))
            .collect();
        if clear.len() >= members.len() || count > 4 * (members.len() + kept.len()) {
            break clear;
        }
        count += kept.len().max(1);
    };
    let flat: Vec<Vec2> = members.iter().map(|&(_, at)| flat(at)).collect();
    let far = flat
        .iter()
        .map(|p| p.length())
        .fold(0.0, f32::max)
        .max(1e-3);
    let mut order: Vec<usize> = (0..members.len()).collect();
    order.sort_by(|&a, &b| {
        flat[a]
            .length()
            .total_cmp(&flat[b].length())
            .then(members[a].0.cmp(&members[b].0))
    });
    let mut free = vec![true; places.len()];
    let mut spread = Vec::with_capacity(members.len());
    for index in order {
        let want = flat[index] / far * reach;
        let Some(place) = (0..places.len()).filter(|&p| free[p]).min_by(|&a, &b| {
            places[a]
                .distance_squared(want)
                .total_cmp(&places[b].distance_squared(want))
        }) else {
            break;
        };
        free[place] = false;
        spread.push((
            members[index].0,
            right * places[place].x + up * places[place].y,
        ));
    }
    spread
}

/// `count` places over a disc of radius `reach` about its middle: rows as close as
/// holds them all, `wide` times further apart along a row, as names run along, each
/// row half a place along from the next; the middle one first, then the nearest it.
pub(super) fn lattice(count: usize, reach: f32, wide: f32) -> Vec<Vec2> {
    spaced((count, count), reach, wide, 0.0)
}

/// As [`lattice`], up to `want` places but rows no nearer than `least` while that
/// leaves `need` places or more: fewer places, further apart, rather than more
/// crowded.
pub(super) fn spaced((need, want): (usize, usize), reach: f32, wide: f32, least: f32) -> Vec<Vec2> {
    let count = want.max(need);
    if count == 0 {
        return Vec::new();
    }
    let rows = |gap: f32| {
        let along = gap * wide;
        let mut places = Vec::new();
        #[expect(clippy::cast_possible_truncation, reason = "a row count")]
        let most = (reach / gap) as i32;
        for row in -most..=most {
            #[expect(clippy::cast_precision_loss, reason = "a row index")]
            let y = row as f32 * gap;
            let half = (reach * reach - y * y).max(0.0).sqrt();
            let shift = if row % 2 == 0 { 0.0 } else { along / 2.0 };
            #[expect(clippy::cast_possible_truncation, reason = "a place count")]
            let most = ((half + along) / along) as i32;
            for place in -most..=most {
                #[expect(clippy::cast_precision_loss, reason = "a place index")]
                let x = place as f32 * along + shift;
                if x.abs() <= half {
                    places.push(Vec2::new(x, y));
                }
            }
        }
        places
    };
    let mut gap = reach;
    let mut places = rows(gap);
    while gap > reach * 1e-3
        && (places.len() < need || (places.len() < count && gap * 0.97 >= least))
    {
        gap *= 0.97;
        places = rows(gap);
    }
    places.sort_by(|a, b| a.length_squared().total_cmp(&b.length_squared()));
    places.truncate(count);
    places
}
