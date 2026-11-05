//! The codebase as a graph of calls: one node per function (as tiled by the
//! [`CodebaseMap`]), one call per caller and callee, and each function's module.
//! [`crate::place`] lays it out; the viewer reads its calls.

use std::collections::{HashMap, HashSet};

use codeviz_core::System;
use codeviz_ir::UnitId;

use crate::CodebaseMap;

#[derive(Debug, Clone, Default)]
pub struct CallGraph {
    /// The cluster each tile is in: its container's module, so a module's types and
    /// free functions gather as one.
    clusters: Vec<usize>,
    /// The module each cluster stands for, by cluster.
    cluster_units: Vec<Option<UnitId>>,
    /// Every call between two tiles: `(caller, callee)`, each once, sorted, none from
    /// a function to itself.
    pub calls: Vec<(usize, usize)>,
    /// Per tile, the tiles it calls in the order the calls are written, repeats kept.
    sequences: Vec<Vec<usize>>,
    /// Per tile, how many other tiles it calls or is called by.
    degrees: Vec<usize>,
}

/// Where a call is written: line, column, then declaration order as a tiebreak.
type SiteKey = (u32, u32, usize);

impl CallGraph {
    #[must_use]
    pub fn new(system: &System, map: &CodebaseMap) -> Self {
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
        let links: HashSet<(usize, usize)> =
            calls.iter().map(|&(a, b)| (a.min(b), a.max(b))).collect();
        let mut calls: Vec<(usize, usize)> = calls.into_iter().collect();
        calls.sort_unstable();
        let mut degrees = vec![0; map.tiles.len()];
        for &(a, b) in &links {
            degrees[a] += 1;
            degrees[b] += 1;
        }

        let (clusters, cluster_units) = module_clusters(system, map);
        Self {
            clusters,
            cluster_units,
            calls,
            sequences,
            degrees,
        }
    }

    /// How many tiles there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.clusters.len()
    }

    /// Whether there are no tiles.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.clusters.is_empty()
    }

    /// The tiles `tile` calls, in the order the calls are written, repeats kept.
    #[must_use]
    pub fn sequence(&self, tile: usize) -> &[usize] {
        self.sequences.get(tile).map_or(&[], Vec::as_slice)
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

    /// Tiles that call `tile`, each once, in order.
    pub fn callers(&self, tile: usize) -> impl Iterator<Item = usize> + '_ {
        self.calls.iter().filter(move |c| c.1 == tile).map(|c| c.0)
    }

    /// Tiles `tile` calls, each once, in order.
    pub fn callees(&self, tile: usize) -> impl Iterator<Item = usize> + '_ {
        self.calls.iter().filter(move |c| c.0 == tile).map(|c| c.1)
    }

    /// How many other tiles `tile` calls or is called by.
    #[must_use]
    pub fn degree(&self, tile: usize) -> usize {
        self.degrees.get(tile).copied().unwrap_or(0)
    }
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
            let mut at = tile.container.as_ref().and_then(|id| system.unit(id));
            while let Some(unit) = at {
                if MODULES.contains(&unit.kind.as_str()) {
                    break;
                }
                at = unit.parent.as_ref().and_then(|p| system.unit(p));
            }
            let key = at.map_or_else(
                || format!("container:{:?}", tile.container),
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
    use crate::map;

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
    fn calls_run_one_way_in_the_order_written() {
        let system = system();
        let map = map(&system);
        let graph = CallGraph::new(&system, &map);
        let tile = |name: &str| map.tile_of(&codeviz_ir::UnitId::new(name)).unwrap();
        assert_eq!(graph.callers(tile("x:a")).count(), 0);
        assert_eq!(graph.degree(tile("x:b")), 2);
        assert_eq!(
            graph.callers(tile("x:b")).collect::<Vec<_>>(),
            [tile("x:a")]
        );
        assert_eq!(
            graph.callees(tile("x:b")).collect::<Vec<_>>(),
            [tile("x:c")]
        );
        assert_eq!(graph.sequence(tile("x:a")), [tile("x:b")]);
    }
}
