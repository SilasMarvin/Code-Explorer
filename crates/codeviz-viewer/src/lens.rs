//! How much of the graph shows. A reveal starts from something, the focused function,
//! a diff's changes or the journey's functions, and shows what is near it,
//! out to a distance the dial sets. Nearness is counted in calls, in space, or both:
//! space with each call between counting against it. What is in reach shows in full,
//! what is just past it dims, and the rest shrinks to specks that keep the shape of
//! the graph; as the dial turns, nodes fade between these instead of popping.

use std::collections::VecDeque;

use bevy::math::Vec3;
use codeviz_layout::ForceGraph;

/// What a reveal starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Source {
    /// The focused function.
    #[default]
    Focus,
    /// A diff's changes; nearness counts only towards callers, as Impact does.
    Changes,
    /// The functions the journey runs.
    Journey,
}

impl Source {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Focus => Self::Changes,
            Self::Changes => Self::Journey,
            Self::Journey => Self::Focus,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Changes => "changes",
            Self::Journey => "journey",
        }
    }
}

/// How nearness is measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Measure {
    /// Calls between, along call chains.
    #[default]
    Calls,
    /// Distance in the layout, in steps of a typical call's length.
    Space,
    /// Distance in the layout, plus a penalty for each call between; what calls or is
    /// called by the start directly always shows.
    Blend,
}

impl Measure {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Calls => Self::Space,
            Self::Space => Self::Blend,
            Self::Blend => Self::Calls,
        }
    }
}

/// The furthest the dial turns before showing everything.
pub(crate) const MAX_REACH: u32 = 8;
/// How bright the ring just past the reach is.
const DIM: f32 = 0.3;
/// Below this a node is a speck: drawn faintly, not labeled, not clickable.
pub(crate) const SPECK: f32 = 0.05;
/// In a blend, how many steps of space one call between counts for.
const CALL_PENALTY: f32 = 0.75;
/// Calls counted at most against something not connected to the start at all.
const UNCONNECTED: u32 = 4;
/// How fast visibility eases towards where the reveal puts it: the share of the way
/// left covered in a second is `1 - e^-EASE`.
const EASE: f32 = 7.0;

/// A reveal: what it starts from, how it measures, and how far it reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Lens {
    pub(crate) source: Source,
    pub(crate) measure: Measure,
    /// Steps out from the start that show in full; `None` shows everything.
    pub(crate) reach: Option<u32>,
    /// Under a diff, show only what changed, whatever the reach.
    pub(crate) changes_only: bool,
}

impl Lens {
    /// One step further out, until everything shows.
    pub(crate) fn wider(self) -> Self {
        let reach = match self.reach {
            Some(reach) if reach < MAX_REACH => Some(reach + 1),
            _ => None,
        };
        Self { reach, ..self }
    }

    /// One step further in; from everything, to the start and what is next to it.
    pub(crate) fn narrower(self) -> Self {
        let reach = match self.reach {
            None => Some(2),
            Some(reach) => Some(reach.saturating_sub(1)),
        };
        Self { reach, ..self }
    }

    /// How fully each node shows, by tile, given the tiles it starts from. `None` when
    /// it shows everything, as it does with nowhere to start.
    pub(crate) fn targets(&self, graph: &ForceGraph, seeds: &[usize]) -> Option<Vec<f32>> {
        let reach = self.reach?;
        if seeds.is_empty() {
            return None;
        }
        let upstream_only = self.source == Source::Changes;
        let calls = call_distances(graph, seeds, upstream_only);
        let reach_f = reach as f32;
        let by_calls = |hops: Option<u32>| match hops {
            Some(hops) if hops <= reach => 1.0,
            Some(hops) if hops == reach + 1 => DIM,
            _ => 0.0,
        };
        // Full out to the reach, fading to nothing over the next step.
        let by_space = |steps: f32| (1.0 - (steps - reach_f - 0.5)).clamp(0.0, 1.0);
        let targets = match self.measure {
            Measure::Calls => calls.iter().map(|&hops| by_calls(hops)).collect(),
            Measure::Space | Measure::Blend => {
                let space = space_distances(graph, seeds);
                calls
                    .iter()
                    .zip(&space)
                    .map(|(&hops, &steps)| {
                        if self.measure == Measure::Space {
                            return by_space(steps);
                        }
                        if hops.is_some_and(|h| h <= 1) {
                            return 1.0;
                        }
                        let between = hops.unwrap_or(UNCONNECTED).min(UNCONNECTED) as f32;
                        by_space(steps + between * CALL_PENALTY)
                    })
                    .collect()
            }
        };
        Some(targets)
    }
}

/// Calls from the nearest seed to each tile, counting along call chains: up through
/// callers and, unless `upstream_only`, down through callees, the nearer way. Seeds
/// are zero.
pub(crate) fn call_distances(
    graph: &ForceGraph,
    seeds: &[usize],
    upstream_only: bool,
) -> Vec<Option<u32>> {
    let count = graph.positions.len();
    let walk = |next: &dyn Fn(usize) -> Vec<usize>| {
        let mut hops: Vec<Option<u32>> = vec![None; count];
        let mut queue = VecDeque::new();
        for &seed in seeds {
            if let Some(slot) = hops.get_mut(seed) {
                *slot = Some(0);
                queue.push_back(seed);
            }
        }
        while let Some(at) = queue.pop_front() {
            let further = hops[at].map_or(0, |h| h + 1);
            for tile in next(at) {
                if let Some(slot @ None) = hops.get_mut(tile) {
                    *slot = Some(further);
                    queue.push_back(tile);
                }
            }
        }
        hops
    };
    let up = walk(&|tile| graph.callers(tile).collect());
    if upstream_only {
        return up;
    }
    let down = walk(&|tile| graph.callees(tile).collect());
    up.into_iter()
        .zip(down)
        .map(|(a, b)| match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        })
        .collect()
}

/// How far each tile is from the nearest seed, in steps of a typical call's length.
pub(crate) fn space_distances(graph: &ForceGraph, seeds: &[usize]) -> Vec<f32> {
    let at = |tile: usize| graph.positions.get(tile).map(|&p| Vec3::from(p));
    let step = typical_call(graph).max(f32::EPSILON);
    let seeds: Vec<Vec3> = seeds.iter().filter_map(|&s| at(s)).collect();
    graph
        .positions
        .iter()
        .map(|&p| {
            let p = Vec3::from(p);
            seeds
                .iter()
                .map(|s| s.distance(p))
                .fold(f32::INFINITY, f32::min)
                / step
        })
        .collect()
}

/// The median length of a call in the layout: one step of space.
fn typical_call(graph: &ForceGraph) -> f32 {
    let at = |tile: usize| graph.positions.get(tile).map(|&p| Vec3::from(p));
    let mut lengths: Vec<f32> = graph
        .calls
        .iter()
        .filter_map(|&(a, b)| Some(at(a)?.distance(at(b)?)))
        .collect();
    if lengths.is_empty() {
        return 1.0;
    }
    let middle = lengths.len() / 2;
    lengths.select_nth_unstable_by(middle, f32::total_cmp);
    lengths[middle]
}

/// Moves each node's visibility towards its target over `dt` seconds. Returns
/// whether any moved enough to redraw.
pub(crate) fn ease(visibility: &mut [f32], targets: Option<&[f32]>, dt: f32) -> bool {
    let share = 1.0 - (-EASE * dt).exp();
    let mut moved = false;
    for (tile, shown) in visibility.iter_mut().enumerate() {
        let target = targets.and_then(|t| t.get(tile)).copied().unwrap_or(1.0);
        let gap = target - *shown;
        if gap.abs() < 0.002 {
            if gap != 0.0 {
                *shown = target;
                moved = true;
            }
            continue;
        }
        *shown += gap * share;
        moved = true;
    }
    moved
}
