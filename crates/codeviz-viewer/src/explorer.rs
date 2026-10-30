//! Viewer state that is independent of rendering: the loaded system, the journey
//! and its cursor, and the codebase as a graph of calls with the node at
//! the center of attention and what reaches it.
//!
//! The view is in one of two modes. Browsing, no journey is open: a click selects a
//! function and moves nothing. In a journey, the view gathers round where it starts
//! once, on entering it, and a click on a function the journey runs moves its cursor
//! there; a click on any other selects it in place.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, PoisonError},
};

use bevy::prelude::*;
use codeviz_core::{
    Journey, JourneyOptions, Level, Scenario, Span, SpanId, SpanTarget, StopReason, System,
    display_name, scenarios,
};
use codeviz_ir::{Certainty, ExitKind, Location, Mode, Unit, UnitId};
use codeviz_layout::{CodebaseMap, ForceConfig, ForceGraph, LayoutConfig, MapConfig, Reach, Scene};

use crate::{
    camera::Pose,
    disclosure::{self, Disclose, Disclosed},
    history::{History, Seen, View},
    lens::{self, Distances, Lens},
    nucleus::{Nucleus, Trail},
};

/// What a disclosure was worked out for: the journey's revision, how it discloses,
/// the spans opened by hand, and the cursor when that matters.
type DisclosureKey = (u64, Disclose, u64, Option<SpanId>);

/// What the nucleus's trail was worked out for: the journey's revision, how it
/// discloses, the spans opened, the cursor when that matters, the middle, and whether
/// the view browses.
type TrailKey = (u64, Disclose, u64, Option<SpanId>, Option<usize>, bool);

/// What the lens's distances were worked out for: where it starts, how it measures,
/// and how often calls were retired and, when measuring in space, nodes moved.
type DistanceKey = (Vec<usize>, lens::Source, lens::Measure, u64, u64);

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

/// The journey: where it starts, which outcome it shows, its cursor, and its layout
/// in time and depth.
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
    /// The color of each of the scene's lanes, by module.
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

    /// The color index for a span's bar: its module's.
    pub(crate) fn color(&self, id: SpanId) -> usize {
        self.scene
            .nodes
            .get(id.index())
            .and_then(|placed| self.colors.get(placed.lane))
            .copied()
            .unwrap_or_default()
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

/// Simulation steps run before the graph is first shown, so it opens nearly settled.
const PRESETTLE_STEPS: usize = 200;
/// At most this many steps to settle at once.
const SETTLE_NOW_STEPS: usize = 600;
/// Below this mean speed the graph counts as settled and stops simulating.
const SETTLED_SPEED: f32 = 0.002;

/// Whether a journey is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewMode {
    /// No journey: a click selects a function and moves nothing.
    Browse,
    /// A journey is open, isolated, its cursor moved by a click on what it runs.
    Journey,
}

/// Either side of the focus: what calls it, or what it calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Callers,
    Callees,
}

/// Which sides of the focus are hidden, with their calls: what calls it (Shift+I),
/// what it calls (Shift+O), each with what is further along that way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Hidden {
    pub(crate) callers: bool,
    pub(crate) callees: bool,
}

impl Hidden {
    pub(crate) fn hides(self, side: Side) -> bool {
        match side {
            Side::Callers => self.callers,
            Side::Callees => self.callees,
        }
    }

    /// The same with one side hidden if it showed, or shown if it was hidden.
    #[must_use]
    pub(crate) fn toggled(self, side: Side) -> Self {
        match side {
            Side::Callers => Self {
                callers: !self.callers,
                ..self
            },
            Side::Callees => Self {
                callees: !self.callees,
                ..self
            },
        }
    }
}

#[derive(Resource, Debug)]
pub(crate) struct Explorer {
    system: System,
    /// What the graph lays out, when wider than `system`.
    layout_system: Option<System>,
    pub(crate) options: JourneyOptions,
    pub(crate) layout: LayoutConfig,
    /// The journey the cursor keys, the panel and the outline follow.
    track: Track,
    /// Every function, grouped by container; graph nodes are its tiles.
    map: CodebaseMap,
    /// The codebase as a graph of calls.
    graph: ForceGraph,
    clustering: Clustering,
    /// Whether the graph is still moving.
    settling: bool,
    /// Map tiles the journey runs, with the color each is drawn in there.
    journey_tiles: HashMap<usize, usize>,
    /// What reaches the focus, and what it reaches.
    reach: Option<(usize, Reach)>,
    /// How much of the graph shows.
    lens: Lens,
    distance_cache: Mutex<Option<(DistanceKey, Option<Arc<Distances>>)>>,
    /// Bumped whenever nodes move.
    moved: u64,
    /// Bumped whenever calls are retired.
    rewired: u64,
    /// How fully each node shows, by tile, eased towards what the lens says.
    visibility: Vec<f32>,
    /// A diff's changed nodes, for a lens that starts from them.
    changed_tiles: Vec<usize>,
    /// How much of the journey shows.
    disclose: Disclose,
    /// Bumped whenever the journey's opened spans change.
    disclosure_epoch: u64,
    disclosure_cache: Mutex<Option<(DisclosureKey, Arc<Disclosed>)>>,
    /// Where the view has been, to go back and forward.
    history: History<View>,
    /// Bumped whenever the view is pointed somewhere by hand, so the camera turns to
    /// it even when that is where it already was.
    aim: u64,
    /// Whether the cursor is being walked with the keys, its start already
    /// remembered, so the rest of the walk is not.
    walking: bool,
    /// The node under the pointer, in the graph or as a bar in the journey.
    pub(crate) hovered_tile: Option<usize>,
    /// A node clicked off the journey: the focus while it is set, until a journey key,
    /// Esc or Enter, which follows it.
    aside: Option<usize>,
    /// What calls the focus, or what it calls, picked out and brought into view: when,
    /// in seconds of app time, and for which node.
    spotlight: Option<(Side, f32, usize)>,
    /// Lane label to color index, in order of first appearance.
    lane_colors: HashMap<String, usize>,
    /// Bumped whenever the journey is rebuilt.
    revision: u64,
    /// The graph gathered round the focus's module, when it is.
    nucleus: Option<Nucleus>,
    /// What the nucleus's trail was last worked out for.
    traced: Option<TrailKey>,
    /// Which sides of the focus are hidden.
    hidden: Hidden,
    /// Whether a journey is open, or the view browses.
    mode: ViewMode,
    /// Browsing, the function last clicked: what the view lights, and what Enter opens
    /// the journey of.
    selection: Option<usize>,
    /// In a journey, whether every outside caller into it shows (I).
    outside: bool,
    /// The journey number on the name last pointed at, so a click on a name goes to
    /// the call it numbers.
    pub(crate) pointed_number: Option<String>,
    /// Something said for a moment, as why a key moved nothing, numbered.
    notice: Option<(u64, String)>,
    /// Where the camera stands, as last seen.
    pose: Option<Pose>,
    /// Where the camera stood browsing when a journey was last entered from there.
    browse_pose: Option<Pose>,
    /// Where the camera is to go back to, numbered: a pose it had, or with none, the
    /// whole codebase.
    return_pose: Option<(u64, Option<Pose>)>,
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
        let root = root.or_else(|| system.entries().first().map(|e| e.unit.clone()));
        let placed = layout.as_ref().unwrap_or(&system);
        let map = codeviz_layout::map(placed, &MapConfig::default());
        let clustering = Clustering::default();
        let mut graph = ForceGraph::new(placed, &map, clustering.forces());
        for _ in 0..PRESETTLE_STEPS {
            graph.step();
        }
        let tiles = map.tiles.len();
        let mut explorer = Self {
            system,
            layout_system: layout,
            options,
            layout: LayoutConfig::default(),
            track: Track::default(),
            map,
            graph,
            clustering,
            settling: true,
            journey_tiles: HashMap::new(),
            reach: None,
            lens: Lens::default(),
            distance_cache: Mutex::new(None),
            moved: 0,
            rewired: 0,
            visibility: vec![1.0; tiles],
            changed_tiles: Vec::new(),
            history: History::default(),
            aim: 0,
            walking: false,
            disclose: Disclose::default(),
            disclosure_epoch: 0,
            disclosure_cache: Mutex::new(None),
            hovered_tile: None,
            aside: None,
            spotlight: None,
            lane_colors: HashMap::new(),
            revision: 0,
            nucleus: None,
            traced: None,
            hidden: Hidden::default(),
            mode: ViewMode::Journey,
            selection: None,
            outside: false,
            pointed_number: None,
            notice: None,
            pose: None,
            browse_pose: None,
            return_pose: None,
        };
        explorer.color_groups();
        if root.is_none() {
            explorer.mode = ViewMode::Browse;
        }
        if let Some(root) = root {
            explorer.replace_root(root);
        }
        explorer
    }

    pub(crate) fn system(&self) -> &System {
        &self.system
    }

    /// A unit of the journey's system, or else of the wider one the graph lays out.
    pub(crate) fn any_unit(&self, id: &UnitId) -> Option<&Unit> {
        self.system
            .unit(id)
            .or_else(|| self.layout_system.as_ref()?.unit(id))
    }

    /// Moves every node to a given place, and stops the simulation moving them.
    pub(crate) fn place_nodes(&mut self, positions: &[[f32; 3]]) {
        self.graph.positions.clone_from_slice(positions);
        self.settling = false;
        self.moved += 1;
    }

    /// Calls the graph still lays out but that no longer run, as a diff's removed
    /// calls: they stop counting as callers and callees.
    pub(crate) fn retire_calls(&mut self, gone: &HashSet<(usize, usize)>) {
        self.graph.retire(gone);
        self.reach = None;
        self.rewired += 1;
    }

    /// The journey.
    pub(crate) fn track(&self) -> &Track {
        &self.track
    }

    fn track_mut(&mut self) -> &mut Track {
        &mut self.track
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
            root: self.root().cloned(),
            scenario: self.scenario(),
            cursor: self.selected(),
            disclose: self.disclose,
            expanded: self.track().expanded.clone(),
            spotlight: self.spotlight,
            aside: self.aside,
            hidden: self.hidden,
            specks: self.nucleus.as_ref().is_some_and(Nucleus::specks_hidden),
            mode: self.mode,
            selection: self.selection,
            outside: self.outside,
            pose: Seen(self.pose),
        }
    }

    /// Puts the view back as it was, all of it: the journey, its outcome and cursor,
    /// how much of it shows, the function clicked, the spotlight and what was hidden.
    /// Gathered round the focus's module, the nucleus eases back round the function
    /// it was, and the camera back to where it stood.
    fn restore(&mut self, view: View) {
        self.leaving_browse(view.mode);
        if let Some(pose) = view.pose.0 {
            self.return_to(Some(pose));
        }
        let track = &mut self.track;
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
        self.aside = view.aside;
        self.mode = view.mode;
        self.selection = view.selection;
        self.outside = view.outside;
        if let Some(nucleus) = &mut self.nucleus {
            nucleus.set_outside(&self.graph, self.outside);
        }
        if self.hidden != view.hidden {
            self.hidden = view.hidden;
            if let Some(nucleus) = &mut self.nucleus {
                nucleus.set_hidden(&self.graph, self.hidden);
            }
        }
        if let Some(nucleus) = &mut self.nucleus
            && nucleus.specks_hidden() != view.specks
        {
            nucleus.toggle_specks();
        }
        self.walking = false;
        self.moved += 1;
        self.recolor();
    }

    /// Goes somewhere: whatever `go` changes, the view before it is remembered, so
    /// Back returns to it. A move that changes nothing is not remembered.
    fn navigate(&mut self, go: impl FnOnce(&mut Self)) {
        let before = self.view();
        go(self);
        if before.mode == ViewMode::Browse && self.mode == ViewMode::Journey {
            self.browse_pose = before.pose.0;
        }
        if self.view() != before {
            self.history.leave(before);
            self.walking = false;
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

    /// The span the journey's cursor is on.
    pub(crate) fn selected(&self) -> Option<SpanId> {
        self.track().selected
    }

    /// Moves the cursor, and the focus with it. The spotlight was on the focus
    /// it leaves, so it goes out; the calls folded over where it lands open, so its bar
    /// shows.
    pub(crate) fn select(&mut self, span: Option<SpanId>) {
        let before = self.graph_focus();
        self.aside = None;
        self.track_mut().selected = span;
        if self.graph_focus() != before {
            self.spotlight = None;
        }
        self.reveal_cursor();
    }

    /// Moves the cursor to a call picked by hand, a bar or a digit, remembering where
    /// it was; stepping with the keys moves it without.
    pub(crate) fn go_to(&mut self, span: SpanId) {
        self.navigate(|explorer| explorer.select(Some(span)));
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

    /// Starts the journey at `root`, remembering where it was.
    pub(crate) fn set_root(&mut self, root: UnitId) {
        self.navigate(|explorer| {
            explorer.mode = ViewMode::Journey;
            explorer.replace_root(root);
        });
        self.aim += 1;
    }

    /// Whether the view browses, with no journey open.
    pub(crate) fn browsing(&self) -> bool {
        self.mode == ViewMode::Browse
    }

    /// Browsing, the function selected.
    pub(crate) fn selection(&self) -> Option<usize> {
        self.selection.filter(|_| self.browsing())
    }

    /// Leaves the journey for browsing, with the function its cursor is on selected,
    /// remembering where it was.
    pub(crate) fn leave_journey(&mut self) {
        if self.browsing() {
            return;
        }
        self.navigate(|explorer| {
            explorer.selection = explorer.graph_focus();
            explorer.mode = ViewMode::Browse;
            explorer.aside = None;
            explorer.spotlight = None;
        });
        self.aim += 1;
        // Back to the codebase as it was seen before the journey, unframed.
        self.return_to(self.browse_pose);
    }

    /// Remembers where the camera stood browsing, when the view is about to go from
    /// browsing into a journey (`next`).
    fn leaving_browse(&mut self, next: ViewMode) {
        if self.mode == ViewMode::Browse && next == ViewMode::Journey {
            self.browse_pose = self.pose;
        }
    }

    /// Sends the camera back to `pose`, or with none, to the whole codebase.
    fn return_to(&mut self, pose: Option<Pose>) {
        let count = self.return_pose.map_or(0, |(n, _)| n + 1);
        self.return_pose = Some((count, pose));
    }

    /// Where the camera is to go back to, numbered, once each time it is asked.
    pub(crate) fn return_pose(&self) -> Option<(u64, Option<Pose>)> {
        self.return_pose
    }

    /// Notes where the camera stands, for the history and for leaving a journey.
    pub(crate) fn see(&mut self, pose: Pose) {
        self.pose = Some(pose);
    }

    /// In a journey, whether every outside caller into it shows.
    pub(crate) fn outside_shown(&self) -> bool {
        self.outside && !self.browsing()
    }

    /// In a journey, shows every function that calls one on it from off it, or leaves
    /// them out again; nothing already there moves.
    pub(crate) fn toggle_outside(&mut self) {
        self.outside = !self.outside;
        if let Some(nucleus) = &mut self.nucleus {
            nucleus.set_outside(&self.graph, self.outside);
            self.moved += 1;
        }
    }

    /// What I does: in a journey gathered round its start, brings in every outside
    /// caller into it; else picks out the focus's callers.
    pub(crate) fn press_callers(&mut self, now: f32) {
        if self.nucleus.is_some() && !self.browsing() {
            self.toggle_outside();
        } else {
            self.toggle_spotlight(Side::Callers, now);
        }
    }

    /// Starts the journey at `root` on its happy path, with the cursor on it.
    fn replace_root(&mut self, root: UnitId) {
        self.aside = None;
        if self.root() == Some(&root) {
            return;
        }
        self.track_mut().root = Some(root);
        // A new journey brings in no one from outside it until asked.
        self.outside = false;
        self.reroot();
    }

    /// Recomputes outcomes for the journey's root and opens on the happy path, with
    /// the cursor on the root.
    fn reroot(&mut self) {
        self.track.reroot(&self.system, &self.options, &self.layout);
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

    /// Follows the focus: the journey starts there, whether the focus is the cursor's
    /// node, one clicked off the journey or the function selected browsing,
    /// remembering where it was.
    pub(crate) fn trace_from_focus(&mut self) {
        let Some(unit) = self.graph_focus().and_then(|t| self.tile_unit(t)).cloned() else {
            return;
        };
        self.navigate(|explorer| {
            explorer.mode = ViewMode::Journey;
            if explorer.root() == Some(&unit) {
                explorer.aside = None;
                explorer.select(explorer.journey().map(Journey::root));
            } else {
                explorer.replace_root(unit);
            }
        });
        self.aim += 1;
    }

    pub(crate) fn toggle_zoom(&mut self, unit: UnitId) {
        let level = match self.options.zoom.level(&unit) {
            Level::Summary => Level::Detail,
            Level::Detail => Level::Summary,
        };
        self.options.zoom.set(unit, level);
        self.reroot();
    }

    /// Rebuilds the journey.
    pub(crate) fn rebuild(&mut self) {
        self.track
            .rebuild(&self.system, &self.options, &self.layout);
        self.recolor();
    }

    /// The codebase as a graph of calls.
    pub(crate) fn graph(&self) -> &ForceGraph {
        &self.graph
    }

    /// Where a graph node sits in the world: in its module's sphere when the graph
    /// is gathered round the focus's module, else where the simulation put it.
    pub(crate) fn tile_point(&self, tile: usize) -> Option<Vec3> {
        match &self.nucleus {
            Some(nucleus) => nucleus.point(tile),
            None => self.graph.positions.get(tile).map(|&p| Vec3::from(p)),
        }
    }

    /// Bumped whenever nodes move.
    pub(crate) fn moved(&self) -> u64 {
        self.moved
    }

    /// The graph gathered round the focus's module, when it is.
    pub(crate) fn nucleus(&self) -> Option<&Nucleus> {
        self.nucleus.as_ref()
    }

    /// Gathers the graph round the focus's module, or lays it flat again. The
    /// nucleus keeps its module's own map from the flat graph, so the graph settles
    /// first.
    pub(crate) fn set_nucleus(&mut self, on: bool) {
        if on == self.nucleus.is_some() {
            return;
        }
        self.nucleus = if on {
            self.settle_now();
            let focus = self.nucleus_focus();
            self.traced = Some(self.trail_key(focus));
            let mut nucleus = Nucleus::gather(
                &self.graph,
                focus,
                self.hidden,
                self.trail_from(focus),
                self.browsing(),
            );
            nucleus.set_outside(&self.graph, self.outside);
            while nucleus.advance(1.0) {}
            Some(nucleus)
        } else {
            None
        };
        self.moved += 1;
    }

    /// In a journey, expands the nucleus to show how calls run inside its start's
    /// module, or shrinks it back, round the start again; gathers the graph round that
    /// module first when it lies flat.
    pub(crate) fn expand_nucleus(&mut self, on: bool) {
        if self.browsing() {
            return;
        }
        if on {
            self.set_nucleus(true);
        }
        if let Some(nucleus) = &mut self.nucleus {
            nucleus.expand(&self.graph, on);
            self.moved += 1;
        }
    }

    /// Tells the nucleus which function the pointer has settled on. Returns whether
    /// that changed.
    pub(crate) fn hover_nucleus(&mut self, tile: Option<usize>) -> bool {
        self.nucleus
            .as_mut()
            .is_some_and(|nucleus| nucleus.hover(&self.graph, tile))
    }

    /// The function under the pointer as drawn: gathered round the focus's module,
    /// the one the pointer has settled on, so passing over a function does not flash.
    pub(crate) fn pointed(&self) -> Option<usize> {
        match &self.nucleus {
            Some(nucleus) => nucleus.hovered(),
            None => self.hovered_tile,
        }
    }

    /// The function the nucleus is arranged round: browsing, the one selected; in a
    /// journey, its start, expanded or not, so it shows the journey and nothing else.
    /// Stepping through the journey, or a click anywhere, leaves it put.
    pub(crate) fn nucleus_focus(&self) -> Option<usize> {
        if self.browsing() {
            return self.selection;
        }
        self.root_tile().or_else(|| self.graph_focus())
    }

    /// The node drawn as the focus, white, whose callers and callees light up: the
    /// nucleus's middle when the graph is gathered round it, else the focus.
    pub(crate) fn lit_focus(&self) -> Option<usize> {
        match &self.nucleus {
            Some(nucleus) => nucleus.focus(),
            None => self.graph_focus(),
        }
    }

    /// Gathered round a journey, the journey's cursor, the one pink node, marked
    /// wherever it sits, the middle included.
    pub(crate) fn cursor_mark(&self) -> Option<usize> {
        self.nucleus.as_ref()?;
        if self.browsing() {
            return None;
        }
        self.focus().and_then(|unit| self.map.tile_of(unit))
    }

    /// What the nucleus's trail from `centre` depends on.
    fn trail_key(&self, centre: Option<usize>) -> TrailKey {
        (
            self.revision,
            self.disclose,
            self.disclosure_epoch,
            (self.disclose == Disclose::Time)
                .then(|| self.selected())
                .flatten(),
            centre,
            self.browsing(),
        )
    }

    /// The open journey from `centre`, as the nucleus names it: from the journey's
    /// start when that is the centre, else from the first run of it the journey shows,
    /// or runs at all; when the journey never runs it, its own calls, and the open
    /// journey's functions so they stay in view.
    fn trail_from(&self, centre: Option<usize>) -> Trail {
        let (Some(centre), Some(journey)) = (centre, self.journey()) else {
            return Trail::default();
        };
        if self.browsing() {
            return Trail::default();
        }
        let disclosed = self.disclosure();
        let start = if self.root_tile() == Some(centre) {
            Some(journey.root())
        } else {
            let runs: Vec<SpanId> = journey
                .spans()
                .map(|(id, _)| id)
                .filter(|&id| {
                    self.track.unit_id(id).and_then(|u| self.map.tile_of(u)) == Some(centre)
                })
                .collect();
            runs.iter()
                .copied()
                .find(|&span| disclosed.shows(span))
                .or_else(|| runs.first().copied())
        };
        match start {
            Some(start) => {
                disclosure::trail(&self.track, &self.map, start, self.disclose, &disclosed)
            }
            None => self.trail_aside(centre, &disclosed),
        }
    }

    /// For a centre off the journey: the calls it makes, in the order written, each
    /// numbered as the panel numbers it where the open journey runs it too, and the
    /// open journey's functions.
    fn trail_aside(&self, centre: usize, disclosed: &Disclosed) -> Trail {
        let shown = disclosed.tiles();
        let mut trail = Trail {
            aside: true,
            journey: shown.keys().copied().collect(),
            ..Trail::default()
        };
        let written = self.graph.sequence(centre);
        let callees: Vec<usize> = if written.is_empty() {
            self.graph.callees(centre).collect()
        } else {
            written.to_vec()
        };
        for callee in callees.into_iter().filter(|&t| t != centre) {
            let numbers = shown.get(&callee).map_or(&[][..], Vec::as_slice);
            trail.add_own(centre, callee, numbers);
        }
        trail
    }

    /// Which sides of the focus are hidden.
    pub(crate) fn hidden(&self) -> Hidden {
        self.hidden
    }

    /// Hides one side of the focus, with its calls, or shows it again.
    pub(crate) fn toggle_hidden(&mut self, side: Side) {
        self.hidden = self.hidden.toggled(side);
        if let Some(nucleus) = &mut self.nucleus {
            nucleus.set_hidden(&self.graph, self.hidden);
            self.moved += 1;
        }
    }

    /// Hides the nucleus's nameless specks, or shows them again, in the view it is in.
    pub(crate) fn toggle_specks(&mut self) {
        if let Some(nucleus) = &mut self.nucleus {
            nucleus.toggle_specks();
        }
    }

    /// Re-centers the nucleus when the function it is arranged round changes, marks the
    /// journey's cursor where it sits, and eases it along by `dt` seconds. Returns
    /// whether what it shows changed, rather than only moved.
    pub(crate) fn ease_nucleus(&mut self, dt: f32) -> bool {
        if self.nucleus.is_none() {
            return false;
        }
        let focus = self.nucleus_focus();
        let cursor = self.cursor_mark();
        let key = self.trail_key(focus);
        let trail = (self.traced != Some(key)).then(|| self.trail_from(focus));
        let browsing = self.browsing();
        let Some(nucleus) = &mut self.nucleus else {
            return false;
        };
        let mut recentered = nucleus.set_browse(&self.graph, browsing);
        nucleus.set_outside(&self.graph, self.outside);
        recentered |= nucleus.focus() != focus;
        if recentered {
            nucleus.recenter(&self.graph, focus);
        }
        if let Some(trail) = trail {
            self.traced = Some(key);
            recentered |= nucleus.retrace(&self.graph, trail);
        }
        let marked = nucleus.mark(&self.graph, cursor);
        if nucleus.advance(dt) || recentered || marked {
            self.moved += 1;
        }
        recentered || marked
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

    /// Runs the graph simulation a few steps while it is still moving. Returns whether
    /// anything moved.
    pub(crate) fn simulate(&mut self, steps: usize) -> bool {
        if !self.settling {
            return false;
        }
        let mut speed = 0.0;
        for _ in 0..steps {
            speed = self.graph.step();
        }
        self.settling = speed > SETTLED_SPEED;
        self.moved += 1;
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
        self.moved += 1;
    }

    /// Gives lanes and containers their colors after the journey changed, and asks
    /// for a redraw.
    fn recolor(&mut self) {
        let colors = &mut self.lane_colors;
        self.track.colors = self
            .track
            .scene
            .lanes
            .iter()
            .map(|lane| {
                let next = colors.len();
                *colors.entry(lane.label.clone()).or_insert(next)
            })
            .collect();
        self.color_groups();
        self.refresh_journey_tiles();
        self.revision += 1;
    }

    /// The focus: the node the graph lights up around, and what I, O and Enter act on.
    /// Browsing, the function selected; in a journey, a node clicked off it while there
    /// is one, else the cursor's node, or the root's when the journey could not be
    /// traced.
    pub(crate) fn graph_focus(&self) -> Option<usize> {
        if self.browsing() {
            return self.selection;
        }
        self.aside.or_else(|| {
            self.focus()
                .and_then(|f| self.map.tile_of(f))
                .or_else(|| self.root_tile())
        })
    }

    /// The node clicked off the journey that is the focus for now, if any.
    pub(crate) fn aside(&self) -> Option<usize> {
        self.aside.filter(|_| !self.browsing())
    }

    /// Puts the focus back on the journey's cursor. Returns whether it was off it.
    pub(crate) fn clear_aside(&mut self) -> bool {
        self.aside.take().is_some()
    }

    /// Moves the focus to a node, as a click does, remembering where it was. Browsing,
    /// it is selected. In a journey that runs it, the cursor goes to its call, the one
    /// numbered `number` (the number on the name clicked) or else the one nearest the
    /// cursor, opening what folds it; off the journey, the node is selected for now,
    /// and the journey stays as it was. Enter follows. Nothing moves, but inside the
    /// expanded nucleus, which gathers round the function clicked there.
    pub(crate) fn focus_tile(&mut self, tile: usize, number: Option<&str>) {
        self.navigate(|explorer| {
            if explorer.browsing() {
                if explorer.selection != Some(tile) {
                    explorer.selection = Some(tile);
                    explorer.spotlight = None;
                }
            } else {
                explorer.focus_on(tile, number);
            }
        });
    }

    fn focus_on(&mut self, tile: usize, number: Option<&str>) {
        if let Some(span) = self.span_at(tile, number) {
            self.select(Some(span));
        } else if self.aside != Some(tile) {
            let before = self.graph_focus();
            self.aside = Some(tile);
            if self.graph_focus() != before {
                self.spotlight = None;
            }
        }
    }

    /// The journey's call that runs a node: the cursor's when it already does, the one
    /// numbered `number`, else of those shown the one nearest the cursor in the order
    /// the journey runs them, else the first at all.
    fn span_at(&self, tile: usize, number: Option<&str>) -> Option<SpanId> {
        let unit = self.tile_unit(tile)?;
        let disclosed = self.disclosure();
        let journey = self.journey()?;
        let runs: Vec<SpanId> = journey
            .spans()
            .filter(|(_, s)| s.target == SpanTarget::Unit(unit.clone()))
            .map(|(id, _)| id)
            .collect();
        if let Some(span) = number.and_then(|n| {
            runs.iter()
                .copied()
                .find(|&span| disclosed.number(span) == Some(n))
        }) {
            return Some(span);
        }
        if self.focus() == Some(unit) {
            return self.selected();
        }
        let cursor = self.selected().map_or(0, SpanId::index);
        runs.iter()
            .copied()
            .filter(|&span| disclosed.shows(span))
            .min_by_key(|span| span.index().abs_diff(cursor))
            .or_else(|| runs.first().copied())
    }

    fn root_tile(&self) -> Option<usize> {
        self.root().and_then(|u| self.map.tile_of(u))
    }

    /// The node the camera orbits: where the journey starts. Stepping through
    /// the journey leaves it put; browsing, there is none, so a click moves nothing.
    pub(crate) fn anchor(&self) -> Option<usize> {
        if self.browsing() {
            return None;
        }
        self.root_tile().or_else(|| self.graph_focus())
    }

    /// Centers on a node and opens its journey: the journey restarts there, as deep as
    /// the journey before it showed. Opening the node the journey already starts at
    /// puts the cursor back on it.
    pub(crate) fn open_at(&mut self, tile: usize) {
        self.navigate(|explorer| explorer.restart_at(tile));
        self.aim += 1;
    }

    fn restart_at(&mut self, tile: usize) {
        let Some(unit) = self.tile_unit(tile).cloned() else {
            return;
        };
        self.mode = ViewMode::Journey;
        if self.root() == Some(&unit) {
            self.aside = None;
            self.select(self.journey().map(Journey::root));
            return;
        }
        self.replace_root(unit);
        self.spotlight = None;
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

    /// What the journey discloses, worked out once per change; browsing, nothing.
    pub(crate) fn disclosure(&self) -> Arc<Disclosed> {
        if self.browsing() {
            return Arc::new(Disclosed::default());
        }
        let key = (
            self.revision,
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

    /// Whether a span of the journey shows.
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

    /// Moves the focus to the call numbered `number` on screen: by the journey's own
    /// numbering, as its bars and its nodes' badges show it, or with nothing of the
    /// journey disclosed, the focus's calls in the order it makes them, as `#1`, `#2`
    /// beside them, as a click on that node would. The journey stays where it starts.
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
                self.focus_tile(tile, None);
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
        self.go_to(call);
    }

    /// What calls `tile`, each once.
    pub(crate) fn callers_of(&self, tile: usize) -> Vec<usize> {
        let mut callers: Vec<usize> = self.graph.callers(tile).collect();
        callers.sort_unstable();
        callers.dedup();
        callers
    }

    /// Which side of the focus is in the spotlight, and since when. Only ever on the
    /// focus: once the focus moves, it is out.
    pub(crate) fn spotlight(&self) -> Option<(Side, f32)> {
        self.spotlight
            .filter(|&(_, _, tile)| Some(tile) == self.graph_focus())
            .map(|(side, since, _)| (side, since))
    }

    /// Puts `side` in the spotlight at `now`, or takes it out when it already is.
    pub(crate) fn toggle_spotlight(&mut self, side: Side, now: f32) {
        let Some(focus) = self.graph_focus() else {
            return;
        };
        self.spotlight = match self.spotlight() {
            Some((on, _)) if on == side => None,
            _ => Some((side, now, focus)),
        };
    }

    /// Takes the spotlight out.
    pub(crate) fn clear_spotlight(&mut self) {
        self.spotlight = None;
    }

    /// The nodes in the spotlight: the focus's callers or its callees, each once.
    pub(crate) fn spotlit(&self) -> Vec<usize> {
        let (Some((side, _)), Some(focus)) = (self.spotlight(), self.graph_focus()) else {
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

    /// Where the lens starts: the focus, the changes, or the journey.
    fn seeds(&self) -> Vec<usize> {
        let mut seeds: Vec<usize> = match self.lens.source {
            lens::Source::Focus => self.graph_focus().into_iter().collect(),
            lens::Source::Changes => self.changed_tiles.clone(),
            lens::Source::Journey => self.disclosed_tiles().into_keys().collect(),
        };
        seeds.sort_unstable();
        seeds.dedup();
        seeds
    }

    /// How far each node is from where the lens starts, worked out once per change of
    /// where it starts, how it measures, or, measuring in space, where nodes sit.
    fn distances(&self) -> Option<Arc<Distances>> {
        let measure = self.lens.measure;
        let moved = if measure == lens::Measure::Calls {
            0
        } else {
            self.moved
        };
        let key = (self.seeds(), self.lens.source, measure, self.rewired, moved);
        let mut cache = self
            .distance_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((cached, distances)) = cache.as_ref()
            && *cached == key
        {
            return distances.clone();
        }
        let distances = self.lens.distances(&self.graph, &key.0).map(Arc::new);
        *cache = Some((key, distances.clone()));
        distances
    }

    /// The top of the lens's dial, where it shows everything: as far out as the
    /// farthest node from where it starts, under how it measures.
    pub(crate) fn lens_top(&self) -> u32 {
        self.distances().map_or(1, |d| d.farthest())
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
            self.lens.targets(&*self.distances()?)?
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

    /// What reaches the focus drawn and what it reaches, once
    /// [`Self::refresh_reach`] has run.
    pub(crate) fn reach(&self) -> Option<&Reach> {
        self.reach
            .as_ref()
            .filter(|(tile, _)| Some(*tile) == self.lit_focus())
            .map(|(_, reach)| reach)
    }

    /// Recomputes [`Self::reach`] when the focus moved. Returns whether it did.
    pub(crate) fn refresh_reach(&mut self) -> bool {
        let focus = self.lit_focus();
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

    /// A module cluster's name: the module's own, else its container's.
    pub(crate) fn cluster_name(&self, cluster: usize) -> String {
        self.graph
            .cluster_unit(cluster)
            .and_then(|id| self.any_unit(id))
            .map_or_else(String::new, |unit| unit.name.clone())
    }

    /// Gives every container a color, after those the journey uses.
    fn color_groups(&mut self) {
        for group in &self.map.groups {
            if let Some(unit) = &group.unit {
                let next = self.lane_colors.len();
                self.lane_colors.entry(unit.to_string()).or_insert(next);
            }
        }
    }

    fn refresh_journey_tiles(&mut self) {
        let track = &self.track;
        self.journey_tiles = track
            .scene
            .nodes
            .iter()
            .filter_map(|placed| {
                let unit = track.unit_id(placed.span)?;
                Some((self.map.tile_of(unit)?, track.color(placed.span)))
            })
            .collect();
    }

    pub(crate) fn map(&self) -> &CodebaseMap {
        &self.map
    }

    /// The color a node is drawn in on the journey, if it runs it.
    pub(crate) fn journey_lane(&self, tile: usize) -> Option<usize> {
        self.journey_tiles.get(&tile).copied()
    }

    /// The unit under the cursor.
    pub(crate) fn focus(&self) -> Option<&UnitId> {
        let track = self.track();
        track.selected.and_then(|id| track.unit_id(id))
    }

    pub(crate) fn span(&self, id: SpanId) -> Option<&Span> {
        self.track().span(id)
    }

    /// The unit a span of the journey runs, if it resolved to exactly one.
    pub(crate) fn unit_id(&self, id: SpanId) -> Option<&UnitId> {
        self.track().unit_id(id)
    }

    pub(crate) fn unit(&self, id: SpanId) -> Option<&Unit> {
        self.system.unit(self.unit_id(id)?)
    }

    pub(crate) fn label(&self, id: SpanId) -> String {
        let Some(span) = self.span(id) else {
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

    /// The call stack at a span of the journey, outermost first.
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

    /// Moves the cursor. Returns the new position. With the focus off the journey, it
    /// only goes back to the cursor. A walk of steps is remembered once, from where it
    /// began.
    pub(crate) fn step(&mut self, step: Step) -> Option<SpanId> {
        let before = self.view();
        let next = self.walk(step);
        if !self.walking && self.view() != before {
            self.history.leave(before);
            self.walking = true;
        }
        next
    }

    fn walk(&mut self, step: Step) -> Option<SpanId> {
        if self.browsing() {
            return None;
        }
        if self.clear_aside() {
            return self.selected();
        }
        let journey = self.journey()?;
        let current = self.selected().unwrap_or_else(|| journey.root());
        let tile = |span: SpanId| self.unit_id(span).and_then(|u| self.map.tile_of(u));
        // A closure is drawn as part of the function around it, with no bar or node of
        // its own: the cursor never stops on one, and the calls made inside it are its
        // caller's.
        let closure = |span: SpanId| {
            tile(span).is_some()
                && journey.span(span).and_then(|s| s.parent).and_then(tile) == tile(span)
        };
        // The keys read the picture as it is: they stop only on a call drawn, one with
        // a node of its own (not a closure, nor a call to one of several functions)
        // and disclosed, so they never open a level or reach past the depth shown.
        // Disclosing through time, the next call in time discloses itself.
        let disclosed = self.disclosure();
        let by_time = self.disclose == Disclose::Time;
        let drawn = |span: SpanId| {
            tile(span).is_some() && !closure(span) && (by_time || disclosed.number(span).is_some())
        };
        let parent = |span: SpanId| {
            std::iter::successors(journey.span(span)?.parent, |&p| journey.span(p)?.parent)
                .find(|&p| drawn(p))
        };
        let calls = |span: SpanId| {
            let mut calls = disclosure::calls_of(journey, &tile, span).0;
            calls.retain(|&c| drawn(c));
            calls
        };
        let nth = |index: usize| journey.spans().nth(index).map(|(id, _)| id);
        // On from `span` in pre-order, past anything not drawn.
        let onward = |span: Option<SpanId>| {
            std::iter::successors(span, |s| nth(s.index() + 1)).find(|&s| drawn(s))
        };
        // The first span after `id`'s subtree, in pre-order.
        let after = |id: SpanId| {
            let end = journey.span(id).map_or(0, |s| s.exit);
            journey
                .spans()
                .skip(id.index() + 1)
                .find(|(other, s)| s.enter >= end || !is_within(journey, *other, id))
                .map(|(id, _)| id)
        };
        let siblings = || parent(current).map(calls).unwrap_or_default();
        let place = |list: &[SpanId]| list.iter().position(|&s| s == current);
        let next = match step {
            Step::Into => onward(nth(current.index() + 1)),
            Step::Back => (0..current.index())
                .rev()
                .filter_map(nth)
                .find(|&s| drawn(s)),
            Step::Over => onward(after(current)),
            Step::Out => parent(current).and_then(|p| onward(after(p)).or(Some(p))),
            Step::Next => {
                let list = siblings();
                place(&list).and_then(|i| list.get(i + 1).copied())
            }
            Step::Previous => {
                let list = siblings();
                place(&list)
                    .and_then(|i| i.checked_sub(1))
                    .and_then(|i| list.get(i).copied())
            }
            Step::Child => calls(current).first().copied(),
            Step::Parent => parent(current),
        };
        if next.is_some() {
            self.select(next);
        } else {
            let hidden = |span: SpanId| {
                tile(span).is_some() && !closure(span) && disclosed.number(span).is_none()
            };
            // Calls past the depth shown: under the cursor for ↓, anywhere ahead
            // for ] and the like.
            let folded = !by_time
                && match step {
                    Step::Child => disclosure::calls_of(journey, &tile, current)
                        .0
                        .into_iter()
                        .any(hidden),
                    _ => journey
                        .spans()
                        .skip(current.index() + 1)
                        .any(|(span, _)| hidden(span)),
                };
            let first = current == journey.root();
            self.say(stuck(step, first, folded));
        }
        next
    }

    /// Says something for a moment, over the graph.
    fn say(&mut self, text: &str) {
        let count = self.notice.as_ref().map_or(0, |(n, _)| n + 1);
        self.notice = Some((count, text.to_owned()));
    }

    /// The last thing said for a moment, numbered so the same words said again show
    /// again.
    pub(crate) fn notice(&self) -> Option<(u64, &str)> {
        self.notice.as_ref().map(|(n, text)| (*n, text.as_str()))
    }
}

/// What a key that cannot move the cursor says: why not, and what would. `first` is
/// whether the cursor is on the journey's start, `folded` whether calls under it are
/// past the depth shown.
fn stuck(step: Step, first: bool, folded: bool) -> &'static str {
    match step {
        Step::Parent | Step::Back if first => "Already at the journey's start",
        Step::Parent => "Nothing above this call is drawn",
        Step::Back => "No earlier call is drawn",
        Step::Next => "No later call from the same caller",
        Step::Previous => "No earlier call from the same caller",
        Step::Child if folded => "Its calls are folded: . shows a level more",
        Step::Child => "It makes no calls",
        Step::Into | Step::Over | Step::Out if folded => {
            "The end of what is shown: . shows a level more"
        }
        Step::Into | Step::Over | Step::Out => "The end of the journey",
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
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "b", "c is folded under a, so ] passes it by");
        assert_eq!(
            explorer.step(Step::Into),
            None,
            "and stops at the end of what shows"
        );
        assert_eq!(
            explorer.notice().map(|(_, text)| text),
            Some("The end of the journey")
        );
        explorer.step(Step::Back);
        explorer.set_disclose(Disclose::Depth(2));
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
        assert_eq!(
            explorer.step(Step::Child),
            None,
            "c is past the depth shown"
        );
        assert_eq!(
            explorer.notice().map(|(_, text)| text),
            Some("Its calls are folded: . shows a level more"),
            "and it says so"
        );
        explorer.set_disclose(Disclose::Depth(2));
        explorer.step(Step::Child);
        assert_eq!(at(&explorer), "c");
        assert_eq!(explorer.step(Step::Child), None);
        assert_eq!(explorer.notice().map(|(_, t)| t), Some("It makes no calls"));
        explorer.step(Step::Parent);
        assert_eq!(at(&explorer), "a");
        explorer.step(Step::Parent);
        assert_eq!(explorer.step(Step::Parent), None);
        assert_eq!(
            explorer.notice().map(|(_, t)| t),
            Some("Already at the journey's start")
        );
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
    fn the_dial_fits_the_graph() {
        let mut explorer = explorer();
        assert_eq!(explorer.lens_top(), 2, "c is two calls from main");
        let everything = explorer.lens();
        let narrower = everything.narrower(explorer.lens_top());
        assert_eq!(narrower.reach, Some(1), "- from everything drops one ring");
        assert_eq!(narrower.narrower(2).reach, Some(1), "and stops at one");
        assert_eq!(narrower.wider(2).reach, None, "= at the top is everything");
        assert_eq!(narrower.level(2), 1);
        assert_eq!(everything.level(2), 2);

        explorer.open_at(tile(&explorer, "c"));
        assert_eq!(explorer.lens_top(), 2, "main is two calls up from c");
        explorer.open_at(tile(&explorer, "b"));
        explorer.set_lens(Lens {
            reach: Some(5),
            ..Lens::default()
        });
        assert_eq!(explorer.lens().level(explorer.lens_top()), 1);
        assert_eq!(
            explorer.lens().narrower(explorer.lens_top()).reach,
            Some(1),
            "a reach past the top narrows from the top"
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
        explorer.set_disclose(Disclose::Depth(2));
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
    fn rerooting_keeps_history() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.trace_from_focus();
        assert_eq!(explorer.journey().map(Journey::len), Some(2));
        assert!(explorer.can_go_back());

        explorer.go_back();
        assert_eq!(explorer.journey().map(Journey::len), Some(4));
        assert_eq!(at(&explorer), "a", "back where the cursor was");
        assert!(explorer.can_go_back(), "and the step there before it");
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
        explorer.toggle_spotlight(Side::Callers, 1.0);
        let before = explorer.view();

        explorer.open_at(tile(&explorer, "b"));
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:b"));
        assert_eq!(
            explorer.disclose(),
            Disclose::Depth(2),
            "b opens as deep as the journey before it"
        );
        assert!(explorer.spotlight().is_none());

        assert!(explorer.go_back());
        assert_eq!(explorer.view(), before);
        assert_eq!(at(&explorer), "c", "the cursor where it was");
        assert_eq!(explorer.disclose(), Disclose::Depth(2));
        assert!(explorer.disclosed(a_span), "a still opened by hand");
        assert_eq!(
            explorer.spotlight().map(|(side, _)| side),
            Some(Side::Callers),
            "the spotlight back on"
        );
        assert!(explorer.go_back(), "then the walk to it");
        assert!(!explorer.go_back(), "from the first place");
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
        explorer.set_disclose(Disclose::Depth(3));
        explorer.toggle_spotlight(Side::Callees, 0.0);
        explorer.toggle_hidden(Side::Callers);
        assert!(
            !explorer.can_go_back(),
            "showing more, the spotlight and hiding are not moves"
        );
        explorer.toggle_hidden(Side::Callers);
        explorer.step(Step::Into);

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
    fn a_walk_with_the_keys_is_one_move() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.set_disclose(Disclose::Depth(2));
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");
        explorer.step(Step::Back);
        assert!(explorer.go_back(), "the walk is remembered");
        assert_eq!(at(&explorer), "main", "from where it began");
        assert_eq!(
            explorer.disclose(),
            Disclose::default(),
            "at the depth it had"
        );
        assert!(!explorer.can_go_back(), "as one move");
        assert!(explorer.go_forward());
        assert_eq!(at(&explorer), "a", "forward to where it ended");

        // A click ends the walk; the next walk is a move of its own.
        explorer.focus_tile(tile(&explorer, "main"), None);
        explorer.set_disclose(Disclose::Depth(2));
        explorer.step(Step::Into);
        explorer.step(Step::Into);
        assert!(explorer.go_back());
        assert_eq!(at(&explorer), "main", "back to where the second walk began");
        assert!(explorer.go_back());
        assert_eq!(at(&explorer), "a", "then to before the click");
    }

    #[test]
    fn back_puts_back_what_was_hidden() {
        let mut explorer = explorer();
        explorer.toggle_hidden(Side::Callers);
        explorer.open_at(tile(&explorer, "b"));
        explorer.toggle_hidden(Side::Callers);
        explorer.toggle_hidden(Side::Callees);
        assert!(explorer.go_back());
        assert!(explorer.hidden().callers && !explorer.hidden().callees);
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
            explorer.focus().map(UnitId::as_str),
            Some("x:d"),
            "2 goes to the node badged 2, not the second function written"
        );
        assert_eq!(
            explorer.root().map(UnitId::as_str),
            Some("x:main"),
            "and the journey stays where it starts"
        );
        assert!(
            explorer.can_go_back(),
            "a digit is remembered, as a click is"
        );
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
    fn the_keys_never_open_what_is_folded() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        let a = explorer.selected().unwrap();
        assert_eq!(explorer.disclosure().folded(a), 1, "c is folded under a");
        assert_eq!(explorer.step(Step::Child), None);
        assert_eq!(at(&explorer), "a", "the cursor stays on what shows");
        assert_eq!(explorer.disclosure().folded(a), 1, "and c stays folded");
        explorer.toggle_expanded(a);
        explorer.step(Step::Child);
        assert_eq!(at(&explorer), "c", "opened by hand, it is there to step to");
        assert_eq!(
            explorer.disclosure().number(explorer.selected().unwrap()),
            Some("1.1")
        );
    }

    #[test]
    fn the_keys_pass_calls_with_no_node_of_their_own() {
        // main's first call could run either b or c, which has no node; its second
        // runs d.
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
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "d", "past the call to one of two");
        assert_eq!(explorer.step(Step::Previous), None);
        explorer.step(Step::Back);
        assert_eq!(at(&explorer), "main");
        explorer.step(Step::Child);
        assert_eq!(at(&explorer), "d");
    }

    #[test]
    fn clicking_a_node_on_the_journey_moves_the_cursor_there() {
        let mut explorer = explorer();
        let main = explorer.journey().unwrap().root();
        let a = explorer.journey().unwrap().span(main).unwrap().children[0];
        assert_eq!(explorer.disclosure().folded(a), 1, "c is folded under a");

        explorer.focus_tile(tile(&explorer, "c"), None);
        assert_eq!(at(&explorer), "c");
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:main"));
        assert!(
            explorer.disclosed(explorer.selected().unwrap()),
            "its bar shows"
        );
        assert!(explorer.aside().is_none());
        assert!(explorer.go_back(), "a click is remembered");
        assert_eq!(at(&explorer), "main", "and Back puts the cursor back");
    }

    #[test]
    fn back_undoes_any_click_with_the_cursor_and_depth_it_left() {
        let mut explorer = explorer();
        explorer.open_at(tile(&explorer, "a"));
        explorer.forget_history();
        explorer.set_disclose(Disclose::Depth(2));
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");
        let b = tile(&explorer, "b");

        // Off the journey, at another depth.
        explorer.focus_tile(b, None);
        explorer.set_disclose(Disclose::Depth(1));
        assert_eq!(explorer.aside(), Some(b));
        assert!(explorer.go_back());
        assert_eq!(explorer.aside(), None, "the node clicked off it is let go");
        assert_eq!(
            at(&explorer),
            "c",
            "the cursor is back where the keys left it"
        );
        assert_eq!(
            explorer.disclose(),
            Disclose::Depth(2),
            "at the depth it was"
        );

        assert!(explorer.go_forward());
        assert_eq!(explorer.aside(), Some(b), "forward clicks it again");
        assert!(explorer.go_back());

        // Gathered round the journey, a click moves the cursor and nothing else.
        explorer.set_nucleus(true);
        explorer.ease_nucleus(1.0);
        let middle = explorer.nucleus().and_then(Nucleus::focus);
        assert_eq!(middle, Some(tile(&explorer, "a")), "the journey's start");
        explorer.focus_tile(tile(&explorer, "a"), None);
        explorer.ease_nucleus(1.0);
        assert_eq!(at(&explorer), "a");
        assert_eq!(explorer.nucleus().and_then(Nucleus::focus), middle);
        assert!(explorer.go_back());
        explorer.ease_nucleus(1.0);
        assert_eq!(explorer.nucleus().and_then(Nucleus::focus), middle);
        assert_eq!(at(&explorer), "c", "Back puts the cursor back");
    }

    #[test]
    fn a_node_off_the_journey_is_the_focus_until_followed_or_left() {
        let mut explorer = explorer();
        explorer.open_at(tile(&explorer, "a"));
        explorer.forget_history();
        let b = tile(&explorer, "b");

        explorer.focus_tile(b, None);
        assert_eq!(explorer.aside(), Some(b));
        assert_eq!(explorer.graph_focus(), Some(b), "I and O act on it");
        assert_eq!(at(&explorer), "a", "the cursor stays put");
        explorer.toggle_spotlight(Side::Callers, 0.0);
        assert_eq!(explorer.spotlit(), vec![tile(&explorer, "main")]);

        assert_eq!(
            explorer.step(Step::Into).map(|s| explorer.label(s)),
            Some("a".to_owned())
        );
        assert!(
            explorer.aside().is_none(),
            "an arrow goes back to the cursor"
        );
        assert_eq!(at(&explorer), "a", "without stepping");

        explorer.focus_tile(b, None);
        assert!(explorer.clear_aside(), "Esc goes back too");
        assert_eq!(explorer.graph_focus(), Some(tile(&explorer, "a")));

        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");
        explorer.focus_tile(b, None);
        explorer.trace_from_focus();
        assert_eq!(
            explorer.root().map(UnitId::as_str),
            Some("x:b"),
            "Enter follows it"
        );
        assert!(explorer.aside().is_none());
        assert!(explorer.go_back());
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:a"));
        assert_eq!(at(&explorer), "c", "back on the journey, cursor and all");
        assert_eq!(
            explorer.aside(),
            Some(b),
            "and the node it had clicked off it"
        );
    }

    #[test]
    fn shift_click_follows_at_once() {
        let mut explorer = explorer();
        explorer.open_at(tile(&explorer, "b"));
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:b"));
        assert!(explorer.go_back(), "one move to go back from");
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:main"));
        assert!(!explorer.can_go_back());
    }

    #[test]
    fn the_spotlight_goes_out_when_the_focus_moves() {
        let mut explorer = explorer();
        explorer.toggle_spotlight(Side::Callees, 0.0);
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

        assert!(explorer.go_back(), "back over the walk");
        assert!(explorer.go_back());
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:a"));
    }

    #[test]
    fn the_nucleus_holds_its_middle_while_the_cursor_steps() {
        let mut explorer = explorer();
        let tile = |explorer: &Explorer, name: &str| {
            explorer
                .map()
                .tile_of(&UnitId::new(format!("x:{name}")))
                .unwrap()
        };
        let (main, a, b) = (
            tile(&explorer, "main"),
            tile(&explorer, "a"),
            tile(&explorer, "b"),
        );
        explorer.set_nucleus(true);
        explorer.ease_nucleus(1.0);
        assert_eq!(explorer.nucleus().and_then(Nucleus::focus), Some(main));

        explorer.step(Step::Into);
        explorer.ease_nucleus(1.0);
        assert_eq!(explorer.graph_focus(), Some(a), "the cursor moved");
        assert_eq!(
            explorer.nucleus().and_then(Nucleus::focus),
            Some(main),
            "the middle did not"
        );
        assert_eq!(explorer.lit_focus(), Some(main));
        assert_eq!(explorer.cursor_mark(), Some(a), "the cursor is marked");
        assert!(explorer.nucleus().is_some_and(|n| n.marks(a)));

        explorer.focus_tile(b, None);
        explorer.ease_nucleus(1.0);
        assert_eq!(
            explorer.nucleus().and_then(Nucleus::focus),
            Some(main),
            "a click moves the cursor, not the middle"
        );
        assert_eq!(
            explorer.cursor_mark(),
            Some(b),
            "the cursor is the one pink node"
        );
        explorer.step(Step::Back);
        explorer.ease_nucleus(1.0);
        assert_eq!(explorer.nucleus().and_then(Nucleus::focus), Some(main));

        explorer.trace_from_focus();
        explorer.ease_nucleus(1.0);
        assert_eq!(
            explorer.nucleus().and_then(Nucleus::focus),
            explorer.graph_focus(),
            "following the focus re-centers on it"
        );
    }

    #[test]
    fn expanded_a_click_centres_the_cursor_and_keeps_the_journey() {
        let mut explorer = explorer();
        let [main, a, b] = ["main", "a", "b"].map(|name| tile(&explorer, name));
        explorer.set_nucleus(true);
        explorer.expand_nucleus(true);
        explorer.ease_nucleus(1.0);
        let named = |explorer: &Explorer| {
            let nucleus = explorer.nucleus().unwrap();
            [main, a, b].map(|t| nucleus.names(t) || nucleus.focus() == Some(t))
        };
        let before = named(&explorer);
        assert_eq!(explorer.nucleus().and_then(Nucleus::centre), Some(main));

        explorer.focus_tile(a, None);
        explorer.ease_nucleus(1.0);
        let nucleus = explorer.nucleus().unwrap();
        assert_eq!(
            nucleus.focus(),
            Some(main),
            "the journey still starts at main"
        );
        assert_eq!(nucleus.centre(), Some(a), "the clicked call is the centre");
        assert_eq!(explorer.cursor_mark(), Some(a), "and the cursor");
        assert_eq!(named(&explorer), before, "naming the same journey");

        explorer.step(Step::Next);
        explorer.ease_nucleus(1.0);
        assert_eq!(
            explorer.nucleus().and_then(Nucleus::centre),
            Some(b),
            "a key centres the same way"
        );
        explorer.go_to(explorer.journey().unwrap().root());
        explorer.ease_nucleus(1.0);
        assert_eq!(
            explorer.nucleus().and_then(Nucleus::centre),
            Some(main),
            "and a bar"
        );
    }

    #[test]
    fn back_and_esc_put_the_camera_back_where_it_stood() {
        use crate::camera::OrbitCamera;
        let mut camera = OrbitCamera::default();
        let first = camera.pose();
        camera.pan_to(Vec3::new(40.0, 0.0, 0.0));
        let second = camera.pose();
        camera.pan_to(Vec3::new(0.0, 40.0, 0.0));
        let third = camera.pose();
        let returned = |explorer: &Explorer| explorer.return_pose().and_then(|(_, p)| p);

        let mut explorer = explorer();
        let [a, b] = ["a", "b"].map(|name| tile(&explorer, name));
        explorer.see(first);
        explorer.focus_tile(a, None);
        explorer.see(second);
        assert!(explorer.go_back());
        assert_eq!(returned(&explorer), Some(first), "Back looks as it looked");
        assert!(explorer.go_forward());
        assert_eq!(returned(&explorer), Some(second), "and so does Forward");

        explorer.leave_journey();
        explorer.see(third);
        explorer.focus_tile(b, None);
        explorer.trace_from_focus();
        explorer.see(first);
        explorer.leave_journey();
        assert!(explorer.browsing());
        assert_eq!(
            returned(&explorer),
            Some(third),
            "Esc goes back to the codebase as it was seen before Enter"
        );
        assert_eq!(
            explorer.selection(),
            Some(b),
            "with the cursor's function selected"
        );
    }

    #[test]
    fn the_nucleus_names_the_journey_to_the_depth_and_no_more() {
        let mut explorer = explorer();
        let [a, b, c] = ["a", "b", "c"].map(|name| tile(&explorer, name));
        explorer.set_nucleus(true);
        explorer.ease_nucleus(1.0);
        let names = |explorer: &Explorer| {
            let nucleus = explorer.nucleus().unwrap();
            [a, b, c].map(|t| nucleus.names(t))
        };
        assert_eq!(names(&explorer), [true, true, false], "one level down");
        assert_eq!(explorer.nucleus().unwrap().numbers(b), ["2"]);

        explorer.focus_tile(c, None);
        explorer.ease_nucleus(1.0);
        assert_eq!(explorer.graph_focus(), Some(c), "the cursor on c");
        assert_eq!(
            names(&explorer),
            [true, true, true],
            "the call clicked opens, as its bar does"
        );
        assert!(explorer.nucleus().unwrap().marks(c), "and rings the cursor");
        explorer.toggle_expanded(explorer.stack(explorer.selected().unwrap())[1]);
        explorer.ease_nucleus(1.0);
        assert_eq!(
            names(&explorer),
            [true, true, false],
            "folded again by hand"
        );

        explorer.set_disclose(Disclose::Depth(2));
        explorer.ease_nucleus(1.0);
        assert_eq!(names(&explorer), [true, true, true], "two levels down");
        assert_eq!(explorer.nucleus().unwrap().numbers(c), ["1.1"]);
        assert_eq!(explorer.nucleus().unwrap().level(c), Some(2));

        explorer.toggle_hidden(Side::Callees);
        assert_eq!(
            names(&explorer),
            [false, false, false],
            "the journey hidden"
        );
    }

    #[test]
    fn hiding_a_side_unlights_it() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.refresh_reach();
        let main = explorer.map().tile_of(&UnitId::new("x:main")).unwrap();
        let reach = explorer.reach().unwrap().clone();
        assert!(crate::graph_view::shown_relation(&explorer, &reach, main).is_some());
        explorer.toggle_hidden(Side::Callers);
        assert!(explorer.hidden().hides(Side::Callers));
        assert_eq!(
            crate::graph_view::shown_relation(&explorer, &reach, main),
            None,
            "a's caller is hidden"
        );
        explorer.toggle_hidden(Side::Callers);
        assert_eq!(explorer.hidden(), Hidden::default());
    }

    #[test]
    fn browsing_a_click_selects_and_moves_nothing() {
        let mut explorer = explorer();
        let [main, a, b, c] = ["main", "a", "b", "c"].map(|name| tile(&explorer, name));
        explorer.set_nucleus(true);
        explorer.ease_nucleus(1.0);
        explorer.leave_journey();
        explorer.ease_nucleus(1.0);
        assert!(explorer.browsing());
        assert_eq!(explorer.selection(), Some(main), "the cursor's function");
        assert!(explorer.disclosed_tiles().is_empty(), "no journey numbers");
        let places = |explorer: &Explorer| {
            let nucleus = explorer.nucleus().unwrap();
            let points: Vec<Option<Vec3>> = [main, a, b, c].map(|t| nucleus.point(t)).into();
            let balls: Vec<Vec3> = nucleus.balls().map(|(_, ball)| ball.center).collect();
            (points, balls)
        };
        let before = places(&explorer);
        for tile in [a, c, b, main] {
            explorer.focus_tile(tile, None);
            while explorer.ease_nucleus(0.5) {}
            explorer.ease_nucleus(1.0);
            assert_eq!(explorer.selection(), Some(tile));
            assert_eq!(explorer.lit_focus(), Some(tile), "lit white where it sits");
            assert_eq!(places(&explorer), before, "nothing moved");
        }
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:main"));
        assert!(explorer.step(Step::Into).is_none(), "no journey to walk");

        assert!(explorer.go_back());
        assert!(explorer.go_back());
        assert_eq!(explorer.selection(), Some(c), "Back walks the selections");
        assert!(explorer.go_forward());
        assert_eq!(explorer.selection(), Some(b));
    }

    #[test]
    fn enter_opens_the_selection_s_journey_and_esc_and_back_cross_the_modes() {
        let mut explorer = explorer();
        let a = tile(&explorer, "a");
        explorer.step(Step::Into);
        explorer.leave_journey();
        assert_eq!(explorer.selection(), Some(a), "the cursor's function");
        explorer.trace_from_focus();
        assert!(!explorer.browsing());
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:a"));
        assert!(explorer.go_back(), "back to browsing");
        assert!(explorer.browsing());
        assert_eq!(explorer.selection(), Some(a));
        assert!(explorer.go_back(), "and into the journey before");
        assert!(!explorer.browsing());
        assert_eq!(at(&explorer), "a");
    }

    #[test]
    fn a_click_goes_to_the_run_nearest_the_cursor_or_the_one_named() {
        // main calls a, b, c, then a again.
        let call = |name: &str, n: usize| {
            format!(
                r#"{{ "key": "{name}#{n}", "text": "{name}", "targets": [ {{ "unit": "x:{name}", "certainty": "exact" }} ] }}"#
            )
        };
        let unit = |name: &str| {
            format!(
                r#"{{ "id": "x:{name}", "name": "{name}", "kind": "fn", "role": "invocable" }}"#
            )
        };
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [
                {{ "id": "x:main", "name": "main", "kind": "fn", "role": "invocable", "calls": [{}, {}, {}, {}] }},
                {}, {}, {} ] }}"#,
            call("a", 0),
            call("b", 0),
            call("c", 0),
            call("a", 1),
            unit("a"),
            unit("b"),
            unit("c")
        );
        let system = System::new([Document::from_json(&json).unwrap()]);
        let mut explorer = Explorer::new(
            system,
            Some(UnitId::new("x:main")),
            JourneyOptions::default(),
        );
        let [a, b, c] = ["a", "b", "c"].map(|name| tile(&explorer, name));
        let number = |explorer: &Explorer| {
            explorer
                .disclosure()
                .number(explorer.selected().unwrap())
                .map(str::to_owned)
        };
        explorer.focus_tile(c, None);
        explorer.focus_tile(a, None);
        assert_eq!(number(&explorer).as_deref(), Some("4"), "the run after c");
        explorer.focus_tile(b, None);
        explorer.focus_tile(a, None);
        assert_eq!(number(&explorer).as_deref(), Some("1"), "the run before b");
        explorer.focus_tile(a, None);
        assert_eq!(number(&explorer).as_deref(), Some("1"), "on it already");
        explorer.focus_tile(a, Some("4"));
        assert_eq!(
            number(&explorer).as_deref(),
            Some("4"),
            "the one its name numbers"
        );
    }

    #[test]
    fn i_brings_in_outside_callers_and_one_clicked_is_selected_in_place() {
        // main calls a; x calls a too, from off the journey that starts at main.
        let json = r#"{ "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" }, "units": [
            { "id": "x:main", "name": "main", "kind": "fn", "role": "invocable", "calls": [
                { "key": "a#0", "text": "a", "targets": [ { "unit": "x:a", "certainty": "exact" } ] } ] },
            { "id": "x:x", "name": "x", "kind": "fn", "role": "invocable", "calls": [
                { "key": "a#0", "text": "a", "targets": [ { "unit": "x:a", "certainty": "exact" } ] } ] },
            { "id": "x:a", "name": "a", "kind": "fn", "role": "invocable" } ] }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let mut explorer = Explorer::new(
            system,
            Some(UnitId::new("x:main")),
            JourneyOptions::default(),
        );
        let x = tile(&explorer, "x");
        explorer.set_nucleus(true);
        explorer.ease_nucleus(1.0);
        assert!(
            !explorer.nucleus().unwrap().lit(x),
            "off the journey, not shown"
        );
        explorer.press_callers(0.0);
        explorer.ease_nucleus(1.0);
        assert!(explorer.outside_shown());
        assert!(explorer.nucleus().unwrap().outside(x), "brought in");
        explorer.focus_tile(x, None);
        explorer.ease_nucleus(1.0);
        assert!(!explorer.browsing(), "still in the journey");
        assert_eq!(explorer.aside(), Some(x), "selected in place");
        assert_eq!(at(&explorer), "main", "the cursor stays");
        explorer.trace_from_focus();
        assert_eq!(
            explorer.root().map(UnitId::as_str),
            Some("x:x"),
            "Enter starts there"
        );
    }

    #[test]
    fn the_cursor_never_stops_on_a_closure() {
        // main runs a closure that calls a, then calls b itself.
        let json = r#"{ "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" }, "units": [
            { "id": "x:main", "name": "main", "kind": "fn", "role": "invocable", "calls": [
                { "key": "c#0", "text": "c", "targets": [ { "unit": "x:main::{closure#0}", "certainty": "exact" } ] },
                { "key": "b#0", "text": "b", "targets": [ { "unit": "x:b", "certainty": "exact" } ] } ] },
            { "id": "x:main::{closure#0}", "name": "{closure#0}", "kind": "closure", "role": "invocable", "parent": "x:main", "calls": [
                { "key": "a#0", "text": "a", "targets": [ { "unit": "x:a", "certainty": "exact" } ] } ] },
            { "id": "x:a", "name": "a", "kind": "fn", "role": "invocable" },
            { "id": "x:b", "name": "b", "kind": "fn", "role": "invocable" } ] }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let mut explorer = Explorer::new(
            system,
            Some(UnitId::new("x:main")),
            JourneyOptions::default(),
        );
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "a", "past the closure, to its call");
        explorer.step(Step::Next);
        assert_eq!(at(&explorer), "b", "a and b are main's calls alike");
        explorer.step(Step::Previous);
        assert_eq!(at(&explorer), "a");
        explorer.step(Step::Back);
        assert_eq!(at(&explorer), "main", "back past the closure");
        explorer.step(Step::Child);
        assert_eq!(at(&explorer), "a");
        explorer.step(Step::Parent);
        assert_eq!(at(&explorer), "main");
    }
}
