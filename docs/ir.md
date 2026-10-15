# codeviz IR — v0.1

The codeviz intermediate representation describes **things that run, what contains
them, and what triggers what**. Producers write it (source analyzers, pipeline
importers, trace importers, people, agents). Core algorithms and the viewer read it.

It deliberately does **not** describe data flow, types, or values.

Status: `0.x`. No stability promise until 1.0. The Rust types in `crates/codeviz-ir`
are the source of truth; `codeviz schema` prints the generated JSON Schema.

## Documents

There are two document types.

- A **static document** is a snapshot of one system at one revision: its units, how
  they invoke each other, and where execution can start.
- A **run document** records observed executions (traces, CI runs) and binds them to
  static documents. Its schema is specified below; tooling does not read it yet.

Journeys (one entry point unrolled into a time-ordered tree of invocations) are never
stored. Core derives them from a static document, or reads them from a run document.

```jsonc
{
  "schema": "codeviz/0.1",
  "producer": { "name": "codeviz-rust", "version": "0.1.0" },
  "root": "file:///home/me/shop",            // optional; base for relative locations
  "revision": { "commit": "9f2c1e0", "timestamp": "2026-10-14T12:00:00Z", "label": null },
  "units": [ /* Unit */ ],
  "entries": [ /* Entry */ ]
}
```

`schema` is `codeviz/<major>.<minor>`. Readers reject a different major version and
warn on a newer minor version.

## Identity

Every unit has a `UnitId`: `<scheme>:<path>`.

- `scheme` is lowercase `[a-z0-9-]+` and names the namespace: `rust`, `python`,
  `buildkite`, `arch`, ….
- `path` is opaque to consumers. Producers choose it, using the domain's own qualified
  names where they exist, and must keep it **deterministic** and **free of line
  numbers**: the same unit must get the same id in every snapshot so documents can be
  compared.
- IDs are case-sensitive and contain no whitespace.

Recommended paths:

| Domain | Example |
| --- | --- |
| Rust | `rust:shop::services::users::find_user`, `rust:shop::models::User::{impl#Debug}::fmt`, `rust:shop::main::{closure#0}` |
| Python | `python:shop.services.orders.place_order`, `python:shop.models.Order.total`, `python:shop.app.make.<locals>.helper` (`__qualname__` style) |
| Buildkite | `buildkite:deploy/build-image` (pipeline slug / step key) |
| Architecture | `arch:orders-service` |

A unit that was renamed or moved lists its previous ids in `aliases` so comparisons
can still match it.

A reference to an id in a **different scheme** than the referring unit is an external
reference: it may resolve when documents are merged, and is not an error when it does
not. Merging takes the union of units by id.

## Units

```jsonc
{
  "id": "rust:shop::services::orders::place",
  "name": "place",
  "kind": "async_fn",                 // open vocabulary, lowercase snake_case; displayed, never interpreted
  "role": "invocable",                // closed: group | invocable | external | channel
  "parent": "rust:shop::services::orders",
  "location": { "uri": "src/services/orders.rs", "range": { "start_line": 9, "start_col": 1, "end_line": 18, "end_col": 2 } },
  "content_hash": { "algo": "codeviz-tokens-blake3", "value": "…" },
  "aliases": [],
  "doc": "Places an order for a user.",
  "attributes": { "visibility": "pub", "is_async": true },
  "calls": [ /* Invocation */ ],
  "body": [ /* Step */ ]              // optional
}
```

Roles:

| Role | Meaning |
| --- | --- |
| `group` | Organizes other units (module, class, package, service boundary, pipeline group). Not invoked. |
| `invocable` | Runs when invoked (function, method, closure, CI step, service endpoint). May also be a parent. |
| `external` | Runs but is not analyzed here: a dependency, a third-party API, a stub for another document. |
| `channel` | Carries triggers without running anything itself: a queue, topic, event bus, webhook. Producers invoke it; consumers are registered on it. |

Unknown roles from newer producers are read as `unknown`.

`parent` builds the single **containment tree** used for grouping and zoom-out. Any
unit may be a parent (a pipeline is invocable and contains its steps). A method's
parent is its type; a closure's parent is the function that defines it.

`kind` is free text from a recommended vocabulary: `module`, `package`, `class`,
`struct`, `trait`, `impl`, `fn`, `async_fn`, `method`, `closure`, `pipeline`, `step`,
`gate`, `service`, `database`, `queue`, `topic`, `endpoint`.

`attributes` holds scalar values only (string, number, bool) for display and filtering.

`content_hash` fingerprints the unit's **own** definition so that "same unit, changed
body" is detectable. Hashes are compared only when their `algo` matches. The
`codeviz-tokens-*` family hashes the unit's own token stream excluding comments and
whitespace, and excluding callee bodies (callee changes surface through `calls`).

## Invocations

`calls` lists the unit's call sites in evaluation order (arguments before the call that receives them).

```jsonc
{
  "key": "users::find_user#0",
  "text": "users::find_user",
  "targets": [ { "unit": "rust:shop::services::users::find_user", "certainty": "exact" } ],
  "dispatch": "one",
  "mode": "call",
  "via": null,
  "propagates": true,
  "location": { "uri": "src/services/orders.rs", "range": { "start_line": 10, "start_col": 16, "end_line": 10, "end_col": 51 } }
}
```

- `key` identifies the call site within the unit and is stable across snapshots:
  `<text>#<n>`, where `n` is the 0-based occurrence among this unit's calls with the
  same `text`. Inserting an unrelated call does not change it.
- `text` is the callee as written.
- `targets` lists what may run. **Empty means unresolved**; `text` still says what was
  called. Each target carries its own `certainty`:
  - `declared` — asserted by a person, agent, or manifest
  - `exact` — resolved by the producer's rules
  - `inferred` — matched heuristically (e.g. by method name)
  - `observed` — seen at runtime
- `dispatch` says how multiple targets combine: `one` (exactly one runs: virtual
  dispatch, ambiguity) or `all` (every target runs: fan-out, pub/sub). Default `one`.
- `mode`:
  - `call` — the caller waits for the target to finish (default).
  - `spawn` — the target starts now and runs concurrently; the caller does not wait.
  - `register` — the target is **not** run now; it is handed to `via` (if known),
    which invokes it later. Route registration, callbacks, event listeners, queue
    subscriptions.
- `propagates` — if the target ends with an error, the caller ends with that error
  here (`?`, an uncaught exception, a failing CI step with no `soft_fail`). Default
  `false`. Panics always propagate.

## Bodies

`body` is an optional control-flow tree over the unit's `calls`. Without it, the unit
runs every call once, in order, then returns. A unit with `calls` and no `body` is
fully valid; producers emit `body` when they know control flow.

```jsonc
"body": [
  { "step": "call", "call": "users::find_user#0" },
  { "step": "branch", "guard": "quantity in range", "arms": [
      { "label": "no",  "body": [ { "step": "exit", "kind": "error", "label": "quantity out of range" } ] },
      { "label": "yes", "body": [] }
  ] },
  { "step": "loop", "label": "for line in lines", "body": [ { "step": "call", "call": "price#0" } ] },
  { "step": "dag", "nodes": [
      { "id": "build", "after": [], "body": [ { "step": "call", "call": "build#0" } ] },
      { "id": "test",  "after": ["build"], "body": [ { "step": "call", "call": "test#0" } ] }
  ] },
  { "step": "exit", "kind": "return" }
]
```

| Step | Semantics |
| --- | --- |
| `call` | Performs the invocation with that `key`. |
| `branch` | Exactly one arm runs. A missing "else" is written as an explicit empty arm. `guard` is the predicate as text; equal guards within one document denote the same predicate, so correlated conditions (every step gated on `branch == main`) do not multiply scenarios. |
| `loop` | The body runs zero or more times. |
| `exit` | The unit ends here. `kind`: `return`, `error`, `panic`. `label` describes why. |
| `dag` | Nodes run concurrently, each starting once every node in its `after` has finished. The step finishes when all nodes have. A `dag` with no `after` edges is plain parallelism. |

Reaching the end of the top-level body is a `return`.

## Entries

Entries are where execution starts.

```jsonc
{ "unit": "rust:shop::handlers::place_order", "label": "POST /users/{id}/orders", "kind": "http",
  "chain": ["rust:shop::middleware::auth"] }
```

`kind` is open (`http`, `cli`, `main`, `schedule`, `webhook`, `ci_trigger`, `test`,
`consumer`). `chain` lists wrappers that run around the unit, outermost first
(middleware, decorators), when the producer cannot express them as calls.

## Locations

`{ "uri": "…", "range": { "start_line", "start_col", "end_line", "end_col" } }`.
`uri` is a POSIX path relative to the document `root`, or an absolute URI
(`https://buildkite.com/…`). Lines and columns are 1-based; columns count UTF-8 bytes.
`range` is optional.

## Validation

`codeviz validate <doc>` checks what JSON Schema cannot.

Errors:

| Rule | Check |
| --- | --- |
| `E001` | Unit ids are unique. |
| `E002` | Unit ids match `<scheme>:<path>`. |
| `E003` | `parent` refers to a unit in this document. |
| `E004` | The containment tree has no cycles. |
| `E005` | Call keys are unique within a unit. |
| `E006` | Every `call` step refers to a key in the unit's `calls`. |
| `E007` | No call key is referenced by more than one `call` step. |
| `E008` | A `branch` has at least two arms. |
| `E009` | `dag` node ids are unique, `after` refers to sibling nodes, and there are no cycles. |
| `E010` | Entry units exist in this document. |

Warnings:

| Rule | Check |
| --- | --- |
| `W001` | A unit with a `body` has calls no `call` step references. |
| `W002` | A same-scheme target does not exist in this document. |
| `W003` | Steps follow an `exit` in the same block (unreachable). |
| `W004` | An entry's unit is not `invocable`. |
| `W005` | A `register` or `spawn` invocation sets `propagates`. |

## Run documents (specified, not yet implemented)

```jsonc
{
  "schema": "codeviz-run/0.1",
  "producer": { "name": "otel-import", "version": "0.1.0" },
  "binds": [ { "scheme": "rust", "revision": { "commit": "9f2c1e0" } } ],
  "runs": [ {
    "id": "trace-4bf92f…",
    "entry": "rust:shop::handlers::place_order",
    "complete": true,                 // false for sampled or partial traces
    "weight": 1.0,                    // 1 / sample rate, for aggregation
    "spans": [ {
      "id": "00f067aa0ba902b7", "parent": null, "links": [],
      "name": "POST /users/{id}/orders",
      "unit": "rust:shop::handlers::place_order",       // optional; a linker may fill it in
      "invocation": null,                                // { "unit", "key" } of the call site, when known
      "start_ns": 0, "end_ns": 1830000,                  // relative to the run's first span
      "status": "ok", "attempt": 1,
      "resource": "orders-7d9f-pod-2",
      "attributes": {}
    } ]
  } ]
}
```

## Deferred

Recorded so later additions do not break the format:

- Several containment hierarchies at once (module vs service vs deployment).
- Instances: one definition that runs N times (CI matrix, `parallelism`, replicas).
- Cross-document entry matching (`fetch("/users/42")` → another service's route).
- A streaming (NDJSON) encoding for very large documents.
