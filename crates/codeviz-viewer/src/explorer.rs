//! Viewer state that is independent of rendering: the loaded system, the current
//! journey and its layout, the cursor, the outcome being shown, and the codebase map
//! with what reaches the cursor's unit.

use std::collections::HashMap;

use bevy::prelude::*;
use codeviz_core::{
    Journey, JourneyOptions, Level, Scenario, Span, SpanId, SpanTarget, StopReason, System,
    Upstream, display_name, scenarios, upstream,
};
use codeviz_ir::{Certainty, ExitKind, Location, Mode, Unit, UnitId};
use codeviz_layout::{CodebaseMap, LayoutConfig, MapConfig, Scene};

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

#[derive(Resource, Debug)]
pub(crate) struct Explorer {
    system: System,
    root: Option<UnitId>,
    history: Vec<UnitId>,
    scenarios: Vec<Scenario>,
    scenario: Option<usize>,
    pub(crate) options: JourneyOptions,
    pub(crate) layout: LayoutConfig,
    journey: Option<Journey>,
    scene: Scene,
    divergences: Vec<Divergence>,
    /// The span the cursor is on.
    pub(crate) selected: Option<SpanId>,
    pub(crate) hovered: Option<SpanId>,
    /// Whether the rest of the codebase is drawn around the journey.
    codebase: bool,
    map: CodebaseMap,
    /// Where the map's origin sits in the world: above the journey.
    map_offset: Vec2,
    /// Map tiles the journey runs, with the lane each is drawn in.
    journey_tiles: HashMap<usize, usize>,
    /// What can reach the cursor's unit.
    upstream: Option<(UnitId, Upstream)>,
    /// Fewest calls from each map tile to the cursor's unit.
    upstream_tiles: HashMap<usize, u32>,
    pub(crate) hovered_tile: Option<usize>,
    /// Bumped whenever `journey` and `scene` are rebuilt.
    revision: u64,
}

impl Explorer {
    pub(crate) fn new(system: System, root: Option<UnitId>, options: JourneyOptions) -> Self {
        let root = root.or_else(|| system.entries().first().map(|e| e.unit.clone()));
        let mut explorer = Self {
            system,
            root: None,
            history: Vec::new(),
            scenarios: Vec::new(),
            scenario: None,
            options,
            layout: LayoutConfig::default(),
            journey: None,
            scene: Scene::default(),
            divergences: Vec::new(),
            selected: None,
            hovered: None,
            codebase: false,
            map: CodebaseMap::default(),
            map_offset: Vec2::ZERO,
            journey_tiles: HashMap::new(),
            upstream: None,
            upstream_tiles: HashMap::new(),
            hovered_tile: None,
            revision: 0,
        };
        if let Some(root) = root {
            explorer.set_root(root);
            explorer.history.clear();
        }
        explorer
    }

    pub(crate) fn system(&self) -> &System {
        &self.system
    }

    pub(crate) fn root(&self) -> Option<&UnitId> {
        self.root.as_ref()
    }

    pub(crate) fn journey(&self) -> Option<&Journey> {
        self.journey.as_ref()
    }

    pub(crate) fn scene(&self) -> &Scene {
        &self.scene
    }

    pub(crate) fn scenarios(&self) -> &[Scenario] {
        &self.scenarios
    }

    /// The outcome shown; `None` shows every path at once.
    pub(crate) fn scenario(&self) -> Option<usize> {
        self.scenario
    }

    pub(crate) fn divergences(&self) -> &[Divergence] {
        &self.divergences
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn can_go_back(&self) -> bool {
        !self.history.is_empty()
    }

    pub(crate) fn set_root(&mut self, root: UnitId) {
        if self.root.as_ref() == Some(&root) {
            return;
        }
        if let Some(previous) = self.root.replace(root) {
            self.history.push(previous);
        }
        self.reroot();
    }

    pub(crate) fn back(&mut self) {
        if let Some(previous) = self.history.pop() {
            self.root = Some(previous);
            self.reroot();
        }
    }

    /// Recomputes outcomes for the root and opens on the happy path.
    pub(crate) fn reroot(&mut self) {
        self.scenarios = self
            .root
            .as_ref()
            .map(|root| scenarios(&self.system, root, &self.options.zoom))
            .unwrap_or_default();
        self.scenario = (!self.scenarios.is_empty()).then_some(0);
        self.rebuild();
    }

    pub(crate) fn set_scenario(&mut self, scenario: Option<usize>) {
        if self.scenario != scenario {
            self.scenario = scenario;
            self.rebuild();
            // Land on the exit the outcome ends at.
            if scenario.is_some_and(|s| s > 0) {
                self.selected = self.journey.as_ref().and_then(|j| {
                    j.spans()
                        .filter(|(_, s)| s.exits.iter().any(|e| e.kind != ExitKind::Return))
                        .map(|(id, _)| id)
                        .last()
                        .or_else(|| j.spans().last().map(|(id, _)| id))
                });
            }
        }
    }

    /// Re-roots the journey at the selected span's unit.
    pub(crate) fn trace_from_selected(&mut self) {
        if let Some(id) = self.selected.and_then(|span| self.unit_id(span)).cloned() {
            self.set_root(id);
        }
    }

    pub(crate) fn toggle_zoom(&mut self, unit: UnitId) {
        let level = match self.options.zoom.level(&unit) {
            Level::Summary => Level::Detail,
            Level::Detail => Level::Summary,
        };
        self.options.zoom.set(unit, level);
        self.reroot();
    }

    pub(crate) fn rebuild(&mut self) {
        let scenario = self.scenario.and_then(|i| self.scenarios.get(i));
        self.journey = self
            .root
            .as_ref()
            .and_then(|root| Journey::unroll(&self.system, root, scenario, self.options.clone()));

        self.scene = self
            .journey
            .as_ref()
            .map(|journey| codeviz_layout::layout(&self.system, journey, &self.layout))
            .unwrap_or_default();
        self.divergences = self.find_divergences();
        self.selected = self.journey.as_ref().map(Journey::root);
        self.hovered = None;
        if self.codebase {
            self.place_map();
        }
        self.revision += 1;
    }

    pub(crate) fn codebase(&self) -> bool {
        self.codebase
    }

    pub(crate) fn toggle_codebase(&mut self) {
        self.codebase = !self.codebase;
        if self.codebase {
            self.place_map();
        }
        self.hovered_tile = None;
        self.revision += 1;
    }

    /// Lays the map out as wide as the journey and rests it just above it.
    fn place_map(&mut self) {
        let (min, max) = self.scene.bounds;
        let config = MapConfig {
            max_width: (max[0] - min[0]).max(30.0),
            ..MapConfig::default()
        };
        self.map = codeviz_layout::map(&self.system, &config);
        self.upstream = None;
        self.map_offset = Vec2::new(min[0], max[1] + 2.5);
        self.journey_tiles = self
            .scene
            .nodes
            .iter()
            .filter_map(|placed| {
                let unit = self.unit_id(placed.span)?;
                Some((self.map.tile_of(unit)?, placed.lane))
            })
            .collect();
    }

    pub(crate) fn map(&self) -> &CodebaseMap {
        &self.map
    }

    /// A map position in world space.
    pub(crate) fn map_point(&self, position: [f32; 2]) -> Vec3 {
        (Vec2::from(position) + self.map_offset).extend(0.0)
    }

    /// The lane a tile's unit is drawn in on the journey, if the journey runs it.
    pub(crate) fn journey_lane(&self, tile: usize) -> Option<usize> {
        self.journey_tiles.get(&tile).copied()
    }

    /// The unit under the cursor.
    pub(crate) fn focus(&self) -> Option<&UnitId> {
        self.selected.and_then(|id| self.unit_id(id))
    }

    /// What can reach the cursor's unit, once [`Self::refresh_upstream`] has run.
    pub(crate) fn upstream(&self) -> Option<&Upstream> {
        self.upstream.as_ref().map(|(_, up)| up)
    }

    /// Recomputes [`Self::upstream`] when the cursor's unit changed. Returns whether it did.
    pub(crate) fn refresh_upstream(&mut self) -> bool {
        let focus = self.focus().cloned();
        if self.upstream.as_ref().map(|(id, _)| id) == focus.as_ref() {
            return false;
        }
        self.upstream = focus.map(|id| {
            let up = upstream(&self.system, &id);
            (id, up)
        });
        self.upstream_tiles.clear();
        let distances = self.upstream.as_ref().map(|(_, up)| &up.distance);
        for (unit, &distance) in distances.into_iter().flatten() {
            if let Some(tile) = self.map.tile_of(unit) {
                let best = self.upstream_tiles.entry(tile).or_insert(distance);
                *best = (*best).min(distance);
            }
        }
        true
    }

    /// Fewest calls from a tile to the cursor's unit, if it can reach it at all.
    pub(crate) fn tile_distance(&self, tile: usize) -> Option<u32> {
        self.upstream_tiles.get(&tile).copied()
    }

    /// Starts the journey at `unit`, with the cursor on the unit it was on before when
    /// the new journey reaches it: from a caller, straight to the call that led here.
    pub(crate) fn start_from(&mut self, unit: UnitId) {
        let focus = self.focus().cloned();
        self.set_root(unit);
        let Some(focus) = focus else {
            return;
        };
        let landing = self.journey.as_ref().and_then(|j| {
            j.spans()
                .find(|(_, s)| s.target == SpanTarget::Unit(focus.clone()))
                .map(|(id, _)| id)
        });
        if landing.is_some() {
            self.selected = landing;
        }
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
        display_name(&self.system, id)
    }

    /// The call stack at a span, outermost first.
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

    /// Moves the cursor. Returns the new position.
    pub(crate) fn step(&mut self, step: Step) -> Option<SpanId> {
        let journey = self.journey.as_ref()?;
        let current = self.selected.unwrap_or_else(|| journey.root());
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
        };
        if next.is_some() {
            self.selected = next;
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
mod tests {
    use codeviz_ir::Document;

    use super::*;

    /// `main -> (a -> c), b`
    fn explorer() -> Explorer {
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
        explorer.label(explorer.selected.unwrap())
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
    fn starting_from_a_caller_keeps_the_cursor_on_the_callee() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");

        explorer.toggle_codebase();
        explorer.refresh_upstream();
        let callers: Vec<&str> = explorer
            .upstream()
            .unwrap()
            .direct()
            .map(|e| e.caller.as_str())
            .collect();
        assert_eq!(callers, ["x:a"]);

        explorer.start_from(UnitId::new("x:a"));
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:a"));
        assert_eq!(at(&explorer), "c");
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
}
