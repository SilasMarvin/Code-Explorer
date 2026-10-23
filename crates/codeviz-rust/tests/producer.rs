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
    let analysis = analyze(&root, &AnalyzeOptions::default()).unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(analysis.packages.len(), 1, "{:?}", analysis.packages);
    assert!(analysis.packages[0].src_dir.ends_with("app/src"));
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

/// `(call text, target ids, certainty of the first target)` for each call site.
fn resolved(unit: &Unit) -> Vec<(String, Vec<String>, Option<Certainty>)> {
    unit.calls
        .iter()
        .map(|c| {
            (
                c.text.clone(),
                c.targets.iter().map(|t| t.unit.to_string()).collect(),
                c.targets.first().map(|t| t.certainty),
            )
        })
        .collect()
}

#[test]
fn names_alone_match_only_distinctive_methods() {
    let document = analyze_files(&[(
        "lib.rs",
        "pub struct Store; pub struct Lane; pub struct Tile;
         impl Store { pub fn map(&self) {} pub fn tally(&self) {} pub fn label(&self) {} }
         impl Lane { pub fn label(&self) {} }
         impl Tile { pub fn label(&self) {} }
         fn run() {
             unknown.map();
             unknown.tally();
             unknown.label();
         }",
    )]);
    assert_eq!(
        resolved(unit(&document, "app::run")),
        [
            // A std name on a value of unknown type is std's.
            (".map".to_owned(), vec![], None),
            // Only `Store` has a `tally`: a guess, marked so.
            (
                ".tally".to_owned(),
                vec!["rust:app::Store::tally".to_owned()],
                Some(Certainty::Inferred)
            ),
            // Three types have a `label`: no telling which.
            (".label".to_owned(), vec![], None),
        ]
    );
}

#[test]
fn names_alone_match_only_in_crates_the_caller_depends_on() {
    let document = analyze_workspace(&[
        (
            "base",
            "",
            "pub struct Base; impl Base { pub fn from_base(&self) {} }",
        ),
        (
            "app",
            "base = { path = \"../base\" }",
            "pub struct App; impl App { pub fn from_app(&self) {} }
             pub fn run() { x.from_base(); x.from_app(); }",
        ),
        ("other", "", "pub fn run() { x.from_base(); x.from_app(); }"),
    ]);
    assert_eq!(
        calls(unit(&document, "app::run")),
        [
            (
                ".from_base".to_owned(),
                vec!["rust:base::Base::from_base".to_owned()]
            ),
            (
                ".from_app".to_owned(),
                vec!["rust:app::App::from_app".to_owned()]
            ),
        ]
    );
    assert_eq!(
        calls(unit(&document, "other::run")),
        [
            (".from_base".to_owned(), vec![]),
            (".from_app".to_owned(), vec![]),
        ],
        "`other` depends on neither"
    );
}

#[test]
fn closures_and_unwrapped_values_keep_their_types() {
    let document = analyze_files(&[(
        "lib.rs",
        "pub struct Journey; pub struct Track;
         impl Journey { pub fn root(&self) {} }
         impl Track {
             pub fn journey(&self) -> Option<&Journey> { None }
             pub fn root(&self) {}
         }
         fn show(track: &Track) {
             let Some(journey) = track.journey() else { return };
             journey.root();
             let again = || journey.root();
             track.journey().map(|j| j.root());
             if let Some(found) = track.journey() { found.root(); }
             let later = || track.root();
         }",
    )]);
    let journey_root = |text: &str| {
        (
            text.to_owned(),
            vec!["rust:app::Journey::root".to_owned()],
            Some(Certainty::Exact),
        )
    };
    let show = resolved(unit(&document, "app::show"));
    assert!(show.contains(&journey_root(".root")), "{show:#?}");
    assert_eq!(
        resolved(unit(&document, "app::show::{closure#0}")),
        [journey_root(".root")],
        "a closure sees what it captures"
    );
    assert_eq!(
        resolved(unit(&document, "app::show::{closure#1}")),
        [journey_root(".root")],
        "`map` hands the closure what the option holds"
    );
    assert_eq!(
        calls(unit(&document, "app::show::{closure#2}")),
        [(".root".to_owned(), vec!["rust:app::Track::root".to_owned()])]
    );
    let if_let = show.iter().filter(|c| **c == journey_root(".root")).count();
    assert_eq!(
        if_let, 2,
        "`if let Some(found)` binds what the option holds"
    );
}

#[test]
fn reexports_resolve_exactly_and_overloads_are_guesses() {
    let document = analyze_files(&[
        (
            "lib.rs",
            "mod store; pub use store::Store;
             pub struct Scalar;
             impl From<bool> for Scalar { fn from(_: bool) -> Self { Scalar } }
             impl From<u8> for Scalar { fn from(_: u8) -> Self { Scalar } }
             fn run() { crate::Store::open(); Scalar::from(true); }",
        ),
        (
            "store.rs",
            "pub struct Store; impl Store { pub fn open() {} }",
        ),
    ]);
    let run = resolved(unit(&document, "app::run"));
    assert_eq!(
        run[0],
        (
            "crate::Store::open".to_owned(),
            vec!["rust:app::store::Store::open".to_owned()],
            Some(Certainty::Exact)
        )
    );
    assert_eq!(
        (run[1].1.len(), run[1].2),
        (2, Some(Certainty::Inferred)),
        "which `From` runs depends on the argument's type"
    );
}

#[test]
fn foreign_values_and_wrappers() {
    let document = analyze_files(&[(
        "lib.rs",
        "pub struct Explorer; pub struct Lane;
         impl Explorer { pub fn tile(&self) {} }
         impl Lane { pub fn tile(&self) {} pub fn color(&self) {} }
         #[derive(Default)] pub struct Diagnostics;
         impl Diagnostics { pub fn push(&mut self, _: u8) {} }
         fn system(explorer: Res<Explorer>) {
             explorer.tile();
             egui::RichText::new(\"x\").color();
             let mut out = Diagnostics::default();
             out.push(1);
         }",
    )]);
    assert_eq!(
        resolved(unit(&document, "app::system")),
        [
            // Bevy's `Res` derefs to what it holds.
            (
                ".tile".to_owned(),
                vec!["rust:app::Explorer::tile".to_owned()],
                Some(Certainty::Exact)
            ),
            ("egui::RichText::new".to_owned(), vec![], None),
            // What a dependency's function returns has the dependency's methods.
            (".color".to_owned(), vec![], None),
            ("Diagnostics::default".to_owned(), vec![], None),
            // A derived `default` makes a `Diagnostics`.
            (
                ".push".to_owned(),
                vec!["rust:app::Diagnostics::push".to_owned()],
                Some(Certainty::Inferred)
            ),
        ]
    );
}

#[test]
fn methods_through_deref_traits_and_cfg_twins() {
    let document = analyze_files(&[(
        "lib.rs",
        "use std::ops::Deref;
         pub struct Context; pub struct Ui { ctx: Context } pub struct Plain;
         impl Context { pub fn input(&self) {} }
         impl Deref for Ui { type Target = Context; fn deref(&self) -> &Context { &self.ctx } }
         pub trait Parser { fn parse(&self); fn map(&self) {} }
         impl Parser for Plain { fn parse(&self) {} }
         pub struct Printer;
         impl Printer {
             #[cfg(unix)] pub fn print(&self) {}
             #[cfg(not(unix))] pub fn print(&self) {}
         }
         fn run(ui: &Ui, plain: &Plain, option: Option<u8>, printer: &Printer) {
             ui.input();
             plain.map();
             option.map();
             printer.print();
         }",
    )]);
    let run = resolved(unit(&document, "app::run"));
    assert_eq!(
        run[0],
        (
            ".input".to_owned(),
            vec!["rust:app::Context::input".to_owned()],
            Some(Certainty::Exact)
        ),
        "`Ui` derefs to `Context`"
    );
    assert_eq!(
        run[1],
        (
            ".map".to_owned(),
            vec!["rust:app::Parser::map".to_owned()],
            Some(Certainty::Inferred)
        ),
        "`Plain` implements `Parser`, so has its provided `map`"
    );
    assert_eq!(run[2], (".map".to_owned(), vec![], None), "`Option::map`");
    assert_eq!(
        (run[3].1.len(), run[3].2),
        (2, Some(Certainty::Inferred)),
        "which `print` builds depends on the target"
    );
}

#[test]
fn closures_take_the_types_their_callee_declares() {
    let document = analyze_files(&[(
        "lib.rs",
        "pub struct Memory; pub struct Data; pub struct Context;
         impl Memory { pub fn focus(&self) {} }
         impl Data { pub fn focus(&self) {} }
         impl Context {
             pub fn memory_mut<R>(&self, writer: impl FnOnce(&mut Memory) -> R) -> R { todo!() }
             pub fn data<F: Fn(u8, &Data)>(&self, reader: F) {}
         }
         fn run(ctx: &Context) {
             ctx.memory_mut(|mem| mem.focus());
             ctx.data(|_, data| data.focus());
         }",
    )]);
    let focus = |ty: &str| {
        vec![(
            ".focus".to_owned(),
            vec![format!("rust:app::{ty}::focus")],
            Some(Certainty::Exact),
        )]
    };
    assert_eq!(
        resolved(unit(&document, "app::run::{closure#0}")),
        focus("Memory")
    );
    assert_eq!(
        resolved(unit(&document, "app::run::{closure#1}")),
        focus("Data")
    );
}

/// Writes each `(name, dependencies, lib.rs)` as a crate in one workspace and
/// analyzes it.
fn analyze_workspace(crates: &[(&str, &str, &str)]) -> Document {
    let root = std::env::temp_dir().join(format!(
        "codeviz-rust-workspace-{}-{}",
        std::process::id(),
        crates.len()
    ));
    let _ = fs::remove_dir_all(&root);
    for (name, dependencies, source) in crates {
        fs::create_dir_all(root.join(name).join("src")).unwrap();
        fs::write(
            root.join(name).join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[dependencies]\n{dependencies}\n"
            ),
        )
        .unwrap();
        fs::write(root.join(name).join("src/lib.rs"), source).unwrap();
    }
    let analysis = analyze(&root, &AnalyzeOptions::default());
    fs::remove_dir_all(&root).unwrap();
    analysis.unwrap().document
}
