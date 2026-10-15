//! Definitions pass: units, scopes and name bindings for every file, before any call
//! is resolved.

use std::collections::{BTreeMap, HashMap, HashSet};

use codeviz_ir::{Location, Role, Scalar, Unit, UnitId};
use tree_sitter::{Node, Tree};

use crate::{
    SCHEME,
    discover::SourceFile,
    hash::content_hash,
    imports::{self, Imported, Importer},
    syntax::{
        compact, docstring, field, location, named_children, parameter_names, target_names, text,
    },
};

/// A parsed source file. Outlives the [`Index`], which borrows its syntax tree.
#[derive(Debug)]
pub(crate) struct Parsed {
    pub(crate) file: SourceFile,
    pub(crate) source: String,
    pub(crate) tree: Tree,
}

pub(crate) type ScopeId = usize;

/// What a name is bound to in a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Binding {
    /// A function, lambda or class defined in the analyzed source.
    Def(UnitId),
    Import(Imported),
    /// A module-level value, such as `app = FastAPI()`.
    Variable,
    /// A parameter or local value whose contents are unknown.
    Local,
    /// The first parameter of an instance method.
    SelfOf(UnitId),
    /// The first parameter of a class method.
    ClsOf(UnitId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeKind {
    Module,
    Class,
    Function,
}

#[derive(Debug)]
pub(crate) struct Scope {
    pub(crate) kind: ScopeKind,
    pub(crate) parent: Option<ScopeId>,
    /// The module, class, function or lambda that owns the scope.
    pub(crate) unit: UnitId,
    pub(crate) file: usize,
    pub(crate) bindings: HashMap<String, Binding>,
    /// Names declared `global` or `nonlocal`, which assignments do not bind here.
    outer_names: HashSet<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Class {
    pub(crate) methods: HashMap<String, UnitId>,
}

/// Code that runs as one invocable unit.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Code<'t> {
    /// A module's top-level statements.
    Module(Node<'t>),
    /// A function body.
    Block(Node<'t>),
    /// A lambda body.
    Expression(Node<'t>),
}

#[derive(Debug)]
pub(crate) struct Executable<'t> {
    pub(crate) unit: UnitId,
    pub(crate) scope: ScopeId,
    pub(crate) code: Code<'t>,
}

/// A module-level assignment, which becomes a channel unit if things are registered on
/// it.
#[derive(Debug)]
pub(crate) struct Variable {
    pub(crate) name: String,
    pub(crate) module: UnitId,
    pub(crate) location: Location,
    /// The callee that produced the value, e.g. `FastAPI`.
    pub(crate) constructor: Option<String>,
}

/// A class base expression, resolved once every module is indexed.
#[derive(Debug)]
pub(crate) struct PendingBase<'t> {
    pub(crate) class: UnitId,
    pub(crate) scope: ScopeId,
    pub(crate) node: Node<'t>,
}

#[derive(Debug)]
pub(crate) struct Index<'t> {
    pub(crate) files: &'t [Parsed],
    /// Units without calls or bodies.
    pub(crate) units: Vec<Unit>,
    pub(crate) scopes: Vec<Scope>,
    /// Module name → its scope.
    pub(crate) modules: HashMap<String, ScopeId>,
    pub(crate) classes: HashMap<UnitId, Class>,
    /// Method name → every method with that name, for name-based inference.
    pub(crate) methods: BTreeMap<String, Vec<UnitId>>,
    pub(crate) executables: Vec<Executable<'t>>,
    /// `(file, node id)` of a definition or lambda → its unit.
    pub(crate) definitions: HashMap<(usize, usize), UnitId>,
    /// `(file, node id)` of a class definition → its body scope.
    pub(crate) class_scopes: HashMap<(usize, usize), ScopeId>,
    pub(crate) function_scopes: HashMap<UnitId, ScopeId>,
    /// Names each function returns as a bare identifier, for finding decorator wrappers.
    pub(crate) returns: HashMap<UnitId, Vec<String>>,
    /// Channel id (`python:<module>.<name>`) → the module-level variable.
    pub(crate) variables: HashMap<UnitId, Variable>,
    pub(crate) bases: Vec<PendingBase<'t>>,
    lambda_counts: HashMap<ScopeId, usize>,
}

/// Where a definition is being visited.
#[derive(Debug, Clone)]
struct Ctx {
    file: usize,
    scope: ScopeId,
    /// Qualified-name prefix for definitions here: `shop.x`, `shop.x.C`, `shop.x.f.<locals>`.
    qual: String,
    parent: UnitId,
    /// The innermost enclosing `def`, whose `return` statements are recorded.
    function: Option<UnitId>,
}

/// The id of the synthetic unit that runs a module's top-level statements.
pub(crate) fn module_body(module: &str) -> UnitId {
    UnitId::from_parts(SCHEME, &format!("{module}.<module>"))
}

fn id(qual: &str, name: &str) -> UnitId {
    UnitId::from_parts(SCHEME, &format!("{qual}.{name}"))
}

impl<'t> Index<'t> {
    pub(crate) fn build(files: &'t [Parsed]) -> Self {
        let mut index = Self {
            files,
            units: Vec::new(),
            scopes: Vec::new(),
            modules: HashMap::new(),
            classes: HashMap::new(),
            methods: BTreeMap::new(),
            executables: Vec::new(),
            definitions: HashMap::new(),
            class_scopes: HashMap::new(),
            function_scopes: HashMap::new(),
            returns: HashMap::new(),
            variables: HashMap::new(),
            bases: Vec::new(),
            lambda_counts: HashMap::new(),
        };
        for file in 0..files.len() {
            index.module(file);
        }
        index.link_packages();
        index
    }

    pub(crate) fn parsed(&self, file: usize) -> &'t Parsed {
        let files = self.files;
        &files[file]
    }

    /// The dotted name of the module a scope belongs to.
    pub(crate) fn module_of(&self, scope: ScopeId) -> &'t str {
        &self.parsed(self.scopes[scope].file).file.module
    }

    pub(crate) fn is_class(&self, unit: &UnitId) -> bool {
        self.classes.contains_key(unit)
    }

    fn module(&mut self, file: usize) {
        let parsed = self.parsed(file);
        let root = parsed.tree.root_node();
        let module = parsed.file.module.as_str();
        let unit_id = UnitId::from_parts(SCHEME, module);
        let kind = if parsed.file.is_package {
            "package"
        } else {
            "module"
        };

        let mut unit = Unit::new(unit_id.clone(), module, kind, Role::Group);
        unit.location = Some(Location {
            uri: parsed.file.uri.clone(),
            range: None,
        });
        unit.content_hash = Some(content_hash(root, root, &parsed.source));
        unit.doc = docstring(root, &parsed.source);
        self.units.push(unit);

        let scope = self.new_scope(ScopeKind::Module, None, unit_id.clone(), file);
        self.modules.insert(module.to_owned(), scope);
        self.executables.push(Executable {
            unit: module_body(module),
            scope,
            code: Code::Module(root),
        });
        let ctx = Ctx {
            file,
            scope,
            qual: module.to_owned(),
            parent: unit_id,
            function: None,
        };
        self.walk(root, &ctx);
    }

    /// A module's parent is the nearest enclosing package that was analyzed.
    fn link_packages(&mut self) {
        let modules = &self.modules;
        for unit in &mut self.units {
            if !matches!(unit.kind.as_str(), "module" | "package") {
                continue;
            }
            let mut name = unit.name.as_str();
            while let Some((package, _)) = name.rsplit_once('.') {
                if modules.contains_key(package) {
                    unit.parent = Some(UnitId::from_parts(SCHEME, package));
                    break;
                }
                name = package;
            }
        }
    }

    fn new_scope(
        &mut self,
        kind: ScopeKind,
        parent: Option<ScopeId>,
        unit: UnitId,
        file: usize,
    ) -> ScopeId {
        self.scopes.push(Scope {
            kind,
            parent,
            unit,
            file,
            bindings: HashMap::new(),
            outer_names: HashSet::new(),
        });
        self.scopes.len() - 1
    }

    /// Binds `name`, except that an unknown value never shadows a known definition or
    /// import (`f = wrap(f)` keeps `f` resolvable).
    fn bind(&mut self, scope: ScopeId, name: &str, binding: Binding) {
        let scope = &mut self.scopes[scope];
        if scope.outer_names.contains(name) {
            return;
        }
        let weak = matches!(binding, Binding::Local | Binding::Variable);
        let known = scope
            .bindings
            .get(name)
            .is_some_and(|existing| !matches!(existing, Binding::Local | Binding::Variable));
        if !(weak && known) {
            scope.bindings.insert(name.to_owned(), binding);
        }
    }

    fn walk(&mut self, node: Node<'t>, ctx: &Ctx) {
        for child in named_children(node) {
            self.visit(child, ctx);
        }
    }

    fn visit(&mut self, node: Node<'t>, ctx: &Ctx) {
        match node.kind() {
            "function_definition" => self.function(node, node, ctx),
            "class_definition" => self.class(node, node, ctx),
            "decorated_definition" => {
                for decorator in named_children(node) {
                    if decorator.kind() == "decorator" {
                        self.walk(decorator, ctx);
                    }
                }
                match field(node, "definition") {
                    Some(def) if def.kind() == "function_definition" => {
                        self.function(node, def, ctx);
                    }
                    Some(def) if def.kind() == "class_definition" => self.class(node, def, ctx),
                    _ => {}
                }
            }
            "lambda" => {
                self.lambda(node, ctx);
            }
            "import_statement" | "import_from_statement" => {
                let file = &self.parsed(ctx.file).file;
                let importer = Importer {
                    module: &file.module,
                    is_package: file.is_package,
                };
                let source = &self.parsed(ctx.file).source;
                for (name, imported) in imports::parse(node, source, importer).bindings {
                    self.bind(ctx.scope, &name, Binding::Import(imported));
                }
            }
            "assignment" => self.assignment(node, ctx),
            "for_statement" | "for_in_clause" => {
                if let Some(left) = field(node, "left") {
                    self.bind_targets(left, ctx, None);
                }
                self.walk(node, ctx);
            }
            "global_statement" | "nonlocal_statement" => {
                let source = &self.parsed(ctx.file).source;
                for name in named_children(node) {
                    self.scopes[ctx.scope]
                        .outer_names
                        .insert(text(name, source).to_owned());
                }
            }
            "return_statement" => {
                let returned = named_children(node);
                if let (Some(function), [value]) = (&ctx.function, returned.as_slice())
                    && value.kind() == "identifier"
                {
                    let name = text(*value, &self.parsed(ctx.file).source).to_owned();
                    self.returns.entry(function.clone()).or_default().push(name);
                }
                self.walk(node, ctx);
            }
            _ => self.walk(node, ctx),
        }
    }

    /// `outer` is the decorated definition if there is one, else `def` itself.
    fn function(&mut self, outer: Node<'t>, def: Node<'t>, ctx: &Ctx) {
        let source = &self.parsed(ctx.file).source;
        let (Some(name), Some(body)) = (field(def, "name"), field(def, "body")) else {
            return;
        };
        let name = text(name, source);
        let unit_id = id(&ctx.qual, name);
        let enclosing = self.scopes[ctx.scope].kind;
        let is_async = def.child(0).is_some_and(|token| token.kind() == "async");
        let decorators = decorator_names(outer, source);
        let binding_kind = ["classmethod", "staticmethod"]
            .into_iter()
            .find(|marker| decorators.iter().any(|name| name == marker));

        let kind = match enclosing {
            ScopeKind::Class => "method",
            ScopeKind::Function => "closure",
            ScopeKind::Module if is_async => "async_fn",
            ScopeKind::Module => "fn",
        };
        let mut unit = Unit::new(unit_id.clone(), name, kind, Role::Invocable);
        unit.parent = Some(ctx.parent.clone());
        unit.location = Some(location(def, &self.parsed(ctx.file).file.uri));
        unit.content_hash = Some(content_hash(outer, def, source));
        unit.doc = docstring(body, source);
        if is_async {
            unit.attributes
                .insert("is_async".into(), Scalar::Bool(true));
        }
        if let Some(binding_kind) = binding_kind {
            unit.attributes
                .insert("binding".into(), binding_kind.into());
        }
        if !decorators.is_empty() {
            unit.attributes
                .insert("decorators".into(), decorators.join(", ").into());
        }
        if name.starts_with('_') && !name.ends_with("__") {
            unit.attributes
                .insert("visibility".into(), "private".into());
        }
        self.units.push(unit);

        self.bind(ctx.scope, name, Binding::Def(unit_id.clone()));
        let owner = (enclosing == ScopeKind::Class).then(|| self.scopes[ctx.scope].unit.clone());
        if let Some(class) = &owner {
            if let Some(class) = self.classes.get_mut(class) {
                class.methods.insert(name.to_owned(), unit_id.clone());
            }
            self.methods
                .entry(name.to_owned())
                .or_default()
                .push(unit_id.clone());
        }

        let scope = self.new_scope(
            ScopeKind::Function,
            Some(ctx.scope),
            unit_id.clone(),
            ctx.file,
        );
        let parameters = field(def, "parameters")
            .map(|parameters| parameter_names(parameters, source))
            .unwrap_or_default();
        for (position, parameter) in parameters.into_iter().enumerate() {
            let binding = match (&owner, binding_kind, position) {
                (Some(class), None, 0) => Binding::SelfOf(class.clone()),
                (Some(class), Some("classmethod"), 0) => Binding::ClsOf(class.clone()),
                _ => Binding::Local,
            };
            self.bind(scope, parameter, binding);
        }

        self.definitions
            .insert((ctx.file, def.id()), unit_id.clone());
        self.function_scopes.insert(unit_id.clone(), scope);
        self.executables.push(Executable {
            unit: unit_id.clone(),
            scope,
            code: Code::Block(body),
        });
        let inner = Ctx {
            file: ctx.file,
            scope,
            qual: format!("{}.{name}.<locals>", ctx.qual),
            parent: unit_id.clone(),
            function: Some(unit_id),
        };
        self.walk(body, &inner);
    }

    fn class(&mut self, outer: Node<'t>, def: Node<'t>, ctx: &Ctx) {
        let source = &self.parsed(ctx.file).source;
        let (Some(name), Some(body)) = (field(def, "name"), field(def, "body")) else {
            return;
        };
        let name = text(name, source);
        let unit_id = id(&ctx.qual, name);

        let mut unit = Unit::new(unit_id.clone(), name, "class", Role::Group);
        unit.parent = Some(ctx.parent.clone());
        unit.location = Some(location(def, &self.parsed(ctx.file).file.uri));
        unit.content_hash = Some(content_hash(outer, def, source));
        unit.doc = docstring(body, source);
        let decorators = decorator_names(outer, source);
        if !decorators.is_empty() {
            unit.attributes
                .insert("decorators".into(), decorators.join(", ").into());
        }
        self.units.push(unit);

        self.bind(ctx.scope, name, Binding::Def(unit_id.clone()));
        self.classes.insert(unit_id.clone(), Class::default());
        self.definitions
            .insert((ctx.file, def.id()), unit_id.clone());
        if let Some(superclasses) = field(def, "superclasses") {
            for base in named_children(superclasses) {
                if base.kind() != "keyword_argument" {
                    self.bases.push(PendingBase {
                        class: unit_id.clone(),
                        scope: ctx.scope,
                        node: base,
                    });
                }
            }
        }

        let scope = self.new_scope(ScopeKind::Class, Some(ctx.scope), unit_id.clone(), ctx.file);
        self.class_scopes.insert((ctx.file, def.id()), scope);
        let inner = Ctx {
            file: ctx.file,
            scope,
            qual: format!("{}.{name}", ctx.qual),
            parent: unit_id,
            function: None,
        };
        self.walk(body, &inner);
    }

    /// Lambdas are numbered in source order within their enclosing scope:
    /// `f.<locals>.<lambda#0>`.
    fn lambda(&mut self, node: Node<'t>, ctx: &Ctx) -> UnitId {
        let count = self.lambda_counts.entry(ctx.scope).or_default();
        let name = format!("<lambda#{count}>");
        *count += 1;
        let unit_id = id(&ctx.qual, &name);
        let source = &self.parsed(ctx.file).source;

        let mut unit = Unit::new(unit_id.clone(), &name, "closure", Role::Invocable);
        unit.parent = Some(ctx.parent.clone());
        unit.location = Some(location(node, &self.parsed(ctx.file).file.uri));
        unit.content_hash = Some(content_hash(node, node, source));
        unit.attributes.insert("lambda".into(), Scalar::Bool(true));
        self.units.push(unit);

        let scope = self.new_scope(
            ScopeKind::Function,
            Some(ctx.scope),
            unit_id.clone(),
            ctx.file,
        );
        if let Some(parameters) = field(node, "parameters") {
            for parameter in parameter_names(parameters, source) {
                self.bind(scope, parameter, Binding::Local);
            }
        }
        self.definitions
            .insert((ctx.file, node.id()), unit_id.clone());
        if let Some(body) = field(node, "body") {
            self.executables.push(Executable {
                unit: unit_id.clone(),
                scope,
                code: Code::Expression(body),
            });
            let inner = Ctx {
                file: ctx.file,
                scope,
                qual: format!("{}.{name}.<locals>", ctx.qual),
                parent: unit_id.clone(),
                function: None,
            };
            self.visit(body, &inner);
        }
        unit_id
    }

    fn assignment(&mut self, node: Node<'t>, ctx: &Ctx) {
        let Some(left) = field(node, "left") else {
            return;
        };
        match field(node, "right") {
            Some(right) if right.kind() == "lambda" => {
                let lambda = self.lambda(right, ctx);
                self.bind_targets(left, ctx, Some(&Binding::Def(lambda)));
            }
            right => {
                if let Some(right) = right {
                    self.visit(right, ctx);
                }
                let constructor = right
                    .filter(|right| right.kind() == "call")
                    .and_then(|call| field(call, "function"))
                    .map(|callee| compact(callee, &self.parsed(ctx.file).source));
                self.bind_targets(left, ctx, None);
                if self.scopes[ctx.scope].kind == ScopeKind::Module {
                    self.record_variables(left, node, ctx, constructor.as_deref());
                }
            }
        }
    }

    /// Binds assignment or loop targets: to `binding` if given, else to an unknown value.
    fn bind_targets(&mut self, left: Node<'t>, ctx: &Ctx, binding: Option<&Binding>) {
        let source = &self.parsed(ctx.file).source;
        let mut names = Vec::new();
        target_names(left, source, &mut names);
        let fallback = if self.scopes[ctx.scope].kind == ScopeKind::Module {
            Binding::Variable
        } else {
            Binding::Local
        };
        for name in names {
            self.bind(ctx.scope, name, binding.unwrap_or(&fallback).clone());
        }
    }

    fn record_variables(
        &mut self,
        left: Node<'t>,
        assignment: Node<'t>,
        ctx: &Ctx,
        constructor: Option<&str>,
    ) {
        let parsed = self.parsed(ctx.file);
        let mut names = Vec::new();
        target_names(left, &parsed.source, &mut names);
        for name in names {
            self.variables
                .entry(id(&ctx.qual, name))
                .or_insert_with(|| Variable {
                    name: name.to_owned(),
                    module: ctx.parent.clone(),
                    location: location(assignment, &parsed.file.uri),
                    constructor: constructor.map(str::to_owned),
                });
        }
    }
}

/// The callee of each decorator, as written: `router.post`, `audited`, `classmethod`.
fn decorator_names(outer: Node<'_>, source: &str) -> Vec<String> {
    named_children(outer)
        .into_iter()
        .filter(|child| child.kind() == "decorator")
        .filter_map(|decorator| decorator.named_child(0))
        .map(|expression| {
            let callee = if expression.kind() == "call" {
                field(expression, "function").unwrap_or(expression)
            } else {
                expression
            };
            compact(callee, source)
        })
        .collect()
}
