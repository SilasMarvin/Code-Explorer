//! Call order on a second floor. The map is the ground floor and never moves; `Z`
//! lifts a glass deck over the map's journey region, towards the eye, and lays the
//! disclosed journey out on it as its own clean diagram: time runs left to right, one
//! column per call, and call depth climbs in rows, as in a flame graph standing on its
//! root. Each call is a tile in its node's color reaching across the calls it makes,
//! with a ball at its start, and hangs a thin string down to its node on the map, so
//! the two floors correlate: the cursor's string white, its neighbors' bright, the rest
//! faint. A function called three times hangs three strings from one node.
//!
//! The deck stays readable in a long journey by showing a window of calls round the
//! cursor, with how many more lie either side.

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

use bevy::{picking::Pickable, prelude::*};
use bevy_egui::egui;
use codeviz_core::SpanId;

use crate::{
    camera::OrbitCamera,
    cues::{self, Eye},
    explorer::Explorer,
    graph_view::{node_color, node_size},
    journey_panel::{self, JourneyPanel, PanelMode},
    ui::{Chrome, egui_color, node_name},
};

/// How call order shows on the graph.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TimeMode {
    #[default]
    Off,
    /// The journey as a diagram on a second floor over the map.
    Floors,
}

impl TimeMode {
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Off => Self::Floors,
            Self::Floors => Self::Off,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Floors => "two floors",
        }
    }
}

/// Where the deck stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Stand {
    /// Flat over the journey's part of the map, towards the eye.
    #[default]
    Over,
    /// Rising beyond the journey's far edge, so its strings fall forward onto the map.
    Behind,
}

/// How strings run from the deck to the map.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Route {
    /// Straight from each ball, bending down to its node.
    #[default]
    Direct,
    /// Down the deck to its near edge, across a narrow band to a slot in the order
    /// their nodes lie left to right, then fanning out to them, so the crossings
    /// gather in the band instead of over the map.
    Comb,
}

/// Which calls hang strings down to their nodes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Strings {
    /// The cursor's call and the calls it makes; the rest are matched by number.
    #[default]
    Tethered,
    /// Every call, the rest faint.
    All,
}

/// The upper floor as it stands this frame.
#[derive(Resource, Debug, Default)]
pub(crate) struct Floors {
    /// Where the deck stands.
    pub(crate) stand: Stand,
    /// How strings run down to the map.
    pub(crate) route: Route,
    /// Which calls hang strings.
    pub(crate) strings: Strings,
    /// The call whose tile is under the pointer, as last painted.
    pub(crate) pointed: Option<usize>,
    /// The node lit by the pointer: a pointed tile's, else the node under it.
    pub(crate) hover: Option<usize>,
    /// How far it has risen: 0 down on the map, 1 at full height.
    lift: f32,
    /// The camera's angles before the deck turned it, to turn back to.
    before: Option<(f32, f32)>,
    /// Whether it was up last frame.
    up: bool,
    /// The deck, eased as the journey changes.
    pub(crate) deck: Deck,
    /// The calls in the order they run.
    pub(crate) calls: Vec<Call>,
    /// The cursor's call, by index into `calls`.
    pub(crate) cursor: Option<usize>,
    /// The calls the deck shows.
    pub(crate) window: Range<usize>,
    /// How wide the deck was when the camera last stepped back to fit it.
    fitted: f32,
    /// Where the journey's nodes lie on the map, corner to corner.
    region: (Vec2, Vec2),
}

/// The glass deck: where its near edge's middle is over the map, how big it is, how
/// high its near edge floats, and how far it leans up from flat.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Deck {
    pub(crate) near: Vec2,
    pub(crate) size: Vec2,
    pub(crate) height: f32,
    pub(crate) lean: f32,
}

impl Deck {
    /// Where a point `(across, up)` of the deck lies in the world, each 0 to 1 from its
    /// left and near edge, at `lift` of its height.
    pub(crate) fn at(&self, across: f32, up: f32, lift: f32) -> Vec3 {
        let rise = self.rise() * self.size.y * up;
        Vec3::new(
            self.near.x + self.size.x * (across - 0.5),
            self.near.y + rise.y,
            (self.height + rise.z) * lift,
        )
    }

    /// The way up the deck, from its near edge to its far one.
    pub(crate) fn rise(&self) -> Vec3 {
        Vec3::new(0.0, self.lean.cos(), self.lean.sin())
    }

    fn eased(self, goal: Self, ease: f32) -> Self {
        Self {
            near: self.near.lerp(goal.near, ease),
            size: self.size.lerp(goal.size, ease),
            height: self.height + (goal.height - self.height) * ease,
            lean: self.lean + (goal.lean - self.lean) * ease,
        }
    }
}

/// One call on the deck.
#[derive(Debug, Clone)]
pub(crate) struct Call {
    pub(crate) span: SpanId,
    pub(crate) tile: usize,
    pub(crate) number: String,
    /// How many calls under the journey's start it runs.
    pub(crate) depth: usize,
    /// The last call that runs under it, itself if none.
    pub(crate) end: usize,
    /// Where its ball sits on the deck, at the start of its tile.
    pub(crate) marker: Vec3,
    /// Where its string meets its node.
    pub(crate) foot: Vec3,
    /// Where its string ends: its node, or the node's name pulled up over a crowd.
    pub(crate) lands: Vec3,
    /// Its string, from its ball to where it lands.
    pub(crate) string: Vec<Vec3>,
}

impl Floors {
    /// A deck that stands and strings that run as asked.
    pub(crate) fn styled(stand: Stand, route: Route, strings: Strings) -> Self {
        Self {
            stand,
            route,
            strings,
            ..default()
        }
    }

    /// How far above the map the camera looks while the deck is up: part way up, so
    /// both floors share the view.
    pub(crate) fn middle(&self) -> f32 {
        self.view().0.z
    }

    /// What the camera looks at to see both floors, and how far either side of it
    /// they reach.
    pub(crate) fn view(&self) -> (Vec3, f32) {
        let lift = self.lift();
        let (min, max) = self.region;
        let deck = self.deck;
        let corners = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)];
        let points: Vec<Vec3> = corners
            .iter()
            .map(|&(x, y)| deck.at(x, y, lift))
            .chain(
                corners
                    .iter()
                    .map(|&(x, y)| (min + (max - min) * Vec2::new(x, y)).extend(0.0)),
            )
            .collect();
        fit(&points)
    }

    /// Where the cursor's ball sits, while the deck shows.
    pub(crate) fn cursor_marker(&self) -> Option<Vec3> {
        let call = self.calls.get(self.cursor?)?;
        self.showing().then_some(call.marker)
    }

    /// Whether call `index` is another call to the cursor's function.
    pub(crate) fn again(&self, index: usize) -> bool {
        let tile = |i: usize| self.calls.get(i).map(|c| c.tile);
        self.cursor
            .is_some_and(|c| c != index && tile(c).is_some() && tile(c) == tile(index))
    }

    /// How many calls from the cursor's call `index` is, the cursor's function called
    /// again counting as the cursor.
    pub(crate) fn away(&self, index: usize) -> Option<usize> {
        let cursor = self.cursor?;
        Some(if self.again(index) {
            0
        } else {
            cursor.abs_diff(index)
        })
    }

    /// How call `index`'s string is drawn: the cursor's call, the calls it makes and
    /// other calls to its function are tethered to their nodes; the rest are left to
    /// their number badges.
    pub(crate) fn tether(&self, index: usize) -> Tether {
        let cursor = self
            .cursor
            .map(|c| (c, self.calls.get(c).map_or(c, |call| call.end)));
        // Other calls to the cursor's function are tethered too, as hovering its node
        // tethers them, so each meets the cursor's string at the node.
        // And the calls beside it, made by the same caller, faintly.
        match tether_of(cursor, index) {
            Tether::Other if self.again(index) => Tether::Made,
            Tether::Other
                if self
                    .cursor
                    .is_some_and(|c| sibling(|i| self.calls.get(i).map(|c| c.depth), c, index)) =>
            {
                Tether::Beside
            }
            tether => tether,
        }
    }

    /// The nodes the bright strings land on: the cursor's call's and those of the
    /// calls it makes.
    pub(crate) fn lit_tiles(&self) -> HashSet<usize> {
        (0..self.calls.len())
            .filter(|&i| self.shows(i) && matches!(self.tether(i), Tether::Cursor | Tether::Made))
            .map(|i| self.calls[i].tile)
            .chain(self.hover)
            .collect()
    }

    /// Whether call `index` runs the node the pointer lights.
    pub(crate) fn hovered(&self, index: usize) -> bool {
        self.hover
            .is_some_and(|tile| self.calls.get(index).is_some_and(|c| c.tile == tile))
    }

    /// Nodes whose names are pulled up off a crowd, to where their strings land.
    pub(crate) fn tags(&self) -> HashMap<usize, Vec3> {
        self.calls
            .iter()
            .filter(|c| c.lands != c.foot)
            .map(|c| (c.tile, c.lands))
            .collect()
    }

    /// The nodes of the calls the deck shows, which keep their numbers on the map.
    pub(crate) fn shown_tiles(&self) -> HashSet<usize> {
        if !self.showing() {
            return HashSet::new();
        }
        self.calls[self.window.clone()]
            .iter()
            .map(|c| c.tile)
            .collect()
    }

    /// Whether the deck shows call `index`.
    pub(crate) fn shows(&self, index: usize) -> bool {
        self.window.contains(&index)
    }

    /// How far it has risen: 0 down on the map, 1 at full height.
    pub(crate) fn lift(&self) -> f32 {
        self.lift.clamp(0.0, 1.0)
    }

    /// How strongly the map's links draw under it: quieter while it is up, so the
    /// strings read over them.
    pub(crate) fn map_strength(&self) -> f32 {
        1.0 - 0.45 * self.lift()
    }

    /// Whether anything of it shows.
    pub(crate) fn showing(&self) -> bool {
        self.lift > 0.01 && !self.calls.is_empty()
    }
}

/// The graph's area and the journey panel's in `area`: while the deck is up a docked
/// panel steps aside, since the deck shows the same journey over the map.
pub(crate) fn split(
    panel: &JourneyPanel,
    floors: &Floors,
    area: egui::Rect,
    explorer: &Explorer,
) -> (egui::Rect, Option<egui::Rect>) {
    let mode = if panel.mode == PanelMode::Docked && floors.showing() {
        PanelMode::Hidden
    } else {
        panel.mode
    };
    journey_panel::split(mode, area, journey_panel::extent(explorer))
}

pub(crate) struct FloorsPlugin;

impl Plugin for FloorsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Floors>()
            .init_gizmo_group::<StringGizmos>()
            .init_gizmo_group::<CursorGizmos>()
            .add_systems(Startup, (line_widths, spawn_glass))
            .add_systems(Update, place_glass.after(raise));
    }
}

/// Strings and the deck's ruling, drawn thin.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct StringGizmos;

/// The cursor's string, drawn bolder.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct CursorGizmos;

fn line_widths(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<StringGizmos>().0.line.width = 1.9;
    store.config_mut::<CursorGizmos>().0.line.width = 2.4;
}

/// The view while the deck is up: square on, tilted gently from in front, so time
/// runs left to right across the screen and both floors show.
pub(crate) const FLOORS_VIEW: (f32, f32) = (0.0, 0.55);

// ---- layout ----

/// The journey as it runs: `root`, then each call under it in order, depth first,
/// each with how deep under `root` it runs. `children` lists the shown calls a call
/// makes.
pub(crate) fn in_order<S: Copy>(root: S, children: impl Fn(S) -> Vec<S>) -> Vec<(S, usize)> {
    let mut order = Vec::new();
    let mut stack = vec![(root, 0)];
    while let Some((call, depth)) = stack.pop() {
        order.push((call, depth));
        stack.extend(children(call).into_iter().rev().map(|c| (c, depth + 1)));
    }
    order
}

/// For calls in depth-first order at `depths`, the last call each one's runs under it:
/// the calls after it that run deeper, up to the first that doesn't.
pub(crate) fn ends(depths: &[usize]) -> Vec<usize> {
    (0..depths.len())
        .map(|i| {
            let deeper = depths[i + 1..]
                .iter()
                .take_while(|&&d| d > depths[i])
                .count();
            i + deeper
        })
        .collect()
}

/// The most calls the deck shows at once; a longer journey shows a window round the
/// cursor.
pub(crate) const WINDOW: usize = 15;

/// Which of `count` calls the deck shows, at most `most` of them: all that fit, else a
/// window centered on the cursor as far as the ends allow.
pub(crate) fn window(count: usize, cursor: Option<usize>, most: usize) -> Range<usize> {
    if count <= most {
        return 0..count;
    }
    let cursor = cursor.unwrap_or(0).min(count - 1);
    let start = cursor.saturating_sub(most / 2).min(count - most);
    start..start + most
}

/// How wide a column is at least, in world units, so a name has room.
const COLUMN: f32 = 6.0;
/// How wide the deck is at least, in the journey region's widest extent.
const WIDE: f32 = 0.95;
/// How wide the deck is at least, as a share of the camera's distance: about half
/// the view across, and less than the camera backs off for it, so the two settle.
const ACROSS: f32 = 0.7;
/// How deep a row is, as a share of a column's width.
const ROW: f32 = 0.8;
/// How deep the deck is at most, as a share of its width.
const SHALLOW: f32 = 0.3;
/// How high the deck floats over the map, as a share of its width, and at least.
const HEIGHT: f32 = 0.45;
const LOWEST: f32 = 14.0;
const CLEAR: f32 = 0.9;
const HIGHEST: f32 = 1.0;

/// How far a standing deck leans up from flat, in radians: nearly upright, tipped
/// back a little to face a camera looking down on the map.
const LEAN: f32 = 1.2;
/// How far beyond the journey's far edge a standing deck stands, and how high its
/// near edge floats, as shares of its width.
const BEYOND: f32 = 0.08;
const FOOT: f32 = 0.12;

/// The deck for `columns` calls `rows` deep over a journey whose nodes span `min` to
/// `max` on the map: as wide as they are or wide enough for its columns, and either
/// centered over them high enough to clear them, or standing beyond their far edge.
pub(crate) fn plan_deck(
    (min, max): (Vec2, Vec2),
    columns: usize,
    rows: usize,
    stand: Stand,
    least: f32,
) -> Deck {
    let span = max - min;
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let (columns, rows) = (columns.max(1) as f32, rows.max(1) as f32);
    let width = (span.max_element() * WIDE).max(columns * COLUMN).max(least);
    let column = width / columns;
    let depth = (rows * column * ROW).min(width * SHALLOW);
    let middle = (min + max) / 2.0;
    match stand {
        Stand::Over => Deck {
            near: Vec2::new(middle.x, middle.y - depth / 2.0),
            size: Vec2::new(width, depth),
            // High enough that, seen from in front at the view's tilt, the deck mostly
            // clears the journey's region of the map under it.
            height: ((span.y + depth) * CLEAR)
                .clamp(width * HEIGHT, width * HIGHEST)
                .max(LOWEST),
            lean: 0.0,
        },
        Stand::Behind => Deck {
            near: Vec2::new(middle.x, max.y + width * BEYOND),
            size: Vec2::new(width, depth),
            height: width * FOOT,
            lean: LEAN,
        },
    }
}

/// Where call `index` of the window `shown` sits on the deck, `rows` deep: across at
/// its column's left edge, up at its row's middle, both 0 to 1.
pub(crate) fn deck_spot(index: usize, depth: usize, shown: &Range<usize>, rows: usize) -> Vec2 {
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let (column, columns, row, rows) = (
        index.saturating_sub(shown.start) as f32,
        shown.len().max(1) as f32,
        depth as f32,
        rows.max(1) as f32,
    );
    Vec2::new(column / columns, (row + 0.5) / rows)
}

/// How a call's string is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tether {
    /// The cursor's call: white and bold.
    Cursor,
    /// A call the cursor's call makes: bright in its node's color, one traceable line.
    Made,
    /// Another call made by the cursor's caller: faint, in its node's color.
    Beside,
    /// Any other: not drawn, or faint if asked for.
    Other,
}

/// Whether call `index` is made by the same call as call `cursor`, for calls in
/// depth-first order each `depth` deep.
pub(crate) fn sibling(depth: impl Fn(usize) -> Option<usize>, cursor: usize, index: usize) -> bool {
    let (Some(at), Some(other)) = (depth(cursor), depth(index)) else {
        return false;
    };
    if index == cursor || other != at || at == 0 {
        return false;
    }
    // Nothing shallower between them: both run under the same caller.
    (cursor.min(index)..=cursor.max(index)).all(|i| depth(i).is_some_and(|d| d >= at))
}

/// How call `index`'s string is drawn, for a cursor on the call that runs from its
/// index to the end of the calls it makes: the cursor's call, and the calls it makes,
/// The first `TETHERS` of them, if it makes more.
pub(crate) fn tether_of(cursor: Option<(usize, usize)>, index: usize) -> Tether {
    match cursor {
        Some((start, _)) if index == start => Tether::Cursor,
        Some((start, end)) if index > start && index <= end.min(start + TETHERS) => Tether::Made,
        _ => Tether::Other,
    }
}

/// The most calls under the cursor's that hang strings; the rest go by their badges.
pub(crate) const TETHERS: usize = 12;

/// How strongly a string draws at the deck, and how much of that it loses by the
/// time it reaches the map; none for the others unless they are asked for.
pub(crate) fn string_strength(tether: Tether, others: bool) -> Option<(f32, f32)> {
    match tether {
        Tether::Cursor => Some((1.0, 0.25)),
        Tether::Made => Some((0.9, 0.15)),
        Tether::Beside => Some((0.45, 0.5)),
        Tether::Other => others.then_some((0.22, 0.8)),
    }
}

/// Points along a string from `top` on the deck to `foot` on the map: it leaves the
/// deck along `out` and arrives at its node straight down, bending between, so both
/// ends read as hanging.
pub(crate) fn string_points(top: Vec3, out: Vec3, foot: Vec3, pieces: usize) -> Vec<Vec3> {
    let gap = top - foot;
    let drop = gap.z.max(gap.truncate().length() * 0.5).max(0.0) * 0.45;
    let (a, b) = (top + out * drop, foot + Vec3::Z * drop);
    (0..=pieces)
        .map(|i| {
            #[expect(clippy::cast_precision_loss, reason = "a few pieces")]
            let t = i as f32 / pieces.max(1) as f32;
            let u = 1.0 - t;
            top * u * u * u + a * 3.0 * u * u * t + b * 3.0 * u * t * t + foot * t * t * t
        })
        .collect()
}

/// For string feet across the screen at `xs`, each one's slot in the comb: its rank
/// left to right, ties kept in time order.
pub(crate) fn slots(xs: &[f32]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..xs.len()).collect();
    order.sort_by(|&a, &b| xs[a].total_cmp(&xs[b]).then(a.cmp(&b)));
    let mut slot = vec![0; xs.len()];
    for (rank, i) in order.into_iter().enumerate() {
        slot[i] = rank;
    }
    slot
}

/// Whether `at` sits in a crowd: at least `many` of `others` lie within `radius`.
pub(crate) fn crowded(at: Vec3, others: &[Vec3], radius: f32, many: usize) -> bool {
    others
        .iter()
        .filter(|o| o.truncate().distance(at.truncate()) < radius)
        .count()
        >= many
}

/// How far below the deck's near edge the comb's slots lie, as a share of its depth.
const COMB: f32 = 0.45;
/// How close other strings' nodes crowd the cursor's, as a share of the deck's width,
/// and how many make a crowd; how high its name is then pulled up.
const CROWD_RADIUS: f32 = 0.08;
const CROWD: usize = 3;
const TAG: f32 = 0.15;

/// The disclosed journey's calls in order, each with its node, number, and depth;
/// calls whose node the graph lacks are left out.
fn journey_calls(explorer: &Explorer) -> Vec<(SpanId, usize, String, usize)> {
    let Some(journey) = explorer.journey() else {
        return Vec::new();
    };
    let disclosed = explorer.disclosure();
    let map = explorer.map();
    let shown = |span: SpanId| -> Vec<SpanId> {
        disclosed
            .children(span)
            .iter()
            .copied()
            .filter(|&c| disclosed.number(c).is_some())
            .collect()
    };
    in_order(journey.root(), shown)
        .into_iter()
        .filter_map(|(span, depth)| {
            let tile = explorer.unit_id(span).and_then(|u| map.tile_of(u))?;
            let number = disclosed.number(span).unwrap_or_default().to_owned();
            Some((span, tile, number, depth))
        })
        .collect()
}

/// Raises and lowers the deck as `Z` asks, turns the camera to read it and back, and
/// lays the calls out on it.
pub(crate) fn raise(
    time: Res<Time>,
    chrome: Res<Chrome>,
    explorer: Res<Explorer>,
    mut floors: ResMut<Floors>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let up = chrome.time == TimeMode::Floors;
    let turned = up != floors.up;
    floors.up = up;
    if turned && !up {
        for mut camera in &mut cameras {
            if let Some(before) = floors.before.take() {
                camera.turn_to(before);
            }
        }
    }
    let ease = 1.0 - (-time.delta_secs() * 4.0).exp();
    let goal = if up { 1.0 } else { 0.0 };
    floors.lift += (goal - floors.lift) * ease;
    if !up && floors.lift < 0.005 {
        floors.lift = 0.0;
        floors.calls.clear();
        floors.cursor = None;
        return;
    }
    let calls = journey_calls(&explorer);
    let index: HashMap<SpanId, usize> = calls
        .iter()
        .enumerate()
        .map(|(i, &(span, ..))| (span, i))
        .collect();
    // The cursor's call, or the nearest one shown that it runs under.
    floors.cursor = explorer.selected().and_then(|span| {
        explorer
            .stack(span)
            .into_iter()
            .rev()
            .find_map(|s| index.get(&s).copied())
    });
    let shown = window(calls.len(), floors.cursor, WINDOW);
    let depths: Vec<usize> = calls.iter().map(|c| c.3).collect();
    let rows = depths[shown.clone()].iter().max().map_or(1, |d| d + 1);
    // Over the shown calls' nodes, so a long journey's deck follows its window.
    let points: Vec<Vec2> = calls[shown.clone()]
        .iter()
        .filter_map(|c| explorer.tile_point(c.1))
        .map(Vec3::truncate)
        .collect();
    let (min, max) = points.iter().fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
        |(min, max), &p| (min.min(p), max.max(p)),
    );
    if points.is_empty() {
        floors.calls.clear();
        return;
    }
    // At least a share of the view across, however small the window's part of the
    // map, so its names have room at any distance.
    let least = cameras.iter().next().map_or(0.0, |c| c.distance * ACROSS);
    let plan = plan_deck((min, max), shown.len(), rows, floors.stand, least);
    let fresh = floors.deck.size == Vec2::ZERO;
    floors.deck = if fresh {
        plan
    } else {
        floors.deck.eased(plan, ease)
    };
    floors.region = (min, max);
    // Back far enough to see the deck and the map under it, when it rises and again
    // when it grows, unless the view was moved by hand.
    let reach = {
        let (deck, lift) = (floors.deck, floors.lift);
        floors.deck = plan;
        floors.lift = 1.0;
        let (_, reach) = floors.view();
        (floors.deck, floors.lift) = (deck, lift);
        reach * FIT
    };
    // And when it shrinks by much, so its columns keep their width on screen.
    let grown = plan.size.x > floors.fitted * 1.1 || plan.size.x < floors.fitted * 0.8;
    if up && (turned || grown) {
        for mut camera in &mut cameras {
            if turned {
                floors.before = Some(camera.angles());
                camera.turn_to(FLOORS_VIEW);
            } else if camera.free {
                continue;
            }
            let target = camera.target;
            camera.glide_to(target, reach);
        }
    }
    if up {
        floors.fitted = plan.size.x;
    }
    floors.calls = place_calls(&explorer, &floors, calls, &shown, rows);
    floors.window = shown;
    floors.hover = floors
        .pointed
        .and_then(|i| floors.calls.get(i))
        .map(|c| c.tile)
        .or(explorer.hovered_tile);
}

/// Places each call's ball on the deck, in the window `shown` of it `rows` deep, the
/// foot of its string on its node, and its string between them.
fn place_calls(
    explorer: &Explorer,
    floors: &Floors,
    calls: Vec<(SpanId, usize, String, usize)>,
    shown: &Range<usize>,
    rows: usize,
) -> Vec<Call> {
    let depths: Vec<usize> = calls.iter().map(|c| c.3).collect();
    let ends = ends(&depths);
    let (deck, lift) = (floors.deck, floors.lift());
    let graph = explorer.graph();
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let half = BALL_AT / shown.len().max(1) as f32;
    let mut placed: Vec<Call> = calls
        .into_iter()
        .enumerate()
        .filter_map(|(i, (span, tile, number, depth))| {
            let at = explorer.tile_point(tile)?;
            let spot = deck_spot(i.clamp(shown.start, shown.end), depth, shown, rows);
            let foot = at + Vec3::Z * node_size(graph, tile) * 0.5;
            Some(Call {
                span,
                tile,
                number,
                depth,
                end: ends[i],
                marker: deck.at(spot.x + half, spot.y, lift),
                foot,
                lands: foot,
                string: Vec::new(),
            })
        })
        .collect();
    // Over a crowd, the cursor's string lands on its name, pulled up off the crowd.
    let feet: Vec<Vec3> = placed
        .iter()
        .enumerate()
        .filter(|&(i, _)| shown.contains(&i) && floors.cursor != Some(i))
        .map(|(_, c)| c.foot)
        .collect();
    if let Some(call) = floors.cursor.and_then(|c| placed.get_mut(c))
        && crowded(call.foot, &feet, deck.size.x * CROWD_RADIUS, CROWD)
    {
        call.lands = call.foot + Vec3::Z * deck.size.x * TAG * lift;
    }
    // Leaving a flat deck straight down; a standing one, down its face.
    let out = match floors.stand {
        Stand::Over => Vec3::NEG_Z,
        Stand::Behind => -deck.rise(),
    };
    let in_window: Vec<usize> = (0..placed.len()).filter(|i| shown.contains(i)).collect();
    let slot = slots(
        &in_window
            .iter()
            .map(|&i| placed[i].lands.x)
            .collect::<Vec<_>>(),
    );
    for (k, &i) in in_window.iter().enumerate() {
        let call = &mut placed[i];
        call.string = match floors.route {
            Route::Direct => string_points(call.marker, out, call.lands, 16),
            Route::Comb => {
                #[expect(clippy::cast_precision_loss, reason = "a small count")]
                let (from, to) = (
                    (k as f32 + 0.5) / in_window.len() as f32,
                    (slot[k] as f32 + 0.5) / in_window.len() as f32,
                );
                let edge = deck.at(from, 0.0, lift);
                let s = |a: f32, u: f32| deck.at(a, u, lift);
                let band = (0..=6).map(|j| {
                    #[expect(clippy::cast_precision_loss, reason = "a few pieces")]
                    let t = j as f32 / 6.0;
                    let ease = t * t * (3.0 - 2.0 * t);
                    s(from + (to - from) * ease, -COMB * t)
                });
                let slot = s(to, -COMB);
                // Out of the comb half towards its node, so a string bound back under
                // the deck doesn't loop out first.
                let toward = (call.lands - slot).normalize_or_zero();
                let out = (out + toward).normalize_or(out);
                std::iter::once(call.marker)
                    .chain(std::iter::once(edge))
                    .chain(band.skip(1))
                    .chain(string_points(slot, out, call.lands, 14).into_iter().skip(1))
                    .collect()
            }
        };
    }
    placed
}

/// How far back to stand, in reaches either side of the middle.
const FIT: f32 = 2.5;

/// The middle of `points` and how far the farthest lies from it.
pub(crate) fn fit(points: &[Vec3]) -> (Vec3, f32) {
    let (min, max) = points.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(min, max), &p| (min.min(p), max.max(p)),
    );
    if points.is_empty() {
        return (Vec3::ZERO, 0.0);
    }
    let middle = (min + max) / 2.0;
    let reach = points
        .iter()
        .map(|p| p.distance(middle))
        .fold(0.0, f32::max);
    (middle, reach)
}

/// While the deck is up, the camera looks between the floors, over the journey's
/// middle, rather than at the journey's start.
pub(crate) fn lift_view(
    explorer: Res<Explorer>,
    floors: Res<Floors>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    if !floors.showing() || explorer.anchor().is_none() {
        return;
    }
    let lift = floors.lift();
    let (over, _) = floors.view();
    for mut camera in &mut cameras {
        if !camera.free {
            camera.target = camera.target.lerp(over, lift);
        }
    }
}

// ---- the glass ----

#[derive(Component)]
struct Glass;

/// The deck's pane: a faint sheet of glass, lit a little blue-grey, that the map shows
/// through.
const GLASS: Color = Color::srgba(0.62, 0.7, 0.9, 0.07);

fn spawn_glass(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Glass,
        Mesh3d(meshes.add(Plane3d::new(Vec3::Z, Vec2::splat(0.5)))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: GLASS,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            fog_enabled: false,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
        Pickable::IGNORE,
    ));
}

/// Fits the glass to the deck and fades it with the lift.
fn place_glass(
    floors: Res<Floors>,
    mut glass: Query<
        (
            &mut Transform,
            &mut Visibility,
            &MeshMaterial3d<StandardMaterial>,
        ),
        With<Glass>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok((mut transform, mut visibility, material)) = glass.single_mut() else {
        return;
    };
    if !floors.showing() {
        *visibility = Visibility::Hidden;
        return;
    }
    let deck = floors.deck;
    // A margin round the diagram, so its tiles don't run to the glass's edge.
    let size = deck.size + Vec2::splat(MARGIN * 2.0);
    *visibility = Visibility::Visible;
    *transform = Transform::from_translation(deck.at(0.5, 0.5, floors.lift()))
        .with_rotation(Quat::from_rotation_x(deck.lean))
        .with_scale(size.extend(1.0));
    if let Some(mut material) = materials.get_mut(&material.0) {
        material.base_color = GLASS.with_alpha(GLASS.alpha() * floors.lift());
    }
}

/// The glass reaches this far past the diagram on each side.
const MARGIN: f32 = 2.0;

// ---- strings ----

/// Strings are pale, so they say nothing about direction to the focus or module.
const STRING: Color = Color::srgb(0.85, 0.88, 0.95);

/// Draws the deck's rim and each shown call's string down to its node, and a ring on
/// the map round the cursor's node.
pub(crate) fn draw_strings(
    floors: Res<Floors>,
    explorer: Res<Explorer>,
    mut gizmos: Gizmos<StringGizmos>,
    mut bold: Gizmos<CursorGizmos>,
) {
    if !floors.showing() {
        return;
    }
    let fade = floors.lift();
    let deck = floors.deck;
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(x, y): (f32, f32)| {
        let pad = Vec2::splat(MARGIN) / deck.size.max(Vec2::splat(1e-3));
        deck.at(
            x * (1.0 + 2.0 * pad.x) - pad.x,
            y * (1.0 + 2.0 * pad.y) - pad.y,
            fade,
        )
    });
    for i in 0..4 {
        gizmos.line(
            corners[i],
            corners[(i + 1) % 4],
            Color::srgba(0.75, 0.82, 1.0, 0.22 * fade),
        );
    }
    for (index, call) in floors.calls.iter().enumerate() {
        if !floors.shows(index) {
            continue;
        }
        let tether = floors.tether(index);
        let hovered = floors.hovered(index);
        // What the pointer lights is tethered whatever it is.
        let shown_as = if hovered && tether == Tether::Other {
            Tether::Made
        } else {
            tether
        };
        let Some((alpha, thin)) = string_strength(shown_as, floors.strings == Strings::All) else {
            continue;
        };
        let alpha = alpha * fade;
        let tint = match shown_as {
            Tether::Cursor => Color::WHITE,
            Tether::Made => node_color(&explorer, None, call.tile).mix(&Color::WHITE, 0.35),
            Tether::Beside | Tether::Other => {
                node_color(&explorer, None, call.tile).mix(&STRING, 0.55)
            }
        };
        let points = &call.string;
        let pieces = points.len().saturating_sub(1);
        let strong = tether == Tether::Cursor || hovered;
        for (i, pair) in points.windows(2).enumerate() {
            // Brightest at the deck, thinning a little towards the map, so each
            // tethered string stays traceable to its node.
            #[expect(clippy::cast_precision_loss, reason = "a few pieces")]
            let t = i as f32 / pieces.max(1) as f32;
            let color = tint.with_alpha(alpha * (1.0 - thin * t));
            if strong {
                bold.line(pair[0], pair[1], color);
            } else {
                gizmos.line(pair[0], pair[1], color);
            }
        }
        // A name pulled up off a crowd hangs a short thread down to its node.
        if call.lands != call.foot {
            gizmos.line(
                call.lands,
                call.foot,
                Color::srgba(1.0, 1.0, 1.0, 0.5 * fade),
            );
        }
    }
    if let Some(call) = floors.cursor.and_then(|c| floors.calls.get(c)) {
        let ring = Isometry3d::from_translation(call.foot);
        bold.circle(ring, 1.6, Color::srgba(1.0, 1.0, 1.0, 0.85 * fade));
    }
}

// ---- painting the deck ----

/// How many pixels across a call's ball is, the cursor's bigger.
pub(crate) fn marker_radius(nearness: f32, cursor: bool) -> f32 {
    let base = if cursor { 7.5 } else { 5.0 };
    (base * nearness).clamp(base * 0.8, base * 1.8)
}

/// Paints the deck's diagram: each call's tile in its node's color across the calls it
/// makes, its ball at the tile's start, its name slanting up from it, and how many
/// calls lie past the window either side.
pub(crate) fn draw_deck(
    painter: &egui::Painter,
    (camera, transform): (&Camera, &GlobalTransform),
    explorer: &Explorer,
    floors: &Floors,
    orbit: &OrbitCamera,
) {
    if !floors.showing() {
        return;
    }
    let eye = Eye::of(transform, orbit.distance);
    let project = |position: Vec3| {
        camera
            .world_to_viewport(transform, position)
            .ok()
            .map(|p| egui::pos2(p.x, p.y))
    };
    let project = &project;
    let fade = floors.lift();
    let backdrop = egui_color(cues::BACKDROP);
    let shown = floors.window.clone();
    let rows = deck_rows(floors);
    // Tiles first, shallow rows first.
    for (index, call) in floors.calls.iter().enumerate() {
        let Some([Some(a), Some(b), Some(c), Some(d)]) =
            tile_corners(floors, index, rows).map(|q| q.map(project))
        else {
            continue;
        };
        let away = floors.away(index);
        let near = match away {
            Some(0) => 1.0,
            Some(1..=2) => 0.75,
            _ => 0.5,
        };
        let haze = eye.haze(call.marker);
        let fill = egui_color(node_color(explorer, None, call.tile));
        let fill = cues::mix(fill, backdrop, 1.0 - near * 0.7 * (1.0 - haze));
        let mut mesh = egui::Mesh::default();
        for p in [a, b, c, d] {
            mesh.colored_vertex(p, fill.gamma_multiply(fade));
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
        if floors.cursor == Some(index) || floors.again(index) || floors.hovered(index) {
            let width = if floors.cursor == Some(index) {
                1.8
            } else {
                1.6
            };
            painter.add(egui::Shape::closed_line(
                vec![a, b, c, d],
                egui::Stroke::new(width, egui::Color32::WHITE.gamma_multiply(fade)),
            ));
        }
    }
    // Balls, then names over everything.
    for index in shown.clone() {
        let call = &floors.calls[index];
        let Some(at) = project(call.marker) else {
            continue;
        };
        let cursor = floors.cursor == Some(index);
        let radius = marker_radius(eye.nearness(call.marker), cursor);
        let fill = if cursor {
            egui::Color32::WHITE
        } else {
            egui_color(node_color(explorer, None, call.tile))
        };
        cues::sphere(painter, at, radius, fill, egui::vec2(-0.6, -0.8), fade);
    }
    // The calls the window opens inside of are named off its left edge, level with
    // their tiles, so every tile on the deck reads by name.
    for index in (0..shown.start).filter(|&i| floors.calls[i].end >= shown.start) {
        let call = &floors.calls[index];
        let row = deck_spot(shown.start, call.depth, &shown, rows).y;
        if let Some(at) = project(floors.deck.at(0.0, row, fade)) {
            let name = node_name(explorer, call.tile);
            painter.text(
                at - egui::vec2(12.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                format!("{} {name} ▸", call.number).trim_start(),
                egui::FontId::proportional(12.5),
                egui::Color32::from_rgb(204, 210, 222).gamma_multiply(fade),
            );
        }
    }
    // The cursor's last, its name in full over its neighbors'.
    let cursor_last = shown
        .clone()
        .filter(|&i| floors.cursor != Some(i))
        .chain(floors.cursor.filter(|&c| floors.shows(c)));
    for index in cursor_last {
        let call = &floors.calls[index];
        let radius = marker_radius(eye.nearness(call.marker), floors.cursor == Some(index));
        name_call(painter, project, explorer, floors, index, (radius, fade));
    }
    count_beyond(painter, project, floors, fade);
}

/// The rows the deck's window runs deep.
fn deck_rows(floors: &Floors) -> usize {
    floors.calls[floors.window.clone()]
        .iter()
        .map(|c| c.depth + 1)
        .max()
        .unwrap_or(1)
}

/// The corners of call `index`'s tile on a deck `rows` deep, from its start across
/// the calls it makes; none if the deck doesn't show it.
fn tile_corners(floors: &Floors, index: usize, rows: usize) -> Option<[Vec3; 4]> {
    let call = floors.calls.get(index)?;
    let shown = &floors.window;
    let on_deck = floors.shows(index) || (index < shown.start && call.end >= shown.start);
    if !on_deck {
        return None;
    }
    let first = index.max(shown.start);
    let last = call.end.min(shown.end.saturating_sub(1));
    let left = deck_spot(first, call.depth, shown, rows);
    let right = deck_spot(last + 1, call.depth, shown, rows);
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let (half, inset) = (0.5 / rows as f32 * 0.5, 0.08 / shown.len().max(1) as f32);
    Some(
        [
            (left.x + inset, left.y - half),
            (right.x - inset, left.y - half),
            (right.x - inset, left.y + half),
            (left.x + inset, left.y + half),
        ]
        .map(|(x, y)| floors.deck.at(x, y, floors.lift())),
    )
}

/// Whether `point` lies inside the convex `quad`, its corners in order either way
/// round.
pub(crate) fn inside(quad: [egui::Pos2; 4], point: egui::Pos2) -> bool {
    let sides: Vec<f32> = (0..4)
        .map(|i| {
            let (a, b) = (quad[i], quad[(i + 1) % 4]);
            (b - a).x * (point - a).y - (b - a).y * (point - a).x
        })
        .collect();
    sides.iter().all(|&s| s >= 0.0) || sides.iter().all(|&s| s <= 0.0)
}

/// The call whose tile or ball is under `pointer`, deepest first, as the deck draws.
pub(crate) fn pointed_call(
    floors: &Floors,
    project: impl Fn(Vec3) -> Option<egui::Pos2>,
    pointer: egui::Pos2,
) -> Option<usize> {
    if !floors.showing() {
        return None;
    }
    let rows = deck_rows(floors);
    let ball = |index: usize| {
        floors
            .calls
            .get(index)
            .filter(|_| floors.shows(index))
            .and_then(|c| project(c.marker))
            .is_some_and(|at| at.distance(pointer) < 9.0)
    };
    (0..floors.calls.len()).rev().find(|&index| {
        ball(index)
            || tile_corners(floors, index, rows)
                .map(|q| q.map(&project))
                .and_then(|[a, b, c, d]| Some([a?, b?, c?, d?]))
                .is_some_and(|quad| inside(quad, pointer))
    })
}

/// Names call `index` on a slant up and right from its ball: the cursor's white and
/// larger, its neighbors light, the rest grey, dimmer further from the cursor.
fn name_call(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    floors: &Floors,
    index: usize,
    (radius, fade): (f32, f32),
) {
    let call = &floors.calls[index];
    let Some(at) = project(call.marker) else {
        return;
    };
    let away = floors.away(index);
    let (size, color) = match away {
        Some(0) if floors.cursor == Some(index) => (15.0, egui::Color32::WHITE),
        _ if floors.hovered(index) => (13.5, egui::Color32::WHITE),
        Some(0) => (13.0, egui::Color32::from_rgb(235, 238, 246)),
        Some(1..=2) => (13.0, egui::Color32::from_rgb(222, 226, 236)),
        // Every call reads by name, the far ones only a little dimmer.
        None | Some(_) => (12.5, egui::Color32::from_rgb(204, 210, 222)),
    };
    let name = node_name(explorer, call.tile);
    let text = if call.number.is_empty() {
        name
    } else {
        format!("{} {name}", call.number)
    };
    // Level along its tile, the name cut short with an ellipsis where the tile ends
    // and the number always whole; the cursor's in full on a dark pill over whatever
    // it runs past.
    let deck = floors.deck;
    let shown = &floors.window;
    let columns = call.end.min(shown.end.saturating_sub(1)) + 1 - index;
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let reach = deck.size.x * (columns as f32 - BALL_AT) / shown.len().max(1) as f32;
    let room = project(call.marker + Vec3::X * reach).map_or(0.0, |end| end.distance(at));
    let cursor = floors.cursor == Some(index);
    let font = egui::FontId::proportional(size);
    let mut galley = painter.layout_no_wrap(text.clone(), font.clone(), color);
    let least = call.number.chars().count() + 2;
    let mut keep = text.chars().count();
    while !cursor && keep > least && !level(galley.size().x, radius, room) {
        keep -= 1;
        galley = painter.layout_no_wrap(shortened(&text, keep), font.clone(), color);
    }
    let start = at + egui::vec2(radius + 3.0, 0.0);
    let corner = start - egui::vec2(0.0, galley.size().y * 0.5);
    if cursor {
        let pill = egui::Rect::from_min_size(corner, galley.size()).expand2(egui::vec2(4.0, 1.0));
        painter.rect_filled(
            pill,
            3.0,
            egui::Color32::from_rgb(18, 20, 26).gamma_multiply(fade),
        );
    }
    painter.galley_with_override_text_color(corner, galley, color.gamma_multiply(fade));
}

/// Where a call's ball sits across its first column, as a share of it: near the
/// tile's start, leaving the rest of the tile to its name.
const BALL_AT: f32 = 0.18;

/// `text` cut to its first `keep` characters and an ellipsis, or whole if that long.
pub(crate) fn shortened(text: &str, keep: usize) -> String {
    if text.chars().count() <= keep {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(keep.saturating_sub(1)).collect();
    short.push('…');
    short
}

/// Whether a name `width` wide fits level beside a ball of `radius`, with `room`
/// to its tile's end.
pub(crate) fn level(width: f32, radius: f32, room: f32) -> bool {
    radius + 3.0 + width + 4.0 <= room
}

/// Says how many calls lie past the window at either end of the deck.
fn count_beyond(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    floors: &Floors,
    fade: f32,
) {
    let deck = floors.deck;
    let (before, after) = (
        floors.window.start,
        floors.calls.len().saturating_sub(floors.window.end),
    );
    // Just off the deck's near edge, clear of the names of the calls it opens inside.
    let row = -0.12;
    let color = egui::Color32::from_rgb(170, 178, 196).gamma_multiply(fade);
    let font = egui::FontId::proportional(11.0);
    if before > 0
        && let Some(at) = project(deck.at(0.0, row, floors.lift()))
    {
        painter.text(
            at - egui::vec2(10.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            format!("◂ {before} earlier"),
            font.clone(),
            color,
        );
    }
    if after > 0
        && let Some(at) = project(deck.at(1.0, row, floors.lift()))
    {
        painter.text(
            at + egui::vec2(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{after} later ▸"),
            font,
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_journey_runs_depth_first_with_its_depth() {
        // 0 calls 1 then 4; 1 calls 2 then 3.
        let tree = |n: u32| match n {
            0 => vec![1, 4],
            1 => vec![2, 3],
            _ => vec![],
        };
        assert_eq!(
            in_order(0, tree),
            vec![(0, 0), (1, 1), (2, 2), (3, 2), (4, 1)]
        );
    }

    #[test]
    fn a_call_reaches_across_the_calls_it_makes() {
        assert_eq!(ends(&[0, 1, 2, 2, 1]), vec![4, 3, 2, 3, 4]);
        assert_eq!(ends(&[]), Vec::<usize>::new());
    }

    #[test]
    fn a_long_journey_shows_a_window_round_the_cursor() {
        assert_eq!(window(10, Some(3), 21), 0..10, "all that fit");
        assert_eq!(window(49, Some(30), 21), 20..41, "centered");
        assert_eq!(window(49, Some(2), 21), 0..21, "held at the start");
        assert_eq!(window(49, Some(48), 21), 28..49, "and at the end");
        assert_eq!(window(49, None, 21), 0..21);
        assert_eq!(window(0, None, 21), 0..0);
    }

    #[test]
    fn the_deck_is_wide_enough_for_its_columns_and_clears_the_map() {
        let small = plan_deck((Vec2::ZERO, Vec2::new(100.0, 40.0)), 4, 2, Stand::Over, 0.0);
        assert!(
            (small.size.x - 100.0 * WIDE).abs() < 1e-4,
            "as wide as the journey"
        );
        assert!(
            small
                .at(0.5, 0.5, 1.0)
                .truncate()
                .distance(Vec2::new(50.0, 20.0))
                < 1e-4
        );
        let crowd = plan_deck((Vec2::ZERO, Vec2::new(10.0, 10.0)), 21, 1, Stand::Over, 0.0);
        assert!(crowd.size.x >= 21.0 * COLUMN - 1e-4, "room for each column");
        assert!(crowd.height >= LOWEST);
        let deep = plan_deck((Vec2::ZERO, Vec2::new(10.0, 10.0)), 21, 3, Stand::Over, 0.0);
        assert!(deep.size.y > crowd.size.y, "deeper with more rows");
    }

    #[test]
    fn a_standing_deck_rises_beyond_the_journey() {
        let deck = plan_deck(
            (Vec2::ZERO, Vec2::new(100.0, 40.0)),
            4,
            2,
            Stand::Behind,
            0.0,
        );
        let (near, far) = (deck.at(0.5, 0.0, 1.0), deck.at(0.5, 1.0, 1.0));
        assert!(near.y > 40.0, "beyond the far edge");
        assert!(
            far.z > near.z * 2.0 && far.y > near.y,
            "rising, leaning back"
        );
        assert!(far.z - near.z > far.y - near.y, "nearly upright");
    }

    #[test]
    fn calls_sit_in_order_left_to_right_and_rise_by_depth() {
        let shown = 0..4;
        let first = deck_spot(0, 0, &shown, 2);
        let second = deck_spot(1, 1, &shown, 2);
        assert!(second.x > first.x, "later is further right");
        assert!(second.y > first.y, "deeper is further up");
        assert!(first.x.abs() < 1e-6 && deck_spot(4, 0, &shown, 2).x > 0.99);
        let later = 10..14;
        assert_eq!(deck_spot(11, 0, &later, 1).x, deck_spot(1, 0, &shown, 1).x);
    }

    #[test]
    fn the_comb_orders_strings_by_where_they_land() {
        assert_eq!(slots(&[5.0, 1.0, 3.0]), vec![2, 0, 1]);
        assert_eq!(slots(&[1.0, 1.0]), vec![0, 1], "ties keep time order");
    }

    #[test]
    fn calls_beside_the_cursor_share_its_caller() {
        // 0 makes 1 and 3; 1 makes 2; 3 makes 4.
        let depths = [0, 1, 2, 1, 2];
        let depth = |i: usize| depths.get(i).copied();
        assert!(sibling(depth, 1, 3));
        assert!(!sibling(depth, 2, 4), "different callers");
        assert!(!sibling(depth, 1, 1));
        assert!(!sibling(depth, 0, 0));
    }

    #[test]
    fn a_long_name_is_cut_short_with_an_ellipsis() {
        assert_eq!(shortened("5.10 stack", 20), "5.10 stack");
        assert_eq!(shortened("5.10 stack", 7), "5.10 s…");
    }

    #[test]
    fn a_name_lies_level_only_where_its_tile_has_room() {
        assert!(level(60.0, 5.0, 100.0));
        assert!(!level(60.0, 5.0, 65.0));
    }

    #[test]
    fn a_crowd_pulls_the_name_up() {
        let near = [Vec3::X, Vec3::Y, Vec3::NEG_X];
        assert!(crowded(Vec3::ZERO, &near, 2.0, 3));
        assert!(!crowded(Vec3::ZERO, &near, 0.5, 3));
    }

    #[test]
    fn a_point_is_inside_a_tile_either_way_round() {
        let quad =
            [(0.0, 0.0), (10.0, 0.0), (10.0, 4.0), (0.0, 4.0)].map(|(x, y)| egui::pos2(x, y));
        assert!(inside(quad, egui::pos2(5.0, 2.0)));
        assert!(!inside(quad, egui::pos2(11.0, 2.0)));
        let mut reversed = quad;
        reversed.reverse();
        assert!(inside(reversed, egui::pos2(5.0, 2.0)));
    }

    #[test]
    fn only_the_cursor_and_its_calls_are_tethered() {
        // 0 makes 1 and 3; 1 makes 2; the cursor is on 1.
        let tethers: Vec<Tether> = (0..4).map(|i| tether_of(Some((1, 2)), i)).collect();
        assert_eq!(
            tethers,
            vec![Tether::Other, Tether::Cursor, Tether::Made, Tether::Other]
        );
        let (cursor, made) = (
            string_strength(Tether::Cursor, false).unwrap(),
            string_strength(Tether::Made, false).unwrap(),
        );
        assert!(cursor.0 >= made.0);
        assert_eq!(
            string_strength(Tether::Other, false),
            None,
            "left to badges"
        );
        let many = Some((0, 40));
        assert_eq!(tether_of(many, TETHERS), Tether::Made);
        assert_eq!(tether_of(many, TETHERS + 1), Tether::Other, "capped");
        assert!(
            string_strength(Tether::Other, true).unwrap().0 < made.0,
            "faint"
        );
    }

    #[test]
    fn a_string_hangs_straight_down_at_both_ends() {
        let top = Vec3::new(10.0, 5.0, 20.0);
        let foot = Vec3::new(0.0, 0.0, 0.0);
        let points = string_points(top, Vec3::NEG_Z, foot, 20);
        assert_eq!(points.first(), Some(&top));
        assert!(points.last().unwrap().distance(foot) < 1e-4);
        let leaving = points[1] - points[0];
        assert!(leaving.z < 0.0 && leaving.truncate().length() < leaving.z.abs() * 0.2);
        let arriving = points[20] - points[19];
        assert!(arriving.truncate().length() < arriving.z.abs() * 0.2);
    }
}
