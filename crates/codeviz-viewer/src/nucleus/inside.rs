//! A module expanded: every one of its functions drawn, in rows across the view
//! wider apart along a row than between rows, so each name has room beside its own
//! dot, with the calls among them. It opens where it is, browsing or in a journey.

use std::collections::HashSet;

use bevy::prelude::*;
use codeviz_layout::{ForceGraph, atlas};

use super::{Arrangement, Flow, Link, Nucleus, view_rotation};

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
        // Spread over the whole of it, its dots are drawn their full size.
        now.dots.remove(&cluster);
        if let Some(ball) = now.balls.get(cluster).copied() {
            let at: Vec<(usize, Vec3)> = members.iter().map(|&t| (t, self.offsets[t])).collect();
            for (tile, offset) in spread(&at, None, atlas::disc(ball.radius)) {
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

    /// The module expanded, once a re-centering ends: its sphere's radius, and each
    /// of its functions with where it sits across and up the usual view from the
    /// sphere's middle, in the world.
    pub(crate) fn expanded_layout(&self) -> Option<(f32, Vec<(usize, Vec2)>)> {
        let cluster = self.expanded?;
        let ball = self.now.balls.get(cluster)?;
        let rotation = view_rotation();
        let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
        let dots = (0..self.now.points.len())
            .filter(|&t| self.opened_up(t) && self.placed(t))
            .map(|t| {
                let at = self.now.points[t] - ball.center;
                (t, Vec2::new(at.dot(right), at.dot(up)))
            })
            .collect();
        Some((ball.radius, dots))
    }

    /// The module a function lives in.
    pub(crate) fn module_of(&self, tile: usize) -> Option<usize> {
        self.module_of.get(tile).copied().flatten()
    }
}

/// Where `members`, each a tile and where it sits about its module's middle, go when
/// the module opens: on [`lattice`]'s places over a disc of radius `reach` square to
/// the usual view, each as near where it sat, against the others, as is free; `first`
/// in the middle, then outwards from it.
pub(super) fn spread(
    members: &[(usize, Vec3)],
    first: Option<usize>,
    reach: f32,
) -> Vec<(usize, Vec3)> {
    let rotation = view_rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    let places = lattice(members.len(), reach, ROW_WIDE);
    let flat: Vec<Vec2> = members
        .iter()
        .map(|&(tile, at)| {
            if Some(tile) == first {
                Vec2::ZERO
            } else {
                Vec2::new(at.dot(right), at.dot(up))
            }
        })
        .collect();
    let far = flat
        .iter()
        .map(|p| p.length())
        .fold(0.0, f32::max)
        .max(1e-3);
    let mut order: Vec<usize> = (0..members.len()).collect();
    order.sort_by(|&a, &b| {
        (Some(members[a].0) != first)
            .cmp(&(Some(members[b].0) != first))
            .then(flat[a].length().total_cmp(&flat[b].length()))
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
