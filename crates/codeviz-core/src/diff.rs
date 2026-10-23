//! What changed between two versions of a system: which units were added, removed or
//! rewritten, and which calls appeared or went away. [`union`] folds both versions
//! into one set of documents, so a single layout can place every unit either version
//! has.

use std::collections::{HashMap, HashSet};

use codeviz_ir::{CallKey, Certainty, Document, Invocation, Role, Unit, UnitId};

use crate::System;

/// How a unit changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Change {
    Added,
    Removed,
    /// In both versions, with different content.
    Modified,
    /// Renamed or moved: the same function under a new id, perhaps changed too.
    Moved,
}

/// A call from one unit to another.
pub type Call = (UnitId, UnitId);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// Every unit that changed; anything absent is the same in both versions.
    pub units: HashMap<UnitId, Change>,
    /// The id each moved unit had before, by its new id.
    pub moved: Moves,
    pub added_calls: HashSet<Call>,
    pub removed_calls: HashSet<Call>,
}

impl Diff {
    /// What changed from `before` to `after`, function by function: closures and other
    /// units nested in a function count as part of it, since their ids are positional
    /// (`{closure#2}`) and shift whenever an earlier one is added.
    #[must_use]
    pub fn between(before: &System, after: &System) -> Self {
        let (old_calls, new_calls) = (calls(before), calls(after));
        let mut units = HashMap::new();
        for unit in after.units().filter(|u| after.owner(&u.id) == &u.id) {
            match before.unit(&unit.id) {
                None => {
                    units.insert(unit.id.clone(), Change::Added);
                }
                Some(old) if rewritten(old, unit, &old_calls, &new_calls) => {
                    units.insert(unit.id.clone(), Change::Modified);
                }
                Some(_) => {}
            }
        }
        for unit in before.units().filter(|u| before.owner(&u.id) == &u.id) {
            if after.unit(&unit.id).is_none() {
                units.insert(unit.id.clone(), Change::Removed);
            }
        }
        let (old, new) = (pairs(&old_calls), pairs(&new_calls));
        Self {
            units,
            moved: Moves::new(),
            added_calls: new.difference(&old).cloned().collect(),
            removed_calls: old.difference(&new).cloned().collect(),
        }
    }

    /// Marks the units that moved, by new id with the id each had before. `before`
    /// should already be [`align`]ed, so they are otherwise compared as one unit.
    #[must_use]
    pub fn with_moves(mut self, moved: Moves) -> Self {
        for new in moved.keys() {
            self.units.insert(new.clone(), Change::Moved);
        }
        self.moved = moved;
        self
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

/// What each function calls, by owner on both ends.
pub(crate) type Calls = HashMap<UnitId, HashSet<UnitId>>;

/// Old ids by the new id of the unit that moved there.
pub type Moves = HashMap<UnitId, UnitId>;

/// Whether a unit that went away and one that appeared are the same, moved.
type Rule<'a> = dyn Fn(&Unit, &Unit) -> bool + 'a;

/// Finds the functions of `before` that were renamed or moved in `after`, and
/// rewrites `before` to call them by their new ids, so a diff of the result compares
/// each with itself. A function moved when the new one lists the old id among its
/// `aliases`; when the two have the same content hash; or when they share kind,
/// parent and everything they call, at least two functions resolved exactly. Only
/// one-to-one matches count.
#[must_use]
pub fn align(before: Vec<Document>, after: &System) -> (Vec<Document>, Moves) {
    let old = System::new(before.clone());
    let functions = |system: &System| -> Vec<UnitId> {
        system
            .units()
            .filter(|u| u.role == Role::Invocable && system.owner(&u.id) == &u.id)
            .map(|u| u.id.clone())
            .collect()
    };
    let mut gone: Vec<UnitId> = functions(&old)
        .into_iter()
        .filter(|id| after.unit(id).is_none())
        .collect();
    let mut new: Vec<UnitId> = functions(after)
        .into_iter()
        .filter(|id| old.unit(id).is_none())
        .collect();
    gone.sort();
    new.sort();
    let exact = |system: &System, id: &UnitId| -> Option<HashSet<UnitId>> {
        let unit = system.unit(id)?;
        let targets: HashSet<UnitId> = unit
            .calls
            .iter()
            .flat_map(|c| &c.targets)
            .filter(|t| t.certainty == Certainty::Exact)
            .map(|t| system.owner(&t.unit).clone())
            .filter(|t| t != id)
            .collect();
        (targets.len() >= 2).then_some(targets)
    };
    let mut moves = Moves::new();
    let rules: [&Rule; 3] = [
        &|was, now| now.aliases.contains(&was.id),
        &|was, now| was.content_hash.is_some() && was.content_hash == now.content_hash,
        &|was, now| {
            was.kind == now.kind
                && was.parent == now.parent
                && exact(&old, &was.id).is_some_and(|c| Some(c) == exact(after, &now.id))
        },
    ];
    for rule in rules {
        let matches = |a: &UnitId, b: &UnitId| {
            old.unit(a)
                .zip(after.unit(b))
                .is_some_and(|(was, now)| rule(was, now))
        };
        let mut found = Vec::new();
        for was in &gone {
            let mut candidates = new.iter().filter(|now| matches(was, now));
            if let (Some(now), None) = (candidates.next(), candidates.next())
                && gone.iter().filter(|other| matches(other, now)).count() == 1
            {
                found.push((was.clone(), now.clone()));
            }
        }
        for (was, now) in found {
            gone.retain(|id| *id != was);
            new.retain(|id| *id != now);
            moves.insert(now, was);
        }
    }
    let renames: HashMap<&UnitId, &UnitId> = moves.iter().map(|(now, was)| (was, now)).collect();
    let before = before
        .into_iter()
        .map(|document| rename(document, &renames))
        .collect();
    (before, moves)
}

/// `document` with every id that `renames` moves, or that sits inside one (`f` and
/// `f::{closure#0}`), rewritten.
fn rename(mut document: Document, renames: &HashMap<&UnitId, &UnitId>) -> Document {
    if renames.is_empty() {
        return document;
    }
    let fix = |id: &mut UnitId| {
        let text = id.as_str();
        let moved = renames.iter().find_map(|(was, now)| {
            let rest = text.strip_prefix(was.as_str())?;
            let inside =
                rest.is_empty() || rest.starts_with(|c: char| !(c.is_alphanumeric() || c == '_'));
            inside.then(|| format!("{now}{rest}"))
        });
        if let Some(moved) = moved {
            *id = UnitId::new(moved);
        }
    };
    for unit in &mut document.units {
        fix(&mut unit.id);
        if let Some(parent) = &mut unit.parent {
            fix(parent);
        }
        for call in &mut unit.calls {
            call.targets.iter_mut().for_each(|t| fix(&mut t.unit));
            if let Some(via) = &mut call.via {
                fix(via);
            }
        }
    }
    for entry in &mut document.entries {
        fix(&mut entry.unit);
        entry.chain.iter_mut().for_each(&fix);
    }
    for link in &mut document.links {
        fix(&mut link.from);
        fix(&mut link.to);
    }
    document
}

/// Different content: by hash when both versions have one, else by what they call.
fn rewritten(old: &Unit, new: &Unit, old_calls: &Calls, new_calls: &Calls) -> bool {
    match (&old.content_hash, &new.content_hash) {
        (Some(a), Some(b)) => a != b,
        _ => old_calls.get(&old.id) != new_calls.get(&new.id),
    }
}

/// What each function calls, its closures' calls included, by owner on both ends.
pub(crate) fn calls(system: &System) -> Calls {
    let mut calls = Calls::new();
    for unit in system.units() {
        let from = system.owner(&unit.id);
        for target in unit.calls.iter().flat_map(|call| &call.targets) {
            let to = system.owner(&target.unit);
            if to != from {
                calls.entry(from.clone()).or_default().insert(to.clone());
            }
        }
    }
    calls
}

fn pairs(calls: &Calls) -> HashSet<Call> {
    calls
        .iter()
        .flat_map(|(from, targets)| targets.iter().map(|to| (from.clone(), to.clone())))
        .collect()
}

/// `after`, plus every function only `before` has and every call only `before` makes:
/// one system holding both versions, for laying them out together; journeys should
/// still follow `after` alone. A removed call keeps its place among the caller's
/// calls when the caller made it directly, and is keyed `…~removed`.
#[must_use]
pub fn union(before: &[Document], mut after: Vec<Document>) -> Vec<Document> {
    let old = System::new(before.to_vec());
    let new = System::new(after.clone());
    let (old_calls, new_calls) = (calls(&old), calls(&new));
    for document in &mut after {
        for unit in &mut document.units {
            let Some(previous) = old_calls.get(&unit.id) else {
                continue;
            };
            let current = new_calls.get(&unit.id);
            let mut gone: Vec<&UnitId> = previous
                .iter()
                .filter(|t| !current.is_some_and(|c| c.contains(*t)))
                .collect();
            gone.sort();
            for target in gone {
                if let Some((index, call)) = removed_call(&old, &unit.id, target) {
                    unit.calls.insert(index.min(unit.calls.len()), call);
                }
            }
        }
    }
    // Whole functions that went away; closures of a function still there went with
    // its rewrite.
    let removed: Vec<Unit> = before
        .iter()
        .flat_map(|d| &d.units)
        .filter(|u| new.unit(&u.id).is_none() && new.unit(old.owner(&u.id)).is_none())
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

/// The call `from` made to `to` in `old`, aimed at `to` itself, and where it sat among
/// `from`'s own calls (past the end when one of its closures made it).
fn removed_call(old: &System, from: &UnitId, to: &UnitId) -> Option<(usize, Invocation)> {
    let reaches = |call: &Invocation| call.targets.iter().any(|t| old.owner(&t.unit) == to);
    let direct = old
        .unit(from)
        .and_then(|u| u.calls.iter().position(reaches).map(|i| (i, &u.calls[i])));
    let (index, call) = direct.or_else(|| {
        old.units()
            .filter(|u| u.id != *from && old.owner(&u.id) == from)
            .find_map(|u| u.calls.iter().find(|c| reaches(c)))
            .map(|call| (usize::MAX, call))
    })?;
    let mut call = call.clone();
    call.targets.retain(|t| old.owner(&t.unit) == to);
    for target in &mut call.targets {
        target.unit = to.clone();
    }
    call.targets.truncate(1);
    call.key = CallKey::new(format!("{}~removed", call.key));
    Some((index, call))
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

    /// One function with closures: `(closure index, what it calls)`.
    fn closures(hash: &str, closures: &[(usize, &str)]) -> System {
        let unit = |id: &str, parent: &str, calls: &str, hash: &str| {
            let calls = if calls.is_empty() {
                String::new()
            } else {
                format!(
                    r#"{{ "key": "{calls}#0", "text": "{calls}", "targets": [{{ "unit": "x:{calls}", "certainty": "exact" }}] }}"#
                )
            };
            let parent = if parent.is_empty() {
                String::new()
            } else {
                format!(r#""parent": "{parent}","#)
            };
            format!(
                r#"{{ "id": "{id}", "name": "{id}", "kind": "fn", "role": "invocable", {parent} "content_hash": {{ "algo": "t", "value": "{hash}" }}, "calls": [{calls}] }}"#
            )
        };
        let mut units = vec![
            unit("x:main", "", "", hash),
            unit("x:a", "", "", "1"),
            unit("x:b", "", "", "1"),
        ];
        for (index, calls) in closures {
            units.push(unit(
                &format!("x:main::{{closure#{index}}}"),
                "x:main",
                calls,
                "c",
            ));
        }
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
            units.join(",")
        );
        System::new(vec![Document::from_json(&json).unwrap()])
    }

    #[test]
    fn closures_count_as_their_function() {
        // A closure inserted ahead of another renumbers it.
        let before = closures("1", &[(0, "a")]);
        let after = closures("2", &[(0, "b"), (1, "a")]);
        let diff = Diff::between(&before, &after);
        let id = |name: &str| UnitId::new(format!("x:{name}"));
        assert_eq!(
            diff.units,
            HashMap::from([(id("main"), Change::Modified)]),
            "closures are not changes of their own"
        );
        assert_eq!(diff.added_calls, HashSet::from([(id("main"), id("b"))]));
        assert!(diff.removed_calls.is_empty(), "main still calls a");
    }

    fn documents(units: &[&str]) -> Vec<Document> {
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
            units.join(",")
        );
        vec![Document::from_json(&json).unwrap()]
    }

    fn function(id: &str, hash: &str, calls: &[&str]) -> String {
        let calls: Vec<String> = calls
            .iter()
            .map(|c| {
                format!(
                    r#"{{ "key": "{c}#0", "text": "{c}", "targets": [{{ "unit": "x:{c}", "certainty": "exact" }}] }}"#
                )
            })
            .collect();
        format!(
            r#"{{ "id": "x:{id}", "name": "{id}", "kind": "fn", "role": "invocable", "parent": "x:m", "content_hash": {{ "algo": "t", "value": "{hash}" }}, "calls": [{}] }}"#,
            calls.join(",")
        )
    }

    #[test]
    fn moved_functions_are_matched_and_take_their_new_ids() {
        let module = r#"{ "id": "x:m", "name": "m", "kind": "module", "role": "group" }"#;
        let closure = r#"{ "id": "x:old::{closure#0}", "name": "{closure#0}", "kind": "closure", "role": "invocable", "parent": "x:old", "calls": [] }"#;
        let before = documents(&[
            module,
            &function("main", "1", &["old", "renamed_from"]),
            // Moved unchanged: the same hash.
            &function("old", "h", &[]),
            closure,
            // Renamed and rewritten: the same calls.
            &function("renamed_from", "2", &["a", "b"]),
            &function("a", "a", &[]),
            &function("b", "b", &[]),
            // Gone for good.
            &function("dropped", "3", &["a"]),
        ]);
        let after = System::new(documents(&[
            module,
            &function("main", "1x", &["new", "renamed_to"]),
            &function("new", "h", &[]),
            &function("renamed_to", "2x", &["a", "b"]),
            &function("a", "a", &[]),
            &function("b", "b", &[]),
        ]));
        let id = |name: &str| UnitId::new(format!("x:{name}"));
        let (aligned, moved) = align(before, &after);
        assert_eq!(
            moved,
            Moves::from([
                (id("new"), id("old")),
                (id("renamed_to"), id("renamed_from"))
            ])
        );
        let aligned = System::new(aligned);
        assert!(
            aligned.unit(&id("new::{closure#0}")).is_some(),
            "what was inside moves with it"
        );
        let diff = Diff::between(&aligned, &after).with_moves(moved);
        assert_eq!(diff.change(&id("new")), Some(Change::Moved));
        assert_eq!(diff.change(&id("renamed_to")), Some(Change::Moved));
        assert_eq!(diff.change(&id("dropped")), Some(Change::Removed));
        assert!(
            diff.added_calls.is_empty(),
            "calls to a moved function are the same calls: {:?}",
            diff.added_calls
        );
    }
}
