//! Building the IR document from typed steps and their schedule.

use std::collections::{BTreeMap, HashMap};

use codeviz_ir::{
    Arm, Certainty, DagNode, Document, Entry, ExitKind, Invocation, Location, Mode, Producer, Role,
    Scalar, Step as IrStep, Target, Unit, UnitId,
};

use crate::{
    SCHEME,
    attributes::{self, Attributes},
    graph::{Action, Node},
    ids::display_name,
    model::{Command, Kind, Pipeline, SoftFail, Step},
};

/// Scheme for executables in the repository, by path. Another document may analyze them.
const SCRIPT_SCHEME: &str = "script";
/// Scheme for Buildkite plugins, by name.
const PLUGIN_SCHEME: &str = "buildkite-plugin";

pub(crate) struct Emitter<'a> {
    slug: &'a str,
    uri: Option<&'a str>,
    units: BTreeMap<UnitId, Unit>,
}

impl<'a> Emitter<'a> {
    pub(crate) fn new(slug: &'a str, uri: Option<&'a str>) -> Self {
        Self {
            slug,
            uri,
            units: BTreeMap::new(),
        }
    }

    pub(crate) fn document(mut self, pipeline: &Pipeline, nodes: &[Node<'_>]) -> Document {
        let id = UnitId::from_parts(SCHEME, self.slug);
        let mut unit = self.unit(id.clone(), self.slug, "pipeline", Role::Invocable);
        for (key, value) in &pipeline.extra {
            attributes::flatten(key, value, &mut unit.attributes);
        }

        let mut dag = Vec::with_capacity(nodes.len());
        for node in nodes {
            let mut body = Vec::new();
            if let Action::Run(step) | Action::Gate(step) = node.action {
                let mut call = Invocation::new(&step.id, 0, vec![exact(self.step_id(&step.id))]);
                call.propagates =
                    matches!(node.action, Action::Run(_)) && step.soft_fail != SoftFail::Always;
                body.push(IrStep::Call {
                    call: call.key.clone(),
                });
                unit.calls.push(call);
            }
            if let Action::Gate(step) = node.action {
                body.push(unblocked(step));
            }
            for guard in node.guards.iter().rev() {
                body = vec![branch(guard, "true", body, "false", Vec::new())];
            }
            dag.push(DagNode {
                id: node.id.to_owned(),
                after: node.after.iter().map(|id| (*id).to_owned()).collect(),
                body,
            });
        }
        unit.body = Some(vec![IrStep::Dag { nodes: dag }]);
        self.units.insert(id.clone(), unit);

        let tolerant: HashMap<&str, bool> = nodes
            .iter()
            .map(|node| (node.id, node.tolerates_failure))
            .collect();
        for step in &pipeline.steps {
            self.step(step, &id, &tolerant);
        }

        let mut document = Document::new(Producer {
            name: env!("CARGO_PKG_NAME").to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        });
        document.units = self.units.into_values().collect();
        document.entries.push(Entry {
            unit: id,
            label: format!("{} build", self.slug),
            kind: "ci_trigger".to_owned(),
            chain: Vec::new(),
        });
        document
    }

    fn step(&mut self, step: &Step, parent: &UnitId, tolerant: &HashMap<&str, bool>) {
        let (kind, role) = match &step.kind {
            Kind::Wait { .. } => return,
            Kind::Command(_) => ("step", Role::Invocable),
            Kind::Gate { .. } => ("gate", Role::Invocable),
            Kind::Trigger { .. } => ("trigger", Role::Invocable),
            Kind::Group(_) => ("group", Role::Group),
        };
        let id = self.step_id(&step.id);
        let name = step
            .label
            .as_deref()
            .map(display_name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| step.id.clone());
        let mut unit = self.unit(id.clone(), &name, kind, role);
        unit.parent = Some(parent.clone());
        step_attributes(step, tolerant, &mut unit.attributes);

        match &step.kind {
            Kind::Command(command) => self.commands(command, &mut unit),
            Kind::Trigger { pipeline, is_async } => self.trigger(pipeline, *is_async, &mut unit),
            Kind::Gate { prompt } => {
                if let Some(prompt) = prompt {
                    unit.attributes
                        .insert("prompt".to_owned(), Scalar::from(prompt.as_str()));
                }
            }
            Kind::Group(children) => {
                for child in children {
                    self.step(child, &id, tolerant);
                }
            }
            Kind::Wait { .. } => {}
        }
        self.units.insert(id, unit);
    }

    /// Plugins first, as their hooks run before the command, then each command line.
    /// A failing line fails the step.
    fn commands(&mut self, command: &Command, unit: &mut Unit) {
        let mut occurrences = HashMap::new();
        for plugin in &command.plugins {
            let name = plugin.name();
            let target = self.stub(UnitId::from_parts(PLUGIN_SCHEME, name), name, "plugin");
            let n = occurrence(&mut occurrences, &plugin.reference);
            let mut call = Invocation::new(&plugin.reference, n, vec![exact(target)]);
            call.propagates = true;
            unit.calls.push(call);
            attributes::flatten(
                &format!("plugins.{name}"),
                &plugin.config,
                &mut unit.attributes,
            );
        }
        for line in &command.lines {
            let targets = script_path(line)
                .map(|path| {
                    let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
                    exact(self.stub(UnitId::from_parts(SCRIPT_SCHEME, &path), &name, "script"))
                })
                .into_iter()
                .collect();
            let n = occurrence(&mut occurrences, line);
            let mut call = Invocation::new(line, n, targets);
            call.propagates = true;
            unit.calls.push(call);
        }
    }

    fn trigger(&mut self, pipeline: &str, is_async: bool, unit: &mut Unit) {
        let id = UnitId::from_parts(SCHEME, pipeline);
        let target = if pipeline == self.slug {
            id
        } else {
            self.stub(id, pipeline, "pipeline")
        };
        let mut call = Invocation::new(pipeline, 0, vec![exact(target)]);
        if is_async {
            call.mode = Mode::Spawn;
        } else {
            call.propagates = true;
        }
        unit.calls.push(call);
    }

    /// An `external` unit standing in for something defined in another document.
    fn stub(&mut self, id: UnitId, name: &str, kind: &str) -> UnitId {
        if !self.units.contains_key(&id) {
            let unit = Unit::new(id.clone(), name, kind, Role::External);
            self.units.insert(id.clone(), unit);
        }
        id
    }

    fn unit(&self, id: UnitId, name: &str, kind: &str, role: Role) -> Unit {
        let mut unit = Unit::new(id, name, kind, role);
        unit.location = self.uri.map(|uri| Location {
            uri: uri.to_owned(),
            range: None,
        });
        unit
    }

    fn step_id(&self, step: &str) -> UnitId {
        UnitId::from_parts(SCHEME, &format!("{}/{step}", self.slug))
    }
}

/// The 0-based occurrence of `text` among the calls seen so far.
fn occurrence<'t>(occurrences: &mut HashMap<&'t str, usize>, text: &'t str) -> usize {
    let n = occurrences.entry(text).or_default();
    *n += 1;
    *n - 1
}

fn exact(unit: UnitId) -> Target {
    Target {
        unit,
        certainty: Certainty::Exact,
    }
}

fn branch(guard: &str, yes: &str, then: Vec<IrStep>, no: &str, otherwise: Vec<IrStep>) -> IrStep {
    IrStep::Branch {
        guard: Some(guard.to_owned()),
        arms: vec![
            Arm {
                label: yes.to_owned(),
                body: then,
            },
            Arm {
                label: no.to_owned(),
                body: otherwise,
            },
        ],
    }
}

/// Nobody unblocking a gate leaves the build blocked: nothing after it runs.
fn unblocked(step: &Step) -> IrStep {
    let exit = IrStep::Exit {
        kind: ExitKind::Return,
        label: Some("build left blocked".to_owned()),
    };
    branch(
        &format!("{} unblocked", step.id),
        "unblocked",
        Vec::new(),
        "blocked",
        vec![exit],
    )
}

fn step_attributes(step: &Step, tolerant: &HashMap<&str, bool>, out: &mut Attributes) {
    if let Some(label) = &step.label {
        out.insert("label".to_owned(), Scalar::from(label.as_str()));
    }
    if step.key.is_none() {
        out.insert("key_derived".to_owned(), Scalar::Bool(true));
    }
    match &step.soft_fail {
        SoftFail::No => {}
        SoftFail::Always => {
            out.insert("soft_fail".to_owned(), Scalar::Bool(true));
        }
        SoftFail::ExitStatuses(statuses) => {
            out.insert("soft_fail".to_owned(), Scalar::String(statuses.join(", ")));
        }
    }
    if tolerant.get(step.id.as_str()).copied().unwrap_or(false) {
        out.insert("allow_dependency_failure".to_owned(), Scalar::Bool(true));
    }
    if let Some(instances) = attributes::instances(&step.extra) {
        out.insert("instances".to_owned(), Scalar::Int(instances));
    }
    for (key, value) in &step.extra {
        attributes::flatten(key, value, out);
    }
}

/// The repository path a command line executes, if it runs a script by path.
fn script_path(line: &str) -> Option<String> {
    let mut words = line
        .split_whitespace()
        .skip_while(|word| is_assignment(word));
    let mut program = words.next()?;
    if matches!(
        program,
        "bash" | "sh" | "zsh" | "python" | "python3" | "ruby"
    ) {
        program = words.next()?;
    }
    let looks_like_path = program.starts_with("./")
        || (program.contains('/')
            && !program.starts_with('/')
            && [".sh", ".bash", ".py", ".rb"]
                .iter()
                .any(|extension| program.ends_with(extension)));
    looks_like_path.then(|| program.trim_start_matches("./").to_owned())
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_scripts_run_by_path() {
        assert_eq!(
            script_path("./scripts/build.sh --release").as_deref(),
            Some("scripts/build.sh")
        );
        assert_eq!(
            script_path("ENV=prod bash scripts/deploy.sh api").as_deref(),
            Some("scripts/deploy.sh")
        );
        assert_eq!(script_path("cargo test --workspace"), None);
        assert_eq!(script_path("/usr/bin/env true"), None);
    }
}
