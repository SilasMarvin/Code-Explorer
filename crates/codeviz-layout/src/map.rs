//! A stable map of the whole codebase: one tile per function, the tiles grouped by
//! container, with the groups ordered from entry points towards what they call.
//!
//! Closures and other units nested in a function share its tile.

use std::collections::{HashMap, VecDeque};

use codeviz_core::System;
use codeviz_ir::{Role, Unit, UnitId};

#[derive(Debug, Clone, PartialEq)]
pub struct MapTile {
    pub unit: UnitId,
    /// The nearest container that is not a function, when it has one.
    pub container: Option<UnitId>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodebaseMap {
    pub tiles: Vec<MapTile>,
    /// The tile each invocable unit is drawn on, by index into `tiles`.
    owner: HashMap<UnitId, usize>,
}

impl CodebaseMap {
    /// The tile `id` is drawn on: its own, or its enclosing function's.
    #[must_use]
    pub fn tile_of(&self, id: &UnitId) -> Option<usize> {
        self.owner.get(id).copied()
    }

    #[must_use]
    pub fn tile(&self, index: usize) -> Option<&MapTile> {
        self.tiles.get(index)
    }
}

#[must_use]
pub fn map(system: &System) -> CodebaseMap {
    let (owner_unit, members) = groups(system);
    let tiles: Vec<MapTile> = members
        .into_iter()
        .flat_map(|(container, _, units)| {
            units.into_iter().map(move |unit| MapTile {
                unit: unit.id.clone(),
                container: container.clone(),
            })
        })
        .collect();
    let tile_index: HashMap<&UnitId, usize> = tiles
        .iter()
        .enumerate()
        .map(|(i, t)| (&t.unit, i))
        .collect();
    let owner: HashMap<UnitId, usize> = owner_unit
        .iter()
        .filter_map(|(unit, top)| Some((unit.clone(), *tile_index.get(top)?)))
        .collect();
    CodebaseMap { tiles, owner }
}

/// A container, the name it is ordered by, and its tiles.
type Group<'s> = (Option<UnitId>, String, Vec<&'s Unit>);

/// Which tile each invocable is drawn on, and the tiles grouped by their nearest
/// non-invocable container, ordered by how close they are to an entry.
fn groups(system: &System) -> (HashMap<UnitId, UnitId>, Vec<Group<'_>>) {
    // Tiles are invocables not nested in another invocable.
    let is_invocable = |id: &UnitId| system.unit(id).is_some_and(|u| u.role == Role::Invocable);
    let mut owner_unit: HashMap<UnitId, UnitId> = HashMap::new();
    let mut tile_units: Vec<&Unit> = Vec::new();
    for unit in system.units().filter(|u| u.role == Role::Invocable) {
        let mut top = &unit.id;
        while let Some(parent) = system
            .unit(top)
            .and_then(|u| u.parent.as_ref())
            .filter(|p| is_invocable(p))
        {
            top = parent;
        }
        if top == &unit.id {
            tile_units.push(unit);
        }
        owner_unit.insert(unit.id.clone(), top.clone());
    }

    let rank = ranks(system, &owner_unit);
    let unreached = u32::MAX;
    let rank_of = |id: &UnitId| rank.get(id).copied().unwrap_or(unreached);

    // Group tiles by their nearest non-invocable container.
    let mut group_index: HashMap<Option<UnitId>, usize> = HashMap::new();
    let mut members: Vec<(Option<UnitId>, Vec<&Unit>)> = Vec::new();
    for unit in tile_units {
        let key = unit.parent.clone().filter(|p| system.unit(p).is_some());
        let index = *group_index.entry(key.clone()).or_insert_with(|| {
            members.push((key, Vec::new()));
            members.len() - 1
        });
        if let Some((_, units)) = members.get_mut(index) {
            units.push(unit);
        }
    }
    for (_, units) in &mut members {
        units.sort_by(|a, b| {
            rank_of(&a.id)
                .cmp(&rank_of(&b.id))
                .then_with(|| a.name.cmp(&b.name))
        });
    }
    let mut members: Vec<Group> = members
        .into_iter()
        .map(|(key, units)| {
            let label = key.as_ref().map_or_else(
                || "(top level)".to_owned(),
                |id| container_label(system, id),
            );
            (key, label, units)
        })
        .collect();
    members.sort_by_cached_key(|(_, label, units)| {
        let first = units.first().map_or(unreached, |u| rank_of(&u.id));
        (first, label.clone())
    });
    (owner_unit, members)
}

/// Fewest calls from any entry to each tile, following calls forward.
fn ranks(system: &System, owner: &HashMap<UnitId, UnitId>) -> HashMap<UnitId, u32> {
    let mut rank: HashMap<UnitId, u32> = HashMap::new();
    let mut queue = VecDeque::new();
    for entry in system.entries() {
        if let Some(tile) = owner.get(&entry.unit)
            && !rank.contains_key(tile)
        {
            rank.insert(tile.clone(), 0);
            queue.push_back(tile.clone());
        }
    }
    // Calls made anywhere inside a tile, closures included, count as the tile's.
    let mut calls: HashMap<&UnitId, Vec<&UnitId>> = HashMap::new();
    for unit in system.units() {
        if let Some(from) = owner.get(&unit.id) {
            let targets = unit
                .calls
                .iter()
                .flat_map(|c| &c.targets)
                .filter_map(|t| owner.get(&t.unit));
            calls.entry(from).or_default().extend(targets);
        }
    }
    while let Some(tile) = queue.pop_front() {
        let next = rank.get(&tile).copied().unwrap_or_default() + 1;
        for &callee in calls.get(&tile).map_or(&[][..], Vec::as_slice) {
            if !rank.contains_key(callee) {
                rank.insert(callee.clone(), next);
                queue.push_back(callee.clone());
            }
        }
    }
    rank
}

/// `services.orders`, `Store`: the container's name under its own parent's, while that
/// parent is not the outermost container.
fn container_label(system: &System, id: &UnitId) -> String {
    let Some(unit) = system.unit(id) else {
        return id.to_string();
    };
    let mut parts = vec![unit.name.as_str()];
    let mut current = unit;
    while let Some(parent) = current.parent.as_ref().and_then(|p| system.unit(p)) {
        if parent.parent.is_none() || parent.role != Role::Group {
            break;
        }
        parts.push(parent.name.as_str());
        current = parent;
    }
    parts.reverse();
    parts.join(".")
}

#[cfg(test)]
mod tests {
    use codeviz_ir::Document;

    use super::*;

    fn system() -> System {
        let json = r#"{
          "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" },
          "entries": [{ "unit": "x:api::handle", "label": "GET /", "kind": "http" }],
          "units": [
            { "id": "x:app", "name": "app", "kind": "crate", "role": "group" },
            { "id": "x:api", "name": "api", "kind": "module", "role": "group", "parent": "x:app" },
            { "id": "x:db", "name": "db", "kind": "module", "role": "group", "parent": "x:app" },
            { "id": "x:api::handle", "name": "handle", "kind": "fn", "role": "invocable", "parent": "x:api" },
            { "id": "x:api::handle::{closure#0}", "name": "{closure#0}", "kind": "closure",
              "role": "invocable", "parent": "x:api::handle", "calls": [
              { "key": "load#0", "text": "load", "targets": [{ "unit": "x:db::load", "certainty": "exact" }] } ] },
            { "id": "x:db::load", "name": "load", "kind": "fn", "role": "invocable", "parent": "x:db" },
            { "id": "x:db::unused", "name": "unused", "kind": "fn", "role": "invocable", "parent": "x:db" }
          ]
        }"#;
        System::new([Document::from_json(json).unwrap()])
    }

    #[test]
    fn groups_flow_from_entries_to_what_they_call() {
        let system = system();
        let map = map(&system);

        let units: Vec<&str> = map.tiles.iter().map(|t| t.unit.as_str()).collect();
        assert_eq!(
            units,
            ["x:api::handle", "x:db::load", "x:db::unused"],
            "closures share their function's tile; reached tiles come first"
        );
        let containers: Vec<Option<&str>> = map
            .tiles
            .iter()
            .map(|t| t.container.as_ref().map(UnitId::as_str))
            .collect();
        assert_eq!(containers, [Some("x:api"), Some("x:db"), Some("x:db")]);
        assert_eq!(
            map.tile_of(&UnitId::new("x:api::handle::{closure#0}")),
            Some(0)
        );
    }
}
