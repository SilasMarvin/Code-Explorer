//! `codeviz-tokens-blake3` content hashes.
//!
//! A unit's hash covers its own tokens: comments, whitespace and string statements
//! (docstrings) are dropped, and each nested definition is reduced to its kind and
//! name so that editing a nested function changes only that function's hash. Python
//! blocks are delimited by indentation rather than tokens, so block boundaries are
//! hashed explicitly.

use codeviz_ir::ContentHash;
use tree_sitter::Node;

use crate::syntax::{field, is_string_statement, text};

const ALGO: &str = "codeviz-tokens-blake3";

/// Hashes `root`, treating `own` (the definition inside a decorated definition, or
/// `root` itself) as part of the unit rather than as a nested definition.
pub(crate) fn content_hash(root: Node<'_>, own: Node<'_>, source: &str) -> ContentHash {
    let mut hasher = blake3::Hasher::new();
    let mut feed = |token: &str| {
        hasher.update(token.as_bytes());
        hasher.update(&[0]);
    };
    tokens(root, [root.id(), own.id()], source, &mut feed);
    ContentHash {
        algo: ALGO.to_owned(),
        value: hasher.finalize().to_hex().to_string(),
    }
}

/// Feeds `node`'s tokens. Definitions other than the unit's own (`unit`) are nested.
fn tokens(node: Node<'_>, unit: [usize; 2], source: &str, feed: &mut impl FnMut(&str)) {
    let nested = !unit.contains(&node.id())
        && matches!(
            node.kind(),
            "decorated_definition" | "function_definition" | "class_definition" | "lambda"
        );
    if nested {
        let definition = field(node, "definition").unwrap_or(node);
        feed(definition.kind());
        if let Some(name) = field(definition, "name") {
            feed(text(name, source));
        }
        return;
    }
    if node.kind() == "comment" || is_string_statement(node) {
        return;
    }
    if node.child_count() == 0 || node.kind() == "string" {
        feed(text(node, source));
        return;
    }
    let is_block = node.kind() == "block";
    if is_block {
        feed("{");
    }
    for child in node.children(&mut node.walk()) {
        tokens(child, unit, source, feed);
    }
    if is_block {
        feed("}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(code: &str) -> String {
        let mut parser = tree_sitter::Parser::new();
        assert!(
            parser
                .set_language(&tree_sitter_python::LANGUAGE.into())
                .is_ok()
        );
        let Some(tree) = parser.parse(code, None) else {
            panic!("parse failed");
        };
        let Some(function) = tree.root_node().named_child(0) else {
            panic!("no definition");
        };
        content_hash(function, function, code).value
    }

    #[test]
    fn ignores_formatting_comments_and_docstrings() {
        assert_eq!(
            hash("def f(a):\n    return g(a)\n"),
            hash("def f( a ):\n    \"\"\"Doc.\"\"\"\n    # note\n    return g(a)  # why\n"),
        );
    }

    #[test]
    fn sees_indentation_and_ignores_nested_bodies() {
        assert_ne!(
            hash("def f():\n    if a:\n        b()\n        c()\n"),
            hash("def f():\n    if a:\n        b()\n    c()\n"),
        );
        assert_eq!(
            hash("def f():\n    def g():\n        return 1\n    return g\n"),
            hash("def f():\n    def g():\n        return 2\n    return g\n"),
        );
    }
}
