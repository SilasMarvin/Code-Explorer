# codeviz

Explore a codebase's call flow in 3D. Point it at a directory, pick a function (an Axum
handler, say), and follow everything it calls.

```sh
mise install
mise run demo-docs                       # regenerate the demo IR documents
mise run demo                            # the Rust demo, on the journey of POST /users/{id}/orders

codeviz analyze <dir> -o app.json        # Rust or Python source → IR
codeviz analyze <dir> --resolve syntax   # skip rust-analyzer: fast, misses some method calls
codeviz validate app.json
codeviz view app.json other.json         # browse the merged codebase
codeviz view app.json --focus orders::place            # open that function's journey
codeviz view app.json --focus orders::place --depth 2  # two levels of it
codeviz view app.json --focus orders::place --expand   # inside the start's module
```

`--focus` takes an entry's label (`POST /users`) or the end of a unit's id
(`orders::place`).

## The view

One window, one view. Every module is a sphere and every function a dot inside its
module's sphere; a sphere is as big as the functions it holds. The layout is worked out
once per document and never changes because of a click. The view is always in one of
two modes, and the top bar says which: `Browsing`, or `Journey ·` and where the journey
starts.

Each color says one thing everywhere. White: the selection, or the journey's start.
Violet: calls into it. Blue: its own calls. Amber, darker the deeper: the journey's
calls further down. Pink: the journey's cursor. Everything else is grey, and each
module's sphere has a muted hue of its own, only to tell modules apart. Each role gets
one treatment: the selection and the start a white ring, the cursor a pink fill and one
pink ring. A call that also calls back into the start keeps its call's fill and gets a
violet ring, and its two arrows run side by side. What the pointer rests on gets one grey
treatment wherever it is: a thin ring round its dot, an outline round its bar, and from
a bar a grey line to its dot.

### Browse

With no `--focus`, the view opens browsing the whole codebase with nothing selected,
and a faint arrow runs from each module to every module it calls. A click on a
function selects it where it sits: its callers light violet and its calls blue, each
named, with arrows; their modules open where they are; nothing moves, not a function,
not a module, not the camera. `Enter`, or a
`Shift`-click on a function, opens a journey there. `/` searches; picking a result is a
click on it. `Esc` clears the selection. The journey keys (arrows, `[ ]`, `, .`, `I`)
say "Open a journey first (Enter)". Two functions of the same name in view, the
selection's callers or calls, are named with their type or module before them.

### Journey

A journey is the call tree from one start function, shown to a disclosed depth. Opening
it is the one time the view moves for you: it gathers round the journey and frames it.
Only the journey shows: its start in the middle of its module's sphere, what calls the
start, and its calls down to the depth disclosed, every module they enter on a ring
round the middle, clockwise in the order the journey first enters them. `,` and `.`
(or the panel's `−` and `+`) show a level less or more, from the start's own calls down
to the journey's deepest, saying so at either end, and nothing else moves; a bar's
triangle opens the calls folded under it. A closure counts as part of the function
around it.

The journey panel docks along the foot: a bar per call, left to right as they run, a
row a level deeper, each filled like its node and numbered as the graph numbers it (`2`
for the start's second call, `2.1` for that call's first). Dashed bars run only
sometimes, and say when; pale ones were only guessed at, or are handed off to run later;
a green end returns and a red one fails. Hovering a bar draws a line to its node. Above
the start's bar, a row of violet bars lists the functions that call the start.

The cursor is one call of the journey. `↓`/`↑` go into a call and out to its caller,
`←`/`→` move between the calls one caller makes, `[`/`]` step through them in the order
they run; a click on a bar, or on a function the journey runs, moves it there. Nothing
on screen moves but the highlight. A search picks a function the same way. A click on
any other function selects it in place,
with `not on this journey · Enter starts one here` in the top bar; an arrow or `Esc` goes
back to the cursor. `↑` from the start goes up into the row of its callers, selecting
the first in place (`calls … · Enter starts a journey here`); `←`/`→` move along the
row, `Enter` opens that caller's journey and `↓` comes back to the start. A click on a
caller's bar selects it the same way. `I` brings in every function off the journey that
calls the cursor's, faint, without moving anything already there, and `I` again takes
them out; with none, it says so. Every caller `I` could bring in, for any call of the
journey, has its place worked out once, so moving the cursor with `I` on shows some and
hides others and moves none. `Esc` steps out one level at a time: the function selected
off the journey, then the module expanded, then the journey, for browsing with its start
selected and the camera back where it stood before the journey. `Shift`-click on a
journey's bar opens that function's journey, as anywhere.

`Backspace` (or `← Back`) goes back to the view before: a selection, a journey, a
cursor position, with the camera each had; `Shift+Backspace` (or `Forward →`) goes
forward again. A walk with the arrows or brackets is remembered once, from where it
began. Changing the depth is not a move of its own.

### Expand

`X` flies into a module (the top bar then reads `· inside` it): that of the selection,
the cursor or the start, else the one in the middle of the view (`--expand` with
`--focus` opens straight into the start's). A double click on a module, on one of its
functions or anywhere on its sphere, does the same for that module. The camera saves where it is and comes in
until the module fills the view. The module opens where it stands, browsing or in a
journey alike: every one of its functions drawn and named where they fit, with the
calls among them, and nothing moves but the camera. In a journey, when the cursor
moves to a function in another module, that module opens instead and the camera flies
to it; moving back flies back. `X` again, `Esc`, or a double click off every function
shrinks it back and returns the camera to exactly where it stood before the first
module opened. `Back` and `Forward` keep it expanded; going to another journey, or
between browsing and a journey, shrinks it back.

### Camera

The wheel or a pinch zooms towards the pointer, in until the module under it fills nine
tenths of the view, drawn into it whole, out until the whole scene takes about half of
it; a drag orbits within a range round the usual view; a right drag, or `Shift` with a
drag, pans, no further than keeps the map in the view. `F` frames the focus: the
selection and what it lights, the journey, or the module expanded. `R` turns back to the
usual view and frames everything shown. A click on empty space clears the selection (in
a journey, the function set aside) and never moves the camera. The camera moves only
when you tell it to, on entering or leaving a journey, a `Back` or `Forward` that
crosses between them, and expanding or shrinking, each move one short glide; besides,
a level more draws it back when what comes in would show off the view, and expanded
in a journey it follows the cursor.
`H` lists the keys that do something in the mode you are in.

## Scripted sessions

`tools/drive` runs the viewer from a script, a step a line (`key x`, `click UNIT`,
`bar 1.2`, `expect mode journey`, `snap LABEL`, …), each sent as a person's mouse and
keyboard would send it, in one window under a memory and time guard. Every `snap` writes
a PNG, a JSON of what is drawn where by name, and the checks run on it (dots inside their
spheres, names by their dots, nothing moving after a click, …). `tools/drive suite` runs
the regression scripts in `tools/scripts/`. See [docs/harness.md](docs/harness.md).

## The model

Producers write an [IR document](docs/ir.md): units (things that run), their
containment, invocations between them, and optional control flow. Core unrolls an entry
point into a **journey** and lists its **scenarios**, the distinct ways it can end; the
viewer shows the happy path, and marks the calls where another outcome can fail.

## Workspace

| Crate | Role |
| --- | --- |
| `codeviz-ir` | The IR types, JSON (de)serialization and validation. |
| `codeviz-core` | Merges documents; unrolls journeys; enumerates scenarios. |
| `codeviz-rust` | Rust producer (tree-sitter, calls resolved by rust-analyzer). |
| `codeviz-python` | Python producer (tree-sitter). |
| `codeviz-layout` | The codebase map and the force-directed graph that seeds each module's interior. Renderer-agnostic. |
| `codeviz-viewer` | Bevy + egui viewer. |
| `codeviz` | CLI. |

Producers depend only on `codeviz-ir`. Core, layout and the viewer never see a producer.

## Call resolution

Rust calls are resolved by `rust-analyzer scip`, which type-checks the workspace: a
method call goes to the method that actually runs, a call into std or a dependency goes
nowhere, and a call through a trait object or generic bound goes to every
implementation in the analyzed source. It builds build scripts and proc macros first,
so it takes seconds where the syntax takes a fraction of one. `mise install` installs
it. Without it, or with `--resolve syntax`, and for any call it leaves unresolved (code
outside the Cargo workspace, or in a crate that does not build), calls are resolved
from the syntax alone: module paths, `use` imports, `self` and declared types are
followed (through smart pointers, Bevy's `Res`/`ResMut`, and into closures), and a
method call on a value of unknown type is left unresolved, since it is far more often
`Option::map` or `Vec::len` than a workspace method of the same name. Capitalized call
targets are treated as constructors and dropped either way.
