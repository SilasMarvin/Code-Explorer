//! Places a [`Trace`] in 3D space. Renderer-agnostic: positions are plain `[f32; 3]`.
//!
//! Axes:
//! - **X — time.** Each invocation sits at its `enter` tick.
//! - **Y — stack depth.** Callees hang below their callers.
//! - **Z — lane.** Invocations are grouped into lanes by a [`LaneKey`], so calls that
//!   cross a boundary (module, file, type) visibly jump between planes.

use std::collections::HashMap;

use codeviz_graph::{CallGraph, Function, Trace, TraceNodeId, TraceTarget};

/// What groups invocations into lanes along the Z axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LaneKey {
    #[default]
    Module,
    File,
    /// The impl type for methods, the module for free functions.
    Owner,
    /// Everything in one plane.
    Flat,
}

impl LaneKey {
    pub const ALL: [Self; 4] = [Self::Module, Self::File, Self::Owner, Self::Flat];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Module => "Module",
            Self::File => "File",
            Self::Owner => "Owner",
            Self::Flat => "Flat",
        }
    }

    fn of(self, function: &Function) -> String {
        match self {
            Self::Module => function.module_path(),
            Self::File => function.location.file.display().to_string(),
            Self::Owner => function.owner.as_ref().map_or_else(
                || function.module_path(),
                |owner| format!("{}::{}", function.module_path(), owner.type_name),
            ),
            Self::Flat => String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutConfig {
    /// World units per clock tick along X.
    pub tick_spacing: f32,
    /// World units per stack frame along -Y.
    pub depth_spacing: f32,
    /// World units between lanes along Z.
    pub lane_spacing: f32,
    pub lane_key: LaneKey,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            tick_spacing: 2.5,
            depth_spacing: 2.5,
            lane_spacing: 3.0,
            lane_key: LaneKey::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedNode {
    pub trace_node: TraceNodeId,
    pub position: [f32; 3],
    /// World length along X of the invocation including everything it calls.
    pub span: f32,
    pub lane: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub caller: TraceNodeId,
    pub callee: TraceNodeId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    pub label: String,
    pub z: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene {
    /// One entry per trace node, in trace order.
    pub nodes: Vec<PlacedNode>,
    pub edges: Vec<Edge>,
    /// Lanes in order of first appearance in the trace.
    pub lanes: Vec<Lane>,
    /// World-space bounds as `(min, max)`.
    pub bounds: ([f32; 3], [f32; 3]),
}

const EXTERNAL_LANE: &str = "<external>";
const AMBIGUOUS_LANE: &str = "<ambiguous>";

#[must_use]
pub fn layout(graph: &CallGraph, trace: &Trace, config: &LayoutConfig) -> Scene {
    let mut lane_index: HashMap<String, usize> = HashMap::new();
    let mut lanes = Vec::new();
    let mut nodes = Vec::with_capacity(trace.len());
    let mut edges = Vec::new();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for (id, node) in trace.nodes() {
        let key = match &node.target {
            TraceTarget::Function { id, .. } => graph
                .function(*id)
                .map_or_else(|| EXTERNAL_LANE.to_owned(), |f| config.lane_key.of(f)),
            TraceTarget::Ambiguous(_) => AMBIGUOUS_LANE.to_owned(),
            TraceTarget::External(_) => EXTERNAL_LANE.to_owned(),
        };
        let lane = *lane_index.entry(key).or_insert_with_key(|label| {
            lanes.push(Lane {
                label: label.clone(),
                z: -(lanes.len() as f32) * config.lane_spacing,
            });
            lanes.len() - 1
        });
        let z = lanes.get(lane).map_or(0.0, |lane| lane.z);
        let position = [
            node.enter as f32 * config.tick_spacing,
            -(node.depth as f32) * config.depth_spacing,
            z,
        ];
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }

        if let Some(parent) = node.parent {
            edges.push(Edge {
                caller: parent,
                callee: id,
            });
        }
        nodes.push(PlacedNode {
            trace_node: id,
            position,
            span: (node.exit - node.enter) as f32 * config.tick_spacing,
            lane,
        });
    }

    if nodes.is_empty() {
        min = [0.0; 3];
        max = [0.0; 3];
    }

    Scene {
        nodes,
        edges,
        lanes,
        bounds: (min, max),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use codeviz_graph::{CallSite, Callee, Location, Resolution, TraceOptions};

    use super::*;

    fn function(module: &str, name: &str, calls: &[usize]) -> Function {
        let location = Location {
            file: PathBuf::from(format!("src/{module}.rs")),
            line: 1,
            column: 1,
        };
        Function {
            name: name.to_owned(),
            module: vec!["app".to_owned(), module.to_owned()],
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

    #[test]
    fn axes_follow_time_depth_and_lane() {
        let graph = CallGraph::new(vec![
            function("handlers", "get", &[1, 2]),
            function("service", "load", &[]),
            function("handlers", "render", &[]),
        ])
        .unwrap();
        let root = CallGraph::id_for_index(0).unwrap();
        let trace = Trace::from_root(&graph, root, TraceOptions::default()).unwrap();
        let config = LayoutConfig::default();

        let scene = layout(&graph, &trace, &config);

        let positions: Vec<_> = scene.nodes.iter().map(|n| n.position).collect();
        assert_eq!(
            positions,
            vec![[0.0, 0.0, 0.0], [2.5, -2.5, -3.0], [5.0, -2.5, 0.0]]
        );
        assert_eq!(scene.lanes.len(), 2);
        assert_eq!(scene.edges.len(), 2);
        assert_eq!(scene.nodes[0].span, 7.5);
        assert_eq!(scene.bounds, ([0.0, -2.5, -3.0], [5.0, 0.0, 0.0]));
    }

    #[test]
    fn flat_lanes_collapse_z() {
        let graph =
            CallGraph::new(vec![function("a", "x", &[1]), function("b", "y", &[])]).unwrap();
        let root = CallGraph::id_for_index(0).unwrap();
        let trace = Trace::from_root(&graph, root, TraceOptions::default()).unwrap();
        let config = LayoutConfig {
            lane_key: LaneKey::Flat,
            ..LayoutConfig::default()
        };

        let scene = layout(&graph, &trace, &config);
        assert_eq!(scene.lanes.len(), 1);
        assert!(scene.nodes.iter().all(|n| n.position[2] == 0.0));
    }
}
