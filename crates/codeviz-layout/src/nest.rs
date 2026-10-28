//! How containers nest around the graph's functions: crate, module, nested module,
//! type. Each container is a [`Nest`]; every tile has a path of them, outermost first.
//!
//! A container holding functions of its own beside nested containers gets one more
//! nest, a loose one, for those functions alone, so they gather apart from the nested
//! containers instead of mingling with them.

use std::collections::HashMap;

use codeviz_core::System;
use codeviz_ir::UnitId;

use crate::CodebaseMap;

/// One container around some of the graph's functions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nest {
    /// The container.
    pub unit: UnitId,
    /// The nest it sits in.
    pub parent: Option<usize>,
    /// How many containers it sits in.
    pub depth: usize,
    /// Whether it stands for its container's own functions, apart from the containers
    /// nested in it: it gathers them, but is not a container of its own.
    pub loose: bool,
}

/// Every nest, and each tile's path through them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Nesting {
    pub nests: Vec<Nest>,
    /// Per tile, the nests holding it, outermost first.
    pub paths: Vec<Vec<usize>>,
}

impl Nesting {
    #[must_use]
    pub fn new(system: &System, map: &CodebaseMap) -> Self {
        let mut nesting = Self::default();
        let mut index: HashMap<UnitId, usize> = HashMap::new();
        for tile in &map.tiles {
            let container = map.groups.get(tile.group).and_then(|g| g.unit.as_ref());
            // The container and everything around it, innermost first.
            let mut chain = Vec::new();
            let mut at = container.and_then(|id| system.unit(id));
            while let Some(unit) = at {
                chain.push(unit.id.clone());
                at = unit.parent.as_ref().and_then(|p| system.unit(p));
            }
            let mut path = Vec::new();
            for unit in chain.into_iter().rev() {
                let parent = path.last().copied();
                let fresh = nesting.nests.len();
                let nest = *index.entry(unit.clone()).or_insert_with(|| {
                    nesting.nests.push(Nest {
                        unit,
                        parent,
                        depth: path.len(),
                        loose: false,
                    });
                    fresh
                });
                path.push(nest);
            }
            nesting.paths.push(path);
        }
        nesting.loosen();
        nesting
    }

    /// Gives the functions of a container that also holds containers a loose nest.
    fn loosen(&mut self) {
        let mut parents = vec![false; self.nests.len()];
        for nest in &self.nests {
            if let Some(parent) = nest.parent {
                parents[parent] = true;
            }
        }
        let mut loose: HashMap<usize, usize> = HashMap::new();
        for path in &mut self.paths {
            let Some(&inner) = path.last().filter(|&&n| parents[n]) else {
                continue;
            };
            let fresh = self.nests.len();
            let nest = *loose.entry(inner).or_insert_with(|| {
                let container = &self.nests[inner];
                self.nests.push(Nest {
                    unit: container.unit.clone(),
                    parent: Some(inner),
                    depth: container.depth + 1,
                    loose: true,
                });
                fresh
            });
            path.push(nest);
        }
    }

    /// The innermost container holding `tile`, loose nests aside.
    #[must_use]
    pub fn container(&self, tile: usize) -> Option<usize> {
        self.paths
            .get(tile)?
            .iter()
            .rev()
            .copied()
            .find(|&n| !self.nests[n].loose)
    }

    /// Whether a nest holds no nests but loose ones.
    #[must_use]
    pub fn innermost(&self, nest: usize) -> bool {
        !self
            .nests
            .iter()
            .any(|n| n.parent == Some(nest) && !n.loose)
    }
}

#[cfg(test)]
mod tests {
    use codeviz_ir::Document;

    use super::*;
    use crate::{MapConfig, map};

    #[test]
    fn functions_beside_nested_containers_get_a_loose_nest() {
        let json = r#"{
          "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" },
          "units": [
            { "id": "x:app", "name": "app", "kind": "crate", "role": "group" },
            { "id": "x:db", "name": "db", "kind": "module", "role": "group", "parent": "x:app" },
            { "id": "x:db::Pool", "name": "Pool", "kind": "struct", "role": "group", "parent": "x:db" },
            { "id": "x:db::load", "name": "load", "kind": "fn", "role": "invocable", "parent": "x:db" },
            { "id": "x:db::Pool::get", "name": "get", "kind": "fn", "role": "invocable", "parent": "x:db::Pool" }
          ]
        }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let map = map(&system, &MapConfig::default());
        let nesting = Nesting::new(&system, &map);
        let tile = |id: &str| map.tile_of(&UnitId::new(id)).unwrap();
        let names = |tile: usize| -> Vec<(String, bool)> {
            nesting.paths[tile]
                .iter()
                .map(|&n| (nesting.nests[n].unit.to_string(), nesting.nests[n].loose))
                .collect()
        };
        assert_eq!(
            names(tile("x:db::Pool::get")),
            [
                ("x:app".to_owned(), false),
                ("x:db".to_owned(), false),
                ("x:db::Pool".to_owned(), false)
            ]
        );
        assert_eq!(
            names(tile("x:db::load")),
            [
                ("x:app".to_owned(), false),
                ("x:db".to_owned(), false),
                ("x:db".to_owned(), true)
            ]
        );
        let db = nesting.container(tile("x:db::load")).unwrap();
        assert_eq!(nesting.nests[db].depth, 1);
        assert!(!nesting.innermost(db));
        assert!(nesting.innermost(nesting.container(tile("x:db::Pool::get")).unwrap()));
    }
}
