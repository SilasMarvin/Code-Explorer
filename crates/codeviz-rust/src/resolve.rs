//! Cross-file pass: turns written call targets into [`Callee`]s.

use std::collections::{HashMap, HashSet};

use codeviz_graph::{CallGraph, CallSite, Callee, Function, FunctionId, Resolution};

use crate::extract::{CallTarget, Extraction, RawCall, RawFunction, Scope};

/// Bound on `use` indirection, so import cycles cannot loop forever.
const MAX_IMPORT_HOPS: usize = 8;

pub(crate) fn resolve(
    extraction: &Extraction,
    crate_names: &HashSet<String>,
) -> Result<CallGraph, codeviz_graph::GraphError> {
    let index = Index::new(extraction, crate_names);
    let functions = extraction
        .functions
        .iter()
        .map(|raw| Function {
            name: raw.name.clone(),
            module: raw.module.clone(),
            owner: raw.owner.clone(),
            is_async: raw.is_async,
            signature: raw.signature.clone(),
            location: raw.location.clone(),
            end_line: raw.end_line,
            calls: raw
                .calls
                .iter()
                .map(|call| CallSite {
                    text: call.text.clone(),
                    callee: index.resolve(raw, call),
                    location: call.location.clone(),
                })
                .collect(),
        })
        .collect();
    CallGraph::new(functions)
}

struct Index<'a> {
    extraction: &'a Extraction,
    crate_names: &'a HashSet<String>,
    /// Full path (`crate::module::[Type::]name`) to functions.
    by_path: HashMap<Vec<String>, Vec<FunctionId>>,
    /// Method name to methods a by-name match may pick: inherent methods and methods
    /// of traits declared in the analyzed source.
    methods: HashMap<&'a str, Vec<FunctionId>>,
}

impl<'a> Index<'a> {
    fn new(extraction: &'a Extraction, crate_names: &'a HashSet<String>) -> Self {
        let mut by_path: HashMap<Vec<String>, Vec<FunctionId>> = HashMap::new();
        let mut methods: HashMap<&str, Vec<FunctionId>> = HashMap::new();
        for (index, function) in extraction.functions.iter().enumerate() {
            let Some(id) = CallGraph::id_for_index(index) else {
                continue;
            };
            by_path.entry(full_path(function)).or_default().push(id);
            if let Some(owner) = &function.owner {
                let is_local_trait = owner
                    .trait_name
                    .as_ref()
                    .is_none_or(|name| extraction.traits.contains(name));
                if is_local_trait {
                    methods.entry(function.name.as_str()).or_default().push(id);
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

    fn resolve(&self, caller: &RawFunction, call: &RawCall) -> Callee {
        match &call.target {
            CallTarget::Path(segments) => self.resolve_path(caller, segments),
            CallTarget::Method { name, on_self } => self.resolve_method(caller, name, *on_self),
        }
    }

    fn resolve_path(&self, caller: &RawFunction, segments: &[String]) -> Callee {
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
                return exact(ids);
            }
        }

        // The path may run through a re-export or a type alias we do not model. Fall
        // back to matching its tail against every function.
        let matches: Vec<FunctionId> = self
            .extraction
            .functions
            .iter()
            .enumerate()
            .filter(|(_, function)| full_path(function).ends_with(&segments))
            .filter_map(|(index, _)| CallGraph::id_for_index(index))
            .collect();
        inferred(&matches)
    }

    fn resolve_method(&self, caller: &RawFunction, name: &str, on_self: bool) -> Callee {
        if on_self && let Some(owner) = &caller.owner {
            let same_type: Vec<FunctionId> = self
                .methods
                .get(name)
                .into_iter()
                .flatten()
                .copied()
                .filter(|id| {
                    self.extraction
                        .functions
                        .get(id.index())
                        .and_then(|f| f.owner.as_ref())
                        .is_some_and(|o| o.type_name == owner.type_name)
                })
                .collect();
            if !same_type.is_empty() {
                return exact(&same_type);
            }
        }
        self.methods
            .get(name)
            .map_or(Callee::External, |ids| inferred(ids))
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
                    .and_then(|scope: &Scope| scope.imports.get(&first))
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
}

fn full_path(function: &RawFunction) -> Vec<String> {
    let mut path = function.module.clone();
    if let Some(owner) = &function.owner {
        path.push(owner.type_name.clone());
    }
    path.push(function.name.clone());
    path
}

fn exact(ids: &[FunctionId]) -> Callee {
    match ids {
        [id] => Callee::Function {
            id: *id,
            resolution: Resolution::Exact,
        },
        _ => Callee::Ambiguous(ids.to_vec()),
    }
}

fn inferred(ids: &[FunctionId]) -> Callee {
    match ids {
        [] => Callee::External,
        [id] => Callee::Function {
            id: *id,
            resolution: Resolution::Inferred,
        },
        _ => Callee::Ambiguous(ids.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{AnalyzeOptions, extract::Extractor};

    /// Builds a graph from `(module path, source)` pairs in a crate named `app`.
    fn graph(files: &[(&str, &str)]) -> CallGraph {
        let mut extractor = Extractor::new(AnalyzeOptions::default()).unwrap();
        for (module, source) in files {
            let module: Vec<String> = module.split("::").map(str::to_owned).collect();
            extractor
                .extract(source.as_bytes(), Path::new("src/x.rs"), &module)
                .unwrap();
        }
        let crates = HashSet::from(["app".to_owned()]);
        resolve(&extractor.finish(), &crates).unwrap()
    }

    /// `caller -> [(call text, resolved qualified name or "?")]`
    fn edges(graph: &CallGraph, caller: &str) -> Vec<(String, String)> {
        let id = graph.find(caller)[0];
        graph
            .function(id)
            .unwrap()
            .calls
            .iter()
            .map(|site| {
                let target = match &site.callee {
                    Callee::Function { id, resolution } => format!(
                        "{}{}",
                        graph.function(*id).unwrap().qualified_name(),
                        if *resolution == Resolution::Inferred {
                            " (inferred)"
                        } else {
                            ""
                        }
                    ),
                    Callee::Ambiguous(ids) => format!("ambiguous x{}", ids.len()),
                    Callee::External => "?".to_owned(),
                };
                (site.text.clone(), target)
            })
            .collect()
    }

    fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
        expected
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    #[test]
    fn resolves_through_modules_and_imports() {
        let graph = graph(&[
            (
                "app",
                "use crate::svc::users; use store::Store as Db;
                 fn main() { users::find(); Db::open(); svc::helper(); unknown(); }",
            ),
            ("app::svc", "pub fn helper() {}"),
            (
                "app::svc::users",
                "use super::helper; pub fn find() { helper(); self::local(); } fn local() {}",
            ),
            (
                "app::store",
                "pub struct Store; impl Store { pub fn open() -> Self { Self::new() } fn new() -> Self { Store } }",
            ),
        ]);

        assert_eq!(
            edges(&graph, "app::main"),
            pairs(&[
                ("users::find", "app::svc::users::find"),
                ("Db::open", "app::store::Store::open"),
                ("svc::helper", "app::svc::helper"),
                ("unknown", "?"),
            ])
        );
        assert_eq!(
            edges(&graph, "users::find"),
            pairs(&[
                ("helper", "app::svc::helper"),
                ("self::local", "app::svc::users::local")
            ])
        );
        assert_eq!(
            edges(&graph, "Store::open"),
            pairs(&[("Self::new", "app::store::Store::new")])
        );
    }

    #[test]
    fn method_calls() {
        let graph = graph(&[(
            "app",
            "struct A; struct B; trait Local { fn go(&self) {} }
             impl A { fn run(&self) { self.step(); other.step(); x.solo(); x.len(); x.fmt(); } fn step(&self) {} fn solo(&self) {} }
             impl B { fn step(&self) {} }
             impl std::fmt::Display for A { fn fmt(&self) {} }
             fn free() { a.go(); }",
        )]);

        assert_eq!(
            edges(&graph, "A::run"),
            pairs(&[
                (".step", "app::A::step"),
                (".step", "ambiguous x2"),
                (".solo", "app::A::solo (inferred)"),
                (".len", "?"),
                (".fmt", "?"),
            ])
        );
        assert_eq!(
            edges(&graph, "free"),
            pairs(&[(".go", "app::Local::go (inferred)")])
        );
    }
}
