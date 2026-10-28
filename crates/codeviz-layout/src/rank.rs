//! Where each function sits in the flow of control: how far down the call graph it is
//! from the entry points (HTTP handlers, `main`, functions nothing calls). Entries rank
//! zero, what they call one more, and so on down to leaves such as storage and IO. A
//! rank depends on the call graph alone, never on what is focused, so it can be learned.
//!
//! Functions that call each other round a cycle share one rank. A declared entry ranks
//! zero even when something calls it; such a call runs uphill.

use std::collections::VecDeque;

/// Which path down from an entry a rank counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RankBy {
    /// The longest: every call runs downhill, except round a cycle or into an entry.
    #[default]
    Longest,
    /// The shortest: a function sits just below its nearest entry, so ranks stay
    /// shallow, and a call that runs uphill takes a shortcut back up.
    Shortest,
}

impl RankBy {
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Longest => Self::Shortest,
            Self::Shortest => Self::Longest,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Longest => "longest path",
            Self::Shortest => "shortest path",
        }
    }
}

/// Ranks `nodes` nodes joined by `calls` (`(caller, callee)`), counting down from
/// `entries` and from whatever nothing calls.
#[must_use]
pub fn ranks(nodes: usize, calls: &[(usize, usize)], entries: &[usize], by: RankBy) -> Vec<u32> {
    let component = components(nodes, calls);
    let count = component.iter().max().map_or(0, |c| c + 1);
    let mut is_entry = vec![false; count];
    for &entry in entries {
        if let Some(&c) = component.get(entry) {
            is_entry[c] = true;
        }
    }
    // Calls between cycles, each once, leaving out those into an entry.
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
    let roots: VecDeque<usize> = (0..count).filter(|&c| incoming[c] == 0).collect();
    match by {
        RankBy::Longest => {
            // In topological order, each takes the deepest of its callers plus one.
            let mut queue = roots;
            while let Some(from) = queue.pop_front() {
                for &to in &next[from] {
                    rank[to] = rank[to].max(rank[from] + 1);
                    incoming[to] -= 1;
                    if incoming[to] == 0 {
                        queue.push_back(to);
                    }
                }
            }
        }
        RankBy::Shortest => {
            let mut seen = vec![false; count];
            for &root in &roots {
                seen[root] = true;
            }
            let mut queue = roots;
            while let Some(from) = queue.pop_front() {
                for &to in &next[from] {
                    if !seen[to] {
                        seen[to] = true;
                        rank[to] = rank[from] + 1;
                        queue.push_back(to);
                    }
                }
            }
        }
    }
    component.iter().map(|&c| rank[c]).collect()
}

/// The strongly connected component of each node, numbered from zero (Kosaraju's, without
/// recursion so a long call chain cannot overflow the stack).
fn components(nodes: usize, calls: &[(usize, usize)]) -> Vec<usize> {
    let mut forward = vec![Vec::new(); nodes];
    let mut backward = vec![Vec::new(); nodes];
    for &(from, to) in calls {
        if from < nodes && to < nodes {
            forward[from].push(to);
            backward[to].push(from);
        }
    }
    // Nodes in the order their depth-first search finishes.
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
    // Backwards from the last to finish, each search gathers one component.
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
        assert_eq!(ranks(4, &calls, &[], RankBy::Longest), [0, 1, 2, 3]);
        assert_eq!(ranks(4, &calls, &[], RankBy::Shortest), [0, 1, 2, 1]);
    }

    #[test]
    fn a_cycle_shares_one_rank() {
        // 0 → 1 ⇄ 2 → 3
        let calls = [(0, 1), (1, 2), (2, 1), (2, 3)];
        assert_eq!(ranks(4, &calls, &[], RankBy::Longest), [0, 1, 1, 2]);
    }

    #[test]
    fn a_declared_entry_stays_on_top_when_called() {
        // 0 → 1 → 2, but 2 is an entry point too.
        let calls = [(0, 1), (1, 2), (2, 3)];
        assert_eq!(ranks(4, &calls, &[2], RankBy::Longest), [0, 1, 0, 1]);
    }

    #[test]
    fn nothing_calls_it_so_it_is_an_entry() {
        let calls = [(0, 2), (1, 2)];
        assert_eq!(ranks(4, &calls, &[], RankBy::Longest), [0, 0, 1, 0]);
    }
}
