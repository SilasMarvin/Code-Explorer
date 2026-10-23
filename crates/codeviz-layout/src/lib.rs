//! Places a [`Journey`] in 3D space. Renderer-agnostic: positions are plain `[f32; 3]`.
//!
//! Axes:
//! - **X — time.** Each span sits at its `enter` tick.
//! - **Y — depth.** Callees hang below their callers. Spans that overlap in time at the
//!   same depth and lane (branch alternatives, parallel work) get separate sub-rows.
//! - **Z — lane.** Spans are grouped into lanes by a [`LaneKey`], so calls that cross a
//!   boundary (module, service, pipeline group) visibly jump between planes.
//!
//! [`map()`] lays out the whole codebase instead, for context around a journey, and
//! [`ForceGraph`] floats it in 3D as a graph of calls.

use std::collections::HashMap;

use codeviz_core::{Journey, SpanId, SpanTarget, System};
use codeviz_ir::{Unit, UnitId};

mod graph;
mod map;
mod repulsion;

pub use graph::{ForceConfig, ForceGraph, GroupStats, Reach, Toward};
pub use map::{CodebaseMap, MapConfig, MapGroup, MapTile, map};

/// What groups spans into lanes along the Z axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LaneKey {
    /// The unit's container: module, type, service, pipeline group.
    #[default]
    Parent,
    /// The outermost container: crate, package, cluster, pipeline.
    Root,
    /// The document namespace: `rust`, `python`, `arch`.
    Scheme,
    /// Everything in one plane.
    Flat,
}

impl LaneKey {
    pub const ALL: [Self; 4] = [Self::Parent, Self::Root, Self::Scheme, Self::Flat];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Parent => "Container",
            Self::Root => "Top-level",
            Self::Scheme => "Namespace",
            Self::Flat => "Flat",
        }
    }

    fn of(self, system: &System, unit: &Unit) -> String {
        match self {
            Self::Parent => unit
                .parent
                .as_ref()
                .map_or_else(|| unit.id.to_string(), ToString::to_string),
            Self::Root => system
                .ancestors(&unit.id)
                .last()
                .map_or_else(|| unit.id.to_string(), |root| root.id.to_string()),
            Self::Scheme => unit.id.scheme().unwrap_or_default().to_owned(),
            Self::Flat => String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutConfig {
    /// World units per clock tick along X.
    pub tick_spacing: f32,
    /// World units per depth level along -Y.
    pub depth_spacing: f32,
    /// World units between sub-rows within one depth level.
    pub track_spacing: f32,
    /// World units between lanes along Z.
    pub lane_spacing: f32,
    pub lane_key: LaneKey,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            tick_spacing: 2.5,
            depth_spacing: 2.5,
            track_spacing: 0.9,
            lane_spacing: 3.0,
            lane_key: LaneKey::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedNode {
    pub span: SpanId,
    pub position: [f32; 3],
    /// World length along X of the span, including everything it calls.
    pub length: f32,
    pub lane: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub caller: SpanId,
    pub callee: SpanId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    pub label: String,
    pub z: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene {
    /// One entry per journey span, in journey order.
    pub nodes: Vec<PlacedNode>,
    pub edges: Vec<Edge>,
    /// Lanes in order of first appearance.
    pub lanes: Vec<Lane>,
    /// World-space bounds as `(min, max)`.
    pub bounds: ([f32; 3], [f32; 3]),
}

const UNRESOLVED_LANE: &str = "<unresolved>";

/// Occupied `(enter, exit)` intervals per sub-row, keyed by depth.
type Occupancy = HashMap<u32, Vec<Vec<(u32, u32)>>>;

#[must_use]
pub fn layout(system: &System, journey: &Journey, config: &LayoutConfig) -> Scene {
    let mut lane_index: HashMap<String, usize> = HashMap::new();
    let mut lanes = Vec::new();
    let mut rows: Occupancy = HashMap::new();
    let mut nodes = Vec::with_capacity(journey.len());
    let mut edges = Vec::new();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for (id, span) in journey.spans() {
        let key = match &span.target {
            SpanTarget::Unit(unit) => system
                .unit(unit)
                .map_or_else(|| scheme_lane(unit), |u| config.lane_key.of(system, u)),
            SpanTarget::Ambiguous(ids) => ids.first().map_or_else(String::new, scheme_lane),
            SpanTarget::Unresolved(_) => UNRESOLVED_LANE.to_owned(),
        };
        let lane = *lane_index.entry(key).or_insert_with_key(|label| {
            lanes.push(Lane {
                label: label.clone(),
                z: -(lanes.len() as f32) * config.lane_spacing,
            });
            lanes.len() - 1
        });

        // Rows are shared across lanes so the flat front view never overlaps bars.
        let tracks = rows.entry(span.depth).or_default();
        let track = tracks
            .iter()
            .position(|taken| {
                taken
                    .iter()
                    .all(|&(a, b)| span.exit <= a || span.enter >= b)
            })
            .unwrap_or(tracks.len());
        if track == tracks.len() {
            tracks.push(Vec::new());
        }
        if let Some(taken) = tracks.get_mut(track) {
            taken.push((span.enter, span.exit));
        }

        let z = lanes.get(lane).map_or(0.0, |lane| lane.z);
        let position = [
            span.enter as f32 * config.tick_spacing,
            -(span.depth as f32) * config.depth_spacing - track as f32 * config.track_spacing,
            z,
        ];
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }
        if let Some(parent) = span.parent {
            edges.push(Edge {
                caller: parent,
                callee: id,
            });
        }
        nodes.push(PlacedNode {
            span: id,
            position,
            length: (span.exit - span.enter) as f32 * config.tick_spacing,
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

fn scheme_lane(id: &UnitId) -> String {
    format!("<{}>", id.scheme().unwrap_or("external"))
}

#[cfg(test)]
mod tests {
    use codeviz_core::JourneyOptions;
    use codeviz_ir::Document;

    use super::*;

    /// `a` (module `m1`) calls `b` (module `m2`) then branches into `c` or `d` (module `m1`).
    fn system() -> System {
        let json = r#"{
          "schema": "codeviz/0.1", "producer": { "name": "test", "version": "0" },
          "units": [
            { "id": "x:m1", "name": "m1", "kind": "module", "role": "group" },
            { "id": "x:m2", "name": "m2", "kind": "module", "role": "group" },
            { "id": "x:a", "name": "a", "kind": "fn", "role": "invocable", "parent": "x:m1",
              "calls": [
                { "key": "b#0", "text": "b", "targets": [{ "unit": "x:b", "certainty": "exact" }] },
                { "key": "c#0", "text": "c", "targets": [{ "unit": "x:c", "certainty": "exact" }] },
                { "key": "d#0", "text": "d", "targets": [{ "unit": "x:d", "certainty": "exact" }] }
              ],
              "body": [
                { "step": "call", "call": "b#0" },
                { "step": "branch", "guard": "flag", "arms": [
                  { "label": "then", "body": [{ "step": "call", "call": "c#0" }] },
                  { "label": "else", "body": [{ "step": "call", "call": "d#0" }] }
                ] }
              ] },
            { "id": "x:b", "name": "b", "kind": "fn", "role": "invocable", "parent": "x:m2" },
            { "id": "x:c", "name": "c", "kind": "fn", "role": "invocable", "parent": "x:m1" },
            { "id": "x:d", "name": "d", "kind": "fn", "role": "invocable", "parent": "x:m1" }
          ]
        }"#;
        System::new([Document::from_json(json).unwrap()])
    }

    #[test]
    fn axes_follow_time_depth_and_lane() {
        let system = system();
        let journey = Journey::unroll(
            &system,
            &UnitId::new("x:a"),
            None,
            JourneyOptions::default(),
        )
        .unwrap();
        let scene = layout(&system, &journey, &LayoutConfig::default());

        let positions: Vec<_> = scene.nodes.iter().map(|n| n.position).collect();
        assert_eq!(
            positions,
            vec![
                [0.0, 0.0, 0.0],
                [2.5, -2.5, -3.0],
                // Alternatives overlap in time and stack in sub-rows.
                [5.0, -2.5, 0.0],
                [5.0, -3.4, 0.0],
            ]
        );
        assert_eq!(scene.lanes.len(), 2);
        assert_eq!(scene.edges.len(), 3);
    }

    #[test]
    fn flat_lanes_collapse_z() {
        let system = system();
        let journey = Journey::unroll(
            &system,
            &UnitId::new("x:a"),
            None,
            JourneyOptions::default(),
        )
        .unwrap();
        let config = LayoutConfig {
            lane_key: LaneKey::Flat,
            ..LayoutConfig::default()
        };
        let scene = layout(&system, &journey, &config);
        assert_eq!(scene.lanes.len(), 1);
        assert!(scene.nodes.iter().all(|n| n.position[2] == 0.0));
    }
}
