//! Viewer state that is independent of rendering: the graph, the current trace and
//! its layout, selection, and playback.

use bevy::prelude::*;
use codeviz_graph::{
    CallGraph, Function, FunctionId, Resolution, StopReason, Trace, TraceNode, TraceNodeId,
    TraceOptions, TraceTarget,
};
use codeviz_layout::{LayoutConfig, Scene};

/// How an invocation should be drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NodeKind {
    Root,
    Exact,
    Inferred,
    Ambiguous,
    External,
    /// Not expanded because of recursion or the depth limit.
    Truncated,
}

impl NodeKind {
    pub(crate) const ALL: [Self; 6] = [
        Self::Root,
        Self::Exact,
        Self::Inferred,
        Self::Ambiguous,
        Self::External,
        Self::Truncated,
    ];

    pub(crate) fn of(node: &TraceNode) -> Self {
        if node.stopped.is_some() {
            return Self::Truncated;
        }
        match &node.target {
            TraceTarget::Function { .. } if node.parent.is_none() => Self::Root,
            TraceTarget::Function {
                resolution: Resolution::Exact,
                ..
            } => Self::Exact,
            TraceTarget::Function {
                resolution: Resolution::Inferred,
                ..
            } => Self::Inferred,
            TraceTarget::Ambiguous(_) => Self::Ambiguous,
            TraceTarget::External(_) => Self::External,
        }
    }

    pub(crate) fn color(self) -> Color {
        match self {
            Self::Root => Color::srgb(1.0, 0.82, 0.3),
            Self::Exact => Color::srgb(0.35, 0.72, 1.0),
            Self::Inferred => Color::srgb(1.0, 0.55, 0.3),
            Self::Ambiguous => Color::srgb(0.75, 0.5, 1.0),
            Self::External => Color::srgb(0.5, 0.52, 0.56),
            Self::Truncated => Color::srgb(0.95, 0.35, 0.4),
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Root => "Root",
            Self::Exact => "Resolved call",
            Self::Inferred => "Inferred by name",
            Self::Ambiguous => "Ambiguous",
            Self::External => "External",
            Self::Truncated => "Recursion / depth limit",
        }
    }
}

#[derive(Resource, Debug)]
pub(crate) struct Explorer {
    graph: CallGraph,
    root: Option<FunctionId>,
    history: Vec<FunctionId>,
    pub(crate) trace_options: TraceOptions,
    pub(crate) layout: LayoutConfig,
    trace: Option<Trace>,
    scene: Scene,
    pub(crate) selected: Option<TraceNodeId>,
    pub(crate) hovered: Option<TraceNodeId>,
    /// Bumped whenever `trace` and `scene` are rebuilt.
    revision: u64,
}

impl Explorer {
    pub(crate) fn new(
        graph: CallGraph,
        root: Option<FunctionId>,
        trace_options: TraceOptions,
    ) -> Self {
        let mut explorer = Self {
            graph,
            root,
            history: Vec::new(),
            trace_options,
            layout: LayoutConfig::default(),
            trace: None,
            scene: Scene::default(),
            selected: None,
            hovered: None,
            revision: 0,
        };
        explorer.rebuild();
        explorer
    }

    pub(crate) fn graph(&self) -> &CallGraph {
        &self.graph
    }

    pub(crate) fn root(&self) -> Option<FunctionId> {
        self.root
    }

    pub(crate) fn trace(&self) -> Option<&Trace> {
        self.trace.as_ref()
    }

    pub(crate) fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Length of the current trace on its clock.
    pub(crate) fn duration(&self) -> f32 {
        self.trace.as_ref().map_or(0, Trace::duration) as f32
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn can_go_back(&self) -> bool {
        !self.history.is_empty()
    }

    pub(crate) fn set_root(&mut self, root: FunctionId) {
        if self.root == Some(root) {
            return;
        }
        if let Some(previous) = self.root.replace(root) {
            self.history.push(previous);
        }
        self.rebuild();
    }

    pub(crate) fn back(&mut self) {
        if let Some(previous) = self.history.pop() {
            self.root = Some(previous);
            self.rebuild();
        }
    }

    /// Re-roots the trace at the selected invocation's function.
    pub(crate) fn trace_from_selected(&mut self) {
        if let Some(id) = self.selected.and_then(|node| self.function_id(node)) {
            self.set_root(id);
        }
    }

    pub(crate) fn rebuild(&mut self) {
        self.trace = self
            .root
            .and_then(|root| Trace::from_root(&self.graph, root, self.trace_options));
        self.scene = self
            .trace
            .as_ref()
            .map(|trace| codeviz_layout::layout(&self.graph, trace, &self.layout))
            .unwrap_or_default();
        self.selected = self.trace.as_ref().map(Trace::root);
        self.hovered = None;
        self.revision += 1;
    }

    pub(crate) fn node(&self, id: TraceNodeId) -> Option<&TraceNode> {
        self.trace.as_ref()?.node(id)
    }

    /// The function an invocation runs, if it resolved to exactly one.
    pub(crate) fn function_id(&self, id: TraceNodeId) -> Option<FunctionId> {
        match self.node(id)?.target {
            TraceTarget::Function { id, .. } => Some(id),
            _ => None,
        }
    }

    pub(crate) fn function(&self, id: TraceNodeId) -> Option<&Function> {
        self.graph.function(self.function_id(id)?)
    }

    pub(crate) fn label(&self, id: TraceNodeId) -> String {
        let Some(node) = self.node(id) else {
            return String::new();
        };
        let name = match &node.target {
            TraceTarget::Function { id, .. } => self
                .graph
                .function(*id)
                .map(Function::short_name)
                .unwrap_or_default(),
            TraceTarget::Ambiguous(ids) => {
                let name = ids
                    .first()
                    .and_then(|id| self.graph.function(*id))
                    .map(|f| f.name.clone())
                    .unwrap_or_default();
                format!("{name} ({} candidates)", ids.len())
            }
            TraceTarget::External(text) => text.clone(),
        };
        match node.stopped {
            Some(StopReason::Recursion) => format!("{name} ↻"),
            Some(StopReason::DepthLimit) => format!("{name} …"),
            None => name,
        }
    }

    /// Moves the selection through the trace. Returns the new selection.
    pub(crate) fn step_selection(&mut self, step: SelectionStep) -> Option<TraceNodeId> {
        let trace = self.trace.as_ref()?;
        let current = self.selected.unwrap_or_else(|| trace.root());
        let node = trace.node(current)?;
        let next = match step {
            SelectionStep::Next => Some(current.index() + 1)
                .filter(|&index| index < trace.len())
                .and_then(|index| trace.nodes().nth(index).map(|(id, _)| id)),
            SelectionStep::Previous => current
                .index()
                .checked_sub(1)
                .and_then(|index| trace.nodes().nth(index).map(|(id, _)| id)),
            SelectionStep::Parent => node.parent,
            SelectionStep::FirstChild => node.children.first().copied(),
        };
        if next.is_some() {
            self.selected = next;
        }
        next
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionStep {
    /// The next invocation in time.
    Next,
    Previous,
    Parent,
    FirstChild,
}

/// The playhead sweeping along the time axis.
#[derive(Resource, Debug)]
pub(crate) struct Playback {
    /// Position on the trace clock. Invocations with `enter <= tick` have happened.
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

    pub(crate) fn has_entered(&self, node: &TraceNode) -> bool {
        node.enter as f32 <= self.tick
    }

    pub(crate) fn is_active(&self, node: &TraceNode) -> bool {
        self.has_entered(node) && self.tick < node.exit as f32
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use codeviz_graph::{CallSite, Callee, Location};

    use super::*;

    fn function(name: &str, calls: &[usize]) -> Function {
        let location = Location {
            file: PathBuf::from("src/lib.rs"),
            line: 1,
            column: 1,
        };
        Function {
            name: name.to_owned(),
            module: vec!["app".to_owned()],
            owner: None,
            is_async: false,
            signature: format!("fn {name}()"),
            location: location.clone(),
            end_line: 1,
            calls: calls
                .iter()
                .map(|&index| CallSite {
                    text: format!("f{index}"),
                    callee: Callee::Function {
                        id: CallGraph::id_for_index(index).unwrap(),
                        resolution: Resolution::Exact,
                    },
                    location: location.clone(),
                })
                .collect(),
        }
    }

    /// `main -> (a -> c), b`
    fn explorer() -> Explorer {
        let graph = CallGraph::new(vec![
            function("main", &[1, 2]),
            function("a", &[3]),
            function("b", &[]),
            function("c", &[]),
        ])
        .unwrap();
        Explorer::new(graph, CallGraph::id_for_index(0), TraceOptions::default())
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
        assert_eq!(explorer.trace().map(Trace::len), Some(2));
        assert!(explorer.can_go_back());

        explorer.back();
        assert_eq!(explorer.trace().map(Trace::len), Some(4));
        assert!(!explorer.can_go_back());
    }
}
