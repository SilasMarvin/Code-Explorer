//! Viewer state that is independent of rendering: the loaded system, the codebase as
//! a graph of calls gathered into module spheres, the journey and its cursor, and
//! where the view has been.
//!
//! The view is in one of two modes. Browsing, no journey is open: a click selects a
//! function and moves nothing. In a journey, the view gathers round where it starts
//! once, on entering it, and a click on a function the journey runs moves its cursor
//! there; a click on any other sets it aside, selected in place.
//!
//! The functions that matter have one name each: the journey's `start`, its `cursor`,
//! the `selection` browsing, and the function set `aside` in a journey.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::prelude::*;
use codeviz_core::{
    Journey, JourneyOptions, Scenario, Span, SpanId, SpanTarget, StopReason, System, display_name,
    scenarios,
};
use codeviz_ir::{Certainty, Mode, UnitId};
use codeviz_layout::{CodebaseMap, ForceConfig, ForceGraph};

use crate::{
    camera::Pose,
    disclosure::{self, Disclose, Disclosed},
    history::{History, Seen, View},
    module_names,
    nucleus::{Nucleus, Trail},
};

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

/// The journey: where it starts, on its happy path, the outcomes that split off it,
/// and its cursor.
#[derive(Debug, Default)]
pub(crate) struct Track {
    start: Option<UnitId>,
    /// Every outcome from the start, the happy path first.
    scenarios: Vec<Scenario>,
    journey: Option<Journey>,
    /// Where each failing outcome splits off the happy path.
    divergences: Vec<SpanId>,
    cursor: Option<SpanId>,
    /// Spans opened one level past the disclosed depth, by hand or by the cursor.
    expanded: HashSet<SpanId>,
    /// Every function the journey runs, at any depth.
    units: HashSet<UnitId>,
    /// Names two or more functions on the journey, its start's callers or the outside
    /// callers shown share, so each is named with its type or module before it.
    homonyms: HashSet<String>,
}

impl Track {
    pub(crate) fn journey(&self) -> Option<&Journey> {
        self.journey.as_ref()
    }

    /// Where on the happy path each failing outcome splits off.
    pub(crate) fn divergences(&self) -> &[SpanId] {
        &self.divergences
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

    /// The node of `map` a span runs, if it resolved to exactly one function.
    pub(crate) fn tile(&self, id: SpanId, map: &CodebaseMap) -> Option<usize> {
        self.unit_id(id).and_then(|unit| map.tile_of(unit))
    }

    /// The nodes of `map` a span runs on the graph: the one function it resolved to;
    /// of a call only guessed among several, those the journey runs anyway, so a
    /// guess never brings in what the journey does not.
    pub(crate) fn ends(&self, id: SpanId, map: &CodebaseMap) -> Vec<usize> {
        match self.span(id).map(|span| &span.target) {
            Some(SpanTarget::Unit(unit)) => map.tile_of(unit).into_iter().collect(),
            Some(SpanTarget::Ambiguous(units)) => units
                .iter()
                .filter(|unit| self.units.contains(*unit))
                .filter_map(|unit| map.tile_of(unit))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Unrolls the journey from its start on the happy path, with the cursor on the
    /// start and nothing opened by hand.
    fn reroot(&mut self, system: &System) {
        let options = JourneyOptions::default();
        self.scenarios = self
            .start
            .as_ref()
            .map(|start| scenarios(system, start, &options.zoom))
            .unwrap_or_default();
        self.journey = self
            .start
            .as_ref()
            .and_then(|start| Journey::unroll(system, start, self.scenarios.first(), options));
        self.divergences = self.find_divergences();
        self.cursor = self.journey.as_ref().map(Journey::root);
        self.units = self
            .journey
            .iter()
            .flat_map(Journey::spans)
            .filter_map(|(_, span)| match &span.target {
                SpanTarget::Unit(unit) => Some(unit.clone()),
                _ => None,
            })
            .collect();
        // A rebuilt journey numbers its spans afresh.
        self.expanded.clear();
    }

    /// On the happy path, where each failing outcome splits off: the deepest unit on
    /// its chain that the happy path also runs.
    fn find_divergences(&self) -> Vec<SpanId> {
        let Some(journey) = &self.journey else {
            return Vec::new();
        };
        self.scenarios
            .iter()
            .skip(1)
            .filter_map(|scenario| {
                scenario.chain.iter().rev().find_map(|site| {
                    journey
                        .spans()
                        .find(|(_, s)| s.target == SpanTarget::Unit(site.unit.clone()))
                        .map(|(id, _)| id)
                })
            })
            .collect()
    }
}

/// The forces the codebase's graph settles under once, at the start: each module's
/// functions pull together, so each module's own map, which seeds where its functions
/// sit inside its sphere, keeps the calls among them short.
fn forces() -> ForceConfig {
    ForceConfig {
        repulsion: 50.0,
        link_length: 8.0,
        flatten: 0.1,
        cohesion: 0.15,
        separation: 0.4,
        cross_spring: 0.3,
        ..ForceConfig::default()
    }
}

/// Simulation steps always run before the graph is gathered into spheres.
const PRESETTLE_STEPS: usize = 200;
/// At most this many more steps for it to settle.
const SETTLE_STEPS: usize = 600;
/// Below this mean speed the graph counts as settled.
const SETTLED_SPEED: f32 = 0.002;

/// Whether a journey is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewMode {
    /// No journey: a click selects a function and moves nothing.
    Browse,
    /// A journey is open, isolated, its cursor moved by a click on what it runs.
    Journey,
}

#[derive(Resource, Debug)]
pub(crate) struct Explorer {
    system: System,
    /// The journey the cursor keys and the panel follow.
    track: Track,
    /// Every function, grouped by container; graph nodes are its tiles.
    map: CodebaseMap,
    /// The codebase as a graph of calls, settled once to seed where each module's
    /// functions sit inside its sphere.
    graph: ForceGraph,
    /// What each module sphere is called, by cluster: its name, told apart from
    /// another module's of the same name.
    module_names: Vec<String>,
    /// By cluster, the names two or more of its functions share: each of those is
    /// named with its type or module before it, wherever it shows.
    module_homonyms: Vec<HashSet<String>>,
    /// Functions whose name with its type or module reads the same as another's in
    /// their module: what tells them apart, the impl each is in.
    twins: HashMap<UnitId, String>,
    /// Bumped whenever nodes move.
    moved: u64,
    /// How much of the journey shows.
    disclose: Disclose,
    /// What the journey discloses, worked out again whenever the journey, its depth
    /// or what is opened by hand changes.
    disclosed: Arc<Disclosed>,
    /// Where the view has been, to go back and forward.
    history: History<View>,
    /// Bumped whenever the view is pointed somewhere by hand, so the camera turns to
    /// it even when that is where it already was.
    aim: u64,
    /// Whether the cursor is being walked with the keys, its start already
    /// remembered, so the rest of the walk is not.
    walking: bool,
    /// The function under the pointer, in the graph or as a bar in the journey, as
    /// the pointer finds it: what a click picks. What is drawn as hovered is the one
    /// the pointer has rested on ([`Self::hovered`]).
    pub(crate) under_pointer: Option<usize>,
    /// The journey number on the name under the pointer, so a click on a name goes to
    /// the call it numbers.
    pub(crate) under_pointer_number: Option<String>,
    /// The module whose sphere the pointer is over, nearest the camera, of those
    /// drawn: what X and a double click expand.
    pub(crate) module_under_pointer: Option<usize>,
    /// In a journey, a function clicked off it, selected in place until a journey
    /// key, Esc or Enter, which starts a journey there.
    aside: Option<usize>,
    /// Bumped whenever the journey is rebuilt or put back.
    revision: u64,
    /// The graph gathered into module spheres: browsing, the whole codebase; in a
    /// journey, round its start's module.
    nucleus: Nucleus,
    /// Whether a journey is open, or the view browses.
    mode: ViewMode,
    /// Browsing, the function last clicked: what the view lights, and what Enter opens
    /// the journey of.
    selection: Option<usize>,
    /// In a journey, whether every outside caller into it shows (I).
    outside: bool,
    /// Something said for a moment, as why a key moved nothing, numbered.
    notice: Option<(u64, String)>,
    /// How many notices have been said, so one said after another was cleared still
    /// counts as new.
    said: u64,
    /// Where the camera stands, as last seen.
    pose: Option<Pose>,
    /// Where the camera stood browsing when a journey was last entered from there,
    /// and the module expanded then: Esc from the journey puts both back.
    browsing_seen: Seen,
    /// Where the camera stood when a module was expanded, to go back to when it
    /// shrinks back, however the view was turned or moved while it was expanded.
    expand_pose: Option<Pose>,
    /// Where the camera is to go back to, until the camera takes it.
    return_pose: Option<Return>,
    /// Whether the panel's Fit asked the camera to frame the focus, until it does.
    frame_asked: bool,
    /// The function set aside X expanded the module expanded on, which letting go of
    /// shrinks back.
    expanded_on: Option<usize>,
}

/// Where the camera is to go back to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Return {
    /// A pose it had.
    To(Pose),
    /// The pose it had before a module was expanded, shrinking back: what the view
    /// was put on meanwhile is then brought into view.
    Shrunk(Pose),
    /// The whole codebase.
    Whole,
}

impl Explorer {
    /// Browsing, with nothing selected: the whole codebase, framed whole.
    pub(crate) fn new(system: System) -> Self {
        let map = codeviz_layout::map(&system);
        let mut graph = ForceGraph::new(&system, &map, forces());
        for _ in 0..PRESETTLE_STEPS {
            graph.step();
        }
        for _ in 0..SETTLE_STEPS {
            if graph.step() < SETTLED_SPEED {
                break;
            }
        }
        let nucleus = Nucleus::gather(&graph, None, Trail::default(), true);
        let module_names = {
            let modules: Vec<Option<(&str, &str)>> = (0..graph.cluster_count())
                .map(|cluster| {
                    let id = graph.cluster_unit(cluster)?;
                    let unit = system.unit(id)?;
                    Some((id.as_str(), unit.name.as_str()))
                })
                .collect();
            module_names::qualified(&modules)
        };
        let twins = twins(&system, &map, &graph);
        let mut module_homonyms = vec![HashSet::new(); graph.cluster_count()];
        let mut seen: HashMap<(usize, &str), usize> = HashMap::new();
        for (tile, at) in map.tiles.iter().enumerate() {
            let (Some(cluster), Some(unit)) = (graph.cluster(tile), system.unit(&at.unit)) else {
                continue;
            };
            let count = seen.entry((cluster, unit.name.as_str())).or_insert(0);
            *count += 1;
            if *count > 1 {
                module_homonyms[cluster].insert(unit.name.clone());
            }
        }
        let mut explorer = Self {
            system,
            track: Track::default(),
            map,
            graph,
            module_names,
            module_homonyms,
            twins,
            moved: 0,
            history: History::default(),
            aim: 0,
            walking: false,
            disclose: Disclose::default(),
            disclosed: Arc::default(),
            under_pointer: None,
            under_pointer_number: None,
            module_under_pointer: None,
            aside: None,
            revision: 0,
            nucleus,
            mode: ViewMode::Browse,
            selection: None,
            outside: false,
            notice: None,
            said: 0,
            pose: None,
            browsing_seen: Seen::default(),
            expand_pose: None,
            return_pose: Some(Return::Whole),
            frame_asked: false,
            expanded_on: None,
        };
        // As long as each name is drawn.
        let chars = (0..explorer.map.tiles.len())
            .map(|tile| {
                let unit = &explorer.map.tiles[tile].unit;
                explorer.function_name(unit).chars().count()
            })
            .collect();
        explorer.nucleus.name_chars(chars);
        explorer
    }

    pub(crate) fn system(&self) -> &System {
        &self.system
    }

    /// The journey.
    pub(crate) fn track(&self) -> &Track {
        &self.track
    }

    pub(crate) fn journey(&self) -> Option<&Journey> {
        self.track.journey()
    }

    /// The unit the journey starts at, in a journey.
    pub(crate) fn start_unit(&self) -> Option<&UnitId> {
        self.track.start.as_ref().filter(|_| !self.browsing())
    }

    /// In a journey, the function it starts at, drawn white in the nucleus's middle.
    pub(crate) fn start(&self) -> Option<usize> {
        self.start_unit().and_then(|u| self.map.tile_of(u))
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// The view as it is now, to come back to.
    fn view(&self) -> View {
        View {
            start: self.track.start.clone(),
            cursor: self.cursor_span(),
            disclose: self.disclose,
            expanded: self.track.expanded.clone(),
            aside: self.aside,
            mode: self.mode,
            selection: self.selection,
            outside: self.outside,
            pose: Seen {
                pose: self.pose,
                expanded: self.nucleus.expanded_module(),
                resting: self.expand_pose.or(self.pose),
            },
        }
    }

    /// Puts the view back as it was, all of it: the journey and its cursor, how much of
    /// it shows and the function clicked. The nucleus eases back round the function it
    /// was, and the camera back to where it stood.
    fn restore(&mut self, view: View) {
        self.leaving_browse(view.mode);
        let seen = view.pose;
        let track = &mut self.track;
        if track.start != view.start {
            track.start = view.start;
            track.reroot(&self.system);
        }
        // A journey rebuilt since may no longer have the span.
        track.cursor = view
            .cursor
            .filter(|&span| track.span(span).is_some())
            .or_else(|| track.journey().map(Journey::root));
        track.expanded = view.expanded;
        self.disclose = view.disclose;
        self.aside = view.aside;
        self.mode = view.mode;
        self.selection = view.selection;
        self.outside = view.outside;
        self.walking = false;
        self.expanded_on = None;
        // The panel follows the cursor afresh.
        self.revision += 1;
        self.disclose_again();
        self.sync_nucleus();
        self.put_back(seen);
    }

    /// Puts back the module expanded at a place and the camera as it stood there.
    /// Shrinking back then returns to where the camera stood there before any module
    /// was expanded.
    fn put_back(&mut self, seen: Seen) {
        match seen.expanded {
            Some(module) => {
                self.expand_pose = seen.resting;
                self.nucleus.expand(&self.graph, Some(module));
            }
            None => self.drop_expansion(),
        }
        if seen.pose.is_some() {
            self.return_to(seen.pose);
        }
    }

    /// Goes somewhere: whatever `go` changes, the view before it is remembered, so
    /// Back returns to it. A move that changes nothing is not remembered.
    fn navigate(&mut self, go: impl FnOnce(&mut Self)) {
        let before = self.view();
        go(self);
        self.sync_nucleus();
        if before.mode == ViewMode::Browse && self.mode == ViewMode::Journey {
            self.browsing_seen = before.pose;
        }
        if self.view() != before {
            self.history.leave(before);
            self.walking = false;
        }
    }

    /// Returns to the view before the last move, or says there is none. Returns
    /// whether there was one.
    pub(crate) fn go_back(&mut self) -> bool {
        let current = self.view();
        if let Some(view) = self.history.back(current) {
            self.restore(view);
            true
        } else {
            self.say("Nothing to go back to");
            false
        }
    }

    /// Returns to the view gone back from, or says there is none. Returns whether
    /// there was one.
    pub(crate) fn go_forward(&mut self) -> bool {
        let current = self.view();
        if let Some(view) = self.history.forward(current) {
            self.restore(view);
            true
        } else {
            self.say("Nothing to go forward to");
            false
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

    /// Asks the camera to frame the focus, as F does: the panel's Fit.
    pub(crate) fn ask_to_frame(&mut self) {
        self.frame_asked = true;
    }

    /// Whether the camera was asked to frame the focus, once.
    pub(crate) fn take_frame_ask(&mut self) -> bool {
        std::mem::take(&mut self.frame_asked)
    }

    /// The span the journey's cursor is on.
    pub(crate) fn cursor_span(&self) -> Option<SpanId> {
        self.track.cursor
    }

    /// In a journey, the function its cursor is on, the one pink node, marked
    /// wherever it sits, the start included.
    pub(crate) fn cursor(&self) -> Option<usize> {
        self.cursor_unit()
            .and_then(|unit| self.map.tile_of(unit))
            .filter(|_| !self.browsing())
    }

    /// The unit the cursor's span runs.
    pub(crate) fn cursor_unit(&self) -> Option<&UnitId> {
        self.cursor_span().and_then(|id| self.track.unit_id(id))
    }

    /// Moves the cursor. The calls folded over where it lands open, so its bar shows;
    /// a function set aside, and a notice, go.
    fn move_cursor(&mut self, span: Option<SpanId>) {
        // Whatever a key said it could not do is past once the cursor moves.
        self.notice = None;
        self.aside = None;
        self.expanded_on = None;
        self.track.cursor = span;
        self.reveal_cursor();
    }

    /// Moves the cursor to a call picked by hand, as a bar does, remembering where
    /// it was; stepping with the keys moves it without.
    pub(crate) fn go_to(&mut self, span: SpanId) {
        self.navigate(|explorer| explorer.move_cursor(Some(span)));
    }

    /// Opens the calls the cursor is folded under, a level at a time, as a click on
    /// each `+N` would: wherever the cursor goes, its bar shows, and the graph numbers
    /// its node.
    fn reveal_cursor(&mut self) {
        let Some(cursor) = self.cursor_span() else {
            return;
        };
        let stack = self.stack(cursor);
        for _ in 0..stack.len() {
            if self.disclosed.shows(cursor) {
                return;
            }
            // The deepest numbered call above it is shown, and folds what is below.
            let Some(&folding) = stack
                .iter()
                .rev()
                .find(|&&span| self.disclosed.number(span).is_some())
            else {
                return;
            };
            if !self.track.expanded.insert(folding) {
                return;
            }
            self.disclose_again();
        }
    }

    /// Whether the view browses, with no journey open.
    pub(crate) fn browsing(&self) -> bool {
        self.mode == ViewMode::Browse
    }

    /// Browsing, the function selected.
    pub(crate) fn selection(&self) -> Option<usize> {
        self.selection.filter(|_| self.browsing())
    }

    /// The function the view is on: browsing, the selection; in a journey, the
    /// function set aside, else the cursor's. What Enter opens a journey at, and F
    /// frames.
    pub(crate) fn current(&self) -> Option<usize> {
        if self.browsing() {
            return self.selection;
        }
        self.aside
            .or_else(|| self.cursor())
            .or_else(|| self.start())
    }

    /// Enter: opens a journey at what the view is on ([`Self::current`]), or with
    /// nothing selected, says how to pick something.
    pub(crate) fn open_current(&mut self) {
        match self.current() {
            Some(tile) => self.open_at(tile),
            None => self.say("Nothing selected: click a function, then Enter opens its journey"),
        }
    }

    /// F, before the camera frames the focus: expanded, with what the view is on in
    /// another module, takes the expansion there, as a pick does; browsing with
    /// nothing selected or expanded, says F has only the whole map to frame.
    pub(crate) fn frame_focus(&mut self) {
        let focus = self.current().and_then(|tile| self.graph.cluster(tile));
        match self.nucleus.expanded_module() {
            Some(expanded) => {
                if let Some(module) = focus.filter(|&m| m != expanded) {
                    self.nucleus.expand(&self.graph, Some(module));
                }
            }
            None if focus.is_none() => self.say("Nothing selected: F shows the whole map"),
            None => {}
        }
    }

    /// Leaves the journey for browsing, with the journey's start selected,
    /// remembering where it was: browsing as it was before the journey, the module
    /// expanded then expanded again, the camera where it stood.
    pub(crate) fn leave_journey(&mut self) {
        if self.browsing() {
            return;
        }
        let seen = self.browsing_seen;
        self.navigate(|explorer| {
            explorer.selection = explorer.start();
            explorer.mode = ViewMode::Browse;
            explorer.aside = None;
            explorer.drop_expansion();
        });
        self.aim += 1;
        self.put_back(seen);
        if seen.pose.is_none() {
            self.return_to(None);
        }
    }

    /// Remembers how browsing was seen, when the view is about to go from browsing
    /// into a journey (`next`).
    fn leaving_browse(&mut self, next: ViewMode) {
        if self.mode == ViewMode::Browse && next == ViewMode::Journey {
            self.browsing_seen = self.view().pose;
        }
    }

    /// Sends the camera back to `pose`, or with none, to the whole codebase.
    fn return_to(&mut self, pose: Option<Pose>) {
        self.return_pose = Some(pose.map_or(Return::Whole, Return::To));
    }

    /// Where the camera is to go back to, once.
    pub(crate) fn take_return_pose(&mut self) -> Option<Return> {
        self.return_pose.take()
    }

    /// Notes where the camera stands, for the history and for leaving a journey.
    pub(crate) fn see(&mut self, pose: Pose) {
        self.pose = Some(pose);
    }

    /// In a journey, whether every outside caller into it shows.
    pub(crate) fn outside_shown(&self) -> bool {
        self.outside && !self.browsing()
    }

    /// In a journey, shows every function off it that calls the one the cursor is on,
    /// or leaves them out again (I); nothing already there moves. With none to show,
    /// says so instead. Browsing, it says a journey must be open first.
    pub(crate) fn toggle_outside(&mut self) {
        if self.no_journey() {
            return;
        }
        self.outside = !self.outside;
        self.sync_nucleus();
        let waiting = self.nucleus.outside_waiting();
        if self.outside && waiting > 0 {
            self.say(&format!(
                "{waiting} more call it with no room here: X shows them in their module"
            ));
        }
        if self.outside && waiting == 0 && self.nucleus.outside_callers().next().is_none() {
            self.outside = false;
            self.sync_nucleus();
            let name = self
                .cursor()
                .or_else(|| self.start())
                .and_then(|tile| self.tile_unit(tile))
                .map(|unit| self.function_name(unit))
                .unwrap_or_default();
            self.say(&format!("Nothing off the journey calls {name}"));
        }
    }

    /// The one way a journey starts: at `tile`, its start's own calls showing, with
    /// the cursor on its start, remembering where the view was, as deep as it showed:
    /// Back puts that back. The journey it already starts at puts the cursor back on
    /// its start, as deep as it shows. Shift-click,
    /// Enter, `--focus` and a search in a journey all come here.
    pub(crate) fn open_at(&mut self, tile: usize) {
        let Some(unit) = self.tile_unit(tile).cloned() else {
            return;
        };
        self.navigate(|explorer| {
            if explorer.browsing() || explorer.track.start.as_ref() != Some(&unit) {
                explorer.drop_expansion();
            }
            explorer.mode = ViewMode::Journey;
            explorer.aside = None;
            if explorer.track.start.as_ref() == Some(&unit) {
                explorer.move_cursor(explorer.journey().map(Journey::root));
            } else {
                explorer.track.start = Some(unit);
                // A new journey brings in no one from outside it until asked.
                explorer.outside = false;
                explorer.disclose = Disclose::default();
                explorer.track.reroot(&explorer.system);
                explorer.revision += 1;
                explorer.disclose_again();
            }
        });
        self.aim += 1;
    }

    /// The codebase as a graph of calls.
    pub(crate) fn graph(&self) -> &ForceGraph {
        &self.graph
    }

    /// Where a graph node sits in the world: in its module's sphere.
    pub(crate) fn tile_point(&self, tile: usize) -> Option<Vec3> {
        self.nucleus.point(tile)
    }

    /// Bumped whenever nodes move.
    pub(crate) fn moved(&self) -> u64 {
        self.moved
    }

    /// The graph gathered into module spheres.
    pub(crate) fn nucleus(&self) -> &Nucleus {
        &self.nucleus
    }

    /// Gathers the graph into spheres round where the view now is, at once, with
    /// nothing easing there: as the window opens.
    pub(crate) fn gather(&mut self) {
        self.sync_nucleus();
        while self.nucleus.advance(1.0) {}
    }

    /// X, Esc while expanded, or a double click off every function of the module
    /// expanded. With a module expanded, shrinks it back, and the camera returns to
    /// exactly where it stood before, however it moved meanwhile. Otherwise expands
    /// `module`, or with none given, the focus's ([`Self::focus_module`]), remembering
    /// where the camera stands; the camera then flies in until the module takes what
    /// its functions need of the view.
    pub(crate) fn toggle_expand(&mut self, module: Option<usize>) {
        if self.nucleus.expanded() {
            self.nucleus.expand(&self.graph, None);
            self.expanded_on = None;
            let pose = self.expand_pose.take();
            self.return_pose = Some(pose.map_or(Return::Whole, Return::Shrunk));
            return;
        }
        match module.or_else(|| self.focus_module()) {
            Some(module) => {
                self.expand_pose = self.pose;
                self.expanded_on = self.aside();
                self.nucleus.expand(&self.graph, Some(module));
            }
            None => self.say("X expands the selection's module, or the one in the middle"),
        }
    }

    /// A double click on `module`'s sphere: expands it, or with another expanded,
    /// expands it instead, and shrinking back still returns to where the camera
    /// stood before the first.
    pub(crate) fn expand_pointed(&mut self, module: usize) {
        if self.nucleus.expanded_module() == Some(module) {
            return;
        }
        if !self.nucleus.expanded() {
            self.expand_pose = self.pose;
        }
        self.nucleus.expand(&self.graph, Some(module));
    }

    /// Browsing, puts back `selection`, which a double click's first click changed:
    /// a double click expands, or shrinks back, and picks nothing. That click is
    /// forgotten.
    pub(crate) fn keep_selection(&mut self, selection: Option<usize>) {
        if self.browsing() && self.selection != selection {
            self.selection = selection;
            self.history.forget_last();
            self.sync_nucleus();
        }
    }

    /// What X expands: browsing, the selection's module; in a journey, the function
    /// set aside's, else the cursor's, else the start's ([`Self::current`]).
    pub(crate) fn focus_module(&self) -> Option<usize> {
        self.current().and_then(|tile| self.graph.cluster(tile))
    }

    /// The module the pointer is on: the module of the function under it, else the
    /// module whose sphere it is over, in a journey only one that shows some of it.
    pub(crate) fn module_pointed(&self) -> Option<usize> {
        let sphere = self.module_under_pointer.filter(|&cluster| {
            self.nucleus
                .ball(cluster)
                .is_some_and(|ball| self.nucleus.shows_body(ball.body))
        });
        self.under_pointer
            .and_then(|tile| self.graph.cluster(tile))
            .or(sphere)
    }

    /// Shrinks back what is expanded and forgets where the camera stood before, as R
    /// does before it frames everything.
    pub(crate) fn collapse(&mut self) {
        self.drop_expansion();
    }

    /// Shrinks back what is expanded without sending the camera anywhere: the view is
    /// going somewhere else, which frames itself.
    fn drop_expansion(&mut self) {
        self.nucleus.expand(&self.graph, None);
        self.expand_pose = None;
        self.expanded_on = None;
    }

    /// Esc browsing with nothing above it: the selection goes, remembering it, and
    /// the camera stays; with nothing selected, says there is nothing to close.
    pub(crate) fn deselect(&mut self) {
        if self.selection().is_some() {
            self.navigate(|explorer| explorer.selection = None);
        } else {
            self.say(
                "Nothing to close: Esc steps back out of search, help, a journey or a selection",
            );
        }
    }

    /// A click on the graph clear of every function: browsing, the selection goes;
    /// in a journey, the function set aside. The camera stays.
    pub(crate) fn click_empty(&mut self) {
        if self.browsing() {
            if self.selection.is_some() {
                self.navigate(|explorer| explorer.selection = None);
            }
        } else {
            self.clear_aside();
        }
    }

    /// Tells the nucleus which function the pointer has rested on. Returns whether
    /// that changed.
    pub(crate) fn hover(&mut self, tile: Option<usize>) -> bool {
        self.nucleus.hover(tile)
    }

    /// The function hovered: the one the pointer has rested on, in the graph or as a
    /// bar, so passing over a function does not flash. Its name and ring show, and its
    /// bars light.
    pub(crate) fn hovered(&self) -> Option<usize> {
        self.nucleus.hovered()
    }

    /// The function the nucleus is gathered round: in a journey, its start, expanded
    /// or not, so it shows the journey and nothing else; browsing, the selection.
    /// Stepping through the journey, or a click anywhere, leaves it put.
    fn nucleus_start(&self) -> Option<usize> {
        if self.browsing() {
            self.selection
        } else {
            self.start()
        }
    }

    /// Tells the nucleus what it gathers round now: whether the view browses, the
    /// function in the middle, the journey from it, its outside callers, its cursor
    /// and the function set aside. It eases there from wherever things are. Every
    /// change to any of them ends here. Expanded, a cursor or a function set aside in
    /// a journey, or browsing a selection, that moves to another module's function
    /// takes the expansion with it: that module expands instead, and shrinking back
    /// still returns to where the camera stood before the first. Letting go of the
    /// function set aside takes it back to the cursor's.
    fn sync_nucleus(&mut self) {
        let start = self.nucleus_start();
        let cursor = self.cursor();
        let aside = self.aside();
        let browsing = self.browsing();
        let outside = self.outside;
        let graph = &self.graph;
        let trail = if browsing {
            &Trail::default()
        } else {
            self.disclosed.trail()
        };
        let nucleus = &mut self.nucleus;
        // What the expansion follows: browsing, the selection; in a journey, the
        // function set aside, else the cursor.
        let was = if browsing {
            nucleus.start().filter(|_| nucleus.browsing())
        } else {
            nucleus
                .aside()
                .or(nucleus.cursor())
                .filter(|_| !nucleus.browsing())
        };
        let follows = if browsing { start } else { aside.or(cursor) };
        let mut recentered = nucleus.set_browse(graph, browsing);
        nucleus.set_outside(graph, outside);
        recentered |= nucleus.start() != start;
        if recentered {
            nucleus.recenter(graph, start);
        }
        recentered |= nucleus.retrace(graph, trail);
        if nucleus.expanded()
            && was != follows
            && let Some(module) = follows.and_then(|tile| graph.cluster(tile))
        {
            nucleus.expand(graph, Some(module));
        }
        let marked = nucleus.set_cursor(graph, cursor);
        let set_aside = nucleus.set_aside(graph, aside);
        if recentered || marked || set_aside {
            self.moved += 1;
        }
        self.rename();
    }

    /// Works out again the names two or more functions in view share, so each is
    /// named with its type or module before it. In a journey: of the functions it
    /// runs at any depth, so a level more or less never renames anything, its start's
    /// callers, and the outside callers shown. Browsing: of the selection, its
    /// callers and its calls. An entry point goes by its route.
    fn rename(&mut self) {
        let system = &self.system;
        let entries: HashSet<&UnitId> = system.entries().iter().map(|e| &e.unit).collect();
        let graph = &self.graph;
        let selected = self.selection().into_iter().flat_map(|tile| {
            [tile]
                .into_iter()
                .chain(graph.callers(tile))
                .chain(graph.callees(tile))
        });
        let callers = self
            .start()
            .into_iter()
            .flat_map(|start| graph.callers(start))
            .chain(self.nucleus.outside_callers())
            .chain(selected)
            .filter_map(|tile| self.tile_unit(tile));
        let journey = self.track.units.iter().filter(|_| !self.browsing());
        let mut seen: HashMap<&str, &UnitId> = HashMap::new();
        let mut shared = HashSet::new();
        for id in journey.chain(callers) {
            let Some(unit) = system.unit(id).filter(|_| !entries.contains(id)) else {
                continue;
            };
            if *seen.entry(unit.name.as_str()).or_insert(id) != id {
                shared.insert(unit.name.clone());
            }
        }
        self.track.homonyms = shared;
    }

    /// Eases the nucleus along by `dt` seconds. Returns whether anything moved.
    pub(crate) fn ease_nucleus(&mut self, dt: f32) -> bool {
        let moved = self.nucleus.advance(dt);
        if moved {
            self.moved += 1;
        }
        moved
    }

    /// The function set aside in a journey, if any.
    pub(crate) fn aside(&self) -> Option<usize> {
        self.aside.filter(|_| !self.browsing())
    }

    /// Lets go of the function set aside, back to the journey's cursor; a module X
    /// expanded on it shrinks back with it. Returns whether there was one.
    pub(crate) fn clear_aside(&mut self) -> bool {
        let cleared = self.aside.take().is_some();
        if cleared {
            if self.expanded_on.is_some() && self.nucleus.expanded() {
                self.toggle_expand(None);
            }
            self.sync_nucleus();
        }
        cleared
    }

    /// In a journey, the functions that call its start, as the row above the start's
    /// bar lists them: those nearest the journey first, as the graph names them.
    pub(crate) fn start_callers(&self) -> Vec<usize> {
        let Some(start) = self.start() else {
            return Vec::new();
        };
        let whole = &self.disclosed.trail().whole;
        let middle = self.graph.cluster(start);
        crate::nucleus::rank_callers(
            &self.graph,
            start,
            |tile| whole.contains_key(&tile),
            |tile| self.graph.cluster(tile).or(middle).unwrap_or_default(),
        )
    }

    /// Whether anything calls the journey's start but itself: the panel lists them in
    /// a row above it.
    pub(crate) fn start_called(&self) -> bool {
        self.start()
            .is_some_and(|start| self.graph.callers(start).any(|tile| tile != start))
    }

    /// Where the function set aside sits in the row of the start's callers, when it
    /// is one of them: ← and → then move along the row, and ↓ goes back to the start.
    pub(crate) fn aside_caller(&self) -> Option<usize> {
        let aside = self.aside()?;
        self.start_callers().iter().position(|&t| t == aside)
    }

    /// Sets `tile` aside, selected in place, as a click off the journey does.
    fn set_aside(&mut self, tile: usize) {
        self.notice = None;
        self.aside = Some(tile);
    }

    /// A click on a caller's bar in the panel: sets it aside, as ↑ from the start
    /// does, remembering where the view was.
    pub(crate) fn pick_caller(&mut self, tile: usize) {
        self.navigate(|explorer| explorer.set_aside(tile));
    }

    /// Goes to a function as a click does, remembering where the view was. Browsing,
    /// it is selected. In a journey that runs it, the cursor goes to its call, the one
    /// numbered `number` (the number on the name clicked) or else the one nearest the
    /// cursor, opening what folds it; off the journey, or drawn plain in the module
    /// expanded as a call folded away, it is set aside, selected in place, and the
    /// journey stays as it was. Nothing moves, but expanded, where the
    /// view keeps the cursor in sight, flying to its module when it moves to another.
    /// A search picks a function this way too.
    pub(crate) fn click(&mut self, tile: usize, number: Option<&str>) {
        self.navigate(|explorer| {
            if explorer.browsing() {
                explorer.selection = Some(tile);
            } else if let Some(span) = explorer
                .span_at(tile, number)
                .filter(|&span| explorer.disclosed.shows(span) || !explorer.nucleus.opened_up(tile))
            {
                explorer.move_cursor(Some(span));
            } else {
                explorer.set_aside(tile);
            }
        });
    }

    /// The journey's call that runs a node: the cursor's when it already does, the one
    /// numbered `number`, else of those shown the one nearest the cursor in the order
    /// the journey runs them, else the first at all.
    fn span_at(&self, tile: usize, number: Option<&str>) -> Option<SpanId> {
        let unit = self.tile_unit(tile)?;
        let disclosed = &self.disclosed;
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
        if self.cursor_unit() == Some(unit) {
            return self.cursor_span();
        }
        let cursor = self.cursor_span().map_or(0, SpanId::index);
        runs.iter()
            .copied()
            .filter(|&span| disclosed.shows(span))
            .min_by_key(|span| span.index().abs_diff(cursor))
            .or_else(|| runs.first().copied())
    }

    pub(crate) fn disclose(&self) -> Disclose {
        self.disclose
    }

    /// `.` and the panel's +: a level more of the journey, down to its deepest call.
    pub(crate) fn deeper(&mut self) {
        if self.no_journey() {
            return;
        }
        let Disclose(depth) = self.disclose;
        if depth >= self.disclosed.deepest() {
            self.say("No deeper calls: every level shows");
        } else {
            self.set_disclose(Disclose(depth + 1));
        }
    }

    /// `,` and the panel's −: a level less of the journey, down to the start's own
    /// calls.
    pub(crate) fn shallower(&mut self) {
        if self.no_journey() {
            return;
        }
        let Disclose(depth) = self.disclose;
        if depth <= 1 {
            self.say("Only the start's own calls show: no fewer levels");
        } else {
            self.set_disclose(Disclose(depth - 1));
        }
    }

    /// Browsing, says that a journey key needs a journey open, and returns true.
    fn no_journey(&mut self) -> bool {
        if self.browsing() {
            self.say(NO_JOURNEY);
        }
        self.browsing()
    }

    /// Shows the journey to another depth. A cursor a level less folds away goes up
    /// to the nearest call above it still drawn, in the graph and the panel alike.
    pub(crate) fn set_disclose(&mut self, disclose: Disclose) {
        self.disclose = disclose;
        self.disclose_again();
        if let Some(cursor) = self.cursor_span()
            && !self.drawn(cursor)
        {
            let above = self
                .stack(cursor)
                .into_iter()
                .rev()
                .find(|&s| self.drawn(s));
            self.track.cursor = above.or_else(|| self.journey().map(Journey::root));
        }
        self.sync_nucleus();
    }

    /// Whether a call is drawn: disclosed, with a node of its own (not a closure,
    /// nor a call to one of several functions). The cursor only ever rests on one.
    fn drawn(&self, span: SpanId) -> bool {
        let Some(journey) = self.journey() else {
            return false;
        };
        let tile = |span: SpanId| self.track.tile(span, &self.map);
        tile(span).is_some()
            && !disclosure::is_closure(journey, &tile, span)
            && self.disclosed.number(span).is_some()
    }

    /// Opens or folds the calls under one span, past the disclosed depth.
    pub(crate) fn toggle_expanded(&mut self, span: SpanId) {
        let expanded = &mut self.track.expanded;
        if !expanded.remove(&span) {
            expanded.insert(span);
        }
        self.disclose_again();
        self.sync_nucleus();
    }

    /// Works out what the journey discloses again, after the journey, its depth or
    /// what is opened by hand changed.
    fn disclose_again(&mut self) {
        self.disclosed = Arc::new(Disclosed::of(
            &self.track,
            &self.map,
            self.disclose,
            &self.track.expanded,
        ));
    }

    /// How many folded calls are opened by hand, beside the depth.
    pub(crate) fn folds_opened(&self) -> usize {
        self.track.expanded.len()
    }

    /// What the journey discloses; browsing, nothing.
    pub(crate) fn disclosure(&self) -> Arc<Disclosed> {
        if self.browsing() {
            return Arc::default();
        }
        Arc::clone(&self.disclosed)
    }

    /// The function a node stands for.
    pub(crate) fn tile_unit(&self, tile: usize) -> Option<&UnitId> {
        self.map.tile(tile).map(|t| &t.unit)
    }

    /// A module's name, with as much of its path as tells it from another module's of
    /// the same name.
    pub(crate) fn cluster_name(&self, cluster: usize) -> String {
        self.module_names.get(cluster).cloned().unwrap_or_default()
    }

    pub(crate) fn map(&self) -> &CodebaseMap {
        &self.map
    }

    pub(crate) fn span(&self, id: SpanId) -> Option<&Span> {
        self.track.span(id)
    }

    /// The name of what a span calls: the function's, as [`Self::function_name`] has
    /// it, so a bar says what its node's label says.
    pub(crate) fn label(&self, id: SpanId) -> String {
        let Some(span) = self.span(id) else {
            return String::new();
        };
        match &span.target {
            SpanTarget::Unit(unit) => self.function_name(unit),
            SpanTarget::Ambiguous(ids) => format!(
                "{} → one of {}",
                span.text.as_deref().unwrap_or("?"),
                ids.len()
            ),
            SpanTarget::Unresolved(text) => text.clone(),
        }
    }

    /// Why the journey stops at a span, as a bar marks it: `↻` it is already running
    /// further up, `…` the journey goes no deeper, `→` handed off to run later.
    pub(crate) fn stop_mark(&self, id: SpanId) -> Option<&'static str> {
        match self.span(id)?.stopped? {
            StopReason::Recursion => Some("↻"),
            StopReason::DepthLimit => Some("…"),
            StopReason::Registered => Some("→"),
            StopReason::NotLoaded => None,
        }
    }

    /// A function's name, the same wherever it shows (the graph, the panel's bars,
    /// the top bar, the search): an entry point's route, else its own name; with two
    /// functions of that name in view, or in its own module, with its type or module
    /// before it (`Track.root`, `Explorer.root`).
    pub(crate) fn function_name(&self, id: &UnitId) -> String {
        if let Some(entry) = self.system.entries().iter().find(|e| &e.unit == id) {
            return entry.label.clone();
        }
        let module = self.map.tile_of(id).and_then(|t| self.graph.cluster(t));
        let shared = module.and_then(|m| self.module_homonyms.get(m));
        match self.system.unit(id) {
            Some(unit)
                if !self.track.homonyms.contains(&unit.name)
                    && !shared.is_some_and(|shared| shared.contains(&unit.name)) =>
            {
                unit.name.clone()
            }
            Some(_) => self.unit_name(id),
            None => id.to_string(),
        }
    }

    /// The names of `ids` as [`Self::function_name`] has them, but with any two of
    /// them alike told apart by their type or module: a list read together, as the
    /// search's rows are.
    pub(crate) fn function_names(&self, ids: &[&UnitId]) -> Vec<String> {
        let names: Vec<String> = ids.iter().map(|id| self.function_name(id)).collect();
        names
            .iter()
            .zip(ids)
            .map(|(name, id)| {
                if names.iter().filter(|n| *n == name).count() > 1 {
                    self.unit_name(id)
                } else {
                    name.clone()
                }
            })
            .collect()
    }

    /// A function's name with its type or module before it; where another in its
    /// module reads the same so, with the impl it is in after it.
    pub(crate) fn unit_name(&self, id: &UnitId) -> String {
        let name = display_name(&self.system, id);
        match self.twins.get(id) {
            Some(apart) => format!("{name} ({apart})"),
            None => name,
        }
    }

    /// The call stack at a span of the journey, outermost first.
    pub(crate) fn stack(&self, id: SpanId) -> Vec<SpanId> {
        let mut stack: Vec<SpanId> =
            std::iter::successors(Some(id), |s| self.span(*s).and_then(|s| s.parent)).collect();
        stack.reverse();
        stack
    }

    /// Moves the cursor. Returns the new position. With a function set aside, it only
    /// goes back to the cursor. A walk of steps is remembered once, from where it
    /// began.
    pub(crate) fn step(&mut self, step: Step) -> Option<SpanId> {
        let before = self.view();
        let next = self.walk(step);
        self.sync_nucleus();
        if !self.walking && self.view() != before {
            self.history.leave(before);
            self.walking = true;
        }
        next
    }

    fn walk(&mut self, step: Step) -> Option<SpanId> {
        if self.no_journey() {
            return None;
        }
        if let Some(index) = self.aside_caller() {
            return self.walk_callers(step, index);
        }
        if self.clear_aside() {
            return self.cursor_span();
        }
        let journey = self.journey()?;
        let current = self.cursor_span().unwrap_or_else(|| journey.root());
        // Up from the start, into the row of its callers, the first of them set aside.
        if step == Step::Parent
            && current == journey.root()
            && let Some(&first) = self.start_callers().first()
        {
            self.set_aside(first);
            return None;
        }
        let tile = |span: SpanId| self.track.tile(span, &self.map);
        // A closure is drawn as part of the function around it, with no bar or node of
        // its own: the cursor never stops on one, and the calls made inside it are its
        // caller's.
        let closure = |span: SpanId| disclosure::is_closure(journey, &tile, span);
        // The keys read the picture as it is: they stop only on a call drawn, so they
        // never open a level or reach past the depth shown.
        let disclosed = &self.disclosed;
        let drawn = |span: SpanId| self.drawn(span);
        let parent = |span: SpanId| {
            std::iter::successors(journey.span(span)?.parent, |&p| journey.span(p)?.parent)
                .find(|&p| drawn(p))
        };
        let calls = |span: SpanId| {
            let mut calls = disclosed.children(span).to_vec();
            calls.retain(|&c| drawn(c));
            calls
        };
        let nth = |index: usize| journey.spans().nth(index).map(|(id, _)| id);
        // On from `span` in pre-order, past anything not drawn.
        let onward = |span: Option<SpanId>| {
            std::iter::successors(span, |s| nth(s.index() + 1)).find(|&s| drawn(s))
        };
        let siblings = || parent(current).map(calls).unwrap_or_default();
        let place = |list: &[SpanId]| list.iter().position(|&s| s == current);
        let next = match step {
            Step::Into => onward(nth(current.index() + 1)),
            Step::Back => (0..current.index())
                .rev()
                .filter_map(nth)
                .find(|&s| drawn(s)),
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
            self.move_cursor(next);
        } else {
            let hidden = |span: SpanId| {
                tile(span).is_some() && !closure(span) && disclosed.number(span).is_none()
            };
            // Calls past the depth shown: under the cursor for ↓, anywhere ahead
            // for ] and the like.
            let folded = match step {
                Step::Child => disclosed.children(current).iter().any(|&c| hidden(c)),
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

    /// The keys along the row of the start's callers, with the caller at `index` set
    /// aside: ← and → move along it, ↓ goes back down to the start, ↑ says how to go
    /// further up; any other key goes back to the cursor.
    fn walk_callers(&mut self, step: Step, index: usize) -> Option<SpanId> {
        let callers = self.start_callers();
        let along = match step {
            Step::Next => Some(index + 1),
            Step::Previous => index.checked_sub(1),
            Step::Parent => {
                self.say("Enter starts a journey at this caller, to go further up");
                return None;
            }
            Step::Child => {
                self.move_cursor(self.journey().map(Journey::root));
                return self.cursor_span();
            }
            Step::Into | Step::Back => {
                self.clear_aside();
                return self.cursor_span();
            }
        };
        match along.and_then(|i| callers.get(i)) {
            Some(&tile) => self.set_aside(tile),
            None if step == Step::Next => self.say("No later caller"),
            None => self.say("No earlier caller"),
        }
        None
    }

    /// Says something for a moment, over the graph.
    pub(crate) fn say(&mut self, text: &str) {
        self.said += 1;
        self.notice = Some((self.said, text.to_owned()));
    }

    /// The last thing said for a moment, numbered so the same words said again show
    /// again.
    pub(crate) fn notice(&self) -> Option<(u64, &str)> {
        self.notice.as_ref().map(|(n, text)| (*n, text.as_str()))
    }
}

/// Functions whose name with its type or module reads the same as another's in
/// their module (two impls of one trait for a type, as `From<&str>` and
/// `From<String>`), each with the impl it is in, from its id.
fn twins(system: &System, map: &CodebaseMap, graph: &ForceGraph) -> HashMap<UnitId, String> {
    let mut by_name: HashMap<(usize, String), Vec<&UnitId>> = HashMap::new();
    for (tile, at) in map.tiles.iter().enumerate() {
        if let Some(cluster) = graph.cluster(tile) {
            let name = display_name(system, &at.unit);
            by_name.entry((cluster, name)).or_default().push(&at.unit);
        }
    }
    by_name
        .into_values()
        .filter(|ids| ids.len() > 1)
        .flatten()
        .map(|id| {
            let path = id.as_str();
            let apart = path
                .split("::")
                .find_map(|part| part.strip_prefix("{impl#")?.strip_suffix('}'))
                .map_or_else(|| path.to_owned(), str::to_owned);
            (id.clone(), apart)
        })
        .collect()
}

/// What a journey key says browsing.
const NO_JOURNEY: &str = "Open a journey first (Enter)";
/// What ↑ from the start says when nothing is found calling it: what the analysis
/// cannot see may still call it.
const NO_CALLERS: &str = "No callers found (callbacks and dynamic calls may not be traced)";

/// What a key that cannot move the cursor says: why not, and what would. `first` is
/// whether the cursor is on the journey's start, `folded` whether calls under it are
/// past the depth shown.
fn stuck(step: Step, first: bool, folded: bool) -> &'static str {
    match step {
        Step::Parent if first => NO_CALLERS,
        Step::Back if first => "Already at the journey's start",
        Step::Parent => "Nothing above this call is drawn",
        Step::Back => "No earlier call is drawn",
        Step::Next => "No later call from the same caller",
        Step::Previous => "No earlier call from the same caller",
        Step::Child if folded => "Its calls are folded: . shows a level more",
        Step::Child => "It makes no calls",
        Step::Into if folded => "The end of what is shown: . shows a level more",
        Step::Into => "The end of the journey",
    }
}

/// Debugger-style cursor movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// The next call in time, entering callees.
    Into,
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

/// Whether a span runs only sometimes on this path: under a condition, or in a loop.
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
        opened(system, "x:main")
    }

    /// An explorer with the journey from `start` open, as if it had opened there.
    pub(crate) fn opened(system: System, start: &str) -> Explorer {
        let mut explorer = Explorer::new(system);
        let tile = explorer.map().tile_of(&UnitId::new(start)).unwrap();
        explorer.open_at(tile);
        explorer.forget_history();
        explorer.gather();
        explorer
    }

    /// What Enter does.
    fn enter(explorer: &mut Explorer) {
        if let Some(tile) = explorer.current() {
            explorer.open_at(tile);
        }
    }

    fn at(explorer: &Explorer) -> String {
        explorer.label(explorer.cursor_span().unwrap())
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
        assert!(
            explorer.notice().is_none(),
            "said only until the cursor moves"
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
        explorer.set_disclose(Disclose(2));
        explorer.step(Step::Child);
        assert_eq!(at(&explorer), "c");
        assert_eq!(explorer.step(Step::Child), None);
        assert_eq!(explorer.notice().map(|(_, t)| t), Some("It makes no calls"));
        explorer.step(Step::Parent);
        assert_eq!(at(&explorer), "a");
        explorer.step(Step::Parent);
        assert_eq!(explorer.step(Step::Parent), None);
        assert_eq!(explorer.notice().map(|(_, t)| t), Some(NO_CALLERS));
    }

    #[test]
    fn rerooting_keeps_history() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        enter(&mut explorer);
        assert_eq!(explorer.journey().map(Journey::len), Some(2));
        assert!(explorer.can_go_back());

        explorer.go_back();
        assert_eq!(explorer.journey().map(Journey::len), Some(4));
        assert_eq!(at(&explorer), "a", "back where the cursor was");
        assert!(explorer.can_go_back(), "and the step there before it");
        assert!(explorer.can_go_forward());

        explorer.go_forward();
        assert_eq!(explorer.start_unit().map(UnitId::as_str), Some("x:a"));
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
        explorer.set_disclose(Disclose(2));
        explorer.toggle_expanded(a_span);
        explorer.step(Step::Into);
        explorer.step(Step::Into);
        let before = explorer.view();

        explorer.open_at(tile(&explorer, "b"));
        assert_eq!(explorer.start_unit().map(UnitId::as_str), Some("x:b"));
        assert_eq!(
            explorer.disclose(),
            Disclose::default(),
            "b opens at its own calls"
        );

        assert!(explorer.go_back());
        assert_eq!(explorer.view(), before);
        assert_eq!(at(&explorer), "c", "the cursor where it was");
        assert_eq!(explorer.disclose(), Disclose(2));
        assert!(
            explorer.disclosure().shows(a_span),
            "a still opened by hand"
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
        let root = |explorer: &Explorer| explorer.start_unit().map(|u| u.as_str().to_owned());
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
        explorer.set_disclose(Disclose(3));
        assert!(!explorer.can_go_back(), "showing more is not a move");
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
            explorer.start_unit().map(UnitId::as_str),
            Some("x:main"),
            "opening the same node twice is one move"
        );
    }

    #[test]
    fn a_walk_with_the_keys_is_one_move() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.set_disclose(Disclose(2));
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
        explorer.click(tile(&explorer, "main"), None);
        explorer.set_disclose(Disclose(2));
        explorer.step(Step::Into);
        explorer.step(Step::Into);
        assert!(explorer.go_back());
        assert_eq!(at(&explorer), "main", "back to where the second walk began");
        assert!(explorer.go_back());
        assert_eq!(at(&explorer), "a", "then to before the click");
    }

    #[test]
    fn the_keys_never_open_what_is_folded() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        let a = explorer.cursor_span().unwrap();
        assert_eq!(explorer.disclosure().folded(a), 1, "c is folded under a");
        assert_eq!(explorer.step(Step::Child), None);
        assert_eq!(at(&explorer), "a", "the cursor stays on what shows");
        assert_eq!(explorer.disclosure().folded(a), 1, "and c stays folded");
        explorer.toggle_expanded(a);
        explorer.step(Step::Child);
        assert_eq!(at(&explorer), "c", "opened by hand, it is there to step to");
        assert_eq!(
            explorer
                .disclosure()
                .number(explorer.cursor_span().unwrap()),
            Some("1.1")
        );
    }

    #[test]
    fn a_guess_is_drawn_only_to_what_the_journey_runs_anyway() {
        // main's first call could run b or c; its second runs d, which runs b. A
        // guess between e and f, neither of which the journey runs, is drawn nowhere.
        let json = r#"{ "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" }, "units": [
            { "id": "x:main", "name": "main", "kind": "fn", "role": "invocable", "calls": [
                { "key": "dyn#0", "text": "handler.run", "targets": [
                    { "unit": "x:b", "certainty": "exact" }, { "unit": "x:c", "certainty": "exact" } ] },
                { "key": "d#0", "text": "d", "targets": [ { "unit": "x:d", "certainty": "exact" } ] },
                { "key": "dyn#1", "text": "other.run", "targets": [
                    { "unit": "x:e", "certainty": "exact" }, { "unit": "x:f", "certainty": "exact" } ] } ] },
            { "id": "x:b", "name": "b", "kind": "fn", "role": "invocable" },
            { "id": "x:c", "name": "c", "kind": "fn", "role": "invocable" },
            { "id": "x:d", "name": "d", "kind": "fn", "role": "invocable", "calls": [
                { "key": "b#0", "text": "b", "targets": [ { "unit": "x:b", "certainty": "exact" } ] } ] },
            { "id": "x:e", "name": "e", "kind": "fn", "role": "invocable" },
            { "id": "x:f", "name": "f", "kind": "fn", "role": "invocable" } ] }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let explorer = opened(system, "x:main");
        let [main, b, c, e] = ["main", "b", "c", "e"].map(|name| tile(&explorer, name));
        let disclosed = explorer.disclosure();
        let trail = disclosed.trail();
        assert!(
            trail
                .calls
                .iter()
                .any(|call| call.from == main && call.to == b && call.guessed)
        );
        for not in [c, e] {
            assert!(
                !trail.calls.iter().any(|call| call.to == not),
                "no line to it"
            );
            assert!(!trail.whole.contains_key(&not), "nor laid out for");
        }
        let journey = explorer.journey().unwrap();
        let calls = &journey.span(journey.root()).unwrap().children;
        assert_eq!(disclosed.bar_number(calls[0]), Some("1"), "on b, numbered");
        assert_eq!(
            disclosed.bar_number(calls[2]),
            None,
            "drawn nowhere, unnumbered"
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
        let mut explorer = opened(system, "x:main");
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

        explorer.click(tile(&explorer, "c"), None);
        assert_eq!(at(&explorer), "c");
        assert_eq!(explorer.start_unit().map(UnitId::as_str), Some("x:main"));
        assert!(
            explorer.disclosure().shows(explorer.cursor_span().unwrap()),
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
        explorer.set_disclose(Disclose(2));
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");
        let b = tile(&explorer, "b");

        // Off the journey, at another depth.
        explorer.click(b, None);
        explorer.set_disclose(Disclose(1));
        assert_eq!(explorer.aside(), Some(b));
        assert!(explorer.go_back());
        assert_eq!(explorer.aside(), None, "the node clicked off it is let go");
        assert_eq!(
            at(&explorer),
            "c",
            "the cursor is back where the keys left it"
        );
        assert_eq!(explorer.disclose(), Disclose(2), "at the depth it was");

        assert!(explorer.go_forward());
        assert_eq!(explorer.aside(), Some(b), "forward clicks it again");
        assert!(explorer.go_back());

        // Gathered round the journey, a click moves the cursor and nothing else.
        explorer.gather();
        explorer.ease_nucleus(1.0);
        let middle = explorer.nucleus().start();
        assert_eq!(middle, Some(tile(&explorer, "a")), "the journey's start");
        explorer.click(tile(&explorer, "a"), None);
        explorer.ease_nucleus(1.0);
        assert_eq!(at(&explorer), "a");
        assert_eq!(explorer.nucleus().start(), middle);
        assert!(explorer.go_back());
        explorer.ease_nucleus(1.0);
        assert_eq!(explorer.nucleus().start(), middle);
        assert_eq!(at(&explorer), "c", "Back puts the cursor back");
    }

    #[test]
    fn a_node_set_aside_stays_aside_until_followed_or_left() {
        let mut explorer = explorer();
        explorer.open_at(tile(&explorer, "a"));
        explorer.forget_history();
        let b = tile(&explorer, "b");

        explorer.click(b, None);
        assert_eq!(explorer.aside(), Some(b));
        assert_eq!(explorer.current(), Some(b), "Enter acts on it");
        assert_eq!(at(&explorer), "a", "the cursor stays put");

        assert_eq!(
            explorer.step(Step::Into).map(|s| explorer.label(s)),
            Some("a".to_owned())
        );
        assert!(
            explorer.aside().is_none(),
            "an arrow goes back to the cursor"
        );
        assert_eq!(at(&explorer), "a", "without stepping");

        explorer.click(b, None);
        assert!(explorer.clear_aside(), "Esc goes back too");
        assert_eq!(explorer.current(), Some(tile(&explorer, "a")));

        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");
        explorer.click(b, None);
        enter(&mut explorer);
        assert_eq!(
            explorer.start_unit().map(UnitId::as_str),
            Some("x:b"),
            "Enter follows it"
        );
        assert!(explorer.aside().is_none());
        assert!(explorer.go_back());
        assert_eq!(explorer.start_unit().map(UnitId::as_str), Some("x:a"));
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
        assert_eq!(explorer.start_unit().map(UnitId::as_str), Some("x:b"));
        assert!(explorer.go_back(), "one move to go back from");
        assert_eq!(explorer.start_unit().map(UnitId::as_str), Some("x:main"));
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
        assert_eq!(explorer.start_unit().map(UnitId::as_str), Some("x:a"));
        assert_eq!(explorer.start(), Some(a));

        explorer.open_at(main);
        let numbers = |explorer: &Explorer| {
            let mut tiles: Vec<(usize, Vec<String>)> =
                explorer.disclosure().tiles().clone().into_iter().collect();
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
            .cursor_span()
            .and_then(|root| explorer.journey()?.span(root)?.children.first().copied());
        assert_eq!(
            a_span.map(|s| explorer.disclosure().folded(s)),
            Some(1),
            "c is folded"
        );

        explorer.toggle_expanded(a_span.unwrap());
        assert!(numbers(&explorer).contains(&(c, vec!["1.1".to_owned()])));
        explorer.toggle_expanded(a_span.unwrap());
        explorer.set_disclose(Disclose(2));
        assert!(numbers(&explorer).contains(&(c, vec!["1.1".to_owned()])));

        explorer.step(Step::Into);
        assert_eq!(explorer.start(), Some(main), "the camera stays on main");

        assert!(explorer.go_back(), "back over the walk");
        assert!(explorer.go_back());
        assert_eq!(explorer.start_unit().map(UnitId::as_str), Some("x:a"));
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
        explorer.gather();
        explorer.ease_nucleus(1.0);
        assert_eq!(explorer.nucleus().start(), Some(main));

        explorer.step(Step::Into);
        explorer.ease_nucleus(1.0);
        assert_eq!(explorer.current(), Some(a), "the cursor moved");
        assert_eq!(explorer.nucleus().start(), Some(main), "the middle did not");
        assert_eq!(explorer.cursor(), Some(a), "the cursor is marked");
        assert_eq!(explorer.nucleus().cursor(), Some(a));

        explorer.click(b, None);
        explorer.ease_nucleus(1.0);
        assert_eq!(
            explorer.nucleus().start(),
            Some(main),
            "a click moves the cursor, not the middle"
        );
        assert_eq!(
            explorer.cursor(),
            Some(b),
            "the cursor is the one pink node"
        );
        explorer.step(Step::Back);
        explorer.ease_nucleus(1.0);
        assert_eq!(explorer.nucleus().start(), Some(main));

        enter(&mut explorer);
        explorer.ease_nucleus(1.0);
        assert_eq!(
            explorer.nucleus().start(),
            explorer.current(),
            "following the function set aside re-centers on it"
        );
    }

    #[test]
    fn expanded_a_click_on_a_call_folded_away_opens_nothing() {
        let mut explorer = explorer();
        let c = tile(&explorer, "c");
        explorer.gather();
        explorer.toggle_expand(None);
        explorer.ease_nucleus(1.0);
        assert!(explorer.nucleus().opened_up(c), "c drawn plain");
        let shown = explorer.disclosure().tiles().len();
        explorer.click(c, None);
        assert_eq!(explorer.aside(), Some(c), "set aside, as off the journey");
        assert_eq!(at(&explorer), "main", "the cursor stays");
        assert_eq!(explorer.disclosure().tiles().len(), shown, "no bar unfolds");
    }

    #[test]
    fn expanded_a_click_moves_the_cursor_and_keeps_the_journey() {
        let mut explorer = explorer();
        let [main, a, b] = ["main", "a", "b"].map(|name| tile(&explorer, name));
        explorer.gather();
        explorer.toggle_expand(None);
        explorer.ease_nucleus(1.0);
        let named = |explorer: &Explorer| {
            let nucleus = explorer.nucleus();
            [main, a, b].map(|t| nucleus.level(t).is_some() || nucleus.start() == Some(t))
        };
        let before = named(&explorer);

        explorer.click(a, None);
        explorer.ease_nucleus(1.0);
        let nucleus = explorer.nucleus();
        assert_eq!(
            nucleus.start(),
            Some(main),
            "the journey still starts at main"
        );
        assert_eq!(explorer.cursor(), Some(a), "the clicked call is the cursor");
        assert_eq!(named(&explorer), before, "naming the same journey");

        explorer.step(Step::Next);
        explorer.ease_nucleus(1.0);
        assert_eq!(explorer.cursor(), Some(b), "a key moves it the same way");
        explorer.go_to(explorer.journey().unwrap().root());
        explorer.ease_nucleus(1.0);
        assert_eq!(explorer.cursor(), Some(main), "and a bar");
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
        let returned = |explorer: &mut Explorer| match explorer.take_return_pose() {
            Some(Return::To(pose) | Return::Shrunk(pose)) => Some(pose),
            _ => None,
        };

        let mut explorer = explorer();
        let [a, b] = ["a", "b"].map(|name| tile(&explorer, name));
        explorer.see(first);
        explorer.click(a, None);
        explorer.see(second);
        assert!(explorer.go_back());
        assert_eq!(
            returned(&mut explorer),
            Some(first),
            "Back looks as it looked"
        );
        assert!(explorer.go_forward());
        assert_eq!(returned(&mut explorer), Some(second), "and so does Forward");

        explorer.leave_journey();
        explorer.see(third);
        explorer.click(b, None);
        enter(&mut explorer);
        explorer.see(first);
        explorer.leave_journey();
        assert!(explorer.browsing());
        assert_eq!(
            returned(&mut explorer),
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
    fn browsing_x_expands_the_selections_module_where_it_is() {
        use crate::camera::OrbitCamera;
        let mut camera = OrbitCamera::default();
        let before = camera.pose();
        camera.pan_to(Vec3::new(40.0, 0.0, 0.0));
        let moved = camera.pose();

        let mut explorer = explorer();
        let a = tile(&explorer, "a");
        explorer.leave_journey();
        explorer.click(a, None);
        while explorer.ease_nucleus(1.0) {}
        let points = |explorer: &Explorer| -> Vec<Option<Vec3>> {
            (0..explorer.map().tiles.len())
                .map(|t| explorer.tile_point(t))
                .collect()
        };
        let placed = points(&explorer);
        explorer.take_return_pose();
        explorer.see(before);
        explorer.toggle_expand(None);
        assert_eq!(
            explorer.nucleus().expanded_module(),
            explorer.graph().cluster(a),
            "the selection's module"
        );
        while explorer.ease_nucleus(1.0) {}
        let module = explorer.graph().cluster(a);
        let others = |points: &[Option<Vec3>]| -> Vec<Option<Vec3>> {
            points
                .iter()
                .enumerate()
                .filter(|&(t, _)| explorer.graph().cluster(t) != module)
                .map(|(_, p)| *p)
                .collect()
        };
        assert_eq!(
            others(&points(&explorer)),
            others(&placed),
            "nothing but the module opened moves"
        );
        assert!(
            explorer.nucleus().opened_up(a),
            "its functions are drawn plainly"
        );

        explorer.see(moved);
        explorer.toggle_expand(None);
        assert!(!explorer.nucleus().expanded());
        assert_eq!(
            explorer.take_return_pose(),
            Some(Return::Shrunk(before)),
            "X again goes back to exactly where the camera stood"
        );
    }

    #[test]
    fn a_click_off_every_function_deselects_and_leaves_the_camera() {
        let mut explorer = explorer();
        let a = tile(&explorer, "a");
        explorer.leave_journey();
        explorer.click(a, None);
        explorer.take_return_pose();
        explorer.click_empty();
        assert_eq!(explorer.selection(), None);
        assert_eq!(explorer.take_return_pose(), None, "the camera stays");
        assert!(explorer.go_back(), "Back selects it again");
        assert_eq!(explorer.selection(), Some(a));

        explorer.open_at(a);
        let b = tile(&explorer, "b");
        explorer.click(b, None);
        assert_eq!(explorer.aside(), Some(b));
        explorer.click_empty();
        assert_eq!(
            explorer.aside(),
            None,
            "in a journey, the function set aside goes"
        );
        assert!(!explorer.browsing());
    }

    #[test]
    fn a_double_click_puts_back_the_selection_its_first_click_changed() {
        let mut explorer = explorer();
        let (a, b) = (tile(&explorer, "a"), tile(&explorer, "b"));
        explorer.leave_journey();
        explorer.click(a, None);
        explorer.click(b, None);
        explorer.keep_selection(Some(a));
        assert_eq!(explorer.selection(), Some(a));
        assert!(explorer.go_back());
        assert_ne!(explorer.selection(), Some(a), "that click is forgotten");
    }

    #[test]
    fn letting_go_of_the_function_x_expanded_on_shrinks_back() {
        use crate::camera::OrbitCamera;
        let mut camera = OrbitCamera::default();
        let before = camera.pose();
        camera.pan_to(Vec3::new(40.0, 0.0, 0.0));
        let mut explorer = explorer();
        let (a, b) = (tile(&explorer, "a"), tile(&explorer, "b"));
        explorer.open_at(a);
        explorer.click(b, None);
        assert_eq!(explorer.aside(), Some(b));
        explorer.see(before);
        explorer.toggle_expand(None);
        assert!(explorer.nucleus().expanded());
        explorer.see(camera.pose());
        explorer.take_return_pose();
        assert!(explorer.clear_aside());
        assert!(!explorer.nucleus().expanded(), "shrunk back with it");
        assert_eq!(explorer.take_return_pose(), Some(Return::Shrunk(before)));
    }

    #[test]
    fn shrinking_back_returns_the_camera_to_where_it_stood_before_expanding() {
        use crate::camera::OrbitCamera;
        let mut camera = OrbitCamera::default();
        let before = camera.pose();
        camera.pan_to(Vec3::new(40.0, 0.0, 0.0));
        let moved = camera.pose();
        let returned = |explorer: &mut Explorer| match explorer.take_return_pose() {
            Some(Return::To(pose) | Return::Shrunk(pose)) => Some(pose),
            _ => None,
        };

        let mut explorer = explorer();
        let b = tile(&explorer, "b");
        explorer.see(before);
        explorer.toggle_expand(None);
        assert!(explorer.nucleus().expanded());
        explorer.see(moved);
        explorer.click(b, None);
        explorer.toggle_expand(None);
        assert_eq!(
            returned(&mut explorer),
            Some(before),
            "however the camera moved while expanded"
        );

        assert!(explorer.go_back());
        assert!(
            explorer.nucleus().expanded(),
            "Back to a place seen expanded"
        );
        assert_eq!(returned(&mut explorer), Some(moved), "seen as it was");
        explorer.toggle_expand(None);
        assert_eq!(
            returned(&mut explorer),
            Some(before),
            "and shrinking back returns to where the camera stood before expanding"
        );
    }

    #[test]
    fn back_and_esc_from_a_journey_entered_while_expanded_expand_again() {
        use crate::camera::OrbitCamera;
        let mut camera = OrbitCamera::default();
        let before = camera.pose();
        camera.pan_to(Vec3::new(40.0, 0.0, 0.0));
        let inside = camera.pose();
        let returned = |explorer: &mut Explorer| match explorer.take_return_pose() {
            Some(Return::To(pose) | Return::Shrunk(pose)) => Some(pose),
            _ => None,
        };
        // Browsing, a module expanded, then into a journey.
        let entered = || {
            let mut explorer = explorer();
            let main = tile(&explorer, "main");
            explorer.leave_journey();
            explorer.click(main, None);
            explorer.see(before);
            explorer.toggle_expand(None);
            explorer.see(inside);
            explorer.open_at(main);
            explorer.take_return_pose();
            explorer
        };
        let mut explorer = entered();
        explorer.leave_journey();
        assert_eq!(
            returned(&mut explorer),
            Some(inside),
            "Esc: where it stood expanded"
        );
        assert!(
            explorer.nucleus().expanded(),
            "Esc browses expanded, as before"
        );
        explorer.toggle_expand(None);
        assert_eq!(returned(&mut explorer), Some(before), "X: before expanding");
        // Back puts the place back as it was left: expanded, the camera inside, and
        // shrinking back still returns to where it stood before expanding.
        let mut explorer = entered();
        assert!(explorer.go_back());
        assert!(explorer.nucleus().expanded(), "Back expands again");
        assert_eq!(
            returned(&mut explorer),
            Some(inside),
            "where it stood expanded"
        );
        explorer.toggle_expand(None);
        assert_eq!(returned(&mut explorer), Some(before), "X: before expanding");
    }

    /// Three modules: `core`, whose `main` calls `put` and `help`, `db`, holding
    /// `put`, and `ui`, holding `draw`, which nothing on the journey calls.
    fn core_and_db() -> System {
        let module = |name: &str| {
            format!(
                r#"{{ "id": "m:{name}", "name": "{name}", "kind": "module", "role": "group" }}"#
            )
        };
        let function = |parent: &str, name: &str, calls: &[&str]| {
            let calls: Vec<String> = calls
                .iter()
                .map(|c| {
                    format!(
                        r#"{{ "key": "{c}#0", "text": "{c}", "targets": [{{ "unit": "f:{c}", "certainty": "exact" }}] }}"#
                    )
                })
                .collect();
            format!(
                r#"{{ "id": "f:{name}", "name": "{name}", "kind": "fn", "role": "invocable", "parent": "m:{parent}", "calls": [{}] }}"#,
                calls.join(",")
            )
        };
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
            [
                module("core"),
                module("db"),
                module("ui"),
                function("core", "main", &["put", "help"]),
                function("ui", "draw", &[]),
                function("db", "put", &[]),
                function("core", "help", &[]),
            ]
            .join(",")
        );
        System::new([Document::from_json(&json).unwrap()])
    }

    #[test]
    fn expanded_in_a_journey_the_expansion_follows_the_aside_and_back_to_the_cursor() {
        let mut explorer = opened(core_and_db(), "f:main");
        let tile =
            |explorer: &Explorer, id: &str| explorer.map().tile_of(&UnitId::new(id)).unwrap();
        let (main, draw) = (tile(&explorer, "f:main"), tile(&explorer, "f:draw"));
        let module = |explorer: &Explorer, t: usize| explorer.graph().cluster(t);
        explorer.toggle_expand(None);
        assert_eq!(
            explorer.nucleus().expanded_module(),
            module(&explorer, main)
        );
        // A search or click off the journey sets `draw` aside: the expansion follows.
        explorer.click(draw, None);
        assert_eq!(explorer.aside(), Some(draw));
        assert_eq!(
            explorer.nucleus().expanded_module(),
            module(&explorer, draw)
        );
        // Esc lets it go: the expansion goes back to the cursor's module, which shows.
        assert!(explorer.clear_aside());
        let back = module(&explorer, main);
        assert_eq!(explorer.nucleus().expanded_module(), back);
        let shown = explorer
            .nucleus()
            .ball(back.unwrap())
            .is_some_and(|ball| explorer.nucleus().shows_body(ball.body));
        assert!(shown, "the module expanded is drawn");
        // X with the aside set expands the aside's module; Esc then shrinks it back.
        explorer.toggle_expand(None);
        explorer.click(draw, None);
        explorer.toggle_expand(None);
        assert_eq!(
            explorer.nucleus().expanded_module(),
            module(&explorer, draw)
        );
        explorer.clear_aside();
        assert_eq!(explorer.nucleus().expanded_module(), None);
    }

    #[test]
    fn back_within_a_journey_puts_back_the_expansion_it_had() {
        let mut explorer = opened(core_and_db(), "f:main");
        let tile =
            |explorer: &Explorer, id: &str| explorer.map().tile_of(&UnitId::new(id)).unwrap();
        let (main, put) = (tile(&explorer, "f:main"), tile(&explorer, "f:put"));
        let module = |explorer: &Explorer, t: usize| explorer.graph().cluster(t);
        explorer.toggle_expand(None);
        explorer.click(put, None);
        assert_eq!(explorer.nucleus().expanded_module(), module(&explorer, put));
        explorer.click(main, None);
        explorer.toggle_expand(None);
        assert!(!explorer.nucleus().expanded());
        assert!(explorer.go_back());
        assert_eq!(
            explorer.nucleus().expanded_module(),
            module(&explorer, put),
            "Back to the cursor on put, seen expanded"
        );
        assert!(explorer.go_back());
        assert_eq!(
            explorer.nucleus().expanded_module(),
            module(&explorer, main),
            "Back to the cursor on main, seen with its own module expanded"
        );
    }

    #[test]
    fn expanded_in_a_journey_the_expansion_follows_the_cursor_across_modules() {
        use crate::camera::OrbitCamera;
        let system = core_and_db();
        let mut explorer = opened(system, "f:main");
        let [main, put] = ["main", "put"].map(|name| {
            explorer
                .map()
                .tile_of(&UnitId::new(format!("f:{name}")))
                .unwrap()
        });
        let module_of = |tile: usize| explorer.graph().cluster(tile);
        let (core, db) = (module_of(main), module_of(put));
        assert_ne!(core, db);

        let before = OrbitCamera::default().pose();
        explorer.see(before);
        explorer.toggle_expand(None);
        assert_eq!(explorer.nucleus().expanded_module(), core);
        explorer.step(Step::Into);
        assert_eq!(explorer.cursor(), Some(put));
        assert_eq!(
            explorer.nucleus().expanded_module(),
            db,
            "the cursor's module expands instead"
        );
        explorer.step(Step::Back);
        assert_eq!(explorer.nucleus().expanded_module(), core, "and back again");
        explorer.toggle_expand(None);
        assert!(!explorer.nucleus().expanded());
        assert_eq!(
            explorer.take_return_pose(),
            Some(Return::Shrunk(before)),
            "shrinking back returns to where the camera stood before the first"
        );
    }

    #[test]
    fn browsing_a_pick_in_another_module_takes_the_expansion_and_x_takes_the_aside_s() {
        let mut explorer = opened(core_and_db(), "f:main");
        let [main, put] = ["main", "put"].map(|name| {
            explorer
                .map()
                .tile_of(&UnitId::new(format!("f:{name}")))
                .unwrap()
        });
        let module_of = |tile: usize| explorer.graph().cluster(tile);
        let (core, db) = (module_of(main), module_of(put));
        explorer.leave_journey();
        explorer.toggle_expand(None);
        assert_eq!(explorer.nucleus().expanded_module(), core);
        explorer.click(put, None);
        assert_eq!(
            explorer.nucleus().expanded_module(),
            db,
            "it follows the pick"
        );
        assert!(explorer.go_back());
        assert_eq!(explorer.nucleus().expanded_module(), core, "and Back");
        explorer.deselect();
        assert_eq!(
            explorer.nucleus().expanded_module(),
            core,
            "nothing to follow"
        );
        explorer.toggle_expand(None);

        // In a journey, X with a function set aside expands the aside's module.
        explorer.open_at(main);
        explorer.click(put, None);
        assert_eq!(explorer.cursor(), Some(put), "put is on the journey");
        explorer.open_at(put);
        explorer.click(main, None);
        assert_eq!(explorer.aside(), Some(main), "main is off put's journey");
        assert_eq!(explorer.focus_module(), core, "X takes the aside's module");
    }

    #[test]
    fn the_nucleus_names_the_journey_to_the_depth_and_no_more() {
        let mut explorer = explorer();
        let [a, b, c] = ["a", "b", "c"].map(|name| tile(&explorer, name));
        explorer.gather();
        explorer.ease_nucleus(1.0);
        let names = |explorer: &Explorer| {
            let nucleus = explorer.nucleus();
            [a, b, c].map(|t| nucleus.level(t).is_some())
        };
        assert_eq!(names(&explorer), [true, true, false], "one level down");
        assert_eq!(explorer.disclosure().tiles()[&b], ["2"]);

        explorer.click(c, None);
        explorer.ease_nucleus(1.0);
        assert_eq!(explorer.current(), Some(c), "the cursor on c");
        assert_eq!(
            names(&explorer),
            [true, true, true],
            "the call clicked opens, as its bar does"
        );
        assert_eq!(explorer.nucleus().cursor(), Some(c), "and rings the cursor");
        explorer.toggle_expanded(explorer.stack(explorer.cursor_span().unwrap())[1]);
        explorer.ease_nucleus(1.0);
        assert_eq!(
            names(&explorer),
            [true, true, false],
            "folded again by hand"
        );

        explorer.set_disclose(Disclose(2));
        explorer.ease_nucleus(1.0);
        assert_eq!(names(&explorer), [true, true, true], "two levels down");
        assert_eq!(explorer.disclosure().tiles()[&c], ["1.1"]);
        assert_eq!(explorer.nucleus().level(c), Some(2));
    }

    #[test]
    fn browsing_a_click_selects_and_moves_nothing() {
        let mut explorer = explorer();
        let [main, a, b, c] = ["main", "a", "b", "c"].map(|name| tile(&explorer, name));
        explorer.gather();
        explorer.ease_nucleus(1.0);
        explorer.leave_journey();
        explorer.ease_nucleus(1.0);
        assert!(explorer.browsing());
        assert_eq!(explorer.selection(), Some(main), "the cursor's function");
        assert!(
            explorer.disclosure().tiles().is_empty(),
            "no journey numbers"
        );
        let balls = |explorer: &Explorer| -> Vec<Vec3> {
            let nucleus = explorer.nucleus();
            nucleus.balls().map(|(_, ball)| ball.center).collect()
        };
        let point = |explorer: &Explorer, tile: usize| explorer.nucleus().point(tile);
        let before = balls(&explorer);
        for tile in [a, c, b, main] {
            let at = point(&explorer, tile);
            explorer.click(tile, None);
            while explorer.ease_nucleus(0.5) {}
            explorer.ease_nucleus(1.0);
            assert_eq!(explorer.selection(), Some(tile));
            assert_eq!(
                explorer.nucleus().start(),
                Some(tile),
                "lit white where it sits"
            );
            assert_eq!(point(&explorer, tile), at, "the function clicked stays put");
            assert_eq!(balls(&explorer), before, "no module moved");
        }
        assert_eq!(
            explorer.track.start.as_ref().map(UnitId::as_str),
            Some("x:main")
        );
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
        let main = tile(&explorer, "main");
        explorer.step(Step::Into);
        explorer.leave_journey();
        assert_eq!(explorer.selection(), Some(main), "the journey's start");
        explorer.click(a, None);
        enter(&mut explorer);
        assert!(!explorer.browsing());
        assert_eq!(explorer.start_unit().map(UnitId::as_str), Some("x:a"));
        assert!(explorer.go_back(), "back to browsing");
        assert!(explorer.browsing());
        assert_eq!(explorer.selection(), Some(a));
        assert!(explorer.go_back(), "to the selection before");
        assert_eq!(explorer.selection(), Some(main));
        assert!(explorer.go_back(), "and into the journey before");
        assert!(!explorer.browsing());
        assert_eq!(at(&explorer), "a");
    }

    #[test]
    fn a_level_less_takes_the_cursor_up_to_the_call_still_drawn() {
        let mut explorer = explorer();
        explorer.set_disclose(Disclose(2));
        explorer.step(Step::Child);
        explorer.step(Step::Child);
        assert_eq!(at(&explorer), "c");
        explorer.set_disclose(Disclose(1));
        assert_eq!(
            at(&explorer),
            "a",
            "c folds away, so the cursor goes up to a"
        );
        assert_eq!(
            explorer
                .disclosure()
                .number(explorer.cursor_span().unwrap()),
            Some("1")
        );
        explorer.set_disclose(Disclose(2));
        assert_eq!(at(&explorer), "a", "and stays there a level more");

        // A call opened by hand, as a click on c opens a, stays open a level less,
        // and so the cursor on it.
        explorer.set_disclose(Disclose(1));
        explorer.click(tile(&explorer, "c"), None);
        explorer.set_disclose(Disclose(2));
        explorer.set_disclose(Disclose(1));
        assert_eq!(at(&explorer), "c");
    }

    #[test]
    fn the_depth_keys_stop_at_either_end_and_say_so() {
        let mut explorer = explorer();
        let said = |explorer: &Explorer| explorer.notice().map(|(_, text)| text.to_owned());
        explorer.shallower();
        assert_eq!(explorer.disclose(), Disclose(1));
        assert_eq!(
            said(&explorer).as_deref(),
            Some("Only the start's own calls show: no fewer levels")
        );
        explorer.deeper();
        assert_eq!(explorer.disclose(), Disclose(2), "c, under a");
        explorer.deeper();
        assert_eq!(explorer.disclose(), Disclose(2), "and no deeper than c");
        assert_eq!(
            said(&explorer).as_deref(),
            Some("No deeper calls: every level shows")
        );
        explorer.leave_journey();
        for key in [
            Explorer::deeper as fn(&mut Explorer),
            Explorer::shallower,
            Explorer::toggle_outside,
            |e: &mut Explorer| {
                e.step(Step::Into);
            },
        ] {
            explorer.notice = None;
            key(&mut explorer);
            assert_eq!(said(&explorer).as_deref(), Some(NO_JOURNEY));
        }
        assert_eq!(
            explorer.disclose(),
            Disclose(2),
            "browsing, none of them do more"
        );
    }

    #[test]
    fn esc_browsing_clears_the_selection_and_back_brings_it_back() {
        let mut explorer = explorer();
        explorer.leave_journey();
        let main = tile(&explorer, "main");
        assert_eq!(explorer.selection(), Some(main));
        explorer.deselect();
        assert_eq!(explorer.selection(), None);
        explorer.go_back();
        assert_eq!(explorer.selection(), Some(main));
    }

    #[test]
    fn a_name_two_functions_on_the_journey_share_says_whose() {
        // main calls Store::root and Track::root, and find.
        let json = r#"{ "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" }, "units": [
            { "id": "x:main", "name": "main", "kind": "fn", "role": "invocable", "calls": [
                { "key": "a#0", "text": "a", "targets": [ { "unit": "x:Store::root", "certainty": "exact" } ] },
                { "key": "b#0", "text": "b", "targets": [ { "unit": "x:Track::root", "certainty": "exact" } ] },
                { "key": "c#0", "text": "c", "targets": [ { "unit": "x:Track::find", "certainty": "exact" } ] } ] },
            { "id": "x:Store", "name": "Store", "kind": "struct", "role": "group" },
            { "id": "x:Track", "name": "Track", "kind": "struct", "role": "group" },
            { "id": "x:Store::root", "name": "root", "kind": "method", "role": "invocable", "parent": "x:Store" },
            { "id": "x:Track::root", "name": "root", "kind": "method", "role": "invocable", "parent": "x:Track" },
            { "id": "x:Track::find", "name": "find", "kind": "method", "role": "invocable", "parent": "x:Track" } ] }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let mut explorer = opened(system, "x:main");
        let names: Vec<String> = explorer
            .journey()
            .unwrap()
            .spans()
            .map(|(id, _)| explorer.label(id))
            .collect();
        assert_eq!(names, ["main", "Store.root", "Track.root", "find"]);
        explorer.leave_journey();
        assert_eq!(
            explorer.function_name(&UnitId::new("x:Track::root")),
            "Track.root",
            "browsing main, both its calls named root show: the same rule"
        );
        explorer.click(tile(&explorer, "Track::find"), None);
        assert_eq!(
            explorer.function_name(&UnitId::new("x:Track::root")),
            "root",
            "browsing find, only one root is in view"
        );
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
        let mut explorer = opened(system, "x:main");
        let [a, b, c] = ["a", "b", "c"].map(|name| tile(&explorer, name));
        let number = |explorer: &Explorer| {
            explorer
                .disclosure()
                .number(explorer.cursor_span().unwrap())
                .map(str::to_owned)
        };
        explorer.click(c, None);
        explorer.click(a, None);
        assert_eq!(number(&explorer).as_deref(), Some("4"), "the run after c");
        explorer.click(b, None);
        explorer.click(a, None);
        assert_eq!(number(&explorer).as_deref(), Some("1"), "the run before b");
        explorer.click(a, None);
        assert_eq!(number(&explorer).as_deref(), Some("1"), "on it already");
        explorer.click(a, Some("4"));
        assert_eq!(
            number(&explorer).as_deref(),
            Some("4"),
            "the one its name numbers"
        );
    }

    #[test]
    fn up_from_the_start_walks_the_row_of_its_callers() {
        // main and b both call a; the journey starts at a.
        let json = r#"{ "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" }, "units": [
            { "id": "x:main", "name": "main", "kind": "fn", "role": "invocable", "calls": [
                { "key": "a#0", "text": "a", "targets": [ { "unit": "x:a", "certainty": "exact" } ] },
                { "key": "b#0", "text": "b", "targets": [ { "unit": "x:b", "certainty": "exact" } ] } ] },
            { "id": "x:b", "name": "b", "kind": "fn", "role": "invocable", "calls": [
                { "key": "a#0", "text": "a", "targets": [ { "unit": "x:a", "certainty": "exact" } ] } ] },
            { "id": "x:a", "name": "a", "kind": "fn", "role": "invocable", "calls": [
                { "key": "c#0", "text": "c", "targets": [ { "unit": "x:c", "certainty": "exact" } ] } ] },
            { "id": "x:c", "name": "c", "kind": "fn", "role": "invocable" } ] }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let mut explorer = opened(system, "x:a");
        let (main, b) = (tile(&explorer, "main"), tile(&explorer, "b"));
        let row = explorer.start_callers();
        assert_eq!(row.len(), 2, "main and b, a row above the start");
        assert!(row.contains(&main) && row.contains(&b));

        explorer.step(Step::Parent);
        assert_eq!(
            explorer.aside(),
            Some(row[0]),
            "up into the row, its first set aside"
        );
        assert_eq!(explorer.aside_caller(), Some(0));
        assert_eq!(at(&explorer), "a", "the cursor stays on the start");
        explorer.step(Step::Previous);
        assert_eq!(explorer.notice().map(|(_, t)| t), Some("No earlier caller"));
        explorer.step(Step::Next);
        assert_eq!(explorer.aside(), Some(row[1]), "along the row");
        explorer.step(Step::Child);
        assert_eq!(explorer.aside(), None, "down, back to the start");
        assert_eq!(at(&explorer), "a");
        assert!(explorer.go_back(), "the walk into the row is remembered");
        assert_eq!(explorer.aside(), None, "as one move, from where it began");

        explorer.step(Step::Parent);
        explorer.step(Step::Next);
        enter(&mut explorer);
        assert_eq!(
            explorer.start_unit(),
            explorer.tile_unit(row[1]),
            "Enter starts a journey at the caller"
        );

        explorer.open_at(tile(&explorer, "a"));
        explorer.pick_caller(main);
        assert_eq!(explorer.aside(), Some(main), "its bar sets it aside too");
    }

    #[test]
    fn a_caller_sharing_the_start_s_name_says_whose() {
        // Explorer::reroot calls Track::reroot, where the journey starts.
        let json = r#"{ "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" }, "units": [
            { "id": "x:Explorer", "name": "Explorer", "kind": "struct", "role": "group" },
            { "id": "x:Track", "name": "Track", "kind": "struct", "role": "group" },
            { "id": "x:Explorer::reroot", "name": "reroot", "kind": "method", "role": "invocable", "parent": "x:Explorer", "calls": [
                { "key": "r#0", "text": "reroot", "targets": [ { "unit": "x:Track::reroot", "certainty": "exact" } ] } ] },
            { "id": "x:Track::reroot", "name": "reroot", "kind": "method", "role": "invocable", "parent": "x:Track" } ] }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let explorer = opened(system, "x:Track::reroot");
        let name = |id: &str| explorer.function_name(&UnitId::new(id));
        assert_eq!(name("x:Track::reroot"), "Track.reroot");
        assert_eq!(name("x:Explorer::reroot"), "Explorer.reroot");
    }

    #[test]
    fn i_with_no_outside_callers_says_so() {
        let mut explorer = explorer();
        explorer.toggle_outside();
        assert!(
            !explorer.outside_shown(),
            "nothing to bring in, so it stays off"
        );
        assert_eq!(
            explorer.notice().map(|(_, t)| t),
            Some("Nothing off the journey calls main")
        );
    }

    #[test]
    fn i_brings_in_outside_callers_and_one_clicked_is_selected_in_place() {
        // main calls a; x calls a too, from off the journey that starts at main, and
        // I with the cursor on a brings it in.
        let json = r#"{ "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" }, "units": [
            { "id": "x:main", "name": "main", "kind": "fn", "role": "invocable", "calls": [
                { "key": "a#0", "text": "a", "targets": [ { "unit": "x:a", "certainty": "exact" } ] } ] },
            { "id": "x:x", "name": "x", "kind": "fn", "role": "invocable", "calls": [
                { "key": "a#0", "text": "a", "targets": [ { "unit": "x:a", "certainty": "exact" } ] } ] },
            { "id": "x:a", "name": "a", "kind": "fn", "role": "invocable" } ] }"#;
        let system = System::new([Document::from_json(json).unwrap()]);
        let mut explorer = opened(system, "x:main");
        let x = tile(&explorer, "x");
        explorer.gather();
        explorer.ease_nucleus(1.0);
        assert!(!explorer.nucleus().lit(x), "off the journey, not shown");
        let a = tile(&explorer, "a");
        explorer.click(a, None);
        explorer.toggle_outside();
        explorer.ease_nucleus(1.0);
        assert!(explorer.outside_shown());
        assert!(explorer.nucleus().outside(x), "brought in");
        explorer.click(x, None);
        explorer.ease_nucleus(1.0);
        assert!(!explorer.browsing(), "still in the journey");
        assert_eq!(explorer.aside(), Some(x), "selected in place");
        assert_eq!(at(&explorer), "a", "the cursor stays");
        enter(&mut explorer);
        assert_eq!(
            explorer.start_unit().map(UnitId::as_str),
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
        let mut explorer = opened(system, "x:main");
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
