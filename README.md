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
ring. The journey is what you follow: a flame graph, docked, of everything its root
calls, what it calls directly first, numbered `1`, `2`, … on its bars and on the graph's
labels alike. `,`/`.` show a level less or more, a bar's `+N` opens the calls folded
under it, as does the cursor wherever it lands, and `V` switches to showing calls only
as the cursor reaches them. A closure counts as part of the function around it.

Each color says one thing. A link's color says which way a call runs: violet into the
focus, blue out of it, with dots flowing along it. A node's fill says where it lives,
its module, the same color as its ring; under a diff it says what changed instead. A
journey bar is filled exactly like its node, and its number sits on a badge of that
color beside the node, so a bar finds its node at a glance; hovering a bar draws a line
to it. `I` picks out everything that calls the focus and `O` everything it calls,
pulling the camera back only as far as it takes to show them all.

The focus is where you are in the journey: the node under its cursor, ringed in white
and named in the top bar beside how many calls it makes and takes. The arrows, a digit
(the call with that number on its badge and bar) and a click on a node the journey runs
all move the cursor, opening whatever folds it, and never restart the journey. A click
on a node off the journey makes it the focus for now, with the journey dimmed behind it
and `not on this journey · Enter to follow` beside its name; an arrow or `Esc` goes back
to the cursor. `I`, `O` and `Enter` act on the focus; `Enter` follows it, starting the
journey there, and `Shift`-click follows a node at once. Hovering a node rings and names
it without moving the focus, so the colors stay put.

`-` and `=` reveal less or more around a starting point, the focus, a diff's changes or
the journey (`L` switches), measured in calls, distance in the graph, or a blend of both
(`G` switches); what is out of reach dims, then shrinks to a speck. The dial runs from
one step out to the farthest node, which is everything, so each press moves one ring; a
slim meter at the graph's right edge shows where it stands and what it measures from
for a couple of seconds after `-`, `=`, `L` or `G`, and whenever the pointer is over it.
In the journey, `←`/`→` move between calls by the same caller, `↓`/`↑` go in and out,
`[`/`]` step through time. `/` searches for a function, `S` opens the side panel (the
view's settings, outcomes, source and details), `H` lists every key, and `Esc` closes
whichever of those is on top, or the spotlight.

`Z` puts call order on a second floor (`--time floors` opens with it; the top bar says
which). The map is the ground floor and no node moves: a faint glass deck lifts over the
journey's part of the map, towards the eye, and the camera tilts gently to show both
floors. On the deck the disclosed journey is its own clean diagram, a flame graph
standing on its root: time runs left to right, one column per call, and each call is a
tile in its node's color, a row above the call that makes it and as wide as the calls it
makes, with a lit ball at its start and its number and name beside it, level along the
tile where it fits and slanting up from the ball where it doesn't. The cursor's tile is
outlined and its ball white; other calls to the cursor's function are outlined too. The
cursor's call, the calls it makes (the first 12, if it makes more) and other calls to
its function hang strings down to their nodes, leaving the deck and reaching each node
straight down: the cursor's white and bold, ending in a ring round its node, its calls'
one bright line each in their node's color, so a function called three times under it
has three strings meeting at its node. Every other call is matched by its number badge
on the map (`--time floors-all` hangs faint strings from them too); over a crowd of
nodes the cursor's string lands on its node's name, pulled up off the crowd on a short
thread. Where the cursor's node and the node of the call just before or after it sit
close together in a crowd of the map's nodes, a small lens at the map's lower left shows
that patch magnified, those calls numbered in their colors and the cursor's in white,
ringed on the map with a leader to the lens. A long journey shows a window of 21 calls
round the cursor, with how many more lie either side, and names the calls it opens
inside of off the deck's left edge. Pointing at a tile lights its string, pool and
node's name, and clicking it moves the cursor there; pointing at a node on the map
outlines its tiles and lights their strings. Depth cues carry the gap between the
floors: the map lies on a faintly gridded floor that fades out at its rim, each shown
call's node sits in a soft pool of its color, brightest for the cursor's, haze dims what
is farther from the eye, and the view rocks a few degrees from side to side, so the deck
slides against the map (`Shift Z` holds it still; `--rock-at 0.25` and `0.75` hold a
screenshot at either end). While the deck is up the map steps back: links draw fainter;
the focus and the nodes the strings land on are named, every other node the deck shows
keeps its number badge, so tile `5.3` finds badge `5.3`, nudged just above the docked
journey with a short leader where the journey covers its node, and only the modules the
journey runs through are named. Two alternatives, kept for comparison: `--time
floors-behind` stands the deck beyond the journey's far edge, rising, so its strings
fall forward onto the map; `--time floors-comb` runs the strings down to the deck's near
edge and through a comb that orders them as their nodes lie left to right before they
fan out (`floors-behind-comb` does both).

Every new start, by `Enter`, a `Shift`-click, a search result, a change, an outcome or
`Start here`, is remembered: `Backspace` (or `‹` in the top bar) goes back to the view
before it, cursor, outcome, depth, opened calls and spotlight included, and
`Shift+Backspace` (or `›`) forward again. Moving the cursor is not remembered. The
camera orbits where the journey starts and turns only when you start somewhere new;
stepping through the journey at most pulls it back to keep the cursor's node in sight.
Pan or zoom and it stays where you put it, saying `camera free · F to follow` until `F`
brings it back.

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
`--show-unchanged`) brings back everything else. Focusing a node shows its old and new source as a line diff.

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
