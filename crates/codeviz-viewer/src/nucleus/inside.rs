//! The nucleus expanded: the focus's module is the whole scene, laid out as the
//! nucleus lays out modules, one level down. The focus is in the middle; the
//! functions in its module that call it or that it calls stand in a near ring round
//! it, by name; those one more call away inside the module just beyond; and the rest
//! of the module's functions small and faint further out. A type, impl or other
//! container inside the module is a faint shell round its functions, which keep
//! towards one another. Every other module shrinks to a small sphere on a far shell,
//! in the direction the module view has it, so a call leaving the module runs out
//! towards where that module was. As in the module view, space says only where
//! things are: the rings are calls inside the module, never order or rank.

use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use bevy::prelude::*;
use codeviz_layout::ForceGraph;

use super::{
    Arrangement, Ball, Body, Exit, Flow, Link, MARGIN, Nucleus, Shell, closed_radius, fibonacci,
    interior, relax, sideways, turned_view, view_rotation,
};

/// From the focus to the functions that call it or that it calls.
const NEAR: f32 = 12.0;
/// From the focus to the functions one more call away inside the module.
const BEYOND: f32 = 21.0;
/// From the focus to the rest of the module.
const REST: f32 = 27.0;
/// The module's own bounds, just outside the rest.
const BOUNDS: f32 = 30.0;
/// From the focus to the other modules.
const AFAR: f32 = 41.0;
/// Functions named in the near ring at most; the busiest first.
const NEAR_CAP: usize = 14;
/// Functions named just beyond it at most.
const BEYOND_CAP: usize = 16;
/// Other modules a function in the near ring shows its calls to or from, at most; the
/// focus shows them all.
const EXITS_EACH: usize = 2;
/// The rings the named functions stand on, drawn as faint guides round the focus.
pub(super) const GUIDES: [f32; 2] = [NEAR, BEYOND];
/// Room a named function keeps clear round itself, as the view sees it.
const ROOM: f32 = 1.6;
/// A module calls or is called from the scene: the size of its sphere.
const PARTNER_RADIUS: f32 = 1.6;
/// Of the module's functions, a container holding more than this share is not
/// shelled: its shell would be the module's own bounds again.
const MOST: f32 = 0.7;
/// Nor is one whose functions lie further apart than this share of the bounds: its
/// shell would cross half the scene.
const WIDEST: f32 = 0.45;

/// The functions inside the module, by how many calls they are from the focus.
struct Rings {
    near: Vec<usize>,
    beyond: Vec<usize>,
    rest: Vec<usize>,
}

impl Nucleus {
    /// Everything placed with `focus`'s module as the whole scene.
    pub(super) fn arrange_inside(&self, graph: &ForceGraph, focus: Option<usize>) -> Arrangement {
        let around = self.arrange_modules(graph, focus);
        let center = around.center;
        let tiles = graph.positions.len();
        let members: Vec<usize> = (0..tiles)
            .filter(|&t| graph.cluster(t) == Some(center))
            .collect();
        let rings = self.rings(graph, focus, &members);
        let kept: HashSet<usize> = focus
            .into_iter()
            .chain(rings.near.iter().copied())
            .chain(rings.beyond.iter().copied())
            .collect();
        // Calls leave the module from the focus and the near ring only: further out
        // they would cross the scene.
        let near: HashSet<usize> = focus
            .into_iter()
            .chain(rings.near.iter().copied())
            .collect();
        let exits = Self::calls_across(graph, focus, &near, center);

        let balls = self.afar(&around, &exits);
        let ways = self.ways(graph, &members, &rings, &exits, &balls);
        let mut points: Vec<Vec3> = (0..tiles)
            .map(|t| graph.cluster(t).map_or(Vec3::ZERO, |c| balls[c].center))
            .collect();
        let mut shown = vec![false; tiles];
        for &tile in &members {
            shown[tile] = true;
        }
        if let Some(focus) = focus {
            points[focus] = Vec3::ZERO;
        }
        for (tile, at) in place_rings(&rings, &ways) {
            points[tile] = at;
        }

        let shells = self.shell_containers(&members, &points);
        let links = links_among(graph, focus, &kept, &near);
        Arrangement {
            focus,
            center,
            balls,
            points,
            shown,
            named: kept.into_iter().filter(|&t| Some(t) != focus).collect(),
            opened: vec![0; around.balls.len()],
            tubes: Vec::new(),
            links,
            shells,
            exits,
        }
    }

    /// Every other module, small on a far shell in the direction the module view has
    /// it; those the scene's calls reach a little bigger, and held apart as the view
    /// sees them. The focus's module is the scene's bounds.
    fn afar(&self, around: &Arrangement, exits: &[Exit]) -> Vec<Ball> {
        let reached: HashSet<usize> = exits.iter().map(|e| e.cluster).collect();
        let mut balls: Vec<Ball> = (0..around.balls.len())
            .map(|cluster| {
                let reached = reached.contains(&cluster);
                let bearing = around.balls[cluster]
                    .center
                    .try_normalize()
                    .unwrap_or_else(|| fibonacci(cluster, around.balls.len()));
                Ball {
                    center: bearing * AFAR,
                    radius: if reached {
                        PARTNER_RADIUS
                    } else {
                        closed_radius(self.counts[cluster]) * 0.7
                    },
                    body: if reached { Body::Open } else { Body::Closed },
                }
            })
            .collect();
        balls[around.center] = Ball {
            center: Vec3::ZERO,
            radius: BOUNDS,
            body: Body::Nucleus,
        };
        on_shell(&mut balls, |b| b.body == Body::Closed, 1.0, &[]);
        on_shell(&mut balls, |b| b.body == Body::Open, MARGIN, &views());
        balls
    }

    /// The way from the focus each of the module's functions lies: towards its own
    /// corner of the module's map, its container's, and the modules it calls or is
    /// called from; one call further out, also towards the function it reaches the
    /// focus through.
    fn ways(
        &self,
        graph: &ForceGraph,
        members: &[usize],
        rings: &Rings,
        exits: &[Exit],
        balls: &[Ball],
    ) -> HashMap<usize, Vec3> {
        let map: HashMap<usize, Vec3> = interior(graph, members, 1.0).into_iter().collect();
        let own = |tile: usize| {
            map.get(&tile)
                .and_then(|p| p.try_normalize())
                .unwrap_or_else(|| fibonacci(tile, graph.positions.len()))
        };
        let mut containers: HashMap<usize, Vec3> = HashMap::new();
        for &tile in members {
            if let Some(group) = self.inner[tile] {
                *containers.entry(group).or_default() += own(tile);
            }
        }
        let pull = |tile: usize| {
            let calls = exits
                .iter()
                .filter(|e| e.tile == tile)
                .map(|e| {
                    #[expect(clippy::cast_precision_loss, reason = "a call count")]
                    let weight = e.calls as f32;
                    balls[e.cluster].center.normalize_or_zero() * weight
                })
                .sum::<Vec3>()
                .normalize_or_zero();
            let container = self.inner[tile]
                .and_then(|g| containers.get(&g))
                .map_or(Vec3::ZERO, |v| v.normalize_or_zero());
            own(tile) + container * 1.6 + calls * 1.2
        };
        let mut ways: HashMap<usize, Vec3> = HashMap::new();
        for &tile in rings.near.iter().chain(&rings.rest) {
            ways.insert(
                tile,
                pull(tile).try_normalize().unwrap_or_else(|| own(tile)),
            );
        }
        for &tile in &rings.beyond {
            let through = rings
                .near
                .iter()
                .find(|&&near| graph.linked(near, tile))
                .and_then(|near| ways.get(near))
                .copied()
                .unwrap_or_default();
            let way = pull(tile) + through * 0.9;
            ways.insert(tile, way.try_normalize().unwrap_or_else(|| own(tile)));
        }
        ways
    }

    /// The module's functions by calls from the focus inside it: those it calls or
    /// that call it, those one more call away, and the rest; the busiest named first.
    fn rings(&self, graph: &ForceGraph, focus: Option<usize>, members: &[usize]) -> Rings {
        let inside: HashSet<usize> = members.iter().copied().collect();
        let mut next: HashMap<usize, Vec<usize>> = HashMap::new();
        for &(a, b) in &graph.links {
            if inside.contains(&a) && inside.contains(&b) {
                next.entry(a).or_default().push(b);
                next.entry(b).or_default().push(a);
            }
        }
        let busiest = |mut list: Vec<usize>| {
            list.sort_unstable();
            list.dedup();
            list.sort_by_key(|&t| (Reverse(self.degrees[t]), t));
            list
        };
        let around = |tile: usize| next.get(&tile).cloned().unwrap_or_default();
        let mut near = focus.map_or_else(Vec::new, |f| busiest(around(f)));
        near.truncate(NEAR_CAP);
        let taken: HashSet<usize> = near.iter().copied().chain(focus).collect();
        let mut beyond = busiest(
            near.iter()
                .flat_map(|&t| around(t))
                .filter(|t| !taken.contains(t))
                .collect(),
        );
        beyond.truncate(BEYOND_CAP);
        let taken: HashSet<usize> = taken.into_iter().chain(beyond.iter().copied()).collect();
        let rest = members
            .iter()
            .copied()
            .filter(|t| !taken.contains(t))
            .collect();
        Rings { near, beyond, rest }
    }

    /// The calls between a named function and another module, one arrow per module
    /// and way: all of the focus's, and the busiest few of each other's.
    fn calls_across(
        graph: &ForceGraph,
        focus: Option<usize>,
        kept: &HashSet<usize>,
        center: usize,
    ) -> Vec<Exit> {
        let mut counts: HashMap<(usize, usize, bool), usize> = HashMap::new();
        for &(from, to) in &graph.calls {
            let (Some(a), Some(b)) = (graph.cluster(from), graph.cluster(to)) else {
                continue;
            };
            if a == center && b != center && kept.contains(&from) {
                *counts.entry((from, b, true)).or_default() += 1;
            } else if b == center && a != center && kept.contains(&to) {
                *counts.entry((to, a, false)).or_default() += 1;
            }
        }
        let mut exits: Vec<Exit> = counts
            .into_iter()
            .map(|((tile, cluster, outward), calls)| Exit {
                tile,
                cluster,
                outward,
                calls,
                flow: match (Some(tile) == focus, outward) {
                    (true, true) => Flow::OutOf,
                    (true, false) => Flow::Into,
                    (false, _) => Flow::Further,
                },
            })
            .collect();
        exits.sort_by_key(|e| (e.tile, Reverse(e.calls), e.cluster, e.outward));
        let mut each: HashMap<usize, usize> = HashMap::new();
        exits.retain(|e| {
            let shown = each.entry(e.tile).or_default();
            *shown += 1;
            Some(e.tile) == focus || *shown <= EXITS_EACH
        });
        exits
    }

    /// A shell round each container inside the module that holds two of its
    /// functions or more, but not most of them.
    fn shell_containers(&self, members: &[usize], points: &[Vec3]) -> Vec<Shell> {
        let mut held: HashMap<usize, Vec<usize>> = HashMap::new();
        for &tile in members {
            if let Some(group) = self.inner[tile] {
                held.entry(group).or_default().push(tile);
            }
        }
        #[expect(clippy::cast_precision_loss, reason = "function counts")]
        let most = members.len() as f32 * MOST;
        let mut shells: Vec<Shell> = held
            .into_iter()
            .filter(|(_, tiles)| {
                #[expect(clippy::cast_precision_loss, reason = "a function count")]
                let count = tiles.len() as f32;
                tiles.len() >= 2 && count <= most
            })
            .map(|(group, tiles)| {
                #[expect(clippy::cast_precision_loss, reason = "a function count")]
                let count = tiles.len() as f32;
                let center = tiles.iter().map(|&t| points[t]).sum::<Vec3>() / count;
                let radius = tiles
                    .iter()
                    .map(|&t| points[t].distance(center))
                    .fold(0.0, f32::max)
                    + 1.4;
                Shell {
                    group,
                    center,
                    radius,
                }
            })
            .filter(|shell| shell.radius <= BOUNDS * WIDEST)
            .collect();
        shells.sort_by_key(|s| s.group);
        shells
    }
}

/// Where the named rings and the rest go round the focus: the rest further out, and
/// the named held apart as the view sees them, so no name hides another.
fn place_rings(rings: &Rings, ways: &HashMap<usize, Vec3>) -> Vec<(usize, Vec3)> {
    let back = view_rotation() * Vec3::Z;
    let named: Vec<(usize, f32)> = rings
        .near
        .iter()
        .map(|&t| (t, NEAR))
        .chain(rings.beyond.iter().map(|&t| (t, BEYOND)))
        .collect();
    let mut spots: Vec<Ball> = std::iter::once(Ball {
        center: Vec3::ZERO,
        radius: ROOM * 1.5,
        body: Body::Nucleus,
    })
    .chain(named.iter().map(|&(tile, ring)| Ball {
        center: facing(ways[&tile], back, 0.45) * ring,
        radius: ROOM,
        body: Body::Open,
    }))
    .collect();
    let views = views();
    for _ in 0..6 {
        relax(&mut spots, |b| b.body == Body::Open, ROOM, &views, 40);
        for (spot, &(_, ring)) in spots[1..].iter_mut().zip(&named) {
            spot.center = spot.center.normalize_or_zero() * ring;
        }
    }
    spots[1..]
        .iter()
        .zip(&named)
        .map(|(spot, &(tile, _))| (tile, spot.center))
        .chain(
            rings
                .rest
                .iter()
                .map(|&tile| (tile, facing(ways[&tile], back, 0.8) * REST)),
        )
        .collect()
}

/// The calls between named functions inside the module that touch the focus or the
/// near ring: violet into the focus, blue out of it, the rest pale.
fn links_among(
    graph: &ForceGraph,
    focus: Option<usize>,
    kept: &HashSet<usize>,
    near: &HashSet<usize>,
) -> Vec<Link> {
    graph
        .calls
        .iter()
        .filter(|(from, to)| kept.contains(from) && kept.contains(to))
        .filter(|(from, to)| near.contains(from) || near.contains(to))
        .map(|&(from, to)| Link {
            from,
            to,
            flow: match focus {
                Some(f) if to == f => Flow::Into,
                Some(f) if from == f => Flow::OutOf,
                _ => Flow::Further,
            },
        })
        .collect()
}

/// The usual view, and that view turned an eighth of the way either side.
fn views() -> Vec<Vec3> {
    [0.0, -1.0, 1.0]
        .into_iter()
        .map(|turn| turned_view(turn * std::f32::consts::FRAC_PI_4) * Vec3::Z)
        .collect()
}

/// `way` tilted towards the view's plane, keeping `keep` of how far it leans towards
/// or away from the camera, so little hides in front of or behind the focus.
fn facing(way: Vec3, back: Vec3, keep: f32) -> Vec3 {
    let toward = way.dot(back);
    let across = (way - back * toward)
        .try_normalize()
        .unwrap_or_else(|| sideways(back));
    (across + back * toward * keep).normalize()
}

/// Holds the spheres `movable` picks apart, as `views` see them or in space, then
/// back on the far shell, a few times over.
fn on_shell(
    balls: &mut [Ball],
    movable: impl Fn(&Ball) -> bool + Copy,
    margin: f32,
    views: &[Vec3],
) {
    for _ in 0..4 {
        relax(balls, movable, margin, views, 30);
        for ball in balls.iter_mut().filter(|b| movable(b)) {
            ball.center = ball.center.normalize_or_zero() * AFAR;
        }
    }
}
