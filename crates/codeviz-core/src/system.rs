use std::{collections::HashMap, path::PathBuf};

use codeviz_ir::{Certainty, Document, Entry, LinkKind, Role, Unit, UnitId};

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

    /// Invocable units whose source in `file` spans `line`, innermost first. `file`
    /// matches a location when either path ends with the other, whole components at
    /// a time, so `src/foo.rs`, `crates/a/src/foo.rs` and an absolute path all find a
    /// unit in `crates/a/src/foo.rs`.
    #[must_use]
    pub fn containing(&self, file: &str, line: u32) -> Vec<&Unit> {
        let mut found: Vec<&Unit> = self
            .units
            .iter()
            .filter(|unit| unit.role == Role::Invocable)
            .filter(|unit| {
                let Some(location) = &unit.location else {
                    return false;
                };
                let path = self.local_path(&unit.id, &location.uri);
                let path = path
                    .as_deref()
                    .and_then(|p| p.to_str())
                    .unwrap_or(&location.uri);
                location
                    .range
                    .is_some_and(|r| (r.start_line..=r.end_line).contains(&line))
                    && (same_file(path, file) || same_file(&location.uri, file))
            })
            .collect();
        found.sort_by_key(|unit| {
            unit.location
                .as_ref()
                .and_then(|l| l.range)
                .map_or(u32::MAX, |r| r.end_line - r.start_line)
        });
        found
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

    /// The function `id` is part of: the outermost invocable in its chain of invocable
    /// parents, so a closure's owner is the function that defines it. Anything else is
    /// its own owner.
    #[must_use]
    pub fn owner<'a>(&'a self, id: &'a UnitId) -> &'a UnitId {
        let invocable = |id: &UnitId| self.unit(id).is_some_and(|u| u.role == Role::Invocable);
        let mut top = id;
        if !invocable(top) {
            return top;
        }
        while let Some(parent) = self
            .unit(top)
            .and_then(|u| u.parent.as_ref())
            .filter(|&p| p != top && invocable(p))
        {
            top = parent;
        }
        top
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

/// Whether two paths name the same file: one ends with the other, on a `/`.
fn same_file(a: &str, b: &str) -> bool {
    let clean = |path: &str| {
        let path = path.strip_prefix("file://").unwrap_or(path);
        path.strip_prefix("./").unwrap_or(path).to_owned()
    };
    let (a, b) = (clean(a), clean(b));
    let (long, short) = if a.len() >= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    !short.is_empty()
        && long.ends_with(short.as_str())
        && (long.len() == short.len() || long[..long.len() - short.len()].ends_with('/'))
}

#[cfg(test)]
mod tests {
    use codeviz_ir::Document;

    use super::*;

    /// `place` with a closure inside it, and another file's `main`.
    fn system() -> System {
        let json = r#"{
          "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" },
          "root": "file:///work/shop",
          "units": [
            { "id": "x:place", "name": "place", "kind": "fn", "role": "invocable",
              "location": { "uri": "src/orders.rs",
                "range": { "start_line": 10, "start_col": 1, "end_line": 30, "end_col": 2 } } },
            { "id": "x:place::{closure#0}", "name": "{closure#0}", "kind": "closure",
              "role": "invocable", "parent": "x:place",
              "location": { "uri": "src/orders.rs",
                "range": { "start_line": 14, "start_col": 9, "end_line": 16, "end_col": 10 } } },
            { "id": "x:main", "name": "main", "kind": "fn", "role": "invocable",
              "location": { "uri": "src/main.rs",
                "range": { "start_line": 1, "start_col": 1, "end_line": 40, "end_col": 2 } } }
          ]
        }"#;
        System::new([Document::from_json(json).unwrap_or_else(|e| panic!("{e}"))])
    }

    #[test]
    fn finds_what_a_line_is_in_innermost_first() {
        let system = system();
        let ids = |file, line| -> Vec<&str> {
            system
                .containing(file, line)
                .iter()
                .map(|u| u.id.as_str())
                .collect()
        };
        assert_eq!(
            ids("src/orders.rs", 15),
            ["x:place::{closure#0}", "x:place"]
        );
        assert_eq!(ids("src/orders.rs", 20), ["x:place"]);
        assert_eq!(ids("orders.rs", 20), ["x:place"], "a shorter path");
        assert_eq!(ids("/work/shop/src/orders.rs", 20), ["x:place"], "absolute");
        assert_eq!(ids("./src/orders.rs", 20), ["x:place"]);
        assert!(ids("src/orders.rs", 31).is_empty(), "past its end");
        assert!(ids("rders.rs", 20).is_empty(), "only whole components");
    }
}
