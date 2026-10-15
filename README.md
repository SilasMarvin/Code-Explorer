# codeviz

Explore a Rust codebase's call flow in 3D. Point it at a directory, pick a function
(an Axum handler, say), and follow everything it calls.

```sh
mise install
mise run demo-docs                   # regenerate the demo IR documents
mise run demo                        # 3D view of the Rust demo
mise run demo-merged                 # architecture + Rust code as one system

codeviz analyze <dir> -o app.json    # Rust or Python source → IR
codeviz import pipeline.yml -o ci.json
codeviz validate app.json
codeviz journey app.json             # list entries
codeviz journey app.json -e 'POST /users/{id}/orders' -s 2   # one scenario as text
codeviz view app.json other.json     # merged 3D view
```

Controls: left-drag orbit, right-drag pan, scroll zoom. `←`/`→` step through calls in
time, `↑` jumps to the caller, `↓` to the first callee, `F` focuses the selection,
`Space` plays the journey, `Enter` re-roots on the selection, `Backspace` goes back.

## The model

Producers write an [IR document](docs/ir.md): units (things that run), their
containment, invocations between them, and optional control flow. Core unrolls an entry
point into a **journey** and lists its **scenarios**, the distinct ways it can end. The
viewer lays a journey out with X as time, Y as depth, and Z as lanes (containers by
default). [docs/ir-findings.md](docs/ir-findings.md) records how v0.1 held up against
Rust, Python, a Buildkite pipeline and a hand-written architecture.

## Workspace

| Crate | Role |
| --- | --- |
| `codeviz-ir` | The IR types, JSON (de)serialization, schema and validation. |
| `codeviz-core` | Merges documents; unrolls journeys; enumerates scenarios. |
| `codeviz-rust` | Rust producer (tree-sitter). |
| `codeviz-python` | Python producer (tree-sitter). |
| `codeviz-buildkite` | Buildkite pipeline importer. |
| `codeviz-layout` | Maps a journey onto 3D coordinates. Renderer-agnostic. |
| `codeviz-viewer` | Bevy + egui viewer. |
| `codeviz` | CLI. |

Producers depend only on `codeviz-ir`. Core, layout and the viewer never see a producer.

## Limits of syntactic resolution

There is no type information. Method calls on anything other than `self` are matched
by name across the analyzed source; calls into trait impls of external traits are not
matched by name. Capitalized call targets are treated as constructors and dropped.
Trait-object and generic dispatch, re-exports via `pub use`, and macro-generated items
are not modeled.
