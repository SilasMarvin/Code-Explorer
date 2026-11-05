//! A module expanded: every one of its functions drawn where it sits, with the calls
//! among them. It opens where it is, browsing or in a journey; nothing moves, and the
//! camera comes in to it.

use std::collections::HashSet;

use bevy::prelude::*;
use codeviz_layout::CallGraph;

use super::{Arrangement, Body, Flow, Link, Nucleus};

impl Nucleus {
    /// Opens `cluster`, the module expanded, in `now`: its sphere drawn open, though
    /// nothing lit lies in it, and every call among its functions that nothing else
    /// names, drawn faintly.
    pub(super) fn open_out(graph: &CallGraph, cluster: usize, now: &mut Arrangement) {
        if let Some(ball) = now.balls.get_mut(cluster)
            && ball.body == Body::Faint
        {
            ball.body = Body::Open;
        }
        let inside: HashSet<usize> = (0..graph.len())
            .filter(|&t| graph.cluster(t) == Some(cluster))
            .collect();
        let mut named: HashSet<(usize, usize)> = now.links.iter().map(|l| (l.from, l.to)).collect();
        for &(from, to) in &graph.calls {
            if from != to
                && inside.contains(&from)
                && inside.contains(&to)
                && named.insert((from, to))
            {
                now.links.push(Link {
                    from,
                    to,
                    flow: Flow::Inner,
                });
            }
        }
    }

    /// The module expanded: its sphere's radius, and each of its functions with where
    /// it sits from the sphere's middle, in the world. What it holds alone sets how
    /// big it expands, so it expands alike wherever it is.
    pub(crate) fn expanded_layout(&self) -> Option<(f32, Vec<(usize, Vec3)>)> {
        let cluster = self.expanded?;
        let (center, radius) = *self.layout.modules.get(cluster)?;
        let members = (0..self.module_of.len())
            .filter(|&t| self.module_of(t) == Some(cluster))
            .map(|t| (t, self.layout.functions[t] - center))
            .collect();
        Some((radius, members))
    }

    /// The module a function lives in.
    pub(crate) fn module_of(&self, tile: usize) -> Option<usize> {
        self.module_of.get(tile).copied().flatten()
    }
}
