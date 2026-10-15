//! Pipeline YAML, parsed into typed steps.
//!
//! Buildkite accepts many shapes for the same thing (`wait` or `- wait: ~`, a string or
//! a list for `command`, `key`/`id`/`identifier`). This module absorbs them so the rest
//! of the crate sees one shape per concept. Fields it does not model are kept in
//! [`Step::extra`] and surface as attributes.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::Error;

#[derive(Debug)]
pub(crate) struct Pipeline {
    pub steps: Vec<Step>,
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug)]
pub(crate) struct Step {
    /// The declared `key`, if any.
    pub key: Option<String>,
    /// The key or a derived stand-in; unique across the pipeline. Set by [`crate::ids`].
    pub id: String,
    pub label: Option<String>,
    /// `None` when absent. `Some(empty)` is an explicit "no dependencies".
    pub depends_on: Option<Vec<Dependency>>,
    /// Predicates that must all hold for the step to run (`if`, `branches`).
    pub guards: Vec<String>,
    pub soft_fail: SoftFail,
    pub allow_dependency_failure: bool,
    pub kind: Kind,
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dependency {
    pub step: String,
    pub allow_failure: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SoftFail {
    No,
    Always,
    /// Soft-fails only on these exit statuses (`*` for any).
    ExitStatuses(Vec<String>),
}

#[derive(Debug)]
pub(crate) enum Kind {
    Command(Command),
    Wait {
        continue_on_failure: bool,
    },
    /// A `block` or `input` step: waits for a person.
    Gate {
        prompt: Option<String>,
    },
    Trigger {
        pipeline: String,
        is_async: bool,
    },
    Group(Vec<Step>),
}

impl Kind {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Command(_) => "command",
            Self::Wait { .. } => "wait",
            Self::Gate { .. } => "block",
            Self::Trigger { .. } => "trigger",
            Self::Group(_) => "group",
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Command {
    /// Shell command lines, in order.
    pub lines: Vec<String>,
    pub plugins: Vec<Plugin>,
}

#[derive(Debug)]
pub(crate) struct Plugin {
    /// As written, including any `#version`.
    pub reference: String,
    pub config: Value,
}

impl Plugin {
    /// The plugin's short name: `docker-compose` for `docker-compose#v5.10.0`,
    /// `my-plugin` for `github.com/acme/my-plugin-buildkite-plugin#v1`.
    pub(crate) fn name(&self) -> &str {
        let path = self
            .reference
            .split_once('#')
            .map_or(self.reference.as_str(), |(path, _)| path);
        let last = path.rsplit('/').next().unwrap_or(path);
        last.strip_suffix("-buildkite-plugin").unwrap_or(last)
    }
}

pub(crate) fn parse(yaml: &str) -> Result<Pipeline, Error> {
    let raw: RawPipeline = serde_saphyr::from_str(yaml).map_err(Box::new)?;
    let (steps, extra) = match raw {
        RawPipeline::Map { steps, extra } => (steps, extra),
        RawPipeline::List(steps) => (steps, BTreeMap::new()),
    };
    Ok(Pipeline {
        steps: steps
            .into_iter()
            .enumerate()
            .map(|(index, step)| step.into_step(&index.to_string()))
            .collect::<Result<_, _>>()?,
        extra,
    })
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawPipeline {
    Map {
        steps: Vec<RawStep>,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    List(Vec<RawStep>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawStep {
    Bare(String),
    Map(Box<RawMap>),
}

#[derive(Deserialize)]
struct RawMap {
    #[serde(alias = "id", alias = "identifier")]
    key: Option<String>,
    #[serde(alias = "name")]
    label: Option<String>,
    command: Option<Value>,
    commands: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    wait: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    block: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    input: Option<Value>,
    trigger: Option<String>,
    #[serde(default, deserialize_with = "present")]
    group: Option<Value>,
    steps: Option<Vec<RawStep>>,
    #[serde(default, deserialize_with = "present")]
    depends_on: Option<Value>,
    #[serde(rename = "if")]
    condition: Option<String>,
    branches: Option<String>,
    soft_fail: Option<Value>,
    allow_dependency_failure: Option<bool>,
    continue_on_failure: Option<bool>,
    #[serde(rename = "async")]
    is_async: Option<bool>,
    prompt: Option<String>,
    plugins: Option<Value>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

/// Distinguishes `field: ~` (present, null) from an absent field.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

impl RawStep {
    /// `at` locates the step for error messages: its index path within the pipeline.
    fn into_step(self, at: &str) -> Result<Step, Error> {
        match self {
            Self::Bare(word) => {
                let kind = match word.as_str() {
                    "wait" | "waiter" => Kind::Wait {
                        continue_on_failure: false,
                    },
                    "block" | "input" => Kind::Gate { prompt: None },
                    _ => return Err(unsupported(at, format!("unknown step `{word}`"))),
                };
                Ok(Step {
                    key: None,
                    id: String::new(),
                    label: None,
                    depends_on: None,
                    guards: Vec::new(),
                    soft_fail: SoftFail::No,
                    allow_dependency_failure: false,
                    kind,
                    extra: BTreeMap::new(),
                })
            }
            Self::Map(map) => map.into_step(at),
        }
    }
}

impl RawMap {
    fn into_step(self, at: &str) -> Result<Step, Error> {
        let label = self.label.or_else(|| {
            [&self.block, &self.input, &self.group]
                .into_iter()
                .flatten()
                .find_map(|value| value.as_str().map(str::to_owned))
        });
        let kind = if self.command.is_some() || self.commands.is_some() || self.plugins.is_some() {
            let mut lines = Vec::new();
            for value in [self.command, self.commands].into_iter().flatten() {
                lines.extend(command_lines(&value, at)?);
            }
            Kind::Command(Command {
                lines,
                plugins: plugins(self.plugins, at)?,
            })
        } else if self.wait.is_some() {
            if self.condition.is_some() {
                return Err(unsupported(at, "a conditional `wait`".into()));
            }
            Kind::Wait {
                continue_on_failure: self.continue_on_failure.unwrap_or(false),
            }
        } else if self.block.is_some() || self.input.is_some() {
            Kind::Gate {
                prompt: self.prompt,
            }
        } else if let Some(pipeline) = self.trigger {
            Kind::Trigger {
                pipeline,
                is_async: self.is_async.unwrap_or(false),
            }
        } else if self.group.is_some() {
            let steps = self.steps.unwrap_or_default();
            Kind::Group(
                steps
                    .into_iter()
                    .enumerate()
                    .map(|(index, step)| step.into_step(&format!("{at}.{index}")))
                    .collect::<Result<_, _>>()?,
            )
        } else {
            return Err(unsupported(at, "a step of unknown type".into()));
        };

        let guards = self
            .condition
            .iter()
            .map(|condition| normalize(condition))
            .chain(
                self.branches
                    .iter()
                    .map(|branches| format!("build.branch in [{}]", normalize(branches))),
            )
            .collect();

        Ok(Step {
            key: self.key,
            id: String::new(),
            label,
            depends_on: self
                .depends_on
                .map(|value| dependencies(&value, at))
                .transpose()?,
            guards,
            soft_fail: soft_fail(self.soft_fail.as_ref(), at)?,
            allow_dependency_failure: self.allow_dependency_failure.unwrap_or(false),
            kind,
            extra: self.extra,
        })
    }
}

/// Collapses whitespace so that guards equal up to formatting compare equal.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn unsupported(at: &str, what: String) -> Error {
    Error::Unsupported {
        step: at.to_owned(),
        what,
    }
}

fn command_lines(value: &Value, at: &str) -> Result<Vec<String>, Error> {
    let scripts = match value {
        Value::String(script) => vec![script.as_str()],
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .ok_or_else(|| unsupported(at, "a non-string command".into()))
            })
            .collect::<Result<_, _>>()?,
        _ => return Err(unsupported(at, "a non-string command".into())),
    };
    Ok(scripts.into_iter().flat_map(script_lines).collect())
}

/// Splits a script into logical lines: joins `\` continuations, drops blanks and comments.
fn script_lines(script: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut pending = String::new();
    for line in script.lines() {
        let line = line.trim();
        if let Some(head) = line.strip_suffix('\\') {
            pending.push_str(head.trim_end());
            pending.push(' ');
            continue;
        }
        pending.push_str(line);
        let logical = std::mem::take(&mut pending);
        let logical = logical.trim();
        if !logical.is_empty() && !logical.starts_with('#') {
            lines.push(logical.to_owned());
        }
    }
    let rest = pending.trim();
    if !rest.is_empty() {
        lines.push(rest.to_owned());
    }
    lines
}

fn plugins(value: Option<Value>, at: &str) -> Result<Vec<Plugin>, Error> {
    let entries: Vec<(String, Value)> = match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .into_iter()
            .map(|item| match item {
                Value::String(reference) => Ok(vec![(reference, Value::Null)]),
                Value::Object(map) => Ok(map.into_iter().collect()),
                _ => Err(unsupported(at, "a malformed plugin".into())),
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect(),
        Some(Value::Object(map)) => map.into_iter().collect(),
        Some(_) => return Err(unsupported(at, "a malformed `plugins`".into())),
    };
    Ok(entries
        .into_iter()
        .map(|(reference, config)| Plugin { reference, config })
        .collect())
}

fn dependencies(value: &Value, at: &str) -> Result<Vec<Dependency>, Error> {
    let one = |item: &Value| match item {
        Value::String(step) => Ok(Dependency {
            step: step.clone(),
            allow_failure: false,
        }),
        Value::Object(map) => map
            .get("step")
            .and_then(Value::as_str)
            .map(|step| Dependency {
                step: step.to_owned(),
                allow_failure: map
                    .get("allow_failure")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
            .ok_or_else(|| unsupported(at, "a `depends_on` entry without `step`".into())),
        _ => Err(unsupported(at, "a malformed `depends_on`".into())),
    };
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items.iter().map(one).collect(),
        item => one(item).map(|dependency| vec![dependency]),
    }
}

fn soft_fail(value: Option<&Value>, at: &str) -> Result<SoftFail, Error> {
    match value {
        None | Some(Value::Bool(false)) => Ok(SoftFail::No),
        Some(Value::Bool(true)) => Ok(SoftFail::Always),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item.get("exit_status") {
                Some(Value::String(status)) => Ok(status.clone()),
                Some(Value::Number(status)) => Ok(status.to_string()),
                _ => Err(unsupported(
                    at,
                    "a `soft_fail` entry without `exit_status`".into(),
                )),
            })
            .collect::<Result<_, _>>()
            .map(SoftFail::ExitStatuses),
        Some(_) => Err(unsupported(at, "a malformed `soft_fail`".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_lines_join_continuations_and_skip_comments() {
        let lines = script_lines("# setup\necho a \\\n  b\n\ncargo test\n");
        assert_eq!(lines, ["echo a b", "cargo test"]);
    }

    #[test]
    fn plugin_names_drop_versions_and_suffixes() {
        let plugin = |reference: &str| Plugin {
            reference: reference.into(),
            config: Value::Null,
        };
        assert_eq!(plugin("docker-compose#v5.10.0").name(), "docker-compose");
        assert_eq!(
            plugin("github.com/acme/cache-buildkite-plugin#v1").name(),
            "cache"
        );
    }

    #[test]
    fn explicit_null_dependencies_differ_from_absent_ones() {
        let pipeline =
            parse("steps:\n  - command: a\n  - command: b\n    depends_on: ~\n").unwrap();
        assert_eq!(pipeline.steps[0].depends_on, None);
        assert_eq!(pipeline.steps[1].depends_on, Some(Vec::new()));
    }

    #[test]
    fn bare_and_mapped_waits_are_the_same_step() {
        let pipeline =
            parse("steps:\n  - wait\n  - wait: ~\n    continue_on_failure: true\n").unwrap();
        assert!(matches!(
            pipeline.steps[0].kind,
            Kind::Wait {
                continue_on_failure: false
            }
        ));
        assert!(matches!(
            pipeline.steps[1].kind,
            Kind::Wait {
                continue_on_failure: true
            }
        ));
    }
}
