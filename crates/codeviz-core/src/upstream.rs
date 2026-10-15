//! Who can reach a unit: its callers, their callers, and the entries at the top.

use std::collections::{HashMap, VecDeque};

use codeviz_ir::{Certainty, Entry, Mode, Role, UnitId};

use crate::System;

/// How a caller reaches the unit below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// An invocation the producer resolved, declared or observed.
    Call,
    /// An invocation matched by name, or one of several candidates.
    Guessed,
    /// The unit is defined inside the caller (a closure, a nested function) and runs
    /// when it does.
    Encloses,
}

/// One step upstream: `caller` can run `callee`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamEdge {
    pub caller: UnitId,
    pub callee: UnitId,
    pub reach: Reach,
}

/// Everything upstream of one unit.
#[derive(Debug, Clone, Default)]
pub struct Upstream {
    /// Each unit that can reach the target, with its fewest steps to it. The target
    /// itself is at distance 0.
    pub distance: HashMap<UnitId, u32>,
    pub edges: Vec<UpstreamEdge>,
    /// Entries whose unit is upstream, nearest first.
    pub entries: Vec<Entry>,
}

impl Upstream {
    /// Whether `id` can reach the target, or is it.
    #[must_use]
    pub fn contains(&self, id: &UnitId) -> bool {
        self.distance.contains_key(id)
    }

    /// Units with an edge straight into the target.
    pub fn direct(&self) -> impl Iterator<Item = &UpstreamEdge> {
        self.edges
            .iter()
            .filter(|e| self.distance.get(&e.callee) == Some(&0))
    }
}

/// Walks callers breadth-first from `target`, through every edge, including guessed
/// ones; [`Reach`] says which those were. Registrations are wiring, not calls: a router
/// that registers a handler does not reach what the handler runs.
#[must_use]
pub fn upstream(system: &System, target: &UnitId) -> Upstream {
    let mut result = Upstream::default();
    result.distance.insert(target.clone(), 0);
    let mut queue = VecDeque::from([target.clone()]);
    while let Some(callee) = queue.pop_front() {
        let distance = result.distance.get(&callee).copied().unwrap_or_default();
        let enclosing = system
            .unit(&callee)
            .and_then(|u| u.parent.as_ref())
            .filter(|p| system.unit(p).is_some_and(|p| p.role == Role::Invocable))
            .map(|p| (p.clone(), Reach::Encloses));
        let callers = system
            .callers(&callee)
            .iter()
            .filter_map(|caller| Some((caller.clone(), reach(system, caller, &callee)?)));
        for (caller, reach) in callers.chain(enclosing) {
            result.edges.push(UpstreamEdge {
                caller: caller.clone(),
                callee: callee.clone(),
                reach,
            });
            if !result.distance.contains_key(&caller) {
                result.distance.insert(caller.clone(), distance + 1);
                queue.push_back(caller);
            }
        }
    }
    let mut entries: Vec<Entry> = system
        .entries()
        .iter()
        .filter(|e| result.distance.contains_key(&e.unit))
        .cloned()
        .collect();
    entries.sort_by_key(|e| result.distance.get(&e.unit).copied().unwrap_or(u32::MAX));
    result.entries = entries;
    result
}

/// The surest way any of `from`'s invocations runs `to`; `None` when it only
/// registers it.
fn reach(system: &System, from: &UnitId, to: &UnitId) -> Option<Reach> {
    let unit = system.unit(from)?;
    let mut calls = unit
        .calls
        .iter()
        .filter(|call| call.mode != Mode::Register)
        .filter(|call| call.targets.iter().any(|t| &t.unit == to))
        .peekable();
    calls.peek()?;
    let sure = calls.any(|call| {
        call.targets.len() == 1
            && call
                .targets
                .iter()
                .all(|t| t.certainty != Certainty::Inferred)
    });
    Some(if sure { Reach::Call } else { Reach::Guessed })
}

#[cfg(test)]
mod tests {
    use codeviz_ir::Document;

    use super::*;

    /// Two handlers reach `place`; one through a closure. `other` guesses at it.
    fn system() -> System {
        let json = r#"{
          "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" },
          "entries": [
            { "unit": "x:a", "label": "A", "kind": "http" },
            { "unit": "x:b", "label": "B", "kind": "http" },
            { "unit": "x:lonely", "label": "L", "kind": "http" }
          ],
          "units": [
            { "id": "x:a", "name": "a", "kind": "fn", "role": "invocable", "calls": [
              { "key": "place#0", "text": "place", "targets": [{ "unit": "x:place", "certainty": "exact" }] } ] },
            { "id": "x:b", "name": "b", "kind": "fn", "role": "invocable" },
            { "id": "x:router", "name": "router", "kind": "fn", "role": "invocable", "calls": [
              { "key": "route#0", "text": "route", "mode": "register",
                "targets": [{ "unit": "x:a", "certainty": "exact" }] } ] },
            { "id": "x:b::{closure#0}", "name": "{closure#0}", "kind": "closure", "role": "invocable",
              "parent": "x:b", "calls": [
              { "key": "place#0", "text": "place", "targets": [{ "unit": "x:place", "certainty": "exact" }] } ] },
            { "id": "x:other", "name": "other", "kind": "fn", "role": "invocable", "calls": [
              { "key": "place#0", "text": "place", "targets": [
                { "unit": "x:place", "certainty": "inferred" }, { "unit": "x:elsewhere", "certainty": "inferred" } ] } ] },
            { "id": "x:place", "name": "place", "kind": "fn", "role": "invocable" },
            { "id": "x:elsewhere", "name": "elsewhere", "kind": "fn", "role": "invocable" },
            { "id": "x:lonely", "name": "lonely", "kind": "fn", "role": "invocable" }
          ]
        }"#;
        System::new([Document::from_json(json).unwrap()])
    }

    #[test]
    fn walks_to_every_entry_that_reaches_the_target() {
        let system = system();
        let up = upstream(&system, &UnitId::new("x:place"));

        let labels: Vec<&str> = up.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["A", "B"], "nearest first; L never reaches place");
        assert_eq!(up.distance[&UnitId::new("x:b")], 2, "through its closure");
        assert!(!up.contains(&UnitId::new("x:elsewhere")));
        assert!(
            !up.contains(&UnitId::new("x:router")),
            "registering a handler is not calling it"
        );

        let reach = |caller: &str| {
            up.edges
                .iter()
                .find(|e| e.caller.as_str() == caller)
                .map(|e| e.reach)
        };
        assert_eq!(reach("x:a"), Some(Reach::Call));
        assert_eq!(reach("x:other"), Some(Reach::Guessed));
        assert_eq!(reach("x:b"), Some(Reach::Encloses));
        assert_eq!(up.direct().count(), 3);
    }
}
