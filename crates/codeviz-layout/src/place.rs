//! Where everything sits, in 3D, worked out once per document by one rule for every
//! container: the world, holding the modules, and each module's sphere, holding its
//! functions. A container holds children, each a ball, tied to each other by the
//! calls between them; [`place`] seeds them from the shape of those ties, settles
//! them, and keeps them apart and inside. [`layout`] applies it at both levels: the
//! modules in the world first, then each module's functions, each also drawn towards
//! the modules it calls or is called from, so the calls between modules run short.
//! Nothing here draws, nor knows where the camera is.

use std::collections::BTreeMap;

use glam::Vec3;

use crate::CallGraph;

/// Of a container's volume, the share its children fill, each with half the gap
/// kept round it: the one rule that sizes every container. Two children fill a
/// quarter of the least sphere that holds them, so it is less than that.
const FILL: f32 = 0.2;
/// Kept clear between two children, as a share of their mean radius.
const GAP: f32 = 0.25;
/// A function's ball: its dot, and room round it for its name.
const FUNCTION: f32 = 2.1;
/// The biggest module's sphere against the smallest's: past it, more functions pack
/// closer.
const RATIO: f32 = 3.95;
/// Settling: how many rounds, and how far a child may move in the first, against a
/// typical child's size (its radius and half the gap); it moves less each round.
const ROUNDS: usize = 300;
const FIRST_STEP: f32 = 0.5;
/// How far a child moves for a unit of force, against a typical child's size.
const GAIN: f32 = 0.2;
/// How hard two children push apart when just touching, gap and all; the push falls
/// with the fourth power of how far apart they are, so it reaches only neighbours.
const PUSH: f32 = 1.0;
/// How hard a tie pulls, for each typical child's size it is stretched past rest.
const SPRING: f32 = 1.0;
/// How hard a child is drawn towards a module outside its container it calls or is
/// called from, however far off.
const ANCHOR: f32 = 1.0;
/// How hard every child is drawn to the middle, at the container's wall.
const CENTRE: f32 = 0.05;
/// Seeding: how far out the seed reaches, as a share of the room in the container;
/// and how many rounds work out the shape of the ties.
const SEED: f32 = 0.6;
const SPECTRAL_ROUNDS: usize = 80;
/// After settling, at most this many rounds pushing apart children that still touch.
const CLEARING_ROUNDS: usize = 500;

/// A pull between two children, as strong as the calls between them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tie {
    pub a: usize,
    pub b: usize,
    pub weight: f32,
}

/// A pull on a child towards a point outside its container, about the container's
/// middle: where a module it calls, or is called from, sits. As strong as the calls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    pub child: usize,
    pub at: Vec3,
    pub weight: f32,
}

/// Where everything sits: every module's sphere in the world, and every function in
/// the world, inside its module's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    /// By module (cluster): its sphere's middle and radius.
    pub modules: Vec<(Vec3, f32)>,
    /// By function (tile): where it sits in the world; at the world's middle when
    /// it has no module.
    pub functions: Vec<Vec3>,
    /// By module: the radius of the ball each of its functions keeps to itself, its
    /// dot and room round it: no two of its functions come nearer than twice this.
    pub rooms: Vec<f32>,
}

/// The gap kept between children as big as `radii`.
fn gap(radii: &[f32]) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a child count")]
    let mean = radii.iter().sum::<f32>() / radii.len().max(1) as f32;
    GAP * mean
}

/// The radius of a container holding children as big as `radii`: one in which they
/// fill [`FILL`] of its volume, each with half the gap round it. So it grows with the
/// cube root of what it holds.
#[must_use]
pub fn container_radius(radii: &[f32]) -> f32 {
    let half_gap = gap(radii) / 2.0;
    let volume: f32 = radii.iter().map(|r| (r + half_gap).powi(3)).sum();
    (volume / FILL).cbrt()
}

/// A module's sphere, by how many functions it holds: [`container_radius`] of that
/// many function balls, no more than [`RATIO`] times a module of one. The same in
/// every view.
#[must_use]
pub fn module_radius(count: usize) -> f32 {
    let (radius, _) = module_size(count);
    radius
}

/// A module's sphere, by how many functions it holds, and the radius of each one's
/// ball in it: the full [`FUNCTION`], or less in a module at the [`RATIO`] cap, so
/// they still fill only their share of it.
fn module_size(count: usize) -> (f32, f32) {
    let free = container_radius(&vec![FUNCTION; count.max(1)]);
    let most = RATIO * container_radius(&[FUNCTION]);
    (free.min(most), FUNCTION * (most / free).min(1.0))
}

/// Where `radii.len()` children, each a ball as big as `radii` has it, sit in a
/// container of radius `bound`, about its middle: seeded from the shape of `ties`
/// ([`seed`]), then settled over a fixed number of rounds, ties pulling, neighbours
/// pushing apart, `anchors` drawing their children their way, every child drawn
/// gently to the middle; after every round no two children nearer than their radii
/// and the gap, and every one inside the container with half the gap to spare. The
/// same every time for the same input.
#[must_use]
pub fn place(radii: &[f32], ties: &[Tie], anchors: &[Anchor], bound: f32) -> Vec<Vec3> {
    let count = radii.len();
    if count < 2 {
        return vec![Vec3::ZERO; count];
    }
    let gap = gap(radii);
    #[expect(clippy::cast_precision_loss, reason = "a child count")]
    let unit = radii.iter().sum::<f32>() / count as f32 + gap / 2.0;
    let room: Vec<f32> = radii
        .iter()
        .map(|r| (bound - r - gap / 2.0).max(0.0))
        .collect();
    let strength = |weight: f32| 1.0 + weight.max(1.0).ln();
    let mut at = seed(radii, ties, anchors, bound - unit);
    for round in 0..ROUNDS {
        #[expect(clippy::cast_precision_loss, reason = "a round count")]
        let cool = 1.0 - round as f32 / ROUNDS as f32;
        let mut force = vec![Vec3::ZERO; count];
        for i in 0..count {
            for j in i + 1..count {
                let rest = radii[i] + radii[j] + gap;
                let between = at[j] - at[i];
                let apart = between.length().max(1e-3 * unit);
                let push = PUSH * (rest / apart).powi(4).min(16.0);
                let away = between / apart * push;
                force[i] -= away;
                force[j] += away;
            }
        }
        for tie in ties {
            let rest = radii[tie.a] + radii[tie.b] + gap;
            let between = at[tie.b] - at[tie.a];
            let apart = between.length().max(1e-3 * unit);
            let pull = between / apart * SPRING * strength(tie.weight) * (apart - rest) / unit;
            force[tie.a] += pull;
            force[tie.b] -= pull;
        }
        for anchor in anchors {
            let toward = (anchor.at - at[anchor.child]).normalize_or_zero();
            force[anchor.child] += toward * ANCHOR * strength(anchor.weight);
        }
        let most = FIRST_STEP * unit * cool;
        for (point, force) in at.iter_mut().zip(force) {
            let force = force - *point / bound * CENTRE;
            *point += (force * unit * GAIN).clamp_length_max(most);
        }
        clear(&mut at, radii, gap, &room);
    }
    for _ in 0..CLEARING_ROUNDS {
        if !clear(&mut at, radii, gap, &room) {
            break;
        }
    }
    at
}

/// Pushes apart every two children nearer than their radii and the gap, half each,
/// and brings every child back within its `room` of the middle. Returns whether any
/// two were too near.
fn clear(at: &mut [Vec3], radii: &[f32], gap: f32, room: &[f32]) -> bool {
    let count = at.len();
    let mut near = false;
    for i in 0..count {
        for j in i + 1..count {
            let rest = radii[i] + radii[j] + gap;
            let between = at[j] - at[i];
            let apart = between.length();
            // Rounding, not leeway: a hair short counts as clear.
            if apart >= rest * 0.999 {
                continue;
            }
            near = true;
            let away = between
                .try_normalize()
                .unwrap_or_else(|| fibonacci(i * 31 + j, 97));
            let push = away * (rest - apart) / 2.0;
            at[i] -= push;
            at[j] += push;
        }
    }
    for (point, &room) in at.iter_mut().zip(room) {
        *point = point.clamp_length_max(room);
    }
    near
}

/// Where children start, within `reach` of the middle: those tied to others where
/// the shape of their ties has them ([`spectral`]), turned to lean the way their
/// `anchors` draw them; the rest towards their anchors, or on Fibonacci points when
/// nothing draws them anywhere. Each nudged a little off its neighbours, so no two
/// start in one place.
fn seed(radii: &[f32], ties: &[Tie], anchors: &[Anchor], reach: f32) -> Vec<Vec3> {
    let count = radii.len();
    let reach = reach.max(0.0) * SEED;
    let mut drawn = vec![Vec3::ZERO; count];
    for anchor in anchors {
        drawn[anchor.child] += anchor.at.normalize_or_zero() * anchor.weight;
    }
    let shape = spectral(count, ties);
    let shape = lean(&shape, &drawn);
    // As far out as the shape's typical child: the settle spreads them from there.
    let shaped: Vec<&Vec3> = shape.iter().filter(|p| **p != Vec3::ZERO).collect();
    #[expect(clippy::cast_precision_loss, reason = "a child count")]
    let typical = (shaped.iter().map(|p| p.length_squared()).sum::<f32>()
        / shaped.len().max(1) as f32)
        .sqrt()
        .max(f32::EPSILON);
    (0..count)
        .map(|i| {
            let nudge = fibonacci(i, count) * reach * 0.05;
            let at = if shape[i] != Vec3::ZERO {
                shape[i] / typical * reach * 0.5
            } else if drawn[i] != Vec3::ZERO {
                drawn[i].normalize() * reach
            } else {
                fibonacci(i, count) * reach
            };
            (at + nudge).clamp_length_max(reach)
        })
        .collect()
}

/// The shape of `ties` among `count` children in 3D: each tied child where the
/// three smoothest ways of spreading them along their ties have it (the eigenvectors
/// of the normalised Laplacian with the smallest non-trivial eigenvalues), found by
/// a fixed number of rounds of orthogonal iteration from the same start every time.
/// A child tied to none is at the middle.
fn spectral(count: usize, ties: &[Tie]) -> Vec<Vec3> {
    let mut degree = vec![0.0_f32; count];
    for tie in ties {
        degree[tie.a] += tie.weight;
        degree[tie.b] += tie.weight;
    }
    let root: Vec<f32> = degree.iter().map(|d| d.sqrt()).collect();
    let norm = root.iter().map(|r| r * r).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return vec![Vec3::ZERO; count];
    }
    // The trivial way, every child alike: what the others are kept square to.
    let trivial: Vec<f32> = root.iter().map(|r| r / norm).collect();
    // Half of one plus D^-1/2 W D^-1/2: the smoothest ways are its largest after the
    // trivial one, and none of its eigenvalues is negative.
    let smooth = |x: &[f32]| {
        let mut y: Vec<f32> = x.iter().map(|v| v * 0.5).collect();
        for tie in ties {
            let w = 0.5 * tie.weight / (root[tie.a] * root[tie.b]);
            y[tie.a] += w * x[tie.b];
            y[tie.b] += w * x[tie.a];
        }
        y
    };
    let mut ways: Vec<Vec<f32>> = (0..3)
        .map(|k| {
            (0..count)
                .map(|i| {
                    if degree[i] > 0.0 {
                        fibonacci(i, count)[k]
                    } else {
                        0.0
                    }
                })
                .collect()
        })
        .collect();
    for _ in 0..SPECTRAL_ROUNDS {
        for k in 0..3 {
            let mut way = smooth(&ways[k]);
            for other in std::iter::once(&trivial).chain(&ways[..k]) {
                let along: f32 = way.iter().zip(other).map(|(a, b)| a * b).sum();
                for (v, o) in way.iter_mut().zip(other) {
                    *v -= along * o;
                }
            }
            let length = way.iter().map(|v| v * v).sum::<f32>().sqrt();
            let scale = if length > 1e-12 { 1.0 / length } else { 0.0 };
            ways[k] = way.into_iter().map(|v| v * scale).collect();
        }
    }
    (0..count)
        .map(|i| {
            if degree[i] > 0.0 {
                Vec3::new(ways[0][i], ways[1][i], ways[2][i]) / root[i]
            } else {
                Vec3::ZERO
            }
        })
        .collect()
}

/// `shape` turned, by one of the 48 ways of swapping and flipping its axes, to lean
/// most the way each child is `drawn`: the shape of the ties says nothing of which
/// way round it lies.
fn lean(shape: &[Vec3], drawn: &[Vec3]) -> Vec<Vec3> {
    const ORDERS: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    if drawn.iter().all(|d| *d == Vec3::ZERO) {
        return shape.to_vec();
    }
    let turn = |p: Vec3, order: [usize; 3], flips: usize| {
        let sign = |k: usize| if flips & (1 << k) == 0 { 1.0 } else { -1.0 };
        Vec3::new(
            p[order[0]] * sign(0),
            p[order[1]] * sign(1),
            p[order[2]] * sign(2),
        )
    };
    let mut best = ([0, 1, 2], 0, f32::NEG_INFINITY);
    for order in ORDERS {
        for flips in 0..8 {
            let fit: f32 = shape
                .iter()
                .zip(drawn)
                .map(|(&p, &d)| turn(p, order, flips).dot(d))
                .sum();
            if fit > best.2 {
                best = (order, flips, fit);
            }
        }
    }
    shape.iter().map(|&p| turn(p, best.0, best.1)).collect()
}

/// Where every module and function of `graph` sits: the modules in the world, tied by
/// the calls between them, in a world as big as [`container_radius`] makes it; then
/// each module's functions in its sphere, tied by the calls among them and anchored
/// to the modules they call or are called from.
#[must_use]
pub fn layout(graph: &CallGraph) -> Layout {
    let clusters = graph.cluster_count();
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); clusters];
    for tile in 0..graph.len() {
        if let Some(cluster) = graph.cluster(tile) {
            members[cluster].push(tile);
        }
    }
    let sizes: Vec<(f32, f32)> = members.iter().map(|m| module_size(m.len())).collect();
    let radii: Vec<f32> = sizes.iter().map(|&(radius, _)| radius).collect();
    let mut flows: BTreeMap<(usize, usize), f32> = BTreeMap::new();
    for &(from, to) in &graph.calls {
        if let (Some(a), Some(b)) = (graph.cluster(from), graph.cluster(to))
            && a != b
        {
            *flows.entry((a.min(b), a.max(b))).or_default() += 1.0;
        }
    }
    let ties: Vec<Tie> = flows
        .into_iter()
        .map(|((a, b), weight)| Tie { a, b, weight })
        .collect();
    let centres = place(&radii, &ties, &[], container_radius(&radii));

    let mut functions = vec![Vec3::ZERO; graph.len()];
    // Each tile's place among its module's members.
    let mut index = vec![usize::MAX; graph.len()];
    for tiles in &members {
        for (i, &tile) in tiles.iter().enumerate() {
            index[tile] = i;
        }
    }
    for (cluster, tiles) in members.iter().enumerate() {
        let mut inner: BTreeMap<(usize, usize), f32> = BTreeMap::new();
        let mut outer: BTreeMap<(usize, usize), f32> = BTreeMap::new();
        for &(from, to) in &graph.calls {
            let (Some(a), Some(b)) = (graph.cluster(from), graph.cluster(to)) else {
                continue;
            };
            match (a == cluster, b == cluster) {
                (true, true) => {
                    let (i, j) = (index[from], index[to]);
                    *inner.entry((i.min(j), i.max(j))).or_default() += 1.0;
                }
                (true, false) => *outer.entry((index[from], b)).or_default() += 1.0,
                (false, true) => *outer.entry((index[to], a)).or_default() += 1.0,
                (false, false) => {}
            }
        }
        let ties: Vec<Tie> = inner
            .into_iter()
            .map(|((a, b), weight)| Tie { a, b, weight })
            .collect();
        let anchors: Vec<Anchor> = outer
            .into_iter()
            .map(|((child, other), weight)| Anchor {
                child,
                at: centres[other] - centres[cluster],
                weight,
            })
            .collect();
        let ball = sizes[cluster].1;
        let inside = place(&vec![ball; tiles.len()], &ties, &anchors, radii[cluster]);
        for (&tile, at) in tiles.iter().zip(inside) {
            functions[tile] = centres[cluster] + at;
        }
    }
    Layout {
        modules: centres.into_iter().zip(radii).collect(),
        functions,
        rooms: sizes.into_iter().map(|(_, ball)| ball).collect(),
    }
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

    /// Two rings of ten children, each a chain closed on itself, tied across by one
    /// pair: what a module's functions often look like.
    fn rings() -> Vec<Tie> {
        let mut ties = Vec::new();
        for ring in [0, 10] {
            for i in 0..10 {
                ties.push(Tie {
                    a: ring + i,
                    b: ring + (i + 1) % 10,
                    weight: 1.0,
                });
            }
        }
        ties.push(Tie {
            a: 0,
            b: 10,
            weight: 1.0,
        });
        ties
    }

    /// Every two of `at`, as big as `radii`, at least their radii and the gap apart,
    /// and every one inside `bound` with half the gap to spare.
    fn assert_apart_and_inside(at: &[Vec3], radii: &[f32], bound: f32) {
        let gap = gap(radii);
        for (i, a) in at.iter().enumerate() {
            assert!(
                a.length() + radii[i] + gap / 2.0 <= bound * 1.001,
                "{i} inside: {} of {bound}",
                a.length() + radii[i]
            );
            for (j, b) in at.iter().enumerate().skip(i + 1) {
                let rest = radii[i] + radii[j] + gap;
                assert!(
                    a.distance(*b) >= rest * 0.998,
                    "{i} and {j} apart: {} of {rest}",
                    a.distance(*b)
                );
            }
        }
    }

    #[test]
    fn one_rule_sizes_every_module_growing_ever_slower() {
        let radii: Vec<f32> = [1, 2, 4, 10, 40, 100, 1000].map(module_radius).into();
        assert!(radii.is_sorted(), "more functions, no smaller: {radii:?}");
        for (pair, counts) in radii.windows(2).zip([[1, 2], [2, 4], [4, 10], [10, 40]]) {
            #[expect(clippy::cast_precision_loss, reason = "small counts")]
            let more = counts[1] as f32 / counts[0] as f32;
            assert!(pair[1] / pair[0] < more, "less than in proportion");
            assert!(
                (pair[1] / pair[0] - more.cbrt()).abs() < 1e-3,
                "with the cube root"
            );
        }
        assert!(
            radii[6] / radii[0] <= 4.0,
            "at most four times the smallest"
        );
        assert!(
            (radii[0] - 4.0).abs() < 0.1,
            "a module of one: {}",
            radii[0]
        );
    }

    #[test]
    fn children_never_touch_and_stay_inside() {
        let radii = vec![FUNCTION; 20];
        let bound = container_radius(&radii);
        assert_apart_and_inside(&place(&radii, &rings(), &[], bound), &radii, bound);
        // Crowded at the cap, as a big module's functions are.
        let (bound, ball) = module_size(300);
        let radii = vec![ball; 300];
        assert_apart_and_inside(&place(&radii, &[], &[], bound), &radii, bound);
        // Children of many sizes, as modules in the world are.
        let radii: Vec<f32> = (0..40).map(|i| module_radius(1 + (i * 37) % 120)).collect();
        let bound = container_radius(&radii);
        assert_apart_and_inside(&place(&radii, &rings(), &[], bound), &radii, bound);
    }

    #[test]
    fn the_same_input_lays_out_the_same() {
        let radii = vec![FUNCTION; 20];
        let anchors = [Anchor {
            child: 3,
            at: Vec3::X * 100.0,
            weight: 2.0,
        }];
        let bound = container_radius(&radii);
        assert_eq!(
            place(&radii, &rings(), &anchors, bound),
            place(&radii, &rings(), &anchors, bound)
        );
    }

    #[test]
    fn tied_children_sit_nearer_than_strangers() {
        let radii = vec![FUNCTION; 20];
        let at = place(&radii, &rings(), &[], container_radius(&radii));
        let tied: f32 = rings()
            .iter()
            .map(|t| at[t.a].distance(at[t.b]))
            .sum::<f32>()
            / 21.0;
        let mut all = 0.0;
        for (i, a) in at.iter().enumerate() {
            for b in &at[i + 1..] {
                all += a.distance(*b);
            }
        }
        let all = all / 190.0;
        assert!(tied < all * 0.6, "tied {tied} against any two {all}");
        // The two rings each keep to a side.
        let middle = |ring: usize| at[ring..ring + 10].iter().sum::<Vec3>() / 10.0;
        assert!(middle(0).distance(middle(10)) > FUNCTION * 2.0);
    }

    #[test]
    fn a_child_leans_towards_the_module_it_calls() {
        let radii = vec![FUNCTION; 20];
        let toward = Vec3::new(1.0, 0.5, -0.3).normalize();
        let anchors: Vec<Anchor> = (0..10)
            .map(|child| Anchor {
                child,
                at: toward * 200.0,
                weight: 1.0,
            })
            .collect();
        let at = place(&radii, &rings(), &anchors, container_radius(&radii));
        let ring = |from: usize| at[from..from + 10].iter().sum::<Vec3>() / 10.0;
        assert!(
            ring(0).dot(toward) > ring(10).dot(toward) + FUNCTION,
            "the ring that calls out sits on that side: {} against {}",
            ring(0).dot(toward),
            ring(10).dot(toward)
        );
    }
}
