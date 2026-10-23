//! How every node pushes every other away, without visiting every pair. Small graphs
//! sum each pair exactly. Bigger ones sort their nodes into an octree (Barnes–Hut):
//! a far-off cell of nodes pushes as one body at its center of mass, so a step costs
//! about `n log n` instead of `n²`. Each node's push is summed on its own, in a fixed
//! order, so splitting the nodes across threads gives the same answer as one thread.

use std::num::NonZeroUsize;

use crate::graph::{add, dot, scale, sub};

/// Below this many nodes, every pair is summed exactly.
const EXACT_BELOW: usize = 400;
/// A cell this much narrower than its distance pushes as one body. Larger is faster
/// and rougher.
const THETA: f32 = 0.8;
/// Nodes a leaf holds before it splits.
const LEAF: usize = 8;
/// How deep the tree goes, so nodes sitting on one point cannot split forever.
const MAX_DEPTH: u32 = 20;
/// Nodes per thread, below which another thread costs more than it saves.
const PER_THREAD: usize = 256;

/// The push on each node from all the others, `strength / distance²` along the line
/// between them.
pub(crate) fn forces(positions: &[[f32; 3]], strength: f32) -> Vec<[f32; 3]> {
    if positions.len() < EXACT_BELOW {
        return exact(positions, strength);
    }
    let tree = Octree::new(positions);
    let mut forces = vec![[0.0_f32; 3]; positions.len()];
    let threads = std::thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .min(positions.len() / PER_THREAD)
        .max(1);
    let chunk = positions.len().div_ceil(threads);
    std::thread::scope(|scope| {
        for (index, slice) in forces.chunks_mut(chunk).enumerate() {
            let tree = &tree;
            scope.spawn(move || {
                for (offset, force) in slice.iter_mut().enumerate() {
                    *force = tree.push_on(index * chunk + offset, positions, strength);
                }
            });
        }
    });
    forces
}

/// Every pair, summed exactly.
fn exact(positions: &[[f32; 3]], strength: f32) -> Vec<[f32; 3]> {
    let count = positions.len();
    let mut forces = vec![[0.0_f32; 3]; count];
    for i in 0..count {
        for j in (i + 1)..count {
            let push = push(positions[i], positions[j], strength, 1.0);
            forces[i] = add(forces[i], push);
            forces[j] = sub(forces[j], push);
        }
    }
    forces
}

/// The push on a node at `on` from `mass` nodes at `from`.
fn push(on: [f32; 3], from: [f32; 3], strength: f32, mass: f32) -> [f32; 3] {
    let delta = sub(on, from);
    let distance_sq = dot(delta, delta).max(0.01);
    scale(delta, strength * mass / (distance_sq * distance_sq.sqrt()))
}

/// A cube of space: either its nodes, or the eight cubes it splits into.
#[derive(Debug, Clone)]
struct Cell {
    /// Edge length.
    size: f32,
    /// Where its nodes' mass centers, and how many there are.
    center: [f32; 3],
    mass: f32,
    /// Its nodes, a range of [`Octree::order`], when it is a leaf.
    leaf: Option<(usize, usize)>,
    /// Its occupied children, by index into [`Octree::cells`].
    children: Vec<usize>,
}

#[derive(Debug)]
struct Octree {
    cells: Vec<Cell>,
    /// Node indices, grouped so each leaf's are contiguous.
    order: Vec<usize>,
}

impl Octree {
    fn new(positions: &[[f32; 3]]) -> Self {
        let mut low = [f32::INFINITY; 3];
        let mut high = [f32::NEG_INFINITY; 3];
        for p in positions {
            for axis in 0..3 {
                low[axis] = low[axis].min(p[axis]);
                high[axis] = high[axis].max(p[axis]);
            }
        }
        let size = (0..3)
            .map(|axis| high[axis] - low[axis])
            .fold(0.0_f32, f32::max)
            .max(1e-3);
        let middle = scale(add(low, high), 0.5);
        let mut tree = Self {
            cells: Vec::with_capacity(positions.len() / 2),
            order: (0..positions.len()).collect(),
        };
        tree.build(positions, 0, positions.len(), middle, size, 0);
        tree
    }

    /// Builds the cell holding `order[start..end]`, a cube of `size` around `middle`,
    /// and returns its index.
    fn build(
        &mut self,
        positions: &[[f32; 3]],
        start: usize,
        end: usize,
        middle: [f32; 3],
        size: f32,
        depth: u32,
    ) -> usize {
        let members = &self.order[start..end];
        let mass = members.len() as f32;
        let center = scale(
            members
                .iter()
                .fold([0.0; 3], |sum, &node| add(sum, positions[node])),
            1.0 / mass,
        );
        let index = self.cells.len();
        self.cells.push(Cell {
            size,
            center,
            mass,
            leaf: None,
            children: Vec::new(),
        });
        if end - start <= LEAF || depth >= MAX_DEPTH {
            self.cells[index].leaf = Some((start, end));
            return index;
        }
        // Sort the members by octant, keeping their order within each.
        let octant = |p: [f32; 3]| {
            usize::from(p[0] >= middle[0])
                | usize::from(p[1] >= middle[1]) << 1
                | usize::from(p[2] >= middle[2]) << 2
        };
        let mut counts = [0_usize; 8];
        for &node in &self.order[start..end] {
            counts[octant(positions[node])] += 1;
        }
        let mut sorted = self.order[start..end].to_vec();
        sorted.sort_by_key(|&node| octant(positions[node]));
        self.order[start..end].copy_from_slice(&sorted);
        let mut first = start;
        let mut children = Vec::new();
        for (which, &count) in counts.iter().enumerate() {
            if count == 0 {
                continue;
            }
            let quarter = size / 4.0;
            let toward = |bit: usize| if which & bit == 0 { -quarter } else { quarter };
            let child_middle = add(middle, [toward(1), toward(2), toward(4)]);
            children.push(self.build(
                positions,
                first,
                first + count,
                child_middle,
                size / 2.0,
                depth + 1,
            ));
            first += count;
        }
        self.cells[index].children = children;
        index
    }

    /// The push on `node` from every other node.
    fn push_on(&self, node: usize, positions: &[[f32; 3]], strength: f32) -> [f32; 3] {
        let at = positions[node];
        let mut force = [0.0_f32; 3];
        let mut stack = vec![0_usize];
        while let Some(index) = stack.pop() {
            let cell = &self.cells[index];
            if let Some((start, end)) = cell.leaf {
                for &other in &self.order[start..end] {
                    if other != node {
                        force = add(force, push(at, positions[other], strength, 1.0));
                    }
                }
                continue;
            }
            let delta = sub(at, cell.center);
            let distance_sq = dot(delta, delta);
            if cell.size * cell.size < THETA * THETA * distance_sq {
                force = add(force, push(at, cell.center, strength, cell.mass));
            } else {
                stack.extend(cell.children.iter().rev());
            }
        }
        force
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Points spread through a cube, the same every run.
    fn scatter(count: usize) -> Vec<[f32; 3]> {
        let mut state = 0x2545_f491_u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state % 10_000) as f32 / 100.0
        };
        (0..count).map(|_| [next(), next(), next() * 0.2]).collect()
    }

    #[test]
    fn far_cells_push_about_as_their_nodes_would() {
        let points = scatter(1500);
        let tree = Octree::new(&points);
        let exact = exact(&points, 50.0);
        let length = |v: [f32; 3]| dot(v, v).sqrt();
        let (mut error, mut total) = (0.0, 0.0);
        for (node, &want) in exact.iter().enumerate() {
            error += length(sub(tree.push_on(node, &points, 50.0), want));
            total += length(want);
        }
        assert!(
            error / total < 0.03,
            "off by {:.1}% overall",
            error / total * 100.0
        );
    }

    #[test]
    fn threads_change_nothing() {
        let points = scatter(2000);
        let tree = Octree::new(&points);
        let alone: Vec<[f32; 3]> = (0..points.len())
            .map(|node| tree.push_on(node, &points, 50.0))
            .collect();
        assert_eq!(forces(&points, 50.0), alone);
    }

    #[test]
    fn nodes_on_one_point_do_not_split_forever() {
        let points = vec![[1.0, 2.0, 3.0]; 1000];
        let tree = Octree::new(&points);
        assert!(tree.cells.len() <= MAX_DEPTH as usize + 1);
        let force = tree.push_on(0, &points, 50.0);
        assert!(force.iter().all(|f| f.is_finite()));
    }
}
