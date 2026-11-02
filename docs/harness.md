# The harness: driving codeviz and checking what it shows

Agents implement and agents test. The harness runs the real viewer in one window from a
script of steps a person would take (keys, clicks, hovers, the wheel, typing), and at
every `snap` writes a PNG, a JSON of what the picture shows by name and place, and the
result of the checks that prove CORE's building blocks. `expect` lines in the script fail
the session when the view is not as they say. `tools/drive suite` runs the regression
scripts in `tools/scripts/` and prints what passes and what fails.

## Five minutes from zero

    cx /path/to/worktree build                  # /Users/silas/Projects/cx-tools/cx
    cat > /tmp/try.txt <<'EOF'
    #: view demo/out/axum-server.json
    snap open
    click pricing::quote
    expect selection pricing::quote
    expect nothing-moved
    snap selected
    EOF
    tools/drive check /tmp/try.txt              # names looked up, no window
    tools/drive run try /tmp/try.txt            # one window, 10-30 s

`run` prints a verdict, then any failed step, failed `expect` and failed check with its
worst offender. The session is in `$CODEVIZ_SESSIONS/try/`; look at
`02-selected.png` with the Read tool and `02-selected.json` beside it. `tools/drive read
try` prints the summary again.

Run `tools/drive` from the worktree whose viewer you want, or pass `--wt DIR`. Never
start the viewer any other way: the runner kills it past 3000 MB or its time limit, and
holds a lock so only one viewer window renders at a time across every agent.

## Commands

| command | does |
|---|---|
| `tools/drive run NAME SCRIPT [--wt DIR] [-- VIEW ARGS]` | one session into `$CODEVIZ_SESSIONS/NAME/`; exit 0 when clean, 1 when anything failed, 2 when it could not run |
| `tools/drive suite [--only TEXT]... [--prefix P]` | every `tools/scripts/*.txt` (or those whose name holds TEXT), names checked first, then one window at a time; prints the table, writes `$CODEVIZ_SESSIONS/<P>suite.json` |
| `tools/drive check SCRIPT...` | reads each script and looks up what it names, without a window |
| `tools/drive read NAME` | the summary of a session already run |
| `tools/drive shot NAME -- VIEW ARGS [--keys "x ."] [--hover UNIT]` | one picture: `$CODEVIZ_SHOTS/NAME.png`, `NAME.json` (the snap), `NAME.truth.json` (its `content`) |
| `tools/drive live [DIR] -- VIEW ARGS` | an interactive window under the memory guard |

`VIEW ARGS` are `view DATA [FLAGS]`, as for `codeviz view`; a script's `#: view ...` line
gives them when none follow `--`, with `$CODEVIZ_DATA` filled in. Relative paths are from
the worktree. Environment: `CODEVIZ_SESSIONS` (default
`/Users/silas/Projects/cx-tools/sessions`), `CODEVIZ_SHOTS` (default `.../cx-tools/shots`),
`CODEVIZ_DATA` (default `.../cx-tools/data`), `CODEVIZ_LOCK`. The older
`cx-tools/drive WORKTREE NAME SCRIPT -- view ...`, `cx-tools/shot` and `cx-tools/live` are
thin wrappers over this.

Data for checks: `demo/out/axum-server.json` (small; focus `orders::place`) and
`$CODEVIZ_DATA/ord-ws-2.json` (this repo, about 2,300 functions; focus
`Explorer::spotlit`, `Finder::walk`). With no `--focus` the viewer opens browsing;
`--focus UNIT` opens that journey, `--depth N` and `--expand` as they say.

## Scripts

One step per line; blank lines and `# comments` are skipped; `#: view ...` gives the view
args. Every line is read, and every function and module named is looked up, before the
window opens: a line it cannot read, or a name matching nothing or several functions,
stops the script with the line and the nearest names. A step that cannot be taken when
it comes (a function not drawn, a button not showing) is logged as failed with why, and
the session goes on.

`UNIT` is a function as `--focus` finds it: an entry label (`POST /users`) or the end of
its id (`orders::place`, `Explorer::spotlit`). `MODULE` is the name a module's sphere
carries, in any case.

| step | does |
|---|---|
| `key K` | presses a key: a letter, a digit, `, . / [ ] - =`, `up down left right enter esc space tab backspace`; `shift+K` holds Shift |
| `type TEXT` | types TEXT a character at a time into the focused text box (`key /` opens search) |
| `click UNIT` | moves the pointer onto the function's dot, checks the pointer picks it, and clicks |
| `click X Y` | clicks a point, in window points |
| `click-label UNIT` | clicks the middle of the function's drawn name, as a person picks a small or crowded dot |
| `click-empty` | clicks a spot on the graph clear of every dot, name, text and overlay, nearest the middle; deselects |
| `hover UNIT`, `hover-label UNIT` | rests the pointer on the dot, or on the name, 0.4 s |
| `shift+click UNIT`, `doubleclick UNIT` | as they say |
| `doubleclick-module MODULE` | double-clicks the module's sphere at the spot farthest from its dots and names |
| `bar N` | clicks the journey bar numbered N (`2`, `1.2`), or named N (the start's bar has no number) |
| `button TEXT` | clicks the button or chip that says TEXT (exact, any case; else a short label holding it) |
| `drag DX DY` | orbits by a left drag from the pointer, or the graph's middle |
| `scroll N`, `scroll DX DY` | wheel notches at the pointer (the graph's middle at first); positive DY zooms in |
| `scroll ... at UNIT`, `scroll ... at X Y` | moves the pointer there first: wheel zoom toward a point |
| `window W H` | resizes the window, in points |
| `wait MS` | lets time pass |
| `snap [LABEL]` | writes `NN-label.png` and `NN-label.json`, with the checks run |

After every step the harness waits until the camera, the spheres and the names stop
moving (at least 0.5 s, at most 8 s, else the step's trouble says it was still moving).
Pointer steps go through the window's real input path, so picking, egui and the
viewer's handlers all see them; when the pointer lands on another function or on an
overlay, the step fails saying what it is over (the click still happens).

### expect

`expect` lines are judged on the view as the step before left it. A failed one is logged
with why, marks the session failed, and the session goes on.

| expect | holds when |
|---|---|
| `expect mode browse\|journey` | the view browses, or has a journey open |
| `expect nucleus modules\|expanded` | a module is expanded, or not |
| `expect depth N` | the journey shows N levels |
| `expect search\|help\|back\|forward\|outside_callers true\|false` | that field of `state` reads so |
| `expect state FIELD VALUE` | the same, spelt out |
| `expect selection UNIT\|none` | the function selected while browsing |
| `expect cursor UNIT\|none`, `expect start UNIT\|none` | the journey's cursor, its start (`none` while browsing) |
| `expect named UNIT` | the function's name is drawn |
| `expect drawn UNIT` | the function's dot is drawn on the graph |
| `expect camera-same-as LABEL` | the camera's target, distance, yaw and pitch are those at the snap LABEL (`before x` or `03-before-x`), within 0.05 and 0.002 rad |
| `expect nothing-moved` | no dot, sphere, name or the camera moved since the last snap |
| `expect check NAME [pass\|fail]` | the check NAME comes out so on the view now, against the last snap and the steps since; one that does not apply fails it |

Snap before the step you want to judge, then `expect` after it: `nothing-moved`,
`zoom-to-pointer` and the other checks against a step compare with the last snap.

## Reading a session

`$CODEVIZ_SESSIONS/NAME/` holds:

- `checks.json`: start here. `ok`; `checks` (each check: `pass`, `passed`, `failed`,
  `not_applied`, `failed_in` snaps); `expects` (each line, `ok`, `why`); `failed_steps`
  (`step`, `line`, `do`, `trouble`); `snaps` (each snap's checks in full).
- `NN-label.png`: the window at that snap, in window points, the same as the JSON's.
- `NN-label.json`: what that snap shows (below).
- `session.json`: every step with `ok`, `trouble`, `settled_ms` and `then` (the state after
  it); `script.txt`, `args.txt`, `viewer.log`.

The JSON says what is there; the PNG says whether a person could see it. Read both.

### A snap's JSON

Positions are window points; rectangles `[x0, y0, x1, y1]`; world positions are the
scene's units.

- `window`, `graph_area` (the graph's part of the window, under the top bar), `camera`
  (`at`, `facing`), `snap`.
- `state`: `mode` (`browse`, `journey`), `selection`, `focus`, `centre`, `centre_module`,
  `nucleus` (`modules`, `expanded`), `journey_start`, `cursor` (`number`, `name`),
  `aside`, `depth`, `outside_callers`, `pointed` (the function the view counts
  as under the pointer) and `under_pointer` (raw), `search`, `help`, `back`, `forward`,
  `camera` (`target`, `distance`, `yaw`, `pitch`), `pointer`.
- `nodes`: every function drawn: `name`, `unit`, `module`, `roles` (`centre`, `start`,
  `cursor`, `caller`, `journey level N`, `callee`, `on the open journey`, `hovered`,
  `speck` for a faint unnamed dot), `numbers` (in the journey), `at`, `radius` (on
  screen), `world`, `world_radius`, `on_screen`, `label`, `label_rect`, `crowded_by`.
- `modules`: each sphere: `name`, `body` (`nucleus`, `open`, `closed`, `faint`), `shown`,
  `functions`, `named`, `at`, `radius`, `world`, `world_radius`, `on_screen`, `label`,
  `label_rect`.
- `labels`: every name on the graph: `text`, `rect`, `names` (`function`, `module`,
  `other`) and `of`, `overlaps` (indices), `cut_off`, `under` (overlays over it),
  `opacity`, `size` (font size).
- `mislabelled`: names nearer another dot than their own, or far from every dot.
- `arrows` (`from`, `to`, `role`, `color`) and `module_arrows` (`from`, `to`, `calls`).
- `bars`: journey bars in view: `number`, `name`, `unit`, `rect`, `cursor`, `lit`,
  `cut_off`. `buttons`: `text`, `rect`, `enabled`, `cut_off`. `overlays`: `name`, `rect`,
  `clipped`. `text`: every other text drawn, with `rect` and `size`.
- `content`: what the view means to show: focus, callers, the disclosed calls with numbers
  and modules.
- `moved`: against the snap before: `since`, `camera`, `nodes` (`compared`,
  `moved_in_world`, `moved_on_screen`, `most_world`, `most_screen`, `examples`),
  `modules`, `labels`, `appeared`, `gone`.
- `checks`: each check's result (below). `log`: the steps so far.

## The checks

Each check passes (`"pass": true`), fails (`false`, with `count` and up to 12
`offenders`, worst first, each naming the functions, modules or labels and the numbers),
or does not apply (`null`, with `why`). They are computed in the viewer
(`crates/codeviz-viewer/src/checks.rs`) from the snap's own JSON; every threshold is a
constant at the top of that file.

| check | holds when | applies |
|---|---|---|
| `dots-in-spheres` | every dot lies inside its module's sphere, in the world (1% slack) | always |
| `dots-apart` | no two dots on screen overlap: centres at least the radii plus 1 point apart (lit pairs listed first) | always |
| `dot-sizes` | a lit dot is at least 3 points across; no dot is over 16 points in radius | always |
| `spheres-apart` | no two spheres intersect, in the world | two spheres or more |
| `sphere-sizes` | the largest sphere is at most 4× the smallest's radius; a module with more functions is bigger, but less than in proportion (the expanded one aside) | always |
| `labels-near` | every function's name is within 40 points of its dot, edge to edge, and every module's name within 40 points of its sphere's rim | always |
| `labels-clear` | no name overlaps another, a function's name covers no other lit dot, a module's name lies over no other sphere | always |
| `named-on-screen` | no name is cut off by the graph's edge or lies under an overlay (the journey panel, help, search) | always |
| `text-sizes` | function names are 11 to 18 points; at most 4 font sizes show at once | always |
| `panel-agrees` | each numbered bar's function carries that number on the graph, no other function does, and the cursor is the same in both | journey bars drawn |
| `expanded-fill` | the expanded module's diameter is 60–90% of the free area's smaller side (the graph less a docked panel), and it lies inside it | after `x` or a double click, when expanded |
| `framed` | after R: every sphere inside the free area, camera head-on (yaw and pitch within 0.05); after F: every lit dot (or the expanded module) inside it with a 4-point margin | when F or R is the last step since the snap before that moves the view |
| `zoom-to-pointer` | the sphere nearest the pointer moved as a zoom about the pointer moves it (within 4 points plus 5% of its distance) | one wheel turn, nothing else that moves, since the snap before |
| `zoom-bounds` | some of the map is on screen, and zoomed out it still spans 40% of the free area | after a wheel turn |
| `nothing-moved` | no dot, sphere or name moved, nor the camera, since the snap before; after `,` or `.` the camera may move, the world may not | every step since is one that must move nothing |

What a step may move decides which checks judge it:

| kind | steps | may move |
|---|---|---|
| still | `hover`, `hover-label`, `key` arrows, `[`, `]`, `i`, `wait`, `expect` | nothing |
| click | `click UNIT`, `click-label`, `bar` | nothing; expanded in a journey it may re-centre |
| depth | `key ,`, `key .` | the camera, to bring new calls on; nothing in the world |
| expand | `key x`, `doubleclick-module` | the view |
| zoom | `scroll` | the camera |
| frame | `key f`, `key r` | the camera |
| moves | everything else (`enter`, `esc`, `backspace`, `/`, `type`, `drag`, `shift+click`, `button`, `click X Y`, `click-empty`, `window`) | the view |

## The regression suite

`tools/scripts/` holds the core flows of CORE.md on both data sets:

| script | covers |
|---|---|
| `axum-browse`, `ord-browse` | click and click-label select, nothing moves; hover; click on empty space deselects; `/` search selects in place; F; wheel in and out about a function; wheel out to the bound; R |
| `axum-journey`, `ord-journey` | Enter opens the journey; cursor by arrows, `[ ]`, a bar, a click on a call and on a name, nothing moving; `.` and `,` depth; `i` outside callers; Esc back to Browse and the camera from before; Back and Forward |
| `axum-expand`, `ord-expand` | X expands to fill the view, wheel and orbit while expanded, X back to the saved camera; double-click a module; Esc collapses; the same in a journey and browsing |

`tools/drive suite` checks every script's names, then runs them one window at a time and
prints a table (failed steps, `expect`s passed, failing checks, verdict) and, for each
failing script, its failed steps, expects and checks with the worst offender. The suite
is how a fixing agent knows it is done: run the scripts its change touches with `--only`,
then the whole suite. Add a script for a flow the suite lacks; give it a `#: view` line,
snaps before the steps you judge, and `expect` lines for what CORE promises.

## Testing the harness

`cx WORKTREE check` runs the viewer's tests, among them the checks on synthetic snapshots
and the script parser (`checks.rs`, `script.rs`). `python3 -m unittest
tools/test_drive.py` tests the runner's own logic.

## Limits

- Black PNGs: macOS stops drawing a window it counts as covered; the harness keeps the
  window on top and draws a covered snap offscreen, saying so in `trouble`. The JSON is
  right regardless.
- Not faked: trackpad gestures (pinch, two-finger orbit), right-drag or Shift-drag pan,
  hovering a journey bar, holding a key. `scroll` is a mouse wheel; `drag` a left-button
  orbit.
- The real mouse moving over the window mixes its events with the script's.
- `click` aims at a dot's centre; a dot mostly hidden behind another is clicked where the
  other is, and `trouble` says so. `click-label` aims at the name instead.
- Checks see what the snap's JSON reports: a dot drawn but missing from the JSON, or a
  stroke's width, is beyond them. Arrow widths and idle CPU are not checked.
