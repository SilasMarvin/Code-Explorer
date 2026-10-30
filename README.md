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
codeviz view app.json --z nucleus    # the focus's module in the middle, the rest round it
codeviz view app.json --z nucleus --expand   # inside the focus's module
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
focus, blue for the focus's own calls out, with dots flowing along them, and amber for
every call further down (the journey's, once it is open); calls further up are pale. A node's fill says where it lives,
its module, the same color as its ring; under a diff it says what changed instead. A
journey bar is filled exactly like its node, and its number sits on a badge of that
color beside the node, so a bar finds its node at a glance; hovering a bar draws a line
to it. `I` picks out everything that calls the focus and `O` everything it calls,
pulling the camera back only as far as it takes to show them all. `Shift+I` hides the
focus's callers instead, and what calls those, with their arrows, and `Shift+O` its
callees; press again to bring them back. On the flat graph they stop lighting up and
fade with the rest; gathered into the nucleus, `Shift+I` takes its middle's callers
out of the view and `Shift+O` the whole journey.

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
view's settings, outcomes, source and details), `H` lists the keys that do something in
the view open (flat, nucleus or diff), in columns that fit the window and scroll past
its height, under a line saying what each color means, and `Esc` closes whichever of
those is on top, or the spotlight.

Every move made by hand is remembered: a click on a function, on or off the journey, a
journey bar or a digit, and every new start, by `Enter`, a `Shift`-click, a search
result, a change, an outcome or `Start here`. `Backspace`, or `← Back` at the left of
the top bar, goes back to the view before it, the function clicked, cursor, outcome,
depth, opened calls and spotlight included, and `Shift+Backspace` (or `Forward →`)
forward again; both buttons are always there, greyed out with nowhere to go. Stepping
the cursor with the keys, and changing the depth, are not moves of their own: going
back puts the cursor and depth back as they were when you left. The
camera orbits where the journey starts and turns only when you start somewhere new;
stepping through the journey at most pulls it back to keep the cursor's node in sight.
Pan or zoom and it stays where you put it, saying `camera free · F to follow` until `F`
brings it back.

## Nucleus

`Z` (or `--z nucleus`, which opens on it) gathers the graph round one function, the
middle: the function whose journey is open, in white, or the one last clicked. Each
module is a sphere, and space says only where things are. The middle's module is a
large glass sphere with every one of its functions inside, where its own corner of the
flat map has them.

One rule says what is named. Shown are the middle; every function that calls it,
violet; and every call of the open journey from the middle down to the disclosed depth,
the depth the journey panel shows (`,` and `.`, the panel's `−`/`+`, `--depth N`; the
depth counts from the middle, so when the middle is the journey's start it is exactly
what the panel shows; disclosing through time, `V`, what the cursor has reached). Each
journey call is drawn as its own arrow, from caller to callee along the journey's tree:
blue for the middle's own calls, amber for every call further down, on arrows, name
borders and names alike. Each function the journey runs carries its number as the
panel numbers it, on a badge filled like its node and bar; one the journey runs more
than once shows once, as `1.2 +2`. Nothing else is named or drawn, but the function
under the pointer and the ends of its calls. Past 60 functions named, only the middle,
the cursor and what is within two calls of the middle keep their names; the rest are
named under the pointer. `Shift+I` leaves the callers out and `Shift+O` the journey;
`N` leaves out the faint specks, the middle module's functions nothing names.

A module holding anything named comes into a ring round the nucleus, a small glass
sphere opened just far enough to name those functions, `3 of 9` saying how much of the
module that is; one reached only through another module sits just beyond it. Every
other module is a small dark sphere further back, in the direction a fixed 3D layout of
the modules (modules that call each other more sit closer) has it, so a module keeps
its bearing as the middle moves. A thin arrow joins two modules whenever a named call
runs between them, colored as its call nearest the middle, and the calls run beside it
side by side, function to function. A new depth brings modules onto the ring or lets
them go, easing every sphere to its new place. The pointer has to rest on a function
for a moment (0.15 s) before the nucleus takes it as pointed at, so passing over a
speck does not flash its calls.

Stepping through the journey (the arrows, `[`/`]`, their `Shift` forms, a journey bar,
`Space`) moves nothing but the cursor: its function is filled pink and ringed twice in a
pink glow, pulsing for a moment where it lands, its name raised over the others on a
larger, pink-edged pill, the journey's call into it drawn pink, and its bar filled pink
with the cursor's white outline and a pink line up to the node. Pointing at a bar, or
at a function, previews it in white: a glow on its node and its call brightened. A cursor the depth
does not reach shows just under its module's sphere. A click on a function, `Enter`,
or a `Shift`-click makes a new middle; one in another module eases every sphere to its
new place, and `Backspace` eases them back. `Q` and `E` turn the view an eighth of the way round, refitting it, so the
depth shows; `F` comes back to the usual three-quarter view, and `Z` again lays the
graph flat.

`X`, or a double click on the nucleus away from its functions, expands it (`--expand`
opens straight into it): the sphere grows to several times its size while the camera
comes in, so you go inside the same object. It names exactly what the module view
names. Inside, the module's functions are laid out by the calls among them: what is
named gathers in the middle, the callers tending to one side and the journey to the
other (a tendency, not an axis); the rest are faint specks out towards the wall, left
out until `N` brings them in. Every module holding anything named sits on a ring just
outside the wall, each at the bearing it had, with its named functions in it; a call
to one runs out through the wall at the point facing its module and on into it. Point
at another function inside to draw its own calls in and out as pale arrows, each end
named: a function, or where the call leaves through the wall, under its module's name.
They go when the pointer leaves. Clicking a function focuses it as usual, and `X`
again shrinks the nucleus back among the other modules.

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
keys first (`--keys "i"`, `--keys ". . o"`), and `--hover UNIT` puts the pointer over a
function, so a scenario can show a view you reach by hand. From that, `readability.py` writes questions with their answers (call order,
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
