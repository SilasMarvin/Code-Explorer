//! The codebase as a graph of calls: how its nodes look, which links light up, and
//! where the camera goes. Whatever the pointer is over, or else the focused node, is
//! the center of attention: its callers turn amber, its callees cyan, and the rest of
//! the graph fades so that neighborhood reads on its own.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use codeviz_layout::ForceGraph;

use crate::{
    camera::{CameraMode, JourneyCamera},
    explorer::Explorer,
    scene::{PALETTE, Tile, accent, lane_color},
};

pub(crate) const CALLER: Color = Color::srgb(1.0, 0.72, 0.25);
pub(crate) const CALLEE: Color = Color::srgb(0.35, 0.85, 1.0);

/// How a node is drawn right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NodeLook {
    /// The center of attention.
    Focus,
    /// Calls the focus.
    Caller,
    /// Called by the focus.
    Callee,
    /// Reaches the focus through other calls.
    Upstream,
    /// Run by the active journey, or anything while nothing is focused.
    Bright(usize),
    Dim(usize),
    /// Away from the focus.
    Faded(usize),
}

pub(crate) fn node_materials(
    materials: &mut Assets<StandardMaterial>,
) -> HashMap<NodeLook, Handle<StandardMaterial>> {
    let glow = |materials: &mut Assets<StandardMaterial>, color: Color, strength: f32| {
        materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * strength,
            ..default()
        })
    };
    let mut table = HashMap::new();
    table.insert(NodeLook::Focus, glow(materials, Color::WHITE, 4.0));
    table.insert(NodeLook::Caller, glow(materials, CALLER, 2.5));
    table.insert(NodeLook::Callee, glow(materials, CALLEE, 2.5));
    table.insert(NodeLook::Upstream, glow(materials, CALLER, 0.6));
    for color in 0..PALETTE.len() {
        let base = lane_color(color);
        table.insert(NodeLook::Bright(color), glow(materials, base, 1.6));
        table.insert(NodeLook::Dim(color), glow(materials, base, 0.35));
        table.insert(
            NodeLook::Faded(color),
            materials.add(StandardMaterial {
                base_color: base.with_alpha(0.12),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            }),
        );
    }
    table
}

/// Bigger for functions with more calls in or out.
pub(crate) fn node_size(graph: &ForceGraph, tile: usize) -> f32 {
    1.0 + (1.0 + graph.degree(tile) as f32).ln() * 0.55
}

/// The node at the center of attention: under the pointer, else focused.
pub(crate) fn attention(explorer: &Explorer) -> Option<usize> {
    explorer.hovered_tile.or_else(|| explorer.graph_focus())
}

/// What calls the attended node and what it calls.
#[derive(Debug, Default)]
pub(crate) struct Neighbors {
    pub(crate) callers: HashSet<usize>,
    pub(crate) callees: HashSet<usize>,
}

impl Neighbors {
    pub(crate) fn of(graph: &ForceGraph, tile: Option<usize>) -> Self {
        let Some(tile) = tile else {
            return Self::default();
        };
        Self {
            callers: graph.callers(tile).collect(),
            callees: graph.callees(tile).collect(),
        }
    }

    pub(crate) fn contains(&self, tile: usize) -> bool {
        self.callers.contains(&tile) || self.callees.contains(&tile)
    }
}

pub(crate) fn node_look(
    explorer: &Explorer,
    tile: usize,
    center: Option<usize>,
    neighbors: &Neighbors,
) -> NodeLook {
    let color = explorer.tile_color(tile) % PALETTE.len();
    if center == Some(tile) {
        return NodeLook::Focus;
    }
    if neighbors.callers.contains(&tile) {
        return NodeLook::Caller;
    }
    if neighbors.callees.contains(&tile) {
        return NodeLook::Callee;
    }
    if center.is_some() && explorer.tile_distance(tile).is_some() {
        return NodeLook::Upstream;
    }
    let run = explorer.journey_lane(tile).is_some();
    match (center.is_some(), run) {
        (false, _) | (true, true) => {
            if run {
                NodeLook::Bright(color)
            } else {
                NodeLook::Dim(color)
            }
        }
        (true, false) => NodeLook::Faded(color),
    }
}

/// Restyles nodes when attention or the journeys change.
pub(crate) fn style_nodes(
    explorer: Res<Explorer>,
    materials: Res<NodeMaterials>,
    mut nodes: Query<(&Tile, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let Some(graph) = explorer.graph() else {
        return;
    };
    if !explorer.is_changed() {
        return;
    }
    let center = attention(&explorer);
    let neighbors = Neighbors::of(graph, center);
    for (tile, mut material) in &mut nodes {
        let look = node_look(&explorer, tile.0, center, &neighbors);
        if let Some(wanted) = materials.0.get(&look)
            && &material.0 != wanted
        {
            material.0 = wanted.clone();
        }
    }
}

#[derive(Resource, Debug)]
pub(crate) struct NodeMaterials(pub(crate) HashMap<NodeLook, Handle<StandardMaterial>>);

pub(crate) fn create_node_materials(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(NodeMaterials(node_materials(&mut materials)));
}

/// Every call as a link: lit by direction around the center of attention, in the
/// journey's accent along the active journey's path, faint elsewhere.
pub(crate) fn draw_graph(explorer: &Explorer, graph: &ForceGraph, gizmos: &mut Gizmos) {
    let center = attention(explorer);
    let point = |tile: usize| explorer.tile_point(tile);
    let map = explorer.map();
    let track = explorer.track();
    let path: HashSet<(usize, usize)> = track
        .scene()
        .edges
        .iter()
        .filter_map(|edge| {
            let tile = |span| track.unit_id(span).and_then(|u| map.tile_of(u));
            Some((tile(edge.caller)?, tile(edge.callee)?))
        })
        .collect();
    let path_color = accent(explorer.active());

    for &(from, to) in &graph.calls {
        let (Some(a), Some(b)) = (point(from), point(to)) else {
            continue;
        };
        let (color, arrow) = if center == Some(to) {
            (CALLER.with_alpha(0.95), true)
        } else if center == Some(from) {
            (CALLEE.with_alpha(0.95), true)
        } else if path.contains(&(from, to)) {
            (
                path_color.with_alpha(if center.is_some() { 0.35 } else { 0.85 }),
                true,
            )
        } else if center.is_some() {
            (Color::srgba(0.6, 0.66, 0.8, 0.04), false)
        } else {
            (Color::srgba(0.6, 0.66, 0.8, 0.16), false)
        };
        if arrow {
            // Stop at the callee's surface so the head stays visible.
            let back = (b - a).normalize_or_zero() * node_size(graph, to) * 0.5;
            gizmos.arrow(a, b - back, color).with_tip_length(0.6);
        } else {
            gizmos.line(a, b, color);
        }
    }
    if let Some(at) = center.and_then(point) {
        let radius = center.map_or(1.0, |c| node_size(graph, c)) * 0.5 + 0.35;
        gizmos.sphere(Isometry3d::from_translation(at), radius, Color::WHITE);
    }
    // The cursor's bar, tethered back to the node it runs.
    let tether = track.selected().and_then(|span| {
        let bar = track.bar(span)?;
        let node = point(map.tile_of(track.unit_id(span)?)?)?;
        Some((bar.label_anchor(), node))
    });
    if let Some((bar, node)) = tether {
        gizmos.line(bar, node, Color::srgba(1.0, 1.0, 1.0, 0.45));
    }
}

/// Whether the camera faces the journeys head-on or orbits the focused node.
#[derive(Resource, Debug, Default)]
pub(crate) struct GraphCamera {
    pub(crate) front: bool,
}

/// The closest the orbit comes to the focused node.
const MIN_ORBIT_DISTANCE: f32 = 25.0;

/// F snaps to the journeys, head-on and framed; again, back to orbiting the focus.
pub(crate) fn graph_keys(
    keys: Res<ButtonInput<KeyCode>>,
    explorer: Res<Explorer>,
    mut view: ResMut<GraphCamera>,
    mut cameras: Query<&mut JourneyCamera>,
) {
    if explorer.graph().is_none() || !keys.just_pressed(KeyCode::KeyF) {
        return;
    }
    view.front = !view.front;
    for mut camera in &mut cameras {
        if view.front {
            face_journeys(&explorer, &mut camera);
        } else {
            camera.set_angles(-0.35, -0.2);
            orbit_focus(&explorer, &mut camera);
        }
    }
}

pub(crate) fn face_journeys(explorer: &Explorer, camera: &mut JourneyCamera) {
    camera.set_mode(CameraMode::Flat);
    let (min, max) = explorer.journeys_bounds();
    camera.glide_frame(min, max);
}

/// Orbits the focused node from far enough back to see everything it calls and
/// everything that calls it.
pub(crate) fn orbit_focus(explorer: &Explorer, camera: &mut JourneyCamera) {
    camera.set_mode(CameraMode::Orbit);
    let (Some(graph), Some(focus)) = (explorer.graph(), explorer.graph_focus()) else {
        return;
    };
    let Some(at) = explorer.tile_point(focus) else {
        return;
    };
    let reach = graph
        .callers(focus)
        .chain(graph.callees(focus))
        .filter_map(|t| explorer.tile_point(t))
        .map(|p| p.distance(at))
        .fold(0.0, f32::max);
    // A 45° view shows about 0.4 of its distance either side of center; leave room.
    camera.glide_to(at, (reach * 3.4).max(MIN_ORBIT_DISTANCE));
}

/// Orbiting, keeps the camera centered on the focused node as it moves or changes.
/// Clicking a node while facing the journeys swings back to orbit it.
pub(crate) fn follow_focus(
    explorer: Res<Explorer>,
    mut view: ResMut<GraphCamera>,
    mut clicked: Local<Option<usize>>,
    mut cameras: Query<&mut JourneyCamera>,
) {
    if explorer.graph().is_none() {
        *clicked = None;
        view.front = false;
        return;
    }
    let click = explorer.clicked_node();
    let newly_clicked = click.is_some() && click != *clicked;
    *clicked = click;
    for mut camera in &mut cameras {
        if newly_clicked && view.front {
            view.front = false;
            camera.set_angles(-0.35, -0.2);
            orbit_focus(&explorer, &mut camera);
        }
        if !view.front
            && let Some(at) = explorer.graph_focus().and_then(|t| explorer.tile_point(t))
        {
            camera.target = at;
        }
    }
}
