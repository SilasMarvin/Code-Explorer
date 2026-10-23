//! A diff laid out for review: the functions it changed in the order to read them,
//! each after the changes it calls, and the entry points each can affect.

use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet, VecDeque},
};

use codeviz_ir::{Role, UnitId};

use crate::{
    System,
    diff::{Calls, Diff, calls},
};

/// Where a system starts: an entry point it declares, or a function nothing calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub unit: UnitId,
    pub label: String,
}

#[derive(Debug, Clone, Default)]
pub struct Review {
    /// The changed functions, each after every change it calls, so a reviewer meets
    /// what a function now relies on before the function itself. Among changes free
    /// to go next, a caller of the last one comes first, then one beside it, then the
    /// one more entry points reach.
    pub order: Vec<UnitId>,
    /// The declared entry points, or the functions nothing calls when there are none.
    pub starts: Vec<Start>,
    /// Whether `starts` are the functions nothing calls, for want of declared entries.
    pub roots: bool,
    /// The starts that can reach each change, by index into `starts`.
    reached_from: HashMap<UnitId, Vec<usize>>,
}

impl Review {
    /// Reviews `diff` from `before` (with moved functions under their new ids, as
    /// [`align`](crate::align) leaves it) to `after`. Calls either version makes
    /// count: a removed function is read before the caller that stopped calling it.
    #[must_use]
    pub fn new(diff: &Diff, before: &System, after: &System) -> Self {
        let function = |id: &UnitId| {
            after
                .unit(id)
                .or_else(|| before.unit(id))
                .is_some_and(|u| u.role == Role::Invocable)
        };
        let mut changes: Vec<UnitId> = diff
            .units
            .keys()
            .filter(|id| function(id))
            .cloned()
            .collect();
        changes.sort();
        let outgoing = both(calls(before), calls(after));
        let mut incoming: Calls = HashMap::new();
        for (from, targets) in &outgoing {
            for to in targets {
                incoming.entry(to.clone()).or_default().insert(from.clone());
            }
        }
        let (starts, roots) = starts(after, &incoming);
        let index: HashMap<&UnitId, usize> = starts
            .iter()
            .enumerate()
            .map(|(i, s)| (&s.unit, i))
            .collect();
        let reached_from: HashMap<UnitId, Vec<usize>> = changes
            .iter()
            .map(|change| {
                let mut from: Vec<usize> = upstream(change, &incoming)
                    .iter()
                    .filter_map(|unit| index.get(unit).copied())
                    .collect();
                from.sort_unstable();
                (change.clone(), from)
            })
            .collect();
        let reach = |id: &UnitId| reached_from.get(id).map_or(0, Vec::len);
        changes.sort_by_key(|id| Reverse(reach(id)));
        let container = |id: &UnitId| {
            after
                .unit(id)
                .or_else(|| before.unit(id))
                .and_then(|u| u.parent.clone())
        };
        let order = reading_order(&changes, &outgoing, container);
        Self {
            order,
            starts,
            roots,
            reached_from,
        }
    }

    /// The starts that can reach a change, by index into [`Self::starts`].
    #[must_use]
    pub fn reached_from(&self, change: &UnitId) -> &[usize] {
        self.reached_from.get(change).map_or(&[], Vec::as_slice)
    }

    /// The starts any change reaches, by index, those reaching most changes first.
    #[must_use]
    pub fn affected(&self) -> Vec<usize> {
        let mut counts: HashMap<usize, usize> = HashMap::new();
        for start in self.reached_from.values().flatten() {
            *counts.entry(*start).or_default() += 1;
        }
        let mut affected: Vec<usize> = counts.keys().copied().collect();
        affected.sort_by_key(|start| (Reverse(counts[start]), *start));
        affected
    }
}

/// What each function calls in either version.
fn both(mut old: Calls, new: Calls) -> Calls {
    for (from, targets) in new {
        old.entry(from).or_default().extend(targets);
    }
    old
}

/// The declared entries, by the function they run; or else every function the new
/// version has that nothing calls. Says which.
fn starts(after: &System, callers: &Calls) -> (Vec<Start>, bool) {
    let mut declared: Vec<Start> = Vec::new();
    for entry in after.entries() {
        let unit = after.owner(&entry.unit).clone();
        if !declared.iter().any(|s| s.unit == unit) {
            declared.push(Start {
                unit,
                label: entry.label.clone(),
            });
        }
    }
    if !declared.is_empty() {
        return (declared, false);
    }
    let mut roots: Vec<Start> = after
        .units()
        .filter(|u| u.role == Role::Invocable && after.owner(&u.id) == &u.id)
        .filter(|u| {
            !callers
                .get(&u.id)
                .is_some_and(|c| c.iter().any(|c| c != &u.id))
        })
        .map(|u| Start {
            unit: u.id.clone(),
            label: crate::display_name(after, &u.id),
        })
        .collect();
    roots.sort_by(|a, b| a.label.cmp(&b.label));
    (roots, true)
}

/// `unit` and everything that can reach it.
fn upstream(unit: &UnitId, callers: &Calls) -> HashSet<UnitId> {
    let mut seen = HashSet::from([unit.clone()]);
    let mut queue = VecDeque::from([unit]);
    while let Some(at) = queue.pop_front() {
        for caller in callers.get(at).into_iter().flatten() {
            if seen.insert(caller.clone()) {
                queue.push_back(caller);
            }
        }
    }
    seen
}

/// The changes `change` calls with no other change between: through any number of
/// unchanged functions, but not through another change.
fn next_changes<'a>(
    change: &UnitId,
    changed: &HashSet<&'a UnitId>,
    callees: &Calls,
) -> HashSet<&'a UnitId> {
    let mut found = HashSet::new();
    let mut seen = HashSet::from([change]);
    let mut queue = VecDeque::from([change]);
    while let Some(at) = queue.pop_front() {
        for callee in callees.get(at).into_iter().flatten() {
            if !seen.insert(callee) {
                continue;
            }
            match changed.get(callee) {
                Some(&other) => {
                    found.insert(other);
                }
                None => queue.push_back(callee),
            }
        }
    }
    found.remove(change);
    found
}

/// `changes`, most important first, put in an order where each comes after the
/// changes it calls. Of the changes free to go next, a caller of the one just read
/// goes first, so a chain reads bottom up as one; then one beside it in the same
/// container; then the most important. A cycle is broken where the fewest of its
/// calls are still unread.
fn reading_order(
    changes: &[UnitId],
    callees: &Calls,
    container: impl Fn(&UnitId) -> Option<UnitId>,
) -> Vec<UnitId> {
    let rank: HashMap<&UnitId, usize> = changes.iter().enumerate().map(|(i, c)| (c, i)).collect();
    let members: HashSet<&UnitId> = changes.iter().collect();
    let next: HashMap<&UnitId, HashSet<&UnitId>> = changes
        .iter()
        .map(|change| (change, next_changes(change, &members, callees)))
        .collect();
    let mut called_by: HashMap<&UnitId, Vec<&UnitId>> = HashMap::new();
    for (&caller, called) in &next {
        for &callee in called {
            called_by.entry(callee).or_default().push(caller);
        }
    }
    let containers: HashMap<&UnitId, Option<UnitId>> =
        changes.iter().map(|c| (c, container(c))).collect();
    let mut unread: HashMap<&UnitId, usize> = next.iter().map(|(c, n)| (*c, n.len())).collect();
    let mut order: Vec<UnitId> = Vec::with_capacity(changes.len());
    let mut last: Option<&UnitId> = None;
    while !unread.is_empty() {
        let fewest = unread.values().copied().min().unwrap_or(0);
        let follows = |change: &UnitId| last.is_some_and(|last| next[change].contains(last));
        let beside = |change: &UnitId| {
            last.is_some_and(|last| {
                containers[change].is_some() && containers[change] == containers[last]
            })
        };
        let Some(pick) = unread
            .iter()
            .filter(|&(_, &left)| left == fewest)
            .map(|(&change, _)| change)
            .min_by_key(|&change| (!follows(change), !beside(change), rank[change]))
        else {
            break;
        };
        unread.remove(pick);
        for caller in called_by.get(pick).into_iter().flatten() {
            if let Some(left) = unread.get_mut(caller) {
                *left = left.saturating_sub(1);
            }
        }
        order.push(pick.clone());
        last = Some(pick);
    }
    order
}

#[cfg(test)]
mod tests {
    use codeviz_ir::Document;

    use super::*;

    /// Functions as `(name, calls, content hash)`, with entries as `(name, label)`.
    fn system(units: &[(&str, &[&str], &str)], entries: &[(&str, &str)]) -> System {
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
        let entries: Vec<String> = entries
            .iter()
            .map(|(name, label)| {
                format!(r#"{{ "unit": "x:{name}", "label": "{label}", "kind": "http" }}"#)
            })
            .collect();
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}], "entries": [{}] }}"#,
            units.join(","),
            entries.join(",")
        );
        System::new(vec![Document::from_json(&json).unwrap()])
    }

    fn review(before: &System, after: &System) -> Review {
        Review::new(&Diff::between(before, after), before, after)
    }

    fn names(review: &Review) -> Vec<&str> {
        review
            .order
            .iter()
            .map(|id| id.as_str().trim_start_matches("x:"))
            .collect()
    }

    #[test]
    fn callees_come_before_their_callers() {
        let before = system(
            &[("main", &["a"], "1"), ("a", &["b"], "1"), ("b", &[], "1")],
            &[],
        );
        let after = system(
            &[("main", &["a"], "2"), ("a", &["b"], "2"), ("b", &[], "2")],
            &[],
        );
        assert_eq!(names(&review(&before, &after)), ["b", "a", "main"]);
    }

    #[test]
    fn unchanged_functions_between_still_order_their_changes() {
        // main calls x, which is untouched and calls y.
        let before = system(
            &[("main", &["x"], "1"), ("x", &["y"], "1"), ("y", &[], "1")],
            &[],
        );
        let after = system(
            &[("main", &["x"], "2"), ("x", &["y"], "1"), ("y", &[], "2")],
            &[],
        );
        assert_eq!(names(&review(&before, &after)), ["y", "main"]);
    }

    #[test]
    fn a_chain_reads_as_one_and_the_wider_reaching_goes_first() {
        // Two routes: `orders` reaches `price` and `tax`; `users` only `tax`.
        let entries = [("orders", "POST /orders"), ("users", "POST /users")];
        let before = system(
            &[
                ("orders", &["price"], "1"),
                ("users", &["tax"], "1"),
                ("price", &["tax"], "1"),
                ("tax", &[], "1"),
                ("log", &["audit"], "1"),
                ("audit", &[], "1"),
            ],
            &entries,
        );
        let after = system(
            &[
                ("orders", &["price"], "1"),
                ("users", &["tax"], "1"),
                ("price", &["tax"], "2"),
                ("tax", &[], "2"),
                ("log", &["audit"], "2"),
                ("audit", &[], "2"),
            ],
            &entries,
        );
        let review = review(&before, &after);
        assert_eq!(
            names(&review),
            ["tax", "price", "audit", "log"],
            "tax reaches both routes, so its chain comes first; log, which no route \
             reaches, comes last"
        );
        let tax = UnitId::new("x:tax");
        assert_eq!(review.reached_from(&tax), [0, 1]);
        assert_eq!(review.reached_from(&UnitId::new("x:log")), [0_usize; 0]);
        assert_eq!(review.affected(), [0, 1]);
        assert!(!review.roots);
    }

    #[test]
    fn a_removed_function_comes_before_the_caller_that_dropped_it() {
        let before = system(&[("main", &["gone"], "1"), ("gone", &[], "1")], &[]);
        let after = system(&[("main", &[], "2")], &[]);
        let review = review(&before, &after);
        assert_eq!(names(&review), ["gone", "main"]);
        assert_eq!(
            review
                .starts
                .iter()
                .map(|s| s.unit.as_str())
                .collect::<Vec<_>>(),
            ["x:main"],
            "without declared entries, what nothing calls starts things"
        );
        assert!(review.roots);
    }

    #[test]
    fn a_cycle_of_changes_still_lists_each_once() {
        let before = system(&[("a", &["b"], "1"), ("b", &["a"], "1")], &[]);
        let after = system(&[("a", &["b"], "2"), ("b", &["a"], "2")], &[]);
        let review = review(&before, &after);
        let mut order = names(&review);
        order.sort_unstable();
        assert_eq!(order, ["a", "b"]);
    }
}
