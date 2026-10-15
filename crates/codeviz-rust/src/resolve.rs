//! Cross-file pass: resolves written call targets and assembles the IR document.

use std::collections::{HashMap, HashSet};

use codeviz_ir::{
    CallKey, Certainty, Dispatch, Document, Entry, Invocation, Step, Target, Unit, UnitId,
};

use crate::extract::{CallTarget, Extraction, RawCall, RawUnit, UnitKind};

/// Bound on `use` indirection, so import cycles cannot loop forever.
const MAX_IMPORT_HOPS: usize = 8;
pub(crate) const SCHEME: &str = "rust";

pub(crate) fn id(path: &[String]) -> UnitId {
    UnitId::from_parts(SCHEME, &path.join("::"))
}

pub(crate) fn assemble(
    extraction: &Extraction,
    crate_names: &HashSet<String>,
    document: &mut Document,
) {
    let index = Index::new(extraction, crate_names);
    for raw in &extraction.units {
        document.units.push(index.unit(raw));
    }
    document.entries = index.entries();
}

struct Index<'a> {
    extraction: &'a Extraction,
    crate_names: &'a HashSet<String>,
    /// Lookup path (`crate::module::[Type::]name`) to functions.
    by_path: HashMap<Vec<String>, Vec<usize>>,
    /// Method name to methods a by-name match may pick: inherent methods and methods
    /// of traits declared in the analyzed source.
    methods: HashMap<&'a str, Vec<usize>>,
}

impl<'a> Index<'a> {
    fn new(extraction: &'a Extraction, crate_names: &'a HashSet<String>) -> Self {
        let mut by_path: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
        let mut methods: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, unit) in extraction.units.iter().enumerate() {
            if !matches!(unit.kind, UnitKind::Function { .. }) {
                continue;
            }
            by_path.entry(unit.lookup_path()).or_default().push(index);
            if let Some(owner) = &unit.owner {
                let local = owner
                    .trait_name
                    .as_ref()
                    .is_none_or(|name| extraction.traits.contains(name));
                if local {
                    methods.entry(unit.name.as_str()).or_default().push(index);
                }
            }
        }
        Self {
            extraction,
            crate_names,
            by_path,
            methods,
        }
    }

    fn unit(&self, raw: &RawUnit) -> Unit {
        let mut unit = Unit::new(id(&raw.path), &raw.name, raw.kind.label(), raw.kind.role());
        unit.parent = raw.parent.as_deref().map(id);
        unit.location.clone_from(&raw.location);
        unit.content_hash.clone_from(&raw.content_hash);
        unit.doc.clone_from(&raw.doc);
        unit.attributes.clone_from(&raw.attributes);

        let mut dropped = HashSet::new();
        for call in &raw.calls {
            let targets = self.targets(raw, call);
            if matches!(call.target, CallTarget::Reference(_)) && targets.is_empty() {
                // A value passed by name that is not a known function: not a call.
                dropped.insert(call.key.clone());
                continue;
            }
            let via = call
                .via_call
                .and_then(|i| raw.calls.get(i))
                .map(|via| self.targets(raw, via))
                .and_then(|targets| match targets.as_slice() {
                    [only] => Some(only.unit.clone()),
                    _ => None,
                });
            unit.calls.push(Invocation {
                key: call.key.clone(),
                text: call.text.clone(),
                targets,
                dispatch: Dispatch::One,
                mode: call.mode,
                via,
                propagates: call.propagates,
                location: Some(call.location.clone()),
            });
        }
        unit.body = raw.body.as_ref().map(|body| without_calls(body, &dropped));
        unit
    }

    fn targets(&self, caller: &RawUnit, call: &RawCall) -> Vec<Target> {
        let (ids, certainty) = match &call.target {
            CallTarget::Path(segments) | CallTarget::Reference(segments) => {
                self.resolve_path(caller, segments)
            }
            CallTarget::Method { name, on_self } => self.resolve_method(caller, name, *on_self),
            CallTarget::Closure(path) => {
                return vec![Target {
                    unit: id(path),
                    certainty: Certainty::Exact,
                }];
            }
        };
        ids.into_iter()
            .filter_map(|i| self.extraction.units.get(i))
            .map(|unit| Target {
                unit: id(&unit.path),
                certainty,
            })
            .collect()
    }

    fn resolve_path(&self, caller: &RawUnit, segments: &[String]) -> (Vec<usize>, Certainty) {
        let mut segments = segments.to_vec();
        if segments.first().is_some_and(|s| s == "Self")
            && let Some(owner) = &caller.owner
        {
            segments.splice(
                ..1,
                caller
                    .module
                    .iter()
                    .cloned()
                    .chain([owner.type_name.clone()]),
            );
        }

        let scope = self.extraction.scopes.get(&caller.module);
        let globbed = scope.into_iter().flat_map(|scope| {
            scope.globs.iter().map(|glob| {
                glob.iter()
                    .chain(segments.iter())
                    .cloned()
                    .collect::<Vec<_>>()
            })
        });
        for candidate in std::iter::once(segments.clone()).chain(globbed) {
            if let Some(ids) = self
                .absolute(&caller.module, candidate, 0)
                .and_then(|path| self.by_path.get(&path))
            {
                return (ids.clone(), Certainty::Exact);
            }
        }

        // Re-exports and aliases are not modeled; match the path's tail instead.
        let matches = self
            .by_path
            .iter()
            .filter(|(path, _)| path.ends_with(&segments))
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        (matches, Certainty::Inferred)
    }

    fn resolve_method(
        &self,
        caller: &RawUnit,
        name: &str,
        on_self: bool,
    ) -> (Vec<usize>, Certainty) {
        let candidates = self.methods.get(name).map_or(&[][..], Vec::as_slice);
        if on_self && let Some(owner) = &caller.owner {
            let same_type: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|&i| {
                    self.extraction
                        .units
                        .get(i)
                        .and_then(|u| u.owner.as_ref())
                        .is_some_and(|o| o.type_name == owner.type_name)
                })
                .collect();
            if !same_type.is_empty() {
                return (same_type, Certainty::Exact);
            }
        }
        (candidates.to_vec(), Certainty::Inferred)
    }

    /// Rewrites a path as written in `module` into a full path from a crate root.
    fn absolute(
        &self,
        module: &[String],
        mut path: Vec<String>,
        hops: usize,
    ) -> Option<Vec<String>> {
        if hops > MAX_IMPORT_HOPS {
            return None;
        }
        let first = path.first()?.clone();
        match first.as_str() {
            "crate" => {
                path.splice(..1, module.first().cloned());
                Some(path)
            }
            "self" => {
                path.splice(..1, module.iter().cloned());
                Some(path)
            }
            "super" => {
                let supers = path.iter().take_while(|s| *s == "super").count();
                let base = module.get(..module.len().checked_sub(supers)?)?;
                Some(base.iter().chain(path.get(supers..)?).cloned().collect())
            }
            _ => {
                if let Some(import) = self
                    .extraction
                    .scopes
                    .get(module)
                    .and_then(|scope| scope.imports.get(&first))
                {
                    let mut resolved = self.absolute(module, import.clone(), hops + 1)?;
                    resolved.extend(path.into_iter().skip(1));
                    Some(resolved)
                } else if self.crate_names.contains(&first) {
                    Some(path)
                } else {
                    Some(module.iter().cloned().chain(path).collect())
                }
            }
        }
    }

    fn entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        for unit in &self.extraction.units {
            if unit.name == "main" && unit.owner.is_none() && unit.module.len() == 1 {
                entries.push(Entry {
                    unit: id(&unit.path),
                    label: format!("{} main", unit.module.join("::")),
                    kind: "main".to_owned(),
                    chain: Vec::new(),
                });
            }
        }
        for route in &self.extraction.routes {
            let Some(handler) = self
                .absolute(&route.module, route.handler.clone(), 0)
                .and_then(|path| self.by_path.get(&path))
                .and_then(|ids| ids.first())
                .and_then(|&i| self.extraction.units.get(i))
            else {
                continue;
            };
            entries.push(Entry {
                unit: id(&handler.path),
                label: format!("{} {}", route.method, route.path),
                kind: "http".to_owned(),
                chain: Vec::new(),
            });
        }
        entries
    }
}

/// Removes `call` steps for keys that did not survive resolution.
fn without_calls(steps: &[Step], dropped: &HashSet<CallKey>) -> Vec<Step> {
    steps
        .iter()
        .filter(|step| !matches!(step, Step::Call { call } if dropped.contains(call)))
        .map(|step| match step {
            Step::Branch { guard, arms } => Step::Branch {
                guard: guard.clone(),
                arms: arms
                    .iter()
                    .map(|arm| codeviz_ir::Arm {
                        label: arm.label.clone(),
                        body: without_calls(&arm.body, dropped),
                    })
                    .collect(),
            },
            Step::Loop { label, body } => Step::Loop {
                label: label.clone(),
                body: without_calls(body, dropped),
            },
            Step::Dag { nodes } => Step::Dag {
                nodes: nodes
                    .iter()
                    .map(|node| codeviz_ir::DagNode {
                        id: node.id.clone(),
                        after: node.after.clone(),
                        body: without_calls(&node.body, dropped),
                    })
                    .collect(),
            },
            other => other.clone(),
        })
        .collect()
}
