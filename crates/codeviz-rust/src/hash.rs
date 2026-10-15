//! Content hashes over a unit's own tokens, insensitive to formatting and comments.

use codeviz_ir::ContentHash;
use tree_sitter::Node;

pub(crate) const ALGORITHM: &str = "codeviz-tokens-blake3";

pub(crate) fn content_hash(node: Node<'_>, source: &[u8]) -> ContentHash {
    let mut hasher = blake3::Hasher::new();
    let mut cursor = node.walk();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.kind().ends_with("comment") {
            continue;
        }
        if current.child_count() == 0 {
            if let Some(token) = source.get(current.start_byte()..current.end_byte()) {
                hasher.update(token);
                hasher.update(b" ");
            }
            continue;
        }
        let children: Vec<Node<'_>> = current.children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    ContentHash {
        algo: ALGORITHM.to_owned(),
        value: hasher.finalize().to_hex().to_string(),
    }
}
