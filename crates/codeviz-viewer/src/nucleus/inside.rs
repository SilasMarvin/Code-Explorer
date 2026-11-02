//! The expanded nucleus in a journey: the start's module grown several times over,
//! its functions laid out by the calls among them, what is named in the middle,
//! callers leaning one way and the journey the other, the rest towards the wall; the
//! modules it names on a ring just outside the wall, in the ring's order.

use std::collections::HashMap;

use bevy::prelude::*;
use codeviz_layout::{
    ForceGraph,
    atlas::{Apart, fibonacci, push_apart},
};

use super::{
    Arrangement, Ball, Body, Flow, GROWTH, Laid, Link, Nucleus,
    journey::{clockwise, kept_free, names_grid},
    view_rotation,
};
use crate::sizes::Dot;

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
        // How big each is drawn there at most, against the sphere, with room for a
        // ring.
        let core = before * GROWTH;
        let size = (Dot::Focus.radius() * GROWTH * 0.5 + 1.0) / core;
        let inside = lay_members(&members, &seeded, &calls, &reach, size);
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
            inside: members.into_iter().zip(inside).collect(),
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
        let tubes = self.join(&modules.links, &cluster_of);
        let free = kept_free(&balls, &modules.reserved);
        Arrangement {
            expanded: true,
            balls,
            points,
            shown,
            outside,
            free,
            tubes,
            ..modules
        }
    }
}

/// [`lay_inside`] for a module's `members`, by tile, from `start`, with the `calls`
/// among them and how far each is from the start by `reach`; turned to the usual view,
/// and what is near the start, each as big as `size`, kept from touching as that view
/// sees it.
fn lay_members(
    members: &[usize],
    start: &[Vec3],
    calls: &[(usize, usize)],
    reach: &HashMap<usize, i32>,
    size: f32,
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
    let mut at = lay_inside(
        &start,
        &calls,
        &hops,
        rotation * Vec3::X,
        rotation * Vec3::Z,
    );
    keep_apart(&mut at, &hops, size, rotation * Vec3::Z);
    at
}

/// Pushes apart the functions near the start, by `hops`, that touch as seen from
/// `back`, each as big as `size`, until a gap shows between every two; the start
/// stays in the middle. They come most of the way towards the plane the view looks
/// across first, so the camera's perspective, which spreads what is nearer it, does
/// not bring two together again.
pub(super) fn keep_apart(at: &mut [Vec3], hops: &[Option<i32>], size: f32, back: Vec3) {
    let near: Vec<usize> = (0..at.len()).filter(|&i| hops[i].is_some()).collect();
    for &i in &near {
        at[i] -= back * at[i].dot(back) * 0.7;
    }
    let shares = |i: usize, j: usize| {
        Some(match (hops[i], hops[j]) {
            (Some(0), _) => (0.0, 1.0),
            (_, Some(0)) => (1.0, 0.0),
            _ => (0.5, 0.5),
        })
    };
    let apart = Apart {
        margin: 0.0,
        slant: 0.0,
        views: &[back],
        bound: Some(0.92),
    };
    push_apart(at, &vec![size; at.len()], &near, shares, &apart, 200);
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
