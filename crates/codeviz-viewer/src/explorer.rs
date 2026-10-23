//! Viewer state that is independent of rendering: the loaded system, the open
//! journeys and their cursors, and the codebase as a graph of calls with the node at
//! the center of attention and what reaches it.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use bevy::prelude::*;
use codeviz_core::{
    Journey, JourneyOptions, Level, Scenario, Span, SpanId, SpanTarget, StopReason, System,
    display_name, scenarios,
};
use codeviz_ir::{Certainty, ExitKind, Location, Mode, Unit, UnitId};
use codeviz_layout::{CodebaseMap, ForceConfig, ForceGraph, LayoutConfig, MapConfig, Reach, Scene};

use crate::{
    disclosure::{Disclose, Disclosed},
    lens::{self, Lens},
    starts::{self, Start},
};

/// What a disclosure was worked out for: the journeys' revision, the active one, how
/// it discloses, the spans opened by hand, and the cursor when that matters.
type DisclosureKey = (u64, usize, Disclose, u64, Option<SpanId>);

/// How sure we are that a span really happens as drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confidence {
    /// Resolved by the producer's rules, declared, or observed.
    Known,
    /// Matched by name, ambiguous, or unresolved.
    Guessed,
}

impl Confidence {
    pub(crate) fn of(span: &Span) -> Self {
        match (&span.target, span.certainty) {
            (SpanTarget::Unit(_), Some(Certainty::Inferred)) => Self::Guessed,
            (SpanTarget::Unit(_), _) => Self::Known,
            _ => Self::Guessed,
        }
    }
}

/// A place in the happy path where another outcome splits off.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Divergence {
    pub(crate) span: SpanId,
    pub(crate) scenario: usize,
    pub(crate) kind: ExitKind,
    pub(crate) label: String,
}

/// A span in one of the open journeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpanRef {
    pub(crate) track: usize,
    pub(crate) span: SpanId,
}

/// One open journey: where it starts, which outcome it shows, its own cursor, and its
/// layout in time and depth.
#[derive(Debug, Default)]
pub(crate) struct Track {
    root: Option<UnitId>,
    history: Vec<UnitId>,
    scenarios: Vec<Scenario>,
    scenario: Option<usize>,
    journey: Option<Journey>,
    scene: Scene,
    divergences: Vec<Divergence>,
    selected: Option<SpanId>,
    /// The color of each of the scene's lanes, shared with the other journeys.
    colors: Vec<usize>,
}

impl Track {
    pub(crate) fn root(&self) -> Option<&UnitId> {
        self.root.as_ref()
    }

    pub(crate) fn journey(&self) -> Option<&Journey> {
        self.journey.as_ref()
    }

    pub(crate) fn scene(&self) -> &Scene {
        &self.scene
    }

    pub(crate) fn divergences(&self) -> &[Divergence] {
        &self.divergences
    }

    pub(crate) fn selected(&self) -> Option<SpanId> {
        self.selected
    }

    pub(crate) fn span(&self, id: SpanId) -> Option<&Span> {
        self.journey.as_ref()?.span(id)
    }

    /// The unit a span runs, if it resolved to exactly one.
    pub(crate) fn unit_id(&self, id: SpanId) -> Option<&UnitId> {
        match &self.span(id)?.target {
            SpanTarget::Unit(unit) => Some(unit),
            _ => None,
        }
    }

    /// The color index for a span's bar: the same module has the same color in
    /// every journey.
    pub(crate) fn color(&self, id: SpanId) -> usize {
        self.scene
            .nodes
            .get(id.index())
            .and_then(|placed| self.colors.get(placed.lane))
            .copied()
            .unwrap_or_default()
    }

    /// The first span that runs `unit`.
    pub(crate) fn find(&self, unit: &UnitId) -> Option<SpanId> {
        self.journey.as_ref().and_then(|j| {
            j.spans()
                .find(|(_, s)| s.target == SpanTarget::Unit(unit.clone()))
                .map(|(id, _)| id)
        })
    }

    fn reroot(&mut self, system: &System, options: &JourneyOptions, layout: &LayoutConfig) {
        self.scenarios = self
            .root
            .as_ref()
            .map(|root| scenarios(system, root, &options.zoom))
            .unwrap_or_default();
        self.scenario = (!self.scenarios.is_empty()).then_some(0);
        self.rebuild(system, options, layout);
    }

    fn rebuild(&mut self, system: &System, options: &JourneyOptions, layout: &LayoutConfig) {
        let scenario = self.scenario.and_then(|i| self.scenarios.get(i));
        self.journey = self
            .root
            .as_ref()
            .and_then(|root| Journey::unroll(system, root, scenario, options.clone()));
        self.scene = self
            .journey
            .as_ref()
            .map(|journey| codeviz_layout::layout(system, journey, layout))
            .unwrap_or_default();
        self.divergences = self.find_divergences();
        self.selected = self.journey.as_ref().map(Journey::root);
    }

    /// On the happy path, where each failing outcome splits off: the deepest unit on
    /// its chain that the happy path also runs.
    fn find_divergences(&self) -> Vec<Divergence> {
        let (Some(journey), Some(0)) = (&self.journey, self.scenario) else {
            return Vec::new();
        };
        self.scenarios
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(index, scenario)| {
                let span = scenario.chain.iter().rev().find_map(|site| {
                    journey
                        .spans()
                        .find(|(_, s)| s.target == SpanTarget::Unit(site.unit.clone()))
                        .map(|(id, _)| id)
                })?;
                Some(Divergence {
                    span,
                    scenario: index,
                    kind: scenario.outcome.kind,
                    label: scenario.outcome.label.clone(),
                })
            })
            .collect()
    }
}

/// How the graph gathers its nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Clustering {
    /// Functions in the same container pull together.
    #[default]
    Modules,
    /// Only calls pull, so functions gather with whatever they talk to. Containers
    /// that stay together anyway call mostly among themselves.
    Calls,
}

impl Clustering {
    pub(crate) fn forces(self) -> ForceConfig {
        // Spread out, so names have room.
        let spread = ForceConfig {
            repulsion: 50.0,
            link_length: 8.0,
            // Mostly flat, so clusters sit side by side like regions on a map.
            flatten: 0.1,
            ..ForceConfig::default()
        };
        match self {
            // Each container a cluster of its own, with clear space between them.
            Self::Modules => ForceConfig {
                cohesion: 0.15,
                separation: SEPARATION,
                rooms: true,
                containment: 0.3,
                crate_cohesion: 0.15,
                cross_spring: 0.3,
                ..spread
            },
            Self::Calls => ForceConfig {
                cohesion: 0.0,
                ..spread
            },
        }
    }
}

/// How hard crowding containers push apart when clustered by module.
const SEPARATION: f32 = 0.4;

/// Simulation steps run before the graph is first shown, so it opens nearly settled,
/// unless they take longer than [`PRESETTLE_TIME`]: a big graph opens sooner and
/// settles in view.
const PRESETTLE_STEPS: usize = 200;
const PRESETTLE_TIME: Duration = Duration::from_millis(600);
/// Steps a frame takes at most while the graph settles.
const MAX_FRAME_STEPS: usize = 8;
/// At most this many steps to settle at once.
const SETTLE_NOW_STEPS: usize = 600;
/// Below this mean speed the graph counts as settled and stops simulating.
const SETTLED_SPEED: f32 = 0.002;

/// Either side of the focus: what calls it, or what it calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Callers,
    Callees,
}

#[derive(Resource, Debug)]
pub(crate) struct Explorer {
    system: System,
    /// What the graph lays out, when wider than `system`.
    layout_system: Option<System>,
    pub(crate) options: JourneyOptions,
    pub(crate) layout: LayoutConfig,
    /// Open journeys.
    tracks: Vec<Track>,
    /// The journey the cursor keys, the panel and the outline follow.
    active: usize,
    pub(crate) hovered: Option<SpanRef>,
    /// Every function, grouped by container; graph nodes are its tiles.
    map: CodebaseMap,
    /// The codebase as a graph of calls.
    graph: ForceGraph,
    clustering: Clustering,
    /// Whether the graph is still moving.
    settling: bool,
    /// A node focused in the graph that the active journey does not run, until the
    /// cursor moves.
    graph_focus: Option<usize>,
    /// Map tiles each journey runs, with the color each is drawn in there.
    journey_tiles: Vec<HashMap<usize, usize>>,
    /// What reaches the node at the center of attention, and what it reaches.
    reach: Option<(usize, Reach)>,
    /// The same for the focused node.
    focus_reach: Option<(usize, Reach)>,
    /// How much of the graph shows.
    lens: Lens,
    /// How fully each node shows, by tile, eased towards what the lens says.
    visibility: Vec<f32>,
    /// A diff's changed nodes, for a lens that starts from them.
    changed_tiles: Vec<usize>,
    /// How much of the active journey shows.
    disclose: Disclose,
    /// Spans opened one level past the disclosed depth, by hand.
    expanded: HashSet<SpanId>,
    /// Bumped whenever `expanded` changes.
    disclosure_epoch: u64,
    disclosure_cache: Mutex<Option<(DisclosureKey, Arc<Disclosed>)>>,
    /// Nodes focused before the current one, most recent last.
    focus_history: Vec<usize>,
    pub(crate) hovered_tile: Option<usize>,
    /// What calls the focus, or what it calls, picked out and brought into view, and
    /// when, in seconds of app time.
    spotlight: Option<(Side, f32)>,
    /// Lane label to color index, in order of first appearance across journeys.
    lane_colors: HashMap<String, usize>,
    /// Bumped whenever a journey is rebuilt.
    revision: u64,
    /// How long the graph took to lay out before it was first shown.
    laid_out: Duration,
    /// Where to start exploring, best first.
    starts: Vec<Start>,
    /// The nodes of declared entry points.
    entry_tiles: HashSet<usize>,
    /// Every node, the most calls in and out first.
    busiest: Vec<usize>,
}

impl Explorer {
    #[cfg(test)]
    pub(crate) fn new(system: System, root: Option<UnitId>, options: JourneyOptions) -> Self {
        Self::with_layout(system, None, root, options)
    }

    /// Journeys run over `system`; the graph lays out `layout` when given, a wider
    /// system such as both sides of a diff, so it can place units `system` lacks.
    pub(crate) fn with_layout(
        system: System,
        layout: Option<System>,
        root: Option<UnitId>,
        options: JourneyOptions,
    ) -> Self {
        let placed = layout.as_ref().unwrap_or(&system);
        let map = codeviz_layout::map(placed, &MapConfig::default());
        let clustering = Clustering::default();
        let started = Instant::now();
        let mut graph = ForceGraph::new(placed, &map, clustering.forces());
        for _ in 0..PRESETTLE_STEPS {
            graph.step();
            if started.elapsed() >= PRESETTLE_TIME {
                break;
            }
        }
        let laid_out = started.elapsed();
        let starts = starts::starts(placed, &map, &graph);
        let entry_tiles = placed
            .entries()
            .iter()
            .filter_map(|e| map.tile_of(&e.unit))
            .collect();
        let mut busiest: Vec<usize> = (0..map.tiles.len()).collect();
        busiest.sort_by_key(|&t| (std::cmp::Reverse(graph.degree(t)), t));
        let tiles = map.tiles.len();
        let mut explorer = Self {
            system,
            layout_system: layout,
            options,
            layout: LayoutConfig::default(),
            tracks: vec![Track::default()],
            active: 0,
            hovered: None,
            map,
            graph,
            clustering,
            settling: true,
            graph_focus: None,
            journey_tiles: Vec::new(),
            reach: None,
            focus_reach: None,
            lens: Lens::default(),
            visibility: vec![1.0; tiles],
            changed_tiles: Vec::new(),
            focus_history: Vec::new(),
            disclose: Disclose::default(),
            expanded: HashSet::new(),
            disclosure_epoch: 0,
            disclosure_cache: Mutex::new(None),
            hovered_tile: None,
            spotlight: None,
            lane_colors: HashMap::new(),
            revision: 0,
            laid_out,
            starts,
            entry_tiles,
            busiest,
        };
        explorer.color_groups();
        if let Some(root) = root {
            explorer.set_root(root);
            explorer.track_mut().history.clear();
        }
        explorer
    }

    pub(crate) fn system(&self) -> &System {
        &self.system
    }

    /// A unit of the journeys' system, or else of the wider one the graph lays out.
    pub(crate) fn any_unit(&self, id: &UnitId) -> Option<&Unit> {
        self.system
            .unit(id)
            .or_else(|| self.layout_system.as_ref()?.unit(id))
    }

    /// Moves every node to a given place, and stops the simulation moving them.
    pub(crate) fn place_nodes(&mut self, positions: &[[f32; 3]]) {
        self.graph.positions.clone_from_slice(positions);
        self.settling = false;
    }

    /// Calls the graph still lays out but that no longer run, as a diff's removed
    /// calls: they stop counting as callers and callees.
    pub(crate) fn retire_calls(&mut self, gone: &HashSet<(usize, usize)>) {
        self.graph.retire(gone);
        self.reach = None;
        self.focus_reach = None;
    }

    pub(crate) fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub(crate) fn active(&self) -> usize {
        self.active
    }

    /// The active journey.
    pub(crate) fn track(&self) -> &Track {
        // `tracks` is never empty.
        &self.tracks[self.active.min(self.tracks.len() - 1)]
    }

    fn track_mut(&mut self) -> &mut Track {
        let index = self.active.min(self.tracks.len() - 1);
        &mut self.tracks[index]
    }

    pub(crate) fn root(&self) -> Option<&UnitId> {
        self.track().root()
    }

    pub(crate) fn journey(&self) -> Option<&Journey> {
        self.track().journey()
    }

    pub(crate) fn scenarios(&self) -> &[Scenario] {
        &self.track().scenarios
    }

    /// The outcome shown; `None` shows every path at once.
    pub(crate) fn scenario(&self) -> Option<usize> {
        self.track().scenario
    }

    pub(crate) fn divergences(&self) -> &[Divergence] {
        self.track().divergences()
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn can_go_back(&self) -> bool {
        !self.track().history.is_empty()
    }

    /// The span the active journey's cursor is on.
    pub(crate) fn selected(&self) -> Option<SpanId> {
        self.track().selected
    }

    /// Moves the active cursor. The graph's focus follows it again.
    pub(crate) fn select(&mut self, span: Option<SpanId>) {
        self.track_mut().selected = span;
        self.graph_focus = None;
    }

    /// Makes `track` active with its cursor on `span`.
    pub(crate) fn select_in(&mut self, at: SpanRef) {
        if at.track < self.tracks.len() {
            if self.active != at.track {
                self.active = at.track;
                self.refresh_journey_tiles();
            }
            self.select(Some(at.span));
        }
    }

    pub(crate) fn set_root(&mut self, root: UnitId) {
        if self.root() == Some(&root) {
            return;
        }
        let track = self.track_mut();
        if let Some(previous) = track.root.replace(root) {
            track.history.push(previous);
        }
        self.reroot();
    }

    pub(crate) fn back(&mut self) {
        if let Some(previous) = self.track_mut().history.pop() {
            self.track_mut().root = Some(previous);
            self.reroot();
        }
    }

    /// Recomputes outcomes for the active journey's root and opens on the happy path,
    /// with the cursor and the graph's focus on the root.
    pub(crate) fn reroot(&mut self) {
        let index = self.active.min(self.tracks.len() - 1);
        if let Some(track) = self.tracks.get_mut(index) {
            track.reroot(&self.system, &self.options, &self.layout);
        }
        self.graph_focus = None;
        self.recolor();
    }

    pub(crate) fn set_scenario(&mut self, scenario: Option<usize>) {
        if self.scenario() != scenario {
            self.track_mut().scenario = scenario;
            self.rebuild();
            // Land on the exit the outcome ends at.
            if scenario.is_some_and(|s| s > 0) {
                let exit = self.journey().and_then(|j| {
                    j.spans()
                        .filter(|(_, s)| s.exits.iter().any(|e| e.kind != ExitKind::Return))
                        .map(|(id, _)| id)
                        .last()
                        .or_else(|| j.spans().last().map(|(id, _)| id))
                });
                self.select(exit);
            }
        }
    }

    /// Re-roots the active journey at the selected span's unit.
    pub(crate) fn trace_from_selected(&mut self) {
        if let Some(id) = self.focus().cloned() {
            self.set_root(id);
        }
    }

    pub(crate) fn toggle_zoom(&mut self, unit: UnitId) {
        let level = match self.options.zoom.level(&unit) {
            Level::Summary => Level::Detail,
            Level::Detail => Level::Summary,
        };
        self.options.zoom.set(unit, level);
        for track in &mut self.tracks {
            track.reroot(&self.system, &self.options, &self.layout);
        }
        self.recolor();
    }

    /// Rebuilds the active journey.
    pub(crate) fn rebuild(&mut self) {
        let index = self.active.min(self.tracks.len() - 1);
        if let Some(track) = self.tracks.get_mut(index) {
            track.rebuild(&self.system, &self.options, &self.layout);
        }
        self.recolor();
    }

    /// The codebase as a graph of calls.
    pub(crate) fn graph(&self) -> &ForceGraph {
        &self.graph
    }

    /// Where a graph node sits in the world.
    pub(crate) fn tile_point(&self, tile: usize) -> Option<Vec3> {
        self.graph.positions.get(tile).map(|&p| Vec3::from(p))
    }

    pub(crate) fn clustering(&self) -> Clustering {
        self.clustering
    }

    /// Regathers the graph; it moves until it settles.
    pub(crate) fn set_clustering(&mut self, clustering: Clustering) {
        if self.clustering != clustering {
            self.clustering = clustering;
            self.graph.set_config(clustering.forces());
            self.settling = true;
        }
    }

    /// How long the graph took to lay out before it was first shown.
    pub(crate) fn laid_out(&self) -> Duration {
        self.laid_out
    }

    /// Where to start exploring, best first.
    pub(crate) fn starts(&self) -> &[Start] {
        &self.starts
    }

    /// Whether a node is a declared entry point.
    pub(crate) fn is_entry(&self, tile: usize) -> bool {
        self.entry_tiles.contains(&tile)
    }

    /// Every node, the most calls in and out first.
    pub(crate) fn busiest(&self) -> &[usize] {
        &self.busiest
    }

    /// Opens the best place to start, if there is one. Returns whether there was.
    pub(crate) fn open_best_start(&mut self) -> bool {
        let Some(tile) = self.starts.first().map(|s| s.tile) else {
            return false;
        };
        self.open_at(tile);
        true
    }

    /// Starts the active journey at the best place to start, without focusing it.
    pub(crate) fn root_at_best_start(&mut self) {
        if let Some(unit) = self
            .starts
            .first()
            .and_then(|s| self.tile_unit(s.tile))
            .cloned()
        {
            self.set_root(unit);
            self.track_mut().history.clear();
        }
    }

    /// Whether the graph is still moving.
    pub(crate) fn settling(&self) -> bool {
        self.settling
    }

    /// Runs the graph simulation while it is still moving, as many steps as fit in
    /// `budget` but at least one, so a big graph settles without stalling the frame.
    /// Returns whether anything moved.
    pub(crate) fn simulate(&mut self, budget: Duration) -> bool {
        if !self.settling {
            return false;
        }
        let started = Instant::now();
        let mut speed = f32::MAX;
        for _ in 0..MAX_FRAME_STEPS {
            speed = self.graph.step();
            if speed <= SETTLED_SPEED || started.elapsed() >= budget {
                break;
            }
        }
        self.settling = speed > SETTLED_SPEED;
        true
    }

    /// Runs the simulation until the graph settles, all at once.
    pub(crate) fn settle_now(&mut self) {
        for _ in 0..SETTLE_NOW_STEPS {
            if self.graph.step() < SETTLED_SPEED {
                break;
            }
        }
        self.settling = false;
    }

    /// Gives lanes and containers their colors after the journeys changed, and asks
    /// for a redraw.
    fn recolor(&mut self) {
        for track in &mut self.tracks {
            let colors = &mut self.lane_colors;
            track.colors = track
                .scene
                .lanes
                .iter()
                .map(|lane| {
                    let next = colors.len();
                    *colors.entry(lane.label.clone()).or_insert(next)
                })
                .collect();
        }
        self.color_groups();
        self.refresh_journey_tiles();
        self.hovered = None;
        self.revision += 1;
    }

    /// Opens another journey from `unit` and makes it active. Its cursor lands on the
    /// unit the active cursor was on, when it reaches it.
    pub(crate) fn add_track(&mut self, unit: UnitId) {
        let focus = self.focus().cloned();
        let mut track = Track {
            root: Some(unit),
            ..Track::default()
        };
        track.reroot(&self.system, &self.options, &self.layout);
        if let Some(landing) = focus.and_then(|f| track.find(&f)) {
            track.selected = Some(landing);
        }
        self.tracks.push(track);
        self.active = self.tracks.len() - 1;
        self.graph_focus = None;
        self.recolor();
    }

    /// Opens the selected span's unit as another journey.
    pub(crate) fn pull_out_selected(&mut self) {
        if let Some(unit) = self.focus().cloned() {
            self.add_track(unit);
        }
    }

    /// Closes a journey; the last one stays open.
    pub(crate) fn close_track(&mut self, index: usize) {
        if self.tracks.len() > 1 && index < self.tracks.len() {
            self.tracks.remove(index);
            if self.active >= index && self.active > 0 {
                self.active -= 1;
            }
            self.recolor();
        }
    }

    /// The node the graph centers on and lights up around: one clicked in the graph,
    /// else the function under the cursor.
    pub(crate) fn graph_focus(&self) -> Option<usize> {
        self.graph_focus
            .or_else(|| self.focus().and_then(|f| self.map.tile_of(f)))
    }

    /// The node the camera orbits: the one focused by hand, else where the active
    /// journey starts, else its cursor. Stepping through a journey leaves it put.
    pub(crate) fn anchor(&self) -> Option<usize> {
        self.graph_focus
            .or_else(|| self.root().and_then(|u| self.map.tile_of(u)))
            .or_else(|| self.graph_focus())
    }

    /// Centers on a node and opens its journey: the active journey restarts there, with
    /// what it calls directly disclosed and the rest folded away.
    pub(crate) fn open_at(&mut self, tile: usize) {
        if let Some(previous) = self.anchor().filter(|&p| p != tile) {
            self.focus_history.push(previous);
        }
        self.restart_at(tile);
    }

    fn restart_at(&mut self, tile: usize) {
        if let Some(unit) = self.tile_unit(tile).cloned() {
            self.set_root(unit);
        }
        self.graph_focus = Some(tile);
        self.spotlight = None;
        self.expanded.clear();
        self.disclosure_epoch += 1;
        if let Disclose::Depth(_) = self.disclose {
            self.disclose = Disclose::default();
        }
    }

    pub(crate) fn disclose(&self) -> Disclose {
        self.disclose
    }

    pub(crate) fn set_disclose(&mut self, disclose: Disclose) {
        self.disclose = disclose;
    }

    /// Opens or folds the calls under one span, past the disclosed depth.
    pub(crate) fn toggle_expanded(&mut self, span: SpanId) {
        if !self.expanded.remove(&span) {
            self.expanded.insert(span);
        }
        self.disclosure_epoch += 1;
    }

    /// What the active journey discloses, worked out once per change.
    pub(crate) fn disclosure(&self) -> Arc<Disclosed> {
        let key = (
            self.revision,
            self.active,
            self.disclose,
            self.disclosure_epoch,
            (self.disclose == Disclose::Time)
                .then(|| self.selected())
                .flatten(),
        );
        let mut cache = self
            .disclosure_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((cached, disclosed)) = cache.as_ref()
            && *cached == key
        {
            return Arc::clone(disclosed);
        }
        let disclosed = Arc::new(Disclosed::of(
            self.track(),
            &self.map,
            self.disclose,
            &self.expanded,
        ));
        *cache = Some((key, Arc::clone(&disclosed)));
        disclosed
    }

    /// Whether a span of the active journey shows.
    pub(crate) fn disclosed(&self, span: SpanId) -> bool {
        self.disclosure().shows(span)
    }

    /// The nodes the disclosed journey runs, each with its numbers in the journey.
    pub(crate) fn disclosed_tiles(&self) -> HashMap<usize, Vec<String>> {
        self.disclosure().tiles().clone()
    }

    /// The function a node stands for.
    pub(crate) fn tile_unit(&self, tile: usize) -> Option<&UnitId> {
        self.map.tile(tile).map(|t| &t.unit)
    }

    /// Starts the active journey at the focused node.
    pub(crate) fn trace_from_focus(&mut self) {
        if let Some(unit) = self.graph_focus.and_then(|t| self.tile_unit(t)).cloned() {
            self.start_from(unit);
        } else {
            self.trace_from_selected();
        }
    }

    /// Brings up a journey through the focused node: the active one when it runs the
    /// node, else one traced from it.
    pub(crate) fn journey_for_focus(&mut self) {
        if let Some(unit) = self.graph_focus.and_then(|t| self.tile_unit(t)).cloned() {
            self.set_root(unit);
        }
    }

    /// What calls `tile`, each once, in the order the focus card numbers them.
    pub(crate) fn callers_of(&self, tile: usize) -> Vec<usize> {
        let mut callers: Vec<usize> = self.graph.callers(tile).collect();
        callers.sort_unstable();
        callers.dedup();
        callers
    }

    /// Which side of the focus is in the spotlight, and since when.
    pub(crate) fn spotlight(&self) -> Option<(Side, f32)> {
        self.spotlight
    }

    /// Puts `side` in the spotlight at `now`, or takes it out when it already is.
    pub(crate) fn toggle_spotlight(&mut self, side: Side, now: f32) {
        self.spotlight = match self.spotlight {
            Some((on, _)) if on == side => None,
            _ => Some((side, now)),
        };
    }

    /// The nodes in the spotlight: the focus's callers or its callees, each once.
    pub(crate) fn spotlit(&self) -> Vec<usize> {
        let (Some((side, _)), Some(focus)) = (self.spotlight, self.graph_focus()) else {
            return Vec::new();
        };
        let mut tiles: Vec<usize> = match side {
            Side::Callers => self.graph.callers(focus).collect(),
            Side::Callees => self.graph.callees(focus).collect(),
        };
        tiles.sort_unstable();
        tiles.dedup();
        tiles.retain(|&t| t != focus);
        tiles
    }

    /// Returns to the node opened before this one. Returns whether there was one.
    pub(crate) fn focus_back(&mut self) -> bool {
        let Some(tile) = self.focus_history.pop() else {
            return false;
        };
        self.restart_at(tile);
        true
    }

    /// Lets the focus follow the journey's cursor again.
    pub(crate) fn unfocus(&mut self) {
        self.spotlight = None;
        if let Some(previous) = self.graph_focus.take() {
            self.focus_history.push(previous);
        }
    }

    pub(crate) fn lens(&self) -> Lens {
        self.lens
    }

    pub(crate) fn set_lens(&mut self, lens: Lens) {
        self.lens = lens;
    }

    /// The nodes a diff changed, for a lens that starts from them.
    pub(crate) fn set_changed_tiles(&mut self, tiles: Vec<usize>) {
        self.changed_tiles = tiles;
    }

    /// Where the lens starts: the focus, the changes, or the active journey.
    fn seeds(&self) -> Vec<usize> {
        match self.lens.source {
            lens::Source::Focus => self.graph_focus().into_iter().collect(),
            lens::Source::Changes => self.changed_tiles.clone(),
            lens::Source::Journey => self.disclosed_tiles().into_keys().collect(),
        }
    }

    /// How fully the lens would show each node, by tile, once eased there; `None`
    /// when it shows everything. The journey's cursor always shows.
    pub(crate) fn reveal_targets(&self) -> Option<Vec<f32>> {
        let mut targets = if self.lens.changes_only && !self.changed_tiles.is_empty() {
            // What changed, and where you stand: the focus and the journey's root.
            let mut targets = vec![0.0; self.map.tiles.len()];
            let root = self.root().and_then(|u| self.map.tile_of(u));
            for tile in self
                .changed_tiles
                .iter()
                .copied()
                .chain(self.graph_focus())
                .chain(root)
            {
                if let Some(target) = targets.get_mut(tile) {
                    *target = 1.0;
                }
            }
            targets
        } else {
            self.lens.targets(&self.graph, &self.seeds())?
        };
        // The disclosed journey always shows: it is what you opened. So does what the
        // spotlight picks out.
        for tile in self.disclosed_tiles().into_keys().chain(self.spotlit()) {
            if let Some(target) = targets.get_mut(tile) {
                *target = 1.0;
            }
        }
        if let Some(cursor) = self.focus().and_then(|u| self.map.tile_of(u))
            && let Some(target) = targets.get_mut(cursor)
        {
            *target = 1.0;
        }
        Some(targets)
    }

    /// Eases every node towards how fully the lens shows it. Returns whether any moved.
    pub(crate) fn ease_visibility(&mut self, dt: f32) -> bool {
        let targets = self.reveal_targets();
        lens::ease(&mut self.visibility, targets.as_deref(), dt)
    }

    /// Puts every node where the lens shows it at once.
    #[cfg(test)]
    pub(crate) fn settle_visibility(&mut self) {
        let targets = self.reveal_targets();
        for (tile, shown) in self.visibility.iter_mut().enumerate() {
            *shown = targets
                .as_ref()
                .and_then(|t| t.get(tile))
                .copied()
                .unwrap_or(1.0);
        }
    }

    /// How fully a node shows, from 0 (a speck) to 1.
    pub(crate) fn visibility(&self, tile: usize) -> f32 {
        self.visibility.get(tile).copied().unwrap_or(1.0)
    }

    /// Whether a node shows as more than a speck.
    pub(crate) fn shown(&self, tile: usize) -> bool {
        self.visibility(tile) >= lens::SPECK
    }

    /// The node at the center of attention: under the pointer, else focused.
    pub(crate) fn attention(&self) -> Option<usize> {
        self.hovered_tile.or_else(|| self.graph_focus())
    }

    /// What reaches the node at the center of attention and what it reaches, once
    /// [`Self::refresh_reach`] has run.
    pub(crate) fn reach(&self) -> Option<&Reach> {
        self.reach
            .as_ref()
            .filter(|(tile, _)| Some(*tile) == self.attention())
            .map(|(_, reach)| reach)
    }

    /// Recomputes [`Self::reach`] when attention moved. Returns whether it did.
    pub(crate) fn refresh_reach(&mut self) -> bool {
        let focus = self.graph_focus();
        let mut moved = false;
        if self.focus_reach.as_ref().map(|(tile, _)| *tile) != focus {
            self.focus_reach = focus.map(|tile| (tile, self.graph.reach(tile)));
            moved = true;
        }
        let attention = self.attention();
        if self.reach.as_ref().map(|(tile, _)| *tile) == attention {
            return moved;
        }
        self.reach = match (attention, &self.focus_reach) {
            (Some(tile), Some((focus, reach))) if tile == *focus => Some((tile, reach.clone())),
            (attention, _) => attention.map(|tile| (tile, self.graph.reach(tile))),
        };
        true
    }

    /// The color index for a node: its module's, so each module reads as one region.
    pub(crate) fn tile_color(&self, tile: usize) -> usize {
        self.graph.cluster(tile).unwrap_or_default()
    }

    /// A module cluster's name: the module's own, else its container's.
    pub(crate) fn cluster_name(&self, cluster: usize) -> String {
        self.graph
            .cluster_unit(cluster)
            .and_then(|id| self.any_unit(id))
            .map_or_else(String::new, |unit| unit.name.clone())
    }

    /// A crate's (or top-level package's) name.
    pub(crate) fn crate_name(&self, top: usize) -> String {
        self.graph
            .top_unit(top)
            .and_then(|id| self.any_unit(id))
            .map_or_else(String::new, |unit| unit.name.clone())
    }

    /// Gives every container a color, after those the journeys use.
    fn color_groups(&mut self) {
        for group in &self.map.groups {
            if let Some(unit) = &group.unit {
                let next = self.lane_colors.len();
                self.lane_colors.entry(unit.to_string()).or_insert(next);
            }
        }
    }

    fn refresh_journey_tiles(&mut self) {
        self.journey_tiles = self
            .tracks
            .iter()
            .map(|track| {
                track
                    .scene
                    .nodes
                    .iter()
                    .filter_map(|placed| {
                        let unit = track.unit_id(placed.span)?;
                        Some((self.map.tile_of(unit)?, track.color(placed.span)))
                    })
                    .collect()
            })
            .collect();
    }

    pub(crate) fn map(&self) -> &CodebaseMap {
        &self.map
    }

    /// The color a node is drawn in on the active journey, if it runs it.
    pub(crate) fn journey_lane(&self, tile: usize) -> Option<usize> {
        self.journey_tiles.get(self.active)?.get(&tile).copied()
    }

    /// The unit under the active cursor.
    pub(crate) fn focus(&self) -> Option<&UnitId> {
        let track = self.track();
        track.selected.and_then(|id| track.unit_id(id))
    }

    /// Starts the active journey at `unit`, with the cursor on the unit it was on
    /// before when the new journey reaches it: from a caller, straight to the call
    /// that led here.
    pub(crate) fn start_from(&mut self, unit: UnitId) {
        let focus = self.focus().cloned();
        self.set_root(unit);
        if let Some(landing) = focus.and_then(|f| self.track().find(&f)) {
            self.select(Some(landing));
        }
    }

    pub(crate) fn span(&self, id: SpanId) -> Option<&Span> {
        self.track().span(id)
    }

    /// The unit a span of the active journey runs, if it resolved to exactly one.
    pub(crate) fn unit_id(&self, id: SpanId) -> Option<&UnitId> {
        self.track().unit_id(id)
    }

    pub(crate) fn unit(&self, id: SpanId) -> Option<&Unit> {
        self.system.unit(self.unit_id(id)?)
    }

    pub(crate) fn label(&self, id: SpanId) -> String {
        self.label_in(self.track(), id)
    }

    pub(crate) fn label_in(&self, track: &Track, id: SpanId) -> String {
        let Some(span) = track.span(id) else {
            return String::new();
        };
        let name = match &span.target {
            SpanTarget::Unit(unit) => self.unit_name(unit),
            SpanTarget::Ambiguous(ids) => format!(
                "{} → one of {}",
                span.text.as_deref().unwrap_or("?"),
                ids.len()
            ),
            SpanTarget::Unresolved(text) => text.clone(),
        };
        let name = if span.merged > 1 {
            format!("{name} ({} merged)", span.merged)
        } else {
            name
        };
        match span.stopped {
            Some(StopReason::Recursion) => format!("{name} ↻"),
            Some(StopReason::DepthLimit) => format!("{name} …"),
            Some(StopReason::Registered) => format!("→ {name}"),
            Some(StopReason::NotLoaded) | None => name,
        }
    }

    pub(crate) fn unit_name(&self, id: &UnitId) -> String {
        match &self.layout_system {
            Some(layout) if self.system.unit(id).is_none() => display_name(layout, id),
            _ => display_name(&self.system, id),
        }
    }

    /// The call stack at a span of the active journey, outermost first.
    pub(crate) fn stack(&self, id: SpanId) -> Vec<SpanId> {
        let mut stack: Vec<SpanId> =
            std::iter::successors(Some(id), |s| self.span(*s).and_then(|s| s.parent)).collect();
        stack.reverse();
        stack
    }

    /// Where the call that started `id` is written: the caller's file and line.
    pub(crate) fn call_site(&self, id: SpanId) -> Option<(&UnitId, &Location)> {
        let (caller, key) = self.span(id)?.call.as_ref()?;
        let location = self.system.unit(caller)?.call(key)?.location.as_ref()?;
        Some((caller, location))
    }

    /// Moves the active cursor. Returns the new position.
    pub(crate) fn step(&mut self, step: Step) -> Option<SpanId> {
        let journey = self.journey()?;
        let current = self.selected().unwrap_or_else(|| journey.root());
        let span = journey.span(current)?;
        let nth = |index: usize| journey.spans().nth(index).map(|(id, _)| id);
        // The first span after `id`'s subtree, in pre-order.
        let after = |id: SpanId| {
            let end = journey.span(id).map_or(0, |s| s.exit);
            journey
                .spans()
                .skip(id.index() + 1)
                .find(|(other, s)| s.enter >= end || !is_within(journey, *other, id))
                .map(|(id, _)| id)
        };
        let next = match step {
            Step::Into => nth(current.index() + 1),
            Step::Back => current.index().checked_sub(1).and_then(nth),
            Step::Over => after(current),
            Step::Out => span.parent.and_then(after).or(span.parent),
            Step::Start => Some(journey.root()),
            Step::End => journey.spans().last().map(|(id, _)| id),
            Step::Next => journey
                .spans()
                .skip(current.index() + 1)
                .find(|(_, s)| s.parent == span.parent)
                .map(|(id, _)| id),
            Step::Previous => journey
                .spans()
                .take(current.index())
                .filter(|(_, s)| s.parent == span.parent)
                .last()
                .map(|(id, _)| id),
            Step::Child => journey
                .spans()
                .skip(current.index() + 1)
                .find(|(_, s)| s.parent == Some(current))
                .map(|(id, _)| id),
            Step::Parent => span.parent,
        };
        if next.is_some() {
            self.select(next);
        }
        next
    }
}

fn is_within(journey: &Journey, span: SpanId, ancestor: SpanId) -> bool {
    std::iter::successors(Some(span), |s| journey.span(*s).and_then(|s| s.parent))
        .any(|s| s == ancestor)
}

/// Debugger-style cursor movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// The next call in time, entering callees.
    Into,
    /// The next call after this one returns.
    Over,
    /// The next call after the caller returns.
    Out,
    /// The previous call in time.
    Back,
    Start,
    /// The last call in time.
    End,
    /// The next call made by the same caller.
    Next,
    /// The previous call made by the same caller.
    Previous,
    /// The first call this one makes.
    Child,
    /// The call that made this one.
    Parent,
}

/// Steps the cursor on a timer.
#[derive(Resource, Debug)]
pub(crate) struct Autoplay {
    pub(crate) playing: bool,
    pub(crate) steps_per_second: f32,
    pub(crate) elapsed: f32,
}

impl Default for Autoplay {
    fn default() -> Self {
        Self {
            playing: false,
            steps_per_second: 2.0,
            elapsed: 0.0,
        }
    }
}

/// Whether a span is reached by a call that always happens on this path.
pub(crate) fn is_conditional(span: &Span) -> bool {
    !span.conditions.is_empty() || span.repeats
}

pub(crate) fn is_handoff(span: &Span) -> bool {
    span.mode == Mode::Register
}

#[cfg(test)]
pub(crate) mod tests {
    use codeviz_ir::Document;

    use super::*;
    use crate::lens::Measure;

    /// `main -> (a -> c), b`
    pub(crate) fn explorer() -> Explorer {
        let unit = |name: &str, calls: &[&str]| {
            let calls: Vec<String> = calls
                .iter()
                .map(|c| {
                    format!(
                        r#"{{ "key": "{c}#0", "text": "{c}", "targets": [{{ "unit": "x:{c}", "certainty": "exact" }}] }}"#
                    )
                })
                .collect();
            format!(
                r#"{{ "id": "x:{name}", "name": "{name}", "kind": "fn", "role": "invocable", "calls": [{}] }}"#,
                calls.join(",")
            )
        };
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}, {}, {}, {}] }}"#,
            unit("main", &["a", "b"]),
            unit("a", &["c"]),
            unit("b", &[]),
            unit("c", &[])
        );
        let system = System::new([Document::from_json(&json).unwrap()]);
        Explorer::new(
            system,
            Some(UnitId::new("x:main")),
            JourneyOptions::default(),
        )
    }

    fn at(explorer: &Explorer) -> String {
        explorer.label(explorer.selected().unwrap())
    }

    #[test]
    fn stepping_like_a_debugger() {
        let mut explorer = explorer();
        assert_eq!(at(&explorer), "main");

        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "a");
        explorer.step(Step::Over);
        assert_eq!(at(&explorer), "b", "over skips a's callee c");

        explorer.step(Step::Back);
        explorer.step(Step::Back);
        assert_eq!(at(&explorer), "a");
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");
        explorer.step(Step::Out);
        assert_eq!(
            at(&explorer),
            "b",
            "out leaves a and lands on what follows it"
        );

        explorer.step(Step::Start);
        assert_eq!(at(&explorer), "main");
    }

    #[test]
    fn walking_the_call_tree() {
        let mut explorer = explorer();
        explorer.step(Step::Child);
        assert_eq!(at(&explorer), "a");
        explorer.step(Step::Next);
        assert_eq!(at(&explorer), "b", "next skips a's own calls");
        assert_eq!(explorer.step(Step::Next), None, "b is main's last call");
        explorer.step(Step::Previous);
        assert_eq!(at(&explorer), "a");
        explorer.step(Step::Child);
        assert_eq!(at(&explorer), "c");
        assert_eq!(explorer.step(Step::Child), None);
        explorer.step(Step::Parent);
        assert_eq!(at(&explorer), "a");
        explorer.step(Step::End);
        assert_eq!(at(&explorer), "b");
    }

    #[test]
    fn the_lens_measures_in_calls_space_or_both() {
        let mut explorer = explorer();
        let tile = |explorer: &Explorer, name: &str| {
            explorer
                .map()
                .tile_of(&UnitId::new(format!("x:{name}")))
                .unwrap()
        };
        let (main, a, b, c) = (
            tile(&explorer, "main"),
            tile(&explorer, "a"),
            tile(&explorer, "b"),
            tile(&explorer, "c"),
        );
        // b sits right next to a but only meets it through main.
        let mut positions = vec![[0.0; 3]; explorer.map().tiles.len()];
        positions[main] = [0.0, 0.0, 0.0];
        positions[a] = [1.0, 0.0, 0.0];
        positions[b] = [1.5, 0.0, 0.0];
        positions[c] = [2.0, 0.0, 0.0];
        explorer.place_nodes(&positions);
        explorer.open_at(a);
        let shown = |explorer: &mut Explorer, lens: Lens| {
            explorer.set_lens(lens);
            explorer.settle_visibility();
            [main, a, b, c].map(|t| explorer.visibility(t))
        };
        let lens = |measure, reach| Lens {
            measure,
            reach: Some(reach),
            ..Lens::default()
        };

        assert_eq!(
            shown(&mut explorer, lens(Measure::Calls, 0)),
            [0.3, 1.0, 0.0, 1.0],
            "c is what a calls, disclosed by a's journey"
        );
        assert_eq!(
            shown(&mut explorer, lens(Measure::Space, 0))[2],
            1.0,
            "in space, b is as near as anything"
        );
        assert_eq!(
            shown(&mut explorer, lens(Measure::Blend, 0)),
            [1.0, 1.0, 0.0, 1.0],
            "a blend shows direct calls, and holds b back for the calls between"
        );
        assert_eq!(shown(&mut explorer, lens(Measure::Blend, 3))[2], 1.0);

        explorer.set_changed_tiles(vec![c]);
        let changes = Lens {
            source: lens::Source::Changes,
            ..lens(Measure::Calls, 1)
        };
        assert_eq!(
            shown(&mut explorer, changes),
            [0.3, 1.0, 0.0, 1.0],
            "from changes, only what calls them counts"
        );
    }

    #[test]
    fn only_changes_hides_the_rest_but_where_you_stand() {
        let mut explorer = explorer();
        let tile = |explorer: &Explorer, name: &str| {
            explorer
                .map()
                .tile_of(&UnitId::new(format!("x:{name}")))
                .unwrap()
        };
        let (main, a, b, c) = (
            tile(&explorer, "main"),
            tile(&explorer, "a"),
            tile(&explorer, "b"),
            tile(&explorer, "c"),
        );
        explorer.set_changed_tiles(vec![c]);
        explorer.open_at(b);
        explorer.set_lens(Lens {
            changes_only: true,
            ..Lens::default()
        });
        explorer.settle_visibility();
        assert!(explorer.shown(c), "what changed");
        assert!(explorer.shown(b), "the node opened");
        assert!(
            !explorer.shown(a) && !explorer.shown(main),
            "and nothing else"
        );
    }

    #[test]
    fn the_lens_keeps_the_focus_neighborhood() {
        let mut explorer = explorer();
        let tile = |name: &str| {
            explorer
                .map()
                .tile_of(&UnitId::new(format!("x:{name}")))
                .unwrap()
        };
        let (main, a, b, c) = (tile("main"), tile("a"), tile("b"), tile("c"));
        explorer.open_at(a);
        explorer.refresh_reach();
        explorer.set_lens(Lens {
            reach: Some(0),
            ..Lens::default()
        });
        explorer.settle_visibility();
        assert!(explorer.shown(a) && explorer.shown(main) && explorer.shown(c));
        assert!(
            explorer.visibility(main) < 1.0,
            "one call past the reach dims"
        );
        explorer.open_at(b);
        explorer.refresh_reach();
        explorer.settle_visibility();
        assert!(!explorer.shown(c), "c is not on any call chain through b");
        assert!(
            !explorer.shown(a),
            "siblings only meet through their caller"
        );
        assert!(explorer.focus_back());
        explorer.refresh_reach();
        assert_eq!(explorer.graph_focus(), Some(a));
    }

    #[test]
    fn starting_from_a_caller_keeps_the_cursor_on_the_callee() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");

        explorer.refresh_reach();
        let tile = |name: &str| explorer.map().tile_of(&UnitId::new(name)).unwrap();
        let reach = explorer.reach().unwrap();
        assert_eq!(reach.upstream.get(&tile("x:a")), Some(&1));
        assert_eq!(reach.upstream.get(&tile("x:main")), Some(&2));

        explorer.start_from(UnitId::new("x:a"));
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:a"));
        assert_eq!(at(&explorer), "c");
    }

    #[test]
    fn pulling_out_a_journey_gives_it_its_own_cursor() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");

        explorer.add_track(UnitId::new("x:a"));
        assert_eq!(explorer.tracks().len(), 2);
        assert_eq!(explorer.active(), 1);
        assert_eq!(at(&explorer), "c", "lands on the unit the cursor was on");
        let [first, second] = explorer.tracks() else {
            panic!("two journeys");
        };
        assert_eq!(
            first.color(first.find(&UnitId::new("x:c")).unwrap()),
            second.color(second.find(&UnitId::new("x:c")).unwrap()),
            "one module, one color, in every journey"
        );

        explorer.step(Step::Back);
        assert_eq!(at(&explorer), "a");
        let first_cursor = explorer.tracks()[0].selected();
        assert_eq!(
            first_cursor.map(|s| explorer.label_in(&explorer.tracks()[0], s)),
            Some("c".to_owned()),
            "each journey keeps its own cursor"
        );

        explorer.close_track(1);
        assert_eq!(explorer.tracks().len(), 1);
        assert_eq!(at(&explorer), "c");
        explorer.close_track(0);
        assert_eq!(explorer.tracks().len(), 1, "the last journey stays");
    }

    #[test]
    fn rerooting_keeps_history() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.trace_from_selected();
        assert_eq!(explorer.journey().map(Journey::len), Some(2));
        assert!(explorer.can_go_back());

        explorer.back();
        assert_eq!(explorer.journey().map(Journey::len), Some(4));
        assert!(!explorer.can_go_back());
    }

    #[test]
    fn opening_a_node_discloses_its_journey_a_level_at_a_time() {
        let mut explorer = explorer();
        let tile =
            |explorer: &Explorer, name: &str| explorer.map().tile_of(&UnitId::new(name)).unwrap();
        let (main, a, b, c) = (
            tile(&explorer, "x:main"),
            tile(&explorer, "x:a"),
            tile(&explorer, "x:b"),
            tile(&explorer, "x:c"),
        );
        explorer.open_at(a);
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:a"));
        assert_eq!(explorer.anchor(), Some(a));

        explorer.open_at(main);
        let numbers = |explorer: &Explorer| {
            let mut tiles: Vec<(usize, Vec<String>)> =
                explorer.disclosed_tiles().into_iter().collect();
            tiles.sort();
            tiles
        };
        let mut expected = vec![
            (main, vec![]),
            (a, vec!["1".to_owned()]),
            (b, vec!["2".to_owned()]),
        ];
        expected.sort();
        assert_eq!(
            numbers(&explorer),
            expected,
            "only what main calls directly"
        );
        let a_span = explorer
            .selected()
            .and_then(|root| explorer.journey()?.span(root)?.children.first().copied());
        assert_eq!(
            a_span.map(|s| explorer.disclosure().folded(s)),
            Some(1),
            "c is folded"
        );

        explorer.toggle_expanded(a_span.unwrap());
        assert!(numbers(&explorer).contains(&(c, vec!["1.1".to_owned()])));
        explorer.toggle_expanded(a_span.unwrap());
        explorer.set_disclose(Disclose::Depth(2));
        assert!(numbers(&explorer).contains(&(c, vec!["1.1".to_owned()])));

        explorer.set_disclose(Disclose::Time);
        assert_eq!(numbers(&explorer), vec![(main, vec![])], "nothing yet");
        explorer.step(Step::Into);
        assert!(
            numbers(&explorer).iter().any(|(t, _)| *t == a),
            "a, once reached"
        );
        assert_eq!(explorer.anchor(), Some(main), "the camera stays on main");

        assert!(explorer.focus_back());
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:a"));
    }
}
