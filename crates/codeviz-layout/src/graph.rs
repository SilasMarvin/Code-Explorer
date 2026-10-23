//! The codebase as a force-directed graph in 3D: one node per function (as tiled by
//! the [`CodebaseMap`]), one link per pair of functions where one calls the other.
//! Nodes repel and links pull, so clusters come from what calls what; with cohesion
//! on, functions in the same container also drift together.
//!
//! The simulation runs a step at a time so a viewer can show it settling.

use std::collections::{HashMap, HashSet, VecDeque};

use codeviz_core::System;
use codeviz_ir::UnitId;

use crate::{CodebaseMap, repulsion};

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
    /// Whether each module keeps to a room of its own: a disc as big as its functions
    /// need, which none of them leaves and no other room overlaps, so a module stays
    /// compact and clear however its calls pull.
    pub rooms: bool,
    /// How hard a node that strays to the edge of its room is pulled back in.
    pub containment: f32,
    /// How hard each module is pulled towards the rest of its crate (or top-level
    /// package), when there are several, so a crate reads as one district.
    pub crate_cohesion: f32,
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
}

impl Default for ForceConfig {
    fn default() -> Self {
        Self {
            repulsion: 30.0,
            spring: 0.06,
            link_length: 6.0,
            cohesion: 0.02,
            separation: 0.0,
            rooms: false,
            containment: 0.0,
            crate_cohesion: 0.0,
            cross_spring: 1.0,
            gravity: 0.004,
            flatten: 0.0,
            damping: 0.82,
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
    /// The crate (or top-level package) each cluster sits in, by cluster.
    tops: Vec<usize>,
    /// The crate each top stands for, by top.
    top_units: Vec<Option<UnitId>>,
    /// Links between tiles, each pair once, lower index first.
    pub links: Vec<(usize, usize)>,
    /// The same links with their direction: `(caller, callee)`, sorted.
    pub calls: Vec<(usize, usize)>,
    /// Calls that still pull as links but no longer run, such as the calls a diff
    /// removed: kept out of [`Self::calls`], callers, reach and sequences.
    pub ghosts: Vec<(usize, usize)>,
    /// Per tile, the tiles it calls in the order the calls are written, repeats kept.
    sequences: Vec<Vec<usize>>,
    /// Per tile, the tiles that call it and the tiles it calls, each sorted, from
    /// [`Self::calls`].
    callers: Vec<Vec<usize>>,
    callees: Vec<Vec<usize>>,
    /// Per tile, how many links touch it.
    degrees: Vec<usize>,
    config: ForceConfig,
    /// Steps taken under this config, which cool the simulation as they add up.
    age: usize,
}

/// The furthest a node moves in one step.
const MAX_STEP: f32 = 2.0;
/// Steps that run at full speed; small graphs settle well within them.
const WARM_STEPS: usize = 400;
/// How much of its reach a step keeps from the one before, once cooling.
const COOLING: f32 = 0.993;

/// The share of [`MAX_STEP`] a node may move at this age: all of it at first, then
/// less each step, so even a graph too big to find its balance stops.
fn cooling(age: usize) -> f32 {
    let cooled = age.saturating_sub(WARM_STEPS);
    COOLING.powi(i32::try_from(cooled).unwrap_or(i32::MAX))
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
        let (tops, top_units) = top_clusters(system, &cluster_units);
        let positions = seed_positions(
            map,
            (&clusters, &cluster_units),
            (&tops, &top_units),
            config.link_length,
        );
        let mut degrees = vec![0; map.tiles.len()];
        for &(a, b) in &links {
            degrees[a] += 1;
            degrees[b] += 1;
        }
        let mut graph = Self {
            velocities: vec![[0.0; 3]; map.tiles.len()],
            groups: map.tiles.iter().map(|t| t.group).collect(),
            clusters,
            cluster_units,
            tops,
            top_units,
            positions,
            links,
            calls,
            ghosts: Vec::new(),
            sequences,
            callers: Vec::new(),
            callees: Vec::new(),
            degrees,
            config,
            age: 0,
        };
        graph.index_calls();
        graph
    }

    /// Lists each tile's callers and callees from [`Self::calls`].
    fn index_calls(&mut self) {
        let count = self.positions.len();
        self.callers = vec![Vec::new(); count];
        self.callees = vec![Vec::new(); count];
        // `calls` is sorted, so each list comes out sorted.
        for &(from, to) in &self.calls {
            self.callees[from].push(to);
            self.callers[to].push(from);
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
        self.index_calls();
    }

    /// Changes the forces; the graph moves until it settles under them.
    pub fn set_config(&mut self, config: ForceConfig) {
        self.config = config;
        self.age = 0;
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
            upstream: walk(&self.callers, tile),
            downstream: walk(&self.callees, tile),
        }
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

    /// How many clusters there are.
    #[must_use]
    pub fn cluster_count(&self) -> usize {
        self.cluster_units.len()
    }

    /// The crate (or top-level package) a cluster sits in.
    #[must_use]
    pub fn top(&self, cluster: usize) -> Option<usize> {
        self.tops.get(cluster).copied()
    }

    /// The crate a top stands for, when it has one.
    #[must_use]
    pub fn top_unit(&self, top: usize) -> Option<&UnitId> {
        self.top_units.get(top)?.as_ref()
    }

    /// How many crates (or top-level packages) there are.
    #[must_use]
    pub fn top_count(&self) -> usize {
        self.top_units.len()
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

    /// Tiles that call `tile`, in order.
    pub fn callers(&self, tile: usize) -> impl Iterator<Item = usize> + '_ {
        self.callers.get(tile).into_iter().flatten().copied()
    }

    /// Tiles `tile` calls, in order.
    pub fn callees(&self, tile: usize) -> impl Iterator<Item = usize> + '_ {
        self.callees.get(tile).into_iter().flatten().copied()
    }

    /// How many links touch `tile`.
    #[must_use]
    pub fn degree(&self, tile: usize) -> usize {
        self.degrees.get(tile).copied().unwrap_or_default()
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
        let mut forces = repulsion::forces(&self.positions, config.repulsion);
        for &(a, b) in &self.links {
            let delta = sub(self.positions[b], self.positions[a]);
            let distance = dot(delta, delta).sqrt().max(0.01);
            let spring = if self.clusters.get(a) == self.clusters.get(b) {
                config.spring
            } else {
                config.spring * config.cross_spring
            };
            let pull = scale(delta, spring * (distance - config.link_length) / distance);
            forces[a] = add(forces[a], pull);
            forces[b] = sub(forces[b], pull);
        }
        self.hold_regions(&mut forces);
        for (force, position) in forces.iter_mut().zip(&self.positions) {
            *force = sub(*force, scale(*position, config.gravity));
            force[2] -= position[2] * config.flatten;
        }

        // Keep any one step small so a crowded start cannot fling nodes away, and
        // smaller still the longer it runs, so a big graph comes to rest.
        let limit = MAX_STEP * cooling(self.age);
        self.age += 1;
        let before = self.positions.clone();
        let moving = self.velocities.iter_mut().zip(&mut self.positions);
        for ((velocity_slot, position), force) in moving.zip(&forces) {
            let velocity = scale(add(*velocity_slot, *force), config.damping);
            let length = dot(velocity, velocity).sqrt();
            let velocity = if length > limit {
                scale(velocity, limit / length)
            } else {
                velocity
            };
            *velocity_slot = velocity;
            *position = add(*position, velocity);
        }
        if config.rooms {
            self.keep_to_rooms();
        }
        // How far nodes actually went, once held to their rooms.
        let moved: f32 = before
            .iter()
            .zip(&self.positions)
            .map(|(&a, &b)| {
                let delta = sub(b, a);
                dot(delta, delta).sqrt()
            })
            .sum();
        moved / count as f32
    }

    /// Moves whole crates apart until no two districts overlap, and each module into
    /// its crate's district; then whole modules apart until no two rooms overlap; then
    /// brings back any node that left its room to the room's edge. What was moved
    /// back no longer heads the way it was stopped, so the graph comes to rest
    /// against its walls.
    fn keep_to_rooms(&mut self) {
        let spacing = self.config.link_length;
        let mut modules = Regions::measure(&self.positions, &self.clusters, self.cluster_count());
        let rooms: Vec<f32> = modules.counts.iter().map(|&n| room(n, spacing)).collect();
        let before = modules.middles.clone();
        if self.crates_apart() {
            let tops = self.tops_of(&modules);
            let districts = self.districts(&modules, &tops, &rooms);
            let shifts = separate(&tops, &districts, spacing * 2.0);
            for (cluster, middle) in modules.middles.iter_mut().enumerate() {
                *middle = add(*middle, shifts[self.tops[cluster]]);
            }
        }
        let shifts = separate(&modules, &rooms, spacing);
        let moves: Vec<[f32; 3]> = modules
            .middles
            .iter()
            .zip(&before)
            .zip(&shifts)
            .map(|((&after, &was), &shift)| add(sub(after, was), shift))
            .collect();
        let moving = self.positions.iter_mut().zip(&mut self.velocities);
        for ((position, velocity), &cluster) in moving.zip(&self.clusters) {
            let shift = moves[cluster];
            *position = add(*position, shift);
            // A module pushed back stops heading into what pushed it.
            let pushed = dot(shift, shift).sqrt();
            if pushed > 1e-4 {
                let back = scale(shift, 1.0 / pushed);
                let into = dot(*velocity, back).min(0.0);
                *velocity = sub(*velocity, scale(back, into));
            }
            let middle = add(before[cluster], shift);
            let [x, y, _] = sub(*position, middle);
            let distance = x.hypot(y);
            let edge = rooms[cluster] - spacing * 0.5;
            if distance > edge {
                let out = [x / distance, y / distance, 0.0];
                let back = distance - edge;
                *position = sub(*position, scale(out, back));
                let outward = dot(*velocity, out).max(0.0);
                *velocity = sub(*velocity, scale(out, outward));
            }
        }
    }

    /// Whether crates keep to districts of their own: when there are several, and
    /// their modules are pulled together.
    fn crates_apart(&self) -> bool {
        self.config.crate_cohesion > 0.0 && self.top_count() > 1
    }

    /// Each crate as a region, from its modules: their middle, weighted by size.
    fn tops_of(&self, modules: &Regions) -> Regions {
        let mut sums = vec![[0.0_f32; 3]; self.top_count()];
        let mut counts = vec![0; self.top_count()];
        for (cluster, &top) in self.tops.iter().enumerate() {
            let n = modules.counts[cluster];
            sums[top] = add(sums[top], scale(modules.middles[cluster], n as f32));
            counts[top] += n;
        }
        let middles = sums
            .iter()
            .zip(&counts)
            .map(|(&sum, &n)| scale(sum, 1.0 / (n.max(1) as f32)))
            .collect();
        Regions { middles, counts }
    }

    /// How wide each crate's district is: room for its modules' rooms loosely packed,
    /// or as far as they actually reach from its middle, whichever is wider.
    fn districts(&self, modules: &Regions, tops: &Regions, rooms: &[f32]) -> Vec<f32> {
        let gap = self.config.link_length;
        let mut area = vec![0.0_f32; self.top_count()];
        let mut reach = vec![0.0_f32; self.top_count()];
        for (cluster, &top) in self.tops.iter().enumerate() {
            if modules.counts[cluster] == 0 {
                continue;
            }
            area[top] += (rooms[cluster] + gap / 2.0).powi(2);
            let [x, y, _] = sub(modules.middles[cluster], tops.middles[top]);
            reach[top] = reach[top].max(x.hypot(y) + rooms[cluster]);
        }
        // Discs packed loosely fill about three fifths of the disc round them.
        area.iter()
            .zip(&reach)
            .map(|(&a, &r)| (a / 0.6).sqrt().max(r))
            .collect()
    }

    /// Keeps each module together in a room of its own, clear of the others, and
    /// each crate's modules together when there are several crates. A whole module
    /// moves as one when pushed, so it stays whole however many calls cross between.
    fn hold_regions(&self, forces: &mut [[f32; 3]]) {
        let config = self.config;
        let modules = Regions::measure(&self.positions, &self.clusters, self.cluster_count());
        let gap = config.link_length;
        let rooms: Vec<f32> = modules
            .counts
            .iter()
            .map(|&n| room(n, config.link_length))
            .collect();
        // Each module's push, the same for every node in it.
        let mut moves = if config.separation > 0.0 {
            keep_apart(&modules, &rooms, gap, config.separation)
        } else {
            vec![[0.0; 3]; modules.middles.len()]
        };
        if self.crates_apart() {
            let tops = self.tops_of(&modules);
            let districts = self.districts(&modules, &tops, &rooms);
            let apart = keep_apart(&tops, &districts, gap * 2.0, config.separation);
            for (cluster, (slot, &middle)) in moves.iter_mut().zip(&modules.middles).enumerate() {
                let top = self.tops[cluster];
                let inward = scale(sub(tops.middles[top], middle), config.crate_cohesion);
                *slot = add(add(*slot, inward), apart[top]);
            }
        }
        for (node, force) in forces.iter_mut().enumerate() {
            let Some(&cluster) = self.clusters.get(node) else {
                continue;
            };
            let position = self.positions[node];
            let middle = modules.middles[cluster];
            let inward = sub(middle, position);
            *force = add(*force, moves[cluster]);
            *force = add(*force, scale(inward, config.cohesion));
            let distance = dot(inward, inward).sqrt();
            let outside = distance - rooms[cluster];
            if config.containment > 0.0 && outside > 0.0 {
                *force = add(
                    *force,
                    scale(inward, config.containment * outside / distance),
                );
            }
        }
    }
}

/// Where each region's nodes are: their middle, and how many.
struct Regions {
    middles: Vec<[f32; 3]>,
    counts: Vec<usize>,
}

impl Regions {
    /// Each of `count` regions, from the region of each node.
    fn measure(positions: &[[f32; 3]], regions: &[usize], count: usize) -> Self {
        let mut sums = vec![[0.0_f32; 3]; count];
        let mut counts = vec![0; count];
        for (position, &region) in positions.iter().zip(regions) {
            sums[region] = add(sums[region], *position);
            counts[region] += 1;
        }
        let middles = sums
            .iter()
            .zip(&counts)
            .map(|(&sum, &n)| scale(sum, 1.0 / (n.max(1) as f32)))
            .collect();
        Self { middles, counts }
    }
}

/// How wide a room `count` functions need, `spacing` apart.
#[must_use]
pub(crate) fn room(count: usize, spacing: f32) -> f32 {
    spacing * (0.5 + 0.55 * (count as f32).sqrt())
}

/// How each region must move so no two of their discs, `radii` wide, come closer than
/// `gap` across the plane facing the view: two that do push apart, harder the more
/// they overlap, the smaller moving more.
fn keep_apart(regions: &Regions, radii: &[f32], gap: f32, strength: f32) -> Vec<[f32; 3]> {
    let count = regions.middles.len();
    let mut moves = vec![[0.0_f32; 3]; count];
    for (a, b) in crowding(&regions.middles, &regions.counts, radii, gap) {
        let (n_a, n_b) = (regions.counts[a] as f32, regions.counts[b] as f32);
        // Apart in the plane facing the view, so no region hides behind another.
        let [x, y, _] = sub(regions.middles[a], regions.middles[b]);
        let delta = [x, y, 0.0];
        let distance = dot(delta, delta).sqrt().max(0.01);
        let overlap = radii[a] + radii[b] + gap - distance;
        if overlap <= 0.0 {
            continue;
        }
        let push = scale(delta, strength * overlap / distance);
        let total = n_a + n_b;
        moves[a] = add(moves[a], scale(push, n_b / total));
        moves[b] = sub(moves[b], scale(push, n_a / total));
    }
    moves
}

/// Pairs of regions whose discs, `radii` wide, might come within `gap` of each other:
/// every pair that does, and few that do not. Sorted by left edge and swept, so a big
/// graph's hundreds of modules are not each checked against all the others.
fn crowding(
    middles: &[[f32; 3]],
    counts: &[usize],
    radii: &[f32],
    gap: f32,
) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..middles.len()).filter(|&r| counts[r] > 0).collect();
    let left = |r: usize| middles[r][0] - radii[r];
    order.sort_by(|&a, &b| left(a).total_cmp(&left(b)).then(a.cmp(&b)));
    let mut pairs = Vec::new();
    for (index, &a) in order.iter().enumerate() {
        let right = middles[a][0] + radii[a] + gap;
        for &b in order[index + 1..].iter().take_while(|&&b| left(b) <= right) {
            let reach = radii[a] + radii[b] + gap;
            if (middles[a][1] - middles[b][1]).abs() < reach {
                pairs.push((a.min(b), a.max(b)));
            }
        }
    }
    pairs.sort_unstable();
    pairs
}

/// How far each region must move, in the plane facing the view, so no two of their
/// discs, `radii` wide, come closer than `gap`: each overlap is shared out, the smaller
/// region moving more. Every overlap in a round is worked out before any moves, so
/// the order regions come in does not push them one way; a few rounds settle what
/// one move makes overlap anew.
fn separate(regions: &Regions, radii: &[f32], gap: f32) -> Vec<[f32; 3]> {
    const ROUNDS: usize = 4;
    let count = regions.middles.len();
    let mut middles = regions.middles.clone();
    for _ in 0..ROUNDS {
        let mut moves = vec![[0.0_f32; 3]; count];
        let mut any = false;
        for (a, b) in crowding(&middles, &regions.counts, radii, gap) {
            let (n_a, n_b) = (regions.counts[a] as f32, regions.counts[b] as f32);
            let [x, y, _] = sub(middles[a], middles[b]);
            let distance = x.hypot(y);
            let overlap = radii[a] + radii[b] + gap - distance;
            if overlap <= 0.0 {
                continue;
            }
            // Two on one spot part along a direction fixed by which they are.
            let along = if distance > 1e-3 {
                [x / distance, y / distance, 0.0]
            } else {
                let angle = (a * 7 + b * 13) as f32;
                [angle.cos(), angle.sin(), 0.0]
            };
            let total = n_a + n_b;
            moves[a] = add(moves[a], scale(along, overlap * n_b / total));
            moves[b] = sub(moves[b], scale(along, overlap * n_a / total));
            any = true;
        }
        if !any {
            break;
        }
        for (middle, shift) in middles.iter_mut().zip(&moves) {
            *middle = add(*middle, scale(*shift, 0.5));
        }
    }
    middles
        .iter()
        .zip(&regions.middles)
        .map(|(&after, &before)| sub(after, before))
        .collect()
}

/// Where each tile starts: its crate somewhere in a ball round the origin, its module
/// somewhere in its crate, itself somewhere in its module's room, each place fixed by
/// a name, so the same codebase always starts from the same shape and each module
/// starts gathered.
fn seed_positions(
    map: &CodebaseMap,
    (clusters, cluster_units): (&[usize], &[Option<UnitId>]),
    (tops, top_units): (&[usize], &[Option<UnitId>]),
    spacing: f32,
) -> Vec<[f32; 3]> {
    let name = |unit: Option<&Option<UnitId>>, index: usize| {
        unit.and_then(Option::as_ref)
            .map_or_else(|| format!("#{index}"), ToString::to_string)
    };
    let mut sizes = vec![0; tops.len()];
    for &cluster in clusters {
        sizes[cluster] += 1;
    }
    let mut top_sizes = vec![0; tops.iter().max().map_or(0, |t| t + 1)];
    for (cluster, &top) in tops.iter().enumerate() {
        top_sizes[top] += sizes[cluster];
    }
    let world = room(map.tiles.len(), spacing);
    map.tiles
        .iter()
        .zip(clusters)
        .map(|(tile, &cluster)| {
            let top = tops[cluster];
            let crate_seed = seed(&name(top_units.get(top), top), world);
            let module_seed = seed(
                &name(cluster_units.get(cluster), cluster),
                room(top_sizes[top], spacing),
            );
            let own = seed(tile.unit.as_str(), room(sizes[cluster], spacing) * 0.7);
            add(add(crate_seed, module_seed), own)
        })
        .collect()
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

/// Breadth-first from `start` along `next`, each tile with the fewest steps to it.
fn walk(next: &[Vec<usize>], start: usize) -> HashMap<usize, u32> {
    let mut found = HashMap::new();
    let mut queue = VecDeque::from([(start, 0)]);
    while let Some((tile, hops)) = queue.pop_front() {
        for &to in next.get(tile).into_iter().flatten() {
            if to != start && !found.contains_key(&to) {
                found.insert(to, hops + 1);
                queue.push_back((to, hops + 1));
            }
        }
    }
    found
}

pub(crate) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
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

/// For each cluster, its top: the outermost crate or package holding its module,
/// numbered in order of first appearance, each with the unit it stands for.
fn top_clusters(
    system: &System,
    cluster_units: &[Option<UnitId>],
) -> (Vec<usize>, Vec<Option<UnitId>>) {
    const TOPS: [&str; 2] = ["crate", "package"];
    let mut numbers: HashMap<Option<UnitId>, usize> = HashMap::new();
    let mut units = Vec::new();
    let tops = cluster_units
        .iter()
        .map(|unit| {
            let top = unit.as_ref().and_then(|id| {
                std::iter::once(system.unit(id)?)
                    .chain(system.ancestors(id))
                    .rfind(|u| TOPS.contains(&u.kind.as_str()))
                    .map(|u| u.id.clone())
            });
            let next = numbers.len();
            *numbers.entry(top.clone()).or_insert_with(|| {
                units.push(top);
                next
            })
        })
        .collect();
    (tops, units)
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

    /// `count` functions, each calling the next.
    fn chain(count: usize) -> System {
        let units: Vec<String> = (0..count)
            .map(|i| {
                format!(
                    r#"{{ "id": "x:f{i}", "name": "f{i}", "kind": "fn", "role": "invocable",
                      "calls": [{{ "key": "c", "text": "f", "targets": [{{ "unit": "x:f{}", "certainty": "exact" }}] }}] }}"#,
                    (i + 1) % count
                )
            })
            .collect();
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
            units.join(",")
        );
        System::new([Document::from_json(&json).unwrap()])
    }

    #[test]
    fn a_big_graph_settles_the_same_way_every_time() {
        let system = chain(900);
        let map = map(&system, &MapConfig::default());
        let settle = || {
            let mut graph = ForceGraph::new(&system, &map, ForceConfig::default());
            for _ in 0..20 {
                graph.step();
            }
            graph.positions
        };
        assert_eq!(settle(), settle());
    }

    #[test]
    fn a_big_graph_comes_to_rest() {
        let system = chain(500);
        let map = map(&system, &MapConfig::default());
        let mut graph = ForceGraph::new(&system, &map, ForceConfig::default());
        let mut steps = 0;
        while graph.step() > 0.002 {
            steps += 1;
            assert!(steps < 3000, "still moving after {steps} steps");
        }
    }

    /// Two modules of `count` functions each, every one calling every function in
    /// the other module.
    fn tangled(count: usize) -> System {
        let mut units = vec![
            r#"{ "id": "x:m0", "name": "m0", "kind": "module", "role": "group" }"#.to_owned(),
            r#"{ "id": "x:m1", "name": "m1", "kind": "module", "role": "group" }"#.to_owned(),
        ];
        for module in 0..2 {
            for i in 0..count {
                let calls: Vec<String> = (0..count)
                    .map(|j| {
                        format!(
                            r#"{{ "key": "c{j}", "text": "f", "targets": [{{ "unit": "x:m{}f{j}", "certainty": "exact" }}] }}"#,
                            1 - module
                        )
                    })
                    .collect();
                units.push(format!(
                    r#"{{ "id": "x:m{module}f{i}", "name": "f{i}", "kind": "fn", "role": "invocable", "parent": "x:m{module}", "calls": [{}] }}"#,
                    calls.join(",")
                ));
            }
        }
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
            units.join(",")
        );
        System::new([Document::from_json(&json).unwrap()])
    }

    #[test]
    fn modules_keep_to_rooms_of_their_own() {
        let system = tangled(12);
        let map = map(&system, &MapConfig::default());
        let config = ForceConfig {
            cohesion: 0.15,
            separation: 0.4,
            rooms: true,
            cross_spring: 0.3,
            ..ForceConfig::default()
        };
        let mut graph = ForceGraph::new(&system, &map, config);
        let mut steps = 0;
        while graph.step() > 0.002 {
            steps += 1;
            assert!(steps < 3000, "still moving after {steps} steps");
        }
        let modules = Regions::measure(&graph.positions, &graph.clusters, graph.cluster_count());
        let spread = |cluster: usize| {
            graph
                .positions
                .iter()
                .zip(&graph.clusters)
                .filter(|&(_, &c)| c == cluster)
                .map(|(p, _)| {
                    let [x, y, _] = sub(*p, modules.middles[cluster]);
                    x.hypot(y)
                })
                .fold(0.0, f32::max)
        };
        let [x, y, _] = sub(modules.middles[0], modules.middles[1]);
        assert!(
            spread(0) + spread(1) < x.hypot(y),
            "however much they call each other, the modules stay apart"
        );
        assert!(spread(0) <= room(12, config.link_length));
    }

    #[test]
    fn the_sweep_finds_every_pair_that_crowds() {
        let middles: Vec<[f32; 3]> = (0..60)
            .map(|i| seed(&format!("region {i}"), 80.0))
            .collect();
        let counts = vec![1; middles.len()];
        let radii: Vec<f32> = (0..middles.len()).map(|i| 2.0 + (i % 7) as f32).collect();
        let found = crowding(&middles, &counts, &radii, 3.0);
        for a in 0..middles.len() {
            for b in (a + 1)..middles.len() {
                let [x, y, _] = sub(middles[a], middles[b]);
                if x.hypot(y) < radii[a] + radii[b] + 3.0 {
                    assert!(found.contains(&(a, b)), "missed {a} and {b}");
                }
            }
        }
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
