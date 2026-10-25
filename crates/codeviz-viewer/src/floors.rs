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

/// The upper floor as it stands this frame.
#[derive(Resource, Debug, Default)]
pub(crate) struct Floors {
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

/// The glass deck: where its middle is over the map, how big it is, and how high.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Deck {
    pub(crate) middle: Vec2,
    pub(crate) size: Vec2,
    pub(crate) height: f32,
}

impl Deck {
    /// Where a point `(across, up)` of the deck lies in the world, each 0 to 1 from its
    /// left and near edge, at `lift` of its height.
    pub(crate) fn at(&self, across: f32, up: f32, lift: f32) -> Vec3 {
        let corner = self.middle - self.size / 2.0;
        (corner + self.size * Vec2::new(across, up)).extend(self.height * lift)
    }

    fn eased(self, goal: Self, ease: f32) -> Self {
        Self {
            middle: self.middle.lerp(goal.middle, ease),
            size: self.size.lerp(goal.size, ease),
            height: self.height + (goal.height - self.height) * ease,
        }
    }
}

/// One call on the deck.
#[derive(Debug, Clone)]
pub(crate) struct Call {
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
}

impl Floors {
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

    /// The nodes the bright strings land on: the cursor's call's, its neighbors' and
    /// those of other calls to its function.
    pub(crate) fn lit_tiles(&self) -> HashSet<usize> {
        (0..self.calls.len())
            .filter(|&i| self.shows(i) && self.away(i).is_some_and(|away| away <= NEAR))
            .map(|i| self.calls[i].tile)
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
    store.config_mut::<StringGizmos>().0.line.width = 1.6;
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
pub(crate) const WINDOW: usize = 21;

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
const COLUMN: f32 = 3.2;
/// How wide the deck is at least, in the journey region's widest extent.
const WIDE: f32 = 0.95;
/// How deep a row is, as a share of a column's width.
const ROW: f32 = 0.8;
/// How deep the deck is at most, as a share of its width.
const SHALLOW: f32 = 0.3;
/// How high the deck floats over the map, as a share of its width, and at least.
const HEIGHT: f32 = 0.45;
const LOWEST: f32 = 14.0;
const CLEAR: f32 = 0.9;
const HIGHEST: f32 = 1.0;

/// The deck for `columns` calls `rows` deep over a journey whose nodes span `min` to
/// `max` on the map: centered over them, as wide as they are or wide enough for its
/// columns, and high enough to clear them.
pub(crate) fn plan_deck(min: Vec2, max: Vec2, columns: usize, rows: usize) -> Deck {
    let span = max - min;
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let (columns, rows) = (columns.max(1) as f32, rows.max(1) as f32);
    let width = (span.max_element() * WIDE).max(columns * COLUMN);
    let column = width / columns;
    let depth = (rows * column * ROW).min(width * SHALLOW);
    Deck {
        middle: (min + max) / 2.0,
        size: Vec2::new(width, depth),
        // High enough that, seen from in front at the view's tilt, the deck mostly
        // clears the journey's region of the map under it.
        height: ((span.y + depth) * CLEAR)
            .clamp(width * HEIGHT, width * HIGHEST)
            .max(LOWEST),
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

/// Calls either side of the cursor whose strings draw bright.
const NEAR: usize = 2;

/// How strongly a string draws, for a call `away` calls from the cursor's: the
/// cursor's in full, its two neighbors either side bright, the rest faint.
pub(crate) fn string_alpha(away: Option<usize>) -> f32 {
    match away {
        Some(0) => 1.0,
        Some(1) => 0.8,
        Some(2) => 0.6,
        None => 0.4,
        Some(_) => 0.25,
    }
}

/// Points along a string from `top` on the deck to `foot` on the map: it leaves the
/// deck straight down and arrives at its node straight down, bending between, so
/// both ends read as hanging.
pub(crate) fn string_points(top: Vec3, foot: Vec3, pieces: usize) -> Vec<Vec3> {
    let drop = (top.z - foot.z).max(0.0) * 0.45;
    let (a, b) = (top - Vec3::Z * drop, foot + Vec3::Z * drop);
    (0..=pieces)
        .map(|i| {
            #[expect(clippy::cast_precision_loss, reason = "a few pieces")]
            let t = i as f32 / pieces.max(1) as f32;
            let u = 1.0 - t;
            top * u * u * u + a * 3.0 * u * u * t + b * 3.0 * u * t * t + foot * t * t * t
        })
        .collect()
}

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
    let plan = plan_deck(min, max, shown.len(), rows);
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
    let grown = plan.size.x > floors.fitted * 1.1;
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
}

/// Places each call's ball on the deck, in the window `shown` of it `rows` deep, and
/// the foot of its string on its node.
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
    calls
        .into_iter()
        .enumerate()
        .filter_map(|(i, (_, tile, number, depth))| {
            let at = explorer.tile_point(tile)?;
            let spot = deck_spot(i.clamp(shown.start, shown.end), depth, shown, rows);
            #[expect(clippy::cast_precision_loss, reason = "a small count")]
            let half = 0.5 / shown.len().max(1) as f32;
            Some(Call {
                tile,
                number,
                depth,
                end: ends[i],
                marker: deck.at(spot.x + half, spot.y, lift),
                foot: at + Vec3::Z * node_size(graph, tile) * 0.5,
            })
        })
        .collect()
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
    *transform = Transform::from_translation(deck.middle.extend(deck.height * floors.lift()))
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
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(x, y)| {
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
        let away = floors.away(index);
        let alpha = string_alpha(away) * fade;
        let tint = if away == Some(0) {
            Color::WHITE
        } else {
            node_color(&explorer, None, call.tile).mix(&STRING, 0.55)
        };
        let points = string_points(call.marker, call.foot, 16);
        let pieces = points.len().saturating_sub(1);
        for (i, pair) in points.windows(2).enumerate() {
            // Brightest at the deck, thinning towards the map: a little for the bright
            // strings, most of the way for the rest, whose pools mark where they land,
            // so a crowd of strings doesn't knot over a crowd of nodes.
            #[expect(clippy::cast_precision_loss, reason = "a few pieces")]
            let t = i as f32 / pieces.max(1) as f32;
            let thin = if away.is_some_and(|a| a <= NEAR) {
                0.35
            } else {
                0.8
            };
            let color = tint.with_alpha(alpha * (1.0 - thin * t));
            if away == Some(0) && floors.cursor == Some(index) {
                bold.line(pair[0], pair[1], color);
            } else {
                gizmos.line(pair[0], pair[1], color);
            }
        }
    }
    if let Some(call) = floors.cursor.and_then(|c| floors.calls.get(c)) {
        let ring = Isometry3d::from_translation(call.foot);
        bold.circle(ring, 1.6, Color::srgba(1.0, 1.0, 1.0, 0.85 * fade));
    }
}

// ---- painting the deck ----

/// Names on the deck are light, set at a slant so a crowd of columns each has room.
const SLANT: f32 = -0.85;

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
    eye: Eye,
) {
    if !floors.showing() {
        return;
    }
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
    let rows = floors.calls[shown.clone()]
        .iter()
        .map(|c| c.depth + 1)
        .max()
        .unwrap_or(1);
    let deck = floors.deck;
    // Tiles first, shallow rows first.
    for (index, call) in floors.calls.iter().enumerate() {
        let on_deck = floors.shows(index) || (index < shown.start && call.end >= shown.start);
        if !on_deck {
            continue;
        }
        let first = index.max(shown.start);
        let last = call.end.min(shown.end.saturating_sub(1));
        let left = deck_spot(first, call.depth, &shown, rows);
        let right = deck_spot(last + 1, call.depth, &shown, rows);
        #[expect(clippy::cast_precision_loss, reason = "small counts")]
        let (half, inset) = (0.5 / rows as f32 * 0.5, 0.08 / shown.len().max(1) as f32);
        let quad = [
            (left.x + inset, left.y - half),
            (right.x - inset, left.y - half),
            (right.x - inset, left.y + half),
            (left.x + inset, left.y + half),
        ]
        .map(|(x, y)| project(deck.at(x, y, fade)));
        let [Some(a), Some(b), Some(c), Some(d)] = quad else {
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
        if floors.cursor == Some(index) || floors.again(index) {
            let width = if floors.cursor == Some(index) {
                1.8
            } else {
                1.2
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
    for index in shown.clone() {
        let call = &floors.calls[index];
        let radius = marker_radius(eye.nearness(call.marker), floors.cursor == Some(index));
        name_call(painter, project, explorer, floors, index, (radius, fade));
    }
    count_beyond(painter, project, floors, rows, fade);
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
        Some(0) => (12.5, egui::Color32::from_rgb(235, 238, 246)),
        Some(1..=2) => (12.5, egui::Color32::from_rgb(214, 219, 230)),
        None => (12.0, egui::Color32::from_rgb(196, 202, 216)),
        Some(_) => (11.5, egui::Color32::from_rgb(172, 179, 194)),
    };
    let name = node_name(explorer, call.tile);
    let text = if call.number.is_empty() {
        name
    } else {
        format!("{} {name}", call.number)
    };
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(size), color);
    // Starting just clear of the ball, along the slant.
    let start = at + egui::vec2(SLANT.cos(), SLANT.sin()) * (radius + 3.0);
    let corner = start - egui::vec2(0.0, galley.size().y * 0.5);
    let shape = egui::epaint::TextShape::new(corner, galley, color)
        .with_angle_and_anchor(SLANT, egui::Align2::LEFT_CENTER)
        .with_override_text_color(color.gamma_multiply(fade));
    painter.add(shape);
}

/// Says how many calls lie past the window at either end of the deck.
fn count_beyond(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    floors: &Floors,
    rows: usize,
    fade: f32,
) {
    let deck = floors.deck;
    let (before, after) = (
        floors.window.start,
        floors.calls.len().saturating_sub(floors.window.end),
    );
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    let row = 0.5 / rows as f32;
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
        let small = plan_deck(Vec2::ZERO, Vec2::new(100.0, 40.0), 4, 2);
        assert!(
            (small.size.x - 100.0 * WIDE).abs() < 1e-4,
            "as wide as the journey"
        );
        assert_eq!(small.middle, Vec2::new(50.0, 20.0));
        let crowd = plan_deck(Vec2::ZERO, Vec2::new(10.0, 10.0), 21, 1);
        assert!(crowd.size.x >= 21.0 * COLUMN - 1e-4, "room for each column");
        assert!(crowd.height >= LOWEST);
        let deep = plan_deck(Vec2::ZERO, Vec2::new(10.0, 10.0), 21, 3);
        assert!(deep.size.y > crowd.size.y, "deeper with more rows");
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
    fn strings_are_brightest_round_the_cursor() {
        assert!(string_alpha(Some(0)) > string_alpha(Some(1)));
        assert!(string_alpha(Some(1)) > string_alpha(Some(2)));
        assert!(string_alpha(Some(2)) > string_alpha(Some(3)));
        assert!(string_alpha(Some(30)) > 0.0, "never gone");
    }

    #[test]
    fn a_string_hangs_straight_down_at_both_ends() {
        let top = Vec3::new(10.0, 5.0, 20.0);
        let foot = Vec3::new(0.0, 0.0, 0.0);
        let points = string_points(top, foot, 20);
        assert_eq!(points.first(), Some(&top));
        assert!(points.last().unwrap().distance(foot) < 1e-4);
        let leaving = points[1] - points[0];
        assert!(leaving.z < 0.0 && leaving.truncate().length() < leaving.z.abs() * 0.2);
        let arriving = points[20] - points[19];
        assert!(arriving.truncate().length() < arriving.z.abs() * 0.2);
    }
}
