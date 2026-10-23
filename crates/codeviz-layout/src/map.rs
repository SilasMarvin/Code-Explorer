//! A stable map of the whole codebase: one tile per function (or per external unit or
//! channel it calls), grouped into boxes by container, with the groups ordered from
//! entry points towards what they call.
//!
//! Closures and other units nested in a function share its tile. The map grows up from
//! `y = 0`, so it can sit on top of a journey.

use std::collections::{HashMap, VecDeque};

use codeviz_core::System;
use codeviz_ir::{Role, Unit, UnitId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapConfig {
    pub tile_width: f32,
    pub tile_height: f32,
    pub gap: f32,
    /// Room above a group's tiles for its name.
    pub header: f32,
    /// Tiles per column before a group wraps into another column.
    pub max_rows: usize,
    /// Groups wrap into another band past this width.
    pub max_width: f32,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            tile_width: 4.4,
            tile_height: 0.7,
            gap: 0.2,
            header: 1.1,
            max_rows: 12,
            max_width: 80.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapTile {
    pub unit: UnitId,
    pub group: usize,
    /// Center of the tile.
    pub position: [f32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapGroup {
    /// The container, when the tiles have one.
    pub unit: Option<UnitId>,
    pub label: String,
    pub min: [f32; 2],
    pub max: [f32; 2],
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodebaseMap {
    pub tiles: Vec<MapTile>,
    pub groups: Vec<MapGroup>,
    /// The tile each invocable unit is drawn on, by index into `tiles`.
    owner: HashMap<UnitId, usize>,
    pub bounds: ([f32; 2], [f32; 2]),
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
pub fn map(system: &System, config: &MapConfig) -> CodebaseMap {
    let (owner_unit, members) = groups(system);

    // Size each group, then flow them left to right, wrapping into bands.
    let step_x = config.tile_width + config.gap;
    let step_y = config.tile_height + config.gap;
    let padding = config.gap * 2.0;
    let group_gap = config.tile_width * 0.35;
    let sizes: Vec<(usize, usize, f32, f32)> = members
        .iter()
        .map(|(_, _, units)| {
            let columns = units.len().div_ceil(config.max_rows).max(1);
            let rows = units.len().div_ceil(columns).max(1);
            let width = columns as f32 * step_x - config.gap + padding * 2.0;
            let height = config.header + rows as f32 * step_y - config.gap + padding;
            (columns, rows, width, height)
        })
        .collect();
    let mut bands: Vec<Vec<usize>> = vec![Vec::new()];
    let mut used = 0.0;
    for (index, (_, _, width, _)) in sizes.iter().enumerate() {
        let band_full = bands.last().is_some_and(|b| !b.is_empty());
        if band_full && used + width > config.max_width {
            bands.push(Vec::new());
            used = 0.0;
        }
        if let Some(band) = bands.last_mut() {
            band.push(index);
        }
        used += width + group_gap;
    }
    let band_height = |band: &[usize]| {
        band.iter()
            .filter_map(|&i| sizes.get(i))
            .map(|s| s.3)
            .fold(0.0_f32, f32::max)
    };
    // The first band goes on top; the map's bottom edge rests on y = 0.
    let total: f32 = bands
        .iter()
        .map(|b| band_height(b) + group_gap)
        .sum::<f32>()
        - group_gap;

    let mut result = CodebaseMap::default();
    let mut top = total;
    let mut max_x = 0.0_f32;
    for band in &bands {
        let mut x = 0.0;
        for &index in band {
            let (Some((key, label, units)), Some(&(columns, rows, width, height))) =
                (members.get(index), sizes.get(index))
            else {
                continue;
            };
            let group = result.groups.len();
            result.groups.push(MapGroup {
                unit: key.clone(),
                label: label.clone(),
                min: [x, top - height],
                max: [x + width, top],
            });
            for (n, unit) in units.iter().enumerate() {
                let (column, row) = (n / rows, n % rows);
                debug_assert!(column < columns);
                let position = [
                    x + padding + column as f32 * step_x + config.tile_width / 2.0,
                    top - config.header - row as f32 * step_y - config.tile_height / 2.0,
                ];
                result.tiles.push(MapTile {
                    unit: unit.id.clone(),
                    group,
                    position,
                });
            }
            x += width + group_gap;
            max_x = max_x.max(x - group_gap);
        }
        top -= band_height(band) + group_gap;
    }

    let tile_index: HashMap<&UnitId, usize> = result
        .tiles
        .iter()
        .enumerate()
        .map(|(i, t)| (&t.unit, i))
        .collect();
    let owner: HashMap<UnitId, usize> = owner_unit
        .iter()
        .filter_map(|(unit, top)| Some((unit.clone(), *tile_index.get(top)?)))
        .collect();
    result.owner = owner;
    result.bounds = ([0.0, 0.0], [max_x, total.max(0.0)]);
    result
}

/// A container, its label and its tiles.
type Group<'s> = (Option<UnitId>, String, Vec<&'s Unit>);

/// Which tile each unit that can be invoked is drawn on, and the tiles grouped by
/// their nearest non-invocable container, ordered by how close they are to an entry.
fn groups(system: &System) -> (HashMap<UnitId, UnitId>, Vec<Group<'_>>) {
    // Tiles are what calls can reach, not nested in an invocable: functions, and what
    // runs or carries calls without being analyzed here (a script, a service, a queue).
    let is_invocable = |id: &UnitId| system.unit(id).is_some_and(|u| u.role == Role::Invocable);
    let invoked =
        |unit: &&Unit| matches!(unit.role, Role::Invocable | Role::External | Role::Channel);
    let mut owner_unit: HashMap<UnitId, UnitId> = HashMap::new();
    let mut tile_units: Vec<&Unit> = Vec::new();
    for unit in system.units().filter(invoked) {
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
        let map = map(&system, &MapConfig::default());

        let labels: Vec<&str> = map.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["api", "db"]);
        let units: Vec<&str> = map.tiles.iter().map(|t| t.unit.as_str()).collect();
        assert_eq!(
            units,
            ["x:api::handle", "x:db::load", "x:db::unused"],
            "closures share their function's tile; reached tiles come first"
        );
        assert_eq!(
            map.tile_of(&UnitId::new("x:api::handle::{closure#0}")),
            Some(0)
        );
        let (min, max) = map.bounds;
        assert_eq!(min, [0.0, 0.0]);
        assert!(
            map.tiles
                .iter()
                .all(|t| t.position[1] > 0.0 && t.position[1] < max[1])
        );
    }

    #[test]
    fn what_calls_reach_has_a_tile_though_it_is_not_analyzed() {
        let json = r#"{
          "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" },
          "units": [
            { "id": "ci:deploy", "name": "deploy", "kind": "pipeline", "role": "invocable", "calls": [
              { "key": "build#0", "text": "build", "targets": [{ "unit": "ci:deploy/build", "certainty": "declared" }] } ] },
            { "id": "ci:deploy/build", "name": "build", "kind": "step", "role": "invocable",
              "parent": "ci:deploy", "calls": [
              { "key": "build.sh#0", "text": "build.sh", "targets": [{ "unit": "script:build.sh", "certainty": "declared" }] } ] },
            { "id": "script:build.sh", "name": "build.sh", "kind": "script", "role": "external" },
            { "id": "q:events", "name": "events", "kind": "topic", "role": "channel" },
            { "id": "q:group", "name": "group", "kind": "module", "role": "group" }
          ]
        }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let map = map(&system, &MapConfig::default());
        let mut units: Vec<&str> = map.tiles.iter().map(|t| t.unit.as_str()).collect();
        units.sort_unstable();
        assert_eq!(
            units,
            ["ci:deploy", "q:events", "script:build.sh"],
            "the step is the pipeline's; the script and the topic have their own"
        );
        assert_eq!(
            map.tile_of(&UnitId::new("ci:deploy/build")),
            map.tile_of(&UnitId::new("ci:deploy"))
        );
    }
}
