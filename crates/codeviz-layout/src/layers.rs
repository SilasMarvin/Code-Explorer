//! The codebase as a layered flow diagram, flat: each module a vertical lane, each
//! function in its module's lane at a row given by its rank in the flow, how far down
//! the call graph it sits from the entry points (HTTP handlers, `main`, whatever
//! nothing calls). Control reads top to bottom and a module boundary is a lane's edge.
//!
//! Lanes run left to right roughly as the modules sit across the map, nudged towards
//! the lanes they call and are called by, so neighbors stay near. Within a lane's row,
//! functions sit under what calls them, so calls cross as little as they can. Nothing
//! depends on the focus, so the diagram can be learned.

use std::collections::VecDeque;

/// Spacing of the diagram, in world units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerConfig {
    /// Between two functions side by side in a lane.
    pub column: f32,
    /// Between two rows of functions within one rank, when a lane's rank wraps.
    pub row: f32,
    /// Extra space between one rank and the next.
    pub rank_gap: f32,
    /// Space between two lanes.
    pub lane_gap: f32,
    /// The most functions side by side in a lane before its rank wraps.
    pub max_columns: usize,
}

impl Default for LayerConfig {
    fn default() -> Self {
        Self {
            column: 4.5,
            row: 3.6,
            rank_gap: 9.0,
            lane_gap: 4.0,
            max_columns: 4,
        }
    }
}

/// One module's lane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lane {
    /// The cluster it holds.
    pub cluster: usize,
    /// Its left and right edges.
    pub left: f32,
    pub right: f32,
}

/// One rank's band across every lane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    pub rank: u32,
    /// Its upper and lower edges; `top` is the greater, as `y` runs up.
    pub top: f32,
    pub bottom: f32,
}

/// Where everything sits.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layers {
    /// Each node's place, `y` running up, so the entries are on top.
    pub positions: Vec<[f32; 2]>,
    /// Each node's rank in the flow.
    pub ranks: Vec<u32>,
    /// Lanes, left to right.
    pub lanes: Vec<Lane>,
    /// Bands, top to bottom.
    pub bands: Vec<Band>,
}

impl Layers {
    /// The lane holding `cluster`.
    #[must_use]
    pub fn lane(&self, cluster: usize) -> Option<&Lane> {
        self.lanes.iter().find(|lane| lane.cluster == cluster)
    }

    /// The top of the diagram.
    #[must_use]
    pub fn top(&self) -> f32 {
        self.bands.first().map_or(0.0, |band| band.top)
    }

    /// The bottom of the diagram.
    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.bands.last().map_or(0.0, |band| band.bottom)
    }
}

/// Lays out nodes in `clusters` (one per node) joined by `calls` (`(caller, callee)`),
/// ranked down from `entries`; `map_x` is where each node sits across the map, which
/// orders the lanes and breaks ties within them.
#[must_use]
pub fn layers(
    clusters: &[usize],
    calls: &[(usize, usize)],
    entries: &[usize],
    map_x: &[f32],
    config: &LayerConfig,
) -> Layers {
    let nodes = clusters.len();
    let ranks = ranks(nodes, calls, entries);
    let lane_order = lane_order(clusters, calls, map_x);
    let mut lane_of_cluster = vec![usize::MAX; clusters.iter().max().map_or(0, |c| c + 1)];
    for (lane, &cluster) in lane_order.iter().enumerate() {
        lane_of_cluster[cluster] = lane;
    }
    let rank_count = ranks.iter().max().map_or(0, |&r| r as usize + 1);
    // Each cell, a lane's rank, and what it holds, first in the map's order.
    let mut grid: Vec<Vec<Vec<usize>>> = vec![vec![Vec::new(); rank_count]; lane_order.len()];
    for node in 0..nodes {
        grid[lane_of_cluster[clusters[node]]][ranks[node] as usize].push(node);
    }
    for lane in &mut grid {
        for cell in lane {
            cell.sort_by(|&a, &b| map_x[a].total_cmp(&map_x[b]).then(a.cmp(&b)));
        }
    }
    let frame = Frame::new(&grid, &lane_order, config);
    let mut positions = vec![[0.0_f32; 2]; nodes];
    for (lane, cells) in grid.iter().enumerate() {
        for (rank, cell) in cells.iter().enumerate() {
            frame.place(cell, lane, rank, &mut positions);
        }
    }
    untangle(&mut grid, &frame, calls, &mut positions);
    Layers {
        positions,
        ranks,
        lanes: frame.lanes,
        bands: frame.bands,
    }
}

/// The lanes and bands a grid of cells takes, and how many columns each lane has.
struct Frame {
    lanes: Vec<Lane>,
    bands: Vec<Band>,
    columns: Vec<usize>,
    config: LayerConfig,
}

impl Frame {
    /// Each lane as wide as its widest rank, up to the most columns; each band as tall
    /// as its rank's most wrapped cell; all centered on the origin.
    fn new(grid: &[Vec<Vec<usize>>], lane_order: &[usize], config: &LayerConfig) -> Self {
        let columns: Vec<usize> = grid
            .iter()
            .map(|cells| {
                let widest = cells.iter().map(Vec::len).max().unwrap_or(1);
                widest.clamp(1, config.max_columns)
            })
            .collect();
        let rank_count = grid.first().map_or(0, Vec::len);
        let band_rows = (0..rank_count).map(|rank| {
            grid.iter()
                .zip(&columns)
                .map(|(cells, &per_row)| cells[rank].len().div_ceil(per_row))
                .max()
                .unwrap_or(1)
                .max(1)
        });
        let mut lanes = Vec::with_capacity(grid.len());
        let mut x = 0.0;
        for (&cluster, &per_row) in lane_order.iter().zip(&columns) {
            #[expect(clippy::cast_precision_loss, reason = "a few columns")]
            let width = per_row as f32 * config.column;
            lanes.push(Lane {
                cluster,
                left: x,
                right: x + width,
            });
            x += width + config.lane_gap;
        }
        let mut bands = Vec::with_capacity(rank_count);
        let mut y = 0.0;
        for (rank, rows) in band_rows.enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "a few rows")]
            let height = rows as f32 * config.row;
            #[expect(clippy::cast_possible_truncation, reason = "ranks came from u32")]
            bands.push(Band {
                rank: rank as u32,
                top: y,
                bottom: y - height,
            });
            y -= height + config.rank_gap;
        }
        let (width, height) = (x - config.lane_gap, -(y + config.rank_gap));
        for lane in &mut lanes {
            lane.left -= width / 2.0;
            lane.right -= width / 2.0;
        }
        for band in &mut bands {
            band.top += height / 2.0;
            band.bottom += height / 2.0;
        }
        Self {
            lanes,
            bands,
            columns,
            config: *config,
        }
    }

    /// Puts `cell`'s nodes in its lane's rank, in order, wrapping rows as they fill.
    fn place(&self, cell: &[usize], lane: usize, rank: usize, positions: &mut [[f32; 2]]) {
        let per_row = self.columns[lane];
        let config = &self.config;
        for (slot, &node) in cell.iter().enumerate() {
            let (row, column) = (slot / per_row, slot % per_row);
            // A short last row sits in the middle of the lane.
            let in_row = (cell.len() - row * per_row).min(per_row);
            #[expect(clippy::cast_precision_loss, reason = "a few slots")]
            let x = self.lanes[lane].left
                + config.column * ((per_row - in_row) as f32 / 2.0 + column as f32 + 0.5);
            #[expect(clippy::cast_precision_loss, reason = "a few rows")]
            let y = self.bands[rank].top - config.row * (row as f32 + 0.5);
            positions[node] = [x, y];
        }
    }
}

/// Reorders each cell so its nodes sit under what calls them, then over what they
/// call, then under their callers once more, so calls cross as little as they can.
fn untangle(
    grid: &mut [Vec<Vec<usize>>],
    frame: &Frame,
    calls: &[(usize, usize)],
    positions: &mut [[f32; 2]],
) {
    let nodes = positions.len();
    let mut above = vec![Vec::new(); nodes];
    let mut below = vec![Vec::new(); nodes];
    for &(from, to) in calls {
        if from < nodes && to < nodes && from != to {
            above[to].push(from);
            below[from].push(to);
        }
    }
    let rank_count = frame.bands.len();
    for (pass, neighbors) in [&above, &below, &above].into_iter().enumerate() {
        let order: Vec<usize> = if pass == 1 {
            (0..rank_count).rev().collect()
        } else {
            (0..rank_count).collect()
        };
        for rank in order {
            for (lane, cells) in grid.iter_mut().enumerate() {
                // The middle of a node's neighbors across, or where it is without any.
                let key = |node: usize| {
                    let near = &neighbors[node];
                    if near.is_empty() {
                        return positions[node][0];
                    }
                    #[expect(clippy::cast_precision_loss, reason = "a few calls")]
                    let count = near.len() as f32;
                    near.iter().map(|&n| positions[n][0]).sum::<f32>() / count
                };
                let mut keyed: Vec<(f32, usize)> =
                    cells[rank].iter().map(|&node| (key(node), node)).collect();
                keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                cells[rank] = keyed.into_iter().map(|(_, node)| node).collect();
                frame.place(&cells[rank], lane, rank, positions);
            }
        }
    }
}

/// Clusters left to right: by where their nodes sit across the map, then a few times
/// drawn towards the clusters they call and are called by, still held by where they
/// began, so lanes that talk sit near each other and the map's order mostly holds.
fn lane_order(clusters: &[usize], calls: &[(usize, usize)], map_x: &[f32]) -> Vec<usize> {
    let count = clusters.iter().max().map_or(0, |c| c + 1);
    let mut sum = vec![0.0_f32; count];
    let mut members = vec![0_usize; count];
    for (node, &cluster) in clusters.iter().enumerate() {
        sum[cluster] += map_x.get(node).copied().unwrap_or(0.0);
        members[cluster] += 1;
    }
    let mut order: Vec<usize> = (0..count).filter(|&c| members[c] > 0).collect();
    #[expect(clippy::cast_precision_loss, reason = "a node count")]
    let middle = |c: usize| sum[c] / members[c] as f32;
    order.sort_by(|&a, &b| middle(a).total_cmp(&middle(b)).then(a.cmp(&b)));
    // How many calls run between each pair of clusters.
    let mut weights: Vec<Vec<(usize, f32)>> = vec![Vec::new(); count];
    for &(from, to) in calls {
        let (Some(&a), Some(&b)) = (clusters.get(from), clusters.get(to)) else {
            continue;
        };
        if a != b {
            weights[a].push((b, 1.0));
            weights[b].push((a, 1.0));
        }
    }
    let mut slot = vec![0.0_f32; count];
    let mut home = vec![0.0_f32; count];
    for (index, &cluster) in order.iter().enumerate() {
        #[expect(clippy::cast_precision_loss, reason = "a cluster count")]
        let at = index as f32;
        slot[cluster] = at;
        home[cluster] = at;
    }
    for _ in 0..6 {
        let next: Vec<f32> = (0..count)
            .map(|c| {
                let pull: f32 = weights[c].iter().map(|&(other, w)| slot[other] * w).sum();
                let weight: f32 = weights[c].iter().map(|&(_, w)| w).sum();
                // Home counts as much as a handful of calls.
                let hold = 3.0;
                (pull + home[c] * hold) / (weight + hold)
            })
            .collect();
        order.sort_by(|&a, &b| next[a].total_cmp(&next[b]).then(a.cmp(&b)));
        for (index, &cluster) in order.iter().enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "a cluster count")]
            let at = index as f32;
            slot[cluster] = at;
        }
    }
    order
}

/// Each node's rank: the longest path of calls down to it from the entries and from
/// whatever nothing calls. Functions calling each other round a cycle share a rank, and
/// a declared entry ranks zero even when something calls it.
fn ranks(nodes: usize, calls: &[(usize, usize)], entries: &[usize]) -> Vec<u32> {
    let component = components(nodes, calls);
    let count = component.iter().max().map_or(0, |c| c + 1);
    let mut is_entry = vec![false; count];
    for &entry in entries {
        if let Some(&c) = component.get(entry) {
            is_entry[c] = true;
        }
    }
    let mut edges: Vec<(usize, usize)> = calls
        .iter()
        .filter(|&&(from, to)| from < nodes && to < nodes)
        .map(|&(from, to)| (component[from], component[to]))
        .filter(|&(from, to)| from != to && !is_entry[to])
        .collect();
    edges.sort_unstable();
    edges.dedup();
    let mut next = vec![Vec::new(); count];
    let mut incoming = vec![0_usize; count];
    for &(from, to) in &edges {
        next[from].push(to);
        incoming[to] += 1;
    }
    let mut rank = vec![0_u32; count];
    // In topological order, each takes the deepest of its callers plus one.
    let mut queue: VecDeque<usize> = (0..count).filter(|&c| incoming[c] == 0).collect();
    while let Some(from) = queue.pop_front() {
        for &to in &next[from] {
            rank[to] = rank[to].max(rank[from] + 1);
            incoming[to] -= 1;
            if incoming[to] == 0 {
                queue.push_back(to);
            }
        }
    }
    component.iter().map(|&c| rank[c]).collect()
}

/// The strongly connected component of each node, numbered from zero (Kosaraju's,
/// without recursion so a long call chain cannot overflow the stack).
fn components(nodes: usize, calls: &[(usize, usize)]) -> Vec<usize> {
    let mut forward = vec![Vec::new(); nodes];
    let mut backward = vec![Vec::new(); nodes];
    for &(from, to) in calls {
        if from < nodes && to < nodes {
            forward[from].push(to);
            backward[to].push(from);
        }
    }
    let mut finished = Vec::with_capacity(nodes);
    let mut visited = vec![false; nodes];
    for start in 0..nodes {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some((node, child)) = stack.pop() {
            if let Some(&to) = forward[node].get(child) {
                stack.push((node, child + 1));
                if !visited[to] {
                    visited[to] = true;
                    stack.push((to, 0));
                }
            } else {
                finished.push(node);
            }
        }
    }
    let mut component = vec![usize::MAX; nodes];
    let mut count = 0;
    for &start in finished.iter().rev() {
        if component[start] != usize::MAX {
            continue;
        }
        component[start] = count;
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            for &from in &backward[node] {
                if component[from] == usize::MAX {
                    component[from] = count;
                    stack.push(from);
                }
            }
        }
        count += 1;
    }
    component
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_on_top_and_leaves_below() {
        // 0 → 1 → 2 → 3, and 0 → 3 directly.
        let calls = [(0, 1), (1, 2), (2, 3), (0, 3)];
        assert_eq!(ranks(4, &calls, &[]), [0, 1, 2, 3]);
    }

    #[test]
    fn a_cycle_shares_one_rank() {
        let calls = [(0, 1), (1, 2), (2, 1), (2, 3)];
        assert_eq!(ranks(4, &calls, &[]), [0, 1, 1, 2]);
    }

    #[test]
    fn a_declared_entry_stays_on_top_when_called() {
        let calls = [(0, 1), (1, 2), (2, 3)];
        assert_eq!(ranks(4, &calls, &[2]), [0, 1, 0, 1]);
    }

    #[test]
    fn each_module_is_a_lane_and_calls_run_down() {
        // Two modules: 0 and 1 in cluster 0, 2 and 3 in cluster 1. 0 → 2 → 1 → 3.
        let clusters = [0, 0, 1, 1];
        let calls = [(0, 2), (2, 1), (1, 3)];
        let map_x = [0.0, 0.0, 10.0, 10.0];
        let layers = layers(&clusters, &calls, &[], &map_x, &LayerConfig::default());
        let lane = |node: usize| {
            let x = layers.positions[node][0];
            layers
                .lanes
                .iter()
                .find(|lane| lane.left <= x && x <= lane.right)
                .map(|lane| lane.cluster)
        };
        assert_eq!(lane(0), Some(0));
        assert_eq!(lane(1), Some(0));
        assert_eq!(lane(2), Some(1));
        assert_eq!(lane(3), Some(1));
        for &(from, to) in &calls {
            assert!(
                layers.positions[from][1] > layers.positions[to][1],
                "{from} calls {to}, so sits above it"
            );
        }
        assert!(
            layers.lanes[0].right < layers.lanes[1].left,
            "lanes do not overlap"
        );
    }

    #[test]
    fn lanes_follow_the_map_from_left_to_right() {
        let clusters = [0, 1, 2];
        let map_x = [5.0, -5.0, 0.0];
        let layers = layers(&clusters, &[], &[], &map_x, &LayerConfig::default());
        let order: Vec<usize> = layers.lanes.iter().map(|lane| lane.cluster).collect();
        assert_eq!(order, [1, 2, 0]);
    }

    #[test]
    fn a_crowded_rank_wraps_within_its_lane() {
        let clusters = [0; 9];
        let map_x = [0.0; 9];
        let config = LayerConfig::default();
        let layers = layers(&clusters, &[], &[], &map_x, &config);
        let lane = layers.lanes[0];
        #[expect(clippy::cast_precision_loss, reason = "a small count")]
        let widest = config.max_columns as f32 * config.column;
        assert!((lane.right - lane.left - widest).abs() < 1e-3);
        let band = layers.bands[0];
        for &[x, y] in &layers.positions {
            assert!(lane.left < x && x < lane.right);
            assert!(band.bottom < y && y < band.top);
        }
    }
}
