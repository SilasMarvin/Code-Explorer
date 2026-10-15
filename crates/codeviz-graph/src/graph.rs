use std::{fmt, path::PathBuf};

/// Index of a [`Function`] within its [`CallGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FunctionId(u32);

impl FunctionId {
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A position in a source file, relative to the analyzed root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    pub file: PathBuf,
    /// 1-based.
    pub line: u32,
    /// 1-based.
    pub column: u32,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file.display(), self.line, self.column)
    }
}

/// The type (and optionally trait) a method is implemented on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Owner {
    pub type_name: String,
    pub trait_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    /// Module path including the crate name, e.g. `["my_app", "services", "users"]`.
    pub module: Vec<String>,
    pub owner: Option<Owner>,
    pub is_async: bool,
    /// The declaration up to (not including) its body.
    pub signature: String,
    pub location: Location,
    /// Last line of the item, 1-based.
    pub end_line: u32,
    /// Call sites in source order.
    pub calls: Vec<CallSite>,
}

impl Function {
    /// `module::Owner::name`, the name used for display and lookup.
    #[must_use]
    pub fn qualified_name(&self) -> String {
        let mut segments: Vec<&str> = self.module.iter().map(String::as_str).collect();
        if let Some(owner) = &self.owner {
            segments.push(&owner.type_name);
        }
        segments.push(&self.name);
        segments.join("::")
    }

    /// The name without the module path: `Owner::name` or `name`.
    #[must_use]
    pub fn short_name(&self) -> String {
        match &self.owner {
            Some(owner) => format!("{}::{}", owner.type_name, self.name),
            None => self.name.clone(),
        }
    }

    #[must_use]
    pub fn module_path(&self) -> String {
        self.module.join("::")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    /// The callee expression as written, e.g. `users::find_user` or `.insert_user`.
    pub text: String,
    pub callee: Callee,
    pub location: Location,
}

/// How confidently a call site was matched to a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resolution {
    /// The path resolved through scope and imports.
    Exact,
    /// Matched by name alone, e.g. a method call on a receiver of unknown type.
    Inferred,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callee {
    Function {
        id: FunctionId,
        resolution: Resolution,
    },
    /// Several analyzed functions match and nothing distinguishes them.
    Ambiguous(Vec<FunctionId>),
    /// Not defined in the analyzed source (std, dependencies, or unresolvable).
    External,
}

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("call site {site} in `{caller}` references function #{index}, but the graph has {len}")]
    DanglingCallee {
        caller: String,
        site: Location,
        index: usize,
        len: usize,
    },
    #[error("graph has {0} functions, more than a FunctionId can address")]
    TooLarge(usize),
}

/// A static call graph. Function ids are indices into the function list.
#[derive(Debug, Clone, Default)]
pub struct CallGraph {
    functions: Vec<Function>,
    callers: Vec<Vec<FunctionId>>,
}

impl CallGraph {
    /// Builds a graph whose [`FunctionId`]s are positions in `functions`.
    ///
    /// # Errors
    /// Fails if a call site references an id outside `functions`.
    pub fn new(functions: Vec<Function>) -> Result<Self, GraphError> {
        u32::try_from(functions.len()).map_err(|_| GraphError::TooLarge(functions.len()))?;
        let mut callers = vec![Vec::new(); functions.len()];
        for (index, function) in functions.iter().enumerate() {
            let caller = FunctionId(u32::try_from(index).map_err(|_| GraphError::TooLarge(index))?);
            for site in &function.calls {
                for callee in site.callee.candidates() {
                    let slot = callers.get_mut(callee.index()).ok_or_else(|| {
                        GraphError::DanglingCallee {
                            caller: function.qualified_name(),
                            site: site.location.clone(),
                            index: callee.index(),
                            len: functions.len(),
                        }
                    })?;
                    if !slot.contains(&caller) {
                        slot.push(caller);
                    }
                }
            }
        }
        Ok(Self { functions, callers })
    }

    /// Converts a position in the list passed to [`CallGraph::new`] into an id.
    ///
    /// Frontends use this to build [`Callee`]s before the graph exists.
    #[must_use]
    pub fn id_for_index(index: usize) -> Option<FunctionId> {
        u32::try_from(index).ok().map(FunctionId)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.functions.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }

    #[must_use]
    pub fn function(&self, id: FunctionId) -> Option<&Function> {
        self.functions.get(id.index())
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "`new` rejects graphs larger than u32::MAX"
    )]
    pub fn functions(&self) -> impl ExactSizeIterator<Item = (FunctionId, &Function)> {
        self.functions
            .iter()
            .enumerate()
            .map(|(index, function)| (FunctionId(index as u32), function))
    }

    /// Functions with a call site that may reach `id`.
    #[must_use]
    pub fn callers(&self, id: FunctionId) -> &[FunctionId] {
        self.callers.get(id.index()).map_or(&[], Vec::as_slice)
    }

    /// Functions nothing in the analyzed source calls: `main`, handlers, trait impls.
    pub fn entry_points(&self) -> impl Iterator<Item = FunctionId> + '_ {
        self.functions()
            .map(|(id, _)| id)
            .filter(|&id| self.callers(id).is_empty())
    }

    /// Finds functions whose qualified name ends with `query` on a `::` boundary.
    ///
    /// `find_user`, `users::find_user` and `my_app::services::users::find_user` all
    /// match `my_app::services::users::find_user`.
    #[must_use]
    pub fn find(&self, query: &str) -> Vec<FunctionId> {
        let wanted: Vec<&str> = query.split("::").collect();
        self.functions()
            .filter(|(_, function)| {
                let name = function.qualified_name();
                let segments: Vec<&str> = name.split("::").collect();
                segments.ends_with(&wanted)
            })
            .map(|(id, _)| id)
            .collect()
    }
}

impl Callee {
    /// Every function this call might invoke.
    #[must_use]
    pub fn candidates(&self) -> &[FunctionId] {
        match self {
            Self::Function { id, .. } => std::slice::from_ref(id),
            Self::Ambiguous(ids) => ids,
            Self::External => &[],
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn location(line: u32) -> Location {
        Location {
            file: PathBuf::from("src/lib.rs"),
            line,
            column: 1,
        }
    }

    pub(crate) fn function(name: &str, calls: &[(usize, Resolution)]) -> Function {
        Function {
            name: name.to_owned(),
            module: vec!["app".to_owned()],
            owner: None,
            is_async: false,
            signature: format!("fn {name}()"),
            location: location(1),
            end_line: 1,
            calls: calls
                .iter()
                .enumerate()
                .map(|(line, &(index, resolution))| CallSite {
                    text: format!("callee_{index}"),
                    callee: Callee::Function {
                        id: CallGraph::id_for_index(index).unwrap(),
                        resolution,
                    },
                    location: location(u32::try_from(line).unwrap() + 2),
                })
                .collect(),
        }
    }

    #[test]
    fn callers_and_entry_points() {
        let graph = CallGraph::new(vec![
            function("main", &[(1, Resolution::Exact), (2, Resolution::Exact)]),
            function("a", &[(2, Resolution::Inferred)]),
            function("b", &[]),
        ])
        .unwrap();

        let ids: Vec<_> = graph.functions().map(|(id, _)| id).collect();
        assert_eq!(graph.callers(ids[2]), &[ids[0], ids[1]]);
        assert_eq!(graph.entry_points().collect::<Vec<_>>(), vec![ids[0]]);
    }

    #[test]
    fn rejects_dangling_callee() {
        let result = CallGraph::new(vec![function("main", &[(7, Resolution::Exact)])]);
        assert!(matches!(
            result,
            Err(GraphError::DanglingCallee { index: 7, .. })
        ));
    }

    #[test]
    fn find_matches_on_segment_boundaries() {
        let mut method = function("user", &[]);
        method.owner = Some(Owner {
            type_name: "Store".to_owned(),
            trait_name: None,
        });
        let graph = CallGraph::new(vec![function("find_user", &[]), method]).unwrap();

        assert_eq!(graph.find("find_user").len(), 1);
        assert_eq!(graph.find("app::find_user").len(), 1);
        assert_eq!(graph.find("Store::user").len(), 1);
        assert_eq!(graph.find("_user"), Vec::new());
    }
}
