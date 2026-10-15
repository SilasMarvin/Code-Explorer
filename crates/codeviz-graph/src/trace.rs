use crate::{CallGraph, Callee, FunctionId, Location, Resolution};

/// Index of a [`TraceNode`] within its [`Trace`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceNodeId(usize);

impl TraceNodeId {
    #[must_use]
    pub fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceOptions {
    /// Invocations deeper than this are recorded but not expanded.
    pub max_depth: u32,
    /// Record calls into code outside the analyzed source as leaf nodes.
    pub include_external: bool,
}

impl Default for TraceOptions {
    fn default() -> Self {
        Self {
            max_depth: 32,
            include_external: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceTarget {
    Function {
        id: FunctionId,
        resolution: Resolution,
    },
    Ambiguous(Vec<FunctionId>),
    External(String),
}

/// Why an invocation's own calls were not expanded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The function is already on the call stack.
    Recursion,
    DepthLimit,
}

/// One invocation in a trace.
///
/// `enter` and `exit` are ticks on a logical clock that advances once per invocation,
/// so a node's span covers exactly the invocations it makes, transitively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceNode {
    pub target: TraceTarget,
    pub parent: Option<TraceNodeId>,
    pub children: Vec<TraceNodeId>,
    pub depth: u32,
    pub enter: u32,
    pub exit: u32,
    /// Where the parent made this call; `None` for the root.
    pub call_site: Option<Location>,
    pub stopped: Option<StopReason>,
}

/// A static call graph unrolled from one entry point into a tree of invocations.
///
/// Calls are expanded depth-first in source order, which approximates the order they
/// would execute in if every branch were taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    nodes: Vec<TraceNode>,
}

impl Trace {
    /// Unrolls `graph` starting at `root`.
    ///
    /// Returns `None` if `root` is not in `graph`.
    #[must_use]
    pub fn from_root(graph: &CallGraph, root: FunctionId, options: TraceOptions) -> Option<Self> {
        graph.function(root)?;
        let mut builder = Builder {
            graph,
            options,
            nodes: Vec::new(),
            stack: Vec::new(),
            clock: 0,
        };
        builder.visit(
            TraceTarget::Function {
                id: root,
                resolution: Resolution::Exact,
            },
            None,
            None,
        );
        Some(Self {
            nodes: builder.nodes,
        })
    }

    #[must_use]
    pub fn root(&self) -> TraceNodeId {
        TraceNodeId(0)
    }

    #[must_use]
    pub fn node(&self, id: TraceNodeId) -> Option<&TraceNode> {
        self.nodes.get(id.0)
    }

    /// Nodes in pre-order, which is also `enter` order.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = (TraceNodeId, &TraceNode)> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (TraceNodeId(index), node))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The tick after the last invocation ends.
    #[must_use]
    pub fn duration(&self) -> u32 {
        self.nodes.first().map_or(0, |root| root.exit)
    }

    #[must_use]
    pub fn max_depth(&self) -> u32 {
        self.nodes.iter().map(|node| node.depth).max().unwrap_or(0)
    }
}

struct Builder<'g> {
    graph: &'g CallGraph,
    options: TraceOptions,
    nodes: Vec<TraceNode>,
    stack: Vec<FunctionId>,
    clock: u32,
}

impl Builder<'_> {
    fn visit(
        &mut self,
        target: TraceTarget,
        parent: Option<TraceNodeId>,
        call_site: Option<Location>,
    ) -> TraceNodeId {
        let id = TraceNodeId(self.nodes.len());
        let depth = u32::try_from(self.stack.len()).unwrap_or(u32::MAX);
        let enter = self.clock;
        self.clock += 1;

        let expand = match &target {
            TraceTarget::Function { id, .. } if self.stack.contains(id) => {
                Err(StopReason::Recursion)
            }
            TraceTarget::Function { .. } if depth >= self.options.max_depth => {
                Err(StopReason::DepthLimit)
            }
            TraceTarget::Function { id, .. } => Ok(Some(*id)),
            TraceTarget::Ambiguous(_) | TraceTarget::External(_) => Ok(None),
        };

        self.nodes.push(TraceNode {
            target,
            parent,
            children: Vec::new(),
            depth,
            enter,
            exit: enter,
            call_site,
            stopped: expand.err(),
        });

        if let Ok(Some(function_id)) = expand {
            self.stack.push(function_id);
            let calls = self
                .graph
                .function(function_id)
                .map(|function| function.calls.as_slice())
                .unwrap_or_default();
            for site in calls {
                let child_target = match &site.callee {
                    Callee::Function { id, resolution } => TraceTarget::Function {
                        id: *id,
                        resolution: *resolution,
                    },
                    Callee::Ambiguous(ids) => TraceTarget::Ambiguous(ids.clone()),
                    Callee::External if self.options.include_external => {
                        TraceTarget::External(site.text.clone())
                    }
                    Callee::External => continue,
                };
                let child = self.visit(child_target, Some(id), Some(site.location.clone()));
                if let Some(node) = self.nodes.get_mut(id.0) {
                    node.children.push(child);
                }
            }
            self.stack.pop();
        }

        if let Some(node) = self.nodes.get_mut(id.0) {
            node.exit = self.clock;
        }
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::function;

    fn id(index: usize) -> FunctionId {
        CallGraph::id_for_index(index).unwrap()
    }

    #[test]
    fn clock_spans_cover_descendants() {
        // main -> (a -> c), b
        let graph = CallGraph::new(vec![
            function("main", &[(1, Resolution::Exact), (2, Resolution::Exact)]),
            function("a", &[(3, Resolution::Exact)]),
            function("b", &[]),
            function("c", &[]),
        ])
        .unwrap();

        let trace = Trace::from_root(&graph, id(0), TraceOptions::default()).unwrap();
        let spans: Vec<_> = trace
            .nodes()
            .map(|(_, node)| (node.depth, node.enter, node.exit))
            .collect();
        assert_eq!(spans, vec![(0, 0, 4), (1, 1, 3), (2, 2, 3), (1, 3, 4)]);
        assert_eq!(trace.duration(), 4);
        assert_eq!(trace.max_depth(), 2);
    }

    #[test]
    fn recursion_is_cut() {
        let graph = CallGraph::new(vec![
            function("ping", &[(1, Resolution::Exact)]),
            function("pong", &[(0, Resolution::Exact)]),
        ])
        .unwrap();

        let trace = Trace::from_root(&graph, id(0), TraceOptions::default()).unwrap();
        assert_eq!(trace.len(), 3);
        let last = trace.nodes().last().unwrap().1;
        assert_eq!(last.stopped, Some(StopReason::Recursion));
    }

    #[test]
    fn depth_limit_stops_expansion() {
        let graph = CallGraph::new(vec![
            function("a", &[(1, Resolution::Exact)]),
            function("b", &[(2, Resolution::Exact)]),
            function("c", &[]),
        ])
        .unwrap();
        let options = TraceOptions {
            max_depth: 1,
            ..TraceOptions::default()
        };

        let trace = Trace::from_root(&graph, id(0), options).unwrap();
        assert_eq!(trace.len(), 2);
        assert_eq!(
            trace.nodes().last().unwrap().1.stopped,
            Some(StopReason::DepthLimit)
        );
    }
}
