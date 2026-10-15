//! End-to-end checks of the Rust producer on small in-memory crates.

#![allow(clippy::unwrap_used, reason = "test helpers fail loudly")]

use std::fs;

use codeviz_ir::{Certainty, Document, ExitKind, Mode, Role, Step, Unit, UnitId};
use codeviz_rust::{AnalyzeOptions, analyze};

/// Writes `files` as crate `app` in a temp dir and analyzes it.
fn analyze_files(files: &[(&str, &str)]) -> Document {
    let root = std::env::temp_dir().join(format!(
        "codeviz-rust-test-{}-{}",
        std::process::id(),
        files.iter().map(|(_, s)| s.len()).sum::<usize>()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    for (path, source) in files {
        let path = root.join("src").join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let document = analyze(&root, AnalyzeOptions::default()).unwrap().document;
    fs::remove_dir_all(&root).unwrap();
    let diagnostics = codeviz_ir::validate(&document);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    document
}

fn unit<'d>(document: &'d Document, id: &str) -> &'d Unit {
    document
        .unit(&UnitId::new(format!("rust:{id}")))
        .unwrap_or_else(|| panic!("no unit {id}"))
}

/// `(call text, target ids, certainty)` for each call site.
fn calls(unit: &Unit) -> Vec<(String, Vec<String>)> {
    unit.calls
        .iter()
        .map(|c| {
            (
                c.text.clone(),
                c.targets.iter().map(|t| t.unit.to_string()).collect(),
            )
        })
        .collect()
}

#[test]
fn resolves_through_modules_imports_and_self() {
    let document = analyze_files(&[
        (
            "main.rs",
            "mod svc; mod store; use crate::svc::users; use store::Store as Db;
             fn main() { users::find(); Db::open(); svc::helper(); unknown(); }",
        ),
        ("svc/mod.rs", "pub mod users; pub fn helper() {}"),
        (
            "svc/users.rs",
            "use super::helper; pub fn find() { helper(); self::local(); } fn local() {}",
        ),
        (
            "store.rs",
            "pub struct Store; impl Store { pub fn open() -> Self { Self::new() } fn new() -> Self { Store } }",
        ),
    ]);

    assert_eq!(
        calls(unit(&document, "app::main")),
        vec![
            (
                "users::find".into(),
                vec!["rust:app::svc::users::find".into()]
            ),
            (
                "Db::open".into(),
                vec!["rust:app::store::Store::open".into()]
            ),
            ("svc::helper".into(), vec!["rust:app::svc::helper".into()]),
            ("unknown".into(), vec![]),
        ]
    );
    assert_eq!(
        calls(unit(&document, "app::store::Store::open")),
        vec![(
            "Self::new".into(),
            vec!["rust:app::store::Store::new".into()]
        )]
    );
    let store = unit(&document, "app::store::Store");
    assert_eq!((store.kind.as_str(), store.role), ("struct", Role::Group));
    assert_eq!(
        unit(&document, "app::store::Store::open").parent,
        Some(store.id.clone())
    );
}

#[test]
fn trait_impls_get_distinct_ids() {
    let document = analyze_files(&[(
        "lib.rs",
        "pub struct User;
         impl std::fmt::Display for User { fn fmt(&self) {} }
         impl std::fmt::Debug for User { fn fmt(&self) {} }",
    )]);
    unit(&document, "app::User::{impl#Display}::fmt");
    unit(&document, "app::User::{impl#Debug}::fmt");
}

#[test]
fn receivers_resolve_through_declared_types() {
    let document = analyze_files(&[(
        "lib.rs",
        "use std::sync::Arc;
         pub struct A { inner: Arc<Inner> } pub struct B; pub struct Inner;
         impl A {
             fn run(&self, b: &B, opaque: u8) {
                 self.step();
                 b.solo();
                 self.inner.work();
                 let made = Inner::new();
                 made.work();
                 opaque.solo();
                 unknown.solo();
             }
             fn step(&self) {}
         }
         impl B { fn solo(&self) {} }
         impl Inner { fn new() -> Self { Inner } fn work(&self) {} }",
    )]);
    let resolved: Vec<(String, Option<Certainty>)> = unit(&document, "app::A::run")
        .calls
        .iter()
        .map(|c| (c.text.clone(), c.targets.first().map(|t| t.certainty)))
        .collect();
    assert_eq!(
        resolved,
        vec![
            (".step".into(), Some(Certainty::Exact)),
            (".solo".into(), Some(Certainty::Exact)),
            (".work".into(), Some(Certainty::Exact)),
            ("Inner::new".into(), Some(Certainty::Exact)),
            // `Inner::new` returns `Self`.
            (".work".into(), Some(Certainty::Exact)),
            // `u8` is known and has no `solo` here: not a guess at `B::solo`.
            (".solo".into(), None),
            // Nothing known about `unknown`: matched by name.
            (".solo".into(), Some(Certainty::Inferred)),
        ]
    );
}

#[test]
fn control_flow_becomes_steps() {
    let document = analyze_files(&[(
        "lib.rs",
        r#"pub fn check(q: u32) -> Result<u32, String> {
               if q == 0 { return Err("zero".into()); }
               for _ in 0..q { tick(); }
               match q { 1 => Ok(one()), _ => Err(String::new()) }
           }
           fn tick() {}
           fn one() -> u32 { 1 }"#,
    )]);
    let body = unit(&document, "app::check").body.clone().unwrap();
    let [
        Step::Branch { arms, .. },
        Step::Loop { .. },
        Step::Branch {
            arms: match_arms, ..
        },
    ] = body.as_slice()
    else {
        panic!("unexpected body {body:#?}");
    };
    assert!(matches!(
        arms[0].body.last(),
        Some(Step::Exit {
            kind: ExitKind::Error,
            ..
        })
    ));
    assert!(
        arms[1].body.is_empty(),
        "missing else is an explicit empty arm"
    );
    assert!(matches!(
        match_arms[1].body.last(),
        Some(Step::Exit {
            kind: ExitKind::Error,
            ..
        })
    ));
}

#[test]
fn question_mark_and_result_tails_propagate() {
    let document = analyze_files(&[(
        "lib.rs",
        "fn a() -> Result<(), ()> { b()?; c(); d() }
         fn b() -> Result<(), ()> { Ok(()) }
         fn c() {}
         fn d() -> Result<(), ()> { Ok(()) }",
    )]);
    let propagates: Vec<(String, bool)> = unit(&document, "app::a")
        .calls
        .iter()
        .map(|c| (c.text.clone(), c.propagates))
        .collect();
    assert_eq!(
        propagates,
        vec![("b".into(), true), ("c".into(), false), ("d".into(), true)]
    );
}

#[test]
fn handoffs_and_routes() {
    let document = analyze_files(&[(
        "main.rs",
        r#"fn main() { let app = Router::new().route("/health", get(health)); std::thread::spawn(|| work()); }
           async fn health() -> &'static str { "ok" }
           fn work() {}"#,
    )]);
    let main = unit(&document, "app::main");
    let handoffs: Vec<(&str, Mode, Vec<String>)> = main
        .calls
        .iter()
        .filter(|c| c.mode != Mode::Call)
        .map(|c| {
            (
                c.text.as_str(),
                c.mode,
                c.targets.iter().map(|t| t.unit.to_string()).collect(),
            )
        })
        .collect();
    assert_eq!(
        handoffs,
        vec![
            (
                "health",
                Mode::Register,
                vec!["rust:app::health".to_owned()]
            ),
            (
                "{closure#0}",
                Mode::Spawn,
                vec!["rust:app::main::{closure#0}".to_owned()]
            ),
        ]
    );
    let closure = unit(&document, "app::main::{closure#0}");
    assert_eq!(
        calls(closure),
        vec![("work".into(), vec!["rust:app::work".into()])]
    );

    let routes: Vec<(&str, &str)> = document
        .entries
        .iter()
        .map(|e| (e.label.as_str(), e.kind.as_str()))
        .collect();
    assert!(routes.contains(&("GET /health", "http")));
}

#[test]
fn content_hash_ignores_formatting_and_comments() {
    let hash = |source: &str| {
        unit(&analyze_files(&[("lib.rs", source)]), "app::f")
            .content_hash
            .clone()
            .unwrap()
    };
    assert_eq!(
        hash("fn f() { g(); }  fn g() {}"),
        hash("// note\nfn f() {\n    g(); // call\n}\nfn g() {}")
    );
    assert_ne!(
        hash("fn f() { g(); } fn g() {}"),
        hash("fn f() { g(); g(); } fn g() {}")
    );
}

#[test]
fn a_variable_passed_along_is_not_a_method_of_the_same_name() {
    let document = analyze_files(&[(
        "main.rs",
        "struct App; impl App { fn state(&self) -> u32 { 1 } }
         fn show(state: u32) {}
         fn main() { let state = 2; show(state); }",
    )]);
    assert_eq!(
        calls(unit(&document, "app::main")),
        [("show".to_owned(), vec!["rust:app::show".to_owned()])]
    );
}

#[test]
fn packages_a_workspace_excludes_are_skipped() {
    let root = std::env::temp_dir().join(format!("codeviz-rust-exclude-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for (dir, name) in [("app", "app"), ("fixtures/old", "app")] {
        fs::create_dir_all(root.join(dir).join("src")).unwrap();
        fs::write(
            root.join(dir).join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
        )
        .unwrap();
        fs::write(root.join(dir).join("src/main.rs"), "fn main() {}").unwrap();
    }
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"app\"]\nexclude = [\"fixtures\"]\n",
    )
    .unwrap();
    let analysis = analyze(&root, AnalyzeOptions::default()).unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(analysis.packages.len(), 1, "{:?}", analysis.packages);
    assert!(analysis.packages[0].src_dir.ends_with("app/src"));
}
