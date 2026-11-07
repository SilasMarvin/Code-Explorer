# codeviz

A 3D map of a Rust codebase's call flow. Point it at a project, pick a function (an Axum
handler, say), and follow everything it calls, level by level.

## Install

You need Rust 1.99 or newer (`rustup update`), and on macOS the Xcode command line tools
(`xcode-select --install`).

```sh
cargo install --locked --git https://github.com/SilasMarvin/Code-Explorer codeviz
rustup component add rust-analyzer      # optional: resolves method calls exactly
```

From a clone, `cargo install --locked --path crates/codeviz` does the same. The first
build compiles Bevy and takes several minutes. If cargo cannot fetch the repository
(it is private, so you need access to it), add `--config net.git-fetch-with-cli=true`
so cargo fetches with your own git login.

On Linux, building needs only a C compiler (`build-essential` on Debian and Ubuntu,
`gcc` elsewhere): the window and graphics libraries are loaded at run time, not linked.
Running needs an X11 or Wayland desktop and a Vulkan driver (`mesa-vulkan-drivers` on
Debian and Ubuntu, `vulkan-radeon`/`vulkan-intel` on Arch, or your GPU vendor's). No
ALSA or udev packages are needed.

## Use

```sh
cd path/to/your/project
codeviz view .                                   # analyze it and open the map
codeviz view . --focus orders::place             # open that function's journey
codeviz view . --focus 'POST /users' --depth 2   # an Axum route's, two levels deep
codeviz analyze . -o app.json                    # write the IR document instead
codeviz view app.json                            # and open it later, without analyzing
```

`view` and `analyze` say on stderr what they found and how long each part took.
rust-analyzer type-checks the workspace and builds its build scripts and proc macros
first (into the project's `target/`), so on a large workspace the first run takes
minutes; `--resolve syntax` skips it. `--focus` takes an entry's label (`POST /users`)
or the end of a function's path (`orders::place`). In a clone of this repository,
`codeviz view demo/out/axum-server.json --focus orders::place` opens a small Axum
service on one handler's journey.

| Key | Does |
| --- | --- |
| click | select a function: its callers light violet, its calls blue |
| `Enter` | open a journey at the selection |
| arrows | move the journey's cursor: `↓` into a call, `↑` out, `←` `→` between siblings |
| `.` `,` | show a level more, or less, of the journey |
| `X` | fly into a module, and back out |
| `F` | frame what is in focus |
| `R` | turn back to the usual view and frame everything |
| `-` `=` | hide the modules far from the journey or the selection, a step at a time, or show them again |
| `Shift+-` `Shift+=` | spread the map closer or wider |
| `/` | search |
| `H` | list the keys that work in the mode you are in |
| `Esc` | step out one level: the selection, the expanded module, the journey |
| `Backspace` | back to the view before (`Shift+Backspace` forward) |

Drag to turn the map, right-drag or `Shift`-drag to pan, the wheel or a pinch to zoom.
Hand-written documents work too: [docs/ir.md](docs/ir.md) has a minimal one.

## The view

One window, one view. Every module is a sphere and every function a dot inside its
module's sphere; a sphere is as big as the functions it holds. A sphere is a lit,
matte, see-through ball, shaded towards its edge; a dot is a small shaded ball, and
what lies further back shows a little dimmer. A lit dot and its name show over
everything else, whatever lies in front, so a journey reads from any side. The layout is worked out
once per document, in 3D, by one rule for the modules in the world and the functions in
each module: what calls what sits near it, and a function sits on the side of its sphere
towards the modules it calls or is called from. Browsing, in a journey and with a module
expanded, everything stays where the layout puts it, whatever is clicked. The view is always in one of
two modes, and the top bar says which: `Browsing`, or `Journey ·` and where the journey
starts.

Each color says one thing everywhere. White: the selection, or the journey's start.
Violet: calls into it. Blue: its own calls. Amber, darker the deeper: the journey's
calls further down. Pink: the journey's cursor, and the one call along its path into
it. Everything else is grey, and each
module's sphere has a muted hue of its own, only to tell modules apart. Each role gets
one treatment: the selection and the start a white ring, the cursor a pink fill and one
pink ring. A call that also calls back into the start keeps its call's fill and gets a
violet ring, and an arrow each way. A function set aside in a journey
keeps its own fill (grey off the journey, violet if it calls the start) with a white
ring, and its calls their own colors. What the pointer rests on gets one grey treatment
wherever it is: a thin ring round its dot, an outline round its bar, and from a bar a
grey line to its dot; it never lights a call. A call only guessed at is faint on the
graph as in the panel. Every call is one straight line from its caller's dot to its
head, short of its callee's; a dot it passes shows over it; a call crossing the view
with neither end in it is faint.

### Browse

With no `--focus`, the view opens browsing the whole codebase with nothing selected,
and a faint line, one width, runs behind the spheres from each module to every module
it calls; a module whose name has no room above its sphere is named, smaller, inside
its top. A click on a
function selects it where it sits: its callers light violet and its calls blue, each
named, with arrows; their modules open where they are; nothing moves, not a function,
not a module, not the camera. `Enter`, or a
`Shift`-click on a function, opens a journey there. `/` searches; picking a result is a
click on it. `Esc` clears the selection. The journey keys (arrows, `[ ]`, `, .`, `I`)
say "Open a journey first (Enter)". Two functions of the same name in view, the
selection's callers or calls, or in one module, are named with their type or module
before them, wherever their names show, the search's rows too.

### Journey

A journey is the call tree from one start function, shown to a disclosed depth. Opening
it is the one time the view moves for you: the camera frames it. Nothing is laid out
again: its start, what calls the start, and its calls down to the depth disclosed light
where they sit, their modules open, and everything else dims. Zoomed into a module, a call that leaves the
screen is labelled where it leaves with the function at its far end. `.` (or the
panel's `+`) shows every branch a level deeper, down to the journey's deepest; `↓` on a
call whose calls are folded, or a click on its bar's triangle, opens that call's alone
and steps in; `,` (or `−`) folds every branch back to a level less than the deepest
shown, down to the start's own calls. Either end says so, nothing else moves, and the
depth beside `−` and `+` is the deepest level shown. Whatever can fail says "can fail"
at every depth, where it fails and every call it passes through. A closure counts as
part of the function around it.

The journey panel docks along the foot: a bar per call, left to right as they run, a
row a level deeper, each filled like its node and numbered as the graph numbers it (`2`
for the start's second call, `2.1` for that call's first). Dashed bars run only
sometimes, and say when; pale ones were only guessed at, or are handed off to run later;
a green end returns and a red one fails. Hovering a bar draws a line to its node. Above
the start's bar, a row of violet bars lists the functions that call the start; the two
stay put while the rows below scroll. Bars past either side are counted at that side,
and a click on the count scrolls on.

The cursor is one call of the journey. `↓`/`↑` go into a call and out to its caller,
`←`/`→` move between the calls one caller makes, `[`/`]` step through them in the order
they run; a click on a bar, or on a function the journey runs, moves it there. Nothing
on screen moves but the highlight. A search picks a function the same way. A click on
any other function selects it in place,
with `not on this journey · Enter starts one here` in the top bar; an arrow or `Esc` goes
back to the cursor. `↑` from the start goes up into the row of its callers, selecting
the first in place (`calls … · Enter starts a journey here`); `←`/`→` move along the
row, `Enter` opens that caller's journey and `↓` comes back to the start. A click on a
caller's bar selects it the same way. `I` lights every function off the journey that
calls the cursor's, where it sits, and `I` again lets them go; with none, it says so.
Moving the cursor with `I` on lights some and lets others go, and moves none. `Esc` steps out one level at a time, the last opened first:
the function selected off the journey (but after `X` on it, the module first, keeping
it), then the module expanded, then the journey, for browsing with its start selected
and the camera back where it stood before the journey. `Shift`-click on a
journey's bar opens that function's journey, as anywhere.

`Backspace` (or `← Back`) goes back to the view before: a selection, a journey, a
cursor position, with the camera each had; `Shift+Backspace` (or `Forward →`) goes
forward again. A walk with the arrows or brackets is remembered once, from where it
began; moving along the row of the start's callers is no move of its own. Changing the
depth is not a move of its own. A journey left and entered again opens at its first
level.

### Expand

`X` flies into a module (the top bar then reads `· inside` it): that of the selection,
the cursor or the start, else the one in the middle of the view (`--expand` with
`--focus` opens straight into the start's). A double click on a module, on one of its
functions or anywhere on its sphere, does the same for that module, and picks nothing:
the selection, or the journey's cursor, stays. The camera saves where it is and comes in
until the module fills the view, in the middle of it. The module opens where it stands, browsing or in a
journey alike: every one of its functions drawn and named where they fit, with the
calls among them, and nothing moves but the camera. In a journey, when the cursor
moves to a function in another module, that module opens instead and the camera flies
to it; moving back flies back. `X` again, `Esc`, or a double click off every function
shrinks it back and returns the camera to exactly where it stood before the first
module opened. `Back` and `Forward` keep it expanded; going to another journey, or
between browsing and a journey, shrinks it back.

### Hiding modules

`-` hides the modules far from what the view lights, a step at a time: first every
module more than two calls from a module holding something lit, counting calls between
modules either way, then more than one, then all but those holding something lit; `=`
shows them again a step at a time. Lit, in a journey, are its start, the row of its
callers, its calls down to the depth shown and what `I` lights; browsing, the
selection, its callers and its calls. A module hidden is not drawn at all: no sphere,
dot, name or line. Nothing is laid out again and nothing moves, the camera neither;
`F` and `R` frame only what shows. The level holds as the cursor moves, the depth
changes, a module expands, and through Back and Forward; what shows follows what is lit.
A search still finds a function in a module hidden; picking it lights it, which shows
its module. Browsing with nothing selected nothing is lit to hide round: every module
shows, and `-` says so. The bar along the top says each step as it is taken, and
while modules are hidden a chip there says how far what shows reaches (`Hiding · +1
call`, `Hiding · journey only`); a click on it shows every module again.

### Camera

The wheel or a pinch zooms towards the pointer, in until the module under it fills nine
tenths of the view, drawn into it whole, out until the whole scene takes about half of
it; a drag turns the map any way round, over the top too, with nothing laid out again
and names holding their places until you let go; a right drag, or `Shift` with a
drag, pans, no further than keeps the map in the view, or close in, a module near its
middle; a drag begun on the graph goes on over the panel. `F` frames the focus: the
selection and what it lights, the journey, or the module expanded. `R` turns back to the
usual view and frames everything shown. A click on empty space clears the selection (in
a journey, the function set aside) and never moves the camera. The camera moves only
when you tell it to, on entering or leaving a journey, a `Back` or `Forward` that
crosses between them, and expanding or shrinking, each move one short glide; besides,
a level more draws it back when what comes in would show off the view, and expanded
in a journey it follows the cursor.
`H` lists the keys that do something in the mode you are in.

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
| `codeviz-layout` | The codebase map and the force-directed graph that seeds each module's interior. Renderer-agnostic. |
| `codeviz-viewer` | Bevy + egui viewer. |
| `codeviz` | CLI. |

Producers depend only on `codeviz-ir`. Core, layout and the viewer never see a producer.

## Call resolution

Rust calls are resolved by `rust-analyzer scip`, which type-checks the workspace: a
method call goes to the method that actually runs, a call into std or a dependency goes
nowhere, and a call through a trait object or generic bound goes to every
implementation in the analyzed source. `rustup component add rust-analyzer` installs
it. It runs once for each Cargo workspace under the directory you point codeviz at (a
workspace nested inside another, like axum's `examples/`, is a run of its own;
cargo-fuzz targets are skipped), from the workspace's directory, so its
`rust-toolchain.toml` applies.

rust-analyzer builds build scripts and proc macros first, with `cargo check`, into the
project's own `target/` directory, as `cargo build` would. That takes seconds on a
small project and minutes on a large one, and a build script can cost more than that:
helix's fetches and compiles about 300 tree-sitter grammars (2 GB) the first time.
Later runs reuse what the first built.

`codeviz analyze` and `codeviz view` say on stderr what they found and did: the crates,
packages and files analyzed, any skipped (test fixtures, packages outside their
workspace, unreadable manifests), each workspace rust-analyzer indexed and how long it
took, and what it could not do: a workspace it failed on, a build script that failed,
or build scripts and proc macros it could not build at all (when `cargo check` fails,
for example on a git dependency it cannot fetch). Those calls fall back to the syntax.

Without rust-analyzer, or with `--resolve syntax`, and for any call it leaves
unresolved, calls are resolved from the syntax alone: module paths, `use` imports and
re-exports, `self` and declared types are followed (through smart pointers, Bevy's
`Res`/`ResMut`, and into closures), and a method call on a value of unknown type is
left unresolved, since it is far more often `Option::map` or `Vec::len` than a
workspace method of the same name. Capitalized call targets are treated as
constructors and dropped either way.

## Development

The tasks run through [mise](https://mise.jdx.dev); each is a plain cargo command in
`mise.toml` if you would rather not install it.

```sh
mise install                # the pinned toolchain, with rust-analyzer
mise run check              # fmt, clippy, tests
mise run demo-docs          # regenerate demo/out/axum-server.json
mise run demo               # the demo, on the journey of POST /users/{id}/orders
```

`tools/drive` runs the viewer from a script of a person's steps and checks every
picture it takes; `tools/drive suite` runs the regression scripts in `tools/scripts/`.
See [docs/harness.md](docs/harness.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.
