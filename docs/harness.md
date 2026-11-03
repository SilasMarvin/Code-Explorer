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
holds a lock so only one viewer window renders at a time across every agent. A
session's window never takes the keyboard or brings codeviz to the front, and covers
nothing: borderless, over other windows but off screen except a 4-point corner at a
screen's bottom right (or bottom left), and click-through. `live` opens as usual.

## Commands

| command | does |
|---|---|
| `tools/drive run NAME SCRIPT [--wt DIR] [-- VIEW ARGS]` | one session into `$CODEVIZ_SESSIONS/NAME/`; exit 0 when clean, 1 when anything failed, 2 when it could not run |
| `tools/drive suite [--only TEXT]... [--prefix P]` | every `tools/scripts/*.txt` (or those whose name holds TEXT), names checked first, then one window at a time; prints the table, writes `$CODEVIZ_SESSIONS/<P>suite.json` |
| `tools/drive check SCRIPT...` | reads each script and looks up what it names, without a window |
| `tools/drive read NAME` | the summary of a session already run |
| `tools/drive shot NAME -- VIEW ARGS [--keys "x ."] [--hover UNIT]` | one picture: `$CODEVIZ_SHOTS/NAME.png`, `NAME.json` (the snap), `NAME.truth.json` (its `content`) |
| `tools/drive live [DIR] [--limit S] -- VIEW ARGS` | an interactive window under the memory guard and the render lock, closed after S seconds (1800) |
| `tools/drive idle [--wt DIR] [--seconds N] -- VIEW ARGS` | opens the viewer as a person would (no script), lets it settle 10 s, and prints the share of a core it uses over N seconds (10) left alone; keep the mouse off the window |

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
`waited_ms` how long the harness waited; a `pan` also logs `view_moved`, `moved_by`
(how far what the camera looked at moved on screen, in points) and `dragged_by`. A pan
follows the hand: one that moves more than a tenth (or 4 points) off what was dragged
fails unless a notice said why (the map's edge, zoomed all the way out), and one that
says the map's edge stopped it yet moved as dragged fails too. Pointer steps go through the window's real
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
| `expect notice TEXT` | the notice the step before said holds TEXT (any case), as for a key that does nothing; one left from an earlier step does not count |
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
  the free area; `farthest`: zoomed out as far as the wheel goes), `pointer`.
- `nodes`: every function drawn: `name`, `unit`, `module`, `roles` (`start`,
  `selection`, `cursor`, `aside`, `caller`, `journey level N`, `calls back` for a
  call that calls the start (or the selection) back, `opened`
  for a function of the module expanded that nothing else lights, `hovered`, `guessed`
  for a call every shown run of which is only guessed or handed off, drawn faint,
  `speck` for a faint unnamed dot), `numbers` (in the journey), `at`, `radius` (on
  screen), `world`, `world_radius`, `on_screen`, `label`, `label_rect`, `name_size`
  (how big the placer laid its name's text out, room found for it or not), `name_spot`
  (the one place its name was tried as the view is: the side it holds from the neutral
  view, or its home; a name stays there or goes, never moves), `kept_off` (how far from
  its middle names keep: a dot drawn bigger than a speck as far as the cursor's),
  `crowded_by`,
  `color` (its fill, `#rrggbbaa`), `rings` (each ring drawn round it: `radius` on
  screen, `color`).
- `held`: where the journey's calls a deeper level shows would be (`name`, `unit`,
  `at`, `radius`): names keep off them as off the dots drawn.
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
  `role` (in words), `color` (as drawn), `guessed` (only guessed or handed off, drawn
  faint), `width` (the line, in points), `head` (its length on screen), `head_width`,
  `tip`, `path` (the points its line runs through, its start to its tip: straight,
  or bowed round a dot), `on_screen` (some piece of it crosses the free area).
- `exits`: where a call leaves the free area, labelled with the function at its far
  end (past 6 such functions, with their module and the first few): with a module
  expanded, the calls with one end in it; collapsed in a journey, the calls in from
  outside. `from_unit`, `to_unit`, `far_unit`, `far`, `text`, `rect` (the label's
  pill, one per far function or module, shared by its calls), `at` (where the line
  crosses the edge).
- `module_arrows`: each arrow between modules: `from`, `to`, `calls`, `flow`, `level`,
  `role`, `drawn`, and when drawn `width` (the shaft), `head`, `head_width`, `tip`,
  `on_screen`.
- `bars`: journey bars in view: `number`, `name`, `unit`, `rect`, `cursor`, `lit`,
  `color` (its fill, `#RRGGBB`, with its alpha after where see-through, as a call only
  guessed is), `guessed` (a call only guessed or handed off, drawn faint), `cut_off`, `folded` (calls folded under it), `chip` (the badge that unfolds
  them); then the bars of the start's callers in the row above its bar: `caller` true,
  no `number`, `aside` (the one set aside), `color`.
  `search_rows` (while the search box shows): `text`, `unit`, `rect`, `shown`,
  `highlighted` (what Enter takes). `buttons`: `text`, `rect`, `enabled`, `cut_off`.
  `overlays`: `name`, `rect`, `clipped` (the journey panel, the key help, the search
  box, and a notice while one shows, in the bar along the top). `text`: every other text drawn, with `rect`
  and `size`.
- `content`: what the view means to show: the `start` or the `selection`, its
  callers, its calls (in a journey the disclosed ones, with numbers and modules;
  browsing the selection's own, unnumbered).
- `moved`: against the snap before: `since`, `camera`, `nodes` (`compared`,
  `moved_in_world`, `moved_on_screen`, `most_world`, `most_screen`, `examples`),
  `modules`, `labels` (`compared`, `moved`, `gone`: names gone whose dot or sphere
  still shows), `arrows` (`compared`, `moved`: calls drawn in both whose line runs
  more than 3 points elsewhere), `appeared`, `gone`.
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
| `labels-near` | every function's name is within 40 points of its dot, edge to edge, and no other lit dot is nearer it (by over 2 points), a plain one (`opened`) only for a plain name; every module's name within 40 points of its sphere's rim | always |
| `labels-clear` | no name overlaps another or lies under a notice, no exit label lies over a name, and a function's name covers no other lit dot (a lit name may lie over a plain one, `opened`) | always |
| `named-on-screen` | no name is cut off by the graph's edge or lies under an overlay (the journey panel; not the search box, which opens over the graph for a moment) | unless the key help is open |
| `text-sizes` | function names are 11 to 18 points; at most 4 font sizes show at once | always |
| `panel-agrees` | each numbered bar's function carries that number on the graph, no other function does; each bar reads the name the graph gives its function (what its label leads with after its numbers); with a cursor on the graph, exactly one cursor bar shows, for that function | journey bars drawn |
| `expanded-fill` | the expanded module's (`state.expanded`) diameter takes 45% to 85% of the free area's smaller side (the graph less a docked panel), 4% either way allowed; below 81% none of its dots crowds another (a quarter of them at most nearer than 18 points), since it could grow; an expand from the collapsed view zooms in (the module shows bigger than before, unless it already filled 81%); and it lies inside the free area | after `x` or a double click, when expanded; or when the cursor, the function set aside, or browsing a pick took the expansion to another module; not while the camera is where the user's hands put it (Back to a place seen expanded counts as that) |
| `expanded-drawn` | expanded, the module expanded is drawn (not closed or faint) and some of its functions show in the free area | while expanded |
| `framed` | after R: every sphere inside the free area, camera at the usual view the map opens at (yaw and pitch within 0.05 of it); after F: every lit dot (or the expanded module) inside it with a 4-point margin | when F or R is the last step since the snap before that moves the view, a `window` after it aside (a window sized anew frames again as the view was last framed) |
| `zoom-to-pointer` | the sphere nearest the pointer (else any sphere, else any dot, drawn in both snaps) moved as a zoom about the pointer moves it (within 4 points plus 5% of its distance) | one wheel turn, nothing else that moves, since the snap before; not one that panned to hold the map, or the sphere it zooms to, in the free area (`state.camera.held`) |
| `zoom-bounds` | some of the map is on screen, and zoomed out it still spans 40% of the free area; zoomed in, the sphere under the pointer spans at most 95% of the free area's smaller side, and from 88% lies inside it | after a wheel turn |
| `nothing-moved` | no dot, sphere or name moved, nor the camera, nor a call's line (more than 3 points, unless a click selected another function), since the snap before (names matched by function, not by what they say), and no name went while its dot or sphere still shows (unless a click selected another function, or a depth change drew the camera back); after `,` `.` or a badge the camera may only draw back, and only when something that came in would have been off the free area from where it was, or a sphere that came in (any, when the panel grew) cut by it or without room for its name; the world may not move | every step since is one that must move nothing |
| `focus-on-screen` | the selection, the start and the cursor, where drawn (expanded, those in the module expanded), lie inside the free area; a function picked browsing since (a click, a search, Back), or set aside in a journey since, is drawn and lies inside it, in whatever module | unless an expand since framed a module, or the camera is where the user's hands put it; then still the cursor, when it moved expanded in a journey |
| `camera-outside` | the camera is inside no sphere | always |
| `map-on-screen` | when the whole map would fit in the free area, 90% of it (by sphere area) is there, all of it after a pan; when it would not, after a pan, it lies over the free area's middle; zoomed out as far as the wheel goes (`farthest`), it covers the free area's middle | after a wheel turn out, a pan, or R |
| `spheres-unclipped` | no sphere is cut by an edge of the graph's part of the window that lies in open window (not the window's edge, not where a panel starts), however big it shows; one the camera is inside aside; nor, unless faint (but the sphere holding the function set aside), by the top of the journey panel, expanded too, unless the camera is where the user's hands put it or a module has just shrunk back | always |
| `arrow-sizes` | every arrow on screen is at most 2.5 points wide, a module arrow however many calls it carries, with a head 5 to 12 points long (or half as wide, seen end on; the whole map's faint module links need no head, but none over 12); the whole map's links all one width; a module arrow has no head of its own where a call drawn along it goes on to its dot | an arrow on screen |
| `module-names` | a module's name is centred over its sphere (within 3 points) just above its top edge (2 below to 8 above), or browsing, smaller (11 points) just inside its top (0 to 8 below it); over no other sphere and none of its own dots (inside, none lit) | a module named |
| `named-when-room` | a lit function with no name would have no room for its pill where the placer tried it (`name_spot`; without one, on any side of its dot), clear of the free area's edge, overlays, the calls a deeper level shows (`held`), other lit dots as far as names keep from them (`kept_off`; a plain one, `opened`, only for a plain name; and nearer none of them than its own) and the names placed as early in the naming order or earlier (start or selection, cursor, set aside, modules, the start's calls, callers, deeper calls, the rest lit, plain): a plain name or dot never excuses a lit one; a shown module with no name, none above its sphere, nor browsing, smaller inside its top clear of its lit dots and every name. Each name is as big as the placer laid it out (`name_size`) | always |
| `journey-fills` | the journey's spheres (but faint ones, other than the one holding the function set aside), lit dots (start, calls, callers) and their names span at least 70% of the free area across or down, whichever they come nearer filling, and the dots and spheres lie inside it | in a journey, not expanded, after it opens (a `--focus` session as it opens too), a depth change, R, F or Fit, unless the camera is where the user's hands put it; not on a `frames` snap, taken as the transition plays |
| `colours` | the function hovered has no white fill and no ring the selection's color (or white); each function's fill and each call's color is its role's hue: white the start or selection, pink the cursor, violet callers, blue the first level, amber deeper, grey the rest (pink allowed on the journey's call into the cursor); the function set aside keeps its role's fill (grey off the journey), never white, with a white ring; a function that `calls back` into the start has a violet ring; a dot only `guessed` darker than its role's color; each bar's `color` is its node's hue (a `guessed` one its role's, at alpha at most 0.5), but the cursor's function's; no call reads white over the ground; only the journey's call into the cursor and the calls out of the function set aside are drawn stronger or wider than the rest, so hovering lights none; a call among the expanded module's functions off the journey, from the function set aside too, and a call only guessed are faint (alpha at most 0.5) | always |
| `arrows-clear` | no call's line (its `path`) runs more than 1.35× as long as straight; a bent one runs through no lit dot nor the ring round one (1 point clear; the pointer's ring aside), but its own ends'; no line runs through another lit function's name (in a journey, one no deeper than the line's ends: a deeper name comes with a level more, which bends no line), nor a bent one through its own ends' names away from where it leaves its dot. A straight line through a dot is drawn broken round it, as no gentle bend clears it. An expanded module's plain functions, the faint calls among them and the calls only guessed aside | a call drawn |
| `names-distinct` | no two functions read the same name on the graph within one module, among the journey's bars, or among the search's rows | a function named |
| `exits-named` | each call that should be labelled (with a module expanded, one with one end in it; collapsed in a journey, one in from outside) drawn from inside the free area to off it has an exit label inside it naming the far function, or its module; no label for a call with both ends in the module expanded or neither; no two labels name one far function or lie over each other | a module expanded, or a journey; a call leaving the free area or a label drawn |
| `calls-drawn` | each call into the selection (or the start) and each of its own, between two dots on screen apart by more than their radii, a head and its gaps, is drawn | a selection or a journey, a call between two dots on screen |

What a step may move decides which checks judge it:

| kind | steps | may move |
|---|---|---|
| still | `hover`, `hover-label`, `key` arrows, `[`, `]`, `i`, `wait`, `expect`, `frames` | nothing; expanded in a journey, a cursor that moves may pan the camera to keep it in view, or take the expansion to its module |
| click | `click UNIT`, `click-label`, `bar` | nothing; expanded in a journey it may re-centre |
| depth | `key ,`, `key .`, `chip` | the camera, drawn back (framing the journey afresh) to bring on what would show off the free area; nothing in the world |
| expand | `key x`, `doubleclick`, `doubleclick-module` | the view |
| zoom, orbit, pan | `scroll`, `drag`, `pan` | the camera, by hand |
| frame | `key f`, `key r`, `button Fit` | the camera |
| resize | `window` | the camera, framing the view again as it was last framed, unless the hands moved it since |
| moves | everything else (`enter`, `esc`, `backspace`, `/`, `type`, `search-pick`, `shift+click`, `button`, `click X Y`, `click-empty`) | the view |

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
| `axum-camera` | double-click on a function expands its module; Back and Forward while expanded; F expanded; the wheel's near bound; X back exactly; a click on empty space moves no camera; in a journey a double click on another module, Esc collapses, R from the usual view, Esc back to browsing as it was; Esc with nothing to close and the wheel at either end saying so; a pan expanded keeping the module drawn; a window sized anew after R framing again |
| `ord-aside` | expanded in a journey, the expansion following the function set aside and back to the cursor's module when it goes; Back within the journey putting back the module each place had expanded; Esc from a journey entered browsing expanded putting that back |
| `ord-camera` | the wheel's near bound and out to the whole map in the middle; a journey framed into the free area by Enter, F and the panel's Fit alike; expanded, the cursor taking the expansion to its module and a level less moving nothing; R expanded shrinks back first; X with nothing selected expands the module in the middle; a double click keeps the selection |
| `ord-framing` | journeys framed with their spheres whole and clear of the panel, at depth 1 and 4; the wheel in stopping with the sphere under the pointer in the free area; a pan keeping the map in the free area, or over its middle |
| `axum-follow` | expanded in a journey, the expansion following the cursor from module to module and back, by keys and a bar; X back to the camera from before the first expand; a module expanded by a double click staying through a level more |
| `axum-size` | how big a module expands: a small start module zooming in, and the cursor taking the expansion to small modules, each at its own size with nothing half under the panel; the journey as it opens filling the view; the notices for Back with no history, Enter and F with nothing selected; a pan stopped at the map's edge, and zoomed all the way out the map kept in the middle |
| `axum-steps` | the harness's own steps: `search-pick`, `bar` by function, `chip`, `frames`, `button Fit` judged as F, `pan`; `map-on-screen` after R |
| `axum-layout` | one sizing rule for every sphere, dots inside their spheres and apart, one size per role, few text sizes; a click on a lit function moves nothing; in a journey `.` and `i` move nothing and keep dots inside their modules; expanded, spheres apart and sized alike |
| `axum-consistency`, `ord-consistency` | the same action the same everywhere: the journey keys browsing do nothing, Esc browsing clears the selection, a search in a journey moves the cursor or sets aside (drawn), the depth keys stop at the deepest level and at 1, with `i` on a cursor step moves nothing; a call that calls the start back ringed violet browsing and in a journey; Esc steps out one level at a time |
| `axum-look`, `ord-look` | how the graph is drawn: arrow sizes, module names above their spheres, names wherever they fit (browsing too), hover unlike the selection, an expanded module showing and naming all its functions, a depth change and `i` taking no name away |
| `axum-callers` | the way up a journey: ↑ from the start into the row of its callers, ← → along it, ↓ back, Enter and Shift-click open a caller's journey, a caller's bar sets it aside; `i` with nothing calling in from outside stays off; `inside` on the top bar; search ranks a function's own name before its module's |
| `axum-search` | search: ↓ and Enter take the row highlighted; a route found by its path and its function's name, listed once; reopened, the last query is selected and typing replaces it; Esc from a journey selects its start |
| `ord-follow` | a big module expanding to the band; browsing expanded, a pick in another module taking the expansion there and Back bringing it back; Back across journeys putting back the expansion the place had; a search off the journey showing its sphere whole; X with a function set aside expanding its module |
| `ord-routes` | calls' lines bent by nothing that only reads: a level more or less, `i`, a cursor step, the pointer, collapsed and expanded; none looping past 1.3× straight |
| `ord-exits` | where calls leave the free area: one label per far function or, past six, per module, over no name or other label; browsing expanded; a narrow window whose panel grows on `.` |
| `ord-names` | names stay or go, never move: the cursor stepping onto a call beside a name; expanded in a journey, the journey's calls named before any plain function |
| `axum-visual`, `ord-visual` | how a journey is drawn: no call through a lit dot or name, hovering lighting no call, the function set aside in its own color with a white ring, a guessed call faint; the whole map's links one width, module names above or inside their spheres; two functions alike told apart in the search; both calls of a function that calls the selection and is called by it, browsing |
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

- Black PNGs: macOS stops drawing a window it counts as covered or wholly off screen;
  the harness keeps a corner of the window on screen and on top. A snap of a covered
  window is retried offscreen, which also comes out black today, saying so in
  `trouble`. The JSON is right regardless.
- Not faked: trackpad gestures (pinch, two-finger orbit), Shift-drag pan, hovering a
  journey bar, holding a key. `scroll` is a mouse wheel; `drag` a left-button orbit;
  `pan` a right-button drag.
- `click` aims at a dot's centre; a dot mostly hidden behind another is clicked where the
  other is, and `trouble` says so. `click-label` aims at the name instead.
- Checks see what the snap's JSON reports: a dot drawn but missing from the JSON is
  beyond them. Arrows are painted on screen and rings drawn as gizmos, neither of
  which the scene can give back, so what draws them notes each in `drive::Marks`
  (`arrows::draw`, `graph_view::draw_graph`); the journey panel notes its bars
  and badges (`drive::note_bars`) and the search box its rows
  (`drive::note_search_rows`). A new way of drawing these must note them too, or the
  JSON loses them. Sessions redraw every frame, so idle CPU is measured apart, by
  `tools/drive idle`.
- `named-when-room` judges a name the placer tried by the one spot it tried
  (`name_spot`), which a name holds so it never moves: it trusts the placer's choice
  of spot, and judges only whether that spot was free. Without a `name_spot` it tries
  four spots round a dot, the placer eight and slides along them, from the dot's own
  size: it may call room the placer could not take.
