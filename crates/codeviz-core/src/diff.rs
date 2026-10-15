//! What changed between two versions of a system: which units were added, removed or
//! rewritten, and which calls appeared or went away. [`union`] folds both versions
//! into one set of documents, so a single layout can place every unit either version
//! has.

use std::collections::{HashMap, HashSet};

use codeviz_ir::{CallKey, Document, Unit, UnitId};

use crate::System;

/// How a unit changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Change {
    Added,
    Removed,
    /// In both versions, with different content.
    Modified,
}

/// A call from one unit to another.
pub type Call = (UnitId, UnitId);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// Every unit that changed; anything absent is the same in both versions.
    pub units: HashMap<UnitId, Change>,
    pub added_calls: HashSet<Call>,
    pub removed_calls: HashSet<Call>,
}

impl Diff {
    #[must_use]
    pub fn between(before: &System, after: &System) -> Self {
        let mut units = HashMap::new();
        for unit in after.units() {
            match before.unit(&unit.id) {
                None => {
                    units.insert(unit.id.clone(), Change::Added);
                }
                Some(old) if rewritten(old, unit) => {
                    units.insert(unit.id.clone(), Change::Modified);
                }
                Some(_) => {}
            }
        }
        for unit in before.units() {
            if after.unit(&unit.id).is_none() {
                units.insert(unit.id.clone(), Change::Removed);
            }
        }
        let (old, new) = (calls(before.units()), calls(after.units()));
        Self {
            units,
            added_calls: new.difference(&old).cloned().collect(),
            removed_calls: old.difference(&new).cloned().collect(),
        }
    }

    #[must_use]
    pub fn change(&self, unit: &UnitId) -> Option<Change> {
        self.units.get(unit).copied()
    }

    /// Units changed in the given way.
    pub fn changed(&self, change: Change) -> impl Iterator<Item = &UnitId> {
        self.units
            .iter()
            .filter(move |(_, c)| **c == change)
            .map(|(unit, _)| unit)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.units.is_empty() && self.added_calls.is_empty() && self.removed_calls.is_empty()
    }
}

/// Different content: by hash when both versions have one, else by what they call.
fn rewritten(old: &Unit, new: &Unit) -> bool {
    match (&old.content_hash, &new.content_hash) {
        (Some(a), Some(b)) => a != b,
        _ => calls([old]) != calls([new]),
    }
}

fn calls<'a>(units: impl IntoIterator<Item = &'a Unit>) -> HashSet<Call> {
    units
        .into_iter()
        .flat_map(|unit| {
            unit.calls
                .iter()
                .flat_map(|call| &call.targets)
                .map(|target| (unit.id.clone(), target.unit.clone()))
        })
        .collect()
}

/// `after`, plus every unit only `before` has and every call only `before` makes: one
/// system holding both versions, for laying them out together; journeys should
/// still follow `after` alone. Removed calls keep their place among the caller's
/// calls.
#[must_use]
pub fn union(before: &[Document], mut after: Vec<Document>) -> Vec<Document> {
    let old: HashMap<&UnitId, &Unit> = before
        .iter()
        .flat_map(|d| &d.units)
        .map(|u| (&u.id, u))
        .collect();
    let mut present = HashSet::new();
    for document in &mut after {
        for unit in &mut document.units {
            present.insert(unit.id.clone());
            let Some(previous) = old.get(&unit.id) else {
                continue;
            };
            let current = calls([&*unit]);
            for (index, call) in previous.calls.iter().enumerate() {
                let gone = call
                    .targets
                    .iter()
                    .any(|t| !current.contains(&(unit.id.clone(), t.unit.clone())));
                if gone {
                    let mut call = call.clone();
                    call.targets
                        .retain(|t| !current.contains(&(unit.id.clone(), t.unit.clone())));
                    call.key = CallKey::new(format!("{}~removed", call.key));
                    unit.calls.insert(index.min(unit.calls.len()), call);
                }
            }
        }
    }
    let removed: Vec<Unit> = before
        .iter()
        .flat_map(|d| &d.units)
        .filter(|u| !present.contains(&u.id))
        .cloned()
        .collect();
    if let (false, Some(template)) = (removed.is_empty(), before.first()) {
        let mut document = template.clone();
        document.units = removed;
        document.entries.clear();
        document.links.clear();
        after.push(document);
    }
    after
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system(units: &[(&str, &[&str], &str)]) -> Vec<Document> {
        let units: Vec<String> = units
            .iter()
            .map(|(name, calls, hash)| {
                let calls: Vec<String> = calls
                    .iter()
                    .map(|c| {
                        format!(
                            r#"{{ "key": "{c}#0", "text": "{c}", "targets": [{{ "unit": "x:{c}", "certainty": "exact" }}] }}"#
                        )
                    })
                    .collect();
                format!(
                    r#"{{ "id": "x:{name}", "name": "{name}", "kind": "fn", "role": "invocable", "content_hash": {{ "algo": "t", "value": "{hash}" }}, "calls": [{}] }}"#,
                    calls.join(",")
                )
            })
            .collect();
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
            units.join(",")
        );
        vec![Document::from_json(&json).unwrap()]
    }

    #[test]
    fn finds_added_removed_and_rewritten_units_and_calls() {
        let before = system(&[("main", &["a", "b"], "1"), ("a", &[], "1"), ("b", &[], "1")]);
        let after = system(&[("main", &["a", "c"], "2"), ("a", &[], "1"), ("c", &[], "1")]);
        let diff = Diff::between(&System::new(before.clone()), &System::new(after.clone()));
        let id = |name: &str| UnitId::new(format!("x:{name}"));
        assert_eq!(diff.change(&id("main")), Some(Change::Modified));
        assert_eq!(diff.change(&id("a")), None);
        assert_eq!(diff.change(&id("b")), Some(Change::Removed));
        assert_eq!(diff.change(&id("c")), Some(Change::Added));
        assert!(diff.removed_calls.contains(&(id("main"), id("b"))));
        assert!(diff.added_calls.contains(&(id("main"), id("c"))));

        let both = System::new(union(&before, after));
        assert!(both.unit(&id("b")).is_some(), "the removed unit is kept");
        let main = both.unit(&id("main")).unwrap();
        let targets: Vec<&str> = main
            .calls
            .iter()
            .flat_map(|c| &c.targets)
            .map(|t| t.unit.as_str())
            .collect();
        assert_eq!(
            targets,
            ["x:a", "x:b", "x:c"],
            "removed calls keep their place"
        );
    }
}
