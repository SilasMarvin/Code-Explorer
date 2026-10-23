//! A diff over the graph, drawn three ways. The graph lays out both versions at once,
//! so every unit either side has keeps one place.
//!
//! - Status: added units glow green, rewritten ones gold, removed ones linger as red
//!   ghosts; calls that appeared are green and calls that went away red.
//! - Impact: the blast radius. Everything that can reach a change heats up, hottest
//!   nearest it, and pulses run from each change out along its callers to the entry
//!   points it can affect.
//! - Morph: the graph as it was, then as it is. Each version is laid out on its own,
//!   starting from the shared layout, and the graph moves between the two: removed
//!   units shrink away, added ones grow in, and the rest shift as the calls between
//!   them change.

use std::collections::{BTreeMap, HashMap, HashSet};

use bevy::prelude::*;
use bevy_egui::{egui, input::egui_wants_any_keyboard_input};
use codeviz_core::{Change, Diff, System};
use codeviz_layout::ForceGraph;

use crate::{
    camera::OrbitCamera,
    explorer::{Clustering, Explorer, Side},
    graph_view::{self, NodeLook},
    lens::{Lens, SPECK},
    routes,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DiffStyle {
    #[default]
    Status,
    Impact,
    Morph,
}

impl DiffStyle {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Status => Self::Impact,
            Self::Impact => Self::Morph,
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

/// An entry point a change can reach.
#[derive(Debug, Clone)]
pub(crate) struct Affected {
    pub(crate) tile: usize,
    pub(crate) label: String,
    /// Calls between it and the nearest change.
    pub(crate) hops: u32,
}

#[derive(Resource, Debug)]
pub(crate) struct DiffView {
    pub(crate) style: DiffStyle,
    pub(crate) totals: Totals,
    changes: HashMap<usize, Change>,
    added_calls: HashSet<(usize, usize)>,
    removed_calls: HashSet<(usize, usize)>,
    /// Calls between each node and the nearest change it can reach; zero for changes.
    impact: HashMap<usize, u32>,
    pub(crate) affected: Vec<Affected>,
    pub(crate) entry_count: usize,
    /// Whether the entries are the functions nothing calls, for want of declared ones.
    pub(crate) roots: bool,
    /// The old version, with moved functions under their new ids.
    old: System,
    /// What each moved node was called before.
    moved: HashMap<usize, String>,
    /// Where each node sits in the old version, the new one, and both together.
    before: Vec<[f32; 3]>,
    after: Vec<[f32; 3]>,
    shared: Vec<[f32; 3]>,
    /// Calls in and out of each node, by tile.
    degrees: Vec<usize>,
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

        // Out from every change along its callers, nearest first.
        let graph = explorer.graph();
        let starts: Vec<usize> = changes.keys().copied().collect();
        let impact = routes::walk(graph, &starts, Side::Callers, None, |_| false);
        let (entries, roots) = entry_points(explorer, &changes);
        let mut affected: Vec<Affected> = entries
            .iter()
            .filter_map(|(tile, label)| {
                Some(Affected {
                    tile: *tile,
                    label: label.clone(),
                    hops: *impact.get(tile)?,
                })
            })
            .collect();
        // What a change reaches comes before what changed itself, nearest first.
        affected
            .sort_by(|a, b| (a.hops == 0, a.hops, &a.label).cmp(&(b.hops == 0, b.hops, &b.label)));

        let (before_at, after_at) = versions(explorer, &before, &changes);
        Self {
            style: DiffStyle::default(),
            totals,
            changes,
            added_calls,
            removed_calls,
            impact,
            affected,
            entry_count: entries.len(),
            roots,
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
            degrees: (0..map.tiles.len()).map(|t| graph.degree(t)).collect(),
            morph: 0.0,
            playing: true,
            clock: 0.0,
        }
    }

    /// The version the diff starts from.
    pub(crate) fn old(&self) -> &System {
        &self.old
    }

    /// The change most tied into the rest: the one with the most calls in and out,
    /// to open on.
    pub(crate) fn hub(&self) -> Option<usize> {
        self.changes
            .iter()
            .filter(|&(_, &change)| change != Change::Removed)
            .max_by_key(|&(&tile, _)| {
                (
                    self.degrees.get(tile).copied().unwrap_or(0),
                    std::cmp::Reverse(tile),
                )
            })
            .map(|(&tile, _)| tile)
    }

    /// What a moved node was called before.
    pub(crate) fn moved_from(&self, tile: usize) -> Option<&str> {
        self.moved.get(&tile).map(String::as_str)
    }

    pub(crate) fn change(&self, tile: usize) -> Option<Change> {
        self.changes.get(&tile).copied()
    }

    /// Calls from `tile` to the nearest change, when it can reach one.
    pub(crate) fn impact(&self, tile: usize) -> Option<u32> {
        self.impact.get(&tile).copied()
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

    /// How much of the "after" version is showing, eased.
    pub(crate) fn progress(&self) -> f32 {
        match self.style {
            DiffStyle::Morph => ease(self.morph),
            DiffStyle::Status | DiffStyle::Impact => 1.0,
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

/// Where things start, by node and name: the declared entries, or else whatever
/// nothing calls (`main`, a framework's callbacks, a library's API). Says which.
fn entry_points(
    explorer: &Explorer,
    changes: &HashMap<usize, Change>,
) -> (Vec<(usize, String)>, bool) {
    let declared = explorer.entries();
    if !declared.is_empty() {
        let mut entries: Vec<(usize, String)> = declared
            .iter()
            .map(|(&t, label)| (t, label.clone()))
            .collect();
        entries.sort();
        return (entries, false);
    }
    let graph = explorer.graph();
    let roots = (0..explorer.map().tiles.len())
        .filter(|&t| graph.callers(t).next().is_none())
        .filter(|&t| changes.get(&t) != Some(&Change::Removed))
        .filter_map(|t| Some((t, explorer.unit_name(explorer.tile_unit(t)?))))
        .collect();
    (roots, true)
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

/// D cycles the styles; T pauses or resumes the morph; C shows only what changed, or
/// everything again.
fn diff_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut view: ResMut<DiffView>,
    mut explorer: ResMut<Explorer>,
) {
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

/// How a node looks under the diff: its change, or how near a change it is, or faded.
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
    match (view.style, view.impact(tile)) {
        (DiffStyle::Impact, Some(hops)) => NodeLook::Impacted(hops.min(3)),
        // Near the focus but left alone: neutral, so where it sits still reads.
        _ => explorer
            .reach()
            .and_then(|reach| graph_view::near(explorer, reach, tile))
            .map_or(
                NodeLook::Faded(explorer.tile_color(tile) % PALETTE.len()),
                NodeLook::Unchanged,
            ),
    }
}

/// Every call, colored by the diff. Calls touching the center of attention say which
/// way they run, violet in and blue out, with dots flowing along them; otherwise added
/// calls are green and removed ones red and dashed; under Impact, calls that carry a
/// change towards an entry point are gold, with pulses running out along them; the
/// spotlight's routes and the journey's path are pale arrows; the rest are faint.
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
    let routes = graph_view::Routes::of(explorer);
    let following = explorer.followed_start().is_some();
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
            _ if let Some(hops) = routes.as_ref().and_then(|r| r.call(from, to)) => {
                let followed = following.then(|| path.contains(&(from, to)));
                let alpha = graph_view::route_strength(hops, shown, followed);
                graph_view::draw_route(gizmos, (a, b - back), alpha, time, index);
            }
            _ => {
                let carries = view.style == DiffStyle::Impact
                    && view
                        .impact(from)
                        .zip(view.impact(to))
                        .is_some_and(|(caller, callee)| caller == callee + 1);
                if carries {
                    let hops = view.impact(from).unwrap_or(1);
                    let strength = match hops {
                        1 => 0.8,
                        2 => 0.55,
                        _ => 0.35,
                    };
                    gizmos.line(a, b, MODIFIED.with_alpha(strength * shown));
                    // A pulse running from the callee out to its caller.
                    let phase = (time * 0.7 + (from * 7 + to * 13) as f32 * 0.137).fract();
                    gizmos.sphere(
                        Isometry3d::from_translation(b.lerp(a, phase)),
                        0.22,
                        MODIFIED.with_alpha((strength + 0.2) * shown),
                    );
                } else if path.contains(&(from, to)) {
                    gizmos
                        .arrow(a, b - back, graph_view::FURTHER.with_alpha(0.45 * shown))
                        .with_tip_length(0.8);
                } else {
                    gizmos.line(a, b, Color::srgba(0.6, 0.66, 0.8, 0.07 * shown));
                }
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

/// What changed, how it is drawn, and what it reaches: top right of the graph.
/// Rows focus their node.
pub(crate) fn diff_card(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &mut Explorer,
    view: &mut DiffView,
) {
    let mut chosen = None;
    let mut style = view.style;
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
                    ui.label(egui::RichText::new("Changes").strong().size(15.0));
                    totals_line(ui, view.totals);
                    let mut lens = explorer.lens();
                    if ui
                        .checkbox(&mut lens.changes_only, "Only changes")
                        .on_hover_text(
                            "Hide what did not change, but for the focus and the journey's root (C)",
                        )
                        .changed()
                    {
                        explorer.set_lens(lens);
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut style, DiffStyle::Status, "Status")
                            .on_hover_text("What changed, and the calls that came and went (D)");
                        ui.selectable_value(&mut style, DiffStyle::Impact, "Impact")
                            .on_hover_text("Everything that can reach a change (D)");
                        ui.selectable_value(&mut style, DiffStyle::Morph, "Morph")
                            .on_hover_text("The graph before, then after (D)");
                    });
                    match style {
                        DiffStyle::Status => {
                            chosen = status_list(ui, explorer, view, area.height() * 0.55);
                        }
                        DiffStyle::Impact => {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Reaches {} of {} {}",
                                    view.affected.len(),
                                    view.entry_count,
                                    if view.roots {
                                        "functions nothing calls"
                                    } else {
                                        "entry points"
                                    }
                                ))
                                .small()
                                .color(MUTED),
                            );
                            chosen = impact_list(ui, view, area.height() * 0.5);
                        }
                        DiffStyle::Morph => {
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

/// The entry points a change reaches, nearest first, in a list that scrolls past
/// `max_height`. Returns the node of a row clicked.
fn impact_list(ui: &mut egui::Ui, view: &DiffView, max_height: f32) -> Option<usize> {
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .max_height(max_height)
        .show(ui, |ui| {
            for affected in &view.affected {
                let note = match affected.hops {
                    0 => "changed itself".to_owned(),
                    1 => "calls a change".to_owned(),
                    n => format!("{n} calls from a change"),
                };
                let text = egui::RichText::new(format!("{}  · {note}", affected.label))
                    .color(egui_color(MODIFIED));
                if ui
                    .add(egui::Label::new(text).sense(egui::Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    chosen = Some(affected.tile);
                }
            }
        });
    chosen
}

/// Every change, grouped by container, in a list that scrolls past `max_height`.
/// Returns the node of a row clicked.
fn status_list(
    ui: &mut egui::Ui,
    explorer: &Explorer,
    view: &DiffView,
    max_height: f32,
) -> Option<usize> {
    let map = explorer.map();
    let mut groups: BTreeMap<&str, Vec<(Change, String, usize)>> = BTreeMap::new();
    for (tile, change) in view.changed() {
        let group = map
            .tile(tile)
            .and_then(|t| map.groups.get(t.group))
            .map_or("", |g| g.label.as_str());
        let name = explorer
            .tile_unit(tile)
            .and_then(|u| explorer.any_unit(u))
            .map_or_else(String::new, |u| u.name.clone());
        groups.entry(group).or_default().push((change, name, tile));
    }
    let open = groups.len() <= 6;
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .max_height(max_height)
        .show(ui, |ui| {
            for (label, rows) in &mut groups {
                rows.sort();
                let mut header = egui::text::LayoutJob::default();
                let format = |color| egui::TextFormat {
                    color,
                    font_id: egui::FontId::proportional(13.5),
                    ..Default::default()
                };
                header.append(label, 0.0, format(egui::Color32::from_rgb(215, 220, 230)));
                for change in [
                    Change::Added,
                    Change::Modified,
                    Change::Moved,
                    Change::Removed,
                ] {
                    let n = rows.iter().filter(|r| r.0 == change).count();
                    if n > 0 {
                        let text = format!("{}{n}", sign(change));
                        header.append(&text, 8.0, format(egui_color(change_color(change))));
                    }
                }
                egui::CollapsingHeader::new(header)
                    .id_salt(*label)
                    .default_open(open || rows.len() <= 3)
                    .show(ui, |ui| {
                        for (change, name, tile) in rows.iter() {
                            let text = egui::RichText::new(format!("{}  {name}", sign(*change)))
                                .color(egui_color(change_color(*change)));
                            if ui
                                .add(egui::Label::new(text).sense(egui::Sense::click()))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                chosen = Some(*tile);
                            }
                        }
                    });
            }
        });
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

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
