//! Syntax-only pass: items, `use` imports and call sites, with no cross-file knowledge.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use codeviz_graph::{Location, Owner};
use tree_sitter::{Node, Parser};

use crate::{AnalyzeOptions, Error};

/// A call target as written, before resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallTarget {
    /// `f`, `module::f`, `Type::f`, `Self::f`.
    Path(Vec<String>),
    /// `receiver.name(..)`.
    Method { name: String, on_self: bool },
}

#[derive(Debug)]
pub(crate) struct RawCall {
    pub(crate) text: String,
    pub(crate) target: CallTarget,
    pub(crate) location: Location,
}

#[derive(Debug)]
pub(crate) struct RawFunction {
    pub(crate) name: String,
    pub(crate) module: Vec<String>,
    pub(crate) owner: Option<Owner>,
    pub(crate) is_async: bool,
    pub(crate) signature: String,
    pub(crate) location: Location,
    pub(crate) end_line: u32,
    pub(crate) calls: Vec<RawCall>,
}

/// Names a module brings into scope with `use`. Paths are as written.
#[derive(Debug, Default)]
pub(crate) struct Scope {
    pub(crate) imports: HashMap<String, Vec<String>>,
    pub(crate) globs: Vec<Vec<String>>,
}

#[derive(Debug, Default)]
pub(crate) struct Extraction {
    pub(crate) functions: Vec<RawFunction>,
    pub(crate) scopes: HashMap<Vec<String>, Scope>,
    /// Names of traits declared in the analyzed source.
    pub(crate) traits: HashSet<String>,
}

pub(crate) struct Extractor {
    parser: Parser,
    options: AnalyzeOptions,
    out: Extraction,
}

struct File<'a> {
    source: &'a [u8],
    path: &'a Path,
}

impl Extractor {
    pub(crate) fn new(options: AnalyzeOptions) -> Result<Self, Error> {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
        Ok(Self {
            parser,
            options,
            out: Extraction::default(),
        })
    }

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
        self.out.scopes.entry(module.to_vec()).or_default();
        self.items(&file, tree.root_node(), module, None);
        Ok(())
    }

    pub(crate) fn finish(self) -> Extraction {
        self.out
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
            self.item(file, child, module, owner);
        }
    }

    /// Handles `node` if it is an item. Returns whether it was.
    fn item(
        &mut self,
        file: &File<'_>,
        node: Node<'_>,
        module: &[String],
        owner: Option<&Owner>,
    ) -> bool {
        match node.kind() {
            "function_item" => {
                if self.options.include_tests || !is_test_only(node, file.source) {
                    self.function(file, node, module, owner);
                }
            }
            "impl_item" => {
                let owner = node.child_by_field_name("type").map(|ty| Owner {
                    type_name: type_name(ty, file.source),
                    trait_name: node
                        .child_by_field_name("trait")
                        .map(|tr| type_name(tr, file.source)),
                });
                if let (Some(owner), Some(body)) = (owner, node.child_by_field_name("body")) {
                    self.items(file, body, module, Some(&owner));
                }
            }
            "trait_item" => {
                let Some(name) = node.child_by_field_name("name") else {
                    return true;
                };
                let name = text(name, file.source).to_owned();
                self.out.traits.insert(name.clone());
                let owner = Owner {
                    type_name: name.clone(),
                    trait_name: Some(name),
                };
                if let Some(body) = node.child_by_field_name("body") {
                    self.items(file, body, module, Some(&owner));
                }
            }
            "mod_item" => {
                if !self.options.include_tests && is_test_only(node, file.source) {
                    return true;
                }
                if let (Some(name), Some(body)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("body"),
                ) {
                    let mut inner = module.to_vec();
                    inner.push(text(name, file.source).to_owned());
                    self.out.scopes.entry(inner.clone()).or_default();
                    self.items(file, body, &inner, None);
                }
            }
            "use_declaration" => {
                if let Some(argument) = node.child_by_field_name("argument") {
                    let scope = self.out.scopes.entry(module.to_vec()).or_default();
                    collect_use(argument, file.source, &[], scope);
                }
            }
            _ => return false,
        }
        true
    }

    fn function(
        &mut self,
        file: &File<'_>,
        node: Node<'_>,
        module: &[String],
        owner: Option<&Owner>,
    ) {
        let (Some(name), Some(body)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("body"),
        ) else {
            return;
        };
        let source = file.source;
        let header = source
            .get(node.start_byte()..body.start_byte())
            .map(String::from_utf8_lossy)
            .unwrap_or_default();
        let is_async = node.named_children(&mut node.walk()).any(|child| {
            child.kind() == "function_modifiers" && text(child, source).contains("async")
        });

        let mut calls = Vec::new();
        self.calls(file, body, module, &mut calls);

        self.out.functions.push(RawFunction {
            name: text(name, source).to_owned(),
            module: module.to_vec(),
            owner: owner.cloned(),
            is_async,
            signature: header.split_whitespace().collect::<Vec<_>>().join(" "),
            location: location(file.path, node),
            end_line: line(node.end_position().row),
            calls,
        });
    }

    /// Collects call sites under `node` in evaluation order: a call's receiver and
    /// arguments are recorded before the call itself.
    ///
    /// Items nested in a body (helper fns, local `use`) are extracted as if they were
    /// declared in the enclosing module.
    fn calls(
        &mut self,
        file: &File<'_>,
        node: Node<'_>,
        module: &[String],
        out: &mut Vec<RawCall>,
    ) {
        if self.item(file, node, module, None) {
            return;
        }
        if node.kind() == "macro_invocation" {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "token_tree" {
                    macro_calls(file, child, out);
                }
            }
            return;
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.calls(file, child, module, out);
        }

        if node.kind() == "call_expression"
            && let Some(function) = node.child_by_field_name("function")
            && let Some((target, written)) = call_target(function, file.source)
            && !is_constructor(&target)
        {
            out.push(RawCall {
                text: written,
                target,
                location: location(file.path, node),
            });
        }
    }
}

fn call_target(function: Node<'_>, source: &[u8]) -> Option<(CallTarget, String)> {
    match function.kind() {
        "generic_function" => call_target(function.child_by_field_name("function")?, source),
        "field_expression" => {
            let name = text(function.child_by_field_name("field")?, source).to_owned();
            let on_self = function
                .child_by_field_name("value")
                .is_some_and(|value| value.kind() == "self");
            let written = format!(".{name}");
            Some((CallTarget::Method { name, on_self }, written))
        }
        _ => {
            let segments = path_segments(function, source)?;
            let written = segments.join("::");
            Some((CallTarget::Path(segments), written))
        }
    }
}

fn path_segments(node: Node<'_>, source: &[u8]) -> Option<Vec<String>> {
    match node.kind() {
        "identifier" | "type_identifier" | "crate" | "self" | "super" => {
            Some(vec![text(node, source).to_owned()])
        }
        "scoped_identifier" | "scoped_type_identifier" => {
            let mut segments = match node.child_by_field_name("path") {
                Some(path) => path_segments(path, source)?,
                None => Vec::new(),
            };
            segments.push(text(node.child_by_field_name("name")?, source).to_owned());
            Some(segments)
        }
        "generic_type" => path_segments(node.child_by_field_name("type")?, source),
        _ => None,
    }
}

/// Tree-sitter leaves macro arguments as raw tokens. Recover `a::b(..)` and
/// `x.m(..)` shapes from them so calls inside `format!`, `vec!` etc. are not lost.
fn macro_calls(file: &File<'_>, tree: Node<'_>, out: &mut Vec<RawCall>) {
    let source = file.source;
    let mut cursor = tree.walk();
    let tokens: Vec<Node<'_>> = tree.children(&mut cursor).collect();
    for (index, token) in tokens.iter().enumerate() {
        if token.kind() != "token_tree" {
            continue;
        }
        macro_calls(file, *token, out);

        let is_paren = text(*token, source).starts_with('(');
        let Some(callee) = index.checked_sub(1).and_then(|i| tokens.get(i)) else {
            continue;
        };
        if !is_paren || callee.kind() != "identifier" {
            continue;
        }
        let name = text(*callee, source).to_owned();
        let before = |offset: usize| index.checked_sub(offset).and_then(|i| tokens.get(i));
        let target = if before(2).is_some_and(|t| t.kind() == ".") {
            let on_self = before(3).is_some_and(|t| t.kind() == "self");
            CallTarget::Method { name, on_self }
        } else {
            let mut segments = vec![name];
            let mut offset = 2;
            while before(offset).is_some_and(|t| t.kind() == "::")
                && let Some(segment) = before(offset + 1)
                && matches!(segment.kind(), "identifier" | "self" | "super" | "crate")
            {
                segments.insert(0, text(*segment, source).to_owned());
                offset += 2;
            }
            CallTarget::Path(segments)
        };
        if is_constructor(&target) {
            continue;
        }
        let written = match &target {
            CallTarget::Path(segments) => segments.join("::"),
            CallTarget::Method { name, .. } => format!(".{name}"),
        };
        out.push(RawCall {
            text: written,
            target,
            location: location(file.path, *callee),
        });
    }
}

fn collect_use(node: Node<'_>, source: &[u8], prefix: &[String], scope: &mut Scope) {
    let joined =
        |segments: Vec<String>| -> Vec<String> { prefix.iter().cloned().chain(segments).collect() };
    match node.kind() {
        "use_as_clause" => {
            if let (Some(path), Some(alias)) = (
                node.child_by_field_name("path")
                    .and_then(|p| path_segments(p, source)),
                node.child_by_field_name("alias"),
            ) {
                scope
                    .imports
                    .insert(text(alias, source).to_owned(), joined(path));
            }
        }
        "scoped_use_list" => {
            let path = node
                .child_by_field_name("path")
                .and_then(|p| path_segments(p, source))
                .unwrap_or_default();
            if let Some(list) = node.child_by_field_name("list") {
                collect_use(list, source, &joined(path), scope);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_use(child, source, prefix, scope);
            }
        }
        "use_wildcard" => {
            let mut cursor = node.walk();
            let path = node
                .named_children(&mut cursor)
                .next()
                .and_then(|p| path_segments(p, source))
                .unwrap_or_default();
            scope.globs.push(joined(path));
        }
        _ => {
            let Some(path) = path_segments(node, source) else {
                return;
            };
            let full = joined(path);
            // `use a::b::{self}` imports `b`.
            let (alias, full) = match full.split_last() {
                Some((last, rest)) if last == "self" => (rest.last().cloned(), rest.to_vec()),
                Some((last, _)) => (Some(last.clone()), full.clone()),
                None => (None, full),
            };
            if let Some(alias) = alias {
                scope.imports.insert(alias, full);
            }
        }
    }
}

fn is_constructor(target: &CallTarget) -> bool {
    match target {
        CallTarget::Path(segments) => segments
            .last()
            .and_then(|s| s.chars().next())
            .is_some_and(char::is_uppercase),
        CallTarget::Method { .. } => false,
    }
}

fn is_test_only(node: Node<'_>, source: &[u8]) -> bool {
    let mut sibling = node.prev_named_sibling();
    while let Some(attribute) = sibling.filter(|s| s.kind() == "attribute_item") {
        let attribute_text = text(attribute, source);
        if attribute_text.contains("cfg(test)") || attribute_text.ends_with("test]") {
            return true;
        }
        sibling = attribute.prev_named_sibling();
    }
    false
}

fn type_name(node: Node<'_>, source: &[u8]) -> String {
    match node.kind() {
        "generic_type" | "reference_type" => node
            .child_by_field_name("type")
            .map_or_else(|| text(node, source).to_owned(), |ty| type_name(ty, source)),
        "scoped_type_identifier" => node.child_by_field_name("name").map_or_else(
            || text(node, source).to_owned(),
            |n| text(n, source).to_owned(),
        ),
        _ => text(node, source).to_owned(),
    }
}

fn text<'s>(node: Node<'_>, source: &'s [u8]) -> &'s str {
    node.utf8_text(source).unwrap_or_default()
}

fn line(row: usize) -> u32 {
    u32::try_from(row + 1).unwrap_or(u32::MAX)
}

fn location(path: &Path, node: Node<'_>) -> Location {
    let start = node.start_position();
    Location {
        file: path.to_path_buf(),
        line: line(start.row),
        column: u32::try_from(start.column + 1).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(source: &str) -> Extraction {
        let mut extractor = Extractor::new(AnalyzeOptions::default()).unwrap();
        extractor
            .extract(
                source.as_bytes(),
                Path::new("src/lib.rs"),
                &["app".to_owned()],
            )
            .unwrap();
        extractor.finish()
    }

    fn call_texts(function: &RawFunction) -> Vec<&str> {
        function.calls.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn calls_are_in_evaluation_order() {
        let out = extract("fn f() { outer(inner(), x.method()).chained(); }");
        assert_eq!(
            call_texts(&out.functions[0]),
            vec!["inner", ".method", "outer", ".chained"]
        );
    }

    #[test]
    fn constructors_are_skipped() {
        let out = extract("fn f() { Ok(Json(Error::Bad(make()))) }");
        assert_eq!(call_texts(&out.functions[0]), vec!["make"]);
    }

    #[test]
    fn calls_inside_macros_are_recovered() {
        let out = extract(r#"fn f() { println!("{} {}", a::b(c()), self.d()); }"#);
        assert_eq!(call_texts(&out.functions[0]), vec!["c", "a::b", ".d"]);
        assert!(matches!(
            &out.functions[0].calls[2].target,
            CallTarget::Method { on_self: true, .. }
        ));
    }

    #[test]
    fn impl_methods_have_owners() {
        let out = extract(
            "struct S; impl S { async fn a(&self) { self.b() } fn b(&self) {} }
             impl std::fmt::Display for S { fn fmt(&self) {} }",
        );
        let owners: Vec<_> = out
            .functions
            .iter()
            .map(|f| {
                (
                    f.name.as_str(),
                    f.owner.clone().map(|o| (o.type_name, o.trait_name)),
                )
            })
            .collect();
        assert_eq!(
            owners,
            vec![
                ("a", Some(("S".into(), None))),
                ("b", Some(("S".into(), None))),
                ("fmt", Some(("S".into(), Some("Display".into())))),
            ]
        );
        assert!(out.functions[0].is_async);
    }

    #[test]
    fn use_trees_are_flattened() {
        let out =
            extract("use crate::{a::{self, b as c}, d::*}; use super::e; mod inner { use x::y; }");
        let root = &out.scopes[&vec!["app".to_owned()]];
        assert_eq!(root.imports["a"], vec!["crate", "a"]);
        assert_eq!(root.imports["c"], vec!["crate", "a", "b"]);
        assert_eq!(root.imports["e"], vec!["super", "e"]);
        assert_eq!(root.globs, vec![vec!["crate".to_owned(), "d".to_owned()]]);
        let inner = &out.scopes[&vec!["app".to_owned(), "inner".to_owned()]];
        assert_eq!(inner.imports["y"], vec!["x", "y"]);
    }

    #[test]
    fn test_code_is_skipped() {
        let out =
            extract("fn real() {} #[cfg(test)] mod tests { fn helper() {} } #[test] fn t() {}");
        let names: Vec<_> = out.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["real"]);
    }
}
