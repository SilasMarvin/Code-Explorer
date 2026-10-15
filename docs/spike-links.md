# Spike: relating units across sources (zoom levels)

The question: how should the IR say that an architecture box, the code that implements
it, and the other sources describing the same system correspond to each other?

## Options tried

| | Mechanism | Demo |
| --- | --- | --- |
| **A** | Inline `refines: [UnitId]` on the summary unit | `demo/spike/a-inline-refines.json` |
| **B** | A `links` section, or a separate links document, with `same_as` and `refines` | `demo/links/orders-platform.links.json` (written by an agent from the spec alone) |
| **C** | `same_as` only, with no authored summaries; zoom out by collapsing spans into containers | `demo/spike/c-same-as.links.json` |

Try them:

```sh
# A or B: summary vs detail
codeviz journey demo/architecture/orders-platform.json demo/out/axum-server.json \
  demo/out/python-service.json demo/links/orders-platform.links.json \
  -e arch:api-gateway/post-users-id-orders --zoom detail

# C: collapse modules, or the whole service via same_as
codeviz journey demo/architecture/orders-platform.json demo/out/axum-server.json \
  demo/spike/c-same-as.links.json -e rust:axum_server::handlers::place_order \
  --collapse rust:axum_server::services::pricing --collapse rust:axum_server::store
```

The viewer has a "Zoom in/out" button on refinable spans, a collapse button for each
of a span's containers, and a "Zoom into everything" toggle.

## Results

**A and B are one mechanism.** Core treats an inline `refines` as a `refines` link, so
the journeys are identical. Zooming into `orders-service POST /users/{id}/orders`
replaces the architecture summary with the Rust implementation, with nothing
double-counted. The gateway stays at architecture level. Scenarios switch to the code's
own exits (`.ok_or fails`, unknown SKU) instead of listing both levels. The only
difference is where the link lives, and the link-author agent preferred a separate
document: "neither the architecture author nor the code analyzer should own the
mapping, and loading or omitting the file is a clean switch."

**C is complementary, not an alternative.** Collapsing works well inside one source
(`pricing` and `store` merge into single spans), and `same_as` lets the Rust crate
collapse into `arch:orders-service`. But collapse can only merge what the code
contains. It can't produce the architecture's view of gateway, Kafka and users-service,
because the code doesn't have those things. Collapse is zoom within a source;
refinement is zoom across sources. Keep both: collapse needs no authoring.

## What we learned

1. **Zooming in loses information.** The summary says orders-service calls
   users-service, Redis, Postgres and Kafka. The code level shows none of it: the demo
   uses an in-memory store, and in real code those calls go through library calls the
   producer can't resolve. The detail level should add to the summary, not erase it.
   The gap between the levels is architecture drift, but it's only visible by
   comparing the two views by hand.
2. **Most of what people want to say isn't equivalence.** The link author's list:
   - a pipeline step *deploys* a service;
   - `Store::insert_order` *persists to* `postgres/orders-db`;
   - a code call is the same dependency as an architecture HTTP edge;
   - a gateway route *proxies to* an endpoint.

   Forcing these into `same_as` or `refines` would corrupt containment and zoom.
3. **Links documents need cross-document validation.** A typo in a links-only document
   passes, because the document defines none of the schemes it references.
4. **Entries don't merge.** `POST /users/{id}/orders` is listed once per document, even
   where `refines` says they're the same endpoint.
5. **The spec left open:**
   - `same_as` direction (it should be irrelevant);
   - `refines` vs `same_as` for a summary with no body (use `refines`);
   - which granularity to link (the endpoint ↔ handler, not deeper).

## Recommendation

Adopt **B** with inline `refines` kept as shorthand, and keep **C**'s collapse as the
zoom that needs no authoring. Then extend in this order:

1. **A third link kind, `relates`,** with a free `label` (`deploys`, `persists_to`,
   `proxies_to`). Core gives it no semantics: no zoom, no containment. The viewer draws
   it as a cross-link. This covers finding 2 without weakening the two kinds that drive
   behavior.
2. **Show summary edges at detail level.** When zoomed in, the summary's interactions
   that the detail doesn't reproduce (users-service, Postgres, Kafka) stay visible as
   ghost edges, flagged as drift when nothing in the detail `relates` to them. This turns
   finding 1 into the conformance check.
3. **`codeviz validate --with <docs>`** to check link endpoints against the documents
   they name.
4. **Merge entries** that a `refines` or `same_as` link connects.

The risk is that `relates` turns links into a general-purpose graph database. The line
to hold: only `same_as` and `refines` change what core computes. Everything else is an
annotation.
