//! A diff over the graph, drawn two ways. The graph lays out both versions at once,
//! so every unit either side has keeps one place.
//!
//! - Status: added units glow green, rewritten ones gold, removed ones linger as red
//!   ghosts; calls that appeared are green and calls that went away red. `N` steps
//!   through the changes in the order to read them, callees first, and the focused
//!   change shows the way up from it to the entry points it can affect.
//! - Morph: the graph as it was, then as it is. Each version is laid out on its own,
//!   starting from the shared layout, and the graph moves between the two: removed
//!   units shrink away, added ones grow in, and the rest shift as the calls between
//!   them change.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::*;
use bevy_egui::{egui, input::egui_wants_any_keyboard_input};
use codeviz_core::{Change, Diff, Review, System};
use codeviz_layout::ForceGraph;

use crate::{
    camera::OrbitCamera,
    change_pane,
    explorer::{Clustering, Explorer},
    graph_view::{self, NodeLook},
    lens::{Lens, SPECK},
    scene::{PALETTE, Tile},
    ui::{MUTED, egui_color},
};

pub(crate) const ADDED: Color = Color::srgb(0.32, 0.95, 0.5);
pub(crate) const REMOVED: Color = Color::srgb(1.0, 0.34, 0.34);
/// Yellow rather than orange, and moved grey rather than blue, to stay clear of the
/// violet and blue that say which way a call runs.
pub(crate) const MODIFIED: Color = Color::srgb(1.0, 0.86, 0.3);
pub(crate) const MOVED: Color = Color::srgb(0.8, 0.82, 0.9);

/// Steps each version's own layout runs from the shared one.
const VERSION_STEPS: usize = 160;
/// Seconds of one morph cycle: hold before, move, hold after, move back.
const HOLD: f32 = 1.2;
const MOVE: f32 = 2.4;
/// The most entry points the way up from a change runs to, nearest first.
const ROUTE_ENDS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DiffStyle {
    #[default]
    Status,
    Morph,
}

impl DiffStyle {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Status => Self::Morph,
            Self::Morph => Self::Status,
        }
    }
}

/// Counts for the summary.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Totals {
    pub(crate) added: usize,
    pub(crate) removed: usize,
    pub(crate) modified: usize,
    pub(crate) moved: usize,
    pub(crate) calls_added: usize,
    pub(crate) calls_removed: usize,
}

/// Where things start: an entry point, or a function nothing calls.
#[derive(Debug, Clone)]
pub(crate) struct Start {
    /// Its node, when the graph has one.
    pub(crate) tile: Option<usize>,
    pub(crate) label: String,
}

/// The way up from a change to the starts it can affect: the shortest call chain to
/// each of the nearest few.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Route {
    /// Every node on the way, the change included.
    pub(crate) tiles: HashSet<usize>,
    /// The calls along it, as `(caller, callee)`.
    pub(crate) calls: HashSet<(usize, usize)>,
    /// The starts it reaches, nearest first.
    pub(crate) ends: Vec<usize>,
}

#[derive(Resource, Debug)]
pub(crate) struct DiffView {
    pub(crate) style: DiffStyle,
    pub(crate) totals: Totals,
    changes: HashMap<usize, Change>,
    added_calls: HashSet<(usize, usize)>,
    removed_calls: HashSet<(usize, usize)>,
    /// The changes in the order to read them: each after the changes it calls.
    order: Vec<usize>,
    /// Where in `order` stepping last stood.
    at: Option<usize>,
    pub(crate) starts: Vec<Start>,
    /// Whether the starts are the functions nothing calls, for want of declared ones.
    pub(crate) roots: bool,
    /// The starts each change can affect, by index into `starts`.
    reached_from: HashMap<usize, Vec<usize>>,
    /// The starts any change affects, those more changes reach first.
    pub(crate) affected: Vec<usize>,
    /// What calls each node in either version, by tile.
    callers: Vec<Vec<usize>>,
    /// The way up from the focused change, and the change it starts from.
    route: Option<(usize, Route)>,
    /// The old version, with moved functions under their new ids.
    old: System,
    /// What each moved node was called before.
    moved: HashMap<usize, String>,
    /// Where each node sits in the old version, the new one, and both together.
    before: Vec<[f32; 3]>,
    after: Vec<[f32; 3]>,
    shared: Vec<[f32; 3]>,
    /// The clustering `before` and `after` were laid out under.
    laid_out_for: Clustering,
    /// From 0 (before) to 1 (after).
    pub(crate) morph: f32,
    pub(crate) playing: bool,
    clock: f32,
}

impl DiffView {
    /// Draws `diff` over the explorer's graph, which stops counting removed calls as
    /// calls. Journeys run over the explorer's system, the new version.
    pub(crate) fn new(explorer: &mut Explorer, diff: &Diff, before: System) -> Self {
        let removed = {
            let map = explorer.map();
            diff.removed_calls
                .iter()
                .filter_map(|(a, b)| Some((map.tile_of(a)?, map.tile_of(b)?)))
                .filter(|(a, b)| a != b)
                .collect()
        };
        explorer.retire_calls(&removed);
        let explorer = &*explorer;
        let map = explorer.map();
        let tile = |unit: &codeviz_ir::UnitId| map.tile_of(unit);
        let changes: HashMap<usize, Change> = diff
            .units
            .iter()
            .filter_map(|(unit, &change)| Some((tile(unit)?, change)))
            .collect();
        let pairs = |calls: &HashSet<codeviz_core::Call>| -> HashSet<(usize, usize)> {
            calls
                .iter()
                .filter_map(|(a, b)| Some((tile(a)?, tile(b)?)))
                .filter(|(a, b)| a != b)
                .collect()
        };
        let (added_calls, removed_calls) = (pairs(&diff.added_calls), pairs(&diff.removed_calls));
        let count = |kind| changes.values().filter(|&&c| c == kind).count();
        let totals = Totals {
            added: count(Change::Added),
            removed: count(Change::Removed),
            modified: count(Change::Modified),
            moved: count(Change::Moved),
            calls_added: added_calls.len(),
            calls_removed: removed_calls.len(),
        };

        let review = Review::new(diff, &before, explorer.system());
        let order: Vec<usize> = review.order.iter().filter_map(tile).collect();
        let starts: Vec<Start> = review
            .starts
            .iter()
            .map(|start| Start {
                tile: tile(&start.unit),
                label: start.label.clone(),
            })
            .collect();
        let reached_from = review
            .order
            .iter()
            .filter_map(|unit| Some((tile(unit)?, review.reached_from(unit).to_vec())))
            .collect();
        let graph = explorer.graph();
        let mut callers = vec![Vec::new(); map.tiles.len()];
        for &(from, to) in graph.calls.iter().chain(&graph.ghosts) {
            if from != to
                && let Some(list) = callers.get_mut(to)
            {
                list.push(from);
            }
        }

        let (before_at, after_at) = versions(explorer, &before, &changes);
        Self {
            style: DiffStyle::default(),
            totals,
            changes,
            added_calls,
            removed_calls,
            order,
            at: None,
            starts,
            roots: review.roots,
            reached_from,
            affected: review.affected(),
            callers,
            route: None,
            moved: diff
                .moved
                .iter()
                .filter_map(|(now, was)| {
                    // The old id, without its scheme: where the function used to live.
                    let path = was
                        .as_str()
                        .split_once(':')
                        .map_or(was.as_str(), |(_, p)| p);
                    Some((map.tile_of(now)?, path.to_owned()))
                })
                .collect(),
            old: before,
            before: before_at,
            after: after_at,
            shared: graph.positions.clone(),
            laid_out_for: explorer.clustering(),
            morph: 0.0,
            playing: true,
            clock: 0.0,
        }
    }

    /// The version the diff starts from.
    pub(crate) fn old(&self) -> &System {
        &self.old
    }

    /// The changes in the order to read them.
    pub(crate) fn order(&self) -> &[usize] {
        &self.order
    }

    /// Where `tile` comes in the reading order, from 0.
    pub(crate) fn place(&self, tile: usize) -> Option<usize> {
        self.order.iter().position(|&t| t == tile)
    }

    /// The change `by` places on from the focus in the reading order: the next or the
    /// previous one, or the first when the focus is not a change and none was stepped
    /// to yet. Stays at either end.
    pub(crate) fn step_from(&mut self, focus: Option<usize>, by: isize) -> Option<usize> {
        let last = self.order.len().checked_sub(1)?;
        let at = focus.and_then(|f| self.place(f)).or(self.at);
        let next = match at {
            Some(at) => at.saturating_add_signed(by).min(last),
            None if by < 0 => last,
            None => 0,
        };
        self.at = Some(next);
        self.order.get(next).copied()
    }

    /// The starts a change can affect, by index into [`Self::starts`].
    pub(crate) fn reached_from(&self, tile: usize) -> &[usize] {
        self.reached_from.get(&tile).map_or(&[], Vec::as_slice)
    }

    /// What a moved node was called before.
    pub(crate) fn moved_from(&self, tile: usize) -> Option<&str> {
        self.moved.get(&tile).map(String::as_str)
    }

    pub(crate) fn change(&self, tile: usize) -> Option<Change> {
        self.changes.get(&tile).copied()
    }

    /// Whether a call appeared or went away.
    pub(crate) fn call_change(&self, from: usize, to: usize) -> Option<Change> {
        if self.added_calls.contains(&(from, to)) {
            Some(Change::Added)
        } else if self.removed_calls.contains(&(from, to)) {
            Some(Change::Removed)
        } else {
            None
        }
    }

    /// The calls a change gained and lost, by callee.
    pub(crate) fn calls_changed(&self, tile: usize) -> (Vec<usize>, Vec<usize>) {
        let from = |calls: &HashSet<(usize, usize)>| {
            let mut callees: Vec<usize> = calls
                .iter()
                .filter(|(caller, _)| *caller == tile)
                .map(|&(_, callee)| callee)
                .collect();
            callees.sort_unstable();
            callees
        };
        (from(&self.added_calls), from(&self.removed_calls))
    }

    /// The way up from the focused change to what it affects, when the focus is one.
    pub(crate) fn route(&self) -> Option<&Route> {
        self.route.as_ref().map(|(_, route)| route)
    }

    /// Whether a node is a start on the way up from the focused change.
    pub(crate) fn route_end(&self, tile: usize) -> bool {
        self.route().is_some_and(|r| r.ends.contains(&tile))
    }

    /// Climbs from `focus` when it is a change, else clears the way up. Returns the
    /// nodes on the way when it moved.
    fn follow(&mut self, focus: Option<usize>) -> Option<Vec<usize>> {
        let from = focus.filter(|&f| self.changes.contains_key(&f));
        if self.route.as_ref().map(|(tile, _)| *tile) == from {
            return None;
        }
        let tiles: HashSet<usize> = self.starts.iter().filter_map(|s| s.tile).collect();
        self.route = from.map(|from| {
            (
                from,
                route(from, &self.callers, |t| tiles.contains(&t), ROUTE_ENDS),
            )
        });
        Some(
            self.route()
                .map(|r| r.tiles.iter().copied().collect())
                .unwrap_or_default(),
        )
    }

    /// How much of the "after" version is showing, eased.
    pub(crate) fn progress(&self) -> f32 {
        match self.style {
            DiffStyle::Morph => ease(self.morph),
            DiffStyle::Status => 1.0,
        }
    }

    /// How present a node is: removed ones fade out over the morph, added ones in.
    pub(crate) fn presence(&self, tile: usize) -> f32 {
        if self.style != DiffStyle::Morph {
            return 1.0;
        }
        let t = self.progress();
        match self.change(tile) {
            Some(Change::Added) => t,
            Some(Change::Removed) => 1.0 - t,
            _ => 1.0,
        }
    }

    /// How present a call is, the same way.
    pub(crate) fn call_presence(&self, from: usize, to: usize) -> f32 {
        if self.style != DiffStyle::Morph {
            return 1.0;
        }
        let t = self.progress();
        match self.call_change(from, to) {
            Some(Change::Added) => t,
            Some(Change::Removed) => 1.0 - t,
            _ => 1.0,
        }
    }

    /// Changed tiles, for listing.
    pub(crate) fn changed(&self) -> impl Iterator<Item = (usize, Change)> + '_ {
        self.changes.iter().map(|(&t, &c)| (t, c))
    }

    fn positions(&self) -> Vec<[f32; 3]> {
        let t = self.progress();
        self.before
            .iter()
            .zip(&self.after)
            .map(|(a, b)| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t))
            .collect()
    }
}

/// The way up from `from` along `callers` to the nearest `limit` starts: breadth
/// first, so each start's chain is a shortest one.
fn route(
    from: usize,
    callers: &[Vec<usize>],
    is_start: impl Fn(usize) -> bool,
    limit: usize,
) -> Route {
    // Each node reached, by the node it calls on the way back down.
    let mut down: HashMap<usize, usize> = HashMap::new();
    let mut ends = Vec::new();
    let mut queue = VecDeque::from([from]);
    let mut seen = HashSet::from([from]);
    while let Some(at) = queue.pop_front() {
        if is_start(at) {
            ends.push(at);
            if ends.len() == limit {
                break;
            }
        }
        for &caller in callers.get(at).into_iter().flatten() {
            if seen.insert(caller) {
                down.insert(caller, at);
                queue.push_back(caller);
            }
        }
    }
    let mut route = Route {
        tiles: HashSet::from([from]),
        ends,
        ..Route::default()
    };
    for &end in &route.ends {
        let mut at = end;
        while let Some(&callee) = down.get(&at) {
            route.tiles.insert(at);
            route.calls.insert((at, callee));
            at = callee;
        }
    }
    route
}

/// Where each node sits in the old version and in the new one: each laid out alone,
/// starting from the graph as it stands. What one side lacks grows or shrinks where
/// the other side has it.
fn versions(
    explorer: &Explorer,
    old: &System,
    changes: &HashMap<usize, Change>,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let settle = |system: &System| {
        let forces = explorer.clustering().forces();
        let mut version = ForceGraph::new(system, explorer.map(), forces);
        version.positions.clone_from(&explorer.graph().positions);
        for _ in 0..VERSION_STEPS {
            version.step();
        }
        version.positions
    };
    let (mut before, mut after) = (settle(old), settle(explorer.system()));
    for (&tile, &change) in changes {
        match change {
            Change::Added => before[tile] = after[tile],
            Change::Removed => after[tile] = before[tile],
            Change::Modified | Change::Moved => {}
        }
    }
    (before, after)
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Where a cycle `clock` seconds in leaves the morph.
fn cycle(clock: f32) -> f32 {
    let period = 2.0 * (HOLD + MOVE);
    let at = clock.rem_euclid(period);
    if at < HOLD {
        0.0
    } else if at < HOLD + MOVE {
        (at - HOLD) / MOVE
    } else if at < 2.0 * HOLD + MOVE {
        1.0
    } else {
        1.0 - (at - 2.0 * HOLD - MOVE) / MOVE
    }
}

pub(crate) struct DiffPlugin;

impl Plugin for DiffPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                diff_keys.run_if(not(egui_wants_any_keyboard_input)),
                follow_route,
                animate,
                frame_changes,
            )
                .chain()
                .after(graph_view::style_nodes)
                .after(graph_view::follow_focus)
                .run_if(resource_exists::<DiffView>),
        );
    }
}

/// N steps to the next change in reading order and shift+N back, bringing the source
/// pane back if it was closed; D cycles the styles; T pauses or resumes the morph; C
/// shows only what changed, or everything again.
fn diff_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut view: ResMut<DiffView>,
    mut explorer: ResMut<Explorer>,
    mut pane: ResMut<change_pane::Closed>,
) {
    if keys.just_pressed(KeyCode::KeyN) {
        let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        if let Some(tile) = view.step_from(explorer.graph_focus(), if shift { -1 } else { 1 }) {
            explorer.open_at(tile);
            pane.0 = false;
        }
    }
    if keys.just_pressed(KeyCode::KeyC) {
        let lens = explorer.lens();
        explorer.set_lens(Lens {
            changes_only: !lens.changes_only,
            ..lens
        });
    }
    if keys.just_pressed(KeyCode::KeyD) {
        view.style = view.style.next();
    }
    if keys.just_pressed(KeyCode::KeyT) {
        view.playing = !view.playing;
    }
}

/// Keeps the way up from the focused change showing.
fn follow_route(mut view: ResMut<DiffView>, mut explorer: ResMut<Explorer>) {
    let focus = explorer.graph_focus();
    if let Some(tiles) = view.bypass_change_detection().follow(focus) {
        explorer.set_trail(tiles);
        view.set_changed();
    }
}

/// Runs the morph, moving and sizing nodes as it goes; puts the shared layout back
/// when the morph stops showing.
fn animate(
    time: Res<Time>,
    mut view: ResMut<DiffView>,
    mut explorer: ResMut<Explorer>,
    mut nodes: Query<(&Tile, &mut Transform)>,
    mut last: Local<Option<DiffStyle>>,
) {
    let entered = *last != Some(view.style);
    let left_morph = entered && *last == Some(DiffStyle::Morph);
    *last = Some(view.style);
    if view.style != DiffStyle::Morph {
        if left_morph {
            let shared = view.shared.clone();
            explorer.bypass_change_detection().place_nodes(&shared);
            for (tile, mut transform) in &mut nodes {
                if let Some(at) = explorer.tile_point(tile.0) {
                    transform.translation = at;
                }
                transform.scale = Vec3::splat(graph_view::node_size(explorer.graph(), tile.0));
            }
            explorer.set_changed();
        }
        return;
    }
    if view.laid_out_for != explorer.clustering() {
        // Regathered since the versions were laid out: settle the shared layout under
        // the new forces, then lay both versions out again from it.
        let explorer = explorer.bypass_change_detection();
        if !entered {
            let shared = view.shared.clone();
            explorer.place_nodes(&shared);
        }
        explorer.settle_now();
        view.shared.clone_from(&explorer.graph().positions);
        let (before, after) = versions(explorer, &view.old, &view.changes);
        (view.before, view.after) = (before, after);
        view.laid_out_for = explorer.clustering();
    }
    if view.playing {
        view.clock += time.delta_secs();
        view.morph = cycle(view.clock);
    }
    let positions = view.positions();
    explorer.bypass_change_detection().place_nodes(&positions);
    for (tile, mut transform) in &mut nodes {
        if let Some(at) = explorer.tile_point(tile.0) {
            transform.translation = at;
        }
        let size = graph_view::node_size(explorer.graph(), tile.0);
        // Never quite zero, so a removed node can still be picked mid-fade.
        transform.scale = Vec3::splat(size * view.presence(tile.0).max(0.02));
    }
    if entered {
        explorer.set_changed();
    }
}

/// Opens looking at everything that changed, wherever either version puts it.
fn frame_changes(view: Res<DiffView>, mut cameras: Query<&mut OrbitCamera>, mut done: Local<bool>) {
    if *done {
        return;
    }
    *done = true;
    let points: Vec<Vec3> = view
        .changes
        .keys()
        .flat_map(|&t| [view.before[t], view.after[t], view.shared[t]])
        .map(Vec3::from)
        .collect();
    if points.is_empty() {
        return;
    }
    let middle = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    let radius = points
        .iter()
        .map(|p| p.distance(middle))
        .fold(0.0, f32::max);
    for mut camera in &mut cameras {
        camera.glide_to(middle, (radius * 3.2).max(30.0));
    }
}

/// How a node looks under the diff: its change; or, left alone, neutral where it is
/// near the focus or on the way up from it, else faded.
pub(crate) fn diff_look(explorer: &Explorer, view: &DiffView, tile: usize) -> NodeLook {
    if !explorer.shown(tile) {
        return NodeLook::Hidden;
    }
    if explorer.attention() == Some(tile) {
        return NodeLook::Focus;
    }
    if let Some(change) = view.change(tile) {
        return NodeLook::Changed(change);
    }
    let near = explorer
        .reach()
        .and_then(|reach| graph_view::near(explorer, reach, tile));
    let on_route = view.route().is_some_and(|r| r.tiles.contains(&tile));
    match near {
        Some(hops) => NodeLook::Unchanged(hops),
        None if view.route_end(tile) => NodeLook::Unchanged(1),
        None if on_route => NodeLook::Unchanged(2),
        None => NodeLook::Faded(explorer.tile_color(tile) % PALETTE.len()),
    }
}

/// Every call, colored by the diff. Calls touching the center of attention say which
/// way they run, violet in and blue out, with dots flowing along them; otherwise added
/// calls are green and removed ones red and dashed; the way up from the focused change
/// to its entry points, and the journey's path, are pale arrows; the rest are faint.
pub(crate) fn draw_diff_graph(
    explorer: &Explorer,
    view: &DiffView,
    time: f32,
    gizmos: &mut Gizmos,
) {
    let graph = explorer.graph();
    let center = explorer.attention();
    // The calls the open journey discloses, drawn as arrows so it reads as a path.
    let track = explorer.track();
    let path: HashSet<(usize, usize)> = track
        .scene()
        .edges
        .iter()
        .filter(|edge| explorer.disclosed(edge.callee))
        .filter_map(|edge| {
            let node = |span| track.unit_id(span).and_then(|u| explorer.map().tile_of(u));
            Some((node(edge.caller)?, node(edge.callee)?))
        })
        .collect();
    let route = view.route();
    for (index, &(from, to)) in graph.calls.iter().chain(&graph.ghosts).enumerate() {
        let (Some(a), Some(b)) = (explorer.tile_point(from), explorer.tile_point(to)) else {
            continue;
        };
        let shown = explorer.visibility(from).min(explorer.visibility(to));
        if shown < SPECK {
            continue;
        }
        let presence = view.call_presence(from, to) * shown;
        if presence <= 0.01 {
            continue;
        }
        let back = (b - a).normalize_or_zero() * graph_view::node_size(graph, to) * 0.5;
        let side = match center {
            Some(c) if c == to => Some(graph_view::CALLER),
            Some(c) if c == from => Some(graph_view::CALLEE),
            _ => None,
        };
        match view.call_change(from, to) {
            Some(Change::Removed) if let Some(color) = side => {
                dashed(gizmos, a, b - back, color.with_alpha(0.85 * presence));
            }
            _ if let Some(color) = side => {
                let color = color.with_alpha(0.95 * presence);
                gizmos.arrow(a, b - back, color).with_tip_length(1.1);
                graph_view::flow(gizmos, a, b - back, color, time, index);
            }
            Some(Change::Added) => {
                gizmos
                    .arrow(a, b - back, ADDED.with_alpha(0.9 * presence))
                    .with_tip_length(0.6);
            }
            Some(Change::Removed) => {
                dashed(gizmos, a, b - back, REMOVED.with_alpha(0.85 * presence));
            }
            _ if route.is_some_and(|r| r.calls.contains(&(from, to))) => {
                gizmos
                    .arrow(a, b - back, graph_view::FURTHER.with_alpha(0.6 * shown))
                    .with_tip_length(0.8);
            }
            _ if path.contains(&(from, to)) => {
                gizmos
                    .arrow(a, b - back, graph_view::FURTHER.with_alpha(0.45 * shown))
                    .with_tip_length(0.8);
            }
            _ => {
                gizmos.line(a, b, Color::srgba(0.6, 0.66, 0.8, 0.07 * shown));
            }
        }
    }
}

/// A line drawn in dashes, for what is gone.
fn dashed(gizmos: &mut Gizmos, a: Vec3, b: Vec3, color: Color) {
    let length = a.distance(b);
    let dashes = (length / 0.9).max(1.0).floor();
    let mut at = 0.0;
    while at < dashes {
        let from = a.lerp(b, at / dashes);
        let to = a.lerp(b, ((at + 1.0) / dashes).min(1.0));
        gizmos.line(from, to, color);
        at += 2.0;
    }
}

/// A change's sign, as labels and lists show it.
pub(crate) fn sign(change: Change) -> &'static str {
    match change {
        Change::Added => "+",
        Change::Removed => "−",
        Change::Modified => "~",
        Change::Moved => "→",
    }
}

pub(crate) fn change_color(change: Change) -> Color {
    match change {
        Change::Added => ADDED,
        Change::Removed => REMOVED,
        Change::Modified => MODIFIED,
        Change::Moved => MOVED,
    }
}

/// What changed, how it is drawn, and what it reaches: top right of the graph. The
/// changes are listed here only when the change pane, which lists them, is closed.
/// Rows focus their node.
pub(crate) fn diff_card(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &mut Explorer,
    view: &mut DiffView,
    listed: bool,
) {
    let mut chosen = None;
    let mut style = view.style;
    let focus = explorer.graph_focus();
    egui::Area::new("diff_card".into())
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(egui::pos2(area.max.x - 12.0, area.min.y + 10.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 225))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_max_width(300.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Changes").strong().size(15.0));
                        if let Some(at) = focus.and_then(|f| view.place(f)) {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} of {}",
                                            at + 1,
                                            view.order.len()
                                        ))
                                        .color(MUTED),
                                    );
                                },
                            );
                        }
                    });
                    totals_line(ui, view.totals);
                    let mut lens = explorer.lens();
                    if ui
                        .checkbox(&mut lens.changes_only, "Only changes")
                        .on_hover_text(
                            "Hide what did not change, but for the focus, its journey's root \
                             and the way up from it to its entry points (C)",
                        )
                        .changed()
                    {
                        explorer.set_lens(lens);
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut style, DiffStyle::Status, "Status")
                            .on_hover_text("What changed, and the calls that came and went (D)");
                        ui.selectable_value(&mut style, DiffStyle::Morph, "Morph")
                            .on_hover_text("The graph before, then after (D)");
                    });
                    match style {
                        DiffStyle::Status => {
                            chosen = affected_list(ui, view);
                            if listed {
                                let height = area.height() * 0.4;
                                chosen =
                                    reading_list(ui, explorer, view, "card", height).or(chosen);
                            }
                        }
                        DiffStyle::Morph => morph_controls(ui, view),
                    }
                });
        });
    if style != view.style {
        view.style = style;
    }
    if let Some(tile) = chosen {
        explorer.open_at(tile);
    }
}

/// Play, pause and scrub the morph.
fn morph_controls(ui: &mut egui::Ui, view: &mut DiffView) {
    ui.horizontal(|ui| {
        let label = if view.playing { "Pause" } else { "Play" };
        if ui
            .button(label)
            .on_hover_text("Play or pause (T)")
            .clicked()
        {
            view.playing = !view.playing;
        }
        ui.label(egui::RichText::new("before").small().color(MUTED));
        let mut t = view.morph;
        if ui
            .add(egui::Slider::new(&mut t, 0.0..=1.0).show_value(false))
            .changed()
        {
            view.morph = t;
            view.playing = false;
        }
        ui.label(egui::RichText::new("after").small().color(MUTED));
    });
}

/// How many functions changed each way, and how many calls came and went.
fn totals_line(ui: &mut egui::Ui, totals: Totals) {
    ui.horizontal(|ui| {
        let count = |ui: &mut egui::Ui, change, n: usize, what: &str| {
            if n == 0 {
                return;
            }
            ui.label(
                egui::RichText::new(format!("{}{n} {what}", sign(change)))
                    .color(egui_color(change_color(change))),
            );
        };
        count(ui, Change::Added, totals.added, "added");
        count(ui, Change::Modified, totals.modified, "rewritten");
        count(ui, Change::Removed, totals.removed, "removed");
        count(ui, Change::Moved, totals.moved, "moved");
    });
    ui.label(
        egui::RichText::new(format!(
            "calls: +{} new, −{} gone",
            totals.calls_added, totals.calls_removed
        ))
        .small()
        .color(MUTED),
    );
}

/// The entry points the changes reach, those more changes reach first, folded away
/// at first when there are many. Returns the node of a row clicked.
fn affected_list(ui: &mut egui::Ui, view: &DiffView) -> Option<usize> {
    if view.affected.is_empty() {
        return None;
    }
    let mut chosen = None;
    let what = if view.roots {
        "functions nothing calls"
    } else {
        "entry points"
    };
    let title = format!(
        "Affects {} of {} {what}",
        view.affected.len(),
        view.starts.len()
    );
    egui::CollapsingHeader::new(egui::RichText::new(title).size(13.0))
        .id_salt("affected")
        .default_open(view.affected.len() <= 4)
        .show(ui, |ui| {
            for &index in &view.affected {
                let Some(start) = view.starts.get(index) else {
                    continue;
                };
                let reached = view
                    .order
                    .iter()
                    .filter(|&&t| view.reached_from(t).contains(&index))
                    .count();
                let plural = if reached == 1 { "" } else { "s" };
                let text =
                    egui::RichText::new(format!("{}  · {reached} change{plural}", start.label));
                let response = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                if let Some(tile) = start.tile
                    && response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                {
                    chosen = Some(tile);
                }
            }
        });
    chosen
}

/// Every change in the order to read them, numbered, each with how many entry points
/// it affects, in a list that scrolls past `max_height`; the focused one is marked
/// and kept in view as stepping moves it. Returns the node of a row clicked.
pub(crate) fn reading_list(
    ui: &mut egui::Ui,
    explorer: &Explorer,
    view: &DiffView,
    id: &str,
    max_height: f32,
) -> Option<usize> {
    let focus = explorer.graph_focus();
    ui.label(
        egui::RichText::new("All changes, callees before callers · N next, Shift N back")
            .small()
            .color(MUTED),
    );
    // Scroll to the focus only when it moves, so the list can still be scrolled.
    let scrolled = egui::Id::new(("reading_list_focus", id));
    let moved = ui.ctx().data_mut(|d| {
        let last = d.get_temp::<Option<usize>>(scrolled).flatten();
        d.insert_temp(scrolled, focus);
        last != focus
    });
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .id_salt(("reading_list", id))
        .max_height(max_height)
        // As tall as it may be from the start, not just what the card had room for
        // before the list showed.
        .min_scrolled_height(max_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (index, &tile) in view.order.iter().enumerate() {
                let Some(change) = view.change(tile) else {
                    continue;
                };
                let name = explorer
                    .tile_unit(tile)
                    .map_or_else(String::new, |u| explorer.unit_name(u));
                let mut row = egui::text::LayoutJob::default();
                let format = |color, size| egui::TextFormat {
                    color,
                    font_id: egui::FontId::proportional(size),
                    ..Default::default()
                };
                row.append(&format!("{:>2} ", index + 1), 0.0, format(MUTED, 12.0));
                row.append(
                    &format!("{} {name}", sign(change)),
                    0.0,
                    format(egui_color(change_color(change)), 13.5),
                );
                let reach = view.reached_from(tile).len();
                if reach > 0 {
                    row.append(&format!("↑{reach}"), 8.0, format(MUTED, 12.0));
                }
                let response = ui.selectable_label(focus == Some(tile), row).on_hover_text(
                    explorer
                        .tile_unit(tile)
                        .map_or_else(String::new, ToString::to_string)
                        + &format!("\naffects {reach} entry points"),
                );
                if focus == Some(tile) && moved {
                    response.scroll_to_me(Some(egui::Align::Center));
                }
                if response.clicked() {
                    chosen = Some(tile);
                }
            }
        });
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_runs_through_the_changes_callees_first() {
        // main -> (a -> c), b; a and c rewritten.
        let mut explorer = crate::explorer::tests::explorer();
        let id = |name: &str| codeviz_ir::UnitId::new(format!("x:{name}"));
        let diff = Diff {
            units: HashMap::from([(id("a"), Change::Modified), (id("c"), Change::Modified)]),
            ..Diff::default()
        };
        let before = explorer.system().clone();
        let mut view = DiffView::new(&mut explorer, &diff, before);
        let tile = |name: &str| explorer.map().tile_of(&id(name)).unwrap();
        let (a, b, c) = (tile("a"), tile("b"), tile("c"));
        assert_eq!(view.order(), [c, a], "c before a, which calls it");
        assert_eq!(view.step_from(None, 1), Some(c), "the first to read");
        assert_eq!(view.step_from(Some(c), 1), Some(a));
        assert_eq!(view.step_from(Some(a), 1), Some(a), "it stays at the end");
        assert_eq!(view.step_from(Some(a), -1), Some(c));
        assert_eq!(
            view.step_from(Some(b), 1),
            Some(a),
            "from elsewhere, on from where stepping left off"
        );
        assert_eq!(view.place(a), Some(1));
        assert_eq!(view.place(b), None);
    }

    #[test]
    fn the_way_up_runs_to_the_nearest_starts_along_shortest_chains() {
        // 0 is the change; 1 and 2 call it, 3 calls 1 and 2; 4 calls 3; 5 calls 2.
        let mut callers = vec![Vec::new(); 6];
        callers[0] = vec![1, 2];
        callers[1] = vec![3];
        callers[2] = vec![3, 5];
        callers[3] = vec![4];
        let starts = [4, 5];
        let route = route(0, &callers, |t| starts.contains(&t), 8);
        assert_eq!(route.ends, [5, 4], "nearest first");
        assert_eq!(
            route.calls,
            HashSet::from([(5, 2), (2, 0), (4, 3), (3, 1), (1, 0)]),
            "one chain each, the first found"
        );
        assert_eq!(route.tiles, HashSet::from([0, 1, 2, 3, 4, 5]));
        let nearest = super::route(0, &callers, |t| starts.contains(&t), 1);
        assert_eq!(nearest.ends, [5]);
        assert_eq!(nearest.tiles, HashSet::from([0, 2, 5]));
    }

    #[test]
    fn the_morph_holds_at_each_end() {
        assert!(cycle(0.5).abs() < f32::EPSILON);
        assert!((cycle(HOLD + MOVE + 0.5) - 1.0).abs() < f32::EPSILON);
        let midway = cycle(HOLD + MOVE / 2.0);
        assert!((midway - 0.5).abs() < 1e-4);
        assert!(
            cycle(2.0 * (HOLD + MOVE) + 0.1).abs() < f32::EPSILON,
            "it loops"
        );
    }
}
