//! End-to-end tests: Python source in, IR document out.

use std::path::{Path, PathBuf};

use codeviz_ir::{
    Arm, Certainty, Document, Entry, ExitKind, Invocation, Mode, Role, Step, Unit, UnitId,
};

/// A temporary source tree, removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(name: &str, files: &[(&str, &str)]) -> anyhow::Result<Self> {
        let root =
            std::env::temp_dir().join(format!("codeviz-python-{name}-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        for (path, contents) in files {
            let path = root.join(path);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, contents)?;
        }
        Ok(Self(root))
    }

    fn analyze(&self) -> anyhow::Result<Document> {
        Ok(codeviz_python::analyze(&self.0)?)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn analyze(name: &str, files: &[(&str, &str)]) -> anyhow::Result<Document> {
    Tree::new(name, files)?.analyze()
}

fn unit<'d>(document: &'d Document, id: &str) -> &'d Unit {
    document
        .unit(&UnitId::new(id))
        .unwrap_or_else(|| panic!("no unit {id}"))
}

fn call<'u>(unit: &'u Unit, key: &str) -> &'u Invocation {
    unit.calls
        .iter()
        .find(|call| call.key.as_str() == key)
        .unwrap_or_else(|| panic!("no call {key} in {}", unit.id))
}

fn targets(invocation: &Invocation) -> Vec<(&str, Certainty)> {
    invocation
        .targets
        .iter()
        .map(|target| (target.unit.as_str(), target.certainty))
        .collect()
}

fn call_step(key: &str) -> Step {
    Step::Call {
        call: codeviz_ir::CallKey::new(key),
    }
}

fn exit(kind: ExitKind, label: Option<&str>) -> Step {
    Step::Exit {
        kind,
        label: label.map(str::to_owned),
    }
}

fn arm(label: &str, body: Vec<Step>) -> Arm {
    Arm {
        label: label.into(),
        body,
    }
}

const PACKAGE: &[(&str, &str)] = &[
    ("app/__init__.py", "from .core import helper as exported\n"),
    (
        "app/core.py",
        "def helper():\n    pass\n\nclass Base:\n    def __init__(self):\n        self.setup()\n\n    def setup(self):\n        pass\n\n    @classmethod\n    def make(cls):\n        return cls()\n",
    ),
    ("app/sub/__init__.py", ""),
    (
        "app/sub/leaf.py",
        "import app.core as core\nfrom .. import core as relative_core\nfrom ..core import Base\nfrom app import exported\n\nclass Child(Base):\n    def __init__(self):\n        super().__init__()\n\n    def setup(self):\n        self.helper_ref = core.helper\n\ndef run(thing):\n    core.helper()\n    relative_core.helper()\n    exported()\n    Base.make()\n    Child()\n    thing.setup()\n    unknown()\n    unknown()\n\ndef outer():\n    def inner():\n        return 1\n    pick = lambda x: inner()\n    return sorted([], key=lambda y: y)\n",
    ),
];

#[test]
fn ids_follow_qualified_names() -> anyhow::Result<()> {
    let document = analyze("ids", PACKAGE)?;
    for (id, kind, role, parent) in [
        ("python:app", "package", Role::Group, None),
        (
            "python:app.sub.leaf",
            "module",
            Role::Group,
            Some("python:app.sub"),
        ),
        (
            "python:app.core.Base",
            "class",
            Role::Group,
            Some("python:app.core"),
        ),
        (
            "python:app.core.Base.make",
            "method",
            Role::Invocable,
            Some("python:app.core.Base"),
        ),
        (
            "python:app.sub.leaf.run",
            "fn",
            Role::Invocable,
            Some("python:app.sub.leaf"),
        ),
        (
            "python:app.sub.leaf.outer.<locals>.inner",
            "closure",
            Role::Invocable,
            Some("python:app.sub.leaf.outer"),
        ),
        (
            "python:app.sub.leaf.outer.<locals>.<lambda#0>",
            "closure",
            Role::Invocable,
            Some("python:app.sub.leaf.outer"),
        ),
        (
            "python:app.sub.leaf.outer.<locals>.<lambda#1>",
            "closure",
            Role::Invocable,
            Some("python:app.sub.leaf.outer"),
        ),
    ] {
        let found = unit(&document, id);
        assert_eq!((found.kind.as_str(), found.role), (kind, role), "{id}");
        assert_eq!(found.parent.as_ref().map(UnitId::as_str), parent, "{id}");
    }
    let hash = unit(&document, "python:app.core.helper")
        .content_hash
        .as_ref();
    assert_eq!(hash.map(|h| h.algo.as_str()), Some("codeviz-tokens-blake3"));
    Ok(())
}

#[test]
fn resolves_imports_classes_and_methods() -> anyhow::Result<()> {
    let document = analyze("resolve", PACKAGE)?;
    let run = unit(&document, "python:app.sub.leaf.run");
    let exact = |id| vec![(id, Certainty::Exact)];
    assert_eq!(
        targets(call(run, "core.helper#0")),
        exact("python:app.core.helper")
    );
    assert_eq!(
        targets(call(run, "relative_core.helper#0")),
        exact("python:app.core.helper")
    );
    assert_eq!(
        targets(call(run, "exported#0")),
        exact("python:app.core.helper")
    );
    assert_eq!(
        targets(call(run, "Base.make#0")),
        exact("python:app.core.Base.make")
    );
    assert_eq!(
        targets(call(run, "Child#0")),
        exact("python:app.sub.leaf.Child.__init__")
    );
    assert_eq!(
        targets(call(run, "thing.setup#0")),
        [
            ("python:app.core.Base.setup", Certainty::Inferred),
            ("python:app.sub.leaf.Child.setup", Certainty::Inferred),
        ]
    );
    assert_eq!(call(run, "unknown#1").targets, Vec::new(), "unresolved");

    let make = unit(&document, "python:app.core.Base.make");
    assert_eq!(
        targets(call(make, "cls#0")),
        exact("python:app.core.Base.__init__")
    );

    // `self.setup()` resolves through the class and may dispatch to an override.
    let init = unit(&document, "python:app.core.Base.__init__");
    assert_eq!(
        targets(call(init, "self.setup#0")),
        [
            ("python:app.core.Base.setup", Certainty::Exact),
            ("python:app.sub.leaf.Child.setup", Certainty::Inferred),
        ]
    );
    let child_init = unit(&document, "python:app.sub.leaf.Child.__init__");
    assert_eq!(
        targets(call(child_init, "super().__init__#0")),
        exact("python:app.core.Base.__init__")
    );
    assert_eq!(child_init.calls.len(), 1, "super() itself is not a call");

    let outer = unit(&document, "python:app.sub.leaf.outer");
    let sorted = call(outer, "sorted#0");
    assert_eq!(sorted.mode, Mode::Register);
    assert_eq!(
        targets(sorted),
        exact("python:app.sub.leaf.outer.<locals>.<lambda#1>")
    );
    let lambda = unit(&document, "python:app.sub.leaf.outer.<locals>.<lambda#0>");
    assert_eq!(
        targets(call(lambda, "inner#0")),
        exact("python:app.sub.leaf.outer.<locals>.inner")
    );
    Ok(())
}

#[test]
fn lowers_branches_and_loops() -> anyhow::Result<()> {
    let source = "\
def f(x, items):
    if x:
        a()
    elif check(x):
        return b()
    else:
        raise Oops(x)
    for item in load(items):
        c(item)
    while more():
        pass
    match x:
        case 1:
            d()
";
    let document = analyze("branches", &[("m.py", source)])?;
    let f = unit(&document, "python:m.f");
    let expected = vec![
        Step::Branch {
            guard: Some("x".into()),
            arms: vec![
                arm("yes", vec![call_step("a#0")]),
                arm(
                    "no",
                    vec![
                        call_step("check#0"),
                        Step::Branch {
                            guard: Some("check(x)".into()),
                            arms: vec![
                                arm("yes", vec![call_step("b#0"), exit(ExitKind::Return, None)]),
                                arm(
                                    "no",
                                    vec![
                                        call_step("Oops#0"),
                                        exit(ExitKind::Error, Some("Oops(x)")),
                                    ],
                                ),
                            ],
                        },
                    ],
                ),
            ],
        },
        call_step("load#0"),
        Step::Loop {
            label: Some("for item in load(items)".into()),
            body: vec![call_step("c#0")],
        },
        Step::Loop {
            label: Some("while more()".into()),
            body: vec![call_step("more#0")],
        },
        Step::Branch {
            guard: Some("match x".into()),
            arms: vec![
                arm("case 1", vec![call_step("d#0")]),
                arm("no match", vec![]),
            ],
        },
    ];
    assert_eq!(f.body.as_ref(), Some(&expected));
    Ok(())
}

#[test]
fn lowers_try_and_scopes_propagation() -> anyhow::Result<()> {
    let source = "\
def f():
    try:
        return risky()
    except KeyError:
        recover()
    finally:
        cleanup()
    after()
";
    let document = analyze("try", &[("m.py", source)])?;
    let f = unit(&document, "python:m.f");
    let expected = vec![
        call_step("risky#0"),
        call_step("cleanup#0"),
        Step::Branch {
            guard: None,
            arms: vec![
                arm("ok", vec![exit(ExitKind::Return, None)]),
                arm("except KeyError", vec![call_step("recover#0")]),
            ],
        },
        call_step("after#0"),
    ];
    assert_eq!(f.body.as_ref(), Some(&expected));
    assert!(!call(f, "risky#0").propagates, "caught by the handler");
    assert!(call(f, "recover#0").propagates);
    assert!(call(f, "after#0").propagates);
    Ok(())
}

#[test]
fn keys_count_occurrences_in_source_order() -> anyhow::Result<()> {
    let document = analyze(
        "keys",
        &[("m.py", "def f():\n    g(g(1))\n    h()\n    g(2)\n")],
    )?;
    let f = unit(&document, "python:m.f");
    let keys: Vec<_> = f.calls.iter().map(|call| call.key.as_str()).collect();
    assert_eq!(keys, ["g#0", "g#1", "h#0", "g#2"]);
    // Arguments are evaluated first.
    let order = f.body.clone().unwrap_or_default();
    assert_eq!(order[..2], [call_step("g#1"), call_step("g#0")]);
    Ok(())
}

#[test]
fn registrations_spawns_and_entries() -> anyhow::Result<()> {
    let source = "\
import asyncio
from fastapi import FastAPI

app = FastAPI()

def logged(fn):
    def wrapper(*args):
        return fn(*args)
    return wrapper

@app.get(\"/items/{id}\")
@logged
async def item(id):
    asyncio.create_task(audit(id))

async def audit(id):
    pass

async def boot():
    pass

app.add_event_handler(\"startup\", boot)

if __name__ == \"__main__\":
    serve(app)
";
    let document = analyze("modes", &[("svc.py", source)])?;
    let body = unit(&document, "python:svc.<module>");
    let route = call(body, "app.get#0");
    assert_eq!(route.mode, Mode::Register);
    assert_eq!(
        route.via.as_ref().map(UnitId::as_str),
        Some("python:svc.app")
    );
    assert_eq!(targets(route), [("python:svc.item", Certainty::Exact)]);
    assert!(!route.propagates);
    assert_eq!(call(body, "logged#0").mode, Mode::Call);
    assert_eq!(call(body, "app.add_event_handler#0").mode, Mode::Register);
    assert_eq!(
        call(body, "serve#0").mode,
        Mode::Register,
        "the app is handed over"
    );
    assert_eq!(unit(&document, "python:svc.app").role, Role::Channel);

    let spawn = call(unit(&document, "python:svc.item"), "asyncio.create_task#0");
    assert_eq!(spawn.mode, Mode::Spawn);
    assert_eq!(targets(spawn), [("python:svc.audit", Certainty::Exact)]);

    let entry = |unit: &str, label: &str, kind: &str, chain: &[&str]| Entry {
        unit: UnitId::new(unit),
        label: label.into(),
        kind: kind.into(),
        chain: chain.iter().map(|id| UnitId::new(*id)).collect(),
    };
    assert_eq!(
        document.entries,
        [
            entry("python:svc.<module>", "python -m svc", "main", &[]),
            entry("python:svc.boot", "startup", "lifecycle", &[]),
            entry(
                "python:svc.item",
                "GET /items/{id}",
                "http",
                &["python:svc.logged.<locals>.wrapper"]
            ),
        ]
    );
    Ok(())
}

#[test]
fn demo_service_validates_cleanly() -> anyhow::Result<()> {
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo/python-service");
    let document = codeviz_python::analyze(&demo)?;
    let diagnostics = codeviz_ir::validate(&document);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert_eq!(
        document
            .unit(&UnitId::new("python:shop.services.orders.place"))
            .map(|u| u.calls.len()),
        Some(6)
    );
    Ok(())
}

#[test]
fn names_alone_match_only_distinctive_methods() -> anyhow::Result<()> {
    let document = analyze(
        "by-name",
        &[
            ("app/__init__.py", ""),
            (
                "app/main.py",
                "class Cache:\n    def get(self, key): pass\n    def evict(self): pass\n    def size(self): pass\n\
             class Queue:\n    def size(self): pass\n\
             def run(thing):\n    thing.get(1)\n    thing.evict()\n    thing.size()\n",
            ),
        ],
    )?;
    let run = unit(&document, "python:app.main.run");
    assert_eq!(
        targets(call(run, "thing.get#0")),
        [],
        "a dict's `get` as likely"
    );
    assert_eq!(
        targets(call(run, "thing.evict#0")),
        [("python:app.main.Cache.evict", Certainty::Inferred)]
    );
    assert_eq!(
        targets(call(run, "thing.size#0")),
        [],
        "two unrelated classes"
    );
    Ok(())
}
