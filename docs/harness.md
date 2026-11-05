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
| `tools/drive run NAME SCRIPT [--wt DIR] [-- VIEW ARGS]` | one session into `$CODEVIZ_SESSIONS/NAME/`; exit 0 when clean, 1 when anything failed, 2 when it could not run or the window was not drawn |
| `tools/drive suite [--only TEXT]... [--prefix P]` | every `tools/scripts/*.txt` (or those whose name holds TEXT), names checked first, then one window at a time; prints the table, writes `$CODEVIZ_SESSIONS/<P>suite.json` |
| `tools/drive check SCRIPT...` | reads each script and looks up what it names, without a window |
| `tools/drive read NAME` | the summary of a session already run |
| `tools/drive shot NAME -- VIEW ARGS [--keys "x ."] [--hover UNIT]` | one picture: `$CODEVIZ_SHOTS/NAME.png`, `NAME.json` (the snap), `NAME.truth.json` (its `content`) |
| `tools/drive live [DIR] [--limit S] -- VIEW ARGS` | an interactive window under the memory guard and the render lock, closed after S seconds (1800) |
| `tools/drive idle [--wt DIR] [--seconds N] -- VIEW ARGS` | opens the viewer as a person would (no script, drawing only when something changes), but tucked away as a session's window is (`CODEVIZ_TUCKED`), lets it settle 10 s, and prints the share of a core it uses over N seconds (10) left alone |

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
| `doubleclick UNIT` | double-clicks the function's dot, which expands its module; the selection, the journey's cursor and the function set aside stay as they were before the first click (a double click picks nothing) |
| `doubleclick-module MODULE` | double-clicks the module's sphere at the spot farthest from its dots and names; where no spot is clear of its dots, on a dot (which expands the module too), with a note |
| `bar N` | clicks the journey bar numbered N (`2`, `1.2`), or the bar of the function N (`Explorer::spotlit`, the start's bar first), or the bar that reads N; else the bar of the start's caller N, in the row above the start's |
| `chip N` | clicks the badge on bar N (as `bar` finds it) that unfolds the calls folded under it, that call's alone, and steps in |
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

After every step the harness waits until the camera, the spheres, the names and the
overlays (the search box growing as what it finds is laid out) stop moving (at least 0.5 s, at most 8 s, else the step's trouble says it was still moving).
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
| `expect depth N` | the journey shows N levels: its deepest call shown, in a branch opened alone too |
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
  (each snap's checks in full); `window_not_drawn` (snaps with no PNG, see "Limits").
- `NN-label.png`: the window at that snap, in window points, the same as the JSON's.
- `NN-label.json`: what that snap shows (below).
- `session.json`: every step with `ok`, `trouble`, `note`, `pointer` (where the pointer
  was as it began), `settled_ms`, `waited_ms` and `then` (the state after it);
  `script.txt`, `args.txt`, `viewer.log`.

The JSON says what is there; the PNG says whether a person could see it. Read both.

### A snap's JSON

Positions are window points; rectangles `[x0, y0, x1, y1]`; world positions are the
scene's units.

- `window`, `graph_area` (the graph's part of the window, under the top bar),
  `free_area` (the graph less a docked panel: where names go, and what the checks
  call the free area), `camera`
  (`at` in the world, `facing`, `target_at`: where what it looks at shows), `snap`.
- `state`: `mode` (`browse`, `journey`), `selection` (browsing), `start` (the
  journey's), `start_module` (the module in the middle: the start's, or the
  selection's), `cursor` (`number`, `name`), `aside` (the function set aside in a
  journey), `nucleus`
  (`modules`, `expanded`), `expanded` (the module expanded, by name), `depth`, `deepest` (in a journey, how many levels below the start its deepest call is), `outside_callers`,
  `outside_waiting` (of the callers I brings in, how many have no room to be drawn), `hovered` (the function the
  pointer has rested on, as drawn) and `under_pointer` (what the pointer is on now),
  `search`, `help`, `back`, `forward`, `camera` (where it stands that frame, on its
  way or not: `target`, `distance`, `yaw`, `pitch`, `held`: the wheel turns since it
  last turned the other way panned to hold the map, or the sphere they zoom to, in
  the free area; `farthest`: zoomed out as far as the wheel goes), `pointer`.
- `nodes`: every function drawn: `name`, `unit`, `module`, `roles` (`start`,
  `selection`, `cursor`, `path` (a call on the cursor's path down from the start), `aside`, `caller`, `journey level N`, `calls back` for a
  call that calls the start (or the selection) back, `opened`
  for a function of the module expanded that nothing else lights, `outside` for a
  caller from off the journey that I brings in, `hovered`, `guessed`
  for a call every shown run of which is only guessed or handed off, drawn faint,
  `dimmed` for a call of a journey two levels deep or more off the cursor's path, not
  one the cursor makes, drawn dimmer,
  `speck` for a faint unnamed dot, `plain` for one drawn as a speck but named),
  `numbers` (in the journey), `at`, `radius` (on screen), `world`, `world_radius`,
  `on_screen`, `label`, `label_rect`, `name_size` (how big the placer laid its name's
  text out, room found for it or not), `arrivals` (each call coming into it, from its
  dot back, that its name keeps off), `home` (left out, the place its name had when
  names were last placed in this view, which a name placed before it took), `kept_off` (how far from
  its middle names keep: a dot drawn bigger than a speck as far as the cursor's),
  `crowded_by`,
  `color` (its fill, `#rrggbbaa`), `rings` (each ring drawn round it: `radius` on
  screen, `color`).
- `modules`: each sphere: `name`, `body` (`nucleus`, the journey's start's; `open`;
  `faint`), `shown`, `functions`, `named`, `at`, `radius`, `world`, `world_radius`,
  `room` (the radius of the ball each of its functions keeps to itself in the world),
  `on_screen`, `label`, `label_rect`, `name_size`.
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
  or bowed round a dot or off another call), `gaps` (where along it, in points from
  its start, the line does not show: never, for a call), `under` (the functions
  whose dots it runs under, each showing over it), `on_screen` (some piece of it
  crosses the free area).
- `exits`: where a call leaves the free area, labelled with the function at its far
  end (past 6 such functions, with their module and the first few): with a module
  expanded, every call lit, whichever modules its ends are in; collapsed in a
  journey, the calls in from outside. `from_unit`, `to_unit`, `far_unit`, `far`, `text`, `rect` (the label's
  pill, one per far function or module, shared by its calls), `at` (where the line
  crosses the edge).
- `module_arrows`: each arrow between modules: `from`, `to`, `calls`, `flow`, `level`,
  `role`, `drawn`, and when drawn `width` (the shaft), `head`, `head_width`, `tip`,
  `on_screen`.
- `bars`: journey bars in view: `number`, `name`, `text` (what it reads, as drawn), `unit`, `rect`, `cursor`, `lit`,
  `color` (its fill, `#RRGGBB`, with its alpha after where see-through, as a call only
  guessed is), `guessed` (a call only guessed or handed off, drawn faint), `cut_off`, `folded` (calls folded under it), `chip` (the badge that unfolds
  them); then the bars of the start's callers in the row above its bar: `caller` true,
  no `number`, `aside` (the one set aside), `color`, `text`. `panel` (while the journey
  panel shows): `below`, how many of its bars lie below its foot, cut off or out of view,
  and `rows_track`, whether a track down its side shows which rows are in view;
  `beside`, how many lie past its left or right edge (`left`, `right`), and
  `across_track`, whether a track along its foot says so, and `left_mark`,
  `right_mark` (`rect`, `count`), the mark at each side saying how many; `cursor_cut_off`,
  the cursor's bar showing less of itself than the panel could. The start's bar stays
  put under the row of its callers while the rows below it scroll; a bar's `cut_off`
  is against where its row shows.
  `search_rows` (while the search box shows): `text`, `unit`, `rect`, `shown` (wholly in view),
  `highlighted` (what Enter takes). `buttons`: `text`, `rect`, `enabled`, `cut_off`.
  `overlays`: `name`, `rect`, `clipped` (the journey panel, the key help, the search
  box, and a notice while one shows, in the bar along the top, or while none does, how
  many functions lit have no room for a name). `text`: every other text drawn, with `rect`
  and `size`.
- `content`: what the view means to show: the `start` or the `selection`, its
  callers, its calls (in a journey the disclosed ones, with numbers and modules;
  browsing the selection's own, unnumbered).
- `layout`: what the layout is, whatever the view lights: `digest` (of where every
  module and function sits), and the mean length in the world of the calls between
  modules' middles (`modules`), within a module and between two, function to function
  (`inside`, `across`), each with `shuffled`, the same with the modules, or each
  module's functions, shuffled among themselves, and `calls`.
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
constant at the top of that file, or the product's own where the product sets it (a
call's width, the expanded module's band, the room framing leaves). They come in
families sharing one geometry: in view (what lies in the free area after the steps
that put it there), names, and calls.

| check | holds when | applies |
|---|---|---|
| `dots-in-spheres` | every dot lies inside its module's sphere, in the world (1% slack) | always |
| `dots-apart` | no two functions of one module lie nearer, in the world, than twice the room each keeps (`room`): no two intersect, at any zoom and from any side (two of different modules lie in two spheres apart) | a module says its room |
| `dot-sizes` | a lit dot is at least 3 points across; no dot is over 16 points in radius; expanded, the module's functions nothing lights (`opened`, not hovered) show smaller than its smallest lit one | always |
| `spheres-apart` | no two spheres intersect, in the world | two spheres or more |
| `sphere-sizes` | the largest sphere is at most 4× the smallest's radius; a module with more functions is bigger, but less than in proportion | always |
| `layout-affinity` | calls run at most 0.8 of the length they would with everything shuffled, between modules and function to function within a module, and 0.9 between two modules' functions (most of such a call runs between the spheres, which a shuffle within each leaves be) (`layout`), each where there are 10 such calls or more | layout facts with 10 calls of a kind |
| `labels-near` | every function's name is within 40 points of its dot, edge to edge, and no other dot drawn, lit, plain or a speck, is nearer it (by over 2 points), but for a lit name one drawn faded (alpha under 0.5) or one nothing lights that its opaque pill covers, or some of, and but for a name shown come what may (the start, the cursor and the calls on its path, the selection, the function set aside) with room nowhere round its dot, as `named-when-room` judges room, and the name of the function hovered, which takes its emptiest side; every module's name within 40 points of its sphere's rim | always |
| `labels-clear` | no name overlaps another or lies under a notice, no exit label lies over a name, a function's name covers no other lit dot (a lit name may lie over a plain one, `opened`) and lies across none of the last 30 points of a call (`into` or `journey`) drawn into its own dot, the side its calls arrive from; but a name shown come what may with room nowhere round its dot; a plain function's name lies across no call lit (`into` or `journey`, not faint) | always |
| `named-on-screen` | no name is cut off by the graph's edge or lies under an overlay (the journey panel; not the search box, which opens over the graph for a moment) | unless the key help is open |
| `text-sizes` | function names are 11 to 18 points; at most 4 font sizes show at once | always |
| `panel-agrees` | each numbered bar's number is on the graph (a call guessed among several functions is drawn, and numbered, only to those the journey runs anyway; drawn nowhere, its bar has no number); each numbered bar's function carries that number on the graph, no other function does; each bar reads the name the graph gives its function (what its label leads with after its numbers), and draws it after its number, whole (its type or module may go first), its type cut short (`T….root`), or, but on the cursor's bar, its name less its type cut short with an ellipsis; never its type alone (`Track…`), never its number alone; with a cursor on the graph, exactly one cursor bar shows, for that function, the
cursor's name leads with that bar's number and each name on its path with a number on
the way down to it; a start with callers has their row above its bar, and the start's bar shows below that row; the depth shown is from 1 to the journey's deepest level, no less than the deepest level a call shown (or a bar) is at, and 1 in a journey opened anew (from browsing, or at another start) but by Back or Forward | journey bars drawn |
| `expanded-fill` | the expanded module's (`state.expanded`) diameter takes 45% to 85% of the free area's smaller side (the graph less a docked panel), 4% either way allowed; below 81% none of its dots crowds another (a quarter of them at most nearer than 18 points), since it could grow; an expand from the collapsed view zooms in (the module shows bigger than before, unless it already filled 81%); and it lies inside the free area, its middle within 4 points of the free area's | after `x` or a double click, when expanded; or when the cursor, the function set aside, or browsing a pick took the expansion to another module; not while the camera is where the user's hands put it (Back to a place seen expanded counts as that) |
| `expanded-drawn` | expanded, the module expanded is drawn (not closed or faint) and some of its functions show in the free area | while expanded |
| `framed` | after R: every sphere inside the free area, camera at the usual view the map opens at (yaw and pitch within 0.05 of it); after F: every lit dot (or the expanded module) inside it with a 4-point margin; after a window sized anew with the camera where the user's hands put it, the camera's target and distance as at the snap before | when F or R is the last step since the snap before that moves the view, a `window` after it aside (a window sized anew frames again as the view was last framed); or only `window` and still steps since, after an F or R before the snap before with nothing moving the view between, or with the camera where the hands put it |
| `zoom-to-pointer` | the sphere nearest the pointer (else any sphere, else any dot, drawn in both snaps) moved as a zoom about the pointer moves it (within 4 points plus 5% of its distance) | one wheel turn, nothing else that moves, since the snap before; not one that panned to hold the map, or the sphere it zooms to, in the free area (`state.camera.held`) |
| `zoom-bounds` | some of the map is on screen, and zoomed out it still spans 40% of the free area; zoomed in, the sphere under the pointer spans at most 95% of the free area's smaller side, and from 88% lies inside it; the module expanded may span more, as its functions' names need, over the free area's middle then, and the wheel stops on it under twice the free area's smaller side only with 90% of its functions in the free area named | after a wheel turn |
| `nothing-moved` | no dot, sphere or name moved, nor the camera, nor a call's line (more than 3 points, unless a click selected another function), since the snap before (names matched by function, not by what they say), and no name went while its dot or sphere still shows (unless a click selected another function, which may also move a lit function's name to another side of its dot and a module's name inside its sphere, or a depth change drew the camera back; and but for a name that gave way to one placed before it, as a step that names others first may (the cursor's path, a level more, I): one whose old place now lies under a name that came or moved there or a dot that came, or one that was the cursor's or on its path and is no longer, or lit and now plain, may go, never move, but a module's holding anything lit, named wherever there is room, may move); after `,` `.` ↓ or a badge the camera may only draw back, about the free area's middle (what showed there stays within 8 points of it), and only when something that came in would have been off the free area from where it was, or a sphere that came in (any, when the panel grew) cut by it or without room for its name; the world may not move; after X in a journey, no journey dot moved in the world; expanded in a journey, a click or a cursor step may pan the camera, and nothing else may move | every step since is one that must move nothing, or X in a journey; not a cursor step, nor ← → ↓ along the start's callers, that took the expansion to another module |
| `focus-on-screen` | the selection, the start and the cursor, where drawn (expanded, those in the module expanded), lie inside the free area, shrunk back too; a function picked browsing since (a click, a search), or set aside in a journey since, is drawn and lies inside it, in whatever module; after only expands and shrinks back (X, a double click), browsing, the selection is the one before; in a journey, after nothing but those, the cursor and the function set aside are | unless an expand since framed a module, Back or Forward put back a camera, or the camera is where the user's hands put it; then still the cursor, when it moved expanded in a journey; the selection after an expand always |
| `camera-outside` | the camera is inside no sphere | always |
| `map-on-screen` | when the whole map would fit in the free area, 90% of it (by sphere area) is there, all of it after a pan; when it would not, after a pan, some sphere covers some of the free area's middle third, across and up (the box round them is no bound); zoomed out as far as the wheel goes (`farthest`), it covers the free area's middle | when the last step since that moves the view is a wheel turn out, a pan, or R |
| `spheres-unclipped` | no sphere is cut by an edge of the graph's part of the window that lies in open window (not the window's edge, not where a panel starts), however big it shows; one the camera is inside aside; nor, unless faint (but the sphere holding the function set aside), by the top of the journey panel, just shrunk back too, unless the camera is where the user's hands put it; expanded, only the module expanded (its neighbours are left where they fall, the panel's top cutting them as the window's edges do) | always |
| `arrow-sizes` | every arrow on screen is at most 2.5 points wide, a module arrow however many calls it carries, with a head 5 to 12 points long (or half as wide, seen end on; the whole map's faint module links need no head, but none over 12); the whole map's links all one width; a module arrow has no head of its own where a call drawn along it goes on to its dot | an arrow on screen |
| `module-names` | a module's name lies just above its sphere's top edge (2 below to 8 above), its middle over the sphere (centred, or slid along no more than 0.8 of the radius; within 3 points), or smaller (11 points) inside it, its corners within the rim and above the sphere's middle or starting at most 0.4 of the radius below its top, a big module's (10 functions or more) down to straddling its middle, or smaller (11 points) just above it; over no other sphere and none of its own dots, but inside, those nothing lights, which its opaque pill hides. A module on screen with no name had no room, biggest first: only a bigger module's name, or a function's lit, excuses it; one holding anything lit (not `faint`), the start's or the selection's (`start_module`) among them, has no place with room at all; any other's first place clear of all but functions' names is taken by a function's name placed before it (it goes rather than moves). Says how many of the spheres on screen are named (`named`: named, on screen), pass or fail | a module on screen |
| `named-when-room` | a lit function with no name (a caller folded into its module's count aside, named only under the pointer), its dot in the free area (one off it an exit label carries), would have no room for its pill at any place round its dot the placer tries (every side of it, level, nudged or slid along; a corner only for a pill at most 100 points wide), clear of the free area's edge, overlays, every other dot drawn as far as names keep from it (`kept_off`; a dot nothing lights, as far as it is drawn), but for a lit name one faded or one nothing lights that the pill would cover, or some of, and nearer none of them than its own, off the calls coming into its dot (`arrivals`), and the names placed as early in the naming order or earlier (start or selection, cursor, the calls on its path down from the start, set aside, the journey's other calls in number order, callers, those drawn dimmer off the cursor's path, the rest lit, the callers I brings in, modules holding anything lit, other modules biggest first, plain): a plain name or dot never excuses a lit one, nor a module's name a function's, nor the calls a deeper level shows, not drawn. Each name is as big as the placer laid it out (`name_size`). A lit function left out where its name was when names were last placed in this view, and a name placed before it took that place (`home`, on its node), passes, and fails where that place is free. Pass or fail, `left_out` lists each lit function unnamed with what keeps its name out of each place level with its dot (`blocked_by`), and `lit_left_out` how many lit functions in the free area are unnamed, of how many | always |
| `plain-names` | a plain function's name shows only in the module expanded, or one whose sphere shows at least 130 points across: no wall of names at mid zoom; and there, on a sphere under 400 points across, each of the module's plain functions in the free area is named, or none: no partial wreath | always |
| `journey-fills` | the journey's spheres (but faint ones, other than the one holding the function set aside), lit dots (start, calls, callers) and their names span at least 70% of the free area across or down, whichever they come nearer filling, and the dots and spheres lie inside it | in a journey, not expanded, after it opens (a `--focus` session as it opens too, with no depth change since: that only draws back), F or Fit (R frames the whole map, the journey dimmed in it), unless the camera is where the user's hands put it or Back or Forward put back the camera a place had; not on a `frames` snap, taken as the transition plays |
| `colours` | the function hovered shows at least as big as a lit dot (2.5 points) with a grey ring outside any ring it has, and has no white fill and no ring the selection's color (or white); each function's fill and each call's color is its role's hue: white the start or selection, pink the cursor, violet callers, blue the first level, amber deeper, grey the rest (pink on every call along the cursor's path down from the start, a call only guessed too; other calls into the cursor's function keep their role's); once two levels show, a call of the journey off the path, not the cursor's own, drawn dimmer (alpha at most 0.45), and its dot (`dimmed`) darker than its role's color; the function set aside keeps its role's fill (grey off the journey), never white, with a white ring; a function that `calls back` into the start has a violet ring; a dot only `guessed` darker than its role's color; each bar's `color` is its node's hue (a `guessed` one its role's, at alpha at most 0.5), but the cursor's function's; a bar in the row of the start's callers violet, at alpha at most 0.5 exactly when its caller is folded into its module's count; no call reads white over the ground; only the calls on the path and the calls out of the function set aside are drawn stronger or wider than the rest, so hovering lights none; a call among the expanded module's functions off the journey, from the function set aside too, and a call only guessed off the cursor's path are faint (alpha at most 0.5) | always |
| `arrows-clear` | no call's line (its `path`) runs more than 1.35× as long as straight, nor, no module expanded, strays more than 20 points off the straight line between its ends; every line is whole (no `gaps`); it starts and ends at a dot drawn (or a caller folded away, at its sphere); no line, bent or straight, comes within 2 points of a dot drawn bigger than a speck, or of the ring round one where it is drawn, unless that dot shows over the line, which runs under it where no gentle bow clears it (`under`; lit ones counted as `under_lit`), but its own ends', one crowding where it starts or ends, and under its head; specks never stand in its way; a bent line does not loop through its callee's name before its last 30 points (over those, `labels-clear` keeps the name off; a call I brings in aside, which names keep no side off); no two lit calls from or into one function run along one line, reading as a chain: one within 3 points of the other for over 20, within 48 of where the other ends at its other function, in the free area, away from its own ends (calls fanning out to dots near each other run close only by the dot they share; both ways between two functions lie apart already; calls whose other ends both lie off the free area come in together, carried by their exit labels). A line may pass under any other name: the pill hides it there, and bending round names would loop past the bound (names prefer sides fewer lines cross) | a call drawn |
| `names-distinct` | no two functions read the same name on the graph within one module, among the journey's bars, or among the search's rows | a function named |
| `exits-named` | each call that should be labelled (with a module expanded, any lit call; collapsed in a journey, one in from outside) drawn from inside the free area to off it has an exit label inside it naming the far function, or its module; no label for a plain call among the expanded module's functions, nor collapsed for one not in from outside; no two labels name one far function or lie over each other; a lit call crossing the free area with both ends off it is faint (alpha at most 0.5) and unlabelled | a module expanded, or a journey; a call leaving the free area or a label drawn |
| `calls-drawn` | each call into the selection (or the start) and each of its own, between two dots on screen apart by more than their radii, a head and its gaps, is drawn | a selection or a journey, a call between two dots on screen |
| `bars-in-view` | bars below the journey panel's foot, or past its sides, cut off or out of view, have a track down its side, or along its foot, showing which part is in view, and those past each side a mark at that side saying how many; the cursor's bar shows wholly, or across the panel when wider | a journey panel |
| `fails-kept` | what can fail says so at any depth and on any step: each call shown in both this snap and the one before (by number and name), the start, and each function whose name says all it says in both ("can fail" in its label or not) | the same journey as the snap before |
| `outside-callers` | each caller I brings in has its own line start at its dot, drawn; one drawn has a line of its own: one whose call no line takes is not drawn, decided when what shows changes and kept as the camera turns or zooms; one on screen unnamed, or one with no room or no line to be drawn (`outside_waiting`), is counted in the bar along the top, unless another notice speaks there | I on |

Two checks are the runner's rather than the viewer's. One reads the picture rather than the JSON, in the runner (`tools/drive`) once the
window closes: `look`, the mean RGB level above the ground of what is drawn faint, the
median over each snap, within the ranges in `LOOK` at the top of `tools/drive`. A faint
line's level is its brightest pixel near its middle over the darker side of it, sampled
clear of dots and names; a faint sphere's, its fill at 0.6 of its radius. It applies on
the whole map with nothing selected (the ground, the faint links between modules, the
faint spheres) and expanded (the faint calls among the module's functions). A range
missed means the picture as a whole is drawn differently (blending, colour space,
tonemapping, lighting, Bevy features): a Bevy build without `bevy_post_process` drew
every translucent stroke alpha-squared, links at 1-3 levels for 13-15.

Another reads the layout across runs: `layout-same`, every snap's `layout.digest` the
same, and the same as the last session run with this build on this document (kept in
`$CODEVIZ_SESSIONS/.layouts.json`): the layout is worked out alike every time.

What a step may move decides which checks judge it:

| kind | steps | may move |
|---|---|---|
| still | `hover`, `hover-label`, `key` ↑ ← →, `[`, `]`, `i`, `wait`, `expect`, `frames` | nothing; expanded in a journey, a cursor that moves may pan the camera to keep it in view, or take the expansion to its module |
| click | `click UNIT`, `click-label`, `bar` | nothing; expanded in a journey it may re-centre |
| depth | `key ,`, `key .`, `key down` (on a call whose calls are folded it opens them), `chip` | the camera, drawn back about the free area's middle to bring on what would show off the free area; nothing in the world |
| expand | `key x`, `doubleclick`, `doubleclick-module` | the view |
| zoom, orbit, pan | `scroll`, `drag`, `pan` | the camera, by hand |
| frame | `key f`, `key r`, `button Fit` | the camera |
| resize | `window` | the camera, framing the view again as it was last framed, unless the hands moved it since: then it keeps its target and distance |
| back | `backspace`, `shift+backspace` | the view, put back as the place had it, the camera perhaps where the hands put it |
| moves | everything else (`enter`, `esc`, `/`, `type`, `search-pick`, `shift+click`, `button`, `click X Y`, `click-empty`) | the view |

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
| `axum-journey`, `ord-journey` | Enter opens the journey; cursor by arrows, `[ ]`, a bar, a click on a call and on a name, nothing moving; Back retracing a walk a step at a time; `.` and `,` depth; `i` outside callers; Esc back to Browse and the camera from before; Back and Forward |
| `axum-expand`, `ord-expand` | X expands to fill the view, wheel and orbit while expanded, X back to the saved camera; double-click a module; Esc collapses; the same in a journey and browsing |
| `axum-camera` | double-click on a function expands its module, keeping the selection; Back and Forward while expanded; F expanded; the wheel's near bound; X back exactly; a click on empty space moves no camera; in a journey a double click on another module, Esc collapses, a double click on a call keeping the cursor, R from the usual view, Esc back to browsing as it was; Esc with nothing to close and the wheel at either end saying so; a pan expanded keeping the module drawn; a window sized anew after R framing again, a snapshot between too; a pan begun just above the panel following the hand over it; X on a caller set aside and Esc back to the journey collapsed, the caller still set aside, the next Esc letting it go; expanded and zoomed all the way out, turned and panned, the whole map in the middle |
| `ord-aside` | expanded in a journey, the expansion following the function set aside and back to the cursor's module when it goes; Back within the journey putting back the module each place had expanded; Esc from a journey entered browsing expanded putting that back; X back bringing the function set aside into view; with I on, a panel grown over the callers it brought in drawing the camera back |
| `ord-camera` | the wheel's near bound and out to the whole map in the middle; a journey framed into the free area by Enter, F and the panel's Fit alike; expanded, the cursor taking the expansion to its module and a level less moving nothing; R expanded shrinks back first; X with nothing selected expands the module in the middle; a double click keeps the selection, on a dense module too; a window sized anew leaving the hands' camera where it was |
| `ord-framing` | journeys framed with their spheres whole and clear of the panel, at depth 1 and 4; the wheel in stopping with the sphere under the pointer in the free area; a pan keeping the map in the free area, or a sphere over its middle third, close in too |
| `ord-near` | the wheel in on a big module expanded going on until its names fit, in a journey and browsing, in a small window; X back after a level more grew the panel drawing the journey's spheres clear of it; close in, a pan stopping with a sphere over the middle third, in a small window |
| `axum-follow` | expanded in a journey, the expansion following the cursor from module to module and back, by keys and a bar; X back to the camera from before the first expand; a module expanded by a double click staying through a level more |
| `axum-size` | how big a module expands: a small start module zooming in, and the cursor taking the expansion to small modules, each at its own size with nothing half under the panel; the journey as it opens filling the view; the notices for Back with no history, Enter and F with nothing selected; a pan stopped at the map's edge, and zoomed all the way out the map kept in the middle |
| `axum-steps` | the harness's own steps: `search-pick`, `bar` by function, `chip`, `frames`, `button Fit` judged as F, `pan`; `map-on-screen` after R |
| `axum-layout` | one sizing rule for every sphere, dots inside their spheres and apart, one size per role, few text sizes; a click on a lit function moves nothing; in a journey `.` and `i` move nothing and keep dots inside their modules; expanded, spheres apart and sized alike |
| `axum-consistency`, `ord-consistency` | the same action the same everywhere: the journey keys browsing do nothing, Esc browsing clears the selection, a search in a journey moves the cursor or sets aside (drawn), the depth keys stop at the deepest level and at 1, with `i` on a cursor step moves nothing; a call that calls the start back ringed violet browsing and in a journey; Esc steps out one level at a time; a new journey opening at depth 1, and one left and entered again, Back putting the depth back |
| `axum-look`, `ord-look` | how the graph is drawn: arrow sizes, module names above their spheres, names wherever they fit (plain ones only where their sphere shows 130 points across), hover unlike the selection, an expanded module showing and naming all its functions, a depth change and `i` taking no name away but where one placed before it takes its place |
| `axum-callers` | the way up a journey: ↑ from the start into the row of its callers, ← → along it, ↓ back, Enter and Shift-click open a caller's journey, Back from it passing over the row, a caller's bar sets it aside; `i` with nothing calling in from outside stays off; `inside` on the top bar; search ranks a function's own name before its module's |
| `axum-search` | search: ↓ and Enter take the row highlighted; every row found in view, whole; a route found by its path and its function's name, listed once; reopened, the last query is selected and typing replaces it; Esc from a journey selects its start; the help in both modes |
| `ord-follow` | a big module expanding to the band; browsing expanded, a pick in another module taking the expansion there and Back bringing it back; Back across journeys putting back the expansion the place had; a search off the journey showing its sphere whole; X with a function set aside expanding its module |
| `axum-lines`, `ord-lines` | calls' lines clear of every dot and ring they do not join, or broken round them: browsing, a journey, a module expanded, the function set aside; calls from one dot the same way bowed apart; pink only along the cursor's path; a lit call crossing the expanded view with neither end in it faint and unlabelled |
| `ord-spine` | the cursor's path pink all the way down and the rest of a deep journey dimmer; moving to another call of the same function moving the path, its pink and its numbers with it; lines whole on the map and expanded; a call only guessed on the path pink (Explorer::reroot) |
| `ord-routes` | calls' lines bent by nothing that only reads: a level more or less, `i`, a cursor step, the pointer, collapsed and expanded; none looping past 1.3× straight |
| `ord-exits` | where calls leave the free area: one label per far function or, past six, per module, over no name or other label; browsing expanded; a narrow window whose panel grows on `.` |
| `ord-names` | names stay or go, never move: the cursor stepping onto a call beside a name; expanded in a journey, the journey's calls named before any plain function; I naming the caller it brings in; the calls on the cursor's path named as it walks down |
| `ord-names-room` | names where there is room: a narrow whole map naming its big modules, inside their spheres where the room above is taken; a fresh selection placing its names afresh, its modules named, plain names all or none and off the calls lit; the calls a deeper level shows keeping out no name; no name over a folded caller nor on the start's own incoming call; expanded, a lit name over the plain dots it must, and plain dots smaller than lit |
| `ord-names-browse`, `axum-names` | names browsing: module names on a narrow and a wide whole map; no plain names at mid zoom; the function hovered standing out; a click inside an expanded module moving no plain name; no name nearer another dot than its own |
| `axum-visual`, `ord-visual` | how a journey is drawn: no call through a lit dot or name, hovering lighting no call, the function set aside in its own color with a white ring, a guessed call faint; the whole map's links one width, module names above or inside their spheres; two functions alike told apart in the search; both calls of a function that calls the selection and is called by it, browsing |
| `ord-outside` | I: every caller from off the journey of the cursor's call lit where it sits, none moving as the cursor steps; X in a journey moving none of its dots; a click on a plain function expanded opening no bar; a deep journey in a small window, the panel's rows track; a caller no line reaches not drawn but counted |
| `ord-branch` | one branch at a time: ↓ on a folded call opening it alone and stepping in, `.` every branch deeper, `,` every branch a level back, a badge stepping in, the depth reading the deepest shown, graph and panel agreeing, "can fail" alike at every depth; the start's bar under its callers in a small window; bars past the panel's side counted there |
| `ord-panel` | the panel on a crowded journey: bars keep their numbers and some of their names, five levels fitting across 1200 wide, one cursor bar, `,` takes a cursor folded away up to the call still drawn; a smaller window; Esc selects the start; `↺ calls itself` (Finder::walk); a wide journey that scrolls (Index::walk); a name two calls share cutting its type, never down to it, at 1000 wide |

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

- Black PNGs: macOS stops drawing a window when the display dims, sleeps or locks, or
  when it counts the window covered or wholly off screen. The runner keeps the display
  awake while a window is open (`caffeinate -d -i -u -w`, which also wakes a dimmed
  one), and the window keeps a corner on screen and on top. A snap that still comes out
  black is taken again with the window tucked into another screen corner, up to three
  times; still black, it has no PNG and is listed in `checks.json`'s
  `window_not_drawn`. That is not a failure of the view: its JSON and checks hold, the
  verdict reads `NOT DRAWN` (exit 2) when nothing else failed, and running again,
  display awake and unlocked, gives the pictures.
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
