//! The layered view (`Z`, `--z layered`): the graph laid flat as a flow diagram. Each
//! module is a vertical lane, ordered left to right roughly as it sits on the map, and
//! each function sits in its module's lane at the row of its rank in the flow, how far
//! down the calls it is from the entry points. Control reads top to bottom, a module
//! boundary is a lane's edge, and the focus's callers sit above it, what it calls below.

use bevy::prelude::*;
use codeviz_layout::{LayerConfig, Layers, layers};

use crate::{
    camera::{FRONT, OrbitCamera},
    explorer::Explorer,
    graph_view::{self, orbit_anchor},
    scene::{PALETTE, lane_color},
};

/// How quickly nodes move between the map and the diagram, per second.
const EASING: f32 = 5.0;

/// Where each node sits in the diagram, and how far the graph has moved into it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Layered {
    on: bool,
    /// From 0, the map, to 1, every node in its lane.
    share: f32,
    layers: Option<Layers>,
}

impl Layered {
    /// Lays the nodes out as a diagram, from `clusters`, `calls` and `entries`, with
    /// lanes ordered by `map_x`; or goes back to the map.
    pub(crate) fn set(
        &mut self,
        on: bool,
        clusters: &[usize],
        calls: &[(usize, usize)],
        entries: &[usize],
        map_x: &[f32],
    ) {
        self.on = on;
        if on {
            self.layers = Some(layers(
                clusters,
                calls,
                entries,
                map_x,
                &LayerConfig::default(),
            ));
        }
    }

    pub(crate) fn on(&self) -> bool {
        self.on
    }

    /// The diagram, while it is on.
    pub(crate) fn diagram(&self) -> Option<&Layers> {
        self.layers.as_ref().filter(|_| self.on)
    }

    /// The diagram, once the graph is most of the way into it.
    pub(crate) fn shown(&self) -> Option<&Layers> {
        self.layers.as_ref().filter(|_| self.share > 0.5)
    }

    /// Where a node at `point` on the map is drawn.
    pub(crate) fn place(&self, tile: usize, point: [f32; 3]) -> [f32; 3] {
        let Some(&[x, y]) = self
            .layers
            .as_ref()
            .and_then(|layers| layers.positions.get(tile))
        else {
            return point;
        };
        let t = self.share * self.share * (3.0 - 2.0 * self.share);
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        [lerp(point[0], x), lerp(point[1], y), lerp(point[2], 0.0)]
    }

    /// Eases towards the diagram or the map. Returns whether anything moved.
    pub(crate) fn ease(&mut self, dt: f32) -> bool {
        let goal = if self.on { 1.0 } else { 0.0 };
        if (self.share - goal).abs() < f32::EPSILON {
            return false;
        }
        let step = EASING * dt;
        self.share = if self.share < goal {
            (self.share + step).min(goal)
        } else {
            (self.share - step).max(goal)
        };
        true
    }

    /// Goes all the way at once.
    pub(crate) fn settle(&mut self) {
        self.share = if self.on { 1.0 } else { 0.0 };
    }
}

/// Each lane as a faint outline in its module's color, brighter round the lanes
/// holding what is lit; and a hairline between ranks across the lit lanes.
pub(crate) fn draw(explorer: &Explorer, gizmos: &mut Gizmos) {
    let Some(layers) = explorer.layered().shown() else {
        return;
    };
    let lit = graph_view::lit_clusters(explorer);
    let (top, bottom) = (layers.top() + 3.0, layers.bottom() - 3.0);
    for lane in &layers.lanes {
        let color = lane_color(lane.cluster % PALETTE.len());
        let alpha = match &lit {
            Some(lit) if lit.contains(&lane.cluster) => 0.5,
            Some(_) => 0.06,
            None => 0.3,
        };
        let (left, right) = (lane.left - 1.2, lane.right + 1.2);
        let corners = [
            Vec3::new(left, top, 0.0),
            Vec3::new(right, top, 0.0),
            Vec3::new(right, bottom, 0.0),
            Vec3::new(left, bottom, 0.0),
            Vec3::new(left, top, 0.0),
        ];
        gizmos.linestrip(corners, color.with_alpha(alpha));
    }
}

/// When the diagram comes on or goes, turns the camera head-on to it and frames what
/// is lit; on the map again, frames the journey's start as usual.
pub(crate) fn pose(
    explorer: Res<Explorer>,
    mut last: Local<Option<bool>>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let on = explorer.layered().on();
    if *last == Some(on) {
        return;
    }
    let first = last.is_none();
    *last = Some(on);
    if first && !on {
        return;
    }
    for mut camera in &mut cameras {
        camera.free = false;
        if on {
            camera.turn_to(FRONT);
        }
        orbit_anchor(&explorer, &mut camera);
    }
}
