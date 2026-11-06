//! End-to-end checks of the Rust producer on small in-memory crates.

#![allow(clippy::unwrap_used, reason = "test helpers fail loudly")]

use std::fs;

use codeviz_ir::{Certainty, Document, ExitKind, Mode, Role, Step, Unit, UnitId};
use codeviz_rust::{AnalyzeOptions, Resolver, analyze, rust_analyzer_available};

/// Writes `files` as crate `app` in a temp dir and analyzes it.
fn analyze_files(files: &[(&str, &str)]) -> Document {
    analyze_files_with(files, &AnalyzeOptions::default())
}

fn analyze_files_with(files: &[(&str, &str)], options: &AnalyzeOptions) -> Document {
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
    let document = analyze(&root, options);
    fs::remove_dir_all(&root).unwrap();
    let document = document.unwrap().document;
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
            // Nothing known about `unknown`: not a guess at `B::solo` either.
            (".solo".into(), None),
        ]
    );
}

#[test]
fn methods_on_values_of_unknown_type_are_not_ours_by_name() {
    // `Option::map`, egui's `Area::order` and `Vec::len` are not the workspace's
    // `Explorer::map`, `Store::order` and `Journey::len`.
    let document = analyze_files(&[(
        "lib.rs",
        "pub struct Explorer { tiles: Vec<u32> }
         impl Explorer { pub fn map(&self) -> &Vec<u32> { &self.tiles } }
         pub struct Store;
         impl Store { pub fn order(&self, id: u32) -> Option<u32> { Some(id) } }
         pub struct Journey;
         impl Journey { pub fn len(&self) -> usize { 0 } pub fn is_empty(&self) -> bool { true } }
         pub fn panel(names: &[String]) -> usize {
             egui::Area::new(\"panel\").order(egui::Order::Foreground);
             let lengths = names.iter().map(|name| name.len());
             names.len() + lengths.sum::<usize>() + usize::from(names.is_empty())
         }
         pub fn real(explorer: &Explorer, store: &Store, journey: &Journey) -> usize {
             explorer.map();
             store.order(1);
             journey.len()
         }",
    )]);
    let resolved: Vec<(String, Vec<String>)> = calls(unit(&document, "app::panel"))
        .into_iter()
        .filter(|(_, targets)| !targets.is_empty())
        .collect();
    assert_eq!(
        resolved,
        [(
            "{closure#0}".to_owned(),
            vec!["rust:app::panel::{closure#0}".to_owned()]
        )],
        "only the closure handed to `map` resolves"
    );
    let closure = unit(&document, "app::panel::{closure#0}");
    assert_eq!(calls(closure), [(".len".to_owned(), vec![])]);
    assert_eq!(
        calls(unit(&document, "app::real")),
        [
            (
                ".map".to_owned(),
                vec!["rust:app::Explorer::map".to_owned()]
            ),
            (
                ".order".to_owned(),
                vec!["rust:app::Store::order".to_owned()]
            ),
            (".len".to_owned(), vec!["rust:app::Journey::len".to_owned()]),
        ]
    );
}

#[test]
fn receivers_resolve_through_wrappers_patterns_and_closures() {
    let document = analyze_files(&[(
        "lib.rs",
        "pub struct Store; pub struct Explorer; pub struct Track;
         impl Store { pub fn order(&self) {} }
         impl Explorer {
             pub fn track(&self) -> Option<&Track> { None }
             pub fn tile(&self) {}
             pub fn settle(&mut self) {}
         }
         impl Track { pub fn span(&self) {} }
         pub async fn reorder(State(store): State<Store>) { store.order(); }
         pub fn system(explorer: Res<Explorer>, mut moving: ResMut<Explorer>) {
             let moving = moving.bypass_change_detection();
             moving.settle();
             let Some(track) = explorer.track() else { return };
             if let Some(again) = explorer.track() { again.span(); }
             let near = |tile: u32| { explorer.tile(); track.span(); };
             let shadowed = |explorer| explorer.tile();
             println!(\"{:?}\", explorer.tile());
         }",
    )]);
    let resolved = |id: &str| -> Vec<(String, Vec<String>)> {
        calls(unit(&document, id))
            .into_iter()
            .filter(|(_, targets)| !targets.is_empty())
            .collect()
    };
    let call = |text: &str, target: &str| (text.to_owned(), vec![format!("rust:app::{target}")]);

    // `State(store): State<Store>` binds `store` to the extractor's one field.
    assert_eq!(resolved("app::reorder"), [call(".order", "Store::order")]);
    // Through `ResMut`, `bypass_change_detection`, `let .. else`, `if let` and a macro.
    assert_eq!(
        resolved("app::system"),
        [
            call(".settle", "Explorer::settle"),
            call(".track", "Explorer::track"),
            call(".track", "Explorer::track"),
            call(".span", "Track::span"),
            call(".tile", "Explorer::tile"),
        ]
    );
    // A closure sees what it captures, a returned value included.
    assert_eq!(
        resolved("app::system::{closure#0}"),
        [
            call(".tile", "Explorer::tile"),
            call(".span", "Track::span")
        ]
    );
    // An untyped parameter shadows the capture of the same name.
    assert_eq!(resolved("app::system::{closure#1}"), []);
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
           fn one() -> u32 { 1 }
           pub enum Outcome { Err(u32), Done }
           pub fn outcome(q: u32) -> Outcome { if q == 0 { return Outcome::Err(q); } Outcome::Done }"#,
    )]);
    // `return Outcome::Err(..)` is one exit, an error: not an error and then a return.
    let Some(Step::Branch { arms, .. }) = unit(&document, "app::outcome")
        .body
        .as_ref()
        .unwrap()
        .first()
    else {
        panic!("no branch");
    };
    assert!(matches!(
        arms[0].body.as_slice(),
        [Step::Exit {
            kind: ExitKind::Error,
            ..
        }]
    ));
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

/// A fresh directory named for the test, with `files` written into it.
fn tree(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("codeviz-rust-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for (path, text) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    root
}

fn ids(document: &Document) -> Vec<String> {
    document.units.iter().map(|u| u.id.to_string()).collect()
}

const PACKAGE: &str = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n";

#[test]
fn packages_outside_their_workspace_and_broken_manifests_are_skipped() {
    let member = |name: &str| format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n");
    let app = format!(
        "{}[dependencies]\nshared = {{ path = \"../shared\" }}\n",
        member("app")
    );
    let root = tree(
        "workspace",
        &[
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"app\"]\nexclude = [\"old\"]\n",
            ),
            ("app/Cargo.toml", &app),
            ("app/src/main.rs", "fn main() {}"),
            // A path dependency is a member without being listed.
            ("shared/Cargo.toml", &member("shared")),
            ("shared/src/lib.rs", "pub fn f() {}"),
            ("old/Cargo.toml", &member("old")),
            ("old/src/lib.rs", "pub fn f() {}"),
            ("app/tests/fixtures/sample/Cargo.toml", &member("sample")),
            ("app/tests/fixtures/sample/src/lib.rs", "pub fn f() {}"),
            ("broken/Cargo.toml", "[package\nname ="),
        ],
    );
    let analysis = analyze(&root, &AnalyzeOptions::default()).unwrap();
    fs::remove_dir_all(&root).unwrap();
    let crates: Vec<&str> = analysis.crates.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(crates, ["app", "shared"]);
    let notes = analysis.notes.join("\n");
    assert!(
        notes.contains("skipped failed to parse manifest"),
        "{notes}"
    );
    assert!(notes.contains("app/tests/fixtures/sample, old"), "{notes}");
}

#[test]
fn every_library_and_binary_is_a_crate() {
    let manifest = format!("{PACKAGE}[[bin]]\nname = \"rg\"\npath = \"core/main.rs\"\n");
    let root = tree(
        "targets",
        &[
            ("Cargo.toml", &manifest),
            ("build.rs", "fn main() {}"),
            (
                "core/main.rs",
                "mod flags; fn main() { flags::parse(); app::run(); }",
            ),
            ("core/flags.rs", "pub fn parse() {}"),
            (
                "src/lib.rs",
                "pub mod store; pub fn run() { store::open(); }",
            ),
            ("src/store.rs", "pub fn open() {}"),
            // Named like the package, the default binary shares the library's crate.
            ("src/main.rs", "fn main() { app::run(); }"),
            ("src/bin/extra.rs", "fn main() { app::store::open(); }"),
            (
                "src/bin/multi/main.rs",
                "mod part; fn main() { part::go(); }",
            ),
            ("src/bin/multi/part.rs", "pub fn go() {}"),
            ("examples/demo.rs", "fn main() {}"),
        ],
    );
    let analysis = analyze(&root, &AnalyzeOptions::default()).unwrap();
    fs::remove_dir_all(&root).unwrap();
    let document = &analysis.document;
    assert_eq!(codeviz_ir::validate(document), Vec::new());
    let crates: Vec<&str> = document
        .units
        .iter()
        .filter(|u| u.kind == "crate")
        .map(|u| u.name.as_str())
        .collect();
    assert_eq!(crates, ["app", "extra", "multi", "rg"]);
    assert_eq!(
        calls(unit(document, "rg::main")),
        [
            (
                "flags::parse".to_owned(),
                vec!["rust:rg::flags::parse".to_owned()]
            ),
            ("app::run".to_owned(), vec!["rust:app::run".to_owned()]),
        ]
    );
    assert_eq!(
        calls(unit(document, "multi::main")),
        [(
            "part::go".to_owned(),
            vec!["rust:multi::part::go".to_owned()]
        )]
    );
    let entries: Vec<&str> = document.entries.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(entries, ["app main", "extra main", "multi main", "rg main"]);
    assert!(
        !ids(document)
            .iter()
            .any(|id| id.contains("bin") || id.contains("demo")),
        "{:?}",
        ids(document)
    );
}

#[test]
fn items_written_inside_macros_are_found() {
    let document = analyze_files(&[
        (
            "lib.rs",
            "macro_rules! cfg_rt { ($($item:item)*) => { $($item)* } }
             cfg_rt! {
                 pub mod rt;
                 pub fn spawn() { rt::run(); }
             }
             cfg_if::cfg_if! {
                 if #[cfg(unix)] { pub fn unix_only() {} } else { pub fn elsewhere() {} }
             }
             pub struct S;
             impl S { cfg_rt! { pub fn inside(&self) { spawn(); } } }
             pub fn body() { cfg_rt! { helper(); } }
             fn helper() {}",
        ),
        ("rt.rs", "pub fn run() {}"),
    ]);
    for id in [
        "app::unix_only",
        "app::elsewhere",
        "app::S::inside",
        "app::rt::run",
    ] {
        unit(&document, id);
    }
    assert_eq!(
        calls(unit(&document, "app::spawn")),
        [("rt::run".to_owned(), vec!["rust:app::rt::run".to_owned()])]
    );
    assert_eq!(
        calls(unit(&document, "app::body")),
        [("helper".to_owned(), vec!["rust:app::helper".to_owned()])],
        "in a body, a macro's tokens are still calls"
    );
}

#[test]
fn a_directory_inside_a_package_is_that_part_of_it() {
    let root = tree(
        "inside",
        &[
            ("Cargo.toml", PACKAGE),
            ("src/lib.rs", "pub mod svc; pub fn top() {}"),
            (
                "src/svc/mod.rs",
                "pub mod users; pub fn helper() { crate::top(); }",
            ),
            ("src/svc/users.rs", "pub fn find() { super::helper(); }"),
        ],
    );
    let analysis = analyze(&root.join("src/svc"), &AnalyzeOptions::default()).unwrap();
    fs::remove_dir_all(&root).unwrap();
    let document = &analysis.document;
    assert_eq!(
        calls(unit(document, "app::svc::users::find")),
        [(
            "super::helper".to_owned(),
            vec!["rust:app::svc::helper".to_owned()]
        )]
    );
    assert!(document.unit(&UnitId::new("rust:app::top")).is_none());
    assert_eq!(
        unit(document, "app::svc::users::find")
            .location
            .as_ref()
            .unwrap()
            .uri,
        "users.rs"
    );
    assert!(
        analysis.notes[0].ends_with("only src/svc of package app"),
        "{:?}",
        analysis.notes
    );
}

#[test]
fn rust_analyzer_resolves_what_names_alone_cannot() {
    if !rust_analyzer_available() {
        eprintln!("skipped: rust-analyzer is not installed");
        return;
    }
    let source = "pub trait Shape {
            fn area(&self) -> f64;
            fn describe(&self) -> String { format!(\"{}\", self.area()) }
        }
        pub struct Circle(pub f64);
        pub struct Square(pub f64);
        impl Shape for Circle { fn area(&self) -> f64 { self.0 * self.0 * 3.0 } }
        impl Shape for Square { fn area(&self) -> f64 { self.0 * self.0 } }
        pub struct Store { items: Vec<u32> }
        impl Store { pub fn map(&self) -> Vec<u32> { self.items.clone() } }
        fn make() -> Circle { Circle(1.0) }
        pub fn circle() -> f64 { let c = make(); c.area() }
        pub fn doubled(store: &Store) -> Vec<u32> {
            store.map().into_iter().map(|x| x * 2).collect()
        }
        pub fn report(apply: fn() -> f64) -> f64 { apply() }";
    let options = AnalyzeOptions {
        resolver: Resolver::RustAnalyzer { target_dir: None },
        ..AnalyzeOptions::default()
    };
    let document = analyze_files_with(&[("lib.rs", source)], &options);
    let circle_area = "rust:app::Circle::{impl#Shape}::area".to_owned();
    let square_area = "rust:app::Square::{impl#Shape}::area".to_owned();

    let doubled = unit(&document, "app::doubled");
    assert_eq!(
        calls(doubled)[..3],
        [
            (".map".to_owned(), vec!["rust:app::Store::map".to_owned()]),
            (".into_iter".to_owned(), vec![]),
            (".map".to_owned(), vec![]),
        ],
        "Iterator::map is not Store::map"
    );
    assert!(
        doubled
            .calls
            .iter()
            .all(|c| c.targets.iter().all(|t| t.certainty == Certainty::Exact))
    );
    assert_eq!(
        calls(unit(&document, "app::circle"))[1],
        (".area".to_owned(), vec![circle_area.clone()]),
        "the type `make` returns picks the implementation"
    );
    assert!(
        calls(unit(&document, "app::Shape::describe"))
            .contains(&(".area".to_owned(), vec![circle_area, square_area])),
        "through the trait, any implementation may run"
    );
    assert_eq!(
        calls(unit(&document, "app::report")),
        [("apply".to_owned(), vec![])],
        "a parameter is not a function of the same name"
    );
}

#[test]
fn rust_analyzer_indexes_each_workspace_and_says_what_it_could_not() {
    if !rust_analyzer_available() {
        eprintln!("skipped: rust-analyzer is not installed");
        return;
    }
    let package = |name: &str| {
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")
    };
    // Only rust-analyzer knows what `v[0]` is.
    let shapes = "pub struct Circle;
        impl Circle { pub fn area(&self) -> u32 { 1 } }
        pub fn total() -> u32 { let v = vec![Circle]; v[0].area() }";
    let root = tree(
        "workspaces",
        &[
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"app\", \"bad\"]\nresolver = \"2\"\n",
            ),
            ("app/Cargo.toml", &package("app")),
            ("app/src/lib.rs", shapes),
            ("bad/Cargo.toml", &package("bad")),
            ("bad/build.rs", "fn main() { panic!(\"no libfoo\"); }"),
            ("bad/src/lib.rs", "pub fn b() {}"),
            // A workspace of its own, as axum's examples are.
            (
                "examples/Cargo.toml",
                "[workspace]\nmembers = [\"demo\"]\nresolver = \"2\"\n",
            ),
            ("examples/demo/Cargo.toml", &package("demo")),
            ("examples/demo/src/lib.rs", shapes),
        ],
    );
    let options = AnalyzeOptions {
        resolver: Resolver::RustAnalyzer {
            target_dir: Some(root.join("target")),
        },
        ..AnalyzeOptions::default()
    };
    let analysis = analyze(&root, &options).unwrap();
    fs::remove_dir_all(&root).unwrap();
    for name in ["app", "demo"] {
        assert_eq!(
            calls(unit(&analysis.document, &format!("{name}::total"))).last(),
            Some(&(
                ".area".to_owned(),
                vec![format!("rust:{name}::Circle::area")]
            )),
        );
    }
    let notes = analysis.notes.join("\n");
    assert!(notes.contains("indexed the workspace ("), "{notes}");
    assert!(
        notes.contains("indexed the workspace in examples ("),
        "{notes}"
    );
    assert!(notes.contains("the build script of bad failed"), "{notes}");
}
