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
| `click-empty` | clicks a spot on the graph clear of every dot, name, text and overlay, nearest the middle; deselects, or in a journey clears the function set aside |
| `hover UNIT`, `hover-label UNIT` | rests the pointer on the dot, or on the name, 0.4 s |
| `shift+click UNIT` | as it says |
| `doubleclick UNIT` | double-clicks the function's dot: the first click picks it, the double click expands its module |
| `doubleclick-module MODULE` | double-clicks the module's sphere at the spot farthest from its dots and names; where no spot is clear of its dots, on a dot (which expands the module too), with a note |
| `bar N` | clicks the journey bar numbered N (`2`, `1.2`), or the bar of the function N (`Explorer::spotlit`, the start's bar first), or the bar that reads N; else the bar of the start's caller N, in the row above the start's |
| `chip N` | clicks the badge on bar N (as `bar` finds it) that unfolds the calls folded under it |
| `search-pick TEXT` | clicks the open search box's row that says TEXT (any case), else the row whose function's id ends with it, else the first row holding it |
| `button TEXT` | clicks the button that says TEXT (exact, any case; else a short label holding it); `button Fit` frames as F does |
| `drag DX DY` | orbits by a left drag from the pointer, or the graph's middle |
| `pan DX DY` | pans by a right drag, from the same place |
| `scroll N`, `scroll DX DY` | wheel notches at the pointer (the graph's middle at first); positive DY zooms in |
| `scroll ... at UNIT`, `scroll ... at X Y` | moves the pointer there first: wheel zoom toward a point |
| `window W H` | resizes the window, in points |
| `wait MS` | lets time pass |
| `snap [LABEL]` | writes `NN-label.png` and `NN-label.json`, with the checks run |
| `frames N` | right after the step before it, without waiting for it to rest, snaps each of the next N frames (1 to 30) as `NN-frame-K`: the transition as it plays |

After every step the harness waits until the camera, the spheres and the names stop
moving (at least 0.5 s, at most 8 s, else the step's trouble says it was still moving).
The step log's `settled_ms` is how long the view kept moving after the step's input,
`waited_ms` how long the harness waited. Pointer steps go through the window's real
input path, so picking, egui and the viewer's handlers all see them; when the pointer
lands on another function or on an overlay, the step fails saying what it is over (the
click still happens). A step that goes through but not quite as a person would mean it
gets a `note`: a `key` that types a character while the search box has the keyboard
(it goes into the box, not to the view), a `doubleclick-module` that had to land on a
dot. A wheel turn `at UNIT` whose function is off the graph or under the panel fails
saying so, before the wheel turns.

Frames snaps carry the checks that hold of one picture, but are not counted in the
session's verdict and are not the snap the next one is judged against.

### expect

`expect` lines are judged on the view as the step before left it. A failed one is logged
with why, marks the session failed, and the session goes on.

| expect | holds when |
|---|---|
| `expect mode browse\|journey` | the view browses, or has a journey open |
| `expect nucleus modules\|expanded` | a module is expanded, or not |
| `expect expanded MODULE\|none` | the module expanded, by the name its sphere carries |
| `expect depth N` | the journey shows N levels |
| `expect search\|help\|back\|forward\|outside_callers true\|false` | that field of `state` reads so |
| `expect state FIELD VALUE` | the same, spelt out |
| `expect selection UNIT\|none` | the function selected while browsing |
| `expect cursor UNIT\|none`, `expect start UNIT\|none` | the journey's cursor, its start (`none` while browsing) |
| `expect aside UNIT\|none` | the function set aside in a journey (a click off the journey, or ↑ into the row of the start's callers) |
| `expect named UNIT` | the function's name is drawn |
| `expect drawn UNIT` | the function's dot is drawn on the graph |
| `expect camera-same-as LABEL` | the camera's target, distance, yaw and pitch are those at the snap LABEL (`before x` or `03-before-x`), within 0.05 and 0.002 rad |
| `expect nothing-moved` | no dot, sphere, name or the camera moved since the last snap, judged as the steps since may move things (a depth change may draw the camera back; a click expanded in a journey may re-centre); any other step is held to moving nothing |
| `expect check NAME [pass\|fail]` | the check NAME comes out so on the view now, against the last snap and the steps since; one that does not apply fails it |

Snap before the step you want to judge, then `expect` after it: `nothing-moved`,
`zoom-to-pointer` and the other checks against a step compare with the last snap.

## Reading a session

`$CODEVIZ_SESSIONS/NAME/` holds:

- `checks.json`: start here. `ok`; `checks` (each check: `pass`, `passed`, `failed`,
  `not_applied`, `failed_in` snaps); `expects` (each line, `ok`, `why`); `failed_steps`
  (`step`, `line`, `do`, `trouble`); `notes` (`step`, `line`, `do`, `note`); `snaps`
  (each snap's checks in full).
- `NN-label.png`: the window at that snap, in window points, the same as the JSON's.
- `NN-label.json`: what that snap shows (below).
- `session.json`: every step with `ok`, `trouble`, `note`, `pointer` (where the pointer
  was as it began), `settled_ms`, `waited_ms` and `then` (the state after it);
  `script.txt`, `args.txt`, `viewer.log`.

The JSON says what is there; the PNG says whether a person could see it. Read both.

### A snap's JSON

Positions are window points; rectangles `[x0, y0, x1, y1]`; world positions are the
scene's units.

- `window`, `graph_area` (the graph's part of the window, under the top bar), `camera`
  (`at` in the world, `facing`, `target_at`: where what it looks at shows), `snap`.
- `state`: `mode` (`browse`, `journey`), `selection` (browsing), `start` (the
  journey's), `start_module` (the module in the middle: the start's, or the
  selection's), `cursor` (`number`, `name`), `aside` (the function set aside in a
  journey), `nucleus`
  (`modules`, `expanded`), `expanded` (the module expanded, by name), `depth`, `outside_callers`, `hovered` (the function the
  pointer has rested on, as drawn) and `under_pointer` (what the pointer is on now),
  `search`, `help`, `back`, `forward`, `camera` (where it stands that frame, on its
  way or not: `target`, `distance`, `yaw`, `pitch`, `held`: the wheel turns since it
  last turned the other way panned to hold the map, or the sphere they zoom to, in
  the free area), `pointer`.
- `nodes`: every function drawn: `name`, `unit`, `module`, `roles` (`start`,
  `selection`, `cursor`, `aside`, `caller`, `journey level N`, `calls back` for a
  call that calls the start (or the selection) back, `opened`
  for a function of the module expanded that nothing else lights, `hovered`, `speck` for a
  faint unnamed dot), `numbers` (in the journey), `at`, `radius` (on
  screen), `world`, `world_radius`, `on_screen`, `label`, `label_rect`, `name_size`
  (how big the placer laid its name's text out, room found for it or not), `crowded_by`,
  `color` (its fill, `#rrggbbaa`), `rings` (each ring drawn round it: `radius` on
  screen, `color`).
- `modules`: each sphere: `name`, `body` (`nucleus`, `open`, `closed`, `faint`), `shown`,
  `functions`, `named`, `at`, `radius`, `world`, `world_radius`, `on_screen`, `label`,
  `label_rect`, `name_size`.
- `labels`: every name on the graph: `text`, `rect`, `names` (`function`, `module`,
  `other`) and `of`, `unit` (a function's), `overlaps` (indices), `cut_off`, `under`
  (overlays over it), `opacity`, `size` (font size). A function's name is the text the
  label placer drew on that function's pill; a module's, the nearest text leading with
  its name. The key hint, notices and footers name nothing (`other`).
- `arrows`: every call drawn, function to function, as drawn: `from`, `to`,
  `from_unit`, `to_unit`, `flow` (`into`, `journey` with `level`, `outside`, `module`),
  `role` (in words), `color` (as drawn), `width` (the line, in points), `head` (its
  length on screen), `head_width`, `tip`, `on_screen`.
- `module_arrows`: each arrow between modules: `from`, `to`, `calls`, `flow`, `level`,
  `role`, `drawn`, and when drawn `width` (the shaft), `head`, `head_width`, `tip`,
  `on_screen`.
- `bars`: journey bars in view: `number`, `name`, `unit`, `rect`, `cursor`, `lit`,
  `color` (its fill, `#RRGGBB`), `guessed` (a call only guessed or handed off, drawn
  faint), `cut_off`, `folded` (calls folded under it), `chip` (the badge that unfolds
  them); then the bars of the start's callers in the row above its bar: `caller` true,
  no `number`, `aside` (the one set aside), `color`.
  `search_rows` (while the search box shows): `text`, `unit`, `rect`, `shown`,
  `highlighted` (what Enter takes). `buttons`: `text`, `rect`, `enabled`, `cut_off`.
  `overlays`: `name`, `rect`, `clipped`. `text`: every other text drawn, with `rect`
  and `size`.
- `content`: what the view means to show: the `start` or the `selection`, its
  callers, its calls (in a journey the disclosed ones, with numbers and modules;
  browsing the selection's own, unnumbered).
- `moved`: against the snap before: `since`, `camera`, `nodes` (`compared`,
  `moved_in_world`, `moved_on_screen`, `most_world`, `most_screen`, `examples`),
  `modules`, `labels` (`compared`, `moved`, `gone`: names gone whose dot or sphere
  still shows), `appeared`, `gone`.
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
| `labels-near` | every function's name is within 40 points of its dot, edge to edge, and no other lit dot is nearer it (by over 2 points); every module's name within 40 points of its sphere's rim | always |
| `labels-clear` | no name overlaps another, and a function's name covers no other lit dot | always |
| `named-on-screen` | no name is cut off by the graph's edge or lies under an overlay (the journey panel; not the search box, which opens over the graph for a moment) | unless the key help is open |
| `text-sizes` | function names are 11 to 18 points; at most 4 font sizes show at once | always |
| `panel-agrees` | each numbered bar's function carries that number on the graph, no other function does; each bar reads the name the graph gives its function (what its label leads with after its numbers); with a cursor on the graph, exactly one cursor bar shows, for that function | journey bars drawn |
| `expanded-fill` | the expanded module's (`state.expanded`) diameter is 60–90% of the free area's smaller side (the graph less a docked panel), and it lies inside it | after `x` or a double click, when expanded; or when the cursor took the expansion to another module |
| `framed` | after R: every sphere inside the free area, camera at the usual view the map opens at (yaw and pitch within 0.05 of it); after F: every lit dot (or the expanded module) inside it with a 4-point margin | when F or R is the last step since the snap before that moves the view |
| `zoom-to-pointer` | the sphere nearest the pointer (else any sphere, else any dot, drawn in both snaps) moved as a zoom about the pointer moves it (within 4 points plus 5% of its distance) | one wheel turn, nothing else that moves, since the snap before; not one that panned to hold the map, or the sphere it zooms to, in the free area (`state.camera.held`) |
| `zoom-bounds` | some of the map is on screen, and zoomed out it still spans 40% of the free area; zoomed in, the sphere under the pointer spans at most 92% of the free area's smaller side, and from 88% lies inside it | after a wheel turn |
| `nothing-moved` | no dot, sphere or name moved, nor the camera, since the snap before (names matched by function, not by what they say), and no name went while its dot or sphere still shows (unless a click selected another function); after `,` `.` or a badge the camera may only draw back, and only when something that came in would have been off the free area from where it was, or a sphere that came in (any, when the panel grew) cut by it or without room for its name; the world may not move | every step since is one that must move nothing |
| `focus-on-screen` | the selection, the start and the cursor, where drawn (expanded, those in the module expanded), lie inside the free area | unless an expand since framed a module, or the camera is where the user's hands put it; then still the cursor, when it moved expanded in a journey |
| `camera-outside` | the camera is inside no sphere | always |
| `map-on-screen` | when the whole map would fit in the free area, 90% of it (by sphere area) is there, all of it after a pan; when it would not, after a pan, it lies over the free area's middle | after a wheel turn out, a pan, or R |
| `spheres-unclipped` | no sphere is cut by an edge of the graph's part of the window that lies in open window (not the window's edge, not where a panel starts), however big it shows; one the camera is inside aside; nor, unless faint, by the top of the journey panel, unless the camera is where the user's hands put it, a module is expanded or one has just shrunk back | always |
| `arrow-sizes` | every arrow on screen is at most 2.5 points wide, a module arrow however many calls it carries, with a head 5 to 12 points long (or half as wide, seen end on; the whole map's faint module links need no head, but none over 12); a module arrow has no head of its own where a call drawn along it goes on to its dot | an arrow on screen |
| `module-names` | a module's name is centred over its sphere (within 3 points) just above its top edge (2 below to 8 above), over no other sphere and none of its own dots | a module named |
| `named-when-room` | a lit function with no name would have no room for its pill on any side of its dot, clear of the free area's edge, overlays, other lit dots (and nearer none of them than its own) and the names placed as early in the naming order or earlier (start or selection, cursor, set aside, modules, the start's calls, callers, deeper calls, the rest lit, plain); a shown module with no name, none above its sphere. Each name is as big as the placer laid it out (`name_size`) | always |
| `journey-fills` | the journey's spheres (but faint ones), lit dots (start, calls, callers) and their names span at least 70% of the free area across or down, whichever they come nearer filling, and the dots and spheres lie inside it | in a journey, not expanded, after it opens, a depth change, R, F or Fit, unless the camera is where the user's hands put it; not on a `frames` snap, taken as the transition plays |
| `colours` | the function hovered has no white fill and no ring the selection's color (or white); each function's fill and each call's color is its role's hue: white the start or selection, pink the cursor, violet callers, blue the first level, amber deeper, grey the rest (pink allowed on a call into the cursor); a function that `calls back` into the start has a violet ring; each bar's `color` is its node's hue, but the cursor's function's and a call only `guessed`; a call among the expanded module's functions off the journey is faint (alpha at most 0.5) unless into the function hovered | always |

What a step may move decides which checks judge it:

| kind | steps | may move |
|---|---|---|
| still | `hover`, `hover-label`, `key` arrows, `[`, `]`, `i`, `wait`, `expect`, `frames` | nothing; expanded in a journey, a cursor that moves may pan the camera to keep it in view, or take the expansion to its module |
| click | `click UNIT`, `click-label`, `bar` | nothing; expanded in a journey it may re-centre |
| depth | `key ,`, `key .`, `chip` | the camera, drawn back (framing the journey afresh) to bring on what would show off the free area; nothing in the world |
| expand | `key x`, `doubleclick`, `doubleclick-module` | the view |
| zoom, orbit, pan | `scroll`, `drag`, `pan` | the camera, by hand |
| frame | `key f`, `key r`, `button Fit` | the camera |
| moves | everything else (`enter`, `esc`, `backspace`, `/`, `type`, `search-pick`, `shift+click`, `button`, `click X Y`, `click-empty`, `window`) | the view |

The camera is where the user's hands put it from a wheel turn, an orbit or a pan until
the view next frames itself: F, R, Fit, an expand, or a journey opened or left. Shrinking
back returns the camera to where it stood before the first expand, and so to the
user's hands if it was there. Checks of where the view puts things (`focus-on-screen`, `journey-fills`) leave
such a camera alone.

## The regression suite

`tools/scripts/` holds the core flows of CORE.md on both data sets:

| script | covers |
|---|---|
| `axum-browse`, `ord-browse` | click and click-label select, nothing moves; hover; click on empty space deselects; `/` search selects in place; F; wheel in and out about a function; wheel out to the bound; R |
| `axum-journey`, `ord-journey` | Enter opens the journey; cursor by arrows, `[ ]`, a bar, a click on a call and on a name, nothing moving; `.` and `,` depth; `i` outside callers; Esc back to Browse and the camera from before; Back and Forward |
| `axum-expand`, `ord-expand` | X expands to fill the view, wheel and orbit while expanded, X back to the saved camera; double-click a module; Esc collapses; the same in a journey and browsing |
| `axum-camera` | double-click on a function expands its module; Back and Forward while expanded; F expanded; the wheel's near bound; X back exactly; a click on empty space moves no camera; in a journey a double click on another module, Esc collapses, R from the usual view, Esc back to browsing as it was |
| `ord-camera` | the wheel's near bound and out to the whole map in the middle; a journey framed into the free area by Enter, F and the panel's Fit alike; expanded, the cursor taking the expansion to its module and a level less moving nothing; R expanded shrinks back first; X with nothing selected expands the module in the middle; a double click keeps the selection |
| `ord-framing` | journeys framed with their spheres whole and clear of the panel, at depth 1 and 4; the wheel in stopping with the sphere under the pointer in the free area; a pan keeping the map in the free area, or over its middle |
| `axum-follow` | expanded in a journey, the expansion following the cursor from module to module and back, by keys and a bar; X back to the camera from before the first expand; a module expanded by a double click staying through a level more |
| `axum-steps` | the harness's own steps: `search-pick`, `bar` by function, `chip`, `frames`, `button Fit` judged as F, `pan`; `map-on-screen` after R |
| `axum-layout` | one sizing rule for every sphere, dots inside their spheres and apart, one size per role, few text sizes; a click on a lit function moves nothing; in a journey `.` and `i` move nothing and keep dots inside their modules; expanded, spheres apart and sized alike |
| `axum-consistency`, `ord-consistency` | the same action the same everywhere: the journey keys browsing do nothing, Esc browsing clears the selection, a search in a journey moves the cursor or sets aside (drawn), the depth keys stop at the deepest level and at 1, with `i` on a cursor step moves nothing; a call that calls the start back ringed violet browsing and in a journey; Esc steps out one level at a time |
| `axum-look`, `ord-look` | how the graph is drawn: arrow sizes, module names above their spheres, names wherever they fit (browsing too), hover unlike the selection, an expanded module showing and naming all its functions, a depth change and `i` taking no name away |
| `axum-callers` | the way up a journey: ↑ from the start into the row of its callers, ← → along it, ↓ back, Enter and Shift-click open a caller's journey, a caller's bar sets it aside; `i` with nothing calling in from outside stays off; `inside` on the top bar; search ranks a function's own name before its module's |
| `axum-search` | search: ↓ and Enter take the row highlighted; a route found by its path and its function's name, listed once; reopened, the last query is selected and typing replaces it; Esc from a journey selects its start |
| `ord-panel` | the panel on a crowded journey: bars keep their numbers, one cursor bar, `,` takes a cursor folded away up to the call still drawn; a smaller window; Esc selects the start; `↺ calls itself` (Finder::walk); a wide journey that scrolls (Index::walk) |

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
- Not faked: trackpad gestures (pinch, two-finger orbit), Shift-drag pan, hovering a
  journey bar, holding a key. `scroll` is a mouse wheel; `drag` a left-button orbit;
  `pan` a right-button drag.
- The real mouse moving over the window mixes its events with the script's.
- `click` aims at a dot's centre; a dot mostly hidden behind another is clicked where the
  other is, and `trouble` says so. `click-label` aims at the name instead.
- Checks see what the snap's JSON reports: a dot drawn but missing from the JSON is
  beyond them. Arrows are painted on screen and rings drawn as gizmos, neither of
  which the scene can give back, so what draws them notes each in `drive::Marks`
  (`arrows::draw`, `graph_view::draw_graph`); the journey panel notes its bars
  and badges (`drive::note_bars`) and the search box its rows
  (`drive::note_search_rows`). A new way of drawing these must note them too, or the
  JSON loses them. Idle CPU is not checked.
- `named-when-room` tries four spots round a dot, the placer eight and slides along
  them; and it takes a dot's spot from its own size, where the placer keeps names as
  far off as the cursor's dot: it may call room the placer could not take.
