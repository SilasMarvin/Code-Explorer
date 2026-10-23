# codeviz

Explore a Rust codebase's call flow in 3D. Point it at a directory, pick a function
(an Axum handler, say), and follow everything it calls.

```sh
mise install
mise run demo-docs                   # regenerate the demo IR documents
mise run demo                        # 3D view of the Rust demo
mise run demo-merged                 # architecture + Rust code as one system
mise run demo-diff                   # the Rust demo after a change, diffed

codeviz analyze <dir> -o app.json    # Rust or Python source → IR
codeviz analyze <dir> --resolve syntax   # skip rust-analyzer: fast, guesses method calls
codeviz import pipeline.yml -o ci.json
codeviz validate app.json
codeviz journey app.json             # list entries
codeviz journey app.json -e 'POST /users/{id}/orders' -s 2   # one scenario as text
codeviz view app.json other.json     # merged 3D view
codeviz view new.json --before old.json   # what changed, over the graph
codeviz diff main...HEAD             # a branch as its pull request shows it
codeviz diff HEAD -C crates/foo      # uncommitted changes in one crate
codeviz diff v1..v2 --summary        # print the changes instead
```

The codebase floats as a graph, one node per function, each module gathered in its own
ring. Click a node and the camera orbits it, and its journey opens docked as a flame
graph: what it calls directly first, numbered `1`, `2`, … on its bars and on the graph's
labels alike. `,`/`.` show a level less or more, a bar's `+N` opens the calls folded
under it, and `V` switches to showing calls only as the cursor reaches them. A closure
counts as part of the function around it.

Each color says one thing. A link's color says which way a call runs: violet into the
focus, blue out of it, with dots flowing along it. A node's fill says where it lives,
its module, the same color as its ring; under a diff it says what changed instead. A
journey bar is filled exactly like its node, and its number sits on a badge of that
color beside the node, so a bar finds its node at a glance; hovering a bar draws a line
to it. `I` picks out everything that calls the focus and `O` everything it calls,
pulling the camera back only as far as it takes to show them all.

Digits walk to the focus's nth call, shift+digit to its nth caller, `-` and `=` reveal
less or more around a starting point, the focus, a diff's changes or the active
journey (`L` switches), measured in calls, distance in the graph, or a blend of both
(`G` switches); what is out of reach dims, then shrinks to a speck. `Q` jumps between
everything, two steps and one. In the journey, `←`/`→` move between calls by the same
caller, `↓`/`↑` go in and out, `[`/`]` step through time. `/` searches for a function,
`S` opens the side panel (the view's settings, outcomes, source and details), `H`
lists every key.

## Diffs

`codeviz diff` checks out each revision with `git archive`, analyzes both, and draws
the difference over the new graph, saying on stderr how far it has got. It compares
function by function: closures count as part of the function that defines them, and a
function that moved or was renamed (found by the IR's `aliases`, an equal content hash,
or the same kind, parent and callees) is one node, not a removal and an addition.
Added functions are green, rewritten ones yellow, removed ones red ghosts and moved
ones grey; new calls are green arrows, removed calls red dashes.

The changes are put in an order to read them: each after the changes it calls, through
unchanged functions too, so you meet what a function now relies on before the function
itself. A caller of the change just read comes next when it can, so a chain reads
bottom up as one, then a change beside it in the same container, then the one more
entry points reach. The diff opens on the first; `N` steps to the next and `Shift N`
back, the camera following. A pane down the left shows the focused change: where it
comes in the order (`4 of 11`), the entry points it affects, the calls it gained and
lost, and its source before and after, with long unchanged runs folded and the part of
a rewritten line that changed picked out; below it, every change in order, `↑3` for
the entry points each affects. In the graph, pale arrows lead up from the focused
change to the nearest entry points it affects (without declared entries, the functions
nothing calls), each named as such. The Changes card lists the entry points the whole
diff affects.

A diff opens showing only what changed, the focus, the journey's root and the way up
from the focus; `C` (or `--show-unchanged`) brings back everything else. `D` switches
to **Morph**, each version laid out alone, animating between them; `T` pauses.
The graph draws functions alone, so the card also lists what changed besides, file by
file: lines outside every function, each named by what holds it (`pub struct Order`,
`const LIMIT: usize`, `mod tests`, `imports`), and files the analysis never read,
such as manifests and docs. Without git, as under `view --before`, it lists the types
and modules the IR says changed. `codeviz diff --summary` prints the same review as
text: the entry points affected, the changes in reading order with how many entry
points each affects, and what changed that the graph does not draw.

## Readability

`eval/readability` measures how much of a view a reader gets right from its screenshot
alone. Each scenario in `scenarios.json` renders bare (`--bare`: the graph and the
journey, nothing else), and `--screenshot` writes `shot.truth.json` beside the image:
the focus, every disclosed call with its number and module, whether it can fail or only
sometimes runs, how many calls it folds away, and what a diff changed. `--keys` presses
keys first (`--keys "i"`, `--keys ". . o"`), so a scenario can show a view you reach by
hand. From that, `readability.py` writes questions with their answers (call order,
nesting, folding, failures, conditions, callers, which module a call lives in, changes)
and grades what readers answer, by aspect, against the previous run.

The `readability` workflow (`.claude/workflows/readability.js`) does the whole loop with
agents as readers: render, a Sonnet and a Haiku reader per scenario who see only the
image, then the report. `mise run readability-render` and `mise run readability-grade` run the two
halves by hand.

## The model

Producers write an [IR document](docs/ir.md): units (things that run), their
containment, invocations between them, and optional control flow. Core unrolls an entry
point into a **journey** and lists its **scenarios**, the distinct ways it can end. The
viewer floats every function in a force-directed graph and draws the journey through
the focus as a flame graph over it. [docs/ir-findings.md](docs/ir-findings.md) records how v0.1 held up against
Rust, Python, a Buildkite pipeline and a hand-written architecture.

## Workspace

| Crate | Role |
| --- | --- |
| `codeviz-ir` | The IR types, JSON (de)serialization, schema and validation. |
| `codeviz-core` | Merges documents; unrolls journeys; enumerates scenarios; diffs versions. |
| `codeviz-rust` | Rust producer (tree-sitter, calls resolved by rust-analyzer). |
| `codeviz-python` | Python producer (tree-sitter). |
| `codeviz-buildkite` | Buildkite pipeline importer. |
| `codeviz-layout` | The codebase map, the force-directed graph, journey layout. Renderer-agnostic. |
| `codeviz-viewer` | Bevy + egui viewer. |
| `codeviz` | CLI. |

Producers depend only on `codeviz-ir`. Core, layout and the viewer never see a producer.

## Call resolution

Rust calls are resolved by `rust-analyzer scip`, which type-checks the workspace: a
method call goes to the method that actually runs, a call into std or a dependency goes
nowhere, and a call through a trait object or generic bound goes to every
implementation in the analyzed source. It builds build scripts and proc macros first,
so it takes seconds where the syntax takes a fraction of one; `codeviz diff` points the
checkouts at the working tree's target directory so they are not built from scratch.
`mise install` installs it. Without it, or with `--resolve syntax`, and for any call it
leaves unresolved (code outside the Cargo workspace, or in a crate that does not build),
calls are resolved from the syntax alone: module paths, `use` imports, `self` and
declared types are followed, and a method call on a value of unknown type is matched by
name and marked `inferred`. Capitalized call targets are treated as constructors and
dropped either way.
