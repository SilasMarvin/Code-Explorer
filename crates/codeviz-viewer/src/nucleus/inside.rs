//! A module expanded: every one of its functions drawn, in rows across the view
//! wider apart along a row than between rows, so each name has room beside its own
//! dot, with the calls among them. In a journey the start's module grows several
//! times over, its functions placed by the calls among them, what is named in the
//! middle, callers leaning one way and the journey the other, the rest towards the
//! wall; the modules it names on a ring just outside the wall, in the ring's order.
//! Any other module opens where it is.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use codeviz_layout::{
    ForceGraph,
    atlas::{self, fibonacci},
};

use super::{
    Arrangement, Ball, Body, Flow, GROWTH, Laid, Link, Nucleus,
    journey::{clockwise, names_grid},
    view_rotation,
};

/// How far outside the expanded nucleus's wall the modules round it sit, against its
/// radius.
const RIM_GAP: f32 = 0.14;
/// Kept clear between two modules round the expanded nucleus, against its radius.
const RIM_MARGIN: f32 = 0.08;

impl Nucleus {
    /// Where the expanded nucleus puts its functions and the modules round it, worked
    /// out once for each start from everything it could name at any depth: a new
    /// depth, outside callers or the pointer moving then only names and draws, and
    /// moves nothing.
    pub(super) fn laid_out(&mut self, graph: &ForceGraph, modules: &Arrangement) -> Laid {
        let key = (modules.start, self.trail.whole.clone());
        if let Some(laid) = self.laid.as_ref().filter(|l| l.key == key) {
            return laid.clone();
        }
        let middle = modules.middle;
        let cluster_of = |tile: usize| graph.cluster(tile).unwrap_or(middle);
        let before = modules.balls[middle].radius;
        // Everything below the start at any depth, and every caller.
        let mut reach: HashMap<usize, i32> = HashMap::new();
        if let Some(start) = modules.start {
            reach.extend(
                self.trail
                    .whole
                    .iter()
                    .map(|(&t, &l)| (t, i32::try_from(l).unwrap_or(i32::MAX))),
            );
            for caller in graph.callers(start) {
                reach.entry(caller).or_insert(-1);
            }
            reach.insert(start, 0);
        }
        let members: Vec<usize> = (0..graph.positions.len())
            .filter(|&t| cluster_of(t) == middle)
            .collect();
        let calls: Vec<(usize, usize)> = graph
            .calls
            .iter()
            .copied()
            .filter(|&(a, b)| a != b && cluster_of(a) == middle && cluster_of(b) == middle)
            .collect();
        // From where they sit collapsed, in a sphere of radius one.
        let seeded: Vec<Vec3> = members.iter().map(|&t| self.offsets[t] / before).collect();
        let inside = lay_members(&members, &seeded, &calls, &reach);
        let inside = spread(
            &members.iter().copied().zip(inside).collect::<Vec<_>>(),
            modules.start,
            UNIT_DISC,
        );
        // Each module that could ever be named round it, clockwise in the same order
        // as round the nucleus collapsed, each as big as all it could name.
        let rim: Vec<usize> = modules
            .ring
            .iter()
            .copied()
            .filter(|&c| !modules.reserved[c].is_empty())
            .collect();
        let radii: Vec<f32> = rim.iter().map(|&c| modules.balls[c].radius).collect();
        let angles = rim_angles(&radii, before * GROWTH);
        let laid = Laid {
            key,
            inside,
            angles: rim.into_iter().zip(angles).collect(),
        };
        self.laid = Some(laid.clone());
        laid
    }

    /// The nucleus grown from `modules`, the module view, to several times its size,
    /// its functions laid out by the calls among them; the modules naming anything on
    /// a ring just outside its wall, at the bearings they had, shrunk; and every other
    /// module pushed out by as much as it grew. It names what the module view names.
    pub(super) fn arrange_inside(
        &self,
        graph: &ForceGraph,
        modules: Arrangement,
        laid: &Laid,
    ) -> Arrangement {
        let middle = modules.middle;
        let cluster_of = |tile: usize| graph.cluster(tile).unwrap_or(middle);
        let before = modules.balls[middle].radius;
        let core = before * GROWTH;
        // Every module the journey could name round it at any depth keeps its place
        // there; one naming nothing yet is left out, undrawn, where it stands.
        let rim: Vec<usize> = (0..self.homes.len())
            .filter(|&c| c != middle && !modules.reserved[c].is_empty())
            .collect();

        let mut balls = modules.balls.clone();
        for (cluster, ball) in balls.iter_mut().enumerate() {
            if cluster == middle {
                ball.radius = core;
            } else {
                ball.body = Body::Closed;
            }
        }
        place_rim(&mut balls, &rim, &laid.angles, core);

        let tiles = graph.positions.len();
        let mut points: Vec<Vec3> = (0..tiles).map(|t| balls[cluster_of(t)].center).collect();
        let mut shown = vec![false; tiles];
        for &(tile, at) in &laid.inside {
            points[tile] = at * core;
            shown[tile] = true;
        }
        for &cluster in &rim {
            let column = &modules.reserved[cluster];
            for (&tile, at) in column
                .iter()
                .zip(names_grid(column.len(), balls[cluster].radius))
            {
                points[tile] = balls[cluster].center + at;
                shown[tile] = modules.columns[cluster].contains(&tile);
            }
            if modules.columns[cluster].is_empty() {
                balls[cluster].body = Body::Closed;
            }
        }
        let calls_in: Vec<Link> = modules
            .links
            .iter()
            .copied()
            .filter(|l| l.flow == Flow::Outside)
            .collect();
        let outside = self.place_outside(
            &mut balls,
            (&mut points, &mut shown),
            (&calls_in, &modules.reserved),
            middle,
            &cluster_of,
        );
        Arrangement {
            expanded: true,
            balls,
            points,
            shown,
            outside,
            ..modules
        }
    }
}

/// [`lay_inside`] for a module's `members`, by tile, from `start`, with the `calls`
/// among them and how far each is from the start by `reach`, turned to the usual
/// view.
fn lay_members(
    members: &[usize],
    start: &[Vec3],
    calls: &[(usize, usize)],
    reach: &HashMap<usize, i32>,
) -> Vec<Vec3> {
    let local: HashMap<usize, usize> = members.iter().enumerate().map(|(i, &t)| (t, i)).collect();
    let calls: Vec<(usize, usize)> = calls.iter().map(|(a, b)| (local[a], local[b])).collect();
    let hops: Vec<Option<i32>> = members.iter().map(|t| reach.get(t).copied()).collect();
    // A nudge apart, so no two start in the same place.
    let start: Vec<Vec3> = start
        .iter()
        .enumerate()
        .map(|(i, &at)| at + fibonacci(i, start.len()) * 0.01)
        .collect();
    let rotation = view_rotation();
    lay_inside(
        &start,
        &calls,
        &hops,
        rotation * Vec3::X,
        rotation * Vec3::Z,
    )
}

/// Of the expanded start's module, grown to a sphere of radius one, the radius of the
/// disc its functions spread over.
const UNIT_DISC: f32 = 0.88;
/// How much further apart two places along a row are than two rows.
const ROW_WIDE: f32 = 4.0;

impl Nucleus {
    /// Opens `cluster`, the module expanded, in `now`: every one of its functions
    /// drawn, spread in rows where it is, unless it is the start's in a journey,
    /// which [`Self::laid_out`] lays out; and every call among them that nothing else
    /// names, drawn faintly.
    pub(super) fn open_out(&self, graph: &ForceGraph, cluster: usize, now: &mut Arrangement) {
        let members: Vec<usize> = (0..graph.positions.len())
            .filter(|&t| graph.cluster(t) == Some(cluster))
            .collect();
        if !now.expanded
            && let Some(ball) = now.balls.get(cluster).copied()
        {
            let at: Vec<(usize, Vec3)> = members.iter().map(|&t| (t, self.offsets[t])).collect();
            for (tile, offset) in spread(&at, now.start, atlas::disc(ball.radius)) {
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
    let places = lattice(members.len(), reach);
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
/// holds them all, [`ROW_WIDE`] times further apart along a row, as names run along,
/// each row half a place along from the next; the middle one first, then the nearest
/// it.
pub(super) fn lattice(count: usize, reach: f32) -> Vec<Vec2> {
    if count == 0 {
        return Vec::new();
    }
    let rows = |gap: f32| {
        let along = gap * ROW_WIDE;
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
    while places.len() < count && gap > reach * 1e-3 {
        gap *= 0.97;
        places = rows(gap);
    }
    places.sort_by(|a, b| a.length_squared().total_cmp(&b.length_squared()));
    places.truncate(count);
    places
}

/// Puts the modules in `rim` on a ring just outside the expanded nucleus's wall, as
/// the usual view sees it, each at its angle round it from `angles`; one without an
/// angle at its bearing.
fn place_rim(balls: &mut [Ball], rim: &[usize], angles: &HashMap<usize, f32>, core: f32) {
    let rotation = view_rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    for &cluster in rim {
        let ball = &mut balls[cluster];
        let angle = angles.get(&cluster).copied().unwrap_or_else(|| {
            let at = ball.center;
            at.dot(up).atan2(at.dot(right))
        });
        ball.center =
            (right * angle.cos() + up * angle.sin()) * (core * (1.0 + RIM_GAP) + ball.radius);
        ball.body = Body::Open;
    }
}

/// The angle round the expanded nucleus, as the usual view sees it, of each module on
/// the ring round it, as big as `radii`, in order clockwise from the top.
fn rim_angles(radii: &[f32], core: f32) -> Vec<f32> {
    let widest = radii.iter().copied().fold(0.0, f32::max);
    clockwise(radii, core * (1.0 + RIM_GAP) + widest, core * RIM_MARGIN).0
}

/// Lays a module's functions out in a sphere of radius one by the calls among them,
/// from where `start` has them. What is near the start, by `hops` (none for the rest),
/// gathers round it in the middle with room between, and the start's callers lean
/// away from `right` and its callees towards it, only as a tendency; calls pull
/// together; everything else keeps out towards the wall. Depth along `back`, the way
/// the usual view looks, counts for half, so what lies in line there spreads across.
pub(super) fn lay_inside(
    start: &[Vec3],
    calls: &[(usize, usize)],
    hops: &[Option<i32>],
    right: Vec3,
    back: Vec3,
) -> Vec<Vec3> {
    const ROUNDS: usize = 300;
    let count = start.len();
    let mut at = start.to_vec();
    let weight: Vec<f32> = hops
        .iter()
        .map(|h| if h.is_some() { 1.0 } else { 0.35 })
        .collect();
    for round in 0..ROUNDS {
        let mut push = vec![Vec3::ZERO; count];
        for i in 0..count {
            for j in i + 1..count {
                let apart = at[i] - at[j];
                let seen = apart - back * apart.dot(back) * 0.5;
                let squared = seen.length_squared().max(1e-4);
                let force =
                    seen / squared.sqrt() * (0.0016 * weight[i] * weight[j] / squared).min(0.05);
                push[i] += force;
                push[j] -= force;
            }
        }
        for &(a, b) in calls {
            let apart = at[b] - at[a];
            let length = apart.length().max(1e-3);
            let strength = if hops[a].is_some() && hops[b].is_some() {
                0.08
            } else {
                0.02
            };
            let pull = apart / length * (length - 0.3) * strength;
            push[a] += pull;
            push[b] -= pull;
        }
        for (i, push) in push.iter_mut().enumerate() {
            *push += match hops[i] {
                #[expect(clippy::cast_precision_loss, reason = "a hop count")]
                Some(hops) => {
                    let lean = hops.clamp(-1, 3) as f32 * 0.22 - at[i].dot(right);
                    right * lean * 0.03 - at[i] * 0.02
                }
                None => at[i].normalize_or_zero() * (0.82 - at[i].length()) * 0.05,
            };
        }
        #[expect(clippy::cast_precision_loss, reason = "a step count")]
        let cooling = 1.0 - round as f32 / ROUNDS as f32;
        for (i, point) in at.iter_mut().enumerate() {
            *point = if hops[i] == Some(0) {
                Vec3::ZERO
            } else {
                (*point + push[i].clamp_length_max(0.05) * cooling).clamp_length_max(0.92)
            };
        }
    }
    at
}
