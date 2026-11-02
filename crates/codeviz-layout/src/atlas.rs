//! Where everything sits browsing, in 3D: one layout of the whole codebase, worked
//! out once per document. Every module is a sphere, as big as one rule makes it for
//! what it holds; the modules stand where a layout of them alone has them, brought in
//! as near as they can be with none hiding another as the usual view sees them; and
//! each module's functions are spread over a disc in it, square to that view, each as
//! near where the settled graph has it as that allows. The usual view is the caller's
//! to give; nothing here draws.

use std::collections::HashMap;

use glam::{EulerRot, Quat, Vec2, Vec3};

use crate::ForceGraph;

/// The usual view, as a camera's yaw and pitch: the layout is shallow along it and
/// keeps every sphere clear of the others as it sees them.
pub type View = (f32, f32);

/// Kept clear between two module spheres browsing.
const ATLAS_MARGIN: f32 = 3.0;
/// Browsing, how near the modules start, against their typical neighbours' just
/// clear of each other: kept apart from there, the map comes out compact, so the
/// whole of it shows each module, and the functions in it, as big as can be.
const ATLAS_START: f32 = 0.4;
/// Browsing, how much wider than high the codebase is laid out, as a window is.
const ATLAS_ASPECT: f32 = 1.5;
/// Browsing, how much of its depth along the usual view the layout of modules keeps.
const ATLAS_DEPTH: f32 = 0.35;
/// Browsing, how much further apart across the view two modules are kept for each
/// unit one lies behind the other: about how far off the middle of the window a
/// camera's perspective moves what is further back, at the window's edge.
const ATLAS_SLANT: f32 = 0.7;
/// The radius of a module of one function: every sphere is a multiple of it.
const SPHERE_UNIT: f32 = 4.0;
/// How fast a sphere grows with the square root of what it holds, past one function.
const SPHERE_GROWTH: f32 = 0.3;
/// The biggest sphere against the smallest: past it, more functions pack closer.
const SPHERE_RATIO: f32 = 3.95;
/// Of a sphere's radius, how much the disc its functions spread over takes, less
/// [`PACK_INSET`]: room at the wall for a dot drawn big when the view is far off.
const PACK_SHARE: f32 = 0.86;
const PACK_INSET: f32 = 0.5;
/// How far apart a module's functions are kept, against the disc's radius over the
/// square root of how many: as near a honeycomb as pushing them apart reaches.
const PACKING: f32 = 1.8;
/// How far the disc a module's functions spread over bows towards the usual view,
/// against how far it could and stay inside.
const PACK_BOW: f32 = 0.3;

/// A module's sphere, by how many functions it holds: the one rule for every module
/// in every view, browsing, in a journey and round the expanded one. It grows with
/// the square root of the count, so a module twice the size is far less than twice as
/// big, and no more than [`SPHERE_RATIO`] times the smallest; past that its functions
/// pack closer.
#[must_use]
pub fn module_radius(count: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a function count")]
    let root = (count.max(1) as f32).sqrt();
    SPHERE_UNIT * (1.0 + SPHERE_GROWTH * (root - 1.0)).min(SPHERE_RATIO)
}

/// Where each module sits browsing, its sphere as big as `radii` has it: where the
/// layout of modules alone, `homes`, has it, brought in as near as its neighbours
/// allow, shallow along `view` and spread so none hides another as `view`, or
/// `view` turned an eighth of the way either side, sees them; as wide as a window,
/// against its height.
#[must_use]
pub fn atlas(homes: &[Vec3], radii: &[f32], view: View) -> Vec<Vec3> {
    let clusters = homes.len();
    let median = |mut values: Vec<f32>| {
        values.sort_by(f32::total_cmp);
        values.get(values.len() / 2).copied()
    };
    // As far apart as the typical module needs, nearest neighbour to nearest.
    let nearest: Vec<f32> = (0..clusters)
        .map(|i| {
            (0..clusters)
                .filter(|&j| j != i)
                .map(|j| homes[i].distance(homes[j]))
                .fold(f32::INFINITY, f32::min)
        })
        .filter(|d| d.is_finite())
        .collect();
    let gap = median(nearest).unwrap_or(1.0).max(1e-3);
    let room = 2.0 * median(radii.to_vec()).unwrap_or(1.0) + ATLAS_MARGIN;
    #[expect(clippy::cast_precision_loss, reason = "a module count")]
    let middle = homes.iter().copied().sum::<Vec3>() / clusters.max(1) as f32;
    // Shallow as the usual view sees it, so a module further back does not slide
    // under one in front as the camera comes in.
    let rotation = turned(view, 0.0);
    let back = rotation * Vec3::Z;
    let mut centers: Vec<Vec3> = homes
        .iter()
        .map(|&home| {
            let at = (home - middle) * (room / gap) * ATLAS_START;
            at - back * at.dot(back) * (1.0 - ATLAS_DEPTH)
        })
        .collect();
    let apart = Apart {
        margin: ATLAS_MARGIN,
        slant: ATLAS_SLANT,
        views: &views(view),
        bound: None,
    };
    let all: Vec<usize> = (0..clusters).collect();
    push_apart(
        &mut centers,
        radii,
        &all,
        |_, _| Some((0.5, 0.5)),
        &apart,
        2000,
    );
    #[expect(clippy::cast_precision_loss, reason = "a module count")]
    let middle = centers.iter().copied().sum::<Vec3>() / clusters.max(1) as f32;
    // As wide as a window is, against its height, as the usual view sees it: pulled
    // apart sideways, never together, so none comes to hide another.
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    let spread = |along: Vec3| {
        centers
            .iter()
            .map(|&center| (center - middle).dot(along).abs())
            .fold(0.0, f32::max)
    };
    let (wide, high) = (spread(right), spread(up));
    let stretch = (ATLAS_ASPECT * high / wide.max(1e-3)).clamp(1.0, 3.0);
    for center in &mut centers {
        let offset = *center - middle;
        *center = offset + right * offset.dot(right) * (stretch - 1.0);
    }
    centers
}

/// Of a sphere of `radius`, the radius of the disc its functions spread over.
#[must_use]
pub fn disc(radius: f32) -> f32 {
    radius * PACK_SHARE - PACK_INSET
}

/// Where a module's `members` sit inside its sphere of `radius`, about its middle:
/// spread evenly over a disc square to `view`, so none hides or touches
/// another from there, or from as far round as the camera turns, at any zoom that
/// shows them apart at all; each as near where the settled graph has it, against the
/// others, as that allows. The disc bows a little towards the view, so the module
/// reads as round.
#[must_use]
pub fn pack(graph: &ForceGraph, members: &[usize], radius: f32, view: View) -> Vec<(usize, Vec3)> {
    if members.len() < 2 {
        return members.iter().map(|&t| (t, Vec3::ZERO)).collect();
    }
    let rotation = turned(view, 0.0);
    let (right, up, back) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z);
    let disc = disc(radius);
    #[expect(clippy::cast_precision_loss, reason = "a function count")]
    let count = members.len() as f32;
    // As far apart as the disc holds them evenly, nearly as close as a honeycomb.
    let apart = (disc * PACKING / count.sqrt()).min(disc);
    let mut at: Vec<Vec2> = interior(graph, members, disc)
        .into_iter()
        .map(|(_, at)| Vec2::new(at.dot(right), at.dot(up)))
        .collect();
    spread(&mut at, apart, disc);
    members
        .iter()
        .zip(at)
        .map(|(&tile, at)| {
            let bow = PACK_BOW * (disc * disc - at.length_squared()).max(0.0).sqrt();
            (tile, right * at.x + up * at.y + back * bow)
        })
        .collect()
}

/// Pushes `at` apart until every two are `apart` or more, each kept within `disc` of
/// the middle: a pair too near moves away from each other, half each, again and
/// again; near pairs are found through a grid of cells `apart` wide.
fn spread(at: &mut [Vec2], apart: f32, disc: f32) {
    const ROUNDS: usize = 600;
    let cell = |p: Vec2| {
        #[expect(clippy::cast_possible_truncation, reason = "a cell index")]
        let index = ((p.x / apart).floor() as i32, (p.y / apart).floor() as i32);
        index
    };
    for round in 0..ROUNDS {
        let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, &p) in at.iter().enumerate() {
            cells.entry(cell(p)).or_default().push(i);
        }
        let mut moved = false;
        for i in 0..at.len() {
            let (x, y) = cell(at[i]);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let Some(near) = cells.get(&(x + dx, y + dy)) else {
                        continue;
                    };
                    for &j in near.iter().filter(|&&j| j > i) {
                        let between = at[j] - at[i];
                        let distance = between.length();
                        if distance >= apart {
                            continue;
                        }
                        let away = between.try_normalize().unwrap_or_else(|| {
                            let turn = fibonacci(i * 31 + j + round, 97);
                            Vec2::new(turn.x, turn.y).normalize_or(Vec2::X)
                        });
                        let push = away * (apart - distance) * 0.5;
                        at[i] = (at[i] - push).clamp_length_max(disc);
                        at[j] = (at[j] + push).clamp_length_max(disc);
                        moved = true;
                    }
                }
            }
        }
        if !moved {
            break;
        }
    }
}

/// Where the nucleus's functions sit inside a sphere of `radius`: as the settled graph
/// has them about their middle, stretched to fill the sphere in all three
/// directions, so the module's own map is kept.
fn interior(graph: &ForceGraph, members: &[usize], radius: f32) -> Vec<(usize, Vec3)> {
    let at = |tile: usize| Vec3::from(graph.positions[tile]);
    if members.len() < 2 {
        return members.iter().map(|&t| (t, Vec3::ZERO)).collect();
    }
    #[expect(clippy::cast_precision_loss, reason = "a function count")]
    let count = members.len() as f32;
    let middle = members.iter().map(|&t| at(t)).sum::<Vec3>() / count;
    let spread = (members
        .iter()
        .map(|&t| (at(t) - middle).powf(2.0))
        .sum::<Vec3>()
        / count)
        .powf(0.5)
        .max(Vec3::splat(1e-3));
    let offsets: Vec<Vec3> = members.iter().map(|&t| (at(t) - middle) / spread).collect();
    let mut lengths: Vec<f32> = offsets.iter().map(|o| o.length()).collect();
    lengths.sort_by(f32::total_cmp);
    let typical = lengths[(lengths.len() * 85 / 100).min(lengths.len() - 1)].max(1e-3);
    members
        .iter()
        .zip(offsets)
        .map(|(&tile, offset)| {
            (
                tile,
                (offset * 0.72 / typical).clamp_length_max(0.88) * radius,
            )
        })
        .collect()
}

/// A layout of the modules alone in 3D, the same every time for the same code:
/// modules repel, and calls between two pull them together, the harder the more.
#[must_use]
pub fn module_homes(
    count: usize,
    flows: impl IntoIterator<Item = ((usize, usize), usize)>,
) -> Vec<Vec3> {
    let mut weights: HashMap<(usize, usize), usize> = HashMap::new();
    for ((a, b), calls) in flows {
        *weights.entry((a.min(b), a.max(b))).or_insert(0) += calls;
    }
    let mut weights: Vec<((usize, usize), usize)> = weights.into_iter().collect();
    weights.sort_unstable();
    #[expect(clippy::cast_precision_loss, reason = "a module count")]
    let spread = (count as f32).sqrt() * 6.0;
    let mut at: Vec<Vec3> = (0..count).map(|i| fibonacci(i, count) * spread).collect();
    for step in 0..300 {
        let mut push = vec![Vec3::ZERO; count];
        for i in 0..count {
            for j in i + 1..count {
                let apart = at[i] - at[j];
                let squared = apart.length_squared().max(0.25);
                let force = apart / squared.sqrt() * (400.0 / squared);
                push[i] += force;
                push[j] -= force;
            }
        }
        for &((a, b), calls) in &weights {
            let apart = at[b] - at[a];
            let length = apart.length().max(0.01);
            #[expect(clippy::cast_precision_loss, reason = "a call count")]
            let pull = apart / length * (length - 10.0) * 0.04 * (1.0 + (calls as f32).ln());
            push[a] += pull;
            push[b] -= pull;
        }
        #[expect(clippy::cast_precision_loss, reason = "a step count")]
        let cooling = 1.0 - step as f32 / 300.0;
        for (point, push) in at.iter_mut().zip(push) {
            let push = push - *point * 0.01;
            *point += push.clamp_length_max(3.0) * cooling;
        }
    }
    at
}

/// How [`push_apart`] keeps things apart: `margin` clear between every two as each
/// of `views` (the directions they look from) sees them, moving across it, and as the
/// first of them sees them by `slant` more for each unit one lies behind the other, so
/// a camera's perspective from there does not bring them together; each kept within
/// `bound` of the middle, when there is one.
#[derive(Debug, Clone, Copy)]
pub struct Apart<'a> {
    pub margin: f32,
    pub slant: f32,
    pub views: &'a [Vec3],
    pub bound: Option<f32>,
}

/// Pushes apart every two of `among`, indices into `at` in order, each as big as
/// `radii` has it, that come nearer than [`Apart`] allows, again and again until none
/// does or `rounds` run out: each of a pair moves away from the other by the share
/// `shares` gives it, and a pair it gives none stays as it is.
pub fn push_apart(
    at: &mut [Vec3],
    radii: &[f32],
    among: &[usize],
    shares: impl Fn(usize, usize) -> Option<(f32, f32)>,
    apart: &Apart,
    rounds: usize,
) {
    for _ in 0..rounds {
        let mut moved = false;
        for (k, &i) in among.iter().enumerate() {
            for &j in &among[k + 1..] {
                let Some((share_i, share_j)) = shares(i, j) else {
                    continue;
                };
                for (view, &back) in apart.views.iter().enumerate() {
                    let between = at[j] - at[i];
                    // One further back shows nearer the middle of the view than it
                    // would side by side, so a gap in depth needs one across too.
                    let deeper = if view == 0 {
                        between.dot(back).abs()
                    } else {
                        0.0
                    };
                    let need = radii[i] + radii[j] + apart.margin + apart.slant * deeper;
                    let seen = between - back * between.dot(back);
                    let distance = seen.length();
                    if distance >= need {
                        continue;
                    }
                    let away = seen.try_normalize().unwrap_or_else(|| sideways(back));
                    let gap = need - distance;
                    at[i] -= away * gap * share_i;
                    at[j] += away * gap * share_j;
                    if let Some(bound) = apart.bound {
                        at[i] = at[i].clamp_length_max(bound);
                        at[j] = at[j].clamp_length_max(bound);
                    }
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
}

/// The directions `view`, and `view` turned an eighth of the way either side, look
/// from: browsing, no sphere hides another from any of them.
#[must_use]
pub fn views(view: View) -> [Vec3; 3] {
    [0.0, -1.0, 1.0].map(|turn| turned(view, turn * std::f32::consts::FRAC_PI_4) * Vec3::Z)
}

/// `view` turned round by `yaw`.
#[must_use]
pub fn turned(view: View, yaw: f32) -> Quat {
    Quat::from_euler(EulerRot::YXZ, view.0 + yaw, view.1, 0.0)
}

/// A direction square to `along`, the same every time for the same direction.
#[must_use]
pub fn sideways(along: Vec3) -> Vec3 {
    along
        .cross(Vec3::Y)
        .try_normalize()
        .unwrap_or_else(|| along.any_orthonormal_vector())
}

/// The `index`th of `count` points spread evenly over the unit sphere.
#[must_use]
pub fn fibonacci(index: usize, count: usize) -> Vec3 {
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let (i, n) = (index as f32 + 0.5, count.max(1) as f32);
    let y = 1.0 - 2.0 * i / n;
    let ring = (1.0 - y * y).max(0.0).sqrt();
    let turn = i * std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    Vec3::new(ring * turn.cos(), y, ring * turn.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The viewer's usual view.
    const VIEW: View = (-0.45, -0.38);

    /// A module's sphere.
    struct Ball {
        center: Vec3,
        radius: f32,
    }

    #[test]
    fn modules_keep_their_homes() {
        let flows = HashMap::from([((0, 1), 3), ((1, 2), 1), ((2, 0), 2)]);
        assert_eq!(module_homes(4, flows.clone()), module_homes(4, flows));
    }

    #[test]
    fn one_rule_sizes_every_module_growing_ever_slower() {
        let radii: Vec<f32> = [1, 2, 4, 10, 40, 100, 1000].map(module_radius).into();
        assert!(radii.is_sorted(), "more functions, no smaller: {radii:?}");
        for (pair, counts) in radii.windows(2).zip([[1, 2], [2, 4], [4, 10], [10, 40]]) {
            #[expect(clippy::cast_precision_loss, reason = "small counts")]
            let more = counts[1] as f32 / counts[0] as f32;
            assert!(pair[1] / pair[0] < more, "less than in proportion");
        }
        assert!(
            radii[6] / radii[0] <= 4.0,
            "at most four times the smallest"
        );
    }

    /// Pairs of `balls`, each a middle and a radius, whose outlines overlap on screen,
    /// as a camera from the usual view, looking at `target` from `distance`, sees them
    /// in perspective.
    fn overlapping(balls: &[Ball], target: Vec3, distance: f32) -> usize {
        let rotation = turned(VIEW, 0.0);
        let (right, up, back) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z);
        let eye = target + back * distance;
        let tan = (std::f32::consts::FRAC_PI_4 / 2.0).tan();
        let seen: Vec<Option<(Vec2, f32)>> = balls
            .iter()
            .map(|b| {
                let offset = b.center - eye;
                let depth = -offset.dot(back);
                let at = Vec2::new(offset.dot(right), offset.dot(up)) / depth / tan;
                let on = depth > b.radius && at.x.abs() < 1.6 && at.y.abs() < 1.0;
                let radius = b.radius / (depth * depth - b.radius * b.radius).sqrt() / tan;
                on.then_some((at, radius))
            })
            .collect();
        let mut count = 0;
        for (i, a) in seen.iter().enumerate() {
            for b in &seen[i + 1..] {
                if let (Some((a, ra)), Some((b, rb))) = (a, b)
                    && a.distance(*b) < ra + rb
                {
                    count += 1;
                }
            }
        }
        count
    }

    #[test]
    fn browsing_no_two_modules_overlap_however_near_the_camera_comes() {
        // Modules spread through a deep ball of space, as a codebase's often are.
        let homes: Vec<Vec3> = (0..60)
            .map(|i| fibonacci(i, 60) * (20.0 + (i % 7) as f32 * 4.0))
            .collect();
        let radii: Vec<f32> = (0..60).map(|i| module_radius(1 + (i * 37) % 120)).collect();
        let balls: Vec<Ball> = atlas(&homes, &radii, VIEW)
            .into_iter()
            .zip(radii)
            .map(|(center, radius)| Ball { center, radius })
            .collect();
        let middle = balls.iter().map(|b| b.center).sum::<Vec3>() / 60.0;
        let wide = balls
            .iter()
            .map(|b| b.center.distance(middle))
            .fold(0.0, f32::max);
        assert_eq!(overlapping(&balls, middle, wide * 2.5), 0, "the whole map");
        for ball in &balls {
            for distance in [80.0, 150.0] {
                assert_eq!(
                    overlapping(&balls, ball.center, distance),
                    0,
                    "framed close on one module, from {distance}"
                );
            }
        }
    }

    #[test]
    fn a_crowded_module_spreads_its_functions_apart_inside_it() {
        // A hundred functions all but on top of each other, as a module's often sit.
        let mut at: Vec<Vec2> = (0..106)
            .map(|i| {
                let p = fibonacci(i % 9, 9) * 0.3;
                Vec2::new(p.x, p.y)
            })
            .collect();
        let (disc, count) = (12.0, 106.0_f32);
        let apart = disc * PACKING / count.sqrt();
        spread(&mut at, apart, disc);
        for (i, a) in at.iter().enumerate() {
            assert!(a.length() <= disc + 1e-3, "inside");
            for b in &at[i + 1..] {
                assert!(a.distance(*b) > apart * 0.9, "apart: {}", a.distance(*b));
            }
        }
    }
}
