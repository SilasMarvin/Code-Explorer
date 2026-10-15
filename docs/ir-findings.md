# IR v0.1 field test

Four producers wrote v0.1 documents. One core and one viewer read all of them, with no
code specific to any domain.

| Document | Producer | Units | Entries | Validation |
| --- | --- | --- | --- | --- |
| `demo/out/axum-server.json` | `codeviz-rust` (tree-sitter) | 47 | 5 | clean |
| `demo/out/python-service.json` | `codeviz-python` (tree-sitter) | 62 | 7 | clean |
| `demo/out/deploy-pipeline.json` | `codeviz-buildkite` (YAML) | 27 | 1 | clean |
| `demo/architecture/orders-platform.json` | an agent, from `docs/ir.md` alone | 35 | 7 | clean on the first run |

Regenerate with `codeviz analyze`, `codeviz import`, and `codeviz validate`. Explore
with `codeviz journey` (text) or `codeviz view` (3D).

## What held

- **Units, containment, invocations and an optional body** fit every domain. None of the
  producers needed a new concept to describe what runs and what triggers it.
- **Scenarios work.** The Rust `POST /users/{id}/orders` has the four outcomes you get
  from reading the code: it completes, the user is not found (via `.ok_or`), the
  quantity is bad, or the SKU is unknown. Each scenario's journey stops at its exit.
- **`register`** links route registration to its handler: `main → router → handlers`.
- **`dag` + `after`** handle Buildkite fan-out, fan-in and `wait` barriers.
- **Merging by id works across documents.** Loading the architecture document together
  with the Rust document gives one journey: gateway → orders-service → into the Rust
  handler → back out to Postgres, Redis and Kafka. Scenarios from both levels combine.

## What broke, ranked

1. **No refinement between levels.** The architecture's `orders-service POST /orders`
   and the Rust `handlers::place_order` describe the same behavior at different zoom
   levels. Cross-document links only exist as calls, so the merged journey runs the
   code and then replays the architecture's summary of it. "User not found" shows up
   twice, once per level. This relation is what zooming out is.
   *Change:* `Unit.refines: Vec<UnitId>` (coarse → fine). When both are loaded, the
   coarse unit's body is a summary of the fine one, and the viewer switches between them.
2. **Caught failures cannot be expressed.** `propagates: bool` can't say "propagates
   unless it is an `AppError`" (the Python handler), and "an upstream 404 becomes a
   local 404" can't be said either. Exits have only free-text labels.
   *Change:* a `try` step (`body`, `handlers: [{ matches, body }]`, `finally`), and an
   optional `error` code on `exit` that handlers match against.
3. **Failure inside concurrency is undefined.** In Buildkite a failed step lets its
   siblings finish, skips the steps that depend on it, and fails the build at the end.
   Core instead ends the pipeline immediately. `continue_on_failure` and
   `allow_dependency_failure` can't be expressed. An `exit` inside a `dag` node (a
   blocked gate) has no defined scope.
   *Change:* define `dag` failure that way, add `after: [{ node, on: success | failure | always }]`,
   and say what an exit inside a node ends.
4. **Implicit failure floods scenarios.** Any Python call can raise, so `propagates` is
   true almost everywhere, and scenarios list "`self._users.get` fails" next to real
   `raise` sites. The spec also never says whether a call that resolves to an
   `external` unit can fail, so the Buildkite scripts became infallible once resolved.
   *Change:* split explicit exits and declared fallibility from "may fail like any
   call"; rank or collapse the implicit kind; state that propagating calls to units
   with no body may fail.
5. **Guards correlate too broadly.** `__name__ == "__main__"` reads the same in every
   Python module but means something different in each. Entries also can't pin a guard's
   value: a push to `main`, or `python -m shop.main`.
   *Change:* guards correlate only within a unit unless they share an explicit
   `predicate` id; add `Entry.assume: { guard: arm }`.
6. **Instances.** The Buildkite matrix (2×2), `parallelism: 4` and 3 service replicas are
   each one box. Both the pipeline and the architecture test hit this.
   *Change:* move `instances` out of the deferred list.
7. **Invocations carry no metadata.** HTTP, SQL and Kafka calls all look the same, and
   status codes live only in labels.
   *Change:* `Invocation.attributes`.
8. **The validator is too lenient.** Unknown enum values are read as `unknown`, so a
   typo like `"mode": "spwan"` passes. `via` and `chain` references aren't checked.
   *Change:* warn on unknown values and dangling references; add one worked example per
   domain to the spec.
9. **Smaller gaps:**
   - The spec says `calls` are in source order, but both code producers emit
     evaluation order (`f(g())` → `g, f`). The spec should say evaluation order.
   - A synchronous callback (`sorted(key=…)`) has no mode; `register` means "later".
   - Channel-to-channel forwarding (`include_router`) is undefined.
   - Python import-time code needs a convention (a `<module>` invocable unit).
   - `loop` has no min/max/until for retries.
   - `{closure#n}` and `<lambda#n>` ids shift when an earlier closure is inserted.

## Verdict

The model holds. Every change above can be added to the current schema except 2 and 4,
which revise what `propagates` means. Change 1 is the zoom-out feature, not a patch.

The risk going forward is the IR turning into a model of language semantics. The line
to hold: model only what changes a journey or a scenario. Typed handlers change
scenarios, so they belong in the schema. Import-time execution doesn't change a
journey's shape, so it stays a convention.
