//! The codebase packed into balls, the same every time: each module a ball sized by
//! how many functions it holds, its functions spread evenly over the ball's surface,
//! and modules packed side by side inside the ball of the crate (or top-level
//! package) that holds them. Nothing here depends on what is focused, so a function
//! is always found where it was last time.

use std::collections::HashMap;

use codeviz_core::System;
use codeviz_ir::UnitId;

use crate::{CodebaseMap, ForceGraph};

/// A ball in space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ball {
    pub center: [f32; 3],
    pub radius: f32,
}

/// The ball that holds some modules: a crate, or a top-level package or module when
/// one root holds everything.
#[derive(Debug, Clone, PartialEq)]
pub struct Holder {
    pub unit: Option<UnitId>,
    pub ball: Ball,
    /// The module clusters inside, as [`ForceGraph::cluster`] numbers them.
    pub modules: Vec<usize>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Packing {
    /// Where each tile sits.
    pub positions: Vec<[f32; 3]>,
    /// Each module cluster's ball, by cluster.
    pub modules: Vec<Ball>,
    pub holders: Vec<Holder>,
    /// The holder each module cluster sits in, by cluster.
    pub holder_of: Vec<usize>,
}

/// Room each function takes on its module's surface, center to center.
const SPACING: f32 = 3.2;
/// Room between a module's functions and its surface.
const PADDING: f32 = 1.8;
/// Clear space between two modules in one holder.
const MODULE_GAP: f32 = 2.4;
/// Clear space between two holders.
const HOLDER_GAP: f32 = 7.0;
/// Room between a holder's modules and its surface.
const HOLDER_PADDING: f32 = 1.6;

/// Packs every tile of `map` into the ball of its module, as `graph` clusters them.
#[must_use]
pub fn pack(system: &System, map: &CodebaseMap, graph: &ForceGraph) -> Packing {
    let count = graph.cluster_count();
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); count];
    for tile in 0..map.tiles.len() {
        if let Some(cluster) = graph.cluster(tile) {
            members[cluster].push(tile);
        }
    }
    // A type's methods sit side by side, in a band round the ball.
    for list in &mut members {
        list.sort_by(|&a, &b| {
            let (a, b) = (&map.tiles[a], &map.tiles[b]);
            (a.group, a.unit.as_str()).cmp(&(b.group, b.unit.as_str()))
        });
    }
    let shells: Vec<f32> = members.iter().map(|m| shell_radius(m.len())).collect();
    let radii: Vec<f32> = shells.iter().map(|r| r + PADDING).collect();

    // How many calls run between each pair of modules, so those that talk sit close.
    let mut talk: HashMap<(usize, usize), f32> = HashMap::new();
    for &(from, to) in &graph.calls {
        if let (Some(a), Some(b)) = (graph.cluster(from), graph.cluster(to))
            && a != b
        {
            *talk.entry((a.min(b), a.max(b))).or_default() += 1.0;
        }
    }

    let (holder_units, holder_of) = holders(system, graph);
    let mut holders: Vec<Holder> = holder_units
        .into_iter()
        .map(|unit| Holder {
            unit,
            ball: Ball {
                center: [0.0; 3],
                radius: 0.0,
            },
            modules: Vec::new(),
        })
        .collect();
    for (cluster, &holder) in holder_of.iter().enumerate() {
        holders[holder].modules.push(cluster);
    }
    // Modules inside each holder, around its middle.
    let mut local = vec![[0.0_f32; 3]; count];
    for holder in &mut holders {
        let sizes: Vec<f32> = holder.modules.iter().map(|&m| radii[m]).collect();
        let modules = &holder.modules;
        let weight = |i: usize, j: usize| {
            let (a, b) = (modules[i], modules[j]);
            talk.get(&(a.min(b), a.max(b))).copied().unwrap_or(0.0)
        };
        let (centers, radius) = pack_balls(&sizes, weight, MODULE_GAP);
        for (&module, center) in holder.modules.iter().zip(centers) {
            local[module] = center;
        }
        holder.ball.radius = if holder.modules.len() == 1 {
            radius
        } else {
            radius + HOLDER_PADDING
        };
    }
    // Then the holders, side by side.
    let mut holder_talk: HashMap<(usize, usize), f32> = HashMap::new();
    for (&(a, b), &calls) in &talk {
        let (a, b) = (holder_of[a], holder_of[b]);
        if a != b {
            *holder_talk.entry((a.min(b), a.max(b))).or_default() += calls;
        }
    }
    let sizes: Vec<f32> = holders.iter().map(|h| h.ball.radius).collect();
    let (centers, _) = pack_balls(
        &sizes,
        |i, j| {
            holder_talk
                .get(&(i.min(j), i.max(j)))
                .copied()
                .unwrap_or(0.0)
        },
        HOLDER_GAP,
    );
    for (holder, center) in holders.iter_mut().zip(centers) {
        holder.ball.center = center;
    }

    let modules: Vec<Ball> = (0..count)
        .map(|cluster| Ball {
            center: add(holders[holder_of[cluster]].ball.center, local[cluster]),
            radius: radii[cluster],
        })
        .collect();
    let mut positions = vec![[0.0; 3]; map.tiles.len()];
    for (cluster, list) in members.iter().enumerate() {
        for (index, &tile) in list.iter().enumerate() {
            let on_shell = scale(fibonacci(index, list.len()), shells[cluster]);
            positions[tile] = add(modules[cluster].center, on_shell);
        }
    }
    Packing {
        positions,
        modules,
        holders,
        holder_of,
    }
}

/// How far from its module's middle a module of `count` functions puts them, so
/// neighbors on the surface sit about [`SPACING`] apart.
fn shell_radius(count: usize) -> f32 {
    if count <= 1 {
        return 0.0;
    }
    // Each function takes about SPACING² of the surface.
    let area = count as f32 * SPACING * SPACING;
    (area / (4.0 * std::f32::consts::PI))
        .sqrt()
        .max(SPACING * 0.6)
}

/// The `index`th of `count` points spread evenly over the unit sphere, top to bottom
/// in a spiral, so points next in order sit next to each other.
fn fibonacci(index: usize, count: usize) -> [f32; 3] {
    if count <= 1 {
        return [0.0; 3];
    }
    let golden = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    let y = 1.0 - 2.0 * (index as f32 + 0.5) / count as f32;
    let ring = (1.0 - y * y).max(0.0).sqrt();
    let angle = golden * index as f32;
    [ring * angle.cos(), y, ring * angle.sin()]
}

/// For each module cluster, the holder it sits in, numbered in order of first
/// appearance, with each holder's unit. A holder is the outermost container shared
/// by no other: under a single root (a lone crate or package), its children.
fn holders(system: &System, graph: &ForceGraph) -> (Vec<Option<UnitId>>, Vec<usize>) {
    let paths: Vec<Vec<UnitId>> = (0..graph.cluster_count())
        .map(|cluster| {
            let mut path = Vec::new();
            let mut at = graph.cluster_unit(cluster).cloned();
            while let Some(id) = at {
                at = system.unit(&id).and_then(|u| u.parent.clone());
                path.push(id);
            }
            path.reverse();
            path
        })
        .collect();
    // How many levels every module shares, from the root down.
    let mut shared = paths.iter().map(Vec::len).min().unwrap_or(0);
    if let Some(first) = paths.first() {
        for path in &paths {
            shared = shared.min(first.iter().zip(path).take_while(|(a, b)| a == b).count());
        }
    }
    let mut numbers: HashMap<String, usize> = HashMap::new();
    let mut units = Vec::new();
    let holder_of = paths
        .iter()
        .enumerate()
        .map(|(cluster, path)| {
            let unit = path.get(shared.min(path.len().saturating_sub(1))).cloned();
            let key = unit
                .as_ref()
                .map_or_else(|| format!("cluster:{cluster}"), ToString::to_string);
            let next = numbers.len();
            *numbers.entry(key).or_insert_with(|| {
                units.push(unit);
                next
            })
        })
        .collect();
    (units, holder_of)
}

/// Directions to try when placing a ball against another.
const DIRECTIONS: usize = 64;

/// Packs balls of `radii` tightly round the origin, `gap` apart, the biggest first,
/// each as near the middle and the balls it `weight`s most as it fits. Returns each
/// ball's center, with the whole recentered, and the radius of a ball holding them
/// all.
fn pack_balls(
    radii: &[f32],
    weight: impl Fn(usize, usize) -> f32,
    gap: f32,
) -> (Vec<[f32; 3]>, f32) {
    let mut order: Vec<usize> = (0..radii.len()).collect();
    order.sort_by(|&a, &b| radii[b].total_cmp(&radii[a]).then(a.cmp(&b)));
    let directions: Vec<[f32; 3]> = (0..DIRECTIONS).map(|i| fibonacci(i, DIRECTIONS)).collect();
    let mut centers = vec![[0.0_f32; 3]; radii.len()];
    let mut placed: Vec<usize> = Vec::new();
    for &ball in &order {
        let radius = radii[ball];
        let pulls: Vec<(usize, f32)> = placed
            .iter()
            .map(|&other| (other, weight(ball, other)))
            .filter(|&(_, w)| w > 0.0)
            .collect();
        let total: f32 = pulls.iter().map(|&(_, w)| w).sum();
        let mut best: Option<([f32; 3], f32)> = None;
        for &against in &placed {
            for &direction in &directions {
                let reach = radii[against] + radius + gap;
                let at = add(centers[against], scale(direction, reach));
                let clear = placed.iter().all(|&other| {
                    length(sub(at, centers[other])) >= radii[other] + radius + gap - 1e-3
                });
                if !clear {
                    continue;
                }
                let pull: f32 = pulls
                    .iter()
                    .map(|&(other, w)| w * length(sub(at, centers[other])))
                    .sum::<f32>()
                    / total.max(1.0);
                // Depth costs double, so the packing is wider than it is deep and
                // less of it hides behind the rest from the front.
                let score = length([at[0], at[1], at[2] * 2.0]) + pull * 0.5;
                if best.is_none_or(|(_, s)| score < s - 1e-4) {
                    best = Some((at, score));
                }
            }
        }
        centers[ball] = best.map_or([0.0; 3], |(at, _)| at);
        placed.push(ball);
    }
    // Recenter on the middle of the bounds, and measure what holds them all.
    let mut low = [f32::MAX; 3];
    let mut high = [f32::MIN; 3];
    for (center, &radius) in centers.iter().zip(radii) {
        for axis in 0..3 {
            low[axis] = low[axis].min(center[axis] - radius);
            high[axis] = high[axis].max(center[axis] + radius);
        }
    }
    if radii.is_empty() {
        return (centers, 0.0);
    }
    let middle = scale(add(low, high), 0.5);
    let centers: Vec<[f32; 3]> = centers.iter().map(|&c| sub(c, middle)).collect();
    let hold = centers
        .iter()
        .zip(radii)
        .map(|(&c, &r)| length(c) + r)
        .fold(0.0, f32::max);
    (centers, hold)
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn length(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use codeviz_ir::Document;

    use super::*;
    use crate::{ForceConfig, MapConfig, map};

    /// Crate `k1` holds modules `m1` (`a`, `b`, `c`) and `m2` (`d`); crate `k2` holds
    /// `m3` (`e`, `f`). `a` calls `d` and `e`.
    fn system() -> System {
        let unit = |id: &str, kind: &str, parent: Option<&str>| {
            let parent = parent.map_or_else(String::new, |p| format!(r#", "parent": "x:{p}""#));
            let role = if kind == "fn" { "invocable" } else { "group" };
            format!(
                r#"{{ "id": "x:{id}", "name": "{id}", "kind": "{kind}", "role": "{role}"{parent} }}"#
            )
        };
        let call = |to: &str| {
            format!(
                r#"{{ "key": "{to}#0", "text": "{to}", "targets": [{{ "unit": "x:{to}", "certainty": "exact" }}] }}"#
            )
        };
        let caller = format!(
            r#"{{ "id": "x:a", "name": "a", "kind": "fn", "role": "invocable", "parent": "x:m1", "calls": [{}, {}] }}"#,
            call("d"),
            call("e")
        );
        let units = [
            unit("k1", "crate", None),
            unit("k2", "crate", None),
            unit("m1", "module", Some("k1")),
            unit("m2", "module", Some("k1")),
            unit("m3", "module", Some("k2")),
            caller,
            unit("b", "fn", Some("m1")),
            unit("c", "fn", Some("m1")),
            unit("d", "fn", Some("m2")),
            unit("e", "fn", Some("m3")),
            unit("f", "fn", Some("m3")),
        ];
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
            units.join(", ")
        );
        System::new([Document::from_json(&json).unwrap()])
    }

    #[test]
    fn functions_sit_in_their_modules_and_modules_in_their_crates() {
        let system = system();
        let map = map(&system, &MapConfig::default());
        let graph = ForceGraph::new(&system, &map, ForceConfig::default());
        let packing = pack(&system, &map, &graph);
        assert_eq!(packing, pack(&system, &map, &graph), "the same every time");
        assert_eq!(packing.holders.len(), 2, "one per crate");
        for tile in 0..map.tiles.len() {
            let cluster = graph.cluster(tile).unwrap();
            let ball = packing.modules[cluster];
            let from_middle = length(sub(packing.positions[tile], ball.center));
            assert!(
                from_middle < ball.radius,
                "tile {tile} is outside its module"
            );
        }
        for (cluster, ball) in packing.modules.iter().enumerate() {
            let holder = packing.holders[packing.holder_of[cluster]].ball;
            let reach = length(sub(ball.center, holder.center)) + ball.radius;
            assert!(reach <= holder.radius + 1e-3, "module {cluster} pokes out");
        }
        let tile = |id: &str| map.tile_of(&UnitId::new(id)).unwrap();
        let module = |id: &str| graph.cluster(tile(id)).unwrap();
        assert_eq!(module("x:a"), module("x:c"));
        assert_ne!(
            packing.holder_of[module("x:a")],
            packing.holder_of[module("x:e")]
        );
    }

    #[test]
    fn balls_never_overlap_and_the_holder_holds_them() {
        let radii = [5.0, 3.0, 3.0, 2.0, 1.0, 1.0, 4.0];
        let (centers, hold) = pack_balls(&radii, |_, _| 0.0, 1.0);
        for a in 0..radii.len() {
            assert!(length(centers[a]) + radii[a] <= hold + 1e-3);
            for b in (a + 1)..radii.len() {
                let apart = length(sub(centers[a], centers[b]));
                assert!(
                    apart >= radii[a] + radii[b] + 1.0 - 1e-2,
                    "{a} and {b} overlap"
                );
            }
        }
    }

    #[test]
    fn balls_that_talk_sit_closer() {
        let radii = [4.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0];
        let talks = |i: usize, j: usize| f32::from(u8::from((i.min(j), i.max(j)) == (1, 6)) * 9);
        let (alone, _) = pack_balls(&radii, |_, _| 0.0, 1.0);
        let (talking, _) = pack_balls(&radii, talks, 1.0);
        let apart = |c: &[[f32; 3]]| length(sub(c[1], c[6]));
        assert!(apart(&talking) <= apart(&alone));
    }

    #[test]
    fn functions_spread_over_the_surface_without_crowding() {
        let count = 40;
        let radius = shell_radius(count);
        let points: Vec<[f32; 3]> = (0..count)
            .map(|i| scale(fibonacci(i, count), radius))
            .collect();
        for (i, a) in points.iter().enumerate() {
            assert!((length(*a) - radius).abs() < 1e-3);
            let nearest = points
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, b)| length(sub(*a, *b)))
                .fold(f32::MAX, f32::min);
            assert!(nearest > SPACING * 0.6, "{i} is crowded: {nearest}");
        }
    }
}
