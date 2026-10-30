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
codeviz view app.json --z nucleus    # the focus's journey, gathered round its module
codeviz view app.json --z nucleus --expand   # inside the journey's start's module
codeviz view app.json --z nucleus --browse   # every module, nothing moving, nothing selected
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
callees; press again to bring them back, and a chip in the top bar says what is hidden.
On the flat graph they stop lighting up and fade with the rest; gathered into spheres,
`Shift+I` takes the callers out of the view and `Shift+O` the journey. Neither moves the
camera.

The focus is what `I`, `O` and `Enter` act on, named in the top bar beside how many
calls it makes and takes: browsing, the function selected; in a journey, the node under
its cursor. The arrows, a digit (the call with that number on its badge and bar) and a
click on a node the journey runs all move the cursor, opening whatever folds it, and
never restart the journey. A click on a node off the journey selects it in place, with
`not on this journey · Enter starts one here` beside its name; an arrow or `Esc` goes
back to the cursor. `Enter` starts the journey at the focus, and `Shift`-click at a node
at once, as deep as the journey before it showed. Hovering a node rings and names it
without moving the focus, so the colors stay put.

`-` and `=` reveal less or more around a starting point, the focus, a diff's changes or
the journey (`L` switches), measured in calls, distance in the graph, or a blend of both
(`G` switches); what is out of reach dims, then shrinks to a speck. The dial runs from
one step out to the farthest node, which is everything, so each press moves one ring; a
slim meter at the graph's right edge shows where it stands and what it measures from
for a couple of seconds after `-`, `=`, `L` or `G`, and whenever the pointer is over it.
In the journey, `←`/`→` move between calls by the same caller, `↓`/`↑` go in and out,
`[`/`]` step through time. `/` searches for a function, `S` opens the side panel (the
view's settings, outcomes, source and details), `H` lists the keys that do something in
the view open (browsing or in a journey, flat, gathered or diff), in columns that fit
the window and scroll past its height, under a line saying what each color means and
what the mode does, and `Esc` closes whichever of those is on top, or the spotlight.

One rule says what is remembered: every selection browsing; entering or leaving a
journey; everything that changes where the journey starts, or moves the cursor by a
click (on a function or its name, on or off the journey), a journey bar, a digit or
`Enter`; and every new start, by a `Shift`-click, a search result, a change, an
outcome or `Start here`. A walk with the arrows or brackets is remembered once, however
many steps it takes. `Backspace`, or `← Back` at the left of the top bar (the two always
agree), goes back to the view before it, the mode, the function selected, cursor,
outcome, depth, opened calls, spotlight and what `Shift+I`, `Shift+O`, `I` and `N` hid
or showed included; after a walk, to where the walk began. `Shift+Backspace` (or
`Forward →`) goes forward again; both buttons are always there, greyed out with nowhere
to go. Changing the depth, or hiding a side, is not a move of its own.

The camera moves only when you tell it to: entering or leaving a journey, a `Back` or
`Forward` that crosses between them or to another start, expanding or shrinking the
nucleus, `F`, `R`, `Q`, `E`, and your own orbit, pan and zoom. A click, a bar, a step of
the cursor and a selection never move it. Gathered into spheres, a level more or less
of the journey refits it. On the flat graph, in a journey, stepping moves it only when
the cursor lands off screen or under the docked journey, and then pulls back just far
enough to show it. Pan or zoom and it stays where you put it, saying `camera free · F to
follow`, until `F` brings it back.

A click picks the function you see: a named function before a faint speck lying a
pixel in front of it, and a function's name picks the function. Specks left out with
`N` are never picked.

## Browse and journey

The view is always in one of two modes, and the top bar says which: `Browsing`, or
`Journey ·` and where the journey starts.

Browsing, no journey is open, and you are looking round the codebase. A click on a
function selects it, white, where it sits: what calls it lights violet and what it
calls blue, each named where it is, with arrows; their modules open where they are;
everything else steps back. Nothing moves, not a function, not a module, not the
camera; another click moves the selection and the picture stays put. `Backspace` and
`Shift+Backspace` walk back and forward through what you selected. The journey panel is
not shown, and nothing carries a journey number. `Enter`, or a `Shift`-click, opens the
selection's journey. `--browse` opens browsing, with `--focus` selected, or without
it, nothing.

In a journey, you follow one call tree. Entering it is the one time the view moves for
you: it gathers round the journey and frames it. Only the journey shows: where it
starts, what calls the start, and its calls down to the depth disclosed; everything else
is hidden. A click on a function the journey runs moves the cursor there, as a click on
its bar does, and nothing else moves; a function it runs more than once goes to the run
nearest the cursor, or a click on its name to the run the name's number gives. A click
on any other function selects it in place, white-ringed, with its calls into the
journey; it does not leave the journey, and `Enter` starts a new journey there. `Esc`
lets go of that, then leaves the journey for browsing, with the cursor's function
selected; `Backspace` crosses back into the journey as it crosses anything.

## Nucleus

`Z` (or `--z nucleus`, which opens on it) gathers the graph into module spheres, and
space says only where things are.

Browsing, every module is a sphere where one layout of the whole codebase puts it,
worked out once per document from how much modules call each other, laid out wider
than high as a window is, and every function is inside its module, where its corner of
the flat map has it. The layout is shallow along the usual view and keeps every two
spheres apart as that view sees them in perspective, however near the camera comes, so
which module a function belongs to is never in doubt. Module centres never move: a
click lights what it touches in place. A module holding something lit is lit glass, its
lit functions spread apart inside it so each can be told from the next and clicked (the
module grows a little into the room its neighbours leave, and the function clicked
stays where it was); the rest are faint. A module's name is as big as the module and
shows once its sphere is big enough on screen to be read as its own, else under the
pointer, with `3 of 9` saying how much of it is lit; zooming in names more, a click
never changes which. With nothing selected, a faint arrow runs from each module to the
module it calls most (to each, when several tie), as thick as the calls it carries, so how modules depend on each
other reads at a glance; every weaker dependency is left out, and a selection's own
calls take over. `F` frames what the selection touches, `R` the whole codebase.

In a journey, the start's module is a large glass sphere in the middle with the start
inside, and every module the journey enters stands round it on a ring, clockwise from
the top in the order the journey first enters them, then the modules of the start's
callers; each is a small glass sphere opened just far enough to name what it holds,
`3 of 9` saying how much of the module that is. Every other module is hidden, and so
is every function the journey does not show; `N` brings the start's module's other
functions back as faint specks.

One rule says what a journey names: the start; every function that calls it, violet;
and every call of the journey from the start down to the disclosed depth, the depth the
journey panel shows (`,` and `.`, the panel's `−`/`+`, `--depth N`, a bar's `+N`, and
whatever the cursor opens; disclosing through time, `V`, what the cursor has reached).
It reads as one numbered path: each call is drawn as its own arrow, from caller to
callee along the journey's tree, blue for the start's own calls, then amber, darker a
level at a time, on arrows, name borders and names alike. Each function the journey
runs carries its number as the panel numbers it, on a badge filled like its node and
bar: the run nearest the journey's start, so a function the root calls second reads `2`
at any depth, and `+2` when the journey runs it twice more; the cursor's function shows
the cursor's own number. `I` brings in every function off the journey that calls one on
it, pale and muted, where it lives: inside the middle, hanging outwards off a module on
the ring, or in its own module beyond the ring; nothing already there moves, and `I`
again takes them out. A name always sits by its own function: never nearer another
node, and one nudged along a side keeps a thread to its node and comes back beside it
once there is room. Past 60 functions named, only the start, the cursor and what is
within two calls of the start keep their names; the rest are named under the pointer.
`Shift+I` leaves the callers out and `Shift+O` the journey, and the top bar says so.

A thin arrow joins two modules whenever a named call runs between them, colored as its
call nearest the start, and the calls run beside it side by side, function to function.
An arrow bends round every sphere drawn in its way, and a call round the functions
drawn near it, so none passes through another module or reads as another function's
call. With `I` on, resting the pointer on an outside caller draws its call into the
journey bright, as a click on it does.
A new depth brings modules onto the ring or lets them go, easing every sphere to its new
place. The pointer has to rest on a function for a moment (0.15 s) before the view
takes it as pointed at, so passing over a speck does not flash its calls, and after a
click or a key nothing is pointed at until the pointer moves; a speck's name under the
pointer is small and muted, and placed after every other name, so pointing moves none
of them.

Stepping through the journey (the arrows, `[`/`]`, their `Shift` forms, a journey bar,
a click, `Space`) moves nothing but the cursor, the one pink node: its function is
filled pink and ringed twice in a pink glow, pulsing for a moment where it lands, its
name raised over the others on a larger, pink-edged pill, the journey's call into it
drawn pink, and its bar filled pink with the cursor's white outline and a pink line up
to the node. The cursor never stops on a closure, which has no bar or node of its own:
its calls count as the function's around it. Pointing at a bar, or at a function,
previews it in white: a glow on its node and its call brightened. `Q` and `E` turn the
view an eighth of the way round, refitting it, so the depth shows; `R` frames the
journey again from the angle the camera has, `F` from the usual three-quarter view, and
`Z` again lays the graph flat.

`X`, or a double click on the nucleus away from its functions, expands it in a journey
(`--expand` opens straight into it): the sphere grows to several times its size while
the camera comes in, so you go inside the same object. It names exactly what the module
view names. Inside, the module's functions are laid out by the calls among them, once
for each middle from everything it could name at any depth, so a new depth, a side left
out or the pointer moving only names and draws, and moves nothing: what is named
gathers in the middle, kept from touching, the callers tending to one side and the
journey to the other (a tendency, not an axis); the rest are faint specks out towards
the wall, left out until `N` brings them in. Every module holding anything named sits
on a ring just outside the wall, clockwise in the order the journey enters them, with
its named functions in it; a call to one runs out through the wall at the point facing
its module and on into it. Point at another function inside to draw its own calls in
and out as pale arrows, each end named: a function, or where the call leaves through
the wall, under its module's name. They go when the pointer leaves. Here, and only
here, a click on a function inside makes it the middle, the expanded module laid out
round it; the view pans to the cursor only when it would otherwise show off screen or
near its edge. `X` again shrinks the nucleus back round the journey's start.

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

`--script FILE --screenshot DIR` runs a whole session in one window instead: a step a
line (`key shift+i`, `click UNIT`, `click-label UNIT` on the function's drawn name,
`hover UNIT`, `doubleclick UNIT`, `bar 1.2`, `button TEXT`, `drag DX DY`, `scroll N`,
`wait MS`, `snap LABEL`), each sent as a person's mouse
and keyboard would send it, the view let settle after each. Every `snap` writes
`NN-label.png` and `NN-label.json`, which says what is drawn where on screen by name:
functions with their roles and labels, module spheres, arrows, label overlaps, journey
bars, buttons and overlays. `session.json` logs how each step went.

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
