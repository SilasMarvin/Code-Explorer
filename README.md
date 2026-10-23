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
codeviz view crates/foo              # analyze and view; kept until the source changes
codeviz view new.json --before old.json   # what changed, over the graph
codeviz view app.json --at src/orders.rs:42   # open on the function at that line
codeviz view app.json --resume       # as you last left it
codeviz diff main...HEAD             # a branch as its pull request shows it
codeviz diff HEAD -C crates/foo      # uncommitted changes in one crate
codeviz diff v1..v2 --summary        # print the changes instead
```

The codebase floats as a graph, one node per function, each module gathered in a ring
of its own that no other overlaps; with several crates, each crate's modules gather
inside a wide grey ring named for the crate. It opens on the whole graph, head-on,
with the busiest functions named and each entry point flying a small flag (`▶` on its
label), and a card of where to start: the entry points that reach the most of the
codebase first, those in demo, example and test code last, or without declared
entries the functions nothing calls. Enter opens the first, `W` or `Esc` hides the
card. `--entry` or `--focus` opens straight on a journey instead.

Click a node and the camera orbits it, and its journey opens docked as a flame graph:
what it calls directly first, numbered `1`, `2`, … on its bars and on the graph's
labels alike. `,`/`.` show a level less or more, a bar's `+N` opens the calls folded
under it, as does the cursor wherever it lands, and `V` switches to showing calls only
as the cursor reaches them. A closure counts as part of the function around it.

A function goes by one name everywhere, on its node, its bar, in search and in the
details: its own, with its type, then module, then crate in front only as far as it takes
to tell it from whatever else is in view, so `new` reads `System.new` only while another
`new` shows. A closure goes by the function around it (`draw_ui › closure 3`), Python's
`<module>` by its module's name, and a ring by its module's name, with its crate when
another crate has a module of that name. Hovering a bar shows the full path.

Each color says one thing. A link's color says which way a call runs: violet into the
focus, blue out of it, with dots flowing along it. A node's fill says where it lives,
its module, the same color as its ring; under a diff it says what changed instead. A
journey bar is filled exactly like its node, and its number sits on a badge of that
color beside the node, so a bar finds its node at a glance; hovering a bar draws a line
to it.

`I` picks out what calls the focus and `O` what it calls, pulling the camera back only
as far as it takes to show them all. Pressed again, each reaches two calls out, then
all the way, then lets go; shift goes all the way at once. All the way up is every
route into the focus: from the entry points where requests come in, or the functions
nothing calls (which a framework or a test runs), down to it, drawn as pale arrows with
dots running towards it, with the rest of the graph stepped out of the way. A route
stops at the first entry point it meets, and a router that registers a handler counts
as depending on it without leading anywhere above. The top bar says how many entry
points reach the focus; with its callers picked out, a card lists them, and picking one
(or `A` for the next, `Shift A` the one before) opens that entry's journey with the
cursor on the focus and every call on the way disclosed, the route it follows drawn
brighter than the rest. When the spotlight picks out more than fifteen, a card lists
every one, grouped by module, and the graph names the first where it finds room.

A dotted, fainter link is a guess: the analyzer matched the call by name without
knowing the type (an `inferred` target in the IR). Guesses keep their direction color
but are left out of the caller and call counts, of `I` and `O`, and of what lights up
around the focus; `U` (or the side panel's checkbox) counts them too. The legend shows
the dotted line only when the view has guesses.

The focus is the node under the journey's cursor, ringed in white and named in the top
bar beside how many calls it makes and takes; `I`, `O` and `Enter` (start the journey
there) act on it. Hovering a node rings and names it without moving the focus, so the
colors stay put. Digits open the call with that number on its badge and bar, `-` and
`=` reveal less or more around a starting point, the focus, a diff's changes or the
active journey (`L` switches), measured in calls, distance in the graph, or a blend of
both (`G` switches); what is out of reach dims, then shrinks to a speck. `Q` jumps
between everything, two steps and one. In the journey, `←`/`→` move between calls by
the same caller, `↓`/`↑` go in and out, `[`/`]` step through time. `S` opens the side
panel (the view's settings, outcomes, source and details), `H` lists every key, and
`Esc` closes whichever of those is on top, or the spotlight.

Every move, opening a node, a numbered call, a search result, a change, an outcome or
a new start, is remembered: `Backspace` or `Alt+←` (or `‹` in the top bar) goes back
to the view before it, cursor, outcome, depth, opened calls and spotlight included, and
`Alt+→` forward again. The camera orbits where the journey starts and turns only when
you move somewhere; stepping through the journey at most pulls it back to keep the
cursor's node in sight. Pan or zoom and it stays where you put it, saying `camera free
· F to follow` until `F` brings it back.

`/` searches functions by name or path, entry points by route, and what function bodies
say: where they fail or panic (an error's message or variant, `unknown sku`,
`NotFound`), the conditions they branch on, and calls into code outside the analysis
(`.ok_or`, `tokio::spawn`). Picking an error opens the function that raises it on the
outcome that ends there, with every route into it from the entry points picked out;
picking a condition or a call opens its function the same way. Functions come before
what they say, each named as the graph names it: the best match first, then the one
nearest the focus, then the most called. A closure is found by what it says, under the
function around it. `↑`/`↓` move between results and Enter opens one. `--search <QUERY>` opens with the box showing what a query
finds.

## From the graph to the code

`B` shows the focused function's source in a card over the graph's corner, colored by
syntax; each line that makes one of the journey's calls carries that call's number on
a badge filled like its node. Its location is a link: clicking it, or pressing `E`,
opens the function in your editor. The editor is `$CODEVIZ_EDITOR`, a command with
`{file}`, `{line}` and `{column}` in it (`code -g {file}:{line}` by default), else
`$VISUAL` or `$EDITOR` when it is an editor with a window of its own (Zed, Sublime,
Cursor, a JetBrains IDE…), else VS Code, else a terminal editor in the terminal
codeviz was started from. The card and the side panel copy a function's id or its
`path:line`. `P` saves a picture of the view to the desktop (`$XDG_PICTURES_DIR` or
`~/Pictures` elsewhere) and copies it. A line at the top says what each did.

`Y` copies the command that opens the view as it stands, the documents (or the `diff`)
and the flags for the focus, the outcome, the cursor's step, the depth, the spotlight
(`--spotlight callers|callees`), the reveal, the layout and the source card
(`--peek`), to paste for a teammate or into a note; the camera is left to frame it. The
same flags are kept as the last view of each set of documents (under
`$XDG_STATE_HOME/codeviz`, or `~/Library/Application Support/codeviz` on a Mac), and
`--resume` opens it again, with any flags given alongside winning.

`codeviz view DIR` analyzes the directory and opens it, taking `analyze`'s `--lang`,
`--resolve` and `--include-tests`. The analysis is kept (under `$XDG_CACHE_HOME/codeviz`,
or `~/Library/Caches/codeviz` on a Mac) for the commit the directory is at plus any
uncommitted changes, so opening it again before the source changes skips
rust-analyzer.

`--at FILE:LINE` opens on the function whose source spans that line. A document
remembers the directory it was analyzed in; opened somewhere else, on a teammate's
machine say, it looks for that directory around the document and the working
directory, and `--root DIR` says where it is.

## Diffs

`codeviz diff` checks out each revision with `git archive`, analyzes both, and draws
the difference over the new graph. It compares function by function: closures count as
part of the function that defines them, and a function that moved or was renamed
(found by the IR's `aliases`, an equal content hash, or the same kind, parent and
callees) is one node, not a removal and an addition. `D` cycles three views:

- **Status**: added (green), rewritten (yellow), removed (red ghosts), moved (grey); new
  calls as arrows, removed calls dashed.
- **Impact**: everything that can reach a change, hotter the closer, out to the entry
  points it affects (or, without declared entries, the functions nothing calls).
- **Morph**: each version laid out alone, animating between them; `T` pauses.

A diff opens showing only what changed, the focus and the journey's root; `C` (or
`--show-unchanged`) brings back everything else. Clicking a node opens its old and new
source as a line diff. `--save DIR` keeps both documents, with a copy of the source
each checked-out revision points at, to open later with `codeviz view`.

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
image, then the report. `mise run readability-render` and `mise run readability-grade`
run the two halves by hand. With `CODEVIZ_BIN` set, rendering uses that binary instead
of building one.

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
calls are resolved from the syntax alone: module paths, `use` imports and re-exports,
`self`, declared types, `Deref` impls, the types functions return, what options,
collections and iterators hold (through `iter`, `filter`, `get`, `unwrap`, `for` and
`if let`), the bindings closures capture and the arguments a function declares its
closure parameters are called with are followed. A method call on a value of unknown type is matched by name only when the
name is distinctive: not a common standard library method (`map`, `push`, `len`, …), and
defined by one type (or one trait's implementations) in a crate the caller depends on.
Such a match is marked `inferred`, as is a call that could be any of several overloads
(`From<bool>` or `From<u8>`). Capitalized call targets are treated as constructors and
dropped either way. `analyze` and `diff` print how many call sites resolved exactly, how
many by a guess, and how many not at all, and warn when rust-analyzer is missing or
fails. `diff` analyzes its two sides one after the other under rust-analyzer (two at
once save little and need over a gigabyte more), side by side otherwise.

`eval/resolution/compare.py` measures one resolution against another, per call site:
analyze the same source with `--resolve syntax` and `--resolve rust-analyzer` and it
prints the syntax's precision and recall, by certainty, and the functions with the most
callers in each. On this repository the syntax gets 99.4% of its edges right and finds
99.1% of rust-analyzer's; on egui and clap_builder, 98.6% and 95% right, finding 95%
and 90%.
