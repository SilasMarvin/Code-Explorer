//! Viewer state that is independent of rendering: the loaded system, the current
//! journey and its layout, selection, and playback.

use bevy::prelude::*;
use std::collections::HashSet;

use codeviz_core::{
    Journey, JourneyOptions, Level, Scenario, Span, SpanId, SpanTarget, StopReason, System,
    scenarios,
};
use codeviz_ir::{Certainty, Mode, Unit, UnitId};
use codeviz_layout::{LayoutConfig, Scene};

/// How a span should be drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NodeKind {
    Root,
    Exact,
    Declared,
    Inferred,
    Ambiguous,
    Unresolved,
    /// Handed off for later: routes, callbacks, subscriptions.
    Handoff,
    /// Not expanded: recursion, depth limit, or not loaded.
    Truncated,
}

impl NodeKind {
    pub(crate) const ALL: [Self; 8] = [
        Self::Root,
        Self::Exact,
        Self::Declared,
        Self::Inferred,
        Self::Ambiguous,
        Self::Unresolved,
        Self::Handoff,
        Self::Truncated,
    ];

    pub(crate) fn of(span: &Span) -> Self {
        match (&span.target, span.stopped, span.mode) {
            (_, _, Mode::Register) => Self::Handoff,
            (
                _,
                Some(StopReason::Recursion | StopReason::DepthLimit | StopReason::NotLoaded),
                _,
            ) => Self::Truncated,
            (SpanTarget::Ambiguous(_), ..) => Self::Ambiguous,
            (SpanTarget::Unresolved(_), ..) => Self::Unresolved,
            (SpanTarget::Unit(_), ..) if span.parent.is_none() => Self::Root,
            (SpanTarget::Unit(_), ..) => match span.certainty {
                Some(Certainty::Inferred) => Self::Inferred,
                Some(Certainty::Declared | Certainty::Observed) => Self::Declared,
                _ => Self::Exact,
            },
        }
    }

    pub(crate) fn color(self) -> Color {
        match self {
            Self::Root => Color::srgb(1.0, 0.82, 0.3),
            Self::Exact => Color::srgb(0.35, 0.72, 1.0),
            Self::Declared => Color::srgb(0.3, 0.85, 0.75),
            Self::Inferred => Color::srgb(1.0, 0.55, 0.3),
            Self::Ambiguous => Color::srgb(0.75, 0.5, 1.0),
            Self::Unresolved => Color::srgb(0.5, 0.52, 0.56),
            Self::Handoff => Color::srgb(0.55, 0.9, 0.4),
            Self::Truncated => Color::srgb(0.95, 0.35, 0.4),
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Root => "Entry",
            Self::Exact => "Resolved",
            Self::Declared => "Declared / observed",
            Self::Inferred => "Inferred by name",
            Self::Ambiguous => "Ambiguous",
            Self::Unresolved => "Unresolved",
            Self::Handoff => "Registered handoff",
            Self::Truncated => "Recursion / depth / not loaded",
        }
    }
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
    /// Containers whose spans are merged into one.
    collapsed: HashSet<UnitId>,
    journey: Option<Journey>,
    scene: Scene,
    pub(crate) selected: Option<SpanId>,
    pub(crate) hovered: Option<SpanId>,
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
            collapsed: HashSet::new(),
            journey: None,
            scene: Scene::default(),
            selected: None,
            hovered: None,
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

    pub(crate) fn scenario(&self) -> Option<usize> {
        self.scenario
    }

    /// Length of the current journey on its clock.
    pub(crate) fn duration(&self) -> f32 {
        self.journey.as_ref().map_or(0, Journey::duration) as f32
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

    /// Recomputes scenarios, which depend on the root and the zoom levels.
    pub(crate) fn reroot(&mut self) {
        self.scenarios = self
            .root
            .as_ref()
            .map(|root| scenarios(&self.system, root, &self.options.zoom))
            .unwrap_or_default();
        self.scenario = None;
        self.rebuild();
    }

    /// Restricts the journey to one scenario, or shows every path with `None`.
    pub(crate) fn set_scenario(&mut self, scenario: Option<usize>) {
        if self.scenario != scenario {
            self.scenario = scenario;
            self.rebuild();
        }
    }

    /// Re-roots the journey at the selected span's unit.
    pub(crate) fn trace_from_selected(&mut self) {
        if let Some(id) = self.selected.and_then(|span| self.unit_id(span)).cloned() {
            self.set_root(id);
        }
    }

    /// Switches a refinable unit between its summary and the units it refines.
    pub(crate) fn toggle_zoom(&mut self, unit: UnitId) {
        let level = match self.options.zoom.level(&unit) {
            Level::Summary => Level::Detail,
            Level::Detail => Level::Summary,
        };
        self.options.zoom.set(unit, level);
        self.reroot();
    }

    pub(crate) fn set_default_zoom(&mut self, level: Level) {
        self.options.zoom.default = level;
        self.options.zoom.overrides.clear();
        self.reroot();
    }

    pub(crate) fn collapsed(&self) -> &HashSet<UnitId> {
        &self.collapsed
    }

    /// Merges or un-merges every span inside `container`.
    pub(crate) fn toggle_collapse(&mut self, container: UnitId) {
        if !self.collapsed.remove(&container) {
            self.collapsed.insert(container);
        }
        self.rebuild();
    }

    pub(crate) fn expand_all(&mut self) {
        self.collapsed.clear();
        self.rebuild();
    }

    pub(crate) fn rebuild(&mut self) {
        let scenario = self.scenario.and_then(|i| self.scenarios.get(i));
        self.journey = self
            .root
            .as_ref()
            .and_then(|root| Journey::unroll(&self.system, root, scenario, self.options.clone()));
        if !self.collapsed.is_empty() {
            self.journey = self
                .journey
                .take()
                .map(|journey| journey.collapse(&self.system, &self.collapsed));
        }
        self.scene = self
            .journey
            .as_ref()
            .map(|journey| codeviz_layout::layout(&self.system, journey, &self.layout))
            .unwrap_or_default();
        self.selected = self.journey.as_ref().map(Journey::root);
        self.hovered = None;
        self.revision += 1;
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
                "{} ({} candidates)",
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

    /// `Parent.name`, which reads well in every scheme.
    pub(crate) fn unit_name(&self, id: &UnitId) -> String {
        let Some(unit) = self.system.unit(id) else {
            return id.to_string();
        };
        match unit.parent.as_ref().and_then(|p| self.system.unit(p)) {
            Some(parent) if parent.parent.is_some() => format!("{}.{}", parent.name, unit.name),
            _ => unit.name.clone(),
        }
    }

    /// Moves the selection through the journey. Returns the new selection.
    pub(crate) fn step_selection(&mut self, step: SelectionStep) -> Option<SpanId> {
        let journey = self.journey.as_ref()?;
        let current = self.selected.unwrap_or_else(|| journey.root());
        let span = journey.span(current)?;
        let nth = |index: usize| journey.spans().nth(index).map(|(id, _)| id);
        let next = match step {
            SelectionStep::Next => nth(current.index() + 1),
            SelectionStep::Previous => current.index().checked_sub(1).and_then(nth),
            SelectionStep::Parent => span.parent,
            SelectionStep::FirstChild => span.children.first().copied(),
        };
        if next.is_some() {
            self.selected = next;
        }
        next
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionStep {
    /// The next span in pre-order.
    Next,
    Previous,
    Parent,
    FirstChild,
}

/// The playhead sweeping along the time axis.
#[derive(Resource, Debug)]
pub(crate) struct Playback {
    /// Position on the journey clock. Spans with `enter <= tick` have happened.
    pub(crate) tick: f32,
    pub(crate) playing: bool,
    pub(crate) ticks_per_second: f32,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            tick: f32::MAX,
            playing: false,
            ticks_per_second: 2.0,
        }
    }
}

impl Playback {
    pub(crate) fn toggle(&mut self, duration: f32) {
        if !self.playing && self.tick >= duration {
            self.tick = 0.0;
        }
        self.playing = !self.playing;
    }

    pub(crate) fn has_entered(&self, span: &Span) -> bool {
        span.enter as f32 <= self.tick
    }

    pub(crate) fn is_active(&self, span: &Span) -> bool {
        self.has_entered(span) && self.tick < span.exit as f32
    }
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

    fn selected_label(explorer: &Explorer) -> String {
        explorer.label(explorer.selected.unwrap())
    }

    #[test]
    fn selection_walks_time_and_stack() {
        let mut explorer = explorer();
        assert_eq!(selected_label(&explorer), "main");

        explorer.step_selection(SelectionStep::Next);
        explorer.step_selection(SelectionStep::Next);
        assert_eq!(selected_label(&explorer), "c");

        explorer.step_selection(SelectionStep::Parent);
        assert_eq!(selected_label(&explorer), "a");

        explorer.step_selection(SelectionStep::FirstChild);
        assert_eq!(selected_label(&explorer), "c");

        explorer.step_selection(SelectionStep::Next);
        assert_eq!(selected_label(&explorer), "b");
        assert_eq!(explorer.step_selection(SelectionStep::Next), None);
        assert_eq!(selected_label(&explorer), "b");
    }

    #[test]
    fn rerooting_keeps_history() {
        let mut explorer = explorer();
        explorer.step_selection(SelectionStep::FirstChild);
        explorer.trace_from_selected();
        assert_eq!(explorer.journey().map(Journey::len), Some(2));
        assert!(explorer.can_go_back());

        explorer.back();
        assert_eq!(explorer.journey().map(Journey::len), Some(4));
        assert!(!explorer.can_go_back());
    }
}
