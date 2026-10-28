//! The codebase as a force-directed graph in 3D: one node per function (as tiled by
//! the [`CodebaseMap`]), one link per pair of functions where one calls the other.
//! Nodes repel and links pull, so clusters come from what calls what; with cohesion
//! on, functions in the same container also drift together.
//!
//! The simulation runs a step at a time so a viewer can show it settling.

use std::collections::{HashMap, HashSet, VecDeque};

use codeviz_core::System;
use codeviz_ir::UnitId;

use crate::{CodebaseMap, Nesting};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForceConfig {
    /// How hard every pair of nodes pushes apart.
    pub repulsion: f32,
    /// How hard a link pulls towards its rest length.
    pub spring: f32,
    pub link_length: f32,
    /// How hard a node is pulled towards the middle of its container.
    pub cohesion: f32,
    /// How hard two containers that crowd each other push apart, so each reads as its
    /// own cluster with space around it; zero lets them mingle.
    pub separation: f32,
    /// How hard a link between two containers pulls, as a share of [`Self::spring`]:
    /// below one, calls across containers bend the clusters towards each other without
    /// merging them.
    pub cross_spring: f32,
    /// How hard everything is pulled towards the origin.
    pub gravity: f32,
    /// How hard everything is pulled towards the plane facing the default view, so
    /// clusters spread side by side instead of hiding behind each other.
    pub flatten: f32,
    /// Velocity kept per step.
    pub damping: f32,
    /// Whether every level of container gathers its functions and keeps clear of its
    /// siblings, crate down to type, rather than the nearest module alone.
    pub nested: bool,
}

impl Default for ForceConfig {
    fn default() -> Self {
        Self {
            repulsion: 30.0,
            spring: 0.06,
            link_length: 6.0,
            cohesion: 0.02,
            separation: 0.0,
            cross_spring: 1.0,
            gravity: 0.004,
            flatten: 0.0,
            damping: 0.82,
            nested: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ForceGraph {
    /// Node positions, by tile index.
    pub positions: Vec<[f32; 3]>,
    velocities: Vec<[f32; 3]>,
    groups: Vec<usize>,
    /// The cluster each tile pulls into: its container's module, so a module's types
    /// and free functions gather as one.
    clusters: Vec<usize>,
    /// The module each cluster stands for, by cluster.
    cluster_units: Vec<Option<UnitId>>,
    /// Every container around the functions, nested.
    nesting: Nesting,
    /// Links between tiles, each pair once, lower index first.
    pub links: Vec<(usize, usize)>,
    /// The same links with their direction: `(caller, callee)`, sorted.
    pub calls: Vec<(usize, usize)>,
    /// Calls that still pull as links but no longer run, such as the calls a diff
    /// removed: kept out of [`Self::calls`], callers, reach and sequences.
    pub ghosts: Vec<(usize, usize)>,
    /// Per tile, the tiles it calls in the order the calls are written, repeats kept.
    sequences: Vec<Vec<usize>>,
    config: ForceConfig,
}

/// Where a call is written: line, column, then declaration order as a tiebreak.
type SiteKey = (u32, u32, usize);

/// How a container's functions call: among themselves, or across its boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GroupStats {
    pub nodes: usize,
    /// Calls between two of its functions.
    pub internal: usize,
    /// Calls from its functions to other containers.
    pub outgoing: usize,
    /// Calls into its functions from other containers.
    pub incoming: usize,
}

/// Fewest calls between a node and the tiles that reach it, or that it reaches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reach {
    /// Tiles that call into the node, directly (1) or through others.
    pub upstream: HashMap<usize, u32>,
    /// Tiles the node calls, directly (1) or through others.
    pub downstream: HashMap<usize, u32>,
}

impl ForceGraph {
    /// Seeds every tile at a deterministic point in a ball around the origin.
    #[must_use]
    pub fn new(system: &System, map: &CodebaseMap, config: ForceConfig) -> Self {
        let mut calls = HashSet::new();
        // Each call site with where it is written, to put a tile's calls in order.
        let mut sites: Vec<Vec<(SiteKey, usize)>> = vec![Vec::new(); map.tiles.len()];
        for (order, unit) in system.units().enumerate() {
            let Some(from) = map.tile_of(&unit.id) else {
                continue;
            };
            for (index, call) in unit.calls.iter().enumerate() {
                let at = call
                    .location
                    .as_ref()
                    .and_then(|l| l.range)
                    .map_or((u32::MAX, u32::MAX), |r| (r.start_line, r.start_col));
                for target in &call.targets {
                    if let Some(to) = map.tile_of(&target.unit)
                        && to != from
                    {
                        calls.insert((from, to));
                        if let Some(list) = sites.get_mut(from) {
                            list.push(((at.0, at.1, order * 1024 + index), to));
                        }
                    }
                }
            }
        }
        let sequences = sites
            .into_iter()
            .map(|mut list| {
                list.sort_by_key(|&(key, _)| key);
                list.into_iter().map(|(_, to)| to).collect()
            })
            .collect();
        let mut links: Vec<(usize, usize)> = calls
            .iter()
            .map(|&(a, b)| (a.min(b), a.max(b)))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        links.sort_unstable();
        let mut calls: Vec<(usize, usize)> = calls.into_iter().collect();
        calls.sort_unstable();

        let (clusters, cluster_units) = module_clusters(system, map);
        let nesting = Nesting::new(system, map);
        let radius = (map.tiles.len() as f32).sqrt() * 2.0;
        let positions = map
            .tiles
            .iter()
            .map(|tile| seed(tile.unit.as_str(), radius))
            .collect();
        Self {
            velocities: vec![[0.0; 3]; map.tiles.len()],
            groups: map.tiles.iter().map(|t| t.group).collect(),
            clusters,
            cluster_units,
            nesting,
            positions,
            links,
            calls,
            ghosts: Vec::new(),
            sequences,
            config,
        }
    }

    /// Turns these calls into ghosts: they keep pulling their ends together, but no
    /// longer count as calls.
    pub fn retire(&mut self, gone: &HashSet<(usize, usize)>) {
        let (ghosts, calls) = self.calls.iter().partition(|call| gone.contains(call));
        self.calls = calls;
        self.ghosts.extend::<Vec<_>>(ghosts);
        self.ghosts.sort_unstable();
        for (from, sequence) in self.sequences.iter_mut().enumerate() {
            sequence.retain(|&to| !gone.contains(&(from, to)));
        }
    }

    /// Changes the forces; the graph moves until it settles under them.
    pub fn set_config(&mut self, config: ForceConfig) {
        self.config = config;
    }

    /// The tiles `tile` calls, in the order the calls are written, repeats kept.
    #[must_use]
    pub fn sequence(&self, tile: usize) -> &[usize] {
        self.sequences.get(tile).map_or(&[], Vec::as_slice)
    }

    /// Everything that reaches `tile` and everything it reaches, by fewest calls.
    #[must_use]
    pub fn reach(&self, tile: usize) -> Reach {
        Reach {
            upstream: self.walk(tile, |(caller, callee)| (callee, caller)),
            downstream: self.walk(tile, |call| call),
        }
    }

    /// Breadth-first from `start` along calls turned `(from, to)` by `direction`.
    fn walk(
        &self,
        start: usize,
        direction: impl Fn((usize, usize)) -> (usize, usize),
    ) -> HashMap<usize, u32> {
        let mut next: HashMap<usize, Vec<usize>> = HashMap::new();
        for &call in &self.calls {
            let (from, to) = direction(call);
            next.entry(from).or_default().push(to);
        }
        let mut found = HashMap::new();
        let mut queue = VecDeque::from([(start, 0)]);
        while let Some((tile, hops)) = queue.pop_front() {
            for &to in next.get(&tile).into_iter().flatten() {
                if to != start && !found.contains_key(&to) {
                    found.insert(to, hops + 1);
                    queue.push_back((to, hops + 1));
                }
            }
        }
        found
    }

    /// The module cluster a tile pulls into.
    #[must_use]
    pub fn cluster(&self, tile: usize) -> Option<usize> {
        self.clusters.get(tile).copied()
    }

    /// The module a cluster stands for, when it has one.
    #[must_use]
    pub fn cluster_unit(&self, cluster: usize) -> Option<&UnitId> {
        self.cluster_units.get(cluster)?.as_ref()
    }

    /// Every container around the functions, nested.
    #[must_use]
    pub fn nesting(&self) -> &Nesting {
        &self.nesting
    }

    /// How many clusters there are.
    #[must_use]
    pub fn cluster_count(&self) -> usize {
        self.cluster_units.len()
    }

    /// The container each tile belongs to, by group index.
    #[must_use]
    pub fn group(&self, tile: usize) -> Option<usize> {
        self.groups.get(tile).copied()
    }

    /// Per container, by group index: its functions and where their calls go.
    #[must_use]
    pub fn group_stats(&self) -> Vec<GroupStats> {
        let count = self.groups.iter().max().map_or(0, |g| g + 1);
        let mut stats = vec![GroupStats::default(); count];
        for &group in &self.groups {
            stats[group].nodes += 1;
        }
        for &(from, to) in &self.calls {
            let (Some(a), Some(b)) = (self.group(from), self.group(to)) else {
                continue;
            };
            if a == b {
                stats[a].internal += 1;
            } else {
                stats[a].outgoing += 1;
                stats[b].incoming += 1;
            }
        }
        stats
    }

    /// Tiles that call `tile`.
    pub fn callers(&self, tile: usize) -> impl Iterator<Item = usize> + '_ {
        self.calls.iter().filter(move |c| c.1 == tile).map(|c| c.0)
    }

    /// Tiles `tile` calls.
    pub fn callees(&self, tile: usize) -> impl Iterator<Item = usize> + '_ {
        self.calls.iter().filter(move |c| c.0 == tile).map(|c| c.1)
    }

    /// How many links touch `tile`.
    #[must_use]
    pub fn degree(&self, tile: usize) -> usize {
        self.links
            .iter()
            .filter(|(a, b)| *a == tile || *b == tile)
            .count()
    }

    /// Whether `a` and `b` are linked.
    #[must_use]
    pub fn linked(&self, a: usize, b: usize) -> bool {
        self.links.binary_search(&(a.min(b), a.max(b))).is_ok()
    }

    /// Advances the simulation one step. Returns the mean speed, which falls towards
    /// zero as the graph settles.
    pub fn step(&mut self) -> f32 {
        let count = self.positions.len();
        if count == 0 {
            return 0.0;
        }
        let config = self.config;
        let mut forces = vec![[0.0_f32; 3]; count];

        for i in 0..count {
            for j in (i + 1)..count {
                let delta = sub(self.positions[i], self.positions[j]);
                let distance_sq = dot(delta, delta).max(0.01);
                let push = scale(delta, config.repulsion / (distance_sq * distance_sq.sqrt()));
                forces[i] = add(forces[i], push);
                forces[j] = sub(forces[j], push);
            }
        }
        for &(a, b) in &self.links {
            let delta = sub(self.positions[b], self.positions[a]);
            let distance = dot(delta, delta).sqrt().max(0.01);
            let spring = if config.nested {
                config.spring * self.nested_spring(a, b)
            } else if self.clusters.get(a) == self.clusters.get(b) {
                config.spring
            } else {
                config.spring * config.cross_spring
            };
            let pull = scale(delta, spring * (distance - config.link_length) / distance);
            forces[a] = add(forces[a], pull);
            forces[b] = sub(forces[b], pull);
        }
        let gathered = if config.nested {
            self.gather(&self.nesting_groups())
        } else {
            self.gather(&self.cluster_groups())
        };
        for (i, force) in forces.iter_mut().enumerate() {
            *force = add(*force, gathered[i]);
            *force = sub(*force, scale(self.positions[i], config.gravity));
            force[2] -= self.positions[i][2] * config.flatten;
        }

        let mut speed = 0.0;
        let moving = self.velocities.iter_mut().zip(&mut self.positions);
        for ((velocity_slot, position), force) in moving.zip(&forces) {
            let velocity = scale(add(*velocity_slot, *force), config.damping);
            // Keep any one step small so a crowded start cannot fling nodes away.
            let length = dot(velocity, velocity).sqrt();
            let velocity = if length > 2.0 {
                scale(velocity, 2.0 / length)
            } else {
                velocity
            };
            *velocity_slot = velocity;
            *position = add(*position, velocity);
            speed += length.min(2.0);
        }
        speed / count as f32
    }
}

/// Groups of tiles that gather and keep apart: per tile the groups it is in, and per
/// group its parent, how hard its tiles pull together and the gap it keeps from its
/// siblings.
struct Groups {
    of_tile: Vec<Vec<usize>>,
    parent: Vec<Option<usize>>,
    cohesion: Vec<f32>,
    gap: Vec<f32>,
    /// Whether a group's room is set by how many tiles it holds rather than how far
    /// they spread: a group torn by calls across its edge then does not claim more
    /// room, which would push its neighbors away and tear it further.
    by_count: bool,
}

impl ForceGraph {
    /// How hard a link pulls when nested, as a share of the spring: fully within one
    /// container, less across the edge of one, less again across the edge of the
    /// outermost, so calls between crates bend them towards each other without
    /// tearing them apart.
    fn nested_spring(&self, a: usize, b: usize) -> f32 {
        let (Some(from), Some(to)) = (self.nesting.paths.get(a), self.nesting.paths.get(b)) else {
            return self.config.cross_spring;
        };
        if from.last() == to.last() {
            1.0
        } else if from.first() == to.first() {
            self.config.cross_spring
        } else {
            self.config.cross_spring * self.config.cross_spring
        }
    }

    /// Each tile in its module cluster, the clusters side by side.
    fn cluster_groups(&self) -> Groups {
        let count = self.clusters.iter().max().map_or(0, |c| c + 1);
        Groups {
            of_tile: self.clusters.iter().map(|&c| vec![c]).collect(),
            parent: vec![None; count],
            cohesion: vec![self.config.cohesion; count],
            gap: vec![self.config.link_length * 2.0; count],
            by_count: false,
        }
    }

    /// Each tile in every container around it. The innermost pull hardest; an outer
    /// one pulls a little, enough to keep what it holds together. Innermost siblings
    /// keep a narrower gap, as they share a container.
    fn nesting_groups(&self) -> Groups {
        let nests = &self.nesting.nests;
        let innermost: Vec<bool> = (0..nests.len())
            .map(|n| nests[n].loose || self.nesting.innermost(n))
            .collect();
        let config = self.config;
        Groups {
            of_tile: self.nesting.paths.clone(),
            parent: nests.iter().map(|n| n.parent).collect(),
            cohesion: innermost
                .iter()
                .map(|&inner| config.cohesion * if inner { 1.0 } else { 0.6 })
                .collect(),
            gap: innermost
                .iter()
                .map(|&inner| config.link_length * if inner { 0.9 } else { 2.0 })
                .collect(),
            by_count: true,
        }
    }

    /// The pull of each tile towards the middle of its groups, and the push apart of
    /// sibling groups that crowd each other. Each group takes up a ball as wide as
    /// its tiles spread; two siblings closer than their radii plus a gap push apart,
    /// harder the more they overlap, the smaller one moving more. So a group stays
    /// whole and clear of its neighbors however many calls cross between.
    fn gather(&self, groups: &Groups) -> Vec<[f32; 3]> {
        let config = self.config;
        let count = groups.parent.len();
        let mut sums = vec![([0.0_f32; 3], 0.0_f32); count];
        for (position, of) in self.positions.iter().zip(&groups.of_tile) {
            for &group in of {
                let (sum, n) = &mut sums[group];
                *sum = add(*sum, *position);
                *n += 1.0;
            }
        }
        let middles: Vec<[f32; 3]> = sums
            .iter()
            .map(|&(sum, n)| scale(sum, 1.0 / n.max(1.0)))
            .collect();
        let mut apart = vec![[0.0_f32; 3]; count];
        if config.separation > 0.0 {
            let mut spread = vec![0.0_f32; count];
            for (position, of) in self.positions.iter().zip(&groups.of_tile) {
                for &group in of {
                    let offset = sub(*position, middles[group]);
                    spread[group] += dot(offset, offset);
                }
            }
            let radii: Vec<f32> = spread
                .iter()
                .zip(&sums)
                .map(|(&sum, &(_, n))| {
                    let spread = if groups.by_count {
                        n.sqrt() * config.link_length * 0.6
                    } else {
                        (sum / n.max(1.0)).sqrt() * 1.8
                    };
                    spread + config.link_length * 0.5
                })
                .collect();
            let mut siblings: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
            for group in (0..count).filter(|&g| sums[g].1 > 0.0) {
                siblings
                    .entry(groups.parent[group])
                    .or_default()
                    .push(group);
            }
            for family in siblings.values() {
                for (i, &a) in family.iter().enumerate() {
                    for &b in &family[i + 1..] {
                        let delta = sub(middles[a], middles[b]);
                        let distance = dot(delta, delta).sqrt().max(0.01);
                        let gap = groups.gap[a].max(groups.gap[b]);
                        let overlap = radii[a] + radii[b] + gap - distance;
                        if overlap <= 0.0 {
                            continue;
                        }
                        let push = scale(delta, config.separation * overlap / distance);
                        let (n_a, n_b) = (sums[a].1, sums[b].1);
                        let total = n_a + n_b;
                        apart[a] = add(apart[a], scale(push, n_b / total));
                        apart[b] = sub(apart[b], scale(push, n_a / total));
                    }
                }
            }
        }
        self.positions
            .iter()
            .zip(&groups.of_tile)
            .map(|(&position, of)| {
                of.iter().fold([0.0; 3], |force, &group| {
                    let pull = scale(sub(middles[group], position), groups.cohesion[group]);
                    add(add(force, apart[group]), pull)
                })
            })
            .collect()
    }
}

/// A point in a ball of `radius`, fixed by `key`, so the same codebase always starts
/// from the same shape.
fn seed(key: &str, radius: f32) -> [f32; 3] {
    let hash = blake3::hash(key.as_bytes());
    let bytes = hash.as_bytes();
    let unit = |i: usize| f32::from(u16::from_le_bytes([bytes[i], bytes[i + 1]])) / 65535.0;
    // Uniform direction, cube-root radius for an even fill.
    let z = unit(0) * 2.0 - 1.0;
    let angle = unit(2) * std::f32::consts::TAU;
    let r = radius * unit(4).cbrt();
    let ring = (1.0 - z * z).sqrt();
    [r * ring * angle.cos(), r * z, r * ring * angle.sin()]
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

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// For each tile, its cluster: the nearest module (or crate, package or file) holding
/// its container, numbered in order of first appearance. A tile whose container is not
/// inside one clusters on its own container.
fn module_clusters(system: &System, map: &CodebaseMap) -> (Vec<usize>, Vec<Option<UnitId>>) {
    const MODULES: [&str; 4] = ["module", "crate", "package", "file"];
    let mut numbers: HashMap<String, usize> = HashMap::new();
    let mut units = Vec::new();
    let clusters = map
        .tiles
        .iter()
        .map(|tile| {
            let container = map.groups.get(tile.group).and_then(|g| g.unit.as_ref());
            let mut at = container.and_then(|id| system.unit(id));
            while let Some(unit) = at {
                if MODULES.contains(&unit.kind.as_str()) {
                    break;
                }
                at = unit.parent.as_ref().and_then(|p| system.unit(p));
            }
            let key = at.map_or_else(
                || format!("group:{}", tile.group),
                |unit| unit.id.to_string(),
            );
            let next = numbers.len();
            *numbers.entry(key).or_insert_with(|| {
                units.push(at.map(|unit| unit.id.clone()));
                next
            })
        })
        .collect();
    (clusters, units)
}

#[cfg(test)]
mod tests {
    use codeviz_ir::Document;

    use super::*;
    use crate::{MapConfig, map};

    /// `a -> b -> c`, and `lonely` calls nothing.
    fn system() -> System {
        let call = |to: &str| {
            format!(
                r#"{{ "key": "{to}#0", "text": "{to}", "targets": [{{ "unit": "x:{to}", "certainty": "exact" }}] }}"#
            )
        };
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }},
              "units": [
                {{ "id": "x:a", "name": "a", "kind": "fn", "role": "invocable", "calls": [{}] }},
                {{ "id": "x:b", "name": "b", "kind": "fn", "role": "invocable", "calls": [{}] }},
                {{ "id": "x:c", "name": "c", "kind": "fn", "role": "invocable" }},
                {{ "id": "x:lonely", "name": "lonely", "kind": "fn", "role": "invocable" }}
              ] }}"#,
            call("b"),
            call("c")
        );
        System::new([Document::from_json(&json).unwrap()])
    }

    #[test]
    fn linked_functions_settle_closer_than_strangers() {
        let system = system();
        let map = map(&system, &MapConfig::default());
        let mut graph = ForceGraph::new(&system, &map, ForceConfig::default());
        let tile = |name: &str| map.tile_of(&codeviz_ir::UnitId::new(name)).unwrap();
        assert!(graph.linked(tile("x:a"), tile("x:b")));
        assert!(!graph.linked(tile("x:a"), tile("x:c")));
        assert_eq!(
            graph.callees(tile("x:a")).collect::<Vec<_>>(),
            [tile("x:b")]
        );
        assert_eq!(graph.callers(tile("x:a")).count(), 0);
        assert_eq!(graph.degree(tile("x:b")), 2);

        let mut speed = f32::MAX;
        for _ in 0..500 {
            speed = graph.step();
        }
        assert!(speed < 0.01, "settles, got {speed}");
        let distance = |a: &str, b: &str| {
            let d = sub(graph.positions[tile(a)], graph.positions[tile(b)]);
            dot(d, d).sqrt()
        };
        assert!(distance("x:a", "x:b") < distance("x:a", "x:lonely"));
    }

    #[test]
    fn reach_counts_calls_both_ways() {
        let system = system();
        let map = map(&system, &MapConfig::default());
        let graph = ForceGraph::new(&system, &map, ForceConfig::default());
        let tile = |name: &str| map.tile_of(&codeviz_ir::UnitId::new(name)).unwrap();
        let reach = graph.reach(tile("x:b"));
        assert_eq!(reach.upstream, HashMap::from([(tile("x:a"), 1)]));
        assert_eq!(reach.downstream, HashMap::from([(tile("x:c"), 1)]));
        let from_a = graph.reach(tile("x:a"));
        assert_eq!(from_a.downstream.get(&tile("x:c")), Some(&2));
        assert_eq!(graph.sequence(tile("x:a")), [tile("x:b")]);
        let stats = graph.group_stats();
        assert_eq!(stats.iter().map(|s| s.nodes).sum::<usize>(), 4);
        assert_eq!(
            stats.iter().map(|s| s.internal + s.outgoing).sum::<usize>(),
            2
        );
    }

    #[test]
    fn nested_containers_settle_apart_without_flying_off() {
        let json = r#"{
          "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" },
          "units": [
            { "id": "x:app", "name": "app", "kind": "crate", "role": "group" },
            { "id": "x:api", "name": "api", "kind": "module", "role": "group", "parent": "x:app" },
            { "id": "x:db", "name": "db", "kind": "module", "role": "group", "parent": "x:app" },
            { "id": "y:lib", "name": "lib", "kind": "crate", "role": "group" },
            { "id": "x:api::a", "name": "a", "kind": "fn", "role": "invocable", "parent": "x:api",
              "calls": [{ "key": "b#0", "text": "b", "targets": [{ "unit": "x:db::b", "certainty": "exact" }] },
                        { "key": "c#0", "text": "c", "targets": [{ "unit": "y:lib::c", "certainty": "exact" }] }] },
            { "id": "x:api::a2", "name": "a2", "kind": "fn", "role": "invocable", "parent": "x:api" },
            { "id": "x:db::b", "name": "b", "kind": "fn", "role": "invocable", "parent": "x:db",
              "calls": [{ "key": "c#0", "text": "c", "targets": [{ "unit": "y:lib::c", "certainty": "exact" }] }] },
            { "id": "x:db::b2", "name": "b2", "kind": "fn", "role": "invocable", "parent": "x:db" },
            { "id": "y:lib::c", "name": "c", "kind": "fn", "role": "invocable", "parent": "y:lib" },
            { "id": "y:lib::d", "name": "d", "kind": "fn", "role": "invocable", "parent": "y:lib" }
          ]
        }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let map = map(&system, &MapConfig::default());
        let config = ForceConfig {
            cohesion: 0.15,
            separation: 0.4,
            cross_spring: 0.3,
            nested: true,
            flatten: 1.0,
            ..ForceConfig::default()
        };
        let mut graph = ForceGraph::new(&system, &map, config);
        let mut speed = f32::MAX;
        for _ in 0..2000 {
            speed = graph.step();
        }
        assert!(speed < 0.01, "settles, got {speed}");
        let tile = |name: &str| map.tile_of(&UnitId::new(name)).unwrap();
        let at = |name: &str| graph.positions[tile(name)];
        let distance = |a: &str, b: &str| {
            let d = sub(at(a), at(b));
            dot(d, d).sqrt()
        };
        assert!(
            distance("x:api::a", "x:api::a2") < distance("x:api::a", "y:lib::d"),
            "a module's functions sit nearer each other than another crate's"
        );
        assert!(
            graph.positions.iter().all(|p| dot(*p, *p).sqrt() < 200.0),
            "nothing flies off"
        );
    }

    #[test]
    fn the_same_codebase_starts_from_the_same_shape() {
        let system = system();
        let map = map(&system, &MapConfig::default());
        let first = ForceGraph::new(&system, &map, ForceConfig::default());
        let second = ForceGraph::new(&system, &map, ForceConfig::default());
        assert_eq!(first.positions, second.positions);
    }
}
