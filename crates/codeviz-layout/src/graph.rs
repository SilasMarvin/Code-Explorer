//! The codebase as a force-directed graph in 3D: one node per function (as tiled by
//! the [`CodebaseMap`]), one link per pair of functions where one calls the other.
//! Nodes repel, links pull, and functions in the same container drift together, so
//! clusters come from what calls what.
//!
//! The simulation runs a step at a time so a viewer can show it settling.

use std::collections::HashSet;

use codeviz_core::System;

use crate::CodebaseMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForceConfig {
    /// How hard every pair of nodes pushes apart.
    pub repulsion: f32,
    /// How hard a link pulls towards its rest length.
    pub spring: f32,
    pub link_length: f32,
    /// How hard a node is pulled towards the middle of its container.
    pub cohesion: f32,
    /// How hard everything is pulled towards the origin.
    pub gravity: f32,
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
            gravity: 0.004,
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
    /// Links between tiles, each pair once, lower index first.
    pub links: Vec<(usize, usize)>,
    config: ForceConfig,
}

impl ForceGraph {
    /// Seeds every tile at a deterministic point in a ball around the origin.
    #[must_use]
    pub fn new(system: &System, map: &CodebaseMap, config: ForceConfig) -> Self {
        let mut links = HashSet::new();
        for unit in system.units() {
            let Some(from) = map.tile_of(&unit.id) else {
                continue;
            };
            for target in unit.calls.iter().flat_map(|c| &c.targets) {
                if let Some(to) = map.tile_of(&target.unit)
                    && to != from
                {
                    links.insert((from.min(to), from.max(to)));
                }
            }
        }
        let mut links: Vec<(usize, usize)> = links.into_iter().collect();
        links.sort_unstable();

        let radius = (map.tiles.len() as f32).sqrt() * 2.0;
        let positions = map
            .tiles
            .iter()
            .map(|tile| seed(tile.unit.as_str(), radius))
            .collect();
        Self {
            velocities: vec![[0.0; 3]; map.tiles.len()],
            groups: map.tiles.iter().map(|t| t.group).collect(),
            positions,
            links,
            config,
        }
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
            let pull = scale(
                delta,
                config.spring * (distance - config.link_length) / distance,
            );
            forces[a] = add(forces[a], pull);
            forces[b] = sub(forces[b], pull);
        }
        let mut centers: Vec<([f32; 3], f32)> = Vec::new();
        for (position, &group) in self.positions.iter().zip(&self.groups) {
            if centers.len() <= group {
                centers.resize(group + 1, ([0.0; 3], 0.0));
            }
            if let Some((sum, n)) = centers.get_mut(group) {
                *sum = add(*sum, *position);
                *n += 1.0;
            }
        }
        for (i, force) in forces.iter_mut().enumerate() {
            let position = self.positions[i];
            if let Some(&(sum, n)) = self.groups.get(i).and_then(|&g| centers.get(g)) {
                let center = scale(sum, 1.0 / n);
                *force = add(*force, scale(sub(center, position), config.cohesion));
            }
            *force = sub(*force, scale(position, config.gravity));
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
    fn the_same_codebase_starts_from_the_same_shape() {
        let system = system();
        let map = map(&system, &MapConfig::default());
        let first = ForceGraph::new(&system, &map, ForceConfig::default());
        let second = ForceGraph::new(&system, &map, ForceConfig::default());
        assert_eq!(first.positions, second.positions);
    }
}
