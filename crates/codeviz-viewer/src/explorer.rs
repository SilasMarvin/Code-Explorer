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
    scenarios,
};
use codeviz_ir::{Certainty, ExitKind, Location, Mode, Role, Unit, UnitId};
use codeviz_layout::{CodebaseMap, ForceConfig, ForceGraph, LayoutConfig, MapConfig, Reach, Scene};

use crate::{
    disclosure::{Disclose, Disclosed},
    history::{History, View},
    lens::{self, Lens},
    names::{Names, Path},
    routes,
    starts::{self, Start},
};

/// What a disclosure was worked out for: the journeys' revision, the active one, how
/// it discloses, the spans opened by hand, and the cursor when that matters.
type DisclosureKey = (u64, usize, Disclose, u64, Option<SpanId>);

/// What the names in view were worked out for: the nodes the lens shows, the calls
/// the journey discloses, and what a diff changed.
#[derive(Debug)]
struct NamesKey {
    shown: Vec<usize>,
    disclosed: Arc<Disclosed>,
    changed: Vec<usize>,
}

impl NamesKey {
    fn same(&self, other: &Self) -> bool {
        self.shown == other.shown
            && Arc::ptr_eq(&self.disclosed, &other.disclosed)
            && self.changed == other.changed
    }
}

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
    scenarios: Vec<Scenario>,
    scenario: Option<usize>,
    journey: Option<Journey>,
    scene: Scene,
    divergences: Vec<Divergence>,
    selected: Option<SpanId>,
    /// Spans opened one level past the disclosed depth, by hand or by the cursor.
    expanded: HashSet<SpanId>,
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
        // A rebuilt journey numbers its spans afresh.
        self.expanded.clear();
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
pub(crate) use codeviz_layout::Toward as Side;

/// One side of the focus picked out and brought into view: what calls it, out to the
/// entry points, or what it calls, as many calls away as `reach` says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Spotlight {
    pub(crate) side: Side,
    /// How many calls away it picks out; `None` goes all the way, to the entry points
    /// above the focus or to everything below it.
    pub(crate) reach: Option<u32>,
    /// When it came on or last widened, in seconds of app time.
    pub(crate) since: f32,
    /// The node it was turned on for: it is out once the focus moves off it.
    pub(crate) focus: usize,
}

impl Spotlight {
    /// Whether a tile this many calls from the focus is picked out.
    pub(crate) fn holds(&self, hops: u32) -> bool {
        hops > 0 && self.reach.is_none_or(|reach| hops <= reach)
    }
}

/// Walks from a node along one side, and where the routes into a node start, each
/// kept until the graph's calls change.
#[derive(Debug, Default)]
struct RouteCache {
    walks: HashMap<(usize, Side), Arc<HashMap<usize, u32>>>,
    starts: HashMap<usize, Arc<Vec<routes::Start>>>,
}

impl RouteCache {
    /// Forgets everything once it holds this much, so it never grows without end.
    const MOST: usize = 64;

    fn clear(&mut self) {
        self.walks.clear();
        self.starts.clear();
    }
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
    /// Map tiles each journey runs, with the color each is drawn in there.
    journey_tiles: Vec<HashMap<usize, usize>>,
    /// What reaches the focus, and what it reaches.
    reach: Option<(usize, Reach)>,
    /// How much of the graph shows.
    lens: Lens,
    /// How fully each node shows, by tile, eased towards what the lens says.
    visibility: Vec<f32>,
    /// A diff's changed nodes, for a lens that starts from them.
    changed_tiles: Vec<usize>,
    /// How much of the active journey shows.
    disclose: Disclose,
    /// Bumped whenever a journey's opened spans change.
    disclosure_epoch: u64,
    disclosure_cache: Mutex<Option<(DisclosureKey, Arc<Disclosed>)>>,
    /// Where the view has been, to go back and forward.
    history: History<View>,
    /// Bumped whenever the view is pointed somewhere by hand, so the camera turns to
    /// it even when that is where it already was.
    aim: u64,
    /// The node under the pointer, in the graph or as a bar in the journey.
    pub(crate) hovered_tile: Option<usize>,
    /// What calls the focus, or what it calls, picked out and brought into view.
    spotlight: Option<Spotlight>,
    /// Declared entry points, by node, with their labels.
    entries: HashMap<usize, Vec<String>>,
    routes: Mutex<RouteCache>,
    /// Lane label to color index, in order of first appearance across journeys.
    lane_colors: HashMap<String, usize>,
    /// Bumped whenever a journey is rebuilt.
    revision: u64,
    /// What each unit in view is called, told apart from the rest in view.
    names: Names,
    names_for: Option<NamesKey>,
    /// What each function is called told apart from every other, for what is not in
    /// view.
    every_name: Names,
    /// Each module cluster's name, told apart from the other clusters'.
    cluster_names: Vec<String>,
    /// How long the graph took to lay out before it was first shown.
    laid_out: Duration,
    /// Where to start exploring, best first.
    starts: Vec<Start>,
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
        let mut busiest: Vec<usize> = (0..map.tiles.len()).collect();
        busiest.sort_by_key(|&t| (std::cmp::Reverse(graph.degree(t)), t));
        let tiles = map.tiles.len();
        let mut entries: HashMap<usize, Vec<String>> = HashMap::new();
        for entry in system.entries() {
            if let Some(tile) = map.tile_of(&entry.unit) {
                entries.entry(tile).or_default().push(entry.label.clone());
            }
        }
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
            journey_tiles: Vec::new(),
            reach: None,
            lens: Lens::default(),
            visibility: vec![1.0; tiles],
            changed_tiles: Vec::new(),
            history: History::default(),
            aim: 0,
            disclose: Disclose::default(),
            disclosure_epoch: 0,
            disclosure_cache: Mutex::new(None),
            hovered_tile: None,
            spotlight: None,
            entries,
            routes: Mutex::new(RouteCache::default()),
            lane_colors: HashMap::new(),
            revision: 0,
            names: Names::default(),
            names_for: None,
            every_name: Names::default(),
            cluster_names: Vec::new(),
            laid_out,
            starts,
            busiest,
        };
        explorer.name_everything();
        explorer.color_groups();
        if let Some(root) = root {
            explorer.replace_root(root);
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
        self.routes_cache().clear();
        self.reach = None;
    }

    /// Whether calls the producer only guessed at count as callers and callees.
    pub(crate) fn counts_guesses(&self) -> bool {
        self.graph.counts_guesses()
    }

    /// Lets guessed calls count as callers, callees and reach, or leaves them out.
    pub(crate) fn set_guesses(&mut self, counted: bool) {
        self.graph.set_guesses(counted);
        self.reach = None;
    }

    /// Whether a guessed call is drawn: both its ends show.
    pub(crate) fn guesses_in_view(&self) -> bool {
        self.graph
            .guesses
            .iter()
            .any(|&(from, to)| self.shown(from) && self.shown(to))
    }

    /// The tiles on `side` of `tile` that only a guess links it to, each once, when
    /// guesses are left out.
    pub(crate) fn guessed_neighbors(&self, tile: usize, side: Side) -> usize {
        if self.counts_guesses() {
            return 0;
        }
        self.graph
            .guesses
            .iter()
            .filter(|&&(from, to)| match side {
                Side::Callers => to == tile && from != tile,
                Side::Callees => from == tile && to != tile,
            })
            .count()
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

    /// The view as it is now, to come back to.
    fn view(&self) -> View {
        View {
            track: self.active,
            root: self.root().cloned(),
            scenario: self.scenario(),
            cursor: self.selected(),
            disclose: self.disclose,
            expanded: self.track().expanded.clone(),
            spotlight: self.spotlight,
        }
    }

    /// Puts the view back as it was, all of it: the journey, its outcome and cursor,
    /// how much of it shows, and the spotlight.
    fn restore(&mut self, view: View) {
        self.active = view.track.min(self.tracks.len() - 1);
        let index = self.active;
        let track = &mut self.tracks[index];
        if track.root != view.root {
            track.root = view.root;
            track.reroot(&self.system, &self.options, &self.layout);
        }
        if track.scenario != view.scenario {
            track.scenario = view.scenario;
            track.rebuild(&self.system, &self.options, &self.layout);
        }
        // A journey rebuilt since may no longer have the span.
        track.selected = view
            .cursor
            .filter(|&span| track.span(span).is_some())
            .or_else(|| track.journey().map(Journey::root));
        track.expanded = view.expanded;
        self.disclose = view.disclose;
        self.disclosure_epoch += 1;
        self.spotlight = view.spotlight;
        self.aim += 1;
        self.recolor();
    }

    /// Goes somewhere: whatever `go` changes, the view before it is remembered, so
    /// Back returns to it. A move that changes nothing is not remembered.
    fn navigate(&mut self, go: impl FnOnce(&mut Self)) {
        let before = self.view();
        go(self);
        if self.view() != before {
            self.history.leave(before);
        }
    }

    /// Returns to the view before the last move. Returns whether there was one.
    pub(crate) fn go_back(&mut self) -> bool {
        let current = self.view();
        match self.history.back(current) {
            Some(view) => {
                self.restore(view);
                true
            }
            None => false,
        }
    }

    /// Returns to the view gone back from. Returns whether there was one.
    pub(crate) fn go_forward(&mut self) -> bool {
        let current = self.view();
        match self.history.forward(current) {
            Some(view) => {
                self.restore(view);
                true
            }
            None => false,
        }
    }

    pub(crate) fn can_go_back(&self) -> bool {
        self.history.can_go_back()
    }

    pub(crate) fn can_go_forward(&self) -> bool {
        self.history.can_go_forward()
    }

    /// Forgets where the view has been, so it starts from how it opened.
    pub(crate) fn forget_history(&mut self) {
        self.history.clear();
    }

    /// Bumped whenever the view is pointed somewhere by hand.
    pub(crate) fn aim(&self) -> u64 {
        self.aim
    }

    /// The span the active journey's cursor is on.
    pub(crate) fn selected(&self) -> Option<SpanId> {
        self.track().selected
    }

    /// Moves the active cursor, and the focus with it. The spotlight was on the focus
    /// it leaves, so it goes out; the calls folded over where it lands open, so its bar
    /// shows.
    pub(crate) fn select(&mut self, span: Option<SpanId>) {
        let before = self.graph_focus();
        self.track_mut().selected = span;
        if self.graph_focus() != before {
            self.spotlight = None;
        }
        self.reveal_cursor();
    }

    /// Opens the calls the cursor is folded under, a level at a time, as a click on
    /// each `+N` would: wherever the cursor goes, its bar shows, and the graph numbers
    /// its node.
    fn reveal_cursor(&mut self) {
        let Some(cursor) = self.selected() else {
            return;
        };
        let stack = self.stack(cursor);
        for _ in 0..stack.len() {
            let disclosed = self.disclosure();
            if disclosed.shows(cursor) {
                return;
            }
            // The deepest numbered call above it is shown, and folds what is below.
            let Some(&folding) = stack
                .iter()
                .rev()
                .find(|&&span| disclosed.number(span).is_some())
            else {
                return;
            };
            if !self.track_mut().expanded.insert(folding) {
                return;
            }
            self.disclosure_epoch += 1;
        }
    }

    /// Makes `track` active with its cursor on `span`.
    pub(crate) fn select_in(&mut self, at: SpanRef) {
        if at.track < self.tracks.len() {
            self.activate(at.track);
            self.select(Some(at.span));
        }
    }

    /// Makes `track` active, its cursor where it was.
    pub(crate) fn activate(&mut self, track: usize) {
        if track < self.tracks.len() && self.active != track {
            self.active = track;
            self.refresh_journey_tiles();
        }
    }

    /// Starts the active journey at `root`, remembering where it was.
    pub(crate) fn set_root(&mut self, root: UnitId) {
        self.navigate(|explorer| explorer.replace_root(root));
    }

    /// Starts the active journey at `root` on its happy path, with the cursor on it.
    fn replace_root(&mut self, root: UnitId) {
        if self.root() == Some(&root) {
            return;
        }
        self.track_mut().root = Some(root);
        self.reroot();
    }

    /// Recomputes outcomes for the active journey's root and opens on the happy path,
    /// with the cursor on the root.
    fn reroot(&mut self) {
        let index = self.active.min(self.tracks.len() - 1);
        if let Some(track) = self.tracks.get_mut(index) {
            track.reroot(&self.system, &self.options, &self.layout);
        }
        self.recolor();
    }

    /// Follows another outcome, landing on the exit it ends at.
    pub(crate) fn set_scenario(&mut self, scenario: Option<usize>) {
        self.navigate(|explorer| explorer.choose_scenario(scenario));
    }

    fn choose_scenario(&mut self, scenario: Option<usize>) {
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

    /// Starts the active journey at the focus: from where the cursor is, as its own
    /// journey.
    pub(crate) fn trace_from_focus(&mut self) {
        if let Some(unit) = self.graph_focus().and_then(|t| self.tile_unit(t)).cloned() {
            self.set_root(unit);
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
        self.entries.contains_key(&tile)
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
            self.replace_root(unit);
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

    /// The focus: the node the graph lights up around, and what I, O and Enter act on.
    /// It is the cursor's node, or the root's when the journey could not be traced.
    pub(crate) fn graph_focus(&self) -> Option<usize> {
        self.focus()
            .and_then(|f| self.map.tile_of(f))
            .or_else(|| self.root_tile())
    }

    fn root_tile(&self) -> Option<usize> {
        self.root().and_then(|u| self.map.tile_of(u))
    }

    /// The node the camera orbits: where the active journey starts. Stepping through
    /// the journey leaves it put.
    pub(crate) fn anchor(&self) -> Option<usize> {
        self.root_tile().or_else(|| self.graph_focus())
    }

    /// Centers on a node and opens its journey: the active journey restarts there, with
    /// what it calls directly disclosed and the rest folded away. Opening the node the
    /// journey already starts at puts the cursor back on it.
    pub(crate) fn open_at(&mut self, tile: usize) {
        self.navigate(|explorer| explorer.restart_at(tile));
        self.aim += 1;
    }

    fn restart_at(&mut self, tile: usize) {
        let Some(unit) = self.tile_unit(tile).cloned() else {
            return;
        };
        if self.root() == Some(&unit) {
            self.select(self.journey().map(Journey::root));
            return;
        }
        self.replace_root(unit);
        self.spotlight = None;
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
        let expanded = &mut self.track_mut().expanded;
        if !expanded.remove(&span) {
            expanded.insert(span);
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
            &self.track().expanded,
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

    /// Opens the call numbered `number` on screen: by the journey's own numbering, as
    /// its bars and its nodes' badges show it, or with nothing of the journey disclosed,
    /// the focus's calls in the order it makes them, as `#1`, `#2` beside them. A call
    /// that could run any of several functions has no one node to open, so the cursor
    /// goes to its bar instead.
    pub(crate) fn open_numbered(&mut self, number: usize) {
        let Some(index) = number.checked_sub(1) else {
            return;
        };
        let disclosed = self.disclosure();
        if !disclosed.opened() {
            if let Some(&tile) = self
                .graph_focus()
                .and_then(|focus| self.graph.sequence(focus).get(index))
            {
                self.open_at(tile);
            }
            return;
        }
        let Some(&call) = self
            .journey()
            .and_then(|journey| disclosed.children(journey.root()).get(index))
            .filter(|&&call| disclosed.number(call).is_some())
        else {
            return;
        };
        match self.unit_id(call).and_then(|unit| self.map.tile_of(unit)) {
            Some(tile) => self.open_at(tile),
            None => self.select(Some(call)),
        }
    }

    /// What calls `tile`, each once.
    pub(crate) fn callers_of(&self, tile: usize) -> Vec<usize> {
        let mut callers: Vec<usize> = self.graph.callers(tile).collect();
        callers.sort_unstable();
        callers.dedup();
        callers
    }

    /// Which side of the focus is in the spotlight, how far, and since when. Only ever
    /// on the focus it was turned on for: once the focus moves, it is out.
    pub(crate) fn spotlight(&self) -> Option<Spotlight> {
        self.spotlight
            .filter(|s| Some(s.focus) == self.graph_focus())
    }

    /// Puts `side` in the spotlight one call out, then two, then all the way, then
    /// takes it out: past two calls, every route is what you want, and taking it out
    /// stays a few presses away. Nothing on that side, nothing to pick out.
    pub(crate) fn widen_spotlight(&mut self, side: Side, now: f32) {
        if self.furthest(side) == 0 {
            return;
        }
        let Some(focus) = self.graph_focus() else {
            return;
        };
        let reach = match self.spotlight() {
            Some(on) if on.side == side => match on.reach {
                Some(1) => Some(Some(2)),
                Some(_) => Some(None),
                None => None,
            },
            _ => Some(Some(1)),
        };
        self.spotlight = reach.map(|reach| Spotlight {
            side,
            reach: reach.filter(|&r| r < self.furthest(side)),
            since: now,
            focus,
        });
    }

    /// Puts all of `side` in the spotlight, out to the entry points or the end of
    /// every call chain, or takes it out when it already is.
    pub(crate) fn spotlight_all(&mut self, side: Side, now: f32) {
        let Some(focus) = self.graph_focus().filter(|_| self.furthest(side) > 0) else {
            return;
        };
        self.spotlight = match self.spotlight() {
            Some(on) if on.side == side && on.reach.is_none() => None,
            _ => Some(Spotlight {
                side,
                reach: None,
                since: now,
                focus,
            }),
        };
    }

    /// Takes the spotlight out.
    pub(crate) fn clear_spotlight(&mut self) {
        self.spotlight = None;
    }

    /// The most calls between the focus and anything on `side` of it.
    fn furthest(&self, side: Side) -> u32 {
        self.graph_focus().map_or(0, |focus| {
            self.walk(focus, side).values().copied().max().unwrap_or(0)
        })
    }

    /// The nodes in the spotlight, each once: the focus's callers or callees, as far
    /// out as it reaches.
    pub(crate) fn spotlit(&self) -> Vec<usize> {
        let mut tiles: Vec<usize> = self.spotlit_hops().into_keys().collect();
        tiles.sort_unstable();
        tiles
    }

    /// The nodes in the spotlight, each with its calls from the focus.
    pub(crate) fn spotlit_hops(&self) -> HashMap<usize, u32> {
        let (Some(spotlight), Some(focus)) = (self.spotlight(), self.graph_focus()) else {
            return HashMap::new();
        };
        self.walk(focus, spotlight.side)
            .iter()
            .filter(|&(&tile, &hops)| tile != focus && spotlight.holds(hops))
            .map(|(&tile, &hops)| (tile, hops))
            .collect()
    }

    /// How many calls from the focus a node in the spotlight is; `None` when it is not
    /// in the spotlight.
    pub(crate) fn spotlight_hops(&self, tile: usize) -> Option<u32> {
        let (spotlight, focus) = (self.spotlight()?, self.graph_focus()?);
        self.walk(focus, spotlight.side)
            .get(&tile)
            .copied()
            .filter(|&hops| spotlight.holds(hops))
    }

    /// Fewest calls from `tile` to everything on `side` of it: up its callers as far
    /// as the entry points, or down everything it calls. `tile` itself is zero.
    pub(crate) fn walk(&self, tile: usize, side: Side) -> Arc<HashMap<usize, u32>> {
        let mut cache = self.routes_cache();
        if cache.walks.len() > RouteCache::MOST {
            cache.walks.clear();
        }
        Arc::clone(cache.walks.entry((tile, side)).or_insert_with(|| {
            let stop = |t: usize| side == Side::Callers && self.entries.contains_key(&t);
            Arc::new(self.graph.walk(&[tile], side, None, stop))
        }))
    }

    fn routes_cache(&self) -> std::sync::MutexGuard<'_, RouteCache> {
        self.routes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Where the routes into `tile` start: the entry points that reach it, then what
    /// nothing calls, nearest first.
    pub(crate) fn route_starts(&self, tile: usize) -> Arc<Vec<routes::Start>> {
        if let Some(starts) = self.routes_cache().starts.get(&tile) {
            return Arc::clone(starts);
        }
        let upstream = self.walk(tile, Side::Callers);
        let starts = Arc::new(routes::starts(&self.graph, &upstream, &self.entries, |t| {
            self.tile_unit(t)
                .map_or_else(String::new, |unit| self.unit_name(unit))
        }));
        let mut cache = self.routes_cache();
        if cache.starts.len() > RouteCache::MOST {
            cache.starts.clear();
        }
        cache.starts.insert(tile, Arc::clone(&starts));
        starts
    }

    /// Declared entry points, by node, with the label of each route it is declared
    /// under.
    pub(crate) fn entries(&self) -> &HashMap<usize, Vec<String>> {
        &self.entries
    }

    /// Follows the next route into the focus, or the one before, in the order
    /// [`Self::route_starts`] lists them, picking out every route as it does. Returns
    /// whether there was one to follow.
    pub(crate) fn follow_next_route(&mut self, forward: bool, now: f32) -> bool {
        let Some(focus) = self.graph_focus() else {
            return false;
        };
        let starts = self.route_starts(focus);
        if starts.is_empty() {
            return false;
        }
        let root = self.root().and_then(|unit| self.map.tile_of(unit));
        let next = match starts.iter().position(|s| Some(s.tile) == root) {
            Some(at) if forward => (at + 1) % starts.len(),
            Some(at) => (at + starts.len() - 1) % starts.len(),
            None if forward => 0,
            None => starts.len() - 1,
        };
        if !self
            .spotlight()
            .is_some_and(|s| s.side == Side::Callers && s.reach.is_none())
        {
            self.spotlight_all(Side::Callers, now);
        }
        self.follow_route(starts[next].tile);
        true
    }

    /// The route start the active journey begins at, while it follows a route into
    /// the focus.
    pub(crate) fn followed_start(&self) -> Option<usize> {
        if !self.spotlight().is_some_and(|s| s.side == Side::Callers) {
            return None;
        }
        let root = self.root().and_then(|unit| self.map.tile_of(unit))?;
        (Some(root) != self.graph_focus() && self.spotlight_hops(root).is_some()).then_some(root)
    }

    /// Opens the journey from `start`, keeping the focus, with the cursor on the focus
    /// and every call on the way there disclosed: one route into it, followed.
    pub(crate) fn follow_route(&mut self, start: usize) {
        let Some(focus) = self.graph_focus() else {
            return;
        };
        let (Some(root), Some(target)) = (
            self.tile_unit(start).cloned(),
            self.tile_unit(focus).cloned(),
        ) else {
            return;
        };
        // The routes stay picked out, on the same focus, as the journey changes under
        // it.
        let spotlight = self.spotlight;
        // Following one route after another keeps one place in the history: Back
        // returns to before the first.
        if self.followed_start().is_some() {
            self.track_mut().root = Some(root);
            self.reroot();
        } else {
            self.set_root(root);
        }
        // A function reached only on the way to a failure is not on the happy path.
        if self.track().find(&target).is_none() && self.scenario().is_some() {
            self.set_scenario(None);
        }
        let landing = self.track().find(&target);
        let above: Vec<SpanId> = landing
            .and_then(|landing| {
                let journey = self.journey()?;
                let parent = journey.span(landing)?.parent;
                Some(std::iter::successors(parent, |&span| journey.span(span)?.parent).collect())
            })
            .unwrap_or_default();
        self.track_mut().expanded = above.into_iter().collect();
        self.disclosure_epoch += 1;
        if let Some(landing) = landing {
            self.track_mut().selected = Some(landing);
        }
        self.spotlight = spotlight;
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
        let mut targets = if self.routes_only() {
            // The routes alone, from where they start to the focus: the rest of the
            // graph would bury them.
            let mut targets = vec![0.0; self.map.tiles.len()];
            if let Some(target) = self.graph_focus().and_then(|f| targets.get_mut(f)) {
                *target = 1.0;
            }
            targets
        } else if self.lens.changes_only && !self.changed_tiles.is_empty() {
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

    /// Whether the spotlight picks out more than what is next to the focus, so that
    /// it shows its routes and leaves the rest of the graph out.
    pub(crate) fn routes_only(&self) -> bool {
        self.spotlight().is_some_and(|s| s.reach != Some(1))
            && self.spotlit_hops().values().any(|&hops| hops > 1)
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

    /// What reaches the focus and what it reaches, once [`Self::refresh_reach`] has
    /// run.
    pub(crate) fn reach(&self) -> Option<&Reach> {
        self.reach
            .as_ref()
            .filter(|(tile, _)| Some(*tile) == self.graph_focus())
            .map(|(_, reach)| reach)
    }

    /// Recomputes [`Self::reach`] when the focus moved. Returns whether it did.
    pub(crate) fn refresh_reach(&mut self) -> bool {
        let focus = self.graph_focus();
        if self.reach.as_ref().map(|(tile, _)| *tile) == focus {
            return false;
        }
        self.reach = focus.map(|tile| (tile, self.graph.reach(tile)));
        true
    }

    /// The color index for a node: its module's, so each module reads as one region.
    pub(crate) fn tile_color(&self, tile: usize) -> usize {
        self.graph.cluster(tile).unwrap_or_default()
    }

    /// A module cluster's name: the module's own, with its parent modules and crate in
    /// front as far as it takes to tell it from the others.
    pub(crate) fn cluster_name(&self, cluster: usize) -> String {
        self.cluster_names.get(cluster).cloned().unwrap_or_default()
    }

    /// Names every function apart from all the others, and every module cluster
    /// apart from the other clusters.
    fn name_everything(&mut self) {
        let placed = self.layout_system.as_ref().unwrap_or(&self.system);
        self.every_name = Names::apart(
            placed
                .units()
                .filter(|u| u.role == Role::Invocable)
                .map(|u| (u.id.clone(), self.unit_path(&u.id))),
        );
        let clusters: Vec<Option<&UnitId>> = (0..self.graph.cluster_count())
            .map(|cluster| self.graph.cluster_unit(cluster))
            .collect();
        let names = Names::apart(
            clusters
                .iter()
                .flatten()
                .map(|&id| (id.clone(), self.unit_path(id))),
        );
        self.cluster_names = clusters
            .iter()
            .map(|id| {
                id.and_then(|id| names.get(id))
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect();
    }

    /// Names what is in view apart from each other: every node the lens shows and
    /// every call the journey discloses, so a node and its bar read the same. What a
    /// diff changed carries its type or module at least: `new` alone could be any
    /// type's.
    pub(crate) fn refresh_names(&mut self) {
        let key = NamesKey {
            shown: (0..self.map.tiles.len())
                .filter(|&tile| self.visibility(tile) >= 0.5)
                .collect(),
            disclosed: self.disclosure(),
            changed: self.changed_tiles.clone(),
        };
        if self.names_for.as_ref().is_some_and(|k| k.same(&key)) {
            return;
        }
        let track = self.track();
        let changed: HashSet<&UnitId> = key
            .changed
            .iter()
            .filter_map(|&tile| self.tile_unit(tile))
            .collect();
        let paths: Vec<(UnitId, Path)> = key
            .shown
            .iter()
            .filter_map(|&tile| self.tile_unit(tile))
            .chain(
                key.disclosed
                    .numbered()
                    .filter_map(|(span, _)| track.unit_id(span)),
            )
            .map(|id| {
                let least = usize::from(changed.contains(id));
                (id.clone(), self.unit_path(id).at_least(least))
            })
            .collect();
        self.names = Names::apart(paths);
        self.names_for = Some(key);
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
        Self::label_named(track, id, |unit| self.unit_name(unit))
    }

    /// A span's label with its function's name alone, nothing in front that tells it
    /// apart, for where the whole label does not fit.
    pub(crate) fn short_label_in(&self, track: &Track, id: SpanId) -> String {
        Self::label_named(track, id, |unit| self.unit_path(unit).short())
    }

    /// A span's label, its function called by `name`.
    fn label_named(track: &Track, id: SpanId, name: impl Fn(&UnitId) -> String) -> String {
        let Some(span) = track.span(id) else {
            return String::new();
        };
        let name = match &span.target {
            SpanTarget::Unit(unit) => name(unit),
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

    /// What a unit is called on screen: told apart from what else is in view, or,
    /// when it is not in view, from everything.
    pub(crate) fn unit_name(&self, id: &UnitId) -> String {
        self.names
            .get(id)
            .or_else(|| self.every_name.get(id))
            .map_or_else(|| self.unit_path(id).short(), str::to_owned)
    }

    /// A unit's path, for naming it.
    pub(crate) fn unit_path(&self, id: &UnitId) -> Path {
        Path::of(|id| self.any_unit(id), id)
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

    /// `GET -> get -> find` and `POST -> post -> place -> (find, quote)`, with the two
    /// routes declared entry points, and `main -> router -> (get, post)` above them.
    pub(crate) fn routed() -> Explorer {
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
        let units = [
            unit("main", &["router"]),
            unit("router", &["get", "post"]),
            unit("get", &["find"]),
            unit("post", &["place"]),
            unit("place", &["find", "quote"]),
            unit("find", &[]),
            unit("quote", &[]),
        ];
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }},
                "entries": [
                  {{ "unit": "x:get", "label": "GET /users", "kind": "http" }},
                  {{ "unit": "x:post", "label": "POST /orders", "kind": "http" }}
                ],
                "units": [{}] }}"#,
            units.join(",")
        );
        let system = System::new([Document::from_json(&json).unwrap()]);
        Explorer::new(system, None, JourneyOptions::default())
    }

    fn tile_named(explorer: &Explorer, name: &str) -> usize {
        explorer
            .map()
            .tile_of(&UnitId::new(format!("x:{name}")))
            .unwrap()
    }

    #[test]
    fn the_spotlight_widens_a_call_at_a_time_up_to_the_entries() {
        let mut explorer = routed();
        let [find, get, post, place, quote] =
            ["find", "get", "post", "place", "quote"].map(|name| tile_named(&explorer, name));
        explorer.open_at(find);
        let sorted = |mut tiles: Vec<usize>| {
            tiles.sort_unstable();
            tiles
        };

        explorer.widen_spotlight(Side::Callers, 0.0);
        assert_eq!(explorer.spotlit(), sorted(vec![get, place]), "its callers");
        explorer.widen_spotlight(Side::Callers, 1.0);
        assert_eq!(
            explorer.spotlit(),
            sorted(vec![get, place, post]),
            "and theirs, up to the entries but not past them to the router"
        );
        assert_eq!(
            explorer.spotlight().map(|s| s.reach),
            Some(None),
            "nothing is further, so it goes all the way"
        );
        assert_eq!(explorer.spotlight_hops(post), Some(2));
        explorer.widen_spotlight(Side::Callers, 2.0);
        assert_eq!(explorer.spotlight(), None, "past all the way, it goes out");

        explorer.spotlight_all(Side::Callers, 3.0);
        assert_eq!(explorer.spotlit().len(), 3, "all the way at once");
        explorer.settle_visibility();
        assert!(
            explorer.shown(post) && explorer.shown(find),
            "the routes and the focus show"
        );
        assert!(
            !explorer.shown(tile_named(&explorer, "router")),
            "and the rest steps out of their way"
        );
        let starts: Vec<(String, u32)> = explorer
            .route_starts(find)
            .iter()
            .map(|s| (s.label.clone(), s.hops))
            .collect();
        assert_eq!(
            starts,
            [("GET /users".to_owned(), 1), ("POST /orders".to_owned(), 2)]
        );
        explorer.spotlight_all(Side::Callers, 4.0);
        assert_eq!(explorer.spotlight(), None);

        explorer.open_at(tile_named(&explorer, "main"));
        explorer.spotlight_all(Side::Callers, 4.5);
        assert_eq!(explorer.spotlight(), None, "nothing calls main");

        explorer.widen_spotlight(Side::Callees, 5.0);
        explorer.open_at(post);
        assert!(explorer.spotlight().is_none(), "a new focus starts unlit");
        explorer.spotlight_all(Side::Callees, 6.0);
        assert_eq!(
            explorer.spotlit(),
            sorted(vec![place, find, quote]),
            "everything it can run"
        );
    }

    #[test]
    fn past_two_calls_the_spotlight_goes_all_the_way() {
        let unit = |name: &str, callee: Option<&str>| {
            let calls = callee.map_or_else(String::new, |c| {
                format!(
                    r#"{{ "key": "{c}#0", "text": "{c}", "targets": [{{ "unit": "x:{c}", "certainty": "exact" }}] }}"#
                )
            });
            format!(
                r#"{{ "id": "x:{name}", "name": "{name}", "kind": "fn", "role": "invocable", "calls": [{calls}] }}"#
            )
        };
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}, {}, {}, {}, {}] }}"#,
            unit("a", Some("b")),
            unit("b", Some("c")),
            unit("c", Some("d")),
            unit("d", Some("e")),
            unit("e", None)
        );
        let system = System::new([Document::from_json(&json).unwrap()]);
        let mut explorer = Explorer::new(system, None, JourneyOptions::default());
        explorer.open_at(tile_named(&explorer, "e"));
        let reaches = |explorer: &mut Explorer| {
            explorer.widen_spotlight(Side::Callers, 0.0);
            explorer.spotlight().map(|s| s.reach)
        };
        assert_eq!(reaches(&mut explorer), Some(Some(1)));
        assert_eq!(reaches(&mut explorer), Some(Some(2)));
        assert_eq!(
            reaches(&mut explorer),
            Some(None),
            "then every route, a to e"
        );
        assert_eq!(explorer.spotlit().len(), 4);
        assert_eq!(reaches(&mut explorer), None, "then off");
    }

    #[test]
    fn following_a_route_opens_its_journey_on_the_focus() {
        let mut explorer = routed();
        let (find, post) = (tile_named(&explorer, "find"), tile_named(&explorer, "post"));
        explorer.open_at(find);
        explorer.spotlight_all(Side::Callers, 0.0);
        explorer.follow_route(post);
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:post"));
        assert_eq!(at(&explorer), "find", "the cursor is on the focus");
        assert_eq!(explorer.graph_focus(), Some(find), "which stays in focus");
        assert!(
            explorer
                .disclosed_tiles()
                .get(&find)
                .is_some_and(|numbers| numbers == &["1.1".to_owned()]),
            "the calls on the way there are disclosed"
        );
        assert!(
            explorer.spotlight().is_some(),
            "the other routes stay picked out"
        );
        assert_eq!(explorer.followed_start(), Some(post));

        let get = tile_named(&explorer, "get");
        assert!(explorer.follow_next_route(true, 1.0));
        assert_eq!(
            explorer.root().map(UnitId::as_str),
            Some("x:get"),
            "round to the first"
        );
        assert_eq!(explorer.followed_start(), Some(get));
        assert!(explorer.follow_next_route(false, 2.0));
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:post"));
        assert_eq!(at(&explorer), "find");
        explorer.go_back();
        assert_eq!(
            explorer.root().map(UnitId::as_str),
            Some("x:find"),
            "back past every route followed"
        );
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
        assert!(explorer.go_back());
        explorer.refresh_reach();
        assert_eq!(explorer.graph_focus(), Some(a));
    }

    #[test]
    fn the_focus_reaches_up_its_callers() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");

        explorer.refresh_reach();
        let tile = |name: &str| explorer.map().tile_of(&UnitId::new(name)).unwrap();
        let reach = explorer.reach().unwrap();
        assert_eq!(reach.upstream.get(&tile("x:a")), Some(&1));
        assert_eq!(reach.upstream.get(&tile("x:main")), Some(&2));
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
        explorer.trace_from_focus();
        assert_eq!(explorer.journey().map(Journey::len), Some(2));
        assert!(explorer.can_go_back());

        explorer.go_back();
        assert_eq!(explorer.journey().map(Journey::len), Some(4));
        assert_eq!(at(&explorer), "a", "back where the cursor was");
        assert!(!explorer.can_go_back());
        assert!(explorer.can_go_forward());

        explorer.go_forward();
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:a"));
        assert!(!explorer.can_go_forward());
    }

    fn tile(explorer: &Explorer, name: &str) -> usize {
        explorer
            .map()
            .tile_of(&UnitId::new(format!("x:{name}")))
            .unwrap()
    }

    #[test]
    fn going_back_puts_the_whole_view_back() {
        let mut explorer = explorer();
        let main = explorer.journey().unwrap().root();
        let a_span = explorer.journey().unwrap().span(main).unwrap().children[0];
        explorer.set_disclose(Disclose::Depth(2));
        explorer.toggle_expanded(a_span);
        explorer.step(Step::Into);
        explorer.step(Step::Into);
        explorer.widen_spotlight(Side::Callers, 1.0);
        let before = explorer.view();

        explorer.open_at(tile(&explorer, "b"));
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:b"));
        assert_eq!(explorer.disclose(), Disclose::default(), "b opens fresh");
        assert!(explorer.spotlight().is_none());

        assert!(explorer.go_back());
        assert_eq!(explorer.view(), before);
        assert_eq!(at(&explorer), "c", "the cursor where it was");
        assert_eq!(explorer.disclose(), Disclose::Depth(2));
        assert!(explorer.disclosed(a_span), "a still opened by hand");
        assert_eq!(
            explorer.spotlight().map(|s| s.side),
            Some(Side::Callers),
            "the spotlight back on"
        );
        assert!(!explorer.go_back(), "and that was the first place");
    }

    #[test]
    fn back_and_forward_never_loop() {
        let mut explorer = explorer();
        for name in ["a", "c", "b"] {
            explorer.open_at(tile(&explorer, name));
        }
        let root = |explorer: &Explorer| explorer.root().map(|u| u.as_str().to_owned());
        let mut seen = Vec::new();
        while explorer.go_back() {
            seen.push(root(&explorer));
        }
        assert_eq!(
            seen,
            ["x:c", "x:a", "x:main"].map(|r| Some(r.to_owned())),
            "each place once, newest first"
        );
        let mut ahead = Vec::new();
        while explorer.go_forward() {
            ahead.push(root(&explorer));
        }
        assert_eq!(ahead, ["x:a", "x:c", "x:b"].map(|r| Some(r.to_owned())));
    }

    #[test]
    fn only_moves_are_remembered() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.set_disclose(Disclose::Depth(3));
        explorer.widen_spotlight(Side::Callees, 0.0);
        assert!(
            !explorer.can_go_back(),
            "stepping and showing more are not moves"
        );

        let main = tile(&explorer, "main");
        explorer.open_at(main);
        assert!(
            explorer.can_go_back(),
            "opening the root again takes the cursor back to it"
        );
        explorer.go_back();
        explorer.open_at(tile(&explorer, "a"));
        explorer.open_at(tile(&explorer, "a"));
        explorer.go_back();
        assert_eq!(
            explorer.root().map(UnitId::as_str),
            Some("x:main"),
            "opening the same node twice is one move"
        );
    }

    #[test]
    fn digits_open_the_calls_as_numbered_on_screen() {
        // main's first call could run either b or c; its second runs d.
        let json = r#"{ "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" }, "units": [
            { "id": "x:main", "name": "main", "kind": "fn", "role": "invocable", "calls": [
                { "key": "dyn#0", "text": "handler.run", "targets": [
                    { "unit": "x:b", "certainty": "exact" }, { "unit": "x:c", "certainty": "exact" } ] },
                { "key": "d#0", "text": "d", "targets": [ { "unit": "x:d", "certainty": "exact" } ] } ] },
            { "id": "x:b", "name": "b", "kind": "fn", "role": "invocable" },
            { "id": "x:c", "name": "c", "kind": "fn", "role": "invocable" },
            { "id": "x:d", "name": "d", "kind": "fn", "role": "invocable" } ] }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let mut explorer = Explorer::new(
            system,
            Some(UnitId::new("x:main")),
            JourneyOptions::default(),
        );
        let badge = |explorer: &Explorer, name: &str| {
            explorer
                .disclosed_tiles()
                .get(&tile(explorer, name))
                .cloned()
                .unwrap_or_default()
        };
        assert_eq!(badge(&explorer, "d"), ["2"]);

        explorer.open_numbered(2);
        assert_eq!(
            explorer.root().map(UnitId::as_str),
            Some("x:d"),
            "2 opens the node badged 2, not the second function written"
        );
        explorer.go_back();
        explorer.open_numbered(1);
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:main"));
        assert!(
            explorer
                .label(explorer.selected().unwrap())
                .contains("one of 2"),
            "a call to one of several functions has no node, so the cursor goes to its bar"
        );
        explorer.open_numbered(9);
        assert!(
            explorer
                .label(explorer.selected().unwrap())
                .contains("one of 2"),
            "no 9th call"
        );
    }

    #[test]
    fn the_cursor_opens_what_it_is_folded_under() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        let a = explorer.selected().unwrap();
        assert_eq!(explorer.disclosure().folded(a), 1, "c is folded under a");
        explorer.step(Step::Child);
        assert_eq!(at(&explorer), "c");
        assert_eq!(
            explorer.disclosure().number(explorer.selected().unwrap()),
            Some("1.1"),
            "and shows, numbered, once the cursor is on it"
        );
        explorer.toggle_expanded(a);
        assert!(
            !explorer.disclosed(explorer.selected().unwrap()),
            "folding it again by hand still folds it"
        );
    }

    #[test]
    fn each_journey_keeps_its_own_opened_calls() {
        let mut explorer = explorer();
        let main = explorer.journey().unwrap().root();
        let a = explorer.journey().unwrap().span(main).unwrap().children[0];
        let c = explorer.journey().unwrap().span(a).unwrap().children[0];
        explorer.toggle_expanded(a);
        assert!(explorer.disclosed(c));

        explorer.add_track(UnitId::new("x:main"));
        assert!(
            !explorer.disclosed(c),
            "the same journey opened again starts folded"
        );
        explorer.select_in(SpanRef {
            track: 0,
            span: main,
        });
        assert!(explorer.disclosed(c), "and the first keeps what it opened");
    }

    #[test]
    fn the_spotlight_goes_out_when_the_focus_moves() {
        let mut explorer = explorer();
        explorer.widen_spotlight(Side::Callees, 0.0);
        let mut callees = vec![tile(&explorer, "a"), tile(&explorer, "b")];
        callees.sort_unstable();
        assert_eq!(explorer.spotlit(), callees);
        explorer.step(Step::Into);
        assert!(explorer.spotlight().is_none());
        explorer.step(Step::Back);
        assert!(
            explorer.spotlight().is_none(),
            "and stays out on coming back"
        );
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

        assert!(explorer.go_back());
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:a"));
    }
}
