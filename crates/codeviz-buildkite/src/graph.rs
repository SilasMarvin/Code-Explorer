//! Scheduling: Buildkite's ordering rules, resolved into one flat DAG.
//!
//! - A step without `depends_on` waits for the closest preceding `wait` or `block` in
//!   its scope, or for its group's dependencies at the start of a group.
//! - `depends_on` replaces that implicit dependency; `depends_on: ~` removes it.
//! - A `wait` or `block` waits for every step above it in its scope. The edge list is
//!   reduced to the steps since the previous barrier, plus that barrier when one of
//!   those steps did not already wait on it.
//! - A group's `depends_on` and `if` apply to every step inside it. Its own node
//!   finishes after all of its steps, so steps can depend on the group's key.
//!
//! The DAG is flat because steps may depend on steps in other groups and `dag.after`
//! may only name siblings. Groups survive as the containment tree.

use std::collections::HashSet;

use crate::{
    Error,
    model::{Kind, Step},
};

#[derive(Debug)]
pub(crate) struct Node<'a> {
    pub id: &'a str,
    pub after: Vec<&'a str>,
    /// Predicates that must all hold, outermost first.
    pub guards: Vec<&'a str>,
    pub action: Action<'a>,
    /// Starts even if what it waits on failed.
    pub tolerates_failure: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Action<'a> {
    /// Runs the step's unit.
    Run(&'a Step),
    /// Waits for someone to unblock the step.
    Gate(&'a Step),
    /// Finishes once everything it waits on has: a `wait`, or the end of a group.
    Barrier,
}

pub(crate) fn schedule(steps: &[Step]) -> Result<Vec<Node<'_>>, Error> {
    let mut known = HashSet::new();
    collect_ids(steps, &mut known);
    check_dependencies(steps, &known)?;

    let mut nodes = Vec::new();
    Scope::new(Vec::new(), Vec::new()).walk(steps, &mut nodes);
    Ok(nodes)
}

fn collect_ids<'a>(steps: &'a [Step], known: &mut HashSet<&'a str>) {
    for step in steps {
        known.insert(&step.id);
        if let Kind::Group(children) = &step.kind {
            collect_ids(children, known);
        }
    }
}

fn check_dependencies(steps: &[Step], known: &HashSet<&str>) -> Result<(), Error> {
    for step in steps {
        for dependency in step.depends_on.iter().flatten() {
            if !known.contains(dependency.step.as_str()) {
                return Err(Error::UnknownDependency {
                    step: step.id.clone(),
                    depends_on: dependency.step.clone(),
                });
            }
        }
        if let Kind::Group(children) = &step.kind {
            check_dependencies(children, known)?;
        }
    }
    Ok(())
}

struct Scope<'a> {
    /// Dependencies every step in the scope has: its group's.
    required: Vec<&'a str>,
    guards: Vec<&'a str>,
    /// What a step without `depends_on` waits for.
    implicit: Vec<&'a str>,
    /// Steps since the last barrier.
    since: Vec<&'a str>,
    /// Whether every step in `since` waits for `implicit`.
    since_implicit: bool,
    /// The last barrier was a `wait` with `continue_on_failure`.
    continues: bool,
}

impl<'a> Scope<'a> {
    fn new(required: Vec<&'a str>, guards: Vec<&'a str>) -> Self {
        Self {
            implicit: required.clone(),
            required,
            guards,
            since: Vec::new(),
            since_implicit: true,
            continues: false,
        }
    }

    fn walk(&mut self, steps: &'a [Step], nodes: &mut Vec<Node<'a>>) {
        for step in steps {
            match &step.kind {
                Kind::Wait {
                    continue_on_failure,
                } => {
                    let after = self.barrier();
                    nodes.push(self.node(step, after, Action::Barrier));
                    self.close(&step.id, *continue_on_failure);
                }
                Kind::Gate { .. } => {
                    let after = match &step.depends_on {
                        Some(_) => self.after(step),
                        None => self.barrier(),
                    };
                    nodes.push(self.node(step, after, Action::Gate(step)));
                    self.close(&step.id, false);
                }
                Kind::Command(_) | Kind::Trigger { .. } => {
                    let after = self.after(step);
                    nodes.push(self.node(step, after, Action::Run(step)));
                    self.open(step);
                }
                Kind::Group(children) => {
                    let mut guards = self.guards.clone();
                    guards.extend(step.guards.iter().map(String::as_str));
                    let mut inner = Scope::new(self.after(step), guards);
                    inner.continues = self.continues && step.depends_on.is_none();
                    inner.walk(children, nodes);
                    nodes.push(self.node(step, inner.barrier(), Action::Barrier));
                    self.open(step);
                }
            }
        }
    }

    fn after(&self, step: &'a Step) -> Vec<&'a str> {
        let mut after = self.required.clone();
        match &step.depends_on {
            Some(dependencies) => after.extend(dependencies.iter().map(|d| d.step.as_str())),
            None => after.extend(&self.implicit),
        }
        dedup(after)
    }

    /// What a `wait` here would wait for.
    fn barrier(&self) -> Vec<&'a str> {
        let mut after = self.required.clone();
        after.extend(&self.since);
        if self.since.is_empty() || !self.since_implicit {
            after.extend(&self.implicit);
        }
        dedup(after)
    }

    /// Barriers run nothing, so they carry no guards: a skipped step still finishes.
    fn node(&self, step: &'a Step, after: Vec<&'a str>, action: Action<'a>) -> Node<'a> {
        let mut guards = Vec::new();
        if !matches!(action, Action::Barrier) {
            guards.extend(&self.guards);
            guards.extend(step.guards.iter().map(String::as_str));
        }
        let explicit_failure = step
            .depends_on
            .iter()
            .flatten()
            .any(|dependency| dependency.allow_failure);
        Node {
            id: &step.id,
            after,
            guards,
            action,
            tolerates_failure: step.allow_dependency_failure
                || explicit_failure
                || (self.continues && step.depends_on.is_none()),
        }
    }

    fn open(&mut self, step: &'a Step) {
        self.since.push(&step.id);
        self.since_implicit &= step.depends_on.is_none();
    }

    fn close(&mut self, barrier: &'a str, continues: bool) {
        self.implicit = vec![barrier];
        self.since.clear();
        self.since_implicit = true;
        self.continues = continues;
    }
}

fn dedup(ids: Vec<&str>) -> Vec<&str> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ids, model::parse};

    fn plan(yaml: &str) -> Vec<(String, Vec<String>)> {
        let mut pipeline = parse(yaml).unwrap();
        ids::assign(&mut pipeline.steps).unwrap();
        schedule(&pipeline.steps)
            .unwrap()
            .into_iter()
            .map(|node| {
                let after = node.after.iter().map(|id| (*id).to_owned()).collect();
                (node.id.to_owned(), after)
            })
            .collect()
    }

    fn edges(pairs: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        pairs
            .iter()
            .map(|(id, after)| {
                let after = after.iter().map(|a| (*a).to_owned()).collect();
                ((*id).to_owned(), after)
            })
            .collect()
    }

    #[test]
    fn waits_split_the_pipeline_into_stages() {
        let yaml = "
steps:
  - {key: a, command: run-a}
  - {key: b, command: run-b}
  - wait
  - {key: c, command: run-c}
  - wait
  - wait
  - {key: d, command: run-d}
";
        assert_eq!(
            plan(yaml),
            edges(&[
                ("a", &[]),
                ("b", &[]),
                ("wait", &["a", "b"]),
                ("c", &["wait"]),
                ("wait-2", &["c"]),
                ("wait-3", &["wait-2"]),
                ("d", &["wait-3"]),
            ])
        );
    }

    #[test]
    fn depends_on_overrides_the_implicit_wait() {
        let yaml = "
steps:
  - {key: build, command: run-b}
  - {key: lint, command: run-l}
  - wait
  - {key: unit, command: run-u, depends_on: build}
  - {key: free, command: run-f, depends_on: ~}
  - {key: e2e, command: run-e, depends_on: [unit, {step: lint, allow_failure: true}]}
  - wait
";
        assert_eq!(
            plan(yaml),
            edges(&[
                ("build", &[]),
                ("lint", &[]),
                ("wait", &["build", "lint"]),
                ("unit", &["build"]),
                ("free", &[]),
                ("e2e", &["unit", "lint"]),
                // `unit`, `free` and `e2e` skipped the first wait, so the second
                // must name it to still wait for everything above.
                ("wait-2", &["unit", "free", "e2e", "wait"]),
            ])
        );
    }

    #[test]
    fn blocks_are_barriers_and_groups_scope_their_waits() {
        let yaml = "
steps:
  - {key: build, command: run-b}
  - block: Ship?
    key: ship
  - group: Deploy
    key: deploy
    steps:
      - {key: migrate, command: run-m}
      - wait
      - {key: api, command: run-a}
      - {key: worker, command: run-w, depends_on: build}
  - {key: notify, command: run-n, depends_on: deploy}
";
        assert_eq!(
            plan(yaml),
            edges(&[
                ("build", &[]),
                ("ship", &["build"]),
                ("migrate", &["ship"]),
                ("wait", &["ship", "migrate"]),
                ("api", &["ship", "wait"]),
                ("worker", &["ship", "build"]),
                ("deploy", &["ship", "api", "worker", "wait"]),
                ("notify", &["deploy"]),
            ])
        );
    }

    #[test]
    fn group_conditions_apply_to_each_member() {
        let yaml = "
steps:
  - group: Deploy
    if: build.branch == 'main'
    steps:
      - {key: api, command: run-a, if: build.tag != null}
";
        let mut pipeline = parse(yaml).unwrap();
        ids::assign(&mut pipeline.steps).unwrap();
        let nodes = schedule(&pipeline.steps).unwrap();
        assert_eq!(
            nodes[0].guards,
            ["build.branch == 'main'", "build.tag != null"]
        );
        assert!(nodes[1].guards.is_empty(), "barriers carry no guards");
    }

    #[test]
    fn continue_on_failure_marks_the_next_stage() {
        let yaml = "
steps:
  - {key: a, command: run-a}
  - wait: ~
    continue_on_failure: true
  - {key: b, command: run-b}
  - {key: c, command: run-c, depends_on: a}
";
        let mut pipeline = parse(yaml).unwrap();
        ids::assign(&mut pipeline.steps).unwrap();
        let nodes = schedule(&pipeline.steps).unwrap();
        let tolerant: Vec<_> = nodes
            .iter()
            .filter(|node| node.tolerates_failure)
            .map(|node| node.id)
            .collect();
        assert_eq!(tolerant, ["b"]);
    }

    #[test]
    fn unknown_dependencies_are_rejected() {
        let mut pipeline =
            parse("steps:\n  - {key: a, command: run-a, depends_on: nope}\n").unwrap();
        ids::assign(&mut pipeline.steps).unwrap();
        assert!(matches!(
            schedule(&pipeline.steps),
            Err(Error::UnknownDependency { depends_on, .. }) if depends_on == "nope"
        ));
    }
}
