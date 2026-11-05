//! Browsing: what the selection touches lit where it sits; with nothing selected, an
//! arrow from each module to every module it calls.

use std::collections::HashSet;

use codeviz_layout::CallGraph;

use super::{
    Arrangement, Body, Flow, Link, Nucleus, Tube,
    journey::{distances, open_up},
};

impl Nucleus {
    /// Browsing: what `start` touches lit where it sits: its callers and its own
    /// calls, named, and their modules opened.
    pub(super) fn arrange_browse(&self, graph: &CallGraph, start: Option<usize>) -> Arrangement {
        let middle = start.and_then(|f| graph.cluster(f)).unwrap_or(0);
        let cluster_of = |tile: usize| graph.cluster(tile).unwrap_or(middle);
        let links = start.map_or_else(Vec::new, |f| Self::browse_calls(graph, f));
        let named = distances(&links, start);
        let columns = open_up(&links, start, self.counts.len(), &cluster_of);
        let mut balls = self.faint_balls();
        for (cluster, ball) in balls.iter_mut().enumerate() {
            if (start.is_some() && cluster == middle) || !columns[cluster].is_empty() {
                ball.body = Body::Open;
            }
        }
        let tubes = if start.is_some() {
            Vec::new()
        } else {
            self.map_tubes()
        };
        Arrangement {
            start,
            middle,
            balls,
            named,
            columns,
            outside: HashSet::new(),
            folded: HashSet::new(),
            tubes,
            links,
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
    fn browse_calls(graph: &CallGraph, start: usize) -> Vec<Link> {
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
