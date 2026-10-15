//! Small tree-sitter helpers shared by the extraction passes.

use std::path::Path;

use codeviz_ir::{Location, Range};
use tree_sitter::Node;

use crate::extract::Scope;

pub(crate) fn text<'s>(node: Node<'_>, source: &'s [u8]) -> &'s str {
    node.utf8_text(source).unwrap_or_default()
}

/// Collapses whitespace and truncates, for guard and arm labels.
pub(crate) fn label(node: Node<'_>, source: &[u8]) -> String {
    const MAX: usize = 80;
    let collapsed = text(node, source)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    match collapsed.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{}…", &collapsed[..cut]),
        None => collapsed,
    }
}

pub(crate) fn location(path: &Path, node: Node<'_>) -> Location {
    let (start, end) = (node.start_position(), node.end_position());
    let to_u32 = |value: usize| u32::try_from(value + 1).unwrap_or(u32::MAX);
    Location {
        uri: path
            .iter()
            .map(|segment| segment.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        range: Some(Range {
            start_line: to_u32(start.row),
            start_col: to_u32(start.column),
            end_line: to_u32(end.row),
            end_col: to_u32(end.column),
        }),
    }
}

/// `a::b::c`, `Self::new`, `Vec::<u8>::new` → path segments. `None` for anything else.
pub(crate) fn path_segments(node: Node<'_>, source: &[u8]) -> Option<Vec<String>> {
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
        "generic_type" | "generic_function" => path_segments(
            node.child_by_field_name("type")
                .or_else(|| node.child_by_field_name("function"))?,
            source,
        ),
        _ => None,
    }
}

/// Tuple structs and enum variants look like calls; their last segment is capitalized.
pub(crate) fn is_constructor(segments: &[String]) -> bool {
    segments
        .last()
        .and_then(|s| s.chars().next())
        .is_some_and(char::is_uppercase)
}

pub(crate) fn type_name(node: Node<'_>, source: &[u8]) -> String {
    match node.kind() {
        "generic_type" | "reference_type" => node
            .child_by_field_name("type")
            .map_or_else(|| text(node, source).to_owned(), |ty| type_name(ty, source)),
        "scoped_type_identifier" => node.child_by_field_name("name").map_or_else(
            || text(node, source).to_owned(),
            |name| text(name, source).to_owned(),
        ),
        _ => text(node, source).to_owned(),
    }
}

/// Attributes and doc comments sit before an item as siblings.
fn preceding(node: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    std::iter::successors(node.prev_named_sibling(), Node::prev_named_sibling).take_while(|s| {
        matches!(
            s.kind(),
            "attribute_item" | "line_comment" | "block_comment"
        )
    })
}

pub(crate) fn is_test_only(node: Node<'_>, source: &[u8]) -> bool {
    preceding(node).any(|sibling| {
        let attribute = text(sibling, source);
        sibling.kind() == "attribute_item"
            && (attribute.contains("cfg(test)") || attribute.ends_with("test]"))
    })
}

/// The `///` doc comment directly above an item.
pub(crate) fn doc_comment(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut lines: Vec<&str> = preceding(node)
        .filter(|s| s.kind() == "line_comment")
        .filter_map(|s| text(s, source).strip_prefix("///"))
        .map(str::trim)
        .collect();
    lines.reverse();
    (!lines.is_empty()).then(|| lines.join(" "))
}

pub(crate) fn collect_use(node: Node<'_>, source: &[u8], prefix: &[String], scope: &mut Scope) {
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
