//! The documents the docs hand a newcomer are valid and open.

use codeviz_core::System;
use codeviz_ir::Document;

/// The JSON block under `## A minimal document` in docs/ir.md.
fn minimal_document() -> Document {
    let ir = include_str!("../../../docs/ir.md");
    let section = ir
        .split_once("## A minimal document")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    let json = section
        .split_once("```json\n")
        .and_then(|(_, rest)| rest.split_once("\n```"))
        .map(|(json, _)| json)
        .unwrap_or_default();
    match Document::from_json(json) {
        Ok(document) => document,
        Err(error) => panic!("docs/ir.md's minimal document does not parse: {error}"),
    }
}

#[test]
fn the_minimal_document_in_docs_ir_md_is_clean_and_has_its_journey() {
    let document = minimal_document();
    let diagnostics = codeviz_ir::validate(&document);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let system = System::new(vec![document]);
    assert_eq!(system.find("POST /checkout").len(), 1);
}

#[test]
fn the_demo_document_is_clean() {
    let json = include_str!("../../../demo/out/axum-server.json");
    let Ok(document) = Document::from_json(json) else {
        panic!("demo/out/axum-server.json does not parse");
    };
    let errors = codeviz_ir::validate(&document)
        .into_iter()
        .filter(|d| d.severity() == codeviz_ir::Severity::Error)
        .count();
    assert_eq!(errors, 0);
}
