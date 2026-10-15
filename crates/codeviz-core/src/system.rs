use std::collections::HashMap;

use codeviz_ir::{Document, Entry, Unit, UnitId};

/// One or more documents merged by unit id. Later documents override earlier ones.
#[derive(Debug, Clone, Default)]
pub struct System {
    units: Vec<Unit>,
    by_id: HashMap<UnitId, usize>,
    entries: Vec<Entry>,
    callers: HashMap<UnitId, Vec<UnitId>>,
    children: HashMap<UnitId, Vec<UnitId>>,
}

impl System {
    #[must_use]
    pub fn new(documents: impl IntoIterator<Item = Document>) -> Self {
        let mut system = Self::default();
        for document in documents {
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
        }
        system.index();
        system
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

    /// Ancestors of `id`, nearest first.
    pub fn ancestors<'a>(&'a self, id: &UnitId) -> impl Iterator<Item = &'a Unit> + 'a {
        let start = self.unit(id).and_then(|u| u.parent.as_ref());
        std::iter::successors(start.and_then(|p| self.unit(p)), |unit| {
            unit.parent.as_ref().and_then(|p| self.unit(p))
        })
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
