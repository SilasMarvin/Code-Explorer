//! Browsing: every module where the map has it, every function inside its module,
//! and what the selection touches lit where it sits; with nothing selected, an arrow
//! from each module to every module it calls.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use codeviz_layout::{ForceGraph, atlas};

use super::{
    Arrangement, Atlas, Ball, Body, Flow, Link, Nucleus, Tube, VIEW,
    journey::{distances, open_up},
};
use crate::sizes::module_radius;

impl Nucleus {
    /// Browsing: every module where the atlas has it, every function inside its
    /// module, and what `start` touches lit where it sits: its callers and its own
    /// calls, named, and their modules opened. Nothing moves for a new start: the
    /// atlas leaves each function room enough to be lit and named where it is.
    pub(super) fn arrange_browse(&self, graph: &ForceGraph, start: Option<usize>) -> Arrangement {
        let atlas = self.atlas.clone();
        let clusters = self.homes.len();
        let middle = start.and_then(|f| graph.cluster(f)).unwrap_or(0);
        let cluster_of = |tile: usize| graph.cluster(tile).unwrap_or(middle);
        let links = start.map_or_else(Vec::new, |f| Self::browse_calls(graph, f));
        let named = distances(&links, start);
        // Every module counts what it names, the start's own among them.
        let columns = open_up(&links, start, clusters, usize::MAX, &cluster_of);
        let mut balls = atlas.balls;
        for (cluster, ball) in balls.iter_mut().enumerate() {
            if (start.is_some() && cluster == middle) || !columns[cluster].is_empty() {
                ball.body = Body::Open;
            }
        }
        let shown = (0..graph.positions.len())
            .map(|t| graph.cluster(t).is_some())
            .collect();
        let tubes = if start.is_some() {
            Vec::new()
        } else {
            self.map_tubes()
        };
        Arrangement {
            start,
            middle,
            balls,
            points: atlas.points,
            shown,
            named,
            ring: Vec::new(),
            columns,
            outside: HashSet::new(),
            aside: None,
            waiting: 0,
            folded: HashSet::new(),
            tubes,
            links,
            dots: HashMap::new(),
        }
    }

    /// Browsing with nothing selected, how modules depend on each other: a line from
    /// each module to every module it calls.
    fn map_tubes(&self) -> Vec<Tube> {
        let mut tubes: Vec<Tube> = self
            .flows
            .iter()
            .map(|(&(from, to), &calls)| Tube { from, to, calls })
            .collect();
        tubes.sort_unstable_by_key(|t| (t.from, t.to));
        tubes
    }

    /// Browsing, the calls named round `start`: each of its callers' into it, then each
    /// of its own calls, in the order written.
    fn browse_calls(graph: &ForceGraph, start: usize) -> Vec<Link> {
        let mut links = Self::calls_into(graph, start);
        for &to in graph.sequence(start) {
            if !links.iter().any(|l| l.from == start && l.to == to) {
                links.push(Link {
                    from: start,
                    to,
                    flow: Flow::Journey(1),
                });
            }
        }
        links
    }
}

/// Browsing, where every module and function sits: each module where the stable
/// layout of modules has it, spread so none hides another as the usual view, or one
/// turned an eighth of the way either side, sees them; each function where `offsets`
/// has it in its module, by `module_of`.
pub(super) fn atlas(
    homes: &[Vec3],
    counts: &[usize],
    offsets: &[Vec3],
    module_of: &[Option<usize>],
) -> Atlas {
    let radii: Vec<f32> = counts.iter().map(|&n| module_radius(n)).collect();
    let balls: Vec<Ball> = atlas::atlas(homes, &radii, VIEW)
        .into_iter()
        .zip(radii)
        .map(|(center, radius)| Ball {
            center,
            radius,
            body: Body::Faint,
        })
        .collect();
    let points = offsets
        .iter()
        .zip(module_of)
        .map(|(&at, module)| module.map_or(at, |c| balls[c].center + at))
        .collect();
    Atlas { balls, points }
}
