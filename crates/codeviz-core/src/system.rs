use std::{collections::HashMap, path::PathBuf};

use codeviz_ir::{Certainty, Document, Entry, LinkKind, Unit, UnitId};

/// One or more documents merged by unit id. Later documents override earlier ones.
///
/// Links from every document are applied on load: `refines` links become zoom levels
/// and `same_as` links continue containment across sources.
#[derive(Debug, Clone, Default)]
pub struct System {
    units: Vec<Unit>,
    by_id: HashMap<UnitId, usize>,
    entries: Vec<Entry>,
    callers: HashMap<UnitId, Vec<UnitId>>,
    children: HashMap<UnitId, Vec<UnitId>>,
    refinements: HashMap<UnitId, Vec<(UnitId, Certainty)>>,
    equivalents: HashMap<UnitId, Vec<UnitId>>,
    /// Each unit's document root, for resolving relative locations.
    roots: HashMap<UnitId, String>,
}

impl System {
    #[must_use]
    pub fn new(documents: impl IntoIterator<Item = Document>) -> Self {
        let mut system = Self::default();
        for document in documents {
            for unit in &document.units {
                if let Some(root) = &document.root {
                    system.roots.insert(unit.id.clone(), root.clone());
                }
            }
            for unit in document.units {
                if let Some(slot) = system
                    .by_id
                    .get(&unit.id)
                    .and_then(|&i| system.units.get_mut(i))
                {
                    *slot = unit;
                } else {
                    system.by_id.insert(unit.id.clone(), system.units.len());
                    system.units.push(unit);
                }
            }
            system.entries.extend(document.entries);
            for link in document.links {
                match link.kind {
                    LinkKind::Refines => system.add_refinement(link.from, link.to, link.certainty),
                    LinkKind::SameAs => {
                        system
                            .equivalents
                            .entry(link.from.clone())
                            .or_default()
                            .push(link.to.clone());
                        system
                            .equivalents
                            .entry(link.to)
                            .or_default()
                            .push(link.from);
                    }
                    LinkKind::Unknown => {}
                }
            }
        }
        let inline: Vec<(UnitId, UnitId)> = system
            .units
            .iter()
            .flat_map(|u| u.refines.iter().map(|to| (u.id.clone(), to.clone())))
            .collect();
        for (from, to) in inline {
            system.add_refinement(from, to, Certainty::Declared);
        }
        system.index();
        system
    }

    fn add_refinement(&mut self, from: UnitId, to: UnitId, certainty: Certainty) {
        let refined = self.refinements.entry(from).or_default();
        if !refined.iter().any(|(id, _)| id == &to) {
            refined.push((to, certainty));
        }
    }

    fn index(&mut self) {
        for unit in &self.units {
            if let Some(parent) = &unit.parent {
                self.children
                    .entry(parent.clone())
                    .or_default()
                    .push(unit.id.clone());
            }
            for call in &unit.calls {
                for target in &call.targets {
                    let callers = self.callers.entry(target.unit.clone()).or_default();
                    if !callers.contains(&unit.id) {
                        callers.push(unit.id.clone());
                    }
                }
            }
        }
    }

    #[must_use]
    pub fn unit(&self, id: &UnitId) -> Option<&Unit> {
        self.by_id.get(id).and_then(|&index| self.units.get(index))
    }

    pub fn units(&self) -> impl ExactSizeIterator<Item = &Unit> {
        self.units.iter()
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Units with an invocation that may reach `id`.
    #[must_use]
    pub fn callers(&self, id: &UnitId) -> &[UnitId] {
        self.callers.get(id).map_or(&[], Vec::as_slice)
    }

    /// Units whose `parent` is `id`.
    #[must_use]
    pub fn children(&self, id: &UnitId) -> &[UnitId] {
        self.children.get(id).map_or(&[], Vec::as_slice)
    }

    /// The local file a location in `id`'s document points at, when it is a file.
    #[must_use]
    pub fn local_path(&self, id: &UnitId, uri: &str) -> Option<PathBuf> {
        if let Some(path) = uri.strip_prefix("file://") {
            return Some(PathBuf::from(path));
        }
        if uri.contains("://") {
            return None;
        }
        let root = self.roots.get(id)?.strip_prefix("file://")?;
        Some(PathBuf::from(root).join(uri))
    }

    /// Finer units `id` summarizes, in order, from `refines` links.
    #[must_use]
    pub fn refinements(&self, id: &UnitId) -> &[(UnitId, Certainty)] {
        self.refinements.get(id).map_or(&[], Vec::as_slice)
    }

    /// Units declared `same_as` `id`.
    #[must_use]
    pub fn equivalents(&self, id: &UnitId) -> &[UnitId] {
        self.equivalents.get(id).map_or(&[], Vec::as_slice)
    }

    /// Ancestors of `id`, nearest first. Where a unit has no parent but is `same_as` a
    /// loaded unit, the chain continues through that unit and its parents.
    #[must_use]
    pub fn ancestors(&self, id: &UnitId) -> Vec<&Unit> {
        let mut chain = Vec::new();
        let mut seen = vec![id];
        let mut current = id;
        loop {
            let next = self
                .unit(current)
                .and_then(|u| u.parent.as_ref())
                .or_else(|| {
                    self.equivalents(current)
                        .iter()
                        .find(|e| self.unit(e).is_some() && !seen.contains(e))
                });
            let Some(next_id) = next.filter(|n| !seen.contains(n)) else {
                break;
            };
            let Some(unit) = self.unit(next_id) else {
                break;
            };
            seen.push(next_id);
            chain.push(unit);
            current = next_id;
        }
        chain
    }

    /// Finds units whose id ends with `query`, or whose entry label equals it.
    #[must_use]
    pub fn find(&self, query: &str) -> Vec<&Unit> {
        if let Some(entry) = self.entries.iter().find(|e| e.label == query)
            && let Some(unit) = self.unit(&entry.unit)
        {
            return vec![unit];
        }
        self.units
            .iter()
            .filter(|unit| {
                let id = unit.id.as_str();
                id == query
                    || id.ends_with(query)
                        && id[..id.len() - query.len()].ends_with([':', '.', '/'])
            })
            .collect()
    }
}
