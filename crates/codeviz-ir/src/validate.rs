//! Referential checks JSON Schema cannot express. Rules are listed in `docs/ir.md`.

use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use crate::{CallKey, DagNode, Document, Mode, Role, Step, Unit, UnitId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    DuplicateUnit,
    MalformedId,
    UnknownParent,
    ContainmentCycle,
    DuplicateCallKey,
    UnknownCallKey,
    CallReferencedTwice,
    SingleArmBranch,
    InvalidDag,
    UnknownEntryUnit,
    UnreferencedCall,
    UnknownTarget,
    UnreachableStep,
    EntryNotInvocable,
    PropagatingHandoff,
    UnknownLinkEndpoint,
}

impl Rule {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::DuplicateUnit => "E001",
            Self::MalformedId => "E002",
            Self::UnknownParent => "E003",
            Self::ContainmentCycle => "E004",
            Self::DuplicateCallKey => "E005",
            Self::UnknownCallKey => "E006",
            Self::CallReferencedTwice => "E007",
            Self::SingleArmBranch => "E008",
            Self::InvalidDag => "E009",
            Self::UnknownEntryUnit => "E010",
            Self::UnreferencedCall => "W001",
            Self::UnknownTarget => "W002",
            Self::UnreachableStep => "W003",
            Self::EntryNotInvocable => "W004",
            Self::PropagatingHandoff => "W005",
            Self::UnknownLinkEndpoint => "W006",
        }
    }

    #[must_use]
    pub fn severity(self) -> Severity {
        if self.code().starts_with('E') {
            Severity::Error
        } else {
            Severity::Warning
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub rule: Rule,
    pub unit: Option<UnitId>,
    pub message: String,
}

impl Diagnostic {
    #[must_use]
    pub fn severity(&self) -> Severity {
        self.rule.severity()
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.rule.code())?;
        if let Some(unit) = &self.unit {
            write!(f, " [{unit}]")?;
        }
        write!(f, " {}", self.message)
    }
}

/// Checks `document` against the rules in `docs/ir.md`, errors first.
#[must_use]
pub fn validate(document: &Document) -> Vec<Diagnostic> {
    let mut out = Diagnostics::default();
    let mut ids: HashMap<&UnitId, &Unit> = HashMap::new();

    for unit in &document.units {
        if ids.insert(&unit.id, unit).is_some() {
            out.push(
                Rule::DuplicateUnit,
                Some(unit),
                "unit id is not unique".into(),
            );
        }
        if !is_well_formed(&unit.id) {
            out.push(
                Rule::MalformedId,
                Some(unit),
                "id must be `<scheme>:<path>`".into(),
            );
        }
    }

    for unit in &document.units {
        if let Some(parent) = &unit.parent
            && !ids.contains_key(parent)
        {
            out.push(
                Rule::UnknownParent,
                Some(unit),
                format!("parent `{parent}` is not in this document"),
            );
        }
        check_calls(unit, &ids, &mut out);
    }
    check_containment_cycles(document, &ids, &mut out);

    for entry in &document.entries {
        match ids.get(&entry.unit) {
            None => out.push_entry(Rule::UnknownEntryUnit, entry, "is not in this document"),
            Some(unit) if unit.role != Role::Invocable => {
                out.push_entry(Rule::EntryNotInvocable, entry, "is not invocable");
            }
            Some(_) => {}
        }
    }

    // A link endpoint in a scheme this document defines should exist here.
    let schemes: HashSet<&str> = document
        .units
        .iter()
        .filter_map(|u| u.id.scheme())
        .collect();
    let refines = document
        .units
        .iter()
        .flat_map(|u| u.refines.iter().map(move |to| (&u.id, to)));
    let links = document.links.iter().map(|l| (&l.from, &l.to));
    for (from, to) in links.chain(refines) {
        for end in [from, to] {
            if end.scheme().is_some_and(|s| schemes.contains(s)) && !ids.contains_key(end) {
                out.0.push(Diagnostic {
                    rule: Rule::UnknownLinkEndpoint,
                    unit: None,
                    message: format!("link `{from}` → `{to}` names unknown `{end}`"),
                });
            }
        }
    }

    let mut diagnostics = out.0;
    diagnostics.sort_by_key(|d| std::cmp::Reverse(d.severity()));
    diagnostics
}

#[derive(Default)]
struct Diagnostics(Vec<Diagnostic>);

impl Diagnostics {
    fn push(&mut self, rule: Rule, unit: Option<&Unit>, message: String) {
        self.0.push(Diagnostic {
            rule,
            unit: unit.map(|u| u.id.clone()),
            message,
        });
    }

    fn push_entry(&mut self, rule: Rule, entry: &crate::Entry, problem: &str) {
        self.0.push(Diagnostic {
            rule,
            unit: None,
            message: format!("entry `{}` unit `{}` {problem}", entry.label, entry.unit),
        });
    }
}

fn is_well_formed(id: &UnitId) -> bool {
    let (Some(scheme), Some(path)) = (id.scheme(), id.path()) else {
        return false;
    };
    !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !path.is_empty()
        && !id.as_str().chars().any(char::is_whitespace)
}

fn check_calls(unit: &Unit, ids: &HashMap<&UnitId, &Unit>, out: &mut Diagnostics) {
    let mut keys = HashSet::new();
    for call in &unit.calls {
        if !keys.insert(&call.key) {
            out.push(
                Rule::DuplicateCallKey,
                Some(unit),
                format!("call key `{}` is repeated", call.key),
            );
        }
        if call.propagates && matches!(call.mode, Mode::Spawn | Mode::Register) {
            out.push(
                Rule::PropagatingHandoff,
                Some(unit),
                format!("`{}` is not awaited but is marked as propagating", call.key),
            );
        }
        for target in &call.targets {
            if target.unit.scheme() == unit.id.scheme() && !ids.contains_key(&target.unit) {
                out.push(
                    Rule::UnknownTarget,
                    Some(unit),
                    format!("`{}` targets unknown `{}`", call.key, target.unit),
                );
            }
        }
    }

    let Some(body) = &unit.body else {
        return;
    };
    let mut referenced = HashSet::new();
    check_block(unit, body, &keys, &mut referenced, out);
    for call in &unit.calls {
        if !referenced.contains(&call.key) {
            out.push(
                Rule::UnreferencedCall,
                Some(unit),
                format!("`{}` is never reached by the body", call.key),
            );
        }
    }
}

fn check_block<'u>(
    unit: &'u Unit,
    block: &'u [Step],
    keys: &HashSet<&CallKey>,
    referenced: &mut HashSet<&'u CallKey>,
    out: &mut Diagnostics,
) {
    for (index, step) in block.iter().enumerate() {
        match step {
            Step::Call { call } => {
                if !keys.contains(call) {
                    out.push(
                        Rule::UnknownCallKey,
                        Some(unit),
                        format!("body calls unknown key `{call}`"),
                    );
                } else if !referenced.insert(call) {
                    out.push(
                        Rule::CallReferencedTwice,
                        Some(unit),
                        format!("`{call}` is called by two steps"),
                    );
                }
            }
            Step::Branch { arms, guard } => {
                if arms.len() < 2 {
                    let guard = guard.as_deref().unwrap_or("<unlabeled>");
                    out.push(
                        Rule::SingleArmBranch,
                        Some(unit),
                        format!("branch `{guard}` needs at least two arms (write the empty arm explicitly)"),
                    );
                }
                for arm in arms {
                    check_block(unit, &arm.body, keys, referenced, out);
                }
            }
            Step::Loop { body, .. } => check_block(unit, body, keys, referenced, out),
            Step::Dag { nodes } => {
                check_dag(unit, nodes, out);
                for node in nodes {
                    check_block(unit, &node.body, keys, referenced, out);
                }
            }
            Step::Exit { .. } => {
                if index + 1 < block.len() {
                    out.push(
                        Rule::UnreachableStep,
                        Some(unit),
                        "steps follow an exit".into(),
                    );
                }
            }
            Step::Unknown => {}
        }
    }
}

fn check_dag(unit: &Unit, nodes: &[DagNode], out: &mut Diagnostics) {
    let mut ids = HashSet::new();
    for node in nodes {
        if !ids.insert(node.id.as_str()) {
            out.push(
                Rule::InvalidDag,
                Some(unit),
                format!("dag node `{}` is repeated", node.id),
            );
        }
    }
    for node in nodes {
        for after in &node.after {
            if !ids.contains(after.as_str()) {
                out.push(
                    Rule::InvalidDag,
                    Some(unit),
                    format!("dag node `{}` waits on unknown `{after}`", node.id),
                );
            }
        }
    }

    // Kahn's algorithm: anything left unscheduled is on a cycle.
    let mut remaining: HashMap<&str, usize> = nodes
        .iter()
        .map(|n| {
            (
                n.id.as_str(),
                n.after.iter().filter(|a| ids.contains(a.as_str())).count(),
            )
        })
        .collect();
    let mut ready: Vec<&str> = remaining
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(id, _)| *id)
        .collect();
    while let Some(done) = ready.pop() {
        remaining.remove(done);
        for node in nodes.iter().filter(|n| n.after.iter().any(|a| a == done)) {
            if let Some(count) = remaining.get_mut(node.id.as_str()) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    ready.push(&node.id);
                }
            }
        }
    }
    if !remaining.is_empty() {
        let mut cycle: Vec<&str> = remaining.into_keys().collect();
        cycle.sort_unstable();
        out.push(
            Rule::InvalidDag,
            Some(unit),
            format!("dag has a cycle through {}", cycle.join(", ")),
        );
    }
}

fn check_containment_cycles(
    document: &Document,
    ids: &HashMap<&UnitId, &Unit>,
    out: &mut Diagnostics,
) {
    for unit in &document.units {
        let mut seen = HashSet::from([&unit.id]);
        let mut current = unit.parent.as_ref();
        while let Some(parent) = current {
            if !seen.insert(parent) {
                if parent == &unit.id {
                    out.push(
                        Rule::ContainmentCycle,
                        Some(unit),
                        "unit contains itself".into(),
                    );
                }
                break;
            }
            current = ids.get(parent).and_then(|p| p.parent.as_ref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Arm, Entry, ExitKind, Invocation, Producer, Target};

    fn document(units: Vec<Unit>) -> Document {
        let mut document = Document::new(Producer {
            name: "test".into(),
            version: "0".into(),
        });
        document.units = units;
        document
    }

    fn unit(id: &str) -> Unit {
        Unit::new(UnitId::new(id), id, "fn", Role::Invocable)
    }

    fn codes(document: &Document) -> Vec<&'static str> {
        validate(document).iter().map(|d| d.rule.code()).collect()
    }

    #[test]
    fn clean_document_passes() {
        let mut main = unit("x:main");
        main.calls.push(Invocation::new(
            "helper",
            0,
            vec![Target {
                unit: UnitId::new("x:helper"),
                certainty: crate::Certainty::Exact,
            }],
        ));
        let mut doc = document(vec![main, unit("x:helper")]);
        doc.entries.push(Entry {
            unit: UnitId::new("x:main"),
            label: "main".into(),
            kind: "main".into(),
            chain: Vec::new(),
        });
        assert!(validate(&doc).is_empty(), "{:?}", validate(&doc));
    }

    #[test]
    fn identity_and_containment() {
        let mut child = unit("x:child");
        child.parent = Some(UnitId::new("x:loop"));
        let mut looped = unit("x:loop");
        looped.parent = Some(UnitId::new("x:child"));
        let mut orphan = unit("x:orphan");
        orphan.parent = Some(UnitId::new("x:missing"));
        let doc = document(vec![
            unit("x:dup"),
            unit("x:dup"),
            unit("NoScheme"),
            child,
            looped,
            orphan,
        ]);

        let codes = codes(&doc);
        for code in ["E001", "E002", "E003", "E004"] {
            assert!(codes.contains(&code), "{code} missing from {codes:?}");
        }
    }

    #[test]
    fn body_rules() {
        let mut f = unit("x:f");
        f.calls.push(Invocation::new("a", 0, Vec::new()));
        f.calls.push(Invocation::new("b", 0, Vec::new()));
        let call = |key: &str| Step::Call {
            call: CallKey::new(key),
        };
        f.body = Some(vec![
            call("a#0"),
            call("a#0"),
            call("nope#0"),
            Step::Branch {
                guard: Some("lonely".into()),
                arms: vec![Arm {
                    label: "only".into(),
                    body: Vec::new(),
                }],
            },
            Step::Exit {
                kind: ExitKind::Return,
                label: None,
            },
            Step::Dag {
                nodes: vec![
                    DagNode {
                        id: "x".into(),
                        after: vec!["y".into()],
                        body: Vec::new(),
                    },
                    DagNode {
                        id: "y".into(),
                        after: vec!["x".into()],
                        body: Vec::new(),
                    },
                ],
            },
        ]);
        let codes = codes(&document(vec![f]));
        for code in ["E006", "E007", "E008", "E009", "W001", "W003"] {
            assert!(codes.contains(&code), "{code} missing from {codes:?}");
        }
        assert_eq!(
            codes.first().map(|c| &c[..1]),
            Some("E"),
            "errors sort first"
        );
    }

    #[test]
    fn external_scheme_targets_are_not_dangling() {
        let mut f = unit("rust:f");
        f.calls.push(Invocation::new(
            "deploy",
            0,
            vec![
                Target {
                    unit: UnitId::new("buildkite:deploy/release"),
                    certainty: crate::Certainty::Declared,
                },
                Target {
                    unit: UnitId::new("rust:gone"),
                    certainty: crate::Certainty::Exact,
                },
            ],
        ));
        assert_eq!(codes(&document(vec![f])), vec!["W002"]);
    }
}
