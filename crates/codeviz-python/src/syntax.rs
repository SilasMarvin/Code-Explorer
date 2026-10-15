//! Small helpers over tree-sitter nodes.

use codeviz_ir::{Location, Range};
use tree_sitter::Node;

/// The source text of `node`.
pub(crate) fn text<'s>(node: Node<'_>, source: &'s str) -> &'s str {
    source.get(node.byte_range()).unwrap_or_default()
}

/// `node`'s text with every whitespace run collapsed to one space.
pub(crate) fn collapsed(node: Node<'_>, source: &str) -> String {
    text(node, source)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `node`'s text with all whitespace removed: how a callee such as a multi-line
/// attribute chain is written, as one token.
pub(crate) fn compact(node: Node<'_>, source: &str) -> String {
    text(node, source).split_whitespace().collect()
}

pub(crate) fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    node.named_children(&mut node.walk()).collect()
}

pub(crate) fn field<'t>(node: Node<'t>, name: &str) -> Option<Node<'t>> {
    node.child_by_field_name(name)
}

pub(crate) fn location(node: Node<'_>, uri: &str) -> Location {
    let (start, end) = (node.start_position(), node.end_position());
    Location {
        uri: uri.to_owned(),
        range: Some(Range {
            start_line: one_based(start.row),
            start_col: one_based(start.column),
            end_line: one_based(end.row),
            end_col: one_based(end.column),
        }),
    }
}

fn one_based(zero_based: usize) -> u32 {
    u32::try_from(zero_based + 1).unwrap_or(u32::MAX)
}

/// Whether `node` is a statement consisting of a lone string literal (a docstring or a
/// no-op).
pub(crate) fn is_string_statement(node: Node<'_>) -> bool {
    node.kind() == "expression_statement"
        && node.named_child_count() == 1
        && node
            .named_child(0)
            .is_some_and(|child| child.kind() == "string")
}

/// The docstring of a module, class or function body.
pub(crate) fn docstring(body: Node<'_>, source: &str) -> Option<String> {
    let first = named_children(body)
        .into_iter()
        .find(|child| child.kind() != "comment")?;
    if !is_string_statement(first) {
        return None;
    }
    let literal = string_literal(first.named_child(0)?, source)?;
    let doc = literal.trim();
    (!doc.is_empty()).then(|| doc.to_owned())
}

/// The contents of a plain string literal, without quotes. `None` for f-strings with
/// interpolations and for anything that is not a string.
pub(crate) fn string_literal(node: Node<'_>, source: &str) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let mut contents = String::new();
    for child in named_children(node) {
        match child.kind() {
            "string_start" | "string_end" => {}
            "string_content" => contents.push_str(text(child, source)),
            _ => return None,
        }
    }
    Some(contents)
}

/// The final name of an identifier or attribute chain: `b` for `a.b`.
pub(crate) fn last_name<'s>(node: Node<'_>, source: &'s str) -> Option<&'s str> {
    match node.kind() {
        "identifier" => Some(text(node, source)),
        "attribute" => field(node, "attribute").map(|name| text(name, source)),
        _ => None,
    }
}

/// Identifiers bound by an assignment or loop target (`a`, `a, b`, `(a, [b, *c])`).
pub(crate) fn target_names<'s>(node: Node<'_>, source: &'s str, out: &mut Vec<&'s str>) {
    match node.kind() {
        "identifier" => out.push(text(node, source)),
        "pattern_list" | "tuple_pattern" | "list_pattern" | "list_splat_pattern" => {
            for child in named_children(node) {
                target_names(child, source, out);
            }
        }
        _ => {}
    }
}

/// The names a parameter list binds, in order.
pub(crate) fn parameter_names<'s>(parameters: Node<'_>, source: &'s str) -> Vec<&'s str> {
    named_children(parameters)
        .into_iter()
        .filter_map(|parameter| {
            let name = match parameter.kind() {
                "identifier" => Some(parameter),
                "default_parameter" | "typed_default_parameter" => field(parameter, "name"),
                "typed_parameter" | "list_splat_pattern" | "dictionary_splat_pattern" => {
                    named_children(parameter)
                        .into_iter()
                        .find(|child| child.kind() == "identifier")
                }
                _ => None,
            };
            name.filter(|name| name.kind() == "identifier")
                .map(|name| text(name, source))
        })
        .collect()
}
