//! Syntax-only pass: items, `use` imports, call sites and control flow, with no
//! cross-file knowledge. Resolution happens in [`crate::resolve`].

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
};

use codeviz_ir::{CallKey, ContentHash, Location, Mode, Role, Scalar, Step};
use tree_sitter::{Node, Parser};

use crate::{
    Error, body,
    hash::content_hash,
    syntax::{
        collect_use, compact, doc_comment, is_test_only, location, receiver_type_path, start, text,
        type_name, unwrapped_type_path,
    },
};

/// The type (and trait) a method is implemented on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Owner {
    pub(crate) type_name: String,
    pub(crate) trait_name: Option<String>,
    /// The trait as written, with generic arguments, for ids: `From<&str>`.
    pub(crate) trait_text: Option<String>,
}

/// Field name → field type path as written.
pub(crate) type Fields = HashMap<String, Vec<String>>;

/// What a method is called on, as far as declarations reveal it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Receiver {
    pub(crate) root: ReceiverRoot,
    /// Field accesses after the root: `self.inner.users` → `["inner", "users"]`.
    pub(crate) fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReceiverRoot {
    SelfValue,
    /// A binding whose type was written down (`declared`) or guessed from a
    /// `Type::constructor(..)` initializer.
    Typed {
        path: Vec<String>,
        declared: bool,
    },
    /// The value returned by an earlier call; `tried` when `?` unwrapped it. The call
    /// is in the unit `up` closures out from this one: 0 for its own calls.
    Returned {
        call: usize,
        tried: bool,
        up: usize,
    },
}

impl ReceiverRoot {
    /// The same value, seen from a closure written where it is bound.
    pub(crate) fn captured(&self) -> Self {
        match self {
            Self::Returned { call, tried, up } => Self::Returned {
                call: *call,
                tried: *tried,
                up: up + 1,
            },
            other => other.clone(),
        }
    }
}

/// A call target as written, before resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallTarget {
    /// `f(..)`, `module::f(..)`, `Type::f(..)`, `Self::f(..)`.
    Path(Vec<String>),
    /// `receiver.name(..)`.
    Method {
        name: String,
        receiver: Option<Receiver>,
    },
    /// A function passed by name, e.g. `get(handlers::health)`. Kept only if it resolves.
    Reference(Vec<String>),
    /// A closure defined at the call site, by its unit path.
    Closure(Vec<String>),
}

#[derive(Debug)]
pub(crate) struct RawCall {
    pub(crate) key: CallKey,
    pub(crate) text: String,
    pub(crate) target: CallTarget,
    pub(crate) mode: Mode,
    /// For registrations: index of the call the target was passed to.
    pub(crate) via_call: Option<usize>,
    pub(crate) propagates: bool,
    pub(crate) location: Location,
    /// Where the name of what it calls is written, as [`crate::syntax::start`] gives it.
    pub(crate) name_at: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnitKind {
    Crate,
    Module,
    Type(&'static str),
    Function { is_async: bool, is_method: bool },
    Closure,
}

impl UnitKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Crate => "crate",
            Self::Module => "module",
            Self::Type(kind) => kind,
            Self::Function {
                is_method: true, ..
            } => "method",
            Self::Function { is_async: true, .. } => "async_fn",
            Self::Function { .. } => "fn",
            Self::Closure => "closure",
        }
    }

    pub(crate) fn role(self) -> Role {
        match self {
            Self::Crate | Self::Module | Self::Type(_) => Role::Group,
            Self::Function { .. } | Self::Closure => Role::Invocable,
        }
    }
}

#[derive(Debug)]
pub(crate) struct RawUnit {
    /// Id path, e.g. `["shop", "models", "User", "{impl#Debug}", "fmt"]`.
    pub(crate) path: Vec<String>,
    pub(crate) parent: Option<Vec<String>>,
    pub(crate) name: String,
    pub(crate) kind: UnitKind,
    /// The module whose scope resolves this unit's paths.
    pub(crate) module: Vec<String>,
    pub(crate) owner: Option<Owner>,
    pub(crate) location: Option<Location>,
    /// Where a function's name is written, as [`crate::syntax::start`] gives it.
    pub(crate) name_at: Option<(u32, u32)>,
    pub(crate) content_hash: Option<ContentHash>,
    pub(crate) doc: Option<String>,
    pub(crate) attributes: BTreeMap<String, Scalar>,
    pub(crate) calls: Vec<RawCall>,
    pub(crate) body: Option<Vec<Step>>,
    /// The declared return type, and the `T` of a returned `Result<T, _>` or `Option<T>`.
    pub(crate) returns: Option<Vec<String>>,
    pub(crate) returns_ok: Option<Vec<String>>,
}

impl RawUnit {
    fn group(path: Vec<String>, kind: UnitKind, module: Vec<String>) -> Self {
        let parent = path
            .split_last()
            .map(|(_, rest)| rest.to_vec())
            .filter(|p| !p.is_empty());
        Self {
            name: path.last().cloned().unwrap_or_default(),
            path,
            parent,
            kind,
            module,
            owner: None,
            location: None,
            name_at: None,
            content_hash: None,
            doc: None,
            attributes: BTreeMap::new(),
            calls: Vec::new(),
            body: None,
            returns: None,
            returns_ok: None,
        }
    }

    /// The path calls are resolved against: `module::[Type::]name`, without impl segments.
    pub(crate) fn lookup_path(&self) -> Vec<String> {
        let mut path = self.module.clone();
        if let Some(owner) = &self.owner {
            path.push(owner.type_name.clone());
        }
        path.push(self.name.clone());
        path
    }
}

/// An Axum-style `.route("/path", get(handler))` registration.
#[derive(Debug)]
pub(crate) struct RouteHint {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) handler: Vec<String>,
    pub(crate) module: Vec<String>,
}

/// Names a module brings into scope with `use`. Paths are as written.
#[derive(Debug, Default)]
pub(crate) struct Scope {
    pub(crate) imports: HashMap<String, Vec<String>>,
    pub(crate) globs: Vec<Vec<String>>,
}

#[derive(Debug, Default)]
pub(crate) struct Extraction {
    pub(crate) units: Vec<RawUnit>,
    pub(crate) scopes: HashMap<Vec<String>, Scope>,
    /// Names of traits declared in the analyzed source.
    pub(crate) traits: HashSet<String>,
    pub(crate) routes: Vec<RouteHint>,
    /// Struct path → its fields.
    pub(crate) fields: HashMap<Vec<String>, Fields>,
    /// Group path to its unit's index.
    groups: HashMap<Vec<String>, usize>,
}

pub(crate) struct Extractor {
    parser: Parser,
    include_tests: bool,
    pub(crate) out: Extraction,
}

pub(crate) struct File<'a> {
    pub(crate) source: &'a [u8],
    pub(crate) path: &'a Path,
}

impl Extractor {
    pub(crate) fn new(include_tests: bool) -> Result<Self, Error> {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
        Ok(Self {
            parser,
            include_tests,
            out: Extraction::default(),
        })
    }

    /// Extracts one file whose module path (including the crate name) is `module`.
    pub(crate) fn extract(
        &mut self,
        source: &[u8],
        path: &Path,
        module: &[String],
    ) -> Result<(), Error> {
        let tree = self
            .parser
            .parse(source, None)
            .ok_or_else(|| Error::Parse(path.to_path_buf()))?;
        let file = File { source, path };
        for depth in 1..=module.len() {
            let kind = if depth == 1 {
                UnitKind::Crate
            } else {
                UnitKind::Module
            };
            self.group(module.get(..depth).unwrap_or_default(), kind, module);
        }
        self.out.scopes.entry(module.to_vec()).or_default();
        self.items(&file, tree.root_node(), module, None);
        Ok(())
    }

    pub(crate) fn finish(self) -> Extraction {
        self.out
    }

    fn group(&mut self, path: &[String], kind: UnitKind, module: &[String]) {
        if !self.out.groups.contains_key(path) {
            self.out.groups.insert(path.to_vec(), self.out.units.len());
            self.out
                .units
                .push(RawUnit::group(path.to_vec(), kind, module.to_vec()));
        }
    }

    fn items(
        &mut self,
        file: &File<'_>,
        parent: Node<'_>,
        module: &[String],
        owner: Option<&Owner>,
    ) {
        let mut cursor = parent.walk();
        for child in parent.named_children(&mut cursor) {
            match child.kind() {
                "macro_invocation" => self.macro_items(file, child, module, owner),
                "expression_statement"
                    if child
                        .named_child(0)
                        .is_some_and(|c| c.kind() == "macro_invocation") =>
                {
                    if let Some(invocation) = child.named_child(0) {
                        self.macro_items(file, invocation, module, owner);
                    }
                }
                _ => {
                    self.item(file, child, module, owner);
                }
            }
        }
    }

    /// Items written inside a macro invocation where items go, such as tokio's
    /// `cfg_rt! { pub mod runtime; .. }` or `cfg_if! { if #[cfg(unix)] { .. } }`:
    /// tree-sitter leaves them as tokens, so the tokens are parsed again as items, in
    /// place. Whatever does not parse as an item is looked through for items inside.
    fn macro_items(
        &mut self,
        file: &File<'_>,
        invocation: Node<'_>,
        module: &[String],
        owner: Option<&Owner>,
    ) {
        let Some(tree) = invocation
            .named_children(&mut invocation.walk())
            .find(|c| c.kind() == "token_tree")
        else {
            return;
        };
        let (Some(open), Some(close)) = (
            tree.child(0),
            tree.child(tree.child_count().saturating_sub(1)),
        ) else {
            return;
        };
        if open.end_byte() >= close.start_byte() {
            return;
        }
        let inside = tree_sitter::Range {
            start_byte: open.end_byte(),
            end_byte: close.start_byte(),
            start_point: open.end_position(),
            end_point: close.start_position(),
        };
        if self.parser.set_included_ranges(&[inside]).is_err() {
            return;
        }
        let parsed = self.parser.parse(file.source, None);
        let _ = self.parser.set_included_ranges(&[]);
        if let Some(parsed) = parsed {
            self.expanded_items(file, parsed.root_node(), module, owner);
        }
    }

    fn expanded_items(
        &mut self,
        file: &File<'_>,
        node: Node<'_>,
        module: &[String],
        owner: Option<&Owner>,
    ) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "macro_invocation" {
                self.macro_items(file, child, module, owner);
            } else if !self.item(file, child, module, owner) {
                self.expanded_items(file, child, module, owner);
            }
        }
    }

    /// Handles `node` if it is an item. Returns whether it was.
    pub(crate) fn item(
        &mut self,
        file: &File<'_>,
        node: Node<'_>,
        module: &[String],
        owner: Option<&Owner>,
    ) -> bool {
        let source = file.source;
        match node.kind() {
            "function_item" => {
                if self.include_tests || !is_test_only(node, source) {
                    self.function(file, node, module, owner);
                }
            }
            "struct_item" | "enum_item" | "union_item" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let kind = node.kind().trim_end_matches("_item");
                    let kind = match kind {
                        "struct" => "struct",
                        "enum" => "enum",
                        _ => "union",
                    };
                    self.type_unit(module, text(name, source), kind);
                    if kind == "struct" {
                        self.struct_fields(module, text(name, source), node, source);
                    }
                }
            }
            "impl_item" => {
                let trait_node = node.child_by_field_name("trait");
                let owner = node.child_by_field_name("type").map(|ty| Owner {
                    type_name: compact(&type_name(ty, source)),
                    trait_name: trait_node.map(|tr| type_name(tr, source)),
                    trait_text: trait_node.map(|tr| trait_label(tr, source)),
                });
                if let (Some(owner), Some(body)) = (owner, node.child_by_field_name("body")) {
                    self.type_unit(module, &owner.type_name, "type");
                    self.items(file, body, module, Some(&owner));
                }
            }
            "trait_item" => {
                let Some(name) = node.child_by_field_name("name") else {
                    return true;
                };
                let name = text(name, source).to_owned();
                self.out.traits.insert(name.clone());
                self.type_unit(module, &name, "trait");
                let owner = Owner {
                    type_name: name.clone(),
                    trait_name: Some(name.clone()),
                    trait_text: Some(name),
                };
                if let Some(body) = node.child_by_field_name("body") {
                    self.items(file, body, module, Some(&owner));
                }
            }
            "mod_item" => {
                if !self.include_tests && is_test_only(node, source) {
                    return true;
                }
                if let (Some(name), Some(body)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("body"),
                ) {
                    let mut inner = module.to_vec();
                    inner.push(text(name, source).to_owned());
                    self.group(&inner, UnitKind::Module, &inner);
                    self.out.scopes.entry(inner.clone()).or_default();
                    self.items(file, body, &inner, None);
                }
            }
            "use_declaration" => {
                if let Some(argument) = node.child_by_field_name("argument") {
                    let scope = self.out.scopes.entry(module.to_vec()).or_default();
                    collect_use(argument, source, &[], scope);
                }
            }
            _ => return false,
        }
        true
    }

    fn type_unit(&mut self, module: &[String], name: &str, kind: &'static str) {
        let mut path = module.to_vec();
        path.push(name.to_owned());
        if let Some(existing) = self
            .out
            .groups
            .get(&path)
            .and_then(|&index| self.out.units.get_mut(index))
        {
            // A struct definition is more specific than an impl seen first.
            if existing.kind == UnitKind::Type("type") {
                existing.kind = UnitKind::Type(kind);
            }
            return;
        }
        self.group(&path, UnitKind::Type(kind), module);
    }

    fn function(
        &mut self,
        file: &File<'_>,
        node: Node<'_>,
        module: &[String],
        owner: Option<&Owner>,
    ) {
        let source = file.source;
        let (Some(name), Some(block)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("body"),
        ) else {
            return;
        };
        let name_at = start(name);
        let name = text(name, source).to_owned();

        let mut parent = module.to_vec();
        if let Some(owner) = owner {
            parent.push(owner.type_name.clone());
        }
        let mut path = parent.clone();
        if let Some(owner) = owner
            && let (Some(trait_name), Some(trait_text)) = (&owner.trait_name, &owner.trait_text)
            && trait_name != &owner.type_name
        {
            path.push(format!("{{impl#{trait_text}}}"));
        }
        path.push(name.clone());

        let header = source
            .get(node.start_byte()..block.start_byte())
            .map(String::from_utf8_lossy)
            .unwrap_or_default();
        let signature = header.split_whitespace().collect::<Vec<_>>().join(" ");
        let is_async = node
            .named_children(&mut node.walk())
            .any(|c| c.kind() == "function_modifiers" && text(c, source).contains("async"));
        let return_type = node.child_by_field_name("return_type");
        let returns_result = return_type.is_some_and(|ty| text(ty, source).contains("Result"));
        let returns = return_type.and_then(|ty| receiver_type_path(ty, source));
        let returns_ok = return_type.and_then(|ty| unwrapped_type_path(ty, source));
        let where_clause = node
            .named_children(&mut node.walk())
            .find(|c| c.kind() == "where_clause");

        let built = body::build(
            self,
            file,
            &body::Context {
                path: &path,
                module,
                owner,
                returns_result,
                parameters: node.child_by_field_name("parameters"),
                generics: [node.child_by_field_name("type_parameters"), where_clause],
                captured: None,
            },
            block,
        );

        let mut attributes = BTreeMap::new();
        attributes.insert("signature".to_owned(), Scalar::from(signature));
        if let Some(trait_name) = owner.and_then(|o| o.trait_name.as_ref()) {
            attributes.insert("trait".to_owned(), Scalar::from(trait_name.as_str()));
        }
        self.out.units.push(RawUnit {
            parent: Some(parent),
            path,
            name,
            kind: UnitKind::Function {
                is_async,
                is_method: owner.is_some(),
            },
            module: module.to_vec(),
            owner: owner.cloned(),
            location: Some(location(file.path, node)),
            name_at: Some(name_at),
            content_hash: Some(content_hash(node, source)),
            doc: doc_comment(node, source),
            attributes,
            calls: built.calls,
            body: Some(built.steps),
            returns,
            returns_ok,
        });
    }

    fn struct_fields(&mut self, module: &[String], name: &str, node: Node<'_>, source: &[u8]) {
        let Some(body) = node.child_by_field_name("body") else {
            return;
        };
        let mut fields = HashMap::new();
        let mut cursor = body.walk();
        let mut position = 0;
        for child in body.named_children(&mut cursor) {
            let (field, ty) = match child.kind() {
                "field_declaration" => (
                    child
                        .child_by_field_name("name")
                        .map(|n| text(n, source).to_owned()),
                    child.child_by_field_name("type"),
                ),
                "visibility_modifier" | "attribute_item" | "line_comment" | "block_comment" => {
                    continue;
                }
                // Tuple struct fields are named by position.
                _ if body.kind() == "ordered_field_declaration_list" => {
                    position += 1;
                    (Some((position - 1).to_string()), Some(child))
                }
                _ => continue,
            };
            if let (Some(field), Some(ty)) = (field, ty.and_then(|t| receiver_type_path(t, source)))
            {
                fields.insert(field, ty);
            }
        }
        let mut path = module.to_vec();
        path.push(name.to_owned());
        self.out.fields.insert(path, fields);
    }

    pub(crate) fn push_unit(&mut self, unit: RawUnit) {
        self.out.units.push(unit);
    }

    pub(crate) fn push_route(&mut self, route: RouteHint) {
        self.out.routes.push(route);
    }
}

/// A trait's own name with its generic arguments: `std::convert::From<&str>` → `From<&str>`.
fn trait_label(node: Node<'_>, source: &[u8]) -> String {
    let arguments = (node.kind() == "generic_type")
        .then(|| node.child_by_field_name("type_arguments"))
        .flatten()
        .map(|args| compact(text(args, source)))
        .unwrap_or_default();
    format!("{}{arguments}", type_name(node, source))
}
