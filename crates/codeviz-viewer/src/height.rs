//! Height as flow: with `Z` on, every node rises by its rank in the call graph, so the
//! entry points sit on top, what they call a level down, and the leaves (storage, IO,
//! helpers) at the bottom. Where a node sits on the map stays as it was; only its height
//! changes, and it never depends on the focus, so the shape can be learned. A focus's
//! callers are then above it and what it calls below; a call that runs uphill stands
//! out.

use codeviz_layout::{RankBy, ranks};

/// How quickly nodes rise and fall, per second.
const EASING: f32 = 6.0;
/// The most and least height between one rank and the next.
const TALLEST_STEP: f32 = 24.0;
const SHORTEST_STEP: f32 = 6.0;
/// How tall the whole stack stands, as a share of how wide the map is.
const STACK_SHARE: f32 = 1.0;

/// Each node's height, and how far the graph has risen into it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Lift {
    /// How ranks are counted while lifted; none when the graph lies flat.
    by: Option<RankBy>,
    /// How they were counted last, for `Z` to turn back on.
    last_by: RankBy,
    /// Each node's rank, as `by` (or `last_by`) counts it.
    ranks: Vec<u32>,
    /// From 0, the flat map, to 1, every node at its rank's height.
    share: f32,
    /// Each node's height as it eases towards its rank's.
    heights: Vec<f32>,
    /// Height between one rank and the next.
    step: f32,
}

impl Lift {
    /// Counts ranks `by` for `nodes` nodes joined by `calls` down from `entries`, or
    /// lays the graph flat with none; `width` is how wide the map is, which sets how
    /// tall the ranks stand.
    pub(crate) fn set(
        &mut self,
        by: Option<RankBy>,
        nodes: usize,
        calls: &[(usize, usize)],
        entries: &[usize],
        width: f32,
    ) {
        self.by = by;
        let Some(by) = by else {
            return;
        };
        self.last_by = by;
        self.ranks = ranks(nodes, calls, entries, by);
        let deepest = self.deepest().max(1);
        #[expect(clippy::cast_precision_loss, reason = "a rank count")]
        let step = width * STACK_SHARE / deepest as f32;
        self.step = step.clamp(SHORTEST_STEP, TALLEST_STEP);
        if self.heights.len() != nodes {
            self.heights = vec![0.0; nodes];
        }
    }

    /// How ranks are counted while lifted.
    pub(crate) fn by(&self) -> Option<RankBy> {
        self.by
    }

    /// How ranks were last counted, lifted or not.
    pub(crate) fn last_by(&self) -> RankBy {
        self.last_by
    }

    /// Whether the graph shows any height at all.
    pub(crate) fn lifted(&self) -> bool {
        self.share > 0.0
    }

    /// How far the graph is rising to: all the way, or back down flat.
    fn goal(&self) -> f32 {
        if self.by.is_some() { 1.0 } else { 0.0 }
    }

    /// Whether the graph has finished rising or lying down.
    pub(crate) fn settled(&self) -> bool {
        (self.share - self.goal()).abs() < f32::EPSILON
    }

    /// How far the graph has risen, from 0 flat to 1.
    pub(crate) fn share(&self) -> f32 {
        self.share
    }

    /// A node's rank, while lifted.
    pub(crate) fn rank(&self, tile: usize) -> Option<u32> {
        self.by?;
        self.ranks.get(tile).copied()
    }

    /// The deepest rank.
    pub(crate) fn deepest(&self) -> u32 {
        self.ranks.iter().copied().max().unwrap_or(0)
    }

    /// The height of a rank: the entries at zero, each rank a step below.
    pub(crate) fn rank_height(&self, rank: u32) -> f32 {
        #[expect(clippy::cast_precision_loss, reason = "a small rank")]
        let rank = rank as f32;
        -rank * self.step
    }

    /// Where `flat`, a node's place on the map, is lifted to.
    pub(crate) fn place(&self, tile: usize, flat: [f32; 3]) -> [f32; 3] {
        if self.share <= 0.0 {
            return flat;
        }
        let height = self.heights.get(tile).copied().unwrap_or(0.0);
        [flat[0], flat[1], flat[2] + (height - flat[2]) * self.share]
    }

    /// Eases every node towards its height and the graph towards lifted or flat.
    /// Returns whether anything moved.
    pub(crate) fn ease(&mut self, dt: f32) -> bool {
        let ease = 1.0 - (-EASING * dt).exp();
        let goal = self.goal();
        let before = self.share;
        self.share += (goal - self.share) * ease;
        if (goal - self.share).abs() < 1e-3 {
            self.share = goal;
        }
        let mut moved = (self.share - before).abs() > 0.0;
        if self.by.is_some() {
            for (tile, height) in self.heights.iter_mut().enumerate() {
                let rank = self.ranks.get(tile).copied().unwrap_or(0);
                #[expect(clippy::cast_precision_loss, reason = "a small rank")]
                let wanted = -(rank as f32) * self.step;
                if (*height - wanted).abs() > 1e-3 {
                    *height += (wanted - *height) * ease;
                    moved = true;
                }
            }
        }
        moved
    }

    /// Puts every node at its height at once.
    pub(crate) fn settle(&mut self) {
        while self.ease(1.0) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rises_by_rank_and_lies_flat_again() {
        let mut lift = Lift::default();
        let calls = [(0, 1), (1, 2)];
        lift.set(Some(RankBy::Longest), 3, &calls, &[], 100.0);
        lift.settle();
        let [a, b, c] = [0, 1, 2].map(|tile| lift.place(tile, [1.0, 2.0, 0.5])[2]);
        assert!(a > b && b > c, "entries above what they call: {a} {b} {c}");
        assert_eq!(
            lift.place(1, [1.0, 2.0, 0.5])[..2],
            [1.0, 2.0],
            "x and y kept"
        );
        lift.set(None, 3, &calls, &[], 100.0);
        lift.settle();
        assert_eq!(lift.place(2, [1.0, 2.0, 0.5]), [1.0, 2.0, 0.5]);
        assert_eq!(lift.last_by(), RankBy::Longest);
    }
}
