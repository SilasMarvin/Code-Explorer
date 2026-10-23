//! Name resolution: what an expression refers to and which units a call may run.
//!
//! Resolution is syntactic. It follows scopes, imports (absolute, relative and
//! re-exported), class bodies and bases, `self`, `cls` and `super()`, but tracks no
//! values: a method called on anything other than `self`, `cls`, a class or a module is
//! matched by name, when the name is distinctive, and marked [`Certainty::Inferred`].

use std::collections::HashMap;

use codeviz_ir::{Certainty, Target, UnitId};
use tree_sitter::Node;

use crate::{
    SCHEME,
    imports::Imported,
    index::{Binding, Index, ScopeId, ScopeKind},
    syntax::{field, named_children, text},
};

/// Methods of `str`, `list`, `dict`, `set`, files and `object` that code calls all the
/// time: on a value of unknown type, far more likely these than a method of ours.
const BUILTIN_METHODS: [&str; 46] = [
    "add",
    "append",
    "clear",
    "close",
    "copy",
    "count",
    "decode",
    "discard",
    "encode",
    "endswith",
    "extend",
    "find",
    "format",
    "get",
    "index",
    "insert",
    "items",
    "join",
    "keys",
    "lower",
    "lstrip",
    "pop",
    "popitem",
    "read",
    "readline",
    "readlines",
    "remove",
    "replace",
    "reverse",
    "rsplit",
    "rstrip",
    "setdefault",
    "sort",
    "split",
    "splitlines",
    "startswith",
    "strip",
    "title",
    "union",
    "update",
    "upper",
    "values",
    "write",
    "writelines",
    "difference",
    "intersection",
];

/// Imports may re-export imports; this bounds the chain (and breaks cycles).
const MAX_IMPORT_HOPS: usize = 16;

/// What an expression refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Value {
    /// A function, method or lambda.
    Function(UnitId),
    Class(UnitId),
    /// An instance of the class: `self`.
    Instance(UnitId),
    /// `super()` inside a method of the class.
    Super(UnitId),
    /// A module, analyzed or not.
    Module(String),
    /// A module-level variable, identified by its channel id.
    Variable(UnitId),
    Unknown,
}

#[derive(Debug)]
pub(crate) struct Resolver<'i, 't> {
    index: &'i Index<'t>,
    bases: HashMap<UnitId, Vec<UnitId>>,
    subclasses: HashMap<UnitId, Vec<UnitId>>,
}

impl<'i, 't> Resolver<'i, 't> {
    pub(crate) fn new(index: &'i Index<'t>) -> Self {
        let mut resolver = Self {
            index,
            bases: HashMap::new(),
            subclasses: HashMap::new(),
        };
        let mut bases: HashMap<UnitId, Vec<UnitId>> = HashMap::new();
        for pending in &index.bases {
            if let Value::Class(base) = resolver.eval(pending.scope, pending.node) {
                bases.entry(pending.class.clone()).or_default().push(base);
            }
        }
        for (class, class_bases) in &bases {
            for base in class_bases {
                resolver
                    .subclasses
                    .entry(base.clone())
                    .or_default()
                    .push(class.clone());
            }
        }
        for subclasses in resolver.subclasses.values_mut() {
            subclasses.sort();
        }
        resolver.bases = bases;
        resolver
    }

    pub(crate) fn index(&self) -> &'i Index<'t> {
        self.index
    }

    /// Resolves a name the way Python does: the enclosing function scopes, then the
    /// module. Class bodies are visible only to code directly inside them.
    pub(crate) fn lookup(&self, scope: ScopeId, name: &str) -> Value {
        let mut current = Some(scope);
        while let Some(id) = current {
            let candidate = &self.index.scopes[id];
            if (id == scope || candidate.kind != ScopeKind::Class)
                && let Some(binding) = candidate.bindings.get(name)
            {
                return self.binding(id, name, binding, 0);
            }
            current = candidate.parent;
        }
        Value::Unknown
    }

    fn binding(&self, scope: ScopeId, name: &str, binding: &Binding, hops: usize) -> Value {
        match binding {
            Binding::Def(unit) if self.index.is_class(unit) => Value::Class(unit.clone()),
            Binding::Def(unit) => Value::Function(unit.clone()),
            Binding::Import(Imported::Module(module)) => Value::Module(module.clone()),
            Binding::Import(Imported::Name { module, name }) => {
                self.module_attribute(module, name, hops + 1)
            }
            Binding::Variable => Value::Variable(UnitId::from_parts(
                SCHEME,
                &format!("{}.{name}", self.index.module_of(scope)),
            )),
            Binding::Local => Value::Unknown,
            Binding::SelfOf(class) => Value::Instance(class.clone()),
            Binding::ClsOf(class) => Value::Class(class.clone()),
        }
    }

    fn module_attribute(&self, module: &str, name: &str, hops: usize) -> Value {
        if hops > MAX_IMPORT_HOPS {
            return Value::Unknown;
        }
        let Some(&scope) = self.index.modules.get(module) else {
            // An external module: keep the path so `os.path.join` is never mistaken
            // for a method call on an unknown value.
            return Value::Module(format!("{module}.{name}"));
        };
        if let Some(binding) = self.index.scopes[scope].bindings.get(name) {
            return self.binding(scope, name, binding, hops);
        }
        let submodule = format!("{module}.{name}");
        if self.index.modules.contains_key(&submodule) {
            Value::Module(submodule)
        } else {
            Value::Unknown
        }
    }

    /// What `node` (an identifier, attribute chain or `super()`) refers to.
    pub(crate) fn eval(&self, scope: ScopeId, node: Node<'_>) -> Value {
        let source = self.source(scope);
        match node.kind() {
            "identifier" => self.lookup(scope, text(node, source)),
            "attribute" => {
                let (Some(object), Some(name)) = (field(node, "object"), field(node, "attribute"))
                else {
                    return Value::Unknown;
                };
                self.attribute(self.eval(scope, object), text(name, source))
            }
            "call" => {
                let is_super = field(node, "function")
                    .is_some_and(|callee| text(callee, source) == "super")
                    && field(node, "arguments")
                        .is_some_and(|arguments| arguments.named_child_count() == 0);
                match self.enclosing_class(scope) {
                    Some(class) if is_super => Value::Super(class),
                    _ => Value::Unknown,
                }
            }
            "parenthesized_expression" => named_children(node)
                .first()
                .map_or(Value::Unknown, |inner| self.eval(scope, *inner)),
            _ => Value::Unknown,
        }
    }

    fn attribute(&self, value: Value, name: &str) -> Value {
        let method = match value {
            Value::Module(module) => return self.module_attribute(&module, name, 0),
            Value::Class(class) | Value::Instance(class) => self.method(&class, name, 0),
            Value::Super(class) => self.method(&class, name, 1),
            Value::Function(_) | Value::Variable(_) | Value::Unknown => None,
        };
        method.map_or(Value::Unknown, Value::Function)
    }

    /// The units a call to `callee` may run.
    ///
    /// Calling a class runs its `__init__` (if the analyzed source defines one). A
    /// method on `self` runs the method its class resolves to, or an override in a
    /// subclass.
    pub(crate) fn callee(&self, scope: ScopeId, callee: Node<'_>) -> Vec<Target> {
        if callee.kind() == "attribute"
            && let (Some(object), Some(name)) =
                (field(callee, "object"), field(callee, "attribute"))
        {
            let name = text(name, self.source(scope));
            match self.eval(scope, object) {
                Value::Instance(class) => return self.dispatch(&class, name),
                Value::Variable(_) | Value::Unknown => return self.by_name(name),
                _ => {}
            }
        }
        match self.eval(scope, callee) {
            Value::Function(unit) => vec![exact(unit)],
            Value::Class(class) => self
                .method(&class, "__init__", 0)
                .map(exact)
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The function a decorator applied `depth` levels deep returns as its wrapper:
    /// `depth` 1 for `@deco`, 2 for a factory `@deco(arg)`.
    pub(crate) fn wrapper(&self, decorator: &UnitId, depth: usize) -> Option<UnitId> {
        let mut current = decorator.clone();
        for _ in 0..depth {
            let scope = *self.index.function_scopes.get(&current)?;
            let returned = self.index.returns.get(&current)?;
            current = returned.iter().find_map(|name| {
                match self.index.scopes[scope].bindings.get(name) {
                    Some(Binding::Def(unit)) if !self.index.is_class(unit) => Some(unit.clone()),
                    _ => None,
                }
            })?;
        }
        Some(current)
    }

    fn dispatch(&self, class: &UnitId, name: &str) -> Vec<Target> {
        let mut targets: Vec<Target> = self.method(class, name, 0).map(exact).into_iter().collect();
        for subclass in self.descendants(class) {
            if let Some(method) = self
                .index
                .classes
                .get(&subclass)
                .and_then(|c| c.methods.get(name))
            {
                targets.push(Target {
                    unit: method.clone(),
                    certainty: Certainty::Inferred,
                });
            }
        }
        targets
    }

    /// Methods called `name`, for a receiver of unknown value: only when the name is
    /// distinctive, neither a dunder nor a builtin type's method (`get`, `append`), and
    /// only one class defines it, or one class and its subclasses' overrides.
    fn by_name(&self, name: &str) -> Vec<Target> {
        if name.starts_with("__") || BUILTIN_METHODS.contains(&name) {
            return Vec::new();
        }
        let owners: Vec<&UnitId> = self
            .index
            .classes
            .iter()
            .filter(|(_, class)| class.methods.contains_key(name))
            .map(|(id, _)| id)
            .collect();
        let one_family = owners
            .iter()
            .any(|root| owners.iter().all(|owner| self.mro(owner).contains(root)));
        if !one_family {
            return Vec::new();
        }
        self.index
            .methods
            .get(name)
            .into_iter()
            .flatten()
            .map(|unit| Target {
                unit: unit.clone(),
                certainty: Certainty::Inferred,
            })
            .collect()
    }

    /// Looks `name` up along the method resolution order, skipping the first `skip`
    /// classes. The order is a depth-first, left-to-right walk of analyzed bases.
    fn method(&self, class: &UnitId, name: &str, skip: usize) -> Option<UnitId> {
        self.mro(class)
            .iter()
            .skip(skip)
            .find_map(|class| self.index.classes.get(class)?.methods.get(name).cloned())
    }

    fn mro(&self, class: &UnitId) -> Vec<UnitId> {
        let mut order = Vec::new();
        let mut stack = vec![class.clone()];
        while let Some(next) = stack.pop() {
            if order.contains(&next) {
                continue;
            }
            if let Some(bases) = self.bases.get(&next) {
                stack.extend(bases.iter().rev().cloned());
            }
            order.push(next);
        }
        order
    }

    fn descendants(&self, class: &UnitId) -> Vec<UnitId> {
        let mut found: Vec<UnitId> = Vec::new();
        let mut stack = vec![class.clone()];
        while let Some(next) = stack.pop() {
            for subclass in self.subclasses.get(&next).into_iter().flatten() {
                if !found.contains(subclass) {
                    found.push(subclass.clone());
                    stack.push(subclass.clone());
                }
            }
        }
        found
    }

    /// The class whose method body (possibly nested) contains `scope`.
    fn enclosing_class(&self, scope: ScopeId) -> Option<UnitId> {
        let mut current = scope;
        loop {
            let parent = self.index.scopes[current].parent?;
            if self.index.scopes[current].kind == ScopeKind::Function
                && self.index.scopes[parent].kind == ScopeKind::Class
            {
                return Some(self.index.scopes[parent].unit.clone());
            }
            current = parent;
        }
    }

    fn source(&self, scope: ScopeId) -> &'t str {
        &self.index.parsed(self.index.scopes[scope].file).source
    }
}

fn exact(unit: UnitId) -> Target {
    Target {
        unit,
        certainty: Certainty::Exact,
    }
}
