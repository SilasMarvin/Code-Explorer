//! Cross-file pass: resolves written call targets and assembles the IR document.

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
};

use codeviz_ir::{
    CallKey, Certainty, Dispatch, Document, Entry, Invocation, Step, Target, Unit, UnitId,
};

use crate::{
    extract::{CallTarget, Extraction, Fields, RawCall, RawUnit, Receiver, ReceiverRoot, UnitKind},
    scip::{self, Scip},
    std_methods::is_std_method,
};

/// Bound on `use` indirection, so import cycles cannot loop forever.
const MAX_IMPORT_HOPS: usize = 8;
/// Bound on re-exports followed along one path; each hop may try every glob.
const MAX_REEXPORT_HOPS: usize = 4;
pub(crate) const SCHEME: &str = "rust";

pub(crate) fn id(path: &[String]) -> UnitId {
    UnitId::from_parts(SCHEME, &path.join("::"))
}

/// Each analyzed crate, to the analyzed crates its code can name.
pub(crate) type Crates = HashMap<String, HashSet<String>>;

pub(crate) fn assemble(
    extraction: &mut Extraction,
    crates: &Crates,
    scip: Option<&Scip>,
    document: &mut Document,
) {
    disambiguate(&mut extraction.units);
    let extraction = &*extraction;
    let mut index = Index::new(extraction, crates);
    index.semantic = scip.map(|scip| Semantic::new(scip, extraction));
    for raw in &extraction.units {
        document.units.push(index.unit(raw));
    }
    document.entries = index.entries();
}

struct Index<'a> {
    extraction: &'a Extraction,
    crates: &'a Crates,
    /// Names of the structs, enums, unions and traits declared in the analyzed source.
    local_types: HashSet<&'a str>,
    /// Each unit's id path, to the unit.
    at_path: HashMap<&'a [String], usize>,
    /// Lookup path (`crate::module::[Type::]name`) to functions.
    by_path: HashMap<Vec<String>, Vec<usize>>,
    /// Method name to methods a by-name match may pick: inherent methods and methods
    /// of traits declared in the analyzed source.
    methods: HashMap<&'a str, Vec<usize>>,
    /// `(type name, method name)` to every method on that type, trait impls included.
    by_type: HashMap<(&'a str, &'a str), Vec<usize>>,
    semantic: Option<Semantic<'a>>,
}

/// What a call site may invoke.
struct Found {
    units: Vec<usize>,
    certainty: Certainty,
    /// Nothing here, because the call runs code outside the analyzed source (std or a
    /// dependency), so what it returns is of a type defined there too.
    outside: bool,
}

impl Found {
    fn new(units: Vec<usize>, certainty: Certainty) -> Self {
        Self {
            units,
            certainty,
            outside: false,
        }
    }

    fn nothing() -> Self {
        Self::new(Vec::new(), Certainty::Exact)
    }
}

/// What a method's receiver is known to be.
enum Value {
    /// A value of this type, as sure as the certainty says.
    Typed(Vec<String>, Certainty),
    /// A value of a type defined outside the analyzed source.
    Foreign,
    Unknown,
}

/// rust-analyzer's resolution, tied to the extracted units.
struct Semantic<'a> {
    scip: &'a Scip,
    /// Each function symbol defined in the analyzed source, to its units.
    units: HashMap<&'a str, Vec<usize>>,
}

impl<'a> Semantic<'a> {
    fn new(scip: &'a Scip, extraction: &Extraction) -> Self {
        let at: HashMap<scip::Position, usize> = extraction
            .units
            .iter()
            .enumerate()
            .filter_map(|(index, unit)| {
                let (line, column) = unit.name_at?;
                Some(((unit.location.as_ref()?.uri.clone(), line, column), index))
            })
            .collect();
        let mut units: HashMap<&str, Vec<usize>> = HashMap::new();
        for (symbol, positions) in &scip.definitions {
            if !scip::is_function(symbol) {
                continue;
            }
            for position in positions {
                if let Some(&index) = at.get(position) {
                    units.entry(symbol.as_str()).or_default().push(index);
                }
            }
        }
        Self { scip, units }
    }
}

impl<'a> Index<'a> {
    fn new(extraction: &'a Extraction, crates: &'a Crates) -> Self {
        let mut by_path: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
        let mut methods: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut by_type: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
        let local_types = extraction
            .units
            .iter()
            .filter(|unit| {
                matches!(
                    unit.kind,
                    UnitKind::Type("struct" | "enum" | "union" | "trait")
                )
            })
            .map(|unit| unit.name.as_str())
            .collect();
        let at_path = extraction
            .units
            .iter()
            .enumerate()
            .map(|(index, unit)| (unit.path.as_slice(), index))
            .collect();
        for (index, unit) in extraction.units.iter().enumerate() {
            if !matches!(unit.kind, UnitKind::Function { .. }) {
                continue;
            }
            by_path.entry(unit.lookup_path()).or_default().push(index);
            if let Some(owner) = &unit.owner {
                by_type
                    .entry((owner.type_name.as_str(), unit.name.as_str()))
                    .or_default()
                    .push(index);
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
            crates,
            local_types,
            at_path,
            by_path,
            methods,
            by_type,
            semantic: None,
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
        if let CallTarget::Closure(path) = &call.target {
            return vec![Target {
                unit: id(path),
                certainty: Certainty::Exact,
            }];
        }
        let Found {
            units: mut ids,
            certainty,
            ..
        } = self.candidates(caller, call);
        // Candidates come from hash maps; source order keeps documents deterministic.
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
            .filter_map(|i| self.extraction.units.get(i))
            .map(|unit| Target {
                unit: id(&unit.path),
                certainty,
            })
            .collect()
    }

    /// Whether code in `caller`'s crate can name the crate unit `index` is in.
    fn visible(&self, caller: &RawUnit, index: usize) -> bool {
        let to = self
            .extraction
            .units
            .get(index)
            .and_then(|u| u.module.first());
        let (Some(from), Some(to)) = (caller.module.first(), to) else {
            return true;
        };
        self.crates
            .get(from)
            .is_none_or(|visible| visible.contains(to))
    }

    /// Whether a type path names a type declared in the analyzed source. Paths are
    /// compared by their last segment: a foreign type used without `use` gets the
    /// module it is used in as its path.
    fn is_local_type(&self, ty: &[String]) -> bool {
        ty.last()
            .is_some_and(|name| self.local_types.contains(name.as_str()))
    }

    /// The functions at a full path, following `use` re-exports along it: where the
    /// crate root says `pub use store::Store`, `app::Store::open` is
    /// `app::store::Store::open`.
    fn functions_at(&self, path: &[String], hops: usize) -> Option<&Vec<usize>> {
        if let Some(ids) = self.by_path.get(path) {
            return Some(ids);
        }
        if hops >= MAX_REEXPORT_HOPS {
            return None;
        }
        (1..path.len()).find_map(|split| {
            let (module, rest) = path.split_at(split);
            let (name, rest) = rest.split_first()?;
            let scope = self.extraction.scopes.get(module)?;
            let exported: Vec<Vec<String>> = match scope.imports.get(name) {
                Some(import) => vec![import.clone()],
                None => scope
                    .globs
                    .iter()
                    .map(|glob| glob.iter().chain([name]).cloned().collect())
                    .collect(),
            };
            exported.into_iter().find_map(|import| {
                let mut full = self.absolute(module, import, 0)?;
                full.extend(rest.iter().cloned());
                self.functions_at(&full, hops + 1)
            })
        })
    }

    /// The functions one path names. Several are implementations of one generic
    /// trait for different arguments (`From<&str>`, `From<bool>`), and which runs
    /// depends on types: a guess. An inherent function takes precedence.
    fn overloads(&self, ids: &[usize]) -> (Vec<usize>, Certainty) {
        if let [_] = ids {
            return (ids.to_vec(), Certainty::Exact);
        }
        let inherent: Vec<usize> = ids
            .iter()
            .copied()
            .filter(|&i| {
                self.extraction
                    .units
                    .get(i)
                    .and_then(|u| u.owner.as_ref())
                    .is_some_and(|o| o.trait_name.is_none())
            })
            .collect();
        match inherent.as_slice() {
            [_] => (inherent, Certainty::Exact),
            _ => (ids.to_vec(), Certainty::Inferred),
        }
    }

    /// For `Type::function`, `Type` when it is declared in the analyzed source (or is
    /// `Self`).
    fn associated_type<'s>(&self, segments: &'s [String]) -> Option<&'s [String]> {
        let ty = segments.split_last().map(|(_, ty)| ty)?;
        let local = ty.last().is_some_and(|name| name == "Self") || self.is_local_type(ty);
        local.then_some(ty)
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
                .and_then(|path| self.functions_at(&path, 0))
            {
                return self.overloads(ids);
            }
        }

        // Aliased re-exports and macro-made items are not modeled; match the path's
        // tail instead. A bare name never names a method: that takes `Type::method` or
        // a receiver.
        let bare = segments.len() == 1;
        let matches = self
            .by_path
            .iter()
            .filter(|(path, _)| path.ends_with(&segments))
            .flat_map(|(_, ids)| ids.iter().copied())
            .filter(|&i| {
                !bare
                    || self
                        .extraction
                        .units
                        .get(i)
                        .is_some_and(|u| u.owner.is_none())
            })
            .filter(|&i| self.visible(caller, i))
            .collect();
        (matches, Certainty::Inferred)
    }

    fn resolve_method(&self, caller: &RawUnit, name: &str, receiver: Option<&Receiver>) -> Found {
        let (ty, certainty) = match self.receiver_value(caller, receiver) {
            Value::Typed(ty, certainty) => (ty, certainty),
            // A std name on an unknown value is std's far more often than ours. What
            // it returns may still be ours: an element of a collection, say.
            Value::Foreign | Value::Unknown if is_std_method(name) => {
                return Found::nothing();
            }
            // Of ours, only a local trait implemented for a foreign type can run.
            Value::Foreign => {
                let units: Vec<usize> = self.extension_methods(name).collect();
                return Found {
                    outside: units.is_empty(),
                    ..Found::new(units, Certainty::Inferred)
                };
            }
            // Nothing is known about the receiver: match by name, among the crates the
            // caller can name, if only one type (or one trait's implementations) has a
            // method so named. A name several types use is vocabulary, not a clue.
            Value::Unknown => {
                let units: Vec<usize> = self
                    .methods
                    .get(name)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|&i| self.visible(caller, i))
                    .collect();
                let mut owners = units.iter().filter_map(|&i| self.dispatcher(i));
                let first = owners.next();
                if owners.any(|owner| Some(owner) != first) {
                    return Found::nothing();
                }
                return Found::new(units, Certainty::Inferred);
            }
        };
        let type_name = ty.last().map_or("", String::as_str);
        if self.extraction.traits.contains(type_name) {
            // A trait object or generic bound: any implementation may run.
            let implementations: Vec<usize> = self
                .methods_named(name)
                .filter(|&i| {
                    self.extraction
                        .units
                        .get(i)
                        .and_then(|u| u.owner.as_ref())
                        .is_some_and(|o| {
                            o.trait_name.as_deref() == Some(type_name) && o.type_name != type_name
                        })
                })
                .collect();
            if !implementations.is_empty() {
                return Found::new(implementations, certainty);
            }
        }
        // The type's own path first: two crates may each have a `Builder`.
        let mut path = ty.clone();
        path.push(name.to_owned());
        if let Some(ids) = self.functions_at(&path, 0) {
            return Found::new(ids.clone(), certainty);
        }
        let on_type: Vec<usize> = self
            .by_type
            .get(&(type_name, name))
            .into_iter()
            .flatten()
            .copied()
            .filter(|&i| self.visible(caller, i))
            .collect();
        if !on_type.is_empty() {
            return Found::new(on_type, certainty);
        }
        // The type is known but defines no such method here: a provided method of a
        // local trait, or something from std or a dependency.
        let provided: Vec<usize> = self
            .methods
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&i| {
                self.extraction
                    .units
                    .get(i)
                    .and_then(|u| u.owner.as_ref())
                    .is_some_and(|o| o.trait_name.as_deref() == Some(o.type_name.as_str()))
            })
            .collect();
        Found {
            // Unless the type is ours, its other methods are std's or a dependency's,
            // and so is what they return, unless it is std's own vocabulary: `get`,
            // `first` or `unwrap` may hand back a value of ours.
            outside: provided.is_empty() && !self.is_local_type(&ty) && !is_std_method(name),
            ..Found::new(provided, Certainty::Inferred)
        }
    }

    /// For a call like `Type::default()` on a type of ours that defines no such
    /// function (a derive or a std trait does), `Type`: what the call returns.
    fn derived_constructor<'c>(&self, call: &'c RawCall) -> Option<&'c [String]> {
        const CONSTRUCTORS: [&str; 6] = ["clone", "default", "from", "from_str", "new", "try_from"];
        let CallTarget::Path(segments) = &call.target else {
            return None;
        };
        let name = segments.last()?;
        CONSTRUCTORS
            .contains(&name.as_str())
            .then(|| self.associated_type(segments))
            .flatten()
    }

    /// What a method belongs to for dispatch: the local trait it implements, or else
    /// the type it is defined on.
    fn dispatcher(&self, index: usize) -> Option<&str> {
        let owner = self.extraction.units.get(index)?.owner.as_ref()?;
        Some(
            owner
                .trait_name
                .as_deref()
                .filter(|t| self.extraction.traits.contains(*t))
                .unwrap_or(&owner.type_name),
        )
    }

    /// Methods called `name` a value of a foreign type may have here: those of local
    /// traits implemented for foreign types, and their provided defaults.
    fn extension_methods<'s>(&'s self, name: &'s str) -> impl Iterator<Item = usize> + 's {
        self.methods_named(name).filter(|&i| {
            let owner = self.extraction.units.get(i).and_then(|u| u.owner.as_ref());
            owner.is_some_and(|o| {
                o.trait_name.as_ref().is_some_and(|t| {
                    self.extraction.traits.contains(t)
                        && (*t == o.type_name || !self.local_types.contains(o.type_name.as_str()))
                })
            })
        })
    }

    /// Every method called `name`, on any type or trait.
    fn methods_named<'s>(&'s self, name: &'s str) -> impl Iterator<Item = usize> + 's {
        self.by_type
            .iter()
            .filter(move |((_, method), _)| *method == name)
            .flat_map(|(_, ids)| ids.iter().copied())
    }

    /// What a call site may invoke, before closures are considered.
    fn candidates(&self, caller: &RawUnit, call: &RawCall) -> Found {
        if let Some((units, certainty)) = self.resolved(call) {
            return Found {
                outside: units.is_empty(),
                ..Found::new(units, certainty)
            };
        }
        match &call.target {
            CallTarget::Path(segments) => {
                let (units, certainty) = self.resolve_path(caller, segments);
                Found {
                    // `Type::default()` on a type of ours is a derive, not foreign code.
                    outside: units.is_empty() && self.associated_type(segments).is_none(),
                    ..Found::new(units, certainty)
                }
            }
            // A bare name passed as a value is far more often a variable than a
            // function imported under another name: only an exact match counts.
            CallTarget::Reference(segments) => match self.resolve_path(caller, segments) {
                (_, Certainty::Inferred) if segments.len() == 1 => Found::nothing(),
                (units, certainty) => Found::new(units, certainty),
            },
            CallTarget::Method { name, receiver } => {
                self.resolve_method(caller, name, receiver.as_ref())
            }
            CallTarget::Closure(_) => Found::nothing(),
        }
    }

    /// What rust-analyzer resolved `call` to, if it did. A call into std or a
    /// dependency, or of a local binding, resolves to nothing.
    fn resolved(&self, call: &RawCall) -> Option<(Vec<usize>, Certainty)> {
        let semantic = self.semantic.as_ref()?;
        if matches!(call.target, CallTarget::Closure(_)) {
            return None;
        }
        let (line, column) = call.name_at?;
        let position = (call.location.uri.clone(), line, column);
        let symbols = semantic.scip.symbols_at(&position)?;
        let Some(symbol) = symbols.iter().find(|s| scip::is_function(s)) else {
            // A local binding, a field, a constructor: nothing of ours runs.
            return Some((Vec::new(), Certainty::Exact));
        };
        let symbol = symbol.as_str();
        let defined = semantic.units.get(symbol).map_or(&[][..], Vec::as_slice);
        let Some((trait_name, method)) =
            scip::trait_method(symbol).filter(|(t, _)| self.extraction.traits.contains(*t))
        else {
            return Some((defined.to_vec(), Certainty::Exact));
        };
        // Called through a generic bound or a trait object: any implementation may run,
        // or the trait's own default.
        let mut implementations: Vec<usize> = self
            .methods_named(method)
            .filter(|&i| {
                self.extraction
                    .units
                    .get(i)
                    .and_then(|u| u.owner.as_ref())
                    .is_some_and(|o| {
                        o.trait_name.as_deref() == Some(trait_name) && o.type_name != trait_name
                    })
            })
            .collect();
        implementations.extend_from_slice(defined);
        Some((implementations, Certainty::Exact))
    }

    /// Resolves a type path written in `scope`'s module; `Self` is `scope`'s owner.
    fn type_path(&self, scope: &RawUnit, mut path: Vec<String>) -> Option<Vec<String>> {
        if path.first().is_some_and(|s| s == "Self") {
            let owner = scope.owner.as_ref()?;
            path.splice(
                ..1,
                scope
                    .module
                    .iter()
                    .cloned()
                    .chain([owner.type_name.clone()]),
            );
            return Some(path);
        }
        self.absolute(&scope.module, path, 0)
    }

    /// What a receiver is, following declared types, field types and return types.
    fn receiver_value(&self, caller: &RawUnit, receiver: Option<&Receiver>) -> Value {
        let Some(receiver) = receiver else {
            return Value::Unknown;
        };
        let (mut ty, certainty) = match self.root_value(caller, &receiver.root) {
            Value::Typed(ty, certainty) => (ty, certainty),
            other => return other,
        };
        for field in &receiver.fields {
            let Some((struct_path, fields)) = self.struct_fields(&ty) else {
                return if self.is_local_type(&ty) {
                    Value::Unknown
                } else {
                    Value::Foreign
                };
            };
            let field_ty = fields.get(field);
            let module = struct_path.get(..struct_path.len().saturating_sub(1));
            let Some(field_ty) = module
                .zip(field_ty)
                .and_then(|(module, field_ty)| self.absolute(module, field_ty.clone(), 0))
            else {
                return Value::Unknown;
            };
            ty = field_ty;
        }
        Value::Typed(ty, certainty)
    }

    /// What the binding or value a receiver starts from is, in `unit`.
    fn root_value(&self, unit: &RawUnit, root: &ReceiverRoot) -> Value {
        let typed = match root {
            ReceiverRoot::SelfValue => unit.owner.as_ref().map(|owner| {
                let mut path = unit.module.clone();
                path.push(owner.type_name.clone());
                (path, Certainty::Exact)
            }),
            ReceiverRoot::Typed { path, declared } => {
                let certainty = if *declared {
                    Certainty::Exact
                } else {
                    Certainty::Inferred
                };
                self.type_path(unit, path.clone()).map(|ty| (ty, certainty))
            }
            ReceiverRoot::Returned { call, tried } => {
                // The receiver is what an earlier call returned: use its declared type.
                let Some(call) = unit.calls.get(*call) else {
                    return Value::Unknown;
                };
                let found = self.candidates(unit, call);
                if found.outside {
                    return Value::Foreign;
                }
                if found.units.is_empty()
                    && let Some(ty) = self.derived_constructor(call)
                {
                    let ty = self.type_path(unit, ty.to_vec());
                    return ty.map_or(Value::Unknown, |ty| Value::Typed(ty, Certainty::Inferred));
                }
                let [function] = found.units.as_slice() else {
                    return Value::Unknown;
                };
                self.extraction.units.get(*function).and_then(|function| {
                    let returned = if *tried {
                        &function.returns_ok
                    } else {
                        &function.returns
                    };
                    let ty = self.type_path(function, returned.clone()?)?;
                    Some((ty, found.certainty))
                })
            }
            ReceiverRoot::Captured { from, root } => {
                return match self.at_path.get(from.as_slice()) {
                    Some(&index) => self
                        .extraction
                        .units
                        .get(index)
                        .map_or(Value::Unknown, |from| self.root_value(from, root)),
                    None => Value::Unknown,
                };
            }
        };
        typed.map_or(Value::Unknown, |(ty, certainty)| {
            Value::Typed(ty, certainty)
        })
    }

    /// A struct's fields by exact path, or by name when only one struct has it.
    fn struct_fields(&self, ty: &[String]) -> Option<(&Vec<String>, &Fields)> {
        if let Some((path, fields)) = self.extraction.fields.get_key_value(ty) {
            return Some((path, fields));
        }
        let name = ty.last()?;
        let mut matches = self
            .extraction
            .fields
            .iter()
            .filter(|(path, _)| path.last() == Some(name));
        match (matches.next(), matches.next()) {
            (Some(only), None) => Some(only),
            _ => None,
        }
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
                } else if self.crates.contains_key(&first) {
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
                .and_then(|path| self.functions_at(&path, 0))
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

/// Items repeated under different `#[cfg]`s (or impls the id grammar cannot tell apart)
/// share a path. Later ones get a `{variant#n}` suffix, in source order. Modules and
/// types live in a different Rust namespace from functions, so a function that shares a
/// module's path gets the suffix too.
fn disambiguate(units: &mut [RawUnit]) {
    let is_group = |unit: &RawUnit| {
        matches!(
            unit.kind,
            UnitKind::Crate | UnitKind::Module | UnitKind::Type(_)
        )
    };
    let mut seen: HashMap<Vec<String>, usize> = units
        .iter()
        .filter(|u| is_group(u))
        .map(|u| (u.path.clone(), 1))
        .collect();
    for unit in units.iter_mut() {
        if is_group(unit) {
            continue;
        }
        let count = seen.entry(unit.path.clone()).or_default();
        if *count > 0
            && let Some(last) = unit.path.last_mut()
        {
            let _ = write!(last, "{{variant#{count}}}");
        }
        *count += 1;
    }
}
